// Terminal flair for shepherd's own commands (`shepherd update`): the wordmark in the brand gradient, spinners and a
// download bar. Only on a terminal that takes colour: plain lines when the output is piped, NO_COLOR is set or TERM is
// dumb; SHEPHERD_FANCY=1 forces it. install.sh draws the same wordmark.
export const fancy = () => Bun.env.SHEPHERD_FANCY === "1" || (!!process.stdout.isTTY && !Bun.env.NO_COLOR && (Bun.env.TERM ?? "dumb") !== "dumb");

const ESC = "\x1b[";
export const RESET = `${ESC}0m`;
const FROM = [94, 231, 239]; // the ion theme's focus cyan
const TO = [179, 154, 255]; // and its accent violet
const truecolor = () => /truecolor|24bit/i.test(Bun.env.COLORTERM ?? "");
const rgb = (r: number, g: number, b: number) => {
  if (truecolor()) return `${ESC}38;2;${r};${g};${b}m`;
  const q = (v: number) => Math.round((v / 255) * 5);
  return `${ESC}38;5;${16 + 36 * q(r) + 6 * q(g) + q(b)}m`;
};

// the brand gradient's colour at t, from 0 (cyan) to 1 (violet)
export const paint = (t: number) => {
  const k = Math.min(1, Math.max(0, t));
  const [r, g, b] = FROM.map((f, i) => Math.round(f + (TO[i]! - f) * k)) as [number, number, number];
  return rgb(r, g, b);
};
export const green = () => rgb(165, 239, 181);
export const red = () => rgb(255, 127, 150);
export const dim = (s: string) => `${ESC}2m${s}${RESET}`;

// text in the gradient, left to right
export function gradient(text: string) {
  const chars = [...new Intl.Segmenter().segment(text)].map((s) => s.segment);
  return chars.map((c, i) => (c === " " ? c : paint(i / Math.max(1, chars.length - 1)) + c)).join("") + RESET;
}

export const WORDMARK = [
  "▄▀▀▀▀ █   █ █▀▀▀▀ █▀▀▀▄ █   █ █▀▀▀▀ █▀▀▀▄ █▀▀▀▄",
  " ▀▀▀▄ █▀▀▀█ █▀▀▀  █▄▄▄▀ █▀▀▀█ █▀▀▀  █▄▄▄▀ █   █",
  "▄▄▄▄▀ █   █ █▄▄▄▄ █     █   █ █▄▄▄▄ █  ▀▄ █▄▄▄▀",
];

const write = (s: string) => process.stdout.write(s);
let hidden = false;
export const showCursor = () => {
  if (!hidden) return;
  hidden = false;
  write(`${ESC}?25h`);
};
const hideCursor = () => {
  if (hidden) return;
  hidden = true;
  write(`${ESC}?25l`);
  process.once("exit", showCursor); // however the command ends
};

// the wordmark, one row at a time, with a line under it
export async function banner(subtitle: string) {
  hideCursor();
  write("\n");
  for (const row of WORDMARK) {
    write(`  ${gradient(row)}\n`);
    await Bun.sleep(40);
  }
  write(`\n  ${dim(subtitle)}\n\n`);
}

const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
// a spinner in front of `label` until done() or fail(); update() sets what follows the label (a bar, a byte count)
export function spinner(label: string) {
  hideCursor();
  let i = 0;
  let suffix = "";
  const draw = () => {
    const frame = i++ % FRAMES.length;
    write(`\r${ESC}K  ${paint(frame / (FRAMES.length - 1))}${FRAMES[frame]}${RESET} ${label}${suffix}`);
  };
  draw();
  const timer = setInterval(draw, 80);
  const end = (mark: string, text: string) => {
    clearInterval(timer);
    write(`\r${ESC}K  ${mark}${RESET} ${text}\n`);
  };
  return {
    update: (s: string) => (suffix = s),
    done: (text = label) => end(`${green()}✓`, text),
    fail: (text = label) => end(`${red()}✗`, text),
  };
}

// a bar `width` cells wide, filled to `fraction` in the gradient
export function bar(fraction: number, width = 24) {
  const filled = Math.round(Math.min(1, Math.max(0, fraction)) * width);
  let out = "";
  for (let k = 0; k < filled; k++) out += paint(k / (width - 1)) + "━";
  return `${out}${RESET}${dim("━".repeat(width - filled))}`;
}

export const megabytes = (n: number) => `${(n / 1048576).toFixed(1)} MB`;
