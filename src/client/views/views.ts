// Plugins' views in this client, drawn from what the server sends (plugin.view, plugin.view.closed, plugin.blit):
// floating over everything (popup) or over the pane they're from (overlay; a popup while that pane isn't on screen),
// the newest on top. The top one has the keyboard: Tab moves between what's in it, its keys run its plugin's actions,
// Escape (or prefix x) closes it. Everything in one is framed and titled with its plugin's name.
import { BoxRenderable, type KeyEvent } from "@opentui/core";
import type { PluginViewState } from "../../protocol/types";
import type { App } from "../context";
import { fit } from "../design";
import { mount, unmount, type Build, type Focusable, type Mounted } from "./build";

export type OpenView = {
  state: PluginViewState;
  frame: BoxRenderable;
  veil: BoxRenderable;
  root?: Mounted;
  theme?: App["th"]; // the theme it was built in: another one builds it again
  focusables: Focusable[];
  rasters: Map<string, Mounted>;
  focus: number; // into focusables; -1 for none
  focusKey?: string; // what has focus, kept across updates
  focusRev?: number; // the rev whose `focus` was applied: a plugin hands the keyboard over once per update
  armed?: boolean; // the prefix was pressed: x closes
};

const MIN = { w: 20, h: 5 };
const Z = { veil: 90, frame: 91 }; // under dialogs (100) and pane popups (101): a dialog opened over a view is on top
const id = (s: { plugin: string; id: string }) => `${s.plugin}/${s.id}`;
const order = (app: App) => [...app.views.values()]; // in the order they opened: the last is on top

// ---------- what the server sends ----------

export function viewSet(app: App, state: PluginViewState) {
  let v = app.views.get(id(state));
  if (!v) {
    const veil = new BoxRenderable(app.r, { position: "absolute", left: 0, top: 0, width: "100%", height: "100%", zIndex: Z.veil, visible: false });
    veil.onMouseDown = (e) => (e.preventDefault(), e.stopPropagation());
    const frame = new BoxRenderable(app.r, { position: "absolute", zIndex: Z.frame, border: true, borderStyle: "rounded", flexDirection: "column", paddingX: 1 });
    app.r.root.add(veil);
    app.r.root.add(frame);
    v = { state, frame, veil, focusables: [], rasters: new Map(), focus: -1 };
    app.views.set(id(state), v);
  }
  v.state = state;
  build(app, v);
  placeViews(app);
}

export function viewClosed(app: App, d: { plugin: string; id: string }) {
  const v = app.views.get(id(d));
  if (!v) return;
  app.views.delete(id(d));
  if (v.root) unmount(v.root);
  v.frame.destroyRecursively();
  v.veil.destroyRecursively();
  if (app.modal === modalOf.get(v)) app.modal = undefined;
  placeViews(app);
}

export function viewBlit(app: App, d: { plugin: string; view: string; key: string; cells: string }) {
  const v = app.views.get(id({ plugin: d.plugin, id: d.view }));
  if (!v) return;
  v.rasters.get(d.key)?.blit?.(d.cells);
  // kept in the state too: a rebuild (a theme change) draws what's there now
  const swap = (n: any): any => (n.type === "raster" && n.key === d.key ? { ...n, cells: d.cells } : n.children ? { ...n, children: n.children.map(swap) } : n);
  v.state = { ...v.state, root: swap(v.state.root) };
}

// The connection went: what plugins showed goes with it (the next attach sends what's open then).
export function clearViews(app: App) {
  for (const v of order(app)) viewClosed(app, v.state);
}

// ---------- building and placing ----------

function build(app: App, v: OpenView) {
  if (v.theme && v.theme !== app.th && v.root) {
    unmount(v.root); // another theme: every colour is baked into what was built
    v.root = undefined;
  }
  v.theme = app.th;
  const b: Build = { app, fire: (action, params, ui) => fire(app, v, action, params, ui), focusables: [], rasters: new Map(), path: "" };
  const before = v.root;
  v.root = mount(b, v.state.root, v.root, "0");
  if (v.root !== before) {
    if (before && !before.r.isDestroyed) v.frame.remove(before.r);
    v.frame.add(v.root.r);
  }
  v.focusables = b.focusables;
  v.rasters = b.rasters;
  // focus goes where the plugin says, once per update; else it stays on what had it, by key; else the first field,
  // list, button or scroll area
  if (v.state.focus && v.focusRev !== v.state.rev) {
    v.focusRev = v.state.rev;
    v.focusKey = `#${v.state.focus}`;
  }
  const kept = v.focusKey ? v.focusables.findIndex((f) => f.key === v.focusKey) : -1;
  const first = (["field", "list", "button", "scroll"] as const).map((k) => v.focusables.findIndex((f) => f.kind === k)).find((i) => i >= 0) ?? -1;
  setFocus(app, v, kept >= 0 ? kept : first);
}

// i: into focusables, or -1 for nothing
function setFocus(app: App, v: OpenView, i: number) {
  v.focus = i >= 0 && i < v.focusables.length ? i : -1;
  v.focusKey = v.focusables[v.focus]?.key;
  const active = isTop(app, v);
  v.focusables.forEach((f, j) => {
    const on = active && j === v.focus;
    f.mark?.(on);
    if (f.kind === "field" || f.kind === "list") on ? f.r.focus() : f.r.blur();
  });
}

const isTop = (app: App, v: OpenView) => order(app).at(-1) === v && !!app.modal && app.modal === modalOf.get(v);

// cells from a view's size: a number of cells, or a percentage of the terminal
const cells = (size: number | string | undefined, total: number, fallback: number) =>
  typeof size === "number" ? size : typeof size === "string" && size.endsWith("%") ? Math.floor((total * Number(size.slice(0, -1))) / 100) : fallback;

function rectOf(app: App, s: PluginViewState) {
  const W = app.r.width, H = app.r.height;
  if (s.placement === "overlay" && s.from) {
    const p = app.panes.get(s.from.pane);
    if (p && p.box.visible && app.info(s.from.pane)?.instance === s.from.instance) return { x: p.box.x, y: p.box.y, w: p.box.width, h: p.box.height };
  }
  const w = Math.max(1, Math.min(W - 2, Math.max(MIN.w, cells(s.width, W, Math.floor(W * 0.7)))));
  const h = Math.max(1, Math.min(H - 2, Math.max(MIN.h, cells(s.height, H, Math.floor(H * 0.6)))));
  return { x: Math.floor((W - w) / 2), y: Math.floor((H - h) / 3), w, h };
}

// Where every view is, how it's titled, and which one has the keyboard: run with every render.
export function placeViews(app: App) {
  const views = order(app);
  const th = app.th;
  views.forEach((v, i) => {
    if (v.theme !== th) build(app, v);
    const top = i === views.length - 1;
    const rect = rectOf(app, v.state);
    const keys = v.state.keys.filter((k) => k.description).map((k) => `${k.key} ${k.description}`);
    Object.assign(v.frame, {
      left: rect.x, top: rect.y, width: rect.w, height: rect.h, zIndex: Z.frame + i, backgroundColor: th.bg,
      borderColor: top ? th.focus : th.border, titleColor: top ? th.focus : th.dim,
      title: fit(` ${v.state.plugin} · ${v.state.title} `, Math.max(0, rect.w - 4)),
      bottomTitle: fit(` ${[...keys, v.focusables.length > 1 ? "tab moves" : "", "esc closes"].filter(Boolean).join(" · ")} `, Math.max(0, rect.w - 4)),
      bottomTitleAlignment: "right",
    });
    v.veil.visible = top;
  });
  // the top view has the keyboard, unless a dialog is open over it: it gets it back when that closes
  const top = views.at(-1);
  if (top && (!app.modal || viewModals.has(app.modal))) {
    const m = modalOf.get(top) ?? modal(app, top);
    if (app.modal !== m) {
      app.modal = m;
      setFocus(app, top, top.focus);
    }
  } else if (!top && app.modal && viewModals.has(app.modal)) app.modal = undefined;
}

// ---------- the keyboard ----------

const modalOf = new WeakMap<OpenView, NonNullable<App["modal"]>>();
const viewModals = new WeakSet<object>(); // app.modal is one of these while a view has the keyboard

// A key as views name it: "j", "J", "enter", "S-tab", "C-s", "M-x".
export function viewKeyName(k: KeyEvent) {
  const base = k.name === "return" ? "enter" : k.name === " " ? "space" : k.name || k.sequence;
  const letter = base.length === 1 && /[a-z]/.test(base);
  return `${k.ctrl ? "C-" : ""}${k.meta || k.option ? "M-" : ""}${k.shift && !letter ? "S-" : ""}${letter && k.shift ? base.toUpperCase() : base}`;
}

const LIST_KEYS = new Set(["up", "down", "j", "k", "S-up", "S-down", "enter", "left", "right", "[", "]"]);
const SCROLL: Record<string, number> = { up: -1, down: 1, k: -1, j: 1, pageup: -10, pagedown: 10, "C-u": -10, "C-d": 10 };

function modal(app: App, v: OpenView): NonNullable<App["modal"]> {
  const m: NonNullable<App["modal"]> = {
    keepEscape: true,
    close: () => closeByUser(app, v),
    resize: () => placeViews(app),
    keys: (k) => {
      // prefix x closes it, as it closes a popup
      if (k.ctrl && k.name === app.prefix.name && !v.armed) return (v.armed = true);
      const name = viewKeyName(k);
      if (v.armed) {
        v.armed = false;
        if (name === "x") return (closeByUser(app, v), true);
      }
      const f = v.focusables[v.focus];
      const typing = f?.kind === "field" && f.r.focused;
      if (name === "escape") {
        if (typing) return (f.r.blur(), setFocus(app, v, -1), true); // out of the field first; Escape again closes
        return (closeByUser(app, v), true);
      }
      if (name === "tab" || name === "S-tab") {
        const n = v.focusables.length, step = name === "tab" ? 1 : -1;
        if (n) setFocus(app, v, v.focus < 0 ? (step > 0 ? 0 : n - 1) : (v.focus + step + n) % n);
        return true;
      }
      const bound = v.state.keys.find((x) => x.key === name);
      // a field or list gets its keys from here, not from OpenTUI's focus: what it doesn't take, nothing else gets
      const give = () => !!(f!.r as unknown as { handleKeyPress?: (k: KeyEvent) => boolean }).handleKeyPress?.(k) || true;
      if (typing) {
        // a field takes what's typed; only a view key with Ctrl or Alt gets past it
        if (bound && /^[CM]-/.test(name)) return (fire(app, v, bound.action, bound.params, {}), true);
        return give();
      }
      if (f?.kind === "list" && LIST_KEYS.has(name) && !bound) return give(); // the list moves and chooses
      if (f?.keys?.(name)) return true; // an element with keys of its own (a diff's cursor) took it
      if (bound) return (fire(app, v, bound.action, bound.params, {}), true);
      if (f?.kind === "button" && (name === "enter" || name === "space")) return (f.press?.(), true);
      const delta = SCROLL[name];
      if (delta) {
        const target = f?.scroll ? f : v.focusables.find((x) => x.scroll);
        target?.scroll?.(delta);
        return true;
      }
      return true; // nothing else reaches the panes under it
    },
  };
  modalOf.set(v, m);
  viewModals.add(m);
  return m;
}

function closeByUser(app: App, v: OpenView) {
  viewClosed(app, v.state);
  app.conn.request("plugin.view.close", { plugin: v.state.plugin, id: v.state.id }).catch(() => {}); // already gone is fine
}

// An element was used: its plugin's action, with what it holds. Only a failure is shown; what an action does, its
// plugin shows.
function fire(app: App, v: OpenView, action: string | undefined, params: Record<string, unknown> | undefined, ui: { key?: string; value?: string; index?: number }) {
  if (!action) return;
  const { plugin, run, id: view } = v.state;
  app.conn.request("plugin.invoke", { plugin, action, params: params ?? {}, run, ui: { view, ...ui } }).catch((e: { code?: string; message?: string }) => {
    app.toast(`${plugin}: ${action}: ${e.message ?? e}`, e.code === "timeout" ? app.th.warn : app.th.blocked);
  });
}
