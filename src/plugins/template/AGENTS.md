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

## Links

`"links": [{ "pattern": "https://github.com/*/pull/*", "action": "open-pr" }]` in `plugin.json` hands a URL the user
Ctrl+clicks in a pane to that action. When several plugins match, shepherd asks the user which to use.

- A pattern is a URL glob, not a regular expression: it starts with `http://` or `https://`, `*` matches any run of
  characters (including `/`), and everything else is literal. It matches the whole URL; the scheme and host ignore
  case, the path and query don't. There are no character classes or alternation: list one link per shape instead.
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
