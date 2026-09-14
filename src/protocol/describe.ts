// The protocol as JSON Schema, generated from the schemas the server validates with: `protocol.describe` over the
// socket, `shepherd plugin schema` without a server.
import { z } from "zod";
import { api, envelope, errorReply, events, PROTOCOL, results } from "./schema";

export function describeProtocol() {
  const json = (schemas: Record<string, z.ZodType>, io: "input" | "output") => Object.fromEntries(Object.entries(schemas).map(([k, schema]) => [k, z.toJSONSchema(schema, { io })]));
  return { protocol: PROTOCOL, envelope: z.toJSONSchema(envelope), requests: json(api, "input"), results: json(results, "output"), events: json(events, "output"), error: z.toJSONSchema(errorReply) };
}
