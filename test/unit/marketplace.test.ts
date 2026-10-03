// A marketplace's file is a stranger's: its names are checked, its text cleaned, and each plugin's source is one of
// the three forms, refused when it could reach git as an option, a remote helper or a path outside the repository.
// A relative source resolves only to a directory really inside the checkout.
import { test, expect, afterAll } from "bun:test";
import { marketplaceFile, marketplaceUrl, within } from "../../src/config/marketplaces";

const tmp = `${Bun.env.TMPDIR ?? "/tmp"}/modisa-marketplace-unit-${Date.now()}`;
afterAll(async () => {
  await Bun.$`rm -rf ${tmp}`.quiet().nothrow();
});

const problems = (raw: unknown) => {
  const r = marketplaceFile.safeParse(raw);
  return r.success ? [] : r.error.issues.map((i) => `${i.path.join(".")}: ${i.message}`);
};
const base = { name: "acme", plugins: [] as unknown[] };

test("the spec's three source forms parse, and extra fields are ignored", () => {
  const m = marketplaceFile.parse({
    name: "acme", description: "Acme's plugins", owner: "acme", version: 3,
    plugins: [
      { name: "worktrees", description: "a space per worktree", source: "./plugins/worktrees", author: "someone" },
      { name: "x", source: { git: "https://github.com/a/x.git", ref: "v1", subdir: "plugin" } },
      { name: "y", source: "owner/repo" },
    ],
  });
  expect(m.plugins.map((p) => p.source)).toEqual(["./plugins/worktrees", { git: "https://github.com/a/x.git", ref: "v1", subdir: "plugin" }, "owner/repo"]);
  expect(m.plugins[1]!.description).toBe(""); // none given
  expect(marketplaceFile.parse({ ...base, owner: { name: "Acme", email: "a@b.c" } }).owner).toBe("Acme"); // Claude Code's owner
});

test("its text can't write to the terminal: escapes, control and bidi characters go, and it's one line, cut to cells", () => {
  const m = marketplaceFile.parse({
    name: "acme", description: "safe\x1b]0;owned\x07 text‮gpj.exe\nnext line", owner: "a\x1b[31mred",
    plugins: [{ name: "p", description: "日".repeat(150), source: "./p" }],
  });
  expect(m.description).toBe("safe textgpj.exe next line");
  expect(m.owner).toBe("ared");
  expect(Bun.stringWidth(m.plugins[0]!.description)).toBe(200);
});

test("names are ids, each plugin named once", () => {
  expect(problems({ ...base, name: "Acme Inc" }).join()).toContain("lowercase letters, digits and dashes");
  expect(problems({ ...base, name: "../up" }).length).toBeGreaterThan(0);
  expect(problems({ ...base, plugins: [{ name: "a/b", source: "./x" }] }).length).toBeGreaterThan(0);
  expect(problems({ ...base, plugins: [{ name: "a", source: "./x" }, { name: "a", source: "./y" }] }).join()).toContain("a second plugin named a");
});

test("a source is ./path, owner/repo or { git, ref, subdir }, and nothing that reaches git as an option or a helper", () => {
  const sourceProblem = (source: unknown) => problems({ ...base, plugins: [{ name: "p", source }] }).join("; ");
  expect(sourceProblem("plugins")).toContain('a source is "./path"'); // neither form
  expect(sourceProblem("https://github.com/a/b.git")).toContain('a source is "./path"'); // a URL goes in { git }
  expect(sourceProblem(42)).toContain('a source is "./path"');
  expect(sourceProblem("./has space")).toContain("can't start with - or hold spaces");
  expect(sourceProblem({ git: "ext::sh" })).toContain("remote-helper");
  expect(sourceProblem({ git: "ext::sh -c touch% /tmp/pwned" })).toContain("can't start with - or hold spaces");
  expect(sourceProblem({ git: "--upload-pack=touch /tmp/x" })).toContain("can't start with -");
  expect(sourceProblem({ git: "http://example.com/a.git" })).toContain("use https");
  expect(sourceProblem({ git: "https://example.com/a.git", ref: "--force" })).toContain("can't start with -");
  expect(sourceProblem({ git: "https://example.com/a.git", subdir: "../out" })).toContain("relative path inside the repository");
  expect(sourceProblem({ git: "https://example.com/a.git", subdir: "/etc" })).toContain("relative path inside the repository");
  expect(sourceProblem({ git: "https://example.com/a.git\nx" })).toContain("control characters");
  expect(sourceProblem("./plugins/p")).toBe("");
  expect(sourceProblem({ git: "git@github.com:a/b.git", ref: "main", subdir: "p" })).toBe("");
});

test("owner/repo means GitHub; anything else is a git URL as given", () => {
  expect(marketplaceUrl("acme/plugins")).toBe("https://github.com/acme/plugins.git");
  expect(marketplaceUrl("https://example.com/m.git")).toBe("https://example.com/m.git");
  expect(marketplaceUrl("git@example.com:m.git")).toBe("git@example.com:m.git");
  expect(marketplaceUrl("./local")).toBe("./local"); // not GitHub, and install refuses it as a URL
});

test("a relative source resolves only to a directory really inside the checkout", async () => {
  const checkout = `${tmp}/checkout`;
  await Bun.$`mkdir -p ${checkout}/plugins/p ${tmp}/outside/p`.quiet();
  await Bun.$`ln -s ${tmp}/outside/p ${checkout}/away`.quiet();
  await Bun.write(`${checkout}/plugins/file`, "not a directory");
  const root = (await Bun.$`realpath ${checkout}`.text()).trim();
  expect(await within(checkout, "./plugins/p")).toEqual({ root, subdir: "plugins/p" });
  expect(await within(checkout, "./plugins/../plugins/p/")).toEqual({ root, subdir: "plugins/p" });
  expect(await within(checkout, "./")).toEqual({ root, subdir: null });
  for (const escape of ["./../outside/p", "./away", "./plugins/file", "./missing"]) await expect(within(checkout, escape)).rejects.toThrow("isn't a directory inside the marketplace's repository");
});
