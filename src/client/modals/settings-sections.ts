// What the settings page shows: one section per entry in its side list, each a list of rows. Rows read the live
// config; a change applies at once and is saved to config.toml. `about` is the line the page shows under the
// selected row. ./settings.ts draws the rows and drives them.
import { saveSetting, type BorderStyle, type IndicatorStyle, type NotifyKind, type Policy } from "../../config/config";
import { THEMES } from "../../config/themes";
import { VERSION } from "../../core/version";
import type { IntegrationStatus, NotifyEvent } from "../../protocol/types";
import { INDICATORS, type App } from "../context";
import { setupLogos } from "../logos";
import { pluginUi } from "../plugin-ui";
import { render } from "../render";
import { playSound } from "../sound/player";
import { SOUNDS } from "../sound/recipes";

type Common = { label: string; about?: string };
export type Row =
  | { kind: "heading"; label: string }
  | (Common & { kind: "radio"; current: boolean; swatches?: string[]; preview?: () => void; apply: () => void })
  | (Common & { kind: "toggle"; on: boolean; flip: () => void })
  | (Common & { kind: "choice"; value: string; step: (by: 1 | -1) => void; enter?: () => void })
  | (Common & { kind: "action"; status: string; tone: "ok" | "warn" | "accent" | "dim"; note?: string; hint: string; run: () => void });

// enter/leave run when the section is shown and when the page moves away or closes
export type Section = { name: string; rows: () => Row[]; enter?: () => void; leave?: () => void };

const EVENTS: [NotifyEvent, string][] = [["blocked", "needs you"], ["done", "done"], ["working", "started working"]];
const ALERTS: [NotifyKind, string, string][] = [
  ["toast", "toast", "A line in the top right corner of modisa"],
  ["system", "system notification", "Your desktop's notification, for when you're in another window"],
  ["bell", "terminal bell", "The terminal's bell: a sound or a flash, as your terminal does it"],
];
const STYLES: IndicatorStyle[] = ["symbols", "dots", "letters"];
const BORDERS: BorderStyle[] = ["single", "rounded", "double", "heavy"];
const POLICIES: Policy[] = ["ask", "allow", "deny"];
// prefixes that don't take a key shells and programs need (Ctrl+C, Ctrl+D, Ctrl+M is Enter, Ctrl+I is Tab…)
const PREFIXES = [..."abgoqstxy"];

// the next of `options` after `current` (by 1 or -1), round the end
const cycle = <T>(options: readonly T[], current: T, by: number) => options[(options.indexOf(current) + by + options.length) % options.length]!;
const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));

// Saves run one at a time: each reads the file the previous one wrote.
let saving = Promise.resolve();

function save(app: App, table: string | null, key: string, value: any) {
  const cfg: any = structuredClone(app.cfg);
  if (table) cfg[table][key] = value;
  else cfg[key] = value;
  app.setConfig(cfg);
  render(app);
  saving = saving.then(() => saveSetting(table, key, value)).catch((e) => app.toast(`not saved: ${e.message ?? e}`, app.th.warn));
}

const toggle = (app: App, label: string, table: string | null, key: string, about: string, after?: (on: boolean) => void): Row => {
  const on = Boolean(table ? (app.cfg as any)[table][key] : (app.cfg as any)[key]);
  return { kind: "toggle", label, on, about, flip: () => { save(app, table, key, !on); after?.(!on); } };
};

// the sidebar's width moves the panes: the server lays them out again for the new area
const relayout = (app: App) => { app.conn.notify("area", { area: app.area() }); render(app); };

function theme(app: App): Section {
  let saved = app.cfg.theme;
  const show = (name: string) => { app.setConfig({ ...app.cfg, theme: name }); render(app); };
  return {
    name: "theme",
    rows: () => Object.entries(THEMES).map(([name, p]) => ({
      kind: "radio", label: name, current: name === saved, swatches: [p.focus, p.accent, p.working, p.blocked, p.done],
      about: "Previewed as you move; ↵ or a click keeps it",
      preview: () => show(name), // live, until you apply or leave
      apply: () => { saved = name; save(app, null, "theme", name); app.toast(`Theme saved: ${name}`, app.th.accent); },
    })),
    enter: () => { saved = app.cfg.theme; },
    leave: () => { if (app.cfg.theme !== saved) show(saved); },
  };
}

function general(app: App): Section {
  return {
    name: "general",
    rows: () => [
      { kind: "heading", label: "keyboard" },
      {
        kind: "choice", label: "prefix key", value: app.cfg.prefix,
        about: "The key that starts every modisa shortcut: Ctrl and a letter. Press it twice to send it to the pane",
        step: (by) => save(app, null, "prefix", `C-${cycle(PREFIXES, app.prefix.name, by)}`),
      },
      { kind: "action", label: "keyboard guide", status: "every shortcut", tone: "dim", hint: "↵ open", about: "The shortcuts, searchable; choosing one runs it", run: () => app.actions.help!.run() },
      { kind: "heading", label: "mouse" },
      toggle(app, "select on hover", "mouse", "hover", "The pointer resting on a row selects it, here and in every menu and picker. Off: only a click does"),
      { kind: "heading", label: "updates" },
      toggle(app, "check for updates", "update", "check", "Look for a new release every few hours; the status row says when one is out"),
      { kind: "choice", label: "channel", value: app.cfg.update.channel, about: "stable: releases. staging: a prerelease of what's coming", step: (by) => save(app, "update", "channel", cycle(["stable", "staging"] as const, app.cfg.update.channel, by)) },
      { kind: "action", label: "check now", status: VERSION, tone: "dim", hint: "↵ check", about: "Look for a newer modisa now", run: () => app.actions["update-modisa"]!.run() },
      { kind: "heading", label: "config file" },
      { kind: "action", label: "edit config.toml", status: "in $EDITOR", tone: "dim", hint: "↵ open", about: "Everything on this page, and more (agents' launch commands, plugins), as text", run: () => app.actions["edit-config"]!.run() },
      { kind: "action", label: "reload config", status: "", tone: "dim", hint: "↵ reload", about: "Read config.toml again (it's also read whenever it changes)", run: () => app.actions["reload-config"]!.run() },
    ],
  };
}

function layout(app: App): Section {
  return {
    name: "layout",
    rows: () => {
      const { sidebar, panes } = app.cfg;
      const takers = [...new Set(["", ...pluginUi(app).filter((p) => p.sidebar).map((p) => p.plugin), sidebar.agents])];
      return [
        { kind: "heading", label: "sidebar" },
        toggle(app, "show the sidebar", "sidebar", "visible", "Whether it's open when modisa starts; the status row's ◧ button and prefix b toggle it", (on) => { app.sidebar = on; relayout(app); }),
        {
          kind: "choice", label: "width", value: `${sidebar.width} columns`, about: "Or drag its edge. It never takes more than a third of the terminal",
          step: (by) => { save(app, "sidebar", "width", clamp(app.cfg.sidebar.width + by * 2, 20, 48)); relayout(app); },
        },
        {
          kind: "choice", label: "agent logos", value: sidebar.logos, about: "auto: agents' real logos where the terminal can show them. off: plain marks",
          step: (by) => { save(app, "sidebar", "logos", cycle(["auto", "on", "off"] as const, app.cfg.sidebar.logos, by)); void setupLogos(app); },
        },
        toggle(app, "branch lines", "sidebar", "graph", "Draw the AGENTS list as a git graph of its tabs; off: just the tab names over their agents"),
        {
          kind: "choice", label: "agents list", value: sidebar.agents || "modisa's", about: "A plugin whose sidebar section takes the AGENTS list's place",
          step: (by) => save(app, "sidebar", "agents", cycle(takers, app.cfg.sidebar.agents, by)),
        },
        { kind: "heading", label: "status row" },
        toggle(app, "agent counts", "status", "agents", "How many agents are working and how many need you; click one to list them"),
        toggle(app, "pane count", "status", "panes", "How many panes this tab has; click it to switch pane"),
        toggle(app, "theme name", "status", "theme", "The theme in use; click it to change theme"),
        { kind: "heading", label: "panes" },
        { kind: "choice", label: "border style", value: panes.border, about: "The line around each pane", step: (by) => save(app, "panes", "border", cycle(BORDERS, app.cfg.panes.border, by)) },
        toggle(app, "agent in the border title", "pane_labels", "agent", "The agent and what it's doing, in its pane's border"),
      ];
    },
  };
}

function git(app: App): Section {
  return {
    name: "git",
    rows: () => [
      { kind: "heading", label: "in the status row" },
      toggle(app, "branch", "git", "status", "The branch of the repository the active space's focused pane is in; green when clean and in step with its upstream"),
      toggle(app, "repository name", "git", "repo", "The repository's name (its folder), before the branch"),
      toggle(app, "commits to push and pull", "git", "counts", "↑ commits to push and ↓ commits to pull, against the branch's upstream"),
      toggle(app, "changed files", "git", "changes", "● files changed, staged or not, and new files"),
    ],
  };
}

function indicators(app: App): Section {
  const place = (label: string, key: "tab" | "pane" | "sidebar", about: string): Row => toggle(app, label, "indicators", key, about);
  return {
    name: "indicators",
    rows: () => [
      { kind: "heading", label: "style" },
      ...STYLES.map((style): Row => ({ kind: "radio", label: `${style.padEnd(9)} ${Object.values(INDICATORS[style]).join(" ")}`, about: "How an agent's state is drawn: needs you, working, done, idle", current: app.cfg.indicators.style === style, apply: () => save(app, "indicators", "style", style) })),
      { kind: "heading", label: "show in" },
      place("tab bar badge", "tab", "A mark on each tab with an agent that needs you"),
      place("pane border title", "pane", "The agent's state in its pane's border"),
      place("sidebar", "sidebar", "The agent's state next to its name"),
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
      { kind: "heading", label: "when an agent…" },
      ...EVENTS.map(([event, label]): Row => {
        const kinds = app.cfg.notify[event];
        const value = kinds.includes("sound") ? app.cfg.sound[event] : "off";
        return {
          kind: "choice", label, value, enter: () => play(value), about: "←→ to hear the others; ↵ plays it again",
          step: (by) => {
            const next = options[(options.indexOf(value) + by + options.length) % options.length]!;
            if (next === "off") return save(app, "notify", event, kinds.filter((k) => k !== "sound"));
            if (!kinds.includes("sound")) save(app, "notify", event, [...kinds, "sound"]);
            save(app, "sound", event, next);
            play(next);
          },
        };
      }),
      { kind: "heading", label: "level" },
      {
        kind: "choice", label: "volume", value: `${Math.round(app.cfg.sound.volume * 100)}%`, about: "For every sound modisa plays",
        enter: () => play(app.cfg.sound.blocked),
        step: (by) => {
          const volume = clamp(Math.round(app.cfg.sound.volume * 10 + by) / 10, 0, 1);
          save(app, "sound", "volume", volume);
          play(app.cfg.sound.blocked, volume);
        },
      },
    ],
  };
}

function alerts(app: App): Section {
  return {
    name: "alerts",
    rows: () => EVENTS.flatMap(([event]): Row[] => [
      { kind: "heading", label: { blocked: "when an agent needs you", done: "when an agent is done", working: "when an agent starts working" }[event] },
      ...ALERTS.map(([kind, name, about]): Row => {
        const kinds = app.cfg.notify[event];
        return { kind: "toggle", label: name, on: kinds.includes(kind), about: `${about}. Only for agents you're not looking at`, flip: () => save(app, "notify", event, kinds.includes(kind) ? kinds.filter((k) => k !== kind) : [...kinds, kind]) };
      }),
    ]),
  };
}

// What agents may do to panes they didn't start, and how much they may message each other.
function agents(app: App): Section {
  type Permissions = typeof app.cfg.permissions;
  type Messaging = typeof app.cfg.messaging;
  const policy = (label: string, key: keyof Permissions, about: string): Row => ({
    kind: "choice", label, value: app.cfg.permissions[key], about: `${about}. ask: you're asked each time`,
    step: (by) => save(app, "permissions", key, cycle(POLICIES, app.cfg.permissions[key], by)),
  });
  const number = (label: string, key: keyof Messaging, lo: number, hi: number, about: string): Row => ({
    kind: "choice", label, value: String(app.cfg.messaging[key]), about,
    step: (by) => save(app, "messaging", key, clamp(app.cfg.messaging[key] + by, lo, hi)),
  });
  return {
    name: "agents",
    rows: () => [
      { kind: "heading", label: "on panes an agent didn't start" },
      policy("send keys", "keys_foreign", "Typing into another pane"),
      policy("run commands", "run_foreign", "Running a command in another pane"),
      policy("close panes", "close_foreign", "Closing another pane"),
      { kind: "heading", label: "messaging" },
      number("reply chain limit", "max_hops", 1, 50, "Agents replying to each other stop after this many messages"),
      number("messages a minute", "per_minute", 1, 60, "From one agent to another, at most"),
    ],
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
  const about = "State is read from every agent's screen; an integration adds its session, so it resumes after a restart";
  return {
    name: "integrations",
    enter: () => { void load(); },
    rows: () => {
      if (!list) return [{ kind: "heading", label: "checking…" }];
      const todo = list.filter((i) => i.status === "outdated" || (i.available && i.status === "none"));
      const rows: Row[] = todo.length ? [{
        kind: "action", label: `Install all (${todo.length})`, status: todo.map((i) => i.name).join(", "), tone: "accent", hint: "↵ install", about,
        run: () => { if (!busy) void apply(todo.map((i) => i.id), () => true); },
      }] : [];
      for (const i of [...list].sort((a, b) => order(a) - order(b))) rows.push({
        kind: "action", label: i.name, about,
        status: busy === i.id ? "working…" : i.status === "current" ? "✓ installed" : i.status === "outdated" ? "↻ update available" : i.available ? "+ available" : "not found",
        tone: i.status === "current" ? "ok" : i.status === "outdated" ? "warn" : i.available ? "accent" : "dim",
        note: i.kind === "lifecycle" ? "state + session" : "session",
        hint: i.status === "current" ? "↵ remove" : i.status === "outdated" ? "↵ update" : "↵ install",
        run: () => { if (!busy) void apply([i.id], () => i.status !== "current"); },
      });
      return rows;
    },
  };
}

export function sections(app: App, repaint: () => void): Section[] {
  return [theme(app), general(app), layout(app), git(app), indicators(app), sound(app), alerts(app), agents(app), integrations(app, repaint)];
}
