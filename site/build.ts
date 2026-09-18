// Builds the modisa site into site/out: the home page, the docs, a search index, sitemap, 404 and
// the install script. Every local link and #anchor is checked before it finishes.
//   bun site/build.ts            (MODISA_VERSION sets the version shown; else the latest git tag)
import { guide } from "./content/guide";
import { escape, type Page } from "./content/html";
import { landing, mark } from "./content/landing";
import { reference } from "./content/reference";

const root = import.meta.dir;
const out = `${root}/out`;
const site = "https://manyeya.github.io/modisa/";
const repo = "https://github.com/manyeya/modisa";
const pages: Page[] = [...guide, ...reference];
const tag = (await Bun.$`git describe --tags --abbrev=0 --match v*`.quiet().nothrow().text()).trim();
const version = (Bun.env.MODISA_VERSION || tag || "0.1.0").replace(/^v/, "");

const fonts = `<link rel="preload" href="__BASE__assets/fonts/space-grotesk-latin.woff2" as="font" type="font/woff2" crossorigin>`;

// Structured data for search engines, escaped so a "</script>" in any text can't end the tag early.
const ld = (data: object) => `<script type="application/ld+json">${JSON.stringify({ "@context": "https://schema.org", ...data }).replace(/</g, "\\u003c")}</script>`;
const cardAlt = "modisa — Every agent. One terminal. A terminal multiplexer for coding agents.";
const summary = "A terminal multiplexer for coding agents. Claude Code, Codex and 22 more in real panes — and you always know which one needs you.";

function shell(o: { title: string; description: string; body: string; base: string; path: string; landing?: boolean; type?: "website" | "article"; data?: object; noindex?: boolean }) {
  const t = escape(o.title), d = escape(o.description);
  return `<!doctype html><html lang="en" data-theme="day"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>${t}</title><meta name="description" content="${d}">
${o.noindex ? `<meta name="robots" content="noindex">` : `<link rel="canonical" href="${site}${o.path}">`}
<meta property="og:site_name" content="modisa"><meta property="og:locale" content="en_US"><meta property="og:type" content="${o.type ?? "website"}"><meta property="og:url" content="${site}${o.path}"><meta property="og:title" content="${t}"><meta property="og:description" content="${d}">
<meta property="og:image" content="${site}assets/social.png"><meta property="og:image:type" content="image/png"><meta property="og:image:width" content="1200"><meta property="og:image:height" content="630"><meta property="og:image:alt" content="${cardAlt}">
<meta name="twitter:card" content="summary_large_image"><meta name="twitter:title" content="${t}"><meta name="twitter:description" content="${d}"><meta name="twitter:image" content="${site}assets/social.png"><meta name="twitter:image:alt" content="${cardAlt}">
<link rel="icon" type="image/png" href="${o.base}assets/modisa-mark.png"><link rel="apple-touch-icon" href="${o.base}assets/apple-touch-icon.png"><meta name="theme-color" content="#f6f6f9">
<link rel="alternate" type="text/plain" title="modisa docs for LLMs" href="${site}llms.txt">${o.data ? ld(o.data) : ""}
<script>try{var t=localStorage.getItem("modisa-theme");if(t==="day"||t==="night")document.documentElement.dataset.theme=t}catch(e){}</script>
${fonts.replace("__BASE__", o.base)}<link rel="stylesheet" href="${o.base}assets/site.css">${o.landing ? `<link rel="stylesheet" href="${o.base}assets/terminal.css"><link rel="stylesheet" href="${o.base}assets/landing.css"><script type="module" src="${o.base}assets/motion.js"></script>` : ""}<script defer src="${o.base}assets/site.js"></script>
</head><body class="${o.landing ? "is-landing" : "is-docs"}" data-base="${o.base}"><a class="skip" href="#main">Skip to content</a>
${o.body}
${footer(o.base)}
${search()}</body></html>`;
}

function footer(base: string) {
  return `<footer class="foot"><a class="brand" href="${base}">${mark(base)}<span>modisa<span class="brand-period">.</span></span></a><p>A little order for your agents.</p><nav aria-label="Footer"><a href="${base}docs/introduction/">Docs</a><a href="${base}docs/install/">Install</a><a href="${repo}/releases">Releases</a><a href="${repo}">GitHub</a></nav><small>v${escape(version)}</small></footer>`;
}

function search() {
  return `<dialog class="search" id="search" aria-label="Search the docs"><div class="search-bar"><span aria-hidden="true">/</span><input type="search" id="search-input" aria-label="Search documentation" placeholder="Search the docs" autocomplete="off" aria-controls="search-results"><button type="button" data-search-close>esc</button></div><div id="search-results" role="listbox"><p class="search-empty">Commands, keys, agents, settings…</p></div></dialog>`;
}

function docsHeader(base: string) {
  return `<header class="top docs-top"><a class="brand" href="${base}" aria-label="modisa home">${mark(base)}<span>modisa<span class="brand-period">.</span></span></a><a class="top-tag" href="${base}docs/introduction/">Documentation</a><nav aria-label="Main"><button type="button" class="search-open" data-search-open><span>Search documentation</span><kbd>⌘ K</kbd></button><a href="${repo}" rel="noopener">GitHub ↗</a><button type="button" class="theme-switch" data-theme-toggle aria-label="Switch between night and day">☾</button></nav></header>`;
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
  <aside class="side" id="side" aria-label="Documentation"><a class="side-home" href="${base}">← Back to modisa</a>${nav}<a class="side-start" href="../quick-start/"><span>YOUR FIRST SESSION</span><strong>Get up and running ↗</strong></a></aside>
  <main id="main" class="doc">
    <p class="crumb">DOCUMENTATION <span>/</span> ${escape(page.group)}</p>
    <h1>${escape(page.title)}</h1>
    <p class="doc-lede">${escape(page.description)}</p>
    ${page.sections.map((s) => `<section id="${s.id}"><h2><a href="#${s.id}">${s.title}</a></h2>${s.html}</section>`).join("\n")}
    <nav class="pager" aria-label="Previous and next">${prev ? `<a href="../${prev.slug}/"><span>← previous</span>${prev.title}</a>` : "<span></span>"}${next ? `<a href="../${next.slug}/" class="next"><span>next →</span>${next.title}</a>` : ""}</nav>
  </main>
  <aside class="toc" aria-label="On this page"><p>On this page</p>${toc}<a class="toc-edit" href="${repo}/tree/main/site/content">Edit these docs ↗</a></aside>
</div>`;
  const url = `${site}docs/${page.slug}/`;
  const data = { "@graph": [
    { "@type": "TechArticle", headline: page.title, description: page.description, url, inLanguage: "en", articleSection: page.group, isPartOf: { "@type": "WebSite", name: "modisa", url: site }, about: { "@type": "SoftwareApplication", name: "modisa" } },
    { "@type": "BreadcrumbList", itemListElement: [["modisa", site], ["Documentation", `${site}docs/introduction/`], [page.title, url]].map(([name, item], i) => ({ "@type": "ListItem", position: i + 1, name, item })) },
  ] };
  return shell({ title: `${page.title} · modisa docs`, description: page.description, body, base, path: `docs/${page.slug}/`, type: "article", data });
}

const plain = (html: string) => html.replace(/<[^>]+>/g, " ").replace(/&gt;/g, ">").replace(/&lt;/g, "<").replace(/&quot;/g, '"').replace(/&amp;/g, "&").replace(/\s+/g, " ").trim();

await Bun.$`rm -rf ${out}`;
// assets/social.svg is the link-preview card's source; assets/social.png is it rasterized. To redo the png after an
// edit, screenshot the svg at a 1200x630 viewport, e.g.
//   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless --disable-gpu \
//     --force-device-scale-factor=1 --window-size=1200,630 \
//     --screenshot=site/assets/social.png "file://$PWD/site/assets/social.svg"
for (const f of await Array.fromAsync(new Bun.Glob("**/*").scan({ cwd: `${root}/assets` }))) await Bun.write(`${out}/assets/${f}`, Bun.file(`${root}/assets/${f}`));
const motion = await Bun.build({ entrypoints: [`${root}/client/motion.js`], outdir: `${out}/assets`, target: "browser", minify: true, naming: "motion.[ext]" });
if (!motion.success) throw new AggregateError(motion.logs, "Site animation bundle failed");
await Bun.write(`${out}/install.sh`, Bun.file(`${root}/../install.sh`));
await Bun.write(`${out}/.nojekyll`, "");
await Bun.write(`${out}/index.html`, shell({ title: "Modisa — Run a crew. Keep your flow.", description: summary, body: landing({ version, repo }), base: "./", path: "", landing: true, data: { "@graph": [
  { "@type": "WebSite", name: "modisa", url: site, description: summary, inLanguage: "en" },
  { "@type": "SoftwareApplication", name: "modisa", description: summary, url: site, applicationCategory: "DeveloperApplication", operatingSystem: "macOS, Linux", softwareVersion: version, license: `${repo}/blob/main/LICENSE`, downloadUrl: `${repo}/releases/latest`, installUrl: `${site}docs/install/`, codeRepository: repo, image: `${site}assets/social.png`, offers: { "@type": "Offer", price: "0", priceCurrency: "USD" } },
] } }));
for (const [i, page] of pages.entries()) await Bun.write(`${out}/docs/${page.slug}/index.html`, docsPage(page, i));
await Bun.write(`${out}/docs/index.html`, `<!doctype html><meta charset="utf-8"><meta http-equiv="refresh" content="0; url=introduction/"><link rel="canonical" href="${site}docs/introduction/"><a href="introduction/">Documentation</a>`);
await Bun.write(`${out}/search.json`, JSON.stringify(pages.flatMap((p) => [
  { title: p.title, group: p.group, text: plain(p.description), url: `docs/${p.slug}/` },
  ...p.sections.map((s) => ({ title: s.title, group: p.title, text: plain(s.html).slice(0, 400), url: `docs/${p.slug}/#${s.id}` })),
])));
await Bun.write(`${out}/404.html`, shell({ title: "Not found · modisa", description: "This page wandered off.", base: "/modisa/", path: "404.html", noindex: true, body: `${docsHeader("/modisa/")}<main id="main" class="lost"><p class="crumb">404</p><h1>This one wandered off.</h1><p>The page isn't here. The rest of the herd is.</p><a class="btn lantern" href="/modisa/docs/introduction/">Open the docs</a></main>` }));
await Bun.write(`${out}/sitemap.xml`, `<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">${["", ...pages.map((p) => `docs/${p.slug}/`)].map((u) => `<url><loc>${site}${u}</loc></url>`).join("")}</urlset>`);

// For AI assistants and crawlers (llmstxt.org): llms.txt indexes the docs, llms-full.txt holds all of them as markdown.
const entities = (t: string) => t.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, "&");
const markdown = (html: string) => entities(html
  .replace(/<div class="code" data-lang="([^"]*)">[\s\S]*?<pre><code>([\s\S]*?)<\/code><\/pre><\/div>/g, (_, lang, body) => `\n\n\`\`\`${lang}\n${body}\n\`\`\`\n\n`)
  .replace(/<code>([\s\S]*?)<\/code>/g, "`$1`").replace(/<kbd>([\s\S]*?)<\/kbd>/g, "`$1`")
  .replace(/<a [^>]*href="([^"]+)"[^>]*>([\s\S]*?)<\/a>/g, (_, href, text) => `[${text}](${new URL(href, site + "docs/x/").href})`)
  .replace(/<(strong|b)>([\s\S]*?)<\/\1>/g, "**$2**").replace(/<li>/g, "\n- ").replace(/<\/(p|ul|ol|aside|table)>/g, "\n\n")
  .replace(/<\/thead>/g, (_, at: number, all: string) => `\n|${" --- |".repeat((all.slice(all.lastIndexOf("<thead>", at), at).match(/<th>/g) ?? []).length)}`)
  .replace(/<tr>/g, "\n|").replace(/<\/t[hd]>/g, " |")
  .replace(/<[^>]+>/g, "")).replace(/[ \t]+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim();
const groups = [...new Set(pages.map((p) => p.group))];
await Bun.write(`${out}/llms.txt`, `# modisa

> ${summary}

Install on macOS or Linux with \`curl -fsSL ${site}install.sh | sh\`, then run \`modisa\`. Source: ${repo}

${groups.map((g) => `## ${g}\n\n${pages.filter((p) => p.group === g).map((p) => `- [${p.title}](${site}docs/${p.slug}/): ${p.description}`).join("\n")}`).join("\n\n")}

## Optional

- [Everything above in one file](${site}llms-full.txt): all of the docs as markdown
- [Releases](${repo}/releases): binaries, checksums and packages
`);
await Bun.write(`${out}/llms-full.txt`, `# modisa\n\n> ${summary}\n\n` + pages.map((p) => `# ${p.title}\n\nSource: ${site}docs/${p.slug}/\n\n> ${p.description}\n\n${p.sections.map((x) => `## ${entities(x.title.replace(/<[^>]+>/g, ""))}\n\n${markdown(x.html)}`).join("\n\n")}`).join("\n\n---\n\n") + "\n");

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
