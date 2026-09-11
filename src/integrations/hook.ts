// `shepherd hook <agent> <action>`: what an agent's hook runs. It reads the hook's JSON on stdin,
// keeps only the events and fields that mean something for that agent, and reports to the pane's
// server. It never prints (some agents feed hook output back into the conversation) and always exits
// 0, so a hook can't break the agent — outside shepherd it does nothing.
import { connectUnix } from "../protocol/transport";

export type HookReport = { state?: "working" | "blocked" | "idle"; session?: string };
type Input = Record<string, unknown>;

const text = (o: Input, ...keys: string[]) => keys.map((k) => o[k]).find((v): v is string => typeof v === "string" && v.length > 0);
const squash = (s: string) => s.replace(/[_-]/g, "").toLowerCase();

// Pure: the report a hook call amounts to, if any.
export function interpret(agent: string, action: string, input: Input, env: Record<string, string | undefined> = {}): HookReport | undefined {
  const event = text(input, "hook_event_name", "hookEventName");
  let session: string | undefined;
  switch (agent) {
    case "claude-code":
      // subagents fire their own hooks, and Cursor runs Claude-compatible hooks: neither is this pane's session
      if ((event && event !== "SessionStart") || input.agent_id || env.CURSOR_VERSION || input.cursor_version) return;
      session = text(input, "session_id");
      break;
    case "codex":
      if ((event && event !== "SessionStart") || !text(input, "transcript_path")) return;
      session = text(input, "session_id");
      if (env.CODEX_THREAD_ID && env.CODEX_THREAD_ID !== session) return; // a child thread
      break;
    case "copilot":
      if (event ? squash(event) !== "sessionstart" : "prompt" in input || text(input, "tool_name", "toolName", "notification_type", "notificationType", "stop_reason", "stopReason", "reason")) return;
      session = text(input, "session_id", "sessionId");
      break;
    case "cursor-agent":
      if (event && event !== "sessionStart") return;
      session = text(input, "session_id", "sessionId", "conversation_id", "conversationId");
      break;
    case "grok":
      if (event && squash(event) !== "sessionstart") return;
      session = env.GROK_SESSION_ID || text(input, "session_id", "sessionId");
      break;
    case "antigravity":
      session = text(input, "conversationId");
      break;
    default: // devin, droid, qodercli, qwen, kimi, mastracode
      session = text(input, "session_id", "sessionId");
  }
  if (action === "session") return session ? { session } : undefined;
  if (action === "working" || action === "blocked" || action === "idle") return { state: action, session };
}

// Hooks pipe their JSON in; run by hand in a terminal, the timeout stops it waiting forever.
async function stdin(): Promise<Input> {
  const raw = await Promise.race([Bun.stdin.text(), Bun.sleep(2000).then(() => "")]);
  try {
    const value = JSON.parse(raw);
    return value && typeof value === "object" ? value : {};
  } catch {
    return {};
  }
}

let seq = 0;
export async function runHook(agent: string | undefined, action: string | undefined): Promise<number> {
  const pane = Bun.env.SHEPHERD_PANE_ID, socket = Bun.env.SHEPHERD_SOCKET;
  if (!agent || !action || !pane || !socket) return 0;
  const report = interpret(agent, action, await stdin(), Bun.env);
  if (!report) return 0;
  try {
    const conn = await connectUnix(socket);
    const params = { pane, source: `shepherd:${agent}`, agent, state: report.state, session: report.session, seq: Date.now() * 1000 + ++seq };
    await Promise.race([conn.request("report", params), Bun.sleep(1500)]);
    conn.close();
  } catch {}
  return 0;
}
