# Plugin permissions, settings and tooling

## Permissions

A plugin says in `plugin.json` what it needs from modisa, and gets only that:

```json
{ "name": "review", "protocol": 1, "run": ["bun", "plugin.ts"],
  "permissions": ["panes.read", "ui", "messages"] }
```

| Permission | Lets it |
|---|---|
| `ui` | show things: status, sidebar, badges, menus, toasts, views, slots that add (`before`/`after`) |
| `ui.replace` | ask to replace modisa's own widgets (CHROME.md `replace`; the user still decides who gets each slot) |
| `panes.read` | read panes' text (`pane.read`), their process info, and `pane.output` events |
| `panes.control` | type into panes and run commands in them (`pane.keys`, `pane.run`), split, close, move, resize, focus and zoom them, open plugin panes and popups |
| `agents` | start agents (`agent.spawn`) |
| `messages` | send messages to agents and read the inbox (`send`, `inbox`, `messages`) |
| `notify` | system notifications and sounds (`notify` with `system`/`sound`, `ui.toast` with `system`) |
| `sessions` | workspaces and tabs: create, rename, close them; `restart` and `kill` are never a plugin's |

Without the permission a request fails with `permission_denied`, naming what's missing. Everyone gets the rest: the
session's snapshot, agents' states and the other events (`events.subscribe`), `plugin.hello`, `protocol.describe`.

The user grants them: `modisa plugin install` and `link` list what a plugin asks for and, at a terminal, ask before
it starts (from a script nobody is asked, and it gets what it asks for). Grants are kept in
`~/.config/modisa/plugin-grants.json`. A plugin gets only what it both asks for and was granted: one that later asks
for more (an update) starts with what was granted, and `modisa plugin list` says what it lacks until
`modisa plugin grant <name> [permission…]` (none named: all it asks for). `modisa plugin revoke <name> [permission…]`
takes them away; either way, `modisa plugin restart <name>` for a running one to have the change. A plugin with no
`permissions` field (written before them) may do everything a plugin could before, and `modisa plugin list` says it
doesn't declare.

Permissions are about what a plugin does through modisa. A plugin is still a program running as you: it can do on
your machine whatever you can. Install what you trust.

## Settings

A plugin declares its settings, and modisa shows them in its settings page (prefix `s` → Plugins → the plugin):

```json
"settings": [
  { "key": "threshold", "type": "number", "default": 80, "min": 0, "max": 100, "title": "Warn above (%)" },
  { "key": "provider", "type": "enum", "options": ["claude", "codex", "both"], "default": "both", "title": "Show" },
  { "key": "compact", "type": "boolean", "default": false, "title": "One-line status" },
  { "key": "label", "type": "string", "default": "", "title": "Label", "max": 40 }
]
```

Values live in `~/.config/modisa/plugin-config/<name>/settings.json` (`$MODISA_PLUGIN_CONFIG/settings.json`). The
plugin gets them with the `plugin.settings` request (and in `plugin.hello`'s result), and a `plugin.settings.changed`
notification (`{ settings }`) when the user changes one in the settings page. The client library:
`await modisa.settings()`, `modisa.onSettings(fn)`.

## Tooling

| | |
|---|---|
| `modisa plugin validate <dir> [--json]` | checks a plugin without running it: its manifest, the permissions it asks for, its actions, keys, panes, links and settings, which client library version it carries, and that its `run` command resolves. Exit 1 on errors. |
| `modisa plugin dev <dir> --watch` | runs it in a throwaway session and restarts it whenever a file in its directory changes |
| `modisa plugin restart <name>` | stops and starts it in the running session |
| `modisa view render tree.json --size 80x24` | draws a view without a session; the client library's `renderView(tree, { width, height, theme })` returns the same text, for snapshot tests |
| `modisa plugin schema` | every request, result and event as JSON Schema, views and slots included |

`modisa plugin new` scaffolds a plugin that declares `permissions: ["ui"]`.
