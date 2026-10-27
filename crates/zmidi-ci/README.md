# zmidi-ci

[![crates.io](https://img.shields.io/crates/v/zmidi-ci.svg)](https://crates.io/crates/zmidi-ci)
[![docs.rs](https://img.shields.io/docsrs/zmidi-ci)](https://docs.rs/zmidi-ci)

MIDI Capability Inquiry (MIDI-CI, M2-101-UM v1.2.1), with the Profile rules of M2-102-U v1.1.
`no_std`, no allocator, sans-IO.

Part of [zetaohm-midi](../../README.md).

## Features

- Reader and writer for every MIDI-CI message: management (Discovery, Endpoint Info, Invalidate MUID,
  ACK, NAK), Profile Configuration, Property Exchange framing and Process Inquiry.
- `CiDevice`: a MIDI-CI Device that keeps its MUID, its peers and its Profiles, answers inquiries
  automatically and starts inquiries as an Initiator.
- Readers take one complete MIDI-CI SysEx message as a byte slice, with or without `F0`/`F7`.
  Writers fill your buffer. You give the time and the random numbers.
- Peer, Profile and request table sizes are const generics.
- The meaning of Property Exchange data (JSON headers, Mcoded7, Resources) is in
  [`zmidi-ci-pe`](../zmidi-ci-pe).

## Example

A Device that answers a Discovery message:

```rust
use zmidi_ci::{
    Categories, CiDevice, DeviceConfig, DeviceIdentity, Discovery, Framing, ManufacturerId, Muid,
    PiCapabilities,
};

// Up to 4 peers, 2 Profiles, 4 open PE requests: fixed sizes, no allocation.
type Device = CiDevice<'static, 4, 2, 4>;

fn main() -> Result<(), Box<dyn core::error::Error>> {
    let identity = DeviceIdentity {
        manufacturer: ManufacturerId::OneByte(0x7D), // 0x7D: prototyping, non-commercial use
        family: 1,
        model: 1,
        revision: [0, 0, 0, 1],
    };
    let config = DeviceConfig {
        identity,
        categories: Categories::default(),
        max_sysex_size: 512,
        function_block: Some(0),
        first_group: 0,
        group_count: 1,
        channels_in_use: 0x0001,
        product_instance_id: Some("SN-0001"),
        framing: Framing::Bare,
        process_inquiry: PiCapabilities { midi_message_report: false },
    };
    let mut random = || 0x0123_4567; // use a real random source
    let mut device = Device::new(config, &mut random)?;

    // A Discovery message from another Device (here made with the message writer).
    let mut msg = [0u8; 64];
    let discovery = Discovery {
        identity,
        categories: Categories::default(),
        max_sysex_size: 256,
        output_path_id: None,
    };
    let len = discovery.write(Muid::new(0x0111_1111)?, Framing::Bare, &mut msg)?;

    // Give each received MIDI-CI message to the Device; send each reply that it writes.
    let mut received = device.receive(0, &msg[..len], 0, &mut random);
    println!("event: {:?}", received.event());
    let mut reply = [0u8; Device::MAX_REPLY_LEN];
    while let Some(result) = received.write_next(&mut reply) {
        let (group, n) = result?;
        println!("reply on group {group}: {:02X?}", &reply[..n]);
    }
    Ok(())
}
```

## Cargo features

| Feature | Default | Description |
|---|---|---|
| `full` | yes | All the features below. |
| `initiator` | via `full` | The Initiator role: start Discovery, inquiries and Profile changes. Without it the Device only responds. |
| `profiles` | via `full` | Profile Configuration. |
| `property-exchange` | via `full` | Property Exchange message framing. |
| `process-inquiry` | via `full` | Process Inquiry and MIDI Message Report. |

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
