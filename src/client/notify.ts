// Telling the user something happened (an agent needs them, finished, or started working), and applying
// config changes.
import { setupLogos } from "./logos";
import { readConfig, type Config } from "../config/config";
import type { NotifyEvent, Tone } from "../protocol/types";
import type { App } from "./context";
import { render } from "./render";
import { playSound } from "./sound/player";
import { toneColor } from "./plugin-ui";

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

// What a toast someone sent may do besides showing: a system notification and a sound only if it asked and the user
// has that kind on for some event. The sound is the one for the event its tone is, else the first event's that plays.
export function toastExtras(cfg: Config, d: { tone: Tone; system?: boolean; sound?: boolean }) {
  const on = (kind: "system" | "sound") => (Object.keys(cfg.notify) as NotifyEvent[]).filter((e) => cfg.notify[e]?.includes(kind));
  const sounding = on("sound");
  const event = sounding.find((e) => e === d.tone) ?? sounding[0];
  return { system: !!d.system && on("system").length > 0, sound: d.sound && event ? cfg.sound[event] : undefined };
}

// A toast someone sent (a plugin's ui.toast, or `modisa notify`), titled with who sent it.
export function sentToast(app: App, d: { plugin: string; text: string; tone: Tone; system?: boolean; sound?: boolean }) {
  app.toast(d.text, toneColor(app, d.tone), undefined, d.plugin);
  const extras = toastExtras(app.cfg, d);
  if (extras.system) systemNotification(app, `${d.plugin}: ${d.text}`);
  if (extras.sound) playSound(extras.sound, app.cfg.sound.volume);
}

export async function reload(app: App, manual = false) {
  const { cfg: next, error } = await readConfig();
  // half-edited, say: keep what's applied rather than fall back to the defaults
  if (error) return app.toast(`config.toml: ${error} (modisa config check)`, app.th.warn);
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
