// Methods only the TUI client uses: attaching, screen replay, input, and the UI commands behind keys,
// menus and mouse gestures.
import { b64, unb64 } from "../../protocol/conn";
import type { ServerContext } from "../context";
import type { Handlers } from "./dispatch";

export function clientMethods(ctx: ServerContext): Handlers {
  const { s } = ctx;
  return {
    attach: (p, c) => {
      c.attached = true;
      if (p.area) s.setArea(p.area);
      ctx.emit("client.attached", {});
      return { ...s.view(), paused: ctx.mail.paused, plugins: ctx.pluginUi(), session: ctx.session, prompts: [...ctx.prompts.keys()], version: ctx.version };
    },
    // Current screen of every pane as a VT stream; the client asks once its terminals exist.
    replay: () => [...s.panes.values()].map((p) => ({ pane: p.id, data: b64(new TextEncoder().encode(p.replay())) })),
    detach: (_p, c) => {
      c.attached = false;
      ctx.changed();
    },
    "detach-all": () => ctx.broadcast("detach", {}),
    area: (p) => s.setArea(p.area),
    input: (p) => s.panes.get(p.pane)?.write(unb64(p.data)),
    promptReply: (p) => ctx.prompts.get(p.id)?.(p.answer),
    cmd: (p, c) => {
      const a = p.args ?? {};
      const ops: Record<string, () => any> = {
        split: () => s.split(a.dir),
        close: () => s.close(a.pane),
        closeTab: () => s.closeTab(),
        focusDir: () => s.focusDir(a.dir),
        focusPane: () => s.focusPane(a.pane),
        zoom: () => s.zoom(),
        resize: () => s.resizePane(a.dir, a.cells),
        selectTab: () => s.selectTab(a.index),
        cycleTab: () => s.cycleTab(a.step),
        newTab: () => s.newTab(a.name, a.command ? { command: a.command, ephemeral: a.ephemeral } : {}),
        selectWorkspace: () => s.selectWorkspace(a.index),
        newWorkspace: () => s.newWorkspace(a.name, a.cwd ?? s.ws.cwd),
        renameTab: () => s.renameTab(a.name),
        renameWorkspace: () => s.renameWorkspace(a.name, a.index),
        closeWorkspace: () => s.closeWorkspace(a.index),
        renamePane: () => s.renamePane(a.pane ?? s.focusedId, a.name),
        dragStart: () => s.dragStart(c, a.x, a.y),
        dragMove: () => s.dragMove(c, a.x, a.y),
        dragEnd: () => s.dragEnd(c),
        spawnAgent: () => s.split("row", ctx.agentOpts(a.harness, undefined, a.name)),
        pause: () => { ctx.mail.paused = !ctx.mail.paused; ctx.changed(); },
      };
      const op = ops[p.name];
      if (!op) throw new Error(`unknown command ${p.name}`);
      op();
      return true;
    },
    search: (p) => {
      const lines = ctx.need(p.pane).text().split("\n");
      const q = p.query.toLowerCase();
      return { total: lines.length, matches: lines.flatMap((l, i) => (l.toLowerCase().includes(q) ? [i] : [])) };
    },
    adapters: () => ctx.adapters.filter((a) => a.id !== "generic").map((a) => ({ id: a.id, name: a.name })),
  };
}
