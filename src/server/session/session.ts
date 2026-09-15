// Session model: workspaces → tabs → split trees of panes. Owns layout and PTY sizes.
import { split, remove, rects, displayRects, neighbor, resize, panes, dividerAt, dragTo, type Node, type Rect, type Dir } from "../../core/layout";
import type { View } from "../../protocol/types";
import { PtyPane } from "./pane";
import { cwd as here } from "../../core/paths";

export type Tab = { id: string; name?: string; tree: Node; focused: string; zoomed: boolean };
export type Workspace = { id: string; name: string; cwd: string; tabs: Tab[]; active: number };
export type SpawnOpts = { cwd?: string; command?: string; harness?: string; name?: string; createdBy?: string; ephemeral?: boolean; env?: Record<string, string> };

export class Session {
  workspaces: Workspace[] = [];
  active = 0;
  panes = new Map<string, PtyPane>();
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

  // "p3", "@coder", "coder", "@p3" (ids still work once a pane is named), "p3:1a2b3c4d" (only that instance of p3)
  resolve(target?: string, fallback?: string): PtyPane | undefined {
    const t = target ?? fallback;
    if (!t) return;
    const inst = /^(p\d+):(\w+)$/.exec(t);
    if (inst) {
      const p = this.panes.get(inst[1]!);
      return p?.info.instance === inst[2] ? p : undefined;
    }
    const name = t.replace(/^@/, "");
    return this.panes.get(t) ?? [...this.panes.values()].find((p) => p.info.name === name) ?? this.panes.get(name);
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
    this.panes.delete(id);
    p.dispose();
    this.hooks.changed();
  }

  newWorkspace(name?: string, cwd = here(), o: SpawnOpts = {}) {
    const ws: Workspace = { id: `w${++this.seq}`, name: name ?? cwd.split("/").pop() ?? "workspace", cwd, tabs: [], active: 0 };
    const previous = this.active;
    this.workspaces.push(ws);
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
    ws.tabs.push({ id: `t${++this.seq}`, name, tree: { pane: p.id }, focused: p.id, zoomed: false });
    ws.active = ws.tabs.length - 1;
    this.active = this.workspaces.indexOf(ws);
    this.layout();
    return p;
  }

  split(dir: "row" | "col", o: SpawnOpts = {}, targetId = this.focusedId, focus = true): PtyPane | undefined {
    const loc = targetId && this.locate(targetId);
    if (!loc) return;
    const cwd = o.cwd ?? this.panes.get(targetId!)?.info.cwd ?? loc.ws.cwd;
    const p = this.spawn({ ...o, cwd }, cwd);
    loc.tab.tree = split(loc.tab.tree, targetId!, dir, p.id);
    loc.tab.zoomed = false;
    if (focus) loc.tab.focused = p.id;
    this.layout();
    return p;
  }

  close(id = this.focusedId) {
    const p = id && this.panes.get(id);
    const loc = id && this.locate(id);
    if (!p || !loc) return;
    const { ws, tab } = loc;
    const rs = rects(tab.tree, this.area);
    const next = neighbor(rs, id, "left") ?? neighbor(rs, id, "up") ?? neighbor(rs, id, "right") ?? neighbor(rs, id, "down");
    this.panes.delete(id);
    p.dispose();
    const tree = remove(tab.tree, id);
    if (tree) {
      tab.tree = tree;
      if (tab.focused === id) tab.focused = next ?? panes(tree)[0]!;
      tab.zoomed = false;
    } else {
      const ti = ws.tabs.indexOf(tab);
      ws.tabs.splice(ti, 1);
      if (ws.active >= ti) ws.active = Math.max(0, ws.active - 1);
      if (!ws.tabs.length) {
        const wi = this.workspaces.indexOf(ws);
        this.workspaces.splice(wi, 1);
        if (this.active >= wi) this.active = Math.max(0, this.active - 1);
        if (!this.workspaces.length) return this.hooks.empty();
      }
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

  focusDir(dir: Dir) {
    const next = neighbor(rects(this.tab.tree, this.area), this.tab.focused, dir);
    if (next) this.focusPane(next);
  }

  zoom() {
    this.tab.zoomed = !this.tab.zoomed;
    this.layout();
  }

  resizePane(dir: Dir, cells = 2) {
    if (resize(this.tab.tree, this.area, this.tab.focused, dir, cells)) this.layout();
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
  renamePane(id: string, name: string) {
    const p = this.panes.get(id);
    if (!p) return;
    p.info.name = name.replace(/^@/, "") || undefined;
    if (p.info.name) p.info.title = p.info.name;
    this.hooks.changed();
  }

  // ---------- layout ----------

  setArea(a: Rect) {
    this.area = a;
    this.layout();
  }

  // Size every PTY to its box (hidden tabs too, so they're right when shown), then notify.
  layout() {
    for (const ws of this.workspaces)
      for (const tab of ws.tabs) {
        const rs = rects(tab.tree, this.area);
        const displayed = displayRects(tab.tree, this.area, tab.focused, tab.zoomed);
        for (const [id, r] of rs) {
          const full = displayed.get(id) ?? r;
          this.panes.get(id)?.resize(full.w - 2, full.h - 2);
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
      })),
      panes: [...this.panes.values()].map((p) => p.info),
    };
  }

  destroy() {
    for (const p of this.panes.values()) p.dispose();
    this.panes.clear();
  }
}

