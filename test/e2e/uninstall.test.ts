// `shepherd uninstall`: asks first, then takes out every integration and the skill, stops running
// sessions and deletes state, keeping config unless --purge. Never from inside a shepherd pane.
import { test, expect, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";

const sb = sandbox("uninstall");
const home = `${sb.root}/home`;
// its own HOME, and no real agents on PATH: uninstall must never reach the real ones
const env = { HOME: home, PATH: `${Bun.which("bun")!.replace(/\/bun$/, "")}:/usr/bin:/bin` };
const run = (...args: string[]) => sb.cli("gone", args, env);
const exists = (path: string) => Bun.$`test -e ${path}`.quiet().nothrow().then((r) => r.exitCode === 0);

afterAll(async () => {
  await sb.cli("gone", ["kill", "gone"], env);
  await sb.cleanup();
});

test("uninstall removes integrations, sessions and state, and keeps config unless --purge", async () => {
  await Bun.write(`${home}/.claude/settings.json`, JSON.stringify({ model: "opus" }));
  await Bun.$`mkdir -p ${`${home}/.config/opencode`}`;
  await run("integration", "install", "all");
  expect(await exists(`${home}/.agents/skills/shepherd/SKILL.md`)).toBe(true);
  await Bun.write(`${sb.root}/config/config.toml`, 'theme = "nord"\n');
  const server = await startServer(sb, "gone", env);

  // without --yes it asks, and with no answer nothing changes
  expect(await run("uninstall")).toContain("nothing removed");
  expect(await exists(`${home}/.config/opencode/plugins/shepherd-agent-state.js`)).toBe(true);

  // inside a shepherd pane it refuses: it would stop its own session
  expect(await sb.cli("gone", ["uninstall", "--yes"], { ...env, SHEPHERD_SOCKET: `${sb.root}/state/gone.sock` })).toContain("outside shepherd");

  const out = await run("uninstall", "--yes");
  expect(out).toContain("Claude Code: removed");
  expect(out).toContain("stopped session gone");
  expect(out).toContain("runs from source");
  expect(await Bun.file(`${home}/.claude/settings.json`).json()).toEqual({ model: "opus" });
  expect(await exists(`${home}/.config/opencode/plugins/shepherd-agent-state.js`)).toBe(false);
  expect(await exists(`${home}/.claude/skills/shepherd`)).toBe(false);
  expect(await exists(`${home}/.agents/skills/shepherd`)).toBe(false);
  expect(await exists(`${sb.root}/state`)).toBe(false);
  expect(await Promise.race([server.exited.then(() => "exited"), Bun.sleep(5000).then(() => "running")])).toBe("exited");
  expect(await Bun.file(`${sb.root}/config/config.toml`).text()).toBe('theme = "nord"\n');

  expect(await run("uninstall", "--yes", "--purge")).toContain(`deleted ${sb.root}/config`);
  expect(await exists(`${sb.root}/config`)).toBe(false);
}, 60000);
