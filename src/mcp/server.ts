// `shepherd mcp`: the socket API as MCP tools (stdio), for harnesses that prefer tools over shell commands.
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { VERSION } from "../core/version";
import type { Transport } from "@modelcontextprotocol/sdk/shared/transport.js";
import type { JSONRPCMessage } from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";
import { api } from "../protocol/schema";
import { connectExisting } from "../protocol/transport";

// MCP stdio framing (one JSON message per line) on Bun.stdin / Bun.stdout.
class BunStdio implements Transport {
  onmessage?: (m: JSONRPCMessage) => void;
  onclose?: () => void;
  onerror?: (e: Error) => void;
  private out = Bun.stdout.writer();

  async start() {
    (async () => {
      const dec = new TextDecoder();
      let buf = "";
      for await (const chunk of Bun.stdin.stream()) {
        buf += dec.decode(chunk, { stream: true });
        let nl;
        while ((nl = buf.indexOf("\n")) >= 0) {
          const line = buf.slice(0, nl).trim();
          buf = buf.slice(nl + 1);
          if (!line) continue;
          try {
            this.onmessage?.(JSON.parse(line));
          } catch (e) {
            this.onerror?.(e as Error);
          }
        }
      }
      this.onclose?.();
    })();
  }

  async send(m: JSONRPCMessage) {
    this.out.write(JSON.stringify(m) + "\n");
    await this.out.flush();
  }

  async close() {
    await this.out.end();
    this.onclose?.();
  }
}

const tools: [name: string, method: keyof typeof api, description: string][] = [
  ["list_panes", "list", "List every pane: id, @name, title, agent harness + state (working/blocked/done/idle), cwd, exit status."],
  ["list_agents", "agent.list", "List agent panes and their current state."],
  ["split_pane", "pane.split", "Open a new pane next to one (default: your own). Give `command` to run a process (the pane stays after it exits so you can read output), or omit it for a shell. Returns the new pane."],
  ["run_in_pane", "pane.run", "Type a command into a pane and press Enter."],
  ["read_pane", "pane.read", "Read a pane: its visible screen plus the last `lines` of scrollback (plain text)."],
  ["send_keys", "pane.keys", "Send keys to a pane. Keys are text or names: Enter, Escape, Tab, C-c, M-x, Up, Down…"],
  ["close_pane", "pane.close", "Close a pane and kill what runs in it."],
  ["spawn_agent", "agent.spawn", "Start another coding agent (claude-code, codex, pi, opencode, gemini, …) in a new pane, optionally with a first prompt."],
  ["wait", "wait", "Block until a pane's process exits, its agent reaches a state (idle also matches done), or its output matches a regex."],
  ["send_message", "send", "Send a message to another agent pane. It's typed into that agent when it's idle, tagged with your name so it can reply."],
  ["read_inbox", "inbox", "Messages other agents sent you that you haven't seen."],
  ["create_tab", "tab.create", "Open a new tab, optionally running a command."],
  ["create_workspace", "workspace.create", "Open a new workspace rooted at a directory."],
];

export async function runMcp(session?: string) {
  const conn = await connectExisting(session);
  const caller = Bun.env.SHEPHERD_PANE_ID;
  const server = new McpServer({ name: "shepherd", version: VERSION });
  for (const [name, method, description] of tools) {
    server.registerTool(name, { description, inputSchema: (api[method] as z.ZodObject<any>).omit({ caller: true }) }, async (args: any) => {
      try {
        const result = await conn.request(method, { ...args, caller });
        return { content: [{ type: "text", text: typeof result === "string" ? result : JSON.stringify(result, null, 2) }] };
      } catch (e: any) {
        return { content: [{ type: "text", text: e.message }], isError: true };
      }
    });
  }
  await server.connect(new BunStdio());
}
