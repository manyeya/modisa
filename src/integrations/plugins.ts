// Sources for the integrations that are plugins loaded into an agent rather than hook commands.
// Each is generated at install time with the command that runs shepherd baked in, and reports
// through `shepherd report`, so none of them talks to the socket itself. They run inside the agent:
// OpenCode and Kilo run on Bun, Pi and OMP load extensions through their own `pi.exec`, Hermes is
// Python. Outside a shepherd pane they do nothing.
export const PLUGIN_VERSION = 2;

const header = (agent: string, comment: string) => `${comment} shepherd-integration: ${agent} v${PLUGIN_VERSION} — managed by shepherd; reinstalling overwrites this file`;

// OpenCode and Kilo (a fork of it): plugin events → state, and the root session id for resume.
export function opencodePlugin(agent: "opencode" | "kilo", cmd: string[]): string {
  return `${header(agent, "//")}
const SHEPHERD = ${JSON.stringify(cmd)};
const AGENT = ${JSON.stringify(agent)};
const WORKING = new Set(["tool.execute.before", "tool.execute.after", "permission.replied", "question.replied", "question.rejected", "session.compacted"]);
const BLOCKED = new Set(["permission.asked", "question.asked", "session.error"]);
const BUSY = new Set(["active", "busy", "pending", "retry", "running", "streaming", "working"]);

export const ShepherdAgentState = async () => {
  if (typeof Bun === "undefined" || !Bun.env.SHEPHERD_PANE_ID || !Bun.env.SHEPHERD_SOCKET) return {};
  let seq = Date.now() * 1000;
  let chain = Promise.resolve();
  const parents = new Map(); // child session → parent, so subagents report on the pane's root session
  const root = (id) => { while (id && parents.has(id)) id = parents.get(id); return id; };
  const report = (state, session) => {
    const args = ["report", "--source", "shepherd:" + AGENT, "--agent", AGENT, "--seq", String(++seq)];
    if (state) args.push("--state", state);
    if (session) args.push("--session-id", session);
    chain = chain.then(() => Bun.spawn([...SHEPHERD, ...args], { stdin: "ignore", stdout: "ignore", stderr: "ignore" }).exited).catch(() => {});
    return chain;
  };
  return {
    "chat.message": async ({ sessionID }) => { if (!parents.has(sessionID)) await report("working", sessionID); },
    event: async ({ event }) => {
      const type = event?.type;
      const props = event?.properties ?? {};
      if (props.info?.id && props.info.parentID) parents.set(props.info.id, props.info.parentID);
      const id = typeof props.sessionID === "string" ? props.sessionID : undefined;
      const child = id && parents.has(id);
      if (BLOCKED.has(type)) return report("blocked", root(id));
      if (WORKING.has(type)) return report("working", root(id));
      if (child) return;
      if (type === "session.idle") return report("idle", id);
      if (type === "session.status") {
        const kind = String(typeof props.status === "string" ? props.status : props.status?.type ?? "").toLowerCase();
        return report(kind === "idle" ? "idle" : BUSY.has(kind) ? "working" : undefined, id);
      }
      if (type === "session.updated" && id) return report(undefined, id);
    },
  };
};
`;
}

// Pi, and OMP (a fork with tool-approval events): extension events → state and the session file.
export function piExtension(agent: "pi" | "omp", cmd: string[]): string {
  const omp = agent === "omp";
  return `${header(agent, "//")}
const SHEPHERD = ${JSON.stringify(cmd)};
const AGENT = ${JSON.stringify(agent)};

export default function shepherdAgentState(pi: any) {
  let seq = Date.now() * 1000;
  let chain: Promise<unknown> = Promise.resolve();
  let tui = false;
  const session = (ctx: any) => ctx?.sessionManager?.getSessionFile?.() ?? ctx?.sessionManager?.getSessionId?.();
  const run = (extra: string[]) => {
    const args = ["report", "--source", "shepherd:" + AGENT, "--agent", AGENT, "--seq", String(++seq), ...extra];
    chain = chain.then(() => pi.exec(SHEPHERD[0], [...SHEPHERD.slice(1), ...args], { timeout: 3000 })).catch(() => {});
  };
  const report = (ctx: any, state?: string) => {
    if (!tui) return; // print/RPC modes have no pane to show
    const id = session(ctx);
    run([...(state ? ["--state", state] : []), ...(id ? ["--session-id", String(id)] : [])]);
  };
  pi.on("session_start", (_e: any, ctx: any) => {
    tui = ctx?.mode === undefined || ctx.mode === "tui";
    report(ctx, ctx?.isIdle?.() === false ? "working" : "idle");
  });
  pi.on("agent_start", (_e: any, ctx: any) => report(ctx, "working"));
  pi.on(${omp ? '"agent_end"' : '"agent_settled"'}, (_e: any, ctx: any) => { if (ctx?.isIdle?.() !== false) report(ctx, "idle"); });${omp ? `
  pi.on("tool_approval_requested", (_e: any, ctx: any) => report(ctx, "blocked"));
  pi.on("tool_approval_resolved", (_e: any, ctx: any) => report(ctx, "working"));` : ""}
  // another extension can say it's waiting on the user: pi.events.emit("shepherd:blocked", { active })
  pi.events?.on?.("shepherd:blocked", (data: any) => run(["--state", data?.active ? "blocked" : "working"]));
  pi.on("session_shutdown", () => { if (tui) run(["--release"]); });
}
`;
}

// Hermes: a Python plugin reporting the resumable session id (state comes from its screen).
export function hermesPlugin(cmd: string[]): { yaml: string; py: string } {
  return {
    yaml: `${header("hermes", "#")}\nname: shepherd-agent-state\nversion: "${PLUGIN_VERSION}"\ndescription: Report the Hermes session to its shepherd pane\n`,
    py: `${header("hermes", "#")}
"""Reports the resumable Hermes session id to the shepherd pane it runs in."""
import os
import subprocess

SHEPHERD = ${JSON.stringify(cmd)}
_last = None


def _report(**kwargs):
    global _last
    session_id = kwargs.get("session_id")
    if not os.environ.get("SHEPHERD_PANE_ID") or not isinstance(session_id, str) or not session_id or session_id == _last:
        return
    _last = session_id
    try:
        subprocess.run([*SHEPHERD, "report", "--source", "shepherd:hermes", "--agent", "hermes", "--session-id", session_id],
                       stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=2, check=False)
    except Exception:
        pass


def register(ctx):
    ctx.register_hook("on_session_start", _report)
    ctx.register_hook("on_session_reset", _report)
    ctx.register_hook("pre_llm_call", _report)
`,
  };
}
