// `modisa logos [status|install|uninstall]`: agents' logos in the terminal (see src/platform/logos.ts). The TUI installs
// them the first time it starts; this shows where they stand, puts them back, or takes them out for good.
import { installLogos, logoStatus, logosVisible, uninstallLogos } from "../platform/logos";

export async function runLogos(verb = "status"): Promise<number> {
  if (verb === "install") {
    const done = await installLogos();
    console.log(`installed modisa's logo font${done.length ? ` and set up ${done.join(", ")}` : ""}`);
    console.log("restart your terminal (in VS Code, reload the window) to see the logos");
    return 0;
  }
  if (verb === "uninstall") {
    await uninstallLogos();
    console.log("removed modisa's logo font and its terminal settings; the sidebar shows plain marks, and the TUI won't install them again");
    return 0;
  }
  if (verb !== "status") {
    console.error("usage: modisa logos [status|install|uninstall]");
    return 2;
  }
  const s = await logoStatus();
  console.log(s.font ? `font: ${s.font}${s.current ? "" : " (an older one: modisa logos install updates it)"}` : "font: not installed (modisa logos install)");
  for (const t of s.terminals) console.log(`${t.name.padEnd(15)} ${t.configured ? "set up" : "not set up"}  ${t.path}`);
  console.log(`this terminal: ${(await logosVisible()) ? "shows the logos" : "shows plain marks"}`);
  return 0;
}
