// --remote: a local client, the server on the far side of ssh via `shepherd proxy`.
import { test, expect, afterAll } from "bun:test";
import { MAIN, Screen, sandbox } from "../support/harness";

const sb = sandbox("remote");

afterAll(async () => {
  await sb.cli("remote", ["kill", "remote"]);
  await sb.cleanup();
});

test("--remote attaches through ssh running `shepherd proxy` on the far side", async () => {
  // fake ssh: drop "-T" and the host, run the rest locally like a remote shell would
  await Bun.write(`${sb.root}/bin/fakessh`, `#!/bin/sh\nshift; shift; eval "$@"\n`);
  await Bun.$`chmod +x ${sb.root}/bin/fakessh`;
  await Bun.write(`${sb.root}/config/config.toml`, `remote_command = "bun ${MAIN}"\n`);
  const remote = new Screen(["-s", "remote", "--remote", "ssh://devbox"], { ...sb.env, SHEPHERD_SSH: `${sb.root}/bin/fakessh` }, sb.root);
  await remote.until("remote client", (s) => s.includes("SPACES") && s.includes("+ agent"), 15000);
  remote.write("echo over-ssh\r");
  await remote.until("remote shell output", (s) => s.includes("over-ssh"));
  remote.write("\x02d");
  expect(await Promise.race([remote.proc.exited, Bun.sleep(5000).then(() => "timeout")])).toBe(0);
  expect(await sb.cli("remote", ["ls"])).toContain("remote\t1 panes");
}, 30000);
