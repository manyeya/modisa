// Docs: agents, automation, and reference. Agent and integration lists come from the code itself.
import { BUILTIN_AGENTS } from "../../src/config/agents";
import { SAMPLE } from "../../src/config/config";
import { TARGETS } from "../../src/integrations/targets";
import { c, code, kbd, p, table, ul, type Page } from "./html";

const agents = BUILTIN_AGENTS.filter((a) => a.id !== "generic");
const integrations = (kind: string) => TARGETS.filter((t) => t.kind === kind).map((t) => t.name).join(", ");

export const reference: Page[] = [
  {
    slug: "agents", group: "Agents", title: "Agents & detection",
    description: "Which agents shepherd knows, the states it tracks, and how it reads them.",
    sections: [
      { id: "known", title: `${agents.length} agents, no setup`, html: p(`Shepherd recognises these by their process, including when they're started through node, bun, python or a shell script:`) +
        `<ul class="agent-list">${agents.map((a) => `<li><span>${a.name}</span><code>${a.launch}</code></li>`).join("")}</ul>` },
      { id: "states", title: "States", html: table(["", "State", "Meaning"], [
        ["!", "needs you", "A dialog, question or permission prompt is waiting on you."],
        ["◆", "working", "The agent is busy."],
        ["✓", "done", "It finished while you weren't looking. Focusing it clears it."],
        ["○", "idle", "Waiting for a prompt."],
      ]) + p("State shows in the pane border, the tab bar and the sidebar. The status row counts who's working and who needs you; clicking either lists just those agents.") },
      { id: "how", title: "How it reads state", html: p(
        `Each agent has a manifest of rules over the live bottom of its screen, its terminal title (Claude, for one, animates a spinner there while it works) and its OSC 9;4 progress. Rules have priorities, regions (the last N lines, the prompt box, the text after the last horizontal rule, …) and nested all / any / not conditions. A screen that looks idle only mid-animation waits one more look before it counts.`,
        `Integrations can add more — see <a href="../integrations/">Integrations</a>.`,
      ) },
      { id: "custom", title: "Add or tune an agent", html: p(`A TOML file in ${c("~/.config/shepherd/adapters/<id>.toml")} adds an agent or overrides a built-in one:`) + code("toml", `name = "My Agent"
process = ["my-agent"]
launch = "my-agent"
resume = "my-agent --continue"
resumeSession = "my-agent --resume {id}"

[[rules]]
id = "approval"
state = "blocked"
priority = 900
region = "bottom_non_empty_lines(6)"
contains = ["allow this command?"]

[[rules]]
id = "busy"
state = "working"
priority = 500
region = "bottom_non_empty_lines(3)"
regex = ['(?i)esc to interrupt']`) + p(`${c("shepherd debug detect <pane>")} shows which rule matched, whether an integration is reporting, and the title and progress it saw.`) },
    ],
  },
  {
    slug: "integrations", group: "Agents", title: "Integrations",
    description: "Agents' own hooks and plugins: exact session resume, and exact state where the hooks see everything.",
    sections: [
      { id: "kinds", title: "Two kinds", html: table(["Kind", "Agents", "What it adds"], [
        ["session", integrations("session"), "The agent reports its session id, so a restart resumes that exact conversation. State stays on screen detection — these agents' hooks miss some transitions (interrupts, permission answers)."],
        ["state + session", integrations("lifecycle"), "The hooks or plugin see every transition, so while they report they decide the pane's state. A dialog visibly waiting on you still wins."],
      ]) },
      { id: "install", title: "Install", html: code("sh", `shepherd integration status          # installed, update available, available, not found
shepherd integration install all     # everything recommended for the agents on this machine
shepherd integration install codex   # or one
shepherd integration uninstall codex # takes out only shepherd's entries`) + p(`Or ${kbd("Ctrl+B")} ${kbd("s")} → integrations. Each one goes into the agent's own config, in its own format, next to your settings; an agent's config directory has to exist (run it once). Installs happen where the server runs, which is where the agents are.`) },
      { id: "yours", title: "Report from your own tools", html: code("sh", `shepherd report --source my-tool --agent my-agent --state working
shepherd report --source my-tool --state blocked --seq 42   # stale seqs are ignored
shepherd report --source my-tool --session-id abc123        # resume this session after a restart
shepherd report --source my-tool --release                  # hand the pane back to screen detection`) + p(`Run it inside a pane (it reads ${c("SHEPHERD_PANE_ID")}) or name the pane first. Reports without ${c("--source")} don't change state.`) },
    ],
  },
  {
    slug: "messaging", group: "Agents", title: "Agents talking to agents",
    description: "Send messages between agents, safely.",
    sections: [
      { id: "send", title: "Send and read", html: code("sh", `shepherd send @reviewer "fixed, please re-check"   # typed into the agent once it's idle
shepherd inbox                                     # messages sent to you
shepherd messages --follow                          # every message, live`) + p("A message waits until its recipient is idle, then is typed in, tagged with the sender so it can reply.") },
      { id: "safety", title: "Guard rails", html: ul([
        `Every message carries a hop count; chains stop after ${c("max_hops")} (10).`,
        `Each sender → recipient pair is rate-limited (${c("per_minute")}, 5).`,
        `${kbd("Ctrl+B")} ${kbd("m")} pauses all delivery.`,
        `When an agent types into or closes a pane it didn't create, you get an allow / always / deny prompt; the policy lives in ${c("[permissions]")}.`,
      ]) },
    ],
  },
  {
    slug: "cli", group: "Automation", title: "CLI & API",
    description: "Everything the TUI can do, as shell commands any agent can run.",
    sections: [
      { id: "env", title: "From inside a pane", html: p(`Every pane gets ${c("SHEPHERD_SOCKET")} and ${c("SHEPHERD_PANE_ID")}, so anything that can run a command can drive the workspace — and targets default to the calling pane.`) + code("sh", `shepherd pane split --name tests "pnpm test"     # opens next to the caller
shepherd wait tests --exited                     # prints "exited <code>"
shepherd pane read tests --lines 80
shepherd agent spawn codex --name reviewer --prompt "review src/auth"
shepherd wait @reviewer --state idle
shepherd events --follow`) },
      { id: "commands", title: "Commands", html: table(["Command", "Does"], [
        [c("pane list | split | run | read | keys | close | rename | focus"), "Panes. Keys are text or names: Enter, C-c, M-x, Up, Escape…"],
        [c("agent spawn <harness> | agent list"), "Start and list agents."],
        [c("wait <target> --exited | --state s | --match re"), "Block until a process exits, an agent reaches a state, or output matches."],
        [c("send | inbox | messages | pause"), "Agent messaging."],
        [c("workspace create | list | rename | close"), "Spaces."],
        [c("tab create"), "A new tab, optionally running a command."],
        [c("events [--follow] [--output]"), "The session's event stream."],
        [c("report"), "State and session reports from integrations."],
        [c("debug detect <target>"), "What detection sees for a pane."],
      ]) + p(`Targets are a pane id (${c("p3")}), ${c("@name")} or a name. ${c("shepherd help")} lists everything. Under the hood it's JSON-RPC 2.0 over the session's unix socket.`) },
    ],
  },
  {
    slug: "skill", group: "Automation", title: "Skill",
    description: "A SKILL.md that teaches an agent the CLI, so it drives the session on its own.",
    sections: [
      { id: "what", title: "What it is", html: p(`Installing an integration also drops in the shepherd skill: a ${c("SKILL.md")} covering splitting panes, running commands in them, reading output, spawning and messaging other agents, and waiting on them. The agent loads it only when the task calls for it, so it costs nothing the rest of the time, and it works through the same ${c("shepherd")} binary that is already on every pane's PATH — there is no server to run and no tool slots to spend.`) },
      { id: "install", title: "Install it", html: p(`There's no separate command — the skill comes with the integration:`) + code("sh", `shepherd integration install claude     # hooks + the skill, for one agent
shepherd integration install all        # …for every agent on this machine
shepherd integration status             # "↻ update available" when a newer skill ships
shepherd integration uninstall claude   # takes the skill back out too`) + p(`Restart a running agent to pick it up.`) },
      { id: "where", title: "Where it goes", html: code("text", `~/.agents/skills/shepherd/SKILL.md          the one copy
~/.claude/skills/shepherd  -> …/.agents/skills/shepherd
~/.codex/skills/shepherd   -> …/.agents/skills/shepherd`) + p(`One copy, symlinked into each agent that reads skills from a directory of its own. Every agent with an integration gets it except MastraCode and OMP, whose skills directories aren't established; Kimi Code reads ${c("~/.agents/skills")} itself, so it needs no link.`) },
    ],
  },
  {
    slug: "plugins", group: "Automation", title: "Plugins",
    description: "Any program, started with the session and given its socket.",
    sections: [
      { id: "config", title: "Declare one", html: code("toml", `[[plugin]]\nrun = "bun ~/code/watcher/plugin.ts"`) + p(
        `Plugins start with the session server and get ${c("SHEPHERD_SOCKET")} — the session's unix socket, which is the whole API — plus ${c("SHEPHERD_SESSION")}. No SDK and no manifest: anything that speaks newline-delimited JSON-RPC qualifies. Edit the config, then ${c("shepherd restart")}.`,
        `${c("run")} goes through a <strong>login</strong> shell, so your profile is sourced first and can rewrite ${c("PATH")}. Use absolute paths.`,
      ) },
      { id: "not-a-pane", title: "A plugin is not a pane", html: p(
        `${c("SHEPHERD_PANE_ID")} is deliberately unset, so commands that default to "the calling pane" have no default — always pass a ${c("target")}. And you act with the <strong>user's</strong> authority: the permission prompts that gate an agent typing into or closing a pane it didn't create do not apply to you.`,
      ) },
      { id: "events", title: "Events", html: p(`Call ${c("events.subscribe")} once, then read notifications. Every event carries ${c("type")} and ${c("at")}.`) + table(["type", "Extra fields"], [
        [c("pane.created"), `${c("pane")}, ${c("name")}, ${c("command")}`],
        [c("process.exited"), `${c("pane")}, ${c("name")}, ${c("exitCode")}`],
        [c("agent.state"), `${c("pane")}, ${c("name")}, ${c("harness")}, ${c("from")}, ${c("to")}`],
        [c("message.sent"), `${c("id")}, ${c("from")}, ${c("to")}, ${c("hops")}`],
        [c("message.delivered"), `${c("id")}, ${c("from")}, ${c("to")}`],
        [c("client.attached"), "—"],
        [c("pane.output"), `${c("pane")}, ${c("text")} — opt in with ${c("{ output: true }")}; it is every byte of every pane`],
      ]) + p(`<strong>Events are not replayed.</strong> You get what happens after your subscription lands. Plugins start while the server is still booting, so subscribe first and then call ${c("list")} if you need the state of the world.`) },
      { id: "shell", title: "The cheap way", html: p("No socket client needed — the CLI reads the same variable:") + code("sh", `#!/bin/sh
shepherd events --follow | while read -r line; do
  echo "$line" | grep -q '"type":"agent.state".*"to":"blocked"' && say "an agent needs you"
done`) },
      { id: "example", title: "A worked example", html: p(
        `<a href="https://github.com/manyeya/shepherd/tree/main/examples/plugins">examples/plugins/</a> is the full guide: the method list, the JSON-RPC error codes, and the rules that will bite you. <code>blocked-notifier/plugin.ts</code> there is a complete plugin — it announces any agent that gets blocked, with the question it is stuck on.`,
        `Remember that plugins run where the <em>server</em> runs. With ${c("--remote")} that is the remote machine, so a desktop notification pops up there.`,
      ) },
    ],
  },
  {
    slug: "config", group: "Reference", title: "Configuration",
    description: "config.toml, with every setting and its default.",
    sections: [
      { id: "where", title: "Where it lives", html: p(`${c("~/.config/shepherd/config.toml")}. The settings page edits it for you; ${c("shepherd config edit")} opens it in your editor. Changes apply live.`) },
      { id: "reference", title: "Every setting", html: code("toml", SAMPLE) },
    ],
  },
  {
    slug: "updating", group: "Reference", title: "Updating",
    description: "Releases, the update badge, and the staging channel.",
    sections: [
      { id: "badge", title: "The update badge", html: p(`When a newer release is out, the status row shows ${c("↑ 0.x.y")}. Click it (or run "Update shepherd" from the palette) to download it and restart the server; your agents come back where they were. Shepherd checks at most every six hours.`) },
      { id: "cli", title: "From the command line", html: code("sh", `shepherd version     # this version, and whether a newer one is out
shepherd update      # download, verify its SHA-256, replace this binary
shepherd restart     # load it into running sessions`) },
      { id: "managed", title: "Installed with a package manager", html: p(`A package manager owns the binary it installed, so ${c("shepherd update")} and the update badge give you that tool's command instead (${c("mise upgrade github:manyeya/shepherd")}, or ${c("apt")} or ${c("dnf")} for the .deb and .rpm packages). Run ${c("shepherd restart")} afterwards. Agents' hooks call the ${c("shepherd")} on your ${c("PATH")}, so they keep working across upgrades.`) },
      { id: "verify", title: "Verifying a release", html: p(`Every release file has signed build provenance. ${c("gh attestation verify <file> -R manyeya/shepherd")} confirms that this repository's release workflow built the file from a tagged commit.`) },
      { id: "channels", title: "Channels", html: p(`${c("stable")} follows releases. ${c("staging")} follows prereleases built from the staging branch. Set it in ${c("[update] channel")}, or install from it with ${c("SHEPHERD_CHANNEL=staging")}. ${c("[update] check = false")} turns the check off.`) },
    ],
  },
  {
    slug: "troubleshooting", group: "Reference", title: "Troubleshooting",
    description: "When something doesn't look right.",
    sections: [
      { id: "state", title: "An agent shows the wrong state", html: p(`Run ${c("shepherd debug detect <pane>")} to see the rule that matched and the screen it read. A new agent version may draw something the rules don't know yet: override the agent's rules in ${c("~/.config/shepherd/adapters/")}.`) },
      { id: "stale", title: "Something changed but nothing's different", html: p(`A running session keeps the shepherd it started with. ${c("shepherd restart")} loads the new one (agents resume), then detach and reattach to refresh the client too.`) },
      { id: "logs", title: "Logs", html: p(`Each session's server logs to ${c("~/.local/state/shepherd/<session>.log")}.`) },
      { id: "outdated", title: "An integration says “update available”", html: p(`It was installed by an older shepherd, or shepherd has moved. ${c("shepherd integration install all")} brings every one up to date.`) },
      { id: "sandbox", title: "An agent in a sandbox can't reach shepherd", html:
        p(`Some agents run their commands in a sandbox that blocks shepherd's socket. Their ${c("shepherd")} commands then fail with “can't connect to its socket: a sandbox is blocking it”. You can still message that agent, but it can't ${c("send")}, read panes or spawn agents.`) +
        p(`<strong>Codex on macOS</strong> blocks Unix sockets unless its commands have network access. Give them a permissions profile that allows it in ${c("~/.codex/config.toml")}, then restart Codex:`) +
        code("toml", `default_permissions = "shepherd"

[permissions.shepherd]
extends = ":workspace"

[permissions.shepherd.network]
enabled = true`) +
        p(`This also lets Codex's commands reach the internet. Codex rejects a config that sets both this and ${c("sandbox_mode")}, so use one or the other.`) },
      { id: "sound", title: "No sound", html: p("Check the sound section of the settings page and your output device. Sounds are generated and played by shepherd itself, so nothing else needs installing.") },
    ],
  },
];

