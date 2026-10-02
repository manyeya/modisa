// The public API (CLI, plugins, integrations). Params are validated against protocol/schema before they get
// here. `caller` is the pane id of the agent calling, if any.
import { codeVersion, cwd } from "../../core/paths";
import { cleanText } from "../../core/text";
import { SIDE, type Dir } from "../../core/layout";
import type { AgentState } from "../../protocol/types";
import type { ServerContext } from "../context";
import type { MoveTo } from "../session/session";
import { cwds } from "../persist/store";
import { foreground, processTable } from "../agents/detect";
import type { PtyPane } from "../session/pane";
import { integrationStatus, setIntegration } from "../../integrations";
import { keyBytes } from "../keys";
import type { Handlers } from "./dispatch";
import { fail } from "../../protocol/conn";
import { PROTOCOL } from "../../protocol/schema";
import { describeProtocol } from "../../protocol/describe";

export function apiMethods(ctx: ServerContext): Handlers {
  const { s, mail, detector } = ctx;
  const list = () =>
    [...s.panes.values()].map((p) => {
      const at = s.locate(p.id);
      return { ...p.info, focused: s.focusedId === p.id, workspace: at?.ws.name, workspaceId: at?.ws.id, tabId: at?.tab.id };
    });
  // what a layout command acts on: a pane with a place in a tab (not a popup)
  const placed = (target: string | undefined, caller?: string) => {
    const pane = ctx.subject(target, caller);
    s.placeOf(pane.id);
    return pane;
  };
  // and to move it, not a plugin's overlay either
  const movable = (id: string) => {
    if (!ctx.movable(id)) throw new Error(`${id} is a plugin's overlay: it stays over the pane it opened on`);
    return id;
  };
  const noNeighbor = (id: string, dir: Dir) => fail("no_such_pane", `no pane ${SIDE[dir]} ${id}`);
  // a pane just made, and where it is
  const created = (pane: PtyPane) => {
    const { ws, tab } = s.placeOf(pane.id);
    return { ...pane.info, workspaceId: ws.id, tabId: tab.id };
  };
  // Its process, the job in the foreground of its terminal, and where the shell is now (it may have cd'd since it
  // started); an exited pane has only the pid it had.
  const processOf = async (pane: PtyPane) => {
    const pid = pane.proc.pid;
    if (pane.info.status !== "running") return { pid };
    const [procs, dirs] = await Promise.all([processTable(), cwds([pid])]);
    const fg = foreground(procs, pid);
    return { pid, ...(fg && { foreground: { pid: fg.pid, args: fg.args } }), ...(dirs.has(pid) && { cwd: dirs.get(pid) }) };
  };
  return {
    // ---------- session & workspaces ----------
    list,
    "session.info": async (p) => {
      const info = { session: ctx.session, clients: ctx.attached().length, paused: mail.paused, version: await codeVersion() };
      if (!p.snapshot) return { ...info, panes: s.panes.size, workspaces: s.workspaces.length };
      // what clients draw, read without attaching: no area is set, no event is sent, nothing changes. A copy, so a
      // change before the reply is written can't reach it
      return { ...info, ...structuredClone({ active: s.active, area: s.area, workspaces: s.view().workspaces, panes: list() }) };
    },
    "workspace.list": () => s.workspaces.map((w, i) => ({ id: w.id, name: w.name, cwd: w.cwd, tabs: w.tabs.length, active: i === s.active })),
    "workspace.rename": (p) => (s.renameWorkspace(p.name, s.findWorkspace(p.workspace)), true),
    "workspace.close": (p) => (s.closeWorkspace(s.findWorkspace(p.workspace)), true),
    "workspace.create": (p) => created(s.newWorkspace(p.name, p.cwd ?? cwd(), { command: p.command, createdBy: p.caller, env: p.env })),
    "tab.create": (p) => {
      if (p.workspace) {
        const i = s.workspaces.findIndex((w) => w.name === p.workspace || w.id === p.workspace);
        if (i < 0) throw new Error(`no such workspace: ${p.workspace}`);
        s.selectWorkspace(i);
      }
      return created(s.newTab(p.name, { command: p.command, name: p.paneName, cwd: p.cwd, createdBy: p.caller, env: p.env }));
    },

    // ---------- panes ----------
    "pane.split": (p) => {
      const pane = s.split(p.dir === "down" ? "col" : "row", { command: p.command, name: p.name, cwd: p.cwd, createdBy: p.caller, env: p.env }, ctx.subject(p.target, p.caller).id, p.focus ?? false, p.ratio);
      if (!pane) throw new Error("nothing to split");
      return created(pane);
    },
    "pane.run": async (p) => {
      const pane = ctx.need(p.target);
      await ctx.permit(p.caller, "run", pane, p.command);
      pane.write(p.command + "\r");
      return true;
    },
    "pane.read": (p) => ctx.snapshot(ctx.need(p.target, p.caller), p.lines, p.source, p.format),
    "pane.keys": async (p) => {
      const pane = ctx.need(p.target);
      await ctx.permit(p.caller, "keys", pane, p.keys.join(" "));
      pane.write(p.keys.map(keyBytes).join(""));
      return true;
    },
    "pane.close": async (p) => {
      const pane = ctx.need(p.target, p.caller);
      await ctx.permit(p.caller, "close", pane);
      s.close(pane.id);
      return true;
    },
    "pane.rename": (p) => (s.renamePane(ctx.need(p.target, p.caller).id, p.name), true),
    "pane.focus": (p) => {
      const pane = ctx.subject(p.target, p.caller);
      if (!p.dir) s.focusPane(pane.id);
      else if (!s.focusDir(p.dir, pane.id)) throw noNeighbor(pane.id, p.dir);
      return true;
    },
    // no permission asked: like focus, these rearrange panes and touch nothing running in them
    "pane.move": async (p) => {
      const pane = placed(p.target, p.caller);
      movable(pane.id);
      // a new space starts where the pane is now: its shell may have cd'd since it started
      const here = p.newWorkspace && pane.info.status === "running" ? (await cwds([pane.proc.pid])).get(pane.proc.pid) : undefined;
      if (!s.panes.has(pane.id)) throw fail("pane_gone", `${pane.id} closed before it could be moved`);
      let to: MoveTo;
      if (p.newWorkspace) to = { newSpace: { name: p.name, cwd: here ?? pane.info.cwd } };
      else if (p.newTab) to = { newTab: p.workspace ? s.workspaces[s.findWorkspace(p.workspace)]! : s.placeOf(pane.id).ws, name: p.name };
      else {
        const tab = p.tab ? s.findTab(p.tab).tab : undefined;
        const beside = p.beside ? ctx.need(p.beside).id : tab!.focused;
        if (tab && s.locate(beside)?.tab !== tab) throw new Error(`${beside} isn't in tab ${p.tab}`);
        to = { beside, dir: p.dir === "down" ? "col" : "row", share: p.ratio };
      }
      const { ws, tab } = s.move(pane.id, to, p.focus);
      return { pane: pane.id, instance: pane.info.instance, workspaceId: ws.id, tabId: tab.id };
    },
    "pane.swap": (p) => {
      const a = placed(p.target, p.caller);
      const b = p.with ? ctx.need(p.with).id : s.neighborOf(a.id, p.dir);
      if (!b) throw noNeighbor(a.id, p.dir);
      s.swap(movable(a.id), movable(b));
      return true;
    },
    "pane.resize": (p) => ({ changed: s.resizePane(p.dir, p.amount, placed(p.target, p.caller).id) }),
    "pane.zoom": (p) => ({ zoomed: s.zoom(placed(p.target, p.caller).id, p.mode) }),
    wait: async (p) => {
      const pane = ctx.need(p.target);
      const re = p.match ? new RegExp(p.match, "m") : undefined;
      const want = p.state as AgentState | undefined;
      const deadline = p.timeout ? Date.now() + p.timeout * 1000 : Infinity;
      for (;;) {
        // exited-then-closed (a shell pane closes itself on exit) still counts as exited; an exit caused by the close
        // (its SIGHUP) doesn't, so that fails like any other close
        if (p.exited && pane.info.status === "exited" && !pane.closedWhileRunning) return { exitCode: pane.info.exitCode };
        if (!s.panes.has(pane.id)) throw fail("pane_gone", `${pane.id} closed before the wait was met`);
        const st = pane.info.agent?.state;
        if (want && (st === want || (want === "idle" && st === "done"))) return { state: st };
        if (re) {
          const m = re.exec(pane.text().split("\n").slice(-500).join("\n"));
          if (m) return { match: m[0] };
        }
        if (Date.now() > deadline) throw fail("timeout", "timeout");
        await Bun.sleep(200);
      }
    },

    // ---------- agents ----------
    "agent.spawn": (p) => {
      const o = { ...ctx.agentOpts(p.harness, p.prompt, p.name, p.caller), env: p.env };
      const pane = p.tab ? s.newTab(p.name, o) : s.split(p.dir === "down" ? "col" : "row", o, ctx.subject(p.target, p.caller).id, p.focus ?? false);
      if (!pane) throw new Error("nothing to split");
      return created(pane);
    },
    "agent.list": () => [...s.panes.values()].filter((p) => p.info.agent).map((p) => ({ id: p.id, name: p.info.name, title: p.info.title, ...p.info.agent, workspace: s.locate(p.id)?.ws.name })),
    report: (p) => {
      const pane = ctx.need(p.pane ?? p.caller);
      const source = p.source ?? "custom";
      if (p.release) detector.release(pane, source);
      if (p.session) {
        const agent = p.agent ?? pane.info.agent?.harness ?? pane.info.harness;
        if (agent) pane.info.session = { agent, id: p.session, source };
        ctx.changed(); // saved, so a restart resumes this exact session
      }
      if (p.title !== undefined) {
        pane.reportedTitle = cleanText(p.title, 200);
        if (pane.refreshTitle()) ctx.changed();
      }
      // state needs a named source: hooks from older modisa versions sent none and are ignored
      if (p.state && p.source) detector.report(pane, { source: p.source, agent: p.agent, state: p.state, seq: p.seq });
      ctx.tick();
      return true;
    },
    "debug.detect": async (p) => {
      const pane = ctx.subject(p.target, p.caller);
      // read before waiting on the process table: the pane can close meanwhile, and its screen with it
      const seen = { pane: pane.id, agent: pane.info.agent, session: pane.info.session, detection: detector.last.get(pane.id), authority: detector.authority.get(pane.id), title: pane.oscTitle, progress: pane.oscProgress, screen: pane.screen() };
      return { ...seen, process: await processOf(pane) };
    },

    // A toast, titled with the pane that sent it (its @name, else its id), else "notify"; a plugin's connection is the
    // plugin, within its budget. How many clients it reached: none attached is 0, not a failure.
    notify: (p, c) => {
      const pane = p.caller ? s.panes.get(p.caller) : undefined;
      const from = c.plugin ?? (pane ? (pane.info.name ? `@${pane.info.name}` : pane.id) : "notify");
      const source = c.plugin ? `plugin:${c.plugin}` : pane ? `pane:${pane.id}` : "user";
      const text = (p.body ? `${p.title}: ${p.body}` : p.title).replace(/\s*\n\s*/g, " "); // one line
      return { clients: ctx.toast({ from, source, plugin: !!c.plugin, text, tone: p.tone, system: p.system, sound: p.sound }) };
    },

    // ---------- messaging ----------
    send: (p) => {
      const from = p.caller && s.panes.has(p.caller) ? p.caller : "user";
      const to = ctx.need(p.to);
      if (!to.info.agent && !to.info.harness) throw new Error(`${ctx.name(to.id)} is not an agent pane`);
      const replyTo = from === "user" ? undefined : `${from}:${s.panes.get(from)!.info.instance}`;
      const m = mail.send(from, ctx.name(from), to.id, ctx.name(to.id), p.body, replyTo);
      ctx.emit("message.sent", { id: m.id, from: m.fromName, to: m.toName, hops: m.hops });
      ctx.changed();
      // queued, not delivered: it's typed in when the recipient is idle (see `messages`)
      return { id: m.id, queued: true, delivered: false, recipientState: to.info.agent?.state };
    },
    inbox: (p) => mail.take(ctx.need(undefined, p.caller).id).map((m) => ({ id: m.id, from: m.fromName, replyTo: m.replyTo, body: m.body, at: m.at })),
    messages: () => mail.log,
    "messaging.pause": (p) => {
      mail.paused = p.paused ?? !mail.paused;
      ctx.changed();
      return { paused: mail.paused };
    },

    // ---------- events & lifecycle ----------
    // The snapshot is taken in the same synchronous step that turns events on, so every change after it is an event
    // with a higher seq and nothing is in both. An event can reach the client before this reply does: order by seq.
    "events.subscribe": (p, c) => {
      c.events = true;
      c.output = !!p.output;
      // a deep copy, so nothing that changes after this step (an agent's state is updated in place) can reach the reply
      return { protocol: PROTOCOL, epoch: ctx.epoch, seq: ctx.seq, ...(p.snapshot && { panes: structuredClone(list()) }) };
    },
    "protocol.describe": () => describeProtocol(),
    // Save, stop, and let the caller start a fresh server on the current code; it restores the session.
    // ---------- integrations (on this machine, where the agents run) ----------
    integrations: () => integrationStatus(),
    integration: (p) => {
      if (p.caller) throw new Error("only the user can change integrations"); // not agents in panes
      return setIntegration(p.id, p.install);
    },
    restart: () => {
      setTimeout(() => ctx.shutdown(false, "restart"), 10);
      return true;
    },
    kill: () => {
      setTimeout(() => ctx.shutdown(true), 10);
      return true;
    },
  };
}
