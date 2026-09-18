// Draw what the server last sent: pane terminals at their layout positions, then the chrome.
import { displayRects } from "../core/layout";
import { b64 } from "../protocol/conn";
import type { PaneInfo } from "../protocol/types";
import type { App } from "./context";
import { fit } from "./design";
import { ClientPane } from "./panes/pane";
import { beginResize, onDivider } from "./panes/resize";
import { pointer } from "./panes/pointer";
import { contextMenu } from "./modals/context-menu";
import { drawTabs } from "./chrome/tabs";
import { drawSidebar } from "./chrome/sidebar";
import { drawStatus } from "./chrome/status";
import { popupRect, pluginLink } from "./plugin-ui";

export function render(app: App) {
  const { r, th } = app;
  let view = app.view;
  if (app.quitting || !view || !view.workspaces.length) return;
  // Older servers could leave a selected, empty workspace after a spawn error.
  // Keep server indices intact and move back to a workspace that has a terminal.
  if (!view.workspaces[view.active]?.tabs.length) {
    const index = view.workspaces.findIndex((w) => w.tabs.length > 0);
    if (index < 0) return;
    view = app.view = { ...view, active: index };
    app.call("selectWorkspace", { index });
  }
  if (!app.ws().tabs[app.ws().active]) {
    app.ws().active = 0;
    app.call("selectTab", { index: 0 });
  }
  const a = app.area();
  const t = app.tab();
  const rs = displayRects(t.tree, a, t.focused, t.zoomed);
  const visible = new Set(rs.keys());
  // create/destroy terminals to match the server
  const live = new Set(view.panes.map((p) => p.id));
  for (const [id, p] of app.panes) if (!live.has(id)) (p.destroy(), app.panes.delete(id));
  for (const p of view.panes) if (!app.panes.has(p.id)) addPane(app, p);
  // a plugin popup this client opened: its process ended, so it's gone
  if (app.popup && !live.has(app.popup.pane)) app.modal?.close(null);
  for (const [id, p] of app.panes) {
    if (id === app.popup?.pane) {
      const rect = popupRect(app);
      // an opaque background: a box's default is transparent, and the spaces in its title would show the pane beneath
      Object.assign(p.box, { visible: true, left: rect.x, top: rect.y, width: rect.w, height: rect.h, zIndex: 101, borderColor: th.focus, titleColor: th.focus, backgroundColor: th.bg });
      p.box.title = fit(` ${view.panes.find((x) => x.id === id)?.title ?? app.popup.title} · prefix x closes `, Math.max(0, rect.w - 4));
      p.colors(th.bg, th.fg);
      if (!p.term.focused) p.term.focus();
      continue;
    }
    p.box.visible = visible.has(id);
    p.box.zIndex = 1;
    p.colors(th.bg, th.fg);
    if (!visible.has(id)) continue;
    const rect = rs.get(id)!;
    Object.assign(p.box, { left: rect.x, top: rect.y, width: rect.w, height: rect.h });
    const i = app.info(id)!;
    const focused = id === t.focused;
    const st = i.agent?.state;
    p.box.borderColor = focused ? th.focus : st === "blocked" ? th.blocked : th.border;
    const { indicators, pane_labels: labels } = app.cfg;
    const agentTag = i.agent ? [indicators.pane && app.icon(i.agent.state), labels.agent && `${i.agent.harness} ${i.agent.state}`].filter(Boolean).map((s) => " " + s).join("") : "";
    const exited = i.status === "exited" ? ` [exited ${i.exitCode ?? "?"}]` : "";
    // plugins' badges, only for the process they were set for
    const badges = (view.plugins ?? []).flatMap((plugin) => plugin.badges.filter((b) => b.pane === id && b.instance === i.instance).map((b) => ` [${plugin.plugin}: ${b.text}]`)).join("");
    p.box.title = fit(` ${focused ? "◆" : "◇"} ${i.name ? "@" + i.name : i.title}${agentTag}${exited}${badges} `, Math.max(0, rect.w - 4));
    p.box.titleColor = focused ? th.focus : st ? th[st] : th.dim;
    const wantFocus = focused && !app.modal && !app.editing && app.mode === "normal";
    if (wantFocus && !p.term.focused) p.term.focus();
    else if (!wantFocus && p.term.focused) p.term.blur();
  }
  // Rebuild the tab bar, sidebar and status row only when what they show changed. Rebuilding
  // replaces their buttons, and a click that lands before the next frame would hit nothing.
  const sig = JSON.stringify([
    view.active, view.paused, view.workspaces, view.plugins,
    view.panes.map((p) => [p.id, p.name, p.title, p.status, p.agent?.state, p.agent?.harness]),
    r.width, r.height, app.sidebar, app.cfg.theme, app.cfg.sidebar.width, app.cfg.indicators, th,
  ]);
  if (sig !== app.chromeSig) {
    app.chromeSig = sig;
    drawTabs(app);
    drawSidebar(app);
    drawStatus(app);
  }
  r.requestRender();
}

function addPane(app: App, p: PaneInfo) {
  const cp = new ClientPane(
    app.r,
    p.id,
    p.cols,
    p.rows,
    (bytes) => app.conn.notify("input", { pane: p.id, data: b64(bytes) }),
    {
      click: () => app.view && app.tab().focused !== p.id && app.call("focusPane", { pane: p.id }),
      context: (x, y) => contextMenu(app, p.id, x, y),
      onDivider: (x, y) => onDivider(app, x, y),
      beginResize: (x, y) => beginResize(app, x, y),
      pointer: (shape) => pointer(app, shape),
      link: (url, x, y) => pluginLink(app, p.id, url, x, y),
    },
  );
  app.panes.set(p.id, cp);
  app.r.root.add(cp.box);
}
