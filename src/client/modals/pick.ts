// Choose one option from a list; typing filters it.
import { SelectRenderable, SelectRenderableEvents } from "@opentui/core";
import type { App, Option } from "../context";
import { frame, open } from "./frame";

export function pick(app: App, title: string, options: Option[]) {
  const { th } = app;
  const box = frame(app, title, Math.min(options.length * 2 + 2, app.r.height - 4));
  const sel = new SelectRenderable(app.r, {
    options, width: "100%", flexGrow: 1, backgroundColor: th.bg, textColor: th.fg,
    focusedBackgroundColor: th.bg, selectedBackgroundColor: th.focus, selectedTextColor: th.bg, descriptionColor: th.dim, wrapSelection: true,
  });
  box.add(sel);
  let filter = "";
  const all = options;
  return open<string>(app, box, (done) => {
    sel.on(SelectRenderableEvents.ITEM_SELECTED, (_i: number, o: Option) => done(o.value));
    sel.focus();
    // type to filter
    return (k) => {
      if (k.name === "backspace") filter = filter.slice(0, -1);
      else if (k.sequence.length === 1 && k.sequence >= " " && !k.ctrl && k.name !== "j" && k.name !== "k") filter += k.sequence;
      else return false;
      sel.options = all.filter((o) => (o.name + " " + o.description).toLowerCase().includes(filter.toLowerCase()));
      box.title = ` ${title}${filter ? `: ${filter}` : ""} `;
      return true;
    };
  });
}
