// `modisa <noun> <verb>` — the socket API as shell commands, so any agent can drive panes with zero integration.
import { connectExisting } from "../protocol/transport";
import { ConnectionClosedError, errorCode, fail, type Conn } from "../protocol/conn";
import { findPane, type ErrorCode } from "../protocol/types";
import { absPath } from "../core/paths";
import { SIDE, displayRects, neighbor, panes, rects, type Dir } from "../core/layout";
import { str, num, type Args } from "./args";
import { HELP } from "./help";

// Exit statuses scripts can branch on; every other failure exits 1. Listed in `modisa help`.
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

const DIRS: Dir[] = ["left", "right", "up", "down"];

// Where a pane is, from a session.info snapshot: the server would take the same pane (named, else the calling pane,
// else the focused one) and refuse the same way.
function placeIn(snap: any, target: string | undefined, caller: string | undefined) {
  const ws = snap.workspaces[snap.active];
  const t = target ?? (caller && snap.panes.some((p: any) => p.id === caller) ? caller : ws?.tabs[ws.active]?.focused);
  const pane = t ? findPane<any>(snap.panes, t) : undefined;
  if (!pane) throw /^p\d+:\w+$/.test(t ?? "") ? fail("pane_gone", `${t!.split(":")[0]} has gone: that pane was closed or restarted since its message was sent`) : fail("no_such_pane", `no such pane: ${t ?? "(none)"}`);
  for (const w of snap.workspaces) for (const tab of w.tabs) if (panes(tab.tree).includes(pane.id)) return { pane, ws: w, tab, rs: rects(tab.tree, snap.area) };
  throw new Error(`${pane.id} is a popup: it has no place in a tab`);
}

export async function runCli(a: Args): Promise<number> {
  const [noun, verb, ...rest] = a._;
  const f = a.flags;
  const json = !!f.json;
  const caller = Bun.env.MODISA_PANE_ID;
  if (noun === "report" && !verb && !caller) return 0; // an integration outside any modisa pane
  let conn: Conn;
  try {
    conn = await connectExisting(str(f.session));
  } catch (e: any) {
    if (noun === "report") return 0; // hooks fire outside modisa too; stay quiet
    return failed("unreachable", e.message, json);
  }
  const call = <T = any>(method: string, params: any = {}) => conn.request<T>(method, { caller, ...params });
  const target = (t?: string) => t ?? str(f.target);
  const cwdOpt = () => (typeof f.cwd === "string" ? absPath(f.cwd) : undefined); // --cwd: ~ and relative paths are from here
  // --env NAME=value, once per variable (the server checks the names)
  const env = () =>
    a.lists.env &&
    Object.fromEntries(
      a.lists.env.map((kv) => {
        const eq = kv.indexOf("=");
        if (eq < 1) throw fail("usage", `--env takes NAME=value, not "${kv}"`);
        return [kv.slice(0, eq), kv.slice(eq + 1)];
      }),
    );
  // what session.info { snapshot } describes: a server from before it sends counts
  const snapshot = async () => {
    const snap = await call("session.info", { snapshot: true });
    if (!Array.isArray(snap.workspaces)) throw fail("error", "server too old: modisa restart");
    return snap;
  };

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
        const p = await call("pane.split", { target: target(), dir: f.down ? "down" : "right", ratio: num(f.ratio), name: str(f.name), cwd: cwdOpt(), command: rest.join(" ") || str(f.command), focus: !!f.focus, env: env() });
        json ? print(p, true) : console.log(p.id);
        break;
      }
      case "pane run":
        await call("pane.run", { target: rest[0], command: rest.slice(1).join(" ") });
        break;
      case "pane read": {
        if (f.screen && f.source !== undefined && f.source !== "visible") throw fail("usage", "--screen is --source visible: use one of them");
        const source = f.screen ? "visible" : str(f.source);
        const snap = await call("pane.read", { target: target(rest[0]), lines: num(f.lines) ?? 50, source, format: str(f.format) });
        if (json) print(snap, true);
        else if (snap.content !== undefined) console.log(snap.content);
        else {
          // a server from before source and format: it sends the plain screen and recent lines, and nothing else
          const old = f.format !== undefined && f.format !== "text" ? undefined : source === "visible" ? snap.screen : source === undefined || source === "recent" ? snap.recentOutput : undefined;
          if (old === undefined) throw fail("error", "server too old: modisa restart");
          console.log(old);
        }
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
        await call("pane.focus", { target: rest[0], dir: str(f.direction) });
        break;
      case "pane move": {
        // --target is where it goes: the pane moved is the positional one
        const r = await call("pane.move", { target: rest[0], tab: str(f.tab), beside: str(f.target), newTab: f["new-tab"] === true || undefined, workspace: str(f.workspace), newWorkspace: f["new-workspace"] === true || undefined, name: str(f.name), dir: str(f.split), ratio: num(f.ratio), focus: !!f.focus });
        if (json) print(r, true);
        break;
      }
      case "pane swap": {
        // pane swap [a] (<b> | --direction d): one positional without a direction is b
        const dir = str(f.direction);
        const [a, b] = dir || rest.length > 1 ? rest : [undefined, rest[0]];
        await call("pane.swap", { target: a, with: b, dir });
        break;
      }
      case "pane resize": {
        const r = await call("pane.resize", { target: rest[0], dir: str(f.direction), amount: num(f.amount) });
        json ? print(r, true) : console.log(r.changed ? "changed" : "unchanged");
        break;
      }
      // worked out here from a snapshot, with the layout math the server and the TUI use
      case "pane layout": {
        const snap = await snapshot();
        const { pane, ws, tab, rs } = placeIn(snap, rest[0], caller);
        const shown = displayRects(tab.tree, snap.area, tab.focused, tab.zoomed);
        const name = (id: string) => snap.panes.find((p: any) => p.id === id)?.name;
        const r = { pane: pane.id, workspaceId: ws.id, tabId: tab.id, area: snap.area, focused: tab.focused, zoomed: tab.zoomed, panes: [...rs].map(([id, box]) => ({ id, name: name(id), ...box, shown: shown.has(id) })) };
        if (json) print(r, true);
        else table(r.panes.map((p) => ({ ...p, name: p.name ? "@" + p.name : "", shown: p.shown ? "yes" : "no", focused: p.id === tab.focused ? "*" : "" })), ["id", "name", "x", "y", "w", "h", "shown", "focused"]);
        break;
      }
      case "pane neighbor": {
        const d = str(f.direction) as Dir | undefined;
        if (!d || !DIRS.includes(d)) throw fail("usage", "pane neighbor needs --direction left|right|up|down");
        const snap = await snapshot();
        const { pane, rs } = placeIn(snap, rest[0], caller);
        const n = neighbor(rs, pane.id, d);
        if (!n) throw fail("no_such_pane", `no pane ${SIDE[d]} ${pane.id}`);
        json ? print(snap.panes.find((p: any) => p.id === n), true) : console.log(n);
        break;
      }
      case "pane edges": {
        const snap = await snapshot();
        const { pane, rs } = placeIn(snap, rest[0], caller);
        const sides = Object.fromEntries(DIRS.map((d) => [d, neighbor(rs, pane.id, d) ?? null])) as Record<Dir, string | null>; // null: the tab's edge
        json ? print({ pane: pane.id, ...sides }, true) : table(DIRS.map((d) => ({ side: d, pane: sides[d] ?? "(edge)" })), ["side", "pane"]);
        break;
      }
      case "pane process-info": {
        const d = await call("debug.detect", { target: rest[0] });
        if (!d.process) throw fail("error", "server too old: modisa restart");
        const r = { pane: d.pane, ...d.process };
        if (json) print(r, true);
        else console.log([`pid ${r.pid}`, r.foreground && `foreground ${r.foreground.pid} ${r.foreground.args}`, r.cwd && `cwd ${r.cwd}`].filter(Boolean).join("\n"));
        break;
      }
      case "pane zoom": {
        const modes = (["on", "off", "toggle"] as const).filter((m) => f[m]);
        if (modes.length > 1) throw fail("usage", "use one of --on, --off and --toggle");
        const r = await call("pane.zoom", { target: rest[0], mode: modes[0] });
        json ? print(r, true) : console.log(r.zoomed ? "zoomed" : "unzoomed");
        break;
      }
      case "agent spawn": {
        const p = await call("agent.spawn", { harness: rest[0], name: str(f.name), prompt: str(f.prompt), dir: f.down ? "down" : "right", tab: !!f.tab, target: target(), focus: !!f.focus, env: env() });
        json ? print(p, true) : console.log(p.id);
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
        else for (const m of ms) console.log(`from @${m.from}${m.replyTo ? ` (reply: modisa send ${m.replyTo} "...")` : ""}:\n${m.body}\n`);
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
        else {
          table(ps.map((p) => ({ name: p.name, status: p.exitCode !== undefined && p.status !== "running" ? `${p.status} ${p.exitCode}` : p.status, connected: p.connected ? "yes" : "no", actions: p.actions.join(","), source: p.install ? `${p.install.source}${p.install.ref ? ` ${p.install.ref}` : ""} @${p.install.commit.slice(0, 7)}` : p.dir ?? "", error: p.error ?? "", log: p.log })), ["name", "status", "connected", "actions", "source", "error", "log"]);
          for (const p of ps) for (const k of p.keys ?? []) if (k.state === "disabled") console.log(`${p.name}: key ${k.key || "(none)"} (${k.action ?? k.pane}) is off in the server's config: ${k.reason}`);
        }
        break;
      }
      case "plugin stop":
      case "plugin start": {
        const p = await call(`plugin.${verb}`, { name: rest[0] });
        json ? print(p, true) : console.log(`${p.name}: ${p.status}${p.error ? ` (${p.error})` : ""}`);
        break;
      }
      case "plugin ui":
        print(await call("ui.state", { plugin: rest[0] }), true);
        break;
      case "plugin pane": {
        let params: unknown;
        try {
          params = rest[2] === undefined ? undefined : JSON.parse(rest[2]);
        } catch {
          throw fail("usage", `params must be JSON, like '{"key":"value"}': ${rest[2]}`);
        }
        const opened = await call("plugin.pane.open", { plugin: rest[0], pane: rest[1], params });
        json ? print(opened, true) : console.log(`${opened.pane} (${opened.placement})`);
        break;
      }
      case "plugin logs": {
        const p = (await call<any[]>("plugin.list")).find((x) => x.name === rest[0]);
        if (!p) throw fail("no_such_plugin", `no plugin named ${rest[0]} (see modisa plugin list)`);
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
      case "workspace create": {
        const p = await call("workspace.create", { name: rest[0], cwd: cwdOpt(), command: str(f.command), env: env() });
        json ? print(p, true) : console.log(p.id);
        break;
      }
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
      case "tab create": {
        const p = await call("tab.create", { name: rest[0], command: str(f.command), workspace: str(f.workspace), cwd: cwdOpt(), paneName: str(f["pane-name"]), env: env() });
        json ? print(p, true) : console.log(p.id);
        break;
      }
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
    return failed(e instanceof ConnectionClosedError ? "unreachable" : errorCode(e.code), `modisa: ${e.message}`, json);
  } finally {
    if (!f.follow) conn.close();
  }
  return 0;
}
