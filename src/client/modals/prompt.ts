// Ask for a line of text: ↵ or the OK button takes it, esc or Cancel doesn't.
import { InputRenderable, InputRenderableEvents } from "@opentui/core";
import type { App } from "../context";
import { blank, button, frame, header, open, row, spacer, text } from "./frame";

export function prompt(app: App, title: string, value = "", placeholder = "") {
  const { th } = app;
  const box = frame(app, 7, 60);
  header(app, box, title);
  blank(app, box);
  const field = row(app, box, { bg: th.bg });
  text(app, field, " › ", th.accent, { bg: th.bg });
  const input = new InputRenderable(app.r, {
    value, placeholder, flexGrow: 1, textColor: th.fg, backgroundColor: th.bg, focusedBackgroundColor: th.bg, focusedTextColor: th.fg, placeholderColor: th.dim, cursorColor: th.focus,
  });
  field.add(input);
  field.onMouseDown = (e) => { e.stopPropagation(); input.focus(); };
  blank(app, box);
  const buttons = row(app, box);
  spacer(app, buttons);
  let answer: (v: string | null) => void = () => {};
  button(app, buttons, "Cancel", "esc", th.fg, false, () => answer(null));
  text(app, buttons, " ", th.fg);
  button(app, buttons, "OK", "↵", th.accent, true, () => answer(input.value));
  return open<string>(app, box, (done) => {
    answer = done;
    input.on(InputRenderableEvents.ENTER, () => done(input.value));
    input.focus();
  });
}
