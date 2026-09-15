// Keys, in priority order: cancel a resize or inline rename, the open modal, copy mode, then the
// prefix (Ctrl+B by default) followed by a binding. Everything else goes to the focused pane.
import type { App } from "../context";
import { render } from "../render";
import { endResize } from "../panes/resize";
import { pointer } from "../panes/pointer";
import { finishRename } from "../chrome/sidebar";
import { bindings, keyName } from "./bindings";
import { copyKey } from "./copy-mode";
import { pluginKey } from "../plugin-ui";

export function installKeyboard(app: App) {
  app.r.keyInput.on("keypress", (k) => {
    if (app.resizing && k.name === "escape") {
      k.preventDefault();
      endResize(app, "escape");
      return pointer(app, "default");
    }
    if (app.editing && k.name === "escape") {
      k.preventDefault();
      return finishRename(app, false);
    }
    const modal = app.modal;
    if (modal) {
      if (k.name === "escape" && !modal.keepEscape) {
        k.preventDefault();
        return modal.close(null);
      }
      if (modal.keys?.(k)) k.preventDefault();
      return;
    }
    const isPrefix = k.ctrl && k.name === app.prefix.name;
    if (app.mode === "copy" && !app.prefixArmed && !isPrefix) {
      k.preventDefault();
      return copyKey(app, k);
    }
    if (app.prefixArmed) {
      k.preventDefault();
      app.prefixArmed = false;
      if (isPrefix) app.focusedPane()?.term.handleKeyPress(k); // prefix twice sends it through
      else if (bindings[keyName(k)]) app.actions[bindings[keyName(k)]!]?.run();
      else pluginKey(app, keyName(k)); // shepherd's keys first; a plugin never gets one of them
      render(app);
    } else if (isPrefix) {
      k.preventDefault();
      app.prefixArmed = true;
      render(app);
    }
  });
}
