// Route a JSON-RPC request to its handler: validate params against the API schema, send back the
// result or a JSON-RPC error.
import { api, type Msg } from "../../protocol/schema";
import type { Client } from "../context";

export type Handlers = Record<string, (params: any, client: Client) => any>;

export function createDispatcher(handlers: Handlers) {
  return async function dispatch(c: Client, m: Msg) {
    if (!m.method) return;
    const reply = (x: Partial<Msg>) => m.id !== undefined && c.conn.send({ jsonrpc: "2.0", id: m.id, ...x });
    const h = handlers[m.method];
    if (!h) return reply({ error: { code: -32601, message: `unknown method ${m.method}`, data: { code: "unknown_method" } } });
    const schema = api[m.method as keyof typeof api];
    const parsed = schema ? schema.safeParse(m.params ?? {}) : { success: true as const, data: m.params ?? {} };
    if (!parsed.success) return reply({ error: { code: -32602, message: `invalid params: ${parsed.error.issues.map((i) => `${i.path.join(".")} ${i.message}`).join("; ")}`, data: { code: "invalid_params" } } });
    try {
      reply({ result: (await h(parsed.data, c)) ?? null });
    } catch (e: any) {
      reply({ error: { code: -32000, message: e?.message ?? String(e), data: { code: e?.code ?? "error" } } });
    }
  };
}
