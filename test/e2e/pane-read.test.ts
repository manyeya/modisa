// pane read: the visible screen, or the scrollback's tail as the pane wraps it or with soft wraps joined, as plain text
// or with colours and styles; --screen is --source visible, and the reply keeps screen and recentOutput. A full-screen
// app on the alternate screen has only its screen, in every source. A server from before source and format still
// answers the plain reads; anything else asks for a restart.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";

const sb = sandbox("pane-read");
const S = "read";
const run = (...args: string[]) => sb.run(S, args);
const read = async (...args: string[]) => {
  const r = await run("pane", "read", ...args);
  expect(r.code).toBe(0);
  return r.stdout;
};
const SOURCES = ["recent", "recent-unwrapped", "visible"] as const;
const FORMATS = ["text", "ansi"] as const;
const lines = (s: string) => s.split("\n");
// wider than the panes here (the first is 58 columns): it soft-wraps
const wide = (c: string) => `head -c 150 /dev/zero | tr '\\0' ${c}; echo`;

beforeAll(async () => {
  await startServer(sb, S);
}, 20000);

afterAll(async () => {
  await run("kill", S);
  await sb.cleanup();
});

test("every source in either format: what it covers, colour, and soft wraps", async () => {
  // 60 lines that scroll a 36-row pane's first ones away, a red word, a line that wraps, a marker
  await run("pane", "split", "--name", "plain", `seq -f 'top-%03g' 1 60; printf '\\033[31mred-text\\033[0m\\n'; ${wide("x")}; echo the-end; sleep 600`);
  expect((await run("wait", "plain", "--match", "the-end", "--timeout", "10")).code).toBe(0);
  const X = "x".repeat(150);

  for (const source of SOURCES) {
    const text = await read("plain", "--source", source, "--format", "text", "--lines", "200");
    const ansi = await read("plain", "--source", source, "--format", "ansi", "--lines", "200");
    expect(text).not.toContain("\x1b");
    expect(ansi).toMatch(/\x1b\[(31|38;5;1)mred-text\x1b\[0m/);
    expect(Bun.stripANSI(ansi)).toBe(text); // the same lines, styled
    expect(lines(text)).toContain("red-text");
    expect(lines(text).at(-1)).toBe("the-end");
    // the scrollback is in the recent reads; the screen is only what's left on it
    if (source === "visible") expect([text.includes("top-001"), text.includes("top-060")]).toEqual([false, true]);
    else expect(lines(text)[0]).toBe("top-001");
    // as the pane wraps it, or joined (the screen joins soft-wrapped rows, as it always has)
    if (source === "recent") expect(lines(text).filter((l) => l.startsWith("x"))).toEqual(["x".repeat(58), "x".repeat(58), "x".repeat(34)]);
    else expect(lines(text).filter((l) => l.startsWith("x"))).toEqual([X]);
  }

  // recent text is the default; --lines counts lines as each source has them
  expect(await read("plain")).toBe(await read("plain", "--source", "recent", "--format", "text"));
  expect(lines(await read("plain", "--lines", "3"))).toEqual(["x".repeat(58), "x".repeat(34), "the-end"]);
  expect(lines(await read("plain", "--source", "recent-unwrapped", "--lines", "2"))).toEqual([X, "the-end"]);
  // --screen is --source visible
  expect(await read("plain", "--screen")).toBe(await read("plain", "--source", "visible"));
  expect(await read("plain", "--screen", "--format", "ansi")).toBe(await read("plain", "--source", "visible", "--format", "ansi"));

  // the reply says what it read, and keeps screen and recentOutput as they were
  const r = JSON.parse(await read("plain", "--source", "visible", "--format", "ansi", "--json"));
  expect(r).toMatchObject({ id: "p2", source: "visible", format: "ansi", content: await read("plain", "--source", "visible", "--format", "ansi"), screen: await read("plain", "--screen"), recentOutput: await read("plain") });
  const plain = JSON.parse(await read("plain", "--json"));
  expect([plain.source, plain.format, plain.content]).toEqual(["recent", "text", plain.recentOutput]);

  for (const args of [["--source", "everything"], ["--format", "html"], ["--screen", "--source", "recent"]]) expect((await run("pane", "read", "plain", ...args)).code).toBe(2);
}, 30000);

test("a full-screen app on the alternate screen has only its screen, in every source and format", async () => {
  await run("pane", "split", "--name", "full", `seq -f 'primary-%03g' 1 60; printf '\\033[?1049h\\033[H\\033[32malt-screen\\033[0m\\n'; ${wide("y")}; echo alt-end; read x; printf '\\033[?1049l'; echo back-home; sleep 600`);
  expect((await run("wait", "full", "--match", "alt-end", "--timeout", "10")).code).toBe(0);
  const { cols } = JSON.parse(await read("full", "--json"));
  for (const source of SOURCES)
    for (const format of FORMATS) {
      const got = await read("full", "--source", source, "--format", format, "--lines", "200");
      const text = Bun.stripANSI(got);
      expect([text.includes("alt-screen"), text.includes("alt-end"), text.includes("primary-")]).toEqual([true, true, false]);
      if (format === "ansi") expect(got).toMatch(/\x1b\[(32|38;5;2)malt-screen/);
      else expect(got).not.toContain("\x1b");
      expect(lines(text).filter((l) => l.startsWith("y")).length).toBe(source === "recent" ? Math.ceil(150 / cols) : 1);
    }
  // back on the normal screen, its scrollback is there again
  await run("pane", "keys", "full", "Enter");
  expect((await run("wait", "full", "--match", "back-home", "--timeout", "10")).code).toBe(0);
  const back = await read("full", "--lines", "200");
  expect([back.includes("primary-001"), back.includes("back-home"), back.includes("alt-screen")]).toEqual([true, true, false]);
}, 30000);

test("a server from before source and format answers the plain reads; the rest ask for a restart", async () => {
  // it replies the way those servers did: no content, and session.info with counts
  const sock = `${sb.root}/state/old.sock`;
  const replies: Record<string, unknown> = { "pane.read": { id: "p1", screen: "the screen", recentOutput: "recent lines" }, "session.info": { session: "old", clients: 0, panes: 1, workspaces: 1 }, "debug.detect": { pane: "p1" } };
  const old = Bun.listen({
    unix: sock,
    socket: {
      data(s, d) {
        for (const line of new TextDecoder().decode(d).split("\n").filter(Boolean)) {
          const m = JSON.parse(line);
          s.write(JSON.stringify({ jsonrpc: "2.0", id: m.id, result: replies[m.method] ?? null }) + "\n");
        }
      },
    },
  });
  const ask = (...args: string[]) => sb.run("old", args);
  try {
    expect(await ask("pane", "read")).toMatchObject({ code: 0, out: "recent lines" });
    expect(await ask("pane", "read", "--source", "recent", "--format", "text")).toMatchObject({ code: 0, out: "recent lines" });
    expect(await ask("pane", "read", "--screen")).toMatchObject({ code: 0, out: "the screen" });
    expect(await ask("pane", "read", "--source", "visible")).toMatchObject({ code: 0, out: "the screen" });
    for (const args of [["pane", "read", "--format", "ansi"], ["pane", "read", "--source", "recent-unwrapped"], ["pane", "layout"], ["pane", "edges"], ["pane", "neighbor", "--direction", "up"], ["pane", "process-info"]])
      expect(await ask(...args)).toMatchObject({ code: 1, out: "modisa: server too old: modisa restart" });
  } finally {
    old.stop(true);
  }
}, 30000);
