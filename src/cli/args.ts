// Command-line argument parsing: positionals, --flags (valued or boolean), and -s <session>.
export type Args = { _: string[]; flags: Record<string, string | boolean> };

const BOOLEAN = new Set(["json", "follow", "exited", "right", "down", "tab", "focus", "output", "help", "idle", "release", "version"]);

export function parseArgs(argv: string[]): Args {
  const a: Args = { _: [], flags: {} };
  for (let i = 0; i < argv.length; i++) {
    const x = argv[i]!;
    if (x === "--") (a._.push(...argv.slice(i + 1)), (i = argv.length));
    else if (x.startsWith("--")) {
      const [k, v] = x.slice(2).split("=", 2) as [string, string | undefined];
      if (v !== undefined) a.flags[k] = v;
      else if (BOOLEAN.has(k) || i + 1 >= argv.length || argv[i + 1]!.startsWith("--")) a.flags[k] = true;
      else a.flags[k] = argv[++i]!;
    } else if (x === "-s") a.flags.session = argv[++i]!;
    else a._.push(x);
  }
  return a;
}

// Flag value helpers.
export const str = (v: string | boolean | undefined) => (typeof v === "string" ? v : undefined);
export const num = (v: string | boolean | undefined) => (typeof v === "string" ? Number(v) : undefined);
