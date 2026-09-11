// tmux-style key names for `shepherd pane keys`: Enter, Escape, C-c, M-x, Up, … or literal text.
export function keyBytes(k: string): string {
  const named: Record<string, string> = {
    Enter: "\r", Escape: "\x1b", Esc: "\x1b", Tab: "\t", BSpace: "\x7f", Backspace: "\x7f", Space: " ",
    Up: "\x1b[A", Down: "\x1b[B", Right: "\x1b[C", Left: "\x1b[D", Home: "\x1b[H", End: "\x1b[F",
    PageUp: "\x1b[5~", PageDown: "\x1b[6~", Delete: "\x1b[3~",
  };
  if (named[k]) return named[k]!;
  const ctrl = /^C-(.)$/.exec(k);
  if (ctrl) return String.fromCharCode(ctrl[1]!.toLowerCase().charCodeAt(0) & 0x1f);
  const meta = /^M-(.)$/.exec(k);
  if (meta) return "\x1b" + meta[1];
  return k;
}
