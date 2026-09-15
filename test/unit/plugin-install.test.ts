import { test, expect } from "bun:test";
import { withoutCredentials } from "../../src/config/plugins";

test("credentials never stay in a recorded or displayed git URL", () => {
  expect(withoutCredentials("https://user:secret@github.com/a/b.git")).toBe("https://github.com/a/b.git");
  expect(withoutCredentials("https://ghp_abc123@github.com/a/b")).toBe("https://github.com/a/b");
  expect(withoutCredentials("ssh://git@example.com/a/b.git")).toBe("ssh://example.com/a/b.git");
  expect(withoutCredentials("git@github.com:a/b.git")).toBe("git@github.com:a/b.git");
  expect(withoutCredentials("file:///tmp/repo.git")).toBe("file:///tmp/repo.git");
});
