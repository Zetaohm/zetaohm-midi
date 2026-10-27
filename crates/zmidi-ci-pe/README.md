# zmidi-ci-pe

[![crates.io](https://img.shields.io/crates/v/zmidi-ci-pe.svg)](https://crates.io/crates/zmidi-ci-pe)
[![docs.rs](https://img.shields.io/docsrs/zmidi-ci-pe)](https://docs.rs/zmidi-ci-pe)

MIDI-CI Property Exchange (M2-103-UM v1.2): Header Data, status codes, encodings, Resources and
Subscriptions. `no_std`, no allocator. The message framing is in [`zmidi-ci`](../zmidi-ci).

Part of [zetaohm-midi](../../README.md).

## Features

- `PeEngine`: the full Property Exchange engine for both roles. Multi-chunk transactions,
  Subscriptions, timeouts and retransmit. Property Data streams one Chunk at a time, so no message
  is collected in memory.
- `one_chunk::respond`: a small Responder for devices that only answer one-chunk Get and Set
  requests, with no transaction state. About 5 KB of flash on a Cortex-M4.
- A streaming JSON reader and writer with no allocation, for Header Data and Property Data.
- ResourceList, `X-` Resource, link and partial Set (JSON Pointer) helpers.
- Mcoded7 encoder and decoder. With the feature `zlib`: a streaming `zlib+Mcoded7` decoder (the
  window is a buffer that you give, up to 32 KiB) and a writer that stores the data uncompressed.

## Example

A one-chunk Responder with one custom Resource:

```rust
use zmidi_ci::{CiDevice, Event};
use zmidi_ci_pe::one_chunk::{respond, Resources};
use zmidi_ci_pe::Status;

const RESOURCE_LIST: &[u8] = br#"[{"resource":"DeviceInfo"},{"resource":"X-Volume","canSet":"full"}]"#;

struct App {
    volume: [u8; 8],
    volume_len: usize,
}

impl Resources for App {
    fn get(&mut self, resource: &[u8], _res_id: Option<&[u8]>, out: &mut [u8]) -> Result<usize, Status> {
        match resource {
            b"X-Volume" => {
                let n = self.volume_len;
                out.get_mut(..n).ok_or(Status::NOT_FOUND)?.copy_from_slice(&self.volume[..n]);
                Ok(n)
            }
            _ => Err(Status::NOT_FOUND),
        }
    }

    fn set(&mut self, resource: &[u8], _res_id: Option<&[u8]>, data: &[u8]) -> Result<(), Status> {
        if resource != b"X-Volume" || data.len() > self.volume.len() {
            return Err(Status::BAD_DATA);
        }
        self.volume[..data.len()].copy_from_slice(data);
        self.volume_len = data.len();
        Ok(())
    }
}

/// Call this with each event that `CiDevice::receive` gives.
fn on_event(device: &mut CiDevice<'_, 4, 2, 4>, event: &Event<'_>, app: &mut App) {
    let (mut body, mut out) = ([0u8; 256], [0u8; 512]);
    respond(device, event, RESOURCE_LIST, app, &mut body, &mut out, |reply| {
        // Send `reply` (one MIDI-CI message) as SysEx on the Group of the Device.
        let _ = reply;
    });
}
```

## Cargo features

| Feature | Default | Description |
|---|---|---|
| `initiator` | yes | The Initiator role of `PeEngine` (send Get, Set and Subscription requests). |
| `zlib` | no | The `zlib+Mcoded7` encoding. Needs up to about 34 KB of RAM with a 32 KiB window. |

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
