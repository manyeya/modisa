// The top row: the space button, the current space's tabs, and new-tab / overflow controls.
import { BoxRenderable } from "@opentui/core";
import { panes as treePanes } from "../../core/layout";
import type { App } from "../context";
import { fit, tabWindow } from "../design";
import { contextMenu } from "../modals/context-menu";
import { button } from "./button";

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
  const available = Math.max(1, r.width - brandWidth - 8);
  const window = tabWindow(w.tabs.length, w.active, available);
  for (let i = window.start; i < window.end; i++) {
    const t = w.tabs[i]!;
    const on = i === w.active;
    const focused = app.info(t.focused);
    const blocked = app.cfg.indicators.tab && treePanes(t.tree).some((id) => app.info(id)?.agent?.state === "blocked");
    const suffix = `${t.zoomed ? " [Z]" : ""}${blocked ? " " + app.icon("blocked") : ""}`;
    const prefix = ` ${i + 1}:`;
    const name = t.name ?? (focused?.name ? "@" + focused.name : focused?.title ?? "shell");
    const text = prefix + fit(name, window.width - Bun.stringWidth(prefix + suffix) - 2) + suffix + " ";
    // sized to the label so tabs sit side by side; window.width only caps long names
    button(app, tabBar, text, Math.min(window.width - 1, Bun.stringWidth(text)), blocked ? th.warn : on ? th.fg : th.dim, on ? th.border : th.bar,
      () => app.call("selectTab", { index: i }),
      (e) => contextMenu(app, t.focused, e.x, e.y));
    tabBar.add(new BoxRenderable(r, { width: 1, height: 1, flexShrink: 0 }));
  }
  button(app, tabBar, " + ", 3, th.focus, th.bar, () => app.actions["new-tab"]!.run());
  if (window.end - window.start < w.tabs.length) {
    tabBar.add(new BoxRenderable(r, { flexGrow: 1, height: 1 }));
    button(app, tabBar, " ‹", 2, th.dim, th.bar, () => app.actions["prev-tab"]!.run());
    button(app, tabBar, " ›", 2, th.dim, th.bar, () => app.actions["next-tab"]!.run());
  }
}
