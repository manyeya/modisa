// Behavioural tests for worktrees, run against a real throwaway session by `modisa plugin check .`. They make their own
// git repository in a temporary directory, with its own identity and no hooks, and remove it when they're done.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { realpath, stat } from "node:fs/promises";
import { checkSession } from "./modisa-plugin";

const s = checkSession();
let root = "";
let repo = "";

beforeAll(async () => {
  root = await realpath((await Bun.$`mktemp -d ${`${Bun.env.TMPDIR ?? "/tmp"}/worktrees-test-XXXXXX`}`.text()).trim());
  repo = `${root}/repo`;
  await Bun.$`mkdir -p ${root}/hooks && git init -q -b main ${repo}`.quiet();
  // whatever the user's git config says: an identity, no signing, and an empty hooks directory
  for (const [key, value] of [["user.name", "worktrees test"], ["user.email", "test@example.com"], ["commit.gpgsign", "false"], ["core.hooksPath", `${root}/hooks`]]) {
    await Bun.$`git -C ${repo} config ${key} ${value}`.quiet();
  }
  await Bun.$`git -C ${repo} commit -q --allow-empty -m init`.quiet();
});
// (awaited in an async hook: a hook that returns the shell's promise never finishes)
afterAll(async () => {
  await Bun.$`rm -rf ${root}`.quiet().nothrow();
});

// An action with these params, in the test repository unless they say otherwise. (No expect.any() in toMatchObject or
// toEqual on a result that's used again: bun's matchers write the matcher into the object they compare.)
const run = (action: string, params: Record<string, unknown> = {}) => s.modisa("plugin", "run", s.plugin, action, JSON.stringify({ repo, ...params }));
const result = async (action: string, params: Record<string, unknown> = {}) => {
  const r = await run(action, params);
  if (r.code !== 0) throw new Error(`${action} exited ${r.code}: ${r.stderr}`);
  return JSON.parse(r.stdout);
};
const spaces = () => s.json<{ id: string; name: string; cwd: string; active: boolean }[]>("workspace", "list");
const isDir = (path: string) => stat(path).then((x) => x.isDirectory(), () => false);

// A pane that modisa sees as a Claude Code agent in `state`: a long command in its shell, reported the way agent
// integrations do, until modisa shows the state.
async function agentIn(pane: string, state: "working" | "blocked") {
  await s.modisa("pane", "run", pane, "sleep 600");
  for (let i = 0; i < 10; i++) {
    await s.modisa("report", pane, "--source", "worktrees-test", "--agent", "claude-code", "--state", state);
    if ((await s.modisa("wait", pane, "--state", state, "--timeout", "1")).code === 0) return;
  }
  throw new Error(`${pane} never showed as ${state}`);
}

test("create makes a worktree on a new branch and a space in it; open finds that space instead of making another; list shows it; remove takes both away", async () => {
  const made = await result("create", { branch: "feat/login" });
  expect(made).toMatchObject({ branch: "feat/login", path: `${root}/repo-worktrees/feat-login` });
  expect(made.workspaceId).toBeString();
  expect((await Bun.$`git -C ${made.path} branch --show-current`.text()).trim()).toBe("feat/login");
  const space = (await spaces()).find((w) => w.id === made.workspaceId)!;
  expect(space).toMatchObject({ name: "feat/login", active: true });
  expect(await realpath(space.cwd)).toBe(made.path);

  // from another space, open twice (by branch, then by path): the same space, focused, and no new one
  expect((await s.modisa("workspace", "create", "elsewhere")).code).toBe(0);
  const before = (await spaces()).length;
  expect(await result("open", { branch: "feat/login" })).toMatchObject({ workspaceId: made.workspaceId, created: false });
  expect(await result("open", { path: made.path })).toMatchObject({ workspaceId: made.workspaceId, created: false });
  expect((await spaces()).length).toBe(before);
  expect((await spaces()).find((w) => w.active)?.id).toBe(made.workspaceId);

  const listed = await result("list");
  const head = (await Bun.$`git -C ${repo} rev-parse HEAD`.text()).trim();
  expect(listed).toEqual([
    { path: repo, branch: "main", head }, // the repository itself, which has no space
    { path: made.path, branch: "feat/login", head, space: { id: made.workspaceId, name: "feat/login" } },
  ]);

  expect(await result("remove", { branch: "feat/login" })).toEqual({ path: made.path, branch: "feat/login", closed: [made.workspaceId] });
  expect((await spaces()).some((w) => w.id === made.workspaceId)).toBe(false);
  expect(await isDir(made.path)).toBe(false);
  expect((await result("list")).map((t: { path: string }) => t.path)).toEqual([repo]);
}, 60_000);

test("open makes a space for a worktree that has none; remove takes a worktree that has no space", async () => {
  const path = `${root}/by-hand`;
  await Bun.$`git -C ${repo} worktree add -q -b by-hand ${path}`.quiet();
  const opened = await result("open", { branch: "by-hand" });
  expect(opened).toMatchObject({ path, branch: "by-hand", created: true });
  expect((await spaces()).filter((w) => w.name === "by-hand")).toHaveLength(1);
  expect((await s.modisa("workspace", "close", opened.workspaceId)).code).toBe(0);

  expect(await result("remove", { path })).toEqual({ path, branch: "by-hand", closed: [] });
  expect(await isDir(path)).toBe(false);
}, 60_000);

test("remove refuses while an agent in the space is working, changing nothing; force removes it anyway", async () => {
  const made = await result("create", { branch: "busy" });
  const pane = (await s.json<{ id: string; workspaceId?: string }[]>("pane", "list")).find((p) => p.workspaceId === made.workspaceId)!;
  await agentIn(pane.id, "working");

  const refused = await run("remove", { branch: "busy" });
  expect(refused.code).not.toBe(0);
  expect(refused.stderr).toContain(`${pane.id} is working`);
  expect((await spaces()).some((w) => w.id === made.workspaceId)).toBe(true);
  expect(await isDir(made.path)).toBe(true);

  expect(await result("remove", { branch: "busy", force: true })).toMatchObject({ closed: [made.workspaceId] });
  expect((await spaces()).some((w) => w.id === made.workspaceId)).toBe(false);
  expect(await isDir(made.path)).toBe(false);
}, 60_000);

test("with no repo in the params, create uses the active space's repository, and puts a worktree made from inside another next to it", async () => {
  const mine = (params: Record<string, unknown>) => s.modisa("plugin", "run", s.plugin, "create", JSON.stringify(params));
  expect((await s.modisa("workspace", "create", "outside", "--cwd", root)).code).toBe(0);
  const outside = await mine({ branch: "nowhere" });
  expect(outside.code).not.toBe(0);
  expect(outside.stderr).toContain("isn't in a git repository");

  expect((await s.modisa("workspace", "create", "in-repo", "--cwd", repo)).code).toBe(0);
  const first = JSON.parse((await mine({ branch: "first" })).stdout);
  expect(first.path).toBe(`${root}/repo-worktrees/first`);
  // the active space is now first's worktree
  const second = JSON.parse((await mine({ branch: "second" })).stdout);
  expect(second.path).toBe(`${root}/repo-worktrees/second`);
  expect((await result("list")).map((t: { branch: string }) => t.branch)).toEqual(["main", "first", "second"]);

  for (const branch of ["first", "second"]) await result("remove", { branch });
}, 60_000);

test("create refuses a branch that's already checked out, and names the open action", async () => {
  const taken = await run("create", { branch: "main" });
  expect(taken.code).not.toBe(0);
  expect(taken.stderr).toContain(`main is already checked out at ${repo}`);
  expect(taken.stderr).toContain("worktrees open");
});

// last: it closes every other space
test("remove refuses to close the session's only space, changing nothing", async () => {
  const made = await result("create", { branch: "last" });
  for (const w of await spaces()) if (w.id !== made.workspaceId) expect((await s.modisa("workspace", "close", w.id)).code).toBe(0);
  const refused = await run("remove", { branch: "last" });
  expect(refused.code).not.toBe(0);
  expect(refused.stderr).toContain("only one");
  expect((await spaces()).map((w) => w.id)).toEqual([made.workspaceId]);
  expect(await isDir(made.path)).toBe(true);
}, 60_000);
