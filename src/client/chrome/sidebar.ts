// A compact navigator: selection is a slim rail, attention is a warning signal,
// and every list owns a measured amount of space above the pinned shortcuts.
import { BoxRenderable, StyledText, TextRenderable, bold, fg, t, type MouseEvent } from "@opentui/core";
import type { App } from "../context";
import { panes as treePanes } from "../../core/layout";
import type { AgentState } from "../../protocol/types";
import { agentGraph, agentMark, agentTask, fit, mix, sidebarBudget, sidebarColumns } from "../design";
import { contextMenu } from "../modals/context-menu";
import { render } from "../render";
import { tabLabel } from "./tabs";
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
  const agents = app.sortedAgents();
  const budget = sidebarBudget(height, agents.length);
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

// modisa's own list of agents, as a git graph of the space's tabs: each tab a node on one trunk, in its own lane colour,
// its agents branching off under it (most pressing first). Click a tab to fold it: its row then counts its agents by
// state. An agent needing you lights its branch.
function agentList(app: App, agents: ReturnType<App["sortedAgents"]>, budget: ReturnType<typeof sidebarBudget>) {
  const { r, th, ui: { side } } = app;
  const w = app.sideWidth();
  const cw = contentWidth(app);
  const columns = sidebarColumns("AGENTS", String(agents.length), cw);
  side.add(new TextRenderable(r, { content: t`  ${bold(columns.left)}${fg(th.dim)(columns.right)}`, width: w - 1, height: 1, flexShrink: 0, fg: th.dim }));
  side.add(new BoxRenderable(r, { width: w - 1, height: 1, flexShrink: 0 }));
  if (!agents.length) {
    side.add(new TextRenderable(r, { content: "  " + fit("No agents here", cw), width: w - 1, height: 1, flexShrink: 0, fg: th.dim }));
    action(app, side, "Launch an agent", "+", () => app.actions["new-agent"]!.run());
    return;
  }
  const ws = app.ws();
  const tabs = ws.tabs.map((tab) => {
    const mine = new Set(treePanes(tab.tree));
    return { id: tab.id, tab, agents: agents.filter((p) => mine.has(p.id)) };
  });
  const graph = agentGraph(tabs, ws.active, app.collapsedTabs, budget.lines);
  const focused = app.tab().focused;
  const trunk = mix(th.border, th.dim, 0.35);
  // a lane per tab, git-graph style; the active tab's at full strength, the others' quieter
  const lanes = [th.focus, th.accent, th.done, th.working];
  const lane = (i: number) => (i === ws.active ? lanes[i % lanes.length]! : mix(lanes[i % lanes.length]!, th.bar, 0.4));
  const labels = { blocked: "Needs you", working: "Working", done: "Done", idle: "Idle" };
  const stateColor = (s: AgentState) => (s === "blocked" ? th.warn : s === "working" ? th.focus : th.dim);

  const lines = app.cfg.sidebar.graph;
  for (const g of graph.rows) {
    if (g.kind === "rail") {
      if (!lines) continue;
      side.add(new TextRenderable(r, { content: t`  ${fg(trunk)("│")}`, width: w - 1, height: 1, flexShrink: 0 }));
      continue;
    }
    const { tab, agents: tabAgents } = tabs[g.tab]!;
    if (g.kind === "tab") {
      const on = g.tab === ws.active;
      const toggle = () => {
        if (app.collapsedTabs.has(tab.id)) app.collapsedTabs.delete(tab.id);
        else app.collapsedTabs.add(tab.id);
        app.chromeSig = "";
        render(app);
      };
      const body = row(app, side, { run: toggle, context: (e) => contextMenu(app, tab.focused, e.x, e.y) });
      // on the right: open, how many agents and ▾; folded, a count per state, what needs you first, and ▸
      const counts = (["blocked", "working", "done", "idle"] as const).map((s) => [s, tabAgents.filter((p) => p.agent!.state === s).length] as const).filter(([, n]) => n);
      const right = !tabAgents.length ? [] : g.open ? [fg(th.dim)(`${tabAgents.length} ▾`)] : [...counts.flatMap(([s, n]) => [fg(stateColor(s))(`${app.icon(s)}${n}`), fg(th.dim)("  ")]), fg(th.dim)("▸")];
      const rightWidth = !tabAgents.length ? 0 : g.open ? Bun.stringWidth(`${tabAgents.length} ▾`) : counts.reduce((n, [s, c]) => n + Bun.stringWidth(`${app.icon(s)}${c}  `), 1);
      const number = `${g.tab + 1} `;
      const name = fit(tabLabel(app, tab), Math.max(1, cw - 2 - number.length - rightWidth - 1));
      const needsYou = !g.open && counts.some(([s]) => s === "blocked");
      const nameColor = needsYou ? th.warn : !tabAgents.length ? th.dim : th.fg;
      const gapWidth = Math.max(1, cw - 2 - number.length - Bun.stringWidth(name) - rightWidth);
      const content = new StyledText([fg(lane(g.tab))(g.node), fg(th.dim)(` ${number}`), fg(nameColor)(on ? bold(name) : name), fg(th.fg)(" ".repeat(gapWidth)), ...right]);
      body.add(new TextRenderable(r, { content, width: cw, height: 1, flexShrink: 0, fg: th.fg }));
      continue;
    }
    // an agent: its branch off the trunk, its mark, its name and state; under them the task its terminal title names
    const pane = g.agent;
    const { harness, state } = pane.agent!;
    const selected = pane.id === focused;
    const color = stateColor(state);
    const branch = state === "blocked" ? th.warn : lane(g.tab); // what needs you lights up its branch
    const mark = agentMark(th, harness, app.logos, app.cellEms());
    const width = cw - 2 - mark.cells;
    const name = sidebarColumns(pane.name ? "@" + pane.name : harness, app.cfg.indicators.sidebar ? app.icon(state) : "", width);
    const task = agentTask(pane.terminalTitle ?? pane.title, pane.name, harness);
    const meta = sidebarColumns(task || (pane.name ? harness : ""), labels[state], width);
    const body = row(app, side, { height: 2, selected, run: () => app.call("focusPane", { pane: pane.id }) });
    const [top, bottom] = mark.halves ?? [mark.glyph, " "];
    const gap = " ".repeat(mark.cells - 1);
    const [g1, g2] = lines ? g.graph : ["  ", "  "];
    body.add(new TextRenderable(r, {
      content: t`${fg(trunk)(g1[0]!)}${fg(branch)(g1[1]!)}${fg(mark.color)(top)}${gap}${selected ? bold(name.left) : name.left}${fg(color)(name.right)}\n${fg(trunk)(g2)}${fg(mark.color)(bottom)}${gap}${fg(th.dim)(meta.left)}${fg(color)(meta.right)}`,
      width: cw, height: 2, flexShrink: 0, fg: state === "done" || state === "idle" ? th.dim : th.fg,
    }));
  }
  if (graph.hidden) action(app, side, `${graph.hidden} more agents`, "›", () => app.actions["pane-picker"]!.run());
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
