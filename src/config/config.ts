// ~/.config/modisa/config.toml, merged over defaults; changes apply live.
import { HOME } from "../core/paths";
import type { NotifyEvent } from "../protocol/types";
import { THEMES } from "./themes";

export const CONFIG_DIR = Bun.env.MODISA_CONFIG_DIR ?? `${HOME}/.config/modisa`;
export const CONFIG_PATH = `${CONFIG_DIR}/config.toml`;

export type NotifyKind = "toast" | "system" | "sound" | "bell";
export type Policy = "allow" | "ask" | "deny";
export type IndicatorStyle = "symbols" | "dots" | "letters";
export type BorderStyle = "single" | "rounded" | "double" | "heavy";

export type Config = {
  prefix: string;
  theme: string;
  mouse: { hover: boolean }; // hover: the pointer resting on a list row selects it
  sidebar: { visible: boolean; width: number; agents: string; logos: "auto" | "on" | "off"; graph: boolean }; // agents: a plugin whose section replaces the AGENTS list
  status: { agents: boolean; panes: boolean; theme: boolean }; // what the status row shows besides the buttons
  git: { status: boolean; repo: boolean; counts: boolean; changes: boolean }; // the active space's repository in the status row
  panes: { border: BorderStyle };
  notify: Record<NotifyEvent, NotifyKind[]>;
  sound: { volume: number } & Record<NotifyEvent, string>; // a cuelume sound name per event
  indicators: { style: IndicatorStyle; tab: boolean; pane: boolean; sidebar: boolean };
  pane_labels: { agent: boolean };
  update: { check: boolean; channel: "stable" | "staging" };
  messaging: { max_hops: number; per_minute: number };
  permissions: { keys_foreign: Policy; close_foreign: Policy; run_foreign: Policy };
  agents: Record<string, { launch?: string; resume?: string }>;
  plugin: { run: string }[];
  plugin_keys: Record<string, string>; // "<plugin>.<action or pane>" → key ("" turns it off); outranks plugin.json
  keys: Record<string, string | string[]>; // action → its key(s) after the prefix, in place of modisa's ("" for none): see keys.ts
  remote_command: string;
};

export const DEFAULTS: Config = {
  prefix: "C-b",
  theme: "ion",
  mouse: { hover: true },
  sidebar: { visible: true, width: 26, agents: "", logos: "auto", graph: false },
  status: { agents: true, panes: true, theme: false },
  git: { status: true, repo: true, counts: true, changes: true },
  panes: { border: "single" },
  notify: { blocked: ["toast", "system", "sound"], done: ["toast"], working: [] },
  sound: { volume: 0.7, blocked: "chime", done: "success", working: "loading" },
  indicators: { style: "symbols", tab: true, pane: true, sidebar: true },
  pane_labels: { agent: true },
  update: { check: true, channel: "stable" },
  messaging: { max_hops: 10, per_minute: 5 },
  permissions: { keys_foreign: "ask", close_foreign: "ask", run_foreign: "ask" },
  agents: {},
  plugin: [],
  plugin_keys: {},
  keys: {},
  remote_command: "modisa",
};

export const SAMPLE = `# modisa config — changes apply live (Ctrl+B s opens the settings page)
prefix = "C-b"              # C-<key>
theme = "ion"               # ion, tokyonight, catppuccin-mocha, gruvbox, nord, dracula, bearded-* (see settings)

[sidebar]
visible = true
width = 26                  # 20 to 48 columns, at most a third of the terminal; dragging its edge sets it
agents = ""                 # a plugin whose sidebar section takes the AGENTS list's place ("radar"); "" keeps modisa's
logos = "auto"              # agents' logos where the terminal can show them (modisa logos); "on", or "off" for plain marks
graph = false               # true draws the AGENTS list as a git graph of its tabs; the dots stay either way

[status]                    # the bottom row, besides its buttons
agents = true               # how many agents are working and need you
panes = true                # how many panes this tab has
theme = false               # the theme's name (click it to change theme)

[git]                       # the active space's repository, on the right of the status row
status = true               # its branch (green when clean and in step with its upstream)
repo = true                 # the repository's name before it
counts = true               # ↑ commits to push, ↓ commits to pull
changes = true              # ● files changed

[panes]
border = "single"           # single, rounded, double or heavy

[mouse]                     # clicks, drags, the wheel and right-click always work (Shift-drag selects text natively)
hover = true                # the pointer resting on a row of a menu, picker or the settings page selects it

[notify]                    # toast, system, sound, bell — when an agent you're not looking at…
blocked = ["toast", "system", "sound"]   # …needs you
done = ["toast"]                         # …finished
working = []                             # …started working

[sound]                     # the sound each event plays (cuelume): chime, sparkle, droplet, bloom,
volume = 0.7                # whisper, tick, press, release, toggle, success, error, page, loading,
blocked = "chime"           # ready, pulse, scan, arrival
done = "success"
working = "loading"

[indicators]
style = "symbols"           # symbols ! ◆ ✓ ○ · dots ● ● ● ○ · letters B W D I
tab = true                  # badge on tabs with an agent that needs you
pane = true                 # in pane border titles
sidebar = true

[pane_labels]
agent = true                # agent and state in the pane's border title

[update]
check = true                # tell me when a new modisa is out (modisa update installs it)
channel = "stable"          # stable, or staging for prerelease builds

[messaging]
max_hops = 10               # stop reply chains after this many hops
per_minute = 5              # per sender→recipient pair

[permissions]               # allow, ask, deny — for agents acting on panes they didn't create
keys_foreign = "ask"
close_foreign = "ask"
run_foreign = "ask"

# [agents.claude-code]
# launch = "claude --model opus"

# Programs started with the session server, with $MODISA_SOCKET set. See examples/plugins.
# [[plugin]]
# run = "my-plugin --socket $MODISA_SOCKET"

# A plugin's keys (after the prefix) are the ones its plugin.json asks for unless you change them here:
# "<plugin>.<action or pane>" = "K", or "" to turn one off.
# [plugin_keys]
# "attention-log.log" = "A"

# modisa's own keys (after the prefix): "<action>" = "K", or a list of keys, or "" for none. An action set here
# loses its default keys; the keyboard guide (prefix ?) names every action. x (close pane), d (detach) and escape
# can't be given away. modisa config check finds mistakes; modisa config reset-keys puts the defaults back.
# [keys]
# zoom = "f"
# split-right = ["v", "|"]

# How --remote starts modisa on the far side of ssh. Set an absolute path when it isn't on the
# PATH of a non-interactive ssh shell (~/.local/bin often isn't).
# remote_command = "modisa"
`;

// A TOML parse error's message, and where it is: Bun puts the line and column (both from 1) in its `position`.
export function tomlError(e: unknown): { message: string; line?: number; column?: number } {
  const err = e as { message?: string; position?: { line?: number; column?: number } } | undefined;
  const at = err?.position?.line ? { line: err.position.line, column: err.position.column } : {};
  return { message: err?.message ?? String(e), ...at };
}

// config.toml merged over the defaults; when it can't be read, the defaults and why.
export async function readConfig(): Promise<{ cfg: Config; error?: string }> {
  const file = Bun.file(CONFIG_PATH);
  if (!(await file.exists())) return { cfg: structuredClone(DEFAULTS) };
  try {
    const user = Bun.TOML.parse(await file.text()) as any;
    return { cfg: {
      ...DEFAULTS,
      ...user,
      sidebar: { ...DEFAULTS.sidebar, ...user.sidebar },
      status: { ...DEFAULTS.status, ...user.status },
      // [sidebar] git was where turning git off lived before it moved to the status row
      git: { ...DEFAULTS.git, ...(user.sidebar?.git === false && { status: false }), ...user.git },
      panes: { ...DEFAULTS.panes, ...user.panes },
      mouse: { ...DEFAULTS.mouse, ...user.mouse },
      notify: { ...DEFAULTS.notify, ...user.notify },
      sound: { ...DEFAULTS.sound, ...user.sound },
      indicators: { ...DEFAULTS.indicators, ...user.indicators },
      pane_labels: { ...DEFAULTS.pane_labels, ...user.pane_labels },
      update: { ...DEFAULTS.update, ...user.update },
      messaging: { ...DEFAULTS.messaging, ...user.messaging },
      permissions: { ...DEFAULTS.permissions, ...user.permissions },
      agents: { ...user.agents },
      plugin: user.plugin ?? [],
      plugin_keys: { ...user.plugin_keys },
      keys: { ...user.keys },
    } };
  } catch (e) {
    const { message, line } = tomlError(e);
    return { cfg: structuredClone(DEFAULTS), error: `${line ? `line ${line}: ` : ""}${message}` };
  }
}

export async function loadConfig(): Promise<Config> {
  const { cfg, error } = await readConfig();
  if (error) console.error(`modisa: bad config ${CONFIG_PATH}: ${error}`);
  return cfg;
}

export async function ensureConfigFile() {
  if (!(await Bun.file(CONFIG_PATH).exists())) await Bun.write(CONFIG_PATH, SAMPLE);
  return CONFIG_PATH;
}

type Value = string | number | boolean | Value[];
const literal = (v: Value): string => (Array.isArray(v) ? `[${v.map(literal).join(", ")}]` : typeof v === "string" ? JSON.stringify(v) : String(v));

// Where a value that starts on lines[at] (text = what follows "key =") ends: its last line, and the comment
// after it there (with the spaces before it). Arrays may span lines; # inside strings isn't a comment.
function valueEnd(lines: string[], at: number, text: string): { stop: number; comment: string } {
  let depth = 0, quote = "";
  for (let line = at; ; text = lines[++line]!) {
    for (let i = 0; i < text.length; i++) {
      const c = text[i]!;
      if (quote) { if (c === "\\" && quote === '"') i++; else if (c === quote) quote = ""; }
      else if (c === '"' || c === "'") quote = c;
      else if (c === "[") depth++;
      else if (c === "]") depth--;
      else if (c === "#") {
        if (depth <= 0) return { stop: line, comment: text.slice(text.slice(0, i).trimEnd().length) };
        break; // a comment inside a multiline array
      }
    }
    if (depth <= 0 || line + 1 >= lines.length) return { stop: line, comment: "" };
  }
}

const header = (l: string) => /^\s*\[/.test(l);
const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const assignment = (key: string) => {
  const k = escapeRe(key);
  return new RegExp(`^(\\s*(?:${k}|"${k}"|'${k}')\\s*=\\s*)(.*)$`);
};

// Where [table] (null: the top of the file) and its `key` are in config.toml's lines: the table's first line and the
// line past its last (start -1: no such table), and the line `key` is set on (-1: it isn't).
export function findKey(lines: string[], table: string | null, key?: string): { start: number; end: number; at: number } {
  let start = 0;
  if (table) {
    start = lines.findIndex((l) => l.replace(/#.*/, "").trim() === `[${table}]`) + 1;
    if (!start) return { start: -1, end: -1, at: -1 };
  }
  let end = lines.findIndex((l, i) => i >= start && header(l));
  if (end < 0) end = lines.length;
  const set = key === undefined ? undefined : assignment(key);
  const at = set ? lines.findIndex((l, i) => i >= start && i < end && set.test(l)) : -1;
  return { start, end, at };
}

// Set one key — at the root (table null) or inside [table] — keeping comments and every other setting.
// A missing key goes at the end of its table; a missing table is added at the end of the file.
export function withValue(source: string, table: string | null, key: string, value: Value): string {
  const lines = source.split("\n");
  const { start, end, at } = findKey(lines, table, key);
  if (start < 0) return `${source.trimEnd()}\n\n[${table}]\n${key} = ${literal(value)}\n`;
  if (at >= 0) {
    const [, lead, rest] = assignment(key).exec(lines[at]!)!;
    const { stop, comment } = valueEnd(lines, at, rest!);
    lines.splice(at, stop - at + 1, lead + literal(value) + comment);
  } else {
    let last = end - 1;
    while (last >= start && !lines[last]!.trim()) last--;
    lines.splice(last + 1, 0, `${key} = ${literal(value)}`);
  }
  const result = lines.join("\n");
  let check: Record<string, any> | undefined;
  try { check = Bun.TOML.parse(result) as Record<string, any>; } catch {}
  if (JSON.stringify((table ? check?.[table] : check)?.[key]) !== JSON.stringify(value)) {
    throw new Error(`couldn't update ${table ? table + "." : ""}${key} in config.toml safely; edit it by hand`);
  }
  return result;
}

// Take [table] out: its header and every setting in it, keeping the comments around and in it, and everything else.
export function withoutTable(source: string, table: string): string {
  const lines = source.split("\n");
  for (let t = findKey(lines, table); t.start > 0; t = findKey(lines, table)) {
    const kept: string[] = [];
    for (let i = t.start; i < t.end; i++) {
      const l = lines[i]!;
      if (!l.trim() || /^\s*#/.test(l)) kept.push(l);
      else i = valueEnd(lines, i, l.slice(l.indexOf("=") + 1)).stop; // a setting, however many lines its value takes
    }
    // no blank line left doubled where the header was
    while (kept.length && !kept[0]!.trim() && (t.start === 1 || !lines[t.start - 2]!.trim())) kept.shift();
    lines.splice(t.start - 1, t.end - t.start + 1, ...kept);
  }
  const result = lines.join("\n");
  let check: Record<string, any> | undefined;
  try { check = Bun.TOML.parse(result) as Record<string, any>; } catch {}
  if (!check || table in check) throw new Error(`couldn't take [${table}] out of config.toml safely; edit it by hand`);
  return result;
}

// modisa's own keys back (`config reset-keys`): [keys] and [plugin_keys] out and the prefix C-b again, each change
// said. Throws when the file doesn't parse: nothing in it can be edited safely then.
export function withDefaultKeys(source: string): { result: string; changes: string[] } {
  let user: Record<string, any>;
  try {
    user = Bun.TOML.parse(source) as Record<string, any>;
  } catch (e) {
    const { message, line } = tomlError(e);
    throw new Error(`config.toml doesn't parse (${line ? `line ${line}: ` : ""}${message}); fix that first`);
  }
  let result = source;
  const changes: string[] = [];
  for (const table of ["keys", "plugin_keys"]) {
    if (!(table in user)) continue;
    result = withoutTable(result, table);
    const n = Object.keys(user[table] ?? {}).length;
    changes.push(`[${table}] removed (${n} ${n === 1 ? "setting" : "settings"})`);
  }
  if (user.prefix !== undefined && user.prefix !== DEFAULTS.prefix) {
    result = withValue(result, null, "prefix", DEFAULTS.prefix);
    changes.push(`prefix ${JSON.stringify(user.prefix)} → ${JSON.stringify(DEFAULTS.prefix)}`);
  }
  return { result, changes };
}

export function withTheme(source: string, name: string): string {
  if (!THEMES[name]) throw new Error(`Unknown theme: ${name}`);
  return withValue(source, null, "theme", name);
}

// Write one setting to config.toml (creating it from SAMPLE first), preserving everything else.
export async function saveSetting(table: string | null, key: string, value: Value) {
  const path = await ensureConfigFile();
  await Bun.write(path, withValue(await Bun.file(path).text(), table, key, value));
}

export async function saveTheme(name: string) {
  if (!THEMES[name]) throw new Error(`Unknown theme: ${name}`);
  await saveSetting(null, "theme", name);
}

// Hot reload: poll mtime once a second.
export function watchConfig(onChange: () => void) {
  let last = Bun.file(CONFIG_PATH).lastModified;
  setInterval(() => {
    const now = Bun.file(CONFIG_PATH).lastModified;
    if (now !== last) {
      last = now;
      onChange();
    }
  }, 1000).unref();
}

export function parsePrefix(p: string): { ctrl: boolean; name: string } {
  const m = /^C-(.)$/i.exec(p);
  return m ? { ctrl: true, name: m[1]!.toLowerCase() } : { ctrl: true, name: "b" };
}
