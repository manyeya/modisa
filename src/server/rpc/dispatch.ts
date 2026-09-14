// Route a JSON-RPC request to its handler: validate params against the API schema, send back the
// result or a JSON-RPC error.
import { api, type Msg } from "../../protocol/schema";
import type { Client } from "../context";
import { errorCode } from "../../protocol/conn";

export type Handlers = Record<string, (params: any, client: Client) => any>;

export function createDispatcher(handlers: Handlers) {
  return async function dispatch(c: Client, m: Msg) {
    if (!m.method) return;
    const reply = (x: Partial<Msg>) => m.id !== undefined && c.conn.send({ jsonrpc: "2.0", id: m.id, ...x });
    // Only a plugin's bound connection (after plugin.hello) is attributed to the plugin, and it can't claim to be a
    // pane: `caller` is how panes (agents) are told apart, and permission prompts depend on it. Any unbound
    // connection, a second one from the same plugin included, is trusted as the local user, caller claims and all.
    // Nothing here is authentication.
    if (c.plugin && m.params?.caller !== undefined) return reply({ error: { code: -32602, message: `invalid params: caller: plugin ${c.plugin} acts as itself, not as a pane`, data: { code: "invalid_params" } } });
    const h = handlers[m.method];
    if (!h) return reply({ error: { code: -32601, message: `unknown method ${m.method}`, data: { code: "unknown_method" } } });
    const schema = api[m.method as keyof typeof api];
    const parsed = schema ? schema.safeParse(m.params ?? {}) : { success: true as const, data: m.params ?? {} };
    if (!parsed.success) return reply({ error: { code: -32602, message: `invalid params: ${parsed.error.issues.map((i) => `${i.path.join(".")} ${i.message}`).join("; ")}`, data: { code: "invalid_params" } } });
    try {
      reply({ result: (await h(parsed.data, c)) ?? null });
    } catch (e: any) {
      reply({ error: { code: -32000, message: e?.message ?? String(e), data: { code: errorCode(e?.code) } } });
    }
  };
}
