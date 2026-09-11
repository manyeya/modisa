// The bottom row: sidebar toggle, new agent and agent counts on the left; pane count and theme on the right.
import { BoxRenderable } from "@opentui/core";
import { panes as treePanes } from "../../core/layout";
import type { App } from "../context";
import { button } from "./button";

export function drawStatus(app: App) {
  const { r, th, ui: { telemetry } } = app;
  const count = treePanes(app.tab().tree).length;
  const agents = app.sortedAgents();
  const blocked = agents.filter((p) => p.agent!.state === "blocked").length;
  const running = agents.filter((p) => p.agent!.state === "working").length;
  for (const child of telemetry.getChildren()) child.destroyRecursively();
  telemetry.backgroundColor = th.bar;
  const segment = (text: string, fg: string, bg: string, run: () => any) => button(app, telemetry, text, Bun.stringWidth(text), fg, bg, run);
  // left: sidebar toggle, new agent, then agent counts
  const side = app.sideWidth();
  segment(side ? " ◧ sidebar " : " ◨ sidebar ", side ? th.bg : th.fg, side ? th.focus : th.border, () => app.actions["toggle-sidebar"]!.run());
  segment(" + agent ", th.accent, th.border, () => app.actions["new-agent"]!.run());
  if (r.width >= 100) {
    segment(` ${app.icon("working")} ${running} working `, th.working, th.bar, () => app.actions["working-agents"]!.run());
    segment(` ${app.icon("blocked")} ${blocked} need you `, blocked ? th.blocked : th.dim, th.bar, () => app.actions["blocked-agents"]!.run());
  }
  telemetry.add(new BoxRenderable(r, { flexGrow: 1, height: 1 }));
  if (app.view?.paused && r.width >= 120) segment(" PAUSED ", th.warn, th.bar, () => app.actions["toggle-messaging"]!.run());
  // right: a newer release, then the pane count next to the theme changer
  if (app.update) segment(` ↑ ${app.update.version} `, th.bg, th.warn, () => app.actions["update-shepherd"]!.run());
  if (r.width >= 50) segment(` ${count} panes `, th.fg, th.bar, () => app.actions["pane-picker"]!.run());
  if (r.width >= 60) segment(` ◐ ${app.cfg.theme} `, th.bg, th.accent, () => app.actions["theme-picker"]!.run());
}
