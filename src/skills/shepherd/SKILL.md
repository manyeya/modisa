---
name: shepherd
description: "Drive a shepherd terminal session: split panes, run commands in them, read their output, spawn and message other coding agents, wait for one to finish. Use when the user asks about panes, tabs, workspaces, or another agent in this session, or when a task needs a long-running process watched while you keep working. Requires $SHEPHERD_PANE_ID — only agents running inside a shepherd pane can use it."
---

# shepherd

Shepherd is a terminal multiplexer that knows what a coding agent is. It organizes terminals into
workspaces, tabs and panes, watches each pane for a known agent, and tracks whether that agent is
`working`, `blocked` (waiting on a human), `done` or `idle`.

You are one of those panes. The `shepherd` binary on your `PATH` talks to the session you are in.

## Check first

```bash
test -n "${SHEPHERD_PANE_ID:-}"
```

If that fails you are not inside a shepherd pane. Say so and stop — do not try to find or drive a
session from outside it, because the session you'd reach is somebody else's.

## The binary is the authority

Don't trust a remembered flag. `shepherd help` lists every command with its flags:

```bash
shepherd help
```

## What you can do

| | |
|---|---|
| `shepherd pane list` | every pane: id, `@name`, title, agent harness and state, cwd, exit status |
| `shepherd pane split [command…]` | open a pane next to one. With a command it runs that; without, a shell |
| `shepherd pane run <target> <command…>` | type a command into a pane and press Enter. **Shells, not agents** |
| `shepherd pane read [target] [--lines n]` | a pane's visible screen plus the last `n` lines of scrollback, as plain text |
| `shepherd pane keys <target> <key…>` | send keys: text, or names like `Enter`, `Escape`, `Tab`, `C-c`, `M-x`, `Up`. **Shells, not agents** |
| `shepherd pane close [target]` | close a pane and kill what runs in it |
| `shepherd agent spawn <harness> [--prompt text]` | start another coding agent (claude, codex, pi, opencode, gemini…) in a new pane |
| `shepherd agent list` | agent panes and their current state |
| `shepherd wait <target> …` | block until a pane exits, reaches a state, or its output matches a regex |
| `shepherd send <target> <message…>` | message another agent pane |
| `shepherd inbox` | messages other agents sent you that you haven't read |
| `shepherd tab create` / `shepherd workspace create` | a new tab, or a new workspace rooted at a directory |

Targets are a pane id (`p3`), an `@name`, or a name.

## The parts you can't guess

**Targets default to you.** Every command that takes a target uses your own pane when you omit it.
`shepherd pane split` opens next to you; `shepherd pane read` reads you.

**Name the panes you make.** `--name tests` once, then `tests` everywhere after. Pane ids shift as
panes come and go; names don't.

**Wait, don't poll.** `shepherd wait` blocks server-side until the thing actually happens:

```bash
shepherd pane split --name tests "bun test"
shepherd wait tests --exited            # prints "exited <code>"
shepherd pane read tests --lines 100    # then read the result
```

Also `--state idle|working|blocked|done` (`idle` also matches `done`), `--match <regex>` against new
output, and `--timeout <seconds>`. Reading in a loop wastes tokens and misses output between reads.
The exit status says what happened: `wait --exited` exits with the pane's code, a timeout exits 124,
an unreachable server 3, bad usage 2, any other failure 1.

**Panes outlive their process.** A pane stays open after its command exits, so the output is still
there to read. Close it when you're done with it.

**To reach another agent, always use `send` — never `pane keys` or `pane run`.** Those two type
straight into a pane, and on a pane you didn't create they ask the human for permission, so a
back-and-forth conducted that way interrupts them on every single message. `send` needs no
permission and is the supported path. Keep `pane keys` and `pane run` for shells and plain commands.

**Messages land only when the other agent is free.** `shepherd send` queues into the recipient's
mailbox and is typed in when they go `idle` or `done` — never mid-turn. It arrives with a reply
hint like `shepherd send p3:1a2b3c4d "…"`: use that target, since it keeps working after a rename and
fails (instead of reaching someone else) if the sender has gone. Don't follow up with a second message because the
first hasn't been answered yet; `shepherd wait <them> --state idle` instead, then `shepherd inbox`.

A full exchange, with no permission prompts anywhere:

```bash
shepherd send @reviewer "auth diff is ready — anything blocking?"
shepherd wait @reviewer --state idle --timeout 600
shepherd inbox
```

**Spawned agents are peers, not subprocesses.** `shepherd agent spawn` starts a real agent in a real
pane with its own context. Give it a `--prompt` that stands alone — it can't see this conversation.

**Someone is watching.** A human is looking at this session. Closing a pane you didn't create
prompts them, so ask before you do it. Reading (`pane read`, `pane list`, `wait`) and messaging
(`send`, `inbox`) never prompt — prefer those.

**If a `shepherd` command says a sandbox is blocking its socket,** your own command sandbox stops you
reaching the session: `send`, `inbox` and the rest won't work from here. Tell the user, and point
them at https://manyeya.github.io/shepherd/docs/troubleshooting/#sandbox. `$SHEPHERD_SOCKET` is set
correctly; don't restart anything or ask for it to be passed in.

## Writing a plugin

If the user wants shepherd to react to something on its own — announce a blocked agent, log state
changes, kick off a job when a pane exits — that's a plugin: any program the session server starts,
given `$SHEPHERD_SOCKET`. Declare it in `~/.config/shepherd/config.toml`:

```toml
[[plugin]]
run = "bun ~/code/watcher/plugin.ts"
```

Read `examples/plugins/README.md` in the shepherd repo before writing one — it has the event
catalogue, the callable methods, and the traps (events are never replayed, `run` goes through a
login shell that can rewrite `PATH`, nothing restarts a plugin that crashes). Don't guess the event
shapes; `shepherd events --follow` prints the real ones.

## A worked example

Run the test suite and a dev server side by side, then act on whichever finishes badly:

```bash
shepherd pane split --name tests "bun test"
shepherd pane split --down --name dev "bun dev"
shepherd wait dev --match "listening on" --timeout 30
shepherd wait tests --exited
shepherd pane read tests --lines 200
```

Hand a slice of work to another agent and collect it:

```bash
shepherd agent spawn claude --name reviewer --prompt "Review the diff in src/auth for auth bypasses. Reply with shepherd send."
shepherd wait reviewer --state idle --timeout 600
shepherd inbox
```
