// Everything the user can do by name: prefix keys, the command palette, menus and buttons all run these.
import type { MouseEvent } from "@opentui/core";
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
import { list, pick } from "./modals/pick";
import { prompt } from "./modals/prompt";
import { openSettings } from "./modals/settings";
import { reload } from "./notify";
import { quit } from "./connection";
import { render } from "./render";
import { deleteSpace, renameSpace, spaceMenu } from "./spaces";
import { pluginKey, pluginKeys, pluginUi, runPluginAction } from "./plugin-ui";

export function createActions(app: App): Record<string, Action> {
  const { r } = app;
  const agentList = async () => (await app.conn.request<{ id: string; name: string }[]>("adapters")).map((a) => ({ name: a.name, description: a.id, value: a.id }));

  const actions: Record<string, Action> = {
    "theme-picker": { label: "Change theme", run: () => openSettings(app, "theme") },
    help: {
      label: "Keyboard guide",
      run: async () => {
        // plugins' keys as this client's config binds them
        const plugins = pluginKeys(app).map((k) => ({
          name: `${k.plugin}: ${k.description}`,
          description: k.state === "active" ? "" : `off: ${k.reason}`,
          key: k.key || "–",
          value: k.state === "active" ? `plugin-key:${k.key}` : "",
        }));
        const action = await pick(app, "Keyboard", [...Object.entries(bindings).map(([key, action]) => ({ name: actions[action]?.label ?? action, description: "", key, value: action })), ...plugins], `prefix ${app.cfg.prefix}`);
        if (action?.startsWith("plugin-key:")) pluginKey(app, action.slice("plugin-key:".length));
        else if (action && action !== "help") actions[action]?.run();
      },
    },
    "pane-menu": { label: "Pane context menu", run: () => contextMenu(app, app.tab().focused, Math.min(r.width - 34, app.area().x + 3), app.area().y + 1) },
    "pane-picker": { label: "Switch pane", run: async () => {
      const id = await pick(app, "Panes", app.view!.panes.map((p) => ({ name: p.name ? "@" + p.name : p.title, description: `${p.id} · ${p.agent?.state ?? p.status} · ${p.cwd}`, value: p.id })));
      if (id) app.call("focusPane", { pane: id });
    } },
    "working-agents": { label: "Agents working", run: () => pickAgents(app, "working", "Working agents") },
    "blocked-agents": { label: "Agents that need you", run: () => pickAgents(app, "blocked", "Agents that need you") },
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
        const spaces = app.view!.workspaces;
        const v = await list(app, {
          title: "Spaces",
          items: [
            ...spaces.map((w, i) => ({
              name: `${i === app.view!.active ? "● " : "  "}${w.name}`,
              description: `${w.git ? `⎇ ${w.git.branch} · ` : ""}${plural(w.tabs.length, "tab")} · ${plural(w.tabs.reduce((n, t) => n + treePanes(t.tree).length, 0), "pane")}`,
              value: String(i),
              context: (e: MouseEvent) => spaceMenu(app, i, e.x, e.y), // rename, delete
              // the last space can't be deleted, so it offers no ✕
              buttons: [
                { icon: "✎", value: `rename:${i}`, key: "r", title: "rename" },
                ...(spaces.length > 1 ? [{ icon: "✕", value: `delete:${i}`, key: "d", title: "delete", danger: true }] : []),
              ],
            })),
            { name: "+ new space", description: "A fresh group of tabs and panes", value: "new" },
          ],
        });
        const [verb, index] = v?.split(":") ?? [];
        if (v === "new") actions["new-workspace"]!.run();
        else if (verb === "rename") renameSpace(app, +index!);
        else if (verb === "delete") deleteSpace(app, +index!);
        else if (v !== null) app.call("selectWorkspace", { index: +v });
      },
    },
    "new-workspace": {
      label: "New space",
      run: async () => {
        const name = await prompt(app, "New space", `space ${app.view!.workspaces.length + 1}`);
        if (name?.trim()) app.call("newWorkspace", { cwd: app.ws().cwd, name: name.trim() });
      },
    },
    "new-agent": {
      label: "New agent pane",
      run: async () => {
        const h = await pick(app, "New agent", await agentList());
        if (!h) return;
        const name = await prompt(app, "Name the agent", "", "optional: @name lets agents message it");
        app.call("spawnAgent", { harness: h, name: name || undefined });
      },
    },
    "toggle-sidebar": { label: "Toggle sidebar", run: () => { app.sidebar = !app.sidebar; app.conn.notify("area", { area: app.area() }); render(app); } },
    "copy-mode": { label: "Copy mode / scrollback", run: () => { app.mode = "copy"; render(app); } },
    search: {
      label: "Search scrollback",
      run: async () => {
        const q = await prompt(app, "Search scrollback", "", "text to find in the focused pane");
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
          ...pluginUi(app).flatMap((plugin) => plugin.actions.map((a) => ({ name: `${plugin.plugin}: ${a.title}`, description: a.description ?? "plugin action", value: `plugin:${plugin.plugin}:${plugin.run}:${a.id}` }))),
          { name: "Kill session", description: "close every pane and stop the server", value: "kill" },
        ];
        const v = await pick(app, "Commands", [
          ...Object.entries(actions).filter(([k]) => k !== "palette" && !k.startsWith("agent-")).map(([k, a]) => ({ name: a.label, description: "", key: Object.entries(bindings).find(([, b]) => b === k)?.[0] ?? "", value: k })),
          ...extra,
        ]);
        if (!v) return;
        if (v.startsWith("agent:")) return app.call("spawnAgent", { harness: v.slice(6) });
        if (v.startsWith("plugin:")) {
          const [, plugin, run, action] = v.split(":");
          const focused = app.tab().focused; // the target is the pane focused now, not when the action finishes
          const instance = app.info(focused)?.instance;
          return runPluginAction(app, { plugin: plugin!, run: run! }, action!, {}, instance ? { pane: focused, instance } : undefined);
        }
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
    "update-modisa": {
      label: "Update modisa",
      run: async () => {
        const m = app.update ?? (await checkForUpdate(true));
        if (!m) return app.toast(`modisa ${VERSION} is up to date`, app.th.done);
        const managed = updateCommand(); // Homebrew or mise installed it: their command, not ours
        if (managed !== "modisa update") return app.toast(`modisa ${m.version} is out: run ${managed}, then modisa restart`, app.th.warn);
        const notes = m.notes.trim().split("\n").filter(Boolean).slice(0, 6).map((l) => fit(l, 60));
        const ok = await confirm(app, `Update to ${m.version}`, [`modisa ${VERSION} → ${m.version}`, ...(notes.length ? ["", ...notes] : []), "", "Downloads it, then restarts the server; agents resume."].join("\n"), "update and restart");
        if (!ok) return;
        const cmd = self().join(" ");
        app.call("newTab", { name: "update", command: `${cmd} update && ${cmd} restart`, ephemeral: true });
      },
    },
    "restart-server": { label: "Restart server (load updated modisa; panes are restored)", run: () => { app.restartedByUs = app.restarting = true; app.conn.request("restart").catch(() => {}); } },
    "toggle-messaging": { label: "Pause/resume agent messaging", run: () => app.call("pause") },
    "message-log": { label: "Message log", run: () => app.call("newTab", { name: "messages", command: `${self().join(" ")} messages --follow`, ephemeral: true }) },
    "send-message": {
      label: "Send message to agent",
      run: async () => {
        const agents = app.view!.panes.filter((p) => p.agent || p.harness);
        if (!agents.length) return app.toast("no agent panes", app.th.warn);
        const to = await pick(app, "Send to", agents.map((p) => ({ name: p.name ? "@" + p.name : p.title, description: p.agent?.state ?? "", value: p.id })));
        if (!to) return;
        const body = await prompt(app, "Message", "", "what to tell the agent");
        if (body) app.conn.request("send", { to, body }).then(() => app.toast("queued"), (e) => app.toast(e.message, app.th.blocked));
      },
    },
    "rename-tab": { label: "Rename tab", run: async () => { const n = await prompt(app, "Rename tab", app.tab().name ?? ""); if (n !== null) app.call("renameTab", { name: n }); } },
    "rename-pane": { label: "Rename pane (@name)", run: async () => { const n = await prompt(app, "Rename pane", app.info(app.tab().focused)?.name ?? ""); if (n !== null) app.call("renamePane", { name: n }); } },
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

const plural = (n: number, what: string) => `${n} ${what}${n === 1 ? "" : "s"}`;

// Only the agents in one state; picking one jumps to it.
async function pickAgents(app: App, state: "working" | "blocked", title: string) {
  const agents = app.sortedAgents().filter((p) => p.agent!.state === state);
  if (!agents.length) return app.toast(state === "working" ? "no agents are working" : "no agents need you", app.th.dim);
  const where = (id: string) => app.view!.workspaces.find((w) => w.tabs.some((t) => treePanes(t.tree).includes(id)))?.name ?? "";
  const id = await pick(app, title, agents.map((p) => ({ name: p.name ? "@" + p.name : p.title, description: `${p.agent!.harness} · ${where(p.id)}`, value: p.id })));
  if (id) app.call("focusPane", { pane: id });
}
