import type { KeyEvent } from "@opentui/core";

// A key as prefix bindings name it (the bindings are app.bindings, from config/keys.ts and this client's [keys]).
export function keyName(key: KeyEvent): string {
  const name = key.name === "minus" ? "-" : key.name;
  if (name.length === 1 && /[a-z]/.test(name)) return key.shift ? name.toUpperCase() : name;
  // symbols that need shift (%, ", $, :) arrive as their text
  return key.sequence.length === 1 && !/[a-z0-9]/i.test(key.sequence) ? key.sequence : name;
}
