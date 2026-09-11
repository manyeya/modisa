// Resizing panes by dragging the border between them.
// Handle a resize before hit-testing so fast moves, off-screen releases and fresh clicks
// cannot get captured by a child terminal or missed while waiting for a new frame.
import type { StdinEvent } from "@opentui/core";
import { displayRects, dividerAt, panes as treePanes } from "../../core/layout";
import type { App } from "../context";
import { pointer } from "./pointer";

type EndReason = "released" | "new press" | "left button released" | "escape" | "blur" | "terminal resize";

export function installResize(app: App) {
  // ponytail: OpenTUI's public input handlers run AFTER mouse dispatch, even when prepended.
  // Keep this private adapter at the parsed-event boundary (which also handles split input packets).
  const inputRenderer = app.r as unknown as { handleStdinEvent: (event: StdinEvent) => void };
  const dispatchInput = inputRenderer.handleStdinEvent.bind(app.r);
  inputRenderer.handleStdinEvent = (event) => {
    if (event.type === "mouse" && app.resizing) {
      resizeEvent(app, event);
      // A fresh press cancels the old drag and can immediately focus a pane or start a new drag.
      if (event.event.type !== "down") return;
    }
    dispatchInput(event);
  };
  app.r.on("blur", () => endResize(app, "blur"));
}

export function onDivider(app: App, x: number, y: number) {
  if (!app.view || app.modal) return false;
  const t = app.tab();
  if (t.zoomed || displayRects(t.tree, app.area(), t.focused).size < treePanes(t.tree).length) return false; // compact view: no borders to drag
  return !!dividerAt(t.tree, app.area(), x, y);
}

export function beginResize(app: App, x: number, y: number) {
  if (app.resizing) return;
  app.resizing = { x, y, sawButtonMotion: false };
  pointer(app, "move");
  app.debug(`resize start at ${x},${y}`);
  app.call("dragStart", { x, y });
}

function resizeEvent(app: App, input: Extract<StdinEvent, { type: "mouse" }>) {
  const resizing = app.resizing;
  if (!resizing) return;
  const e = input.event;
  app.debug(`  ${e.type} button=${e.button} at ${e.x},${e.y} (${input.encoding})`);
  // VS Code can send SGR code 35 even DURING a held drag. Only treat that ambiguous
  // report as a missed release after this drag has used explicit left-button motion.
  // Otherwise the actual release, a fresh press, blur or Escape ends the resize.
  const button = input.encoding === "sgr"
    ? Number(input.raw.slice(3, input.raw.indexOf(";"))) & 3
    : e.button;
  if (e.type === "up" || e.type === "drag-end" || e.type === "down") {
    endResize(app, e.type === "down" ? "new press" : "released");
  } else if (e.type === "drag" || e.type === "move") {
    const ambiguousMotion = input.encoding === "sgr" && button === 3;
    const released = ambiguousMotion ? resizing.sawButtonMotion : button !== 0;
    if (released) {
      endResize(app, "left button released");
    } else {
      if (input.encoding === "sgr" && button === 0) resizing.sawButtonMotion = true;
      if (e.x !== resizing.x || e.y !== resizing.y) {
        resizing.x = e.x;
        resizing.y = e.y;
        app.call("dragMove", { x: e.x, y: e.y });
      }
    }
  }
  if (!app.resizing) pointer(app, onDivider(app, e.x, e.y) ? "move" : "default");
}

export function endResize(app: App, reason: EndReason) {
  if (!app.resizing) return;
  app.resizing = undefined;
  app.debug(`resize end: ${reason}`);
  app.call("dragEnd");
  pointer(app, "default");
}
