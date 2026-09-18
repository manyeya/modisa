// A compact navigator: selection is a slim rail, attention is a warning signal,
// and every list owns a measured amount of space above the pinned shortcuts.
import { BoxRenderable, InputRenderable, InputRenderableEvents, StyledText, TextRenderable, bold, fg, t, type MouseEvent, type TextChunk } from "@opentui/core";
import type { GitView } from "../../protocol/types";
import type { App } from "../context";
import { agentMark, agentTask, fit, mix, sidebarAgents, sidebarBudget, sidebarColumns } from "../design";
import { render } from "../render";
import { deleteSpace, spaceMenu } from "../spaces";
import { pluginUi, runPluginAction, spanText, toneColor } from "../plugin-ui";
import { beginResize } from "../panes/resize";

type RowOptions = { selected?: boolean; height?: number; run: () => any; context?: (e: MouseEvent) => void; hover?: (on: boolean) => void };
const selectedBg = (app: App) => mix(app.th.bar, app.th.focus, 0.12);
const contentWidth = (app: App) => Math.max(0, app.sideWidth() - 4);

function row(app: App, parent: BoxRenderable, o: RowOptions) {
  const { r, th } = app;
  const height = o.height ?? 1;
  const rest = o.selected ? selectedBg(app) : th.bar;
  const box: BoxRenderable = new BoxRenderable(r, {
    width: app.sideWidth() - 1, height, flexDirection: "row", flexShrink: 0, paddingRight: 1, backgroundColor: rest,
    onMouseDown: (e) => {
      e.stopPropagation();
      if (app.modal) return;
      if (e.button === 2) o.context?.(e);
      else if (e.button === 0) o.run();
    },
    onMouseOver: () => { box.backgroundColor = mix(rest, th.fg, 0.07); o.hover?.(true); },
    onMouseOut: () => { box.backgroundColor = rest; o.hover?.(false); },
  });
  box.add(new BoxRenderable(r, { width: 2, height, flexShrink: 0 })); // left gutter; selection reads from the row tint alone
  const body = new BoxRenderable(r, { width: contentWidth(app), height, flexDirection: "row", flexShrink: 0, overflow: "hidden" });
  box.add(body);
  parent.add(box);
  app.clickable.add(box);
  return body;
}

function action(app: App, parent: BoxRenderable, text: string, hint: string, run: () => any) {
  const columns = sidebarColumns(text, hint, contentWidth(app));
  const label = new TextRenderable(app.r, { content: columns.left, width: Bun.stringWidth(columns.left), height: 1, flexShrink: 0, fg: app.th.dim });
  const body = row(app, parent, { run, hover: (on) => { label.fg = on ? app.th.fg : app.th.dim; } });
  body.add(label);
  body.add(new TextRenderable(app.r, { content: columns.right, width: Bun.stringWidth(columns.right), height: 1, fg: app.th.dim }));
}

export function drawSidebar(app: App) {
  const { r, th, ui: { side } } = app;
  for (const c of side.getChildren()) c.destroyRecursively();
  const w = app.sideWidth();
  side.visible = w > 0;
  if (!w) return;
  const height = app.area().h;
  Object.assign(side, { top: app.metrics().top, width: w, height, backgroundColor: th.bar, paddingLeft: 0, paddingRight: 0, overflow: "hidden" });
  // its edge: drag it to make the sidebar wider or narrower
  side.add(new TextRenderable(r, {
    position: "absolute", right: 0, top: 0, width: 1, height, content: Array(height).fill("│").join("\n"), fg: mix(th.bar, th.border, 0.65),
    onMouseDown: (e) => {
      e.stopPropagation();
      if (e.button === 0 && !app.modal) beginResize(app, e.x, e.y, true);
    },
  }));
  const blank = () => side.add(new BoxRenderable(r, { width: w - 1, height: 1, flexShrink: 0 }));
  const heading = (name: string, count: number) => {
    const columns = sidebarColumns(name, String(count), contentWidth(app));
    side.add(new TextRenderable(r, { content: t`  ${bold(columns.left)}${fg(th.dim)(columns.right)}`, width: w - 1, height: 1, flexShrink: 0, fg: th.dim }));
  };
  const view = app.view!;
  const agents = app.sortedAgents();
  const budget = sidebarBudget(height, view.workspaces.length, agents.length);
  blank();
  heading("SPACES", view.workspaces.length);
  const start = Math.max(0, Math.min(view.active - 1, view.workspaces.length - budget.spaceRows));
  view.workspaces.slice(start, start + budget.spaceRows).forEach((_space, index) => spaceRow(app, start + index));
  if (budget.moreSpaces) action(app, side, "More spaces…", "", () => app.actions["workspace-picker"]!.run());
  action(app, side, "New space", "+", () => app.actions["new-workspace"]!.run());
  blank();
  // a plugin the config names in [sidebar] agents takes the AGENTS list's place and its room, while it shows a section;
  // otherwise (not installed, stopped, nothing to show) modisa's own list is there as ever
  const takeover = app.cfg.sidebar.agents ? pluginUi(app).find((p) => p.plugin === app.cfg.sidebar.agents && p.sidebar) : undefined;
  if (takeover) pluginSection(app, takeover, budget.lines);
  else agentList(app, agents, budget);
  for (const plugin of pluginUi(app)) {
    if (!plugin.sidebar || plugin === takeover) continue;
    blank();
    pluginSection(app, plugin, 8);
  }
  const footer = new BoxRenderable(r, { position: "absolute", left: 0, bottom: 1, width: w - 1, height: 4, flexDirection: "column" });
  side.add(footer);
  footer.add(new TextRenderable(r, { content: "  " + "─".repeat(contentWidth(app)), height: 1, width: w - 1, flexShrink: 0, fg: th.border }));
  action(app, footer, "Commands", ":", () => app.actions.palette!.run());
  action(app, footer, "Keyboard guide", "?", () => app.actions.help!.run());
  action(app, footer, "Settings", "⚙", () => app.actions.settings!.run());
}

// modisa's own list of agents: most pressing first, the focused one never hidden behind the overflow row
function agentList(app: App, agents: ReturnType<App["sortedAgents"]>, budget: ReturnType<typeof sidebarBudget>) {
  const { r, th, ui: { side } } = app;
  const w = app.sideWidth();
  const columns = sidebarColumns("AGENTS", String(agents.length), contentWidth(app));
  side.add(new TextRenderable(r, { content: t`  ${bold(columns.left)}${fg(th.dim)(columns.right)}`, width: w - 1, height: 1, flexShrink: 0, fg: th.dim }));
  side.add(new BoxRenderable(r, { width: w - 1, height: 1, flexShrink: 0 }));
  const focused = app.tab().focused;
  const visible = sidebarAgents(agents, focused, budget.agentRows);
  const labels = { blocked: "Needs you", working: "Working", done: "Done", idle: "Idle" };
  for (const pane of visible) {
    const { harness, state } = pane.agent!;
    const selected = pane.id === focused;
    const color = state === "blocked" ? th.warn : state === "working" ? th.focus : th.dim;
    // the agent's mark, then its name (its tool when unnamed); under it, the task its terminal title names
    const mark = agentMark(th, harness, app.logos);
    const width = contentWidth(app) - mark.cells;
    const name = sidebarColumns(pane.name ? "@" + pane.name : harness, app.cfg.indicators.sidebar ? app.icon(state) : "", width);
    const task = agentTask(pane.terminalTitle ?? pane.title, pane.name, harness);
    const meta = sidebarColumns(task || (pane.name ? harness : ""), labels[state], width);
    const body = row(app, side, { height: 2, selected, run: () => app.call("focusPane", { pane: pane.id }) });
    const gap = " ".repeat(mark.cells - 1), indent = " ".repeat(mark.cells); // the task lines up under the name
    body.add(new TextRenderable(r, {
      content: t`${fg(mark.color)(mark.glyph)}${gap}${selected ? bold(name.left) : name.left}${fg(color)(name.right)}\n${indent}${fg(th.dim)(meta.left)}${fg(color)(meta.right)}`,
      width: contentWidth(app), height: 2, flexShrink: 0, fg: state === "done" || state === "idle" ? th.dim : th.fg,
    }));
  }
  if (!agents.length) {
    side.add(new TextRenderable(r, { content: "  " + fit("No agents here", contentWidth(app)), width: w - 1, height: 1, flexShrink: 0, fg: th.dim }));
    action(app, side, "Launch an agent", "+", () => app.actions["new-agent"]!.run());
  }
  if (budget.moreAgents) action(app, side, `${agents.length - visible.length} more agents`, "›", () => app.actions["pane-picker"]!.run());
}

// A plugin's section, at most `rows` of its rows: click its heading to fold it, a row to run its action or focus its pane.
// It's headed by the plugin's name on its own row, so it can't pass for one of modisa's however narrow the sidebar; the
// title the plugin chose goes under it.
function pluginSection(app: App, plugin: ReturnType<typeof pluginUi>[number], rows: number) {
  const { r, th, ui: { side } } = app;
  const section = plugin.sidebar!;
  const folded = app.collapsedPlugins.has(plugin.plugin);
  const head = row(app, side, {
    run: () => {
      if (folded) app.collapsedPlugins.delete(plugin.plugin);
      else app.collapsedPlugins.add(plugin.plugin);
      app.chromeSig = "";
      render(app);
    },
  });
  // the count is of rows that do something (focus a pane, run an action), not a plugin's headers and spacing
  const items = section.rows.filter((x) => x.pane || x.action).length;
  const columns = sidebarColumns(`${folded ? "▸" : "▾"} ${plugin.plugin}`, String(items || section.rows.length), contentWidth(app));
  head.add(new TextRenderable(r, { content: t`${bold(columns.left)}${fg(th.dim)(columns.right)}`, width: contentWidth(app), height: 1, flexShrink: 0, fg: th.dim }));
  if (folded) return;
  side.add(new TextRenderable(r, { content: "  " + fit(section.title, contentWidth(app)), width: app.sideWidth() - 1, height: 1, flexShrink: 0, fg: th.dim }));
  for (const item of section.rows.slice(0, rows)) {
    // focus the process the row was set for (the server checks the instance), never another pane given its id
    const focus = () => app.conn.request("pane.focus", { target: `${item.pane}:${item.instance}` }).catch(() => app.toast(`${plugin.plugin}: that pane is gone`, th.warn));
    const body = row(app, side, { run: () => (item.pane ? focus() : item.action && runPluginAction(app, plugin, item.action)) });
    const content = item.spans?.length ? spanText(app, item.spans, item.tone, contentWidth(app)) : fit(item.text, contentWidth(app));
    body.add(new TextRenderable(r, { content, width: contentWidth(app), height: 1, flexShrink: 0, fg: toneColor(app, item.tone) }));
  }
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
    const box = new BoxRenderable(r, { width: width - 1, height: 1, flexDirection: "row", flexShrink: 0, paddingLeft: 2, paddingRight: 1, backgroundColor: selectedBg(app) });
    const input = new InputRenderable(r, {
      value: app.editing.draft, flexGrow: 1, minWidth: 0,
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
  const lineWidth = Math.max(1, contentWidth(app) - 4); // the last 4 are ✎ and ✕, on hover
  // git stops a cell short of them, so ✎ never runs into a count
  body.add(new TextRenderable(r, { content: spaceLine(app, space.name, app.cfg.sidebar.git ? space.git : undefined, active, Math.max(1, lineWidth - 1)), width: lineWidth, flexShrink: 0, height: 1, fg: th.fg }));
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
  icon("✎", th.focus, () => startRename(app, i));
  icon("✕", th.blocked, () => deleteSpace(app, i));
}

// A space's row: its name, and on the right where its focused pane's repository stands: the branch (in the done colour
// when clean and in step with its upstream), ↑ commits to push, ↓ commits to pull, ● files changed. The repository is
// named too when the space isn't, room permitting. Git takes what the name doesn't need, and at least 60% of the row.
function spaceLine(app: App, name: string, git: GitView | undefined, active: boolean, width: number): StyledText {
  const { th } = app;
  const right: TextChunk[] = [];
  let used = 0;
  if (git) {
    const room = Math.max(Math.floor(width * 0.6), width - Bun.stringWidth(name) - 1); // a short name leaves it more
    const counts: [string, string][] = [];
    if (git.ahead) counts.push([`↑${git.ahead}`, th.working]);
    if (git.behind) counts.push([`↓${git.behind}`, th.warn]);
    if (git.changes) counts.push([`●${git.changes}`, th.accent]);
    const tail = counts.reduce((n, [s]) => n + 1 + Bun.stringWidth(s), 0);
    const clean = !git.changes && !git.ahead && !git.behind && git.ahead !== undefined;
    const branchRoom = room - tail - 2; // "⎇ "
    if (branchRoom >= 3) {
      const branch = fit(git.branch, branchRoom);
      const repo = git.repo !== name && branchRoom - Bun.stringWidth(branch) >= Bun.stringWidth(git.repo) + 1 ? `${git.repo} ` : "";
      right.push(fg(th.dim)(repo), fg(clean ? th.done : th.dim)(`⎇ ${branch}`));
      used += Bun.stringWidth(repo) + 2 + Bun.stringWidth(branch);
    }
    for (const [s, color] of counts) if (used + 1 + Bun.stringWidth(s) <= room) (right.push(fg(th.dim)(" "), fg(color)(s)), used += 1 + Bun.stringWidth(s));
  }
  const left = fit(name, Math.max(1, width - used - (used ? 1 : 0)));
  const title = fg(th.fg)(left);
  const gap = " ".repeat(Math.max(0, width - Bun.stringWidth(left) - used));
  return new StyledText([active ? bold(title) : title, fg(th.dim)(gap), ...right]);
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
