# Slots: plugins in modisa's own chrome (UI 4)

Views (VIEWS.md) are a plugin's own windows. Slots are the rest of modisa's screen: the status row, the tab bar,
spaces, pane borders, the sidebar's agent rows and sections, menus, the palette and toasts. A plugin puts content in
a slot **before** or **after** what modisa draws there, or **replaces** it. Content is UI 3 text (Spans, Lines,
Styles, `$tokens`), so it follows the user's theme; a sidebar section can be a whole UI 3 element.

The user decides who may replace what (`[slots]` in config.toml); everything else just adds. All of it is attributed
to its plugin (hover shows whose it is), and goes when the plugin stops.

## ui.slot.set

```json
{ "slot": "agent.row", "pane": "p3", "instance": "…", "id": "ctx", "position": "after",
  "line": [{ "text": "43%", "style": "$warn" }], "action": "details" }
```

| Field | |
|---|---|
| `slot` | where (see Slots) |
| `id` | names this piece among the plugin's pieces in the slot (1–40 chars); setting it again (same slot, id and pane, tab or space) replaces it |
| target | what it's about, for slots that are per something: `pane` + `instance` (a pane's current process, running; it goes when that process does), `tab` (a tab's id; a name is taken too), or `space` (a space's name; an id is taken too). A target a slot isn't per is refused |
| `position` | `before`, `after` (default) or `replace` (see Replacing) |
| `order` | among pieces at the same place: lower first (a whole number, default 0; ties by plugin name, then id) |
| `line` | a Line (VIEWS.md); or `lines` (several, where the slot has room for them: agent rows and sidebar sections, where one `line` is taken as `lines` of one); or `element` (a UI 3 element: sidebar sections only) |
| `action` | runs on a click (and Enter where the slot has the keyboard); see Actions |
| `title` | menu and palette entries: the entry's text (a string, at most 40 cells); a sidebar section's heading (at most 30) |
| `hide_below` | `{ "width": n }`: not drawn when the terminal is narrower |

It answers `true`. `ui.slot.clear { slot?, id?, pane?, tab?, space? }` removes the plugin's pieces that match all
that's given (nothing given: all of them) and answers how many went.

## Slots

| Slot | Per | Where | Takes |
|---|---|---|---|
| `status.left`, `status.right` | — | the status row, after modisa's buttons / before its right-hand segments | `line` (no `replace`: nothing of modisa's is there) |
| `status.agents`, `status.panes`, `status.git`, `status.theme` | — | modisa's own status segments (working/need-you counts, pane count, the repository, the theme's name) | `line`, around or instead of it |
| `tab` | `tab` | a tab's label in the tab bar | `line` |
| `space` | `space` | a space's chip in the tab bar | `line` |
| `pane.title` | `pane` | a pane's border title (its name, agent and state) | `line` |
| `pane.top_right`, `pane.bottom_left`, `pane.bottom_right` | `pane` | the other places on a pane's border | `line` (no `replace`: nothing of modisa's is there) |
| `agent.row` | `pane` | the agent's rows in the sidebar's AGENTS list | `lines` (at most 3) |
| `sidebar` | — | a section of the plugin's own in the sidebar, headed by its name and `title`; `before` puts it above the AGENTS list | `lines` (rows, at most 30: see below), or `element` and `height` (rows it takes, 1–30); no `replace` |
| `sidebar.agents` | — | the AGENTS list itself | `replace` only (the default here): `lines` or `element` (`height` optional: it has the list's room) |
| `menu.pane`, `menu.tab`, `menu.space` | `pane` / `tab` / `space`, or none for every one | right-click menus | `title`, `action` (no `line`) |
| `palette` | — | the command palette | `title`, `action`, optional `line` (shown beside it) |
| `toast` | — | a toast: not a piece, so not `ui.slot.set`; `ui.toast` takes it (see Toasts) | |

A sidebar row is a Line; in the object form, `{ "spans": [...], "style", "align" }`, it can also have its own
`action`, or a `pane` with its `instance` (a click focuses that process, or says it's gone). A sidebar `element` is
drawn like a view's (lists, gauges, sparklines, trees…); its interactive elements work with the mouse.

The older methods are slots too, and keep working: `ui.status.set` is `status.right`, `ui.badge.set` is
`pane.title` `after` (id `badge`), `ui.sidebar.set` is `sidebar` (id `sidebar`), `ui.menu.set` is `menu.pane` entries
for every pane. Their `text` and `tone` are a one-span Line (`tone` → `$tone`), named the way they were always drawn,
so they can't pass for modisa's own: `radar: 3 waiting`, `[radar: blocked]`; a sidebar row's spans keep their tones
(`bold` too). They keep their own limits (4 status segments, 12 across the session's plugins, and so on) and rate.

## Actions

A piece's `action` (or a sidebar row's) runs that action of the plugin with what it came from as `ui`:
`{ "slot", "id", "pane"?, "instance"?, "tab"?, "space"?, "row"? }`, where `row` is the line clicked in a section's
`lines`. A menu entry for every pane (tab, space) adds the one it was opened on, and its action gets that pane as
`target`, as before. A sidebar element's interactive elements send what a view's do (VIEWS.md: `id` is the
element's, `event`, `index`, `value`, …), with `slot` and `piece` (the piece's id). A toast's button sends
`{ "slot": "toast", "id"? }` (the toast's `id`, if it had one). It's what the user did: treat it as data.

## Toasts

`ui.toast` takes, besides `text`, `tone` and `system`: `lines` (Lines, at most 3; with them `text` may be left out,
and older clients show the lines' text), `actions` (`[{ "title", "action" }]`, at most 3 buttons), `timeout` (ms, 1000
to 60000) and `id` (1–40 characters, what its buttons' actions say). Toasts aren't kept: a client attached later
never sees one, and they keep their budget (3 every 10 s per plugin run, 6 across the session's plugins).

## Replacing

`position: "replace"` asks to draw instead of modisa. It takes effect when the user lets it:

```toml
[slots]
"agent.row" = "radar"        # radar's agent rows replace modisa's
"pane.title" = "builtin"     # nobody replaces pane titles
```

With no `[slots]` entry for a slot, the first plugin (by name) that asks to replace it does, and `modisa plugin list`
shows who holds which slot and who asked. A plugin that replaces a per-something slot (an agent's row, a pane's
title) replaces it only where it has set a piece; elsewhere modisa draws its own. A replacing piece that isn't let
isn't drawn at all (not even as `after`). `sidebar.agents` is the old `[sidebar] agents = "plugin"` setting, which
still works: `[slots]` outranks it, and the plugin either names gets its first sidebar section put in the AGENTS
list's place when it has no `sidebar.agents` piece of its own. In `ui.state`, a replacing piece that's drawn
`replaces`.

## What it costs

Clients get slot updates at most 30 times a second, however many there are. `ui.slot.set` and `ui.slot.clear` aren't
refused for rate, except a piece with an `element`, which counts like a view update. A plugin has at most 500 pieces
across all slots; a line at most 200 cells (an agent's mark takes two; what's past it is dropped), `lines` at most 3
(30 for `sidebar` and `sidebar.agents`), an `element` the view limits. Text is cleaned as in views.

## What clients receive

A client that attaches with `ui: 4` (types.rs `PLUGIN_UI`) gets, in the attach result and every `view`:

- `plugins`: per running plugin `{ plugin, run, actions, keys, panes, links }` as before, but **no** `status`,
  `sidebar`, `badges` or `menu`: those are pieces now.
- `slots`: `View.slots`, a `SlotPiece` (types.rs) per piece of every running plugin, left out when there are none. A
  piece change sends a new view at most 30 times a second; and every view carries them all.

```json
{ "plugin": "usage", "run": "1a2b3c4d", "slot": "agent.row", "id": "ctx", "position": "after", "order": 0,
  "pane": "p3", "instance": "9f8e7d6c", "lines": [[{ "text": "43%", "style": "$warn" }]], "action": "details" }
```

- Always: `plugin`, `run` (the run to name in `plugin.invoke`), `slot`, `id`, `position` (`before`, `after`,
  `replace`), `order`. The list is sorted by `order`, plugin, then id.
- Its target, for slots per something: `pane` and `instance` (a running process: draw it for that instance only),
  `tab` (a tab's id), `space` (a space's name). Menu entries without one are for every pane (tab, space). Pieces for a
  process that ended, or a tab or space that's gone, are out of the next view.
- What it shows, checked and cleaned by the server (read it with `src/protocol/ui.rs`): `line` (single-line slots, and
  a palette entry's extra), `lines` (`agent.row`, `sidebar`, `sidebar.agents`: always an array, even of one; a sidebar
  row can be an object with `spans` plus `action`, or `pane` and `instance`), or `element` (sidebar slots) and
  `height` (rows: on `sidebar` always, on `sidebar.agents` if given); `title` (menu and palette entries, and sidebar
  sections when given); `action`; `hide_below` (`{ "width": n }`).
- `replaces`: replacing is decided. It's true on the `replace` pieces drawn instead of modisa's own (for a
  per-something slot, just for its target); a `replace` piece without it lost and isn't drawn (it says who asked).
  When `[sidebar] agents` (or `[slots]`) gives the AGENTS list's place to a plugin's sidebar section, that section
  comes as `slot: "sidebar.agents"`, `position: "replace"`, `replaces: true`, with its `title`, and not as a
  `sidebar` piece too. `before` and `after` pieces still go around a replacement.

How the older methods' pieces look, which the end-to-end suite checks on screen: a status segment is a `status.right`
piece whose line already reads `ui-demo: 2 need you`; a badge is a `pane.title` `after` piece reading
`[ui-demo: needs you]`; a sidebar section is headed `▾ ui-demo` with its title on the row under it, its rows each a
Line object with the row's own `action`, or `pane` and `instance` (a click focuses that process: `pane.focus` with
target `p3:<instance>`, as a sidebar row always did); pane menu entries are `title` and `action`, shown as
`ui-demo: Mark seen`. With `[sidebar] agents = "ui-demo"` the suite expects that section where the AGENTS list was,
still headed `▾ ui-demo`, and no `AGENTS`.

A piece's action runs with `plugin.invoke { plugin, run, action, ui }` (`ui` as in Actions), plus
`target: { pane, instance }` for a piece about a pane or a menu entry taken on one: the server then refuses it if that
process is gone.

Toasts aren't in `View.slots`: they're `plugin.toast` notifications as before (`plugin`, `text`, `tone`, `system`,
`sound`?), from a plugin also with `slot: "toast"`, `run` and, when it gave them, `id`, `lines`, `actions`
(`[{ title, action }]`) and `timeout`, so a client can read one as a `SlotPiece`.

A client attaching with `ui` 1 to 3 gets what it always did: `plugins` with `status`, `sidebar`, `badges` and `menu`
from the older methods only (what `ui.slot.set` sets isn't there), and toasts' extra fields, which it ignores. It
never gets `slots`. A client with no `ui` gets no plugin UI at all.
