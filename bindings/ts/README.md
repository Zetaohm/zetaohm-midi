# zetaohm-midi for TypeScript

[![npm](https://img.shields.io/npm/v/zetaohm-midi.svg)](https://www.npmjs.com/package/zetaohm-midi)

MIDI 2.0 for TypeScript and JavaScript: UMP, MIDI-CI, Network MIDI 2.0, BLE-MIDI, USB-MIDI and MIDI
Clip Files. The package runs the [zetaohm-midi](../../README.md) C ABI as WebAssembly, in Node.js and
in the browser.

## Features

- UMP: `parse`, builders such as `midi2NoteOn`, MIDI 2.0 → 1.0 translation, value scaling.
- MIDI 1.0 byte stream ↔ UMP: `BytesToUmp`, `UmpToBytes`. Use them with Web MIDI.
- MIDI-CI: `CiDevice`. Network MIDI 2.0: `NetHost` and `NetClient`, with authentication.
- USB-MIDI: `UsbLink` (for WebUSB). BLE-MIDI: `BleReader` and `BleWriter` (for Web Bluetooth).
- USB descriptors: `UsbFunction` takes the full description of a USB MIDI function in one constructor
  and gives its descriptors (USB-MIDI v2.0 Appendix B order), its endpoints, the Group Terminal masks
  for `UsbLink`, and the Group Terminal Block requests (device side `answerSetup`, host side
  `gtbRequest`, `gtbTotalLength`, `gtbBlocks`).
- MIDI Clip File: `readClip` and `ClipWriter`.
- Sans-IO: you own the transport. One small WASM file, no other runtime dependency.

## Install

The package is not yet on npm. Build it from the repository with a Rust toolchain and
[pnpm](https://pnpm.io):

```sh
cd bindings/ts
sh scripts/build-wasm.sh       # writes wasm/zmidi.wasm
pnpm install
pnpm exec tsc -p tsconfig.build.json
```

## Example

```ts
import { BytesToUmp, UmpToBytes, init, midi2NoteOn, parse } from "zetaohm-midi";

await init(); // load the WASM module once

// Build and parse a MIDI 2.0 Note On.
const ump = parse(midi2NoteOn(0, 0, 60, 0xc000));
console.log(ump.messageType, ump.group, ump.channel, ump.words);

// MIDI 1.0 bytes to MIDI 2.0 Protocol UMPs, and back to bytes.
const dinIn = new BytesToUmp(0, 2);
console.log(dinIn.feed(new Uint8Array([0x90, 0x3c, 0x64])));
dinIn.free(); // optional: frees the WASM memory now, not at garbage collection

const dinOut = new UmpToBytes(true);
console.log(dinOut.write([0x20903c64]));
dinOut.free(); // optional
```

Call `init()` once before other functions. In a browser, it fetches `zmidi.wasm` from beside the
package; you can also give it a URL, a `Response`, bytes or a compiled `WebAssembly.Module`.
`initSync` takes bytes or a module.

## Errors

Every error is a `MidiError`: `InvalidValueError` for an argument out of range, `MalformedError` for
received data that breaks the specification, and `RejectedError` for an operation that is not
possible in the current state.

## Development

```sh
sh scripts/build-wasm.sh
pnpm install --frozen-lockfile
pnpm exec tsc --noEmit
pnpm exec vitest run
```

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
