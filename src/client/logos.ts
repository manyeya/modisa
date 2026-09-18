// Agents' logos in the sidebar and plugins' rows: installed the first time the TUI starts (a terminal only picks up a
// new font when it restarts, so they show from then on), and drawn wherever this terminal can show them.
// [sidebar] logos = "auto" (that), "on" (always: for a terminal modisa doesn't recognise) or "off"; MODISA_LOGOS=off
// overrides it (the test suite, which must never touch a real home's fonts).
import { installLogosOnce, logosVisible } from "../platform/logos";
import type { App } from "./context";
import { render } from "./render";

export async function setupLogos(app: App) {
  const mode = Bun.env.MODISA_LOGOS === "off" ? "off" : app.cfg.sidebar.logos;
  let installed: string[] | undefined;
  if (mode === "auto") {
    installed = await installLogosOnce().catch((e) => {
      app.debug(`logos not installed: ${e instanceof Error ? e.message : e}`);
      return undefined;
    });
    if (installed) app.toast(`agent logos installed${installed.length ? ` for ${installed.join(", ")}` : ""}: restart the terminal to see them`, app.th.accent);
  }
  const logos = mode === "on" || (mode === "auto" && !installed && (await logosVisible()));
  if (logos === app.logos) return;
  app.logos = logos;
  app.chromeSig = "";
  render(app);
}
