// The public API (CLI, plugins, integrations). Params are validated against protocol/schema before they get
// here. `caller` is the pane id of the agent calling, if any.
import { codeVersion, cwd } from "../../core/paths";
import type { AgentState } from "../../protocol/types";
import type { ServerContext } from "../context";
import { integrationStatus, setIntegration } from "../../integrations";
import { keyBytes } from "../keys";
import type { Handlers } from "./dispatch";
import { fail } from "../../protocol/conn";
import { PROTOCOL } from "../../protocol/schema";
import { describeProtocol } from "../../protocol/describe";

export function apiMethods(ctx: ServerContext): Handlers {
  const { s, mail, detector } = ctx;
  const list = () => [...s.panes.values()].map((p) => ({ ...p.info, focused: s.focusedId === p.id, workspace: s.locate(p.id)?.ws.name }));
  return {
    // ---------- session & workspaces ----------
    list,
    "session.info": async () => ({ session: ctx.session, clients: ctx.attached().length, panes: s.panes.size, workspaces: s.workspaces.length, paused: mail.paused, version: await codeVersion() }),
    "workspace.list": () => s.workspaces.map((w, i) => ({ id: w.id, name: w.name, cwd: w.cwd, tabs: w.tabs.length, active: i === s.active })),
    "workspace.rename": (p) => (s.renameWorkspace(p.name, s.findWorkspace(p.workspace)), true),
    "workspace.close": (p) => (s.closeWorkspace(s.findWorkspace(p.workspace)), true),
    "workspace.create": (p) => s.newWorkspace(p.name, p.cwd ?? cwd(), { command: p.command, createdBy: p.caller }).info,
    "tab.create": (p) => {
      if (p.workspace) {
        const i = s.workspaces.findIndex((w) => w.name === p.workspace || w.id === p.workspace);
        if (i < 0) throw new Error(`no such workspace: ${p.workspace}`);
        s.selectWorkspace(i);
      }
      return s.newTab(p.name, { command: p.command, name: p.paneName, cwd: p.cwd, createdBy: p.caller }).info;
    },

    // ---------- panes ----------
    "pane.split": (p) => {
      const target = p.target ? ctx.need(p.target) : p.caller && s.panes.has(p.caller) ? s.panes.get(p.caller)! : undefined;
      const pane = s.split(p.dir === "down" ? "col" : "row", { command: p.command, name: p.name, cwd: p.cwd, createdBy: p.caller }, target?.id ?? s.focusedId, p.focus ?? false);
      if (!pane) throw new Error("nothing to split");
      return pane.info;
    },
    "pane.run": async (p) => {
      const pane = ctx.need(p.target);
      await ctx.permit(p.caller, "run", pane, p.command);
      pane.write(p.command + "\r");
      return true;
    },
    "pane.read": (p) => ctx.snapshot(ctx.need(p.target, p.caller), p.lines ?? 50),
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
    "pane.focus": (p) => (s.focusPane(ctx.need(p.target).id), true),
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
      const o = ctx.agentOpts(p.harness, p.prompt, p.name, p.caller);
      const pane = p.tab ? s.newTab(p.name, o) : s.split(p.dir === "down" ? "col" : "row", o, p.target ? ctx.need(p.target).id : p.caller && s.panes.has(p.caller) ? p.caller : s.focusedId, p.focus ?? false);
      if (!pane) throw new Error("nothing to split");
      return pane.info;
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
      // state needs a named source: hooks from older shepherd versions sent none and are ignored
      if (p.state && p.source) detector.report(pane, { source: p.source, agent: p.agent, state: p.state, seq: p.seq });
      ctx.tick();
      return true;
    },
    "debug.detect": (p) => {
      const pane = ctx.need(p.target);
      return { pane: pane.id, agent: pane.info.agent, session: pane.info.session, detection: detector.last.get(pane.id), authority: detector.authority.get(pane.id), title: pane.oscTitle, progress: pane.oscProgress, screen: pane.screen() };
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
