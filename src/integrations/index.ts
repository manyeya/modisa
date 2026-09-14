// `shepherd integration status | install | uninstall <agent|all>`: connect agents to shepherd through
// their own hooks or plugins (see ./targets.ts for what each one does).
import { lstat, readlink } from "node:fs/promises"; // no Bun equivalent, and status must not spawn a process per agent
import type { IntegrationStatus } from "../protocol/types";
import { SKILL } from "../skills";
import { skillDir, skillsHome, TARGETS, type Status, type Target } from "./targets";

export { TARGETS };

// The skill: one copy in the shared skills directory, symlinked into every agent that reads skills
// somewhere else. Agents whose skills directory *is* the shared one just find it there.
// ponytail: symlinks only. An agent that doesn't follow them gets nothing; add a copy fallback if one
// turns up.
const linkPath = (t: Target) => (t.skills && t.skills() !== skillsHome() ? `${t.skills()}/shepherd` : undefined);
// A directory of their own, as opposed to our symlink (lstat, so a link to a directory is not one).
const isDir = (path: string) => lstat(path).then((s) => s.isDirectory()).catch(() => false);

// Status runs for every target on every settings refresh, so this stays free of subprocesses.
async function skillStatus(t: Target): Promise<Status | undefined> {
  if (!t.skills) return undefined;
  if ((await Bun.file(`${skillDir()}/SKILL.md`).text().catch(() => "")) !== SKILL) return "none";
  const link = linkPath(t);
  if (!link) return "current";
  return (await readlink(link).catch(() => "")) === skillDir() ? "current" : "none";
}

async function setSkill(t: Target, install: boolean) {
  if (!t.skills) return;
  const link = linkPath(t);
  if (install) {
    await Bun.write(`${skillDir()}/SKILL.md`, SKILL);
    // A real directory there is someone's own copy of the skill, not ours: leave it be rather than
    // link inside it. Status keeps saying "update available" until they take it out.
    if (link && !(await isDir(link))) await Bun.$`mkdir -p ${t.skills()} && ln -sfn ${skillDir()} ${link}`.quiet().nothrow();
    return;
  }
  if (link && !(await isDir(link))) await Bun.$`rm -f ${link}`.quiet().nothrow();
  // The shared copy goes when the last agent that wanted it does.
  const others = await Promise.all(TARGETS.filter((o) => o.skills && o.id !== t.id).map((o) => o.status().catch(() => "none" as Status)));
  if (!others.includes("current") && !others.includes("outdated")) await Bun.$`rm -rf ${skillDir()}`.quiet().nothrow();
}

// Names people type for an agent, besides its id.
const ALIASES: Record<string, string> = { claude: "claude-code", cursor: "cursor-agent", agy: "antigravity", "antigravity-cli": "antigravity", "kilo-code": "kilo", qoder: "qodercli" };
const find = (name: string) => TARGETS.find((t) => t.id === (ALIASES[name] ?? name));

async function statusOf(t: Target): Promise<IntegrationStatus> {
  const configured = await Bun.$`test -d ${t.dir()}`.quiet().nothrow().then((r) => r.exitCode === 0);
  const hooks = await t.status().catch(() => "outdated" as const);
  // A missing or stale skill makes an otherwise-installed integration an update, not a fresh install.
  const skill = await skillStatus(t).catch(() => "none" as const);
  return {
    id: t.id, name: t.name, kind: t.kind, configured,
    available: configured || t.binaries.some((b) => Bun.which(b)),
    status: hooks === "current" && skill === "none" ? "outdated" : hooks,
  };
}

export const integrationStatus = () => Promise.all(TARGETS.map(statusOf));

// Recommended: agents you have (on PATH or set up) whose integration is missing or out of date.
export const recommended = (list: IntegrationStatus[]) => list.filter((s) => s.status === "outdated" || (s.available && s.status === "none"));

export async function setIntegration(id: string, install: boolean): Promise<string> {
  const t = find(id);
  if (!t) throw new Error(`no integration for ${id}`);
  if (install && !t.create && !(await statusOf(t)).configured) throw new Error(`${t.name} isn't set up here (no ${t.dir()}); run it once, then install`);
  await (install ? t.install() : t.uninstall());
  await setSkill(t, install);
  const what = (t.kind === "lifecycle" ? "state and session reports" : "session reports") + (t.skills ? " and the shepherd skill" : "");
  return install ? `${t.name}: installed ${what} (restart running ${t.name} sessions to load it)` : `${t.name}: removed`;
}

const LABEL = { current: "✓ installed", outdated: "↻ update available", none: "not installed" };

export async function runIntegration(verb: string | undefined, agent: string | undefined): Promise<number> {
  if (verb === "status") {
    for (const s of await integrationStatus()) {
      const note = s.status === "none" && !s.available ? "  (not found)" : "";
      console.log(`${s.name.padEnd(16)} ${LABEL[s.status].padEnd(20)} ${s.kind === "lifecycle" ? "state + session" : "session"}${note}`);
    }
    return 0;
  }
  if ((verb !== "install" && verb !== "uninstall") || !agent || (agent !== "all" && !find(agent))) {
    console.error(`usage: shepherd integration status | install|uninstall <agent|all>\nagents: ${TARGETS.map((t) => t.id).join(", ")}\nevery agent's state is read from its screen with no setup; integrations add session resume, and exact state for ${TARGETS.filter((t) => t.kind === "lifecycle").map((t) => t.name).join(", ")}.`);
    return 2;
  }
  const all = agent === "all" ? await integrationStatus() : [];
  const ids = agent !== "all" ? [agent] : verb === "install" ? recommended(all).map((s) => s.id) : all.filter((s) => s.status !== "none").map((s) => s.id);
  if (!ids.length) console.log(verb === "install" ? "nothing to install: every agent found here is up to date" : "no integrations installed");
  let failed = 0;
  for (const id of ids) {
    try {
      console.log(await setIntegration(id, verb === "install"));
    } catch (e) {
      failed++;
      console.error((e as Error).message);
    }
  }
  return failed ? 1 : 0;
}
