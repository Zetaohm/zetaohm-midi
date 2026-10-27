// Cost of a call through the TypeScript package (docs/optimization-backlog.md OPT-029 step 4).
//
// Wall-clock time on the host, not a gate. Each row gives the time of one call and of each MIDI
// message, so that the fixed cost of a call (the WASM call, the scratch memory) shows against the
// work in the library. Build first (`sh scripts/specdev-gates.sh` or `pnpm exec tsc -p
// tsconfig.build.json`), then:
//
//     node scripts/bench.mjs

import { readFileSync } from "node:fs";
import { BytesToUmp, UmpToBytes, initSync } from "../dist/index.js";

initSync(readFileSync(new URL("../wasm/zmidi.wasm", import.meta.url)));

const notes = new Uint8Array(Array.from({ length: 64 }, (_, i) => [0x90, i, 0x40]).flat());
const words = Array.from({ length: 64 }, (_, i) => (0x20900040 | (i << 8)) >>> 0);

function row(name, f, messages, n = 2000) {
  let best = Infinity;
  for (let r = 0; r < 5; r++) {
    const t0 = performance.now();
    for (let i = 0; i < n; i++) f();
    best = Math.min(best, (performance.now() - t0) / n);
  }
  const us = best * 1000;
  console.log(`${name.padEnd(40)} ${us.toFixed(2).padStart(9)} us/call ${((us * 1000) / messages).toFixed(1).padStart(9)} ns/message`);
}

const r = new BytesToUmp();
const w = new UmpToBytes(true);
row("BytesToUmp.push, one byte", () => r.push(0xf8), 1, 20000);
row("BytesToUmp.feed, 64 notes (192 bytes)", () => r.feed(notes), 64);
row("BytesToUmp.feed, one note (3 bytes)", () => r.feed(notes.subarray(0, 3)), 1, 20000);
row("UmpToBytes.write, one note", () => w.write(words.slice(0, 1)), 1, 20000);
