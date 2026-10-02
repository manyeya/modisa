// `modisa help`.
export const HELP = `modisa — an agent-aware terminal multiplexer

sessions
  modisa [-s name] [--remote ssh://host]   attach (starts the server if needed)
  modisa new <name> [--cwd dir]            new session
  modisa ls | kill [name] | restart [name] | detach   (restart: load updated code, keep the session)

panes (from inside a pane, targets default to the calling pane)
  modisa pane list [--json]
  modisa pane split [--right|--down] [--ratio 0.5] [--name n] [--cwd d] [--env K=V]… [--target p] [command…]
                                            --ratio: the new pane's share of the room (0.1 to 0.9)
  modisa pane run <target> <command…>
  modisa pane read [target] [--source recent|visible|recent-unwrapped] [--format text|ansi] [--lines 50] [--json]
                                            recent: the last --lines lines as the pane wraps them; recent-unwrapped:
                                              with soft wraps joined; visible (or --screen): the screen. ansi keeps
                                              colours and styles. A full-screen app (vim, less) has only its screen
  modisa pane keys <target> <key…>          keys: text, Enter, C-c, M-x, Up, Escape…
  modisa pane close [target] | rename [target] <name>
  modisa pane focus [target] [--direction left|right|up|down]   with a direction: the pane on that side of it
  modisa pane move [target] --tab t [--target p] | --target p | --new-tab [--workspace w] | --new-workspace
                   [--name n] [--split right|down] [--ratio 0.5] [--focus] [--json]
                                            beside a pane (else the tab's focused one), or alone in a new tab or space
                                              (--name names it); emptied tabs and spaces close; --focus follows it
  modisa pane swap [target] <other> | swap [target] --direction d   trade places, in a tab or across tabs
  modisa pane resize [target] --direction d [--amount 2]   move the border on that side (prints changed or unchanged)
  modisa pane zoom [target] [--on|--off|--toggle]   show it alone in its tab (toggle by default)
  modisa pane layout [target]               its tab: each pane's box (x, y, w, h in cells) and whether it's shown
  modisa pane neighbor [target] --direction d   the pane on that side of it
  modisa pane edges [target]                the pane on each side of it, or (edge)
  modisa pane process-info [target]         its pid, the job in the foreground of its terminal, and its shell's cwd

agents
  modisa agent spawn <harness> [--name n] [--prompt text] [--down] [--tab] [--env K=V]…
  modisa agent list [--json]
  modisa wait <target> [--exited | --state idle|working|blocked|done | --match regex] [--timeout s]
  modisa send <target> <message…>          message another agent (delivered when it's idle)
  modisa inbox | messages [--follow] | pause
  modisa report [pane] [--state s] [--source id] [--agent id] [--seq n] [--session-id id] [--release]   (integrations)

plugins (examples/plugins/README.md)
  modisa plugin new <name> [--dir d]        scaffold one: TypeScript, modisa's client library, a guide, a test
  modisa plugin check <dir> | dev <dir>     verify it in a throwaway session | try it in one (not a sandbox)
  modisa plugin sdk | schema                the client library's source | every request, result and event (JSON Schema)
  modisa plugin list [--json] | logs <name> [--lines 50] | stop <name> | start <name>
  modisa plugin link <dir> [--json]         register it for every session, then start it in the running session
                                              reached (the default, or -s) and wait for it to connect
  modisa plugin search [words] [--json]     GitHub repositories with the modisa-tui-plugin topic, most starred first,
                                              each with the command that installs it (none of them vetted)
  modisa plugin install <git-url> [--ref r] [--subdir d] [--json]   clone, check and link it, then start it like link.
                                              No dependencies are installed and nothing is built; it runs as you
  modisa plugin unlink <name> [--json]      remove the link. Installed: stopped in every running session, then its
                                              checkout deleted (kept, saying why, if a session still uses it or can't be
                                              reached). Linked by you: stopped in the session reached; never deleted
  modisa plugin run <name> <action> [json]   call an action a running plugin offers. A timeout means its outcome is
                                              unknown: running it again can repeat its effects

workspace
  modisa workspace create [name] [--cwd dir] [--command cmd] [--env K=V]… | workspace list
  modisa workspace rename <space> <name> | workspace close <space>   (space: id or name)
  modisa tab create [name] [--command cmd] [--cwd dir] [--pane-name n] [--workspace w] [--env K=V]…
  modisa events [--follow] [--output]

setup
  modisa update                            install the newest release (then modisa restart); names brew or mise's command when they installed it
  modisa uninstall [--purge] [--yes]       remove integrations, sessions, state (and config with --purge), and install.sh's binary
  modisa version | --version               this version, and whether a newer one is out
  modisa integration status | install|uninstall <agent|all>   (hooks + the modisa skill)
  modisa logos [status|install|uninstall]  agents' logos in the sidebar: a font, and your terminals told about it
  modisa config [path|edit]
  modisa debug detect [target]

targets: pane id (p3), @name or name; p3:1a2b3c4d (from a message's reply hint) reaches only that pane: it survives a
  rename, but fails once the pane closes or the server restarts. Outside a pane, split, focus, move, swap, resize, zoom,
  layout, neighbor, edges and process-info default to the focused pane. Tabs: id or name

pane split, agent spawn, tab create and workspace create print the new pane's id; with --json, the whole pane and its
  workspaceId and tabId. --env NAME=value (once per variable; not MODISA_*) is set in the new pane and saved with the
  session, so a restart keeps it: it's on disk, in ~/.local/state/modisa/modisa.db (readable only by you). A --cwd of
  ~ or a relative path (new included) is from where you run modisa

exit status: 0 ok, 1 failed, 2 usage, 3 server unreachable, 124 wait timed out; wait --exited exits with the pane's code
  (so a child's own 1/2/3/124 looks the same: with --json its result is on stdout, modisa's error on stderr)
  with --json, a failure prints {"error":{"code","message"}} to stderr (codes: no_such_pane, pane_gone, timeout, unreachable…)`;
