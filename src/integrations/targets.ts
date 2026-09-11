// Every agent integration: where the agent keeps its config, what shepherd adds there, and how to
// take it out again. Two kinds:
//   lifecycle — hooks or a plugin that see every transition report working / blocked / idle
//               (and the session); they are the pane's state authority while they report.
//   session   — hooks that report only the agent's session id, for exact resume; state keeps coming
//               from the agent's screen, because these agents' hooks miss some transitions
//               (interrupts, permission answers).
import { self } from "../core/paths";
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
  install: () => Promise<void>;
  uninstall: () => Promise<void>;
  status: () => Promise<Status>;
};

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
        if (o.owned && o.key) delete root[o.key];
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

// The shepherd MCP server, registered through the agent's own CLI when it's installed.
const mcp = (cli: string, add: string[], remove: string[]) => async (install: boolean) => {
  if (!Bun.which(cli)) return;
  await Bun.$`${cli} ${remove}`.quiet().nothrow();
  if (install) await Bun.$`${cli} ${add} -- ${self()} mcp`.quiet().nothrow();
};

const claudeDir = () => env("CLAUDE_CONFIG_DIR") ?? `${home()}/.claude`;
const codexDir = () => env("CODEX_HOME") ?? `${home()}/.codex`;
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
    id: "claude-code", name: "Claude Code", binaries: ["claude"], kind: "session", dir: claudeDir,
    ...jsonHooks({
      file: () => `${claudeDir()}/settings.json`,
      entries: () => session("claude-code", "SessionStart"),
      add: (h, e, c) => addNested(h, e, c, { matcher: "*" }),
      after: mcp("claude", ["mcp", "add", "--scope", "user", "shepherd"], ["mcp", "remove", "--scope", "user", "shepherd"]),
    }),
  },
  {
    id: "codex", name: "Codex", binaries: ["codex"], kind: "session", dir: codexDir,
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
        await mcp("codex", ["mcp", "add", "shepherd"], ["mcp", "remove", "shepherd"])(install);
      },
    }),
  },
  {
    id: "copilot", name: "Copilot CLI", binaries: ["copilot"], kind: "session", dir: () => env("COPILOT_HOME") ?? `${home()}/.copilot`,
    ...jsonHooks({
      file: () => `${env("COPILOT_HOME") ?? `${home()}/.copilot`}/settings.json`,
      entries: () => session("copilot", "SessionStart"),
      add: (h, e, c) => addFlat(h, e, c, { timeoutSec: 10 }, "bash"),
    }),
  },
  {
    id: "cursor-agent", name: "Cursor Agent", binaries: ["cursor-agent"], kind: "session", dir: () => env("CURSOR_CONFIG_DIR") ?? `${home()}/.cursor`,
    ...jsonHooks({
      file: () => `${env("CURSOR_CONFIG_DIR") ?? `${home()}/.cursor`}/hooks.json`,
      entries: () => session("cursor-agent", "sessionStart"),
      prepare: (root) => void (root.version ??= 1),
      add: (h, e, c) => void (h[e] ??= []).push({ command: c }),
    }),
  },
  {
    id: "devin", name: "Devin CLI", binaries: ["devin"], kind: "session", dir: () => `${env("XDG_CONFIG_HOME") ?? `${home()}/.config`}/devin`,
    ...jsonHooks({
      file: () => `${env("XDG_CONFIG_HOME") ?? `${home()}/.config`}/devin/config.json`,
      entries: () => session("devin", "SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PermissionRequest", "Stop"),
      add: (h, e, c) => addNested(h, e, c),
    }),
  },
  {
    id: "droid", name: "Droid", binaries: ["droid"], kind: "session", dir: () => `${home()}/.factory`,
    ...jsonHooks({ file: () => `${home()}/.factory/settings.json`, entries: () => session("droid", "SessionStart"), add: (h, e, c) => addNested(h, e, c) }),
  },
  {
    id: "qodercli", name: "Qoder CLI", binaries: ["qodercli"], kind: "session", dir: () => env("QODER_CONFIG_DIR") ?? `${home()}/.qoder`,
    ...jsonHooks({ file: () => `${env("QODER_CONFIG_DIR") ?? `${home()}/.qoder`}/settings.json`, entries: () => session("qodercli", "SessionStart"), add: (h, e, c) => addNested(h, e, c, { matcher: "*" }) }),
  },
  {
    id: "qwen", name: "Qwen Code", binaries: ["qwen"], kind: "session", dir: () => env("QWEN_HOME") ?? `${home()}/.qwen`,
    ...jsonHooks({ file: () => `${env("QWEN_HOME") ?? `${home()}/.qwen`}/settings.json`, entries: () => session("qwen", "SessionStart"), add: (h, e, c) => addNested(h, e, c, { matcher: "*", timeout: 10_000 }) }),
  },
  {
    // Grok merges every hooks/*.json, so ours is a file of its own
    id: "grok", name: "Grok CLI", binaries: ["grok"], kind: "session", dir: () => env("GROK_CONFIG_DIR") ?? env("GROK_HOME") ?? `${home()}/.grok`,
    ...pluginFiles(() => [[`${env("GROK_CONFIG_DIR") ?? env("GROK_HOME") ?? `${home()}/.grok`}/hooks/shepherd.json`, () => JSON.stringify({ hooks: { SessionStart: [{ hooks: [{ type: "command", command: hookCommand("grok", "session"), timeout: 10 }] }] } }, null, 2) + "\n"]]),
  },
  {
    // Antigravity keys hooks.json by hook name: the "shepherd" block is ours
    id: "antigravity", name: "Antigravity CLI", binaries: ["agy"], kind: "session", dir: () => env("ANTIGRAVITY_CLI_CONFIG_DIR") ?? `${home()}/.gemini/config`,
    ...jsonHooks({
      file: () => `${env("ANTIGRAVITY_CLI_CONFIG_DIR") ?? `${home()}/.gemini/config`}/hooks.json`,
      key: "shepherd",
      owned: true,
      entries: () => session("antigravity", "PreInvocation"),
      add: (h, e, c) => addFlat(h, e, c, { timeout: 10 }),
    }),
  },
  {
    id: "hermes", name: "Hermes Agent", binaries: ["hermes"], kind: "session", dir: hermesDir,
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
    id: "kimi", name: "Kimi Code", binaries: ["kimi"], kind: "lifecycle", dir: () => env("KIMI_CODE_HOME") ?? `${home()}/.kimi-code`,
    ...(() => {
      const path = () => `${env("KIMI_CODE_HOME") ?? `${home()}/.kimi-code`}/config.toml`;
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
  { id: "opencode", name: "OpenCode", binaries: ["opencode"], kind: "lifecycle", dir: () => `${home()}/.config/opencode`, ...pluginFiles(() => [[`${home()}/.config/opencode/plugins/shepherd-agent-state.js`, () => opencodePlugin("opencode", self())]]) },
  { id: "kilo", name: "Kilo Code", binaries: ["kilo", "kilo-code"], kind: "lifecycle", dir: () => `${home()}/.config/kilo`, ...pluginFiles(() => [[`${home()}/.config/kilo/plugin/shepherd-agent-state.js`, () => opencodePlugin("kilo", self())]]) },
  { id: "pi", name: "Pi", binaries: ["pi"], kind: "lifecycle", dir: piDir, ...pluginFiles(() => [[`${piDir()}/extensions/shepherd-agent-state.ts`, () => piExtension("pi", self())]]) },
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
