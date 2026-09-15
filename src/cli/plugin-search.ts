// `shepherd plugin search [words]`: plugins published on GitHub with the shepherd-plugin topic, most starred first,
// each with the command that installs it. Nothing is installed or run, and nothing in the list is vetted.
// SHEPHERD_PLUGIN_INDEX points the search at another API base (a mirror, a test server), or turns it "off".
export const TOPIC = "shepherd-plugin";
const DEFAULT_INDEX = "https://api.github.com";

export type Found = { name: string; repo: string; url: string; description: string; stars: number; updated: string; archived: boolean; install: string };

// Text from the index is a stranger's: no control or bidirectional-override characters reach the terminal.
const plain = (text: unknown, max = 200) => String(text ?? "").replace(/[\x00-\x1f\x7f-\x9f​-‏‪-‮⁦-⁩]/g, " ").trim().slice(0, max);

function failed(message: string, json: boolean) {
  console.error(json ? JSON.stringify({ error: { code: "unreachable", message } }) : `shepherd: ${message}`);
  return 3;
}

export async function search(words: string[], json: boolean): Promise<number> {
  const base = (Bun.env.SHEPHERD_PLUGIN_INDEX ?? DEFAULT_INDEX).replace(/\/+$/, "");
  if (base === "off") return failed("plugin search is turned off (SHEPHERD_PLUGIN_INDEX=off)", json);
  const query = words.join(" ").trim();
  const q = [`topic:${TOPIC}`, ...words].join(" ");
  const url = `${base}/search/repositories?q=${encodeURIComponent(q)}&sort=stars&order=desc&per_page=30`;
  // a token only ever goes to GitHub itself, never to an overriding index
  const token = base === DEFAULT_INDEX ? Bun.env.GITHUB_TOKEN : undefined;
  const res = await fetch(url, {
    signal: AbortSignal.timeout(8000),
    headers: { accept: "application/vnd.github+json", "user-agent": "shepherd", ...(token && { authorization: `Bearer ${token}` }) },
  }).catch((e: unknown) => (e instanceof Error ? e : new Error(String(e))));
  if (res instanceof Error) return failed(`couldn't reach the plugin index at ${base}: ${res.message}`, json);
  if (res.status === 403 || res.status === 429) return failed(`the plugin index at ${base} is rate-limiting searches: try again in a minute${base === DEFAULT_INDEX ? ", or set GITHUB_TOKEN" : ""}`, json);
  if (!res.ok) return failed(`the plugin index at ${base} answered ${res.status}`, json);
  const body = (await res.json().catch(() => undefined)) as { total_count?: number; items?: any[] } | undefined;
  if (!body || !Array.isArray(body.items)) return failed(`the plugin index at ${base} sent something that isn't a search result`, json);

  const results: Found[] = body.items
    .filter((r) => typeof r?.clone_url === "string" && r.clone_url.startsWith("https://")) // only what install can fetch as-is
    .map((r) => ({
      name: plain(r.name, 100),
      repo: plain(r.full_name, 200),
      url: plain(r.html_url, 300),
      description: plain(r.description),
      stars: Number.isFinite(r.stargazers_count) ? r.stargazers_count : 0,
      updated: plain(r.pushed_at, 40),
      archived: !!r.archived,
      install: `shepherd plugin install ${plain(r.clone_url, 300)}`,
    }));

  if (json) {
    console.log(JSON.stringify({ query, total: Number.isFinite(body.total_count) ? body.total_count : results.length, results }, null, 2));
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
