// Renders cuelume recipes offline: the oscillators, noise, envelopes, biquad filters, shimmer echo and
// output limiter its Web Audio engine builds live, computed sample by sample into PCM and wrapped as a
// WAV that OpenTUI's audio engine can load. Mirrors cuelume 0.2.2's audio/engine.js.
import type { NoiseLayer, Recipe, ToneLayer } from "./recipes";

export const RATE = 48000;
const FLOOR = 0.0001; // exponential ramps start and end here
const PAD = 0.05; // sources run this long past their envelope
const OUTPUT_GAIN = 4;
const THRESHOLD = -8, RATIO = 12; // the output compressor, as a static curve
const MAKEUP = (1 / 10 ** ((THRESHOLD - THRESHOLD / RATIO) / 20)) ** 0.6; // Web Audio's automatic makeup gain

// exponentialRampToValueAtTime from FLOOR up to peak over attack, then back down over decay
function envelope(t: number, { attack, decay, peak }: ToneLayer | NoiseLayer) {
  if (t < 0 || t > attack + decay) return 0;
  return t < attack ? FLOOR * (peak / FLOOR) ** (t / attack) : peak * (FLOOR / peak) ** ((t - attack) / decay);
}

// An RBJ-cookbook biquad, like BiquadFilterNode: lowpass Q is in dB, bandpass Q is linear.
function biquad(type: "lowpass" | "bandpass", frequency: number, q = 1) {
  const w = (2 * Math.PI * frequency) / RATE, cos = Math.cos(w);
  const alpha = Math.sin(w) / (2 * (type === "lowpass" ? 10 ** (q / 20) : q));
  const [b0, b1, b2] = type === "lowpass" ? [(1 - cos) / 2, 1 - cos, (1 - cos) / 2] : [alpha, 0, -alpha];
  const a0 = 1 + alpha, a1 = -2 * cos, a2 = 1 - alpha;
  let x1 = 0, x2 = 0, y1 = 0, y2 = 0;
  return (x: number) => {
    const y = (b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2) / a0;
    [x2, x1, y2, y1] = [x1, x, y1, y];
    return y;
  };
}

// Deterministic white noise, so a sound renders the same every time.
function noise(seed: number) {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return (((t ^ (t >>> 14)) >>> 0) / 4294967296) * 2 - 1;
  };
}

function addTone(out: Float32Array, layer: ToneLayer) {
  const start = Math.round((layer.offset ?? 0) * RATE);
  const length = Math.ceil((layer.attack + layer.decay + PAD) * RATE);
  const detune = 2 ** ((layer.detune ?? 0) / 1200);
  const glide = layer.glideTime ?? layer.attack + layer.decay;
  let phase = 0;
  for (let i = 0; i < length && start + i < out.length; i++) {
    const t = i / RATE;
    const f = (layer.glideTo === undefined ? layer.frequency : layer.frequency * (layer.glideTo / layer.frequency) ** Math.min(1, t / glide)) * detune;
    phase = (phase + f / RATE) % 1;
    const wave = layer.waveform === "sine" ? Math.sin(2 * Math.PI * phase) : 1 - 4 * Math.abs(phase - 0.5);
    out[start + i]! += wave * envelope(t, layer);
  }
}

function addNoise(out: Float32Array, layer: NoiseLayer, seed: number) {
  const start = Math.round((layer.offset ?? 0) * RATE);
  const length = Math.ceil((layer.attack + layer.decay + PAD) * RATE);
  const filter = biquad(layer.filterType, layer.filterFrequency, layer.filterQ);
  const random = noise(seed);
  for (let i = 0; i < length && start + i < out.length; i++) out[start + i]! += filter(random()) * envelope(i / RATE, layer);
}

// How long a recipe rings: its last source, plus echoes until they fall below -60 dB.
export function duration(recipe: Recipe) {
  const end = Math.max(...recipe.layers.map((l) => (l.offset ?? 0) + l.attack + l.decay + PAD));
  const s = recipe.shimmer;
  const tail = !s || s.feedback <= 0 ? 0 : s.delay * (1 + Math.ceil(Math.log(0.001) / Math.log(s.feedback)));
  return end + tail + 0.05;
}

// Mono PCM in -1..1 at RATE.
export function render(recipe: Recipe): Float32Array {
  const dry = new Float32Array(Math.ceil(duration(recipe) * RATE));
  recipe.layers.forEach((layer, i) => (layer.kind === "tone" ? addTone(dry, layer) : addNoise(dry, layer, i + 1)));
  const out = new Float32Array(dry.length);
  const s = recipe.shimmer;
  const lowpass = s && biquad("lowpass", s.lowpass);
  const delay = s ? new Float32Array(Math.max(1, Math.round(s.delay * RATE))) : undefined; // a ring buffer
  for (let i = 0; i < dry.length; i++) {
    const master = dry[i]! * recipe.masterGain;
    let wet = 0;
    if (s && delay && lowpass) {
      const echoed = lowpass(delay[i % delay.length]!);
      delay[i % delay.length] = master + s.feedback * echoed;
      wet = s.wet * echoed;
    }
    const x = (master + wet) * OUTPUT_GAIN;
    const level = 20 * Math.log10(Math.abs(x) || 1e-9);
    const limited = level > THRESHOLD ? Math.sign(x) * 10 ** ((THRESHOLD + (level - THRESHOLD) / RATIO) / 20) : x;
    out[i] = Math.max(-1, Math.min(1, limited * MAKEUP));
  }
  return out;
}

// 16-bit mono PCM WAV.
export function toWav(pcm: Float32Array): Uint8Array {
  const bytes = new Uint8Array(44 + pcm.length * 2);
  const view = new DataView(bytes.buffer);
  const text = (at: number, s: string) => [...s].forEach((c, i) => (bytes[at + i] = c.charCodeAt(0)));
  text(0, "RIFF");
  view.setUint32(4, 36 + pcm.length * 2, true);
  text(8, "WAVEfmt ");
  view.setUint32(16, 16, true); // fmt chunk size
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, RATE, true);
  view.setUint32(28, RATE * 2, true); // bytes per second
  view.setUint16(32, 2, true); // bytes per frame
  view.setUint16(34, 16, true); // bits per sample
  text(36, "data");
  view.setUint32(40, pcm.length * 2, true);
  pcm.forEach((s, i) => view.setInt16(44 + i * 2, Math.round(s * 32767), true));
  return bytes;
}
