// Text from a stranger (a plugin, a plugin index) shown in a terminal: no escape sequences, control characters, or
// invisible formatting characters (bidi overrides and isolates, zero-width marks) that could reorder or hide part of
// it; and at most `cells` terminal cells, cut between whole characters (a wide character counts 2).
const graphemes = new Intl.Segmenter();
export const cleanText = (text: string, cells: number) => {
  const plain = text
    .replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?/g, "") // OSC
    .replace(/\x1b\[[0-9;?]*[ -/]*[@-~]/g, "") // CSI
    .replace(/[\x00-\x1f\x7f-\x9f]|\p{Cf}/gu, "");
  let out = "";
  let width = 0;
  for (const { segment } of graphemes.segment(plain)) {
    const w = Bun.stringWidth(segment);
    if (width + w > cells) break;
    out += segment;
    width += w;
  }
  return out;
};
