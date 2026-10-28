# zmidi-ffi

The C ABI of [zetaohm-midi](../../README.md): one header, [`zmidi.h`](include/zmidi.h), for C, C++
and other languages. `no_std`, no allocator, no callbacks.

## Features

- All `zmidi_` functions return a `ZmidiStatus`. No function panics on input from the program.
- The program owns the storage of each object. `zmidi_<object>_size()` gives the size, and the
  header gives a `Zmidi<Object>` type of that size for static or stack storage. The library never
  calls `malloc`.
- Parts: UMP, MIDI 1.0 byte stream, MIDI-CI (each category separately), MIDI Clip File, Network MIDI
  2.0 (with optional authentication), USB-MIDI and BLE-MIDI. A build contains only the parts that
  you select, and the header shows only their declarations.
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
| `std` | – | Host builds. |
| `panic-handler` | – | Bare-metal and WebAssembly builds: a panic handler for the library. |

`ZMIDI_CI` also defines the five `ZMIDI_CI_*` macros. `ZMIDI_ABI_MAJOR` and `ZMIDI_ABI_MINOR` give
the ABI version of the header.

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
