// Builds src/config/agents/marks.ttf, the font modisa draws agents' logos from: one glyph per entry of LOGOS
// (src/config/agents/brands.ts), from Lobe Icons' monochrome SVGs (MIT; see src/config/agents/marks.NOTICE), each
// scaled to the em square with the logo's holes kept. Run it after changing LOGOS:
//   bun .github/scripts/agent-logos.ts
import svgpath from "svgpath";
import svg2ttf from "svg2ttf";
import { LOGOS, LOGO_FIRST } from "../../src/config/agents/brands";

const ICONS = `${import.meta.dir}/../../node_modules/@lobehub/icons-static-svg/icons`;
const EM = 1000, ASCENT = 850, DESCENT = 150; // a logo fills the em, square; terminals fit it to their cells

const glyphs: string[] = [];
for (const [i, [agent, icon]] of LOGOS.entries()) {
  const svg = await Bun.file(`${ICONS}/${icon}.svg`).text().catch(() => { throw new Error(`no Lobe icon ${icon} for ${agent}`); });
  if (!/viewBox="0 0 24 24"/.test(svg)) throw new Error(`${icon}: expected a 24×24 viewBox`);
  if (/<(?!\/?(svg|title|path)\b)[a-z]/i.test(svg)) throw new Error(`${icon}: only <path> elements are supported`);
  const d = [...svg.matchAll(/<path[^>]*\sd="([^"]+)"/g)].map((m) => m[1]).join(" ");
  // SVG's y grows down, a font's up: flip about the baseline, then lift to the ascent
  const outline = svgpath(d).scale(EM / 24, -EM / 24).translate(0, ASCENT).round(1).toString();
  const code = (LOGO_FIRST + i).toString(16);
  glyphs.push(`<glyph glyph-name="${agent}" unicode="&#x${code};" horiz-adv-x="${EM}" d="${outline}"/>`);
}
const font = `<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"><defs><font id="ModisaMarks" horiz-adv-x="${EM}">
<font-face font-family="Modisa Marks" units-per-em="${EM}" ascent="${ASCENT}" descent="-${DESCENT}"/>
<missing-glyph horiz-adv-x="${EM}"/>
${glyphs.join("\n")}
</font></defs></svg>`;
const ttf = svg2ttf(font, { copyright: "Logos: Lobe Icons (MIT). Trademarks of their owners.", description: "Coding agents' logos, for modisa", version: "1.0" });
const out = `${import.meta.dir}/../../src/config/agents/marks.ttf`;
await Bun.write(out, ttf.buffer);
console.log(`${LOGOS.length} logos → ${out} (${ttf.buffer.length} bytes)`);
