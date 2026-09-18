// Telling the user something happened (an agent needs them, finished, or started working), and applying
// config changes.
import { setupLogos } from "./logos";
import { loadConfig } from "../config/config";
import type { NotifyEvent } from "../protocol/types";
import type { App } from "./context";
import { render } from "./render";
import { playSound } from "./sound/player";

export function systemNotification(app: App, text: string) {
  if (Bun.which("osascript")) Bun.spawn(["osascript", "-e", `display notification ${JSON.stringify(text)} with title "modisa"`]);
  else if (Bun.which("notify-send")) Bun.spawn(["notify-send", "modisa", text]);
  else app.r.triggerNotification(text, "modisa");
}

export async function notify(app: App, state: NotifyEvent, text: string) {
  const kinds = app.cfg.notify[state] ?? [];
  if (kinds.includes("toast")) app.toast(text, app.th[state]);
  if (kinds.includes("system")) systemNotification(app, text);
  if (kinds.includes("sound")) playSound(app.cfg.sound[state], app.cfg.sound.volume);
  if (kinds.includes("bell")) Bun.write(Bun.stdout, "\x07");
}

export async function reload(app: App, manual = false) {
  const next = await loadConfig();
  // unchanged: usually the settings page saving what it already applied
  if (JSON.stringify(next) === JSON.stringify(app.cfg)) return manual && app.toast("config unchanged", app.th.dim);
  if (next.sidebar.visible !== app.cfg.sidebar.visible) app.sidebar = next.sidebar.visible;
  const logos = next.sidebar.logos !== app.cfg.sidebar.logos;
  app.setConfig(next);
  if (logos) void setupLogos(app);
  app.conn.notify("area", { area: app.area() });
  render(app);
  app.toast("config reloaded", app.th.accent);
}
