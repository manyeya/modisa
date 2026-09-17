// Which modisa this is: the release version baked in at build time (--define BUILD_VERSION=…),
// or package.json's version + "-dev" when running from source; its update channel; and the platform
// name release assets use.
import pkg from "../../package.json";

declare const BUILD_VERSION: string;
export const VERSION: string = typeof BUILD_VERSION === "string" ? BUILD_VERSION : `${pkg.version}-dev`;
export const FROM_SOURCE = VERSION.endsWith("-dev");
export const CHANNEL: "stable" | "staging" = VERSION.includes("-staging") ? "staging" : "stable";
export const REPO = "manyeya/modisa";

// darwin-arm64, linux-x64, linux-arm64: the release assets there are (libghostty ships no Intel Mac build)
export function platform(): string | undefined {
  const [os, arch] = Bun.spawnSync(["uname", "-sm"]).stdout.toString().trim().split(/\s+/);
  const o = os === "Darwin" ? "darwin" : os === "Linux" ? "linux" : undefined;
  const a = arch === "arm64" || arch === "aarch64" ? "arm64" : arch === "x86_64" ? "x64" : undefined;
  const p = o && a ? `${o}-${a}` : undefined;
  return p === "darwin-x64" ? undefined : p;
}

// Semver order, prereleases included: 1.2.0-staging.3 < 1.2.0-staging.10 < 1.2.0; -dev sorts like any prerelease.
export function compare(a: string, b: string): number {
  const split = (v: string) => {
    const [core, pre] = v.replace(/^v/, "").split(/-(.*)/s);
    return { nums: core!.split(".").map((n) => Number(n) || 0), pre: pre ? pre.split(".") : [] };
  };
  const x = split(a), y = split(b);
  for (let i = 0; i < 3; i++) if ((x.nums[i] ?? 0) !== (y.nums[i] ?? 0)) return (x.nums[i] ?? 0) - (y.nums[i] ?? 0);
  if (!x.pre.length || !y.pre.length) return y.pre.length - x.pre.length; // a release beats its prereleases
  for (let i = 0; i < Math.max(x.pre.length, y.pre.length); i++) {
    const p = x.pre[i], q = y.pre[i];
    if (p === undefined || q === undefined) return p === undefined ? -1 : 1;
    const np = /^\d+$/.test(p), nq = /^\d+$/.test(q);
    if (np && nq && Number(p) !== Number(q)) return Number(p) - Number(q);
    if (np !== nq) return np ? -1 : 1;
    if (p !== q) return p < q ? -1 : 1;
  }
  return 0;
}

export const newer = (candidate: string, current: string) => compare(candidate, current) > 0;
