// attention-log: when an agent newly becomes blocked, append one line to attention.log in the plugin's data
// directory, and show who's blocked in shepherd's TUI: a count in the status row, a sidebar row per blocked agent
// (clicking it focuses that agent), and a badge on its pane until someone marks it seen from the pane's menu. Prefix A
// shows the log in a popup. Agents already blocked in the startup snapshot are shown but not logged; an agent that
// stops being blocked and blocks again is logged again. A disconnect is a gap: an agent that blocked and unblocked
// while the plugin was away isn't logged. Read AGENTS.md before changing it.
import { runPlugin } from "./shepherd-plugin";

const LOG = `${Bun.env.SHEPHERD_PLUGIN_DATA ?? import.meta.dir}/attention.log`;

type Blocked = { pane: string; instance: string; name?: string; harness?: string };

runPlugin(async (shepherd) => {
  const blocked = new Map<string, Blocked>(); // by pane instance: the agents blocked right now
  const seen = new Set<string>(); // instances whose badge the user dismissed while they stay blocked
  const badged = new Map<string, string>(); // pane → the instance its badge is for
  let logged = 0;

  // The TUI follows `blocked`, redrawn at most every 250ms so a burst of events stays within the update rate. A failed
  // update is logged, not thrown: the next change redraws everything anyway.
  let pending: ReturnType<typeof setTimeout> | undefined;
  const show = () => {
    pending ??= setTimeout(async () => {
      pending = undefined;
      const agents = [...blocked.values()];
      try {
        if (agents.length) {
          await shepherd.ui.status("blocked", `${agents.length} blocked`, { tone: "warn", action: "summary" });
          await shepherd.ui.sidebar("Blocked agents", agents.map((a) => ({ text: [a.name ? `@${a.name}` : a.pane, a.harness].filter(Boolean).join(" "), tone: "warn", pane: a.pane, instance: a.instance })));
        } else {
          await shepherd.ui.clearStatus("blocked");
          await shepherd.ui.clearSidebar();
        }
        const wanted = new Map(agents.filter((a) => !seen.has(a.instance)).map((a) => [a.pane, a.instance]));
        for (const [pane, instance] of badged) if (wanted.get(pane) !== instance) (await shepherd.ui.clearBadge(pane), badged.delete(pane));
        for (const [pane, instance] of wanted) {
          if (badged.get(pane) === instance) continue;
          // a pane that closed meanwhile refuses its badge (pane_gone): nothing to show
          await shepherd.ui.badge(pane, instance, "blocked", "warn").then(() => badged.set(pane, instance), () => {});
        }
      } catch (error) {
        console.error(`attention-log: couldn't update the TUI: ${error instanceof Error ? error.message : error}`);
      }
    }, 250);
  };
  const unblock = (instance: string) => {
    blocked.delete(instance);
    seen.delete(instance);
    show();
  };

  await shepherd.hello({
    summary: () => ({ blocked: blocked.size, logged, log: LOG }),
    clear: async () => {
      await Bun.write(LOG, "");
      logged = 0;
      return "cleared";
    },
    // from a pane's context menu (or a key or the palette): shepherd says which pane, already checked to be current
    seen: (_params, call) => {
      if (!call.target) throw new Error("take this from a pane: its context menu, or the palette with that pane focused");
      seen.add(call.target.instance);
      show();
      return `${call.target.pane} marked seen`;
    },
  });
  await shepherd.ui.menu([{ id: "seen", title: "Mark seen", action: "seen" }]);

  await shepherd.subscribe({
    // already blocked when the plugin started: shown, but not news for the log
    onSnapshot: (snapshot) => {
      for (const pane of snapshot.panes) {
        if (pane.agent?.state === "blocked") blocked.set(pane.instance, { pane: pane.id, instance: pane.instance, name: pane.name, harness: pane.agent.harness });
      }
      show();
    },
    onEvent: async (event) => {
      if (!event.instance || !event.pane) return;
      if (event.type === "process.exited") return unblock(event.instance);
      if (event.type !== "agent.state") return;
      if (event.to !== "blocked") return unblock(event.instance);
      if (blocked.has(event.instance)) return;
      blocked.set(event.instance, { pane: event.pane, instance: event.instance, name: event.name, harness: event.harness });
      show();
      logged++;
      const entry = { at: new Date(event.at).toISOString(), pane: event.pane, instance: event.instance, name: event.name, harness: event.harness };
      // ponytail: rewrites the whole file per entry; fine for a log of attention events, append if it ever grows large
      await Bun.write(LOG, (await Bun.file(LOG).text().catch(() => "")) + JSON.stringify(entry) + "\n");
    },
  });
  console.log(`attention-log: watching; logging to ${LOG}`);
});
