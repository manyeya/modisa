// Rebuild a saved session on server start.
import { panes as treePanes, type Node } from "../../core/layout";
import type { Adapter } from "../../config/adapters";
import type { Session, SpawnOpts } from "../session/session";
import type { Saved } from "./store";
import { quote } from "./template";

// Rebuild the saved layout. Every pane comes back as a shell in its old cwd; agents are resumed in it,
// plain commands are typed but not run (re-running a deploy on reboot would be rude).
export function restore(s: Session, data: Saved, adapters: Adapter[]) {
  const ids = new Map<string, string>();
  const followUps: [string, string, boolean][] = [];
  const opts = (old: string): SpawnOpts => {
    const sp = data.panes[old];
    return sp ? { cwd: sp.cwd, name: sp.name, createdBy: sp.createdBy } : {};
  };
  const after = (old: string, id: string) => {
    ids.set(old, id);
    const sp = data.panes[old];
    if (!sp) return;
    const agent = adapters.find((a) => a.id === (sp.harness ?? sp.agent));
    // the exact session its integration reported, else the agent's "latest session", else a fresh start
    const exact = agent?.resumeSession && sp.session?.agent === agent.id ? agent.resumeSession.replace("{id}", quote(sp.session.id)) : undefined;
    if (agent) followUps.push([id, exact ?? agent.resume ?? agent.launch, true]);
    else if (sp.command) followUps.push([id, sp.command, false]);
  };
  // Replay a tree: first leaf opens the tab, every split re-creates its right/bottom subtree.
  const build = (node: Node, at: string) => {
    if ("pane" in node) return;
    const firstB = treePanes(node.b)[0]!;
    const p = s.split(node.dir, opts(firstB), at, false)!;
    after(firstB, p.id);
    build(node.a, at);
    build(node.b, p.id);
  };
  const workspaces = data.workspaces.filter((w) => w.tabs.length > 0);
  const selected = data.workspaces[data.active];
  workspaces.forEach((w) => {
    w.tabs.forEach((t, ti) => {
      const first = treePanes(t.tree)[0]!;
      const p = ti === 0 ? s.newWorkspace(w.name, w.cwd, opts(first)) : s.newTab(t.name, opts(first));
      after(first, p.id);
      build(t.tree, p.id);
      const tab = s.tab;
      tab.name = t.name;
      restoreRatios(tab.tree, t.tree);
      tab.focused = ids.get(t.focused) ?? tab.focused;
      tab.zoomed = t.zoomed;
    });
    s.ws.active = Math.max(0, Math.min(w.active, s.ws.tabs.length - 1));
  });
  s.active = Math.max(0, workspaces.indexOf(selected!));
  s.layout();
  setTimeout(() => {
    for (const [id, cmd, run] of followUps) s.panes.get(id)?.write(cmd + (run ? "\r" : ""));
  }, 300); // let the shells print their prompts first
}

function restoreRatios(live: Node, saved: Node) {
  if ("pane" in live || "pane" in saved) return;
  live.ratio = saved.ratio;
  restoreRatios(live.a, saved.a);
  restoreRatios(live.b, saved.b);
}
