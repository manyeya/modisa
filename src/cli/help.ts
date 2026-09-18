// `modisa help`.
export const HELP = `modisa — an agent-aware terminal multiplexer

sessions
  modisa [-s name] [--remote ssh://host]   attach (starts the server if needed)
  modisa new <name> [--cwd dir]            new session
  modisa ls | kill [name] | restart [name] | detach   (restart: load updated code, keep the session)

panes (from inside a pane, targets default to the calling pane)
  modisa pane list [--json]
  modisa pane split [--right|--down] [--name n] [--cwd d] [--target p] [command…]
  modisa pane run <target> <command…>
  modisa pane read [target] [--lines 50] [--json]
  modisa pane keys <target> <key…>          keys: text, Enter, C-c, M-x, Up, Escape…
  modisa pane close [target] | rename [target] <name> | focus <target>

agents
  modisa agent spawn <harness> [--name n] [--prompt text] [--down] [--tab]
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
  modisa workspace create [name] [--cwd dir] | workspace list
  modisa workspace rename <space> <name> | workspace close <space>   (space: id or name)
  modisa tab create [name] [--command cmd]
  modisa events [--follow] [--output]

setup
  modisa update                            install the newest release (then modisa restart); names brew or mise's command when they installed it
  modisa uninstall [--purge] [--yes]       remove integrations, sessions, state (and config with --purge), and install.sh's binary
  modisa version | --version               this version, and whether a newer one is out
  modisa integration status | install|uninstall <agent|all>   (hooks + the modisa skill)
  modisa logos [status|install|uninstall]  agents' logos in the sidebar: a font, and your terminals told about it
  modisa config [path|edit]
  modisa debug detect <target>

targets: pane id (p3), @name or name; p3:1a2b3c4d (from a message's reply hint) reaches only that pane: it survives a
  rename, but fails once the pane closes or the server restarts

exit status: 0 ok, 1 failed, 2 usage, 3 server unreachable, 124 wait timed out; wait --exited exits with the pane's code
  (so a child's own 1/2/3/124 looks the same: with --json its result is on stdout, modisa's error on stderr)
  with --json, a failure prints {"error":{"code","message"}} to stderr (codes: no_such_pane, pane_gone, timeout, unreachable…)`;
