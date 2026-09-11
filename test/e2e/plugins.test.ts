// Plugins: commands from [[plugin]] start with the server and get $SHEPHERD_SOCKET.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";

const sb = sandbox("plugins");
const S = "plug";
let server: Bun.Subprocess;

beforeAll(async () => {
  await Bun.write(`${sb.root}/config/config.toml`, `[[plugin]]\nrun = "echo $SHEPHERD_SOCKET > ${sb.root}/plugin.out"\n`);
  server = await startServer(sb, S);
}, 20000);

afterAll(async () => {
  await sb.cli(S, ["kill", S]);
  await server?.exited;
  await sb.cleanup();
});

test("plugins start with SHEPHERD_SOCKET", async () => {
  for (let i = 0; i < 30 && !(await Bun.file(`${sb.root}/plugin.out`).exists()); i++) await Bun.sleep(100);
  expect((await Bun.file(`${sb.root}/plugin.out`).text()).trim()).toBe(`${sb.root}/state/${S}.sock`);
});
