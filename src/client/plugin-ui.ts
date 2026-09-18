// What plugins contribute to the TUI, drawn in the user's theme from the data the server sends with the view, and
// running their actions from a status segment, sidebar row, menu entry or the palette. Everything a plugin shows is
// attributed to it by name, so none of it can pass for modisa's own prompts.
import { BoxRenderable, StyledText, bold, fg, type TextChunk } from "@opentui/core";
import type { PluginUiView, Span, Tone } from "../protocol/types";
import { linkMatches } from "../protocol/links";
import { bindPluginKeys } from "../config/keys";
import type { App } from "./context";
import { systemNotification } from "./notify";
import { menu } from "./modals/menu";
import { agentMark, fit } from "./design";
import { render } from "./render";

export const pluginUi = (app: App): PluginUiView[] => app.view?.plugins ?? [];
export const toneColor = (app: App, tone: Tone) => {
  const th = app.th;
  return ({ fg: th.fg, dim: th.dim, accent: th.accent, warn: th.warn, working: th.working, blocked: th.blocked, done: th.done, idle: th.idle })[tone] ?? th.fg;
};

// A sidebar row's spans as styled text, cut to `width` cells with … where it runs out. An icon is the agent's glyph
// in its brand colour, or in the theme's text colour when the brand has none or it would be faint on this theme.
export function spanText(app: App, spans: Span[], base: Tone, width: number): StyledText {
  const chunks: TextChunk[] = [];
  let left = width;
  for (const x of spans) {
    if (left <= 0) break;
    if ("icon" in x) {
      // a logo is drawn two cells wide: the cell after it is its own, so what follows doesn't run into it
      const { glyph, color, cells } = agentMark(app.th, x.icon, app.logos);
      chunks.push(fg(color)(glyph + " ".repeat(cells - 2)));
      left -= cells - 1;
      continue;
    }
    const text = fit(x.text, left);
    const chunk = fg(toneColor(app, x.tone ?? base))(text);
    chunks.push(x.bold ? bold(chunk) : chunk);
    left -= Bun.stringWidth(text);
  }
  return new StyledText(chunks);
}
const titleOf = (app: App, plugin: string, action: string) => pluginUi(app).find((p) => p.plugin === plugin)?.actions.find((a) => a.id === action)?.title ?? action;
const short = (value: unknown) => {
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length > 80 ? `${text.slice(0, 79)}…` : text;
};

// Run a plugin's action and say how it went: its result, its error, or that a timeout left the outcome unknown.
// `from` is the run whose UI it was taken from (captured when that was drawn): the server refuses it if that run ended.
export async function runPluginAction(app: App, from: { plugin: string; run: string }, action: string, params: Record<string, unknown> = {}, target?: { pane: string; instance: string }, link?: string) {
  const label = `${from.plugin}: ${titleOf(app, from.plugin, action)}`;
  try {
    const result = await app.conn.request("plugin.invoke", { plugin: from.plugin, action, params, run: from.run, ...(target && { target }), ...(link && { link }) });
    app.toast(result === null || result === undefined ? `${label} ✓` : `${label} → ${short(result)}`, app.th.done);
  } catch (e) {
    const { code, message } = e as { code?: string; message: string };
    if (code === "timeout") app.toast(`${label}: no answer in time, so its outcome is unknown`, app.th.warn, 8000);
    else app.toast(`${label}: ${message}`, app.th.blocked, 8000);
  }
}

// Plugins' keys as this client's own config binds them: the session's server sends what plugin.json declares, and
// [plugin_keys] here, not on the server, decides which key runs what, so each attached client can differ.
export const pluginKeys = (app: App) => bindPluginKeys(pluginUi(app).flatMap((p) => p.keys.map((k) => ({ plugin: p.plugin, ...k }))), app.cfg.plugin_keys);

// Prefix + a plugin's key: its action, or its pane, for the pane focused now (not when the action finishes).
export function pluginKey(app: App, key: string) {
  if (!app.view) return;
  const bound = pluginKeys(app).find((k) => k.key === key && k.state === "active");
  const plugin = bound && pluginUi(app).find((p) => p.plugin === bound.plugin);
  if (!bound || !plugin) return;
  const pane = app.tab().focused;
  const instance = app.info(pane)?.instance;
  const target = instance ? { pane, instance } : undefined;
  if (bound.action) return runPluginAction(app, plugin, bound.action, {}, target);
  if (bound.pane) return openPluginPane(app, plugin, bound.pane, {}, target);
}

// Ctrl+click on a URL: the plugin actions whose link pattern matches it, by plugin name then manifest order. One runs;
// several ask which; none says so. The URL goes as the invocation's link, never as params.
export function pluginLink(app: App, pane: string, url: string, x: number, y: number) {
  if (app.modal) return;
  const handlers = [...pluginUi(app)].sort((a, b) => a.plugin.localeCompare(b.plugin)).flatMap((plugin) => [...new Set(plugin.links.filter((l) => linkMatches(l, url)).map((l) => l.action))].map((action) => ({ plugin, action }))); // an action once, however many of its links match
  const instance = app.info(pane)?.instance;
  const go = (h: (typeof handlers)[number]) => runPluginAction(app, h.plugin, h.action, {}, instance ? { pane, instance } : undefined, url);
  if (!handlers.length) return app.toast(`No plugin handles ${short(url)}`, app.th.dim);
  if (handlers.length === 1) return go(handlers[0]!);
  // the title says what's being chosen, with the URL cut to fit the menu (fit also blanks control characters)
  menu(app, `Open ${fit(url, 18)} with`, handlers.map((h, i) => ({ name: `${h.plugin.plugin}: ${titleOf(app, h.plugin.plugin, h.action)}`, key: "", action: String(i) })), x, y)
    .then((i) => {
      const h = i === null ? undefined : handlers[Number(i)];
      if (h) void go(h);
    })
    .catch((e) => app.toast(String(e), app.th.blocked));
}

// Open one of a plugin's panes. A popup is shown only here, by the client that asked, over everything else; if a dialog
// is already open it waits for nothing and says so (ui_busy).
export async function openPluginPane(app: App, from: { plugin: string; run: string }, pane: string, params: Record<string, unknown> = {}, origin?: { pane: string; instance?: string }) {
  if (app.modal) return app.toast(`${from.plugin}: can't open ${pane} while a dialog is open`, app.th.warn);
  const placement = pluginUi(app).find((p) => p.plugin === from.plugin)?.panes.find((p) => p.id === pane)?.placement;
  if (placement === "popup" && (app.r.width < POPUP_MIN.w + 2 || app.r.height < POPUP_MIN.h + 2)) return app.toast(`${from.plugin}: the terminal is too small for ${pane}`, app.th.warn);
  try {
    const opened = await app.conn.request<{ pane: string; placement: string; title: string; width?: number | string; height?: number | string }>("plugin.pane.open", { plugin: from.plugin, pane, params, run: from.run, ...(origin && { from: origin }) });
    if (opened.placement === "popup") showPopup(app, opened);
  } catch (e) {
    const { code, message } = e as { code?: string; message: string };
    app.toast(code === "ui_busy" ? `${from.plugin}: a popup is already open` : `${from.plugin}: ${message}`, code === "ui_busy" ? app.th.warn : app.th.blocked);
  }
}

// cells from a manifest size: a number of cells, or a percentage of the terminal
const cells = (size: number | string | undefined, total: number, fallback: number) =>
  typeof size === "number" ? size : typeof size === "string" && size.endsWith("%") ? Math.floor((total * Number(size.slice(0, -1))) / 100) : fallback;

// The popup's box: its manifest size, at least POPUP_MIN, but never past the terminal (a cell of margin each side), even
// when the terminal shrinks below the minimum while the popup is open. Below the minimum, a popup doesn't open.
export const POPUP_MIN = { w: 20, h: 5 };
export function popupRect(app: App) {
  const p = app.popup!;
  const w = Math.max(1, Math.min(app.r.width - 2, Math.max(POPUP_MIN.w, cells(p.width, app.r.width, Math.floor(app.r.width * 0.7)))));
  const h = Math.max(1, Math.min(app.r.height - 2, Math.max(POPUP_MIN.h, cells(p.height, app.r.height, Math.floor(app.r.height * 0.6)))));
  return { x: Math.floor((app.r.width - w) / 2), y: Math.floor((app.r.height - h) / 3), w, h };
}

// The popup is a modal: a veil over everything, and the popup's terminal on top. Its program gets every key,
// Escape included; prefix x closes it.
function showPopup(app: App, opened: { pane: string; title: string; width?: number | string; height?: number | string }) {
  app.popup = { pane: opened.pane, title: opened.title, width: opened.width, height: opened.height };
  // its program sizes to the popup, again whenever the terminal does
  const fitProgram = () => {
    const rect = popupRect(app);
    app.conn.request("plugin.popup.resize", { pane: opened.pane, cols: Math.max(10, rect.w - 2), rows: Math.max(3, rect.h - 2) }).catch(() => {});
  };
  fitProgram();
  let armed = false;
  const veil = new BoxRenderable(app.r, { position: "absolute", left: 0, top: 0, width: "100%", height: "100%", zIndex: 90 });
  veil.onMouseDown = (e) => { e.preventDefault(); e.stopPropagation(); };
  app.r.root.add(veil);
  const close = () => {
    if (app.popup?.pane !== opened.pane) return;
    app.popup = undefined;
    app.modal = undefined;
    veil.destroyRecursively();
    app.conn.request("plugin.popup.close", { pane: opened.pane }).catch(() => {}); // already gone is fine
    render(app);
  };
  app.modal = {
    keepEscape: true,
    close,
    resize: () => (fitProgram(), render(app)),
    keys: (k) => {
      if (k.ctrl && k.name === app.prefix.name && !armed) return (armed = true);
      if (armed && k.name === "x") {
        armed = false;
        close();
        return true;
      }
      armed = false;
      return false; // everything else is the popup program's
    },
  };
  render(app);
}

// A plugin's toast. A system notification too only if it asked and the user has system notifications on for something.
export function pluginToast(app: App, d: { plugin: string; text: string; tone: Tone; system?: boolean }) {
  app.toast(`${d.plugin}: ${d.text}`, toneColor(app, d.tone));
  if (d.system && Object.values(app.cfg.notify).some((kinds) => kinds.includes("system"))) systemNotification(app, `${d.plugin}: ${d.text}`);
}
