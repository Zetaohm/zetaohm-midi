# zmidi-ble

[![crates.io](https://img.shields.io/crates/v/zmidi-ble.svg)](https://crates.io/crates/zmidi-ble)
[![docs.rs](https://img.shields.io/docsrs/zmidi-ble)](https://docs.rs/zmidi-ble)

BLE-MIDI (MIDI over Bluetooth Low Energy v1.0) packet codec and GATT data, with UMP on the
application side. `no_std`, no allocator, sans-IO.

Part of [zetaohm-midi](../../README.md).

## Features

- `PacketReader`: BLE-MIDI packets to UMPs, each with its 13-bit timestamp. Handles running status,
  SysEx across packets and the timestamp rules of the specification.
- `PacketWriter`: UMPs with a time in milliseconds to packets that fit the ATT_MTU of the connection,
  with optional running status.
- `gatt`: the MIDI Service and MIDI Data I/O Characteristic UUIDs and properties, and the connection
  interval limit.
- No BLE stack dependency. Your BLE host moves the bytes. For
  [`trouble-host`](https://crates.io/crates/trouble-host), use
  [`zmidi-ble-trouble`](../zmidi-ble-trouble).

## Example

```rust
use zmidi_ble::zmidi_ump::{Group, StreamProtocol};
use zmidi_ble::{PacketReader, PacketWriter, TimedUmp};

fn main() -> Result<(), Box<dyn core::error::Error>> {
    // Send: UMPs with a time in ms, written into packets that fit the ATT_MTU.
    let mut writer = PacketWriter::new(23)?;
    let note_on = [0x2090_3C64u32];
    let mut umps = [TimedUmp { millis: 1000, words: &note_on }].into_iter().peekable();
    let mut packet = [0u8; 20];
    let len = writer.write_packet(&mut umps, &mut packet)?;
    // Notify `packet[..len]` on the MIDI Data I/O characteristic with your BLE stack.

    // Receive: give each written packet to the reader; it gives UMPs with their timestamps.
    let mut reader = PacketReader::new(Group::new(0)?, StreamProtocol::Midi1);
    reader.read(&packet[..len], |result| match result {
        Ok(rx) => println!("{:?} at {:?}", rx.ump, rx.timestamp),
        Err(e) => println!("bad packet: {e}"),
    });
    Ok(())
}
```

## Cargo features

None.

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
