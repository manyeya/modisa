// `shepherd plugin check <dir>` and `plugin dev <dir>`: a throwaway session (its own state and config directories,
// with only this plugin linked) to verify a plugin against the real server, or to try it by hand. It isn't a sandbox:
// the plugin runs as you, with your files and network.
import { self } from "../core/paths";
import { readManifest } from "../config/plugins";
import { PROTOCOL } from "../protocol/schema";
import { connectUnix } from "../protocol/transport";
import type { Conn } from "../protocol/conn";
import { SDK_TEXT, sdkVersion } from "./plugin";
import { OwnedGroup } from "../server/plugins";

const SDK_VERSION = sdkVersion(SDK_TEXT);

const ok = (step: string, note?: string) => console.log(`✓ ${step}${note ? `: ${note}` : ""}`);
const skip = (step: string, why: string) => console.log(`– ${step}: skipped (${why})`);
const bad = (step: string, detail: string) => {
  console.log(`✗ ${step}\n${detail.trimEnd().split("\n").map((l) => `    ${l}`).join("\n")}`);
  return false;
};
const tail = async (file: string | undefined, lines = 20) => (file ? (await Bun.file(file).text().catch(() => "")).trimEnd().split("\n").slice(-lines).join("\n") : "") || "(empty)";

async function pluginDir(arg: string) {
  const dir = (await Bun.$`realpath ${arg}`.quiet().nothrow().text()).trim();
  if (!dir) console.error(`shepherd: no such directory: ${arg}`);
  return dir;
}

async function throwaway(dir: string, name: string) {
  const root = (await Bun.$`mktemp -d ${`${Bun.env.TMPDIR ?? "/tmp"}/shepherd-plugin-XXXXXX`}`.text()).trim();
  const env = { SHEPHERD_DIR: `${root}/state`, SHEPHERD_CONFIG_DIR: `${root}/config`, SHEPHERD_SOUND: "off", SHEPHERD_UPDATE_URL: "off" };
  await Bun.$`mkdir -p ${env.SHEPHERD_DIR} ${env.SHEPHERD_CONFIG_DIR}/plugins`.quiet();
  await Bun.$`ln -s ${dir} ${env.SHEPHERD_CONFIG_DIR}/plugins/${name}`.quiet();
  const childEnv: Record<string, string | undefined> = { ...Bun.env, ...env };
  for (const k of ["SHEPHERD_SOCKET", "SHEPHERD_PANE_ID", "SHEPHERD_SESSION", "SHEPHERD_CHECK"]) delete childEnv[k];
  const session = "check";
  return { root, env, childEnv, session, sock: `${env.SHEPHERD_DIR}/${session}.sock`, data: `${env.SHEPHERD_DIR}/plugins/${name}` };
}

export async function checkPlugin(arg: string): Promise<number> {
  const dir = await pluginDir(arg);
  if (!dir) return 2;
  const { manifest, error } = await readManifest(dir);
  if (!manifest) return bad("manifest", `${error}\nplugin.json needs at least: { "name": "my-plugin", "protocol": ${PROTOCOL}, "run": ["bun", "plugin.ts"] }`), 1;
  if (manifest.protocol !== PROTOCOL) return bad("manifest", `it says protocol ${manifest.protocol}; this shepherd speaks protocol ${PROTOCOL}`), 1;
  ok("manifest", `${manifest.name}, protocol ${manifest.protocol}, runs ${manifest.run.join(" ")}`);
  let pass = true;

  const lib = Bun.file(`${dir}/shepherd-plugin.ts`);
  if (!(await lib.exists())) skip("client library", "the plugin doesn't use shepherd-plugin.ts");
  else {
    const version = sdkVersion(await lib.text());
    if (version === SDK_VERSION) ok("client library", `version ${version}`);
    else pass = bad("client library", `shepherd-plugin.ts is version ${version || "unknown"}; this shepherd's is ${SDK_VERSION}. Refresh it: shepherd plugin sdk > shepherd-plugin.ts`);
  }

  const entry = manifest.run.find((a) => /\.[cm]?[jt]sx?$/.test(a));
  if (manifest.run[0] !== "bun" || !entry) skip("builds", "not started as bun <file>");
  else {
    const out = `${Bun.env.TMPDIR ?? "/tmp"}/shepherd-check-build-${process.pid}`;
    const built = await Bun.$`bun build ${entry} --target=bun --outdir=${out}`.cwd(dir).quiet().nothrow();
    await Bun.$`rm -rf ${out}`.quiet().nothrow();
    if (built.exitCode === 0) ok("builds", entry);
    else pass = bad("builds", built.stderr.toString() + built.stdout.toString());
  }

  const tsc = `${dir}/node_modules/.bin/tsc`;
  if (!(await Bun.file(tsc).exists())) skip("types", "no TypeScript in the plugin; bun add -d typescript @types/bun to check types");
  else {
    const typed = await Bun.$`${tsc} --noEmit`.cwd(dir).quiet().nothrow();
    if (typed.exitCode === 0) ok("types");
    else pass = bad("types", typed.stdout.toString());
  }
  if (!pass) {
    console.log("\nfix the above, then run shepherd plugin check again");
    return 1;
  }

  const t = await throwaway(dir, manifest.name);
  const serverLog = `${t.root}/server.log`;
  const server = Bun.spawn([...self(), "server", "-s", t.session], { env: t.childEnv, cwd: t.root, stdout: "ignore", stderr: Bun.file(serverLog) });
  // signal the server only while it hasn't exited, and the plugin's group only while it's provably still the plugin's
  const serverRunning = () => server.exitCode === null && server.signalCode === null;
  let conn: Conn | undefined;
  let group: OwnedGroup | undefined;
  let exited = false;
  try {
    for (let i = 0; i < 100 && !conn; i++) {
      await Bun.sleep(100);
      conn = await connectUnix(t.sock).catch(() => undefined);
    }
    if (!conn) return bad("throwaway session", `the server didn't start:\n${await tail(serverLog)}`), 1;
    const me = async () => (await conn!.request<any[]>("plugin.list")).find((p) => p.name === manifest.name);

    let state: any;
    for (const end = Date.now() + 10_000; Date.now() < end; await Bun.sleep(200)) {
      state = await me();
      if (state?.connected || (state && state.status !== "running")) break;
    }
    if (!state?.connected) {
      pass = bad("starts and connects", `${state ? `status ${state.status}${state.error ? `: ${state.error}` : ""}` : "shepherd didn't find it"}. Within 10s it should connect and call hello (runPlugin and shepherd.hello do).\nits log:\n${await tail(state?.log)}`);
      if (state?.pid) group = new OwnedGroup(state.pid);
    } else {
      ok("starts and connects", `actions: ${state.actions.join(", ") || "none"}`);
      group = new OwnedGroup(state.pid);
      await Bun.sleep(1000);
      state = await me();
      if (state?.connected && state.status === "running") ok("stays up");
      else pass = bad("stays up", `it's ${state?.status}${state?.connected ? "" : " and disconnected"}; its log:\n${await tail(state?.log)}`);

      const tests = (await Array.fromAsync(new Bun.Glob("**/*.test.{ts,js}").scan({ cwd: dir }))).filter((f) => !f.includes("node_modules"));
      if (!tests.length) skip("its tests", "none: add plugin.test.ts (AGENTS.md shows how)");
      else {
        const check = JSON.stringify({ bin: self(), session: t.session, env: t.env, plugin: manifest.name, data: t.data });
        const run = await Bun.$`bun test`.cwd(dir).env({ ...t.childEnv, SHEPHERD_CHECK: check } as Record<string, string>).quiet().nothrow();
        const output = run.stdout.toString() + run.stderr.toString();
        if (run.exitCode === 0) ok("its tests", /(\d+) pass/.exec(output)?.[0]);
        else pass = bad("its tests", output.split("\n").filter((l) => /\(fail\)|error|Expected|Received|✗/.test(l)).slice(0, 40).join("\n") || output.slice(-4000));
      }

      // The server dies outright, as in a crash: nothing stops the plugin but the plugin itself.
      if (serverRunning()) process.kill(server.pid, "SIGKILL");
      await server.exited;
      conn.close();
      for (const end = Date.now() + 5000; Date.now() < end && !(exited = !group.alive()); ) await Bun.sleep(100);
      if (exited) ok("exits when the session dies");
      else pass = bad("exits when the session dies", "it kept running after its session's server went away. Exit when the connection closes (runPlugin does); otherwise each restart leaves another copy running.");
    }
  } finally {
    if (!exited) group?.signal("SIGKILL");
    if (serverRunning()) {
      try {
        process.kill(server.pid, "SIGKILL");
      } catch {}
    }
    await Bun.$`rm -rf ${t.root}`.quiet().nothrow();
  }
  console.log(pass ? `\n${manifest.name} passes` : "\nfix the above, then run shepherd plugin check again");
  return pass ? 0 : 1;
}

export async function devPlugin(arg: string): Promise<number> {
  const dir = await pluginDir(arg);
  if (!dir) return 2;
  const { manifest, error } = await readManifest(dir);
  if (!manifest) {
    console.error(`shepherd: ${error}`);
    return 1;
  }
  const t = await throwaway(dir, manifest.name);
  console.log(`a throwaway session with ${manifest.name} running. It isn't a sandbox: the plugin runs as you, with your files and network.
its files (removed when you exit): ${t.root}
from another terminal:
  SHEPHERD_DIR=${t.env.SHEPHERD_DIR} SHEPHERD_CONFIG_DIR=${t.env.SHEPHERD_CONFIG_DIR} shepherd -s ${t.session} plugin logs ${manifest.name}`);
  await Bun.sleep(1500);
  const code = await Bun.spawn([...self(), "-s", t.session], { env: t.childEnv, cwd: dir, stdio: ["inherit", "inherit", "inherit"] }).exited;
  await Bun.spawn([...self(), "kill", t.session], { env: t.childEnv, stdio: ["ignore", "ignore", "ignore"] }).exited;
  await Bun.$`rm -rf ${t.root}`.quiet().nothrow();
  return code;
}
