// Keeping shepherd current: the release manifest for this install's channel, a cached "is there a
// newer one?" for the TUI's update badge, and `shepherd update`, which replaces the binary with the
// new release after checking its SHA-256.
import { loadConfig } from "../config/config";
import { DIR, self } from "../core/paths";
import { installedBy } from "../core/install";
import { CHANNEL, FROM_SOURCE, REPO, VERSION, newer, platform } from "../core/version";

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

export async function runUpdate(): Promise<number> {
  if (FROM_SOURCE) {
    console.log(`shepherd ${VERSION} runs from source: update it with git pull`);
    return 0;
  }
  // replacing a binary a package manager owns would leave the manager's records wrong
  const how = installedBy(self()[0]!, FROM_SOURCE);
  if (how.upgrade) {
    console.error(`shepherd was installed with ${how.manager}: update it with \`${how.upgrade}\`, then \`shepherd restart\``);
    return 1;
  }
  const plat = platform();
  if (!plat) {
    console.error("there's no shepherd release for this platform; run it from source");
    return 1;
  }
  const m = await checkForUpdate(true);
  if (!m) {
    console.log(`shepherd ${VERSION} is up to date`);
    return 0;
  }
  const asset = m.assets[plat];
  if (!asset) {
    console.error(`shepherd ${m.version} has no build for ${plat}`);
    return 1;
  }
  console.log(`downloading shepherd ${m.version} for ${plat}…`);
  const res = await fetch(asset.url, { redirect: "follow" }).catch((e) => e as Error);
  if (!(res instanceof Response) || !res.ok) {
    console.error(`download failed: ${res instanceof Response ? res.status : res.message}`);
    return 1;
  }
  const bytes = new Uint8Array(await res.arrayBuffer());
  if (new Bun.CryptoHasher("sha256").update(bytes).digest("hex") !== asset.sha256) {
    console.error("the download doesn't match its published checksum; nothing was changed");
    return 1;
  }
  const target = self()[0]!;
  const tmp = `${target}.update-${Date.now()}`; // same directory, so the rename is atomic
  await Bun.write(tmp, bytes);
  const moved = await Bun.$`chmod 755 ${tmp} && mv -f ${tmp} ${target}`.quiet().nothrow();
  if (moved.exitCode !== 0) {
    await Bun.file(tmp).delete().catch(() => {});
    console.error(`couldn't replace ${target}: ${moved.stderr.toString().trim()}`);
    return 1;
  }
  console.log(`updated shepherd ${VERSION} → ${m.version}. Run \`shepherd restart\` to load it into running sessions.`);
  const notes = m.notes.trim().split("\n").filter(Boolean).slice(0, 8);
  if (notes.length) console.log(`\n${notes.join("\n")}`);
  return 0;
}
