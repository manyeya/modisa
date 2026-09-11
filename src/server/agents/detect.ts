// Agent detection: which panes run an agent (from the foreground process) and what state it's in.
// Each pane has one authority: an integration reporting lifecycle state (hooks or a plugin that see
// every transition), or else the agent's screen rules (./manifest.ts). A dialog visibly waiting on
// you still wins over a reported state.
import type { Adapter, AgentState } from "../../config/adapters";
import type { PtyPane } from "../session/pane";
import { compileRules, evaluate, type Rule, type Verdict } from "./manifest";

type Proc = { pid: number; ppid: number; tpgid: number; args: string };
type Raw = "working" | "blocked" | "idle";

export async function processTable(): Promise<Map<number, Proc>> {
  const out = await Bun.$`ps -A -o pid=,ppid=,tpgid=,args=`.quiet().nothrow().text();
  const m = new Map<number, Proc>();
  for (const line of out.split("\n")) {
    const r = /^\s*(\d+)\s+(\d+)\s+(-?\d+)\s+(.*)$/.exec(line);
    if (r) m.set(+r[1]!, { pid: +r[1]!, ppid: +r[2]!, tpgid: +r[3]!, args: r[4]! });
  }
  return m;
}

// ---------- which agent a process is ----------

const basename = (path: string) => path.split(/[/\\]/).filter(Boolean).pop() ?? path;
// "codex.js", ".codex-wrapped" (nix) and "Claude.exe" all name the agent inside
const normalize = (token: string) => basename(token.replace(/^["']|["']$/g, "")).toLowerCase().replace(/\.(exe|cmd|bat|ps1|m?js|cjs|ts|py)$/, "").replace(/^\./, "").replace(/-wrapped$/, "");
const isRuntime = (name: string) => /^(node|bun|sh|bash|zsh|fish|python(\d+(\.\d+)*)?)$/.test(name);

// Installs whose entry script has a generic name (cli.js, index.js).
function knownPackage(path: string): string | undefined {
  const p = path.toLowerCase();
  if (/node_modules\/@earendil-works\/pi-coding-agent\/dist\/(bundle\/)?cli\.js$/.test(p)) return "pi";
  if (/node_modules\/@qwen-code\/qwen-code\/dist\/index\.js/.test(p)) return "qwen";
  if (/node_modules\/mastracode\/dist\/cli/.test(p)) return "mastracode";
}

// The script an interpreter runs: its first argument that isn't a flag (none for -e / -c / -m).
function scriptOf(argv: string[], runtime: string): string | undefined {
  const inline = runtime.startsWith("python") ? ["-c", "-m"] : /^(sh|bash|zsh|fish)$/.test(runtime) ? ["-c"] : ["-e", "--eval", "-p", "--print"];
  const takesValue = ["-r", "--require", "--loader", "--import", "--experimental-loader", "--inspect-port", "-W", "-X", "-S", "-L", "-o"];
  for (let i = 1; i < argv.length; i++) {
    const a = argv[i]!;
    if (a === "--") return argv[i + 1];
    if (inline.some((f) => a === f || a.startsWith(`${f}=`))) return;
    if (a.startsWith("-")) { if (takesValue.includes(a)) i++; continue; }
    return a;
  }
}

export function identify(args: string, adapters: Adapter[]): Adapter | undefined {
  const argv = args.split(/\s+/).filter(Boolean);
  if (!argv[0]) return;
  const byName = (name: string) => adapters.find((a) => a.process.includes(name)) ?? (/^muse-bin-\d/.test(name) ? adapters.find((a) => a.id === "muse") : undefined);
  const byPath = (path: string) => { const id = knownPackage(path); return id ? adapters.find((a) => a.id === id) : undefined; };
  const first = normalize(argv[0]);
  if (!isRuntime(first)) return byName(first) ?? byPath(argv[0]);
  const script = scriptOf(argv, first);
  return script ? byName(normalize(script)) ?? byPath(script) : undefined;
}

// The shell's foreground job: the process group it handed the terminal to.
export function foreground(procs: Map<number, Proc>, shellPid: number): Proc | undefined {
  const shell = procs.get(shellPid);
  return shell && shell.tpgid > 0 ? procs.get(shell.tpgid) : undefined;
}

// Fallback when the shell has no foreground group (no job control): any descendant running an agent.
export function descendantAgent(procs: Map<number, Proc>, shellPid: number, adapters: Adapter[]): { proc: Proc; adapter: Adapter } | undefined {
  const kids = new Map<number, Proc[]>();
  for (const p of procs.values()) kids.set(p.ppid, [...(kids.get(p.ppid) ?? []), p]);
  const queue = [...(kids.get(shellPid) ?? [])];
  for (let p; (p = queue.shift()); ) {
    const adapter = identify(p.args, adapters);
    if (adapter) return { proc: p, adapter };
    queue.push(...(kids.get(p.pid) ?? []));
  }
}

// ---------- what state it's in ----------

const compiled = new WeakMap<Adapter, Rule[]>();
function rulesOf(a: Adapter): Rule[] {
  let rules = compiled.get(a);
  if (!rules) {
    try { rules = compileRules(a.rules); }
    catch (e) { console.error(`shepherd: ${a.id} screen rules: ${(e as Error).message}`); rules = []; }
    compiled.set(a, rules);
  }
  return rules;
}

// The screen rules' verdict; agents that opt in count recent output as working when nothing matched.
export function screenVerdict(a: Adapter, screen: string, title = "", progress = "", lastOutput = 0, now = Date.now()): Verdict {
  const v = evaluate(rulesOf(a), { screen, title, progress });
  if (!v.rule && a.activity && now - lastOutput < 2000) return { ...v, state: "working", rule: "recent output" };
  return v;
}

// working → idle while you weren't looking = done; done → idle once you focus it.
export function nextState(prev: AgentState | undefined, raw: Raw, focused: boolean): AgentState {
  if (raw !== "idle") return raw;
  if (focused) return "idle";
  if (prev === "working" || prev === "blocked" || prev === "done") return "done";
  return "idle";
}

export type Detection = { harness: string; raw: Raw; rule?: string; source: "hook" | "screen"; fg?: string };
// An integration that reports lifecycle state for a pane, until it releases or its agent exits.
export type Authority = { source: string; agent?: string; state: Raw; seq?: number; pid?: number };

export class Detector {
  authority = new Map<string, Authority>();
  last = new Map<string, Detection>();
  private agentPid = new Map<string, number>();
  private pendingIdle = new Set<string>();

  constructor(private adapters: () => Adapter[]) {}

  async tick(panes: PtyPane[], focused: (id: string) => boolean): Promise<{ pane: PtyPane; from?: AgentState; to: AgentState }[]> {
    const procs = await processTable();
    const changes: { pane: PtyPane; from?: AgentState; to: AgentState }[] = [];
    const adapters = this.adapters();
    for (const p of panes) {
      if (p.disposed) continue; // closed while we waited on the process table
      if (p.info.status !== "running") {
        this.forget(p.id);
        if (p.info.agent && p.info.agent.state !== "idle") {
          const from = p.info.agent.state;
          p.info.agent.state = focused(p.id) ? "idle" : "done";
          if (from !== p.info.agent.state) changes.push({ pane: p, from, to: p.info.agent.state });
        }
        continue;
      }
      let fg = foreground(procs, p.proc.pid);
      let adapter = fg && identify(fg.args, adapters);
      if (!adapter) {
        const d = descendantAgent(procs, p.proc.pid, adapters);
        if (d) [adapter, fg] = [d.adapter, d.proc];
      }
      // an integration's authority ends when the agent it spoke for is gone
      const auth = this.authority.get(p.id);
      if (auth) {
        const shellOwnsTerminal = !fg || fg.pid === p.proc.pid;
        if (auth.pid ? !procs.has(auth.pid) : shellOwnsTerminal) this.authority.delete(p.id);
        else if (!auth.pid) auth.pid = fg!.pid; // reported before we'd seen its process
      }
      const live = this.authority.get(p.id);
      if (!adapter && live?.agent) adapter = adapters.find((a) => a.id === live.agent || a.process.includes(live.agent!)); // agents only their plugin identifies
      if (!adapter && p.info.harness) adapter = adapters.find((a) => a.id === p.info.harness) ?? adapters.find((a) => a.id === "generic");
      if (!adapter) {
        if (p.info.agent) {
          p.info.agent = undefined;
          changes.push({ pane: p, to: "idle" });
        }
        this.forget(p.id);
        continue;
      }
      if (fg) this.agentPid.set(p.id, fg.pid);
      const seen = screenVerdict(adapter, p.screen(), p.oscTitle, p.oscProgress, p.lastOutput);
      const prev = this.last.get(p.id);
      let raw: Raw, rule = seen.rule, source: "hook" | "screen" = "screen";
      if (live && !seen.visibleBlocker) {
        [raw, rule, source] = [live.state, `${live.source} report`, "hook"];
      } else if (seen.skip || seen.state === "unknown") {
        raw = prev?.raw ?? "idle"; // an agent-owned viewer (transcript, picker): keep the last state
      } else {
        raw = seen.state;
        // a spinner frame without its marker reads as idle: wait one more look unless idle is visible
        if (prev?.raw === "working" && raw === "idle" && !seen.visibleIdle && !this.pendingIdle.has(p.id)) {
          this.pendingIdle.add(p.id);
          raw = "working";
        } else this.pendingIdle.delete(p.id);
      }
      this.last.set(p.id, { harness: adapter.id, raw, rule, source, fg: fg?.args });
      const from = p.info.agent?.state;
      const to = nextState(p.info.agent?.harness === adapter.id ? from : undefined, raw, focused(p.id));
      p.info.agent = { harness: adapter.id, state: to, source };
      if (from !== to) changes.push({ pane: p, from, to });
    }
    return changes;
  }

  // A lifecycle report. Reports older than the last one from the same source (by seq) are ignored.
  report(pane: PtyPane, r: { source: string; agent?: string; state: AgentState; seq?: number }) {
    const cur = this.authority.get(pane.id);
    if (cur && cur.source === r.source && r.seq !== undefined && cur.seq !== undefined && r.seq <= cur.seq) return false;
    this.authority.set(pane.id, { source: r.source, agent: r.agent, state: r.state === "done" ? "idle" : r.state, seq: r.seq, pid: this.agentPid.get(pane.id) });
    return true;
  }

  release(pane: PtyPane, source: string) {
    if (this.authority.get(pane.id)?.source === source) this.authority.delete(pane.id);
  }

  private forget(id: string) {
    this.authority.delete(id);
    this.agentPid.delete(id);
    this.pendingIdle.delete(id);
    this.last.delete(id);
  }
}
