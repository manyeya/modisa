#!/usr/bin/env bash
# The end-to-end suite of the TypeScript build (its last commit, $TS below), run against this one: its src/ and test/
# from git history (the tests import protocol types and schemas from src/), with a harness that starts our binary
# instead of `bun src/main.ts`. Needs bun. Usage: parity/run.sh [test files or patterns…]   (default: the e2e suite)
set -e -o pipefail
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
TS=b83399d53c96 # the last commit of the TypeScript build
here=$(cd "$(dirname "$0")/.." && pwd)
out=$here/target/parity
(cd "$here" && cargo build -q)
rm -rf "$out" && mkdir -p "$out"
git -C "$here" archive "$TS" src test examples package.json bun.lock bunfig.toml tsconfig.json | tar -x -C "$out"
(cd "$out" && bun install --frozen-lockfile --silent)
cp "$here/install.sh" "$out/install.sh" # this project's installer, which the update tests run
# Tests that pin UI 2's views, which this build replaced with UI 3's (examples/plugins/VIEWS.md: PLUGIN_UI 3, client
# library 11), skipped:
# - plugin-views.test.ts, all of it: UI 2's elements (box, select, progress, …), and clients attaching with ui 2 to get views
# - plugin-authoring.test.ts "every copy of the client library is the maintained one": `plugin sdk` prints library 11,
#   the TypeScript build's copy is 10
# - plugin-authoring.test.ts "the attention-log example passes plugin check…" and "the worktrees example passes plugin
#   check…": the examples from git history vendor library 10, and plugin check wants this build's 11
rm "$out/test/e2e/plugin-views.test.ts"
perl -pi -e 's/^test\((?="(every copy of the client library|the attention-log example passes|the worktrees example passes))/test.skip(/' "$out/test/e2e/plugin-authoring.test.ts"
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
log=$(mktemp)
bun test --timeout 60000 "$@" 2>&1 | tee "$log" || status=$?
pkill -f "$MODISA_BIN server" || true # a test that only deletes its sandbox leaves that sandbox's server running
# ponytail: a file that failed runs once more, and passing then is reported as flaky, not hidden. macOS CI's slow
# runners sometimes leave a TUI test's menu open after a resize (test/e2e/ui/mouse.test.ts); it doesn't happen
# locally. A file that fails twice fails the run.
if [ $status -ne 0 ]; then
  # a file's header is "test/….test.ts:" (in GitHub Actions, "##[group]test/….test.ts:")
  failed=$(awk '{ line = $0; sub(/^##\[group\]/, "", line) } line ~ /^test\/.*\.test\.ts:$/ { file = substr(line, 1, length(line) - 1) } /^\(fail\)/ && file { print file }' "$log" | sort -u)
  if [ -n "$failed" ]; then
    echo "re-running what failed: $failed"
    # shellcheck disable=SC2086
    if bun test --timeout 60000 $failed; then
      echo "FLAKY (passed on the second run): $failed"
      status=0
    fi
    pkill -f "$MODISA_BIN server" || true
  fi
fi
rm -f "$log"
exit $status
