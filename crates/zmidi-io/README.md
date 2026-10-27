# zmidi-io

[![crates.io](https://img.shields.io/crates/v/zmidi-io.svg)](https://crates.io/crates/zmidi-io)
[![docs.rs](https://img.shields.io/docsrs/zmidi-io)](https://docs.rs/zmidi-io)

MIDI 2.0 I/O building blocks: MIDI 1.0 byte stream ↔ UMP, UMP Endpoint behavior, Jitter Reduction
timestamps and Group routing. `no_std`, no allocator, no I/O.

Part of [zetaohm-midi](../../README.md).

## Features

- `BytesToUmp`: MIDI 1.0 bytes (DIN, UART, USB-MIDI 1.0) to UMP, with running status, System Real
  Time between data bytes and SysEx. Output in MIDI 1.0 or MIDI 2.0 Protocol.
- `UmpToBytes`: UMP to MIDI 1.0 bytes, with optional running status. A value form and a callback form.
- `Midi1Scheduler`: send MIDI 1.0 bytes at the time of their Jitter Reduction timestamp.
- `Endpoint`: answers UMP Stream discovery (Endpoint Info, Device Identity, names, Function Blocks)
  and Stream Configuration requests.
- `JrSender` and `JrReceiver`: Jitter Reduction Clock and Timestamp handling.
- `route` and `blocks_for_group`: which Function Block and MIDI-CI destination a message is for.
- `SendOrder`: the send order rules for Mixed Data Sets and other multi-packet messages.

## Example

```rust
use zmidi_io::{BytesToUmp, UmpToBytes};
use zmidi_ump::Group;

fn main() -> Result<(), Box<dyn core::error::Error>> {
    // MIDI 1.0 bytes (for example from a DIN UART) to UMP, one byte at a time.
    let mut din_in = BytesToUmp::midi1(Group::new(0)?);
    let mut received = [0u32; 2];
    for byte in [0x90, 0x3C, 0x64] {
        for packet in din_in.push(byte) {
            received[..packet.words().len()].copy_from_slice(packet.words());
        }
    }
    assert_eq!(received[0], 0x2090_3C64); // MIDI 1.0 Channel Voice UMP

    // UMP back to MIDI 1.0 bytes, with running status.
    let mut din_out = UmpToBytes::new(true);
    let mut bytes = [0u8; 16];
    let n = din_out.write(&[0x2090_3C64], &mut bytes)?;
    assert_eq!(&bytes[..n], &[0x90, 0x3C, 0x64]);
    Ok(())
}
```

## Cargo features

| Feature | Default | Description |
|---|---|---|
| `full` | yes | Same as `midi2-protocol`. |
| `midi2-protocol` | via `full` | MIDI 2.0 Protocol output in `BytesToUmp` (MIDI 1.0 → 2.0 translation). |

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
