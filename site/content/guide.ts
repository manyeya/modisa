// Docs: getting started and everyday use.
import { c, code, kbd, note, ol, p, table, ul, type Page } from "./html";

const INSTALL = "curl -fsSL https://manyeya.github.io/modisa/install.sh | sh";

export const guide: Page[] = [
  {
    slug: "introduction", group: "Start", title: "Introduction",
    description: "A terminal multiplexer for running many coding agents at once, and always knowing which one needs you.",
    sections: [
      { id: "what", title: "What modisa is", html: p(
        "Modisa is a terminal multiplexer, like tmux, built for coding agents. It has no AI of its own. Claude Code, Codex, OpenCode, Pi, Gemini and 20 other agents run in real terminal panes next to your shells, exactly as they would anywhere else.",
        "What modisa adds is attention. It recognises every agent it hosts, reads its state, and tells you the moment one is <strong>blocked on you</strong>, is <strong>working</strong>, or has <strong>finished</strong> while you were looking elsewhere — in the pane border, the tab bar, the sidebar, and with a toast, a system notification or a sound.",
      ) },
      { id: "shape", title: "How it's shaped", html: ul([
        "<strong>Sessions</strong> are owned by a background server, so closing your terminal only detaches. Reattach from anywhere, including over ssh.",
        "<strong>Spaces</strong> group tabs; <strong>tabs</strong> hold splits of <strong>panes</strong>. Everything works with the keyboard and the mouse.",
        "<strong>Agents talk to each other.</strong> Any pane can split panes, spawn agents, wait on them and message them through the CLI, which the modisa skill teaches them.",
        "<strong>Restarts are cheap.</strong> Layouts, names and agents are saved; agents come back in the exact conversation they were in.",
      ]) },
      { id: "next", title: "Where to go next", html: p(`<a href="../install/">Install modisa</a>, then take the <a href="../quick-start/">five-minute tour</a>.`) },
    ],
  },
  {
    slug: "install", group: "Start", title: "Install",
    description: "One command on macOS and Linux. Or run it from source.",
    sections: [
      { id: "script", title: "Install script", html:
        code("sh", INSTALL) +
        p(`It downloads the release for your platform, checks it against the published SHA-256, and puts ${c("modisa")} in ${c("~/.local/bin")}. Releases exist for <strong>macOS on Apple silicon</strong> and <strong>Linux on x64 and arm64</strong> (glibc).`) +
        table(["Variable", "Effect"], [
          [c("MODISA_INSTALL_DIR"), `Where the binary goes (default ${c("~/.local/bin")}).`],
          [c("MODISA_CHANNEL=staging"), "Install the latest prerelease instead of the latest release."],
        ]) },
      { id: "managers", title: "With a package manager", html:
        p("A package manager installs modisa, updates it, and removes it again.") +
        table(["With", "Install", "Remove"], [
          ["mise", c("mise use -g github:manyeya/modisa"), c("mise uninstall github:manyeya/modisa")],
          ["Debian, Ubuntu", c("sudo apt install ./modisa_<version>_<arch>.deb"), c("sudo apt remove modisa")],
          ["Fedora, RHEL", c("sudo dnf install ./modisa-<version>-1.<arch>.rpm"), c("sudo dnf remove modisa")],
        ]) +
        p(`The .deb and .rpm files are attached to every release. When one of these tools installed modisa, ${c("modisa update")} tells you its update command rather than replacing the binary itself.`) },
      { id: "releases", title: "From a release", html: p(`Every release on <a href="https://github.com/manyeya/modisa/releases">GitHub Releases</a> carries one binary per platform, the .deb and .rpm packages, a ${c("SHA256SUMS")} file, and the ${c("manifest.json")} that the installer and ${c("modisa update")} read. Download the binary for your platform, ${c("chmod +x")} it, and put it on your ${c("PATH")}.`) },
      { id: "verify", title: "Verify a download", html: p(`Every file in a release has signed build provenance: proof that the repository's release workflow built it from a tagged commit. With the GitHub CLI:`) + code("sh", "gh attestation verify modisa-darwin-arm64 -R manyeya/modisa") + p("mise checks this automatically when it installs modisa.") },
      { id: "source", title: "From source", html: p(`Needs <a href="https://bun.sh">Bun</a> 1.3.5 or newer (for its built-in PTY). This is also the way to run modisa on an Intel Mac.`) + code("sh", "git clone https://github.com/manyeya/modisa\ncd modisa\nbun install\nbun start") },
      { id: "check", title: "Check it", html: code("sh", "modisa --version") + p(`Then run ${c("modisa")} to open your first session.`) },
      { id: "uninstall", title: "Uninstall", html:
        code("sh", `modisa uninstall
# or, through the install script
curl -fsSL https://manyeya.github.io/modisa/install.sh | sh -s -- --uninstall`) +
        p(`It lists what it will do and asks first. Then it removes modisa's hooks and skill from every agent, stops running sessions, and deletes saved state in ${c("~/.local/state/modisa")}. Your config in ${c("~/.config/modisa")} stays unless you add ${c("--purge")}; ${c("--yes")} skips the question.`) +
        p(`If the install script put the binary there, uninstall deletes it too. If mise or a package did, finish with that tool's remove command, which modisa prints. Run it from a terminal outside modisa, since it stops every session.`) },
    ],
  },
  {
    slug: "quick-start", group: "Start", title: "Quick start",
    description: "Open a session, start two agents, and watch modisa tell you which one needs you.",
    sections: [
      { id: "open", title: "Open a session", html: code("sh", "modisa") + p(`That attaches the <em>default</em> session, starting its server if needed. You get a shell in a pane, the sidebar on the left (spaces and agents) and the status row at the bottom. Every command starts with the prefix ${kbd("Ctrl+B")}.`) },
      { id: "agents", title: "Start some agents", html: ol([
        `${kbd("Ctrl+B")} ${kbd("a")} opens the agent picker. Pick one: it starts in a new pane.`,
        `Or split with ${kbd("Ctrl+B")} ${kbd("v")} and run ${c("claude")}, ${c("codex")} or any other agent by hand. Modisa recognises it from its process either way.`,
        "Give each one a task. Their state shows in the sidebar: ◆ working, ! needs you, ✓ done.",
      ]) },
      { id: "attention", title: "Let it find you", html: p(
        "When an agent you aren't looking at stops to ask something, its pane border and tab turn to the blocked colour, the status row counts <strong>! 1 need you</strong>, and you get a toast, a system notification and a sound. Click the counter, or press the agent's number with <kbd>Ctrl+B</kbd> <kbd>1</kbd>–<kbd>9</kbd>, to jump straight there.",
      ) },
      { id: "leave", title: "Leave and come back", html: p(`${kbd("Ctrl+B")} ${kbd("d")} detaches; everything keeps running. Run ${c("modisa")} again (here or over ssh) to pick up where you were.`) },
      { id: "layout-file", title: "Save a layout", html: p(`A ${c("modisa.toml")} in the directory you start from lays out a new session:`) + code("toml", `[[pane]]\nname = "server"\nrun = "pnpm dev"\n\n[[pane]]\nname = "coder"\nagent = "claude-code"\nprompt = "read TODO.md and start on the first item"`) },
    ],
  },
  {
    slug: "keys-and-mouse", group: "Using modisa", title: "Keys & mouse",
    description: "Every binding behind the prefix, and everything the mouse can do.",
    sections: [
      { id: "prefix", title: "Prefix bindings", html: p(`Press ${kbd("Ctrl+B")}, then:`) + table(["Key", "Action"], [
        [`${kbd("v")} / ${kbd("%")}`, "split right"], [`${kbd("-")} / ${kbd('"')}`, "split down"],
        [`${kbd("h")} ${kbd("j")} ${kbd("k")} ${kbd("l")} / arrows`, "focus pane"], [`${kbd("H")} ${kbd("J")} ${kbd("K")} ${kbd("L")}`, "resize pane"],
        [kbd("z"), "zoom pane"], [kbd("x"), "close pane"], [kbd("c"), "new tab"], [`${kbd("n")} / ${kbd("p")}`, "next / previous tab"],
        [`${kbd("w")} / ${kbd("W")}`, "switch space / new named space"], [`${kbd("$")} / ${kbd("&")}`, "rename / delete the current space"],
        [kbd("b"), "hide / show the sidebar"], [`${kbd("s")} / ${kbd("t")}`, "settings / settings on the theme section"],
        [`${kbd("o")} / ${kbd("1")}–${kbd("9")}`, "pane picker / jump to agent"], [kbd("e"), "pane context menu"], [kbd("a"), "launch an agent"],
        [`${kbd(":")} / ${kbd("?")}`, "command palette / keyboard guide"], [kbd("["), "copy mode"], [kbd("/"), "search"],
        [kbd("m"), "pause / resume agent messaging"], [kbd("Ctrl+B"), "send a literal Ctrl+B"], [kbd("d"), "detach (panes keep running)"],
      ]) },
      { id: "mouse", title: "Mouse", html: ul([
        "Click a pane to focus it; scroll for its scrollback.",
        "Drag the border between two panes to resize them — the pointer turns into a move cursor over a border.",
        "Drag the sidebar's edge to make it wider or narrower; the width is kept for next time.",
        "Click a tab to switch to it; click a space in the sidebar to switch spaces.",
        "Right-click a pane or a tab for split, zoom, rename, copy visible output, search, theme, sidebar and close.",
        "Menus take arrows, Enter and Escape, and close when you click outside them.",
      ]) + p("Copying uses OSC 52, so it reaches your clipboard even over ssh in terminals that support it.") },
      { id: "palette", title: "Command palette", html: p(`${kbd("Ctrl+B")} ${kbd(":")} lists every action by name, including the ones without a key: restart the server, edit config.toml, update modisa, message log, and more.`) },
    ],
  },
  {
    slug: "spaces-tabs-panes", group: "Using modisa", title: "Spaces, tabs & panes",
    description: "How a session is organised, and how it adapts to small terminals.",
    sections: [
      { id: "model", title: "The model", html: ul([
        "A <strong>session</strong> holds <strong>spaces</strong>; a space holds <strong>tabs</strong>; a tab is a tree of split <strong>panes</strong>.",
        "Each pane is a real terminal: a shell, a command, or an agent.",
        "The sidebar lists spaces and the agents in the current one, with whoever needs you first. Drag its edge to make it wider or narrower (20 to 48 columns, at most a third of the terminal); the width is saved as <code>[sidebar] width</code>, and <code>[sidebar] agents</code> can hand the agent list to a plugin such as <a href=\"https://github.com/manyeya/modisa-radar\">radar</a>.",
      ]) },
      { id: "spaces", title: "Spaces", html: p(
        `A space is a named group of tabs — one per project, say. ${kbd("Ctrl+B")} ${kbd("W")} creates one; it starts in the current space's working directory. In the sidebar, click a space to switch, double-click its name (or click ✎) to rename it in place, and click ✕ to delete it (it asks first, and closes its panes). The last space can't be deleted.`,
      ) },
      { id: "small", title: "Small terminals", html: p(
        "The sidebar hides below 100 columns or 22 rows. If a split would get too small to use, the focused pane fills the space instead; widen the terminal and the splits come back. Tabs that don't fit scroll, keeping the active one visible.",
      ) },
    ],
  },
  {
    slug: "sessions", group: "Using modisa", title: "Sessions & remote",
    description: "A background server owns every pane: detach, reattach, restart, and work over ssh.",
    sections: [
      { id: "commands", title: "Sessions", html: code("sh", `modisa                          # attach the default session (starts it if needed)
modisa new api --cwd ~/code/api  # a named session
modisa -s api                    # attach it
modisa ls                        # running sessions, plus saved ones you can restore
modisa restart                   # load updated modisa into a running session
modisa kill api`) },
      { id: "restore", title: "Surviving restarts", html: p(
        `Layouts, pane names, working directories and agents are saved in ${c("~/.local/state/modisa/modisa.db")}. After a reboot, attaching restores the session: shells come back in their directories, and agents are relaunched — into the <strong>exact conversation</strong> they were in when their integration reported it (${c("claude --resume <id>")}, ${c("codex resume <id>")}, …), otherwise into their latest one. Plain commands are typed back in but not run.`,
      ) },
      { id: "remote", title: "Remote over ssh", html: code("sh", `modisa --remote ssh://devbox              # host
modisa --remote ssh://me@build-01:2222    # user and port
modisa --remote devbox                    # a Host alias from ~/.ssh/config
modisa -s api --remote ssh://devbox       # a named session on that machine`) + p(
        `The TUI runs here with your keybindings, theme, sounds and notifications; the server and every pane run on the far side. No daemon to install and no port to open — modisa shells out to your own ${c("ssh")}, so agent forwarding, jump hosts and ${c("~/.ssh/config")} all apply.`,
      ) + code("sh", `ssh -T devbox modisa proxy -s default             # what ssh://devbox runs
ssh -T -p 2222 me@build-01 modisa proxy -s api    # …and ssh://me@build-01:2222 with -s api`) + p(
        `${c("modisa proxy")} starts the remote server if it isn't running, then bridges its socket to stdio. ${c("-T")} means no pty: the protocol is JSON, not a terminal.`,
      ) },
      { id: "remote-path", title: "When the remote can't find modisa", html: p(
        `Modisa has to be installed on the remote machine, and on the ${c("PATH")} of a <em>non-interactive</em> ssh shell — which often excludes ${c("~/.local/bin")}, where the installer puts it. Check with ${c("ssh devbox 'command -v modisa'")}. If it comes back empty, set an absolute path in your <strong>local</strong> config:`,
      ) + code("toml", `remote_command = "/home/me/.local/bin/modisa"`) },
      { id: "remote-notes", title: "What runs where", html: ul([
        `Panes, agents, integrations and ${c("[[plugin]]")} programs all run on the remote machine. A plugin that raises a desktop notification raises it there.`,
        `Sessions are per machine: ${c("modisa ls")} lists local ones, ${c("ssh devbox modisa ls")} the remote ones.`,
        `${kbd("Ctrl+B")} ${kbd("d")} drops the ssh connection; the remote session and its agents keep running. Reattach from any machine.`,
        `Copying uses OSC 52, so it reaches your local clipboard through ssh in terminals that support it.`,
        `Updates are per machine — ${c("modisa update")} here doesn't touch the remote install.`,
      ]) },
    ],
  },
  {
    slug: "settings", group: "Using modisa", title: "Settings",
    description: "Themes, indicators, sounds, alerts, pane labels and integrations, from one page.",
    sections: [
      { id: "page", title: "The settings page", html: p(`${kbd("Ctrl+B")} ${kbd("s")}, or ⚙ settings in the sidebar. Tab switches section, ↑↓ or the pointer selects, ←→ changes a value, Enter or a click applies, Esc closes. Every change applies at once and is saved to ${c("~/.config/modisa/config.toml")}, keeping your comments.`) },
      { id: "sections", title: "Sections", html: table(["Section", "What it sets"], [
        ["theme", "Ion, Tokyo Night, Catppuccin Mocha, Gruvbox, Nord, Dracula — previewed live."],
        ["indicators", "The state glyphs (symbols ! ◆ ✓ ○, dots or letters) and where they show: tab badge, pane border, sidebar."],
        ["sound", `What plays when an agent needs you, is done, or starts working — 17 sounds from <a href="https://cuelume.dev">cuelume</a>, or off — and the volume.`],
        ["toasts", "Toast, system notification and terminal bell, per event."],
        ["pane labels", "Agent and state in the border title."],
        ["integrations", `Every agent's integration: installed, update available, available, not found. See <a href="../integrations/">Integrations</a>.`],
      ]) + note("Alerts are for agents you're not looking at", "The focused pane never alerts; a background agent that blocks or finishes does.") },
    ],
  },
];
