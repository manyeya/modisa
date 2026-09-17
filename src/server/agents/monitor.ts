// Every 500ms: read each agent's state, tell attached clients when one needs attention, and type queued
// messages into agents that are idle.
import type { ServerContext } from "../context";
import { Mailbox } from "./mailbox";

export function startMonitor(ctx: ServerContext): { stop(): void } {
  const cooldown = new Map<string, number>();
  // One tick at a time, but a tick that never finishes must not stop detection for good: after 10s the
  // next one starts anyway. Failures go to the server log, each distinct one once.
  let running = 0; // id of the tick in progress, 0 when none
  let startedAt = 0;
  let lastError = "";
  const focused = (id: string) => ctx.attached().length > 0 && ctx.s.focusedId === id && ctx.s.isVisible(id);

  let ids = 0;
  ctx.tick = async () => {
    if (running && Date.now() - startedAt < 10_000) return;
    if (running) console.error(`modisa: agent detection was stuck for ${Math.round((Date.now() - startedAt) / 1000)}s; starting over`);
    const id = (running = ++ids);
    startedAt = Date.now();
    try {
      const changes = await ctx.detector.tick([...ctx.s.panes.values()], focused);
      for (const { pane, from, to } of changes) {
        ctx.emit("agent.state", { pane: pane.id, instance: pane.info.instance, name: pane.info.name, harness: pane.info.agent?.harness, from, to });
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
      lastError = "";
    } catch (e) {
      const message = e instanceof Error ? e.stack ?? e.message : String(e);
      if (message !== lastError) console.error(`modisa: agent detection failed: ${message}`);
      lastError = message;
    } finally {
      if (running === id) running = 0; // a stuck tick that finishes late leaves the newer one alone
    }
  };
  const ticker = setInterval(() => ctx.tick(), 500);
  return { stop: () => clearInterval(ticker) };
}
