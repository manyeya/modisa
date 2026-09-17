import { test, expect } from "bun:test";
import { descendantAgent, foreground, identify, nextState, screenVerdict } from "../../src/server/agents/detect";
import { compileRules, evaluate, region, toRegExp } from "../../src/server/agents/manifest";
import { loadAdapters } from "../../src/config/adapters";
import { DEFAULTS } from "../../src/config/config";

const adapters = await loadAdapters(DEFAULTS);
const agent = (id: string) => adapters.find((a) => a.id === id)!;
const state = (id: string, screen: string, title = "") => screenVerdict(agent(id), screen, title).state;

test("every bundled agent's screen rules compile", () => {
  expect(adapters.length).toBeGreaterThanOrEqual(25);
  for (const a of adapters) expect(() => compileRules(a.rules)).not.toThrow();
});

test("agents are recognised by name, alias, and through interpreters and wrappers", () => {
  const cases: [string, string | undefined][] = [
    ["claude", "claude-code"],
    ["/opt/homebrew/bin/codex", "codex"],
    ["node /usr/local/lib/node_modules/@openai/codex/bin/codex.js", "codex"],
    ["/nix/store/abc-codex/bin/.codex-wrapped --yolo", "codex"],
    ["bun /home/me/.bun/bin/omp", "omp"],
    ["python3.12 /home/me/.local/bin/hermes chat", "hermes"],
    ["node /usr/lib/node_modules/@earendil-works/pi-coding-agent/dist/cli.js", "pi"],
    ["node /usr/lib/node_modules/@qwen-code/qwen-code/dist/index.js", "qwen"],
    ["/opt/muse/muse-bin-0.1.0-R708.1", "muse"],
    ["agy", "antigravity"],
    ["kiro-cli chat", "kiro"],
    ["ghcs", "copilot"],
    ["node --require ./x.js /usr/lib/node_modules/@qwen-code/qwen-code/dist/index.js", "qwen"],
    ["node -e console.log(1)", undefined],
    ["python3 -m http.server", undefined],
    ["-zsh", undefined],
    ["muse-binary", undefined],
  ];
  for (const [args, id] of cases) expect([args, identify(args, adapters)?.id]).toEqual([args, id]);
});

test("foreground job resolves through the shell's tpgid; without one, any descendant agent counts", () => {
  const procs = new Map([
    [10, { pid: 10, ppid: 1, tpgid: 20, args: "-zsh" }],
    [20, { pid: 20, ppid: 10, tpgid: 20, args: "node /usr/local/lib/node_modules/@openai/codex/bin/codex.js" }],
  ]);
  expect(foreground(procs, 10)!.pid).toBe(20);
  const noJobControl = new Map([
    [10, { pid: 10, ppid: 1, tpgid: 0, args: "/bin/zsh -l" }],
    [11, { pid: 11, ppid: 10, tpgid: 0, args: "sleep 5" }],
    [12, { pid: 12, ppid: 10, tpgid: 0, args: "claude" }],
  ]);
  expect(foreground(noJobControl, 10)).toBeUndefined();
  expect(descendantAgent(noJobControl, 10, adapters)?.adapter.id).toBe("claude-code");
  expect(descendantAgent(noJobControl, 11, adapters)).toBeUndefined();
});

test("the rule engine: highest priority wins, not gates exclude, viewers keep the last state", () => {
  const rules = compileRules([
    { id: "low", state: "working", priority: 1, contains: ["busy"] },
    { id: "high", state: "blocked", priority: 5, contains: ["busy"], not: [{ contains: ["ignore me"] }] },
    { id: "viewer", state: "unknown", priority: 9, region: "bottom_non_empty_lines(1)", skip_state_update: true, line_regex: ["(?i)^transcript$"] },
  ]);
  expect(evaluate(rules, { screen: "busy" }).rule).toBe("high");
  expect(evaluate(rules, { screen: "busy, ignore me" }).rule).toBe("low");
  expect(evaluate(rules, { screen: "busy\nTRANSCRIPT" })).toMatchObject({ rule: "viewer", skip: true });
  const none = evaluate(rules, { screen: "nothing" });
  expect([none.state, none.rule]).toEqual(["idle", undefined]);
});

test("regions, and manifest regexes written in Rust syntax, work", () => {
  const screen = "old ─── line\nhistory\n────────\n❯ typed\n────────\n  footer\n\n";
  expect(region({ screen }, "bottom_non_empty_lines(2)")).toBe("────────\n  footer\n\n");
  expect(region({ screen }, "prompt_box_body")).toBe("❯ typed\n");
  expect(region({ screen }, "after_last_horizontal_rule")).toBe("  footer\n\n");
  expect(region({ screen }, "top_non_empty_lines(1)")).toBe("old ─── line\n");
  expect(region({ screen, title: "⠋ Claude" }, "osc_title")).toBe("⠋ Claude");
  expect(toRegExp("^[\\x{2800}-\\x{28FF}] ").test("⠙ task")).toBe(true);
  expect(toRegExp("(?i)\\Adone\\z").test("DONE")).toBe(true);
  expect(toRegExp("(?i)\\Adone\\z").test("x\ndone")).toBe(false);
});

test("Claude's terminal-title spinner means working", () => {
  expect(state("claude-code", "❯ ", "⠙ Refactoring auth")).toBe("working");
  expect(state("claude-code", "❯ ", "Claude Code")).toBe("idle");
});

test("agents that opt in count recent output as working when no rule matches", () => {
  expect(screenVerdict(agent("generic"), "", "", "", Date.now()).state).toBe("working");
  expect(screenVerdict(agent("generic"), "", "", "", 0).state).toBe("idle");
});

test("done means finished while unfocused, and clears on focus", () => {
  expect(nextState("working", "idle", false)).toBe("done");
  expect(nextState("done", "idle", false)).toBe("done");
  expect(nextState("done", "idle", true)).toBe("idle");
  expect(nextState(undefined, "idle", false)).toBe("idle");
  expect(nextState("idle", "blocked", true)).toBe("blocked");
});

// Screens captured from the real Claude Code v2.1.268 running in a modisa pane.
test("claude-code rules on real screens", async () => {
  const screen = (name: string) => Bun.file(`${import.meta.dir}/../fixtures/claude-code/${name}.txt`).text();
  const expected: Record<string, string> = {
    idle: "idle",
    done: "idle", // finished turn: the input box is back
    working: "working",
    question: "blocked", // AskUserQuestion picker
    permission: "blocked", // Bash permission prompt
    trust: "blocked", // folder trust dialog
    "idle-chat-question": "idle", // chat text asking "Do you want to…?" with the input box still there
  };
  for (const [name, want] of Object.entries(expected)) expect([name, state("claude-code", await screen(name))]).toEqual([name, want]);
});

// Codex v0.154.0 screens (working is reconstructed from its status line format).
test("codex rules on real screens", async () => {
  const screen = (name: string) => Bun.file(`${import.meta.dir}/../fixtures/codex/${name}.txt`).text();
  const expected: Record<string, string> = { idle: "idle", "confirm-dialog": "blocked", "idle-after-dialog": "idle", working: "working" };
  for (const [name, want] of Object.entries(expected)) expect([name, state("codex", await screen(name))]).toEqual([name, want]);
});
