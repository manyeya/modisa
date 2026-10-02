// The protocol as JSON Schema, generated from the schemas the server validates with: `protocol.describe` over the
// socket, `modisa plugin schema` without a server. `attach` is what a pane.attach connection is sent; `cli` is what
// `modisa plugin … --json` prints.
import { z } from "zod";
import { api, attachNotifications, cliResults, envelope, errorReply, events, PROTOCOL, results } from "./schema";

export function describeProtocol() {
  const json = (schemas: Record<string, z.ZodType>, io: "input" | "output") => Object.fromEntries(Object.entries(schemas).map(([k, schema]) => [k, z.toJSONSchema(schema, { io })]));
  return { protocol: PROTOCOL, envelope: z.toJSONSchema(envelope), requests: json(api, "input"), results: json(results, "output"), events: json(events, "output"), attach: json(attachNotifications, "output"), error: z.toJSONSchema(errorReply), cli: json(cliResults, "output") };
}
