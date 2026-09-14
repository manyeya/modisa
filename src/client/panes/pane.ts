// One pane on screen: a bordered box around OpenTUI's embedded terminal, fed by the server's PTY.
import { BoxRenderable, EmbeddedTerminalRenderable, type CliRenderer, type MouseEvent } from "@opentui/core";
import type { PointerShape } from "../context";

// Hooks a pane needs from the client.
export type PaneHooks = {
  click: () => void;
  context: (x: number, y: number) => void;
  onDivider: (x: number, y: number) => boolean; // is (x, y) on a border between two panes?
  beginResize: (x: number, y: number) => void;
  pointer: (shape: PointerShape) => void;
};

class PaneTerminal extends EmbeddedTerminalRenderable {
  hooks?: PaneHooks;
  override processMouseEvent(e: MouseEvent) {
    // Intercept before OpenTUI encodes mouse input for mouse-aware child programs.
    if (e.button === 2 && e.type !== "scroll") {
      e.preventDefault();
      e.stopPropagation();
      if (e.type === "down") this.hooks?.context(e.x, e.y);
      return;
    }
    super.processMouseEvent(e);
  }
}

export class ClientPane {
  box: BoxRenderable;
  term: EmbeddedTerminalRenderable;
  constructor(r: CliRenderer, readonly id: string, cols: number, rows: number, send: (bytes: Uint8Array) => void, hooks: PaneHooks) {
    const terminal = new PaneTerminal(r, {
      cols: Math.max(cols, 2),
      rows: Math.max(rows, 1),
      width: "100%",
      height: "100%",
      // the server's terminal answers VT queries; only real input goes back
      onData: (bytes, source) => source === "input" && send(bytes),
      onMouseDown: (e) => { e.stopPropagation(); if (e.button === 0) hooks.click(); },
      onMouseMove: () => hooks.pointer("default"), // mouse-aware programs may swallow the move before it bubbles
      onMouseDrag: (e) => e.stopPropagation(),
      onMouseUp: (e) => e.stopPropagation(),
    });
    terminal.hooks = hooks;
    this.term = terminal;
    this.box = new BoxRenderable(r, { position: "absolute", border: true, borderStyle: "rounded", zIndex: 1 });
    this.box.add(this.term);
    // The border: press on a divider to resize, anywhere else to focus.
    this.box.onMouseDown = (e) => {
      e.stopPropagation();
      if (e.button === 2) return hooks.context(e.x, e.y);
      if (e.button !== 0) return;
      if (hooks.onDivider(e.x, e.y)) hooks.beginResize(e.x, e.y);
      else hooks.click();
    };
  }
  // The embedded terminal's default colours (OSC 10/11), so unstyled cells take the theme instead of black.
  private shade = "";
  colors(bg: string, fg: string) {
    if (this.shade === bg + fg) return;
    this.shade = bg + fg;
    this.term.write(`\x1b]11;${bg}\x07\x1b]10;${fg}\x07`);
    this.term.invalidate(); // rows only repaint when dirty, and a colour change alone doesn't mark them
  }
  scroll(delta: number) {
    const t = this.term as any; // ponytail: OpenTUI exposes wheel scroll only internally
    t.lib.embeddedTerminalScroll(t.handle, delta);
    this.term.requestRender();
  }
  destroy() {
    this.box.destroyRecursively();
  }
}
