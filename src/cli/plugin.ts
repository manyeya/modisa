// The `shepherd plugin` commands that need no session server: new, sdk, schema, check, dev, link, unlink.
import { PLUGINS_DIR, readManifest } from "../config/plugins";
import { connectExisting } from "../protocol/transport";
import { PROTOCOL } from "../protocol/schema";
import { describeProtocol } from "../protocol/describe";
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

export async function runPluginLocal(verb: string, args: string[], flags: Args["flags"]): Promise<number> {
  switch (verb) {
    case "sdk":
      process.stdout.write(String(SDK));
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
      return link(args[0]);
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
  await Bun.write(`${dir}/shepherd-plugin.ts`, String(SDK));
  await Bun.write(`${dir}/AGENTS.md`, fill(GUIDE));
  await Bun.write(`${dir}/CLAUDE.md`, "Read AGENTS.md: it explains how to write, test and install this shepherd plugin.\n");
  console.log(`created ${dir}: plugin.json, plugin.ts, plugin.test.ts, shepherd-plugin.ts, AGENTS.md
next: put the plugin's logic in plugin.ts (AGENTS.md explains how), then
  shepherd plugin check ${dir}
  shepherd plugin link ${dir} && shepherd restart`);
  return 0;
}

async function link(arg?: string) {
  if (!arg) {
    console.error("usage: shepherd plugin link <dir>");
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
  await Bun.$`mkdir -p ${PLUGINS_DIR}`.quiet();
  await Bun.$`ln -sfn ${dir} ${target}`.quiet();
  console.log(`linked ${manifest.name} → ${dir}\nit starts with the session server: shepherd restart`);
  return 0;
}

// Removes the link, which every session reads at its next start. A running copy is stopped only in the one session
// this reaches (the default, or -s); other running sessions keep theirs until they restart.
async function unlink(name?: string, session?: string) {
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
  const where = session ?? Bun.env.SHEPHERD_SESSION ?? "default";
  const conn = await connectExisting(session).catch(() => undefined);
  const stopped = conn ? await conn.request("plugin.stop", { name }).then(() => true, () => false) : false;
  conn?.close();
  console.log(stopped ? `stopped it in session ${where}; other running sessions keep their copy until they restart` : `no running copy in session ${where}; other running sessions keep theirs until they restart`);
  return 0;
}
