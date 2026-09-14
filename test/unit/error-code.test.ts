import { test, expect } from "bun:test";
import { errorCode } from "../../src/protocol/conn";

test("only shepherd's own error codes pass through", () => {
  expect(errorCode("pane_gone")).toBe("pane_gone");
  expect(errorCode("ECONNRESET")).toBe("error");
  expect(errorCode(undefined)).toBe("error");
  expect(errorCode(42)).toBe("error");
});
