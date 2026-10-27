# zetaohm-midi

[![crates.io](https://img.shields.io/crates/v/zmidi.svg)](https://crates.io/crates/zmidi)
[![docs.rs](https://img.shields.io/docsrs/zmidi)](https://docs.rs/zmidi)
[![CI](https://github.com/zetaohm/zetaohm-midi/actions/workflows/ci.yml/badge.svg)](https://github.com/zetaohm/zetaohm-midi/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A complete MIDI 2.0 implementation in Rust for microcontrollers first, and for desktops, servers and
browsers too.

- **`no_std` and no allocator.** Every crate builds for bare-metal targets. Multi-packet data is read
  through iterators and written into buffers that you give; capacities are const generics.
- **Sans-IO.** The protocol code takes bytes, time and random numbers from you and gives bytes back.
  It owns no socket, no USB peripheral and no BLE radio, so it runs the same on an RTOS, under
  `embassy`, in a bare `loop {}` or on a desktop.
- **Spec first.** The MIDI Association specifications are the reference. The code cites the section,
  table or figure for each rule, and the tests use the specification examples as their vectors.
- **Small.** Cargo features remove what you do not use. Measured flash and RAM for each job are in
  [`docs/sizes.md`](docs/sizes.md).
- **Many languages.** A C ABI (`zmidi.h`) and packages for Python, TypeScript (WASM) and Dart.

> **Status: pre-release.** The crates and packages are not yet published. The 0.x API can change
> in any release. See [Status](#status) for what is tested on hardware.

## Contents

- [What it covers](#what-it-covers)
- [Crates](#crates)
- [Quick start](#quick-start)
- [Other languages](#other-languages)
- [Example firmware](#example-firmware)
- [Size](#size)
- [Status](#status)
- [Building and testing](#building-and-testing)
- [Contributing](#contributing)
- [License](#license)

## What it covers

| Specification | Version | Crate |
|---|---|---|
| Universal MIDI Packet (UMP) Format and MIDI 2.0 Protocol (M2-104-UM) | 1.1.2 | [`zmidi-ump`](crates/zmidi-ump) |
| MIDI 2.0 Bit Scaling and Resolution (M2-115-U) | 1.0.2 | [`zmidi-ump`](crates/zmidi-ump) |
| MIDI 1.0 Detailed Specification (byte stream) | 4.2.1 | [`zmidi-io`](crates/zmidi-io) |
| MIDI Capability Inquiry (MIDI-CI, M2-101-UM) | 1.2.1 | [`zmidi-ci`](crates/zmidi-ci) |
| Common Rules for MIDI-CI Profiles (M2-102-U) | 1.1 | [`zmidi-ci`](crates/zmidi-ci) |
| Common Rules for MIDI-CI Property Exchange (M2-103-UM) | 1.2 | [`zmidi-ci-pe`](crates/zmidi-ci-pe) |
| MIDI Clip File (M2-116-U) | 1.0 | [`zmidi-file`](crates/zmidi-file) |
| Standard MIDI Files 1.0 (RP-001) | 1.0 | [`zmidi-file`](crates/zmidi-file) |
| Network MIDI 2.0 (UDP) (M2-124-UM) | 1.0.1 | [`zmidi-net`](crates/zmidi-net) |
| USB Device Class Definition for MIDI Devices | 2.0 and 1.0 | [`zmidi-usb`](crates/zmidi-usb) |
| Specification for MIDI over Bluetooth Low Energy (BLE-MIDI) | 1.0 | [`zmidi-ble`](crates/zmidi-ble) |

Where a specification is ambiguous or contradicts itself, the doc comment of the rule gives the chosen
behavior and the section of the specification. A public register of these ambiguities will follow.

## Crates

Each crate has its own README with an example and its Cargo features.

**Protocol crates** — `no_std`, no alloc, no I/O:

| Crate | What it does |
|---|---|
| [`midi`](crates/zmidi) | Facade: re-exports all the protocol crates below under one name. |
| [`zmidi-ump`](crates/zmidi-ump) | Every UMP message as a typed value; parse, write, MIDI 1.0 ↔ 2.0 translation. |
| [`zmidi-io`](crates/zmidi-io) | MIDI 1.0 byte stream ↔ UMP, UMP Endpoint replies, Jitter Reduction, Group routing. |
| [`zmidi-ci`](crates/zmidi-ci) | MIDI-CI Device: Discovery, Profiles, Process Inquiry, Property Exchange framing. |
| [`zmidi-ci-pe`](crates/zmidi-ci-pe) | Property Exchange: Header Data, JSON, Mcoded7, zlib, Resources, Subscriptions. |
| [`zmidi-file`](crates/zmidi-file) | MIDI Clip File reader and writer. |
| [`zmidi-net`](crates/zmidi-net) | Network MIDI 2.0 Host and Client: sessions, FEC, retransmit, DNS-SD records, authentication. |
| [`zmidi-usb`](crates/zmidi-usb) | USB-MIDI 2.0 and 1.0 descriptors, packets and the data link, with no USB stack. |
| [`zmidi-ble`](crates/zmidi-ble) | BLE-MIDI packet reader and writer and GATT data, with no BLE stack. |

**Adapters** — connect a protocol crate to a driver stack:

| Crate | Stack |
|---|---|
| [`zmidi-usb-device`](crates/zmidi-usb-device) | [`usb-device`](https://crates.io/crates/usb-device) 0.3 (`UsbClass`, polled or interrupt-driven) |
| [`zmidi-usb-embassy`](crates/zmidi-usb-embassy) | [`embassy-usb`](https://crates.io/crates/embassy-usb) 0.6 (async) |
| [`zmidi-ble-trouble`](crates/zmidi-ble-trouble) | [`trouble-host`](https://crates.io/crates/trouble-host) 0.7 (async BLE host) |

**Other languages:**

| Crate or package | What it does |
|---|---|
| [`zmidi-ffi`](crates/zmidi-ffi) | C ABI with one header, `zmidi.h`, and a CMake package. |
| [`zmidi-py`](crates/zmidi-py) | The native extension of the Python package (PyO3). |
| [`bindings/python`](bindings/python) | Python package `zetaohm-midi`. |
| [`bindings/ts`](bindings/ts) | TypeScript package `zetaohm-midi` over the WASM build of the C ABI. |
| [`bindings/dart`](bindings/dart) | Dart package `zetaohm_midi` over `dart:ffi`. |

## Quick start

Until the crates are on crates.io, use a git dependency:

```toml
[dependencies]
midi = { git = "https://github.com/zetaohm/zetaohm-midi" }
# or only the parts you need:
zmidi-ump = { git = "https://github.com/zetaohm/zetaohm-midi" }
```

Build and parse a MIDI 2.0 Note On:

```rust
use zmidi::ump::{
    Channel, Group, Midi2ChannelVoiceMessage, Midi2NoteOn, NoteAttribute, Ump, UmpWords,
};

fn main() -> Result<(), zmidi::ump::UmpError> {
    // Group 1, Channel 1, note 60, 16-bit velocity.
    let note_on = Midi2NoteOn::try_new(
        Group::new(0)?,
        Channel::new(0)?,
        60,
        0xC000,
        NoteAttribute::None,
    )?;

    // Write it into a buffer that you own.
    let mut words = [0u32; 4];
    let len = note_on.to_words(&mut words)?;
    assert_eq!(&words[..len], &[0x4090_3C00, 0xC000_0000]);

    // Parse any UMP from a word slice.
    let (ump, _used) = Ump::parse(&words)?;
    if let Ump::Midi2ChannelVoice(Midi2ChannelVoiceMessage::NoteOn(m)) = ump {
        assert_eq!(m.note(), 60);
    }
    Ok(())
}
```

Turn a MIDI 1.0 byte stream (a DIN port, a UART) into UMPs and back:

```rust
use zmidi::io::{BytesToUmp, UmpToBytes};
use zmidi::ump::Group;

fn main() -> Result<(), Box<dyn core::error::Error>> {
    let mut din_in = BytesToUmp::midi1(Group::new(0)?);
    for byte in [0x90, 0x3C, 0x64] {
        for packet in din_in.push(byte) {
            println!("UMP {:08X?}", packet.words()); // [20903C64]
        }
    }

    let mut din_out = UmpToBytes::new(true); // with running status
    let mut bytes = [0u8; 16];
    let n = din_out.write(&[0x2090_3C64], &mut bytes)?;
    assert_eq!(&bytes[..n], &[0x90, 0x3C, 0x64]);
    Ok(())
}
```

More examples are in the README of each crate.

## Other languages

All the bindings use the same Rust code.

**C and C++** — add the CMake package and select the parts you need:

```cmake
set(ZMIDI_FEATURES io ump)
add_subdirectory(path/to/zetaohm-midi/crates/zmidi-ffi zmidi)
target_link_libraries(app PRIVATE zmidi::zmidi)
```

**Python**

```python
import zetaohm_midi as zm

zm.BytesToUmp(group=0, protocol=2).feed(bytes([0x90, 0x3C, 0x64]))
# [[0x40903C00, 0xC9240000]]  (MIDI 2.0 Note On, velocity scaled to 16 bits)
```

**TypeScript**

```ts
import { BytesToUmp, init } from "zetaohm-midi";

await init(); // loads the WASM module once
new BytesToUmp(0, 2).feed(new Uint8Array([0x90, 0x3c, 0x64]));
```

**Dart**

```dart
import 'package:zetaohm_midi/zetaohm_midi.dart';

BytesToUmp(protocol: 2).feed([0x90, 0x3C, 0x64]);
```

See the README of each package for the build and the API.

## Example firmware

Complete programs are in [`examples/`](examples). They are not workspace members, so each one has its
own toolchain and lock file.

| Example | Board | What it shows |
|---|---|---|
| [`mk20dx-usb-midi`](examples/mk20dx-usb-midi) | Teensy 3.2 (MK20DX256) | USB MIDI 2.0 echo with `zmidi-usb-device`, from an interrupt handler |
| [`stm32wb55-usb-midi`](examples/stm32wb55-usb-midi) | P-NUCLEO-WB55 | USB MIDI 2.0 echo with `zmidi-usb-embassy` |
| [`esp32s3-usb-midi`](examples/esp32s3-usb-midi) | ESP32-S3 | USB MIDI 2.0 echo with `zmidi-usb-embassy` on the USB OTG peripheral |
| [`esp32s3-ble-midi`](examples/esp32s3-ble-midi) | ESP32-S3 | BLE-MIDI echo peripheral with `zmidi-ble-trouble` |
| [`esp32s3-net-echo`](examples/esp32s3-net-echo) | ESP32-S3 | Network MIDI 2.0 Host over Wi-Fi, with mDNS announcement |
| [`net-echo`](examples/net-echo) | Desktop | Network MIDI 2.0 Host and Client on `std` UDP sockets, with DNS-SD |

The ESP32-S3 examples need the Espressif Rust toolchain (`espup install`).

## Size

Flash and static RAM on a Cortex-M4 (`thumbv7em-none-eabi`, `opt-level = "s"`, LTO), compared with
the smallest other C or Rust library that does the same job:

| Job | Flash (B) | Other (B) | RAM (B) | Other (B) | Other library |
|---|---:|---:|---:|---:|---|
| UMP decode, selected types | 756 | 808 | 0 | – | Rust `midi2` 0.11.1 |
| MIDI 2.0 → 1.0 translation, word API | 488 | 680 | 0 | 40 | AM_MIDI2.0Lib |
| MIDI 1.0 bytes → UMP | 676 | 852 | 24 | 52 | midi2 C99 |
| UMP → MIDI 1.0 bytes | 936 | 1,160 | 4 | 104 | AM_MIDI2.0Lib |
| USB MIDI 2.0 class on `usb-device`, no Alt 0 data path | 4,852 | 3,439 | 364 | 316 | TinyUSB `midi2_device` |
| USB MIDI 2.0 class on `usb-device`, with the USB-MIDI 1.0 fallback | 8,020 | 3,556 | 464 | 666 | `tusb_ump` |
| MIDI-CI Device, responder only, no category | 6,844 | – | 680 | – | – |
| MIDI-CI responder with Profiles, Process Inquiry and PE Get/Set | 14,948 | 4,615 | 1,472 | 736 | midi2 C99 |
| BLE-MIDI packet reader, word form | 3,508 | – | 168 | – | – |
| The UMP, DIN, USB, CI and PE jobs of `docs/sizes.md` in one firmware | 62,536 | – | 5,284 | – | – |

The full table, the method and the stack use are in [`docs/sizes.md`](docs/sizes.md). CI checks each
number against a budget. [`docs/c-comparison.md`](docs/c-comparison.md) compares each job with every C MIDI 2.0
library that does the same work, with the notes on the feature match (the MIDI-CI responder also checks the
destination MUID, the Device ID and the Message Version, and reads the PE header as JSON).

## Status

- All the specifications in the table above are implemented, with tests that use the specification
  examples and property tests over the full input range.
- Tested on hardware: USB MIDI 2.0 on a Teensy 3.2 with macOS; BLE-MIDI and Network MIDI 2.0 on an
  ESP32-S3.
- Not yet tested: interoperability with other MIDI-CI, Network MIDI 2.0 and BLE-MIDI
  implementations; USB on Windows and Linux hosts.
- The C ABI does not yet expose the full Property Exchange engine.

## Building and testing

```sh
cargo build
cargo test
cargo clippy

# Bare-metal check of one crate:
cargo check -p zmidi-ump --lib --no-default-features --target thumbv7em-none-eabihf

# Adapters and example firmware:
sh scripts/adapters-check.sh
sh scripts/examples-check.sh

# Feature combinations (needs cargo-hack and cargo-nextest):
sh scripts/feature-matrix.sh
```

The Python extension `zmidi-py` is not a default workspace member, because it needs a Python
interpreter. Build it through [`bindings/python`](bindings/python).

## Contributing

Issues and pull requests are welcome. Before you send a change:

- Run `cargo fmt`, `cargo clippy` and `cargo test`.
- Keep the crates `no_std` with no `alloc` in the core API.
- Cite the specification section, table or figure in the doc comment of each rule, and take test
  vectors from the specification first.
- If you find an ambiguity in a specification, open an issue with the document, the section, the page and
  your reading of it.

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.

Products that ship this library must include the copyright notice and the license text, for example in their
documentation or their third-party notices.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this work by you
shall be licensed under the MIT license, without any additional terms or conditions.

This project is not affiliated with or endorsed by The MIDI Association or AMEI.
