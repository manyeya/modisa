// modisa's own prefix bindings (key → action name), shared by the client, which runs them, and the server, which
// refuses plugin keys that would shadow them.
export const BUILTIN_BINDINGS: Record<string, string> = {
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

// Keys no plugin can have, whatever the bindings say: x closes any pane or popup, d detaches, and escape (with the prefix
// twice, which types it through) are the ways out of anything a plugin opens.
export const RESERVED_KEYS = ["x", "d", "escape"];

// Why a plugin can't have `key`, if it can't.
export const modisaKey = (key: string) => (RESERVED_KEYS.includes(key) ? "reserved for getting out of plugin panes" : BUILTIN_BINDINGS[key] ? `modisa's ${BUILTIN_BINDINGS[key]}` : undefined);

// A plugin key as plugin.json declares it, and as one config binds it.
export type DeclaredKey = { plugin: string; key: string; action?: string; pane?: string; description: string };
export type BoundKey = DeclaredKey & { state: "active" | "disabled"; reason?: string };

// Bind plugins' declared keys under one config's [plugin_keys] ("<plugin>.<action or pane>" → key; "" turns it off),
// then turn off a key modisa uses or reserves, and one that two plugins (or two of one plugin's) want. Every client
// binds with its own config, so clients attached to one session can differ; the server binds with its own, only for
// what `modisa plugin list` and `plugin check` report.
export function bindPluginKeys(declared: DeclaredKey[], remaps: Record<string, string> = {}): BoundKey[] {
  const wanted = declared.map((k) => ({ ...k, key: remaps[`${k.plugin}.${k.action ?? k.pane}`] ?? k.key }));
  const byKey = new Map<string, string[]>();
  for (const w of wanted) if (w.key) byKey.set(w.key, [...(byKey.get(w.key) ?? []), w.plugin]);
  return wanted.map((w) => {
    const others = (byKey.get(w.key) ?? []).filter((p) => p !== w.plugin);
    const shared = others.length > 0 || (byKey.get(w.key)?.length ?? 0) > 1;
    const reason = !w.key ? "turned off in [plugin_keys]" : modisaKey(w.key) ?? (shared ? `also wanted by ${others.length ? others.join(", ") : `another key of ${w.plugin}`}` : undefined);
    return { ...w, state: reason ? "disabled" : "active", ...(reason && { reason }) };
  });
}
