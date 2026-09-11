# shepherd

An agent-aware terminal multiplexer. No AI of its own — it hosts coding agents (Claude Code, Codex, Pi, …) and shells in real terminal panes, and tells you which one needs you. Docs: **https://manyeya.github.io/shepherd**.

Built with Bun and OpenTUI: real terminal panes, named spaces, an agent sidebar, a segmented status bar, mouse menus, and live themes.

## Install

```bash
curl -fsSL https://manyeya.github.io/shepherd/install.sh | sh
```

Releases are built for macOS on Apple silicon and Linux on x64 and arm64. The installer checks the download against its published SHA-256 and puts `shepherd` in `~/.local/bin` (`SHEPHERD_INSTALL_DIR` changes that; `SHEPHERD_CHANNEL=staging` installs the latest prerelease). Every release on [GitHub Releases](https://github.com/manyeya/shepherd/releases) has the binaries, `SHA256SUMS` and the `manifest.json` the installer and updater read.

When a newer release is out, the status row shows `↑ <version>`: click it to update and restart the server (agents resume). From the command line: `shepherd version`, `shepherd update`, then `shepherd restart`. `[update] channel = "staging"` follows prereleases; `[update] check = false` turns the check off.

From source (also the way to run it on an Intel Mac), with Bun ≥ 1.3.5:

```bash
bun install
bun start
```

## Keys

Prefix is `Ctrl+B`, then:

| Key | Action |
|-----|--------|
| `v` / `%` | split right |
| `-` / `"` | split down |
| `h j k l` / arrows | focus pane |
| `H J K L` | resize pane |
| `z` | zoom pane |
| `x` | close pane |
| `c` | new tab |
| `n` / `p` | next / previous tab |
| `w` / `W` | switch space / new named space |
| `$` / `&` | rename / delete the current space (delete asks first) |
| `b` | hide / show sidebar |
| `s` / `t` | settings / settings on the theme section |
| `o` / `1-9` | pane picker / jump to agent |
| `e` | open pane context menu |
| `a` | launch an agent |
| `:` / `?` | command palette / keyboard guide |
| `Ctrl+B` | send a literal `Ctrl+B` |
| `d` | detach (panes keep running) |

Mouse: click a pane to focus it, drag the border between two panes to resize them (the pointer turns into a move cursor over a border), click a tab to switch, and scroll for scrollback. Right-click a pane or tab for split, zoom, rename, copy visible output, search, theme, sidebar, and close actions. Menus support arrows, Enter, Escape, and outside-click dismissal. Clipboard copying uses OSC 52 where supported by your terminal.

Spaces are named groups of tabs and panes. In the sidebar, click a space to switch to it, double-click its name (or click ✎) to rename it in place — Enter saves, Esc cancels — and click ✕ to delete it, which asks first and closes its panes. Right-click a space for the same actions. The last space can't be deleted. A new space starts in the current space's working directory; creating one does not create a directory. Use the shell's `cd` command to change a pane's working directory.

The settings page (`Ctrl+B s`, or ⚙ settings in the sidebar) has a tab per section; Tab switches section, ↑↓ or the pointer selects, ←→ changes a value, Enter or a click applies, Esc closes. Every change applies at once and is saved to `~/.config/shepherd/config.toml`, keeping your comments and other settings.

- **theme** — Ion (default), Tokyo Night, Catppuccin Mocha, Gruvbox, Nord, Dracula, previewed live as you move
- **indicators** — the agent-state glyphs (symbols `! ◆ ✓ ○`, dots, or letters) and where they show: tab badge, pane border, sidebar
- **sound** — the sound for when an agent needs you, is done, or starts working, from [cuelume](https://cuelume.dev)'s 17 (or off), and the volume; each plays as you pick it
- **toasts** — toast, system notification and terminal bell per event
- **pane labels** — agent and state in a pane's border title, and the id/status line on its bottom border
- **integrations** — every agent's integration (installed, update available, available, not found); Enter installs, updates or removes one, and "Install all" does the recommended ones

The sidebar hides automatically below 100 columns or 22 rows. If a split would become too small, the focused pane fills the available space; expanding the terminal restores the split layout. Use the pane picker (`Ctrl+B`, `o`) or directional focus keys to reach other panes. Overflowing tabs keep the active tab visible, with previous/next buttons at the right.

## Sessions

A background server owns every pane, so closing the terminal only detaches.

```bash
shepherd                    # attach the default session (starts it if needed)
shepherd new api --cwd ~/code/api
shepherd ls                 # running sessions, plus saved ones you can restore
shepherd kill api
shepherd --remote ssh://devbox   # thin client here, server there (needs shepherd on the remote PATH)
```

Layouts, pane names, working directories and agents are saved in `~/.local/state/shepherd/shepherd.db`. After a reboot, attaching restores the session: shells come back in their directories, agents are relaunched into the exact conversation their integration reported (`claude --resume <id>`, `codex resume <id>`, …) or else their latest one (`claude --continue`, …), and plain commands are typed back in but not run.

A `shepherd.toml` in the directory you start from lays out a new session:

```toml
[[pane]]
name = "server"
run = "pnpm dev"

[[pane]]
name = "coder"
agent = "claude-code"
prompt = "read TODO.md and start on the first item"
```

## Agents

Shepherd has no AI of its own. It notices when a pane runs a known agent and tracks its state: blocked (needs you), working, done (finished while you weren't looking), or idle. The state shows in the pane border, the tab bar and the sidebar, and a background agent that gets blocked or finishes triggers a notification.

Known agents: Claude Code, Codex, Gemini CLI, Cursor Agent, Copilot CLI, OpenCode, Pi, OMP, Droid, Amp, Kiro CLI, Kimi Code, Kilo Code, Devin CLI, Grok CLI, Hermes Agent, Qoder CLI, Qwen Code, Antigravity CLI, Cline, MastraCode, Maki, Muse and Aider — found by process name, also when started through node, bun, python or a shell script.

State is read from the agent's screen with no setup: each agent has a manifest of rules over the live bottom of its screen, its terminal title and its OSC 9;4 progress (`src/config/agents/manifests`). Integrations use the agent's own hooks or plugins to add more:

- **session** — Claude Code, Codex, Copilot CLI, Cursor Agent, Devin CLI, Droid, Qoder CLI, Qwen Code, Grok CLI, Antigravity CLI and Hermes Agent report their session id, so a restart resumes the exact conversation (`claude --resume <id>`, `codex resume <id>`, …). Their state stays on screen detection, because their hooks miss some transitions (interrupts, permission answers).
- **state + session** — OpenCode, Kilo Code, Pi, OMP, Kimi Code and MastraCode see every transition, so while they report they decide the pane's state. A dialog visibly waiting on you still wins.

```bash
shepherd integration status              # every agent: installed, update available, available, not found
shepherd integration install all         # everything recommended for the agents on this machine
shepherd integration install codex       # or one agent; uninstall takes out only shepherd's entries
```

The settings page (`Ctrl+B s` → integrations) does the same. Integrations install on the machine the server runs on, where the agents are. Claude Code and Codex also get the shepherd MCP server.

Your own tools can report too: `shepherd report --source my-tool --agent my-agent --state working|blocked|idle [--seq n] [--session-id id]`, and `--release` hands the pane back to screen detection. Reports without `--source` don't change state.

Add an agent or override one with a TOML file in `~/.config/shepherd/adapters/<id>.toml` (`name`, `process`, `launch`, `resume`, `resumeSession`, and `[[rules]]` in the manifest format). `shepherd debug detect <pane>` shows the matched rule, the reporting integration, and the title and progress it saw.

## Driving shepherd from an agent

Every pane gets `SHEPHERD_SOCKET` and `SHEPHERD_PANE_ID`, so any agent that can run shell commands can drive the workspace:

```bash
shepherd pane split --name tests "pnpm test"     # opens next to the caller
shepherd wait tests --exited                     # prints "exited <code>"
shepherd pane read tests --lines 80
shepherd agent spawn codex --name reviewer --prompt "review src/auth"
shepherd wait @reviewer --state idle
shepherd send @reviewer "fixed, please re-check"  # typed into the agent once it's idle
shepherd inbox
shepherd events --follow
```

`shepherd help` lists every command. `shepherd mcp` exposes the same operations as MCP tools. Messages carry a hop count and a per-pair rate limit so two agents can't ping-pong forever, and `Ctrl+B m` pauses delivery. When an agent types into or closes a pane it didn't create, you get an allow / always / deny prompt; the policy is set in `[permissions]` in the config.

## Config

`Ctrl+B s` opens the settings page; `shepherd config edit` (or "Edit config.toml" in the command palette) opens `~/.config/shepherd/config.toml` itself; changes apply live. It covers the prefix key, theme, sidebar, which notifications fire (toast, system, sound, bell), messaging limits, permissions, per-agent launch commands, and plugins. Plugins are any commands started with `SHEPHERD_SOCKET` set.

## Build

```bash
bun run build   # single binary at dist/shepherd (ad-hoc signed on macOS)
bun site/build.ts   # the docs site into site/out, every link checked
```

## Contributing & releases

Work lands on `dev` (CI: typecheck and tests on Linux and macOS). A pull request from `dev` to `staging` publishes a rolling `staging` prerelease; one from `staging` to `main` releases.

Every push to `main` is a release (`.github/workflows/release.yml`): the tests run, the next version is taken from the git tags — the patch goes up, or the minor/major when a commit since the last release says `[minor]`/`[major]` — each platform is compiled on its own runner, and the release gets the binaries, `SHA256SUMS` and `manifest.json`. The docs site deploys to GitHub Pages in the same run. A push that changes only docs skips the release and just redeploys the site.

## Test

```bash
bun run test              # everything
bun test test/unit        # fast, pure tests
bun test test/e2e/ui      # the TUI driven in a real PTY
```

- `test/unit/` — pure logic: layout math, chrome geometry, config, detection against captured screens, mailbox, connections
- `test/e2e/` — one file per feature, each with its own sandboxed session: sessions, panes, cli, agents, messaging, permissions, mcp, integrations, plugins, remote, reconnect
- `test/e2e/ui/` — keyboard, mouse, spaces, chrome, read back through `libghostty-vt`
- `test/support/` — the harness (`sandbox`, `Screen`, `startServer`), a fake agent, and mouse encoders
- `test/fixtures/` — captured agent screens used by the detection tests

## Layout

- `src/main.ts` — entry point; dispatches to the CLI, server, client, MCP or integrations
- `src/cli/` — argument parsing, help, API commands, session commands (attach, ls, kill, restart)
- `src/core/` — paths and the split-tree layout math
- `src/protocol/` — shared types, the Zod JSON-RPC schema, connections and transports
- `src/config/` — config file, themes, and the agents shepherd knows (`agents/`: process names, launch/resume, screen manifests)
- `src/server/` — the session server
  - `server.ts` startup/shutdown, `context.ts` shared state
  - `rpc/` — dispatch, TUI methods, public API
  - `session/` — spaces, tabs, panes; `PtyPane` is `Bun.Terminal` + a headless libghostty terminal
  - `agents/` — detection (process identification, the manifest rule engine, integration authority), the monitor tick, the mailbox
  - `persist/` — `bun:sqlite` store, restore, `shepherd.toml` templates
- `src/client/` — the OpenTUI client
  - `app.ts` startup, `context.ts` the `App` state, `render.ts`, `connection.ts`, `actions.ts`
  - `panes/` — terminal panes, mouse resize, pointer shape
  - `chrome/` — tabs, sidebar, status bar, buttons
  - `modals/` — prompt, pick, menu, confirm, permission, context menu, the settings page
  - `sound/` — cuelume's sound recipes, rendered to WAV and played through OpenTUI's audio engine
  - `input/` — prefix bindings, keyboard handler, copy mode
- `src/platform/` — controlling-terminal exec (`bun:ffi`) and the embedded libghostty libraries
- `src/mcp/` — the MCP server
- `site/` — the docs site: `build.ts` (a dependency-free static generator), `content/` (the landing page and docs), `assets/` (CSS, JS, icons)
- `.github/` — CI, the release workflow and its scripts; `install.sh` is the installer
- `src/integrations/` — every agent integration (`targets.ts`), config-file editing, plugin sources, and `shepherd hook`

## License

[MIT](LICENSE). The agent detection manifests in `src/config/agents/manifests` are third-party files under the Apache License 2.0 (see the `LICENSE` and `NOTICE` there); the site's bundled fonts carry their own licenses in `site/assets/fonts`.
