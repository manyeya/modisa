// Behavioural tests for attention-log, run against a real throwaway session by `shepherd plugin check .`
import { test, expect } from "bun:test";
import { checkSession } from "./shepherd-plugin";

const s = checkSession();
const entries = async () =>
  (await Bun.file(`${s.data}/attention.log`).text().catch(() => ""))
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));

// A pane shepherd sees as a Claude Code agent, whose state the test reports the way agent integrations do: a shell
// with a long-running process in the foreground. A report only sticks once that process is in the foreground, so
// it's repeated until shepherd shows the state.
async function agent(name: string) {
  const id = (await s.shepherd("pane", "split", "--name", name)).stdout;
  await s.shepherd("pane", "run", id, "sleep 600");
  return async (state: "working" | "blocked") => {
    for (let i = 0; i < 10; i++) {
      await s.shepherd("report", id, "--source", "attention-log-test", "--agent", "claude-code", "--state", state);
      if ((await s.shepherd("wait", id, "--state", state, "--timeout", "1")).code === 0) return;
    }
    throw new Error(`${name} never showed as ${state}`);
  };
}
const connected = () => s.until("the plugin to connect", async () => (await s.json<any[]>("plugin", "list")).find((p) => p.name === s.plugin)?.connected);

test("an agent already blocked at startup isn't logged; a newly blocked one is, once per time it blocks", async () => {
  const early = await agent("early");
  await early("blocked"); // the plugin is running, so this is news to it: logged

  // start the plugin again, so `early` is blocked in its startup snapshot
  expect((await s.shepherd("plugin", "stop", s.plugin)).code).toBe(0);
  const before = (await entries()).length;
  expect((await s.shepherd("plugin", "start", s.plugin)).code).toBe(0);
  await connected();
  const since = async () => (await entries()).slice(before).map((e) => e.name);

  const late = await agent("late");
  await late("working");
  await late("blocked");
  await s.until("late's entry", async () => (await since()).length > 0);
  await late("blocked"); // reported again, but still blocked: not a new entry
  await Bun.sleep(1000);
  expect(await since()).toEqual(["late"]); // and nothing for `early`, blocked since before the start

  // unblocked, then blocked again: that's new
  await late("working");
  await late("blocked");
  await s.until("the second entry", async () => (await since()).length === 2);
  expect(await since()).toEqual(["late", "late"]);

  const summary = await s.shepherd("plugin", "run", s.plugin, "summary");
  expect(JSON.parse(summary.stdout)).toMatchObject({ logged: 2 });
}, 90_000);
