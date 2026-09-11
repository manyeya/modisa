// The server asks whether an agent may act on a pane it didn't create: allow, always, or deny.
import { TextRenderable } from "@opentui/core";
import type { App } from "../context";
import { frame, open } from "./frame";

export function permission(app: App, id: number, text: string) {
  const { r, th } = app;
  const lines = text.split("\n");
  const box = frame(app, "permission", lines.length + 4);
  for (const l of lines) box.add(new TextRenderable(r, { content: l, fg: th.fg, height: 1 }));
  box.add(new TextRenderable(r, { content: "", height: 1 }));
  box.add(new TextRenderable(r, { content: "[y] allow   [a] always   [n] deny", fg: th.warn, height: 1 }));
  open<string>(app, box, (done) => (k) => {
    const answer = { y: "allow", a: "always", n: "deny" }[k.name as "y" | "a" | "n"];
    if (!answer) return true;
    app.conn.notify("promptReply", { id, answer });
    done(answer);
    return true;
  }).then((v) => v === null && app.conn.notify("promptReply", { id, answer: "deny" }));
  app.promptIds.set(id, box);
}
