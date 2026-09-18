// The home page. The hero is a live HTML rendering of a modisa session (animated by landing.js),
// not a screenshot: agents working, one waiting on you, one done.
import { BUILTIN_AGENTS } from "../../src/config/agents";
import { escape } from "./html";
import { icon } from "./icons";

export const INSTALL = "curl -fsSL https://manyeya.github.io/modisa/install.sh | sh";
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
<figure class="demo" id="workspace" aria-label="A modisa session: four agents in panes, one waiting on you">
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
    <div class="tui-status"><span class="seg on">◧ sidebar</span><span class="seg acc">+ agent</span><span class="seg working" data-count-working>◆ 1 working</span><span class="seg blocked" data-count-blocked>! 1 need you</span><span class="spacer"></span><span class="seg">4 panes</span><span class="seg theme">◐ tokyonight</span></div>
    <div class="tui-toast" data-toast aria-live="off"></div>
  </div>
  <figcaption><span><span class="caption-dot"></span><span data-caption>@reviewer is waiting on you.</span></span><button type="button" data-demo-pause aria-pressed="false">Pause demo ${icon("pause-linear")}</button></figcaption>
</figure>`;


const arrow = icon("arrow-right-linear");
const diagonal = icon("arrow-right-up-linear");

export function landing(o: { version: string; repo: string }) {
  return `<header class="top landing-top">
  <a class="brand" href="./" aria-label="Modisa home">${mark()}<span>modisa<span class="brand-period">.</span></span></a>
  <nav class="desktop-nav" aria-label="Main"><a href="#workflow">The workflow</a><a href="docs/introduction/">Documentation</a><a href="plugins/">Plugins</a><a href="${o.repo}">GitHub ${diagonal}</a></nav>
  <a class="nav-install" href="#start">Get modisa ${arrow}</a>
  <button class="mobile-menu-toggle" type="button" aria-label="Open navigation" aria-expanded="false" aria-controls="mobile-nav" data-nav-toggle>${icon("hamburger-menu-linear")}</button>
  <nav class="mobile-nav" id="mobile-nav" aria-label="Mobile navigation" hidden><a href="#workflow">The workflow</a><a href="docs/introduction/">Documentation</a><a href="plugins/">Plugins</a><a href="${o.repo}">GitHub</a><a href="#start">Get modisa</a></nav>
</header>
<main id="main">
<section class="hero">
  <div class="hero-meta"><p class="eyebrow"><span class="status-dot"></span> THE MULTI-AGENT TERMINAL</p><a class="release" href="${o.repo}/releases">OPEN SOURCE <span> / </span> v${escape(o.version)} ${diagonal}</a></div>
  <div class="hero-heading"><h1>Run a crew.<br>Keep your <em>flow.</em></h1><div class="hero-copy"><p>One terminal. All your coding agents.<br>Give every task its own space, and know<br class="desktop-break"> exactly when you’re needed.</p><div class="hero-actions"><a class="button primary" href="#start">Get modisa ${arrow}</a><a class="quiet-link" href="docs/introduction/">Explore the docs ${diagonal}</a></div><p class="platforms">Free &amp; open source <span>·</span> macOS &amp; Linux</p></div></div>
  <div class="workspace-stage"><div class="stage-rail" aria-hidden="true"><span>LESS SWITCHING. MORE SHIPPING.</span><span>01 — LIVE PREVIEW</span></div>${demo}</div>
  <div class="hero-bottom"><span><span class="status-dot"></span> MANY AGENTS. ONE CLEAR VIEW.</span><a href="#workflow">Meet your new workspace ${icon("arrow-down-linear")}</a></div>
</section>
<section class="compatibility" aria-label="Supported agents"><p class="eyebrow">YOUR AGENTS.<br>ALREADY AT HOME.</p><div class="agent-wordmarks"><span>Claude Code</span><span>Codex</span><span>Gemini CLI</span><span>OpenCode</span></div><a href="docs/agents/">${agentNames.length} agents &amp; counting ${diagonal}</a></section>
<section class="workflow" id="workflow">
  <div class="section-heading"><p class="eyebrow">01 / FIND YOUR FOCUS</p><div><h2>More agents.<br><span class="muted">Less mental overhead.</span></h2><p>You’re here to build. Modisa keeps the sessions,<br class="desktop-break"> the signals, and the next move in view.</p></div></div>
  <article class="attention-feature">
    <div class="feature-copy"><span class="feature-index">[ 01.1 ] &nbsp; ATTENTION</span><h3>The right signal.<br>At the right time.</h3><p>A permission prompt. A question. A finished task. See who needs you without checking every tab.</p><a class="text-link" href="docs/agents/">How agent detection works ${arrow}</a><div class="feature-note"><span class="status-dot"></span><span>Terminal detection built in.<br>Agent hooks for richer state.</span></div></div>
    <div class="attention-visual" aria-label="Example agent activity: reviewer needs permission, coder is working, docs is done"><div class="activity-header"><span>AGENT ACTIVITY</span><span>03 CONNECTED</span></div><div class="activity-card active"><span class="agent-avatar">!</span><div><strong>@reviewer</strong><small>codex</small></div><span class="state-badge">Needs you</span></div><div class="permission-card"><span class="permission-label">PERMISSION REQUEST</span><p>Ready to push the changes.<br>May I continue?</p><code>git push origin feature/auth</code><div class="permission-response"><span>↵ Allow command</span><span>esc Cancel</span></div></div><div class="activity-card"><span class="agent-avatar">↗</span><div><strong>@coder</strong><small>claude-code</small></div><span class="activity-state">Working</span></div><div class="activity-card"><span class="agent-avatar">✓</span><div><strong>@docs</strong><small>opencode</small></div><span class="activity-state">Done</span></div><p class="visual-caption">The important thing comes to you.</p></div>
  </article>
  <div class="feature-pair">
    <article class="feature-panel"><span class="feature-index">[ 01.2 ] &nbsp; COORDINATION</span><h3>A place for<br>every moving part.</h3><p>Spaces for projects. Tabs for context. Real terminal panes with each agent’s full interface intact.</p><div class="layout-visual" aria-label="Example workspace with coder, reviewer, and tests"><div class="layout-tabs"><b>api</b><span>web</span><span>infra</span><span>+</span></div><div class="layout-panes"><div><span>@coder</span><strong>claude</strong><small>Refactoring session middleware</small><div class="code-strokes" aria-hidden="true"><i></i><i></i><i></i></div></div><div><span>@reviewer</span><strong>codex</strong><small>Reviewing changes</small></div><div><span>@tests</span><strong>214 passed <span>✓</span></strong><small>bun test --watch</small></div></div><div class="layout-caption"><kbd>Ctrl+B</kbd><span>One prefix. Every pane.</span></div></div><a class="text-link" href="docs/quick-start/">Make it your workspace ${arrow}</a></article>
    <article class="feature-panel continuity"><span class="feature-index">[ 01.3 ] &nbsp; CONTINUITY</span><h3>Close the window.<br>Keep the momentum.</h3><p>A background server keeps your panes running. Detach, reconnect, or pick things up over SSH.</p><div class="continuity-visual"><div class="session-line"><span class="terminal-prompt">~</span><code>modisa detach</code><span>↗</span></div><div class="session-persistence"><span class="persistence-line"></span><span class="session-live"><span class="status-dot"></span> Session still running</span><span class="persistence-line"></span></div><div class="session-line"><span class="terminal-prompt">~</span><code>modisa attach</code><span>↵</span></div><div class="restored"><span>✓</span><p>Right where you left off.<small>Same panes. Same conversations.</small></p></div></div><a class="text-link" href="docs/introduction/">Meet the persistent session ${arrow}</a></article>
  </div>
</section>
<section class="start" id="start"><div class="start-heading"><p class="eyebrow">02 / READY WHEN YOU ARE</p><h2>Your next session.<br><em>All together.</em></h2><p>A little setup. A lot less switching.</p><a class="text-link" href="docs/quick-start/">Take the five-minute tour ${arrow}</a><span class="start-platforms">macOS &amp; Linux · Free &amp; open source</span></div><ol class="install-steps"><li><div class="step-title"><span>01</span><h3>Install modisa</h3></div><div class="install"><span class="prompt">$</span><code>${escape(INSTALL)}</code>${copyButton(INSTALL)}</div></li><li><div class="step-title"><span>02</span><h3>Open a session</h3></div><div class="install"><span class="prompt">$</span><code>modisa</code>${copyButton("modisa")}</div></li><li><div class="step-title"><span>03</span><h3>Bring your agents</h3></div><p>Press <kbd>Ctrl+B</kbd> then <kbd>a</kbd> to pick an agent.<br>Or run <code>claude</code> in any pane.</p></li></ol></section>
<section class="closing"><p>Built in the open.<br>Better with your crew.</p><a class="button" href="${o.repo}">Find us on GitHub ${diagonal}</a><span class="closing-wordmark" aria-hidden="true">modisa<span>.</span></span></section>
</main>`;
}
