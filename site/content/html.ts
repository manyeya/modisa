// Small HTML helpers for the site's content modules.
export const escape = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

// A code block with its language and a copy button (site.js wires the button).
export const code = (lang: string, text: string) =>
  `<div class="code" data-lang="${lang}"><div class="code-bar"><span>${lang}</span><button type="button" data-copy aria-label="Copy ${lang} code">copy</button></div><pre><code>${escape(text.trim())}</code></pre></div>`;

export const p = (...paragraphs: string[]) => paragraphs.map((x) => `<p>${x}</p>`).join("");
export const ul = (items: string[]) => `<ul>${items.map((i) => `<li>${i}</li>`).join("")}</ul>`;
export const ol = (items: string[]) => `<ol>${items.map((i) => `<li>${i}</li>`).join("")}</ol>`;
export const kbd = (k: string) => `<kbd>${escape(k)}</kbd>`;
export const c = (s: string) => `<code>${escape(s)}</code>`;
export const note = (title: string, body: string) => `<aside class="note"><strong>${title}</strong><p>${body}</p></aside>`;

export function table(head: string[], rows: string[][]) {
  return `<div class="table"><table><thead><tr>${head.map((h) => `<th>${h}</th>`).join("")}</tr></thead><tbody>${rows.map((r) => `<tr>${r.map((d) => `<td>${d}</td>`).join("")}</tr>`).join("")}</tbody></table></div>`;
}

export type Section = { id: string; title: string; html: string };
export type Page = { slug: string; group: string; title: string; description: string; sections: Section[] };
