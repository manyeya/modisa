// A message's reply hint names its sender pane by id and instance: it still works after the sender is
// renamed, and fails clearly (never reaching someone else) once the sender has gone.
import { test, expect, beforeAll, afterAll } from "bun:test";
import { sandbox, startServer } from "../support/harness";
import { installFakeAgent } from "../support/fake-agent";

const sb = sandbox("replies");
const S = "rep";
const cli = (...args: string[]) => sb.cli(S, args);
let server: Bun.Subprocess;
let fake: string;

// an agent in its own tab (full width, so the hint isn't wrapped), idle at its prompt
async function spawn(...flags: string[]) {
  const id = await cli("agent", "spawn", "fakeagent", "--tab", ...flags);
  await cli("wait", id, "--state", "working", "--timeout", "10");
  await cli("wait", id, "--state", "idle", "--timeout", "15");
  return id;
}

// send from a pane, then read the reply target the recipient was shown
async function sendFrom(from: string, label: string, body: string) {
  expect(await sb.cli(S, ["send", "@fake", body], { MODISA_PANE_ID: from })).toContain("queued for @fake");
  const hint = await cli("wait", "@fake", "--match", `message from @${label} \\(reply: modisa send \\S+`, "--timeout", "10");
  return hint.split(" ").at(-1)!;
}

beforeAll(async () => {
  await installFakeAgent(sb.root);
  server = await startServer(sb, S);
  fake = await spawn("--name", "fake");
}, 40000);

afterAll(async () => {
  await cli("kill", S);
  await server?.exited;
  await sb.cleanup();
});

test("send → rename the sender → reply to the hinted target → delivered", async () => {
  const a = await spawn();
  const hint = await sendFrom(a, a, "hello-1");
  expect(hint).toMatch(new RegExp(`^${a}:\\w+$`));
  await cli("pane", "rename", a, "gary");
  expect(await sb.cli(S, ["send", hint, "pong-1"], { MODISA_PANE_ID: fake })).toContain(`queued for ${hint}`);
  expect(await cli("wait", "@gary", "--match", "got: pong-1", "--timeout", "10")).toBe("got: pong-1");
  expect(await cli("pane", "read", `@${a}`)).toContain("pong-1"); // and @<id> still resolves once it has a name
}, 60000);

test("send → close the sender → a new pane takes its name → the reply fails and isn't misdelivered", async () => {
  const b = await spawn("--name", "bob");
  const hint = await sendFrom(b, "bob", "hello-2");
  await cli("pane", "close", b);
  await spawn("--name", "bob");
  const r = await sb.run(S, ["send", hint, "pong-2"], { MODISA_PANE_ID: fake });
  expect(r.code).toBe(1);
  expect(r.out).toContain(`${b} has gone`);
  expect(await cli("messages")).not.toContain("pong-2");
}, 60000);

// last: it pauses delivery, so messages stay queued and are pulled with `inbox`
test("the pull flow (inbox, then reply) keeps its target through a rename, and fails after a close-and-replace", async () => {
  expect(await cli("pause")).toBe("messaging paused");
  try {
    const c = await spawn("--name", "carol");
    const as = (id: string) => ({ MODISA_PANE_ID: id });
    await sb.cli(S, ["send", "@fake", "hello-3"], as(c));
    const plain = await sb.cli(S, ["inbox"], as(fake));
    const hint = /from @carol \(reply: modisa send (\S+) "\.\.\."\):\nhello-3/.exec(plain)?.[1];
    expect(hint).toMatch(new RegExp(`^${c}:\\w+$`));
    await cli("pane", "rename", c, "dave");
    await sb.cli(S, ["send", hint!, "pong-3"], as(fake));
    expect(JSON.parse(await sb.cli(S, ["inbox", "--json"], as(c)))).toMatchObject([{ from: "fake", body: "pong-3" }]);

    await sb.cli(S, ["send", "@fake", "hello-4"], as(c));
    const [m] = JSON.parse(await sb.cli(S, ["inbox", "--json"], as(fake)));
    expect(m).toMatchObject({ from: "dave", body: "hello-4", replyTo: hint });
    await cli("pane", "close", c);
    await spawn("--name", "dave");
    const r = await sb.run(S, ["send", m.replyTo, "pong-4"], as(fake));
    expect(r.code).toBe(1);
    expect(r.out).toContain(`${c} has gone`);
    expect(await cli("messages")).not.toContain("pong-4");
  } finally {
    await cli("pause"); // resumed
  }
}, 90000);
