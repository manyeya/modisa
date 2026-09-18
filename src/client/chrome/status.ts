// The bottom row: sidebar toggle, new agent and agent counts on the left; pane count, theme and the active space's
// git on the right. [status] and [git] choose which of them show.
import { BoxRenderable, StyledText, TextRenderable, fg, type TextChunk } from "@opentui/core";
import type { GitView } from "../../protocol/types";
import { panes as treePanes } from "../../core/layout";
import type { App } from "../context";
import { button } from "./button";
import { fit } from "../design";
import { pluginUi, runPluginAction, toneColor } from "../plugin-ui";

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
  segment(" + agent ", th.focus, th.border, () => app.actions["new-agent"]!.run());
  if (r.width >= 100 && app.cfg.status.agents) {
    segment(` ${app.icon("working")} ${running} working `, th.focus, th.bar, () => app.actions["working-agents"]!.run());
    segment(` ${app.icon("blocked")} ${blocked} need you `, blocked ? th.warn : th.dim, th.bar, () => app.actions["blocked-agents"]!.run());
  }
  // plugins' segments while there's room; modisa's own come first, and the right side keeps its space
  if (r.width >= 110) {
    let room = r.width - 100;
    for (const plugin of pluginUi(app)) {
      for (const s of plugin.status) {
        const text = ` ${plugin.plugin}: ${fit(s.text, 24)} `; // named, so it can't pass for modisa's own
        const width = Bun.stringWidth(text);
        if (width > room) break;
        room -= width;
        segment(text, toneColor(app, s.tone), th.bar, () => s.action && runPluginAction(app, plugin, s.action));
      }
    }
  }
  telemetry.add(new BoxRenderable(r, { flexGrow: 1, height: 1 }));
  if (app.view?.paused && r.width >= 120) segment(" PAUSED ", th.warn, th.bar, () => app.actions["toggle-messaging"]!.run());
  // right: a newer release, the pane count, the theme, then the active space's git
  if (app.update) segment(` ↑ ${app.update.version} `, th.bg, th.warn, () => app.actions["update-modisa"]!.run());
  if (r.width >= 50 && app.cfg.status.panes) segment(` ${count} ${count === 1 ? "pane" : "panes"} `, th.fg, th.bar, () => app.actions["pane-picker"]!.run());
  if (r.width >= 60 && app.cfg.status.theme) segment(` ◐ ${app.cfg.theme} `, th.dim, th.bar, () => app.actions["theme-picker"]!.run());
  const git = app.cfg.git.status ? app.view?.workspaces[app.view.active]?.git : undefined;
  if (git && r.width >= 60) {
    const line = gitLine(app, git);
    telemetry.add(new TextRenderable(r, { content: line, width: Bun.stringWidth(line.chunks.map((c) => c.text).join("")), height: 1, flexShrink: 0, bg: th.bar }));
  }
}

// Where the repository stands: its name, the branch (in the done colour when clean and in step with its upstream),
// ↑ commits to push, ↓ commits to pull, ● files changed.
function gitLine(app: App, git: GitView): StyledText {
  const { th } = app;
  const clean = !git.changes && !git.ahead && !git.behind && git.ahead !== undefined;
  const chunks: TextChunk[] = app.cfg.git.repo ? [fg(th.fg)(` ${fit(git.repo, 24)}`)] : [];
  chunks.push(fg(clean ? th.done : th.dim)(` ⎇ ${fit(git.branch, 24)}`));
  if (git.ahead && app.cfg.git.counts) chunks.push(fg(th.working)(` ↑${git.ahead}`));
  if (git.behind && app.cfg.git.counts) chunks.push(fg(th.warn)(` ↓${git.behind}`));
  if (git.changes && app.cfg.git.changes) chunks.push(fg(th.accent)(` ●${git.changes}`));
  chunks.push(fg(th.dim)(" "));
  return new StyledText(chunks);
}
