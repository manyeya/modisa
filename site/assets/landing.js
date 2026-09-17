// The home page's live session: agents change state the way they do in modisa — @reviewer asks,
// you approve, @coder finishes while you look away, @reviewer asks again. Still under reduced motion.
(() => {
  const tui = document.querySelector("[data-demo]");
  if (!tui) return;
  const $ = (s) => tui.querySelector(s);
  const panes = Object.fromEntries([...tui.querySelectorAll("[data-pane]")].map((p) => [p.dataset.pane, p]));
  const original = Object.fromEntries(Object.entries(panes).map(([k, p]) => [k, p.querySelector(".pane-body").innerHTML]));
  const caption = document.querySelector("[data-caption]");
  const agents = [
    { id: "reviewer", name: "reviewer", harness: "codex" },
    { id: "coder", name: "coder", harness: "claude-code" },
    { id: "docs", name: "docs", harness: "opencode" },
  ];
  const glyph = { blocked: "!", working: "◆", done: "✓", idle: "○" };
  const color = { blocked: "var(--t-amber)", working: "var(--t-blue)", done: "var(--t-green)", idle: "var(--t-dim)" };
  const label = { blocked: "needs you", working: "working", done: "done", idle: "idle" };
  const order = { blocked: 0, done: 1, working: 2, idle: 3 };

  const set = (id, state, body) => {
    panes[id].dataset.state = state;
    if (body !== undefined) panes[id].querySelector(".pane-body").innerHTML = body;
  };
  const sidebar = () => {
    const list = agents.map((a) => ({ ...a, state: panes[a.id].dataset.state })).sort((a, b) => order[a.state] - order[b.state]);
    $("[data-agents]").innerHTML = list.map((a) => `<div class="tui-agent${a.id === "coder" ? " focus" : ""}"><span style="color:${color[a.state]}">${glyph[a.state]}</span> <b>@${a.name}</b><small>${a.harness} · ${label[a.state]}</small></div>`).join("");
    const working = list.filter((a) => a.state === "working").length, blocked = list.filter((a) => a.state === "blocked").length;
    $("[data-count-working]").textContent = `◆ ${working} working`;
    const b = $("[data-count-blocked]");
    b.textContent = `! ${blocked} need you`;
    b.classList.toggle("zero", !blocked);
    tui.querySelector("[data-tab-blocked]").innerHTML = blocked ? "2:@reviewer <i>!</i>" : "2:@reviewer";
    tui.querySelector("[data-tab-blocked]").style.color = blocked ? "" : "var(--t-dim)";
  };
  let toastTimer;
  const toast = (text, kind = "") => {
    const t = $("[data-toast]");
    t.textContent = text;
    t.className = `tui-toast show ${kind}`;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (t.className = "tui-toast"), 2600);
  };
  const say = (text) => { if (caption) caption.textContent = text; };

  const reset = () => {
    for (const [id, html] of Object.entries(original)) set(id, id === "tests" ? "idle" : id === "docs" ? "done" : id === "reviewer" ? "blocked" : "working", html);
    sidebar();
    say("@reviewer is waiting on you.");
  };
  reset();
  const motionPreference = matchMedia("(prefers-reduced-motion: reduce)");
  const pauseButton = document.querySelector("[data-demo-pause]");
  let paused = motionPreference.matches;
  let visible = true;
  const paintPause = () => {
    if (!pauseButton) return;
    pauseButton.setAttribute("aria-pressed", String(paused));
    pauseButton.innerHTML = paused ? 'Play demo <span aria-hidden="true">▷</span>' : 'Pause demo <span aria-hidden="true">Ⅱ</span>';
  };
  pauseButton?.addEventListener("click", () => { paused = !paused; paintPause(); });
  motionPreference.addEventListener("change", (event) => { paused = event.matches; paintPause(); });
  if ("IntersectionObserver" in window) {
    new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; }).observe(tui);
  }
  const inactive = () => paused || !visible || document.hidden;
  paintPause();

  // @coder's live output, typed out a line at a time
  const lines = ["● Update src/auth/middleware.ts +54 −20", "● Run pnpm test auth", "● Update src/auth/rotate.ts +22 −3"];
  let line = 0, char = 0;
  setInterval(() => {
    if (inactive()) return;
    const stream = tui.querySelector("[data-stream]");
    if (!stream) return;
    char++;
    if (char > lines[line].length + 14) { char = 0; line = (line + 1) % lines.length; }
    stream.innerHTML = `<span class="ok">${lines[line].slice(0, 1)}</span>${lines[line].slice(1, char)}`;
  }, 45);
  const spin = ["✻", "✢", "✳", "✶", "✽"];
  let s = 0;
  setInterval(() => { if (inactive()) return; const el = tui.querySelector("[data-spinner]"); if (el) el.textContent = spin[s++ % spin.length]; }, 180);

  const steps = [
    () => { reset(); toast("@reviewer is blocked — needs you"); },
    () => {
      set("reviewer", "working", `<p class="dim">› review the auth changes before pushing</p><p>• Ran <u>pnpm lint</u> <span class="dim">— clean</span></p><p>• Ran <u>git push origin feature/auth</u></p><p class="spin"><span class="dim">•</span> Working <span class="dim">(4s · esc to interrupt)</span></p>`);
      sidebar();
      say("Approved. @reviewer is back to work.");
    },
    () => {
      set("coder", "done", `<p class="dim">❯ refactor the session middleware</p><p><span class="ok">●</span> Read <u>src/auth/session.ts</u></p><p><span class="ok">●</span> Update <u>src/auth/cookies.ts</u> <span class="dim">+38 −12</span></p><p><span class="ok">●</span> Update <u>src/auth/middleware.ts</u> <span class="dim">+54 −20</span></p><p>Sessions now rotate on login. 4 files changed.</p><p class="dim">❯ _</p>`);
      sidebar();
      toast("@coder is done", "done");
      say("@coder finished while you were looking elsewhere.");
    },
    () => {
      set("reviewer", "blocked", original.reviewer);
      sidebar();
      toast("@reviewer is blocked — needs you");
      say("@reviewer has another question.");
    },
  ];
  let step = 0;
  setInterval(() => { if (inactive()) return; step = (step + 1) % steps.length; steps[step](); }, 5000);
})();
