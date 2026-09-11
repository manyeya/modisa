// Editing agents' own config files: add shepherd's hook entries in each agent's native shape, and
// remove only ours again. Ours are recognised by the SHEPHERD_HOOK=<version> marker in the command.
export const MARK = "SHEPHERD_HOOK=";
const COMMAND_KEYS = ["command", "bash", "powershell"];
const isOurs = (value: unknown) => typeof value === "string" && value.includes(MARK);
const entryOurs = (entry: any) => COMMAND_KEYS.some((k) => isOurs(entry?.[k]));

export async function readJson(path: string): Promise<any> {
  const f = Bun.file(path);
  if (!(await f.exists())) return {};
  const text = await f.text();
  if (!text.trim()) return {};
  try {
    const value = JSON.parse(text);
    if (value && typeof value === "object" && !Array.isArray(value)) return value;
  } catch {}
  throw new Error(`${path} isn't a JSON object; fix it by hand, then retry`);
}

export const writeJson = (path: string, value: unknown) => Bun.write(path, JSON.stringify(value, null, 2) + "\n");

// The object mapping event names to hook arrays, created if missing.
export function hooksOf(root: any, key = "hooks"): Record<string, any[]> {
  if (root[key] === undefined) root[key] = {};
  if (typeof root[key] !== "object" || Array.isArray(root[key])) throw new Error(`"${key}" must be an object`);
  return root[key];
}

const list = (hooks: Record<string, any[]>, event: string) => {
  if (!Array.isArray(hooks[event] ?? [])) throw new Error(`hooks for ${event} must be a list`);
  return (hooks[event] ??= []);
};

// Claude-style group: { matcher?, hooks: [{ type: "command", command, timeout }] }
export function addNested(hooks: Record<string, any[]>, event: string, command: string, o: { timeout?: number; matcher?: string } = {}) {
  list(hooks, event).push({ ...(o.matcher ? { matcher: o.matcher } : {}), hooks: [{ type: "command", command, timeout: o.timeout ?? 10 }] });
}

// A single handler: { type: "command", <key>: command, ...extra }
export function addFlat(hooks: Record<string, any[]>, event: string, command: string, extra: Record<string, unknown> = {}, key = "command") {
  list(hooks, event).push({ type: "command", [key]: command, ...extra });
}

// Remove every entry of ours, in any shape, from every event; empty events disappear.
export function removeOurs(hooks: Record<string, any[]>): boolean {
  let changed = false;
  for (const [event, entries] of Object.entries(hooks)) {
    if (!Array.isArray(entries)) continue;
    const kept = entries.flatMap((entry) => {
      if (entryOurs(entry)) return (changed = true), [];
      if (Array.isArray(entry?.hooks) && entry.hooks.some(entryOurs)) {
        changed = true;
        const inner = entry.hooks.filter((h: any) => !entryOurs(h));
        return inner.length ? [{ ...entry, hooks: inner }] : [];
      }
      return [entry];
    });
    if (kept.length) hooks[event] = kept;
    else delete hooks[event];
  }
  return changed;
}

// Every command of ours, as "event: command", so status can compare with what install would write.
export function oursIn(hooks: Record<string, any[]> | undefined): string[] {
  const found: string[] = [];
  for (const [event, entries] of Object.entries(hooks ?? {})) {
    if (!Array.isArray(entries)) continue;
    for (const entry of entries) {
      for (const h of [entry, ...(Array.isArray(entry?.hooks) ? entry.hooks : [])]) {
        for (const k of COMMAND_KEYS) if (isOurs(h?.[k])) found.push(`${event}: ${h[k]}`);
      }
    }
  }
  return found.sort();
}

// ---------- TOML and YAML, by line, so the user's formatting and comments stay ----------

// Codex: [features] hooks = true (and drop the retired codex_hooks key).
export function withCodexHooksFeature(text: string): string {
  const lines = text.split("\n");
  const header = lines.findIndex((l) => l.trim() === "[features]");
  if (header < 0) return `${text.trimEnd()}${text.trim() ? "\n\n" : ""}[features]\nhooks = true\n`;
  let end = lines.findIndex((l, i) => i > header && /^\s*\[/.test(l));
  if (end < 0) end = lines.length;
  const section = lines.slice(header + 1, end).filter((l) => !/^\s*codex_hooks\s*=/.test(l));
  const at = section.findIndex((l) => /^\s*hooks\s*=/.test(l));
  if (at >= 0) section[at] = "hooks = true";
  else section.unshift("hooks = true");
  return [...lines.slice(0, header + 1), ...section, ...lines.slice(end)].join("\n");
}

// A block of lines between markers that we own entirely (Kimi's config.toml).
export function withBlock(text: string, begin: string, end: string, body?: string): string {
  const start = text.indexOf(begin);
  const stop = start < 0 ? -1 : text.indexOf(end, start);
  const rest = start < 0 || stop < 0 ? text : text.slice(0, start) + text.slice(stop + end.length).replace(/^\n/, "");
  if (body === undefined) return rest.replace(/\n{3,}/g, "\n\n");
  return `${rest.trimEnd()}${rest.trim() ? "\n\n" : ""}${begin}\n${body}${end}\n`;
}

// Hermes config.yaml: add or remove a name under plugins: enabled:.
export function withHermesPlugin(text: string, name: string, enabled: boolean): string {
  const lines = text.split("\n");
  const plugins = lines.findIndex((l) => /^plugins:\s*$/.test(l));
  if (plugins < 0) return enabled ? `${text.trimEnd()}${text.trim() ? "\n" : ""}plugins:\n  enabled:\n    - ${name}\n` : text;
  let end = lines.findIndex((l, i) => i > plugins && /^\S/.test(l));
  if (end < 0) end = lines.length;
  const en = lines.findIndex((l, i) => i > plugins && i < end && /^ {2}enabled:/.test(l));
  const item = `    - ${name}`;
  if (en < 0) {
    if (enabled) lines.splice(plugins + 1, 0, "  enabled:", item);
    return lines.join("\n");
  }
  if (/^ {2}enabled:\s*\[\s*\]\s*$/.test(lines[en]!)) lines[en] = "  enabled:";
  const items = lines.findIndex((l, i) => i > en && i < end && l.trim() === `- ${name}`);
  if (enabled && items < 0) lines.splice(en + 1, 0, item);
  if (!enabled && items >= 0) lines.splice(items, 1);
  return lines.join("\n");
}
