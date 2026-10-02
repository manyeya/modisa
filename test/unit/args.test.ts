import { test, expect } from "bun:test";
import { parseArgs } from "../../src/cli/args";

test("a --flag=value keeps everything after the first =", () => {
  expect(parseArgs(["wait", "p1", "--match=a=b"]).flags.match).toBe("a=b");
  expect(parseArgs(["--name=", "x"])).toEqual({ _: ["x"], flags: { name: "" }, lists: {} });
});

test("switches never take the next word as their value", () => {
  for (const flag of ["screen", "new-tab", "new-workspace", "on", "off", "toggle"]) expect(parseArgs(["pane", "x", `--${flag}`, "p1"])).toEqual({ _: ["pane", "x", "p1"], flags: { [flag]: true }, lists: {} });
});

test("--tab is a switch for agent spawn but names a tab for pane move", () => {
  expect(parseArgs(["agent", "spawn", "codex", "--tab", "--name", "x"]).flags).toEqual({ tab: true, name: "x" });
  expect(parseArgs(["agent", "spawn", "--tab", "codex"])).toEqual({ _: ["agent", "spawn", "codex"], flags: { tab: true }, lists: {} });
  expect(parseArgs(["-s", "s", "pane", "move", "p3", "--tab", "t2", "--focus"])).toEqual({ _: ["pane", "move", "p3"], flags: { session: "s", tab: "t2", focus: true }, lists: {} });
  expect(parseArgs(["pane", "move", "--tab", "t2", "p3"])._).toEqual(["pane", "move", "p3"]);
});

test("--env adds up, in order, with either spelling, and keeps every = after the first", () => {
  expect(parseArgs(["pane", "split", "--env", "A=1", "--env=B=x=y", "--down", "--env", "A=2", "echo"])).toEqual({ _: ["pane", "split", "echo"], flags: { down: true }, lists: { env: ["A=1", "B=x=y", "A=2"] } });
  // with no value it's still there, empty, for the command to refuse
  expect(parseArgs(["pane", "split", "--env", "--down"]).lists.env).toEqual([""]);
  expect(parseArgs(["pane", "split", "--env"]).lists.env).toEqual([""]);
});
