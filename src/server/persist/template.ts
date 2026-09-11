// shepherd.toml: a project's starting layout (panes, commands, agents) for a new session.
import type { Adapter } from "../../config/adapters";
import type { Session, SpawnOpts } from "../session/session";

type TemplatePane = { name?: string; run?: string; agent?: string; prompt?: string; cwd?: string };

export const quote = (s: string) => `'${s.replace(/'/g, `'\\''`)}'`;

export function templateOpts(t: TemplatePane, adapters: Adapter[], cwd: string): SpawnOpts {
  const a = t.agent ? adapters.find((x) => x.id === t.agent) : undefined;
  if (t.agent && !a) throw new Error(`unknown agent "${t.agent}" in shepherd.toml`);
  const command = a ? [a.launch, t.prompt && quote(t.prompt)].filter(Boolean).join(" ") : t.run;
  return { name: t.name, command, harness: a?.id, cwd: t.cwd ? (t.cwd.startsWith("/") ? t.cwd : `${cwd}/${t.cwd}`) : cwd };
}

// First pane on the left, the rest stacked on the right.
export async function applyTemplate(s: Session, dir: string, adapters: Adapter[]): Promise<boolean> {
  const file = Bun.file(`${dir}/shepherd.toml`);
  if (!(await file.exists())) return false;
  const t = Bun.TOML.parse(await file.text()) as { name?: string; pane?: TemplatePane[] };
  const list = t.pane ?? [];
  if (!list.length) return false;
  const first = s.newWorkspace(t.name, dir, templateOpts(list[0]!, adapters, dir));
  let prev = first.id;
  list.slice(1).forEach((p, i) => {
    prev = s.split(i === 0 ? "row" : "col", templateOpts(p, adapters, dir), i === 0 ? first.id : prev, false)!.id;
  });
  return true;
}
