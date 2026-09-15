// TUI client: renders what the server holds, sends input back. Local or over ssh.
// This file is the startup order; the pieces live in context, render, chrome/, panes/, modals/, input/.
import { createCliRenderer } from "@opentui/core";
import { loadConfig, watchConfig } from "../config/config";
import { App, type ClientOptions } from "./context";
import { openMouseLog, logHandlerErrors } from "./debug";
import { createActions } from "./actions";
import { render } from "./render";
import { installResize, endResize } from "./panes/resize";
import { installPointer } from "./panes/pointer";
import { installKeyboard } from "./input/keyboard";
import { installSelectionCopy } from "./input/copy-mode";
import { reload } from "./notify";
import { connectWithRetry } from "./connection";
import { checkForUpdate } from "../cli/update";

export async function runClient(opts: ClientOptions) {
  const cfg = await loadConfig();
  const debug = await openMouseLog();
  const r = await createCliRenderer({ exitOnCtrlC: false, targetFps: 60 });
  logHandlerErrors(r, debug);
  const app = new App(r, opts, cfg, debug);
  app.paintBackground();

  // Bun updates stream dimensions after SIGWINCH; use the stream event's fresh size.
  const resizeStream = () => r.resize(process.stdout.columns, process.stdout.rows);
  process.stdout.on("resize", resizeStream);
  app.cleanup.push(() => process.stdout.off("resize", resizeStream));

  app.actions = createActions(app);
  installResize(app);
  installPointer(app);
  installSelectionCopy(app);
  installKeyboard(app);
  r.on("resize", () => {
    endResize(app, "terminal resize");
    app.conn?.notify("area", { area: app.area() });
    app.modal?.resize();
    app.ui.toastBox.visible = false;
    render(app);
  });
  watchConfig(() => reload(app));

  // a newer release lights the ↑ badge in the status row (checked in the background, cached 6h)
  const lookForUpdate = () => checkForUpdate().then((m) => { app.update = m; app.chromeSig = ""; render(app); });
  setTimeout(lookForUpdate, 1500);
  const updates = setInterval(lookForUpdate, 6 * 60 * 60 * 1000);
  app.cleanup.push(() => clearInterval(updates));

  await connectWithRetry(app);
}
