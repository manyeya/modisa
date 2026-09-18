// `modisa plugin search [words]`: plugins published on GitHub with the modisa-tui-plugin topic, most starred first,
// each with the command that installs it. Nothing is installed or run, and nothing in the list is vetted.
// MODISA_PLUGIN_INDEX points the search at another API base (a mirror, a test server), or turns it "off".
import { cleanText } from "../core/text";

export const TOPIC = "modisa-tui-plugin";
const DEFAULT_INDEX = "https://api.github.com";

export type Found = { name: string; repo: string; url: string; description: string; stars: number; updated: string; created: string; archived: boolean; install: string };

// Text from the index is a stranger's: cleaned of escape, control and bidi characters, and cut to terminal cells.
const plain = (text: unknown, cells = 200) => cleanText(String(text ?? "").replace(/\s+/g, " "), cells).trim();

function failed(message: string, json: boolean) {
  console.error(json ? JSON.stringify({ error: { code: "unreachable", message } }) : `modisa: ${message}`);
  return 3;
}

// The plugins with the topic (and `words`), most starred first: what `plugin search` prints and the site's plugin
// directory lists. Throws with a message for the user when the index can't be read.
export async function find(words: string[] = [], limit = 30): Promise<{ total: number; results: Found[] }> {
  const base = (Bun.env.MODISA_PLUGIN_INDEX ?? DEFAULT_INDEX).replace(/\/+$/, "");
  if (base === "off") throw new Error("plugin search is turned off (MODISA_PLUGIN_INDEX=off)");
  const q = [`topic:${TOPIC}`, ...words].join(" ");
  const url = `${base}/search/repositories?q=${encodeURIComponent(q)}&sort=stars&order=desc&per_page=${limit}`;
  // a token only ever goes to GitHub itself, never to an overriding index
  const token = base === DEFAULT_INDEX ? Bun.env.GITHUB_TOKEN : undefined;
  const res = await fetch(url, {
    signal: AbortSignal.timeout(8000),
    headers: { accept: "application/vnd.github+json", "user-agent": "modisa", ...(token && { authorization: `Bearer ${token}` }) },
  }).catch((e: unknown) => (e instanceof Error ? e : new Error(String(e))));
  if (res instanceof Error) throw new Error(`couldn't reach the plugin index at ${base}: ${res.message}`);
  if (res.status === 403 || res.status === 429) throw new Error(`the plugin index at ${base} is rate-limiting searches: try again in a minute${base === DEFAULT_INDEX ? ", or set GITHUB_TOKEN" : ""}`);
  if (!res.ok) throw new Error(`the plugin index at ${base} answered ${res.status}`);
  const body = (await res.json().catch(() => undefined)) as { total_count?: number; items?: any[] } | undefined;
  if (!body || !Array.isArray(body.items)) throw new Error(`the plugin index at ${base} sent something that isn't a search result`);

  const results: Found[] = body.items
    .filter((r) => typeof r?.clone_url === "string" && r.clone_url.startsWith("https://")) // only what install can fetch as-is
    .map((r) => ({
      name: plain(r.name, 100),
      repo: plain(r.full_name, 200),
      url: plain(r.html_url, 300),
      description: plain(r.description),
      stars: Number.isFinite(r.stargazers_count) ? r.stargazers_count : 0,
      updated: plain(r.pushed_at, 40),
      created: plain(r.created_at, 40),
      archived: !!r.archived,
      install: `modisa plugin install ${plain(r.clone_url, 300)}`,
    }));
  return { total: Number.isFinite(body.total_count) ? body.total_count! : results.length, results };
}

export async function search(words: string[], json: boolean): Promise<number> {
  const query = words.join(" ").trim();
  let found: Awaited<ReturnType<typeof find>>;
  try {
    found = await find(words);
  } catch (e) {
    return failed((e as Error).message, json);
  }
  const { total, results } = found;

  if (json) {
    console.log(JSON.stringify({ query, total, results }, null, 2));
    return 0;
  }
  if (!results.length) {
    console.log(`no plugins with the ${TOPIC} topic${query ? ` matching "${plain(query)}"` : ""}`);
    return 0;
  }
  for (const r of results) {
    console.log(`${r.repo}  ★ ${r.stars}${r.archived ? "  (archived)" : ""}${r.updated ? `  updated ${r.updated.slice(0, 10)}` : ""}`);
    if (r.description) console.log(`  ${r.description}`);
    console.log(`  ${r.install}`);
  }
  console.log(`\nNone of these is vetted: a plugin runs as you, with your files and network. Read it before installing it.`);
  return 0;
}
