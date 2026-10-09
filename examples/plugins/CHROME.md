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
| `id` | names this piece among the plugin's pieces in the slot (1–40 chars); setting it again replaces it |
| target | what it's about, for slots that are per something: `pane` + `instance` (a pane's current process; it goes when that process does), `tab` (a tab's id), or `space` (a workspace's name) |
| `position` | `before`, `after` (default) or `replace` (see Replacing) |
| `order` | among pieces at the same place: lower first (default 0; ties by plugin name, then id) |
| `line` | a Line (VIEWS.md); or `lines` (several, where the slot has room for them: agent rows, sidebar sections, toasts); or `element` (a UI 3 element: sidebar sections only) |
| `action` | runs on a click (and Enter where the slot has the keyboard), with `ui: { "slot", "id", "pane"?, "instance"?, "tab"?, "space"? }` |
| `title` | menu and palette entries: the entry's text (a string) |
| `hide_below` | `{ "width": n }`: not drawn when the terminal is narrower |

`ui.slot.clear { slot?, id?, pane?, tab?, space? }` removes the plugin's matching pieces (nothing given: all of them).

## Slots

| Slot | Per | Where | Takes |
|---|---|---|---|
| `status.left`, `status.right` | — | the status row, after modisa's buttons / before its right-hand segments | `line` |
| `status.agents`, `status.panes`, `status.git`, `status.theme` | — | modisa's own status segments (working/need-you counts, pane count, the repository, the theme's name) | `line`, around or instead of it |
| `tab` | `tab` | a tab's label in the tab bar | `line` |
| `space` | `space` | a space's chip in the tab bar | `line` |
| `pane.title` | `pane` | a pane's border title (its name, agent and state) | `line` |
| `pane.top_right`, `pane.bottom_left`, `pane.bottom_right` | `pane` | the other places on a pane's border | `line` (no `replace`: nothing of modisa's is there) |
| `agent.row` | `pane` | the agent's rows in the sidebar's AGENTS list | `lines` (at most 3) |
| `sidebar` | — | a section of the plugin's own in the sidebar, titled with `title` | `lines` (rows: each a Line, with `action`), or `element` and `height` (rows it takes, 1–30) |
| `sidebar.agents` | — | the AGENTS list itself | `replace` only: `lines` or `element` |
| `menu.pane`, `menu.tab`, `menu.space` | `pane` / `tab` / `space`, or none for every one | right-click menus | `title`, `action` (no `line`) |
| `palette` | — | the command palette | `title`, `action`, optional `line` (shown beside it) |
| `toast` | — | a toast (`ui.toast` with rich text) | `lines`, `actions`: `[{ "title", "action" }]` (buttons), `timeout` (ms) |

A sidebar `element` is drawn like a view's (lists, gauges, sparklines, trees…); its interactive elements work with
the mouse, and their actions come with `ui.slot` set to `sidebar`.

The older methods are slots too, and keep working: `ui.status.set` is `status.right`, `ui.badge.set` is
`pane.title` `after`, `ui.sidebar.set` is `sidebar`, `ui.menu.set` is `menu.pane` entries; their `text` and `tone`
are a one-span Line (`tone` → `$tone`).

## Replacing

`position: "replace"` asks to draw instead of modisa. It takes effect when the user lets it:

```toml
[slots]
"agent.row" = "radar"        # radar's agent rows replace modisa's
"pane.title" = "builtin"     # nobody replaces pane titles
```

With no `[slots]` entry for a slot, the first plugin (by name) that asks to replace it does, and `modisa plugin list`
shows who holds which slot and who asked. A plugin that replaces a per-something slot (an agent's row, a pane's
title) replaces it only where it has set a piece; elsewhere modisa draws its own. `sidebar.agents` is the old
`[sidebar] agents = "plugin"` setting, which still works.

## What it costs

Slot updates are drawn at most 30 times a second per plugin. A plugin has at most 500 pieces across all slots; a
line at most 200 cells, `lines` at most 3 (30 for `sidebar`), an `element` the view limits. Text is cleaned as in
views.
