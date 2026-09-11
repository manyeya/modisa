// `shepherd integration status | install | uninstall <agent|all>`: connect agents to shepherd through
// their own hooks or plugins (see ./targets.ts for what each one does).
import type { IntegrationStatus } from "../protocol/types";
import { TARGETS, type Target } from "./targets";

export { TARGETS };

// Names people type for an agent, besides its id.
const ALIASES: Record<string, string> = { claude: "claude-code", cursor: "cursor-agent", agy: "antigravity", "antigravity-cli": "antigravity", "kilo-code": "kilo", qoder: "qodercli" };
const find = (name: string) => TARGETS.find((t) => t.id === (ALIASES[name] ?? name));

async function statusOf(t: Target): Promise<IntegrationStatus> {
  const configured = await Bun.$`test -d ${t.dir()}`.quiet().nothrow().then((r) => r.exitCode === 0);
  return {
    id: t.id, name: t.name, kind: t.kind, configured,
    available: configured || t.binaries.some((b) => Bun.which(b)),
    status: await t.status().catch(() => "outdated" as const),
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
  const what = t.kind === "lifecycle" ? "state and session reports" : "session reports";
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
