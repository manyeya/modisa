// Keeping shepherd current: the release manifest for this install's channel, a cached "is there a
// newer one?" for the TUI's update badge, and `shepherd update`, which replaces the binary with the
// new release after checking its SHA-256.
import { loadConfig } from "../config/config";
import { DIR, self } from "../core/paths";
import { installedBy } from "../core/install";
import { CHANNEL, FROM_SOURCE, REPO, VERSION, newer, platform } from "../core/version";
import { RESET, banner, bar, dim, fancy, gradient, megabytes, red, showCursor, spinner } from "./flair";

// What to run to update this install: its package manager's command, or `shepherd update`.
export const updateCommand = () => installedBy(self()[0]!, FROM_SOURCE).upgrade ?? "shepherd update";

export type Manifest = { version: string; channel: string; notes: string; assets: Record<string, { url: string; sha256: string }> };

const CACHE = `${DIR}/update.json`;
const EVERY = 6 * 60 * 60 * 1000;

// SHEPHERD_UPDATE_URL points elsewhere (a mirror, a test server) or turns checks "off".
export function manifestUrl(channel: string): string | undefined {
  const override = Bun.env.SHEPHERD_UPDATE_URL;
  if (override === "off") return;
  if (override) return override;
  return channel === "staging" ? `https://github.com/${REPO}/releases/download/staging/manifest.json` : `https://github.com/${REPO}/releases/latest/download/manifest.json`;
}

export function parseManifest(raw: any): Manifest | undefined {
  if (typeof raw?.version !== "string" || typeof raw.assets !== "object" || !raw.assets) return;
  const assets = Object.fromEntries(Object.entries(raw.assets).filter(([, a]: any) => typeof a?.url === "string" && /^[0-9a-f]{64}$/.test(a?.sha256)));
  return { version: raw.version, channel: String(raw.channel ?? "stable"), notes: String(raw.notes ?? ""), assets: assets as Manifest["assets"] };
}

async function fetchManifest(url: string): Promise<Manifest | undefined> {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(4000), redirect: "follow" });
    return res.ok ? parseManifest(await res.json()) : undefined;
  } catch {
    return undefined;
  }
}

// The release to move to, if there's a newer one for this install. Cached for 6h; never throws.
export async function checkForUpdate(force = false): Promise<Manifest | undefined> {
  const cfg = await loadConfig();
  if (!force && !cfg.update.check) return;
  if (FROM_SOURCE && !Bun.env.SHEPHERD_UPDATE_URL) return; // a checkout updates with git pull
  const url = manifestUrl(CHANNEL === "staging" ? "staging" : cfg.update.channel);
  if (!url) return;
  const cached = await Bun.file(CACHE).json().catch(() => undefined);
  let m: Manifest | undefined = !force && cached?.url === url && Date.now() - cached.at < EVERY ? parseManifest(cached.manifest) : undefined;
  if (!m) {
    m = await fetchManifest(url);
    if (m) await Bun.write(CACHE, JSON.stringify({ url, at: Date.now(), manifest: m })).catch(() => {});
  }
  return m && newer(m.version, VERSION) ? m : undefined;
}

export async function runVersion(): Promise<number> {
  console.log(`shepherd ${VERSION}${CHANNEL === "staging" ? " (staging)" : ""}${FROM_SOURCE ? " (from source)" : ""}`);
  const m = await checkForUpdate();
  if (m) console.log(`update available: ${m.version} — run \`${updateCommand()}\``);
  return 0;
}

// The release file, read as it streams in, telling `progress` how much has arrived (and of how much, when known).
async function download(url: string, progress: (got: number, total: number) => void = () => {}) {
  const res = await fetch(url, { redirect: "follow" }).catch((e: Error) => {
    throw new Error(`download failed: ${e.message}`);
  });
  if (!res.ok || !res.body) throw new Error(`download failed: ${res.status}`);
  const total = Number(res.headers.get("content-length")) || 0;
  const chunks: Uint8Array[] = [];
  let got = 0;
  for await (const chunk of res.body) {
    chunks.push(chunk);
    got += chunk.length;
    progress(got, total);
  }
  const bytes = new Uint8Array(got);
  let at = 0;
  for (const chunk of chunks) (bytes.set(chunk, at), (at += chunk.length));
  return bytes;
}

// What `shepherd update` shows: plain lines, or on a colour terminal the wordmark, spinners and a download bar.
type Reporter = {
  banner: (subtitle: string) => Promise<void>;
  step: <T>(label: string, work: () => Promise<T>, verdict?: (result: T) => string | false) => Promise<T>;
  download: (label: string, url: string) => Promise<Uint8Array>;
  say: (message: string) => void;
  fail: (message: string) => void;
  finale: (main: string, detail: string, notes: string[]) => void;
};

const plainReporter: Reporter = {
  banner: async () => {},
  step: (_label, work) => work(),
  download: (label, url) => (console.log(`${label}…`), download(url)),
  say: (message) => console.log(message),
  fail: (message) => console.error(message),
  finale: (main, detail, notes) => {
    console.log(`${main}. ${detail}`);
    if (notes.length) console.log(`\n${notes.join("\n")}`);
  },
};

const fancyReporter: Reporter = {
  banner,
  step: async (label, work, verdict) => {
    const s = spinner(label);
    try {
      const result = await work();
      const said = verdict?.(result);
      if (said === false) s.fail(label);
      else s.done(said ?? label);
      return result;
    } catch (e) {
      s.fail(label);
      throw e;
    }
  },
  download: async (label, url) => {
    const s = spinner(label);
    try {
      const bytes = await download(url, (got, total) =>
        s.update(total ? `  ${bar(got / total)} ${String(Math.floor((got / total) * 100)).padStart(3)}%  ${dim(`${megabytes(got)} / ${megabytes(total)}`)}` : `  ${dim(megabytes(got))}`),
      );
      s.done(`${label}  ${dim(megabytes(bytes.length))}`);
      return bytes;
    } catch (e) {
      s.fail(label);
      throw e;
    }
  },
  say: (message) => console.log(`  ${dim(message)}`),
  fail: (message) => console.error(`  ${red()}error:${RESET} ${message}`),
  finale: (main, detail, notes) => {
    console.log(`\n  ${gradient(`✓ ${main}`)}\n  ${dim(detail)}`);
    if (notes.length) console.log(`\n${notes.map((note) => `  ${dim(note)}`).join("\n")}`);
    console.log();
  },
};

export async function runUpdate(): Promise<number> {
  try {
    return await update(fancy() ? fancyReporter : plainReporter);
  } finally {
    showCursor();
  }
}

async function update(ui: Reporter): Promise<number> {
  if (FROM_SOURCE) {
    ui.say(`shepherd ${VERSION} runs from source: update it with git pull`);
    return 0;
  }
  // replacing a binary a package manager owns would leave the manager's records wrong
  const how = installedBy(self()[0]!, FROM_SOURCE);
  if (how.upgrade) {
    ui.fail(`shepherd was installed with ${how.manager}: update it with \`${how.upgrade}\`, then \`shepherd restart\``);
    return 1;
  }
  const plat = platform();
  if (!plat) {
    ui.fail("there's no shepherd release for this platform; run it from source");
    return 1;
  }
  await ui.banner(`shepherd ${VERSION} · ${plat}`);
  const m = await ui.step("looking for a newer release", () => checkForUpdate(true), (found) => (found ? `found shepherd ${found.version}` : "no newer release"));
  if (!m) {
    ui.say(`shepherd ${VERSION} is up to date`);
    return 0;
  }
  const asset = m.assets[plat];
  if (!asset) {
    ui.fail(`shepherd ${m.version} has no build for ${plat}`);
    return 1;
  }
  let bytes: Uint8Array;
  try {
    bytes = await ui.download(`downloading shepherd ${m.version} for ${plat}`, asset.url);
  } catch (e) {
    ui.fail((e as Error).message);
    return 1;
  }
  const matches = await ui.step("verifying its SHA-256 checksum", async () => new Bun.CryptoHasher("sha256").update(bytes).digest("hex") === asset.sha256, (ok) => (ok ? "checksum verified" : false));
  if (!matches) {
    ui.fail("the download doesn't match its published checksum; nothing was changed");
    return 1;
  }
  const target = self()[0]!;
  const tmp = `${target}.update-${Date.now()}`; // same directory, so the rename is atomic
  const moved = await ui.step(
    `replacing ${target}`,
    async () => {
      await Bun.write(tmp, bytes);
      return Bun.$`chmod 755 ${tmp} && mv -f ${tmp} ${target}`.quiet().nothrow();
    },
    (r) => (r.exitCode === 0 ? `replaced ${target}` : false),
  );
  if (moved.exitCode !== 0) {
    await Bun.file(tmp).delete().catch(() => {});
    ui.fail(`couldn't replace ${target}: ${moved.stderr.toString().trim()}`);
    return 1;
  }
  ui.finale(`updated shepherd ${VERSION} → ${m.version}`, "Run `shepherd restart` to load it into running sessions.", m.notes.trim().split("\n").filter(Boolean).slice(0, 8));
  return 0;
}
