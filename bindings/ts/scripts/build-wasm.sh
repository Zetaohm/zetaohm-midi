#!/bin/sh
# Builds wasm/zmidi.wasm: the zmidi-ffi C ABI with every part, net-auth included, for
# wasm32-unknown-unknown (DEC-019), with the Cargo profile `ffi` and the panic handler of bare-metal
# and WASM builds.
set -eu
cd "$(dirname "$0")/.."
root=../..
cargo rustc --quiet --manifest-path "$root/Cargo.toml" -p zmidi-ffi --profile ffi \
  --target wasm32-unknown-unknown --no-default-features \
  --features panic-handler,io,ump,ble,file,ci,net,net-auth,usb --crate-type cdylib
mkdir -p wasm
cp "$root/target/wasm32-unknown-unknown/ffi/zmidi.wasm" wasm/zmidi.wasm
