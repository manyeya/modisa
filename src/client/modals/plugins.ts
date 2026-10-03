// The plugin manager (prefix P, "Plugins…" in the palette, or the settings page): discover plugins in the index and
// your marketplaces, install one once you've seen exactly where it comes from, and start, stop, restart, read the log
// of, update and remove what's installed. It asks the server for all of it (plugin.*, marketplace.*), so with --remote
// it acts on the server's machine, where plugins run. Each view is a list; an action runs behind a spinner (esc hides
// it, and the result is still toasted), then its view opens again with what changed.
import type { z } from "zod";
import type { results } from "../../protocol/schema";
import type { PluginStatus } from "../../protocol/types";
import type { App } from "../context";
import { fit } from "../design";
import { ask, confirm } from "./confirm";
import { blank, clear, footer, frame, frameLayouts, header, innerWidth, open, row, text } from "./frame";
import { list, type ListButton, type ListItem } from "./pick";
import { prompt } from "./prompt";

type Of<M extends keyof typeof results> = z.infer<(typeof results)[M]>;
type Step = (() => Promise<Step | null>) | null; // the view to show next; null closes the manager
export type PluginsView = "menu" | "discover" | "installed" | "marketplaces" | "url";

export async function openPlugins(app: App, start: PluginsView = "menu") {
  const views: Record<PluginsView, () => Promise<Step>> = { menu: () => menu(app), discover: () => discover(app), installed: () => installed(app), marketplaces: () => marketplaces(app), url: () => fromUrl(app) };
  for (let step: Step = views[start]; step; ) step = await step();
}

// ---------- waiting, and saying what happened ----------
const SPIN = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";
type Outcome<T> = { ok: true; value: T } | { ok: false; error: Error };

// `work` behind a spinner, until it settles: its outcome, or null if the user hid it (esc). Hidden work carries on,
// and `report` still toasts how it went.
async function busy<T>(app: App, title: string, what: string, work: Promise<T>, report: (value: T) => void = () => {}): Promise<Outcome<T> | null> {
  const settled = work.then((value): Outcome<T> => ({ ok: true, value }), (error): Outcome<T> => ({ ok: false, error }));
  const box = frame(app, 6, 64);
  const place = frameLayouts.get(box)!;
  let frameNo = 0;
  const paint = () => {
    if (box.isDestroyed) return;
    place();
    clear(box);
    header(app, box, title);
    blank(app, box);
    text(app, row(app, box), fit(`${SPIN[frameNo++ % SPIN.length]} ${what}`, innerWidth(box)), app.th.fg);
    footer(app, box, [["esc", "hide: it carries on"]]);
  };
  frameLayouts.set(box, paint);
  paint();
  const timer = setInterval(paint, 100);
  let close: (v: null) => void = () => {};
  const hidden = open<null>(app, box, (done) => {
    close = done;
    return () => true;
  }).then(() => null);
  const first = await Promise.race([settled, hidden]);
  clearInterval(timer);
  close(null);
  if (first) return first;
  void settled.then((o) => (o.ok ? report(o.value) : failed(app, o.error)));
  return null;
}

const failed = (app: App, e: unknown) => app.toast(String((e as Error)?.message ?? e), app.th.blocked);
const short = (commit?: string | null) => commit?.slice(0, 7) ?? "";
const request = <M extends keyof typeof results>(app: App, method: M, params: object = {}) => app.conn.request<Of<M>>(method, params);
const where = (app: App) => (app.opts.remote ? "on the server's machine" : "on this machine");

// ---------- the views ----------
async function menu(app: App): Promise<Step> {
  const v = await list(app, {
    title: "Plugins",
    meta: app.opts.remote ? "on the server's machine" : "",
    width: 80,
    items: [
      { name: "Discover", description: "plugins in the index and in your marketplaces", value: "discover" },
      { name: "Installed", description: "start, stop, restart, logs, update, remove", value: "installed" },
      { name: "Marketplaces", description: "repositories that list plugins: add, update, remove", value: "marketplaces" },
      { name: "Add from URL…", description: "install a plugin from a git URL", value: "url" },
    ],
  });
  return v === "discover" ? () => discover(app) : v === "installed" ? () => installed(app) : v === "marketplaces" ? () => marketplaces(app) : v === "url" ? () => fromUrl(app) : null;
}

// The index's plugins and the marketplaces' (or one marketplace's), searchable; choosing one installs it.
async function discover(app: App, only?: string): Promise<Step> {
  const back: Step = only ? () => marketplaces(app) : () => menu(app);
  const out = await busy(app, "Plugins", only ? `Reading marketplace ${only}…` : "Reading the plugin index and your marketplaces…", request(app, "plugin.catalog"));
  if (!out) return null;
  if (!out.ok) return failed(app, out.error), back;
  const cat = out.value;
  const listed = cat.marketplaces.filter((p) => !only || p.marketplace === only);
  const index = only ? [] : cat.index.results;
  const items: ListItem[] = [
    ...listed.map((p) => ({ name: p.name, description: `@${p.marketplace}${p.description ? ` · ${p.description}` : ""}`, key: p.installed ? "✓ installed" : "", value: `m:${p.name}@${p.marketplace}` })),
    ...index.map((r, i) => ({ name: r.name, description: `★${r.stars} ${r.repo}${r.description ? ` · ${r.description}` : ""}`, key: r.archived ? "archived" : r.installed ? "✓ installed" : "", value: `i:${i}` })),
  ];
  if (!only && cat.index.error) items.push({ name: "(the plugin index)", description: cat.index.error, value: "" });
  if (!items.length) items.push({ name: only ? `(marketplace ${only} lists no plugins)` : "(nothing found)", description: only ? "" : "add a marketplace, or search the index again later", value: "" });
  const v = await list(app, { title: only ? `Marketplace ${only}` : "Discover plugins", meta: `${listed.length + index.length} · none of them vetted`, items, width: 100, rows: 16 });
  if (v === null) return back;
  if (!v) return () => discover(app, only);
  const from = v.startsWith("m:") ? { marketplacePlugin: v.slice(2) } : { source: index[+v.slice(2)]!.source };
  const name = v.startsWith("m:") ? v.slice(2) : index[+v.slice(2)]!.repo;
  return (await install(app, from, name)) ? () => installed(app) : () => discover(app, only);
}

// Looked up first (the exact source and the commit its ref is at), shown with a warning, and installed only if the user
// says so, at that commit: if the ref has moved meanwhile, the install refuses.
async function install(app: App, from: { source?: string; marketplacePlugin?: string; ref?: string }, name: string) {
  const seen = await busy(app, `Install ${name}`, `Looking up ${from.marketplacePlugin ?? from.source}…`, request(app, "plugin.resolve", from));
  if (!seen) return false;
  if (!seen.ok) return failed(app, seen.error), false;
  const r = seen.value;
  const width = Math.min(100, Math.max(60, app.r.width - 4));
  const room = width - 4 - 9; // the border and padding, and the labels
  const wrap = (label: string, value: string) => (value.match(new RegExp(`.{1,${room}}`, "g")) ?? [""]).map((part, i) => `${i ? "" : label}`.padEnd(9) + part);
  const body = [
    `${r.name ?? name} will run unsandboxed, with your permissions, ${where(app)}:`,
    "your files, your network, your credentials. Nothing has vetted it.",
    "",
    ...wrap("from", r.from),
    ...(r.subdir ? wrap("folder", r.subdir) : []),
    ...wrap("ref", r.ref ?? "the default branch"),
    ...wrap("commit", r.commit ?? "(known once fetched: an abbreviated commit)"),
    ...(r.marketplace ? wrap("listed", `by marketplace ${r.marketplace}`) : []),
  ].join("\n");
  const go = await ask(app, `Install ${r.name ?? name}?`, body, [{ label: "Cancel", key: "n", value: false }, { label: "Install", key: "y", value: true, tone: "primary" }], 0, width);
  if (!go) return false;
  const work = request(app, "plugin.install", { ...from, ...(r.commit && { commit: r.commit }) });
  const done = await busy(app, `Installing ${r.name ?? name}`, `Fetching ${r.from}${r.commit ? ` at ${short(r.commit)}` : ""}…`, work, (x) => installedToast(app, x));
  if (!done) return false;
  if (!done.ok) return failed(app, done.error), false;
  installedToast(app, done.value);
  return done.value.installed || !!done.value.alreadyInstalled;
}

function installedToast(app: App, r: Of<"plugin.install">) {
  if (!r.installed && !r.alreadyInstalled) return app.toast(`not installed (${r.stage}): ${r.reason}`, app.th.blocked);
  const s = r.start;
  const state = !s ? "" : s.state === "started" ? `, running (pid ${s.pid})` : s.state === "already-running" ? ", already running" : `, but ${s.state === "no-hello" ? "it never connected" : `it didn't start: ${s.reason}`}`;
  app.toast(`${r.alreadyInstalled ? "already installed" : "installed"} ${r.name} @${short(r.commit)}${state}`, s && s.state !== "started" && s.state !== "already-running" ? app.th.warn : app.th.done);
  for (const hint of r.hints ?? []) app.toast(hint, app.th.warn);
}

const status = (p: PluginStatus) => (p.status === "running" ? `running · pid ${p.pid}` : p.status === "failed" ? `failed${p.error ? `: ${p.error}` : ""}` : p.status === "exited" ? `exited ${p.exitCode ?? ""}`.trim() : p.status);
const origin = (p: PluginStatus) =>
  p.install ? `${p.install.marketplace ? `@${p.install.marketplace} · ` : ""}${p.install.source.replace(/^[a-z]+:\/\//, "").replace(/\.git$/, "")}${p.install.ref ? ` ${p.install.ref}` : ""} @${short(p.install.commit)}`
  : p.source === "config" ? "from config.toml" : `linked ${p.dir ?? ""}`;
const live = (p: PluginStatus) => p.status === "running" || p.status === "starting";

// What can be done to one: a [[plugin]] line from config.toml only starts and stops
function actionsOf(p: PluginStatus): ListButton[] {
  const toggle: ListButton = live(p) ? { icon: "■", value: `stop:${p.name}`, key: "t", title: "stop" } : { icon: "▶", value: `start:${p.name}`, key: "t", title: "start" };
  if (p.source === "config") return [toggle];
  return [
    toggle,
    { icon: "↻", value: `restart:${p.name}`, key: "r", title: "restart" },
    { icon: "≡", value: `logs:${p.name}`, key: "l", title: "logs" },
    ...(p.install ? [{ icon: "⇣", value: `update:${p.name}`, key: "g", title: "update" }] : []),
    { icon: "✕", value: `remove:${p.name}`, key: "d", title: "remove", danger: true },
  ];
}

async function installed(app: App): Promise<Step> {
  let ps: PluginStatus[];
  try {
    ps = await request(app, "plugin.list");
  } catch (e) {
    return failed(app, e), () => menu(app);
  }
  const items: ListItem[] = ps.map((p) => ({ name: p.name, description: `${status(p)} · ${origin(p)}`, key: live(p) ? "● on" : p.status === "failed" ? "✕ failed" : "○ off", value: `open:${p.name}`, buttons: actionsOf(p) }));
  items.push({ name: "+ Discover plugins", description: "in the index and your marketplaces", value: "discover" }, { name: "+ Add from URL…", description: "install a plugin from a git URL", value: "url" });
  const v = await list(app, { title: "Installed plugins", meta: `${ps.length} ${where(app)}`, items, width: 100, rows: 14 });
  if (v === null) return () => menu(app);
  if (v === "discover") return () => discover(app);
  if (v === "url") return () => fromUrl(app);
  const [verb, name] = v.split(":") as [string, string];
  const p = ps.find((x) => x.name === name)!;
  if (verb === "open") {
    // every action, by name: what the row's buttons do
    const chosen = await list(app, { title: `${name}: ${status(p)}`, items: actionsOf(p).map((b) => ({ name: b.title[0]!.toUpperCase() + b.title.slice(1), description: `^${b.key} in the list`, value: b.value, danger: b.danger })) });
    return chosen ? () => act(app, chosen, p) : () => installed(app);
  }
  return () => act(app, v, p);
}

async function act(app: App, value: string, p: PluginStatus): Promise<Step> {
  const verb = value.slice(0, value.indexOf(":"));
  const { name } = p;
  const again = () => installed(app);
  const said = (s: PluginStatus) => app.toast(`${name}: ${status(s)}`, s.status === "running" ? app.th.done : s.status === "failed" ? app.th.blocked : app.th.fg);
  switch (verb) {
    case "start":
    case "stop": {
      const out = await busy(app, name, verb === "start" ? `Starting ${name}…` : `Stopping ${name}…`, app.conn.request<PluginStatus>(`plugin.${verb}`, { name }), said);
      if (out) out.ok ? said(out.value) : failed(app, out.error);
      return again;
    }
    case "restart": {
      const work = app.conn.request("plugin.stop", { name }).then(() => app.conn.request<PluginStatus>("plugin.start", { name }));
      const out = await busy(app, name, `Restarting ${name}…`, work, said);
      if (out) out.ok ? said(out.value) : failed(app, out.error);
      return again;
    }
    case "logs": {
      let log: Of<"plugin.logs">;
      try {
        log = await request(app, "plugin.logs", { name, lines: 500 });
      } catch (e) {
        return failed(app, e), again;
      }
      const lines = log.text ? log.text.split("\n") : ["(empty)"];
      await list(app, { title: `${name}: its log`, meta: log.log.split("/").slice(-2).join("/"), items: lines.map((l) => ({ name: l || " ", value: "" })), width: 120, rows: 24, selected: lines.length - 1, placeholder: "Type to search the log" });
      return again;
    }
    case "update": {
      const report = (r: Of<"plugin.update">) =>
        r.stage ? app.toast(`${name} not updated (${r.stage}): ${r.reason}`, app.th.blocked)
        : r.upToDate ? app.toast(`${name} is up to date (@${short(r.commit)})`, app.th.done)
        : app.toast(`updated ${name} ${short(r.from)} → ${short(r.commit)}${r.restarted.length ? `; restarted (${r.restarted.map((s) => s.state).join(", ")})` : ""}`, r.restarted.every((s) => s.state === "started") ? app.th.done : app.th.warn);
      const out = await busy(app, name, `Fetching the latest of ${name}…`, request(app, "plugin.update", { name }), report);
      if (out) out.ok ? report(out.value) : failed(app, out.error);
      return again;
    }
    case "remove": {
      const managed = !!p.install;
      const yes = await confirm(app, `Remove ${name}`, managed ? `Stop ${name} in every session and delete its checkout?\nIts data and logs are kept.` : `Unlink ${name} and stop it here?\nIts directory (${fit(p.dir ?? "", 40)}) is untouched.`, "remove");
      if (!yes) return again;
      const report = (r: Of<"plugin.unlink">) => app.toast(`removed ${name}${r.checkout && !r.checkout.deleted ? `; its checkout is kept: ${[...r.stillUsing, ...r.unreachable].join(", ")} still use it` : ""}`, app.th.done);
      const out = await busy(app, name, `Removing ${name}…`, request(app, "plugin.unlink", { name }), report);
      if (out) out.ok ? report(out.value) : failed(app, out.error);
      return again;
    }
  }
  return again;
}

async function marketplaces(app: App): Promise<Step> {
  let ms: Of<"marketplace.list">;
  try {
    ms = await request(app, "marketplace.list");
  } catch (e) {
    return failed(app, e), () => menu(app);
  }
  const items: ListItem[] = ms.map((m) => ({
    name: m.name,
    description: `${m.error ? m.error : `${m.plugins} plugin${m.plugins === 1 ? "" : "s"}`} · ${m.source.replace(/^[a-z]+:\/\//, "")} @${short(m.commit)}${m.description ? ` · ${m.description}` : ""}`,
    value: `open:${m.name}`,
    buttons: [{ icon: "↻", value: `update:${m.name}`, key: "r", title: "update" }, { icon: "✕", value: `remove:${m.name}`, key: "d", title: "remove", danger: true }],
  }));
  items.push({ name: "+ Add marketplace…", description: "owner/repo on GitHub, or a git URL", value: "add" });
  const v = await list(app, { title: "Marketplaces", meta: `${ms.length} ${where(app)}`, items, width: 100 });
  if (v === null) return () => menu(app);
  const again = () => marketplaces(app);
  if (v === "add") {
    const source = (await prompt(app, "Add a marketplace", "", "owner/repo, or a git URL"))?.trim();
    if (!source) return again;
    const report = (r: Of<"marketplace.add">) =>
      r.stage ? app.toast(`marketplace not added (${r.stage}): ${r.reason}`, app.th.blocked)
      : app.toast(`${r.alreadyAdded ? "already added" : "added"} marketplace ${r.name}: ${r.plugins?.length ?? 0} plugins`, app.th.done);
    const out = await busy(app, "Marketplaces", `Fetching ${source}…`, request(app, "marketplace.add", { source }), report);
    if (out) out.ok ? report(out.value) : failed(app, out.error);
    return again;
  }
  const [verb, name] = v.split(":") as [string, string];
  if (verb === "open") return () => discover(app, name);
  if (verb === "update") {
    const report = (rs: Of<"marketplace.update">) => rs.forEach((r) => (r.reason ? app.toast(`marketplace ${r.name} not updated: ${r.reason}`, app.th.blocked) : app.toast(r.updated ? `updated marketplace ${r.name}: ${short(r.from)} → ${short(r.commit)}` : `marketplace ${r.name} is up to date`, app.th.done)));
    const out = await busy(app, "Marketplaces", `Fetching the latest of ${name}…`, request(app, "marketplace.update", { name }), report);
    if (out) out.ok ? report(out.value) : failed(app, out.error);
  } else if (verb === "remove" && (await confirm(app, `Remove ${name}`, `Remove marketplace ${name}?\nPlugins installed from it stay installed.`, "remove"))) {
    try {
      const r = await request(app, "marketplace.remove", { name });
      app.toast(`removed marketplace ${name}${r.installed.length ? `; still installed from it: ${r.installed.join(", ")}` : ""}`, app.th.done);
    } catch (e) {
      failed(app, e);
    }
  }
  return again;
}

async function fromUrl(app: App): Promise<Step> {
  const source = (await prompt(app, "Install from a git URL", "", "https://…, ssh://…, git@host:path or file://…"))?.trim();
  if (!source) return () => menu(app);
  const ref = await prompt(app, "Branch, tag or commit", "", "optional: empty for the default branch");
  if (ref === null) return () => menu(app);
  const name = source.replace(/\/+$/, "").replace(/\.git$/, "").split(/[/:]/).pop() || source;
  return (await install(app, { source, ...(ref.trim() && { ref: ref.trim() }) }, name)) ? () => installed(app) : () => menu(app);
}
