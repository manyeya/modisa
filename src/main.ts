#!/usr/bin/env bun
// Entry point: route a command line to the session commands, the server, the integrations, or the API CLI.
import { execWithTty } from "./platform/ctty";
import { parseArgs } from "./cli/args";
import { HELP } from "./cli/help";
import { runCli } from "./cli/commands";
import { attach, proxy, listSessions, restartSession, killSession, configCommand } from "./cli/sessions";
import { cwd } from "./core/paths";

// Panes start through here so their shell owns the PTY (see platform/ctty.ts). Before arg parsing: argv is the shell's.
if (Bun.argv[2] === "__pty-exec") execWithTty(Bun.argv.slice(3));

const a = parseArgs(Bun.argv.slice(2));
const [cmd, ...rest] = a.flags.help ? ["help"] : a.flags.version ? ["version"] : a._;
const session = typeof a.flags.session === "string" ? a.flags.session : "default";
const flag = (k: string) => (typeof a.flags[k] === "string" ? (a.flags[k] as string) : undefined);

switch (cmd) {
  case undefined:
  case "attach":
  case "a":
    await attach(rest[0] ?? session, cwd(), flag("remote"));
    break;
  case "new":
    await attach(rest[0] ?? session, flag("cwd") ?? cwd(), flag("remote"));
    break;
  case "server":
    await (await import("./server/server")).runServer(session);
    break;
  case "proxy":
    await proxy(session);
    break;
  case "ls":
  case "list-sessions":
    await listSessions();
    break;
  case "restart":
    await restartSession(rest[0] ?? session);
    break;
  case "kill":
    await killSession(rest[0] ?? session);
    break;
  case "integration":
    await (await import("./integrations")).runIntegration(rest[0], rest[1]);
    break;
  case "update":
    await (await import("./cli/update")).runUpdate();
    break;
  case "version":
    await (await import("./cli/update")).runVersion();
    break;
  case "uninstall":
    process.exitCode = await (await import("./cli/uninstall")).runUninstall({ purge: a.flags.purge === true, yes: a.flags.yes === true });
    break;
  case "hook": // run by agents' hooks: shepherd hook <agent> <action>
    await (await import("./integrations/hook")).runHook(rest[0], rest[1]);
    break;
  case "config":
    await configCommand(rest[0]);
    break;
  case "help":
  case "--help":
  case "-h":
    console.log(HELP);
    break;
  default:
    await runCli(a);
}
