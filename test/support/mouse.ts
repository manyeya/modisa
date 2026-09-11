// Terminal input a real terminal would send for the mouse and window, for driving a Screen.
import type { Screen } from "./harness";

// SGR (1006) mouse report: button b at cell (x, y), press or release. 32 = motion with left held, 35 = motion, nothing held.
export const sgr = (b: number, x: number, y: number, up = false) => `\x1b[<${b};${x + 1};${y + 1}${up ? "m" : "M"}`;

// Legacy X10 mouse report: every motion is "move"; 32 = held, 35 = nothing held; 3 = release.
export const x10 = (code: number, x: number, y: number) => `\x1b[M${String.fromCharCode(32 + code, 33 + x, 33 + y)}`;

// Press and release in one go (0 = left, 2 = right).
export const click = (ui: Screen, button: number, x: number, y: number) => ui.write(sgr(button, x, y) + sgr(button, x, y, true));

// Resize the terminal the way a window resize does: new size, then SIGWINCH.
export function resizeTerminal(ui: Screen, width: number, height: number) {
  ui.vt.resize(width, height);
  ui.pty.resize(width, height);
  ui.proc.kill("SIGWINCH");
}

// Move the pointer onto (x, y) with no button held, coming from column 0 (or 1) so the element under it
// sees the pointer arrive even if it was the last thing clicked.
export const hover = (ui: Screen, x: number, y: number) => ui.write(sgr(35, x ? 0 : 1, y) + sgr(35, x, y));
