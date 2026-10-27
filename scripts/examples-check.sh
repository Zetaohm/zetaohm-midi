#!/bin/sh
# Builds the examples (DEC-003: examples are outside the Cargo workspace, and CI builds them).
# - Firmware: its .cargo/config.toml sets the target; a bare-metal target has no test harness,
#   so build and clippy only. The Cortex-M firmware (mk20dx-usb-midi, stm32wb55-usb-midi) needs
#   `rustup target add thumbv7em-none-eabi thumbv7em-none-eabihf`.
# - An example whose rust-toolchain.toml asks for the Espressif "esp" toolchain (Xtensa, for
#   example the ESP32-S3) needs `espup install`; without it the example is skipped with a
#   message, or fails when EXAMPLES_REQUIRE_ESP=1. ~/export-esp.sh (written by espup) gives the
#   linker path. An esp example with a rust-version newer than the esp rustc is skipped (or
#   fails) the same way.
# - A host example: clippy for all targets, then its tests.
# CI runs this script after the adapter checks.
set -eu
cd "$(dirname "$0")/.."
if [ -f "$HOME/export-esp.sh" ]; then
  . "$HOME/export-esp.sh"
fi
for dir in examples/*/; do
  echo "examples-check: $dir" >&2
  if grep -qs 'channel = "esp"' "$dir/rust-toolchain.toml"; then
    if ! rustup toolchain list | grep -q '^esp'; then
      if [ "${EXAMPLES_REQUIRE_ESP:-0}" = 1 ]; then
        echo "examples-check: $dir needs the esp toolchain (espup install)" >&2
        exit 1
      fi
      echo "examples-check: skipped $dir: no esp toolchain (espup install)" >&2
      continue
    fi
    # The rust-version of the example (for example 1.95 for esp-hal 1.2) against the esp rustc.
    need=$(sed -n 's/^rust-version *= *"\([0-9.]*\)".*/\1/p' "$dir/Cargo.toml")
    have=$(cd "$dir" && rustc --version | sed -n 's/^rustc \([0-9.]*\).*/\1/p')
    if [ -n "$need" ] && [ "$(printf '%s\n%s\n' "$need" "$have" | sort -t. -k1,1n -k2,2n -k3,3n | head -n1)" != "$need" ]; then
      if [ "${EXAMPLES_REQUIRE_ESP:-0}" = 1 ]; then
        echo "examples-check: $dir needs rustc $need; the esp toolchain is $have" >&2
        exit 1
      fi
      echo "examples-check: skipped $dir: needs rustc $need, the esp toolchain is $have (espup install)" >&2
      continue
    fi
  fi
  if [ -f "$dir/.cargo/config.toml" ]; then
    (cd "$dir" && cargo build --quiet --release && cargo clippy --quiet --release -- -D warnings)
  else
    (cd "$dir" && cargo clippy --quiet --release --all-targets -- -D warnings && cargo test --quiet --release)
  fi
done
