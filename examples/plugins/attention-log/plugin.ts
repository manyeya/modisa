// attention-log: when an agent newly becomes blocked, append one line to attention.log in the plugin's data
// directory. Agents already blocked in the startup snapshot aren't logged; an agent that stops being blocked and
// blocks again is. A disconnect is a gap: an agent that blocked and unblocked while the plugin was away isn't logged.
// Read AGENTS.md before changing it.
import { runPlugin } from "./shepherd-plugin";

const LOG = `${Bun.env.SHEPHERD_PLUGIN_DATA ?? import.meta.dir}/attention.log`;

runPlugin(async (shepherd) => {
  const blocked = new Set<string>(); // pane instances whose agent is blocked right now
  let logged = 0;

  await shepherd.hello({
    summary: () => ({ blocked: blocked.size, logged, log: LOG }),
  });

  await shepherd.subscribe({
    // already blocked when the plugin started: not news
    onSnapshot: (snapshot) => {
      for (const pane of snapshot.panes) if (pane.agent?.state === "blocked") blocked.add(pane.instance);
    },
    onEvent: async (event) => {
      if (!event.instance) return;
      if (event.type === "process.exited") return void blocked.delete(event.instance);
      if (event.type !== "agent.state") return;
      if (event.to !== "blocked") return void blocked.delete(event.instance);
      if (blocked.has(event.instance)) return;
      blocked.add(event.instance);
      logged++;
      const entry = { at: new Date(event.at).toISOString(), pane: event.pane, instance: event.instance, name: event.name, harness: event.harness };
      // ponytail: rewrites the whole file per entry; fine for a log of attention events, append if it ever grows large
      await Bun.write(LOG, (await Bun.file(LOG).text().catch(() => "")) + JSON.stringify(entry) + "\n");
    },
  });
  console.log(`attention-log: watching; logging to ${LOG}`);
});
