# zmidi-ffi

The C ABI of [zetaohm-midi](../../README.md): one header, [`zmidi.h`](include/zmidi.h), for C, C++
and other languages. `no_std`, no allocator, no callbacks.

## Features

- All `zmidi_` functions return a `ZmidiStatus`. No function panics on input from the program.
- The program owns the storage of each object. `zmidi_<object>_size()` gives the size, and the
  header gives a `Zmidi<Object>` type of that size for static or stack storage. The library never
  calls `malloc`.
- Parts: UMP, MIDI 1.0 byte stream, MIDI-CI (each category separately), MIDI Clip File, Network MIDI
  2.0 (with optional authentication), USB-MIDI and BLE-MIDI, and the MIDI 1.0 receiver states (typed
  messages on bytes, Channel Mode, controllers, RPN and NRPN). A build contains only the parts that
  you select, and the header shows only their declarations.
- A MIDI 1.0-only build (`midi1`) links no MIDI 2.0 code: about 11 KB of flash on Cortex-M4F for a
  16-channel DIN receiver with the io, ble and usb parts.
- The same library runs on bare metal, on a desktop and in WebAssembly. The
  [Python](../../bindings/python), [TypeScript](../../bindings/ts) and
  [Dart](../../bindings/dart) packages use it.

## Example

```c
#include <stdio.h>
#include "zmidi.h"

int main(void) {
  /* The program owns the storage; there is no malloc in the library. */
  ZmidiBytesToUmp din_in;
  uint32_t words[ZMIDI_BYTES_TO_UMP_MAX_WORDS];
  const uint8_t bytes[] = {0x90, 0x3C, 0x64}; /* Note On, MIDI 1.0 byte stream */
  size_t i, count = 0;

  /* Group 0, MIDI 2.0 Protocol output (1: MIDI 1.0 Protocol). */
  if (zmidi_bytes_to_ump_init(&din_in, 0, 2) != ZMIDI_STATUS_OK) return 1;
  for (i = 0; i < sizeof bytes; i++) {
    if (zmidi_bytes_to_ump_push(&din_in, bytes[i], words, ZMIDI_BYTES_TO_UMP_MAX_WORDS, &count)
        != ZMIDI_STATUS_OK) return 1;
  }
  if (count == 2) printf("UMP %08X %08X\n", (unsigned)words[0], (unsigned)words[1]);
  return 0;
}
```

Output: `UMP 40903C00 C9240000`.

More examples: the C tests in [`tests/c`](tests/c), one file for each part.

## USB descriptors in firmware

The `usb` part writes the descriptors of a USB MIDI function at run time (`zmidi_usb_function_*`,
at most 2 USB MIDI 2.0 settings, 4 endpoints and 16 Group Terminal Blocks for each setting) and
answers the Group Terminal Block requests (`zmidi_usb_gtb_setup`). On Cortex-M4F that is about
8.4 KB of flash. A firmware with a fixed function can do the same in about 1.3 KB:

1. A host program builds the function with the same calls and writes it as C source with
   `zmidi_usb_function_print_c` of [`zmidi_usb_gen.h`](include/zmidi_usb_gen.h): the descriptors,
   the Group Terminal Block sets, the endpoints and the Group Terminal masks as constant arrays.
2. The firmware includes that source, gives the descriptors to its USB stack, and answers the
   requests with `zmidi_usb_fixed_gtb_setup`. It links no descriptor writer and no rule check.

## Byte streams in bulk

`zmidi_bytes_to_ump_feed` reads many bytes of a MIDI 1.0 byte stream in one call. It stops before a
byte that could not fit in the word buffer and gives the number of bytes read, so the caller calls
again with the rest. The words are the same as one `zmidi_bytes_to_ump_push` for each byte.

## Build with CMake

The package uses [Corrosion](https://github.com/corrosion-rs/corrosion) to build the Rust library.
It defines the `ZMIDI_<PART>` macros of the parts that you select.

```cmake
set(ZMIDI_FEATURES io ump ci)   # the parts that you need
add_subdirectory(path/to/zetaohm-midi/crates/zmidi-ffi zmidi)
target_link_libraries(app PRIVATE zmidi::zmidi)
```

For bare metal, set `Rust_CARGO_TARGET` (for example `thumbv7em-none-eabihf`) in your toolchain
file. The package then builds the `no_std` library with its own panic handler.

## Build with Meson

The repository root is a Meson project: `meson.build` builds the Rust library with Cargo and gives
the dependency `zmidi`, with the header, the `ZMIDI_<PART>` macros of the parts that you select and
the system libraries that the library needs. It needs Meson 1.1 or later, Cargo and Python 3. Add
the repository as a subproject with a wrap file, `subprojects/zetaohm-midi.wrap`:

```ini
[wrap-git]
url = https://github.com/Zetaohm/zetaohm-midi.git
revision = main
depth = 1

[provide]
dependency_names = zmidi
```

Then use it in your `meson.build`:

```meson
zmidi = dependency('zmidi', default_options: ['features=io,ump,ci'])
executable('app', 'main.c', dependencies: zmidi)
```

Set `revision` to a tag or a commit for a reproducible build. Meson applies the `default_options`
of a subproject only at the first setup: after you change them, run `meson setup --wipe`. The options:

| Option | Default | Use |
|---|---|---|
| `features` | `io` | The parts of the C ABI (the Cargo features below), for example `io,ump,ci,net,net-auth`. |
| `cargo_target` | (host) | The Rust target triple. A cross build needs it, for example `thumbv7em-none-eabihf`. |
| `runtime` | `auto` | `std` for a host, `panic-handler` for bare metal and `wasm32-unknown-unknown`. `auto` selects by `cargo_target`. |
| `tests` | `false` | Builds the C tests (`meson test`) on a host, or the bare-metal link checks in a cross build. |

For bare metal, give the target in your cross file:

```ini
[project options]
cargo_target = 'thumbv7em-none-eabihf'
```

`tests/meson/thumbv7em.ini` is a complete cross file for a Cortex-M4F with `arm-none-eabi-gcc`. A
build of the repository on its own (not as a subproject) also installs the library, the headers
and a `zmidi.pc` file for pkg-config, so another build system can find it with `pkg-config zmidi`.

## Build with Cargo

```sh
# Host (static library; use cdylib for a shared library):
cargo rustc -p zmidi-ffi --lib --crate-type staticlib --profile ffi --features "std,io,ump"

# Bare metal:
cargo rustc -p zmidi-ffi --lib --crate-type staticlib --profile ffi \
    --target thumbv7em-none-eabihf --features "panic-handler,io,ump"
```

The library is `target/ffi/libzmidi.a`, or `target/<triple>/ffi/libzmidi.a` with `--target`. Define the `ZMIDI_<PART>` macro of each part (for
example `-DZMIDI_IO -DZMIDI_UMP`) before you include `zmidi.h`. The `ffi` profile optimizes for size
with LTO, so the linker removes the functions that the program does not call.

## Cargo features

| Feature | Header macro | Description |
|---|---|---|
| `ump` | `ZMIDI_UMP` | Parse and build UMP messages, MIDI 2.0 ↔ 1.0 translation. |
| `io` | `ZMIDI_IO` | MIDI 1.0 byte stream ↔ UMP. |
| `ci` | `ZMIDI_CI` | MIDI-CI with all the categories below. |
| `ci-device` | `ZMIDI_CI_DEVICE` | MIDI-CI Device: Discovery and management only. |
| `ci-initiator` | `ZMIDI_CI_INITIATOR` | The Initiator role. |
| `ci-profiles` | `ZMIDI_CI_PROFILES` | Profile Configuration. |
| `ci-property-exchange` | `ZMIDI_CI_PROPERTY_EXCHANGE` | Property Exchange messages. |
| `ci-process-inquiry` | `ZMIDI_CI_PROCESS_INQUIRY` | Process Inquiry. |
| `file` | `ZMIDI_FILE` | MIDI Clip File reader and writer. |
| `net` | `ZMIDI_NET` | Network MIDI 2.0 Host and Client. |
| `net-auth` | `ZMIDI_NET_AUTH` | Network MIDI 2.0 authentication. Makes the `ZMIDI_NET_*_WORDS` storage sizes larger. |
| `usb` | `ZMIDI_USB` | USB-MIDI data link, the USB descriptor writer and the Group Terminal Block requests. |
| `ble` | `ZMIDI_BLE` | BLE-MIDI packet reader and writer. |
| `io-midi1` | `ZMIDI_IO_MIDI1` | `io` with the MIDI 1.0 Protocol only (no MIDI 2.0 code, a smaller reader). |
| `usb-midi1` | `ZMIDI_USB_MIDI1` | `usb` with Alternate Setting 0 only (USB-MIDI 1.0). |
| `ble-midi1` | `ZMIDI_BLE_MIDI1` | `ble` with the MIDI 1.0 Protocol only (a smaller reader). |
| `messages` | `ZMIDI_MESSAGES` | Typed MIDI 1.0 messages on bytes: a reader with System Exclusive in a caller buffer, and a writer. |
| `channel-mode` | `ZMIDI_CHANNEL_MODE` | MIDI 1.0 receiver mode state: Modes 1 to 4, Basic Channel, Local Control. |
| `controllers` | `ZMIDI_CONTROLLERS` | MIDI 1.0 controller state of one channel: 14-bit pairs, bank and program, Reset All Controllers (RP-015). |
| `parameters` | `ZMIDI_PARAMETERS` | MIDI 1.0 RPN and NRPN receiver of one channel, Data Increment and Decrement (RP-018). |
| `sync` | `ZMIDI_SYNC` | MIDI 1.0 sync receiver: Start, Stop, Continue, Timing Clock, Song Position Pointer, Song Select. |
| `active-sensing` | `ZMIDI_ACTIVE_SENSING` | MIDI 1.0 Active Sensing monitor (330 ms timeout) and sender (270 ms interval). |
| `midi1` | `ZMIDI_MIDI1` | All MIDI 1.0 parts above, with no MIDI 2.0 code. |
| `std` | – | Host builds. |
| `panic-handler` | – | Bare-metal and WebAssembly builds: a panic handler for the library. |

`ZMIDI_CI` also defines the five `ZMIDI_CI_*` macros. `ZMIDI_MIDI1` defines the nine MIDI 1.0 macros, and
`ZMIDI_IO`, `ZMIDI_USB` and `ZMIDI_BLE` also define their `*_MIDI1` macro. Define the macros of the features of
your library build: with neither `io` nor `ble`, the readers of `io-midi1` and `ble-midi1` have a smaller storage size. `ZMIDI_ABI_MAJOR` and `ZMIDI_ABI_MINOR` give
the ABI version of the header.

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
