// The settings page: section tabs across the top, the section's rows below. ↑↓ (or the pointer) moves
// the selection, ←→ changes a value, ↵ / click applies, tab switches section, esc closes. Every change
// applies at once and is saved to config.toml; the rows themselves live in ./settings-sections.ts.
import { BoxRenderable, TextAttributes, TextRenderable, type KeyEvent } from "@opentui/core";
import type { App } from "../context";
import { fit, floating, mix } from "../design";
import { frame, frameLayouts, open } from "./frame";
import { sections, type Row } from "./settings-sections";

const HEIGHT = 24;
const BOLD = TextAttributes.BOLD;
const focusable = (row: Row | undefined) => !!row && row.kind !== "heading";

export async function openSettings(app: App, start = "theme") {
  const { r } = app;
  const box = frame(app, "settings", HEIGHT);
  let paint = () => {};
  const all = sections(app, () => paint());
  let current = Math.max(0, all.findIndex((s) => s.name === start));
  let sel = 0;
  let pointerAt = ""; // where the last hover that counted happened
  let close: (v: null) => void = () => {};

  const rows = () => all[current]!.rows();
  const show = (index: number) => {
    all[current]!.leave?.();
    current = (index + all.length) % all.length;
    all[current]!.enter?.();
    const list = rows();
    sel = list.findIndex((row) => row.kind === "radio" && row.current);
    if (sel < 0) sel = list.findIndex(focusable);
    paint();
  };
  const select = (index: number) => {
    sel = index;
    const row = rows()[sel];
    if (row?.kind === "radio") row.preview?.();
    paint();
  };
  const move = (by: 1 | -1) => {
    const list = rows();
    for (let i = sel + by; i >= 0 && i < list.length; i += by) if (focusable(list[i])) return select(i);
  };
  const activate = (row: Row | undefined) => {
    if (row?.kind === "radio") row.apply();
    else if (row?.kind === "toggle") row.flip();
    else if (row?.kind === "choice") row.enter?.();
    else if (row?.kind === "action") row.run();
    paint();
  };

  paint = () => {
    if (box.isDestroyed) return;
    const { th } = app;
    const { w, h } = floating(r.width, r.height, 70, HEIGHT);
    const inner = w - 2;
    box.borderColor = th.focus;
    box.backgroundColor = th.bar;
    for (const child of box.getChildren()) child.destroyRecursively();
    const text = (parent: BoxRenderable, content: string, fg: string, extra: Record<string, any> = {}) => {
      const node = new TextRenderable(r, { content, height: 1, flexShrink: 0, fg, ...extra });
      parent.add(node);
      return node;
    };
    const grow = (parent: BoxRenderable) => parent.add(new BoxRenderable(r, { flexGrow: 1, height: 1 }));

    // section tabs: the current one filled, the others lift under the pointer
    const tabs = new BoxRenderable(r, { height: 1, flexShrink: 0, flexDirection: "row", paddingLeft: 1 });
    box.add(tabs);
    all.forEach((section, i) => {
      const on = i === current;
      const tab: TextRenderable = text(tabs, ` ${section.name} `, on ? th.bg : th.dim, {
        bg: on ? th.accent : th.bar, attributes: on ? BOLD : 0,
        onMouseDown: (e: any) => { e.stopPropagation(); if (e.button === 0 && !on) show(i); },
        onMouseOver: () => { if (!on) Object.assign(tab, { fg: th.fg, bg: mix(th.bar, th.fg, 0.12) }); },
        onMouseOut: () => { if (!on) Object.assign(tab, { fg: th.dim, bg: th.bar }); },
      });
      app.clickable.add(tab);
      text(tabs, " ", th.dim);
    });
    text(box, "─".repeat(Math.max(0, inner)), th.border);

    // the rows, scrolled so the selection stays in view
    const list = rows();
    if (!focusable(list[sel])) sel = list.findIndex(focusable); // e.g. once integrations have loaded
    const capacity = Math.max(1, h - 7);
    const first = Math.max(0, Math.min(sel - Math.floor(capacity / 2), list.length - capacity));
    list.slice(first, first + capacity).forEach((row, k) => {
      const index = first + k;
      if (row.kind === "heading") {
        if (k) text(box, "", th.dim); // space between groups
        return void text(box, fit(`  ${row.label}`, inner), th.dim, { attributes: BOLD });
      }
      const selected = index === sel;
      const line = new BoxRenderable(r, {
        width: inner, height: 1, flexShrink: 0, flexDirection: "row", backgroundColor: selected ? mix(th.bar, th.focus, 0.25) : th.bar,
        // Only a pointer that moved selects: every repaint rebuilds the rows, and the new row under a
        // resting pointer gets a fresh mouseover that would undo the arrow key that caused the repaint.
        onMouseOver: (e) => {
          const at = `${e.x},${e.y}`;
          if (at === pointerAt) return;
          pointerAt = at;
          if (sel !== index) select(index);
        },
        onMouseDown: (e) => { e.stopPropagation(); if (e.button === 0) { sel = index; activate(row); } },
      });
      box.add(line);
      app.clickable.add(line);
      text(line, selected ? " ▸ " : "   ", th.focus);
      const label = (s: string, width: number) => text(line, fit(s, width), th.fg, { attributes: selected ? BOLD : 0 });
      if (row.kind === "radio") {
        label(row.label, inner - 20);
        if (row.current) text(line, " ✓", th.done);
        grow(line);
        for (const color of row.swatches ?? []) text(line, "  ", th.fg, { bg: color });
        text(line, " ", th.fg);
      } else if (row.kind === "toggle") {
        text(line, row.on ? "[x] " : "[ ] ", row.on ? th.accent : th.dim);
        label(row.label, inner - 8);
      } else if (row.kind === "choice") {
        label(row.label.padEnd(20), 20);
        grow(line);
        const arrow = (glyph: string, by: 1 | -1) => {
          const node: TextRenderable = text(line, glyph, th.dim, {
            onMouseDown: (e: any) => { e.stopPropagation(); if (e.button === 0) { sel = index; row.step(by); paint(); } },
            onMouseOver: () => { node.fg = th.accent; },
            onMouseOut: () => { node.fg = th.dim; },
          });
          app.clickable.add(node);
        };
        arrow(" ◂ ", -1);
        text(line, fit(row.value, 12).padEnd(12), row.value === "off" ? th.dim : th.accent);
        arrow(" ▸ ", 1);
      } else {
        label(row.label.padEnd(17), 17);
        text(line, fit(row.status, 20).padEnd(20), { ok: th.done, warn: th.warn, accent: th.accent, dim: th.dim }[row.tone]);
        if (row.note && inner >= 60) text(line, fit(row.note, 16), th.dim);
        grow(line);
        if (selected) text(line, `${row.hint} `, th.accent);
      }
    });

    // footer: key hints, then apply / close
    box.add(new BoxRenderable(r, { flexGrow: 1 }));
    text(box, fit(" ↑↓ select   ←→ change   tab section", inner), th.dim);
    const buttons = new BoxRenderable(r, { height: 1, flexShrink: 0, flexDirection: "row", justifyContent: "center" });
    box.add(buttons);
    const button = (label: string, fg: string, bg: string, run: () => void) => {
      const node: TextRenderable = text(buttons, label, fg, {
        bg, attributes: BOLD,
        onMouseDown: (e: any) => { e.stopPropagation(); if (e.button === 0) run(); },
        onMouseOver: () => { node.bg = mix(bg, th.fg, 0.2); },
        onMouseOut: () => { node.bg = bg; },
      });
      app.clickable.add(node);
    };
    button(" ↵ apply ", th.bg, th.accent, () => activate(rows()[sel]));
    text(buttons, "  ", th.dim);
    button(" esc close ", th.fg, th.border, () => close(null));
  };

  const layout = frameLayouts.get(box)!;
  frameLayouts.set(box, () => { layout(); paint(); });
  await open<null>(app, box, (done) => {
    close = done;
    show(current);
    return (k: KeyEvent) => {
      const row = rows()[sel];
      if (k.name === "tab") show(current + (k.shift ? -1 : 1));
      else if (k.name === "up" || k.name === "k") move(-1);
      else if (k.name === "down" || k.name === "j") move(1);
      else if ((k.name === "left" || k.name === "h") && row?.kind === "choice") (row.step(-1), paint());
      else if ((k.name === "right" || k.name === "l") && row?.kind === "choice") (row.step(1), paint());
      else if (k.name === "return" || k.name === "space") activate(row);
      return true;
    };
  }, () => all[current]!.leave?.()); // drop an unapplied theme preview
}
