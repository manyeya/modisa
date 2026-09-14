// Where shepherd keeps things, and how it re-runs itself.
export const HOME = Bun.env.HOME ?? "/tmp";
export const DIR = Bun.env.SHEPHERD_DIR ?? `${HOME}/.local/state/shepherd`;
export const SRC = `${import.meta.dir}/..`;
export const MAIN = `${SRC}/main.ts`;
export const socketPath = (session: string) => `${DIR}/${session}.sock`;
export const cwd = () => Bun.env.PWD ?? HOME;

// The command that re-runs this program (server spawn, hooks, plugins).
// Inside a compiled binary Bun.argv[0] is just "bun", so ask the OS where the executable lives.
let selfCmd: string[] | undefined;
export function self(): string[] {
  if (selfCmd) return selfCmd;
  if (!import.meta.path.startsWith("/$bunfs/")) return (selfCmd = [Bun.argv[0]!, MAIN]);
  const text = (b: Uint8Array) => new TextDecoder().decode(b).trim();
  const pid = text(Bun.spawnSync(["sh", "-c", "echo $PPID"]).stdout);
  const linux = Bun.spawnSync(["readlink", `/proc/${pid}/exe`]);
  const exe = linux.success
    ? text(linux.stdout)
    : text(Bun.spawnSync([Bun.which("lsof") ?? "/usr/sbin/lsof", "-a", "-p", pid, "-d", "txt", "-Fn"]).stdout).split("\n").find((l) => l.startsWith("n"))!.slice(1); // lsof lives in /usr/sbin, often off PATH
  return (selfCmd = [exe]);
}

// What agents' hooks and plugins run. A release binary prefers the `shepherd` on PATH: Homebrew and mise
// install into a directory per version, and a hook pointing there breaks at the next upgrade, while
// the PATH entry (Homebrew's bin symlink, mise's shim, /usr/bin, ~/.local/bin) stays put.
// ponytail: trusts that whatever is called shepherd on PATH is this program.
export function stableSelf(): string[] {
  if (!import.meta.path.startsWith("/$bunfs/")) return self();
  const onPath = Bun.which("shepherd");
  return onPath ? [onPath] : self();
}

// Identifies the code a process runs, so a client can tell its server is out of date.
let version: Promise<string> | undefined;
export function codeVersion(): Promise<string> {
  return (version ??= (async () => {
    if (import.meta.path.startsWith("/$bunfs/")) {
      const exe = Bun.file(self()[0]!);
      return `bin-${exe.size}-${exe.lastModified}`;
    }
    const files = (await Array.fromAsync(new Bun.Glob("**/*.{ts,toml,md}").scan({ cwd: SRC }))).filter((f) => !f.endsWith(".test.ts")).sort();
    let text = "";
    for (const f of files) text += f + (await Bun.file(`${SRC}/${f}`).text());
    return `src-${Bun.hash(text).toString(36)}`;
  })());
}
