// Marketplaces: git repositories that list plugins, in a modisa-marketplace.json at the top (or
// .modisa/marketplace.json). `modisa plugin marketplace add owner/repo` clones one into <state>/marketplaces/<name>,
// with install's git policy, and records it in <state>/marketplaces.json; `plugin install <plugin>@<marketplace>`
// installs a plugin it lists. A marketplace is a list someone keeps, not a review: its plugins run as you.
import { z } from "zod";
import { rename } from "node:fs/promises";
import { DIR } from "../core/paths";
import { cleanText } from "../core/text";
import { checkoutRef, fetchLatest, git, sourceProblem } from "../cli/plugin-git";
import { inside, installs, isDir, linkedPlugins, real, withoutCredentials } from "./plugins";
import type { marketplaceResults } from "../protocol/schema";

export const MARKETPLACES_DIR = `${DIR}/marketplaces`;
const REGISTRY = `${DIR}/marketplaces.json`;
const FILES = ["modisa-marketplace.json", ".modisa/marketplace.json"];
const MAX_BYTES = 1024 * 1024;

// ---------- the file: a stranger's, so every name is checked and every text cleaned ----------
const id = z.string().max(64).regex(/^[a-z0-9][a-z0-9-]*$/, "use lowercase letters, digits and dashes");
const text = (cells: number) => z.string().transform((s) => cleanText(s.replace(/\s+/g, " "), cells).trim());
const GITHUB = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;
const SOURCE = 'a source is "./path" in this repository, "owner/repo" on GitHub, or { "git": url, "ref", "subdir" }';
// what goes to git or names a path: no option, whitespace or control character
const unsafe = (s: string) => (s.startsWith("-") || /[\s\x00-\x1f\x7f-\x9f]/.test(s) ? `${JSON.stringify(cleanText(s, 60))} can't start with - or hold spaces or control characters` : undefined);
type Source = string | { git: string; ref?: string; subdir?: string };
const sourceProblemOf = (s: Source) => {
  if (typeof s === "string") return s.startsWith("./") ? unsafe(s) : GITHUB.test(s) ? undefined : SOURCE;
  const subdir = s.subdir && (unsafe(s.subdir) ?? (s.subdir.startsWith("/") || s.subdir.split("/").includes("..") ? "subdir must be a relative path inside the repository" : undefined));
  return unsafe(s.git) ?? sourceProblem(s.git) ?? (s.ref && unsafe(s.ref)) ?? subdir;
};
const entry = z
  .object({
    name: id,
    description: text(200).default(""),
    source: z.union([z.string().max(500), z.object({ git: z.string().max(500), ref: z.string().min(1).max(200).optional(), subdir: z.string().min(1).max(300).optional() })], { error: SOURCE }),
  })
  .superRefine((e, ctx) => {
    const why = sourceProblemOf(e.source);
    if (why) ctx.addIssue({ code: "custom", path: ["source"], message: why });
  });
export const marketplaceFile = z
  .object({
    name: id,
    description: text(200).optional(),
    owner: z.union([text(100), z.object({ name: text(100) }).transform((o) => o.name)]).optional(), // or Claude Code's { name, email }
    plugins: z.array(entry).max(1000),
  })
  .superRefine((m, ctx) => {
    const seen = new Set<string>();
    m.plugins.forEach((p, i) => (seen.has(p.name) ? ctx.addIssue({ code: "custom", path: ["plugins", i, "name"], message: `a second plugin named ${p.name}` }) : seen.add(p.name)));
  });
export type MarketplaceFile = z.output<typeof marketplaceFile>;

// A checkout's marketplace file, validated, and itself inside the checkout
export async function readMarketplace(dir: string): Promise<{ file?: MarketplaceFile; error?: string }> {
  const root = await real(dir);
  if (!root) return { error: `its checkout is gone (${dir})` };
  for (const name of FILES) {
    const path = await real(`${dir}/${name}`);
    if (!path) continue;
    if (!inside(path, root)) return { error: `${name} points outside the repository` };
    if (Bun.file(path).size > MAX_BYTES) return { error: `${name} is over ${MAX_BYTES / 1024 / 1024} MB` };
    let raw: unknown;
    try {
      raw = await Bun.file(path).json();
    } catch (e) {
      return { error: cleanText(`${name} isn't valid JSON: ${(e as Error).message}`, 300) };
    }
    const r = marketplaceFile.safeParse(raw);
    if (r.success) return { file: r.data };
    return { error: cleanText(`${name}: ${r.error.issues.map((i) => `${i.path.join(".") || "(top level)"}: ${i.message}`).join("; ")}`, 500) };
  }
  return { error: `no ${FILES.join(" or ")} at the top of the repository` };
}

// ---------- the registry: which are added, from where, at which commit ----------
export type Marketplace = { name: string; source: string; ref: string | null; commit: string; addedAt: string; updatedAt: string };

// a name that isn't a plain id never makes it into a path
export async function registry(): Promise<Marketplace[]> {
  const list = await Bun.file(REGISTRY).json().catch(() => []);
  return Array.isArray(list) ? list.filter((m) => id.safeParse(m?.name).success && typeof m.source === "string") : [];
}
async function save(list: Marketplace[]) {
  await Bun.$`mkdir -p ${DIR}`.quiet();
  const tmp = `${REGISTRY}.${crypto.randomUUID().slice(0, 8)}.tmp`;
  await Bun.write(tmp, JSON.stringify(list, null, 2) + "\n");
  await rename(tmp, REGISTRY); // whole or not at all
}
const dirOf = (name: string) => `${MARKETPLACES_DIR}/${name}`;
const changed = async (name: string, update: Partial<Marketplace>) => save((await registry()).map((m) => (m.name === name ? { ...m, ...update } : m)));

// owner/repo is GitHub's; anything else is a git URL as install takes them
export const marketplaceUrl = (arg: string) => (GITHUB.test(arg) && !arg.startsWith(".") ? `https://github.com/${arg}.git` : arg);

type Added = z.infer<typeof marketplaceResults.add>;
type Updated = z.infer<typeof marketplaceResults.update>[number];

// Clone it into a staging directory, check its file, and only then take its name. A failure leaves nothing behind.
export async function addMarketplace(arg: string, ref?: string): Promise<Added> {
  const url = marketplaceUrl(arg);
  const source = withoutCredentials(url);
  let staging: string | undefined;
  const failed = async (stage: NonNullable<Added["stage"]>, reason: string): Promise<Added> => {
    if (staging) await Bun.$`rm -rf ${staging}`.quiet().nothrow();
    return { added: false, source, ref: ref ?? null, stage, reason: withoutCredentials(reason) };
  };
  const problem = sourceProblem(url);
  if (problem) return failed("source", problem);
  if (!Bun.which("git")) return failed("git", "git isn't installed");
  if (ref && unsafe(ref)) return failed("ref", `not a ref: ${ref}`);

  await Bun.$`mkdir -p ${MARKETPLACES_DIR}`.quiet();
  staging = (await Bun.$`mktemp -d ${`${MARKETPLACES_DIR}/.staging-XXXXXX`}`.text()).trim();
  const checkout = `${staging}/checkout`;
  const cloned = await git(["clone", "--quiet", "--", url, checkout]);
  if (cloned.code !== 0) return failed("clone", cloned.err || "git clone failed");
  await git(["remote", "set-url", "origin", source], checkout); // no credentials left in the checkout's own config
  const at = await checkoutRef(checkout, ref ?? null, source);
  if (!at.commit) return failed("ref", at.reason!);
  const { file, error } = await readMarketplace(checkout);
  if (!file) return failed("manifest", error!);
  const about = { description: file.description, plugins: file.plugins.map((p) => p.name) };

  const existing = (await registry()).find((m) => m.name === file.name);
  if (existing?.source === source) {
    await Bun.$`rm -rf ${staging}`.quiet().nothrow();
    return { added: false, alreadyAdded: true, name: file.name, source, ref: existing.ref, commit: existing.commit, ...about };
  }
  if (existing) return failed("collision", `a marketplace named ${file.name} is already added from ${existing.source}; modisa plugin marketplace remove ${file.name} first`);
  // rename won't replace a directory with anything in it, so two adds racing for the name can't both get it
  if (!(await rename(checkout, dirOf(file.name)).then(() => true, () => false))) return failed("collision", `${dirOf(file.name)} is already there`);
  await Bun.$`rm -rf ${staging}`.quiet().nothrow();
  const now = new Date().toISOString();
  await save([...(await registry()).filter((m) => m.name !== file.name), { name: file.name, source, ref: ref ?? null, commit: at.commit, addedAt: now, updatedAt: now }]);
  return { added: true, name: file.name, source, ref: ref ?? null, commit: at.commit, ...about };
}

export async function listMarketplaces() {
  return Promise.all(
    (await registry()).map(async (m) => {
      const { file, error } = await readMarketplace(dirOf(m.name));
      return { ...m, ...(file?.description && { description: file.description }), ...(file?.owner && { owner: file.owner }), plugins: file?.plugins.length ?? 0, ...(error && { error }) };
    }),
  );
}

// Each one (or the one named) moved to what its ref (none: its source's HEAD) is at now. A new commit whose file
// doesn't check out, or that renames the marketplace, is undone: it stays where it was.
export async function updateMarketplaces(name?: string): Promise<Updated[]> {
  const chosen = (await registry()).filter((m) => !name || m.name === name);
  if (name && !chosen.length) throw new Error(`no marketplace named ${name} (modisa plugin marketplace list)`);
  const out: Updated[] = [];
  for (const m of chosen) {
    const dir = dirOf(m.name);
    const left = (reason: string) => ({ name: m.name, updated: false, from: m.commit, commit: m.commit, reason: withoutCredentials(reason) });
    const latest = await fetchLatest(dir, m.source, m.ref);
    if (!latest.commit) {
      out.push(left(latest.reason!));
      continue;
    }
    if (latest.commit === m.commit) {
      out.push({ name: m.name, updated: false, from: m.commit, commit: m.commit, plugins: (await readMarketplace(dir)).file?.plugins.length ?? 0 });
      continue;
    }
    const moved = await git(["checkout", "--quiet", "--detach", latest.commit], dir);
    if (moved.code !== 0) {
      out.push(left(moved.err || `couldn't check out ${latest.commit}`));
      continue;
    }
    const { file, error } = await readMarketplace(dir);
    if (!file || file.name !== m.name) {
      await git(["checkout", "--quiet", "--detach", m.commit], dir);
      out.push(left(file ? `it calls itself ${file.name} now: remove it and add it again` : `at ${latest.commit.slice(0, 7)}: ${error}`));
      continue;
    }
    await changed(m.name, { commit: latest.commit, updatedAt: new Date().toISOString() });
    out.push({ name: m.name, updated: true, from: m.commit, commit: latest.commit, plugins: file.plugins.length });
  }
  return out;
}

// Its checkout and its record go; plugins installed from it stay installed (`installed` names them)
export async function removeMarketplace(name: string) {
  const list = await registry();
  const m = list.find((x) => x.name === name);
  if (!m) throw new Error(`no marketplace named ${name} (modisa plugin marketplace list)`);
  await save(list.filter((x) => x !== m));
  await Bun.$`rm -rf ${dirOf(m.name)}`.quiet();
  return { name: m.name, removed: true as const, dir: dirOf(m.name), installed: (await installs()).filter((r) => r.marketplace === m.name).map((r) => r.name) };
}

// ---------- the plugins they list ----------
// Where a listed plugin's code comes from, as shown: the marketplace's own repository for one inside it
const shown = (source: Source, m: Marketplace) =>
  typeof source !== "string" ? { from: withoutCredentials(source.git), ref: source.ref ?? null, subdir: source.subdir ?? null }
  : source.startsWith("./") ? { from: m.source, ref: m.ref, subdir: source.slice(2).replace(/\/+$/, "") || null }
  : { from: marketplaceUrl(source), ref: null, subdir: null };

// Every plugin the marketplaces list that matches every word (none: all of them); installed: one by that name is linked
export async function marketplacePlugins(words: string[] = []) {
  const linked = new Set((await linkedPlugins()).map((l) => l.name));
  const want = words.map((w) => w.toLowerCase());
  const out = [];
  for (const m of await registry()) {
    for (const p of (await readMarketplace(dirOf(m.name))).file?.plugins ?? []) {
      if (!want.every((w) => `${p.name} ${p.description} ${m.name}`.toLowerCase().includes(w))) continue;
      out.push({ name: p.name, marketplace: m.name, description: p.description, ...shown(p.source, m), install: `modisa plugin install ${p.name}@${m.name}`, installed: linked.has(p.name) });
    }
  }
  return out;
}

// What installs <plugin>@<marketplace>: a git URL, ref and subdir. A plugin inside the marketplace's repository comes
// from its checkout here (a file:// URL, at the commit the marketplace is at), from a directory that's really inside it.
export async function resolveEntry(spec: string) {
  const at = spec.lastIndexOf("@");
  const [name, market] = [spec.slice(0, at), spec.slice(at + 1)];
  if (at <= 0 || !id.safeParse(market).success) throw new Error(`not <plugin>@<marketplace>: ${cleanText(spec, 100)}`);
  const m = (await registry()).find((x) => x.name === market);
  if (!m) throw new Error(`no marketplace named ${market} (modisa plugin marketplace list)`);
  const dir = dirOf(m.name);
  const { file, error } = await readMarketplace(dir);
  if (!file) throw new Error(`marketplace ${m.name}: ${error}`);
  const p = file.plugins.find((x) => x.name === name);
  if (!p) throw new Error(`marketplace ${m.name} lists no plugin named ${cleanText(name, 64)}`);
  const base = { name: p.name, marketplace: m.name, ...shown(p.source, m) };
  if (typeof p.source !== "string") return { ...base, url: p.source.git };
  if (!p.source.startsWith("./")) return { ...base, url: base.from };
  const { root, subdir } = await within(dir, p.source);
  return { ...base, url: `file://${root}`, ref: null, subdir };
}

// A relative source's directory, which must really be inside the checkout at `dir` (no .., no symlink out): the
// checkout's real path, and the directory's path in it (null: the top).
export async function within(dir: string, source: string) {
  const root = await real(dir);
  const target = await real(`${dir}/${source}`);
  if (!root || !target || !inside(target, root) || !(await isDir(target))) throw new Error(`${cleanText(source, 100)} isn't a directory inside the marketplace's repository`);
  return { root, subdir: target === root ? null : target.slice(root.length + 1) };
}
