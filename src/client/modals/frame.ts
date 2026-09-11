// The modal machinery: a centred bordered frame, and open() which shows it over a dismiss-on-click veil,
// routes keys to it, and resolves with its result (null when dismissed).
import { BoxRenderable, type KeyEvent } from "@opentui/core";
import type { App } from "../context";
import { fit, floating } from "../design";
import { render } from "../render";

// how each open frame re-lays itself out on terminal resize
export const frameLayouts = new WeakMap<BoxRenderable, () => void>();

export function frame(app: App, title: string, h: number) {
  const { r, th } = app;
  const rect = floating(r.width, r.height, 70, h);
  const box = new BoxRenderable(r, {
    position: "absolute", left: rect.x, top: rect.y,
    width: rect.w, height: rect.h, border: true, borderStyle: "rounded", borderColor: th.focus, title: fit(` ${title} `, rect.w - 4),
    backgroundColor: th.bar, zIndex: 100, flexDirection: "column", overflow: "hidden",
  });
  frameLayouts.set(box, () => {
    const rect = floating(r.width, r.height, 70, h);
    Object.assign(box, { left: rect.x, top: rect.y, width: rect.w, height: rect.h });
  });
  r.root.add(box);
  return box;
}

// onClose runs before the closing redraw: state restored after it (a theme preview, say) can miss the
// frame OpenTUI is already drawing and stay stale on screen until something else redraws.
export function open<T>(app: App, box: BoxRenderable, setup: (done: (v: T | null) => void) => ((k: KeyEvent) => boolean) | void, onClose?: () => void): Promise<T | null> {
  app.modal?.close(null);
  return new Promise((resolve) => {
    const veil = new BoxRenderable(app.r, { position: "absolute", left: 0, top: 0, width: "100%", height: "100%", zIndex: 90 });
    app.r.root.add(veil);
    const done = (v: T | null) => {
      if (app.modal?.close !== done) return;
      app.modal = undefined;
      onClose?.();
      veil.destroyRecursively();
      box.destroyRecursively();
      resolve(v);
      render(app);
    };
    veil.onMouseDown = (e) => { e.preventDefault(); e.stopPropagation(); done(null); };
    veil.onMouse = (e) => { e.preventDefault(); e.stopPropagation(); };
    box.onMouse = (e) => e.stopPropagation();
    app.modal = { close: done, resize: frameLayouts.get(box) ?? (() => {}) };
    app.modal.keys = setup(done) ?? undefined;
    render(app);
  });
}
