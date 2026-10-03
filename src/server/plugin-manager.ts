// The plugin manager's requests: plugin.resolve, install, update, unlink, logs and catalog, and marketplace.*. They do
// what the CLI does (config/plugin-manage.ts, config/marketplaces.ts) on the server's machine, so a --remote TUI
// manages plugins where they run. Only the user can: an agent in a pane (a caller) or a plugin's own connection is
// refused, since these fetch code that then runs as the user. Git runs async, so a slow clone holds nothing else up.
import { find } from "../cli/plugin-search";
import { cleanText } from "../core/text";
import { installs } from "../config/plugins";
import { addMarketplace, listMarketplaces, marketplacePlugins, removeMarketplace, updateMarketplaces } from "../config/marketplaces";
import { installPlugin, resolvePlugin, startWith, unlinkPlugin, updatePlugin, type Here } from "../config/plugin-manage";
import { fail } from "../protocol/conn";
import type { Client, ServerContext } from "./context";
import type { Handlers } from "./rpc/dispatch";

export function pluginManager(ctx: ServerContext, host: { methods: Handlers; forget: (name: string) => void }): Handlers {
  // this session, through its own plugin host
  const here = (c: Client): Here => ({ session: ctx.session, ask: async (method, params = {}) => host.methods[method]!(params, c) });

  const methods: Handlers = {
    "plugin.resolve": (p) => resolvePlugin(p),
    "plugin.install": async (p, c) => {
      const result = await installPlugin(p);
      if (!result.installed && !result.alreadyInstalled) return result;
      const { hints, ...rest } = result;
      return { ...rest, start: await startWith(here(c).ask, ctx.session, result.name!), ...(hints && { hints }) };
    },
    "plugin.update": (p, c) => updatePlugin(p.name, { here: here(c) }),
    // and, here, out of the plugin list once stopped (the CLI's unlink leaves it listed as stopped)
    "plugin.unlink": async (p, c) => {
      const result = await unlinkPlugin(p.name, { here: here(c) });
      host.forget(p.name);
      return result;
    },
    // the last lines of its log here, cleaned to show in a terminal
    "plugin.logs": async (p, c) => {
      const pl = (await here(c).ask<any[]>("plugin.list")).find((x) => x.name === p.name);
      if (!pl) throw fail("no_such_plugin", `no plugin named ${p.name} (see modisa plugin list)`);
      const lines = (await Bun.file(pl.log).text().catch(() => "")).trimEnd().split("\n").slice(-p.lines);
      return { name: pl.name, log: pl.log, text: lines.map((l) => cleanText(l.replace(/\t/g, "  "), 1000)).join("\n") };
    },
    // the index's plugins (as `plugin search` finds them) and the marketplaces', each saying whether it's here
    "plugin.catalog": async (p) => {
      const words = (p.query ?? "").split(/\s+/).filter(Boolean);
      const [index, listed, records] = await Promise.all([find(words, 100).catch((e: Error) => ({ total: 0, results: [], error: e.message })), marketplacePlugins(words), installs()]);
      const sources = new Set(records.map((r) => r.source));
      const results = index.results.map((r) => {
        const source = r.install.replace(/^modisa plugin install /, ""); // its clone URL
        return { ...r, source, installed: sources.has(source) };
      });
      return { query: words.join(" "), index: { total: index.total, results, ...("error" in index && { error: index.error }) }, marketplaces: listed };
    },
    "marketplace.list": () => listMarketplaces(),
    "marketplace.add": (p) => addMarketplace(p.source, p.ref),
    "marketplace.update": (p) => updateMarketplaces(p.name),
    "marketplace.remove": (p) => removeMarketplace(p.name),
  };
  return Object.fromEntries(
    Object.entries(methods).map(([method, handle]) => [
      method,
      (p: any, c: Client) => {
        if (p.caller || c.plugin) throw new Error("only the user can manage plugins and marketplaces"); // not agents in panes, not plugins
        return handle(p, c);
      },
    ]),
  );
}
