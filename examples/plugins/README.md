# Writing a shepherd plugin

## Quick start

```sh
shepherd plugin new my-plugin      # TypeScript, shepherd's client library, AGENTS.md (the guide), a test
cd my-plugin                       # put the logic in plugin.ts
shepherd plugin check .            # manifest, build, then a throwaway session: starts, connects, your tests, exits
shepherd plugin link . && shepherd restart
```

`attention-log/` is a complete one made that way: when an agent newly becomes blocked, it appends a line to a log.
The rest of this page is the protocol underneath, for plugins that don't use the client library.

A plugin is **any program** shepherd starts alongside the session server. There is no SDK, no
manifest and no API to implement: shepherd hands you a unix socket and gets out of the way. If it can
speak newline-delimited JSON, it can be a plugin — Bun, Python, Go, a shell script wrapping the
`shepherd` CLI.

Everything below is verified against a running session; `blocked-notifier/plugin.ts` in this
directory is a complete working example.

## Link it

A plugin is a directory with a `plugin.json`:

```json
{ "name": "attention-log", "protocol": 1, "run": ["bun", "plugin.ts"] }
```

`run` is an argv list, started in the plugin's directory, with no shell in between. `protocol` is the
protocol version the plugin speaks; one the server doesn't speak fails to start, and says so.

```sh
shepherd plugin link ./attention-log    # checks plugin.json, links it into ~/.config/shepherd/plugins
shepherd restart                         # plugins start with the session server
shepherd plugin list                     # status, exit code, connected or not, actions, log file
shepherd plugin logs attention-log
shepherd plugin run attention-log <action> '{"any":"params"}'
shepherd plugin unlink attention-log     # removes the link, never the directory
```

**Shepherd owns the process.** Each plugin runs in its own process group. When the session stops,
the whole group gets TERM, and whatever is still running 2 seconds later gets KILL, so children and
grandchildren go too. stdout and stderr go to `~/.local/state/shepherd/plugins/<session>.<name>.log`.
Nothing restarts a plugin that exits: `plugin list` shows it `exited` or `failed`, with the reason.
If the server itself is killed outright, it can't stop anyone, which is why a plugin must still exit
when its socket closes (rule 1 below).

**Actions.** To be callable with `shepherd plugin run`, call `plugin.hello` on your connection with
the token shepherd started you with and the actions you offer:

```json
{"jsonrpc":"2.0","id":1,"method":"plugin.hello","params":{"token":"<$SHEPHERD_PLUGIN_TOKEN>","actions":["summary"]}}
```

Shepherd then sends `plugin.action` requests (`{"action":"summary","params":{...}}`) on that
connection. Reply with a result or an error on the request's `id`; the caller gets it, or
`plugin_error`, `plugin_unavailable`, `no_such_action` or `timeout` (30s) as its error code.
Each request carries an `invocation` id.

**A timeout means the outcome is unknown, not failed.** The plugin may still finish the action, and
running it again can repeat its effects, so nothing retries it. Shepherd sends the plugin a
`plugin.cancel` notification (`{invocation, action}`), which is advisory: nothing proves the action
stopped. A reply that arrives after the timeout is dropped and noted in the plugin's log.

**Only a bound connection is attributed to your plugin.** A request on it that passes `caller`
(claiming to be a pane) is rejected. Any other connection, including a second one your plugin opens,
is trusted as the local user, `caller` claims and all. The token tells plugins apart; **it isn't a
security boundary.** Anything running as you can reach the socket, and a plugin is not sandboxed.

**A run's token stops working when the run ends.** Stopping the plugin (`plugin stop`, `unlink`,
the session stopping) or its process exiting revokes it and closes its connection; a new start gets a
new token. Each plugin's log keeps the first 5 MB of output per run.

`shepherd plugin unlink` removes the link, so no session starts the plugin again, and stops the running
copy in the one session it reaches (the default, or `-s`). Other running sessions keep theirs until
they restart.

## Declare it in config.toml instead

`~/.config/shepherd/config.toml`:

```toml
[[plugin]]
run = "bun ~/code/shepherd/examples/plugins/blocked-notifier/plugin.ts"
```

`run` is a shell command line, so `~`, pipes and redirection all work. Add as many `[[plugin]]`
blocks as you like. They start when the server starts, so after editing, run `shepherd restart`.

It runs through a **login** shell (`$SHELL -lc`), which means your `~/.zprofile` / `~/.bash_profile`
is sourced first and can reorder or replace `PATH`. Don't assume a `PATH` you set elsewhere survives
— give absolute paths to your interpreter and script, or set `PATH` inside `run` itself.

## What you get

The server starts your process with its own environment plus:

| Variable | |
|---|---|
| `SHEPHERD_SOCKET` | absolute path to the session's unix socket — this is your API |
| `SHEPHERD_SESSION` | the session name |

stdout and stderr are inherited by the server, so they land in `~/.local/state/shepherd/<session>.log`.
Redirect in `run` if you want your own file.

**You are not a pane.** `SHEPHERD_PANE_ID` is deliberately unset for plugins. Two consequences:

- Commands that default to "the calling pane" have no default for you — always pass an explicit
  `target`.
- You act with the **user's** authority, not an agent's. The permission prompts that gate an agent
  typing into or closing a pane it didn't create do not apply to you, and privileged calls like
  `integration` (which refuses any caller with a pane id) will work. Be careful.

## Talking to the socket

Newline-delimited [JSON-RPC 2.0](https://www.jsonrpc.org/specification). One JSON object per line,
both directions.

Request → response, matched by `id`:

```json
{"jsonrpc":"2.0","id":1,"method":"list","params":{}}
{"jsonrpc":"2.0","id":1,"result":[ ... ]}
```

Errors come back on the same `id`:

```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"no such pane: p9"}}
```

`-32601` unknown method, `-32602` invalid params (the message names the offending field), `-32000`
the handler threw. Params are validated against a Zod schema before your call runs, so a typo in a
field name is an error, not a silent default.

Events are **notifications** — no `id`, always `method: "event"`:

```json
{"jsonrpc":"2.0","method":"event","params":{"type":"pane.created","at":1789292993937,"seq":42,"epoch":"3f9c1a2b","pane":"p2","instance":"8d1e0c7a","name":"demo","command":"echo hi"}}
```

Call `events.subscribe` once to start receiving them. Pass `snapshot: true` to get every pane as of
that moment in the same reply:

```json
{"jsonrpc":"2.0","id":2,"method":"events.subscribe","params":{"snapshot":true}}
{"jsonrpc":"2.0","id":2,"result":{"protocol":1,"epoch":"3f9c1a2b","seq":41,"panes":[ ... ]}}
```

**The snapshot and the stream don't overlap and don't leave a gap.** The server takes the snapshot in
the same step that turns your events on, so every later change arrives as an event with a `seq`
greater than the snapshot's, and nothing is in both. Starting up, precisely:

1. Send `events.subscribe` with `snapshot: true`, and buffer any events that arrive before its reply.
2. Build your state from the reply's `panes`. Remember its `epoch` and `seq`.
3. Apply the buffered events, then each new one, if its `epoch` matches and its `seq` is greater than
   the snapshot's `seq`. Ignore anything else.

That's how you tell "already blocked when I started" from "just became blocked".

**Events are not replayed.** Within one connection they arrive in `seq` order and none are dropped.
A connection that stops reading doesn't make the server buffer without limit: once more than 16 MB is
waiting to be written to it, the server closes it (other clients aren't affected). The same 16 MB is
also the most one message can be: a single reply bigger than that (a huge `pane.read`) closes the
connection too, even when nothing else is waiting. After a disconnect,
or when `epoch` changes (the server restarted: pane ids and seqs from the old epoch mean nothing
now), subscribe again with `snapshot: true`. **A disconnect is a gap in history.** The new snapshot
has the current state, but a transition that started and ended during the gap (blocked, then
unblocked) is gone for good, so a plugin can't promise a complete record of everything that happened. `seq` rises with every event the
server emits, including ones you didn't ask for, so the seqs you see can skip: a skip is not a lost
event.

`protocol.describe` returns the protocol version and JSON Schemas generated from the schemas the
server uses: every request (`requests`), the results of the supported ones (`results`: `list`,
`events.subscribe`, `pane.read`, `agent.list`, `wait`, `send`, `plugin.list`, `plugin.hello`), every
event (`events`), the envelope, and the error reply (`error`). The e2e suite checks real replies and
events against them.

### The cheap way

If you don't want to write a socket client, shell out to the CLI — it reads `$SHEPHERD_SOCKET` and
does the same thing:

```sh
#!/bin/sh
# absolute path: the login shell may have rewritten PATH
/usr/local/bin/shepherd events --follow | while read -r line; do
  echo "$line" | grep -q '"type":"agent.state".*"to":"blocked"' && say "an agent needs you"
done
```

That is a perfectly good plugin. Write a socket client when you need request/response mid-stream, as
the example does when it reads the blocked pane's screen.

## Events

Every event has `type`, `at` (epoch ms), `seq` and `epoch`. Pane events name the pane by `pane` (its
id) and `instance` (unique to the process it was started with; an id can come back after a restart,
an instance never does). These shapes are checked against every event the server emits in the e2e
suite (`protocol.describe` has them as JSON Schema):

| `type` | Fires when | Extra fields |
|---|---|---|
| `pane.created` | a pane opens | `pane`, `instance`, `name`, `command` |
| `process.exited` | a pane's process ends (the pane stays) | `pane`, `instance`, `name`, `exitCode` (killed by signal n: 128+n) |
| `agent.state` | a detected agent changes state | `pane`, `instance`, `name`, `harness`, `from`, `to` |
| `message.sent` | one agent messages another | `id`, `from`, `to`, `hops` |
| `message.delivered` | that message is typed into the recipient | `id`, `from`, `to` |
| `client.attached` | someone attaches the TUI | — |
| `pane.output` | bytes are written to any pane | `pane`, `text` |

States for `agent.state` are `working`, `blocked`, `done`, `idle`.

`pane.output` is opt-in: `events.subscribe` with `{"output": true}`. It is every byte of every pane,
escape codes included — take it only if you genuinely parse terminal output, and never log it
verbatim.

## Methods

The full list lives in `src/protocol/schema.ts`; these are the ones plugins want. Omit `caller`,
it's for panes.

| Method | Params | Returns |
|---|---|---|
| `list` | — | every pane: id, name, title, cwd, agent harness + state, exit status |
| `agent.list` | — | just the agent panes |
| `session.info` | — | the session |
| `pane.read` | `target`, `lines` | the pane's info plus `screen` (visible) and `recentOutput` (scrollback tail) |
| `pane.split` | `target`, `dir`, `name`, `cwd`, `command`, `focus` | the new pane |
| `pane.run` | `target`, `command` | types a command and presses Enter |
| `pane.keys` | `target`, `keys[]` | text, or names: `Enter` `Escape` `Tab` `C-c` `M-x` `Up` |
| `pane.close` / `pane.focus` / `pane.rename` | `target`(, `name`) | |
| `agent.spawn` | `harness`, `name`, `prompt`, `dir`, `tab` | starts an agent in a new pane |
| `wait` | `target`, `exited` \| `state` \| `match`, `timeout` | blocks server-side |
| `send` | `to`, `body` | queues a message; the target **must be an agent pane** |
| `inbox` / `messages` | — | |
| `tab.create`, `workspace.create` / `.list` / `.rename` / `.close` | | |
| `events.subscribe` | `output` | start the event stream |
| `report` | `pane`, `source`, `agent`, `state`, `seq`, `session`, `release` | drive a pane's state yourself |

`target` is a pane id (`p3`), an `@name`, or a bare name.

### Reporting state for your own tool

If you run something shepherd can't recognise from its screen, report for it — this is the same
mechanism the built-in integrations use:

```sh
shepherd report --source my-tool --agent my-agent --state working
shepherd report --source my-tool --state blocked --seq 42   # stale seqs ignored
shepherd report --source my-tool --release                  # hand back to screen detection
```

While a source reports, it is authoritative for that pane. Reports without `--source` don't change
state.

## Rules that will bite you

1. **Exit when the socket closes — don't reconnect in a loop.** The server starts plugins itself, so
   `shepherd restart` spawns a fresh copy of yours. A plugin that reconnects forever keeps the old
   process running alongside the new one, and every restart adds another. Shepherd ends your whole
   process group when the session stops, but a server that's killed outright can't, and a retry loop
   that outlives it is a process leak that quietly chews the machine. Exit; let the next server start
   you.
2. **Never die on bad input.** Wrap `JSON.parse` per line. One malformed line should not end the
   process.
3. **Don't block the read loop.** Handle an event asynchronously, or you will stall behind your own
   pending request — especially with `pane.output` on.
4. **`wait` blocks server-side**, so it is cheap. Use it instead of polling `list` in a loop.
5. **Nothing supervises you.** If your process crashes, it stays dead until `shepherd restart`. Keep
   the top-level loop boring.
6. **Plugins run where the server runs.** With `--remote`, that is the remote machine — not your
   laptop. A plugin that pops a desktop notification will pop it there, where nobody is looking.
   See [Remote sessions](../../README.md#remote-sessions-over-ssh).
7. **Subscribe before you act.** See the no-replay note above; it is the most common reason a plugin
   "does nothing" on the first run.

## Try the example

```sh
bun examples/plugins/blocked-notifier/plugin.ts   # refuses to run outside a session
```

Then add the `[[plugin]]` block above, `shepherd restart`, and put an agent into a prompt that needs
you. The line appears in the session log:

```text
[blocked-notifier] @asker (fakeagent) needs you
  Pick a colour
   1. red
  Enter to confirm · Esc to cancel
```
