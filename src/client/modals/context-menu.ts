// Right-click on a pane or tab: the pane's actions.
import type { App } from "../context";
import { menu } from "./menu";
import { pluginUi, runPluginAction } from "../plugin-ui";

export function contextMenu(app: App, pane: string, x: number, y: number) {
  if (app.modal || !app.view) return;
  const p = app.info(pane);
  if (!p) return;
  menu(app, p.name ? "@" + p.name : p.title, [
    { name: "Focus pane", key: "Enter", action: "focus" },
    { name: "Split right", key: "v", action: "split-right" },
    { name: "Split down", key: "-", action: "split-down" },
    { name: "Zoom / restore", key: "z", action: "zoom" },
    { name: "Rename pane", key: ".", action: "rename-pane" },
    { name: "Copy visible output", key: "y", action: "copy-output" },
    { name: "Search scrollback", key: "/", action: "search" },
    { name: "New tab", key: "c", action: "new-tab" },
    { name: "Launch agent", key: "a", action: "new-agent" },
    { name: "All panes", key: "o", action: "pane-picker" },
    { name: app.sidebar ? "Hide sidebar" : "Show sidebar", key: "b", action: "toggle-sidebar" },
    { name: "Change theme", key: "t", action: "theme-picker" },
    ...pluginUi(app).flatMap((plugin) => plugin.menu.map((item) => ({ name: `${plugin.plugin}: ${item.title}`, key: "", action: `plugin:${plugin.plugin}:${plugin.run}:${item.action}` }))),
    { name: "Close pane", key: "x", action: "close-pane", danger: true },
  ], x, y).then(async (action) => {
    if (!action || !app.info(pane)) return;
    // a plugin's entry runs its action for this pane
    if (action.startsWith("plugin:")) {
      const [, plugin, run, act] = action.split(":");
      return runPluginAction(app, { plugin: plugin!, run: run! }, act!, {}, { pane, instance: p.instance });
    }
    // Complete target selection before running actions that use the active pane.
    try { await app.conn.request("cmd", { name: "focusPane", args: { pane } }); }
    catch (e) { return app.toast(String(e), app.th.blocked); }
    if (action === "focus") return;
    if (action === "copy-output") {
      const output = app.panes.get(pane)?.term.screen().text;
      if (output) { app.r.copyToClipboardOSC52(output); app.toast("Visible output copied", app.th.focus); }
      return;
    }
    await app.actions[action]?.run();
  }).catch((e) => app.toast(String(e), app.th.blocked));
}
