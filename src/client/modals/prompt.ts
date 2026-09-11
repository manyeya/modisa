// Ask for a line of text.
import { InputRenderable, InputRenderableEvents } from "@opentui/core";
import type { App } from "../context";
import { frame, open } from "./frame";

export function prompt(app: App, title: string, value = "") {
  const { th } = app;
  const box = frame(app, title, 3);
  const input = new InputRenderable(app.r, { value, width: "100%", textColor: th.fg, backgroundColor: th.bg, focusedBackgroundColor: th.bg });
  box.add(input);
  return open<string>(app, box, (done) => {
    input.on(InputRenderableEvents.ENTER, () => done(input.value));
    input.focus();
  });
}
