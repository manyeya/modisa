// `modisa config check`: config.toml read the way modisa reads it, with no session needed. What modisa can't parse or
// would misread is an error; a setting it doesn't know (a typo, or one from another version) is a warning, since
// modisa ignores it.
import { z } from "zod";
import { THEMES } from "./themes";
import { SOUNDS } from "../client/sound/recipes";
import { findKey, tomlError } from "./config";
import { resolveKeys } from "./keys";

export type ConfigProblem = { level: "error" | "warning"; key?: string; message: string; line?: number; column?: number };

const bool = z.boolean();
const kinds = z.array(z.enum(["toast", "system", "sound", "bell"]));
const sound = z.enum(SOUNDS as [string, ...string[]], { error: `a sound: ${SOUNDS.join(", ")}` });
const policy = z.enum(["allow", "ask", "deny"]);
const table = <T extends z.ZodRawShape>(shape: T) => z.strictObject(shape).partial();

// What each top-level setting may be: one schema per table.
const SETTINGS: Record<string, z.ZodType> = {
  prefix: z.string().regex(/^C-.$/, 'C- and one key, like "C-b"'),
  theme: z.string().refine((t) => !!THEMES[t], { error: (i) => `there's no theme ${JSON.stringify(i.input)} (the settings page lists them)` }),
  remote_command: z.string().min(1),
  mouse: table({ hover: bool }),
  sidebar: table({ visible: bool, width: z.number().int().min(20, "20 to 48 columns").max(48, "20 to 48 columns"), agents: z.string(), logos: z.enum(["auto", "on", "off"]), graph: bool, git: bool }),
  status: table({ agents: bool, panes: bool, theme: bool }),
  git: table({ status: bool, repo: bool, counts: bool, changes: bool }),
  panes: table({ border: z.enum(["single", "rounded", "double", "heavy"]) }),
  notify: table({ blocked: kinds, done: kinds, working: kinds }),
  sound: table({ volume: z.number().min(0).max(1), blocked: sound, done: sound, working: sound }),
  indicators: table({ style: z.enum(["symbols", "dots", "letters"]), tab: bool, pane: bool, sidebar: bool }),
  pane_labels: table({ agent: bool }),
  update: table({ check: bool, channel: z.enum(["stable", "staging"]) }),
  messaging: table({ max_hops: z.number().int().positive(), per_minute: z.number().int().positive() }),
  permissions: table({ keys_foreign: policy, close_foreign: policy, run_foreign: policy }),
  agents: z.record(z.string(), z.record(z.string(), z.unknown())), // per agent, over its adapter's fields
  plugin: z.array(z.strictObject({ run: z.string().min(1) })),
  plugin_keys: z.record(z.string(), z.string()),
  keys: z.record(z.string(), z.union([z.string(), z.array(z.string())])),
};

export function checkConfig(source: string): ConfigProblem[] {
  let user: Record<string, unknown>;
  try {
    user = Bun.TOML.parse(source) as Record<string, unknown>;
  } catch (e) {
    return [{ level: "error", ...tomlError(e) }];
  }
  const lines = source.split("\n");
  // the line a setting is on, else the header of the nearest table around it, else nothing
  const lineOf = (path: PropertyKey[]) => {
    const keys = path.filter((p): p is string => typeof p === "string");
    for (let n = keys.length; n > 0; n--) {
      const at = findKey(lines, n > 1 ? keys.slice(0, n - 1).join(".") : null, keys[n - 1]).at;
      if (at >= 0) return at + 1;
      const start = findKey(lines, keys.slice(0, n).join(".")).start;
      if (start > 0) return start;
    }
  };
  const problems: ConfigProblem[] = [];
  const add = (level: ConfigProblem["level"], path: PropertyKey[], message: string) => {
    const line = lineOf(path);
    problems.push({ level, key: path.map(String).join("."), message, ...(line && { line }) });
  };
  for (const [name, value] of Object.entries(user)) {
    const schema = SETTINGS[name];
    if (!schema) {
      add("warning", [name], "modisa has no such setting, so it's ignored");
      continue;
    }
    const parsed = schema.safeParse(value);
    for (const issue of parsed.error?.issues ?? []) {
      if (issue.code === "unrecognized_keys") for (const key of issue.keys) add("warning", [name, ...issue.path, key], "modisa has no such setting, so it's ignored");
      else add("error", [name, ...issue.path], issue.message);
    }
  }
  if ((user.sidebar as any)?.git !== undefined) add("warning", ["sidebar", "git"], "this moved: [git] status = false turns the branch off");
  if (user.keys && typeof user.keys === "object") for (const p of resolveKeys(user.keys as Record<string, unknown>).problems) add(p.level, ["keys", p.action], p.message);
  return problems.sort((a, b) => (a.line ?? 0) - (b.line ?? 0));
}
