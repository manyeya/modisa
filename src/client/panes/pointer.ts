// One place decides the pointer shape: every mouse move bubbles up to the root. Move cursor over a
// pane border (or while resizing), hand over anything clickable, the default arrow everywhere else.
import type { Renderable } from "@opentui/core";
import type { App, PointerShape } from "../context";
import { onDivider } from "./resize";

export function pointer(app: App, shape: PointerShape) {
  if (shape === app.pointerShape) return;
  app.pointerShape = shape;
  app.r.setMousePointer(shape);
}

export function installPointer(app: App) {
  app.r.root.onMouseMove = (e) => {
    if (app.resizing) return;
    pointer(app, onDivider(app, e.x, e.y) ? "move" : clickable(app, e.target) ? "pointer" : "default");
  };
}

// The target or anything it sits in (a sidebar row is clickable, the text inside it isn't registered).
function clickable(app: App, node: Renderable | null) {
  for (; node; node = node.parent) if (app.clickable.has(node)) return true;
  return false;
}
