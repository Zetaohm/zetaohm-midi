#!/bin/sh
# Checks of the adapter crates: USB (DEC-014, DEC-015) and BLE (DEC-016). They are Cargo-only
# members, not specdev members, so scripts/specdev-gates.sh does not check them. CI runs this
# script after the gates.
set -eu
cd "$(dirname "$0")/.."
for crate in zmidi-usb-device zmidi-usb-embassy zmidi-ble-trouble; do
  echo "adapters-check: $crate" >&2
  # DEC-002: no_std, no alloc, bare-metal target.
  cargo check --quiet -p "$crate" --lib --no-default-features --target thumbv7em-none-eabihf
  cargo test --quiet -p "$crate"
  cargo clippy --quiet -p "$crate" --all-targets -- -D warnings
done
