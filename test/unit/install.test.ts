import { test, expect } from "bun:test";
import { installedBy } from "../../src/core/install";

test("a package manager's install is recognised by where the binary lives", () => {
  expect(installedBy("/opt/homebrew/Cellar/shepherd/0.2.0/bin/shepherd", false)).toMatchObject({ by: "homebrew", upgrade: "brew upgrade shepherd" });
  expect(installedBy("/home/linuxbrew/.linuxbrew/Cellar/shepherd/0.2.0/bin/shepherd", false).by).toBe("homebrew");
  expect(installedBy("/Users/me/.local/share/mise/installs/github-manyeya-shepherd/0.2.0/shepherd", false)).toMatchObject({ by: "mise", remove: "mise uninstall github:manyeya/shepherd" });
  expect(installedBy("/usr/bin/shepherd", false).by).toBe("system");
});

test("install.sh's binary, and a checkout, manage themselves", () => {
  expect(installedBy("/Users/me/.local/bin/shepherd", false)).toEqual({ by: "script" });
  expect(installedBy("/usr/local/bin/shepherd", false)).toEqual({ by: "script" }); // SHEPHERD_INSTALL_DIR=/usr/local/bin
  expect(installedBy("/opt/homebrew/Cellar/shepherd/0.2.0/bin/shepherd", true)).toEqual({ by: "source" });
});
