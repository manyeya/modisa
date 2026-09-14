// Plugins: commands from [[plugin]] start with the server and get $SHEPHERD_SOCKET, and the example
// plugin in examples/plugins really does react to an agent getting blocked.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { Screen, sandbox } from "../support/harness";
import { installFakeAgent } from "../support/fake-agent";

const sb = sandbox("plugins");
const S = "plug";
const cli = (...args: string[]) => sb.cli(S, args);
const EXAMPLE = `${import.meta.dir}/../../examples/plugins/blocked-notifier/plugin.ts`;
let ui: Screen;

beforeAll(async () => {
  await installFakeAgent(sb.root);
  await Bun.write(
    `${sb.root}/config/config.toml`,
    `[[plugin]]\nrun = "echo $SHEPHERD_SOCKET > ${sb.root}/plugin.out"\n\n[[plugin]]\nrun = "bun ${EXAMPLE} >> ${sb.root}/notifier.log 2>&1"\n`,
  );
  ui = new Screen(["-s", S], sb.env, sb.root);
  await ui.until("attached", (s) => s.includes("SPACES"), 15000);
}, 30000);

afterAll(async () => {
  ui?.close();
  await cli("kill", S);
  await sb.cleanup();
});

const log = () => Bun.file(`${sb.root}/notifier.log`).text().catch(() => "");
const until = async (what: string, ok: (s: string) => boolean, ms = 20000) => {
  for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(200)) if (ok(await log())) return;
  throw new Error(`timed out waiting for ${what}; log so far:\n${await log()}`);
};

test("plugins start with SHEPHERD_SOCKET", async () => {
  for (let i = 0; i < 50 && !(await Bun.file(`${sb.root}/plugin.out`).exists()); i++) await Bun.sleep(100);
  expect((await Bun.file(`${sb.root}/plugin.out`).text()).trim()).toBe(`${sb.root}/state/${S}.sock`);
}, 15000);

test("the example plugin reports an agent that gets blocked", async () => {
  // events are never replayed, so only block an agent once the plugin says it's subscribed
  await until("the plugin to subscribe", (s) => s.includes("watching"));
  await cli("pane", "split", "--name", "asker", "fakeagent --ask");
  expect(await cli("wait", "@asker", "--state", "blocked", "--timeout", "10")).toBe("blocked");
  await until("the blocked report", (s) => s.includes("@asker") && s.includes("needs you"));
  // it reads the blocked pane, so the question itself comes through
  expect(await log()).toContain("Enter to confirm");
}, 40000);
