// Linked plugins: ~/.config/shepherd/plugins/<name> links to a directory holding a plugin.json. Plugins fetched with
// `shepherd plugin install` live under <state>/plugins-src/<name>: the checkout, and shepherd's own install record.
import { CONFIG_DIR } from "./config";
import { DIR } from "../core/paths";
import { pluginManifest, type PluginManifest } from "../protocol/schema";

export const PLUGINS_DIR = `${CONFIG_DIR}/plugins`;
export const MANAGED_DIR = `${DIR}/plugins-src`;

// Written by `plugin install` next to (not inside) the checkout. It's the only thing that makes a checkout shepherd's
// to delete: a manifest or a path can't claim that.
export type InstallRecord = { name: string; source: string; ref: string | null; commit: string; checkout: string; dir: string; subdir: string | null; installedAt: string };
export const readInstall = (name: string): Promise<InstallRecord | undefined> => Bun.file(`${MANAGED_DIR}/${name}/install.json`).json().catch(() => undefined);

// user:password@ or token@ in a URL is never recorded or shown
export const withoutCredentials = (url: string) => url.replace(/^([a-z][a-z0-9+.-]*:\/\/)[^/@]*@/i, "$1");
export type Linked = { name: string; dir: string; manifest?: PluginManifest; error?: string };

// plugin.json, validated; an error that says what to fix otherwise
export async function readManifest(dir: string): Promise<{ manifest?: PluginManifest; error?: string }> {
  const file = Bun.file(`${dir}/plugin.json`);
  if (!(await file.exists())) return { error: `no plugin.json in ${dir}` };
  let raw: unknown;
  try {
    raw = await file.json();
  } catch (e) {
    return { error: `${dir}/plugin.json isn't valid JSON: ${(e as Error).message}` };
  }
  const r = pluginManifest.safeParse(raw);
  if (r.success) return { manifest: r.data };
  return { error: `${dir}/plugin.json: ${r.error.issues.map((i) => `${i.path.join(".") || "(top level)"}: ${i.message}`).join("; ")}` };
}

export async function linkedPlugins(): Promise<Linked[]> {
  // Bun.Glob doesn't follow links to directories the way we want here, so list with the shell
  const names = (await Bun.$`ls -1 ${PLUGINS_DIR}`.quiet().nothrow().text()).split("\n").filter(Boolean).sort();
  return Promise.all(
    names.map(async (name): Promise<Linked> => {
      const dir = (await Bun.$`realpath ${PLUGINS_DIR}/${name}`.quiet().nothrow().text()).trim();
      if (!dir) return { name, dir: `${PLUGINS_DIR}/${name}`, error: "the link points nowhere (was the directory moved?)" };
      const { manifest, error } = await readManifest(dir);
      if (manifest && manifest.name !== name) return { name, dir, error: `linked as ${name}, but plugin.json names it ${manifest.name}` };
      return { name, dir, manifest, error };
    }),
  );
}
