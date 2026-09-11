// Every 500ms: read each agent's state, tell attached clients when one needs attention, and type queued
// messages into agents that are idle.
import type { ServerContext } from "../context";
import { Mailbox } from "./mailbox";

export function startMonitor(ctx: ServerContext): { stop(): void } {
  const cooldown = new Map<string, number>();
  let ticking = false;
  const focused = (id: string) => ctx.attached().length > 0 && ctx.s.focusedId === id && ctx.s.isVisible(id);

  ctx.tick = async () => {
    if (ticking) return;
    ticking = true;
    try {
      const changes = await ctx.detector.tick([...ctx.s.panes.values()], focused);
      for (const { pane, from, to } of changes) {
        ctx.emit("agent.state", { pane: pane.id, name: pane.info.name, harness: pane.info.agent?.harness, from, to });
        if (to !== "idle" && !focused(pane.id)) {
          const what = { blocked: "is blocked — needs you", done: "is done", working: "started working" }[to];
          ctx.broadcast("notify", { pane: pane.id, state: to, text: `${pane.info.name ? "@" + pane.info.name : pane.info.title} ${what}` });
        }
      }
      if (!ctx.mail.paused) {
        for (const p of ctx.s.panes.values()) {
          const st = p.info.agent?.state;
          if ((st !== "idle" && st !== "done") || (cooldown.get(p.id) ?? 0) > Date.now()) continue;
          const m = ctx.mail.pending(p.id)[0];
          if (!m) continue;
          p.paste(Mailbox.frame(m));
          setTimeout(() => p.write("\r"), 150);
          ctx.mail.markDelivered(m);
          cooldown.set(p.id, Date.now() + 4000);
          ctx.emit("message.delivered", { id: m.id, from: m.fromName, to: m.toName });
        }
      }
      if (changes.length) ctx.changed();
    } finally {
      ticking = false;
    }
  };
  const ticker = setInterval(() => ctx.tick(), 500);
  return { stop: () => clearInterval(ticker) };
}
