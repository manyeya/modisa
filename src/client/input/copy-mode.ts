// Copy mode: scroll the focused pane's scrollback with the keyboard, jump between search matches,
// and copy mouse selections to the clipboard (OSC 52).
import type { KeyEvent } from "@opentui/core";
import type { App } from "../context";
import { render } from "../render";
import { keyName } from "./bindings";

export function jump(app: App) {
  const p = app.focusedPane();
  const search = app.search;
  if (!p || !search) return;
  const line = search.matches[search.i]!;
  const rows = app.info(p.id)?.rows ?? 20;
  p.scroll(1e7); // bottom
  p.scroll(-Math.max(0, search.total - line - Math.ceil(rows / 2)));
  app.toast(`match ${search.i + 1}/${search.matches.length}`, app.th.accent);
  render(app);
}

export function copyKey(app: App, k: KeyEvent) {
  const p = app.focusedPane();
  const half = Math.floor((p ? app.info(p.id)?.rows ?? 20 : 20) / 2);
  const moves: Record<string, number> = { k: -1, up: -1, j: 1, down: 1, u: -half, pageup: -half * 2, d: half, pagedown: half * 2, g: -1e7, G: 1e7 };
  const name = keyName(k);
  if (name === "q" || name === "escape") {
    p?.scroll(1e7);
    app.mode = "normal";
    app.search = undefined;
    return render(app);
  }
  if (name === "/") return app.actions.search!.run();
  const search = app.search;
  if (search && (name === "n" || name === "N")) {
    search.i = (search.i + (name === "n" ? -1 : 1) + search.matches.length) % search.matches.length;
    return jump(app);
  }
  if (moves[name] !== undefined) p?.scroll(moves[name]!);
}

export function installSelectionCopy(app: App) {
  app.r.on("selection", (sel: any) => {
    const text = sel?.getSelectedText?.();
    if (text) app.r.copyToClipboardOSC52(text);
  });
}
