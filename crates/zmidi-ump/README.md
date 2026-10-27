# zmidi-ump

[![crates.io](https://img.shields.io/crates/v/zmidi-ump.svg)](https://crates.io/crates/zmidi-ump)
[![docs.rs](https://img.shields.io/docsrs/zmidi-ump)](https://docs.rs/zmidi-ump)

Universal MIDI Packet (UMP) messages for MIDI 2.0, as defined in M2-104-UM v1.1.2. `no_std`, no
allocator.

Part of [zetaohm-midi](../../README.md).

## Features

- Every UMP message family: Utility, System, MIDI 1.0 and MIDI 2.0 Channel Voice, SysEx7, SysEx8 and
  Mixed Data Set, Flex Data, UMP Stream.
- Each message is a typed value with its fields in specification order. Constructors that take your
  values check them and return an error; nothing panics on input.
- `Ump::parse` reads any packet from a `&[u32]`. Packets of an unknown or disabled Message Type come
  back as `Ump::Unknown` with their raw words.
- Reassemblers and packet iterators for SysEx7, SysEx8, Flex Data text and UMP Stream text, with
  fixed-capacity buffers.
- MIDI 1.0 ↔ MIDI 2.0 Protocol translation (Appendix D) and the value scaling of M2-115-U.

## Example

```rust
use zmidi_ump::{
    Channel, Group, Midi2ChannelVoiceMessage, Midi2NoteOn, NoteAttribute, Ump, UmpWords,
};

fn main() -> Result<(), zmidi_ump::UmpError> {
    // Build a MIDI 2.0 Note On: Group 1, Channel 1, note 60, 16-bit velocity.
    let note_on = Midi2NoteOn::try_new(
        Group::new(0)?,
        Channel::new(0)?,
        60,
        0xC000,
        NoteAttribute::None,
    )?;

    // Write it into a caller buffer: no allocation.
    let mut words = [0u32; 4];
    let len = note_on.to_words(&mut words)?;
    assert_eq!(&words[..len], &[0x4090_3C00, 0xC000_0000]);

    // Parse any UMP from a word slice.
    let (ump, used) = Ump::parse(&words)?;
    assert_eq!(used, 2);
    if let Ump::Midi2ChannelVoice(Midi2ChannelVoiceMessage::NoteOn(m)) = ump {
        assert_eq!(m.note(), 60);
        assert_eq!(m.velocity(), 0xC000);
    }
    Ok(())
}
```

## Cargo features

| Feature | Default | Description |
|---|---|---|
| `full` | yes | All message families below. |
| `utility` | via `full` | Message Type 0x0: NOOP, JR Clock and Timestamp, Delta Clockstamps. |
| `system-common` | via `full` | Message Type 0x1: System Common and System Real Time. |
| `channel-voice1` | via `full` | Message Type 0x2: MIDI 1.0 Channel Voice. |
| `channel-voice2` | via `full` | Message Type 0x4: MIDI 2.0 Channel Voice. |
| `sysex7` | via `full` | Message Type 0x3: System Exclusive (7-bit). |
| `data128` | via `full` | Message Type 0x5: System Exclusive 8 and Mixed Data Set. |
| `flex-data` | via `full` | Message Type 0xD: Flex Data. |
| `ump-stream` | via `full` | Message Type 0xF: UMP Stream. |
| `alloc` | no | Convenience functions that allocate. |

Turn off the families that you do not use to make `Ump::parse` smaller: with only `channel-voice2`
and `system-common`, it is about half the flash.

```toml
zmidi-ump = { version = "0.1", default-features = false, features = ["channel-voice2", "system-common"] }
```

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
