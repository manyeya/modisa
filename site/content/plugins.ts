// The plugin directory: every public repository with the modisa-tui-plugin topic, as found when the site was built
// (site/build.ts refreshes it daily). The list is real HTML, so it reads without JavaScript; site.js adds the search
// box and the sort. Everything from GitHub is a stranger's text: already cleaned by find(), and escaped here.
import type { Found } from "../../src/cli/plugin-search";
import { TOPIC } from "../../src/cli/plugin-search";
import { c, code, escape } from "./html";

const day = (iso: string) => escape(iso.slice(0, 10));

function card(p: Found) {
  const owner = p.repo.split("/")[0] ?? "";
  const install = p.install;
  const text = `${p.name} ${p.repo} ${p.description}`.toLowerCase();
  return `<article class="plugin" data-name="${escape(p.name.toLowerCase())}" data-stars="${p.stars}" data-updated="${escape(p.updated)}" data-created="${escape(p.created)}" data-text="${escape(text)}">
  <h3><a href="${escape(p.url)}" rel="noopener">${escape(p.name)}</a><span>${escape(owner)}</span></h3>
  <p>${p.description ? escape(p.description) : '<span class="plugin-none">No description.</span>'}</p>
  <div class="plugin-install"><code>${escape(install)}</code><button type="button" class="copy" data-copy-text="${escape(install)}" aria-label="Copy the install command for ${escape(p.name)}">Copy</button></div>
  <p class="plugin-meta"><span>★ ${p.stars}</span>${p.updated ? `<span>updated ${day(p.updated)}</span>` : ""}${p.archived ? `<span class="plugin-archived">archived</span>` : ""}</p>
</article>`;
}

export function pluginsPage(o: { found: Found[]; built: string }) {
  const { found } = o;
  const fresh = [...found].sort((a, b) => b.created.localeCompare(a.created)).slice(0, 3);
  return `<main id="main" class="doc plugins">
  <p class="crumb">PLUGINS</p>
  <h1>Plugins</h1>
  <p class="doc-lede">What people have built on modisa: sidebars, status segments, popups and actions, installed with one command. Found automatically on GitHub by the ${c(TOPIC)} topic, and refreshed daily.</p>
  <aside class="note"><strong>Read before you install.</strong><p>Nobody reviews these. A plugin runs as you, with your files and network: look at its code first.</p></aside>
${found.length > 6 ? `  <section class="plugin-fresh" aria-labelledby="fresh"><h2 id="fresh">New</h2><div class="plugin-grid">${fresh.map(card).join("")}</div></section>` : ""}
  <section aria-labelledby="browse">
    <h2 id="browse">${found.length > 6 ? "Browse everything" : "Browse"}</h2>
    <div class="plugin-tools" hidden>
      <label><span>Search plugins</span><input type="search" id="plugin-q" placeholder="Search by name or description" autocomplete="off"></label>
      <label><span>Sort</span><select id="plugin-sort"><option value="stars">Most starred</option><option value="updated">Recently updated</option><option value="created">Newest</option><option value="name">Name</option></select></label>
    </div>
    <p class="plugin-count" id="plugin-count" aria-live="polite">${found.length} plugin${found.length === 1 ? "" : "s"}</p>
    <div class="plugin-grid" id="plugin-list">${found.map(card).join("")}</div>
    <p class="plugin-empty" id="plugin-empty"${found.length ? " hidden" : ""}>${found.length ? "No plugin matches that." : "None yet: yours could be the first."}</p>
  </section>
  <section aria-labelledby="publish">
    <h2 id="publish">Publish yours</h2>
    <ol>
      <li>Write it: ${c("modisa plugin new my-plugin")} scaffolds one, and the <a href="../docs/plugins/">plugin docs</a> cover the rest.</li>
      <li>Push it to a public GitHub repository with ${c("plugin.json")} at the top.</li>
      <li>Give the repository the ${c(TOPIC)} topic. It shows up here within a day, and in ${c("modisa plugin search")} right away.</li>
    </ol>
    ${code("sh", `gh repo edit --add-topic ${TOPIC}`)}
  </section>
  <p class="plugin-built">Last refreshed ${escape(o.built)}.</p>
</main>`;
}
