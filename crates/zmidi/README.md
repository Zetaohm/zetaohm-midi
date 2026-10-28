# zmidi

[![crates.io](https://img.shields.io/crates/v/zmidi.svg)](https://crates.io/crates/zmidi)
[![docs.rs](https://img.shields.io/docsrs/zmidi)](https://docs.rs/zmidi)

A complete MIDI 2.0 implementation in one crate. `no_std`, no allocator, sans-IO.

This is the facade of [zetaohm-midi](../../README.md). It re-exports each protocol crate as a
module:

| Module | Crate | What it does |
|---|---|---|
| `zmidi::ump` | [`zmidi-ump`](../zmidi-ump) | Universal MIDI Packet messages |
| `zmidi::io` | [`zmidi-io`](../zmidi-io) | MIDI 1.0 byte stream, UMP Endpoint, Jitter Reduction |
| `zmidi::ci` | [`zmidi-ci`](../zmidi-ci) | MIDI Capability Inquiry |
| `zmidi::ci_pe` | [`zmidi-ci-pe`](../zmidi-ci-pe) | MIDI-CI Property Exchange |
| `zmidi::file` | [`zmidi-file`](../zmidi-file) | MIDI Clip File |
| `zmidi::net` | [`zmidi-net`](../zmidi-net) | Network MIDI 2.0 (UDP) |
| `zmidi::usb` | [`zmidi-usb`](../zmidi-usb) | USB-MIDI 2.0 and 1.0 |
| `zmidi::ble` | [`zmidi-ble`](../zmidi-ble) | BLE-MIDI |

For the smallest firmware, turn off the default features (`default-features = false`) and select
the parts, or depend on the individual crates and select their features.

## Example

```rust
use zmidi::io::BytesToUmp;
use zmidi::ump::{Group, Ump};

fn main() -> Result<(), Box<dyn core::error::Error>> {
    let mut din_in = BytesToUmp::midi1(Group::new(0)?);
    for byte in [0x90, 0x3C, 0x64] {
        for packet in din_in.push(byte) {
            if let Ump::Midi1ChannelVoice(message) = packet.ump() {
                println!("{message:?}");
            }
        }
    }
    Ok(())
}
```

## Cargo features

| Feature | Default | Description |
|---|---|---|
| `full` | yes | All the parts below that are on by default. |
| `ump` | yes | All UMP message families (`zmidi-ump/full`). The `ump` module is always present. |
| `io`, `ci`, `ci-pe`, `file`, `net`, `usb`, `ble` | yes | The module of the same name, with the default features of its crate. |
| `auth` | no | Network MIDI 2.0 authentication (`zmidi-net/auth`). |
| `zlib` | no | The `zlib+Mcoded7` Property Exchange encoding (`zmidi-ci-pe/zlib`). |

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
