#!/bin/sh
# The end-to-end suite of the TypeScript build (its last commit, $TS below), run against this one: its src/ and test/
# from git history (the tests import protocol types and schemas from src/), with a harness that starts our binary
# instead of `bun src/main.ts`. Needs bun. Usage: parity/run.sh [test files or patterns…]   (default: the e2e suite)
set -e
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
TS=b83399d53c96 # the last commit of the TypeScript build
here=$(cd "$(dirname "$0")/.." && pwd)
out=$here/target/parity
(cd "$here" && cargo build -q)
rm -rf "$out" && mkdir -p "$out"
git -C "$here" archive "$TS" src test examples package.json bun.lock bunfig.toml tsconfig.json | tar -x -C "$out"
(cd "$out" && bun install --frozen-lockfile --silent)
cp "$here/install.sh" "$out/install.sh" # this project's installer, which the update tests run
# every way the suite starts modisa, pointed at the Rust binary
find "$out/test" -name '*.ts' -exec perl -pi -e '
  s/\["bun", MAIN,/[process.env.MODISA_BIN!,/g;
  s/\["bun", `\$\{import\.meta\.dir\}\/\.\.\/\.\.\/src\/main\.ts`,/[process.env.MODISA_BIN!,/g;
  s/bun \$\{MAIN\}/\$\{process.env.MODISA_BIN\}/g;
  s/l\.includes\(MAIN\)/l.includes(process.env.MODISA_BIN!)/g;
' {} +
export MODISA_BIN="$here/target/debug/modisa"
unset GHOSTTY_VT_LIB GHOSTTY_VT_SHIM_LIB # a TypeScript-build session sets these for its panes; the suite finds its own
export MODISA_DIR="$out/state" # the suite's own process (its libghostty) never touches the real ~/.local/state/modisa
cd "$out"
if [ $# -eq 0 ]; then set -- test/e2e; fi
status=0
bun test --timeout 60000 "$@" || status=$?
pkill -f "$MODISA_BIN server" || true # a test that only deletes its sandbox leaves that sandbox's server running
exit $status
