#!/bin/sh
# Generated from this project's compile-gate list, which is kept with the
# project's specification. Do not edit: a copy that differs from the list is
# reported there. CI runs it: `sh scripts/compile-gates.sh`.
set -u
cd "$(dirname "$0")/.." || exit 2
exec </dev/null

# Run one step; stop at the first non-zero exit and exit with it. A step
# that could not start (126, 127) exits 2, as the runner does.
run() {
  "$@"
  status=$?
  if [ "$status" -ne 0 ]; then
    if [ "$status" -eq 126 ] || [ "$status" -eq 127 ]; then
      echo "compile-gates: $1 could not start: install it (on a CI runner, install it in CI's workflow)" >&2
      status=2
    fi
    echo "compile-gates: step failed (exit $status): $*" >&2
    exit "$status"
  fi
}

echo 'compile-gates: gate wasm' >&2
run 'sh' 'scripts/build-wasm.sh'

echo 'compile-gates: gate install' >&2
run 'pnpm' 'install' '--frozen-lockfile'

echo 'compile-gates: gate types' >&2
run 'pnpm' 'exec' 'tsc' '--noEmit'

echo 'compile-gates: gate tests' >&2
run 'pnpm' 'exec' 'vitest' 'run'

echo 'compile-gates: gate build' >&2
run 'pnpm' 'exec' 'tsc' '-p' 'tsconfig.build.json'
