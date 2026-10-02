// Session model: workspaces → tabs → split trees of panes. Owns layout and PTY sizes.
import { split, remove, rects, displayRects, neighbor, resize, panes, leaf, dividerAt, dragTo, type Node, type Rect, type Dir } from "../../core/layout";
import { findPane, type GitView, type View } from "../../protocol/types";
import { PtyPane } from "./pane";
import { cwd as here } from "../../core/paths";

export type Tab = { id: string; name?: string; tree: Node; focused: string; zoomed: boolean };
export type Workspace = { id: string; name: string; cwd: string; tabs: Tab[]; active: number; git?: GitView }; // git: see ../git.ts
export type SpawnOpts = { cwd?: string; command?: string; harness?: string; name?: string; createdBy?: string; ephemeral?: boolean; env?: Record<string, string> };
// Where a moved pane goes: beside a pane (right of or below it, with `share` of the space), alone in a new tab of a
// space, or alone in a new space.
export type MoveTo =
  | { beside: string; dir: "row" | "col"; share: number }
  | { newTab: Workspace; name?: string }
  | { newSpace: { name?: string; cwd: string } };

export class Session {
  workspaces: Workspace[] = [];
  active = 0;
  panes = new Map<string, PtyPane>();
  // panes held at a size of their own, not their box's: one taken over from another terminal (attach.ts)
  sizeLocks = new Map<string, { cols: number; rows: number }>();
  area: Rect = { x: 0, y: 1, w: 120, h: 38 };
  private seq = 0;
  private paneSeq = 0;
  private drags = new Map<unknown, ReturnType<typeof dividerAt>>();

  constructor(
    private hooks: {
      output: (p: PtyPane, bytes: Uint8Array) => void;
      exited: (p: PtyPane) => void;
      changed: () => void;
      empty: () => void;
      created: (p: PtyPane) => void;
      closing?: (id: string, focused: boolean) => void; // focused: it's the focused pane of the tab on screen
    },
  ) {}

  // ---------- lookup ----------

  get ws() {
    return this.workspaces[this.active]!;
  }
  get tab() {
    return this.ws.tabs[this.ws.active]!;
  }
  get focusedId() {
    return this.workspaces.length ? this.tab.focused : undefined;
  }

  locate(id: string): { ws: Workspace; tab: Tab } | undefined {
    for (const ws of this.workspaces) for (const tab of ws.tabs) if (panes(tab.tree).includes(id)) return { ws, tab };
  }

  // A target: see findPane.
  resolve(target?: string, fallback?: string): PtyPane | undefined {
    const t = target ?? fallback;
    const info = t ? findPane([...this.panes.values()].map((p) => p.info), t) : undefined;
    return info && this.panes.get(info.id);
  }

  // Where a pane sits in the layout; a popup has no place.
  placeOf(id: string): { ws: Workspace; tab: Tab } {
    const loc = this.locate(id);
    if (!loc) throw new Error(`${id} is a popup: it has no place in a tab`);
    return loc;
  }

  // The pane on that side of `id`, in its tab.
  neighborOf(id: string, dir: Dir): string | undefined {
    const loc = this.locate(id);
    return loc && neighbor(rects(loc.tab.tree, this.area), id, dir);
  }

  isVisible(id: string) {
    const loc = this.locate(id);
    return !!loc && loc.ws === this.ws && loc.tab === this.tab && displayRects(loc.tab.tree, this.area, loc.tab.focused, loc.tab.zoomed).has(id);
  }

  // ---------- panes ----------

  private spawn(o: SpawnOpts, cwd: string): PtyPane {
    const id = `p${++this.paneSeq}`;
    const p = new PtyPane(
      { id, cwd: o.cwd ?? cwd, command: o.command, harness: o.harness, name: o.name, createdBy: o.createdBy ?? "user", cols: this.area.w - 2, rows: this.area.h - 2, env: o.env },
      {
        output: this.hooks.output,
        title: () => this.hooks.changed(),
        exit: (p) => {
          // shells close their pane; command/agent panes stay so their output and exit code can be read
          if ((!p.info.command || o.ephemeral) && this.panes.has(p.id)) this.locate(p.id) ? this.close(p.id) : this.dropHidden(p.id);
          else this.hooks.changed();
          this.hooks.exited(p);
        },
      },
    );
    this.panes.set(id, p);
    this.hooks.created(p);
    return p;
  }

  // A pane with no place in any tab (a plugin's popup). Every client gets its output; only the one that opened it
  // shows it. Removed when its process exits, or by dropHidden.
  spawnHidden(o: SpawnOpts): PtyPane {
    const p = this.spawn({ ...o, ephemeral: true }, o.cwd ?? here());
    p.info.popup = true;
    this.hooks.changed();
    return p;
  }
  dropHidden(id: string) {
    const p = this.panes.get(id);
    if (!p || this.locate(id)) return;
    this.hooks.closing?.(id, false); // closing all the same, to whoever watches it
    this.panes.delete(id);
    p.dispose();
    this.hooks.changed();
  }

  private addWorkspace(name: string | undefined, cwd: string): Workspace {
    const ws: Workspace = { id: `w${++this.seq}`, name: name ?? cwd.split("/").pop() ?? "workspace", cwd, tabs: [], active: 0 };
    this.workspaces.push(ws);
    return ws;
  }

  newWorkspace(name?: string, cwd = here(), o: SpawnOpts = {}) {
    const previous = this.active;
    const ws = this.addWorkspace(name, cwd);
    this.active = this.workspaces.length - 1;
    try { return this.newTab(undefined, o, ws); }
    catch (error) {
      this.workspaces.splice(this.workspaces.indexOf(ws), 1);
      this.active = previous;
      throw error;
    }
  }

  newTab(name?: string, o: SpawnOpts = {}, ws = this.ws): PtyPane {
    const p = this.spawn(o, ws.cwd);
    this.addTab(ws, p.id, name, true);
    this.layout();
    return p;
  }

  // A tab holding one pane that already exists; select: show it.
  private addTab(ws: Workspace, paneId: string, name: string | undefined, select: boolean): Tab {
    const tab: Tab = { id: `t${++this.seq}`, name, tree: { pane: paneId }, focused: paneId, zoomed: false };
    ws.tabs.push(tab);
    if (select) {
      ws.active = ws.tabs.length - 1;
      this.active = this.workspaces.indexOf(ws);
    }
    return tab;
  }

  // share: the new pane's part of the target's room
  split(dir: "row" | "col", o: SpawnOpts = {}, targetId = this.focusedId, focus = true, share = 0.5): PtyPane | undefined {
    const loc = targetId && this.locate(targetId);
    if (!loc) return;
    const cwd = o.cwd ?? this.panes.get(targetId!)?.info.cwd ?? loc.ws.cwd;
    const p = this.spawn({ ...o, cwd }, cwd);
    loc.tab.tree = split(loc.tab.tree, targetId!, dir, p.id, 1 - share);
    loc.tab.zoomed = false;
    if (focus) loc.tab.focused = p.id;
    this.layout();
    return p;
  }

  close(id = this.focusedId) {
    const p = id && this.panes.get(id);
    const loc = id && this.locate(id);
    if (!p || !loc) return;
    this.hooks.closing?.(id, loc.tab.focused === id && loc.tab === this.tab);
    this.panes.delete(id);
    p.dispose();
    this.unlink(id);
    if (!this.workspaces.length) return this.hooks.empty();
    this.layout();
  }

  // Take a pane out of its tab, which focuses its nearest neighbour; a tab left empty goes, then a space left empty.
  // The pane itself is untouched: close disposes of it, move puts it somewhere else.
  private unlink(id: string) {
    const { ws, tab } = this.locate(id)!;
    const rs = rects(tab.tree, this.area);
    const next = neighbor(rs, id, "left") ?? neighbor(rs, id, "up") ?? neighbor(rs, id, "right") ?? neighbor(rs, id, "down");
    const tree = remove(tab.tree, id);
    if (tree) {
      tab.tree = tree;
      if (tab.focused === id) tab.focused = next ?? panes(tree)[0]!;
      tab.zoomed = false;
      return;
    }
    const ti = ws.tabs.indexOf(tab);
    ws.tabs.splice(ti, 1);
    if (ws.active >= ti) ws.active = Math.max(0, ws.active - 1);
    if (ws.tabs.length) return;
    const wi = this.workspaces.indexOf(ws);
    this.workspaces.splice(wi, 1);
    if (this.active >= wi) this.active = Math.max(0, this.active - 1);
  }

  // Move a pane, process and all. Refused, before anything changes, where it would go nowhere. What it leaves empty
  // closes, the tab it lands in is unzoomed, and the view stays where it is unless `focus`.
  move(id: string, to: MoveTo, focus = false): { ws: Workspace; tab: Tab } {
    const from = this.placeOf(id);
    const alone = panes(from.tab.tree).length === 1;
    if ("beside" in to && to.beside === id) throw new Error(`can't move ${id} beside itself`);
    if ("beside" in to) this.placeOf(to.beside);
    if ("newTab" in to && alone && to.newTab === from.ws) throw new Error(`${id} is already alone in its tab`);
    if ("newSpace" in to && alone && from.ws.tabs.length === 1) throw new Error(`${id} is already alone in its space`);
    this.unlink(id);
    let dest: { ws: Workspace; tab: Tab };
    if ("beside" in to) {
      dest = this.locate(to.beside)!;
      dest.tab.tree = split(dest.tab.tree, to.beside, to.dir, id, 1 - to.share);
      dest.tab.zoomed = false;
    } else {
      const ws = "newTab" in to ? to.newTab : this.addWorkspace(to.newSpace.name, to.newSpace.cwd);
      dest = { ws, tab: this.addTab(ws, id, "newTab" in to ? to.name : undefined, false) };
    }
    if (focus) this.focusPane(id);
    else this.layout();
    return dest;
  }

  // Two panes trade places, in one tab or across tabs; the tree keeps its shape and ratios. Each tab keeps focus on its
  // pane if it's still there, and otherwise gives it to the pane that took its place.
  swap(a: string, b: string) {
    const la = this.placeOf(a), lb = this.placeOf(b);
    if (a === b) throw new Error(`can't swap ${a} with itself`);
    const na = leaf(la.tab.tree, a)!, nb = leaf(lb.tab.tree, b)!;
    na.pane = b;
    nb.pane = a;
    if (la.tab !== lb.tab) {
      if (la.tab.focused === a) la.tab.focused = b;
      if (lb.tab.focused === b) lb.tab.focused = a;
    }
    this.layout();
  }

  closeTab() {
    for (const id of panes(this.tab.tree)) this.close(id);
  }

  // ---------- focus & navigation ----------

  focusPane(id: string) {
    const loc = this.locate(id);
    if (!loc) return;
    this.active = this.workspaces.indexOf(loc.ws);
    loc.ws.active = loc.ws.tabs.indexOf(loc.tab);
    if (loc.tab.zoomed && loc.tab.focused !== id) loc.tab.zoomed = false;
    loc.tab.focused = id;
    this.layout();
  }

  // Focus the pane on that side of `from`; returns it, or undefined when there's none.
  focusDir(dir: Dir, from = this.focusedId) {
    const next = from && this.neighborOf(from, dir);
    if (next) this.focusPane(next);
    return next;
  }

  // A zoomed tab shows only its focused pane, so zooming a pane focuses it in its tab (the view stays where it is).
  // Returns whether its tab is zoomed now.
  zoom(id = this.focusedId, mode: "on" | "off" | "toggle" = "toggle") {
    const tab = id && this.locate(id)?.tab;
    if (!tab) return false;
    const on = mode === "toggle" ? !(tab.zoomed && tab.focused === id) : mode === "on";
    if (on) tab.focused = id!;
    tab.zoomed = on;
    this.layout();
    return on;
  }

  // Move the divider on the pane's `dir` side; returns whether the pane's size changed (not when there's no divider
  // there, or it's as far as it goes).
  resizePane(dir: Dir, cells = 2, id = this.focusedId) {
    const tab = id && this.locate(id)?.tab;
    if (!tab) return false;
    const was = rects(tab.tree, this.area).get(id!)!;
    if (!resize(tab.tree, this.area, id!, dir, cells)) return false;
    this.layout();
    const now = rects(tab.tree, this.area).get(id!)!;
    return now.w !== was.w || now.h !== was.h;
  }

  dragStart(key: unknown, x: number, y: number) {
    if (displayRects(this.tab.tree, this.area, this.tab.focused, this.tab.zoomed).size < panes(this.tab.tree).length) return false;
    const hit = dividerAt(this.tab.tree, this.area, x, y);
    if (hit) this.drags.set(key, hit);
    return !!hit;
  }
  dragMove(key: unknown, x: number, y: number) {
    const hit = this.drags.get(key);
    if (!hit) return;
    dragTo(hit, x, y);
    this.layout();
  }
  dragEnd(key: unknown) {
    this.drags.delete(key);
  }

  selectTab(i: number) {
    if (i < 0 || i >= this.ws.tabs.length) return;
    this.ws.active = i;
    this.hooks.changed();
  }
  cycleTab(step: number) {
    this.selectTab((this.ws.active + step + this.ws.tabs.length) % this.ws.tabs.length);
  }
  selectWorkspace(i: number) {
    if (i < 0 || i >= this.workspaces.length) return;
    this.active = i;
    this.hooks.changed();
  }
  renameTab(name: string) {
    this.tab.name = name || undefined;
    this.hooks.changed();
  }
  renameWorkspace(name: string, i = this.active) {
    const ws = this.workspaces[i];
    if (ws && name.trim()) ws.name = name.trim();
    this.hooks.changed();
  }
  // Close every pane in a space. The last space can't go: that would end the session.
  closeWorkspace(i = this.active) {
    const ws = this.workspaces[i];
    if (!ws) throw new Error(`no such space: ${i}`);
    if (this.workspaces.length < 2) throw new Error("can't delete the only space");
    for (const tab of [...ws.tabs]) for (const id of panes(tab.tree)) this.close(id);
  }
  // A space by index, id or name.
  findWorkspace(ref: string | number): number {
    const i = typeof ref === "number" ? ref : this.workspaces.findIndex((w) => w.id === ref || w.name === ref);
    if (i < 0 || i >= this.workspaces.length) throw new Error(`no such space: ${ref}`);
    return i;
  }
  // A tab by id, or by name (in the active space first).
  findTab(ref: string): { ws: Workspace; tab: Tab } {
    const all = [this.ws, ...this.workspaces.filter((w) => w !== this.ws)].flatMap((ws) => ws.tabs.map((tab) => ({ ws, tab })));
    const found = all.find((x) => x.tab.id === ref) ?? all.find((x) => x.tab.name === ref);
    if (!found) throw new Error(`no such tab: ${ref}`);
    return found;
  }
  renamePane(id: string, name: string) {
    const p = this.panes.get(id);
    if (!p) return;
    p.info.name = name.replace(/^@/, "") || undefined;
    p.refreshTitle(); // a name cleared gives the title back to what else names it
    this.hooks.changed();
  }

  // ---------- layout ----------

  setArea(a: Rect) {
    this.area = a;
    this.layout();
  }

  // Size every PTY to its box (hidden tabs too, so they're right when shown), or to its lock, then notify.
  layout() {
    for (const ws of this.workspaces)
      for (const tab of ws.tabs) {
        const rs = rects(tab.tree, this.area);
        const displayed = displayRects(tab.tree, this.area, tab.focused, tab.zoomed);
        for (const [id, r] of rs) {
          const full = displayed.get(id) ?? r;
          const lock = this.sizeLocks.get(id);
          this.panes.get(id)?.resize(lock?.cols ?? full.w - 2, lock?.rows ?? full.h - 2);
        }
      }
    this.hooks.changed();
  }

  // What clients render.
  view(): View {
    return {
      active: this.active,
      workspaces: this.workspaces.map((ws) => ({
        id: ws.id,
        name: ws.name,
        cwd: ws.cwd,
        active: ws.active,
        tabs: ws.tabs.map((t) => ({ id: t.id, name: t.name, tree: t.tree, focused: t.focused, zoomed: t.zoomed })),
        ...(ws.git && { git: ws.git }),
      })),
      panes: [...this.panes.values()].map((p) => p.info),
    };
  }

  destroy() {
    for (const p of this.panes.values()) p.dispose();
    this.panes.clear();
  }
}

