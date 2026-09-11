// A small floating menu at (x, y): arrows/tab + Enter, mouse, or each option's key. Resolves to the action.
import { TextRenderable } from "@opentui/core";
import type { App } from "../context";
import { fit, floating } from "../design";
import { keyName } from "../input/bindings";
import { frame, frameLayouts, open } from "./frame";

export type MenuOption = { name: string; key: string; action: string; danger?: boolean };

export function menu(app: App, title: string, options: MenuOption[], x: number, y: number): Promise<string | null> {
  const { r } = app;
  const box = frame(app, title, options.length + 4);
  let selected = 0;
  let done: (value: string | null) => void;
  const paint = () => {
    const { th } = app;
    for (const child of box.getChildren()) child.destroyRecursively();
    const rect = floating(r.width, r.height, 34, options.length + 4, x, y);
    Object.assign(box, { left: rect.x, top: rect.y, width: rect.w, height: rect.h });
    const capacity = Math.max(1, rect.h - 4);
    const start = Math.max(0, selected - capacity + 1);
    options.slice(start, start + capacity).forEach((option, offset) => {
      const index = start + offset;
      const active = selected === index;
      const width = Math.max(1, rect.w - 2);
      const label = fit(` ${active ? "▸" : " "} ${option.name}`, Math.max(1, width - 6));
      const text = label + " ".repeat(Math.max(1, width - Bun.stringWidth(label) - option.key.length - 1)) + option.key;
      const row = new TextRenderable(r, {
        content: fit(text, width), height: 1, width: "100%", flexShrink: 0,
        fg: active ? th.bg : option.danger ? th.blocked : th.fg,
        bg: active ? th.focus : th.bar,
        onMouseOver: () => { if (selected !== index) { selected = index; paint(); } },
        onMouseDown: (e) => { e.stopPropagation(); if (e.button === 0) done(option.action); },
      });
      box.add(row);
    });
    box.add(new TextRenderable(r, { content: fit(" ──────────────────────────────", rect.w - 2), fg: th.border, height: 1 }));
    box.add(new TextRenderable(r, { content: fit(" ↑↓ navigate · esc dismiss", rect.w - 2), fg: th.dim, height: 1 }));
  };
  frameLayouts.set(box, paint);
  paint();
  return open<string>(app, box, (close) => {
    done = close;
    return (k) => {
      if (k.name === "up" || k.name === "down" || k.name === "tab") {
        selected = (selected + (k.name === "up" || k.shift ? -1 : 1) + options.length) % options.length;
        paint();
      } else if (k.name === "return") close(options[selected]!.action);
      else {
        const option = options.find((o) => o.key === keyName(k));
        if (option) close(option.action);
      }
      return true;
    };
  });
}
