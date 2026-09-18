// Renaming and deleting spaces, from the space picker's right-click menu, keys or the palette.
import { panes as treePanes } from "../core/layout";
import type { App } from "./context";
import { confirm } from "./modals/confirm";
import { menu } from "./modals/menu";
import { prompt } from "./modals/prompt";

export async function renameSpace(app: App, index: number) {
  const current = app.view!.workspaces[index];
  if (!current) return;
  const name = await prompt(app, "Rename space", current.name);
  if (name?.trim()) app.call("renameWorkspace", { index, name });
}

export async function deleteSpace(app: App, index: number) {
  const view = app.view!;
  const target = view.workspaces[index];
  if (!target) return;
  if (view.workspaces.length < 2) return app.toast("can't delete the only space", app.th.warn);
  const count = target.tabs.reduce((n, t) => n + treePanes(t.tree).length, 0);
  const agents = target.tabs.flatMap((t) => treePanes(t.tree)).filter((id) => app.info(id)?.agent).length;
  const what = `${count} pane${count === 1 ? "" : "s"}${agents ? `, ${agents} running an agent` : ""}`;
  if (await confirm(app, "Delete space", `Delete space "${target.name}"?\nThis closes its ${what}.`, "delete")) app.call("closeWorkspace", { index });
}

export function spaceMenu(app: App, index: number, x: number, y: number) {
  if (app.modal || !app.view?.workspaces[index]) return;
  menu(app, `Space · ${app.view.workspaces[index]!.name}`, [
    { name: "Switch to space", key: "Enter", action: "switch" },
    { name: "Rename space", key: "r", action: "rename" },
    { name: "Delete space", key: "d", action: "delete", danger: true },
  ], x, y).then((action) => {
    if (action === "switch") app.call("selectWorkspace", { index });
    else if (action === "rename") renameSpace(app, index);
    else if (action === "delete") deleteSpace(app, index);
  }).catch((e) => app.toast(String(e), app.th.blocked));
}
