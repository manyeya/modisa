import { test, expect } from "bun:test";
import { Audio } from "@opentui/core";
import { RECIPES, SOUNDS } from "../../src/client/sound/recipes";
import { duration, RATE, render, toWav } from "../../src/client/sound/synth";

test("every cuelume recipe renders to audible, unclipped PCM of the right length", () => {
  expect(SOUNDS).toHaveLength(17);
  for (const name of SOUNDS) {
    const pcm = render(RECIPES[name]);
    expect(pcm.length).toBe(Math.ceil(duration(RECIPES[name]) * RATE));
    const peak = pcm.reduce((m, s) => Math.max(m, Math.abs(s)), 0);
    expect(Number.isFinite(peak)).toBe(true); // a NaN or Infinity anywhere poisons the max
    expect(peak).toBeGreaterThan(0.05);
    expect(peak).toBeLessThanOrEqual(1);
  }
  expect(render(RECIPES.chime)).toEqual(render(RECIPES.chime)); // deterministic, noise included
});

test("sounds are 16-bit mono WAVs that OpenTUI's audio engine decodes", () => {
  const wav = toWav(render(RECIPES.success));
  const view = new DataView(wav.buffer);
  const text = (at: number, n: number) => new TextDecoder().decode(wav.slice(at, at + n));
  expect([text(0, 4), text(8, 4), text(12, 4), text(36, 4)]).toEqual(["RIFF", "WAVE", "fmt ", "data"]);
  expect([view.getUint16(22, true), view.getUint32(24, true), view.getUint16(34, true)]).toEqual([1, RATE, 16]);
  expect(view.getUint32(4, true)).toBe(wav.length - 8);
  const audio = Audio.create({ autoStart: false }); // decoding needs no output device
  try {
    expect(audio.loadSound(wav)).not.toBeNull();
  } finally {
    audio.dispose();
  }
});
