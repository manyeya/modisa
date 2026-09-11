// `shepherd help`.
export const HELP = `shepherd — an agent-aware terminal multiplexer

sessions
  shepherd [-s name] [--remote ssh://host]   attach (starts the server if needed)
  shepherd new <name> [--cwd dir]            new session
  shepherd ls | kill [name] | restart [name] | detach   (restart: load updated code, keep the session)

panes (from inside a pane, targets default to the calling pane)
  shepherd pane list [--json]
  shepherd pane split [--right|--down] [--name n] [--cwd d] [--target p] [command…]
  shepherd pane run <target> <command…>
  shepherd pane read [target] [--lines 50] [--json]
  shepherd pane keys <target> <key…>          keys: text, Enter, C-c, M-x, Up, Escape…
  shepherd pane close [target] | rename [target] <name> | focus <target>

agents
  shepherd agent spawn <harness> [--name n] [--prompt text] [--down] [--tab]
  shepherd agent list [--json]
  shepherd wait <target> [--exited | --state idle|working|blocked|done | --match regex] [--timeout s]
  shepherd send <target> <message…>          message another agent (delivered when it's idle)
  shepherd inbox | messages [--follow] | pause
  shepherd report [pane] [--state s] [--source id] [--agent id] [--seq n] [--session-id id] [--release]   (integrations)

workspace
  shepherd workspace create [name] [--cwd dir] | workspace list
  shepherd workspace rename <space> <name> | workspace close <space>   (space: id or name)
  shepherd tab create [name] [--command cmd]
  shepherd events [--follow] [--output]

setup
  shepherd update                            install the newest release (then shepherd restart)
  shepherd version | --version               this version, and whether a newer one is out
  shepherd integration status | install|uninstall <agent|all>
  shepherd mcp                               MCP server (stdio) exposing the same API
  shepherd config [path|edit]
  shepherd debug detect <target>

targets: pane id (p3), @name or name`;
