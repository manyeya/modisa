# Writing the worktrees modisa plugin

This directory is a modisa plugin. Modisa starts it with the session server and connects it to the session's
socket; the plugin reacts to what happens in the session. Put the user's logic in `plugin.ts` and leave the protocol
to `modisa-plugin.ts`.

## Files

- `plugin.json`: name, protocol version, and how to start it (`run`: argv, run in this directory)
- `plugin.ts`: the plugin
- `modisa-plugin.ts`: modisa's client library. Don't edit it; refresh it with `modisa plugin sdk > modisa-plugin.ts`
- `plugin.test.ts`: behavioural tests, run against a real throwaway session by `modisa plugin check .`

## The loop

```sh
modisa plugin check .          # manifest, build, then a throwaway session: it starts, connects, passes your tests,
                                 # and exits when the session dies. Fix what it reports, run it again.
modisa plugin dev .            # a throwaway session with the plugin running, to try it by hand
modisa plugin link .           # install it; it starts with the session server (modisa restart)
modisa plugin logs worktrees    # its stdout and stderr
modisa plugin run worktrees status '{"any":"params"}'
modisa plugin stop worktrees    # and plugin start worktrees
```

To share it, push it to a public git repository and give the repository the `modisa-tui-plugin` topic:
`modisa plugin search` finds it, and `modisa plugin install <url>` installs it. Put `plugin.json` at the top of the
repository, or, for a plugin in a subdirectory, say in the repository's README which `--subdir` to install.

## The library

- `runPlugin(async (modisa) => …)` connects, runs your code, and exits when the session's connection closes.
  Don't reconnect or loop: the next server starts the plugin again.
- `modisa.hello({ name: (params) => result })` binds the plugin and offers actions to `modisa plugin run`.
  Throwing in an action returns an error to the caller. An action's second argument is `{ invocation, signal }`:
  `signal` aborts if modisa stops waiting (30s). That's advisory. The caller has already been told the outcome is
  unknown, stopping early can't undo what the action already did, and nothing retries it.
- `modisa.subscribe({ onSnapshot, onEvent })`: `onSnapshot` gets every pane as the subscription starts; `onEvent`
  gets each later event once, in order, one at a time. Whatever the snapshot shows was already true when the plugin
  started: don't treat it as news.
- `modisa.request(method, params)` for anything else. Errors are `ModisaError`, with a stable `code`.
- `$MODISA_PLUGIN_DATA` is a directory for the plugin's own files.

## Showing things in modisa's TUI

A plugin can put a little into the TUI; modisa draws it in the user's theme and names the plugin on every piece, so
nothing a plugin shows can pass for modisa's own. Everything is cleared when the plugin stops.

- From the library, after `hello`:
  - `modisa.ui.status(id, text, { tone, action })`: a segment in the status row (up to 4); clicking runs `action`.
  - `modisa.ui.sidebar(title, rows)`: the plugin's section in the sidebar (up to 40 rows; the sidebar shows 8 of a
    section, or all that fit when the user's `[sidebar] agents` names the plugin, putting its section in place of
    modisa's agent list). A row runs its `action`, or focuses its `pane`; a row's `pane` comes with that pane's
    `instance`, so a click reaches that process or says it's gone. A row is `text` in one `tone`, or `spans`:
    `{ text, tone, bold }` pieces and `{ icon: "claude-code" }`, an agent's mark, which modisa draws in its brand
    colour. Give `text` and a plain `tone` with spans too: an older modisa ignores spans and shows those.
  - `modisa.ui.badge(pane, instance, text, tone)`: a label on a pane's border, for that process only.
  - `modisa.ui.menu(items)`: entries in the pane context menu (up to 8).
  - `modisa.ui.toast(text, { tone, system })`: a passing message in every attached client (3 every 10s).
  - `clearStatus`, `clearSidebar`, `clearBadge` and `closePopup` take them away; `modisa.ui.state()` is what shows now.
- Tones are `fg`, `dim`, `accent`, `warn`, and the agent states' `working`, `blocked`, `done` and `idle`, mapped to
  the user's theme. Text loses control characters and escape sequences and is cut to fit.
- An `action` must be one the plugin offered in `hello`. Offer them in `plugin.json` too, with titles, so they're listed
  in the command palette:
  `"actions": [{ "id": "clear", "title": "Clear the log" }]`.
- `"panes"` in `plugin.json` are programs the plugin can open, run in its directory with its environment:
  `{ "id": "log", "title": "Attention log", "run": ["tail", "-f", "attention.log"], "placement": "popup" }`. Placements:
  `split`, `tab` and `zoomed` open ordinary panes that stay after the plugin stops; `overlay` opens zoomed over the
  focused pane and gives focus back when it closes; `popup` floats over everything in the one client that asked (one at
  a time; `width` and `height` as cells or `"80%"`; prefix x closes it) and closes when the plugin stops.
- `"keys"` bind a key under the prefix to an action or a pane: `{ "key": "A", "pane": "log", "description": "show the
  log" }`. A key modisa uses, or one two plugins want, is off for both (`modisa plugin list` says why); the user can
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

## Views: whole screens of your own

For more than a segment or a row, a plugin opens a **view**: an element tree modisa draws in the user's theme, framed
and titled with the plugin's name, over everything (`placement: "popup"`, the default) or over one pane
(`placement: "overlay"`, `from: { pane, instance }`). It needs no program of its own: the plugin sends the tree, and
sends it again when what it shows changes. The elements are [ratatui](https://ratatui.rs)'s: constraint layouts,
blocks around anything, its widgets, and a few modisa draws itself. Every element and field is in
[VIEWS.md](https://github.com/manyeya/modisa/blob/main/examples/plugins/VIEWS.md); `modisa plugin schema` has them
as JSON Schema.

```tsx
// plugin.tsx (and "run": ["bun", "plugin.tsx"] in plugin.json): Bun's JSX works as it comes. Without JSX, call the
// functions: Layout({ direction: "horizontal", constraints: [Length(24), Fill(1)] }, List({ … }), Block({ … }, …))
import { runPlugin, Block, Fill, Gauge, Input, Layout, Length, List, Text, span } from "./modisa-plugin";

runPlugin(async (modisa) => {
  const agents = [{ name: "@coder", used: 0.23 }, { name: "@reviewer", used: 0.61 }];
  let at = 0;
  const show = () =>
    modisa.ui.view("main", (
      <Layout direction="horizontal" constraints={[Length(24), Fill(1)]} spacing={1}>
        <List id="agents" items={agents.map((a) => a.name)} selected={at} change="pick" block={{ title: "agents", border_type: "rounded" }} />
        <Block title={agents[at]!.name} border_type="rounded" padding={[0, 1]}>
          <Layout constraints={[Length(1), Length(2), Fill(1)]}>
            <Text>context {span(`${Math.round(agents[at]!.used * 100)}%`, "bold $warn")}</Text>
            <Gauge ratio={agents[at]!.used} gauge_style="$accent" />
            <Input id="note" placeholder="a note…" action="note" />
          </Layout>
        </Block>
      </Layout>
    ), { title: "Agents", keys: [{ key: "r", action: "refresh", description: "refresh" }], close: "closed" });
  await modisa.hello({
    pick: (_params, call) => ((at = call.ui?.index ?? 0), show()),
    note: (_params, call) => modisa.ui.toast(`noted: ${call.ui?.value ?? ""}`),
    refresh: () => show(),
    closed: () => {},
  });
  await show();
});
```

- Elements: `Layout` (children in a `direction`, sized by `constraints`, or each child's `size`: `Length(n)`,
  `Min(n)`, `Max(n)`, `Percentage(p)`, `Ratio(a, b)`, `Fill(weight)`), `Block` (a frame: `borders`, `border_type`,
  `title`/`titles`, `padding`, `shadow`; any element also takes one as its `block`), `Text` (spans in lines, or a
  program's coloured output as `ansi`; it scrolls with an `id`), `List`, `Table`, `Tabs`, `Tree`, `Gauge`,
  `LineGauge`, `Sparkline`, `BarChart`, `Chart`, `Canvas`, `Calendar`, `Code` and `Diff` (highlighted; a diff can have
  a line cursor and marked lines), `Markdown`, `BigText`, `Image` (PNG, JPEG or GIF), `Input`, `Textarea`, `Button`,
  `Spinner`, `Fill`, `Clear`, and `Raster`.
- Text is spans in lines: a string, `span("23%", "bold $warn")`, `icon("claude-code")` (an agent's mark), and
  `line([...spans], { align })`. In JSX a Text's children are its spans, and a `\n` starts a line. A style is a string,
  `"bold italic $accent on $bar"`, or `{ fg, bg, bold, … }`; `style("bold", late && "$warn")` puts one together.
  Colours are the theme's tokens (`$fg`, `$dim`, `$accent`, `$warn`, `$working`, `$blocked`, `$done`, `$idle`, `$bar`,
  …), so everything follows the user's theme; hex colours and names work too. Text loses control characters and
  escape sequences (an `ansi` text keeps its colours).
- Anything interactive or stateful needs an `id`: it's what keeps the user's selection, scroll and typing across
  updates (while you don't change `selected` or `value`), what `focus` names, and what an action is told. An element
  runs your actions with `call.ui`: `{ view, id, event, … }` and what it holds: a List's or Tabs' `index`, a Table's
  `row` and `column`, a Diff's `line`, `old`, `new` and `text`, an Input's or Textarea's `value`, a Tree's `path`
  (and `open`). `action` runs on Enter (an Input) or Ctrl+S (a Textarea), a click or a press; `change` when the
  selection or text moves (an Input's at most every 150 ms); a Tree's `toggle` when a node opens or closes. It's what
  the user typed or chose: data, not a command.
- The view's `keys` bind keys while it has the keyboard ("j", "J", "enter", "S-tab", "C-s"); `description` shows them
  in its footer. Tab moves between interactive elements, Escape (or prefix x) closes the view and runs its `close`
  action. `focus` (an element's id) hands the keyboard to an element. Close it yourself with `modisa.ui.closeView(id)`.
- `Raster` is a box of cells you paint: `rasterCells(columns, rows, (x, y) => [char, ink.tone("accent"), ink.rgb("#202020")])`.
  `modisa.ui.blit(view, id, cells)` repaints the one with that `id` in place, up to 60 times a second: animation.
- Limits: 4 views per plugin and 8 in a session, 5000 elements, 40 deep, 2 MB a view; an image at most 4 MB. Actions
  named anywhere in a view must be offered in `hello`. A client too old to draw these views doesn't get them.

## Links

`"links"` in `plugin.json` hands a URL the user Ctrl+clicks in a pane to one of the plugin's actions. When several
plugins' links match, modisa asks the user which to use. Each link has exactly one of `pattern` or `regex`:

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
- Limits, which `modisa plugin check` reports: a pattern or regex is at most 500 characters; a regex nests groups at
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
- Every request, result and event: `modisa plugin schema` (JSON Schema).
- It isn't a sandbox. The plugin runs as the user, with their files and network.

## Testing

`checkSession()` (from the library) is the throwaway session `modisa plugin check` started with this plugin
running: `s.modisa(...args)` runs the CLI against it, `s.json(...args)` parses a `--json` reply, `s.plugin` is this
plugin's name, `s.data` its data directory, and `s.until(what, condition)` waits.

To make a pane look like an agent in some state, open a shell pane, run a long command in it (a report for a pane
with nothing running in its shell doesn't stick), and report the state, the way agent integrations do:

```ts
const pane = (await s.modisa("pane", "split", "--name", "worker")).stdout;
await s.modisa("pane", "run", pane, "sleep 600");
await s.modisa("report", pane, "--source", "test", "--agent", "claude-code", "--state", "blocked");
await s.modisa("wait", pane, "--state", "blocked", "--timeout", "10");
```

To test what happens at startup, stop the plugin, set things up, and start it again:
`s.modisa("plugin", "stop", s.plugin)`, then `s.modisa("plugin", "start", s.plugin)`, then wait until
`plugin list` shows it connected.
