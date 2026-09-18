// Survive server restarts: each session's layout and pane metadata, saved in bun:sqlite.
import { Database } from "bun:sqlite";
import { DIR } from "../../core/paths";
import type { Node } from "../../core/layout";
import type { Session } from "../session/session";

type SavedPane = { name?: string; cwd: string; command?: string; harness?: string; agent?: string; session?: { agent: string; id: string }; createdBy: string };
export type Saved = {
  active: number;
  workspaces: { name: string; cwd: string; active: number; tabs: { name?: string; zoomed: boolean; focused: string; tree: Node }[] }[];
  panes: Record<string, SavedPane>;
};

// Live cwd of each shell (follows `cd`), batched into one lsof call.
export async function cwds(pids: number[]): Promise<Map<number, string>> {
  const m = new Map<number, string>();
  if (!pids.length) return m;
  if (await Bun.file("/proc/self/stat").exists()) {
    for (const pid of pids) {
      const cwd = (await Bun.$`readlink /proc/${pid}/cwd`.quiet().nothrow().text()).trim();
      if (cwd) m.set(pid, cwd);
    }
    return m;
  }
  const out = await Bun.$`lsof -a -d cwd -p ${pids.join(",")} -Fpn`.quiet().nothrow().text();
  let pid = 0;
  for (const line of out.split("\n")) {
    if (line[0] === "p") pid = +line.slice(1);
    else if (line[0] === "n" && pid) m.set(pid, line.slice(1));
  }
  return m;
}

// One row per session in ~/.local/state/modisa/modisa.db.
let db: Database | undefined;
async function store() {
  if (!db) {
    await Bun.$`mkdir -p ${DIR}`.quiet();
    db = new Database(`${DIR}/modisa.db`, { create: true });
    db.run("CREATE TABLE IF NOT EXISTS sessions (name TEXT PRIMARY KEY, data TEXT NOT NULL, saved_at INTEGER NOT NULL)");
  }
  return db;
}

// `alive` is checked after the async cwd lookup so a save racing a shutdown never resurrects a killed session.
export async function save(s: Session, session: string, alive: () => boolean = () => true) {
  if (!s.workspaces.length) return;
  const live = await cwds([...s.panes.values()].filter((p) => p.info.status === "running").map((p) => p.proc.pid));
  const data: Saved = {
    active: s.active,
    workspaces: s.workspaces.map((ws) => ({
      name: ws.name,
      cwd: ws.cwd,
      active: ws.active,
      tabs: ws.tabs.map((t) => ({ name: t.name, zoomed: t.zoomed, focused: t.focused, tree: t.tree })),
    })),
    panes: Object.fromEntries(
      [...s.panes.values()].map((p) => [
        p.id,
        {
          name: p.info.name,
          cwd: live.get(p.proc.pid) ?? p.info.cwd,
          command: p.info.command,
          harness: p.info.harness,
          agent: p.info.agent?.harness,
          session: p.info.agent && p.info.session?.agent === p.info.agent.harness ? p.info.session : undefined, // only while that agent runs
          createdBy: p.info.createdBy,
        },
      ]),
    ),
  };
  if (!alive()) return;
  (await store()).run("INSERT OR REPLACE INTO sessions (name, data, saved_at) VALUES (?, ?, ?)", [session, JSON.stringify(data), Date.now()]);
}

export async function load(session: string): Promise<Saved | undefined> {
  const row = (await store()).query("SELECT data FROM sessions WHERE name = ?").get(session) as { data: string } | null;
  return row ? JSON.parse(row.data) : undefined;
}

export async function forget(session: string) {
  (await store()).run("DELETE FROM sessions WHERE name = ?", [session]);
}

export async function saved(): Promise<{ name: string; saved_at: number }[]> {
  return (await store()).query("SELECT name, saved_at FROM sessions ORDER BY name").all() as any;
}
