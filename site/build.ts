// Builds the shepherd site into site/out: the home page, the docs, a search index, sitemap, 404 and
// the install script. Every local link and #anchor is checked before it finishes.
//   bun site/build.ts            (SHEPHERD_VERSION sets the version shown; else the latest git tag)
import { guide } from "./content/guide";
import { escape, type Page } from "./content/html";
import { landing, mark } from "./content/landing";
import { reference } from "./content/reference";

const root = import.meta.dir;
const out = `${root}/out`;
const site = "https://manyeya.github.io/shepherd/";
const repo = "https://github.com/manyeya/shepherd";
const pages: Page[] = [...guide, ...reference];
const tag = (await Bun.$`git describe --tags --abbrev=0 --match v*`.quiet().nothrow().text()).trim();
const version = (Bun.env.SHEPHERD_VERSION || tag || "0.1.0").replace(/^v/, "");

const fonts = `<link rel="preload" href="__BASE__assets/fonts/space-grotesk-latin.woff2" as="font" type="font/woff2" crossorigin>`;

function shell(o: { title: string; description: string; body: string; base: string; path: string; landing?: boolean }) {
  return `<!doctype html><html lang="en" data-theme="night"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>${escape(o.title)}</title><meta name="description" content="${escape(o.description)}">
<meta property="og:title" content="${escape(o.title)}"><meta property="og:description" content="${escape(o.description)}"><meta property="og:type" content="website"><meta property="og:image" content="${site}assets/social.png"><meta name="twitter:card" content="summary_large_image">
<link rel="canonical" href="${site}${o.path}"><link rel="icon" type="image/png" href="${o.base}assets/shepherd-mark.png"><meta name="theme-color" content="#0b0b0d">
<script>try{var t=localStorage.getItem("shepherd-theme");if(t)document.documentElement.dataset.theme=t}catch(e){}</script>
${fonts.replace("__BASE__", o.base)}<link rel="stylesheet" href="${o.base}assets/site.css">${o.landing ? `<link rel="stylesheet" href="${o.base}assets/terminal.css"><link rel="stylesheet" href="${o.base}assets/landing.css"><link rel="stylesheet" href="${o.base}assets/motion.css"><script type="module" src="${o.base}assets/motion.js"></script>` : ""}<script defer src="${o.base}assets/site.js"></script>
</head><body class="${o.landing ? "is-landing" : "is-docs"}" data-base="${o.base}"><a class="skip" href="#main">Skip to content</a>
${o.body}
${footer(o.base)}
${search()}</body></html>`;
}

function footer(base: string) {
  return `<footer class="foot"><a class="brand" href="${base}">${mark(base)}<span>shepherd.</span></a><p>A little order for your agents.</p><nav aria-label="Footer"><a href="${base}docs/introduction/">Docs</a><a href="${base}docs/install/">Install</a><a href="${repo}/releases">Releases</a><a href="${repo}">GitHub</a></nav><small>v${escape(version)}</small></footer>`;
}

function search() {
  return `<dialog class="search" id="search" aria-label="Search the docs"><div class="search-bar"><span aria-hidden="true">/</span><input type="search" id="search-input" placeholder="Search the docs" autocomplete="off" aria-controls="search-results"><button type="button" data-search-close>esc</button></div><div id="search-results" role="listbox"><p class="search-empty">Commands, keys, agents, settings…</p></div></dialog>`;
}

function docsHeader(base: string) {
  return `<header class="top docs-top"><a class="brand" href="${base}" aria-label="shepherd home">${mark(base)}<span>shepherd.</span></a><span class="top-tag">docs</span><nav aria-label="Main"><button type="button" class="search-open" data-search-open><span>Search</span><kbd>/</kbd></button><a href="${repo}" rel="noopener">GitHub ↗</a><button type="button" class="theme-switch" data-theme-toggle aria-label="Switch between night and day">☾</button></nav></header>`;
}

function docsPage(page: Page, index: number) {
  const base = "../../";
  const groups = [...new Set(pages.map((p) => p.group))];
  const nav = groups.map((g) => `<div class="side-group"><p>${g}</p>${pages.filter((p) => p.group === g).map((p) => `<a href="../${p.slug}/"${p.slug === page.slug ? ' aria-current="page"' : ""}>${p.title}</a>`).join("")}</div>`).join("");
  const toc = page.sections.map((s) => `<a href="#${s.id}">${s.title}</a>`).join("");
  const prev = pages[index - 1], next = pages[index + 1];
  const body = `${docsHeader(base)}
<div class="docs-bar"><button type="button" data-side-toggle aria-expanded="false" aria-controls="side">☰ ${escape(page.group)}</button><span>${escape(page.title)}</span></div>
<div class="docs">
  <aside class="side" id="side" aria-label="Documentation">${nav}</aside>
  <main id="main" class="doc">
    <p class="crumb">${escape(page.group)}</p>
    <h1>${escape(page.title)}</h1>
    <p class="doc-lede">${escape(page.description)}</p>
    ${page.sections.map((s) => `<section id="${s.id}"><h2><a href="#${s.id}">${s.title}</a></h2>${s.html}</section>`).join("\n")}
    <nav class="pager" aria-label="Previous and next">${prev ? `<a href="../${prev.slug}/"><span>← previous</span>${prev.title}</a>` : "<span></span>"}${next ? `<a href="../${next.slug}/" class="next"><span>next →</span>${next.title}</a>` : ""}</nav>
  </main>
  <aside class="toc" aria-label="On this page"><p>On this page</p>${toc}<a class="toc-edit" href="${repo}/tree/main/site/content">Edit these docs ↗</a></aside>
</div>`;
  return shell({ title: `${page.title} · shepherd docs`, description: page.description, body, base, path: `docs/${page.slug}/` });
}

const plain = (html: string) => html.replace(/<[^>]+>/g, " ").replace(/&gt;/g, ">").replace(/&lt;/g, "<").replace(/&quot;/g, '"').replace(/&amp;/g, "&").replace(/\s+/g, " ").trim();

await Bun.$`rm -rf ${out}`;
for (const f of await Array.fromAsync(new Bun.Glob("**/*").scan({ cwd: `${root}/assets` }))) await Bun.write(`${out}/assets/${f}`, Bun.file(`${root}/assets/${f}`));
const motion = await Bun.build({ entrypoints: [`${root}/client/motion.js`], outdir: `${out}/assets`, target: "browser", minify: true, naming: "motion.[ext]" });
if (!motion.success) throw new AggregateError(motion.logs, "Site animation bundle failed");
await Bun.write(`${out}/install.sh`, Bun.file(`${root}/../install.sh`));
await Bun.write(`${out}/.nojekyll`, "");
await Bun.write(`${out}/index.html`, shell({ title: "Shepherd — your agents, under control", description: "A terminal multiplexer for coding agents. Claude Code, Codex and 22 more in real panes — and you always know which one needs you.", body: landing({ version, repo }), base: "./", path: "", landing: true }));
for (const [i, page] of pages.entries()) await Bun.write(`${out}/docs/${page.slug}/index.html`, docsPage(page, i));
await Bun.write(`${out}/docs/index.html`, `<!doctype html><meta charset="utf-8"><meta http-equiv="refresh" content="0; url=introduction/"><link rel="canonical" href="${site}docs/introduction/"><a href="introduction/">Documentation</a>`);
await Bun.write(`${out}/search.json`, JSON.stringify(pages.flatMap((p) => [
  { title: p.title, group: p.group, text: plain(p.description), url: `docs/${p.slug}/` },
  ...p.sections.map((s) => ({ title: s.title, group: p.title, text: plain(s.html).slice(0, 400), url: `docs/${p.slug}/#${s.id}` })),
])));
await Bun.write(`${out}/404.html`, shell({ title: "Not found · shepherd", description: "This page wandered off.", base: "/shepherd/", path: "404.html", body: `${docsHeader("/shepherd/")}<main id="main" class="lost"><p class="crumb">404</p><h1>This one wandered off.</h1><p>The page isn't here. The rest of the herd is.</p><a class="btn lantern" href="/shepherd/docs/introduction/">Open the docs</a></main>` }));
await Bun.write(`${out}/sitemap.xml`, `<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">${["", ...pages.map((p) => `docs/${p.slug}/`)].map((u) => `<url><loc>${site}${u}</loc></url>`).join("")}</urlset>`);

// Every local href/src must resolve to a file, and every #anchor to an id on that page.
let checked = 0;
for (const file of [`${out}/index.html`, ...pages.map((p) => `${out}/docs/${p.slug}/index.html`)]) {
  const html = await Bun.file(file).text();
  const dir = file.slice(0, file.lastIndexOf("/"));
  for (const [, href] of html.matchAll(/(?:href|src)="([^"]+)"/g)) {
    if (/^(https?:|mailto:|data:)/.test(href!)) continue;
    const [path, hash] = href!.split("#");
    let target = path ? new URL(path, `file://${dir}/`).pathname : file;
    if (target.endsWith("/")) target += "index.html";
    const text = await Bun.file(target).text().catch(() => undefined);
    if (text === undefined) throw new Error(`broken link in ${file.slice(out.length)}: ${href}`);
    if (hash && !text.includes(`id="${hash}"`)) throw new Error(`missing anchor in ${file.slice(out.length)}: ${href}`);
    checked++;
  }
}
console.log(`built ${pages.length + 1} pages for v${version}; ${checked} local links and anchors check out`);
