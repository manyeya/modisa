// Plays cuelume sounds through OpenTUI's native audio engine. The output device opens on the first
// sound; each sound is rendered once and kept. No device (or SHEPHERD_SOUND=off) means silence.
import { Audio, type AudioSound } from "@opentui/core";
import { RECIPES, isSound } from "./recipes";
import { render, toWav } from "./synth";

let audio: Audio | null | undefined; // undefined: not tried yet; null: unavailable
const loaded = new Map<string, AudioSound>();

function engine() {
  if (audio !== undefined) return audio;
  if (Bun.env.SHEPHERD_SOUND === "off") return (audio = null);
  try {
    const a = Audio.create({ autoStart: false });
    a.on("error", () => {}); // failed calls return false/null, which is all we need
    audio = a.start() ? a : (a.dispose(), null);
  } catch {
    audio = null;
  }
  return audio;
}

// true if it's playing
export function playSound(name: string, volume = 1): boolean {
  const a = engine();
  if (!a || !isSound(name)) return false;
  let sound = loaded.get(name);
  if (!sound) {
    const decoded = a.loadSound(toWav(render(RECIPES[name])));
    if (!decoded) return false;
    loaded.set(name, (sound = decoded));
  }
  return a.play(sound, { volume }) !== null;
}
