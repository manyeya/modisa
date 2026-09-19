import { expect, test } from "bun:test";
import { agentGraph, sidebarBudget, sidebarColumns } from "../../src/client/design";

test("sidebar lists leave the footer and bottom spacing clear at every supported height", () => {
  for (const height of [20, 22, 30, 38, 58]) {
    for (const agents of [0, 1, 3, 30]) {
      const b = sidebarBudget(height, agents);
      const rows = 3 + (agents ? b.agentRows * 2 : 2) + Number(b.moreAgents);
      expect(rows).toBeLessThanOrEqual(height - 5);
      if (agents) expect(b.agentRows).toBeGreaterThanOrEqual(1);
      expect(b.moreAgents).toBe(agents > b.agentRows);
    }
  }
});

test("agents as a git graph: tabs on one trunk, their agents branching off, the trunk ending under the last", () => {
  const tabs = [{ id: "a", agents: ["x", "y"] }, { id: "b", agents: [] }, { id: "c", agents: ["z"] }];
  const draw = (g: ReturnType<typeof agentGraph<string>>) =>
    g.rows.map((r) => (r.kind === "tab" ? `${r.node}${r.tab}${r.open ? "" : "+"}` : r.kind === "rail" ? "|" : `${r.graph.join("")}${r.agent}`));
  // room for everything: rails between tabs; the active tab's node is ◉, an empty tab's ○ and never open
  expect(draw(agentGraph(tabs, 0, new Set(), 20))).toEqual(["◉0", "├─│ x", "├─│ y", "|", "○1+", "|", "●2", "╰─  z"]);
  // a folded tab keeps its node and drops its agents
  expect(draw(agentGraph(tabs, 0, new Set(["a"]), 20))).toEqual(["◉0+", "|", "○1+", "|", "●2", "╰─  z"]);
  // too tall for rails: none; too tall still: every tab but the active one folds
  expect(draw(agentGraph(tabs, 2, new Set(), 9))).toEqual(["●0", "├─│ x", "├─│ y", "○1+", "◉2", "╰─  z"]);
  expect(draw(agentGraph(tabs, 2, new Set(), 5))).toEqual(["●0+", "○1+", "◉2", "╰─  z"]);
  // too tall even then: cut, a line left for the overflow row, the cut agents counted
  const cut = agentGraph([{ id: "a", agents: ["x", "y", "w"] }], 0, new Set(), 4);
  expect(draw(cut)).toEqual(["◉0", "├─│ x"]);
  expect(cut.hidden).toBe(2);
});

test("sidebar columns keep complete states aligned and fit Unicode into terminal cells", () => {
  for (const width of [16, 22, 30]) {
    for (const left of ["@reviewer", "claude-code", "日本語のターミナル", "👨‍👩‍👧‍👦 developer", "e\u0301e\u0301e\u0301", "line\nbreak"]) {
      for (const right of ["Needs you", "Working", "Done", "Idle", "!", "", "30"]) {
        const result = sidebarColumns(left, right, width);
        expect(result.right).toBe(right);
        expect(Bun.stringWidth(result.left + result.right)).toBe(width);
        expect(result.left + result.right).not.toContain("\n");
      }
    }
  }
});
