// Yes/no before something destructive.
import { TextRenderable } from "@opentui/core";
import type { App } from "../context";
import { frame, open } from "./frame";

export function confirm(app: App, title: string, text: string, yes: string): Promise<boolean> {
  const { r, th } = app;
  const lines = text.split("\n");
  const box = frame(app, title, lines.length + 4);
  for (const l of lines) box.add(new TextRenderable(r, { content: l, fg: th.fg, height: 1 }));
  box.add(new TextRenderable(r, { content: "", height: 1 }));
  box.add(new TextRenderable(r, { content: `[y] ${yes}   [n] cancel`, fg: th.warn, height: 1 }));
  return open<boolean>(app, box, (done) => (k) => {
    if (k.name === "y") done(true);
    else if (k.name === "n") done(false);
    return true;
  }).then((v) => v === true);
}
