// `modisa config check` and `config reset-keys`, which need no session, and [keys] as the server sees it: a plugin can't
// have a key [keys] gives to modisa, and gets one [keys] frees.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { cliResults } from "../../src/protocol/schema";

const sb = sandbox("config-cli");
const S = "cfg";
const REPO = `${import.meta.dir}/../..`;
const FILE = () => `${sb.root}/config/config.toml`;
const run = (...args: string[]) => sb.run(S, args);
const write = (text: string) => Bun.write(FILE(), text);

beforeAll(async () => {
  await Bun.$`mkdir -p ${sb.root}/config`.quiet();
});

afterAll(async () => {
  await run("kill", S);
  await sb.cleanup();
});

test("check: no file, or the sample, is fine", async () => {
  expect(await run("config", "check")).toMatchObject({ code: 0, stdout: expect.stringContaining("uses its defaults") });
  await run("config"); // prints the file, writing the sample first
  expect(await run("config", "check")).toMatchObject({ code: 0, stdout: expect.stringContaining("no problems") });
});

test("check: errors exit 1 with their line, unknown settings are warnings that exit 0, --json says the same", async () => {
  await write('theme = "ion"\nshiny = true\n\n[sidebar]\nwidth = 60\n\n[keys]\nzoom = "x"\n');
  const r = await run("config", "check");
  expect(r.code).toBe(1);
  expect(r.stdout).toContain(`${FILE()}:2: warning: shiny:`);
  expect(r.stdout).toContain(`${FILE()}:5: error: sidebar.width: 20 to 48 columns`);
  expect(r.stdout).toContain(`${FILE()}:8: error: keys.zoom: x is reserved`);
  expect(r.stdout.split("\n").at(-1)).toBe("2 errors, 1 warning");

  const j = await run("config", "check", "--json");
  expect(j.code).toBe(1);
  const parsed = cliResults["config check"].safeParse(JSON.parse(j.stdout));
  expect(parsed.success, parsed.error?.message).toBe(true);
  expect(parsed.data).toMatchObject({ file: FILE(), exists: true, ok: false });
  expect(parsed.data!.problems.map((p) => [p.level, p.key, p.line])).toEqual([["warning", "shiny", 2], ["error", "sidebar.width", 5], ["error", "keys.zoom", 8]]);

  await write("shiny = true\n");
  expect((await run("config", "check")).code).toBe(0);
});

test("check: a file that doesn't parse points at the line and column", async () => {
  await write('theme = "ion"\n[sidebar]\nwidth = \n');
  const r = await run("config", "check");
  expect(r.code).toBe(1);
  expect(r.stdout).toMatch(new RegExp(`${FILE().replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}:3:\\d+: error: `));
  expect((await run("config", "nope")).code).toBe(2);
});

test("reset-keys: [keys] and [plugin_keys] out, the prefix back, comments kept, the old file saved; then nothing to do", async () => {
  const before = '# mine\nprefix = "C-a"   # my prefix\ntheme = "nord"\n\n# my keys\n[keys]\nzoom = "f"\n\n[plugin_keys]\n"radar.open" = "R"\n\n[sidebar]\nwidth = 30\n';
  await write(before);
  const r = await run("config", "reset-keys");
  expect(r.code).toBe(0);
  expect(r.stdout.split("\n")).toEqual(["[keys] removed (1 setting)", "[plugin_keys] removed (1 setting)", 'prefix "C-a" → "C-b"', `the file as it was: ${FILE()}.bak`]);
  expect(await Bun.file(FILE()).text()).toBe('# mine\nprefix = "C-b"   # my prefix\ntheme = "nord"\n\n# my keys\n\n[sidebar]\nwidth = 30\n');
  expect(await Bun.file(`${FILE()}.bak`).text()).toBe(before);

  await Bun.write(`${FILE()}.bak`, "untouched");
  expect(await run("config", "reset-keys")).toMatchObject({ code: 0, stdout: expect.stringContaining("nothing changed") });
  expect(await Bun.file(`${FILE()}.bak`).text()).toBe("untouched"); // nothing to keep a copy of

  await write("[keys]\nzoom = \n");
  expect(await run("config", "reset-keys")).toMatchObject({ code: 1, stderr: expect.stringContaining("doesn't parse (line 2") });
  expect(await Bun.file(FILE()).text()).toBe("[keys]\nzoom = \n");
});

test("the server binds plugin keys around its config's [keys], live", async () => {
  await write("");
  await startServer(sb, S);
  const dir = `${sb.root}/keyed`;
  await Bun.write(`${dir}/plugin.json`, JSON.stringify({ name: "keyed", protocol: 1, run: ["bun", "plugin.ts"], actions: [{ id: "go", title: "Go" }], keys: [{ key: "z", action: "go", description: "lower" }, { key: "Z", action: "go", description: "upper" }] }));
  await Bun.write(`${dir}/modisa-plugin.ts`, await Bun.file(`${REPO}/src/plugins/modisa-plugin.ts`).text());
  await Bun.write(`${dir}/plugin.ts`, `import { runPlugin } from "./modisa-plugin";\nrunPlugin(async (modisa) => { await modisa.hello({ go: () => "went" }); });\n`);
  expect((await run("plugin", "link", dir)).code).toBe(0);
  const keys = async () => Object.fromEntries((await sb.json<any[]>(S, ["plugin", "list"])).find((p) => p.name === "keyed").keys.map((k: any) => [k.key, k.reason ?? k.state]));
  expect(await keys()).toEqual({ z: "modisa's zoom", Z: "active" });
  await write('[keys]\nzoom = "Z"\n');
  for (let i = 0; i < 50 && (await keys()).z !== "active"; i++) await Bun.sleep(200);
  expect(await keys()).toEqual({ z: "active", Z: "modisa's zoom" });
}, 40000);
