// Shared behaviour: night/day theme, search (/ or ⌘K), copy buttons, the docs menu on phones, and
// the "on this page" highlight.
(() => {
  const base = document.body.dataset.base || "./";
  const $ = (s, el = document) => el.querySelector(s);
  const $$ = (s, el = document) => [...el.querySelectorAll(s)];

  // ---------- theme ----------
  const toggle = $("[data-theme-toggle]");
  const paint = () => { if (toggle) toggle.textContent = document.documentElement.dataset.theme === "day" ? "☀" : "☾"; };
  paint();
  toggle?.addEventListener("click", () => {
    const next = document.documentElement.dataset.theme === "day" ? "night" : "day";
    document.documentElement.dataset.theme = next;
    try { localStorage.setItem("shepherd-theme", next); } catch {}
    paint();
  });

  // ---------- copy ----------
  const copy = async (button, text) => {
    try { await navigator.clipboard.writeText(text); } catch {
      const source = button.closest(".install, .code")?.querySelector("code");
      if (source) {
        const range = document.createRange();
        range.selectNodeContents(source);
        const selection = window.getSelection();
        selection.removeAllRanges();
        selection.addRange(range);
      }
      const label = button.textContent;
      button.textContent = "Press ⌘/Ctrl+C";
      setTimeout(() => { button.textContent = label; }, 2500);
      return;
    }
    const was = button.textContent;
    button.textContent = "copied";
    button.classList.add("done");
    setTimeout(() => { button.textContent = was; button.classList.remove("done"); }, 1400);
  };
  document.addEventListener("click", (e) => {
    const b = e.target.closest("[data-copy], [data-copy-text]");
    if (!b) return;
    copy(b, b.dataset.copyText ?? b.closest(".code")?.querySelector("pre").innerText.trim() ?? "");
  });

  // ---------- docs menu (phones) ----------
  const side = $("#side"), sideToggle = $("[data-side-toggle]");
  sideToggle?.addEventListener("click", () => {
    const open = side.classList.toggle("open");
    sideToggle.setAttribute("aria-expanded", String(open));
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && side?.classList.contains("open")) {
      side.classList.remove("open");
      sideToggle?.setAttribute("aria-expanded", "false");
      sideToggle?.focus();
    }
  });

  // ---------- on this page ----------
  const tocLinks = $$(".toc a[href^='#']");
  if (tocLinks.length && "IntersectionObserver" in window) {
    const byId = new Map(tocLinks.map((a) => [a.getAttribute("href").slice(1), a]));
    const seen = new IntersectionObserver((entries) => {
      for (const e of entries) if (e.isIntersecting) { tocLinks.forEach((a) => a.classList.remove("on")); byId.get(e.target.id)?.classList.add("on"); }
    }, { rootMargin: "-80px 0px -65% 0px" });
    $$(".doc section[id]").forEach((s) => seen.observe(s));
  }

  // ---------- search ----------
  const dialog = $("#search"), input = $("#search-input"), results = $("#search-results");
  if (!dialog) return;
  let index, selected = 0;
  const load = async () => (index ??= await fetch(`${base}search.json`).then((r) => r.json()).catch(() => []));
  const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const render = async () => {
    const q = input.value.trim().toLowerCase();
    if (!q) { results.innerHTML = '<p class="search-empty">Commands, keys, agents, settings…</p>'; return; }
    const words = q.split(/\s+/);
    const hits = (await load())
      .map((d) => { const hay = `${d.title} ${d.group} ${d.text}`.toLowerCase(); const score = words.every((w) => hay.includes(w)) ? (d.title.toLowerCase().includes(q) ? 3 : 0) + words.filter((w) => d.title.toLowerCase().includes(w)).length : -1; return { d, score }; })
      .filter((h) => h.score >= 0).sort((a, b) => b.score - a.score).slice(0, 12);
    selected = 0;
    results.innerHTML = hits.length
      ? hits.map(({ d }, i) => `<a href="${base}${d.url}" role="option" aria-selected="${i === 0}"><small>${esc(d.group)}</small><strong>${esc(d.title)}</strong><span>${esc(d.text.slice(0, 140))}${d.text.length > 140 ? "…" : ""}</span></a>`).join("")
      : `<p class="search-empty">Nothing for “${esc(input.value)}”.</p>`;
  };
  const move = (by) => {
    const items = $$("a", results);
    if (!items.length) return;
    items[selected]?.setAttribute("aria-selected", "false");
    selected = (selected + by + items.length) % items.length;
    items[selected].setAttribute("aria-selected", "true");
    items[selected].scrollIntoView({ block: "nearest" });
  };
  const open = () => { if (!dialog.open) { dialog.showModal(); input.select(); load(); } };
  $$("[data-search-open]").forEach((b) => b.addEventListener("click", open));
  $("[data-search-close]")?.addEventListener("click", () => dialog.close());
  dialog.addEventListener("click", (e) => { if (e.target === dialog) dialog.close(); });
  input.addEventListener("input", render);
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown") { e.preventDefault(); move(1); }
    else if (e.key === "ArrowUp") { e.preventDefault(); move(-1); }
    else if (e.key === "Enter") { const a = $$("a", results)[selected]; if (a) location.href = a.href; }
  });
  document.addEventListener("keydown", (e) => {
    const typing = /INPUT|TEXTAREA/.test(document.activeElement?.tagName ?? "");
    if ((e.key === "k" && (e.metaKey || e.ctrlKey)) || (e.key === "/" && !typing)) { e.preventDefault(); open(); }
  });
})();
