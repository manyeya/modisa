// SHEPHERD_DEBUG_MOUSE=1: OpenTUI logs raw terminal input (its OTUI_STDIN_LOG) and we log what the
// mouse handling decided, both next to the session state, to debug mouse issues in a real terminal.
import type { CliRenderer } from "@opentui/core";
import { DIR } from "../core/paths";

// Call before creating the renderer: OpenTUI reads OTUI_STDIN_LOG when it starts.
export async function openMouseLog(): Promise<(line: string) => void> {
  if (!Bun.env.SHEPHERD_DEBUG_MOUSE) return () => {};
  await Bun.$`mkdir -p ${DIR}`.quiet();
  const log = Bun.file(`${DIR}/mouse.log`).writer();
  Bun.env.OTUI_STDIN_LOG ??= `${DIR}/stdin.log`;
  return (line) => {
    log.write(`${new Date().toISOString().slice(11, 23)} ${line}\n`);
    log.flush();
  };
}

export function logHandlerErrors(r: CliRenderer, debug: (line: string) => void) {
  if (Bun.env.SHEPHERD_DEBUG_MOUSE) r.on("handler:error", ({ error, event }: any) => debug(`opentui handler error on ${event?.type}: ${error}`));
}
