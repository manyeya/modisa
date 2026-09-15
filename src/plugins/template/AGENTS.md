# Writing the {{name}} shepherd plugin

This directory is a shepherd plugin. Shepherd starts it with the session server and connects it to the session's
socket; the plugin reacts to what happens in the session. Put the user's logic in `plugin.ts` and leave the protocol
to `shepherd-plugin.ts`.

## Files

- `plugin.json`: name, protocol version, and how to start it (`run`: argv, run in this directory)
- `plugin.ts`: the plugin
- `shepherd-plugin.ts`: shepherd's client library. Don't edit it; refresh it with `shepherd plugin sdk > shepherd-plugin.ts`
- `plugin.test.ts`: behavioural tests, run against a real throwaway session by `shepherd plugin check .`

## The loop

```sh
shepherd plugin check .          # manifest, build, then a throwaway session: it starts, connects, passes your tests,
                                 # and exits when the session dies. Fix what it reports, run it again.
shepherd plugin dev .            # a throwaway session with the plugin running, to try it by hand
shepherd plugin link .           # install it; it starts with the session server (shepherd restart)
shepherd plugin logs {{name}}    # its stdout and stderr
shepherd plugin run {{name}} status '{"any":"params"}'
shepherd plugin stop {{name}}    # and plugin start {{name}}
```

To share it, push it to a public git repository and give the repository the `shepherd-tui-plugin` topic:
`shepherd plugin search` finds it, and `shepherd plugin install <url>` installs it. Put `plugin.json` at the top of the
repository, or, for a plugin in a subdirectory, say in the repository's README which `--subdir` to install.

## The library

- `runPlugin(async (shepherd) => …)` connects, runs your code, and exits when the session's connection closes.
  Don't reconnect or loop: the next server starts the plugin again.
- `shepherd.hello({ name: (params) => result })` binds the plugin and offers actions to `shepherd plugin run`.
  Throwing in an action returns an error to the caller. An action's second argument is `{ invocation, signal }`:
  `signal` aborts if shepherd stops waiting (30s). That's advisory. The caller has already been told the outcome is
  unknown, stopping early can't undo what the action already did, and nothing retries it.
- `shepherd.subscribe({ onSnapshot, onEvent })`: `onSnapshot` gets every pane as the subscription starts; `onEvent`
  gets each later event once, in order, one at a time. Whatever the snapshot shows was already true when the plugin
  started: don't treat it as news.
- `shepherd.request(method, params)` for anything else. Errors are `ShepherdError`, with a stable `code`.
- `$SHEPHERD_PLUGIN_DATA` is a directory for the plugin's own files.

## Showing things in shepherd's TUI

A plugin can put a little into the TUI; shepherd draws it in the user's theme and names the plugin on every piece, so
nothing a plugin shows can pass for shepherd's own. Everything is cleared when the plugin stops.

- From the library, after `hello`:
  - `shepherd.ui.status(id, text, { tone, action })`: a segment in the status row (up to 4); clicking runs `action`.
  - `shepherd.ui.sidebar(title, rows)`: the plugin's section in the sidebar (up to 20 rows). A row runs its `action`,
    or focuses its `pane`; a row's `pane` comes with that pane's `instance`, so a click reaches that process or says
    it's gone.
  - `shepherd.ui.badge(pane, instance, text, tone)`: a label on a pane's border, for that process only.
  - `shepherd.ui.menu(items)`: entries in the pane context menu (up to 8).
  - `shepherd.ui.toast(text, { tone, system })`: a passing message in every attached client (3 every 10s).
  - `clearStatus`, `clearSidebar`, `clearBadge` and `closePopup` take them away; `shepherd.ui.state()` is what shows now.
- Tones are `fg`, `dim`, `accent` and `warn`, mapped to the user's theme. Text loses control characters and escape
  sequences and is cut to fit.
- An `action` must be one the plugin offered in `hello`. Offer them in `plugin.json` too, with titles, so they're listed
  in the command palette:
  `"actions": [{ "id": "clear", "title": "Clear the log" }]`.
- `"panes"` in `plugin.json` are programs the plugin can open, run in its directory with its environment:
  `{ "id": "log", "title": "Attention log", "run": ["tail", "-f", "attention.log"], "placement": "popup" }`. Placements:
  `split`, `tab` and `zoomed` open ordinary panes that stay after the plugin stops; `overlay` opens zoomed over the
  focused pane and gives focus back when it closes; `popup` floats over everything in the one client that asked (one at
  a time; `width` and `height` as cells or `"80%"`; prefix x closes it) and closes when the plugin stops.
- `"keys"` bind a key under the prefix to an action or a pane: `{ "key": "A", "pane": "log", "description": "show the
  log" }`. A key shepherd uses, or one two plugins want, is off for both (`shepherd plugin list` says why); the user can
  move a key in `[plugin_keys]` in config.toml.
- An action taken from a menu entry, key or palette entry gets the pane it was taken on as `call.target`
  (`{ pane, instance }`), apart from its params, already checked to be that pane's current process.
- Limits: updates are rate-limited (10 a second, bursts of 30) and so is the whole session's (30 a second across its
  plugins); a session also shows at most 12 status segments, 6 sidebar sections, 24 menu entries and 4 plugins' badges
  on one pane across all its plugins, first come first served. Over a limit a call fails with `rate_limited` or an
  error: show less, or less often, rather than retrying in a loop.
- The plugin runs with the session's server, also when the user attaches from another machine: its UI is drawn by
  whichever client is attached. While a client is disconnected it shows none of it; a client too old to draw plugin UI
  gets none, and a popup can't open in it.

## Links

`"links"` in `plugin.json` hands a URL the user Ctrl+clicks in a pane to one of the plugin's actions. When several
plugins' links match, shepherd asks the user which to use. Each link has exactly one of `pattern` or `regex`:

```json
"links": [
  { "pattern": "https://github.com/*/pull/*", "action": "open-pr" },
  { "regex": "^https://github\\.com/[^/]+/[^/]+/pull/\\d+$", "action": "open-pr" }
]
```

- `pattern` is a URL glob: it starts with `http://` or `https://`, `*` matches any run of characters (including `/`),
  and everything else is literal. It matches the whole URL; the scheme and host ignore case, the path and query don't.
- `regex` is a regular expression in RE2 syntax (https://github.com/google/re2/wiki/Syntax), run by re2js 2.8.6, which
  matches in linear time: there are no backreferences or lookaround. It's found anywhere in the URL unless you anchor
  it with `^` and `$`, and it sees the URL exactly as shown, so write `(?i)` to ignore case in a host.
- Limits, which `shepherd plugin check` reports: a pattern or regex is at most 500 characters; a regex nests groups at
  most 16 deep, repeats at most 1000 times (and a repetition can't itself repeat), and compiles to at most 300
  instructions; a plugin has at most 8 links. Matching a URL against a link takes time proportional to the URL's
  length (at most 2048 characters) times the link's size, so no link can stall a click.
- Only http and https URLs are ever handed over.
- The action gets the URL as `call.link`, exactly as it appeared on screen (case, escapes and query kept), and never
  in `params`. It's data the user clicked, not a command: don't pass it to a shell.
- Only text on screen is matched. A terminal hyperlink (OSC 8) whose visible label differs from its destination isn't
  supported: the destination can't be read.

## What to rely on, and what not to

- A pane is `id` (like `p3`) plus `instance`, unique to its process. Key state by `instance`: ids come back after a
  restart.
- `pane.agent` and `agent.state` events are what detection observed: the agent's screen, or its integration's
  report. No `agent` means none is detected, not that there's none. States: `working`, `blocked`, `done`, `idle`.
- A disconnect or a server restart is a gap. Changes that came and went meanwhile are lost for good, so don't
  promise a complete history.
- Every request, result and event: `shepherd plugin schema` (JSON Schema).
- It isn't a sandbox. The plugin runs as the user, with their files and network.

## Testing

`checkSession()` (from the library) is the throwaway session `shepherd plugin check` started with this plugin
running: `s.shepherd(...args)` runs the CLI against it, `s.json(...args)` parses a `--json` reply, `s.plugin` is this
plugin's name, `s.data` its data directory, and `s.until(what, condition)` waits.

To make a pane look like an agent in some state, open a shell pane, run a long command in it (a report for a pane
with nothing running in its shell doesn't stick), and report the state, the way agent integrations do:

```ts
const pane = (await s.shepherd("pane", "split", "--name", "worker")).stdout;
await s.shepherd("pane", "run", pane, "sleep 600");
await s.shepherd("report", pane, "--source", "test", "--agent", "claude-code", "--state", "blocked");
await s.shepherd("wait", pane, "--state", "blocked", "--timeout", "10");
```

To test what happens at startup, stop the plugin, set things up, and start it again:
`s.shepherd("plugin", "stop", s.plugin)`, then `s.shepherd("plugin", "start", s.plugin)`, then wait until
`plugin list` shows it connected.
