#!/bin/sh
# Feature matrix of the workspace crates (DEC-017, OPT-052 step 4).
#
# For each crate and each feature combination of `cargo hack --feature-powerset --depth 2` (each
# feature alone, each pair, no default features, the default features):
#   - clippy of the library for the bare-metal target (DEC-002), warnings as errors
#   - clippy of the library and the tests on the host, warnings as errors
#   - the host tests with cargo-nextest (each test in its own process, in parallel)
#
# usage: sh scripts/feature-matrix.sh [WORKERS]
#   WORKERS parallel workers on this machine (default: 4). Each worker runs one partition
#   (`--partition i/WORKERS`) with its own CARGO_TARGET_DIR, because workers that share a target
#   directory wait for its lock.
# In CI, one job per partition: PARTITION=i/N sh scripts/feature-matrix.sh
#
# Needs cargo-hack and cargo-nextest (cargo install --locked cargo-hack cargo-nextest). Do not add
# --no-dev-deps: it edits the manifests on disk while it runs, which breaks parallel workers.

set -u
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT" || exit 2
for tool in hack nextest; do
    cargo "$tool" --version >/dev/null 2>&1 || {
        echo "feature-matrix: cargo-$tool is missing (cargo install --locked cargo-hack cargo-nextest)" >&2
        exit 2
    }
done

# DEC-019: zmidi-py (PyO3) needs a Python interpreter; the Python package tests it.
HACK="cargo hack --workspace --exclude zmidi-py --feature-powerset --depth 2 --keep-going"

# Each part: the bare-metal check, then the host tests. $HACK holds several words on purpose.
part_cmds() {
    # The feature std of zmidi-ffi is the host build of its staticlib and cdylib (DEC-019); a
    # bare-metal target has no std, so the bare-metal check leaves it out.
    # shellcheck disable=SC2086
    $HACK --partition "$1" clippy -q --lib --target thumbv7em-none-eabihf --exclude-features std -- -D warnings &&
        # shellcheck disable=SC2086
        $HACK --partition "$1" clippy -q --all-targets -- -D warnings &&
        # shellcheck disable=SC2086
        $HACK --partition "$1" nextest run --no-tests=pass --hide-progress-bar --status-level fail
}

if [ -n "${PARTITION:-}" ]; then
    part_cmds "$PARTITION"
    exit $?
fi

N=${1:-4}
LOGS="$ROOT/target/feature-matrix"
mkdir -p "$LOGS"
pids=""
i=1
while [ "$i" -le "$N" ]; do
    (CARGO_TARGET_DIR="$ROOT/target/fm-$i" part_cmds "$i/$N" >"$LOGS/part-$i.log" 2>&1; echo $? >"$LOGS/part-$i.exit") &
    pids="$pids $!"
    i=$((i + 1))
done
wait $pids

status=0
i=1
while [ "$i" -le "$N" ]; do
    code=$(cat "$LOGS/part-$i.exit" 2>/dev/null || echo 2)
    runs=$(grep -c "^info: running" "$LOGS/part-$i.log" 2>/dev/null || echo 0)
    echo "part $i/$N: exit $code, $runs runs (log: target/feature-matrix/part-$i.log)"
    [ "$code" = 0 ] || status=1
    i=$((i + 1))
done
exit $status
