import { test, expect } from "bun:test";
import { withoutCredentials } from "../../src/config/plugins";
import { gitEnv, sourceProblem, TRANSPORTS } from "../../src/cli/plugin-git";

test("credentials never stay in a recorded or displayed git URL", () => {
  expect(withoutCredentials("https://user:secret@github.com/a/b.git")).toBe("https://github.com/a/b.git");
  expect(withoutCredentials("https://ghp_abc123@github.com/a/b")).toBe("https://github.com/a/b");
  expect(withoutCredentials("ssh://git@example.com/a/b.git")).toBe("ssh://example.com/a/b.git");
  expect(withoutCredentials("git@github.com:a/b.git")).toBe("git@github.com:a/b.git");
  expect(withoutCredentials("file:///tmp/repo.git")).toBe("file:///tmp/repo.git");
});

test("install fetches only https, ssh, git, file and user@host:path URLs; never a remote helper, plain http or a bare path", () => {
  for (const ok of ["https://github.com/a/b.git", "HTTPS://github.com/a/b", "ssh://git@example.com:2222/a/b.git", "git://example.com/a/b.git", "file:///tmp/repo.git", "git@github.com:a/b.git", "me@build-01.internal:plugins/x.git"]) expect(sourceProblem(ok)).toBeUndefined();
  const refused = (url: string) => sourceProblem(url) ?? "";
  expect(refused("ext::sh -c touch% /tmp/pwned")).toContain("remote-helper");
  expect(refused("fd::3")).toContain("remote-helper");
  expect(refused("foo::bar")).toContain("remote-helper");
  expect(refused("::bar")).toContain("remote-helper");
  expect(refused("http://github.com/a/b")).toContain("use https");
  expect(refused("ftp://example.com/a.git")).toContain("isn't a supported git transport");
  expect(refused("/tmp/repo.git")).toContain("not a git URL");
  expect(refused("./repo")).toContain("not a git URL");
  expect(refused("--upload-pack=touch /tmp/x")).toContain("not a git URL");
  expect(refused("git@github.com:a/b.git with spaces")).toContain("not a git URL");
});

test("installer git never inherits repository routing, injected config or a transport allowlist, and keeps what authenticated transports need", () => {
  const env = gitEnv({
    GIT_DIR: "/elsewhere/.git", GIT_WORK_TREE: "/elsewhere", GIT_INDEX_FILE: "x", GIT_OBJECT_DIRECTORY: "x", GIT_ALTERNATE_OBJECT_DIRECTORIES: "x",
    GIT_COMMON_DIR: "x", GIT_NAMESPACE: "x", GIT_CEILING_DIRECTORIES: "x", GIT_DISCOVERY_ACROSS_FILESYSTEM: "1",
    GIT_CONFIG_PARAMETERS: "'protocol.ext.allow'='always'", GIT_CONFIG_COUNT: "1", GIT_CONFIG_KEY_0: "protocol.ext.allow", GIT_CONFIG_VALUE_0: "always",
    GIT_ALLOW_PROTOCOL: "ext", GIT_TERMINAL_PROMPT: "1",
    HOME: "/home/me", PATH: "/usr/bin", SSH_AUTH_SOCK: "/tmp/agent.sock", GIT_SSH_COMMAND: "ssh -i key", GIT_SSH: "/usr/bin/ssh",
    GIT_CONFIG_GLOBAL: "/home/me/.gitconfig", HTTPS_PROXY: "http://proxy:3128", NO_PROXY: "localhost", UNSET: undefined,
  });
  for (const k of ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_COMMON_DIR", "GIT_NAMESPACE", "GIT_CEILING_DIRECTORIES", "GIT_DISCOVERY_ACROSS_FILESYSTEM", "GIT_CONFIG_PARAMETERS", "GIT_CONFIG_COUNT", "GIT_CONFIG_KEY_0", "GIT_CONFIG_VALUE_0", "UNSET"]) expect(env).not.toHaveProperty(k);
  expect(env).toMatchObject({ GIT_ALLOW_PROTOCOL: TRANSPORTS, GIT_TERMINAL_PROMPT: "0", HOME: "/home/me", PATH: "/usr/bin", SSH_AUTH_SOCK: "/tmp/agent.sock", GIT_SSH_COMMAND: "ssh -i key", GIT_SSH: "/usr/bin/ssh", GIT_CONFIG_GLOBAL: "/home/me/.gitconfig", HTTPS_PROXY: "http://proxy:3128", NO_PROXY: "localhost" });
  expect(TRANSPORTS.split(":")).not.toContain("ext");
});
