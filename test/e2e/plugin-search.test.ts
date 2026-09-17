// `modisa plugin search`, against a local stand-in for GitHub's search API (no network): it asks for repositories
// with the modisa-tui-plugin topic and the given words, most starred first; lists each with its install command and
// says none is vetted; strips control and bidi characters from what the index sends; drops what install can't fetch;
// never sends a GitHub token to another index; and fails plainly (exit 3) when the index is off, unreachable or
// rate-limiting. Every result is checked in its --json form too.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox } from "../support/harness";
import { cliResults } from "../../src/protocol/schema";

const sb = sandbox("plugin-search");
let server: ReturnType<typeof Bun.serve>;
// what each request asked for, copied while it's handled: Bun clears a Request once its handler returns
let requests: { url: string; authorization: string | null }[] = [];
let reply: () => Response;

const repo = (name: string, extra: object = {}) => ({
  name, full_name: `someone/${name}`, html_url: `https://github.com/someone/${name}`, clone_url: `https://github.com/someone/${name}.git`,
  description: `${name} for modisa`, stargazers_count: 7, pushed_at: "2026-09-01T10:00:00Z", archived: false, ...extra,
});
const results = (items: object[]) => () => Response.json({ total_count: items.length, items });

beforeAll(async () => {
  await Bun.$`mkdir -p ${sb.root}`.quiet(); // the CLI runs there, and no session starts to make it
  server = Bun.serve({ port: 0, fetch: (req) => (requests.push({ url: req.url, authorization: req.headers.get("authorization") }), reply()) });
});
afterAll(async () => {
  server?.stop(true);
  await sb.cleanup();
});

const search = async (args: string[], env: Record<string, string> = {}) => {
  requests = [];
  return sb.run("unused", ["plugin", "search", ...args], { MODISA_PLUGIN_INDEX: `http://localhost:${server.port}`, ...env });
};

test("it searches the modisa-tui-plugin topic with the words given, most starred first, and lists each with its install command", async () => {
  reply = results([repo("attention-log", { stargazers_count: 42 }), repo("pr-opener")]);
  const r = await search(["github", "pr"]);
  expect(r.code).toBe(0);
  const url = new URL(requests[0]!.url);
  expect(url.pathname).toBe("/search/repositories");
  expect(url.searchParams.get("q")).toBe("topic:modisa-tui-plugin github pr");
  expect(url.searchParams.get("sort")).toBe("stars");
  expect(r.stdout).toContain("someone/attention-log  ★ 42");
  expect(r.stdout).toContain("modisa plugin install https://github.com/someone/attention-log.git");
  expect(r.stdout).toContain("None of these is vetted");

  const json = await search(["github", "pr", "--json"]);
  const parsed = cliResults["plugin search"].safeParse(JSON.parse(json.stdout));
  expect(parsed.success, parsed.error?.message).toBe(true);
  expect(parsed.data).toMatchObject({ query: "github pr", total: 2, results: [{ repo: "someone/attention-log", stars: 42, install: "modisa plugin install https://github.com/someone/attention-log.git" }, { name: "pr-opener" }] });
});

test("what the index sends can't write to the terminal, and a repository install can't fetch as-is isn't listed", async () => {
  reply = results([
    repo("sneaky", { description: "safe\x1b]0;owned\x07 text‮gpj.exe\ron a new line" }),
    repo("ssh-only", { clone_url: "git@github.com:someone/ssh-only.git" }),
    repo("gone", { clone_url: undefined }),
  ]);
  const r = await search([]);
  expect(r.code).toBe(0);
  expect(r.stdout).not.toMatch(/[\x00-\x08\x0b-\x1f\x7f‮]/);
  expect(r.stdout).toContain("someone/sneaky");
  expect(r.stdout).not.toContain("ssh-only");
  expect(r.stdout).not.toContain("someone/gone");
  const json = JSON.parse((await search(["--json"])).stdout);
  expect(json.results.map((x: any) => x.name)).toEqual(["sneaky"]);
  expect(json.results[0].description).not.toMatch(/[\x00-\x1f‮]/);
});

test("remote text is cut by terminal cells, never through a wide character", async () => {
  reply = results([repo("wide", { description: "日".repeat(150) })]); // 300 cells
  const json = JSON.parse((await search(["--json"])).stdout);
  expect(json.results[0].description).toBe("日".repeat(100)); // 200 cells
  expect(Bun.stringWidth(json.results[0].description)).toBe(200);
  const text = (await search([])).stdout;
  expect(text).toContain(`  ${"日".repeat(100)}\n`);
});

test("nothing found says so", async () => {
  reply = results([]);
  const r = await search(["nothing-like-this"]);
  expect(r.code).toBe(0);
  expect(r.stdout).toContain('no plugins with the modisa-tui-plugin topic matching "nothing-like-this"');
});

test("a GitHub token is never sent to another index", async () => {
  reply = results([]);
  await search([], { GITHUB_TOKEN: "ghp_secret" });
  expect(requests).toHaveLength(1);
  expect(requests[0]!.authorization).toBeNull();
});

test("an index that's off, unreachable or rate-limiting fails with exit 3 and says why", async () => {
  const off = await search(["--json"], { MODISA_PLUGIN_INDEX: "off" });
  expect(off.code).toBe(3);
  expect(JSON.parse(off.stderr).error).toMatchObject({ code: "unreachable" });

  reply = () => new Response("slow down", { status: 403 });
  const limited = await search([]);
  expect(limited.code).toBe(3);
  expect(limited.stderr).toContain("rate-limiting");

  reply = () => new Response("<html>not json</html>");
  expect((await search([])).stderr).toContain("isn't a search result");

  const closed = Bun.serve({ port: 0, fetch: () => new Response("") });
  const port = closed.port;
  closed.stop(true);
  const unreachable = await search([], { MODISA_PLUGIN_INDEX: `http://localhost:${port}` });
  expect(unreachable.code).toBe(3);
  expect(unreachable.stderr).toContain("couldn't reach the plugin index");
}, 30000);
