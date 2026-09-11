// Checksums and the update manifest for a release, next to the binaries the build jobs produced.
// `shepherd update`, the TUI's update badge and install.sh all read manifest.json.
//   bun .github/scripts/release-assets.ts <version> <channel> <tag> <dir> [notes file]
const [version, channel, tag, dir, notesFile] = Bun.argv.slice(2);
if (!version || !channel || !tag || !dir) throw new Error("usage: release-assets.ts <version> <channel> <tag> <dir> [notes file]");
const repo = Bun.env.GITHUB_REPOSITORY ?? "manyeya/shepherd";
const files = (await Array.fromAsync(new Bun.Glob("shepherd-*").scan({ cwd: dir }))).sort();
if (!files.length) throw new Error(`no shepherd-* binaries in ${dir}`);
const assets: Record<string, { url: string; sha256: string }> = {};
const sums: string[] = [];
for (const f of files) {
  const sha256 = new Bun.CryptoHasher("sha256").update(await Bun.file(`${dir}/${f}`).bytes()).digest("hex");
  assets[f.replace(/^shepherd-/, "")] = { url: `https://github.com/${repo}/releases/download/${tag}/${f}`, sha256 };
  sums.push(`${sha256}  ${f}`);
}
const notes = notesFile ? await Bun.file(notesFile).text() : "";
await Bun.write(`${dir}/SHA256SUMS`, sums.join("\n") + "\n");
// pretty-printed, one field per line: install.sh reads it with awk
await Bun.write(`${dir}/manifest.json`, JSON.stringify({ version, channel, notes, assets }, null, 2) + "\n");
console.log(`manifest for ${version} (${channel}): ${Object.keys(assets).join(", ")}`);
