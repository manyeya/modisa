import type { KeyEvent } from "@opentui/core";

// Prefix bindings → action names (the table lives in config/keys.ts, where the server checks plugin keys against it).
export { BUILTIN_BINDINGS as bindings } from "../../config/keys";

export function keyName(key: KeyEvent): string {
  const name = key.name === "minus" ? "-" : key.name;
  if (name.length === 1 && /[a-z]/.test(name)) return key.shift ? name.toUpperCase() : name;
  // symbols that need shift (%, ", $, :) arrive as their text
  return key.sequence.length === 1 && !/[a-z0-9]/i.test(key.sequence) ? key.sequence : name;
}
