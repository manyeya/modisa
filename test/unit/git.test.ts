import { test, expect } from "bun:test";
import { parseStatus } from "../../src/server/git";

test("git status is read for a branch, what's ahead and behind its upstream, and what's changed", () => {
  const status = ["# branch.oid 1234567890abcdef", "# branch.head main", "# branch.upstream origin/main", "# branch.ab +2 -1",
    "1 .M N... 100644 100644 100644 aaa bbb src/a.ts", "? notes.md", ""].join("\n");
  expect(parseStatus("/home/me/code/shop/", status)).toEqual({ repo: "shop", branch: "main", ahead: 2, behind: 1, changes: 2 });
});

test("no upstream: no ahead and behind; a detached HEAD is named by its short commit", () => {
  expect(parseStatus("/r/api", "# branch.oid abcdef1234\n# branch.head feature\n")).toEqual({ repo: "api", branch: "feature", changes: 0 });
  expect(parseStatus("/r/api", "# branch.oid abcdef1234\n# branch.head (detached)\n").branch).toBe("abcdef1");
});
