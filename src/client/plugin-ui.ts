// What plugins contribute to the TUI, drawn in the user's theme from the data the server sends with the view, and
// running their actions from a status segment, sidebar row, menu entry or the palette. Everything a plugin shows is
// attributed to it by name, so none of it can pass for shepherd's own prompts.
import type { PluginUiView, Tone } from "../protocol/types";
import type { App } from "./context";
import { systemNotification } from "./notify";

export const pluginUi = (app: App): PluginUiView[] => app.view?.plugins ?? [];
export const toneColor = (app: App, tone: Tone) => ({ fg: app.th.fg, dim: app.th.dim, accent: app.th.accent, warn: app.th.warn })[tone] ?? app.th.fg;
const titleOf = (app: App, plugin: string, action: string) => pluginUi(app).find((p) => p.plugin === plugin)?.actions.find((a) => a.id === action)?.title ?? action;
const short = (value: unknown) => {
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length > 80 ? `${text.slice(0, 79)}…` : text;
};

// Run a plugin's action and say how it went: its result, its error, or that a timeout left the outcome unknown.
// `from` is the run whose UI it was taken from (captured when that was drawn): the server refuses it if that run ended.
export async function runPluginAction(app: App, from: { plugin: string; run: string }, action: string, params: Record<string, unknown> = {}) {
  const label = `${from.plugin}: ${titleOf(app, from.plugin, action)}`;
  try {
    const result = await app.conn.request("plugin.invoke", { plugin: from.plugin, action, params, run: from.run });
    app.toast(result === null || result === undefined ? `${label} ✓` : `${label} → ${short(result)}`, app.th.done);
  } catch (e) {
    const { code, message } = e as { code?: string; message: string };
    if (code === "timeout") app.toast(`${label}: no answer in time, so its outcome is unknown`, app.th.warn, 8000);
    else app.toast(`${label}: ${message}`, app.th.blocked, 8000);
  }
}

// A plugin's toast. A system notification too only if it asked and the user has system notifications on for something.
export function pluginToast(app: App, d: { plugin: string; text: string; tone: Tone; system?: boolean }) {
  app.toast(`${d.plugin}: ${d.text}`, toneColor(app, d.tone));
  if (d.system && Object.values(app.cfg.notify).some((kinds) => kinds.includes("system"))) systemNotification(app, `${d.plugin}: ${d.text}`);
}
