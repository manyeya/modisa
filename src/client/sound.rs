// Cuelume sounds: the recipes, an offline renderer for them, and a player.
//
// Renders cuelume recipes offline: the oscillators, noise, envelopes, biquad filters, shimmer echo and output limiter its
// Web Audio engine builds live, computed sample by sample into PCM and wrapped as a WAV. Mirrors cuelume 0.2.2's
// audio/engine.js. The arithmetic follows the TS sample for sample: f64 everywhere, rounded to f32 wherever the TS stored
// into a Float32Array (the dry mix, the echo's ring buffer, the output), so the PCM matches it.
use std::cell::RefCell;
use std::f64::consts::PI;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

use crate::core::paths::{which, DIR};

// ---- recipes ----

// The sound palette from cuelume 0.2.2 (https://cuelume.dev, github.com/Danilaa1/cuelume), copied verbatim. cuelume
// synthesizes these live with Web Audio in a browser; the synth below renders the same recipes to PCM so a terminal can
// play them.
//
// MIT License
//
// Copyright (c) 2026 Daniel Belyi
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

#[derive(Clone, Copy, PartialEq)]
enum Waveform {
    Sine,
    Triangle,
}

#[derive(Clone, Copy, PartialEq)]
enum FilterType {
    Lowpass,
    Bandpass,
}

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Tone(Waveform),
    Noise(FilterType),
}

// A tone or a filtered noise burst. `frequency` is a tone's pitch, or a noise layer's filter frequency.
#[derive(Clone, Copy)]
struct Layer {
    source: Source,
    frequency: f64,
    offset: f64,
    attack: f64,
    decay: f64,
    peak: f64,
    detune: f64, // tones: cents
    glide_to: Option<f64>,
    glide_time: Option<f64>, // unset: the whole envelope
    filter_q: f64, // noise
}

const fn layer(source: Source) -> Layer {
    Layer { source, frequency: 0.0, offset: 0.0, attack: 0.0, decay: 0.0, peak: 0.0, detune: 0.0, glide_to: None, glide_time: None, filter_q: 1.0 }
}
const SINE: Layer = layer(Source::Tone(Waveform::Sine));
const TRIANGLE: Layer = layer(Source::Tone(Waveform::Triangle));
const LOWPASS: Layer = layer(Source::Noise(FilterType::Lowpass));
const BANDPASS: Layer = layer(Source::Noise(FilterType::Bandpass));

struct Shimmer {
    delay: f64,
    feedback: f64,
    wet: f64,
    lowpass: f64,
}

struct Recipe {
    master_gain: f64,
    layers: &'static [Layer],
    shimmer: Option<Shimmer>,
}

const RECIPES: [(&str, Recipe); 17] = [
    // A soft two-note ascending bell, like an iOS/macOS confirmation tink.
    ("chime", Recipe {
        master_gain: 0.5,
        layers: &[
            Layer { frequency: 1046.5, attack: 0.006, decay: 0.22, peak: 0.09, ..SINE },
            Layer { frequency: 1568.0, offset: 0.09, attack: 0.006, decay: 0.26, peak: 0.08, ..SINE },
        ],
        shimmer: Some(Shimmer { delay: 0.12, feedback: 0.25, wet: 0.18, lowpass: 4000.0 }),
    }),
    // A quick ascending twinkle of four notes — bright and playful.
    ("sparkle", Recipe {
        master_gain: 0.5,
        layers: &[
            Layer { frequency: 1760.0, offset: 0.0, attack: 0.003, decay: 0.09, peak: 0.045, ..SINE },
            Layer { frequency: 2217.0, offset: 0.045, attack: 0.003, decay: 0.09, peak: 0.04, ..SINE },
            Layer { frequency: 2637.0, offset: 0.09, attack: 0.003, decay: 0.1, peak: 0.038, ..SINE },
            Layer { frequency: 3520.0, offset: 0.135, attack: 0.003, decay: 0.12, peak: 0.032, ..SINE },
        ],
        shimmer: Some(Shimmer { delay: 0.07, feedback: 0.35, wet: 0.22, lowpass: 6000.0 }),
    }),
    // A single note gliding smoothly downward, like a drop of water.
    ("droplet", Recipe {
        master_gain: 0.55,
        layers: &[Layer { frequency: 1200.0, glide_to: Some(550.0), glide_time: Some(0.14), attack: 0.004, decay: 0.2, peak: 0.075, ..SINE }],
        shimmer: Some(Shimmer { delay: 0.09, feedback: 0.2, wet: 0.15, lowpass: 3000.0 }),
    }),
    // A warm, slow-swelling pad from two gently detuned sines.
    ("bloom", Recipe {
        master_gain: 0.5,
        layers: &[
            Layer { frequency: 528.0, attack: 0.06, decay: 0.32, peak: 0.06, ..SINE },
            Layer { frequency: 528.0, detune: 12.0, attack: 0.06, decay: 0.34, peak: 0.05, ..SINE },
        ],
        shimmer: Some(Shimmer { delay: 0.15, feedback: 0.2, wet: 0.12, lowpass: 2500.0 }),
    }),
    // A soft hush with a falling tone — for tooltips and low-priority previews.
    ("whisper", Recipe {
        master_gain: 0.48,
        layers: &[
            Layer { frequency: 1600.0, filter_q: 0.7, attack: 0.025, decay: 0.13, peak: 0.04, ..LOWPASS },
            Layer { frequency: 880.0, glide_to: Some(660.0), glide_time: Some(0.14), offset: 0.01, attack: 0.012, decay: 0.14, peak: 0.025, ..SINE },
        ],
        shimmer: None,
    }),
    // A focused, bandpass-filtered tick with a bright sine ping on top — crisp and instant.
    ("tick", Recipe {
        master_gain: 0.4,
        layers: &[
            Layer { frequency: 5400.0, filter_q: 1.8, attack: 0.001, decay: 0.018, peak: 0.14, ..BANDPASS },
            Layer { frequency: 2600.0, attack: 0.001, decay: 0.012, peak: 0.018, ..SINE },
        ],
        shimmer: None,
    }),
    // A dull, muted knock — the "down" half of a press/release pair, like a key bottoming out.
    ("press", Recipe {
        master_gain: 0.4,
        layers: &[Layer { frequency: 1700.0, filter_q: 1.4, attack: 0.001, decay: 0.02, peak: 0.13, ..BANDPASS }],
        shimmer: None,
    }),
    // A brighter, springier tick — the "up" half of a press/release pair, like a key returning.
    ("release", Recipe {
        master_gain: 0.4,
        layers: &[
            Layer { frequency: 4600.0, filter_q: 1.8, attack: 0.001, decay: 0.016, peak: 0.12, ..BANDPASS },
            Layer { frequency: 3200.0, offset: 0.006, attack: 0.001, decay: 0.05, peak: 0.02, ..SINE },
        ],
        shimmer: None,
    }),
    // A two-part click-clack, like a mechanical switch flipping between states.
    ("toggle", Recipe {
        master_gain: 0.4,
        layers: &[
            Layer { frequency: 2200.0, filter_q: 1.6, attack: 0.001, decay: 0.016, peak: 0.12, ..BANDPASS },
            Layer { frequency: 3800.0, filter_q: 1.6, offset: 0.024, attack: 0.001, decay: 0.02, peak: 0.1, ..BANDPASS },
        ],
        shimmer: None,
    }),
    // A short, warm three-note ascending confirmation — "done", not a fanfare.
    ("success", Recipe {
        master_gain: 0.5,
        layers: &[
            Layer { frequency: 880.0, attack: 0.004, decay: 0.09, peak: 0.06, ..SINE },
            Layer { frequency: 1108.73, offset: 0.06, attack: 0.004, decay: 0.1, peak: 0.06, ..SINE },
            Layer { frequency: 1318.51, offset: 0.12, attack: 0.004, decay: 0.18, peak: 0.07, ..SINE },
        ],
        shimmer: Some(Shimmer { delay: 0.1, feedback: 0.22, wet: 0.16, lowpass: 4500.0 }),
    }),
    // A muted knock followed by two descending tones — a calm, recoverable refusal.
    ("error", Recipe {
        master_gain: 0.42,
        layers: &[
            Layer { frequency: 850.0, filter_q: 1.1, attack: 0.001, decay: 0.035, peak: 0.13, ..BANDPASS },
            Layer { frequency: 440.0, offset: 0.025, attack: 0.004, decay: 0.09, peak: 0.045, ..TRIANGLE },
            Layer { frequency: 349.23, offset: 0.1, attack: 0.004, decay: 0.14, peak: 0.04, ..TRIANGLE },
        ],
        shimmer: None,
    }),
    // A papery filtered flick with a tiny glass tick — for pages, galleries, and carousels.
    ("page", Recipe {
        master_gain: 0.38,
        layers: &[
            Layer { frequency: 1800.0, filter_q: 0.7, attack: 0.006, decay: 0.08, peak: 0.11, ..LOWPASS },
            Layer { frequency: 4200.0, filter_q: 1.2, offset: 0.04, attack: 0.004, decay: 0.065, peak: 0.08, ..BANDPASS },
            Layer { frequency: 2400.0, offset: 0.075, attack: 0.002, decay: 0.045, peak: 0.02, ..SINE },
        ],
        shimmer: None,
    }),
    // A brief unresolved lift — signals that user-initiated work has started.
    ("loading", Recipe {
        master_gain: 0.42,
        layers: &[
            Layer { frequency: 1400.0, filter_q: 0.6, attack: 0.035, decay: 0.14, peak: 0.035, ..LOWPASS },
            Layer { frequency: 420.0, glide_to: Some(630.0), glide_time: Some(0.18), attack: 0.025, decay: 0.18, peak: 0.05, ..SINE },
        ],
        shimmer: Some(Shimmer { delay: 0.11, feedback: 0.18, wet: 0.12, lowpass: 2800.0 }),
    }),
    // A quick lock-on sweep resolving to a clear tone — the system is ready.
    ("ready", Recipe {
        master_gain: 0.48,
        layers: &[
            Layer { frequency: 3600.0, filter_q: 1.8, attack: 0.001, decay: 0.02, peak: 0.11, ..BANDPASS },
            Layer { frequency: 330.0, glide_to: Some(660.0), glide_time: Some(0.12), offset: 0.012, attack: 0.004, decay: 0.16, peak: 0.055, ..TRIANGLE },
            Layer { frequency: 990.0, offset: 0.13, attack: 0.004, decay: 0.22, peak: 0.06, ..SINE },
        ],
        shimmer: Some(Shimmer { delay: 0.1, feedback: 0.16, wet: 0.1, lowpass: 4200.0 }),
    }),
    // A compact synthetic chirp — crisp feedback for primary buttons and controls.
    ("pulse", Recipe {
        master_gain: 0.42,
        layers: &[
            Layer { frequency: 2600.0, filter_q: 2.4, attack: 0.001, decay: 0.022, peak: 0.08, ..BANDPASS },
            Layer { frequency: 620.0, glide_to: Some(1240.0), glide_time: Some(0.07), attack: 0.002, decay: 0.085, peak: 0.055, ..TRIANGLE },
        ],
        shimmer: None,
    }),
    // A fast three-step locator signal — playful feedback for menus and secondary buttons.
    ("scan", Recipe {
        master_gain: 0.4,
        layers: &[
            Layer { frequency: 740.0, attack: 0.002, decay: 0.055, peak: 0.05, ..SINE },
            Layer { frequency: 1110.0, offset: 0.045, attack: 0.002, decay: 0.055, peak: 0.045, ..SINE },
            Layer { frequency: 1665.0, offset: 0.09, attack: 0.002, decay: 0.07, peak: 0.04, ..SINE },
        ],
        shimmer: Some(Shimmer { delay: 0.065, feedback: 0.16, wet: 0.1, lowpass: 4200.0 }),
    }),
    // A rising harmonic portal with a soft tail — for client-side page arrivals.
    ("arrival", Recipe {
        master_gain: 0.44,
        layers: &[
            Layer { frequency: 900.0, filter_q: 0.8, attack: 0.05, decay: 0.24, peak: 0.035, ..LOWPASS },
            Layer { frequency: 220.0, glide_to: Some(440.0), glide_time: Some(0.32), attack: 0.04, decay: 0.34, peak: 0.055, ..SINE },
            Layer { frequency: 659.25, offset: 0.12, attack: 0.045, decay: 0.32, peak: 0.04, ..SINE },
            Layer { frequency: 987.77, offset: 0.19, attack: 0.045, decay: 0.34, peak: 0.032, ..SINE },
        ],
        shimmer: Some(Shimmer { delay: 0.16, feedback: 0.28, wet: 0.18, lowpass: 3200.0 }),
    }),
];

pub const SOUNDS: &[&str] = &{
    let mut names = [""; RECIPES.len()];
    let mut i = 0;
    while i < names.len() {
        names[i] = RECIPES[i].0;
        i += 1;
    }
    names
};

#[cfg(test)]
pub fn is_sound(name: &str) -> bool {
    SOUNDS.contains(&name)
}

fn recipe(name: &str) -> Option<&'static Recipe> {
    static TABLE: [(&str, Recipe); 17] = RECIPES;
    TABLE.iter().find(|(n, _)| *n == name).map(|(_, r)| r)
}

// ---- synth ----

const RATE: f64 = 48000.0;
const FLOOR: f64 = 0.0001; // exponential ramps start and end here
const PAD: f64 = 0.05; // sources run this long past their envelope
const OUTPUT_GAIN: f64 = 4.0;
const THRESHOLD: f64 = -8.0; // the output compressor, as a static curve
const RATIO: f64 = 12.0;

// Web Audio's automatic makeup gain
fn makeup() -> f64 {
    (1.0 / 10f64.powf((THRESHOLD - THRESHOLD / RATIO) / 20.0)).powf(0.6)
}

// Math.round: a half rounds up, toward +∞ (Rust's round takes it away from zero).
fn js_round(x: f64) -> f64 {
    let r = x.floor();
    if x - r >= 0.5 {
        r + 1.0
    } else {
        r
    }
}

// exponentialRampToValueAtTime from FLOOR up to peak over attack, then back down over decay
fn envelope(t: f64, l: &Layer) -> f64 {
    if t < 0.0 || t > l.attack + l.decay {
        return 0.0;
    }
    if t < l.attack {
        FLOOR * (l.peak / FLOOR).powf(t / l.attack)
    } else {
        l.peak * (FLOOR / l.peak).powf((t - l.attack) / l.decay)
    }
}

// An RBJ-cookbook biquad, like BiquadFilterNode: lowpass Q is in dB, bandpass Q is linear.
fn biquad(kind: FilterType, frequency: f64, q: f64) -> impl FnMut(f64) -> f64 {
    let lowpass = kind == FilterType::Lowpass;
    let w = (2.0 * PI * frequency) / RATE;
    let cos = w.cos();
    let alpha = w.sin() / (2.0 * if lowpass { 10f64.powf(q / 20.0) } else { q });
    let (b0, b1, b2) = if lowpass { ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0) } else { (alpha, 0.0, -alpha) };
    let (a0, a1, a2) = (1.0 + alpha, -2.0 * cos, 1.0 - alpha);
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    move |x| {
        let y = (b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2) / a0;
        (x2, x1, y2, y1) = (x1, x, y1, y);
        y
    }
}

// Deterministic white noise (mulberry32), so a sound renders the same every time.
fn noise(mut seed: u32) -> impl FnMut() -> f64 {
    move || {
        seed = seed.wrapping_add(0x6d2b79f5);
        let mut t = (seed ^ (seed >> 15)).wrapping_mul(1 | seed);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        ((t ^ (t >> 14)) as f64 / 4294967296.0) * 2.0 - 1.0
    }
}

// Mixes into `out` like the TS's Float32Array: each sum is rounded to f32 as it's stored.
fn add_tone(out: &mut [f32], l: &Layer, waveform: Waveform) {
    let start = js_round(l.offset * RATE) as usize;
    let length = ((l.attack + l.decay + PAD) * RATE).ceil() as usize;
    let detune = 2f64.powf(l.detune / 1200.0);
    let glide = l.glide_time.unwrap_or(l.attack + l.decay);
    let mut phase = 0.0;
    for (i, s) in out.iter_mut().skip(start).take(length).enumerate() {
        let t = i as f64 / RATE;
        let f = match l.glide_to {
            None => l.frequency,
            Some(to) => l.frequency * (to / l.frequency).powf((t / glide).min(1.0)),
        } * detune;
        phase = (phase + f / RATE) % 1.0;
        let wave = match waveform {
            Waveform::Sine => (2.0 * PI * phase).sin(),
            Waveform::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
        };
        *s = (*s as f64 + wave * envelope(t, l)) as f32;
    }
}

fn add_noise(out: &mut [f32], l: &Layer, kind: FilterType, seed: u32) {
    let start = js_round(l.offset * RATE) as usize;
    let length = ((l.attack + l.decay + PAD) * RATE).ceil() as usize;
    let mut filter = biquad(kind, l.frequency, l.filter_q);
    let mut random = noise(seed);
    for (i, s) in out.iter_mut().skip(start).take(length).enumerate() {
        *s = (*s as f64 + filter(random()) * envelope(i as f64 / RATE, l)) as f32;
    }
}

// How long a recipe rings: its last source, plus echoes until they fall below -60 dB.
fn duration(r: &Recipe) -> f64 {
    let end = r.layers.iter().map(|l| l.offset + l.attack + l.decay + PAD).fold(f64::NEG_INFINITY, f64::max);
    let tail = match &r.shimmer {
        Some(s) if s.feedback > 0.0 => s.delay * (1.0 + (0.001f64.ln() / s.feedback.ln()).ceil()),
        _ => 0.0,
    };
    end + tail + 0.05
}

fn samples(r: &Recipe) -> usize {
    (duration(r) * RATE).ceil() as usize
}

// Mono PCM in -1..1 at RATE.
fn render_recipe(r: &Recipe) -> Vec<f32> {
    let mut dry = vec![0f32; samples(r)];
    for (i, l) in r.layers.iter().enumerate() {
        match l.source {
            Source::Tone(waveform) => add_tone(&mut dry, l, waveform),
            Source::Noise(kind) => add_noise(&mut dry, l, kind, i as u32 + 1),
        }
    }
    let makeup = makeup();
    // the echo: its lowpass and a ring buffer
    let mut shimmer = r.shimmer.as_ref().map(|s| (s, biquad(FilterType::Lowpass, s.lowpass, 1.0), vec![0f32; js_round(s.delay * RATE).max(1.0) as usize]));
    dry.iter()
        .enumerate()
        .map(|(i, &d)| {
            let master = d as f64 * r.master_gain;
            let mut wet = 0.0;
            if let Some((s, lowpass, delay)) = &mut shimmer {
                let at = i % delay.len();
                let echoed = lowpass(delay[at] as f64);
                delay[at] = (master + s.feedback * echoed) as f32;
                wet = s.wet * echoed;
            }
            let x = (master + wet) * OUTPUT_GAIN;
            let level = 20.0 * if x.abs() > 0.0 { x.abs() } else { 1e-9_f64 }.log10(); // 0 (or NaN) reads as 1e-9
            let limited = if level > THRESHOLD { x.signum() * 10f64.powf((THRESHOLD + (level - THRESHOLD) / RATIO) / 20.0) } else { x };
            (limited * makeup).clamp(-1.0, 1.0) as f32
        })
        .collect()
}

#[cfg(test)]
pub fn render(name: &str) -> Option<Vec<f32>> {
    recipe(name).map(render_recipe)
}

// 16-bit mono PCM WAV.
pub fn to_wav(pcm: &[f32]) -> Vec<u8> {
    let rate = RATE as u32;
    let data = pcm.len() as u32 * 2;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend(b"RIFF");
    b.extend((36 + data).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes()); // fmt chunk size
    b.extend(1u16.to_le_bytes()); // PCM
    b.extend(1u16.to_le_bytes()); // mono
    b.extend(rate.to_le_bytes());
    b.extend((rate * 2).to_le_bytes()); // bytes per second
    b.extend(2u16.to_le_bytes()); // bytes per frame
    b.extend(16u16.to_le_bytes()); // bits per sample
    b.extend(b"data");
    b.extend(data.to_le_bytes());
    for &s in pcm {
        b.extend((js_round(s as f64 * 32767.0) as i16).to_le_bytes());
    }
    b
}

// ---- player ----

// Plays cuelume sounds with the system's command-line player. Each sound is rendered once into <DIR>/sounds and kept
// there. No player (or MODISA_SOUND=off) means silence.

#[derive(Clone, Copy)]
enum Player {
    Afplay,
    Paplay,
    Aplay,
}

// ponytail: the TS played through OpenTUI's audio engine; here it's whichever player is on PATH, a process per sound:
// afplay (macOS), else paplay (PulseAudio/PipeWire), else aplay (ALSA, no volume).
fn player() -> Option<(Player, &'static str)> {
    static PLAYER: OnceLock<Option<(Player, String)>> = OnceLock::new();
    let found = PLAYER.get_or_init(|| {
        if std::env::var("MODISA_SOUND").is_ok_and(|v| v == "off") {
            return None;
        }
        [(Player::Afplay, "afplay"), (Player::Paplay, "paplay"), (Player::Aplay, "aplay")].into_iter().find_map(|(p, exe)| Some((p, which(exe)?)))
    });
    found.as_ref().map(|(p, exe)| (*p, exe.as_str()))
}

// The sound's WAV in `dir`, rendered and written unless a file of the right size is already there.
fn wav_in(dir: &str, name: &str) -> Option<String> {
    let recipe = recipe(name)?;
    let path = format!("{dir}/{name}.wav");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() == 44 + 2 * samples(recipe) as u64) {
        return Some(path);
    }
    std::fs::create_dir_all(dir).ok()?;
    // written aside and renamed in, so another client playing it never reads half a file
    let tmp = format!("{path}.{}.tmp", std::process::id());
    if std::fs::write(&tmp, to_wav(&render_recipe(recipe))).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    Some(path)
}

thread_local! {
    // Players still running. Each is reaped once it's done, when a later sound starts, so none lingers as a zombie.
    static PLAYING: RefCell<Vec<Child>> = const { RefCell::new(Vec::new()) };
}

// true if it started playing; never waits for it
pub fn play(name: &str, volume: f64) -> bool {
    let Some((player, exe)) = player() else { return false };
    let Some(path) = wav_in(&format!("{}/sounds", *DIR), name) else { return false };
    let volume = volume.clamp(0.0, 1.0);
    let mut cmd = Command::new(exe);
    match player {
        Player::Afplay => cmd.arg("-v").arg(volume.to_string()),
        Player::Paplay => cmd.arg(format!("--volume={}", (volume * 65536.0).round() as u32)),
        Player::Aplay => cmd.arg("-q"),
    };
    cmd.arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    PLAYING.with_borrow_mut(|playing| {
        playing.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
        cmd.spawn().map(|c| playing.push(c)).is_ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_recipe_renders_audible_unclipped_pcm_of_the_right_length() {
        assert_eq!(SOUNDS.len(), 17);
        for (name, recipe) in &RECIPES {
            let pcm = render(name).unwrap();
            assert_eq!(pcm.len(), (duration(recipe) * RATE).ceil() as usize, "{name}");
            assert!(pcm.iter().all(|s| s.is_finite()), "{name}: a NaN or Infinity");
            let peak = pcm.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.05 && peak <= 1.0, "{name}: peak {peak}");
        }
        // deterministic, noise included
        assert_eq!(render("chime"), render("chime"));
        assert_eq!(render("page"), render("page"));
        assert_eq!(render("nope"), None);
    }

    #[test]
    fn sounds_are_16_bit_mono_wavs() {
        let pcm = render("success").unwrap();
        let wav = to_wav(&pcm);
        assert_eq!([&wav[0..4], &wav[8..12], &wav[12..16], &wav[36..40]], [b"RIFF", b"WAVE", b"fmt ", b"data"]);
        let u16_at = |at: usize| u16::from_le_bytes([wav[at], wav[at + 1]]);
        let u32_at = |at: usize| u32::from_le_bytes([wav[at], wav[at + 1], wav[at + 2], wav[at + 3]]);
        assert_eq!((u16_at(20), u16_at(22), u32_at(24), u16_at(34)), (1, 1, 48000, 16));
        assert_eq!(u32_at(4) as usize, wav.len() - 8);
        assert_eq!(u32_at(40) as usize, pcm.len() * 2);
        // samples round like Math.round: a half goes toward +∞
        assert_eq!(to_wav(&[1.0, -1.0, 0.5, -0.5, 0.0])[44..], [0xff_u8, 0x7f, 0x01, 0x80, 0x00, 0x40, 0x01, 0xc0, 0x00, 0x00]);
    }

    #[test]
    fn sound_names_match_the_config() {
        assert_eq!(SOUNDS, crate::config::SOUND_NAMES);
        assert!(is_sound("chime") && is_sound("arrival"));
        assert!(!is_sound("off") && !is_sound("toString") && !is_sound(""));
    }

    #[test]
    fn wavs_are_written_once_and_reused() {
        let dir = std::env::temp_dir().join(format!("modisa-sound-test-{}", std::process::id())).to_string_lossy().into_owned();
        let path = wav_in(&dir, "tick").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), to_wav(&render("tick").unwrap()));
        let fake = vec![7u8; std::fs::metadata(&path).unwrap().len() as usize];
        std::fs::write(&path, &fake).unwrap();
        assert_eq!(std::fs::read(wav_in(&dir, "tick").unwrap()).unwrap(), fake); // the right size: kept
        std::fs::write(&path, b"short").unwrap();
        assert_eq!(std::fs::read(wav_in(&dir, "tick").unwrap()).unwrap(), to_wav(&render("tick").unwrap())); // rewritten
        assert_eq!(wav_in(&dir, "nope"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
