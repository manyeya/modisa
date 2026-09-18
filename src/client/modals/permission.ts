// The server asks whether an agent may act on a pane it didn't create: allow, always, or deny.
import type { App } from "../context";
import { ask } from "./confirm";

export function permission(app: App, id: number, text: string) {
  ask(app, "Permission", text, [
    { label: "Deny", key: "n", value: "deny", tone: "danger" },
    { label: "Always", key: "a", value: "always" },
    { label: "Allow", key: "y", value: "allow", tone: "primary" },
  ], 2).then((answer) => app.conn.notify("promptReply", { id, answer: answer ?? "deny" }));
  app.promptIds.add(id);
}
