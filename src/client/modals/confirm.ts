// A question with buttons: yes/no before something destructive, or the answers a permission request can have.
// Each button answers to its key, a click, or ←→ / tab and ↵.
import type { App } from "../context";
import { fit } from "../design";
import { blank, button, clear, frame, frameLayouts, header, innerWidth, open, row, spacer, text } from "./frame";

export type Choice<T> = { label: string; key: string; value: T; tone?: "danger" | "primary" };

export function ask<T>(app: App, title: string, body: string, choices: Choice<T>[], start = 0): Promise<T | null> {
  const lines = body.split("\n");
  const box = frame(app, lines.length + 6, 60);
  const place = frameLayouts.get(box)!;
  let sel = start;
  let answer: (v: T | null) => void = () => {};
  const paint = () => {
    if (box.isDestroyed) return;
    const { th } = app;
    place();
    clear(box);
    header(app, box, title);
    blank(app, box);
    for (const l of lines) text(app, row(app, box), fit(l, innerWidth(box)), th.fg);
    blank(app, box);
    const buttons = row(app, box);
    spacer(app, buttons);
    choices.forEach((c, i) => {
      if (i) text(app, buttons, " ", th.fg);
      const tone = c.tone === "danger" ? th.blocked : c.tone === "primary" ? th.accent : th.focus;
      button(app, buttons, c.label, c.key, tone, i === sel, () => answer(c.value));
    });
  };
  frameLayouts.set(box, paint);
  paint();
  return open<T>(app, box, (done) => {
    answer = done;
    return (k) => {
      const chosen = choices.find((c) => c.key === k.name);
      if (chosen) done(chosen.value);
      else if (k.name === "left" || (k.name === "tab" && k.shift)) (sel = (sel - 1 + choices.length) % choices.length, paint());
      else if (k.name === "right" || k.name === "tab") (sel = (sel + 1) % choices.length, paint());
      else if (k.name === "return") done(choices[sel]!.value);
      return true;
    };
  });
}

// Cancel is where the focus starts: ↵ alone never deletes anything.
export function confirm(app: App, title: string, text: string, yes: string): Promise<boolean> {
  const danger = /delete|close|kill|remove/i.test(yes);
  return ask(app, title, text, [
    { label: "Cancel", key: "n", value: false },
    { label: yes[0]!.toUpperCase() + yes.slice(1), key: "y", value: true, tone: danger ? "danger" : "primary" },
  ]).then((v) => v === true);
}
