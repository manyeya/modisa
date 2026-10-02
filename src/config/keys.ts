// modisa's prefix bindings (key → action), shared by the client, which runs them, the server, which refuses plugin keys
// that would shadow them, and `config check`. [keys] in config.toml changes them.

// Every action the client can run by name: a key, the palette, a menu or a button. The client's table of actions is
// typed by these, so the two can't drift apart.
export const ACTION_IDS = [
  "theme-picker", "help", "pane-menu", "pane-picker", "working-agents", "blocked-agents",
  "split-right", "split-down", "focus-left", "focus-right", "focus-up", "focus-down",
  "resize-left", "resize-right", "resize-up", "resize-down", "zoom", "close-pane", "close-tab",
  "new-tab", "next-tab", "prev-tab", "workspace-picker", "new-workspace", "new-agent", "toggle-sidebar",
  "copy-mode", "search", "palette", "settings", "edit-config", "reload-config", "update-modisa", "restart-server",
  "toggle-messaging", "message-log", "send-message", "rename-tab", "rename-pane", "rename-workspace", "delete-workspace",
  "detach", "agent-1", "agent-2", "agent-3", "agent-4", "agent-5", "agent-6", "agent-7", "agent-8", "agent-9",
] as const;
export type ActionId = (typeof ACTION_IDS)[number];
export const isAction = (id: string): id is ActionId => (ACTION_IDS as readonly string[]).includes(id);

export type Bindings = Record<string, ActionId>; // key after the prefix → action

export const DEFAULT_KEYS: Bindings = {
  v: "split-right", "%": "split-right", "-": "split-down", '"': "split-down",
  h: "focus-left", j: "focus-down", k: "focus-up", l: "focus-right",
  left: "focus-left", down: "focus-down", up: "focus-up", right: "focus-right",
  H: "resize-left", J: "resize-down", K: "resize-up", L: "resize-right",
  z: "zoom", x: "close-pane", X: "close-tab",
  c: "new-tab", n: "next-tab", p: "prev-tab",
  w: "workspace-picker", W: "new-workspace",
  a: "new-agent", b: "toggle-sidebar",
  o: "pane-picker", e: "pane-menu", "?": "help",
  t: "theme-picker",
  "[": "copy-mode", "/": "search", ":": "palette",
  s: "settings", R: "reload-config",
  m: "toggle-messaging", i: "message-log", M: "send-message",
  ",": "rename-tab", ".": "rename-pane", $: "rename-workspace", "&": "delete-workspace",
  d: "detach",
  ...Object.fromEntries([1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => [String(n), `agent-${n}`])),
};

// Keys no plugin can have, and [keys] can't give away: x closes any pane or popup, d detaches, and escape (with the
// prefix twice, which types it through) are the ways out of anything a plugin opens. x and d stay on their actions
// whatever else [keys] gives those.
export const RESERVED_KEYS = ["x", "d", "escape"];
const KEPT: Record<string, ActionId> = { x: "close-pane", d: "detach" };

// What a key is called after the prefix: one character as typed (H is shift+h), or a key with a name.
const NAMED = ["left", "right", "up", "down", "home", "end", "pageup", "pagedown", ...Array.from({ length: 12 }, (_, i) => `f${i + 1}`)];
const isKey = (key: string) => /^\S$/u.test(key) || NAMED.includes(key);

export type KeyProblem = { level: "error" | "warning"; action: string; message: string };

// The bindings [keys] makes ("<action>" = "K", or a list of keys; "" or [] for none), and what in it can't be bound. An
// action set there has only the keys given, each taken from whatever had it by default.
export function resolveKeys(keys: Record<string, unknown> = {}): { table: Bindings; problems: KeyProblem[] } {
  const table: Bindings = { ...DEFAULT_KEYS };
  const problems: KeyProblem[] = [];
  const given = new Map<string, ActionId>(); // key → the action [keys] gave it
  for (const [action, value] of Object.entries(keys)) {
    if (!isAction(action)) {
      problems.push({ level: "warning", action, message: `there's no action ${action} (the keyboard guide, prefix ?, lists them)` });
      continue;
    }
    const wanted = [value].flat();
    if (wanted.some((k) => typeof k !== "string")) {
      problems.push({ level: "error", action, message: 'a key is a string ("K"), or a list of them' });
      continue;
    }
    for (const [key, a] of Object.entries(table)) if (a === action && !KEPT[key]) delete table[key];
    for (const key of (wanted as string[]).filter(Boolean)) {
      const error = (message: string) => problems.push({ level: "error", action, message });
      if (RESERVED_KEYS.includes(key) && KEPT[key] !== action) {
        error(`${key} is reserved: it's how you get out of anything a plugin opens`);
        continue;
      }
      if (!isKey(key)) {
        error(`${JSON.stringify(key)} isn't a key: one character (H is shift+h), or ${NAMED.slice(0, 8).join(", ")} or f1 to f12`);
        continue;
      }
      const other = given.get(key);
      if (other && other !== action) error(`${key} is given to ${other} too; the last one wins`);
      given.set(key, action);
      table[key] = action;
    }
  }
  return { table, problems };
}

export const bindings = (cfg: { keys?: Record<string, unknown> }) => resolveKeys(cfg.keys).table;

// Why a plugin can't have `key` under these bindings, if it can't.
export const modisaKey = (key: string, table: Bindings = DEFAULT_KEYS) => (RESERVED_KEYS.includes(key) ? "reserved for getting out of plugin panes" : table[key] ? `modisa's ${table[key]}` : undefined);

// A plugin key as plugin.json declares it, and as one config binds it.
export type DeclaredKey = { plugin: string; key: string; action?: string; pane?: string; description: string };
export type BoundKey = DeclaredKey & { state: "active" | "disabled"; reason?: string };

// Bind plugins' declared keys under one config's [plugin_keys] ("<plugin>.<action or pane>" → key; "" turns it off),
// then turn off a key that config's bindings use or modisa reserves, and one that two plugins (or two of one plugin's)
// want. Every client binds with its own config, so clients attached to one session can differ; the server binds with
// its own, only for what `modisa plugin list` and `plugin check` report.
export function bindPluginKeys(declared: DeclaredKey[], remaps: Record<string, string> = {}, table: Bindings = DEFAULT_KEYS): BoundKey[] {
  const wanted = declared.map((k) => ({ ...k, key: remaps[`${k.plugin}.${k.action ?? k.pane}`] ?? k.key }));
  const byKey = new Map<string, string[]>();
  for (const w of wanted) if (w.key) byKey.set(w.key, [...(byKey.get(w.key) ?? []), w.plugin]);
  return wanted.map((w) => {
    const others = (byKey.get(w.key) ?? []).filter((p) => p !== w.plugin);
    const shared = others.length > 0 || (byKey.get(w.key)?.length ?? 0) > 1;
    const reason = !w.key ? "turned off in [plugin_keys]" : modisaKey(w.key, table) ?? (shared ? `also wanted by ${others.length ? others.join(", ") : `another key of ${w.plugin}`}` : undefined);
    return { ...w, state: reason ? "disabled" : "active", ...(reason && { reason }) };
  });
}
