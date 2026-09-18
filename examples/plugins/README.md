# Writing a modisa plugin

## Quick start

```sh
modisa plugin new my-plugin      # TypeScript, modisa's client library, AGENTS.md (the guide), a test
cd my-plugin                       # put the logic in plugin.ts
modisa plugin check .            # manifest, build, then a throwaway session: starts, connects, your tests, exits
modisa plugin link .             # every session starts it; the running one starts it now
modisa plugin install <git-url>  # someone else's: --ref and --subdir pick a version and a directory
```

`attention-log/` is a complete one made that way: when an agent newly becomes blocked, it appends a line to a log.
The rest of this page is the protocol underneath, for plugins that don't use the client library.

The client library (`modisa plugin sdk`) is TypeScript, but nothing requires it: a plugin is a program modisa
starts with the session and connects to its unix socket, so anything that speaks newline-delimited JSON can be one —
Bun, Python, Go, or a shell script wrapping the `modisa` CLI. What a plugin can show in the TUI, its links and its
limits are explained in [`attention-log/AGENTS.md`](attention-log/AGENTS.md) (the guide `plugin new` writes); this
page is the wire protocol under all of it.

Everything below is verified against a running session; `blocked-notifier/plugin.ts` in this directory is a working
example with no manifest and no library.

## Link it

A plugin is a directory with a `plugin.json`:

```json
{ "name": "attention-log", "protocol": 1, "run": ["bun", "plugin.ts"] }
```

`run` is an argv list, started in the plugin's directory, with no shell in between. `protocol` is the
protocol version the plugin speaks; one the server doesn't speak fails to start, and says so.

```sh
modisa plugin link ./attention-log    # checks plugin.json, links it for every session, starts it in the running one
modisa plugin install <git-url>       # or fetch one: https, ssh, git or file URL, --ref and --subdir
modisa plugin list                     # status, exit code, connected or not, actions, source, keys that are off
modisa plugin logs attention-log
modisa plugin run attention-log <action> '{"any":"params"}'
modisa plugin stop attention-log       # and plugin start attention-log
modisa plugin unlink attention-log     # removes the link; a directory you linked is never deleted
```

**Modisa owns the process.** Each plugin runs in its own process group. When the session stops,
the whole group gets TERM, and whatever is still running 2 seconds later gets KILL, so children and
grandchildren go too. stdout and stderr go to `~/.local/state/modisa/plugins/<session>.<name>.log`.
Nothing restarts a plugin that exits: `plugin list` shows it `exited` or `failed`, with the reason.
If the server itself is killed outright, it can't stop anyone, which is why a plugin must still exit
when its socket closes (rule 1 below).

**Actions.** To be callable with `modisa plugin run`, call `plugin.hello` on your connection with
the token modisa started you with and the actions you offer:

```json
{"jsonrpc":"2.0","id":1,"method":"plugin.hello","params":{"token":"<$MODISA_PLUGIN_TOKEN>","actions":["summary"]}}
```

Modisa then sends `plugin.action` requests on that connection:
`{"action":"summary","params":{...},"invocation":"attention-log-7"}`, plus `"target":{"pane","instance"}` when the
user took the action on a pane (a menu entry, key or palette entry; already checked to be that pane's current process)
and `"link":"https://…"` when it came from a Ctrl+clicked URL. `target` and `link` are never inside `params`. Reply
with a result or an error on the request's `id`; the caller gets it, or `plugin_error`, `plugin_unavailable`,
`no_such_action` or `timeout` (30s) as its error code.

**A timeout means the outcome is unknown, not failed.** The plugin may still finish the action, and
running it again can repeat its effects, so nothing retries it. Modisa sends the plugin a
`plugin.cancel` notification (`{invocation, action}`), which is advisory: nothing proves the action
stopped. A reply that arrives after the timeout is dropped and noted in the plugin's log.

**Only a bound connection is attributed to your plugin.** A request on it that passes `caller`
(claiming to be a pane) is rejected. Any other connection, including a second one your plugin opens,
is trusted as the local user, `caller` claims and all. The token tells plugins apart; **it isn't a
security boundary.** Anything running as you can reach the socket, and a plugin is not sandboxed.

**A run's token stops working when the run ends.** Stopping the plugin (`plugin stop`, `unlink`,
the session stopping) or its process exiting revokes it and closes its connection; a new start gets a
new token. Each plugin's log keeps the first 5 MB of output per run.

`modisa plugin unlink` removes the link, so no session starts the plugin again. For a directory you linked, it stops
the running copy in the one session it reaches (the default, or `-s`); other running sessions keep theirs until they
restart. For a plugin `plugin install` fetched, it stops it in every running session it can reach, then deletes the
checkout (never the plugin's data or logs), or keeps the checkout and says why when a session still runs it or can't
be reached.

## Declare it in config.toml instead

`~/.config/modisa/config.toml`:

```toml
[[plugin]]
run = "bun ~/code/modisa/examples/plugins/blocked-notifier/plugin.ts"
```

`run` is a shell command line, so `~`, pipes and redirection all work. Add as many `[[plugin]]`
blocks as you like. They start when the server starts, so after editing, run `modisa restart`.

It runs through a **login** shell (`$SHELL -lc`), which means your `~/.zprofile` / `~/.bash_profile`
is sourced first and can reorder or replace `PATH`. Don't assume a `PATH` you set elsewhere survives
— give absolute paths to your interpreter and script, or set `PATH` inside `run` itself.

## What you get

The server starts your process with its own environment plus:

| Variable | |
|---|---|
| `MODISA_SOCKET` | absolute path to the session's unix socket — this is your API |
| `MODISA_SESSION` | the session name |
| `MODISA_PLUGIN` | a linked plugin's name |
| `MODISA_PLUGIN_TOKEN` | a linked plugin's token for `plugin.hello`, good for this run only |
| `MODISA_PLUGIN_DATA` | a linked plugin's own directory for files, `~/.local/state/modisa/plugins/<name>` |
| `MODISA_PLUGIN_CONFIG` | a linked plugin's own config directory, `~/.config/modisa/plugin-config/<name>` |

A pane a plugin opens (`plugin.pane.open`) gets `MODISA_PLUGIN`, `MODISA_PLUGIN_DATA` and `MODISA_PLUGIN_CONFIG`
too, and `MODISA_PLUGIN_CONTEXT`: JSON saying which pane it was opened from and with which params.

A linked plugin's stdout and stderr go to `~/.local/state/modisa/plugins/<session>.<name>.log` (`plugin logs`). A
`[[plugin]]` program's are inherited by the server, so they land in `~/.local/state/modisa/<session>.log`; redirect
in `run` if you want your own file.

**You are not a pane.** `MODISA_PANE_ID` is deliberately unset for plugins. Two consequences:

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
that moment in the same reply (`list` gives the same panes any time). A pane's `title` is its name when
it has one; `terminalTitle` is what the program in it last set, such as the task an agent is on:

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
connection too, even when nothing else is waiting. That cap is on what the server sends. What it
reads has no per-message cap: a local client can send one enormous line and make the server buffer
it, which is in line with trusting local clients as the user (see below). After a disconnect,
or when `epoch` changes (the server restarted: pane ids and seqs from the old epoch mean nothing
now), subscribe again with `snapshot: true`. **A disconnect is a gap in history.** The new snapshot
has the current state, but a transition that started and ended during the gap (blocked, then
unblocked) is gone for good, so a plugin can't promise a complete record of everything that happened. `seq` rises with every event the
server emits, including ones you didn't ask for, so the seqs you see can skip: a skip is not a lost
event.

`protocol.describe` returns the protocol version and JSON Schemas generated from the schemas the
server uses: every request (`requests`), the results of the supported ones (`results`: `list`,
`events.subscribe`, `pane.read`, `agent.list`, `wait`, `send`, `plugin.list`, `plugin.hello`, `ui.state`,
`plugin.pane.open`), every
event (`events`), the envelope, and the error reply (`error`). The e2e suite checks real replies and
events against them.

### The cheap way

If you don't want to write a socket client, shell out to the CLI — it reads `$MODISA_SOCKET` and
does the same thing:

```sh
#!/bin/sh
# absolute path: the login shell may have rewritten PATH
/usr/local/bin/modisa events --follow | while read -r line; do
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
| `plugin.hello` | `token`, `actions[]` | binds this connection to your plugin's run |
| `plugin.list` / `plugin.start` / `plugin.stop` | (`name`) | plugins' status; start or stop one |
| `plugin.invoke` | `plugin`, `action`, `params` | calls another plugin's action, as `modisa plugin run` does |
| `protocol.describe` | — | the protocol version and JSON Schemas for everything here |

`target` is a pane id (`p3`), an `@name`, or a bare name.

## The TUI

These work only on a plugin's bound connection (after `plugin.hello`); from any other connection they fail with
`plugin_unavailable`. What each shows, how it's drawn, and the limits are in
[`attention-log/AGENTS.md`](attention-log/AGENTS.md).

| Method | Params | |
|---|---|---|
| `ui.status.set` / `ui.status.clear` | `id`, `text`, `tone`, `action` / `id` | a status row segment |
| `ui.sidebar.set` / `ui.sidebar.clear` | `title`, `rows[]` (`text`, `tone`, `spans`, `action`, `pane` + `instance`) | the plugin's sidebar section; `spans` are `{ text, tone, bold }` and `{ icon: <agent id> }` pieces |
| `ui.badge.set` / `ui.badge.clear` | `pane`, `instance`, `text`, `tone` / `pane` | a label on a pane's border |
| `ui.menu.set` | `items[]` (`id`, `title`, `action`) | pane context menu entries |
| `ui.toast` | `text`, `tone`, `system` | a passing message in every attached client |
| `ui.state` | `plugin` | what it shows now |
| `plugin.pane.open` | `plugin`, `pane`, `params`, `from` | opens one of `plugin.json`'s `panes` |
| `ui.popup.close` | — | closes the plugin's popup |

An `action` must be one the run offered in `plugin.hello`, or the call fails with `no_such_action`. A pane named with
its `instance` that has closed or restarted fails with `pane_gone`. Too many updates fail with `rate_limited`; a popup
while another is open fails with `ui_busy`. Everything a run showed is cleared when it ends. `plugin.json`'s `keys` and
`links` need no calls: modisa sends the plugin a `plugin.action` when one is used.

### Reporting state for your own tool

If you run something modisa can't recognise from its screen, report for it — this is the same
mechanism the built-in integrations use:

```sh
modisa report --source my-tool --agent my-agent --state working
modisa report --source my-tool --state blocked --seq 42   # stale seqs ignored
modisa report --source my-tool --release                  # hand back to screen detection
```

While a source reports, it is authoritative for that pane. Reports without `--source` don't change
state.

## Rules that will bite you

1. **Exit when the socket closes — don't reconnect in a loop.** The server starts plugins itself, so
   `modisa restart` spawns a fresh copy of yours. A plugin that reconnects forever keeps the old
   process running alongside the new one, and every restart adds another. Modisa ends your whole
   process group when the session stops, but a server that's killed outright can't, and a retry loop
   that outlives it is a process leak that quietly chews the machine. Exit; let the next server start
   you.
2. **Never die on bad input.** Wrap `JSON.parse` per line. One malformed line should not end the
   process.
3. **Don't block the read loop.** Handle an event asynchronously, or you will stall behind your own
   pending request — especially with `pane.output` on.
4. **`wait` blocks server-side**, so it is cheap. Use it instead of polling `list` in a loop.
5. **Nothing supervises you.** If your process crashes, it stays dead until `modisa plugin start <name>` or the next
   server start. Keep the top-level loop boring.
6. **Plugins run where the server runs.** With `--remote`, that is the remote machine — not your
   laptop — with its files. A desktop notification your process raises itself pops up there, where nobody is
   looking; what you show through `ui.*` (a `ui.toast` with `system: true` included) is drawn by the attached client,
   on the user's machine. See [Remote sessions](../../README.md#remote-sessions-over-ssh).
7. **Subscribe before you act.** See the no-replay note above; it is the most common reason a plugin
   "does nothing" on the first run.

## Try the examples

```sh
modisa plugin check examples/plugins/attention-log   # its manifest, build and behavioural tests
modisa plugin link examples/plugins/attention-log    # prefix A shows its log; blocked agents show in the sidebar
```

`blocked-notifier` is the no-manifest kind:

```sh
bun examples/plugins/blocked-notifier/plugin.ts   # refuses to run outside a session
```

Add the `[[plugin]]` block above, `modisa restart`, and put an agent into a prompt that needs
you. The line appears in the session log:

```text
[blocked-notifier] @asker (fakeagent) needs you
  Pick a colour
   1. red
  Enter to confirm · Esc to cancel
```
