// Link handlers: which URLs a plugin's link entry takes. One module for the manifest check, the server's check on
// plugin.invoke and the TUI's chooser, so all three match the same way under the same limits.
//
// - `pattern`: a URL glob. `*` is any run of characters, everything else is literal; the scheme and host ignore case.
// - `regex`: RE2 syntax, run by re2js (a port of RE2 with its linear-time guarantee: no backreferences or lookaround),
//   found anywhere in the URL unless anchored. It sees the URL exactly as captured; `(?i)` makes it ignore case.
//
// Matching one URL against one entry is O(URL length × entry size): URLs are at most LINK.url characters, globs at most
// LINK.source, and a regex's compiled program at most LINK.program instructions (with RE2's own repeat cap of 1000 and
// nesting at most LINK.depth). With at most LINK.perPlugin entries a plugin, a click or an invoke does bounded work.
import { RE2JS } from "re2js";

export const LINK = { url: 2048, source: 500, program: 300, depth: 16, perPlugin: 8 };
export type LinkEntry = { pattern?: string; regex?: string; action: string };

const lowerOrigin = (s: string) => s.replace(/^[a-z][a-z0-9+.-]*:\/\/[^/?#]*/i, (origin) => origin.toLowerCase());
// the greedy wildcard match: at most pattern × URL steps
export function urlMatches(glob: string, link: string) {
  const pattern = lowerOrigin(glob);
  const url = lowerOrigin(link);
  let p = 0, u = 0, star = -1, resume = 0;
  while (u < url.length) {
    if (p < pattern.length && pattern[p] !== "*" && pattern[p] === url[u]) (p++, u++);
    else if (p < pattern.length && pattern[p] === "*") (star = p++, resume = u);
    else if (star >= 0) (p = star + 1, u = ++resume);
    else return false;
  }
  while (pattern[p] === "*") p++;
  return p === pattern.length;
}

// how deep a regex's groups nest, skipping escapes and character classes
// ponytail: a `]` first in a class is taken as its end; the compiled-size limit still bounds such a pattern
const nesting = (source: string) => {
  let deepest = 0, open = 0, inClass = false;
  for (let i = 0; i < source.length; i++) {
    const c = source[i];
    if (c === "\\") i++;
    else if (inClass) inClass = c !== "]";
    else if (c === "[") inClass = true;
    else if (c === "(") deepest = Math.max(deepest, ++open);
    else if (c === ")") open--;
  }
  return deepest;
};

// ponytail: never evicted; its entries come from manifests, a few per plugin
const compiled = new Map<string, RE2JS>();
const compile = (source: string) => {
  let re = compiled.get(source);
  if (!re) compiled.set(source, (re = RE2JS.compile(source)));
  return re;
};

export const globProblem = (pattern: string) =>
  /^https?:\/\//.test(pattern) ? undefined : "a link pattern is a URL glob starting with http:// or https://, like https://github.com/*/pull/*";

// why a regex can't be a link's, with what to change; undefined when it can
export function regexProblem(source: string) {
  if (source.length > LINK.source) return `a link regex is at most ${LINK.source} characters`;
  const depth = nesting(source);
  if (depth > LINK.depth) return `its groups nest ${depth} deep; the limit is ${LINK.depth}`;
  try {
    const size = compile(source).programSize();
    if (size > LINK.program) return `it compiles to ${size} instructions; the limit is ${LINK.program}: simplify it, or split it into several links`;
  } catch (e) {
    return `not a supported regular expression (RE2 syntax: no backreferences or lookaround): ${(e as Error).message}`;
  }
}

// Does this entry take this URL? Only http(s) URLs within the length limit are taken, and an entry that breaks its
// limits takes nothing.
export function linkMatches(entry: LinkEntry, url: string) {
  if (url.length > LINK.url || !/^https?:\/\//i.test(url)) return false;
  if (entry.pattern !== undefined) return entry.pattern.length <= LINK.source && !globProblem(entry.pattern) && urlMatches(entry.pattern, url);
  return entry.regex !== undefined && !regexProblem(entry.regex) && compile(entry.regex).test(url);
}
