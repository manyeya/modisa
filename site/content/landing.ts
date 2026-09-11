// The home page. The hero is a live HTML rendering of a shepherd session (animated by landing.js),
// not a screenshot: agents working, one waiting on you, one done.
import { BUILTIN_AGENTS } from "../../src/config/agents";
import { escape } from "./html";
import { icon } from "./icons";

export const INSTALL = "curl -fsSL https://manyeya.github.io/shepherd/install.sh | sh";
export const mark = (_base = "./") => `<span class="mark" aria-hidden="true"></span>`;

const agentNames = BUILTIN_AGENTS.filter((a) => a.id !== "generic").map((a) => a.name);
const copyButton = (text: string, label = "Copy") => `<button type="button" class="copy" aria-label="Copy command" data-copy-text="${escape(text)}">${label}</button>`;

// One pane of the demo session. Its body lines are what the agent has drawn.
const pane = (id: string, name: string, agent: string, state: string, lines: string[]) => `
  <div class="pane" data-pane="${id}" data-state="${state}">
    <div class="pane-title"><span class="pane-glyph"></span><b>@${name}</b><span class="pane-agent">${agent}</span><span class="pane-state"></span></div>
    <div class="pane-body">${lines.join("")}</div>
  </div>`;

const demo = `
<figure class="demo" id="workspace" aria-label="A shepherd session: four agents in panes, one waiting on you">
  <div class="demo-topline"><span><span class="status-dot"></span> YOUR WORKSPACE</span><span>EXAMPLE SESSION <span class="demo-slash">/</span> 04 PANES</span></div>
  <div class="tui" data-demo>
    <div class="tui-tabs"><span class="tui-space">◈ api ▸</span><span class="tui-tab on">1:@coder</span><span class="tui-tab" data-tab-blocked>2:@reviewer <i>!</i></span><span class="tui-tab">3:tests</span><span class="tui-plus">+</span></div>
    <div class="tui-main">
      <aside class="tui-side">
        <p class="tui-label">SPACES</p>
        <p class="tui-row sel">api</p><p class="tui-row">web</p><p class="tui-row">infra</p>
        <p class="tui-label">AGENTS / 3</p>
        <div class="tui-agents" data-agents><div class="tui-agent"><span class="amber">!</span> <b>@reviewer</b><small>codex · needs you</small></div><div class="tui-agent focus"><span>◆</span> <b>@coder</b><small>claude-code · working</small></div><div class="tui-agent"><span>✓</span> <b>@docs</b><small>opencode · done</small></div></div>
        <p class="tui-foot">⚙ settings</p>
      </aside>
      <div class="tui-panes">
        ${pane("coder", "coder", "claude-code", "working", [
          `<p class="dim">❯ refactor the session middleware</p>`,
          `<p><span class="ok">●</span> Read <u>src/auth/session.ts</u></p>`,
          `<p><span class="ok">●</span> Update <u>src/auth/cookies.ts</u> <span class="dim">+38 −12</span></p>`,
          `<p class="stream" data-stream></p>`,
          `<p class="spin"><span data-spinner>✻</span> Refactoring… <span class="dim">(esc to interrupt)</span></p>`,
        ])}
        ${pane("reviewer", "reviewer", "codex", "blocked", [
          `<p class="dim">› review the auth changes before pushing</p>`,
          `<p>• Ran <u>pnpm lint</u> <span class="dim">— clean</span></p>`,
          `<div class="ask"><p><b>Allow command?</b></p><p class="cmd">git push origin feature/auth</p><p><span class="amber">›</span> 1. Yes, run it</p><p class="dim">  2. No, tell Codex what to do</p><p class="dim">Press enter to confirm or esc to cancel</p></div>`,
        ])}
        ${pane("tests", "tests", "shell", "idle", [
          `<p><span class="dim">~/api $</span> pnpm test --watch</p>`,
          `<p><span class="ok">✓</span> auth/session <span class="dim">(41)</span></p>`,
          `<p><span class="ok">✓</span> auth/cookies <span class="dim">(19)</span></p>`,
          `<p><span class="ok">✓</span> <b>214 passed</b> <span class="dim">in 3.8s</span></p>`,
        ])}
        ${pane("docs", "docs", "opencode", "done", [
          `<p class="dim">> write the migration notes</p>`,
          `<p>Wrote <u>docs/migrating.md</u> — sessions now rotate on login.</p>`,
          `<p class="dim">> _</p>`,
        ])}
      </div>
    </div>
    <div class="tui-status"><span class="seg on">◧ sidebar</span><span class="seg acc">+ agent</span><span class="seg working" data-count-working>◆ 1 working</span><span class="seg blocked" data-count-blocked>! 1 need you</span><span class="spacer"></span><span class="seg">4 panes</span><span class="seg theme">◐ nightwatch</span></div>
    <div class="tui-toast" data-toast aria-live="off"></div>
  </div>
  <figcaption><span><span class="caption-dot"></span><span data-caption>@reviewer is waiting on you.</span></span><button type="button" data-demo-pause aria-pressed="false">Pause demo ${icon("pause-linear")}</button></figcaption>
</figure>`;


const storyVisuals = [
  `<div class="story-screen attention-screen"><div class="screen-header"><span>AGENT ACTIVITY</span><span>03 CONNECTED</span></div><div class="activity-row needs"><span class="activity-symbol">!</span><div><strong>@reviewer</strong><small>codex</small></div><span class="activity-state">Needs you<span class="status-dot"></span></span></div><div class="activity-row"><span class="activity-symbol">↗</span><div><strong>@coder</strong><small>claude-code</small></div><span class="activity-state">Working</span></div><div class="activity-row"><span class="activity-symbol">✓</span><div><strong>@docs</strong><small>opencode</small></div><span class="activity-state">Done</span></div><div class="permission"><span>FROM @reviewer</span><p>Ready to push the changes.<br>May I continue?</p><div><span class="permission-option">↵ Allow command</span><span>esc Cancel</span></div></div><div class="screen-bottom"><span class="status-dot"></span> One agent needs your attention.</div></div>`,
  `<div class="story-screen layout-screen"><div class="screen-header"><span>WORKSPACE / API</span><span>03 PANES</span></div><div class="split-demo"><div class="build-pane"><span>01 / @coder</span><strong>claude</strong><p>Refactoring session middleware…</p><div class="code-strokes" aria-hidden="true"><i></i><i></i><i></i><i></i></div></div><div><span>02 / @reviewer</span><strong>codex</strong><p>Reviewing changes.</p></div><div><span>03 / @tests</span><strong>bun test</strong><p>214 passed.</p></div></div><div class="screen-bottom"><kbd>Ctrl+B</kbd><span>One prefix. Every pane.</span></div></div>`,
  `<div class="story-screen resume-screen"><div class="screen-header"><span>SESSION / API</span><span>RUNNING</span></div><div class="session-command"><span>$</span> shepherd detach</div><div class="session-track"><span>Terminal closed</span><span class="session-track-line"></span><span class="session-live"><span class="status-dot"></span> Server running</span></div><div class="session-command"><span>$</span> shepherd attach</div><div class="restored"><span>${icon("check-circle-linear")}</span><div><strong>Welcome back.</strong><p>Same panes. Same conversations.</p></div></div><div class="screen-bottom">Your session has a life beyond the window.</div></div>`,
];
const stories = [
  { label: "01 / ATTENTION", title: "Know when<br>you’re needed.", text: "A permission prompt. A question. A finished task. Shepherd reads the terminal and brings the agent that needs you to the top.", detail: "Follow the signal in the pane, sidebar, or a notification. Connect agent hooks for richer state detection.", link: "How detection works", url: "docs/agents/" },
  { label: "02 / COORDINATION", title: "A place for<br>every agent.", text: "Spaces for projects. Tabs for context. Real terminal panes, arranged your way—with every agent’s full interface intact.", detail: "Agents can spawn a reviewer, wait for tests, and message another pane through shell commands or MCP.", link: "Explore the workflow", url: "docs/quick-start/" },
  { label: "03 / CONTINUITY", title: "Leave the window.<br>Keep the work.", text: "A background server owns your panes. Detach, reattach, or connect over SSH. Your running session stays with you.", detail: "After a reboot, restore your layout and resume supported agents in their previous conversations.", link: "Read the documentation", url: "docs/introduction/" },
];

export function landing(o: { version: string; repo: string }) {
  return `<header class="top landing-top">
  <a class="brand" href="./" aria-label="Shepherd home">${mark()}<span>shepherd</span></a>
  <nav class="desktop-nav" aria-label="Main"><a href="#workflow">Workflow</a><a href="docs/introduction/">Documentation</a><a href="${o.repo}">GitHub ${icon("arrow-right-up-linear")}</a></nav>
  <a class="nav-install" href="#start">Get shepherd ${icon("arrow-right-up-linear")}</a>
  <button class="mobile-menu-toggle" type="button" aria-label="Open navigation" aria-expanded="false" aria-controls="mobile-nav" data-nav-toggle>${icon("hamburger-menu-linear")}</button>
  <nav class="mobile-nav" id="mobile-nav" aria-label="Mobile navigation" hidden><a href="#workflow">Workflow</a><a href="docs/introduction/">Documentation</a><a href="${o.repo}">GitHub</a><a href="#start">Install shepherd</a></nav>
</header>
<main id="main">
<section class="hero">
  <div class="hero-meta"><p class="eyebrow"><span class="status-dot"></span> A TERMINAL FOR CODING AGENTS</p><a class="release" href="${o.repo}/releases">v${escape(o.version)} <span> / </span> OPEN SOURCE ${icon("arrow-right-up-linear")}</a></div>
  <div class="hero-heading"><h1 data-reveal>Your agents.<br><span class="headline-muted">Under control.</span></h1><div class="hero-copy"><p>Claude Code. Codex. Your whole crew.<br>Run them side by side, and know<br class="desktop-break"> exactly who needs you.</p><div class="hero-actions"><a class="button primary" href="#start">Install shepherd ${icon("arrow-right-up-linear")}</a><a class="quiet-link" href="docs/introduction/">Read the docs ${icon("arrow-right-linear")}</a></div><p class="platforms">macOS &amp; Linux <span>·</span> Free &amp; open source</p></div></div>
  ${demo}
  <div class="hero-bottom"><span>Your tools. Your shell. One terminal.</span><a href="#workflow">Find your flow ${icon("arrow-down-linear")}</a></div>
</section>
<section class="workflow" id="workflow"><div class="workflow-heading"><p class="eyebrow">BUILT AROUND YOUR ATTENTION</p><h2 data-reveal>Stay with the work.</h2><p>Shepherd keeps the moving parts in view.<br>You decide where to go next.</p></div><div class="story-layout"><div class="story-copy">${stories.map((story, i) => `<article class="story" data-story="${i}"><p class="eyebrow">${story.label}</p><h3 data-reveal>${story.title}</h3><p class="story-lede">${story.text}</p><p class="story-detail">${story.detail}</p><a class="text-link" href="${story.url}">${story.link} ${icon("arrow-right-linear")}</a><div class="mobile-visual">${storyVisuals[i]}</div></article>`).join("")}</div><div class="story-stage" aria-hidden="true"><div class="stage-label"><span>THE WORKSPACE, IN PRACTICE</span><span data-story-count>01 / 03</span></div>${storyVisuals.map((visual,i) => `<div class="story-panel" data-panel="${i}">${visual}</div>`).join("")}<div class="stage-progress"><i class="on"></i><i></i><i></i></div></div></div></section>
<section class="compatibility"><p class="eyebrow">ALREADY SPEAKS THEIR LANGUAGE</p><div><p>Claude Code<span>/</span>Codex<span>/</span>Gemini CLI<span>/</span>OpenCode</p><a class="text-link" href="docs/agents/">All ${agentNames.length} supported agents ${icon("arrow-right-linear")}</a></div></section>
<section class="start" id="start"><div class="start-heading"><p class="eyebrow">YOUR NEXT SESSION</p><h2 data-reveal>Bring everyone<br>together.</h2><p>One install. Then get back to building.</p><a class="text-link" href="docs/quick-start/">Take the five-minute tour ${icon("arrow-right-linear")}</a></div><ol class="install-steps"><li><div class="step-title"><span>01</span><h3>Install shepherd</h3><small>macOS &amp; Linux</small></div><div class="install"><span class="prompt">$</span><code>${escape(INSTALL)}</code>${copyButton(INSTALL)}</div></li><li><div class="step-title"><span>02</span><h3>Open a session</h3></div><div class="install"><span class="prompt">$</span><code>shepherd</code>${copyButton("shepherd")}</div></li><li><div class="step-title"><span>03</span><h3>Bring your agents</h3></div><p>Press <kbd>Ctrl+B</kbd> then <kbd>a</kbd> to pick an agent.<br>Or run <code>claude</code> in any pane.</p></li></ol></section>
<section class="closing"><span class="closing-title">Your terminal. A little more together.</span><a href="${o.repo}">Build with us on GitHub ${icon("arrow-right-up-linear")}</a></section>
</main>`;
}
