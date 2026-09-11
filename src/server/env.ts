// The environment every pane and plugin inherits from the server.

// Markers of whatever agent session launched this server (e.g. restarting from inside Claude Code).
// Inherited by panes they'd make every agent think it's a child session (Claude then stops saving
// transcripts). User settings like CLAUDE_CODE_USE_BEDROCK are left alone.
const LAUNCHER_MARKERS = [
  "CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "CLAUDE_CODE_SSE_PORT", "CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_CODE_MESSAGING_TOKEN",
  "CLAUDE_CODE_BRIDGE_SESSION_ID", "CLAUDE_CODE_EXECPATH", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_CHILD_SESSION", "CLAUDE_PID", "CLAUDE_EFFORT",
  "AI_AGENT", "SHEPHERD_PANE_ID",
];

export function preparePaneEnv(session: string, sock: string) {
  for (const k of LAUNCHER_MARKERS) delete Bun.env[k];
  Bun.env.SHEPHERD_SOCKET = sock; // inherited by every pane and plugin
  Bun.env.SHEPHERD_SESSION = session;
}
