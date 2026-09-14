// Everything the user can do by name: prefix keys, the command palette, menus and buttons all run these.
import { panes as treePanes } from "../core/layout";
import { self } from "../core/paths";
import { ensureConfigFile } from "../config/config";
import { checkForUpdate, updateCommand } from "../cli/update";
import { VERSION } from "../core/version";
import type { Action, App, Option } from "./context";
import { bindings } from "./input/bindings";
import { jump } from "./input/copy-mode";
import { confirm } from "./modals/confirm";
import { contextMenu } from "./modals/context-menu";
import { fit } from "./design";
import { pick } from "./modals/pick";
import { prompt } from "./modals/prompt";
import { openSettings } from "./modals/settings";
import { reload } from "./notify";
import { quit } from "./connection";
import { render } from "./render";
import { deleteSpace, renameSpace } from "./spaces";

export function createActions(app: App): Record<string, Action> {
  const { r } = app;
  const agentList = async () => (await app.conn.request<{ id: string; name: string }[]>("adapters")).map((a) => ({ name: a.name, description: a.id, value: a.id }));

  const actions: Record<string, Action> = {
    "theme-picker": { label: "Change theme", run: () => openSettings(app, "theme") },
    help: { label: "Keyboard guide", run: () => pick(app, "KEYBOARD / prefix " + app.cfg.prefix, Object.entries(bindings).map(([key, action]) => ({ name: `${app.cfg.prefix}  ${key}`, description: actions[action]?.label ?? action, value: action }))).then((action) => { if (action && action !== "help") actions[action]?.run(); }) },
    "pane-menu": { label: "Pane context menu", run: () => contextMenu(app, app.tab().focused, Math.min(r.width - 34, app.area().x + 3), app.area().y + 1) },
    "pane-picker": { label: "Switch pane", run: async () => {
      const id = await pick(app, "PANES", app.view!.panes.map((p) => ({ name: p.name ? "@" + p.name : p.title, description: `${p.id} · ${p.agent?.state ?? p.status} · ${p.cwd}`, value: p.id })));
      if (id) app.call("focusPane", { pane: id });
    } },
    "working-agents": { label: "Agents working", run: () => pickAgents(app, "working", "WORKING") },
    "blocked-agents": { label: "Agents that need you", run: () => pickAgents(app, "blocked", "NEEDS YOU") },
    "split-right": { label: "Split right", run: () => app.call("split", { dir: "row" }) },
    "split-down": { label: "Split down", run: () => app.call("split", { dir: "col" }) },
    "focus-left": { label: "Focus left", run: () => app.call("focusDir", { dir: "left" }) },
    "focus-right": { label: "Focus right", run: () => app.call("focusDir", { dir: "right" }) },
    "focus-up": { label: "Focus up", run: () => app.call("focusDir", { dir: "up" }) },
    "focus-down": { label: "Focus down", run: () => app.call("focusDir", { dir: "down" }) },
    "resize-left": { label: "Resize left", run: () => app.call("resize", { dir: "left" }) },
    "resize-right": { label: "Resize right", run: () => app.call("resize", { dir: "right" }) },
    "resize-up": { label: "Resize up", run: () => app.call("resize", { dir: "up" }) },
    "resize-down": { label: "Resize down", run: () => app.call("resize", { dir: "down" }) },
    zoom: { label: "Zoom pane", run: () => app.call("zoom") },
    "close-pane": { label: "Close pane", run: () => app.call("close") },
    "close-tab": { label: "Close tab", run: () => app.call("closeTab") },
    "new-tab": { label: "New tab", run: () => app.call("newTab") },
    "next-tab": { label: "Next tab", run: () => app.call("cycleTab", { step: 1 }) },
    "prev-tab": { label: "Previous tab", run: () => app.call("cycleTab", { step: -1 }) },
    "workspace-picker": {
      label: "Switch space",
      run: async () => {
        const v = await pick(app, "spaces", [
          ...app.view!.workspaces.map((w, i) => ({ name: w.name, description: `${w.tabs.length} tabs · ${w.tabs.reduce((n, t) => n + treePanes(t.tree).length, 0)} panes`, value: String(i) })),
          { name: "+ new space", description: "A fresh group of tabs and panes", value: "new" },
        ]);
        if (v === "new") actions["new-workspace"]!.run();
        else if (v !== null) app.call("selectWorkspace", { index: +v });
      },
    },
    "new-workspace": {
      label: "New space",
      run: async () => {
        const name = await prompt(app, "new space — name", `space ${app.view!.workspaces.length + 1}`);
        if (name?.trim()) app.call("newWorkspace", { cwd: app.ws().cwd, name: name.trim() });
      },
    },
    "new-agent": {
      label: "New agent pane",
      run: async () => {
        const h = await pick(app, "new agent", await agentList());
        if (!h) return;
        const name = await prompt(app, "name (optional, for @addressing)");
        app.call("spawnAgent", { harness: h, name: name || undefined });
      },
    },
    "toggle-sidebar": { label: "Toggle sidebar", run: () => { app.sidebar = !app.sidebar; app.conn.notify("area", { area: app.area() }); render(app); } },
    "copy-mode": { label: "Copy mode / scrollback", run: () => { app.mode = "copy"; render(app); } },
    search: {
      label: "Search scrollback",
      run: async () => {
        const q = await prompt(app, "search");
        const p = app.focusedPane();
        if (!q || !p) return;
        const res = await app.conn.request<{ total: number; matches: number[] }>("search", { pane: p.id, query: q });
        if (!res.matches.length) return app.toast(`no match for "${q}"`, app.th.warn);
        app.search = { ...res, i: res.matches.length - 1 };
        app.mode = "copy";
        jump(app);
      },
    },
    palette: {
      label: "Command palette",
      run: async () => {
        const extra: Option[] = [
          ...(await agentList()).map((a) => ({ name: `New agent: ${a.name}`, description: a.value, value: `agent:${a.value}` })),
          { name: "Kill session", description: "close every pane and stop the server", value: "kill" },
        ];
        const v = await pick(app, "commands", [
          ...Object.entries(actions).filter(([k]) => k !== "palette" && !k.startsWith("agent-")).map(([k, a]) => ({ name: a.label, description: Object.entries(bindings).find(([, b]) => b === k)?.[0] ?? "", value: k })),
          ...extra,
        ]);
        if (!v) return;
        if (v.startsWith("agent:")) return app.call("spawnAgent", { harness: v.slice(6) });
        if (v === "kill") return app.conn.request("kill");
        actions[v]?.run();
      },
    },
    settings: { label: "Settings", run: () => openSettings(app) },
    "edit-config": {
      label: "Edit config.toml",
      run: async () => app.call("newTab", { name: "settings", command: `${Bun.env.EDITOR || "vi"} ${await ensureConfigFile()}`, ephemeral: true }),
    },
    "reload-config": { label: "Reload config", run: () => reload(app, true) },
    "update-shepherd": {
      label: "Update shepherd",
      run: async () => {
        const m = app.update ?? (await checkForUpdate(true));
        if (!m) return app.toast(`shepherd ${VERSION} is up to date`, app.th.done);
        const managed = updateCommand(); // Homebrew or mise installed it: their command, not ours
        if (managed !== "shepherd update") return app.toast(`shepherd ${m.version} is out: run ${managed}, then shepherd restart`, app.th.warn);
        const notes = m.notes.trim().split("\n").filter(Boolean).slice(0, 6).map((l) => fit(l, 60));
        const ok = await confirm(app, `UPDATE / ${m.version}`, [`shepherd ${VERSION} → ${m.version}`, ...(notes.length ? ["", ...notes] : []), "", "Downloads it, then restarts the server; agents resume."].join("\n"), "update and restart");
        if (!ok) return;
        const cmd = self().join(" ");
        app.call("newTab", { name: "update", command: `${cmd} update && ${cmd} restart`, ephemeral: true });
      },
    },
    "restart-server": { label: "Restart server (load updated shepherd; panes are restored)", run: () => { app.restartedByUs = app.restarting = true; app.conn.request("restart").catch(() => {}); } },
    "toggle-messaging": { label: "Pause/resume agent messaging", run: () => app.call("pause") },
    "message-log": { label: "Message log", run: () => app.call("newTab", { name: "messages", command: `${self().join(" ")} messages --follow`, ephemeral: true }) },
    "send-message": {
      label: "Send message to agent",
      run: async () => {
        const agents = app.view!.panes.filter((p) => p.agent || p.harness);
        if (!agents.length) return app.toast("no agent panes", app.th.warn);
        const to = await pick(app, "send to", agents.map((p) => ({ name: p.name ? "@" + p.name : p.title, description: p.agent?.state ?? "", value: p.id })));
        if (!to) return;
        const body = await prompt(app, "message");
        if (body) app.conn.request("send", { to, body }).then(() => app.toast("queued"), (e) => app.toast(e.message, app.th.blocked));
      },
    },
    "rename-tab": { label: "Rename tab", run: async () => { const n = await prompt(app, "rename tab", app.tab().name ?? ""); if (n !== null) app.call("renameTab", { name: n }); } },
    "rename-pane": { label: "Rename pane (@name)", run: async () => { const n = await prompt(app, "rename pane", app.info(app.tab().focused)?.name ?? ""); if (n !== null) app.call("renamePane", { name: n }); } },
    "rename-workspace": { label: "Rename space", run: () => renameSpace(app, app.view!.active) },
    "delete-workspace": { label: "Delete space", run: () => deleteSpace(app, app.view!.active) },
    detach: { label: "Detach", run: () => quit(app, "detached") },
    ...Object.fromEntries([1, 2, 3, 4, 5, 6, 7, 8, 9].map((n) => [`agent-${n}`, { label: `Jump to agent ${n}`, run: () => { const p = app.sortedAgents()[n - 1]; if (p) app.call("focusPane", { pane: p.id }); } }])),
  };

  // UI events do not await actions. Own their promises so a disconnect during an
  // adapter lookup, search, or dialog cannot become an unhandled rejection.
  for (const [name, action] of Object.entries(actions)) {
    const run = action.run;
    action.run = () => {
      if (app.quitting || (app.conn?.closed && name !== "detach")) return;
      const failed = (error: any) => {
        if (!app.quitting && !app.conn?.closed) app.toast(String(error.message ?? error), app.th.blocked);
      };
      try { return Promise.resolve(run()).catch(failed); }
      catch (error) { failed(error); }
    };
  }
  return actions;
}

// Only the agents in one state; picking one jumps to it.
async function pickAgents(app: App, state: "working" | "blocked", title: string) {
  const agents = app.sortedAgents().filter((p) => p.agent!.state === state);
  if (!agents.length) return app.toast(state === "working" ? "no agents are working" : "no agents need you", app.th.dim);
  const where = (id: string) => app.view!.workspaces.find((w) => w.tabs.some((t) => treePanes(t.tree).includes(id)))?.name ?? "";
  const id = await pick(app, title, agents.map((p) => ({ name: p.name ? "@" + p.name : p.title, description: `${p.agent!.harness} · ${where(p.id)}`, value: p.id })));
  if (id) app.call("focusPane", { pane: id });
}
