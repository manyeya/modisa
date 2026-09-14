// `shepherd <noun> <verb>` — the socket API as shell commands, so any agent can drive panes with zero integration.
import { connectExisting } from "../protocol/transport";
import { ConnectionClosedError, errorCode, fail, type Conn } from "../protocol/conn";
import type { ErrorCode } from "../protocol/types";
import { str, num, type Args } from "./args";
import { HELP } from "./help";

// Exit statuses scripts can branch on; every other failure exits 1. Listed in `shepherd help`.
const EXIT: Partial<Record<ErrorCode, number>> = { usage: 2, invalid_params: 2, unreachable: 3, timeout: 124 };

// Print a failure (as {"error":{code,message}} with --json) and return its exit status.
function failed(code: ErrorCode, message: string, json: boolean) {
  console.error(json ? JSON.stringify({ error: { code, message } }) : message);
  return EXIT[code] ?? 1;
}

function print(x: unknown, json: boolean) {
  if (json || typeof x !== "object" || x === null) return console.log(typeof x === "string" ? x : JSON.stringify(x, null, 2));
  console.log(JSON.stringify(x, null, 2));
}

function table(rows: Record<string, unknown>[], cols: string[]) {
  if (!rows.length) return console.log("(none)");
  const cells = rows.map((r) => cols.map((c) => String(r[c] ?? "")));
  const w = cols.map((c, i) => Math.max(c.length, ...cells.map((r) => r[i]!.length)));
  console.log(cols.map((c, i) => c.toUpperCase().padEnd(w[i]!)).join("  "));
  for (const r of cells) console.log(r.map((c, i) => c.padEnd(w[i]!)).join("  ").trimEnd());
}

export async function runCli(a: Args): Promise<number> {
  const [noun, verb, ...rest] = a._;
  const f = a.flags;
  const json = !!f.json;
  const caller = Bun.env.SHEPHERD_PANE_ID;
  if (noun === "report" && !verb && !caller) return 0; // an integration outside any shepherd pane
  let conn: Conn;
  try {
    conn = await connectExisting(str(f.session));
  } catch (e: any) {
    if (noun === "report") return 0; // hooks fire outside shepherd too; stay quiet
    return failed("unreachable", e.message, json);
  }
  const call = <T = any>(method: string, params: any = {}) => conn.request<T>(method, { caller, ...params });
  const target = (t?: string) => t ?? str(f.target);

  try {
    switch (`${noun} ${verb ?? ""}`.trim()) {
      case "pane list":
      case "list": {
        const ps = await call<any[]>("list");
        if (json) print(ps, true);
        else table(ps.map((p) => ({ id: p.id, name: p.name ? "@" + p.name : "", title: p.title, agent: p.agent ? `${p.agent.harness}:${p.agent.state}` : "", status: p.status === "exited" ? `exited ${p.exitCode}` : "running", workspace: p.workspace, cwd: p.cwd })), ["id", "name", "title", "agent", "status", "workspace", "cwd"]);
        break;
      }
      case "pane split": {
        const p = await call("pane.split", { target: target(), dir: f.down ? "down" : "right", name: str(f.name), cwd: str(f.cwd), command: rest.join(" ") || str(f.command), focus: !!f.focus });
        console.log(p.id);
        break;
      }
      case "pane run":
        await call("pane.run", { target: rest[0], command: rest.slice(1).join(" ") });
        break;
      case "pane read": {
        const snap = await call("pane.read", { target: target(rest[0]), lines: num(f.lines) ?? 50 });
        if (json) print(snap, true);
        else console.log(f.screen ? snap.screen : snap.recentOutput);
        break;
      }
      case "pane keys":
        await call("pane.keys", { target: rest[0], keys: rest.slice(1) });
        break;
      case "pane close":
        await call("pane.close", { target: target(rest[0]) });
        break;
      case "pane rename":
        await call("pane.rename", rest.length > 1 ? { target: rest[0], name: rest[1] } : { name: rest[0] });
        break;
      case "pane focus":
        await call("pane.focus", { target: rest[0] });
        break;
      case "agent spawn": {
        const p = await call("agent.spawn", { harness: rest[0], name: str(f.name), prompt: str(f.prompt), dir: f.down ? "down" : "right", tab: !!f.tab, target: target(), focus: !!f.focus });
        console.log(p.id);
        break;
      }
      case "agent list": {
        const as = await call<any[]>("agent.list");
        if (json) print(as, true);
        else table(as.map((x) => ({ id: x.id, name: x.name ? "@" + x.name : "", harness: x.harness, state: x.state, source: x.source, workspace: x.workspace })), ["id", "name", "harness", "state", "source", "workspace"]);
        break;
      }
      case `wait ${verb}`: {
        const res = await call("wait", { target: verb, exited: !!f.exited, state: f.idle ? "idle" : str(f.state), match: str(f.match), timeout: num(f.timeout) });
        if (json) print(res, true);
        else console.log("exitCode" in res ? `exited ${res.exitCode}` : res.state ?? res.match);
        if ("exitCode" in res) return res.exitCode ?? 0; // the child's status, in either output format
        break;
      }
      case `send ${verb}`: {
        const r = await call("send", { to: verb, body: rest.join(" ") });
        if (json) print(r, true);
        else console.log(`queued for ${verb}${r.recipientState ? ` (${r.recipientState})` : ""}: message ${r.id}, not delivered yet`);
        break;
      }
      case "inbox": {
        const ms = await call<any[]>("inbox");
        if (json) print(ms, true);
        else if (!ms.length) console.log("(no messages)");
        else for (const m of ms) console.log(`from @${m.from}${m.replyTo ? ` (reply: shepherd send ${m.replyTo} "...")` : ""}:\n${m.body}\n`);
        break;
      }
      case "messages": {
        const show = (m: any) => console.log(`${new Date(m.at).toLocaleTimeString()}  @${m.fromName} → @${m.toName}${m.hops ? ` (hop ${m.hops})` : ""}${m.delivered ? "" : "  [queued]"}\n  ${m.body.replace(/\n/g, "\n  ")}`);
        for (const m of await call<any[]>("messages")) show(m);
        if (!f.follow) break;
        conn.onMessage = (m) => m.method === "event" && m.params.type === "message.sent" && call<any[]>("messages").then((all) => show(all.at(-1)));
        await call("events.subscribe");
        await new Promise(() => {});
        break;
      }
      case "pause":
        console.log((await call("messaging.pause")).paused ? "messaging paused" : "messaging resumed");
        break;
      case "report":
      case `report ${verb}`:
        await call("report", { pane: verb, state: str(f.state), source: str(f.source), agent: str(f.agent), seq: num(f.seq), session: str(f["session-id"]), release: f.release === true || undefined });
        break;
      case "plugin list": {
        const ps = await call<any[]>("plugin.list");
        if (json) print(ps, true);
        else table(ps.map((p) => ({ name: p.name, status: p.exitCode !== undefined && p.status !== "running" ? `${p.status} ${p.exitCode}` : p.status, connected: p.connected ? "yes" : "no", actions: p.actions.join(","), error: p.error ?? "", log: p.log })), ["name", "status", "connected", "actions", "error", "log"]);
        break;
      }
      case "plugin logs": {
        const p = (await call<any[]>("plugin.list")).find((x) => x.name === rest[0]);
        if (!p) throw fail("no_such_plugin", `no plugin named ${rest[0]} (see shepherd plugin list)`);
        console.log((await Bun.file(p.log).text().catch(() => "")).trimEnd().split("\n").slice(-(num(f.lines) ?? 50)).join("\n"));
        break;
      }
      case "plugin run": {
        let params: unknown;
        try {
          params = rest[2] === undefined ? undefined : JSON.parse(rest[2]);
        } catch {
          throw fail("usage", `params must be JSON, like '{"key":"value"}': ${rest[2]}`);
        }
        print(await call("plugin.invoke", { plugin: rest[0], action: rest[1], params }), true);
        break;
      }
      case "workspace create":
        console.log((await call("workspace.create", { name: rest[0], cwd: str(f.cwd) })).id);
        break;
      case "workspace rename":
        await call("workspace.rename", { workspace: rest[0], name: rest.slice(1).join(" ") });
        break;
      case "workspace close":
        await call("workspace.close", { workspace: rest[0] });
        break;
      case "workspace list": {
        const ws = await call<any[]>("workspace.list");
        json ? print(ws, true) : table(ws.map((w) => ({ ...w, active: w.active ? "*" : "" })), ["id", "name", "tabs", "active", "cwd"]);
        break;
      }
      case "tab create":
        console.log((await call("tab.create", { name: rest[0], command: str(f.command), workspace: str(f.workspace) })).id);
        break;
      case "events": {
        conn.onMessage = (m) => m.method === "event" && console.log(JSON.stringify(m.params));
        await call("events.subscribe", { output: !!f.output });
        if (f.follow) await new Promise(() => {});
        await Bun.sleep(100);
        break;
      }
      case "debug detect":
        print(await call("debug.detect", { target: rest[0] }), true);
        break;
      case "detach":
        await call("detach-all");
        break;
      default:
        return failed("usage", `unknown command: ${a._.join(" ")}${json ? "" : `\n\n${HELP}`}`, json);
    }
  } catch (e: any) {
    return failed(e instanceof ConnectionClosedError ? "unreachable" : errorCode(e.code), `shepherd: ${e.message}`, json);
  } finally {
    if (!f.follow) conn.close();
  }
  return 0;
}
