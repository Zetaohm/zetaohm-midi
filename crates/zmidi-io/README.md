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
| `full` | yes | All the features below. |
| `midi2-protocol` | via `full` | MIDI 2.0 Protocol output in `BytesToUmp` (MIDI 1.0 → 2.0 translation), and the zmidi-ump family `channel-voice2`, so that `UmpToBytes` translates a MIDI 2.0 Channel Voice UMP to bytes. Without it and without that family, a MIDI 1.0-only build: `UmpToBytes` skips a MIDI 2.0 UMP (the DIN writer is 576 B of flash instead of 1,276 B on Thumb-2). |
| `messages` | via `full` | `Midi1Reader` and `Midi1Writer`: typed MIDI 1.0 messages on bytes (DEC-022). |
| `channel-mode` | via `full` | `ModeState`: the MIDI 1.0 receive modes, Omni, Mono and Poly, and their actions (M1 pp. 20 to 25). |
| `controllers` | via `full` | `ControllerState`: 14-bit controllers, bank and program, Portamento Control, the Reset All Controllers list of RP-015. |
| `parameters` | via `full` | `ParameterState`: the RPN and NRPN receiver with Data Entry, Increment and Decrement (RP-018). |
| `sync` | via `full` | `SyncReceiver`: Timing Clock, Start, Stop, Continue and Song Position Pointer. |
| `active-sensing` | via `full` | The Active Sensing monitor and sender (M1 p. 30). |
| `mpe` | via `full` | MIDI Polyphonic Expression (M1-100): Zone layout, receiver, note allocator and writers. Needs `parameters`. |
| `gm` | via `full` | `GmReceiver`: the General MIDI receiver of RP-024 and RP-003 (channel kinds, GM reset, GM2 Channel Mode rules, default values, Rhythm Channel table). Needs `channel-mode`, `controllers` and `parameters`. |

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
