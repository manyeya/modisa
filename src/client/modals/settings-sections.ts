// What the settings page shows: one section per tab, each a list of rows. Rows read the live config;
// a change applies at once and is saved to config.toml. ./settings.ts draws the rows and drives them.
import { saveSetting, type IndicatorStyle, type NotifyKind } from "../../config/config";
import { THEMES } from "../../config/themes";
import type { IntegrationStatus, NotifyEvent } from "../../protocol/types";
import { INDICATORS, type App } from "../context";
import { render } from "../render";
import { playSound } from "../sound/player";
import { SOUNDS } from "../sound/recipes";

export type Row =
  | { kind: "heading"; label: string }
  | { kind: "radio"; label: string; current: boolean; swatches?: string[]; preview?: () => void; apply: () => void }
  | { kind: "toggle"; label: string; on: boolean; flip: () => void }
  | { kind: "choice"; label: string; value: string; step: (by: 1 | -1) => void; enter?: () => void }
  | { kind: "action"; label: string; status: string; tone: "ok" | "warn" | "accent" | "dim"; note?: string; hint: string; run: () => void };

// enter/leave run when the section is shown and when the page moves away or closes
export type Section = { name: string; rows: () => Row[]; enter?: () => void; leave?: () => void };

const EVENTS: [NotifyEvent, string][] = [["blocked", "needs you"], ["done", "done"], ["working", "started working"]];
const ALERTS: [NotifyKind, string][] = [["toast", "toast"], ["system", "system notification"], ["bell", "terminal bell"]];
const STYLES: IndicatorStyle[] = ["symbols", "dots", "letters"];

// Saves run one at a time: each reads the file the previous one wrote.
let saving = Promise.resolve();

function save(app: App, table: "notify" | "sound" | "indicators" | "pane_labels" | null, key: string, value: any) {
  const cfg: any = structuredClone(app.cfg);
  if (table) cfg[table][key] = value;
  else cfg[key] = value;
  app.setConfig(cfg);
  render(app);
  saving = saving.then(() => saveSetting(table, key, value)).catch((e) => app.toast(`not saved: ${e.message ?? e}`, app.th.warn));
}

function theme(app: App): Section {
  let saved = app.cfg.theme;
  const show = (name: string) => { app.setConfig({ ...app.cfg, theme: name }); render(app); };
  return {
    name: "theme",
    rows: () => Object.entries(THEMES).map(([name, p]) => ({
      kind: "radio", label: name, current: name === saved, swatches: [p.focus, p.accent, p.working, p.blocked, p.done],
      preview: () => show(name), // live, until you apply or leave
      apply: () => { saved = name; save(app, null, "theme", name); app.toast(`Theme saved: ${name}`, app.th.accent); },
    })),
    enter: () => { saved = app.cfg.theme; },
    leave: () => { if (app.cfg.theme !== saved) show(saved); },
  };
}

function indicators(app: App): Section {
  const place = (label: string, key: "tab" | "pane" | "sidebar"): Row => ({ kind: "toggle", label, on: app.cfg.indicators[key], flip: () => save(app, "indicators", key, !app.cfg.indicators[key]) });
  return {
    name: "indicators",
    rows: () => [
      { kind: "heading", label: "style" },
      ...STYLES.map((style): Row => ({ kind: "radio", label: `${style.padEnd(9)} ${Object.values(INDICATORS[style]).join(" ")}`, current: app.cfg.indicators.style === style, apply: () => save(app, "indicators", "style", style) })),
      { kind: "heading", label: "show in" },
      place("tab bar badge", "tab"),
      place("pane border title", "pane"),
      place("sidebar", "sidebar"),
    ],
  };
}

// Which sound each event plays: "off" takes "sound" out of that event's [notify] list.
function sound(app: App): Section {
  const options = ["off", ...SOUNDS];
  const play = (name: string, volume = app.cfg.sound.volume) => name !== "off" && playSound(name, volume);
  return {
    name: "sound",
    rows: () => [
      ...EVENTS.map(([event, label]): Row => {
        const kinds = app.cfg.notify[event];
        const value = kinds.includes("sound") ? app.cfg.sound[event] : "off";
        return {
          kind: "choice", label, value, enter: () => play(value),
          step: (by) => {
            const next = options[(options.indexOf(value) + by + options.length) % options.length]!;
            if (next === "off") return save(app, "notify", event, kinds.filter((k) => k !== "sound"));
            if (!kinds.includes("sound")) save(app, "notify", event, [...kinds, "sound"]);
            save(app, "sound", event, next);
            play(next);
          },
        };
      }),
      {
        kind: "choice", label: "volume", value: `${Math.round(app.cfg.sound.volume * 100)}%`,
        enter: () => play(app.cfg.sound.blocked),
        step: (by) => {
          const volume = Math.min(1, Math.max(0, Math.round(app.cfg.sound.volume * 10 + by) / 10));
          save(app, "sound", "volume", volume);
          play(app.cfg.sound.blocked, volume);
        },
      },
    ],
  };
}

function toasts(app: App): Section {
  return {
    name: "toasts",
    rows: () => EVENTS.flatMap(([event]): Row[] => [
      { kind: "heading", label: { blocked: "when an agent needs you", done: "when an agent is done", working: "when an agent starts working" }[event] },
      ...ALERTS.map(([kind, name]): Row => {
        const kinds = app.cfg.notify[event];
        return { kind: "toggle", label: name, on: kinds.includes(kind), flip: () => save(app, "notify", event, kinds.includes(kind) ? kinds.filter((k) => k !== kind) : [...kinds, kind]) };
      }),
    ]),
  };
}

function paneLabels(app: App): Section {
  const toggle = (label: string, key: "agent"): Row => ({ kind: "toggle", label, on: app.cfg.pane_labels[key], flip: () => save(app, "pane_labels", key, !app.cfg.pane_labels[key]) });
  return {
    name: "pane labels",
    rows: () => [toggle("agent and state in the border title", "agent")],
  };
}

// Installed on the machine the server runs on (where the agents are), so it asks the server. Every
// agent is listed: installed, out of date, available (the agent is here), or not found.
function integrations(app: App, repaint: () => void): Section {
  let list: IntegrationStatus[] | undefined;
  let busy = "";
  const load = () => app.conn.request("integrations", {}).then((l: IntegrationStatus[]) => { list = l; repaint(); }, (e: any) => app.toast(e.message ?? String(e), app.th.blocked));
  const apply = async (ids: string[], install: (id: string) => boolean) => {
    for (const id of ids) {
      busy = id;
      repaint();
      try { app.toast(await app.conn.request("integration", { id, install: install(id) }), app.th.done); }
      catch (e: any) { app.toast(e.message ?? String(e), app.th.blocked); }
    }
    busy = "";
    await load();
  };
  const order = (i: IntegrationStatus) => (i.status !== "none" ? 0 : i.available ? 1 : 2);
  return {
    name: "integrations",
    enter: () => { void load(); },
    rows: () => {
      if (!list) return [{ kind: "heading", label: "checking…" }];
      const todo = list.filter((i) => i.status === "outdated" || (i.available && i.status === "none"));
      const rows: Row[] = todo.length ? [{
        kind: "action", label: `Install all (${todo.length})`, status: todo.map((i) => i.name).join(", "), tone: "accent", hint: "↵ install",
        run: () => { if (!busy) void apply(todo.map((i) => i.id), () => true); },
      }] : [];
      for (const i of [...list].sort((a, b) => order(a) - order(b))) rows.push({
        kind: "action", label: i.name,
        status: busy === i.id ? "working…" : i.status === "current" ? "✓ installed" : i.status === "outdated" ? "↻ update available" : i.available ? "+ available" : "not found",
        tone: i.status === "current" ? "ok" : i.status === "outdated" ? "warn" : i.available ? "accent" : "dim",
        note: i.kind === "lifecycle" ? "state + session" : "session",
        hint: i.status === "current" ? "↵ remove" : i.status === "outdated" ? "↵ update" : "↵ install",
        run: () => { if (!busy) void apply([i.id], () => i.status !== "current"); },
      });
      rows.push({ kind: "heading", label: "state is read from every agent's screen; integrations add session resume" });
      return rows;
    },
  };
}

export function sections(app: App, repaint: () => void): Section[] {
  return [theme(app), indicators(app), sound(app), toasts(app), paneLabels(app), integrations(app, repaint)];
}
