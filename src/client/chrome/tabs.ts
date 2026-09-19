// The top row: the space button, the current space's tabs, and new-tab / overflow controls.
import { BoxRenderable } from "@opentui/core";
import { panes as treePanes } from "../../core/layout";
import type { App } from "../context";
import { fit, tabWindow } from "../design";
import { contextMenu } from "../modals/context-menu";
import { menu } from "../modals/menu";
import { button } from "./button";
import type { TabView } from "../../protocol/types";

// A tab's name: the one it was given, else its focused pane's @name or title
export function tabLabel(app: App, t: TabView) {
  const focused = app.info(t.focused);
  return t.name ?? (focused?.name ? "@" + focused.name : focused?.title ?? "shell");
}

export function drawTabs(app: App) {
  const { r, th, ui: { tabBar } } = app;
  for (const c of tabBar.getChildren()) c.destroyRecursively();
  const m = app.metrics();
  tabBar.top = m.top - 1;
  tabBar.backgroundColor = th.bar;
  const w = app.ws();
  // The workspace is a compact chip, not a column aligned with the sidebar. Keep
  // its arrow visible even when a long workspace name needs truncation.
  const brandWidth = Math.min(Bun.stringWidth(w.name) + 6, 24, Math.max(1, r.width - 16));
  const brand = brandWidth >= 6 ? ` ◈ ${fit(w.name, brandWidth - 6)} ▸ ` : fit(` ${w.name}`, brandWidth);
  button(app, tabBar, brand, brandWidth, th.bg, th.focus, () => app.actions["workspace-picker"]!.run());
  tabBar.add(new BoxRenderable(r, { width: 1, height: 1, flexShrink: 0 }));
  const available = Math.max(1, r.width - brandWidth - 10); // 2 for the active tab's ✕
  const window = tabWindow(w.tabs.length, w.active, available);
  for (let i = window.start; i < window.end; i++) {
    const t = w.tabs[i]!;
    const on = i === w.active;
    const blocked = app.cfg.indicators.tab && treePanes(t.tree).some((id) => app.info(id)?.agent?.state === "blocked");
    const suffix = `${t.zoomed ? " [Z]" : ""}${blocked ? " " + app.icon("blocked") : ""}`;
    const prefix = ` ${i + 1}:`;
    const name = tabLabel(app, t);
    const text = prefix + fit(name, window.width - Bun.stringWidth(prefix + suffix) - 2) + suffix + " ";
    // sized to the label so tabs sit side by side; window.width only caps long names.
    // Clicking the tab you're on renames it.
    button(app, tabBar, text, Math.min(window.width - 1, Bun.stringWidth(text)), blocked ? th.warn : on ? th.fg : th.dim, on ? th.border : th.bar,
      () => (on ? app.actions["rename-tab"]!.run() : app.call("selectTab", { index: i })),
      (e) => tabMenu(app, i, e.x, e.y).catch((err) => app.toast(String(err), th.blocked)));
    if (on) button(app, tabBar, "✕ ", 2, th.dim, th.border, () => app.actions["close-tab"]!.run());
    tabBar.add(new BoxRenderable(r, { width: 1, height: 1, flexShrink: 0 }));
  }
  button(app, tabBar, " + ", 3, th.focus, th.bar, () => app.actions["new-tab"]!.run());
  if (window.end - window.start < w.tabs.length) {
    tabBar.add(new BoxRenderable(r, { flexGrow: 1, height: 1 }));
    button(app, tabBar, " ‹", 2, th.dim, th.bar, () => app.actions["prev-tab"]!.run());
    button(app, tabBar, " ›", 2, th.dim, th.bar, () => app.actions["next-tab"]!.run());
  }
}

// Right-click on a tab: it becomes the active one, then rename, close, or its focused pane's menu.
async function tabMenu(app: App, index: number, x: number, y: number) {
  const t = app.ws().tabs[index];
  if (app.modal || !t) return;
  await app.conn.request("cmd", { name: "selectTab", args: { index } });
  const action = await menu(app, `Tab · ${tabLabel(app, t)}`, [
    { name: "Rename tab", key: "r", action: "rename-tab" },
    { name: "Pane menu", key: "p", action: "pane" },
    { name: "Close tab", key: "x", action: "close-tab", danger: true },
  ], x, y);
  if (action === "pane") contextMenu(app, t.focused, x, y);
  else if (action) app.actions[action]?.run();
}
