// A one-line clickable label: left-click runs it, right-click opens its context menu (if any).
import { TextRenderable, type BoxRenderable, type MouseEvent } from "@opentui/core";
import type { App } from "../context";
import { fit } from "../design";

export function button(app: App, parent: BoxRenderable, text: string, width: number, fg: string, bg: string, run: () => any, context?: (e: MouseEvent) => void) {
  const node = new TextRenderable(app.r, {
    content: fit(text, width), width, height: 1, flexShrink: 0, fg, bg,
    onMouseDown: (e) => {
      e.stopPropagation();
      if (app.modal) return;
      if (e.button === 2) context?.(e);
      else if (e.button === 0) run();
    },
    onMouseOver: () => { node.bg = app.th.border; node.fg = app.th.fg; },
    onMouseOut: () => { node.bg = bg; node.fg = fg; },
  });
  app.clickable.add(node);
  parent.add(node);
  return node;
}
