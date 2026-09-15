import { test, expect } from "bun:test";
import { pluginManifest } from "../../src/protocol/schema";
import { linkMatches, LINK } from "../../src/protocol/links";

const REPO = `${import.meta.dir}/../..`;
const problems = (links: object[]) => {
  const r = pluginManifest.safeParse({ name: "demo", protocol: 1, run: ["bun", "plugin.ts"], actions: [{ id: "open", title: "Open" }], links });
  return r.success ? [] : r.error.issues.map((i) => i.message);
};

test("a regex link matches the URL exactly as captured, anywhere in it unless anchored", () => {
  const pr = { regex: "^https://github\\.com/[^/]+/[^/]+/pull/\\d+$", action: "open" };
  expect(linkMatches(pr, "https://github.com/o/r/pull/12")).toBe(true);
  expect(linkMatches(pr, "https://github.com/o/r/pull/12/files")).toBe(false);
  expect(linkMatches(pr, "https://github.com/o/r/issues/12")).toBe(false);
  expect(linkMatches(pr, "https://GitHub.com/o/r/pull/12")).toBe(false); // not normalised: the author writes (?i)
  expect(linkMatches({ regex: "(?i)^https://github\\.com/", action: "open" }, "https://GitHub.com/o")).toBe(true);
  expect(linkMatches({ regex: "/issues/\\d+", action: "open" }, "https://x.dev/a/issues/3")).toBe(true);
  expect(linkMatches({ regex: ".*", action: "open" }, "ftp://x.dev/")).toBe(false); // only http(s), whatever the regex
  expect(linkMatches({ regex: ".*", action: "open" }, `https://x.dev/${"a".repeat(LINK.url)}`)).toBe(false); // over the cap
});

test("a link has exactly one of pattern or regex, and a regex the engine can't run in linear time or bounded size is refused", () => {
  const both = "a link needs exactly one of pattern (a URL glob) or regex";
  expect(problems([{ action: "open" }])).toContain(both);
  expect(problems([{ pattern: "https://x/*", regex: "x", action: "open" }])).toContain(both);
  const refused = (regex: string) => problems([{ regex, action: "open" }]).join("\n");
  expect(refused("(a)\\1")).toContain("RE2 syntax: no backreferences or lookaround"); // a backreference
  expect(refused("(?=x)")).toContain("RE2 syntax: no backreferences or lookaround"); // lookahead
  expect(refused("(?<=x)a")).toContain("RE2 syntax: no backreferences or lookaround"); // lookbehind
  expect(refused("a{1001}")).toContain("invalid repeat count");
  expect(refused("a{1000}{1000}")).toContain("invalid nested repetition");
  expect(refused("[a-z]{1000}")).toMatch(/it compiles to \d+ instructions; the limit is 300/);
  expect(refused(`${"(".repeat(17)}a${")".repeat(17)}`)).toContain("its groups nest 17 deep; the limit is 16");
  expect(refused("x".repeat(LINK.source + 1))).not.toBe("");
  expect(problems(Array.from({ length: LINK.perPlugin + 1 }, () => ({ pattern: "https://x/*", action: "open" }))).length).toBeGreaterThan(0);
  expect(problems([{ regex: "^https://github\\.com/[^/]+/[^/]+/pull/\\d+$", action: "open" }, { pattern: "https://github.com/*", action: "open" }])).toEqual([]);
});

// The matcher in a process of its own, with this one holding the deadline and killing it on overrun: a timer in the
// matcher's own thread couldn't fire while a match is stuck.
async function watched(code: string, deadlineMs: number) {
  const child = Bun.spawn(["bun", "-e", code], { cwd: REPO, stdout: "pipe", stderr: "pipe" });
  let killed = false;
  const timer = setTimeout(() => ((killed = true), child.kill("SIGKILL")), deadlineMs);
  const [out, err] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text()]);
  await child.exited;
  clearTimeout(timer);
  return { killed, out: out.trim(), err };
}

test("patterns that take exponential time in a backtracking engine finish fast here: bare, after a URL prefix, and at the size limit", async () => {
  const { killed, out, err } = await watched(
    `
import { RE2JS } from "re2js";
import { linkMatches, LINK } from "./src/protocol/links";
const near = "a".repeat(LINK.url - 1) + "!";
const url = (prefix) => prefix + "a".repeat(LINK.url - prefix.length - 1) + "!";
const cases = [
  () => RE2JS.compile("^(a|aa)+$").test(near),
  () => RE2JS.compile("(a+)+$").test(near),
  () => linkMatches({ regex: "^https://example\\\\.test/(a|aa)+$", action: "o" }, url("https://example.test/")),
  () => linkMatches({ regex: "^https://example\\\\.test/(a+)+$", action: "o" }, url("https://example.test/")),
  () => linkMatches({ regex: "[a-z]{290}", action: "o" }, url("https://example.test/")), // near the size limit (292 instructions)
  () => linkMatches({ pattern: "https://" + "a*".repeat(245) + "b", action: "o" }, url("https://")),
];
const ms = cases.map((run) => { const t = performance.now(); const matched = run(); return { matched, ms: performance.now() - t }; });
console.log(JSON.stringify(ms));
`,
    5000,
  );
  expect(killed).toBe(false);
  expect(err).toBe("");
  const results = JSON.parse(out) as { matched: boolean; ms: number }[];
  expect(results.map((r) => r.matched)).toEqual([false, false, false, false, true, false]);
  for (const r of results) expect(r.ms).toBeLessThan(200);
}, 15000);
