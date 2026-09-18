// Agents' logos in the terminal. Modisa's logo font (src/config/agents/marks.ttf) goes into the user's font folder, and
// each terminal that has to be told where those characters are learns it: Ghostty and kitty get a codepoint map, in a
// block modisa owns; VS Code and its forks get the font at the end of their terminal font list, after whatever they
// already use, so text looks the same. WezTerm finds the font by itself. Nothing needs admin rights, and uninstalling
// takes back exactly what was added. What was done is kept in DIR/logos.json, and a user who took the logos out isn't
// given them again.
import MARKS from "../config/agents/marks.ttf" with { type: "file" };
import { LOGO_RANGE } from "../config/agents/brands";
import { DIR, HOME } from "../core/paths";
import { withBlock } from "../integrations/edit";

export const FAMILY = "Modisa Marks";
const MAC = Bun.spawnSync(["uname", "-s"]).stdout.toString().trim() === "Darwin";
const FONT = MAC ? `${HOME}/Library/Fonts/ModisaMarks.ttf` : `${Bun.env.XDG_DATA_HOME ?? `${HOME}/.local/share`}/fonts/ModisaMarks.ttf`;
const STATE = `${DIR}/logos.json`;
const XDG = Bun.env.XDG_CONFIG_HOME ?? `${HOME}/.config`;
const SUPPORT = `${HOME}/Library/Application Support`;
const BEGIN = "# >>> modisa agent logos (managed; `modisa logos uninstall` removes it)";
const END = "# <<< modisa agent logos";

type State = { removed?: boolean; inserted?: string[] }; // inserted: VS Code settings whose font list modisa created
const readState = async (): Promise<State> => Bun.file(STATE).json().catch(() => ({}));
const exists = (path: string) => Bun.file(path).exists();
const dirExists = async (path: string) => (await Bun.$`test -d ${path}`.quiet().nothrow()).exitCode === 0;
const firstExisting = async (paths: string[]) => {
  for (const p of paths) if (await exists(p)) return p;
  return undefined;
};

// Ghostty and kitty: a codepoint map, in modisa's block
type Mapped = { name: string; present: () => Promise<boolean>; path: () => Promise<string>; line: string };
const MAPPED: Mapped[] = [
  {
    name: "Ghostty",
    present: async () => !!Bun.which("ghostty") || (MAC && (await dirExists("/Applications/Ghostty.app"))) || (await dirExists(`${XDG}/ghostty`)),
    path: async () => (await firstExisting([`${XDG}/ghostty/config`, `${XDG}/ghostty/config.ghostty`, `${SUPPORT}/com.mitchellh.ghostty/config`, `${SUPPORT}/com.mitchellh.ghostty/config.ghostty`])) ?? `${XDG}/ghostty/config`,
    line: `font-codepoint-map = ${LOGO_RANGE}=${FAMILY}`,
  },
  {
    name: "kitty",
    present: async () => !!Bun.which("kitty") || (MAC && (await dirExists("/Applications/kitty.app"))) || (await dirExists(`${XDG}/kitty`)),
    path: async () => `${XDG}/kitty/kitty.conf`,
    line: `symbol_map ${LOGO_RANGE} ${FAMILY}`,
  },
];

// VS Code and its forks: the settings files that exist
const vscodeSettings = async () => {
  const base = MAC ? SUPPORT : XDG;
  const found: [string, string][] = [];
  for (const app of ["Code", "Code - Insiders", "Cursor", "VSCodium"]) {
    const path = `${base}/${app}/User/settings.json`;
    if (await exists(path)) found.push([app === "Code" ? "VS Code" : app, path]);
  }
  return found;
};

// Edited as text, not parsed and rewritten: settings.json may hold comments and trailing commas, and every byte of it
// that isn't the terminal's font list stays as it was.
const KEY = /("terminal\.integrated\.fontFamily"\s*:\s*")((?:[^"\\]|\\.)*)(")/;
const DEFAULT_FONTS = MAC ? "Menlo, Monaco, 'Courier New', monospace" : "'Droid Sans Mono', 'monospace', monospace";
export function withVscodeFont(text: string, add: boolean, created = false): { text: string; created: boolean } {
  const found = KEY.exec(text);
  if (add) {
    if (found) return { text: found[2]!.includes(FAMILY) ? text : text.replace(KEY, `$1$2, '${FAMILY}'$3`), created: false };
    const editor = /"editor\.fontFamily"\s*:\s*"((?:[^"\\]|\\.)*)"/.exec(text)?.[1] || DEFAULT_FONTS;
    const line = `"terminal.integrated.fontFamily": "${editor}, '${FAMILY}'"`;
    const open = text.indexOf("{");
    if (open < 0 || /^\{\s*\}$/.test(text.trim())) return { text: `{\n    ${line}\n}\n`, created: true };
    return { text: `${text.slice(0, open + 1)}\n    ${line},${text.slice(open + 1)}`, created: true };
  }
  if (!found) return { text, created: false };
  if (created) return { text: text.replace(/\n[ \t]*"terminal\.integrated\.fontFamily"\s*:\s*"(?:[^"\\]|\\.)*",?/, ""), created: false };
  const rest = found[2]!.split(",").map((f) => f.trim()).filter((f) => f.replace(/['"]/g, "") !== FAMILY).join(", ");
  return { text: text.replace(KEY, `$1${rest}$3`), created: false };
}

export type LogoStatus = { font: string | undefined; terminals: { name: string; path: string; configured: boolean }[] };

export async function logoStatus(): Promise<LogoStatus> {
  const terminals: LogoStatus["terminals"] = [];
  for (const t of MAPPED) {
    if (!(await t.present())) continue;
    const path = await t.path();
    terminals.push({ name: t.name, path, configured: (await Bun.file(path).text().catch(() => "")).includes(BEGIN) });
  }
  for (const [name, path] of await vscodeSettings()) terminals.push({ name, path, configured: KEY.exec(await Bun.file(path).text())?.[2]?.includes(FAMILY) ?? false });
  return { font: (await exists(FONT)) ? FONT : undefined, terminals };
}

// The font, and every terminal found that needs telling: returns the terminals it set up.
export async function installLogos(): Promise<string[]> {
  await Bun.write(FONT, Bun.file(MARKS));
  if (!MAC && Bun.which("fc-cache")) await Bun.$`fc-cache -f ${FONT.slice(0, FONT.lastIndexOf("/"))}`.quiet().nothrow();
  const state = await readState();
  const inserted = new Set(state.inserted ?? []);
  const done: string[] = [];
  for (const t of MAPPED) {
    if (!(await t.present())) continue;
    const path = await t.path();
    await Bun.write(path, withBlock(await Bun.file(path).text().catch(() => ""), BEGIN, END, `${t.line}\n`));
    done.push(t.name);
  }
  for (const [name, path] of await vscodeSettings()) {
    const { text, created } = withVscodeFont(await Bun.file(path).text(), true);
    await Bun.write(path, text);
    if (created) inserted.add(path);
    done.push(name);
  }
  await Bun.write(STATE, JSON.stringify({ inserted: [...inserted] }));
  return done;
}

export async function uninstallLogos(): Promise<void> {
  const state = await readState();
  await Bun.file(FONT).delete().catch(() => {});
  for (const t of MAPPED) {
    const path = await t.path();
    const text = await Bun.file(path).text().catch(() => undefined);
    if (text?.includes(BEGIN)) await Bun.write(path, withBlock(text, BEGIN, END));
  }
  for (const [, path] of await vscodeSettings()) {
    const text = await Bun.file(path).text();
    const next = withVscodeFont(text, false, state.inserted?.includes(path)).text;
    if (next !== text) await Bun.write(path, next);
  }
  await Bun.write(STATE, JSON.stringify({ removed: true }));
}

// The first time a TUI starts: install them, unless the user took them out. Returns the terminals set up, or nothing.
export async function installLogosOnce(): Promise<string[] | undefined> {
  const state = await readState();
  if (state.removed || (await exists(FONT))) return undefined;
  return installLogos();
}

// Whether the terminal this runs in will draw the logos: the font is installed and the terminal is one that finds it
// (told by its codepoint map or settings, or on its own). Inside tmux or screen the terminal outside isn't known.
export async function logosVisible(): Promise<boolean> {
  if (!(await exists(FONT)) || Bun.env.TMUX || Bun.env.STY) return false;
  const program = Bun.env.TERM_PROGRAM ?? "", term = Bun.env.TERM ?? "";
  const status = await logoStatus();
  const configured = (name: string) => status.terminals.some((t) => t.name === name && t.configured);
  if (program === "ghostty" || term === "xterm-ghostty" || Bun.env.GHOSTTY_RESOURCES_DIR) return configured("Ghostty");
  if (Bun.env.KITTY_WINDOW_ID || term === "xterm-kitty") return configured("kitty");
  if (program === "WezTerm") return true;
  if (program === "vscode") return status.terminals.some((t) => ["VS Code", "Code - Insiders", "Cursor", "VSCodium"].includes(t.name) && t.configured);
  return false;
}
