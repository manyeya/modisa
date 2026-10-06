import { test, expect } from "bun:test";
import { Box, Client, Fragment, Raster, Span, Text, h, ink, rasterCells } from "../../src/plugins/modisa-plugin";
import { checkView, cleanBlock } from "../../src/server/views";
import { RASTER } from "../../src/protocol/types";

// what a plugin's client sends for ui.view.set
const sent = (root: unknown) => {
  let line: any;
  const c = new Client({ write: (l) => (line = JSON.parse(l)), close() {} });
  void c.ui.view("v", root as never, { title: "T" });
  return line.params;
};

test("elements as functions, classic JSX (h) and React-style elements all make the same tree", () => {
  const want = { type: "box", direction: "row", children: [{ type: "text", children: ["hi ", { type: "span", tone: "accent", children: ["x"] }] }, { type: "text", children: ["plain"] }, { type: "raster", key: "r", columns: 1, rows: 1, cells: "" }] };
  expect(sent(Box({ direction: "row" }, Text({}, "hi ", Span({ tone: "accent" }, "x")), "plain", Raster({ key: "r", columns: 1, rows: 1, cells: "" }))).root).toEqual(want);
  expect(sent(h(Box, { direction: "row" }, h(Text, null, "hi ", h(Span, { tone: "accent" }, "x")), "plain", h(Fragment, null, h(Raster, { key: "r", columns: 1, rows: 1, cells: "" })))).root).toEqual(want);
  // what Bun's automatic JSX runtime makes: { type, props, key }, children in props, key apart
  const el = (type: unknown, props: object, key?: string) => ({ $$typeof: Symbol.for("react.element"), type, props, key: key ?? null, _owner: null, _store: {} });
  const react = el(Box, { direction: "row", children: [el(Text, { children: ["hi ", el(Span, { tone: "accent", children: "x" })] }), "plain", el(Symbol.for("react.fragment"), { children: el(Raster, { columns: 1, rows: 1, cells: "" }, "r") })] });
  expect(sent(react).root).toEqual(want);
  expect(sent(react)).toMatchObject({ id: "v", title: "T" });
});

test("rasterCells packs [codePoint, fg, bg] triplets, colours as tones, RGB or the default", () => {
  const words = new Uint32Array(Buffer.from(rasterCells(2, 1, (x) => (x ? ["█", ink.tone("warn"), ink.rgb("#ff8800")] : "a")), "base64").buffer.slice(0));
  expect([...words]).toEqual([0x61, RASTER.DEFAULT, RASTER.DEFAULT, 0x2588, RASTER.TONE | 3, 0xff8800]);
});

test("the server cleans a view's text, keeps newlines where they mean something, and checks what it names", () => {
  expect(cleanBlock("a\x1b[31mb\x1b]0;title\x07c\n\td\x07")).toBe("abc\n\td");
  const { root, rasters } = checkView(
    { type: "box", title: "t\x1b[1mx", children: [{ type: "text", children: ["line\none\x1b[2J"] }, { type: "raster", key: "r", columns: 2, rows: 1, cells: rasterCells(2, 1, () => "x") }, { type: "button", label: "go\n", action: "go" }] },
    [{ key: "j", action: "go", description: "down\x1b[A" }],
    undefined,
    ["go"],
  );
  expect(root).toMatchObject({ title: "tx", children: [{ children: ["line\none"] }, {}, { label: "go" }] });
  expect(rasters.get("r")).toEqual({ columns: 2, rows: 1 });
  const refused = (f: () => unknown) => {
    try {
      f();
    } catch (e) {
      return (e as { code?: string }).code;
    }
  };
  expect(refused(() => checkView({ type: "button", label: "x", action: "nope" }, [], undefined, ["go"]))).toBe("no_such_action");
  expect(refused(() => checkView({ type: "text" }, [], "nope", ["go"]))).toBe("no_such_action");
  expect(refused(() => checkView({ type: "raster", key: "r", columns: 1, rows: 1, cells: rasterCells(1, 1, () => "中") }, [], undefined, []))).toBe("invalid_params"); // two cells wide
  let deep: any = { type: "text" };
  for (let i = 0; i < 50; i++) deep = { type: "box", children: [deep] };
  expect(refused(() => checkView(deep, [], undefined, []))).toBe("invalid_params");
});
