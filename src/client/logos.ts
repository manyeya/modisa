// Agents' logos in the sidebar and plugins' rows: installed the first time the TUI starts (a terminal only picks up a
// new font when it restarts, so they show from then on), and drawn wherever this terminal can show them.
// [sidebar] logos = "auto" (that), "on" (always: for a terminal modisa doesn't recognise) or "off"; MODISA_LOGOS=off
// overrides it (the test suite, which must never touch a real home's fonts).
import { installLogosOnce, logosLoaded, logosVisible, terminalFont } from "../platform/logos";
import { fontEms } from "./design";
import type { App } from "./context";
import { render } from "./render";

export async function setupLogos(app: App) {
  const mode = Bun.env.MODISA_LOGOS === "off" ? "off" : app.cfg.sidebar.logos;
  let installed: Awaited<ReturnType<typeof installLogosOnce>>;
  if (mode === "auto") {
    installed = await installLogosOnce().catch((e) => {
      app.debug(`logos not installed: ${e instanceof Error ? e.message : e}`);
      return undefined;
    });
    const where = installed?.terminals.length ? ` for ${installed.terminals.join(", ")}` : "";
    if (installed) app.toast(installed.updated ? `agent logos updated${where}: quit and reopen the terminal to centre them` : `agent logos installed${where}: quit and reopen the terminal to see them`, app.th.accent);
  }
  // drawn where the terminal is set up for them, as much as it has loaded: an older font's whole logos until it's
  // restarted after an update, then the halves
  const logos = mode === "on" ? "halves" : mode === "auto" && (await logosVisible()) ? await logosLoaded() : false;
  // where a logo's halves meet, for a terminal that doesn't say how big its cells are
  const font = await terminalFont().catch(() => undefined);
  app.cellGuess = (font && fontEms(font.family, font.lineHeight)) || 1.2;
  if (logos === app.logos) return;
  app.logos = logos;
  app.chromeSig = "";
  render(app);
}
