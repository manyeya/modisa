// The version this push releases, for the release workflow (appends key=value lines to GITHUB_OUTPUT,
// or prints them). Versions come from git tags: the first release is package.json's version; after
// that every push to main bumps the patch, or the minor/major when a commit since the last release
// says [minor]/[major]. Staging builds are the next version plus -staging.<run>.
//   bun .github/scripts/next-version.ts <stable|staging> <run number>
import pkg from "../../package.json";

const [channel = "stable", run = "0"] = Bun.argv.slice(2);
const git = async (...args: string[]) => (await Bun.$`git ${args}`.quiet().nothrow().text()).trim();

const last = (await git("tag", "--list", "v*", "--sort=-v:refname")).split("\n").find((t) => /^v\d+\.\d+\.\d+$/.test(t));
const range = last ? `${last}..HEAD` : "HEAD";
const bodies = await git("log", "--format=%B", range);
let [major, minor, patch] = (last ? last.slice(1) : pkg.version).split(".").map(Number) as [number, number, number];
if (last) {
  if (/\[major\]/i.test(bodies)) [major, minor, patch] = [major + 1, 0, 0];
  else if (/\[minor\]/i.test(bodies)) [minor, patch] = [minor + 1, 0];
  else patch++;
}
const base = `${major}.${minor}.${patch}`;
const version = channel === "staging" ? `${base}-staging.${run}` : base;
// only code changes make a release; a docs-only push just redeploys the site
const changed = !last || (await Bun.$`git diff --quiet ${last} HEAD -- src package.json bun.lock tsconfig.json`.quiet().nothrow()).exitCode !== 0;
const notes = (await git("log", "--no-merges", "--format=- %s", range)).split("\n").filter(Boolean).slice(0, 60).join("\n") || "- Maintenance";

const out = [`version=${version}`, `tag=${channel === "staging" ? "staging" : `v${version}`}`, `channel=${channel}`, `changed=${changed}`, `last=${last ?? ""}`, `notes<<NOTES_END`, notes, "NOTES_END"].join("\n") + "\n";
if (Bun.env.GITHUB_OUTPUT) await Bun.write(Bun.env.GITHUB_OUTPUT, (await Bun.file(Bun.env.GITHUB_OUTPUT).text().catch(() => "")) + out);
else await Bun.write(Bun.stdout, out);
