// Command-line argument parsing: positionals, --flags (valued, boolean or repeated), and -s <session>.
export type Args = { _: string[]; flags: Record<string, string | boolean>; lists: Record<string, string[]> };

const BOOLEAN = new Set(["json", "follow", "exited", "right", "down", "tab", "focus", "output", "help", "idle", "release", "version", "purge", "yes", "screen", "new-tab", "new-workspace", "on", "off", "toggle", "system", "sound"]);
// Flags given once per value, which all count: --env A=1 --env B=2. They're in `lists`, not `flags`.
const REPEATABLE = new Set(["env"]);
// Flags that take a value in one command though they're switches elsewhere: `agent spawn --tab` opens a tab, while
// `pane move --tab t` names one.
const VALUED: Record<string, string[]> = { "pane move": ["tab"] };

export function parseArgs(argv: string[]): Args {
  const a: Args = { _: [], flags: {}, lists: {} };
  for (let i = 0; i < argv.length; i++) {
    const x = argv[i]!;
    if (x === "--") (a._.push(...argv.slice(i + 1)), (i = argv.length));
    else if (x.startsWith("--")) {
      const eq = x.indexOf("="); // the first one: --match=a=b matches a=b
      const k = eq < 0 ? x.slice(2) : x.slice(2, eq);
      const boolean = BOOLEAN.has(k) && !VALUED[a._.slice(0, 2).join(" ")]?.includes(k);
      const bare = i + 1 >= argv.length || argv[i + 1]!.startsWith("--"); // no value follows
      if (REPEATABLE.has(k)) (a.lists[k] ??= []).push(eq >= 0 ? x.slice(eq + 1) : bare ? "" : argv[++i]!);
      else if (eq >= 0) a.flags[k] = x.slice(eq + 1);
      else if (boolean || bare) a.flags[k] = true;
      else a.flags[k] = argv[++i]!;
    } else if (x === "-s") a.flags.session = argv[++i]!;
    else a._.push(x);
  }
  return a;
}

// Flag value helpers.
export const str = (v: string | boolean | undefined) => (typeof v === "string" ? v : undefined);
export const num = (v: string | boolean | undefined) => (typeof v === "string" ? Number(v) : undefined);
