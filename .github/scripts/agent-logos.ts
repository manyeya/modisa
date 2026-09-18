// Builds src/config/agents/marks.ttf, the font modisa draws agents' logos from, from Lobe Icons' monochrome SVGs (MIT;
// see src/config/agents/marks.NOTICE), one per entry of LOGOS (src/config/agents/brands.ts):
// - the logo, scaled to the em square, at LOGO_FIRST + i: for a one-line row;
// - its top and bottom halves, HALF_VARIANTS times, at HALVES_FIRST + v * 0x100 + i (+ 0x80 for the bottom): drawn on
//   two lines one cell apart, they're the logo centred between them. Variant v is for cells halfCell(v) ems tall, and
//   each half runs a sliver past the middle, so the two overlap there instead of leaving an anti-aliased seam.
// The SVGs fill even-odd; a font fills by winding, so each logo is flattened to polygons, and their even-odd area
// (the XOR of its rings) is what the glyphs draw. Run it after changing LOGOS or the halves:
//   bun .github/scripts/agent-logos.ts
import svgpath from "svgpath";
import svg2ttf from "svg2ttf";
import polygonClipping, { type MultiPolygon, type Pair } from "polygon-clipping";
import { HALF_VARIANTS, HALVES_FIRST, LOGOS, LOGO_FIRST, halfCell } from "../../src/config/agents/brands";

const ICONS = `${import.meta.dir}/../../node_modules/@lobehub/icons-static-svg/icons`;
const EM = 1000, ASCENT = 850, DESCENT = 150;
const S = EM / 24; // the icons are drawn on a 24×24 grid, y down
const OVERLAP = 0.3; // in grid units: how far each half runs past the middle

// A path's rings, curves flattened into short segments.
function rings(d: string): Pair[][] {
  const out: Pair[][] = [];
  let ring: Pair[] = [];
  let x = 0, y = 0, sx = 0, sy = 0;
  const curve = (pts: Pair[], n = 12) => {
    for (let k = 1; k <= n; k++) {
      const t = k / n;
      const w = pts.length === 3 ? [(1 - t) ** 2, 2 * (1 - t) * t, t * t] : [(1 - t) ** 3, 3 * (1 - t) ** 2 * t, 3 * (1 - t) * t * t, t ** 3];
      ring.push([w.reduce((s, c, j) => s + c * pts[j]![0], 0), w.reduce((s, c, j) => s + c * pts[j]![1], 0)]);
    }
  };
  const close = () => {
    if (ring.length > 2) out.push([...ring, ring[0]!]);
    ring = [];
  };
  svgpath(d).abs().unarc().unshort().iterate((seg) => {
    const [c, ...a] = seg as [string, ...number[]];
    if (c === "M") (close(), ring.push([a[0]!, a[1]!]), (x = sx = a[0]!), (y = sy = a[1]!));
    else if (c === "L") (ring.push([a[0]!, a[1]!]), (x = a[0]!), (y = a[1]!));
    else if (c === "H") (ring.push([a[0]!, y]), (x = a[0]!));
    else if (c === "V") (ring.push([x, a[0]!]), (y = a[0]!));
    else if (c === "C") (curve([[x, y], [a[0]!, a[1]!], [a[2]!, a[3]!], [a[4]!, a[5]!]]), (x = a[4]!), (y = a[5]!));
    else if (c === "Q") (curve([[x, y], [a[0]!, a[1]!], [a[2]!, a[3]!]]), (x = a[2]!), (y = a[3]!));
    else if (c === "Z") (close(), (x = sx), (y = sy));
  });
  close();
  return out;
}

// A logo's filled area on its 24×24 grid.
async function area(icon: string, agent: string): Promise<MultiPolygon> {
  const svg = await Bun.file(`${ICONS}/${icon}.svg`).text().catch(() => { throw new Error(`no Lobe icon ${icon} for ${agent}`); });
  if (!/viewBox="0 0 24 24"/.test(svg)) throw new Error(`${icon}: expected a 24×24 viewBox`);
  if (/<(?!\/?(svg|title|path)\b)[a-z]/i.test(svg)) throw new Error(`${icon}: only <path> elements are supported`);
  const d = [...svg.matchAll(/<path[^>]*\sd="([^"]+)"/g)].map((m) => m[1]).join(" ");
  const [first, ...rest] = rings(d).map((r) => [r]);
  return polygonClipping.xor(first!, ...rest); // even-odd
}

const band = (from: number, to: number): MultiPolygon => [[[[-1, from], [25, from], [25, to], [-1, to], [-1, from]]]];
// polygons as glyph outline data, each point placed by `at`
const outline = (shape: MultiPolygon, at: (p: Pair) => Pair) =>
  shape.flatMap((poly) => poly.map((ring) => "M" + ring.slice(0, -1).map((p) => at(p).map((v) => Math.round(v)).join(" ")).join("L") + "Z")).join("");
const glyph = (code: number, d: string) => `<glyph unicode="&#x${code.toString(16)};" horiz-adv-x="${EM}" d="${d}"/>`;

const glyphs: string[] = [];
for (const [i, [agent, icon]] of LOGOS.entries()) {
  const shape = await area(icon, agent);
  glyphs.push(glyph(LOGO_FIRST + i, outline(shape, ([x, y]) => [x * S, ASCENT - y * S])));
  const top = polygonClipping.intersection(shape, band(-1, 12 + OVERLAP));
  const bottom = polygonClipping.intersection(shape, band(12 - OVERLAP, 25));
  for (let v = 0; v < HALF_VARIANTS; v++) {
    // the logo's middle on the boundary between the lines: a cell's descent is about 21.5% of its height
    const cell = halfCell(v) * EM, middle = -0.215 * cell;
    glyphs.push(glyph(HALVES_FIRST + v * 0x100 + i, outline(top, ([x, y]) => [x * S, middle + (12 - y) * S])));
    glyphs.push(glyph(HALVES_FIRST + v * 0x100 + 0x80 + i, outline(bottom, ([x, y]) => [x * S, middle + cell + (12 - y) * S])));
  }
}
const font = `<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"><defs><font id="ModisaMarks" horiz-adv-x="${EM}">
<font-face font-family="Modisa Marks" units-per-em="${EM}" ascent="${ASCENT}" descent="-${DESCENT}"/>
<missing-glyph horiz-adv-x="${EM}"/>
${glyphs.join("\n")}
</font></defs></svg>`;
const ttf = svg2ttf(font, { copyright: "Logos: Lobe Icons (MIT). Trademarks of their owners.", description: "Coding agents' logos, for modisa", version: "1.1" });
const out = `${import.meta.dir}/../../src/config/agents/marks.ttf`;
await Bun.write(out, ttf.buffer);
console.log(`${LOGOS.length} logos, ${HALF_VARIANTS} sizes of halves → ${out} (${ttf.buffer.length} bytes)`);
