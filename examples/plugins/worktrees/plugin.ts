// worktrees: a space per git worktree. `create` adds a worktree (on a new branch, unless the branch exists) and opens a
// space in it; `open` focuses the space a worktree has, or opens one; `remove` closes that space and removes the
// worktree, but not while an agent in the space is working or blocked (unless `force`); `list` is the repository's
// worktrees, each with its space. The repository is `repo` from the params, else the one the shell is in, in the pane
// the action was taken on, else the active space's. A space belongs to a worktree when its directory is the worktree's,
// compared as real paths. Nothing is kept between actions: each reads the session and git afresh. Only `list` is in the
// command palette (plugin.json): the palette passes no params, and the others need a branch or a path, so they're run
// with `modisa plugin run worktrees <action> '{…}'`. Read AGENTS.md before changing it.
import { realpath, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, resolve } from "node:path";
import { runPlugin, type Client, type Pane } from "./modisa-plugin";

type Params = Record<string, unknown>;
type Call = { target?: { pane: string; instance: string } };
type Worktree = { path: string; head?: string; branch?: string; locked?: boolean };
type Repo = { top: string; main: string; trees: Worktree[] };
type Space = { id: string; name: string; cwd: string; active: number; tabs: { focused: string }[] };
type Session = { active: number; workspaces: Space[]; panes: (Pane & { workspaceId?: string })[] };

const GIT_TIMEOUT = 30_000;

const str = (value: unknown) => (typeof value === "string" && value.trim() ? value.trim() : undefined);
const isDir = (path: string) => stat(path).then((s) => s.isDirectory(), () => false);
const real = (path: string) => realpath(path).catch(() => resolve(path));
// ~ and a relative path, read the way a shell in `base` would
const expand = (path: string, base: string) => resolve(base, path.replace(/^~(?=$|\/)/, homedir()));
// a branch as one directory name: feat/login → feat-login
const slug = (branch: string) => branch.replace(/[^\w.-]+/g, "-");
const nameOf = (tree: Worktree) => tree.branch ?? basename(tree.path);

// git, run in `cwd`: what it printed, or an error with its stderr. It never waits on a prompt and is stopped after 30s.
async function git(cwd: string, ...args: string[]) {
  if (!(await isDir(cwd))) throw new Error(`${cwd} doesn't exist`);
  const started = Date.now();
  const p = Bun.spawn(["git", ...args], { cwd, stdin: "ignore", stdout: "pipe", stderr: "pipe", timeout: GIT_TIMEOUT, env: { ...Bun.env, GIT_TERMINAL_PROMPT: "0" } });
  const [out, err, code] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text(), p.exited]);
  // (Bun sets `killed` once a process has exited at all, so a timeout shows as a signal at the deadline)
  if (p.signalCode) throw new Error(`git ${args.join(" ")} ${Date.now() - started >= GIT_TIMEOUT ? `didn't finish within ${GIT_TIMEOUT / 1000}s, so it was stopped` : `ended with ${p.signalCode}`}`);
  if (code !== 0) throw new Error(`git ${args.join(" ")}: ${err.trim() || `exit code ${code}`}`);
  return out.trim();
}

// `git worktree list --porcelain`: a block per worktree, the main one first
async function worktrees(dir: string): Promise<Worktree[]> {
  return (await git(dir, "worktree", "list", "--porcelain")).split(/\n\n+/).filter(Boolean).map((block) => {
    const tree: Worktree = { path: "" };
    for (const line of block.split("\n")) {
      const gap = line.indexOf(" ");
      const key = gap < 0 ? line : line.slice(0, gap);
      const value = gap < 0 ? "" : line.slice(gap + 1);
      if (key === "worktree") tree.path = value;
      else if (key === "HEAD") tree.head = value;
      else if (key === "branch") tree.branch = value.replace(/^refs\/heads\//, "");
      else if (key === "locked") tree.locked = true;
    }
    return tree;
  });
}

// The repository `dir` is in: the worktree it's in (top), the main worktree, and all of them.
async function repoAt(dir: string): Promise<Repo> {
  const top = await git(dir, "rev-parse", "--show-toplevel").catch((error: Error) => {
    throw new Error(`${dir} isn't in a git repository (${error.message.split("\n")[0]}): pass {"repo": "/path/to/repo"}, or take the action from a pane in one`);
  });
  const trees = await worktrees(top);
  return { top, main: trees[0]?.path ?? top, trees };
}

runPlugin(async (modisa: Client) => {
  const session = () => modisa.request<Session>("session.info", { snapshot: true });

  // The directory an action is about: where the shell is now in the pane it was taken on, else the active space's.
  const here = async (call: Call, s: Session) => {
    if (call.target) {
      const seen = await modisa.request<{ process?: { cwd?: string } }>("debug.detect", { target: call.target.pane }).catch(() => undefined);
      if (seen?.process?.cwd) return seen.process.cwd;
    }
    return s.workspaces[s.active]?.cwd ?? homedir();
  };
  const repo = async (params: Params, where: string) => repoAt(str(params.repo) ? expand(str(params.repo)!, where) : where);

  // the spaces whose directory is the worktree's
  const spacesAt = async (s: Session, path: string) => {
    const want = await real(path);
    const at = await Promise.all(s.workspaces.map(async (w) => ((await real(w.cwd)) === want ? w : undefined)));
    return at.filter((w): w is Space => !!w);
  };

  // The worktree the params name, by branch or by path. A path is looked up in its own repository (unless `repo` says
  // otherwise, or the directory is gone); a branch in the repository the action is about.
  const named = async (params: Params, call: Call, s: Session) => {
    const branch = str(params.branch), path = str(params.path);
    if (!branch === !path) throw new Error(`name the worktree, by branch or by path: '{"branch":"feat-x"}' or '{"path":"/path/to/it"}'`);
    const where = await here(call, s);
    const at = path && expand(path, where);
    const r = at && !str(params.repo) && (await isDir(at)) ? await repoAt(at) : await repo(params, where);
    const want = at && (await real(at));
    for (const tree of r.trees) if (branch ? tree.branch === branch : (await real(tree.path)) === want) return { tree, repo: r };
    throw new Error(branch ? `no worktree of ${r.main} has ${branch} checked out (create makes one)` : `${at} isn't a worktree of ${r.main}`);
  };

  // A new space in a worktree, named for its branch: workspace.create, which also makes it the space on screen.
  const newSpace = async (tree: Worktree, command?: string) =>
    (await modisa.request<{ workspaceId: string }>("workspace.create", { name: nameOf(tree), cwd: tree.path, ...(command && { command }) })).workspaceId;

  // A toast is a courtesy: one over the rate limit (3 every 10s) is logged, never the action's error.
  const toast = (text: string) => modisa.ui.toast(text, { tone: "done" }).catch((error: Error) => console.error(`worktrees: no toast (${error.message}): ${text}`));

  await modisa.hello({
    list: async (params, call) => {
      const s = await session();
      const r = await repo(params, await here(call, s));
      return Promise.all(
        r.trees.map(async (tree) => {
          const [space] = await spacesAt(s, tree.path);
          return { path: tree.path, branch: tree.branch, head: tree.head, ...(space && { space: { id: space.id, name: space.name } }) };
        }),
      );
    },

    // {branch, base?, path?, repo?, command?}: the branch checked out in a new worktree, then a space in it
    create: async (params, call) => {
      const branch = str(params.branch);
      if (!branch) throw new Error(`create needs a branch: modisa plugin run worktrees create '{"branch":"feat-x"}'`);
      const base = str(params.base);
      if (base?.startsWith("-")) throw new Error(`base ${base} isn't a commit or a branch`);
      const s = await session();
      const where = await here(call, s);
      const r = await repo(params, where);
      await git(r.top, "check-ref-format", "--branch", branch); // fails, saying why, on a name git won't take
      const taken = r.trees.find((t) => t.branch === branch);
      if (taken) throw new Error(`${branch} is already checked out at ${taken.path}: modisa plugin run worktrees open '{"branch":"${branch}"}'`);
      const exists = await git(r.top, "rev-parse", "--verify", "--quiet", `refs/heads/${branch}`).then(() => true, () => false);
      if (exists && base) throw new Error(`${branch} already exists, so it isn't made from ${base}: leave base out to check it out as it is`);
      // next to the main worktree, so a worktree made from inside another one isn't nested in it
      const path = str(params.path) ? expand(str(params.path)!, where) : `${dirname(r.main)}/${basename(r.main)}-worktrees/${slug(branch)}`;
      await git(r.top, "worktree", "add", ...(exists ? [path, branch] : ["-b", branch, path, ...(base ? [base] : [])]));
      const tree = { path: await real(path), branch };
      const workspaceId = await newSpace(tree, str(params.command));
      await toast(`worktree ${branch} at ${tree.path}`);
      return { path: tree.path, branch, workspaceId };
    },

    // {branch | path, repo?, command?}: the worktree's space focused, or a new one when it has none
    open: async (params, call) => {
      const s = await session();
      const { tree } = await named(params, call, s);
      const [space] = await spacesAt(s, tree.path);
      if (space) {
        await modisa.request("pane.focus", { target: space.tabs[space.active]!.focused });
        return { path: tree.path, branch: tree.branch, workspaceId: space.id, created: false };
      }
      const workspaceId = await newSpace(tree, str(params.command));
      await toast(`opened worktree ${nameOf(tree)}`);
      return { path: tree.path, branch: tree.branch, workspaceId, created: true };
    },

    // {branch | path, repo?, force?}: its spaces closed, then the worktree removed. Each refusal comes before anything
    // is closed, so a refused remove changes nothing.
    remove: async (params, call) => {
      const s = await session();
      const { tree, repo: r } = await named(params, call, s);
      const name = nameOf(tree);
      const force = params.force === true;
      if ((await real(tree.path)) === (await real(r.main))) throw new Error(`${tree.path} is the repository itself (its main worktree): remove only takes worktrees added to it`);
      const spaces = await spacesAt(s, tree.path);
      const ids = new Set(spaces.map((w) => w.id));
      const busy = s.panes.filter((p) => p.workspaceId && ids.has(p.workspaceId) && (p.agent?.state === "working" || p.agent?.state === "blocked"));
      if (busy.length && !force) {
        const who = busy.map((p) => `${p.name ? `@${p.name}` : p.id} is ${p.agent!.state}`).join(", ");
        throw new Error(`not while an agent in ${name}'s space is busy (${who}): let it finish, or pass {"force": true}`);
      }
      if (spaces.length && spaces.length >= s.workspaces.length) throw new Error(`${name}'s space is the session's only one, and modisa never closes the last space: open another (modisa workspace create), then remove it`);
      if (tree.locked && !force) throw new Error(`${tree.path} is locked (git worktree unlock it), or pass {"force": true}`);
      if (!force && (await isDir(tree.path)) && (await git(tree.path, "status", "--porcelain"))) {
        throw new Error(`${tree.path} has changes that removing it would lose: commit or stash them, or pass {"force": true}`);
      }
      for (const w of spaces) await modisa.request("workspace.close", { workspace: w.id });
      // twice for a locked one: that's how git takes force for those
      await git(r.main, "worktree", "remove", ...(force ? ["--force"] : []), ...(force && tree.locked ? ["--force"] : []), tree.path).catch((error: Error) => {
        throw new Error(spaces.length ? `closed its space, but the worktree is still there: ${error.message}` : error.message);
      });
      await toast(`removed worktree ${name}`);
      return { path: tree.path, branch: tree.branch, closed: [...ids] };
    },
  });
  console.log("worktrees: ready");
});
