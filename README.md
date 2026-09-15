# shepherd

An agent-aware terminal multiplexer. No AI of its own — it hosts coding agents (Claude Code, Codex, Pi, …) and shells in real terminal panes, and tells you which one needs you. Docs: **https://manyeya.github.io/shepherd**.

Built with Bun and OpenTUI: real terminal panes, named spaces, an agent sidebar, a segmented status bar, mouse menus, and live themes.

## Install

```bash
curl -fsSL https://manyeya.github.io/shepherd/install.sh | sh
```

Releases are built for macOS on Apple silicon and Linux on x64 and arm64. The installer checks the download against its published SHA-256 and puts `shepherd` in `~/.local/bin` (`SHEPHERD_INSTALL_DIR` changes that; `SHEPHERD_CHANNEL=staging` installs the latest prerelease).

Or install with a package manager, which then updates and removes it:

| With | Install | Remove |
|------|---------|--------|
| mise | `mise use -g github:manyeya/shepherd` | `mise uninstall github:manyeya/shepherd` |
| Debian, Ubuntu | `sudo apt install ./shepherd_<version>_<arch>.deb` | `sudo apt remove shepherd` |
| Fedora, RHEL | `sudo dnf install ./shepherd-<version>-1.<arch>.rpm` | `sudo dnf remove shepherd` |

Every release on [GitHub Releases](https://github.com/manyeya/shepherd/releases) has the binaries, the .deb and .rpm packages, `SHA256SUMS`, the `manifest.json` the installer and updater read, and signed build provenance. To check that a file was built by this repository's release workflow from a tagged commit, run `gh attestation verify shepherd-darwin-arm64 -R manyeya/shepherd`.

When a newer release is out, the status row shows `↑ <version>`: click it to update and restart the server (agents resume). From the command line: `shepherd version`, `shepherd update`, then `shepherd restart`. If mise or a package installed shepherd, update with that tool instead (shepherd tells you the command). `[update] channel = "staging"` follows prereleases; `[update] check = false` turns the check off.

### Uninstall

```bash
shepherd uninstall        # or: curl -fsSL https://manyeya.github.io/shepherd/install.sh | sh -s -- --uninstall
```

It lists what it will do and asks first. Then it removes shepherd's hooks and skill from every agent, stops running sessions, and deletes saved state in `~/.local/state/shepherd`. Your config in `~/.config/shepherd` stays unless you add `--purge`. If the install script put the binary there, uninstall deletes it too; if mise or a package did, finish with that tool's remove command.

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

### Remote sessions over SSH

`--remote` splits shepherd in two: the TUI runs on your machine, the server and every pane run on the far side. Your keybindings, theme, sounds and notifications stay local; the panes, the agents and their files are remote.

```bash
shepherd --remote ssh://devbox              # host
shepherd --remote ssh://me@build-01:2222    # user and port
shepherd --remote devbox                    # a Host alias from ~/.ssh/config
shepherd -s api --remote ssh://devbox       # a named session on that machine
```

There is no daemon to install and no port to open. Shepherd shells out to your own `ssh`, so agent forwarding, jump hosts, `Match` blocks and keys from `~/.ssh/config` all apply. The exact command it runs:

```bash
ssh -T devbox shepherd proxy -s default             # ssh://devbox
ssh -T -p 2222 me@build-01 shepherd proxy -s api    # ssh://me@build-01:2222, -s api
```

`shepherd proxy` on the far side starts the session server if it isn't running, then bridges its unix socket to stdio. `-T` means no pty is allocated: the protocol is newline-delimited JSON, not a terminal.

**Shepherd must be on the remote machine**, and by default it must be on the `PATH` of a
*non-interactive* ssh shell — which often excludes `~/.local/bin`, where the installer puts it. If
`--remote` fails with something like `shepherd: command not found`, set an absolute path in your
**local** config (`~/.config/shepherd/config.toml`):

```toml
remote_command = "/home/me/.local/bin/shepherd"
```

Check what the remote shell actually sees with `ssh devbox 'command -v shepherd'`.

Other things worth knowing:

- **Everything server-side happens remotely.** Panes, agents, integrations and plugins all run on the remote machine, with its files. What a plugin shows in the TUI is drawn by your local client, in your theme and with your own `[plugin_keys]`.
- **Sessions are per machine.** `shepherd ls` lists local sessions; to see the remote ones, `ssh devbox shepherd ls`.
- **Detaching leaves it running.** `Ctrl+B d` drops the ssh connection; the remote session keeps going, and so do the agents. Reattach from any machine.
- **Copy uses OSC 52**, so `Ctrl+B` copy actions reach your local clipboard through ssh in terminals that support it.
- **Updating is per machine.** `shepherd update` on your laptop doesn't touch the remote install; run it there too, then `shepherd restart`.
- `SHEPHERD_SSH` overrides the ssh binary, which is how the test suite substitutes a fake.

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

The settings page (`Ctrl+B s` → integrations) does the same. Integrations install on the machine the server runs on, where the agents are.

### The skill

Installing an integration also drops in the **shepherd skill** — a `SKILL.md` teaching the agent to split panes, spawn and message other agents, and wait on them, so it drives the session without you explaining the CLI each time. There's no separate command; it comes with the integration:

```bash
shepherd integration install claude       # hooks + the skill, for one agent
shepherd integration install all          # …for every agent on this machine
shepherd integration status               # "↻ update available" when a newer skill ships
shepherd integration uninstall claude     # takes the skill back out too
```

```text
~/.agents/skills/shepherd/SKILL.md          the one copy
~/.claude/skills/shepherd  -> …/.agents/skills/shepherd
~/.codex/skills/shepherd   -> …/.agents/skills/shepherd
```

Every agent above gets it except MastraCode and OMP, whose skills directories aren't established. Kimi Code reads `~/.agents/skills` itself, so it needs no link. Restart a running agent to pick the skill up.

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

`shepherd help` lists every command, and the shepherd skill teaches them to an agent that has it. Messages carry a hop count and a per-pair rate limit so two agents can't ping-pong forever, and `Ctrl+B m` pauses delivery. When an agent types into or closes a pane it didn't create, you get an allow / always / deny prompt; the policy is set in `[permissions]` in the config.

## Plugins

A plugin is a program shepherd starts with each session, connected to the session's socket: it watches what happens, offers actions, and can put a little into the TUI. `shepherd plugin new` scaffolds one in TypeScript with shepherd's client library, a behavioural test, and `AGENTS.md`, a guide an agent can follow to write the rest.

```sh
shepherd plugin new my-plugin          # plugin.json, plugin.ts, a test, the client library, AGENTS.md
shepherd plugin check my-plugin        # manifest, build, then a throwaway session: starts, connects, passes its tests
shepherd plugin link my-plugin         # every session starts it; the running one starts it now
shepherd plugin search [words]         # GitHub repositories with the shepherd-tui-plugin topic, and how to install each
shepherd plugin install https://github.com/you/shepherd-plugins --subdir attention-log --ref v1.2.0
shepherd plugin list | logs <name> | stop <name> | start <name>
shepherd plugin run <name> <action> '{"any":"params"}'
shepherd plugin unlink <name>
```

- **`plugin.json`** names the plugin, its protocol version and how to start it (`run`, an argv run in its directory), and what it offers: `actions` (also listed in the command palette), `panes`, `keys` under the prefix, and `links`.
- **Install** clones a git repository (optionally a `--ref` branch, tag or commit, and a `--subdir`), checks it, links it and starts it. It takes `https://`, `ssh://`, `git://` and `file://` URLs and `user@host:path`, never a remote-helper (`<helper>::`) URL or plain http, and git only uses those transports whatever your git config or environment says; your ssh agent, ssh command and credential helpers still apply. No build or dependency scripts run before the plugin starts, and it says when the plugin needs setup. `plugin list` shows each install's source and commit. Unlinking an install stops it in every running session, then deletes its checkout (never its data), or keeps the checkout and says why.
- **In the TUI**: status segments, a sidebar section, badges on pane borders, entries in the pane menu, toasts, and panes opened as a split, tab, zoomed pane, overlay or popup. Shepherd draws all of it in your theme, names the plugin on every piece, and limits how much each plugin, and a session's plugins together, can show.
- **Keys**: `keys` bind a key under the prefix to an action or a pane. A key shepherd uses, or one two plugins want, is off; move one in `[plugin_keys]` (`"<plugin>.<action or pane>" = "Y"`, or `""` to turn it off). Each client binds keys with its own config.
- **Links**: Ctrl+click an http(s) URL in a pane to hand it to a plugin's action, matched by a `pattern` URL glob or a `regex` in RE2 syntax (linear time, so no backreferences or lookaround). When several plugins match, you choose. Terminal hyperlinks (OSC 8) whose label differs from their destination aren't followed.
- **Remote**: with `--remote`, plugins, their processes and their files stay on the server's machine, and your client draws their UI with your theme, keys and notification settings. A client too old for plugin UI simply doesn't show it.
- **Startup and event workflows** aren't manifest hooks: a plugin is a long-lived program, so it subscribes to events (a snapshot of every pane, then each change) and reacts.
- A plugin runs as you, with your files and network. It isn't sandboxed.

A `[[plugin]]` `run` line in config.toml still starts a program with no manifest. **[`examples/plugins/`](examples/plugins/) is the full guide** to the protocol underneath: the events, the methods, the error codes, and the rules that will bite you. [`attention-log/`](examples/plugins/attention-log/) is a complete plugin built with `plugin new`.

## Config

`Ctrl+B s` opens the settings page; `shepherd config edit` (or "Edit config.toml" in the command palette) opens `~/.config/shepherd/config.toml` itself; changes apply live. It covers the prefix key, theme, sidebar, which notifications fire (toast, system, sound, bell), messaging limits, permissions, per-agent launch commands, `remote_command` for `--remote`, `[plugin_keys]`, and `[[plugin]]` programs.

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
- `test/e2e/` — one file per feature, each with its own sandboxed session: sessions, panes, cli, agents, messaging, permissions, integrations, plugins, remote, reconnect
- `test/e2e/ui/` — keyboard, mouse, spaces, chrome, read back through `libghostty-vt`
- `test/support/` — the harness (`sandbox`, `Screen`, `startServer`), a fake agent, and mouse encoders
- `test/fixtures/` — captured agent screens used by the detection tests

## Layout

- `src/main.ts` — entry point; dispatches to the CLI, server, client or integrations
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
- `src/integrations/` — every agent integration (`targets.ts`), config-file editing, plugin sources, and `shepherd hook`
- `src/skills/` — the shepherd skill (`shepherd/SKILL.md`), installed by the integrations
- `src/plugins/` — the plugin authoring kit: `shepherd-plugin.ts` (the client library plugins vendor) and the `shepherd plugin new` templates
- `examples/plugins/` — how to write a plugin, the attention log (built from `shepherd plugin new`, checked by `test/e2e/plugin-authoring.test.ts`) and an older hand-written one
- `site/` — the docs site: `build.ts` (a dependency-free static generator), `content/` (the landing page and docs), `assets/` (CSS, JS, icons)
- `.github/` — CI, the release workflow and its scripts; `install.sh` is the installer

## License

[MIT](LICENSE). The agent detection manifests in `src/config/agents/manifests` are third-party files under the Apache License 2.0 (see the `LICENSE` and `NOTICE` there); the site's bundled fonts carry their own licenses in `site/assets/fonts`.
