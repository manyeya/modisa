// Writing a plugin: `plugin new` scaffolds one that `plugin check` passes; check's failures say what to fix (a bad
// manifest, a plugin that never connects, one that keeps running after its session dies, a failing test); the
// attention-log example passes check with its own behavioural tests; and every copy of the client library matches
// the maintained one.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox } from "../support/harness";

const sb = sandbox("plugin-authoring");
const REPO = `${import.meta.dir}/../..`;
const modisa = (...args: string[]) => sb.run("unused", args);

beforeAll(async () => {
  await Bun.$`mkdir -p ${sb.root}`.quiet(); // commands run there
});
afterAll(() => sb.cleanup());

test("plugin new scaffolds a plugin that plugin check passes", async () => {
  const dir = `${sb.root}/demo`;
  const made = await modisa("plugin", "new", "demo", "--dir", dir);
  expect(made.code).toBe(0);
  for (const file of ["plugin.json", "plugin.ts", "plugin.test.ts", "modisa-plugin.ts", "AGENTS.md", "CLAUDE.md"]) expect(await Bun.file(`${dir}/${file}`).exists()).toBe(true);
  expect(await Bun.file(`${dir}/plugin.json`).json()).toMatchObject({ name: "demo", protocol: 1 });
  expect(await Bun.file(`${dir}/plugin.ts`).text()).not.toContain("{{name}}");
  expect((await modisa("plugin", "new", "demo", "--dir", dir)).code).toBe(1); // never overwrites

  const check = await modisa("plugin", "check", dir);
  expect(check.code, check.out).toBe(0);
  for (const step of ["manifest", "client library", "builds", "starts and connects", "its tests", "exits when the session dies"]) expect(check.stdout).toContain(`✓ ${step}`);
}, 90000);

// a scaffolded plugin with one file changed
async function variant(name: string, change: (dir: string) => Promise<unknown>) {
  const dir = `${sb.root}/${name}`;
  expect((await modisa("plugin", "new", name, "--dir", dir)).code).toBe(0);
  await change(dir);
  return modisa("plugin", "check", dir);
}

test("check's failures say what to fix", async () => {
  const manifest = await variant("bad-manifest", (dir) => Bun.write(`${dir}/plugin.json`, JSON.stringify({ name: "bad-manifest", protocol: 1 })));
  expect(manifest.code).toBe(1);
  expect(manifest.stdout).toContain("✗ manifest");
  expect(manifest.stdout).toContain("run:");

  const silent = await variant("silent", (dir) => Bun.write(`${dir}/plugin.ts`, `console.log("started, but never connects"); setInterval(() => {}, 1000);\n`));
  expect(silent.code).toBe(1);
  expect(silent.stdout).toContain("✗ starts and connects");
  expect(silent.stdout).toContain("started, but never connects"); // its log, to see why

  const clingy = await variant("clingy", async (dir) => {
    const source = await Bun.file(`${dir}/plugin.ts`).text();
    await Bun.write(`${dir}/plugin.ts`, source.replace(`import { runPlugin`, `import { connect as _connect, runPlugin`).replace("runPlugin(async (modisa) => {", "_connect().then(async (modisa) => {\n  process.on(\"SIGTERM\", () => {});\n  setInterval(() => {}, 1000);"));
  });
  expect(clingy.code).toBe(1);
  expect(clingy.stdout).toContain("✗ exits when the session dies");

  const failing = await variant("failing", (dir) => Bun.write(`${dir}/plugin.test.ts`, `import { test, expect } from "bun:test";\ntest("the brief", () => expect(1).toBe(2));\n`));
  expect(failing.code).toBe(1);
  expect(failing.stdout).toContain("✗ its tests");
  expect(failing.stdout).toContain("the brief");
}, 180000);

test("a plugin that crashes at startup: check shows its last stderr and how it exited", async () => {
  const crashy = await variant("crashy", (dir) => Bun.write(`${dir}/plugin.ts`, `console.error("CRASH-MARKER: boom at startup");\nprocess.exit(1);\n`));
  expect(crashy.code).toBe(1);
  expect(crashy.stdout).toContain("✗ starts and connects");
  expect(crashy.stdout).toContain("exit code 1");
  expect(crashy.stdout).toContain("exited with 1");
  expect(crashy.stdout).toContain("CRASH-MARKER: boom at startup");
  expect(crashy.stdout).not.toContain("log may be incomplete");
}, 60000);

test("a plugin that exits while a child holds its output open fails within the drain deadline, says its log may be incomplete, and the child goes with its group", async () => {
  const started = Date.now();
  const leaky = await variant("leaky", (dir) => Bun.write(`${dir}/plugin.json`, JSON.stringify({ name: "leaky", protocol: 1, run: ["sh", "-c", "sleep 37 & echo LEAK-MARKER >&2; exit 1"] })));
  expect(leaky.code).toBe(1);
  expect(Date.now() - started).toBeLessThan(9000); // well within check's 10s wait: the failure shows after the ~1s drain
  expect(leaky.stdout).toContain("exit code 1");
  expect(leaky.stdout).toContain("log may be incomplete");
  expect(leaky.stdout).toContain("LEAK-MARKER");
  // the child in its process group is ended by the owned-group cleanup
  for (let i = 0; i < 30 && (await Bun.$`pgrep -f "sleep 37"`.quiet().nothrow()).exitCode === 0; i++) await Bun.sleep(100);
  expect((await Bun.$`pgrep -f "sleep 37"`.quiet().nothrow()).exitCode).not.toBe(0);
}, 60000);

test("a large action result (over 2 MB, non-ASCII) reaches the caller intact, and the connection keeps working", async () => {
  const check = await variant("big-reply", async (dir) => {
    const source = await Bun.file(`${dir}/plugin.ts`).text();
    await Bun.write(`${dir}/plugin.ts`, source.replace("status: () =>", `big: () => ({ text: "é✓ 日本 ".repeat(400_000) }),\n    status: () =>`));
    await Bun.write(
      `${dir}/plugin.test.ts`,
      `import { test, expect } from "bun:test";
import { checkSession } from "./modisa-plugin";
const s = checkSession();
test("big arrives whole, and the next action still works", async () => {
  const r = await s.modisa("plugin", "run", s.plugin, "big");
  expect(r.code).toBe(0);
  expect(JSON.parse(r.stdout).text).toBe("é✓ 日本 ".repeat(400_000));
  expect((await s.modisa("plugin", "run", s.plugin, "status")).code).toBe(0);
});
`,
    );
  });
  expect(check.code, check.out).toBe(0);
  expect(check.stdout).toContain("✓ its tests");
}, 90000);

test("the attention-log example passes plugin check, including its own behavioural tests", async () => {
  const check = await modisa("plugin", "check", `${REPO}/examples/plugins/attention-log`);
  expect(check.code, check.out).toBe(0);
  expect(check.stdout).toContain("✓ its tests");
}, 120000);

test("every copy of the client library is the maintained one", async () => {
  const maintained = await Bun.file(`${REPO}/src/plugins/modisa-plugin.ts`).text();
  expect((await modisa("plugin", "sdk")).stdout).toBe(maintained.trim());
  expect(await Bun.file(`${REPO}/examples/plugins/attention-log/modisa-plugin.ts`).text()).toBe(maintained);
});
