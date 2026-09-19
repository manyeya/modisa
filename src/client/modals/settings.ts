// The settings page: the sections down the side, the section's rows beside them, what the selected row does under
// them. Typing searches every section at once. ↑↓ (or the pointer) moves the selection, ←→ changes a value, ↵ / space
// / a click applies, tab switches section, the wheel scrolls, esc closes. Every change applies at once and is saved
// to config.toml; the rows themselves live in ./settings-sections.ts.
import { BoxRenderable, StyledText, TextAttributes, fg, type KeyEvent } from "@opentui/core";
import { CONFIG_PATH } from "../../config/config";
import { HOME } from "../../core/paths";
import type { App } from "../context";
import { fit, mix } from "../design";
import { blank, clear, footer, frame, frameLayouts, header, highlight, innerHeight, innerWidth, matches, open, pointerMoved, row, searchField, spacer, text, thumb, typed } from "./frame";
import { sections, type Row } from "./settings-sections";

const HEIGHT = 32;
const WIDTH = 100;
const NAV = 18; // the side list's width
const BOLD = TextAttributes.BOLD;
const focusable = (row: Row | undefined) => !!row && row.kind !== "heading";
const title = (s: string) => s[0]!.toUpperCase() + s.slice(1);

type Entry = { row: Row; section: number };

export async function openSettings(app: App, start = "theme") {
  const box = frame(app, HEIGHT, WIDTH);
  const place = frameLayouts.get(box)!;
  let paint = () => {};
  const all = sections(app, () => paint());
  let current = Math.max(0, all.findIndex((s) => s.name === start));
  let sel = 0;
  let query = "";
  const moved = pointerMoved();

  // what's listed: the current section's rows, or while searching, every section's matches under headings that
  // say where each one lives
  const entries = (): Entry[] => spaced(listed());
  // a blank line between groups
  const spaced = (list: Entry[]) => list.flatMap((e, i) => (i && e.row.kind === "heading" ? [{ row: { kind: "heading", label: "" } as Row, section: e.section }, e] : [e]));
  const listed = (): Entry[] => {
    if (!query) return all[current]!.rows().map((row) => ({ row, section: current }));
    const found: Entry[] = [];
    all.forEach((section, i) => {
      let heading = "";
      const rows = section.rows().flatMap((row) => {
        if (row.kind === "heading") return (heading = row.label, []);
        return [{ row, where: `${section.name} ${heading}` }];
      });
      let last = "";
      for (const { row, where } of matches(rows, query, (r) => r.row.label, (r) => `${r.where} ${r.row.about ?? ""} ${r.row.kind === "choice" ? r.row.value : ""}`)) {
        const group = where.slice(section.name.length + 1);
        if (group !== last || !found.some((e) => e.section === i)) found.push({ row: { kind: "heading", label: group ? `${section.name} · ${group}` : section.name }, section: i });
        last = group;
        found.push({ row, section: i });
      }
    });
    return found;
  };
  const counts = () => all.map((_, i) => (query ? entries().filter((e) => e.section === i && focusable(e.row)).length : 0));

  const show = (index: number) => {
    all[current]!.leave?.();
    current = (index + all.length) % all.length;
    query = "";
    all[current]!.enter?.();
    const list = entries();
    sel = list.findIndex((e) => e.row.kind === "radio" && e.row.current);
    if (sel < 0) sel = list.findIndex((e) => focusable(e.row));
    paint();
  };
  const select = (index: number) => {
    sel = index;
    const row = entries()[sel]?.row;
    if (row?.kind === "radio") row.preview?.();
    paint();
  };
  const move = (by: number) => {
    const list = entries();
    let to = sel;
    for (let i = sel + Math.sign(by), left = Math.abs(by); i >= 0 && i < list.length && left; i += Math.sign(by)) {
      if (focusable(list[i]!.row)) (to = i, left--);
    }
    if (to !== sel) select(to);
  };
  const activate = (row: Row | undefined) => {
    if (row?.kind === "radio") row.apply();
    else if (row?.kind === "toggle") row.flip();
    else if (row?.kind === "choice") row.enter?.();
    else if (row?.kind === "action") row.run();
    paint();
  };
  const search = (next: string) => {
    query = next;
    const list = entries();
    sel = list.findIndex((e) => focusable(e.row));
    paint();
  };

  paint = () => {
    if (box.isDestroyed) return;
    const { th } = app;
    place();
    clear(box);
    const inner = innerWidth(box);
    const body = Math.max(3, innerHeight(box) - 6); // header, search, gap … gap, about, footer
    const list = entries();
    if (!focusable(list[sel]?.row)) sel = list.findIndex((e) => focusable(e.row)); // e.g. once integrations have loaded

    header(app, box, "Settings", CONFIG_PATH.replace(HOME, "~"));
    searchField(app, box, query, "Search settings");
    blank(app, box);

    const main = row(app, box, { height: body });
    // the sections down the side; while searching, how many matches each has
    const nav = new BoxRenderable(app.r, { width: NAV, height: body, flexShrink: 0, flexDirection: "column" });
    main.add(nav);
    const found = counts();
    all.forEach((section, i) => {
      if (i >= body) return;
      const on = !query && i === current;
      const bg = on ? mix(th.bar, th.focus, 0.16) : th.bar;
      const line = row(app, nav, { bg });
      const dim = query && !found[i];
      text(app, line, " ", th.focus, { bg }); // left padding inside the highlight
      text(app, line, fit(` ${title(section.name)}`, NAV - 5), on ? th.fg : dim ? mix(th.dim, th.bar, 0.4) : th.dim, { bg, attributes: on ? BOLD : 0 });
      spacer(app, line);
      if (query && found[i]) text(app, line, `${found[i]} `, th.accent, { bg });
      line.onMouseDown = (e) => { e.stopPropagation(); if (e.button === 0) show(i); };
      line.onMouseOver = () => { if (!on) line.backgroundColor = mix(th.bar, th.fg, 0.07); };
      line.onMouseOut = () => { line.backgroundColor = bg; };
      app.clickable.add(line);
    });
    text(app, main, Array(body).fill(" │ ").join("\n"), th.border, { width: 3, height: body });

    // the rows, scrolled so the selection stays in view
    const rows = new BoxRenderable(app.r, { flexGrow: 1, height: body, flexDirection: "column", minWidth: 0 });
    main.add(rows);
    rows.onMouseScroll = (e) => move(e.scroll?.direction === "up" ? -3 : 3);
    const width = inner - NAV - 3 - 1; // the scrollbar's column
    const first = Math.max(0, Math.min(sel - Math.floor(body / 2), list.length - body));
    const bar = thumb(list.length, body, first, body);
    list.slice(first, first + body).forEach(({ row: r }, k) => {
      const index = first + k;
      const selected = index === sel;
      const bg = selected ? mix(th.bar, th.focus, 0.16) : th.bar;
      const line = row(app, rows, { bg });
      const scrollbar = () => text(app, line, bar && k >= bar.top && k < bar.top + bar.size ? "▐" : " ", th.border, { bg });
      if (r.kind === "heading") {
        text(app, line, fit(` ${r.label.toUpperCase()}`, width), th.dim, { attributes: BOLD });
        spacer(app, line);
        return scrollbar();
      }
      line.onMouseOver = (e) => {
        if (!moved(e) || selected) return;
        if (app.cfg.mouse.hover) select(index);
        else line.backgroundColor = mix(th.bar, th.fg, 0.07); // under the pointer, not selected
      };
      line.onMouseOut = () => { if (!line.isDestroyed) line.backgroundColor = bg; };
      line.onMouseDown = (e) => { e.stopPropagation(); if (e.button === 0) { sel = index; activate(r); } };
      app.clickable.add(line);
      text(app, line, " ", th.focus, { bg }); // left padding inside the highlight
      const label = (room: number) => text(app, line, new StyledText([fg(th.fg)(" "), ...highlight(fit(r.label, room).padEnd(room), query, th.fg, th.accent, selected ? BOLD : 0)]), th.fg, { bg });
      if (r.kind === "radio") {
        text(app, line, r.current ? " ◉" : " ○", r.current ? th.accent : th.dim, { bg });
        label(width - 23);
        spacer(app, line);
        if (r.current) text(app, line, "in use  ", th.done, { bg });
        for (const color of r.swatches ?? []) text(app, line, "  ", th.fg, { bg: color });
        text(app, line, " ", th.fg, { bg });
      } else if (r.kind === "toggle") {
        label(width - 10);
        spacer(app, line);
        text(app, line, r.on ? "● on " : "○ off", r.on ? th.accent : th.dim, { bg, attributes: r.on ? BOLD : 0 });
        text(app, line, "  ", th.fg, { bg });
      } else if (r.kind === "choice") {
        label(width - 24);
        spacer(app, line);
        const arrow = (glyph: string, by: 1 | -1) => text(app, line, glyph, selected ? th.fg : th.dim, {
          bg, run: () => { sel = index; r.step(by); paint(); }, hover: [th.accent, bg],
        });
        arrow(" ‹ ", -1);
        text(app, line, fit(r.value, 14).padEnd(14), r.value === "off" ? th.dim : th.accent, { bg });
        arrow(" › ", 1);
        text(app, line, " ", th.fg, { bg });
      } else {
        label(20);
        text(app, line, fit(` ${r.status}`, 22).padEnd(22), { ok: th.done, warn: th.warn, accent: th.accent, dim: th.dim }[r.tone], { bg });
        if (r.note && width >= 70) text(app, line, fit(r.note, 16), th.dim, { bg });
        spacer(app, line);
        if (selected) text(app, line, ` ${r.hint} `, th.fg, { bg: mix(th.bar, th.fg, 0.14) });
        text(app, line, " ", th.fg, { bg });
      }
      scrollbar();
    });
    if (!list.length) text(app, row(app, rows), fit(`  No settings match “${query}”`, width), th.dim);

    // what the selected row does
    blank(app, box);
    const about = list[sel]?.row;
    const aboutLine = row(app, box);
    text(app, aboutLine, fit(about && about.kind !== "heading" && about.about ? about.about : "", inner), th.dim);
    footer(app, box, [["↑↓", "move"], ["←→", "change"], ["↵", "apply"], ["tab", "section"], ["esc", "close"]], query ? "^u clear" : "");
  };

  frameLayouts.set(box, paint);
  await open<null>(app, box, () => {
    show(current);
    return (k: KeyEvent) => {
      const row = entries()[sel]?.row;
      if (k.name === "tab") show(current + (k.shift ? -1 : 1));
      else if (k.name === "up" || (k.ctrl && k.name === "p")) move(-1);
      else if (k.name === "down" || (k.ctrl && k.name === "n")) move(1);
      else if (k.name === "pageup") move(-8);
      else if (k.name === "pagedown") move(8);
      else if (k.name === "left" && row?.kind === "choice") (row.step(-1), paint());
      else if (k.name === "right" && row?.kind === "choice") (row.step(1), paint());
      else if (k.name === "return" || (k.name === "space" && !query)) activate(row);
      else {
        const next = typed(query, k);
        if (next !== undefined && next !== query) search(next);
      }
      return true;
    };
  }, () => { for (const section of all) section.leave?.(); }); // drop an unapplied theme preview
}
