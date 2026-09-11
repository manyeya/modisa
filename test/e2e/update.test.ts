// Updates: a release build finds a newer release in its channel's manifest, `shepherd update`
// replaces the binary only when the download matches its checksum, and the TUI shows the ↑ badge.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { MAIN, Screen, sandbox } from "../support/harness";

const sb = sandbox("update");
const bin = `${sb.root}/bin/shepherd`;
const plat = (() => {
  const [os, arch] = Bun.spawnSync(["uname", "-sm"]).stdout.toString().trim().split(/\s+/);
  return `${os === "Darwin" ? "darwin" : "linux"}-${arch === "x86_64" ? "x64" : "arm64"}`;
})();
const newBuild = new TextEncoder().encode("#!/bin/sh\necho i am shepherd 9.9.9\n");
const sha = (b: Uint8Array) => new Bun.CryptoHasher("sha256").update(b).digest("hex");
let manifest: any;
let server: ReturnType<typeof Bun.serve>;

beforeAll(async () => {
  server = Bun.serve({
    port: 0,
    fetch: (req) => {
      const path = new URL(req.url).pathname;
      if (path === "/manifest.json") return new Response(JSON.stringify(manifest, null, 2)); // as the release workflow writes it
      if (path === "/shepherd") return new Response(newBuild);
      return new Response("not found", { status: 404 });
    },
  });
  const url = `http://localhost:${server.port}`;
  manifest = { version: "9.9.9", channel: "stable", notes: "### Added\n- everything", assets: { [plat]: { url: `${url}/shepherd`, sha256: sha(newBuild) } } };
  // a release build of this checkout, as the installer would put it
  await Bun.$`bun build --compile --define BUILD_VERSION='"0.0.1"' ${MAIN} --outfile ${bin}`.quiet();
  if (Bun.which("codesign")) await Bun.$`codesign --remove-signature ${bin} && codesign -s - ${bin}`.quiet().nothrow();
}, 60000);

afterAll(async () => {
  server?.stop(true);
  await sb.cleanup();
});

const run = async (args: string[]) => {
  const r = await Bun.$`${bin} ${args}`.env({ ...sb.env, SHEPHERD_UPDATE_URL: `http://localhost:${server.port}/manifest.json` }).nothrow().quiet();
  return r.stdout.toString() + r.stderr.toString();
};

test("a release build reports its version and the newer release", async () => {
  const out = await run(["--version"]);
  expect(out).toContain("shepherd 0.0.1");
  expect(out).toContain("update available: 9.9.9");
});

test("a download that doesn't match its checksum is refused and nothing changes", async () => {
  const before = await Bun.file(bin).bytes();
  manifest.assets[plat].sha256 = "0".repeat(64);
  expect(await run(["update"])).toContain("doesn't match its published checksum");
  expect(await Bun.file(bin).bytes()).toEqual(before);
  manifest.assets[plat].sha256 = sha(newBuild);
}, 20000);

test("shepherd update replaces the binary with the new release", async () => {
  const out = await run(["update"]);
  expect(out).toContain("updated shepherd 0.0.1 → 9.9.9");
  expect(await Bun.file(bin).bytes()).toEqual(newBuild);
  expect(await Bun.$`${bin}`.text()).toBe("i am shepherd 9.9.9\n");
}, 20000);

test("the TUI shows the ↑ badge for a newer release, and none without one", async () => {
  const env = (url: string) => ({ ...sb.env, SHEPHERD_UPDATE_URL: url });
  let ui = new Screen(["-s", "upd"], env(`http://localhost:${server.port}/manifest.json`), sb.root);
  await ui.until("update badge", (s) => s.split("\n").at(-1)!.includes("↑ 9.9.9"), 10000);
  ui.write("\x02d");
  await Promise.race([ui.proc.exited, Bun.sleep(3000)]);
  ui.close();
  manifest.version = "0.0.0"; // older than this checkout
  await Bun.file(`${sb.root}/state/update.json`).delete().catch(() => {});
  ui = new Screen(["-s", "upd"], env(`http://localhost:${server.port}/manifest.json`), sb.root);
  await ui.until("no badge", (s) => s.includes("SPACES"));
  await Bun.sleep(2500);
  expect(ui.lines().at(-1)).not.toContain("↑");
  await sb.cli("upd", ["kill", "upd"]);
  ui.close();
}, 30000);

test("install.sh installs the release for this platform, and refuses a bad checksum", async () => {
  manifest.version = "9.9.9";
  const url = `http://localhost:${server.port}/manifest.json`;
  const dir = `${sb.root}/installed`;
  const install = () => Bun.$`sh ${import.meta.dir}/../../install.sh`.env({ ...sb.env, SHEPHERD_MANIFEST_URL: url, SHEPHERD_INSTALL_DIR: dir }).nothrow().quiet();
  // the workflow writes the manifest pretty-printed, one field per line, which the installer reads
  const serve = manifest;
  manifest = JSON.parse(JSON.stringify(serve));
  let r = await install();
  expect(r.exitCode).toBe(0);
  expect(r.stdout.toString()).toContain("installing for");
  expect(await Bun.file(`${dir}/shepherd`).bytes()).toEqual(newBuild);
  await Bun.file(`${dir}/shepherd`).delete();
  manifest.assets[plat].sha256 = "0".repeat(64);
  r = await install();
  expect(r.exitCode).not.toBe(0);
  expect(r.stderr.toString()).toContain("doesn't match its published checksum");
  expect(await Bun.file(`${dir}/shepherd`).exists()).toBe(false);
}, 20000);
