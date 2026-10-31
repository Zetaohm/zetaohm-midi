#!/bin/sh
# Runs each fuzz target of fuzz/ for SECONDS seconds (default 60) with libFuzzer, and stops at
# the first target that finds a crash, a panic or a timeout (the input is in fuzz/artifacts/).
# Needs a nightly toolchain and `cargo install cargo-fuzz`. FUZZ_SANITIZER (default none) picks the
# sanitizer: the library has no unsafe code, so a panic is the failure to find; `address` needs a
# platform where the AddressSanitizer runtime starts (on macOS 27 it hangs at start-up).
#   sh scripts/fuzz.sh [SECONDS] [TARGET...]
set -eu
cd "$(dirname "$0")/../fuzz"
secs="${1:-60}"
[ $# -gt 0 ] && shift
targets="${*:-ump byte_stream ci net ble usb clip smf}"
san="${FUZZ_SANITIZER:-none}"
cargo +nightly fuzz build -O -s "$san"
mkdir -p target
for t in $targets; do
  echo "fuzz: $t (${secs} s)" >&2
  log="target/fuzz-$t.log"
  if ! cargo +nightly fuzz run -O -s "$san" "$t" -- -max_total_time="$secs" -timeout=10 -rss_limit_mb=2048 \
    >"$log" 2>&1; then
    tail -n 40 "$log" >&2
    echo "fuzz: $t failed; the input is in fuzz/artifacts/$t/" >&2
    exit 1
  fi
  tail -n 1 "$log" >&2
done
echo "fuzz: no crash in $targets" >&2
