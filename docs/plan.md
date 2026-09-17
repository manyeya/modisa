# Modisa — Agent-Native Terminal Multiplexer — Build Plan

## 1. Product Vision

Modisa is a terminal multiplexer built for running many coding agents at once — tmux rebuilt for the agent era.

> A terminal pane is an execution surface. It can contain a shell, a running process, a coding-agent harness, or any other interactive tool.

What Modisa does:

- runs shells, processes, and coding agents in real terminal panes, organised into workspaces and tabs
- knows which panes are agents and shows their live state (blocked / working / done / idle) in a sidebar
- notifies you when a background agent needs you
- keeps everything running in a background server — detach, reattach, survive SSH drops
- works over SSH as a thin client
- exposes a socket API + CLI so agents (and scripts) can drive panes
- lets agents send messages to each other

**Modisa has no AI of its own.** No model, no built-in agent, no LLM calls. Intelligence comes entirely from external harnesses (Claude Code, Codex, Pi, …). Modisa hosts, observes, and connects them.

A typical workspace:

```text
Workspace: my-project
├── Tab: dev
│   ├── Terminal        zsh
│   └── Server          pnpm dev
└── Tab: agents
    ├── @coder          Claude Code   🟡 working
    ├── @reviewer       Codex         🟢 idle
    └── @docs           Pi            🔴 blocked
```

---

## 2. Product Principles

### Terminal-native

Runs inside your existing terminal (Ghostty, Kitty, WezTerm, iTerm2, Alacritty). Fast, keyboard-first, mouse-friendly.

### Agent-aware, not agent-powered

Modisa never talks to a model. It understands agents — which panes they are, what state they're in, how to reach them — but the thinking is done by harnesses.

### Harness-agnostic

No harness-specific logic in the core. Each harness is a small adapter. Zero-config detection for the common ones; anything else works through a generic adapter.

### Pane-native

Shells, processes, and agents are all the same `Pane`. An agent pane is a PTY pane plus an adapter.

### Persistent

A background server owns every PTY. Closing the terminal, laptop sleep, or an SSH drop never kills an agent.

### Local-first

Single binary, no account, no cloud, no telemetry.

---

# 3. Feature Set

This is the target for 1.0. Agent messaging is one feature among these, not the headline.

| Area | Features |
|------|----------|
| **Multiplexing** | workspaces, tabs, panes; horizontal/vertical splits; resize; zoom/fullscreen; close; rename |
| **Agent awareness** | auto-detect agents by process name + screen rules; states: 🔴 blocked, 🟡 working, 🔵 done, 🟢 idle; optional hook integrations for exact state |
| **Sidebar** | workspaces list; agents list rolled up across all workspaces with state; click/jump to agent; collapsible |
| **Notifications** | toast in-app, system notification, sound — when a background agent becomes blocked or done |
| **Sessions** | background server; named sessions; detach/reattach; state survives client exit |
| **Remote** | `modisa --remote ssh://host`: local thin client, remote server; local keybindings/theme; handles slow links |
| **Input** | prefix keybindings (configurable); full mouse — click to focus, drag borders to resize, right-click context menu, scroll |
| **Scrollback** | per-pane scrollback, copy mode, search |
| **Layout** | adapts to narrow terminals |
| **Config** | `~/.config/modisa/config.toml`; hot reload; settings menu; themes (Catppuccin, Gruvbox, Nord, Dracula) |
| **Socket API + CLI** | create/split/close panes, run commands, read output, wait for state, manage workspaces/tabs, subscribe to events |
| **Integrations** | `modisa integration install <agent>` installs the modisa skill and state-reporting hooks |
| **Agent messaging** | agents send messages to each other, delivered when the recipient is idle |
| **Skill** | a `modisa` skill (Claude Code, Codex) teaches agents the CLI: panes, waiting, spawning and messaging agents |
| **Plugins** | later: extension points for sidebars, viewers, bridges |

Supported agents (detection rules shipped): Claude Code, Codex, Pi, OpenCode, Gemini CLI, Copilot CLI, Cursor Agent, Amp, Droid, Aider, Kiro, and a generic fallback.

---

# 4. High-Level Architecture

Client-server, like tmux. The server owns all state and PTYs; the TUI is a thin client.

```text
 ┌──────────────────────┐   ┌──────────────┐   ┌──────────────┐
 │ TUI client (OpenTUI) │   │ modisa CLI  │   │   plugins    │
 │ local or over SSH    │   │ (agents,     │   │              │
 └──────────┬───────────┘   │  scripts)    │   └──────┬───────┘
            │               └──────┬───────┘          │
            └──────────────────────┼──────────────────┘
                                   │ unix socket, JSON-RPC
                        ┌──────────▼──────────┐
                        │   Modisa Server    │
                        │                     │
                        │ Workspaces / Tabs   │
                        │ Panes / Layout      │
                        │ Agent detection     │
                        │ Notifications       │
                        │ Mailbox             │
                        │ Events              │
                        │ Persistence         │
                        └──────────┬──────────┘
                                   │
                  ┌────────────────┼────────────────┐
                  │                │                │
            ┌─────▼─────┐    ┌─────▼─────┐    ┌─────▼─────┐
            │ Terminal  │    │  Process  │    │   Agent   │
            │   Pane    │    │   Pane    │    │   Pane    │
            └─────┬─────┘    └─────┬─────┘    └─────┬─────┘
                  └────────────────┼────────────────┘
                                   │
                        PTY + libghostty-vt
```

The TUI, the CLI, and plugins all speak the same socket API. There is one API.

---

# 5. Core Domain Model

## Pane

```ts
type PaneType = "terminal" | "process" | "agent";

type AgentState =
  | "working"   // 🟡 actively running
  | "blocked"   // 🔴 waiting for user input / approval
  | "done"      // 🔵 finished, not yet looked at
  | "idle";     // 🟢 finished and seen

interface Pane {
  id: string;
  name?: string;          // addressable handle: "coder" → @coder
  type: PaneType;
  title: string;
  cwd: string;
  command?: string;
  status: "running" | "exited";
  exitCode?: number;
  agent?: {
    harness: string;      // adapter id
    state: AgentState;
    source: "hook" | "screen";
  };
  createdBy?: string;     // pane id of the agent that spawned it, or "user"
}
```

`done` becomes `idle` once the user focuses the pane.

## Tab

```ts
interface Tab {
  id: string;
  name: string;
  layout: Layout;         // split tree of pane ids
  zoomedPane?: string;
}
```

## Workspace

```ts
interface Workspace {
  id: string;
  name: string;
  rootPath: string;
  tabs: string[];
}
```

## Session

```ts
interface Session {
  id: string;
  name: string;
  workspaces: string[];
  clients: number;        // attached clients
  createdAt: number;
}
```

## Layout

A split tree per tab:

```text
horizontal split
vertical split
zoom (fullscreen one pane)
```

Floating panes can come later.

```text
┌──────────┬───────────────────────┬─────────────────────┐
│ SPACES   │                       │                     │
│ my-proj  │       Terminal        │   @coder            │
│ api      │                       │                     │
│          ├───────────────────────┼─────────────────────┤
│ AGENTS   │                       │                     │
│ 🟡 coder │       Server          │   @reviewer         │
│ 🟢 review│                       │                     │
│ 🔴 docs  │                       │                     │
└──────────┴───────────────────────┴─────────────────────┘
```

---

# 6. Terminal Layer

## OpenTUI

The client UI framework: rendering, layout, keyboard/mouse input, focus, scrolling. Do not write a renderer from scratch.

## PTY

```ts
interface Pty {
  spawn(options: PtyOptions): Promise<void>;
  write(data: string): void;
  resize(cols: number, rows: number): void;
  kill(signal?: string): void;
  onData(callback: (data: Uint8Array) => void): void;
  onExit(callback: (code: number) => void): void;
}
```

## Terminal Emulator

Use `libghostty-vt`. The server keeps an emulator per pane, so:

- reattaching clients get the current screen instantly
- detection rules and `read` see the rendered screen, not raw bytes
- scrollback lives server-side

Agent TUIs (Claude Code, Codex) are the primary compatibility target, alongside vim, nvim, htop, less, ssh, git, and interactive shells.

Known limitation: running tmux inside a Modisa pane hides its contents from detection.

---

# 7. Agent Detection

The core of the product. Knowing what every agent is doing, without asking.

## Identification

A pane is an agent if its foreground process matches an adapter (`claude`, `codex`, `pi`, …). Checked on spawn and whenever the foreground process changes — so running `claude` inside a normal shell pane is detected too.

## State

1. **Hooks** — installed via `modisa integration install <agent>`. The agent's own hooks (Claude Code hooks, Codex `notify`) call `modisa report --state working|blocked|done`. A dialog on screen still reads as blocked.
2. **Screen rules** — declarative TOML per adapter, matched against the bottom of the rendered screen (`region` = last N lines): the dialog key-hint footer → blocked, "esc to interrupt" → working, otherwise idle. Zero-config; verified against the real Claude Code and Codex, whose real screens are test fixtures.

```toml
# adapters/claude-code.toml
process = ["claude"]

[[state]]
name = "blocked"
match = "Do you want to proceed\\?"

[[state]]
name = "working"
match = "esc to interrupt"
```

## Harness adapter

```ts
interface HarnessAdapter {
  id: string;                          // "claude-code"
  process: string[];                   // names to match
  rules: StateRule[];                  // screen rules
  launch?(ctx: LaunchContext): { cmd: string; args: string[]; env: Record<string, string> };
  deliver?(pty: Pty, text: string): void;   // default: bracketed paste + Enter
  interrupt?(pty: Pty): void;               // default: Esc
}
```

Mostly data. Rules live in TOML so a harness UI change is a config fix, not a release.

Every pane receives:

```text
MODISA_SOCKET      path to the server socket
MODISA_PANE_ID     its own pane id
MODISA_WORKSPACE   workspace id
```

---

# 8. Sidebar and Notifications

## Sidebar

- **Spaces:** workspaces in the session, current one highlighted
- **Agents:** every agent across all workspaces, with state icon, name, harness, and workspace
- Sorted blocked → done → working → idle, so what needs you is on top
- Click or `prefix + <n>` jumps to that agent's pane
- Toggle with `prefix + b`; auto-collapses on narrow terminals

## Notifications

Fired when an agent in a **non-focused** pane transitions to `blocked` or `done`:

- in-app toast
- system notification (macOS `osascript` / Linux `notify-send`)
- optional sound
- terminal bell / OSC 9 so the host terminal can flag the tab

Configurable per state in `config.toml`.

---

# 9. Sessions and Persistence

- `modisa` starts or attaches to the default session
- The server outlives clients; closing the terminal only detaches
- Multiple clients may attach to the same session

```bash
modisa                       # attach default session (start if needed)
modisa new my-project
modisa attach my-project
modisa detach                # or prefix + d
modisa ls
modisa kill my-project
```

Persist to disk (for server restarts / reboots): sessions, workspaces, tabs, layouts, pane names, cwd, commands, harness + launch args. On restore, shells respawn and agents relaunch with their harness's resume flag where supported (`claude --continue`, `codex resume`).

---

# 10. Remote

```bash
modisa --remote ssh://user@host
modisa --remote myserver     # ~/.ssh/config alias
```


- Server runs on the remote machine; client runs locally
- Client uses local keybindings, theme, and notifications
- Only render diffs cross the wire, so it stays responsive on slow links
- Reconnects automatically after drops; agents never notice

---

# 11. Input, Keybindings, Mouse

Prefix `Ctrl+B` by default (tmux muscle memory), configurable.

```text
prefix + v        split vertical
prefix + -        split horizontal
prefix + arrows   focus pane (also h/j/k/l)
prefix + z        zoom pane
prefix + x        close pane
prefix + c        new tab
prefix + n / p    next / previous tab
prefix + w        workspace picker
prefix + a        new agent pane (pick harness)
prefix + b        toggle sidebar
prefix + [        copy mode / scrollback
prefix + /        search scrollback
prefix + :        command palette
prefix + s        settings
prefix + R        reload config
prefix + m        pause/resume agent messaging
prefix + d        detach
```

Mouse: click to focus, drag borders to resize, scroll for scrollback, right-click for pane menu (split, close, rename, zoom).

---

# 12. Config and Themes

`~/.config/modisa/config.toml`, hot-reloaded.

```toml
prefix = "C-b"
theme = "catppuccin-mocha"

[sidebar]
visible = true
width = 24

[notify]
blocked = ["toast", "system", "sound"]
done = ["toast"]

[agents.claude-code]
launch = "claude"
```

Built-in themes: Catppuccin, Gruvbox, Nord, Dracula. Settings menu (`prefix + s`) edits the same file.

---

# 13. Socket API and CLI

The same JSON-RPC API the TUI uses, available to anything with `MODISA_SOCKET`. Every agent can already run shell commands, so the CLI works with any harness, zero integration.

```bash
modisa workspace create api --cwd ~/code/api
modisa tab create agents
modisa pane split --right --name tests "pnpm test"
modisa pane run tests "pnpm test --watch"
modisa pane read tests --lines 50          # rendered screen / scrollback
modisa pane keys server C-c
modisa pane close tests
modisa agent spawn codex --name reviewer
modisa agent list                          # agents + state
modisa wait tests --exited
modisa wait @reviewer --state idle
modisa wait server --match "listening on"
modisa events --follow                     # stream state changes etc.
```

`read` returns a structured snapshot, small by default:

```ts
interface PaneSnapshot {
  paneId: string;
  name?: string;
  type: PaneType;
  cwd: string;
  command?: string;
  status: "running" | "exited";
  exitCode?: number;
  agent?: { harness: string; state: AgentState };
  screen: string;        // visible screen, ANSI stripped
  recentOutput: string;  // last N lines of scrollback
}
```

This is what enables lead-agent workflows: a Claude Code pane can split off test runners, spawn helper agents, `wait` on them, and `read` results.

## Skill

Agents learn the CLI from a `modisa` skill (`SKILL.md`, the format Claude Code and Codex share), installed by `modisa integration install <agent>`. Every pane has `modisa` on its PATH, so the skill's commands just work. No MCP server: the CLI already is the API, and a skill costs no tool slots.

---

# 14. Agent Messaging

A feature on top of the socket API: agents can send text to each other.

```bash
modisa send @reviewer "Review the diff in src/auth"
modisa inbox
```

- Messages go into the recipient's mailbox
- Delivered (typed into the harness via its adapter) only when the recipient is `idle` or `done` — never while `working` or `blocked`
- Framed so the recipient knows who sent it and how to reply:

  ```text
  [modisa] message from @coder (reply: modisa send @coder "..."):
  Review the diff in src/auth
  ```

- `inbox` lets an agent pull instead of waiting
- The user can send messages too (`from: "user"`), via command palette

Loop safety: hop limit on reply chains, per-pair rate limit, `prefix + m` pauses all delivery, message log view.

---

# 15. Permissions

Harnesses keep their own approval systems for file edits and commands. Modisa only governs cross-pane actions, which harnesses can't see:

| Action | Default |
|--------|---------|
| `list`, `read`, `wait`, `events` | allow |
| `split`, `spawn`, `run` in own workspace | allow |
| `close` / `keys` on panes the agent created | allow |
| `send` message to an agent | allow (loop limits apply) |
| `keys` into a pane the agent didn't create | ask |
| `close` a pane the agent didn't create | ask |

"Ask" shows an in-app prompt (Allow / Always / Deny). One permission check on every socket call from day one.

---

# 16. Shared Event System

Internal event bus inside the server; streamed to clients and `modisa events`.

```ts
type Event =
  | PaneCreatedEvent
  | PaneClosedEvent
  | PaneOutputEvent
  | PaneFocusedEvent
  | ProcessExitedEvent
  | AgentDetectedEvent
  | AgentStateChangedEvent
  | MessageSentEvent
  | MessageDeliveredEvent;
```

```text
PTY / Socket API
        │
        ▼
    Event Bus
        │
        ├── Clients (render, sidebar)
        ├── Notifier
        ├── Mailbox
        ├── wait() resolvers
        └── Persistence
```

---

# 17. Build Phases

## Phase 1 — Multiplexer

- OpenTUI client, PTY, libghostty-vt
- Tabs, splits, resize, zoom, focus, close
- Keybindings + mouse
- Test Claude Code and Codex rendering from day one

## Phase 2 — Server / Client

- Background server owns PTYs and emulators
- Client attaches over unix socket
- Detach / reattach, named sessions, `modisa ls/new/attach/kill`
- Workspaces

It is now a usable tmux alternative.

## Phase 3 — Agent Awareness

- Process-name identification
- TOML screen rules for Claude Code, Codex, Pi, OpenCode, Gemini CLI + generic
- Sidebar with spaces + agents list
- Notifications (toast, system, sound, bell)

It is now usable day to day. **This is the first real release.**

## Phase 4 — Socket API + CLI

- Public JSON-RPC API
- `modisa pane / agent / workspace / wait / events`
- `MODISA_*` env in every pane
- Permission check on every call

## Phase 5 — Integrations, Config, Themes

- `modisa integration install <agent>` (skill + hooks)
- `config.toml` + hot reload + settings menu
- Themes
- Scrollback copy mode + search

## Phase 6 — Remote

- `--remote ssh://…` thin client
- Reconnect handling

## Phase 7 — Persistence Across Restarts

- Save layouts/sessions to disk
- Restore with harness resume flags

## Phase 8 — Agent Messaging + Skill

- Mailbox, `send`, `inbox`, idle-gated delivery, loop safety
- `modisa` skill for Claude Code and Codex

## Phase 9 — Templates and Plugins

```toml
# modisa.toml (per project)
[[pane]]
name = "server"
run = "pnpm dev"

[[pane]]
name = "coder"
agent = "claude-code"

[[pane]]
name = "reviewer"
agent = "codex"
```

Plugin extension points (sidebar sections, pane viewers, notification bridges) once the API is stable.

---

# 18. Suggested Repository Structure

```text
modisa/
├── apps/
│   └── modisa/          single entry: server, client, CLI
│
├── packages/
│   ├── server/
│   │   ├── session/
│   │   ├── workspace/
│   │   ├── pane/
│   │   ├── layout/
│   │   ├── detect/       process matching + screen rules
│   │   ├── notify/
│   │   ├── mailbox/
│   │   ├── events/
│   │   └── persistence/
│   │
│   ├── terminal/
│   │   ├── pty/
│   │   └── emulator/
│   │
│   ├── protocol/         JSON-RPC schema (Zod), shared by all clients
│   │
│   ├── client/           OpenTUI app: panes, sidebar, palette, mouse
│   │
│   └── adapters/         *.toml detection rules + launch/deliver overrides
│
├── tests/
├── package.json
└── README.md
```

---

# 19. Technology Direction

```text
Language:           TypeScript (Bun)
TUI:                OpenTUI
PTY:                Bun.Terminal (built-in, spawn with detached: true for a controlling tty)
Terminal emulator:  libghostty — libghostty-vt on the server (one headless screen per pane), OpenTUI's EmbeddedTerminalRenderable in the client
Protocol:           JSON-RPC 2.0 over unix socket (SSH stdio for remote)
Validation:         Zod
Persistence:        SQLite
Config:             TOML
Agent guide:        SKILL.md skill (Claude Code, Codex)
Distribution:       single compiled binary
```

No LLM SDKs. No API keys. No cloud.

---

# 20. Important Technical Risks

## Terminal compatibility

Agent TUIs are the hardest, most important clients. Test them from Phase 1.

## Detection accuracy

Screen rules break when harnesses change their UI. Keep rules in TOML, test them against real screens, and ship a `modisa debug detect <pane>` command that shows which rule matched.

## Server/client split

Rendering over a socket (and SSH) must stay fast. Send screen diffs, not full frames.

## Message injection

Typing into a harness input is fragile. Bracketed paste, idle-only delivery, adapter overrides.

## Agent loops

Messaging needs hop limits, rate limits, and a pause switch before it ships.

---

# 21. The Differentiator

Not "tmux with AI" — Modisa has no AI.

> **A terminal multiplexer that knows what every agent is doing, tells you when one needs you, never loses a session, and gives agents the same control over the workspace that you have.**

Agents from any vendor share one workspace. Modisa watches the flock; it doesn't do the grazing.

---

# 22. Milestones

## Milestone 1 — It's a multiplexer

`modisa` opens a shell. Split, tab, zoom, resize with mouse. Claude Code renders perfectly in a pane.

## Milestone 2 — It never loses anything

Close the terminal. Run `modisa`. Everything is exactly where it was, agents still running.

## Milestone 3 — It knows your agents

Open Claude Code, Codex, and Pi in three panes. Switch to another tab. When Codex asks for approval, the sidebar shows 🔴 and a system notification fires. Click it, approve, back to work.

## Milestone 4 — Agents drive it

Ask Claude Code *"run the tests and tell me what's wrong."* It runs `modisa pane split --name tests "pnpm test"`, `modisa wait tests --exited`, `modisa pane read tests`, and reports back. The test run is visible the whole time.

## Milestone 5 — Agents talk

Tell Claude Code *"when you're done, ask @reviewer to review."* The message lands in Codex when it's idle; the reply lands back in Claude Code.

---

# 23. Definition of Success

```text
modisa --remote devbox
     │
     ▼
Session restored: my-project
     │
     ├── Tab: dev      Terminal, Server
     └── Tab: agents   @coder 🟡  @reviewer 🟢  @docs 🔴
             │
             ▼
   Notification: "@docs is blocked"
             │
             ▼
   Click → approve → back to your tab
             │
             ▼
   @coder spawns a test pane, waits, reads, fixes
             │
             ▼
   @coder → @reviewer: "review src/auth"
             │
             ▼
   Detach. Close laptop. Reattach tomorrow. Still there.
```

Many agents, one terminal, nothing lost, nothing missed — and Modisa never called a model once.

That is the product.
