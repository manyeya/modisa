// `shepherd integration`: every agent's hooks or plugin go into its own config, in its own format,
// next to the user's settings; status tells installed from outdated; uninstall takes out only ours.
import { test, expect, afterAll } from "bun:test";
import { sandbox } from "../support/harness";

const sb = sandbox("integrations");
const home = `${sb.root}/home`;
// its own HOME, and no real agents on PATH
const run = (...args: string[]) => sb.cli("x", args, { HOME: home, PATH: `${Bun.which("bun")!.replace(/\/bun$/, "")}:/usr/bin:/bin` });
const json = (path: string) => Bun.file(`${home}/${path}`).json();
const text = (path: string) => Bun.file(`${home}/${path}`).text();
const status = async () => Object.fromEntries((await run("integration", "status")).split("\n").map((l) => [l.slice(0, 16).trim(), l.slice(17, 37).trim()]));

afterAll(() => sb.cleanup());

test("install all, status, and uninstall across every agent's config format", async () => {
  // every agent set up, with settings of the user's own (and an older shepherd's hooks for Claude and Codex)
  const mine = { hooks: [{ type: "command", command: "echo mine" }] };
  await Bun.write(`${home}/.claude/settings.json`, JSON.stringify({ model: "opus", hooks: { Stop: [mine, { hooks: [{ type: "command", command: "SHEPHERD_HOOK=1 shepherd report --state done" }] }] } }));
  await Bun.write(`${home}/.codex/config.toml`, 'model = "o3"\nnotify = ["shepherd","report","--state","done"] # SHEPHERD_HOOK=1\n');
  await Bun.write(`${home}/.copilot/settings.json`, "{}");
  await Bun.write(`${home}/.cursor/hooks.json`, JSON.stringify({ version: 1, hooks: { stop: [{ command: "echo mine" }] } }));
  await Bun.write(`${home}/.config/devin/config.json`, "{}");
  await Bun.write(`${home}/.factory/settings.json`, "{}");
  await Bun.write(`${home}/.qoder/settings.json`, "{}");
  await Bun.write(`${home}/.qwen/settings.json`, "{}");
  await Bun.write(`${home}/.grok/config.toml`, "");
  await Bun.write(`${home}/.gemini/config/hooks.json`, JSON.stringify({ mine: { Stop: [] } }));
  await Bun.write(`${home}/.hermes/config.yaml`, "model: x\n");
  await Bun.write(`${home}/.kimi-code/config.toml`, 'theme = "dark"\n');
  for (const dir of [".config/opencode", ".config/kilo", ".pi/agent", ".omp/agent"]) await Bun.$`mkdir -p ${`${home}/${dir}`}`;

  let s = await status();
  expect(s["Claude Code"]).toBe("↻ update available"); // the older shepherd's hooks
  expect(s.Pi).toBe("not installed");

  // "all" installs what's here: every configured agent (MastraCode isn't, so it's left out)
  await run("integration", "install", "all");
  await run("integration", "install", "mastracode");
  s = await status();
  expect(Object.values(s).filter((v) => v !== "✓ installed")).toEqual([]);
  expect(Object.keys(s)).toHaveLength(17);

  // each agent's own format, the user's settings kept
  const claude = await json(".claude/settings.json");
  expect(claude.model).toBe("opus");
  expect(claude.hooks.Stop).toEqual([mine]); // the old state hook is gone
  expect(claude.hooks.SessionStart[0]).toMatchObject({ matcher: "*", hooks: [{ type: "command", timeout: 10 }] });
  expect(claude.hooks.SessionStart[0].hooks[0].command).toMatch(/^SHEPHERD_HOOK=2 .* hook claude-code session$/);
  expect((await json(".codex/hooks.json")).hooks.SessionStart[0].hooks[0].command).toContain("hook codex session");
  expect(await text(".codex/config.toml")).toBe('model = "o3"\n\n[features]\nhooks = true\n');
  expect((await json(".copilot/settings.json")).hooks.SessionStart[0]).toMatchObject({ type: "command", timeoutSec: 10 });
  expect((await json(".copilot/settings.json")).hooks.SessionStart[0].bash).toContain("hook copilot session");
  expect(await json(".cursor/hooks.json")).toMatchObject({ version: 1, hooks: { stop: [{ command: "echo mine" }] } });
  expect((await json(".cursor/hooks.json")).hooks.sessionStart[0].command).toContain("hook cursor-agent session");
  expect(Object.keys((await json(".config/devin/config.json")).hooks)).toEqual(["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PermissionRequest", "Stop"]);
  expect((await json(".qwen/settings.json")).hooks.SessionStart[0]).toMatchObject({ matcher: "*", hooks: [{ timeout: 10000 }] });
  expect((await json(".grok/hooks/shepherd.json")).hooks.SessionStart[0].hooks[0].command).toContain("hook grok session");
  const agy = await json(".gemini/config/hooks.json");
  expect(agy.mine).toEqual({ Stop: [] });
  expect(agy.shepherd.PreInvocation[0].command).toContain("hook antigravity session");
  const kimi = await text(".kimi-code/config.toml");
  expect(kimi).toStartWith('theme = "dark"\n\n# >>> shepherd kimi integration');
  expect(kimi.match(/\[\[hooks\]\]/g)).toHaveLength(12);
  expect(Bun.TOML.parse(kimi)).toMatchObject({ theme: "dark" });
  expect(Object.keys(await json(".mastracode/hooks.json"))).toContain("PermissionRequest");
  expect(await text(".hermes/config.yaml")).toBe("model: x\nplugins:\n  enabled:\n    - shepherd-agent-state\n");
  expect(await text(".hermes/plugins/shepherd-agent-state/__init__.py")).toContain('ctx.register_hook("on_session_start"');
  expect(await text(".config/opencode/plugins/shepherd-agent-state.js")).toContain("export const ShepherdAgentState");
  expect(await text(".config/kilo/plugin/shepherd-agent-state.js")).toContain('const AGENT = "kilo"');
  expect(await text(".pi/agent/extensions/shepherd-agent-state.ts")).toContain('pi.on("agent_settled"');
  expect(await text(".omp/agent/extensions/shepherd-omp-agent-state.ts")).toContain('pi.on("tool_approval_requested"');

  // reinstalling changes nothing; an edited file is outdated
  await run("integration", "install", "claude");
  expect((await json(".claude/settings.json")).hooks.SessionStart).toHaveLength(1);
  await Bun.write(`${home}/.pi/agent/extensions/shepherd-agent-state.ts`, "// edited");
  expect((await status()).Pi).toBe("↻ update available");

  // uninstall takes out only shepherd's parts
  await run("integration", "uninstall", "all");
  s = await status();
  expect(Object.values(s).filter((v) => v !== "not installed")).toEqual([]);
  expect(await json(".claude/settings.json")).toEqual({ model: "opus", hooks: { Stop: [mine] } });
  expect(await json(".cursor/hooks.json")).toEqual({ version: 1, hooks: { stop: [{ command: "echo mine" }] } });
  expect(await json(".gemini/config/hooks.json")).toEqual({ mine: { Stop: [] } });
  expect(await text(".kimi-code/config.toml")).toBe('theme = "dark"\n\n');
  expect(await text(".hermes/config.yaml")).toBe("model: x\nplugins:\n  enabled:\n");
  expect(await Bun.file(`${home}/.config/opencode/plugins/shepherd-agent-state.js`).exists()).toBe(false);
  expect(await Bun.file(`${home}/.grok/hooks/shepherd.json`).exists()).toBe(false);
}, 60000);

test("installing for an agent that isn't set up says so", async () => {
  expect(await run("integration", "install", "droid")).toContain(""); // set up above: fine
  await Bun.$`rm -rf ${`${home}/.qoder`}`;
  expect(await run("integration", "install", "qodercli")).toContain("isn't set up here");
});
