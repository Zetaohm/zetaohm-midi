# zmidi-file

[![crates.io](https://img.shields.io/crates/v/zmidi-file.svg)](https://crates.io/crates/zmidi-file)
[![docs.rs](https://img.shields.io/docsrs/zmidi-file)](https://docs.rs/zmidi-file)

MIDI Clip File (M2-116-U v1.0) and Standard MIDI Files 1.0 (RP-001) reader and writer. `no_std`, no allocator.

Part of [zetaohm-midi](../../README.md).

## Features

- `ClipReader`: an iterator over the messages of a clip in a byte slice, with absolute tick times.
  It checks the file structure and the order of the configuration messages.
- `ClipWriter`: writes a clip into your buffer and computes the Delta Clockstamps from absolute
  ticks. It checks that tempo and time signature changes fall where the specification permits.

- `SmfReader` and `TrackReader`: Standard MIDI Files formats 0, 1 and 2 from a byte slice: the header, the
  tracks in file order (unknown chunks are skipped), and each event with its delta-time, absolute ticks and
  running status.
- `SmfWriter`: writes a Standard MIDI File into your buffer, with running status (on by default).
- Feature `full` (on by default): the Clip reader decodes every UMP message family. With it off, it decodes
  only the families of its own rules and gives the others as `Ump::Unknown`.

## Example

```rust
use zmidi_file::{ClipConfig, ClipReader, ClipWriter};
use zmidi_ump::Group;

fn main() -> Result<(), Box<dyn core::error::Error>> {
    // Write a clip into a caller buffer.
    let mut buf = [0u8; 256];
    let config = ClipConfig {
        ticks_per_quarter_note: 96,
        profiles: &[],
        tempo: None,
        time_signature: None,
        group: Group::new(0)?,
    };
    let mut clip = ClipWriter::new(&mut buf, &config)?;
    clip.start(0, None, None)?;
    clip.event(0, &[0x4090_3C00, 0xC000_0000])?; // Note On
    clip.event(96, &[0x4080_3C00, 0x0000_0000])?; // Note Off, one quarter note later
    let len = clip.finish(96)?;

    // Read it back: an iterator over the borrowed bytes.
    for event in ClipReader::new(&buf[..len])? {
        let event = event?;
        println!("{:>4} {:?}", event.ticks, event.ump);
    }
    Ok(())
}
```

## Cargo features

None.

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
