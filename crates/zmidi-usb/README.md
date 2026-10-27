# zmidi-usb

[![crates.io](https://img.shields.io/crates/v/zmidi-usb.svg)](https://crates.io/crates/zmidi-usb)
[![docs.rs](https://img.shields.io/docsrs/zmidi-usb)](https://docs.rs/zmidi-usb)

USB-MIDI 2.0 and 1.0 (USB Device Class Definition for MIDI Devices): descriptors, packets and the
data link, with no USB stack. `no_std`, no allocator, sans-IO.

Part of [zetaohm-midi](../../README.md).

## Features

- Class descriptors for a USB MIDI function: Alternate Setting 0 (USB-MIDI 1.0 Jacks and Event
  Packets) and USB MIDI 2.0 settings with Group Terminal Blocks. Built at run time from a
  `FunctionConfig`, or at build time as constant bytes (`fixed`) so the firmware links no
  descriptor builder.
- `MidiLink`: reads and writes the bulk packets of the selected Alternate Setting. USB-MIDI 1.0 Event
  Packets and USB MIDI 2.0 UMP packets both come out as UMPs.
- The Group Terminal Block request (`handle_gtb_setup`).
- Host side: `read_configuration` reads the MIDI functions of a Configuration descriptor.
- No USB stack dependency. Your USB stack moves the bytes. Ready-made adapters:
  [`zmidi-usb-device`](../zmidi-usb-device) for `usb-device` and
  [`zmidi-usb-embassy`](../zmidi-usb-embassy) for `embassy-usb`.

## Example

```rust
use zmidi_usb::device::write_function;
use zmidi_usb::{BusSpeed, FunctionConfig, GroupMask, LinkConfig, LinkEvent, MidiLink, SettingNumber};

fn main() -> Result<(), Box<dyn core::error::Error>> {
    // The descriptors of one USB MIDI function: AudioControl interface 0, MIDI Streaming
    // interface 1, Alternate Setting 0 (USB-MIDI 1.0) and 1 (USB MIDI 2.0), full speed.
    let function: FunctionConfig = FunctionConfig::new(0, 1, BusSpeed::Full)?;
    let mut descriptors = [0u8; 256];
    let len = write_function(&function, &mut descriptors)?;
    // Give `descriptors[..len]` to your USB stack as part of the Configuration descriptor.
    println!("{len} bytes of class descriptors");

    // The data link: report what the host selects, then read and write the bulk packets.
    let mut link: MidiLink = MidiLink::new(LinkConfig::default())?;
    link.report(LinkEvent::AlternateSetting(SettingNumber(1)));
    link.report_setting(SettingNumber(1), GroupMask::ALL, 64)?;

    // One bulk OUT packet from the host at Alternate Setting 1: a MIDI 2.0 Note On UMP.
    let packet = [0x00, 0x3C, 0x90, 0x40, 0x00, 0x00, 0x00, 0xC0];
    link.read_with(&packet, |item| match item {
        Ok(rx) => println!("{:?}", rx.ump()),
        Err(e) => println!("{e}"),
    });

    // Write UMPs into one bulk IN packet.
    let note_off: &[u32] = &[0x4080_3C00, 0];
    let mut umps = [note_off].into_iter().peekable();
    let mut out = [0u8; 64];
    let n = link.write_packet(&mut umps, &mut out)?;
    println!("IN packet: {:02X?}", &out[..n]);
    Ok(())
}
```

## Cargo features

| Feature | Default | Description |
|---|---|---|
| `alt0` | yes | The USB-MIDI 1.0 data path of Alternate Setting 0. Without it, Alternate Setting 0 has no Jack and no endpoint: about 3 KB less flash, but a host with only a USB-MIDI 1.0 driver shows no port. |

A crate that depends on `zmidi-usb` with `default-features = false` must name `features = ["alt0"]`
to keep the USB-MIDI 1.0 data path.

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
