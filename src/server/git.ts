// Each space's git state, for the sidebar: the repository its focused pane is in (following the pane's cd), the branch,
// how far it's ahead of and behind its upstream, and how many files have changes. Polled while a client is attached.
// Modisa never fetches, so "behind" is as of your last fetch; a repository too slow to answer in 2s shows nothing;
// and GIT_OPTIONAL_LOCKS=0 keeps `git status` from taking the index lock under your own git commands.
import type { GitView } from "../protocol/types";
import type { ServerContext } from "./context";
import { cwds } from "./persist/store";

const EVERY_MS = 5_000;

async function git(cwd: string, ...args: string[]) {
  const p = Bun.spawn(["git", "-C", cwd, ...args], { stdout: "pipe", stderr: "ignore", timeout: 2000, env: { ...Bun.env, GIT_OPTIONAL_LOCKS: "0" } });
  const out = await new Response(p.stdout).text();
  return (await p.exited) === 0 ? out : undefined;
}

// `git status --porcelain=v2 --branch`, read: the branch (a detached HEAD by its short commit), ahead/behind when there's
// an upstream, and a count of changed, staged and untracked files.
export function parseStatus(top: string, status: string): GitView {
  let branch = "", oid = "", changes = 0;
  let ab: { ahead: number; behind: number } | undefined;
  for (const line of status.split("\n")) {
    if (line.startsWith("# branch.head ")) branch = line.slice(14);
    else if (line.startsWith("# branch.oid ")) oid = line.slice(13);
    else if (line.startsWith("# branch.ab ")) {
      const [a, b] = line.slice(12).split(" ");
      ab = { ahead: Math.abs(Number(a)), behind: Math.abs(Number(b)) };
    } else if (line && !line.startsWith("#")) changes++;
  }
  return { repo: top.replace(/\/+$/, "").split("/").pop() || top, branch: branch === "(detached)" ? oid.slice(0, 7) : branch, ...ab, changes };
}

export async function gitStatus(cwd: string): Promise<GitView | undefined> {
  const top = (await git(cwd, "rev-parse", "--show-toplevel"))?.trim();
  if (!top) return undefined;
  const status = await git(cwd, "status", "--porcelain=v2", "--branch");
  return status === undefined ? undefined : parseStatus(top, status);
}

export function startGit(ctx: ServerContext): { stop(): void } {
  let running = false;
  const tick = async () => {
    if (running || ctx.down || !ctx.attached().length) return;
    running = true;
    try {
      const s = ctx.s;
      const focused = s.workspaces.map((ws) => s.panes.get(ws.tabs[ws.active]?.focused ?? ""));
      const live = await cwds(focused.flatMap((p) => (p?.info.status === "running" ? [p.proc.pid] : [])));
      const dirs = s.workspaces.map((ws, i) => {
        const p = focused[i];
        return (p && live.get(p.proc.pid)) ?? p?.info.cwd ?? ws.cwd;
      });
      const found = new Map(await Promise.all([...new Set(dirs)].map(async (d) => [d, await gitStatus(d)] as const)));
      let changed = false;
      s.workspaces.forEach((ws, i) => {
        const next = found.get(dirs[i]!);
        if (JSON.stringify(next) === JSON.stringify(ws.git)) return;
        ws.git = next;
        changed = true;
      });
      if (changed) ctx.changed();
    } catch (e) {
      console.error(`modisa: git status: ${e instanceof Error ? e.message : e}`);
    } finally {
      running = false;
    }
  };
  const timer = setInterval(tick, EVERY_MS);
  void tick();
  return { stop: () => clearInterval(timer) };
}
