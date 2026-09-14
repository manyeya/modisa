// ~/.config/shepherd/config.toml, merged over defaults; changes apply live.
import { HOME } from "../core/paths";
import type { NotifyEvent } from "../protocol/types";
import { THEMES } from "./themes";

export const CONFIG_DIR = Bun.env.SHEPHERD_CONFIG_DIR ?? `${HOME}/.config/shepherd`;
export const CONFIG_PATH = `${CONFIG_DIR}/config.toml`;

export type NotifyKind = "toast" | "system" | "sound" | "bell";
export type Policy = "allow" | "ask" | "deny";
export type IndicatorStyle = "symbols" | "dots" | "letters";

export type Config = {
  prefix: string;
  theme: string;
  sidebar: { visible: boolean; width: number };
  notify: Record<NotifyEvent, NotifyKind[]>;
  sound: { volume: number } & Record<NotifyEvent, string>; // a cuelume sound name per event
  indicators: { style: IndicatorStyle; tab: boolean; pane: boolean; sidebar: boolean };
  pane_labels: { agent: boolean; status: boolean };
  update: { check: boolean; channel: "stable" | "staging" };
  messaging: { max_hops: number; per_minute: number };
  permissions: { keys_foreign: Policy; close_foreign: Policy; run_foreign: Policy };
  agents: Record<string, { launch?: string; resume?: string }>;
  plugin: { run: string }[];
  remote_command: string;
};

export const DEFAULTS: Config = {
  prefix: "C-b",
  theme: "ion",
  sidebar: { visible: true, width: 26 },
  notify: { blocked: ["toast", "system", "sound"], done: ["toast"], working: [] },
  sound: { volume: 0.7, blocked: "chime", done: "success", working: "loading" },
  indicators: { style: "symbols", tab: true, pane: true, sidebar: true },
  pane_labels: { agent: true, status: true },
  update: { check: true, channel: "stable" },
  messaging: { max_hops: 10, per_minute: 5 },
  permissions: { keys_foreign: "ask", close_foreign: "ask", run_foreign: "ask" },
  agents: {},
  plugin: [],
  remote_command: "shepherd",
};

export const SAMPLE = `# shepherd config — changes apply live (Ctrl+B s opens the settings page)
prefix = "C-b"              # C-<key>
theme = "ion"               # ion, tokyonight, catppuccin-mocha, gruvbox, nord, dracula, bearded-* (see settings)

[sidebar]
visible = true
width = 26

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
status = true               # the bottom line: pane id, status, active

[update]
check = true                # tell me when a new shepherd is out (shepherd update installs it)
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

# Programs started with the session server, with $SHEPHERD_SOCKET set. See examples/plugins.
# [[plugin]]
# run = "my-plugin --socket $SHEPHERD_SOCKET"

# How --remote starts shepherd on the far side of ssh. Set an absolute path when it isn't on the
# PATH of a non-interactive ssh shell (~/.local/bin often isn't).
# remote_command = "shepherd"
`;

export async function loadConfig(): Promise<Config> {
  const file = Bun.file(CONFIG_PATH);
  if (!(await file.exists())) return structuredClone(DEFAULTS);
  try {
    const user = Bun.TOML.parse(await file.text()) as any;
    return {
      ...DEFAULTS,
      ...user,
      sidebar: { ...DEFAULTS.sidebar, ...user.sidebar },
      notify: { ...DEFAULTS.notify, ...user.notify },
      sound: { ...DEFAULTS.sound, ...user.sound },
      indicators: { ...DEFAULTS.indicators, ...user.indicators },
      pane_labels: { ...DEFAULTS.pane_labels, ...user.pane_labels },
      update: { ...DEFAULTS.update, ...user.update },
      messaging: { ...DEFAULTS.messaging, ...user.messaging },
      permissions: { ...DEFAULTS.permissions, ...user.permissions },
      agents: { ...user.agents },
      plugin: user.plugin ?? [],
    };
  } catch (e) {
    console.error(`shepherd: bad config ${CONFIG_PATH}: ${e}`);
    return structuredClone(DEFAULTS);
  }
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

// Set one key — at the root (table null) or inside [table] — keeping comments and every other setting.
// A missing key goes at the end of its table; a missing table is added at the end of the file.
export function withValue(source: string, table: string | null, key: string, value: Value): string {
  const lines = source.split("\n");
  const header = (l: string) => /^\s*\[/.test(l);
  let start = 0;
  if (table) {
    start = lines.findIndex((l) => l.replace(/#.*/, "").trim() === `[${table}]`) + 1;
    if (!start) return `${source.trimEnd()}\n\n[${table}]\n${key} = ${literal(value)}\n`;
  }
  let end = lines.findIndex((l, i) => i >= start && header(l));
  if (end < 0) end = lines.length;
  const assignment = new RegExp(`^(\\s*(?:${key}|"${key}"|'${key}')\\s*=\\s*)(.*)$`);
  let at = -1;
  for (let i = start; i < end && at < 0; i++) if (assignment.test(lines[i]!)) at = i;
  if (at >= 0) {
    const [, lead, rest] = assignment.exec(lines[at]!)!;
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
