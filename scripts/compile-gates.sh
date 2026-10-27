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

# Each member checks itself; stop at the first failure.
echo 'compile-gates: member zmidi-ump' >&2
(cd 'crates/zmidi-ump' && sh scripts/compile-gates.sh) || exit $?
echo 'compile-gates: member zmidi-ci' >&2
(cd 'crates/zmidi-ci' && sh scripts/compile-gates.sh) || exit $?
echo 'compile-gates: member zmidi-ci-pe' >&2
(cd 'crates/zmidi-ci-pe' && sh scripts/compile-gates.sh) || exit $?
echo 'compile-gates: member zmidi-file' >&2
(cd 'crates/zmidi-file' && sh scripts/compile-gates.sh) || exit $?
echo 'compile-gates: member zmidi-io' >&2
(cd 'crates/zmidi-io' && sh scripts/compile-gates.sh) || exit $?
echo 'compile-gates: member zmidi-net' >&2
(cd 'crates/zmidi-net' && sh scripts/compile-gates.sh) || exit $?
echo 'compile-gates: member zmidi-usb' >&2
(cd 'crates/zmidi-usb' && sh scripts/compile-gates.sh) || exit $?
echo 'compile-gates: member zmidi-ble' >&2
(cd 'crates/zmidi-ble' && sh scripts/compile-gates.sh) || exit $?
echo 'compile-gates: member zmidi-ffi' >&2
(cd 'crates/zmidi-ffi' && sh scripts/compile-gates.sh) || exit $?
echo 'compile-gates: member zmidi-py' >&2
(cd 'crates/zmidi-py' && sh scripts/compile-gates.sh) || exit $?
