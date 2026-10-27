# zetaohm-midi for Dart

[![pub package](https://img.shields.io/pub/v/zetaohm_midi.svg)](https://pub.dev/packages/zetaohm_midi)

MIDI 2.0 for Dart and Flutter: UMP, MIDI-CI with Property Exchange, Network MIDI 2.0, BLE-MIDI,
USB-MIDI and MIDI Clip Files, through the [zetaohm-midi](../../README.md) C ABI: `dart:ffi` on the
native platforms, and the WASM build on the web (dart2js and dart2wasm).

## Features

- UMP: `parse`, builders such as `midi2NoteOn`, MIDI 2.0 → 1.0 translation, value scaling.
- MIDI 1.0 byte stream ↔ UMP: `BytesToUmp`, `UmpToBytes`.
- MIDI-CI: `CiDevice`. Network MIDI 2.0: `NetHost` and `NetClient`, with authentication.
- USB-MIDI: `UsbLink`. BLE-MIDI: `BleReader` and `BleWriter`.
- MIDI Clip File: `readClip` and `ClipWriter`.
- Sans-IO: you own the sockets, the USB device and the BLE connection.

## Install

The package is not yet on pub.dev. Its build hook compiles the native library with Cargo, so you
need a Rust toolchain. Use a path dependency:

```yaml
dependencies:
  zetaohm_midi:
    path: path/to/zetaohm-midi/bindings/dart
```

Dart SDK 3.10 or later.

## Example

```dart
import 'package:zetaohm_midi/zetaohm_midi.dart';

void main() {
  // Build and parse a MIDI 2.0 Note On.
  final ump = parse(midi2NoteOn(0, 0, 60, 0xC000));
  print('${ump.messageType} ${ump.group} ${ump.words}');

  // MIDI 1.0 bytes to MIDI 2.0 Protocol UMPs, and back to bytes.
  final dinIn = BytesToUmp(protocol: 2);
  print(dinIn.feed([0x90, 0x3C, 0x64])); // == [[0x40903C00, 0xC9240000]]
  dinIn.free(); // optional: frees the native memory now, not at garbage collection

  final dinOut = UmpToBytes(runningStatus: true);
  print(dinOut.write([0x20903C64])); // == [0x90, 0x3C, 0x64]
  dinOut.free();
}
```

## Web

On the web the package uses the WASM build of the C ABI through the TypeScript package of
[`bindings/ts`](../ts/README.md). The API is the same. Call `initWeb` once, and wait for it, before
the other calls; on the native platforms it does nothing:

```dart
Future<void> main() async {
  await initWeb();
  print(wordCount(0x40000000)); // 2
}
```

`initWeb` loads `packages/zetaohm_midi/js/dist/index.js` beside the page, or the URL that you give
with `moduleUrl` (for example when your build puts the package assets in another directory, as
Flutter web does). `tool/web_assets.sh` copies the TypeScript package into `lib/js/`. On the web,
times and tick counts go to 2^53 - 1, and `NetAddress.address` is an address type of the package
with the parts of `dart:io` `InternetAddress` that `NetAddress` uses: use `NetAddress.parse`.

## Errors

Every error is a `MidiException`: `InvalidValueException` for an argument out of range,
`MalformedException` for received data that breaks the specification, and `RejectedException` for
an operation that is not possible in the current state.

## Development

```sh
dart pub get
dart analyze --fatal-infos
dart test
sh tool/web_assets.sh
dart test -p chrome -c dart2js -c dart2wasm
```

After a change of `zmidi.h`, regenerate the bindings with `dart run tool/ffigen.dart`.

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
