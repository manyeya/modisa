// Agents acting on panes they didn't create: allowed, denied, or asked of whoever is attached.
import type { ServerContext } from "./context";
import type { PtyPane } from "./session/pane";

export function installPermissions(ctx: ServerContext) {
  const always = new Set<string>();
  let promptSeq = 0;
  ctx.permit = async (caller: string | undefined, action: "keys" | "close" | "run", target: PtyPane, detail = "") => {
    if (!caller || !ctx.s.panes.has(caller) || target.id === caller || target.info.createdBy === caller) return;
    const policy = ctx.cfg.permissions[`${action}_foreign`];
    if (policy === "allow" || always.has(`${caller}:${action}:${target.id}`)) return;
    // Typing at another agent is what the mailbox is for: say so, or every exchange interrupts the user.
    const instead = action !== "close" && (target.info.agent || target.info.harness) ? `; to talk to it use: modisa send @${ctx.name(target.id)} "…"` : "";
    if (policy === "deny") throw new Error(`permission denied: ${action} on ${ctx.name(target.id)}${instead}`);
    const to = ctx.attached();
    if (!to.length) throw new Error(`permission needed for ${action} on ${ctx.name(target.id)}, but no one is attached to approve it${instead}`);
    const id = ++promptSeq;
    const answer = await new Promise<string>((resolve) => {
      ctx.prompts.set(id, resolve);
      ctx.broadcast("prompt", { id, text: `@${ctx.name(caller)} wants to ${action} pane "${ctx.name(target.id)}"${detail ? `:\n\n  ${detail}` : ""}` }, to);
      setTimeout(() => resolve("deny"), 60_000);
    });
    ctx.prompts.delete(id);
    ctx.broadcast("prompt.done", { id });
    if (answer === "always") always.add(`${caller}:${action}:${target.id}`);
    else if (answer !== "allow") throw new Error(`denied by user: ${action} on ${ctx.name(target.id)}${instead}`);
  };
}
