// Linked plugins: ~/.config/shepherd/plugins/<name> links to a directory holding a plugin.json.
import { CONFIG_DIR } from "./config";
import { pluginManifest, type PluginManifest } from "../protocol/schema";

export const PLUGINS_DIR = `${CONFIG_DIR}/plugins`;
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
