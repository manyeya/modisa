import { expect, test } from "bun:test";
import { sidebarAgents, sidebarBudget, sidebarColumns } from "../../src/client/design";

test("sidebar lists leave the footer and bottom spacing clear at every supported height", () => {
  for (const height of [20, 22, 30, 38, 58]) {
    for (const spaces of [1, 3, 6, 30]) {
      for (const agents of [0, 1, 3, 30]) {
        const b = sidebarBudget(height, spaces, agents);
        const rows = 6 + b.spaceRows + Number(b.moreSpaces) + (agents ? b.agentRows * 2 : 2) + Number(b.moreAgents);
        expect(rows).toBeLessThanOrEqual(height - 5);
        expect(b.spaceRows).toBeGreaterThanOrEqual(1);
        expect(b.spaceRows).toBeLessThanOrEqual(6);
        if (agents) expect(b.agentRows).toBeGreaterThanOrEqual(1);
        expect(b.moreSpaces).toBe(spaces > b.spaceRows);
        expect(b.moreAgents).toBe(agents > b.agentRows);
      }
    }
  }
});

test("agent overflow retains attention priority and the focused agent without duplicates", () => {
  const agents = ["blocked-a", "blocked-b", "done", "working", "focused"].map(id => ({ id }));
  expect(sidebarAgents(agents, "focused", 3).map(p => p.id)).toEqual(["blocked-a", "blocked-b", "focused"]);
  expect(sidebarAgents(agents, "focused", 1).map(p => p.id)).toEqual(["focused"]);
  expect(sidebarAgents(agents, "blocked-a", 3)).toEqual(agents.slice(0, 3));
  expect(sidebarAgents(agents, "shell-not-an-agent", 3)).toEqual(agents.slice(0, 3));
  expect(sidebarAgents(agents, "focused", 10)).toEqual(agents);
  expect(sidebarAgents(agents, "focused", 0)).toEqual([]);
  expect(sidebarAgents([], "focused", 2)).toEqual([]);
  expect(agents.map(p => p.id)).toEqual(["blocked-a", "blocked-b", "done", "working", "focused"]);
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
