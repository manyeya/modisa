import { test, expect } from "bun:test";
import { compare, newer } from "../../src/core/version";
import { parseManifest } from "../../src/cli/update";

test("versions order like semver, prereleases included", () => {
  const sorted = ["0.1.0-dev", "0.1.0-staging.2", "0.1.0-staging.10", "0.1.0", "0.1.1-staging.1", "0.1.1", "0.2.0", "1.0.0"];
  for (let i = 1; i < sorted.length; i++) expect([sorted[i - 1], compare(sorted[i - 1]!, sorted[i]!) < 0]).toEqual([sorted[i - 1], true]);
  expect(newer("v0.1.1", "0.1.0")).toBe(true);
  expect(newer("0.1.0", "0.1.0")).toBe(false);
  expect(newer("0.1.0-staging.3", "0.1.0")).toBe(false);
});

test("release manifests keep only well-formed assets", () => {
  const sha = "a".repeat(64);
  expect(parseManifest({ version: "0.2.0", assets: { "darwin-arm64": { url: "https://x/y", sha256: sha }, "linux-x64": { url: "https://x/z", sha256: "nope" } } }))
    .toEqual({ version: "0.2.0", channel: "stable", notes: "", assets: { "darwin-arm64": { url: "https://x/y", sha256: sha } } });
  expect(parseManifest({ assets: {} })).toBeUndefined();
  expect(parseManifest(null)).toBeUndefined();
});
