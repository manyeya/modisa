// A stand-in coding agent, installed into a sandbox's PATH with a matching adapter.
// `fakeagent`: shows a "working" marker for 4s, then a prompt that echoes what it's sent.
// `fakeagent --ask`: first sits in a dialog, like a real agent waiting on you.
// Each launch appends its arguments to <root>/fakeagent.log (the screen is cleared after 4s).
export async function installFakeAgent(root: string) {
  await Bun.write(
    `${root}/bin/fakeagent`,
    `#!/bin/sh
echo "$*" >> "${root}/fakeagent.log"
if [ "$1" = "--ask" ]; then echo "Pick a colour"; echo " 1. red"; echo "Enter to confirm · Esc to cancel"; read x; fi
echo "thinking... esc to interrupt"; sleep 4; printf '\\033[2J\\033[H'
while true; do printf '> '; IFS= read -r line || exit 0; [ -n "$line" ] && echo "got: $line"; done
`,
  );
  await Bun.$`chmod +x ${root}/bin/fakeagent`;
  await Bun.write(
    `${root}/config/adapters/fakeagent.toml`,
    `name = "Fake"
process = ["fakeagent"]
launch = "fakeagent"

[[state]]
name = "blocked"
region = 3
match = "Enter to confirm"

[[state]]
name = "working"
region = 8
match = "esc to interrupt"
`,
  );
}
