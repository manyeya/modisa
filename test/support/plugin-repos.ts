// Local git repositories for the plugin manager's tests (no network): a bare repository holding a scaffolded plugin,
// or a marketplace, and later commits pushed to it.
import { expect } from "bun:test";
import type { Sandbox } from "./harness";

export const git = (cwd: string, ...args: string[]) => Bun.$`git -c user.name=test -c user.email=test@example.com -c init.defaultBranch=main ${args}`.cwd(cwd).quiet();
export type Repo = { work: string; bare: string; url: string; head: string };

// A bare repository with a scaffolded plugin (at `subdir`, or the root) and whatever `change` adds, one commit on main
export async function pluginRepo(sb: Sandbox, id: string, options: { plugin?: string; subdir?: string; change?: (dir: string) => Promise<unknown> } = {}): Promise<Repo> {
  const work = `${sb.root}/work-${id}`;
  const dir = options.subdir ? `${work}/${options.subdir}` : work;
  expect((await sb.run("unused", ["plugin", "new", options.plugin ?? id, "--dir", dir])).code).toBe(0);
  await options.change?.(dir);
  return commitBare(sb, id, work);
}

// `work` as one commit on main, cloned bare beside it
export async function commitBare(sb: Sandbox, id: string, work: string): Promise<Repo> {
  await git(work, "init", "--quiet");
  await git(work, "add", "-A");
  await git(work, "commit", "--quiet", "-m", id);
  const bare = `${sb.root}/${id}.git`;
  await Bun.$`git clone --quiet --bare ${work} ${bare}`.quiet();
  return { work, bare, url: `file://${bare}`, head: (await Bun.$`git -C ${bare} rev-parse HEAD`.text()).trim() };
}

// What `change` does in the working copy, committed and pushed to `branch`: the commit it made
export async function push(repo: Repo, change: (work: string) => Promise<unknown>, branch = "main") {
  await change(repo.work);
  await git(repo.work, "add", "-A");
  await git(repo.work, "commit", "--quiet", "-m", "change");
  await git(repo.work, "push", "--quiet", repo.bare, `HEAD:${branch}`);
  return (await Bun.$`git -C ${repo.bare} rev-parse ${branch}`.text()).trim();
}
