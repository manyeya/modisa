import type { KeyEvent } from "@opentui/core";

// Prefix bindings → action names (actions live in tui.ts so the palette can list them too).
export const bindings: Record<string, string> = {
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

export function keyName(key: KeyEvent): string {
  const name = key.name === "minus" ? "-" : key.name;
  if (name.length === 1 && /[a-z]/.test(name)) return key.shift ? name.toUpperCase() : name;
  // symbols that need shift (%, ", $, :) arrive as their text
  return key.sequence.length === 1 && !/[a-z0-9]/i.test(key.sequence) ? key.sequence : name;
}
