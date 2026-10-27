# zmidi-usb-device

[![crates.io](https://img.shields.io/crates/v/zmidi-usb-device.svg)](https://crates.io/crates/zmidi-usb-device)
[![docs.rs](https://img.shields.io/docsrs/zmidi-usb-device)](https://docs.rs/zmidi-usb-device)

[`usb-device`](https://crates.io/crates/usb-device) 0.3 adapter for the
[`zmidi-usb`](../zmidi-usb) USB-MIDI 2.0 and 1.0 codecs. `no_std`, no allocator.

Part of [zetaohm-midi](../../README.md).

## Features

- `MidiClass` is a `UsbClass` for one USB MIDI function: descriptors, the Group Terminal Block
  request, SET_INTERFACE and the data endpoints.
- Works with any HAL that gives a `usb-device` 0.3 bus, for example `stm32-usbd`,
  `synopsys-usb-otg` or `mk20dx-hal`. Poll it from the main loop or from the USB interrupt.
- `MidiClass::fixed` takes a function that a build script made with `zmidi_usb::fixed`: about 10 KB
  less flash than the run-time builder.

## Example

An excerpt of [`examples/mk20dx-usb-midi`](../../examples/mk20dx-usb-midi), a USB MIDI 2.0 echo on
a Teensy 3.2:

```rust,ignore
use zmidi_usb::device::FunctionConfig;
use zmidi_usb::{BusSpeed, LinkConfig};
use zmidi_usb_device::{InterfaceNames, MidiClass};

static STREAMING: [Option<&str>; 2] = [Some("MIDI 1.0"), Some("MIDI 2.0")];

// Make the class before the device: the allocator gives interfaces and endpoints in order.
let cfg = FunctionConfig::new(0, 0, BusSpeed::Full).unwrap();
let names = InterfaceNames { audio_control: Some("My Synth"), streaming: &STREAMING };
let mut midi = MidiClass::new(&alloc, cfg, names, LinkConfig::default()).unwrap();
let mut dev = UsbDeviceBuilder::new(&alloc, UsbVidPid(0x1209, 0x0001))
    .max_packet_size_0(64)
    .unwrap()
    .build();

// In the USB interrupt or the main loop:
dev.poll(&mut [&mut midi]);

let mut buf = [0u8; 64];
while midi.read(&mut buf, |ump| { /* handle each received UMP */ }).is_ok() {}

let mut out = [0u8; 64];
let mut umps = outgoing.iter().peekable(); // items: &[u32], one UMP each
midi.write(&mut umps, &mut out).ok();
```

## Cargo features

| Feature | Default | Description |
|---|---|---|
| `alt0` | yes | Turns on `zmidi-usb/alt0`: the USB-MIDI 1.0 data path of Alternate Setting 0. About 3.3 KB of flash. |

## Notes

- The crate turns on the `usb-device` feature `control-buffer-256`. A Configuration descriptor
  longer than 256 bytes does not fit.
- One endpoint address can serve more than one Alternate Setting when its type and size are the same.

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
