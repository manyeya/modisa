// A list to choose from, with a search field on top: typing filters it, ↑↓ (or the pointer) selects, ↵ (or a click)
// chooses, the wheel scrolls, a right-click opens an item's own menu. pick() is the centred picker; menus are lists
// placed where they were asked for, whose items also answer to their own keys.
import { StyledText, TextAttributes, fg, type MouseEvent } from "@opentui/core";
import type { App, Option } from "../context";
import { fit, mix } from "../design";
import { keyName } from "../input/bindings";
import { blank, clear, footer, frame, frameLayouts, header, highlight, innerHeight, innerWidth, matches, open, pointerMoved, row, searchField, spacer, text, thumb, typed } from "./frame";

export type ListItem = { name: string; description?: string; value: string; key?: string; danger?: boolean; context?: (e: MouseEvent) => void; buttons?: ListButton[] };
// An action on the right of an item's row: a click chooses `value`, and so does ^key while the item is selected.
export type ListButton = { icon: string; value: string; key: string; title: string; danger?: boolean };
export type ListOptions = {
  title: string;
  meta?: string; // on the right of the header
  items: ListItem[];
  placeholder?: string;
  width?: number;
  rows?: number; // at most this many at a time
  at?: { x: number; y: number }; // a menu: placed here, and each item's key chooses it while nothing's typed
};

export function list(app: App, o: ListOptions): Promise<string | null> {
  const most = Math.max(1, Math.min(o.rows ?? 12, o.items.length));
  const height = most + 7; // border, header, search, gap, rows, gap, footer
  const box = frame(app, height, o.width ?? 72, o.at?.x, o.at?.y);
  let query = "";
  let shown = o.items;
  let sel = 0;
  let first = 0;
  let choose: (v: string | null) => void = () => {};
  const moved = pointerMoved();
  const place = frameLayouts.get(box)!;
  const keyWidth = Math.max(0, ...o.items.map((i) => Bun.stringWidth(i.key ?? ""))); // keycaps line up
  const buttonsWidth = Math.max(0, ...o.items.map((i) => (i.buttons?.length ?? 0) * 3)); // so do buttons: " ✎ " each

  const paint = () => {
    if (box.isDestroyed) return;
    const { th } = app;
    place();
    clear(box);
    const width = innerWidth(box);
    const capacity = Math.max(1, innerHeight(box) - 5); // header, search, gap … gap, footer
    first = Math.max(0, Math.min(first, sel, shown.length - capacity));
    if (sel >= first + capacity) first = sel - capacity + 1;

    header(app, box, o.title, o.meta ?? (query ? `${shown.length} of ${o.items.length}` : o.at ? "" : `${o.items.length}`));
    searchField(app, box, query, o.placeholder ?? (o.at ? "Filter, or press a key" : "Type to search"));
    blank(app, box);
    const bar = thumb(shown.length, capacity, first, capacity);
    const lines = shown.slice(first, first + capacity);
    lines.forEach((item, k) => {
      const index = first + k;
      const on = index === sel;
      const bg = on ? mix(th.bar, th.focus, 0.16) : th.bar;
      const line = row(app, box, { bg });
      line.onMouseOver = (e) => {
        if (!moved(e) || on) return;
        if (app.cfg.mouse.hover) (sel = index, paint());
        else line.backgroundColor = mix(th.bar, th.fg, 0.07); // under the pointer, not selected
      };
      line.onMouseOut = () => { if (!box.isDestroyed && !line.isDestroyed) line.backgroundColor = bg; };
      line.onMouseDown = (e) => {
        e.stopPropagation();
        if (e.button === 0) choose(item.value);
        else if (e.button === 2 && item.context) { choose(null); item.context(e); }
      };
      app.clickable.add(line);
      text(app, line, " ", th.focus, { bg }); // left padding inside the highlight
      const right = item.key ?? "";
      const rightWidth = (keyWidth ? keyWidth + 2 : 0) + buttonsWidth;
      const room = width - 3 - rightWidth; // the marker, a space before it, and the scrollbar
      const nameColor = item.danger ? th.blocked : th.fg;
      const name = fit(item.name, Math.max(1, Math.min(room, Math.max(Math.ceil(room * 0.55), room - Bun.stringWidth(item.description ?? "") - 2))));
      text(app, line, new StyledText([fg(th.fg)(" "), ...highlight(name, query, nameColor, th.accent, on ? TextAttributes.BOLD : 0)]), nameColor, { bg });
      const descRoom = room - Bun.stringWidth(name) - 3;
      if (item.description && descRoom >= 4) {
        text(app, line, "  ", th.dim, { bg });
        text(app, line, new StyledText(highlight(fit(item.description, descRoom), query, th.dim, mix(th.dim, th.accent, 0.6))), th.dim, { bg });
      }
      spacer(app, line);
      if (right) text(app, line, ` ${right.padStart(keyWidth)} `, on ? th.fg : th.dim, { bg: mix(th.bar, th.fg, on ? 0.14 : 0.07) });
      else if (keyWidth) text(app, line, " ".repeat(keyWidth + 2), th.dim, { bg });
      const buttons = item.buttons ?? [];
      if (buttonsWidth > buttons.length * 3) text(app, line, " ".repeat(buttonsWidth - buttons.length * 3), th.dim, { bg });
      for (const b of buttons) text(app, line, ` ${b.icon} `, on ? (b.danger ? th.blocked : th.fg) : th.dim, { bg, run: () => choose(b.value) });
      text(app, line, bar && k >= bar.top && k < bar.top + bar.size ? "▐" : " ", th.border, { bg });
    });
    if (!shown.length) {
      const line = row(app, box);
      text(app, line, fit(`  No matches for “${query}”`, width), th.dim);
    }
    for (let k = Math.max(lines.length, shown.length ? 0 : 1); k < capacity; k++) blank(app, box);
    blank(app, box);
    const buttonHints = (shown[sel]?.buttons ?? []).map((b): [string, string] => [`^${b.key}`, b.title]);
    footer(app, box, [["↑↓", "move"], ["↵", o.at ? "run" : "choose"], ...buttonHints, ["esc", "close"]], query ? "^u clear" : "");
  };

  const step = (by: number) => {
    if (!shown.length) return;
    sel = (sel + by + shown.length) % shown.length;
    paint();
  };
  const scroll = (by: number) => {
    sel = Math.max(0, Math.min(shown.length - 1, sel + by));
    paint();
  };
  box.onMouseScroll = (e) => scroll(e.scroll?.direction === "up" ? -3 : 3);
  frameLayouts.set(box, paint);
  paint();

  return open<string>(app, box, (done) => {
    choose = done;
    return (k) => {
      if (k.name === "up" || (k.ctrl && k.name === "p") || (k.name === "tab" && k.shift)) step(-1);
      else if (k.name === "down" || (k.ctrl && k.name === "n") || k.name === "tab") step(1);
      else if (k.name === "pageup") scroll(-8);
      else if (k.name === "pagedown") scroll(8);
      else if (k.name === "return") { if (shown[sel]) done(shown[sel]!.value); }
      else if (k.ctrl && shown[sel]?.buttons?.some((b) => b.key === k.name)) done(shown[sel]!.buttons!.find((b) => b.key === k.name)!.value);
      else {
        // a menu item's own key, while nothing's been typed
        const own = o.at && !query ? o.items.find((i) => i.key && i.key.length === 1 && i.key === keyName(k)) : undefined;
        if (own) return (done(own.value), true);
        const next = typed(query, k);
        if (next === undefined) return true;
        query = next;
        shown = matches(o.items, query, (i) => i.name, (i) => i.description ?? "");
        sel = 0;
        first = 0;
        paint();
      }
      return true;
    };
  });
}

export function pick(app: App, title: string, options: Option[], meta?: string) {
  return list(app, { title, meta, items: options.map((o) => ({ name: o.name, description: o.description, key: o.key, value: o.value, context: o.context })) });
}
