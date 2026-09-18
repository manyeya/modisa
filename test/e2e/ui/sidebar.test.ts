// What the sidebar says: each agent with its mark and the task it's on; each space with its repository's branch,
// what's to push and pull, and what's changed.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox } from "../../support/harness";

const sb = sandbox("sidebar");
const S = "sidebar";
const cli = (...args: string[]) => sb.cli(S, args);
let ui: Screen;
const work = `${sb.root}/work`;

beforeAll(async () => {
  // a clone 2 commits ahead of its upstream and 1 behind it (fetched, not pulled), with an untracked file
  const g = (dir: string, ...a: string[]) => Bun.$`git -C ${dir} -c user.email=t@t -c user.name=t ${a}`.quiet();
  await Bun.$`git init -q --bare -b main ${sb.root}/remote.git`.quiet();
  await Bun.$`git clone -q ${sb.root}/remote.git ${work} && git clone -q ${sb.root}/remote.git ${sb.root}/other`.quiet().nothrow();
  await g(work, "commit", "-q", "--allow-empty", "-m", "one");
  await g(work, "push", "-q", "origin", "main");
  await g(`${sb.root}/other`, "pull", "-q", "origin", "main");
  await g(`${sb.root}/other`, "commit", "-q", "--allow-empty", "-m", "theirs");
  await g(`${sb.root}/other`, "push", "-q", "origin", "main");
  await g(work, "fetch", "-q");
  await g(work, "commit", "-q", "--allow-empty", "-m", "two");
  await g(work, "commit", "-q", "--allow-empty", "-m", "three");
  await Bun.write(`${work}/notes.md`, "untracked\n");
  await Bun.write(`${sb.root}/config/config.toml`, "[sidebar]\nwidth = 40\n");
  ui = new Screen(["-s", S], sb.env, work);
  await ui.until("dashboard", (s) => s.includes("SPACES") && s.includes("+ agent"));
}, 30000);

afterAll(async () => {
  await cli("kill", S);
  ui?.close();
  await sb.cleanup();
});

test("a space shows its focused pane's repository: its branch, ↑ to push, ↓ to pull and ● changed; none outside one", async () => {
  await Bun.sleep(6000); // a poll or more: the session starts outside any repository
  expect(ui.text()).not.toContain("⎇");
  ui.write(`cd ${work}\r`);
  await ui.until("the space's git, once its pane is in the repository", (s) => s.includes("⎇ main ↑2 ↓1 ●1"), 15000);
  ui.write(`cd ${sb.root}\r`);
  await ui.until("nothing once it's out again", (s) => s.includes("SPACES") && !s.includes("⎇"), 15000);
}, 45000);

test("an agent shows its mark, its name and, under it, the task its terminal title names", async () => {
  const id = (await cli("pane", "split", "--name", "review")).trim();
  await cli("pane", "run", id, `printf '\\033]0;⠋ Pin the flaky test\\007'; sleep 600`);
  for (let i = 0; i < 10; i++) {
    await cli("report", id, "--source", "sidebar-test", "--agent", "codex", "--state", "working");
    if ((await sb.run(S, ["wait", id, "--state", "working", "--timeout", "1"])).code === 0) break;
  }
  await ui.until("the agent's row", (s) => /◎ @review/.test(s) && /^\s*Pin the flaky test\s+Working/m.test(s.split("\n").map((l) => l.slice(0, 44)).join("\n")), 15000);
}, 45000);
