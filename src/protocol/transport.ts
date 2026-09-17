// Getting a connection: to a local session server (starting it if needed), or over ssh.
import { DIR, cwd, self, socketPath } from "../core/paths";
import { Conn, socketConn } from "./conn";

export async function connectUnix(path: string): Promise<Conn> {
  let conn!: Conn;
  let flush = () => {};
  const dec = new TextDecoder();
  await Bun.connect({
    unix: path,
    socket: {
      open(s) { ({ conn, flush } = socketConn(s)); },
      data(_s, d) { conn.feed(dec.decode(d, { stream: true })); },
      drain() { flush(); },
      end(s) { s.end(); },
      close() { conn?.closedByPeer(); },
      error(_socket, error) { conn?.closedByPeer(error); },
    },
  });
  return conn;
}

// Remote transport: `modisa proxy` on the far side of ssh, same protocol over stdio.
export function connectStdio(cmd: string[]): Conn {
  const proc = Bun.spawn(cmd, { stdin: "pipe", stdout: "pipe", stderr: "ignore" });
  const conn = new Conn((s) => { proc.stdin.write(s); proc.stdin.flush(); }, () => proc.kill());
  (async () => {
    try {
      const dec = new TextDecoder();
      for await (const chunk of proc.stdout) conn.feed(dec.decode(chunk, { stream: true }));
      conn.closedByPeer();
    } catch (error) {
      conn.closedByPeer(error instanceof Error ? error : new Error(String(error)));
    }
  })();
  return conn;
}

// A server that's running but can't be reached, from the pid file next to its socket. Inside an agent's
// sandbox (Codex's on macOS) connecting fails with ENOENT, just like a dead server's socket, but
// signalling the process still says it exists (EPERM). So its socket must stay and no second server
// start. Outside a sandbox, ps also checks the pid still belongs to a modisa server.
export async function runningPid(sock: string): Promise<number | undefined> {
  const pid = Number(await Bun.file(sock.replace(/\.sock$/, ".pid")).text().catch(() => ""));
  if (!pid) return;
  try {
    process.kill(pid, 0);
  } catch (e) {
    if ((e as { code?: string }).code !== "EPERM") return; // no such process: a dead server
  }
  try {
    const ps = Bun.spawnSync(["ps", "-p", String(pid), "-o", "args="]);
    if (ps.success && !ps.stdout.toString().includes(" server ")) return; // the pid now belongs to something else
  } catch {} // spawning is blocked too: trust the signal
  return pid;
}

export const unreachable = (pid: number) =>
  `modisa's server is running (pid ${pid}) but this process can't connect to its socket: a sandbox is blocking it. See https://manyeya.github.io/modisa/docs/troubleshooting/#sandbox`;

// Connect to a session's server, starting it if needed.
export async function ensureServer(session: string, dir = cwd()): Promise<Conn> {
  const path = socketPath(session);
  try {
    return await connectUnix(path);
  } catch {
    const pid = await runningPid(path);
    if (pid) throw new Error(unreachable(pid));
    await Bun.file(path).delete().catch(() => {}); // stale socket from a dead server
  }
  await Bun.$`mkdir -p ${DIR}`.quiet();
  const log = Bun.file(`${DIR}/${session}.log`);
  await Bun.write(log, ""); // as stdio the file isn't truncated: a shorter run would keep the last one's tail
  const proc = Bun.spawn([...self(), "server", "-s", session], { cwd: dir, env: { ...Bun.env, PWD: dir }, stdio: ["ignore", log, log], detached: true });
  proc.unref();
  for (let i = 0; i < 100; i++) {
    await Bun.sleep(50);
    try {
      return await connectUnix(path);
    } catch {}
  }
  throw new Error(`server for session "${session}" did not start; see ${DIR}/${session}.log`);
}

// Inside a pane, MODISA_SOCKET points at our own server unless a session is named explicitly.
export async function connectExisting(session?: string): Promise<Conn> {
  const path = !session && Bun.env.MODISA_SOCKET ? Bun.env.MODISA_SOCKET : socketPath(session ?? "default");
  try {
    return await connectUnix(path);
  } catch {
    const pid = await runningPid(path);
    throw new Error(pid ? unreachable(pid) : `no modisa server for session "${session ?? "default"}"`);
  }
}
