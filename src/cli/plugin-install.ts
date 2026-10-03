// `modisa plugin install <git-url | plugin@marketplace> [--ref r] [--subdir d]`, `plugin update <name>` and
// `plugin unlink <name>`: config/plugin-manage.ts does each, and this prints what it did. An install starts the plugin
// in the running session reached (the default, or -s); an update restarts it wherever it was running.
import { MANAGED_DIR } from "../config/plugins";
import { installPlugin, sessionName, unlinkPlugin, updatePlugin, type InstallResult, type UpdateResult } from "../config/plugin-manage";
import { describeStart, startIn } from "./plugin";

// <plugin>@<marketplace>: both ids, so no git URL looks like one
const MARKETPLACE_PLUGIN = /^[a-z0-9][a-z0-9-]*@[a-z0-9][a-z0-9-]*$/;
const started = (start?: { state: string }) => !start || ["started", "already-running", "not-started"].includes(start.state);
const at = (ref: string | null | undefined, commit?: string) => `${ref ? `ref ${ref}, ` : ""}commit ${commit}`;

function report(result: InstallResult, json: boolean) {
  if (json) console.log(JSON.stringify(result, null, 2));
  else if (!result.installed && !result.alreadyInstalled) console.error(`modisa: not installed (${result.stage}): ${result.reason}`);
  else {
    console.log(`${result.alreadyInstalled ? "already installed" : "installed"} ${result.name} from ${result.source} (${at(result.ref, result.commit)})\n  ${result.dir}`);
    if (result.start) console.log(describeStart(result.start, result.alreadyInstalled ? "already installed" : "installed"));
    for (const hint of result.hints ?? []) console.log(`note: ${hint}`);
  }
  return (result.installed || result.alreadyInstalled) && started(result.start) ? 0 : 1;
}

export async function install(arg: string | undefined, options: { ref?: string; subdir?: string; session?: string; json: boolean }): Promise<number> {
  if (!arg) {
    console.error("usage: modisa plugin install <git-url | plugin@marketplace> [--ref branch|tag|commit] [--subdir path] [--json]");
    return 2;
  }
  const listed = MARKETPLACE_PLUGIN.test(arg);
  if (listed && (options.ref || options.subdir)) {
    console.error(`usage: ${arg} is installed from where its marketplace says: --ref and --subdir are for a git URL`);
    return 2;
  }
  const { json } = options;
  const result = await installPlugin({
    ...(listed ? { marketplacePlugin: arg } : { source: arg, ref: options.ref, subdir: options.subdir }),
    cloning: json ? undefined : (source) => console.log(`installing from ${source}: only https, ssh, git and file transports are used, and no build or dependency scripts run before it starts. A plugin runs as you, with your files and network; it isn't sandboxed.`),
  });
  if (!result.installed && !result.alreadyInstalled) return report(result, json);
  const { hints, ...rest } = result; // the start goes before the hints, as it always has
  return report({ ...rest, start: await startIn(options.session, result.name!), ...(hints && { hints }) }, json);
}

function reportUpdate(r: UpdateResult) {
  if (r.stage) return console.error(`modisa: not updated (${r.stage}): ${r.reason}`);
  if (r.upToDate) return console.log(`${r.name} is up to date: ${r.source} (${at(r.ref, r.commit)})`);
  console.log(`updated ${r.name} from ${r.source} (${r.ref ? `ref ${r.ref}, ` : ""}${r.from!.slice(0, 7)} → ${r.commit!.slice(0, 7)})`);
  for (const start of r.restarted) console.log(describeStart(start, "updated").replace(/^started in/, "restarted in"));
  if (!r.restarted.length) console.log("it wasn't running in any session: each starts the new version when it starts it");
  for (const hint of r.hints ?? []) console.log(`note: ${hint}`);
}

export async function update(name: string | undefined, json: boolean): Promise<number> {
  if (!name) {
    console.error("usage: modisa plugin update <name> [--json]");
    return 2;
  }
  let result: UpdateResult;
  try {
    result = await updatePlugin(name);
  } catch (e) {
    console.error(`modisa: ${(e as Error).message}`);
    return 1;
  }
  if (json) console.log(JSON.stringify(result, null, 2));
  else reportUpdate(result);
  return !result.stage && result.restarted.every(started) ? 0 : 1;
}

export async function unlink(name: string | undefined, session: string | undefined, json: boolean): Promise<number> {
  if (!name) {
    console.error("usage: modisa plugin unlink <name> [--json]");
    return 2;
  }
  let result: Awaited<ReturnType<typeof unlinkPlugin>>;
  try {
    result = await unlinkPlugin(name, { session });
  } catch (e) {
    console.error(`modisa: ${(e as Error).message}`);
    return 1;
  }
  if (json) console.log(JSON.stringify(result, null, 2));
  else if (!result.managed) {
    const where = sessionName(session);
    console.log(`unlinked ${name}\n${result.stoppedIn.length ? `stopped it in session ${where}` : `no running copy in session ${where}`}; other running sessions keep theirs until they restart. Its directory is untouched.`);
  } else {
    console.log(`unlinked ${name}${result.stoppedIn.length ? `; stopped it in session ${result.stoppedIn.join(", ")}` : ""}`);
    if (result.checkout!.deleted) console.log(`deleted its checkout (${result.checkout!.path}); its data and logs are kept`);
    else {
      const why = [...result.stillUsing.map((s) => `session ${s} still runs it`), ...result.unreachable.map((s) => `session ${s} can't be reached (a sandbox?)`)].join("; ");
      console.log(`kept its checkout at ${result.checkout!.path}: ${why}. Once that's stopped, remove ${MANAGED_DIR}/${name}.`);
    }
  }
  return 0;
}
