# zmidi-ble-trouble

[![crates.io](https://img.shields.io/crates/v/zmidi-ble-trouble.svg)](https://crates.io/crates/zmidi-ble-trouble)
[![docs.rs](https://img.shields.io/docsrs/zmidi-ble-trouble)](https://docs.rs/zmidi-ble-trouble)

[`trouble-host`](https://crates.io/crates/trouble-host) 0.7 adapter for the
[`zmidi-ble`](../zmidi-ble) BLE-MIDI codec. Async, `no_std`, no allocator.

Part of [zetaohm-midi](../../README.md).

## Features

- `MidiService::add` adds the MIDI Service and the MIDI Data I/O Characteristic to an
  `AttributeTable`.
- `advertising_data` and `scan_response_data` give the advertising data with the MIDI Service UUID
  and the device name.
- `request_link` asks for a connection interval of 7.5 ms to 15 ms and for 251-byte link-layer
  packets.
- `Receiver::next` runs the GATT events of one connection and gives the received UMPs to a
  synchronous callback. `Sender::send` writes UMPs into notifications of at most ATT_MTU − 3 bytes.
  Run the two in separate futures so that a slow notification never stops the GATT events.

## Example

A complete BLE-MIDI peripheral on an ESP32-S3 is in
[`examples/esp32s3-ble-midi`](../../examples/esp32s3-ble-midi). Its GATT server and echo are in
[`src/ble.rs`](../../examples/esp32s3-ble-midi/src/ble.rs), which has no chip-specific code.

## Cargo features

None.

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
