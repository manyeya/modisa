// The home page: a night in the pasture, scrolled from moonrise to first light. The canvases are drawn and moved by
// assets/flock.js; everything you read is plain HTML here.
import { DATA } from "./data";
import { escape } from "./html";

export const INSTALL = "curl -fsSL https://manyeya.github.io/modisa/install.sh | sh";
export const mark = (_base = "./") => `<span class="mark" aria-hidden="true"></span>`;

const agentNames = DATA.agents.map((a) => a.name);
const install = (text: string) => `<div class="install"><span class="prompt">$</span><code>${escape(text)}</code><button type="button" class="copy" aria-label="Copy command" data-copy-text="${escape(text)}">Copy</button></div>`;

export function landing(o: { version: string; repo: string }) {
  return `<header class="flock-top"><div class="in">
  <a class="brand" href="./">${mark()}modisa</a>
  <nav aria-label="Main"><a href="#flock">The flock</a><a href="#field">The field</a><a href="#notch">The notch</a><a class="keep" href="docs/introduction/">Docs</a><a href="plugins/">Plugins</a><a href="${o.repo}">GitHub ↗</a></nav>
  <a class="button gold" href="#gate">Get modisa</a>
</div></header>
<main id="main">
<section class="hero" id="top">
  <canvas class="ly ly-sky" data-f=".55" data-px="5" aria-hidden="true"></canvas>
  <canvas class="ly ly-far" data-f=".42" data-px="12" aria-hidden="true"></canvas>
  <canvas class="ly ly-mid" data-f=".3" data-px="20" aria-hidden="true"></canvas>
  <div class="hero-copy in" id="hero-copy" data-f=".34" data-px="6">
    <p class="kicker">The shepherd for your coding agents</p>
    <h1>Tend the flock.<br><em>Keep your flow.</em></h1>
    <div class="hero-row">
      <p class="lede">Your coding agents are the flock. Modisa keeps every one of them in a real terminal pane, watches them all, and calls you over the moment one needs you.</p>
      <div class="hero-side">
        <div class="hero-actions"><a class="button gold" href="#gate">Get modisa →</a><a class="button ghost" href="#flock">Meet the flock</a></div>
        ${install(INSTALL)}
        <p class="hero-meta">Free and open source · macOS and Linux · v${escape(o.version)}</p>
      </div>
    </div>
  </div>
  <div class="pasture-zone">
    <canvas class="ly" id="pasture" data-f=".12" data-px="28" aria-label="A night pasture. The shepherd stands on the left with his crook. Five sheep, one per agent: one needs you and runs over to hop in front of him, two graze while they work, one that finished waits by him, and one sleeps at the back."></canvas>
    <div class="toast" id="toast" role="status" aria-live="polite"><span class="t-glyph">!</span><span class="t-text"></span></div>
  </div>
  <canvas class="ly ly-fg" data-f="-.08" data-px="44" aria-hidden="true"></canvas>
  <a class="scroll-hint" href="#flock">WALK THE PASTURE ↓</a>
</section>

<section class="fold" aria-label="Supported agents">
  <div class="in">
    <p class="fold-label">YOUR AGENTS,<br>ALREADY IN THE FOLD</p>
    <canvas id="count" data-agents="${escape(agentNames.join("|"))}" aria-label="Sheep jump a fence one at a time, each wearing the name of an agent modisa supports."></canvas>
    <p class="fold-count"><b id="count-n">01</b><span id="count-name">${escape(agentNames[0] ?? "")}</span> of ${agentNames.length}<br><a href="docs/agents/">Every agent →</a></p>
  </div>
</section>

<section class="field" id="flock">
  <canvas class="deco" id="deco-hills" data-depth=".16" style="left:0; bottom:-40px; width:100%; height:320px" aria-hidden="true"></canvas>
  <div class="in">
    <div class="section-head" data-depth=".05">
      <p class="kicker">The flock</p>
      <h2>The shepherd sees <em>every one.</em></h2>
      <p class="lede">Modisa recognises every agent it hosts and reads its state from the screen and from the agent's own hooks. When one changes, you know without going to look.</p>
    </div>
    <div class="states">
      <article class="state needs" data-depth=".01"><canvas data-mini="needs" aria-hidden="true"></canvas><div class="body"><div class="head"><span class="glyph">!</span><h3>Needs you</h3></div><p>A permission prompt or a question. It runs to the shepherd and hops until you come.</p></div></article>
      <article class="state" data-depth=".025"><canvas data-mini="working" aria-hidden="true"></canvas><div class="body"><div class="head"><span class="glyph">◆</span><h3>Working</h3></div><p>Head down, grazing. You don't have to stand and watch it.</p></div></article>
      <article class="state done" data-depth=".04"><canvas data-mini="done" aria-hidden="true"></canvas><div class="body"><div class="head"><span class="glyph">✓</span><h3>Done</h3></div><p>Finished while you were somewhere else. It waits by the shepherd.</p></div></article>
      <article class="state idle" data-depth=".055"><canvas data-mini="idle" aria-hidden="true"></canvas><div class="body"><div class="head"><span class="glyph">○</span><h3>Idle</h3></div><p>Nothing to do yet. It sleeps at the back of the field.</p></div></article>
    </div>
    <p class="where">You hear it in <b>the pane border</b>, <b>the tab bar</b>, <b>the sidebar</b>, <b>a toast</b>, <b>a system notification</b> or <b>a sound</b>. You pick which.</p>
  </div>
</section>

<section class="field" id="field">
  <canvas class="deco" id="deco-acacia" data-depth=".22" style="right:-40px; top:6%; width:620px; height:420px" aria-hidden="true"></canvas>
  <div class="in">
    <div class="field-grid">
      <div class="field-copy" data-depth=".04">
        <p class="kicker">The field</p>
        <h2>A field for <em>every task.</em></h2>
        <p class="lede">Spaces for projects, tabs for context, and a real terminal pane for every agent with its own interface intact. The keyboard and the mouse both work everywhere.</p>
        <div class="keys">
          <p class="crook"><kbd>Ctrl</kbd><kbd>B</kbd> One crook. Every pane.</p>
          <kbd>a</kbd><span>bring in an agent</span>
          <kbd>v</kbd><span>split right</span>
          <kbd>-</kbd><span>split down</span>
          <kbd>w</kbd><span>switch space</span>
          <kbd>d</kbd><span>detach, panes keep running</span>
        </div>
      </div>
      <div class="tui" role="img" aria-label="A modisa session: a sidebar of agents with their states, and three panes. Codex in the focused pane asks to run git push." data-depth="-.04">
        <div class="tui-top"><span class="space">api</span><span class="tab on">main<i>!</i></span><span class="tab">web</span><span class="tab">infra</span><span class="tab">+</span><span class="alert">! 1 needs you</span></div>
        <div class="tui-main">
          <div class="tui-side">
            <p class="h">AGENTS</p>
            <p class="needs"><span class="g">!</span>@reviewer</p>
            <p><span class="g">◆</span>@coder</p>
            <p><span class="g">◆</span>@tests</p>
            <p class="done"><span class="g">✓</span>@docs</p>
            <p class="idle"><span class="g">○</span>@notes</p>
            <p class="settings">⚙ settings</p>
          </div>
          <div class="panes">
            <div class="pane"><span class="title">@coder · claude ◆</span>
              <p class="you">❯ refactor the session middleware</p>
              <p class="tool">● Read src/auth/session.ts</p>
              <p class="tool">● Read src/auth/cookies.ts</p>
              <p class="tool">● Update src/auth/session.ts</p>
              <p class="add">  + rotateSession(req) on login</p>
              <p class="add">  + clearCookie on logout</p>
              <p class="dim">  ⎿ 2 files changed</p>
              <p>&nbsp;</p>
              <p class="dim">✻ Refactoring… (esc to interrupt)</p>
            </div>
            <div class="pane focus"><span class="title">@reviewer · codex !</span>
              <p class="you">› review the auth changes, then push</p>
              <div class="ask"><p class="q">Allow command?</p><p><code>git push origin feature/auth</code></p><p>› 1. Yes, run it</p><p class="dim">  2. No, tell Codex what to do</p></div>
            </div>
            <div class="pane"><span class="title">@tests · gemini ◆</span>
              <p class="you">$ bun test --watch</p>
              <p class="add">✓ auth/session   12 passed</p>
              <p class="add">✓ auth/cookies    9 passed</p>
              <p class="dim">214 passed · 0 failed</p>
            </div>
          </div>
        </div>
        <div class="tui-status"><span>modisa</span><span>api</span><span>main ↑2 ●3</span><span><b>1 needs you</b> · 3 working · 1 done</span></div>
      </div>
    </div>
    <div class="trio">
      <article data-depth=".03"><p class="kicker">Sheepdogs</p><h3>Agents can herd too.</h3><p>Any pane can split panes, spawn agents and message them through the modisa CLI, which modisa teaches every agent it hosts.</p><pre><span>$</span> modisa agent spawn codex --name reviewer
<span>$</span> modisa send @reviewer "check the auth diff"</pre><a class="more" href="docs/messaging/">Agents talking to agents →</a></article>
      <article data-depth=".06"><p class="kicker">Plugins</p><h3>Grow the field.</h3><p>Add views, commands and status segments, written in any language. One command installs one.</p><pre><span>$</span> modisa plugin install &lt;git-url&gt;</pre><a class="more" href="plugins/">Browse plugins →</a></article>
      <article data-depth=".09"><p class="kicker">Make it yours</p><h3>Your own pasture.</h3><p>Themes, keys, layout, sounds and hooks all live in one config file, and modisa keeps your comments in it.</p><pre>~/.config/modisa/config.toml</pre><a class="more" href="docs/customize/">Make modisa yours →</a></article>
    </div>
  </div>
</section>

<section class="night" id="night">
  <div class="in">
    <div class="section-head" data-depth=".05">
      <p class="kicker">Through the night</p>
      <h2>Close the window.<br><em>The flock keeps grazing.</em></h2>
      <p class="lede">A background server keeps every pane running. Detach, close the terminal, and come back later, or from another machine over SSH. Agents pick up in the conversation they were in.</p>
    </div>
  </div>
  <div class="night-scene">
    <canvas id="night-canvas" aria-label="Sheep grazing under the moon while you're away. The moon crosses the sky as you scroll."></canvas>
    <div class="session" data-depth="-.06">
      <p class="cmd">modisa detach</p>
      <p class="live"><i></i>session still running</p>
      <p class="cmd">modisa attach</p>
      <p class="back">Right where you left off.<small>Same panes. Same conversations.</small></p>
    </div>
  </div>
</section>

<section class="field" id="notch">
  <div class="in notch-grid">
    <div class="notch-copy" data-depth=".05">
      <p class="kicker">In the notch · macOS</p>
      <h2>The flock, up in <em>your notch.</em></h2>
      <p class="lede">Every agent from every session can live in your MacBook's notch. The shepherd sits on one side and the sheep that wants you most sits on the other. Point at it and it opens into the pasture. Click a sheep to go to its pane.</p>
      ${install("modisa notch")}
      <p class="small">Off until you turn it on. Point at the notch to open it.</p>
    </div>
    <div class="macbook" data-depth="-.05">
      <div class="screen">
        <div class="menubar"><b>Ghostty</b><span>Shell</span><span>Edit</span><span>View</span><span>Thu 21:40</span></div>
        <div class="window"><p><b>modisa</b> · api · main</p><p class="w">@reviewer is waiting on you.</p><p>@coder ◆ refactoring session middleware</p><p>@tests ◆ 214 passed</p></div>
        <button class="notch" id="notch-btn" type="button" aria-expanded="false" aria-label="Open the flock in the notch">
          <span class="ear left"><canvas id="ear-l" aria-hidden="true"></canvas></span>
          <span></span>
          <span class="ear right"><canvas id="ear-r" aria-hidden="true"></canvas><b>2</b></span>
          <span class="notch-panel"><canvas id="notch-pasture" aria-hidden="true"></canvas></span>
        </button>
      </div>
      <div class="bezel"></div>
    </div>
  </div>
</section>

<section class="gate" id="gate">
  <div class="in">
    <div class="section-head" data-depth=".05">
      <p class="kicker">First light</p>
      <h2>Open <em>the gate.</em></h2>
      <p class="lede">A minute of setup, then no more rounding them up. Releases are built for macOS on Apple silicon and Linux on x64 and arm64.</p>
    </div>
  </div>
  <div class="gate-scene"><canvas id="gate-canvas" aria-label="A wooden gate in a kraal fence swings open as you scroll, and the flock walks out through it past the shepherd."></canvas></div>
  <div class="in">
    <ol class="steps">
      <li><span class="n">i.</span><h3>Install modisa</h3>${install(INSTALL)}<a class="more" href="docs/install/">Other ways to install →</a></li>
      <li><span class="n">ii.</span><h3>Open a session</h3>${install("modisa")}<a class="more" href="docs/quick-start/">The five-minute tour →</a></li>
      <li><span class="n">iii.</span><h3>Bring in the flock</h3><p>Press <kbd>Ctrl</kbd> <kbd>B</kbd> then <kbd>a</kbd> to pick an agent, or run <code>claude</code> in any pane.</p></li>
    </ol>
  </div>
</section>
</main>

<footer class="dawn" id="dawn">
  <div class="dawn-sky">
    <canvas id="dawn-canvas" aria-label="Sunrise over the hills. The flock walks along the ridge with the shepherd behind."></canvas>
    <div class="dawn-copy"><p class="wordmark">modisa</p><p class="sign">Built in the open. Better as a flock.</p></div>
  </div>
  <div class="in dawn-foot">
    <nav aria-label="Footer"><a href="docs/introduction/">Docs</a><a href="docs/install/">Install</a><a href="plugins/">Plugins</a><a href="${o.repo}/releases">Releases</a><a href="${o.repo}">GitHub</a></nav>
    <span>Free and open source · macOS and Linux · v${escape(o.version)}</span>
  </div>
</footer>`;
}
