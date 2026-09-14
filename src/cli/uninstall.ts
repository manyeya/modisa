// `shepherd uninstall [--purge] [--yes]`: take shepherd back off this machine. Package managers only
// remove the files they installed, so this does the rest: every agent integration and the shared
// skill, running sessions and saved state (config too, with --purge). The binary goes as well when
// install.sh put it there; otherwise the package manager's own remove command finishes the job.
import { CONFIG_DIR } from "../config/config";
import { DIR, self, socketPath } from "../core/paths";
import { installedBy } from "../core/install";
import { FROM_SOURCE } from "../core/version";
import { integrationStatus, uninstallAll } from "../integrations";
import { connectUnix } from "../protocol/transport";

export async function runUninstall(o: { purge: boolean; yes: boolean }): Promise<number> {
  if (Bun.env.SHEPHERD_SOCKET) {
    console.error("run `shepherd uninstall` from a terminal outside shepherd: it stops every session, this one included");
    return 1;
  }
  const exe = self()[0]!;
  const how = installedBy(exe, FROM_SOURCE);
  // Bun.Glob skips unix sockets, so list the directory with the shell
  const sessions = (await Bun.$`ls -1 ${DIR}`.quiet().nothrow().text()).split("\n").filter((f) => f.endsWith(".sock")).map((f) => f.slice(0, -5));
  const agents = (await integrationStatus()).filter((s) => s.status !== "none").map((s) => s.name);
  const hasConfig = await Bun.$`test -d ${CONFIG_DIR}`.quiet().nothrow().then((r) => r.exitCode === 0);

  if (!o.yes) {
    console.log("shepherd uninstall will:");
    if (sessions.length) console.log(`  stop running sessions: ${sessions.join(", ")}`);
    if (agents.length) console.log(`  remove shepherd's hooks and skill from: ${agents.join(", ")}`);
    console.log(`  delete saved sessions and state in ${DIR}`);
    if (hasConfig) console.log(o.purge ? `  delete your config in ${CONFIG_DIR}` : `  keep your config in ${CONFIG_DIR} (--purge deletes it)`);
    if (how.by === "script") console.log(`  delete ${exe}`);
    const answer = prompt("Continue? [y/N]");
    if (!/^y(es)?$/i.test(answer?.trim() ?? "")) {
      console.log("nothing removed");
      return 1;
    }
  }

  const { removed, failed } = await uninstallAll();
  for (const line of removed) console.log(line);
  for (const line of failed) console.error(line);
  for (const name of sessions) {
    const c = await connectUnix(socketPath(name)).catch(() => undefined);
    if (!c) continue; // a dead server's socket: the state directory goes below anyway
    await c.request("kill").catch(() => {});
    c.close();
    console.log(`stopped session ${name}`);
  }
  await Bun.$`rm -rf ${DIR}`.quiet().nothrow();
  console.log(`deleted ${DIR}`);
  if (hasConfig && o.purge) {
    await Bun.$`rm -rf ${CONFIG_DIR}`.quiet().nothrow();
    console.log(`deleted ${CONFIG_DIR}`);
  } else if (hasConfig) console.log(`kept your config in ${CONFIG_DIR} (--purge deletes it)`);

  if (how.by === "script") {
    const gone = await Bun.file(exe).delete().then(() => true, () => false);
    console.log(gone ? `deleted ${exe}` : `couldn't delete ${exe}: remove it by hand`);
  } else if (how.remove) console.log(`shepherd was installed with ${how.manager}; finish with: ${how.remove}`);
  else console.log("shepherd runs from source here: delete the checkout to finish");
  return failed.length ? 1 : 0;
}
