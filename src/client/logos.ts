// Agents' logos in the sidebar and plugins' rows: installed the first time the TUI starts (a terminal only picks up a
// new font when it restarts, so they show from then on), and drawn wherever this terminal can show them.
// [sidebar] logos = "auto" (that), "on" (always: for a terminal modisa doesn't recognise) or "off"; MODISA_LOGOS=off
// overrides it (the test suite, which must never touch a real home's fonts).
import { installLogosOnce, logosVisible, terminalFont } from "../platform/logos";
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
    if (installed) app.toast(`agent logos ${installed.updated ? "updated" : "installed"}${where}: restart the terminal to see them`, app.th.accent);
  }
  const logos = mode === "on" || (mode === "auto" && !installed && (await logosVisible()));
  // where a logo's halves meet, for a terminal that doesn't say how big its cells are
  const font = await terminalFont().catch(() => undefined);
  app.cellGuess = (font && fontEms(font.family, font.lineHeight)) || 1.2;
  if (logos === app.logos) return;
  app.logos = logos;
  app.chromeSig = "";
  render(app);
}
