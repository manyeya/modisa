// Agents (adapters): how to recognise an agent, launch it, and read its state from the screen.
// Built-ins live in ./agents; ~/.config/shepherd/adapters/<id>.toml adds an agent or overrides one
// (any field; `rules` replaces its screen rules), and [agents.<id>] in config.toml sets launch/resume.
import { CONFIG_DIR, type Config } from "./config";
import { BUILTIN_AGENTS, type AgentDef, type RawRule } from "./agents";
import type { AgentState } from "../protocol/types";

export type { AgentState };
export type Adapter = AgentDef;

// The older format, still accepted: [[state]] name/match/region (last N non-blank lines), first match wins.
const fromStates = (states: any[]): RawRule[] =>
  states.map((s, i) => ({ id: `state-${i + 1}`, state: s.name, priority: states.length - i, region: s.region ? `bottom_non_empty_lines(${s.region})` : "whole_recent", regex: [`(?im)${s.match}`] }));

function fromFile(id: string, raw: any, base?: Adapter): Adapter {
  const rules = raw.rules ?? (raw.state ? fromStates(raw.state) : undefined);
  return {
    id,
    name: raw.name ?? base?.name ?? id,
    process: raw.process ?? base?.process ?? [id],
    launch: raw.launch ?? base?.launch ?? id,
    resume: raw.resume ?? base?.resume,
    resumeSession: raw.resumeSession ?? base?.resumeSession,
    activity: raw.activity ?? base?.activity ?? false,
    rules: rules ?? base?.rules ?? [],
  };
}

export async function loadAdapters(cfg: Config): Promise<Adapter[]> {
  const byId = new Map<string, Adapter>(BUILTIN_AGENTS.map((a) => [a.id, { ...a }]));
  const dir = `${CONFIG_DIR}/adapters`;
  const files = await Array.fromAsync(new Bun.Glob("*.toml").scan({ cwd: dir })).catch(() => [] as string[]);
  for (const f of files.sort()) {
    try {
      const raw = Bun.TOML.parse(await Bun.file(`${dir}/${f}`).text()) as any;
      const id = raw.id ?? f.replace(/\.toml$/, "");
      byId.set(id, fromFile(id, raw, byId.get(id)));
    } catch (e) {
      console.error(`shepherd: ignoring ${dir}/${f}: ${e}`);
    }
  }
  for (const [id, o] of Object.entries(cfg.agents)) {
    const a = byId.get(id);
    if (a) Object.assign(a, Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)));
  }
  return [...byId.values()];
}
