// The sidebar: spaces (switch, rename in place, delete), agents by what needs you first, shortcuts.
// Three looks that never get confused: labels are dim text that ignores the mouse; clickable rows lift
// to a neutral tint under the pointer; the current space and the focused agent sit on a focus tint in
// bold. Actions stay dim until hovered, so color at rest means state, not "click me".
import { BoxRenderable, InputRenderable, InputRenderableEvents, TextAttributes, TextRenderable, bold, fg, t, type MouseEvent } from "@opentui/core";
import type { App } from "../context";
import { fit, mix } from "../design";
import { render } from "../render";
import { deleteSpace, spaceMenu } from "../spaces";

type RowOptions = { selected?: boolean; height?: number; run: () => any; context?: (e: MouseEvent) => void; hover?: (on: boolean) => void };

const selectedBg = (app: App) => mix(app.th.bar, app.th.focus, 0.25);

// A clickable row with a body the caller fills. Hover tints the
// whole row whichever child the pointer is on, since mouse events bubble up to it.
function row(app: App, parent: BoxRenderable, o: RowOptions) {
  const { r, th } = app;
  const height = o.height ?? 1;
  const rest = o.selected ? selectedBg(app) : th.bar;
  const box: BoxRenderable = new BoxRenderable(r, {
    width: app.sideWidth(), height, flexDirection: "row", flexShrink: 0, paddingLeft: 1, backgroundColor: rest,
    onMouseDown: (e) => {
      e.stopPropagation();
      if (app.modal) return;
      if (e.button === 2) o.context?.(e);
      else if (e.button === 0) o.run();
    },
    onMouseOver: () => { box.backgroundColor = mix(rest, th.fg, 0.12); o.hover?.(true); },
    onMouseOut: () => { box.backgroundColor = rest; o.hover?.(false); },
  });
  const body = new BoxRenderable(r, { flexGrow: 1, height, flexDirection: "row" });
  box.add(body);
  parent.add(box);
  app.clickable.add(box);
  return body;
}

// A row that does something ("+ new space", shortcuts): dim, accent under the pointer.
function action(app: App, parent: BoxRenderable, text: string, run: () => any) {
  const label = new TextRenderable(app.r, { content: fit(` ${text}`, app.sideWidth() - 1), height: 1, fg: app.th.dim });
  row(app, parent, { run, hover: (on) => { label.fg = on ? app.th.accent : app.th.dim; } }).add(label);
}

export function drawSidebar(app: App) {
  const { r, th, ui: { side } } = app;
  for (const c of side.getChildren()) c.destroyRecursively();
  const w = app.sideWidth();
  side.visible = w > 0;
  if (!w) return;
  const height = app.area().h;
  Object.assign(side, { top: app.metrics().top, width: w, height, backgroundColor: th.bar, paddingLeft: 0, paddingRight: 0, overflow: "hidden" });
  let used = 0;
  const room = (lines = 1) => used + lines <= height - 4 && (used += lines) > 0;
  const text = (s: string, attributes = 0) => room() && side.add(new TextRenderable(r, { content: fit(`  ${s}`, w), height: 1, flexShrink: 0, fg: th.dim, attributes }));
  const label = (s: string) => text(s, TextAttributes.BOLD);
  label("");
  label("SPACES");
  const view = app.view!;
  const spaceLimit = Math.min(view.workspaces.length, Math.max(2, Math.floor(height / 5)));
  const start = Math.max(0, Math.min(view.active - 1, view.workspaces.length - spaceLimit));
  view.workspaces.slice(start, start + spaceLimit).forEach((_x, j) => room() && spaceRow(app, start + j));
  if (room()) action(app, side, "+ new space", () => app.actions["new-workspace"]!.run());
  label("");
  const agents = app.sortedAgents();
  label(`AGENTS / ${agents.length}`);
  if (!agents.length) {
    text("No agents in this space");
    if (room()) action(app, side, "+ launch an agent", () => app.actions["new-agent"]!.run());
  }
  const budget = Math.max(0, Math.floor((height - used - 7) / 2));
  const focused = app.tab().focused;
  for (const p of agents.slice(0, budget)) {
    if (!room(2)) break;
    const st = p.agent!.state, selected = p.id === focused;
    const name = fit(p.name ? "@" + p.name : p.title, w - 4);
    row(app, side, { height: 2, selected, run: () => app.call("focusPane", { pane: p.id }) }).add(new TextRenderable(r, {
      content: t` ${app.cfg.indicators.sidebar ? fg(th[st])(app.icon(st) + " ") : ""}${selected ? bold(name) : name}\n   ${fg(th.dim)(fit(`${p.agent!.harness} · ${st}`, w - 4))}`,
      height: 2, flexGrow: 1, fg: th.fg,
    }));
  }
  if (agents.length > budget && room()) action(app, side, `+ ${agents.length - budget} more · all panes`, () => app.actions["pane-picker"]!.run());
  // shortcuts pinned to the bottom
  const footer = new BoxRenderable(r, { position: "absolute", left: 0, bottom: 1, width: w, height: 3 });
  side.add(footer);
  action(app, footer, ": command palette", () => app.actions.palette!.run());
  action(app, footer, "? keyboard guide", () => app.actions.help!.run());
  action(app, footer, "⚙ settings", () => app.actions.settings!.run());
}

// ---------- spaces ----------
// Each row: the name (click = switch, double-click = rename in place, right-click = menu), with ✎ / ✕
// shown while the pointer is on the row. Renaming swaps the row for an OpenTUI input: Enter saves,
// Esc or clicking away cancels.
function spaceRow(app: App, i: number) {
  const { r, th } = app;
  const space = app.view!.workspaces[i]!;
  const active = i === app.view!.active;
  const width = app.sideWidth();
  if (app.editing?.index === i) {
    const box = new BoxRenderable(r, { width, height: 1, flexDirection: "row", flexShrink: 0, paddingLeft: 1, backgroundColor: selectedBg(app) });
    const input = new InputRenderable(r, {
      value: app.editing.draft, flexGrow: 1,
      textColor: th.fg, backgroundColor: th.bg, focusedBackgroundColor: th.bg, focusedTextColor: th.fg,
    });
    input.on(InputRenderableEvents.INPUT, () => { if (app.editing) app.editing.draft = input.value; });
    input.on(InputRenderableEvents.ENTER, () => finishRename(app, true));
    // clicking away cancels; a redraw that replaces this input (the draft is kept) doesn't
    input.on("blurred", () => queueMicrotask(() => !input.isDestroyed && app.editing?.index === i && finishRename(app, false)));
    box.add(input);
    box.add(new TextRenderable(r, { content: " ↵", width: 2, flexShrink: 0, fg: th.dim })); // Enter saves
    app.ui.side.add(box);
    queueMicrotask(() => input.focus());
    return;
  }
  const icons: [TextRenderable, string][] = [];
  const body = row(app, app.ui.side, {
    selected: active,
    run: () => {
      const double = app.lastSpaceClick.index === i && Date.now() - app.lastSpaceClick.at < 450;
      app.lastSpaceClick = { index: i, at: Date.now() };
      if (double) startRename(app, i);
      else if (!active) app.call("selectWorkspace", { index: i });
    },
    context: (e) => spaceMenu(app, i, e.x, e.y),
    hover: (on) => { for (const [node, glyph] of icons) node.content = on ? glyph : "  "; },
  });
  body.add(new TextRenderable(r, {
    content: fit(` ${space.name}`, Math.max(1, width - 5)), flexGrow: 1, height: 1,
    fg: th.fg, attributes: active ? TextAttributes.BOLD : 0,
  }));
  const icon = (glyph: string, color: string, run: () => void) => {
    const node: TextRenderable = new TextRenderable(r, {
      content: "  ", width: 2, height: 1, flexShrink: 0, fg: th.dim,
      onMouseDown: (e) => { e.stopPropagation(); if (!app.modal && e.button === 0) run(); },
      onMouseOver: () => { node.fg = color; },
      onMouseOut: () => { node.fg = th.dim; },
    });
    icons.push([node, glyph]);
    body.add(node);
  };
  icon("✎", th.accent, () => startRename(app, i));
  icon("✕", th.blocked, () => deleteSpace(app, i));
}

export function startRename(app: App, index: number) {
  const space = app.view?.workspaces[index];
  if (!space || app.modal) return;
  app.editing = { index, draft: space.name };
  app.chromeSig = ""; // force the sidebar to redraw with the input
  render(app);
}

export function finishRename(app: App, save: boolean) {
  const edit = app.editing;
  if (!edit) return;
  app.editing = undefined;
  app.chromeSig = "";
  if (save && edit.draft.trim() && edit.draft !== app.view?.workspaces[edit.index]?.name) app.call("renameWorkspace", { index: edit.index, name: edit.draft });
  render(app);
}
