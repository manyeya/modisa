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
  const brandWidth = Math.min(m.side || (r.width >= 60 ? 18 : 10), Math.max(1, r.width - 16));
  button(app, tabBar, ` ◈ ${w.name} ▸`, brandWidth, th.bg, th.accent, () => app.actions["workspace-picker"]!.run());
  const available = Math.max(1, r.width - brandWidth - 7);
  const window = tabWindow(w.tabs.length, w.active, available);
  for (let i = window.start; i < window.end; i++) {
    const t = w.tabs[i]!;
    const on = i === w.active;
    const focused = app.info(t.focused);
    const blocked = app.cfg.indicators.tab && treePanes(t.tree).some((id) => app.info(id)?.agent?.state === "blocked");
    const suffix = `${t.zoomed ? " [Z]" : ""}${blocked ? " " + app.icon("blocked") : ""}`;
    const prefix = ` ${i + 1}:`;
    const name = t.name ?? (focused?.name ? "@" + focused.name : focused?.title ?? "shell");
    const text = prefix + fit(name, window.width - Bun.stringWidth(prefix + suffix) - 1) + suffix + " ";
    // sized to the label so tabs sit side by side; window.width only caps long names
    button(app, tabBar, text, Bun.stringWidth(text), on ? th.bg : blocked ? th.blocked : th.dim, on ? th.focus : th.bar,
      () => app.call("selectTab", { index: i }),
      (e) => contextMenu(app, t.focused, e.x, e.y));
  }
  button(app, tabBar, " + ", 3, th.accent, th.bar, () => app.actions["new-tab"]!.run());
  if (window.end - window.start < w.tabs.length) {
    tabBar.add(new BoxRenderable(r, { flexGrow: 1, height: 1 }));
    button(app, tabBar, " ‹", 2, th.dim, th.bar, () => app.actions["prev-tab"]!.run());
    button(app, tabBar, " ›", 2, th.dim, th.bar, () => app.actions["next-tab"]!.run());
  }
}
