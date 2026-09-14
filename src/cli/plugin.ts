// `shepherd plugin link <dir>` / `unlink <name>`: linking is a symlink under ~/.config/shepherd/plugins, so it
// needs no server. The plugin starts with the next session server (shepherd restart).
import { PLUGINS_DIR, readManifest } from "../config/plugins";
import { connectExisting } from "../protocol/transport";

export async function runPluginLink(verb: "link" | "unlink", arg?: string, session?: string): Promise<number> {
  if (!arg) {
    console.error(verb === "link" ? "usage: shepherd plugin link <dir>" : "usage: shepherd plugin unlink <name>");
    return 2;
  }
  if (verb === "link") {
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
    const link = `${PLUGINS_DIR}/${manifest.name}`;
    const existing = (await Bun.$`readlink ${link}`.quiet().nothrow().text()).trim();
    if (existing && existing !== dir) {
      console.error(`shepherd: ${manifest.name} is already linked to ${existing}; shepherd plugin unlink ${manifest.name} first`);
      return 1;
    }
    await Bun.$`mkdir -p ${PLUGINS_DIR}`.quiet();
    await Bun.$`ln -sfn ${dir} ${link}`.quiet();
    console.log(`linked ${manifest.name} → ${dir}\nit starts with the session server: shepherd restart`);
    return 0;
  }
  const link = `${PLUGINS_DIR}/${arg}`;
  if ((await Bun.$`test -L ${link}`.quiet().nothrow()).exitCode !== 0) {
    console.error(`shepherd: no linked plugin named ${arg} in ${PLUGINS_DIR}`);
    return 1;
  }
  await Bun.$`rm ${link}`.quiet(); // the link only, never the plugin's directory
  console.log(`unlinked ${arg}`);
  // and stop it in the session, if one is running it
  const conn = await connectExisting(session).catch(() => undefined);
  if (conn) {
    const stopped = await conn.request("plugin.stop", { name: arg }).then(() => true, () => false);
    conn.close();
    if (stopped) console.log(`stopped ${arg} in session ${session ?? Bun.env.SHEPHERD_SESSION ?? "default"}`);
  }
  return 0;
}
