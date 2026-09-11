import { test, expect } from "bun:test";
import { interpret } from "../../src/integrations/hook";
import { addFlat, addNested, oursIn, removeOurs, withBlock, withCodexHooksFeature, withHermesPlugin } from "../../src/integrations/edit";

test("hooks keep only the events and sessions that belong to the pane's agent", () => {
  const start = { hook_event_name: "SessionStart", session_id: "s1", transcript_path: "/t" };
  expect(interpret("claude-code", "session", start)).toEqual({ session: "s1" });
  expect(interpret("claude-code", "session", { ...start, agent_id: "sub" })).toBeUndefined(); // a subagent
  expect(interpret("claude-code", "session", start, { CURSOR_VERSION: "1" })).toBeUndefined(); // Cursor's Claude-compatible hooks
  expect(interpret("claude-code", "session", { ...start, hook_event_name: "Stop" })).toBeUndefined();
  expect(interpret("codex", "session", start)).toEqual({ session: "s1" });
  expect(interpret("codex", "session", { ...start, transcript_path: undefined })).toBeUndefined();
  expect(interpret("codex", "session", start, { CODEX_THREAD_ID: "other" })).toBeUndefined(); // a child thread
  expect(interpret("copilot", "session", { hookEventName: "session_start", sessionId: "c1" })).toEqual({ session: "c1" });
  expect(interpret("copilot", "session", { prompt: "hi", sessionId: "c1" })).toBeUndefined();
  expect(interpret("cursor-agent", "session", { hook_event_name: "sessionStart", conversation_id: "k1" })).toEqual({ session: "k1" });
  expect(interpret("grok", "session", {}, { GROK_SESSION_ID: "g1" })).toEqual({ session: "g1" });
  expect(interpret("antigravity", "session", { conversationId: "a1" })).toEqual({ session: "a1" });
  expect(interpret("kimi", "blocked", { session_id: "m1" })).toEqual({ state: "blocked", session: "m1" });
  expect(interpret("mastracode", "idle", {})).toEqual({ state: "idle", session: undefined });
  expect(interpret("droid", "session", {})).toBeUndefined();
  expect(interpret("droid", "bogus", { session_id: "x" })).toBeUndefined();
});

test("our hook entries go in and come out in every agent's shape, leaving the user's own", () => {
  const ours = "SHEPHERD_HOOK=2 /bin/shepherd hook x session";
  const hooks: Record<string, any[]> = {
    SessionStart: [{ hooks: [{ type: "command", command: "echo mine" }] }],
    Stop: [{ type: "command", bash: "echo mine too" }],
  };
  addNested(hooks, "SessionStart", ours, { matcher: "*" });
  addFlat(hooks, "sessionStart", ours, { timeoutSec: 10 }, "bash");
  addFlat(hooks, "Stop", ours);
  hooks.Mixed = [{ hooks: [{ type: "command", command: "echo keep" }, { type: "command", command: ours }] }];
  expect(oursIn(hooks)).toHaveLength(4);
  expect(removeOurs(hooks)).toBe(true);
  expect(hooks).toEqual({
    SessionStart: [{ hooks: [{ type: "command", command: "echo mine" }] }],
    Stop: [{ type: "command", bash: "echo mine too" }],
    Mixed: [{ hooks: [{ type: "command", command: "echo keep" }] }],
  });
  expect(removeOurs(hooks)).toBe(false);
});

test("config text edits keep the user's lines", () => {
  expect(withCodexHooksFeature('model = "o3"\n')).toBe('model = "o3"\n\n[features]\nhooks = true\n');
  expect(withCodexHooksFeature("[features]\ncodex_hooks = true\nweb = true\n[tui]\nx = 1\n")).toBe("[features]\nhooks = true\nweb = true\n[tui]\nx = 1\n");
  expect(withCodexHooksFeature("[features]\nhooks = false\n")).toBe("[features]\nhooks = true\n");
  const block = withBlock('theme = "x"\n', "# >>> s", "# <<< s", "[[hooks]]\nevent = \"Stop\"\n");
  expect(block).toBe('theme = "x"\n\n# >>> s\n[[hooks]]\nevent = "Stop"\n# <<< s\n');
  expect(withBlock(block, "# >>> s", "# <<< s")).toBe('theme = "x"\n\n');
  const yaml = "model: x\nplugins:\n  enabled:\n    - other\nui: y\n";
  const on = withHermesPlugin(yaml, "shepherd-agent-state", true);
  expect(on).toBe("model: x\nplugins:\n  enabled:\n    - shepherd-agent-state\n    - other\nui: y\n");
  expect(withHermesPlugin(on, "shepherd-agent-state", false)).toBe(yaml);
  expect(withHermesPlugin("", "p", true)).toBe("plugins:\n  enabled:\n    - p\n");
});
