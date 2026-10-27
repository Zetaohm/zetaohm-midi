# zmidi-usb-embassy

[![crates.io](https://img.shields.io/crates/v/zmidi-usb-embassy.svg)](https://crates.io/crates/zmidi-usb-embassy)
[![docs.rs](https://img.shields.io/docsrs/zmidi-usb-embassy)](https://docs.rs/zmidi-usb-embassy)

[`embassy-usb`](https://crates.io/crates/embassy-usb) 0.6 adapter for the
[`zmidi-usb`](../zmidi-usb) USB-MIDI 2.0 and 1.0 codecs. Async, `no_std`, no allocator.

Part of [zetaohm-midi](../../README.md).

## Features

- `MidiClass` adds one USB MIDI function to an `embassy_usb::Builder`. It answers the Group Terminal
  Block request and the interface strings, and follows SET_INTERFACE.
- `MidiClass::split` gives a `Sender` and a `Receiver` for two tasks.
- `default_config` gives a working endpoint layout: OUT 1 / IN 1 on Alternate Setting 0, OUT 2 /
  IN 2 on Alternate Setting 1.
- `MidiClass::fixed` takes a function that a build script made with `zmidi_usb::fixed`, with no
  descriptor builder in the firmware.

## Example

An excerpt of [`examples/stm32wb55-usb-midi`](../../examples/stm32wb55-usb-midi), a USB MIDI 2.0
echo on a P-NUCLEO-WB55:

```rust,ignore
use zmidi_usb::{BusSpeed, LinkConfig};
use zmidi_usb_embassy::{default_config, InterfaceNames, MidiClass, State, FUNCTION_MAX};

let mut builder = embassy_usb::Builder::new(
    driver,
    config,
    CONFIG_DESC.init([0; FUNCTION_MAX + 64]),
    BOS_DESC.init([0; 64]),
    &mut [],
    CONTROL.init([0; 256]),
);
let names = InterfaceNames { audio_control: Some("My Synth"), streaming: &STREAMING };
let midi = MidiClass::new(
    &mut builder,
    MIDI_STATE.init(State::new()),
    default_config(BusSpeed::Full).unwrap(),
    names,
    LinkConfig::default(),
)
.unwrap();
let (mut sender, mut receiver) = midi.split();
let mut device = builder.build();

let rx = async {
    let mut buf = [0u8; 64];
    loop {
        receiver.wait_connection().await;
        // The callback runs with the adapter lock held: queue the UMP, do not block.
        while receiver.read_packet(&mut buf, |ump| { /* queue it */ }).await.is_ok() {}
    }
};
let tx = async {
    let mut buf = [0u8; 64];
    loop {
        sender.wait_connection().await;
        let mut umps = next_batch().await; // a Peekable iterator of &[u32], one UMP each
        let size = sender.packet_size();
        while let Ok(n) = sender.write_packet(&mut umps, &mut buf[..size]).await {
            if n == 0 { break; }
        }
    }
};
join3(device.run(), rx, tx).await;
```

## Cargo features

| Feature | Default | Description |
|---|---|---|
| `alt0` | yes | Turns on `zmidi-usb/alt0`: the USB-MIDI 1.0 data path of Alternate Setting 0. About 3.1 KB of flash. |

## Notes

- Each endpoint address is in exactly one Alternate Setting: `embassy-usb` 0.6 disables an endpoint
  that two settings share.
- The endpoint allocation of `embassy-usb` panics when the driver has no free endpoint.

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
