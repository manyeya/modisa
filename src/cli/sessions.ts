// Session commands: attach (locally or over ssh), the ssh proxy, ls, restart, kill, config.
import { DIR, cwd, socketPath } from "../core/paths";
import { connectStdio, connectUnix, ensureServer } from "../protocol/transport";
import { loadConfig, ensureConfigFile, CONFIG_PATH } from "../config/config";

export async function attach(name: string, dir = cwd(), remote?: string) {
  const { runClient } = await import("../client/app");
  if (!remote) {
    return runClient({ session: name, connect: (spawn) => (spawn ? ensureServer(name, dir) : connectUnix(socketPath(name))) });
  }
  // ssh://user@host:port or an ~/.ssh/config alias; the far side runs `shepherd proxy`
  const u = /^ssh:\/\/([^/:]+)(?::(\d+))?/.exec(remote);
  const host = u ? u[1]! : remote;
  const port = u?.[2] ? ["-p", u[2]] : [];
  const cfg = await loadConfig();
  const ssh = Bun.env.SHEPHERD_SSH ?? "ssh";
  return runClient({ session: name, remote: true, connect: async () => connectStdio([ssh, "-T", ...port, host, cfg.remote_command, "proxy", "-s", name]) });
}

// Bridge stdio to the local socket byte-for-byte (used over ssh by --remote).
export async function proxy(name: string) {
  (await ensureServer(name)).close();
  const out = Bun.stdout.writer();
  const sock = await Bun.connect({
    unix: socketPath(name),
    socket: {
      data(_s, d) { out.write(d); out.flush(); },
      close() { out.end(); },
    },
  });
  for await (const chunk of Bun.stdin.stream()) sock.write(chunk);
  sock.end();
}

export async function listSessions() {
  const { saved } = await import("../server/persist/store");
  const names = new Set<string>();
  // Bun.Glob skips unix sockets, so list the directory with the shell
  const socks = (await Bun.$`ls -1 ${DIR}`.quiet().nothrow().text()).split("\n").filter((f) => f.endsWith(".sock"));
  const rows: string[] = [];
  for (const n of socks.map((f) => f.replace(/\.sock$/, "")).sort()) {
    try {
      const c = await connectUnix(socketPath(n));
      const i = await c.request("session.info");
      c.close();
      names.add(n);
      rows.push(`${n}\t${i.panes} panes, ${i.workspaces} workspaces${i.clients ? ` (${i.clients} attached)` : ""}`);
    } catch {
      await Bun.file(socketPath(n)).delete().catch(() => {}); // dead server
    }
  }
  for (const s of await saved()) if (!names.has(s.name)) rows.push(`${s.name}\tsaved ${new Date(s.saved_at).toLocaleString()} — attach to restore`);
  console.log(rows.length ? rows.join("\n") : "no sessions");
}

// Save the session, stop its server, start a fresh one on the current code (it restores everything).
export async function restartSession(name: string) {
  const c = await connectUnix(socketPath(name)).catch(() => undefined);
  if (!c) {
    console.log(`no running session "${name}"`);
    return;
  }
  const supported = await c.request("restart").then(() => true, () => false);
  c.close();
  if (!supported) {
    // servers from before `restart` existed: their state is already saved (within 1s of any change), so stop them
    await Bun.sleep(1200);
    // only the process listening on this session's socket (not same-named sessions elsewhere)
    for (const pid of (await Bun.$`lsof -t ${socketPath(name)}`.quiet().nothrow().text()).split("\n").filter(Boolean)) {
      if ((await Bun.$`ps -o args= -p ${pid}`.quiet().nothrow().text()).includes(`server -s ${name}`)) await Bun.$`kill ${pid}`.quiet().nothrow();
    }
  }
  for (let i = 0; i < 50 && (await connectUnix(socketPath(name)).then((x) => (x.close(), true), () => false)); i++) await Bun.sleep(100);
  (await ensureServer(name)).close();
  console.log(`restarted ${name}`);
}

export async function killSession(name: string) {
  try {
    const c = await connectUnix(socketPath(name));
    await c.request("kill");
    c.close();
  } catch {
    await (await import("../server/persist/store")).forget(name);
  }
  console.log(`killed ${name}`);
}

// `shepherd config [path|edit]`
export async function configCommand(sub?: string) {
  if (sub === "edit") await Bun.spawn([Bun.env.EDITOR || "vi", await ensureConfigFile()], { stdio: ["inherit", "inherit", "inherit"] }).exited;
  else console.log(sub === "path" ? CONFIG_PATH : await Bun.file(await ensureConfigFile()).text());
}
