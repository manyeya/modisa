// The home page. The hero is a live HTML rendering of a shepherd session (animated by landing.js),
// not a screenshot: agents working, one waiting on you, one done.
import { BUILTIN_AGENTS } from "../../src/config/agents";
import { escape } from "./html";

export const INSTALL = "curl -fsSL https://manyeya.github.io/shepherd/install.sh | sh";
export const mark = `<svg class="mark" viewBox="0 0 26 32" aria-hidden="true"><path d="M8 30V12a6.5 6.5 0 0 1 13 0v1.5" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round"/><path d="M21 15.5v1.2" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/><rect x="17.6" y="17" width="6.8" height="8.2" rx="2" class="mark-lamp"/></svg>`;

const agentNames = BUILTIN_AGENTS.filter((a) => a.id !== "generic").map((a) => a.name);
const copyButton = (text: string, label = "copy") => `<button type="button" class="copy" data-copy-text="${escape(text)}">${label}</button>`;

// One pane of the demo session. Its body lines are what the agent has drawn.
const pane = (id: string, name: string, agent: string, state: string, lines: string[]) => `
  <div class="pane" data-pane="${id}" data-state="${state}">
    <div class="pane-title"><span class="pane-glyph"></span><b>@${name}</b><span class="pane-agent">${agent}</span><span class="pane-state"></span></div>
    <div class="pane-body">${lines.join("")}</div>
  </div>`;

const demo = `
<figure class="demo" aria-label="A shepherd session: four agents in panes, one waiting on you">
  <div class="tui" data-demo>
    <div class="tui-tabs"><span class="tui-space">◈ api ▸</span><span class="tui-tab on">1:@coder</span><span class="tui-tab" data-tab-blocked>2:@reviewer <i>!</i></span><span class="tui-tab">3:tests</span><span class="tui-plus">+</span></div>
    <div class="tui-main">
      <aside class="tui-side">
        <p class="tui-label">SPACES</p>
        <p class="tui-row sel">api</p><p class="tui-row">web</p><p class="tui-row">infra</p>
        <p class="tui-label">AGENTS / 3</p>
        <div class="tui-agents" data-agents></div>
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
    <div class="tui-toast" data-toast role="status"></div>
  </div>
  <figcaption>Live, not a screenshot: this is what shepherd draws. <span data-caption>@reviewer is waiting on you.</span></figcaption>
</figure>`;

export function landing(o: { version: string; repo: string }) {
  const herd = agentNames.map((n) => `<li>${escape(n)}</li>`).join("");
  return `
<div class="night" aria-hidden="true"></div>
<header class="top">
  <a class="brand" href="./" aria-label="shepherd home">${mark}<span>shepherd</span></a>
  <nav aria-label="Main"><a href="docs/introduction/">Docs</a><a href="docs/agents/">Agents</a><a href="#start">Install</a><a href="${o.repo}" rel="noopener">GitHub ↗</a></nav>
  <span class="version-pill">v${escape(o.version)}</span>
</header>
<main id="main">
  <section class="hero">
    <svg class="hero-art" viewBox="0 0 320 460" aria-hidden="true">
      <defs>
        <radialGradient id="glow" cx="50%" cy="50%" r="50%"><stop offset="0" stop-color="#ffc964" stop-opacity=".55"/><stop offset=".45" stop-color="#f5a524" stop-opacity=".16"/><stop offset="1" stop-color="#f5a524" stop-opacity="0"/></radialGradient>
        <linearGradient id="pane" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffd98a"/><stop offset="1" stop-color="#f5a524"/></linearGradient>
      </defs>
      <circle class="art-glow" cx="228" cy="262" r="150" fill="url(#glow)"/>
      <path d="M110 452V150a82 82 0 0 1 164 0v26" fill="none" stroke="#e8ecf5" stroke-width="9" stroke-linecap="round"/>
      <path d="M274 176v30" stroke="#a7b1ca" stroke-width="3" stroke-dasharray="4 5" stroke-linecap="round"/>
      <g class="art-lamp">
        <path d="M252 214h44l-6 -10h-32z" fill="#2a3760"/>
        <rect x="248" y="214" width="52" height="66" rx="10" fill="#141d36" stroke="#2a3760" stroke-width="3"/>
        <rect x="258" y="224" width="32" height="46" rx="6" fill="url(#pane)"/>
        <path d="M274 236c6 7 6 14 0 22c-6-8-6-15 0-22z" fill="#fff4dc" opacity=".85"/>
        <rect x="244" y="280" width="60" height="8" rx="4" fill="#2a3760"/>
      </g>
      <g fill="#e8ecf5"><circle cx="40" cy="60" r="1.6"/><circle cx="80" cy="24" r="1"/><circle cx="300" cy="40" r="1.4"/><circle cx="20" cy="190" r="1"/><circle cx="190" cy="16" r="1.2"/></g>
    </svg>
    <p class="eyebrow"><span class="lamp" aria-hidden="true"></span>open source · macOS &amp; Linux · a terminal multiplexer for coding agents</p>
    <h1>Every agent.<br><span>One terminal.</span></h1>
    <p class="lede">Shepherd runs Claude Code, Codex and ${agentNames.length - 2} other agents side by side in real terminal panes — and lights a lantern the moment one of them <em>needs you</em>.</p>
    <div class="install" data-install><span class="prompt" aria-hidden="true">$</span><code>${escape(INSTALL)}</code>${copyButton(INSTALL)}</div>
    <div class="actions"><a class="btn lantern" href="docs/quick-start/">Take the five-minute tour</a><a class="btn" href="docs/introduction/">Read the docs</a></div>
  </section>

  ${demo}

  <section class="herd" aria-label="Agents shepherd knows">
    <p class="section-tag">${agentNames.length} agents, recognised on sight</p>
    <div class="herd-track"><ul>${herd}</ul><ul aria-hidden="true">${herd}</ul></div>
  </section>

  <section class="chapters">
    <article class="chapter">
      <span class="num">01</span>
      <h2>Know who needs you.</h2>
      <p>A permission prompt, a question, a finished task: shepherd reads each agent's screen, title and progress and sorts them — <b class="t-blocked">needs you</b>, <b class="t-working">working</b>, <b class="t-done">done</b>. The pane, its tab and the sidebar light up; you get a toast, a system notification or one of 17 sounds.</p>
      <div class="vignette states"><span class="s blocked">! @reviewer <i>codex · needs you</i></span><span class="s working">◆ @coder <i>claude-code · working</i></span><span class="s done">✓ @docs <i>opencode · done</i></span></div>
    </article>
    <article class="chapter">
      <span class="num">02</span>
      <h2>Real terminals, arranged your way.</h2>
      <p>Spaces for projects, tabs inside them, panes split any way you like. Every pane is a true PTY — your shell, your tools, the agent's full interface. Drive it all with <kbd>Ctrl+B</kbd> or the mouse: drag borders, right-click for menus.</p>
      <div class="vignette split"><i></i><i></i><i class="hot"></i><i></i></div>
    </article>
    <article class="chapter">
      <span class="num">03</span>
      <h2>Agents that work together.</h2>
      <p>From inside any pane, an agent can split a pane for the tests, spawn a reviewer, wait on it and message it — through plain shell commands or MCP. Hop counts and rate limits keep two agents from talking forever.</p>
      <pre class="vignette cli"><code><span class="amber">$</span> shepherd agent spawn codex --name reviewer
<span class="amber">$</span> shepherd wait @reviewer --state idle
<span class="amber">$</span> shepherd send @reviewer "fixed — re-check?"</code></pre>
    </article>
    <article class="chapter">
      <span class="num">04</span>
      <h2>Close the laptop. Nothing stops.</h2>
      <p>A background server owns every pane: detach, reattach, work over ssh with <code>--remote</code>. Reboot, and your layout comes back with each agent resumed in the exact conversation it was having.</p>
      <div class="vignette resume"><span>claude --resume 7f3c…</span><span>codex resume 019a…</span><span>opencode --session ses_4b…</span></div>
    </article>
  </section>

  <section class="knows">
    <div class="knows-copy">
      <p class="section-tag">How shepherd knows</p>
      <h2>It reads the screen.<br><span>Then it asks the agent.</span></h2>
      <p>Every agent comes with a manifest of rules over the live bottom of its screen, its terminal title and its progress signal. No setup, nothing to install. For more, <code>shepherd integration install all</code> connects each agent's own hooks: exact session resume for all of them, and exact state for the ones whose hooks see every step.</p>
      <a class="text-link" href="docs/agents/">How detection works →</a>
    </div>
    <div class="knows-art">
      <div class="screen-card"><p class="dim">codex · bottom of the screen</p><p>• Ran pnpm lint — clean</p><p class="hit">Press enter to confirm or esc to cancel</p></div>
      <div class="rule-card"><p><span class="dim">rule</span> live_strong_blocker</p><p><span class="dim">region</span> after_last_prompt_marker</p><p><span class="dim">then</span> <b class="t-blocked">needs you</b></p></div>
    </div>
  </section>

  <section class="start" id="start">
    <p class="section-tag">Start</p>
    <h2>Three steps to a quieter terminal.</h2>
    <ol class="steps">
      <li><span>1</span><h3>Install</h3><div class="install small"><code>${escape(INSTALL)}</code>${copyButton(INSTALL)}</div></li>
      <li><span>2</span><h3>Open a session</h3><div class="install small"><code>shepherd</code>${copyButton("shepherd")}</div></li>
      <li><span>3</span><h3>Bring an agent</h3><p><kbd>Ctrl+B</kbd> <kbd>a</kbd> and pick one — or just run <code>claude</code> in a pane.</p></li>
    </ol>
  </section>

  <section class="finale">
    <h2>Go on.<br><span>Start the herd.</span></h2>
    <div class="actions"><a class="btn lantern" href="docs/install/">Install shepherd</a><a class="btn" href="${o.repo}" rel="noopener">Star it on GitHub ↗</a></div>
  </section>
</main>`;
}
