# Writing a shepherd plugin

A plugin is **any program** shepherd starts alongside the session server. There is no SDK, no
manifest and no API to implement: shepherd hands you a unix socket and gets out of the way. If it can
speak newline-delimited JSON, it can be a plugin — Bun, Python, Go, a shell script wrapping the
`shepherd` CLI.

Everything below is verified against a running session; `blocked-notifier/plugin.ts` in this
directory is a complete working example.

## Declare it

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
{"jsonrpc":"2.0","method":"event","params":{"type":"pane.created","at":1789292993937,"pane":"p2","name":"demo","command":"echo hi"}}
```

Call `events.subscribe` once to start receiving them.

**Events are not replayed.** You get what happens *after* your subscription lands, and nothing
before it. Plugins start while the server is still booting, so a slow interpreter can easily miss the
first transitions of a restored session. If you need the state of the world at startup, call `list`
once after subscribing — subscribe first, then read, or you have a gap between the two.

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

Every event has `type` and `at` (epoch ms). Verified shapes:

| `type` | Fires when | Extra fields |
|---|---|---|
| `pane.created` | a pane opens | `pane`, `name`, `command` |
| `process.exited` | a pane's process ends (the pane stays) | `pane`, `name`, `exitCode` |
| `agent.state` | a detected agent changes state | `pane`, `name`, `harness`, `from`, `to` |
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
   process running alongside the new one, and every restart adds another. Worse, `plugins.stop()`
   kills the shell in `run`, not its grandchildren, so a wrapper script's child can outlive the
   session — a retry loop inside one becomes a process leak that will quietly chew the machine and
   make agent detection stall. Exit; let the next server start you.
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
