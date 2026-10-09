# Customizing modisa

Everything below lives in `~/.config/modisa/config.toml` (and the files it names) and applies live: save the file and
every attached TUI redraws. `modisa config check` reports mistakes with their line; the settings page (prefix `s`)
edits the common settings. Formats, colours and styles are the same small languages plugins use (VIEWS.md): a style
is `"bold $accent on $bar"`, a colour is `$token`, `#rrggbb`, a name or `0`–`255`.

## Themes

```toml
theme = "tokyonight"                               # one theme
theme = { dark = "tokyonight", light = "github-light" }   # follow the terminal's light or dark background
```

With `dark`/`light`, modisa asks the terminal for its background colour when a client starts and whenever the
terminal says its colour scheme changed (CSI ? 2031, Ghostty, kitty, iTerm2, WezTerm) or regains focus, and uses the
matching theme.

A theme of your own is a file in `~/.config/modisa/themes/<name>.toml`, used by its name:

```toml
inherits = "catppuccin-mocha"   # optional: start from another theme
bg = "#11111b"
accent = "#f5c2e7"
# every token: bg bar fg dim border focus accent warn working blocked done idle
# and the roles, each a style, defaulting to the tokens:
[roles]
"tab.active" = "bold $fg on $bg"
"tab.inactive" = "$dim on $bar"
"pane.border" = "$border"
"pane.border.focused" = "$focus"
"pane.title" = "$fg"
"sidebar" = "$fg on $bar"
"sidebar.selected" = "on $border"
"status" = "$fg on $bar"
"selection" = "on #2a2b3d"
"menu" = "$fg on $bar"
"menu.selected" = "bold $bg on $accent"
"toast" = "$fg on $bar"
```

`modisa theme import <file>` turns another app's colour scheme into a modisa theme (written to the themes directory,
named after the file): Ghostty theme files, iTerm2 `.itermcolors`, Alacritty `.toml`, kitty `.conf`, Windows
Terminal JSON, base16/base24 YAML. `modisa theme list` lists built-in and installed themes, `modisa theme show
<name>` prints one as a file to start from.

## Formats

A format is text with `{variables}`, `#[styles]` and conditionals. The status row, tab labels, pane titles, the
terminal's window title and the sidebar's agent rows each have one.

| | |
|---|---|
| `{name}` | a variable (below); unknown ones are empty, and `config check` warns |
| `{name:arg}` | with an argument: `{clock:%H:%M}`, `{cwd:short}`, `{sh:git log -1 --format=%s|30s}` |
| `{?name\|then\|else}` | `then` when `name` is set and not 0/false/empty, else `else` (both may hold variables) |
| `{name=value\|then\|else}` | `then` when `name` equals `value` |
| `{name:=N}` / `{name:<N}` | at most N cells (cut with …) / padded to N cells |
| `#[style]` | style from here on (`#[bold $warn]`, `#[on $bar]`); `#[]` back to the format's base style |
| `#[click=action]…#[/click]` | a clickable part: a modisa action (`palette`, `split-right`, …), `plugin:<name>.<action>`, or `sh:<command>` |
| `#[align=right]` | what follows goes to the right edge |

Variables (where they make sense): `session` `space` `tab` `tab.index` `pane` `pane.index` `name` `title`
`terminal_title` `cwd` `command` `agent` (harness name) `agent.id` `state` (working/blocked/done/idle) `icon` (the
agent's mark) `working` `blocked` `done` `idle` (counts) `panes` `tabs` `spaces` `zoomed` `mode` (the current key
mode) `prefix` (when the prefix is armed) `git.branch` `git.repo` `git.ahead` `git.behind` `git.changes`
`git.clean` `theme` `host` `user` `clock` `date` `meta.<key>` (see Metadata) `plugin.<name>.<id>` (a plugin's
status piece, CHROME.md) `sh` (a command's first output line, cached for its interval; default 10s).

```toml
[status]
left = "#[click=palette]#[bold $accent] modisa #[] {space} {?blocked|#[$blocked]! {blocked} need you|}"
right = "{?git.branch|{git.branch}{?git.changes| ●{git.changes}} |}#[$dim]{clock:%H:%M}"
rows = 1                    # 1–3; with more, left2/right2, left3/right3

[tabs]
format = "{tab.index}:{tab}{?blocked| #[$blocked]!}"
position = "top"            # top, bottom or hidden

[panes]
title = "{name} {?agent|{icon} {agent} {state}|}"
title_position = "top_left" # top_left, top_center, top_right, bottom_left, …

[window]
title = "{space} · {?agent|{agent} {state}|{command}}"   # the terminal's title (OSC 2); "" leaves it alone
```

The status row's built-in buttons are variables too (`{sidebar_button}`, `{agent_button}`, `{working_button}`,
`{needyou_button}`, `{panes_button}`), so a format places them or leaves them out. Without `left`/`right` the row is
modisa's default.

## Sidebar

```toml
[sidebar]
position = "left"           # left or right
sections = ["agents", "plugin:radar", "commands"]   # order; leave one out to hide it
row = ["{icon} {name}", "  #[$dim]{agent} · {state}{?meta.context| · {meta.context}|}"]   # 1–3 lines per agent
group = "tab"               # tab, space, repo, state or none
sort = "attention"          # attention (needs-you first), recent, name, created
show = "all"                # all, space (this space's agents), tab
```

## Panes

```toml
[panes]
border = "rounded"          # plain, rounded, double, thick, light_double_dashed, heavy_double_dashed,
                            # light_triple_dashed, heavy_triple_dashed, light_quadruple_dashed,
                            # heavy_quadruple_dashed, quadrant_inside, quadrant_outside, none
dim_unfocused = 0.25        # 0–0.8: mix unfocused panes' text toward the background
gap = 0                     # cells between panes (0–2)
```

## Keys

Every key table is `action = key`, or a list of keys (`""` for none), like `[keys]` has always been.

```toml
prefix = "C-b"

[keys]                      # after the prefix
zoom = "f"
"split-right" = ["v", "|"]
"dev-layout" = "D"          # a list from [actions], a command or a mode binds the same way

[actions]                   # a name for a list of actions, run in order
"dev-layout" = ["split-right", "focus-left", "zoom"]

[root_keys]                 # without the prefix: these never reach the panes, so each needs C- or M- (or is an f-key)
"focus-left" = "M-h"
"focus-right" = "M-l"
palette = "M-return"

[modes.resize]              # a key mode: enter it, then its keys work alone until escape (or the prefix)
enter = "r"                 # after the prefix
sticky = true               # stay after each key (false: one key, then back)
timeout = 0                 # ms after its last key before it ends; 0 never
keys = { "resize-left" = "h", "resize-right" = "l", "resize-down" = "j", "resize-up" = "k" }
```

A key with modifiers is written `C-` (ctrl), `M-` (alt), `S-` (shift, for a named key: a shifted character is just
that character) before its name: `M-h`, `C-M-left`, `S-tab`, `f5`. Names: one character, `return`, `tab`, `space`,
`backspace`, `escape`, `delete`, `left`/`right`/`up`/`down`, `home`, `end`, `pageup`, `pagedown`, `f1`–`f12`.

An action is anything in the keyboard guide (prefix `?`), a name from `[actions]`, a `[[command]]`'s name,
`mode:<name>`, `plugin:<name>.<action>`, or `sh:<command>` (runs in the background, where the focused pane is). While a
mode is on, its name shows at the left of the status row.

## Commands

Your own entries in the palette, with keys and prompts:

```toml
[[command]]
name = "Run tests"
key = "T"                   # after the prefix; or root = "M-t"
run = "bun test {filter}"   # {cwd} {pane} {name} {space} {tab} {session} and the prompts' names; shell-quoted here
in = "split"                # split (default), split-down, tab, zoomed, background
cwd = "{cwd}"               # the focused pane's directory (default)
prompts = [{ name = "filter", title = "Test filter", default = "" }]

[[command]]
name = "Switch branch"
run = "git switch {branch}"
in = "background"
prompts = [{ name = "branch", pick = "git branch --format=%(refname:short)" }]   # pick from a command's lines
```

## Hooks

Commands run on what happens. A hook gets the event as JSON on stdin and `MODISA_EVENT`, `MODISA_PANE_ID` etc. in
its environment.

```toml
[[hook]]
on = "agent.state"
when = "to == 'blocked'"    # optional: a condition on the event's fields (==, !=, contains, &&, ||, !)
run = "say '{name} needs you'"

[[hook]]
on = "message.send"         # before an agent's message is delivered: may change or stop it
run = "~/bin/redact-secrets"
timeout = 2000              # ms (default 1000); on a timeout or failure the event goes on unchanged
```

Observing events: `agent.state`, `pane.created`, `pane.exited`, `pane.cwd`, `command.finished` (OSC 133, with its
exit code and duration), `client.attached`, `client.detached`, `theme.changed`, `notify`, `plugin.started`,
`plugin.stopped`. Intercepting events (the hook may print `{"allow": false, "reason": "…"}` to stop it, or the event's
fields changed, e.g. `{"text": "…"}`): `message.send`, `agent.spawn` (its prompt and launch command), `pane.keys`
and `pane.run` (from agents), `notify` (its text, or drop it).

## Notifications and sound

```toml
[notify]
blocked = ["toast", "system", "sound"]
unread = true               # a tab keeps a • after an agent in it needed you or finished, until you look at it
click = "focus"             # clicking a system notification focuses the pane (macOS: needs terminal-notifier)

[notify.codex]              # per agent (its id: claude-code, codex, …), over the above
done = []

[sound]
pack = "~/.config/modisa/sounds/retro"   # blocked.wav, done.mp3, working.aiff… (wav, mp3, aiff, ogg) play instead
volume = 0.7
[sound.claude-code]
done = "~/sounds/ding.wav"  # a file, or a built-in sound's name
```

A pane's right-click menu has "Mute this pane": its agent then makes no sound and no system notification (toasts
still show).

## Metadata

Any program can show values in modisa (`{meta.<key>}` in formats, and plugins' snapshot): context %, cost, the
model, a test run's progress.

- `modisa pane meta set context=43% cost=$1.20 [--pane p3] [--ttl 60s]`, `modisa pane meta clear [keys…]`
- from inside a pane, without the CLI: the user-var escape sequence other terminals use,
  `printf '\e]1337;SetUserVar=modisa_context=%s\a' "$(printf 43%% | base64)"`

modisa's integrations fill some keys for you when the agent reports them: `model`, `context`, `cost`, `session`.

## Profiles

`modisa profile export > my-setup.toml` writes everything above (config, your themes, sound pack paths, plugin
sources) to one file. `modisa profile import <file or URL>` merges one into yours, after backing yours up to
`config.toml.bak`, and installs its plugins after showing what they are. Share setups as a gist.
