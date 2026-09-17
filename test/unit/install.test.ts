import { test, expect } from "bun:test";
import { installedBy } from "../../src/core/install";

test("a package manager's install is recognised by where the binary lives", () => {
  expect(installedBy("/opt/homebrew/Cellar/modisa/0.2.0/bin/modisa", false)).toMatchObject({ by: "homebrew", upgrade: "brew upgrade modisa" });
  expect(installedBy("/home/linuxbrew/.linuxbrew/Cellar/modisa/0.2.0/bin/modisa", false).by).toBe("homebrew");
  expect(installedBy("/Users/me/.local/share/mise/installs/github-manyeya-modisa/0.2.0/modisa", false)).toMatchObject({ by: "mise", remove: "mise uninstall github:manyeya/modisa" });
  expect(installedBy("/usr/bin/modisa", false).by).toBe("system");
});

test("install.sh's binary, and a checkout, manage themselves", () => {
  expect(installedBy("/Users/me/.local/bin/modisa", false)).toEqual({ by: "script" });
  expect(installedBy("/usr/local/bin/modisa", false)).toEqual({ by: "script" }); // MODISA_INSTALL_DIR=/usr/local/bin
  expect(installedBy("/opt/homebrew/Cellar/modisa/0.2.0/bin/modisa", true)).toEqual({ by: "source" });
});
