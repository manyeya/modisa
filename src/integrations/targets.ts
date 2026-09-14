// Every agent integration: where the agent keeps its config, what shepherd adds there, and how to
// take it out again. Two kinds:
//   lifecycle — hooks or a plugin that see every transition report working / blocked / idle
//               (and the session); they are the pane's state authority while they report.
//   session   — hooks that report only the agent's session id, for exact resume; state keeps coming
//               from the agent's screen, because these agents' hooks miss some transitions
//               (interrupts, permission answers).
import { stableSelf as self } from "../core/paths"; // hooks outlive upgrades, so never a per-version path
import { addFlat, addNested, hooksOf, MARK, oursIn, readJson, removeOurs, withBlock, withCodexHooksFeature, withHermesPlugin, writeJson } from "./edit";
import { hermesPlugin, opencodePlugin, piExtension } from "./plugins";

export const HOOK_VERSION = 2;
export type Status = "current" | "outdated" | "none";

export type Target = {
  id: string; // the agent's id in shepherd
  name: string;
  binaries: string[]; // the agent is "available" when one of these is on PATH
  kind: "lifecycle" | "session";
  dir: () => string; // the agent's config directory; it must exist (the agent is set up) unless `create`
  create?: boolean;
  // Where this agent reads skills from, for the ones that do. Not derived from dir(): several agents
  // keep skills somewhere else entirely (see antigravity, kilo), and writing a skill into the wrong
  // directory is worse than shipping none, so every target opts in by hand.
  skills?: () => string;
  install: () => Promise<void>;
  uninstall: () => Promise<void>;
  status: () => Promise<Status>;
};

// The shepherd skill lives here once; every agent that reads skills somewhere else gets a symlink to
// it. This is also the directory Kimi and its family read natively, so they need no link.
export const skillsHome = () => `${home()}/.agents/skills`;
export const skillDir = () => `${skillsHome()}/shepherd`;

// The common case: the agent reads skills from its own config directory.
const skillsIn = (dir: () => string) => () => `${dir()}/skills`;

const env = (name: string) => Bun.env[name] || undefined;
const home = () => Bun.env.HOME ?? "/tmp";
const sh = (arg: string) => (/^[\w@%+=:,./-]+$/.test(arg) ? arg : `'${arg.replace(/'/g, `'\\''`)}'`);
// What an agent's hook runs: `shepherd hook <agent> <action>`, reading the agent's hook JSON on stdin.
export const hookCommand = (agent: string, action: string) => `${MARK}${HOOK_VERSION} ${self().map(sh).join(" ")} hook ${agent} ${action}`;

const exists = (path: string) => Bun.file(path).exists();
const same = (found: string[], want: string[]): Status => (!found.length ? "none" : found.join("\n") === [...want].sort().join("\n") ? "current" : "outdated");

// Agents whose hooks live in a JSON file: an events object (under `key`, or the file itself) holding
// shepherd's entries next to the user's own.
function jsonHooks(o: {
  file: () => string;
  key?: string | null; // null: events at the top level of the file
  owned?: boolean; // the key is ours alone: uninstall deletes it
  entries: () => [event: string, command: string][];
  add: (hooks: Record<string, any[]>, event: string, command: string) => void;
  prepare?: (root: any) => void;
  after?: (install: boolean) => Promise<void>;
}) {
  const events = (root: any) => (o.key === null ? root : hooksOf(root, o.key ?? "hooks"));
  return {
    install: async () => {
      const root = await readJson(o.file());
      o.prepare?.(root);
      const hooks = events(root);
      removeOurs(hooks);
      for (const [event, command] of o.entries()) o.add(hooks, event, command);
      await writeJson(o.file(), root);
      await o.after?.(true);
    },
    uninstall: async () => {
      if (!(await exists(o.file()))) return;
      const root = await readJson(o.file());
      if (removeOurs(events(root))) {
        // ours alone, or left empty by taking ours out: no `"hooks": {}` stays behind in their settings
        if (o.key !== null && (o.owned || !Object.keys(events(root)).length)) delete root[o.key ?? "hooks"];
        await writeJson(o.file(), root);
      }
      await o.after?.(false);
    },
    status: async () => {
      if (!(await exists(o.file()))) return "none" as Status;
      const root = await readJson(o.file()).catch(() => ({}));
      const hooks = o.key === null ? root : root[o.key ?? "hooks"];
      return same(oursIn(hooks), o.entries().map(([e, c]) => `${e}: ${c}`));
    },
  };
}

// Agents that load a file of ours (a plugin or extension): written whole, removed whole. Contents are
// built only when needed (they embed the command that runs shepherd).
function pluginFiles(files: () => [path: string, content: () => string][], after?: (install: boolean) => Promise<void>) {
  return {
    install: async () => {
      for (const [path, content] of files()) await Bun.write(path, content());
      await after?.(true);
    },
    uninstall: async () => {
      for (const [path] of files()) await Bun.file(path).delete().catch(() => {});
      await after?.(false);
    },
    status: async (): Promise<Status> => {
      const found = await Promise.all(files().map(async ([path, content]) => ((await exists(path)) ? (await Bun.file(path).text()) === content() : undefined)));
      return found.every((f) => f === undefined) ? "none" : found.every((f) => f === true) ? "current" : "outdated";
    },
  };
}

// Older shepherds registered an MCP server here; the skill replaced it. Take the stale registration
// out on install and on uninstall, so an upgrade doesn't leave a server behind that no longer exists.
// Deletable once 0.1.x is gone.
const dropMcp = (cli: string, remove: string[]) => async () => {
  if (Bun.which(cli)) await Bun.$`${cli} ${remove}`.quiet().nothrow();
};

const claudeDir = () => env("CLAUDE_CONFIG_DIR") ?? `${home()}/.claude`;
const codexDir = () => env("CODEX_HOME") ?? `${home()}/.codex`;
const copilotDir = () => env("COPILOT_HOME") ?? `${home()}/.copilot`;
const cursorDir = () => env("CURSOR_CONFIG_DIR") ?? `${home()}/.cursor`;
const devinDir = () => `${env("XDG_CONFIG_HOME") ?? `${home()}/.config`}/devin`;
const droidDir = () => `${home()}/.factory`;
const qoderDir = () => env("QODER_CONFIG_DIR") ?? `${home()}/.qoder`;
const qwenDir = () => env("QWEN_HOME") ?? `${home()}/.qwen`;
const grokDir = () => env("GROK_CONFIG_DIR") ?? env("GROK_HOME") ?? `${home()}/.grok`;
const antigravityDir = () => env("ANTIGRAVITY_CLI_CONFIG_DIR") ?? `${home()}/.gemini/config`;
const opencodeDir = () => `${home()}/.config/opencode`;
const kimiDir = () => env("KIMI_CODE_HOME") ?? `${home()}/.kimi-code`;
const piDir = () => env("PI_CODING_AGENT_DIR") ?? `${home()}/.pi/agent`;
const ompDir = () => env("PI_CODING_AGENT_DIR") ?? `${home()}/${env("PI_CONFIG_DIR") ?? ".omp"}/agent`;
const hermesDir = () => env("HERMES_HOME") ?? `${home()}/.hermes`;
const KIMI_BEGIN = "# >>> shepherd kimi integration (managed; reinstalling replaces this block)";
const KIMI_END = "# <<< shepherd kimi integration";
const KIMI_EVENTS: [string, string, string?][] = [
  ["SessionStart", "session"], ["UserPromptSubmit", "working"],
  ["PreToolUse", "working", "^(?!AskUserQuestion$).*$"], ["PreToolUse", "blocked", "^AskUserQuestion$"],
  ["PostToolUse", "working", "^AskUserQuestion$"], ["PostToolUseFailure", "working", "^AskUserQuestion$"],
  ["SubagentStart", "working"], ["PreCompact", "working"], ["PermissionRequest", "blocked"], ["PermissionResult", "working"],
  ["Stop", "idle"], ["Interrupt", "idle"],
];
const MASTRA_EVENTS: [string, string][] = [
  ["SessionStart", "session"], ["UserPromptSubmit", "working"], ["AgentStart", "working"], ["PreToolUse", "working"],
  ["PermissionRequest", "blocked"], ["PermissionResult", "working"], ["SubagentStart", "working"], ["SubagentEnd", "working"],
  ["Interrupt", "idle"], ["AgentEnd", "idle"], ["Stop", "idle"],
];
const session = (agent: string, ...events: string[]): [string, string][] => events.map((e) => [e, hookCommand(agent, "session")]);

export const TARGETS: Target[] = [
  {
    id: "claude-code", name: "Claude Code", binaries: ["claude"], kind: "session", dir: claudeDir, skills: skillsIn(claudeDir),
    ...jsonHooks({
      file: () => `${claudeDir()}/settings.json`,
      entries: () => session("claude-code", "SessionStart"),
      add: (h, e, c) => addNested(h, e, c, { matcher: "*" }),
      after: dropMcp("claude", ["mcp", "remove", "--scope", "user", "shepherd"]),
    }),
  },
  {
    id: "codex", name: "Codex", binaries: ["codex"], kind: "session", dir: codexDir, skills: skillsIn(codexDir),
    ...jsonHooks({
      file: () => `${codexDir()}/hooks.json`,
      entries: () => session("codex", "SessionStart"),
      add: (h, e, c) => addNested(h, e, c),
      after: async (install) => {
        const path = `${codexDir()}/config.toml`;
        const text = (await exists(path)) ? await Bun.file(path).text() : "";
        const kept = text.split("\n").filter((l) => !l.includes(MARK)).join("\n"); // our old notify line
        const next = install ? withCodexHooksFeature(kept) : kept;
        if (next !== text) await Bun.write(path, next);
        await dropMcp("codex", ["mcp", "remove", "shepherd"])();
      },
    }),
  },
  {
    id: "copilot", name: "Copilot CLI", binaries: ["copilot"], kind: "session", dir: copilotDir, skills: skillsIn(copilotDir),
    ...jsonHooks({
      file: () => `${copilotDir()}/settings.json`,
      entries: () => session("copilot", "SessionStart"),
      add: (h, e, c) => addFlat(h, e, c, { timeoutSec: 10 }, "bash"),
    }),
  },
  {
    id: "cursor-agent", name: "Cursor Agent", binaries: ["cursor-agent"], kind: "session", dir: cursorDir, skills: skillsIn(cursorDir),
    ...jsonHooks({
      file: () => `${cursorDir()}/hooks.json`,
      entries: () => session("cursor-agent", "sessionStart"),
      prepare: (root) => void (root.version ??= 1),
      add: (h, e, c) => void (h[e] ??= []).push({ command: c }),
    }),
  },
  {
    id: "devin", name: "Devin CLI", binaries: ["devin"], kind: "session", dir: devinDir, skills: skillsIn(devinDir),
    ...jsonHooks({
      file: () => `${devinDir()}/config.json`,
      entries: () => session("devin", "SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PermissionRequest", "Stop"),
      add: (h, e, c) => addNested(h, e, c),
    }),
  },
  {
    id: "droid", name: "Droid", binaries: ["droid"], kind: "session", dir: droidDir, skills: skillsIn(droidDir),
    ...jsonHooks({ file: () => `${droidDir()}/settings.json`, entries: () => session("droid", "SessionStart"), add: (h, e, c) => addNested(h, e, c) }),
  },
  {
    id: "qodercli", name: "Qoder CLI", binaries: ["qodercli"], kind: "session", dir: qoderDir, skills: skillsIn(qoderDir),
    ...jsonHooks({ file: () => `${qoderDir()}/settings.json`, entries: () => session("qodercli", "SessionStart"), add: (h, e, c) => addNested(h, e, c, { matcher: "*" }) }),
  },
  {
    id: "qwen", name: "Qwen Code", binaries: ["qwen"], kind: "session", dir: qwenDir, skills: skillsIn(qwenDir),
    ...jsonHooks({ file: () => `${qwenDir()}/settings.json`, entries: () => session("qwen", "SessionStart"), add: (h, e, c) => addNested(h, e, c, { matcher: "*", timeout: 10_000 }) }),
  },
  {
    // Grok merges every hooks/*.json, so ours is a file of its own
    id: "grok", name: "Grok CLI", binaries: ["grok"], kind: "session", dir: grokDir, skills: skillsIn(grokDir),
    ...pluginFiles(() => [[`${grokDir()}/hooks/shepherd.json`, () => JSON.stringify({ hooks: { SessionStart: [{ hooks: [{ type: "command", command: hookCommand("grok", "session"), timeout: 10 }] }] } }, null, 2) + "\n"]]),
  },
  {
    // Antigravity keys hooks.json by hook name: the "shepherd" block is ours. Its skills live beside
    // its config directory, not inside it.
    id: "antigravity", name: "Antigravity CLI", binaries: ["agy"], kind: "session", dir: antigravityDir, skills: () => `${home()}/.gemini/antigravity-cli/skills`,
    ...jsonHooks({
      file: () => `${antigravityDir()}/hooks.json`,
      key: "shepherd",
      owned: true,
      entries: () => session("antigravity", "PreInvocation"),
      add: (h, e, c) => addFlat(h, e, c, { timeout: 10 }),
    }),
  },
  {
    id: "hermes", name: "Hermes Agent", binaries: ["hermes"], kind: "session", dir: hermesDir, skills: skillsIn(hermesDir),
    ...pluginFiles(
      () => [[`${hermesDir()}/plugins/shepherd-agent-state/plugin.yaml`, () => hermesPlugin(self()).yaml], [`${hermesDir()}/plugins/shepherd-agent-state/__init__.py`, () => hermesPlugin(self()).py]],
      async (install) => {
        const path = `${hermesDir()}/config.yaml`;
        const text = (await exists(path)) ? await Bun.file(path).text() : "";
        const next = withHermesPlugin(text, "shepherd-agent-state", install);
        if (next !== text) await Bun.write(path, next);
        if (!install) await Bun.$`rm -rf ${`${hermesDir()}/plugins/shepherd-agent-state`}`.quiet().nothrow();
      },
    ),
  },
  {
    // Kimi reads the shared skills directory, which is where shepherd keeps the skill anyway: nothing
    // to link, the skill is simply there.
    id: "kimi", name: "Kimi Code", binaries: ["kimi"], kind: "lifecycle", dir: kimiDir, skills: skillsHome,
    ...(() => {
      const path = () => `${kimiDir()}/config.toml`;
      const q = (s: string) => JSON.stringify(s);
      const body = () => KIMI_EVENTS.map(([event, action, matcher]) => `[[hooks]]\nevent = ${q(event)}\n${matcher ? `matcher = ${q(matcher)}\n` : ""}command = ${q(hookCommand("kimi", action))}\ntimeout = 10\n\n`).join("");
      const read = async () => ((await exists(path())) ? await Bun.file(path()).text() : "");
      return {
        install: async () => void (await Bun.write(path(), withBlock(await read(), KIMI_BEGIN, KIMI_END, body()))),
        uninstall: async () => { if (await exists(path())) await Bun.write(path(), withBlock(await read(), KIMI_BEGIN, KIMI_END)); },
        status: async (): Promise<Status> => {
          const text = await read();
          return !text.includes(KIMI_BEGIN) ? "none" : text.includes(`${KIMI_BEGIN}\n${body()}${KIMI_END}`) ? "current" : "outdated";
        },
      };
    })(),
  },
  {
    // MastraCode's hooks.json maps events straight to handlers
    id: "mastracode", name: "MastraCode", binaries: ["mastracode"], kind: "lifecycle", dir: () => `${home()}/.mastracode`, create: true,
    ...jsonHooks({
      file: () => `${home()}/.mastracode/hooks.json`,
      key: null,
      entries: () => MASTRA_EVENTS.map(([e, a]) => [e, hookCommand("mastracode", a)]),
      add: (h, e, c) => addFlat(h, e, c, { timeout: 10_000, description: "Report MastraCode state to shepherd" }),
    }),
  },
  { id: "opencode", name: "OpenCode", binaries: ["opencode"], kind: "lifecycle", dir: opencodeDir, skills: skillsIn(opencodeDir), ...pluginFiles(() => [[`${opencodeDir()}/plugins/shepherd-agent-state.js`, () => opencodePlugin("opencode", self())]]) },
  // Kilo's plugin lives under ~/.config/kilo but it reads skills from ~/.kilo
  { id: "kilo", name: "Kilo Code", binaries: ["kilo", "kilo-code"], kind: "lifecycle", dir: () => `${home()}/.config/kilo`, skills: () => `${home()}/.kilo/skills`, ...pluginFiles(() => [[`${home()}/.config/kilo/plugin/shepherd-agent-state.js`, () => opencodePlugin("kilo", self())]]) },
  { id: "pi", name: "Pi", binaries: ["pi"], kind: "lifecycle", dir: piDir, skills: skillsIn(piDir), ...pluginFiles(() => [[`${piDir()}/extensions/shepherd-agent-state.ts`, () => piExtension("pi", self())]]) },
  {
    id: "omp", name: "OMP", binaries: ["omp"], kind: "lifecycle", dir: ompDir,
    ...(() => {
      const files = pluginFiles(() => [[`${ompDir()}/extensions/shepherd-omp-agent-state.ts`, () => piExtension("omp", self())]]);
      const install = async () => {
        if (ompDir() === piDir()) throw new Error("OMP and Pi share an agent directory, so Pi would load OMP's extension; give them separate directories first");
        await files.install();
      };
      return { ...files, install };
    })(),
  },
];
