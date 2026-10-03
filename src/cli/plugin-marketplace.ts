// `modisa plugin marketplace add|list|update|remove`: config/marketplaces.ts does each, and this prints what it did.
// No session is needed: marketplaces live in the state directory, shared by every session.
import { addMarketplace, listMarketplaces, removeMarketplace, updateMarketplaces } from "../config/marketplaces";
import { str, type Args } from "./args";
import { table } from "./commands";

const USAGE = "usage: modisa plugin marketplace add <owner/repo | git-url> [--ref r] | list | update [name] | remove <name>   [--json]";
const short = (commit?: string) => commit?.slice(0, 7) ?? "";

export async function marketplace(args: string[], flags: Args["flags"]): Promise<number> {
  const [verb, arg] = args;
  const json = !!flags.json;
  const print = (x: unknown) => console.log(JSON.stringify(x, null, 2));
  try {
    switch (verb) {
      case "add": {
        if (!arg) break;
        const r = await addMarketplace(arg, str(flags.ref));
        if (json) print(r);
        else if (r.stage) console.error(`modisa: marketplace not added (${r.stage}): ${r.reason}`);
        else {
          const what = r.alreadyAdded ? `marketplace ${r.name} is already added` : `added marketplace ${r.name}`;
          console.log(`${what} from ${r.source} (${r.ref ? `ref ${r.ref}, ` : ""}commit ${short(r.commit)})${r.description ? `: ${r.description}` : ""}`);
          console.log(`  ${r.plugins!.length} plugin${r.plugins!.length === 1 ? "" : "s"}${r.plugins!.length ? `: ${r.plugins!.join(", ")}` : ""}`);
          console.log(r.alreadyAdded ? `  modisa plugin marketplace update ${r.name} fetches its latest` : `  install one with modisa plugin install <plugin>@${r.name}. A marketplace is a list, not a review: each plugin runs as you.`);
        }
        return r.stage ? 1 : 0;
      }
      case "list": {
        const list = await listMarketplaces();
        if (json) print(list);
        else table(list.map((m) => ({ ...m, commit: short(m.commit), ref: m.ref ?? "", plugins: m.error ? `(${m.error})` : m.plugins, description: m.description ?? "" })), ["name", "plugins", "source", "ref", "commit", "description"]);
        return 0;
      }
      case "update": {
        const results = await updateMarketplaces(arg);
        if (json) print(results);
        else if (!results.length) console.log("no marketplaces added (modisa plugin marketplace add <owner/repo>)");
        for (const r of json ? [] : results) {
          if ("reason" in r) console.error(`modisa: marketplace ${r.name} not updated: ${r.reason}`);
          else console.log(r.updated ? `updated marketplace ${r.name}: ${short(r.from)} → ${short(r.commit)}, ${r.plugins} plugins` : `marketplace ${r.name} is up to date (${short(r.commit)})`);
        }
        return results.some((r) => "reason" in r) ? 1 : 0;
      }
      case "remove": {
        if (!arg) break;
        const r = await removeMarketplace(arg);
        if (json) print(r);
        else {
          console.log(`removed marketplace ${r.name} (deleted ${r.dir})`);
          if (r.installed.length) console.log(`still installed from it: ${r.installed.join(", ")} (modisa plugin unlink <name> removes one; plugin update can't fetch one that came from inside it)`);
        }
        return 0;
      }
    }
  } catch (e) {
    const message = (e as Error).message;
    console.error(json ? JSON.stringify({ error: { code: "error", message } }) : `modisa: ${message}`);
    return 1;
  }
  console.error(USAGE);
  return 2;
}
