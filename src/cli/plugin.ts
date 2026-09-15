// The `shepherd plugin` commands that work without a session server (new, sdk, schema, check, dev), and link and
// unlink, which change the global link and then act on the one running session they reach (the default, or -s).
import { PLUGINS_DIR, readManifest } from "../config/plugins";
import { connectExisting } from "../protocol/transport";
import { PROTOCOL } from "../protocol/schema";
import { describeProtocol } from "../protocol/describe";
import type { Conn } from "../protocol/conn";
import { str, type Args } from "./args";
// @ts-ignore: embedded as text (it's a .ts file, which would otherwise be imported as a module)
import SDK from "../plugins/shepherd-plugin.ts" with { type: "text" };
import PLUGIN from "../plugins/template/plugin.ts.txt" with { type: "text" };
import TEST from "../plugins/template/plugin.test.ts.txt" with { type: "text" };
import GUIDE from "../plugins/template/AGENTS.md" with { type: "text" };

// The client library as text. (Importing the same file as a module too would clash with this in Bun's module cache,
// so its version is read from the text.)
export const SDK_TEXT = String(SDK);
export const sdkVersion = (text: string) => Number(/SDK_VERSION = (\d+)/.exec(text)?.[1]) || undefined;

const LOCAL = ["new", "sdk", "schema", "check", "dev", "link", "unlink"];
export const isLocalPluginCommand = (verb?: string) => !!verb && LOCAL.includes(verb);

const HELLO_MS = 10_000;

export async function runPluginLocal(verb: string, args: string[], flags: Args["flags"]): Promise<number> {
  const json = !!flags.json;
  switch (verb) {
    case "sdk":
      process.stdout.write(SDK_TEXT);
      return 0;
    case "schema":
      console.log(JSON.stringify(describeProtocol(), null, 2));
      return 0;
    case "new":
      return scaffold(args[0], str(flags.dir));
    case "check":
      return (await import("./plugin-check")).checkPlugin(args[0] ?? ".");
    case "dev":
      return (await import("./plugin-check")).devPlugin(args[0] ?? ".");
    case "link":
      return link(args[0], str(flags.session), json);
    default:
      return unlink(args[0], str(flags.session));
  }
}

async function scaffold(name: string | undefined, dirFlag?: string) {
  if (!name || !/^[a-z0-9][a-z0-9-]*$/.test(name)) {
    console.error("usage: shepherd plugin new <name> [--dir d]   (name: lowercase letters, digits and dashes)");
    return 2;
  }
  const dir = dirFlag ?? name;
  const existing = (await Bun.$`ls -A ${dir}`.quiet().nothrow().text()).trim();
  if (existing) {
    console.error(`shepherd: ${dir} already exists and isn't empty; plugin new never overwrites`);
    return 1;
  }
  const fill = (text: unknown) => String(text).replaceAll("{{name}}", name);
  await Bun.write(`${dir}/plugin.json`, JSON.stringify({ name, protocol: PROTOCOL, run: ["bun", "plugin.ts"], description: `${name}: a shepherd plugin` }, null, 2) + "\n");
  await Bun.write(`${dir}/plugin.ts`, fill(PLUGIN));
  await Bun.write(`${dir}/plugin.test.ts`, fill(TEST));
  await Bun.write(`${dir}/shepherd-plugin.ts`, SDK_TEXT);
  await Bun.write(`${dir}/AGENTS.md`, fill(GUIDE));
  await Bun.write(`${dir}/CLAUDE.md`, "Read AGENTS.md: it explains how to write, test and install this shepherd plugin.\n");
  console.log(`created ${dir}: plugin.json, plugin.ts, plugin.test.ts, shepherd-plugin.ts, AGENTS.md
next: put the plugin's logic in plugin.ts (AGENTS.md explains how), then
  shepherd plugin check ${dir}
  shepherd plugin link ${dir}`);
  return 0;
}

export const sessionName = (session?: string) => session ?? Bun.env.SHEPHERD_SESSION ?? "default";

// "<key>: <why>" for each of a plugin's keys that's off in that session
const offKeys = (status: any): string[] => (status?.keys ?? []).filter((k: any) => k.state === "disabled").map((k: any) => `${k.key || "(none)"}: ${k.reason}`);
const keysOff = (keys?: string[]) => (keys?.length ? `\n  keys off in the server's config: ${keys.join("; ")}` : "");

// Start a linked plugin in the one running session this reaches, and wait for it to connect: a process that started
// isn't a plugin that's ready. Never starts a session.
export async function startIn(session: string | undefined, name: string) {
  const where = sessionName(session);
  const conn: Conn | undefined = await connectExisting(session).catch(() => undefined);
  if (!conn) return { session: where, state: "not-started" as const, reason: `no running session ${where}; it starts with the next one` };
  const listed = async () => (await conn.request<any[]>("plugin.list")).find((p) => p.name === name);
  try {
    let status: any;
    try {
      status = await conn.request("plugin.start", { name });
    } catch (e) {
      const code = (e as { code?: string }).code;
      if (code === "already_running") {
        const s = await listed();
        return { session: where, state: "already-running" as const, pid: s?.pid, log: s?.log, disabledKeys: offKeys(s) };
      }
      return { session: where, state: "failed" as const, reason: (e as Error).message };
    }
    for (const end = Date.now() + HELLO_MS; ; await Bun.sleep(200)) {
      const s = (await listed()) ?? status;
      if (s.connected) return { session: where, state: "started" as const, pid: s.pid, log: s.log, disabledKeys: offKeys(s) };
      if (s.status !== "running") return { session: where, state: "failed" as const, reason: s.error ?? `it ${s.status}`, pid: s.pid, log: s.log };
      if (Date.now() > end) return { session: where, state: "no-hello" as const, reason: `it started but didn't connect within ${HELLO_MS / 1000}s`, pid: s.pid, log: s.log };
    }
  } finally {
    conn.close();
  }
}

export type StartOutcome = Awaited<ReturnType<typeof startIn>>;

export function describeStart(start: StartOutcome, what = "linked") {
  switch (start.state) {
    case "started":
      return `started in session ${start.session}, connected (pid ${start.pid})${keysOff(start.disabledKeys)}`;
    case "already-running":
      return `already running in session ${start.session} (pid ${start.pid}); not started again${keysOff(start.disabledKeys)}`;
    case "not-started":
      return `not started: ${start.reason}`;
    case "failed":
      return `${what}, but failed to start in session ${start.session}: ${start.reason}${start.log ? `\n  log: ${start.log}` : ""}`;
    case "no-hello":
      return `${what} and started in session ${start.session}, but ${start.reason}${start.log ? `\n  log: ${start.log}` : ""}`;
  }
}

// Registers the plugin for every session (they start it when they start), then starts it in the one running session
// this reaches. Exit 0 when it's connected, already running or there's no session; 1 when it's linked but didn't
// start or connect.
async function link(arg: string | undefined, session: string | undefined, json: boolean) {
  if (!arg) {
    console.error("usage: shepherd plugin link <dir> [--json]");
    return 2;
  }
  const dir = (await Bun.$`realpath ${arg}`.quiet().nothrow().text()).trim();
  if (!dir) {
    console.error(`shepherd: no such directory: ${arg}`);
    return 1;
  }
  const { manifest, error } = await readManifest(dir);
  if (!manifest) {
    console.error(`shepherd: ${error}`);
    return 1;
  }
  const target = `${PLUGINS_DIR}/${manifest.name}`;
  const existing = (await Bun.$`readlink ${target}`.quiet().nothrow().text()).trim();
  if (existing && existing !== dir) {
    console.error(`shepherd: ${manifest.name} is already linked to ${existing}; shepherd plugin unlink ${manifest.name} first`);
    return 1;
  }
  if (!existing) {
    await Bun.$`mkdir -p ${PLUGINS_DIR}`.quiet();
    await Bun.$`ln -sfn ${dir} ${target}`.quiet();
  }
  const start = await startIn(session, manifest.name);
  const result = { name: manifest.name, dir, linked: true as const, alreadyLinked: !!existing, start };
  if (json) console.log(JSON.stringify(result, null, 2));
  else console.log(`${existing ? "already linked" : "linked"} ${manifest.name} → ${dir} (every session starts it)\n${describeStart(start)}`);
  return start.state === "failed" || start.state === "no-hello" ? 1 : 0;
}

// Removes the link, which every session reads at its next start. A running copy is stopped only in the one session
// this reaches (the default, or -s); other running sessions keep theirs until they restart.
async function unlink(name: string | undefined, session: string | undefined) {
  if (!name) {
    console.error("usage: shepherd plugin unlink <name>");
    return 2;
  }
  const target = `${PLUGINS_DIR}/${name}`;
  if ((await Bun.$`test -L ${target}`.quiet().nothrow()).exitCode !== 0) {
    console.error(`shepherd: no linked plugin named ${name} in ${PLUGINS_DIR}`);
    return 1;
  }
  await Bun.$`rm ${target}`.quiet(); // the link only, never the plugin's directory
  console.log(`unlinked ${name}`);
  const where = sessionName(session);
  const conn = await connectExisting(session).catch(() => undefined);
  const stopped = conn ? await conn.request("plugin.stop", { name }).then(() => true, () => false) : false;
  conn?.close();
  console.log(stopped ? `stopped it in session ${where}; other running sessions keep their copy until they restart` : `no running copy in session ${where}; other running sessions keep theirs until they restart`);
  return 0;
}
