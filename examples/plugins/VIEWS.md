# Views (UI 3)

A view is a tree of elements a plugin sends with `ui.view.set`; modisa draws it in the user's theme, keeps what the
user does in it (selection, scroll, what's typed, which tree nodes are open), and sends the plugin an action when
something happens. Elements are [ratatui](https://ratatui.rs)'s own model: constraint layouts, blocks, and ratatui's
widgets, plus a few modisa draws itself (code, diffs, Markdown, big text, images, inputs, trees, buttons, rasters).

The client library builds trees for you (`Layout`, `Block`, `List`, … in `modisa-plugin.ts`, or JSX). This page is
the wire format: what each element is and what it takes. `modisa plugin schema` prints the same as JSON Schema, and
`modisa view render tree.json --size 80x24` draws a tree without a session (for snapshot tests).

## Elements

Every element is a JSON object with a `type`. Any element can also have:

| Field | |
|---|---|
| `id` | names it: needed for anything interactive or stateful (list, table, tabs, tree, input, textarea, button, scrolling text, code, diff, markdown), and for `focus`. Unique within the view. |
| `size` | its constraint inside its parent `layout`, when the parent's `constraints` doesn't give one (see Constraints) |
| `block` | a Block drawn around it (see Block) |
| `style` | its area's base style (see Styles) |
| `hide_below` | `{ "width": n, "height": n }`: not drawn (its constraint gets no room) when its area would be smaller |

### Layout

```json
{ "type": "layout", "direction": "horizontal", "constraints": ["34", "*"], "spacing": 2, "children": [ … ] }
```

| Field | |
|---|---|
| `direction` | `vertical` (default) or `horizontal` |
| `constraints` | one per child (see Constraints); a child without one uses its own `size`, else `*` |
| `flex` | where leftover room goes: `legacy`, `start` (default), `end`, `center`, `space_between`, `space_around`, `space_evenly` |
| `spacing` | cells between children; negative overlaps them (adjacent borders then merge, see Block `merge`) |
| `margin` | `n`, or `[vertical, horizontal]` |
| `children` | elements |

### Constraints

Strings (numbers are lengths):

| | ratatui | |
|---|---|---|
| `"12"` or `12` | `Length(12)` | exactly 12 cells |
| `"30%"` | `Percentage(30)` | |
| `"1/3"` | `Ratio(1, 3)` | |
| `">=5"` | `Min(5)` | at least 5 |
| `"<=20"` | `Max(20)` | at most 20 |
| `"*"`, `"2*"` | `Fill(1)`, `Fill(2)` | shares what's left, by weight |

### Text

Text is spans in lines. Anywhere text is taken:

- a **Span** is a string, or `{ "text": "…", "style": Style }`, or `{ "icon": "claude-code" }` (an agent's mark in its colour);
- a **Line** is a string (one span), an array of Spans, or `{ "spans": [Span…], "style": Style, "align": "left|center|right" }`;
- a **Text** is a string (`\n` separates lines), an array of Lines, or `{ "lines": [Line…], "style": Style, "align": … }`.

Text never carries escape sequences: modisa strips them. To show a program's coloured output, use `text` with `ansi`.

### text

A paragraph.

| Field | |
|---|---|
| `text` | a Text |
| `ansi` | instead of `text`: a string with SGR colour codes (a program's coloured output); other escape sequences are dropped |
| `align` | `left` (default), `center`, `right` |
| `wrap` | `true` (default: at words, keeping indentation), `"trim"` (at words, trimming leading space), `false` (cut) |
| `scroll` | with an `id`, the user scrolls it (wheel, keys when focused); `"bottom"` starts at, and follows, the end |
| `scrollbar` | `true` to show one when it's scrollable |

### block

A Block on its own (an empty frame, a titled box) is `{ "type": "block", … }` with an optional `child`. As the
`block` field of another element, the same fields:

| Field | |
|---|---|
| `borders` | `"all"` (default), `"none"`, or any of `["top", "right", "bottom", "left"]` |
| `border_type` | `plain` (default), `rounded`, `double`, `thick`, `light_double_dashed`, `heavy_double_dashed`, `light_triple_dashed`, `heavy_triple_dashed`, `light_quadruple_dashed`, `heavy_quadruple_dashed`, `quadrant_inside`, `quadrant_outside` |
| `border_style` | Style of the border |
| `title` | a Line, drawn top-left; or `titles`: `[{ "content": Line, "position": "top|bottom", "align": "left|center|right" }]` |
| `padding` | `n`, `[vertical, horizontal]`, or `[top, right, bottom, left]` |
| `style` | Style inside it |
| `shadow` | `true`, or `{ "kind": "dark_shade|medium_shade|light_shade|block|overlay", "offset": [x, y], "style": Style }` |
| `merge` | where borders overlap (negative `spacing`): `replace` (default), `exact`, `fuzzy` |

### list

| Field | |
|---|---|
| `items` | Lines, or `{ "content": Text, "style": Style }` (an item can be several lines) |
| `selected` | the index selected when the view opens (the user's choice is kept across updates while the plugin doesn't change this) |
| `highlight_style` | Style of the selected item (default: the theme's selection) |
| `highlight_symbol` | e.g. `"▶ "`; `highlight_spacing`: `always`, `when_selected` (default), `never` |
| `direction` | `top_to_bottom` (default) or `bottom_to_top` |
| `scroll_padding` | items kept visible around the selection |
| `action` | runs on Enter or a double-click, with `{ "index": i }` |
| `change` | runs when the selection moves, with `{ "index": i }` |

### table

| Field | |
|---|---|
| `header`, `footer` | a Row |
| `rows` | Rows: `{ "cells": [Cell…], "style": Style, "height": n, "top_margin": n, "bottom_margin": n }`, or just an array of Cells |
| `widths` | Constraints, one per column (default: equal shares) |
| `column_spacing` | default 1; `flex` as for layout |
| `select` | what the cursor selects: `row` (default), `cell`, `column`, or `none` |
| `selected` | `[row, column]` (or a row index) |
| `row_highlight_style`, `column_highlight_style`, `cell_highlight_style`, `highlight_symbol`, `highlight_spacing` | |
| `action`, `change` | as for list, with `{ "row": r, "column": c }` |

A Cell is a Text, or `{ "content": Text, "style": Style, "span": n }` (`span`: it takes n columns).

### tabs

| Field | |
|---|---|
| `titles` | Lines |
| `selected` | index |
| `divider` | a Span (default `│`); `padding`: `[left, right]` Spans |
| `highlight_style` | |
| `action` / `change` | `{ "index": i }` when the user picks one (←/→ when focused, a click) |

### gauge, line_gauge

| Field | |
|---|---|
| `ratio` | 0 to 1 (or `percent`: 0 to 100) |
| `label` | a Span (default: the percentage) |
| `gauge_style` | the filled part's Style (gauge); `filled_style`, `unfilled_style`, `filled_symbol`, `unfilled_symbol` (line_gauge) |
| `unicode` | gauge: draw the edge with eighth blocks (default true) |

### sparkline

`data` (whole numbers, 0 or more; `null` for a missing one), `max`, `direction` (`left_to_right`, `right_to_left`), `bar_set`
(`nine_levels`, `three_levels`), `absent_symbol`, `absent_style`.

### bar_chart

| Field | |
|---|---|
| `groups` | `[{ "label": Line, "bars": [{ "value": n, "label": Line, "text_value": "…", "style": Style, "value_style": Style }] }]`; or `data`: `[["label", value], …]` for one group. Values are whole numbers, 0 or more |
| `direction` | `vertical` (default) or `horizontal` |
| `bar_width`, `bar_gap`, `group_gap`, `max` | |
| `bar_style`, `value_style`, `label_style` | |

### chart

| Field | |
|---|---|
| `datasets` | `[{ "name": Line, "data": [[x, y], …], "graph_type": "line|scatter|bar|area", "marker": Marker, "style": Style, "fill_to": y }]` |
| `x_axis`, `y_axis` | `{ "title": Line, "bounds": [min, max], "labels": [Span…], "labels_align": "left|center|right", "style": Style }` (bounds default to the data's) |
| `legend` | `top_right` (default), `top_left`, `top`, `left`, `right`, `bottom`, `bottom_left`, `bottom_right`, or `none` |

Markers: `dot`, `block`, `bar`, `braille` (default), `half_block`, `quadrant`, `sextant`, `octant`, or one character.

### canvas

| Field | |
|---|---|
| `x_bounds`, `y_bounds` | `[min, max]` |
| `marker` | as for chart |
| `background` | a colour |
| `shapes` | `{ "line": [x1, y1, x2, y2], "color": c }`, `{ "rectangle": [x, y, w, h], "color": c }`, `{ "circle": [x, y, r], "color": c }`, `{ "points": [[x, y], …], "color": c }`, `{ "map": "low|high", "color": c }`, `{ "text": Line, "at": [x, y] }`; `{ "layer": true }` starts a new layer |

### calendar

A month: `year`, `month` (1–12), `events` (`{ "2026-10-09": Style }`), `month_header` / `weekday_header` (Styles, or
`false`), `surrounding` (Style of other months' days, or `false`), `default_style`.

### fill, clear

`fill` paints its area with `symbol` (default space) in `style`. `clear` blanks what's under it (for something drawn
on top of other elements with a negative `spacing`).

### code

| Field | |
|---|---|
| `content` | the code |
| `language` | a name or file extension (`rust`, `ts`, `tsx`, `toml`, `Dockerfile`, …); none: guessed from the first line, else plain |
| `line_numbers` | `true`, or the first line's number |
| `highlight` | line numbers drawn as marked |
| `wrap` | default `false` (scrolls sideways) |
| `syntax_theme` | colours from one of the built-in syntax themes (`modisa plugin schema` lists them) instead of the user's theme |

### diff

A unified diff (`git diff` output, one file or many).

| Field | |
|---|---|
| `diff` | the diff |
| `language` | highlights the code in it (as for code); default from the file names in it |
| `view` | `unified` (default) or `split` |
| `line_numbers` | default `true` |
| `cursor` | `true` gives it a line cursor (j/k, arrows, clicks) |
| `marks` | indexes of body lines drawn as marked |
| `action` | Enter (or a click) on a line: `{ "line": i, "old": n, "new": n, "text": "…" }` |
| `change` | the cursor moved: the same |

### markdown

`content` (CommonMark plus GitHub tables, task lists, strikethrough, footnotes, alerts), scrollable with an `id`.
Fenced code is highlighted as `code` is.

### big_text

`text` (a Text), `pixel_size` (`full` default, `half_height`, `half_width`, `quadrant`, `third_height`, `sextant`,
`quarter_height`, `octant`), `align`, `style`.

### image

`data` (base64 PNG, JPEG or GIF; a GIF shows its first frame), `alt` (shown where it can't be drawn), `resize`
(`fit` default, `crop`, `scale`). Drawn with the terminal's graphics protocol (kitty, iTerm2, sixel) where it has one,
else in half-blocks.

### input, textarea

| Field | |
|---|---|
| `value` | the text it starts with (the user's text is kept across updates while the plugin doesn't change this) |
| `placeholder` | |
| `mask` | input: a character drawn for each one typed (passwords) |
| `line_numbers` | textarea |
| `action` | input: Enter; textarea: Ctrl+S (Enter is a newline); with `{ "value": "…" }` |
| `change` | every edit, at most every 150 ms: `{ "value": "…" }` |

### tree

| Field | |
|---|---|
| `items` | `[{ "id": "…", "text": Line, "children": [ … ] }]` (`id` unique among siblings) |
| `open` | the paths open when the view opens (a path is the ids from the root, e.g. `["src", "client"]`) |
| `selected` | a path |
| `highlight_style`, `highlight_symbol` | |
| `action` | Enter on a node: `{ "path": [ … ] }`; `change`: the selection moved; `toggle`: a node opened or closed (`{ "path": [ … ], "open": bool }`) |

### button

`label` (a Line), `action` (Enter, Space or a click), `style`, `focus_style`. With `block` it's a framed button.

### spinner

`label` (a Line), `set` (`braille` default, `dots`, `ascii`, `arrows`, `clock`, `circle`, `box`, `bounce`, `pulse`),
`style`. It animates while it's on screen.

### raster

A grid a plugin paints and repaints in place without resending the view: `id` (needed), `columns` (1–512), `rows`
(1–256), `cells`: base64 of three little-endian u32 per cell, row by row: its character's code point (printable, one
cell wide), then fg and bg, each `0x01000000` (the default), `0x02000000` + n (the theme's colour n of `fg`, `dim`,
`accent`, `warn`, `working`, `blocked`, `done`, `idle`), or `0xRRGGBB`. `ui.blit` with `{ "view", "id", "cells" }`
repaints it, cells of the same size. For animation.

## Styles

A Style is an object or a string.

```json
{ "fg": "$accent", "bg": "#1a1b26", "bold": true, "underline_color": "$warn" }
```
`"bold italic $accent on $bar"`

- Colours: a theme token (`$fg`, `$bg`, `$bar`, `$dim`, `$border`, `$focus`, `$accent`, `$warn`, `$working`,
  `$blocked`, `$done`, `$idle`), a hex colour (`#rrggbb`), a named one (`red`, `light-blue`, `gray`, …), an index
  (`"0"`–`"255"`: a colour is always a string), or `reset`. Prefer tokens: they follow the user's theme, light or dark.
- Modifiers (`true` to add, `false` to remove what's inherited): `bold`, `dim`, `italic`, `underlined`,
  `slow_blink`, `rapid_blink`, `reversed`, `hidden`, `crossed_out`.
- In the string form: modifier names, a colour (the foreground), and `on <colour>` (the background).

## Focus and keys

Tab and Shift+Tab move the keyboard between the view's interactive elements, in tree order; a click focuses what's
under it. A focused list, table, tree, diff, code, markdown or scrolling text takes ↑ ↓ (and j k), PageUp/PageDown,
Home/End and the wheel; tabs take ← →; inputs and textareas take typing. `ui.view.set`'s `focus` names the element
that gets the keyboard (once per update), and its `keys` (`[{ "key": "s", "action": "send", "description": "send" }]`)
run actions from anywhere in the view, shown in its footer and its help (`?`). Escape (or the prefix then `x`) closes
it, running its `close` action.

## Actions from a view

An element's `action`, `change` or `toggle` runs that action of the plugin with what the element holds as `ui`:
`{ "view": "…", "id": "…", "event": "action|change|toggle", … }` plus the fields listed for each element. It's what
the user typed or chose: treat it as data.

## Limits

A view: at most 5000 elements, 40 deep, 2 MB; text in one element at most 512 KB; an image at most 4 MB. A plugin
has at most 4 views open, a session 8. `ui.blit`: at most 60 repaints a second.
