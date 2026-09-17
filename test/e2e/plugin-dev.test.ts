// `modisa plugin dev`: a throwaway session with the plugin running and the TUI attached; detaching ends the
// session and removes its files, and the user's own state and config directories are left alone.
import { test, expect, afterAll } from "bun:test";
import { Screen, sandbox } from "../support/harness";

const sb = sandbox("plugin-dev");
const TMP = Bun.env.TMPDIR ?? "/tmp";
const throwaways = async () => (await Bun.$`ls -d ${TMP}/modisa-plugin-*`.quiet().nothrow().text()).split("\n").map((d) => d.replace(/\/$/, "")).filter(Boolean);
let ui: Screen | undefined;

afterAll(async () => {
  ui?.close();
  await sb.cleanup();
});

test("plugin dev runs the plugin in a throwaway session and cleans it up on exit", async () => {
  await Bun.$`mkdir -p ${sb.root}`.quiet();
  const dir = `${sb.root}/demo`;
  expect((await sb.run("unused", ["plugin", "new", "demo", "--dir", dir])).code).toBe(0);
  const before = new Set(await throwaways());

  ui = new Screen(["plugin", "dev", dir], sb.env, sb.root);
  await ui.until("the TUI", (s) => s.includes("SPACES"), 20000);
  const root = (await throwaways()).find((d) => !before.has(d));
  expect(root).toBeDefined();

  // the throwaway session has the plugin, connected
  const env = { MODISA_DIR: `${root}/state`, MODISA_CONFIG_DIR: `${root}/config` };
  let demo: any;
  for (let i = 0; i < 100 && !demo?.connected; i++) {
    await Bun.sleep(100);
    const r = await sb.run("check", ["plugin", "list", "--json"], env);
    if (r.code === 0) demo = JSON.parse(r.stdout).find((p: any) => p.name === "demo");
  }
  expect(demo).toMatchObject({ status: "running", connected: true });

  ui.write("\x02d"); // prefix, then detach
  const code = await Promise.race([ui.proc.exited, Bun.sleep(15000).then(() => "still running")]);
  expect(code).not.toBe("still running");
  expect(await Bun.file(`${root}/state/check.sock`).exists()).toBe(false);
  expect((await throwaways()).includes(root!)).toBe(false); // its files are gone

  // the sandbox's own session state and config were never used
  expect((await Bun.$`ls -A ${sb.root}/state`.quiet().nothrow().text()).trim()).toBe("");
  expect((await Bun.$`ls -A ${sb.root}/config/plugins`.quiet().nothrow().text()).trim()).toBe("");
}, 60000);
