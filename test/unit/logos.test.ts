import { test, expect } from "bun:test";
import { FAMILY, withVscodeFont } from "../../src/platform/logos";
import { LOGOS, logo } from "../../src/config/agents/brands";

const settings = `{
    // my settings
    "editor.fontFamily": "JetBrains Mono, monospace",
    "files.autoSave": "afterDelay",
}
`;

test("VS Code: the logo font goes at the end of the terminal's font list, the file's comments and commas untouched", () => {
  const added = withVscodeFont(settings, true);
  expect(added.created).toBe(true); // there was no terminal font list: one is made from the editor's
  expect(added.text).toContain(`"terminal.integrated.fontFamily": "JetBrains Mono, monospace, '${FAMILY}'",`);
  expect(added.text).toContain("// my settings");
  expect(added.text.endsWith(`"files.autoSave": "afterDelay",\n}\n`)).toBe(true);
  expect(withVscodeFont(added.text, true).text).toBe(added.text); // twice is once
  expect(withVscodeFont(added.text, false, true).text).toBe(settings); // and out again, byte for byte
});

test("VS Code: an existing terminal font list keeps its fonts, gets the logo font last, and loses only it again", () => {
  const own = `{ "terminal.integrated.fontFamily": "Fira Code, 'Symbols Nerd Font'" }`;
  const added = withVscodeFont(own, true);
  expect(added).toEqual({ text: `{ "terminal.integrated.fontFamily": "Fira Code, 'Symbols Nerd Font', '${FAMILY}'" }`, created: false });
  expect(withVscodeFont(added.text, false).text).toBe(own);
  expect(withVscodeFont("{}", true).text).toContain(`, '${FAMILY}'"\n}`); // an empty object: no stray comma
});

test("every agent with a logo gets its own character, in order, from U+F5A00", () => {
  expect(logo("claude-code")).toBe(String.fromCodePoint(0xf5a00));
  expect(logo("mastracode")).toBe(String.fromCodePoint(0xf5a00 + LOGOS.length - 1));
  expect(logo("aider")).toBeUndefined(); // no logo: its glyph instead
  expect(new Set(LOGOS.map(([a]) => a)).size).toBe(LOGOS.length);
});
