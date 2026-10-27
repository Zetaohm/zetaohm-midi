#!/bin/sh
# Copies the TypeScript package (bindings/ts: dist/*.js and wasm/zmidi.wasm) into lib/js/, where the
# web form of the package loads it (DEC-019 addendum of 2026-10-24). It builds the TypeScript
# package first when its files are missing or older than its sources. Needs pnpm, and the Rust
# target wasm32-unknown-unknown for scripts/build-wasm.sh of bindings/ts.
set -eu
cd "$(dirname "$0")/.."
ts=../ts
if [ ! -f "$ts/wasm/zmidi.wasm" ] || [ -n "$(find ../../crates -name '*.rs' -newer "$ts/wasm/zmidi.wasm" | head -1)" ]; then
  (cd "$ts" && sh scripts/build-wasm.sh)
fi
if [ ! -f "$ts/dist/index.js" ] || [ -n "$(find "$ts/src" -name '*.ts' -newer "$ts/dist/index.js" | head -1)" ]; then
  (cd "$ts" && pnpm install --frozen-lockfile >/dev/null && pnpm exec tsc -p tsconfig.build.json)
fi
rm -rf lib/js
mkdir -p lib/js/dist lib/js/wasm
cp "$ts"/dist/*.js lib/js/dist/
cp "$ts/wasm/zmidi.wasm" lib/js/wasm/
