# zmidi-net

[![crates.io](https://img.shields.io/crates/v/zmidi-net.svg)](https://crates.io/crates/zmidi-net)
[![docs.rs](https://img.shields.io/docsrs/zmidi-net)](https://docs.rs/zmidi-net)

Network MIDI 2.0 (UDP), M2-124-UM v1.0.1: Host and Client sessions over UDP. `no_std`, no
allocator, sans-IO.

Part of [zetaohm-midi](../../README.md).

## Features

- `Host` and `Client` endpoints: invitation, session states, ping, Bye, Session Reset.
- UMP Data with sequence numbers, Forward Error Correction and retransmit.
- Optional authentication (shared secret and user name and password) with SHA-256.
- DNS-SD record data (PTR, SRV, TXT) for the `_midi2._udp` service. Your platform or a small mDNS
  responder announces it.
- You own the UDP socket, the clock (milliseconds) and the random numbers, so the same code runs on
  `std::net`, `embassy-net`, smoltcp or lwIP. Session and buffer counts are const generics.

## Example

A Host on a `std` UDP socket:

```rust
use std::net::UdpSocket;
use std::time::Instant;

use zmidi_net::{EventKind, Host, HostConfig, Identity, NoCredentials};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind("0.0.0.0:5673")?;
    let mut host: Host = Host::new(HostConfig::new(Identity::new("My Synth", "SN-0001")?))?;
    let start = Instant::now();
    let (mut rx, mut tx) = ([0u8; 1500], [0u8; 1500]);
    let mut credentials = NoCredentials; // no authentication

    loop {
        // 1. Give each received UDP payload to the Host, then send its replies.
        let (n, from) = socket.recv_from(&mut rx)?;
        let now = start.elapsed().as_millis() as u64;
        let mut received = host.receive(now, from, &rx[..n], &mut credentials);
        for event in received.by_ref() {
            if let EventKind::Data(data) = &event.kind {
                for ump in data.messages() {
                    println!("{} sent {:08X?}", event.peer, ump.words());
                }
            }
        }
        while let Some((to, len)) = received.write_next(&mut tx) {
            socket.send_to(&tx[..len], to)?;
        }
        drop(received);

        // 2. Run the timers, then send what is pending (UMP data, pings, retransmits).
        for event in host.poll(now) {
            println!("{event:?}");
        }
        while let Some((to, len)) = host.write_pending(now, &mut tx) {
            socket.send_to(&tx[..len], to)?;
        }
    }
}
```

A complete Host and Client, with DNS-SD, is in
[`examples/net-echo`](../../examples/net-echo). An embedded Host on an ESP32-S3 is in
[`examples/esp32s3-net-echo`](../../examples/esp32s3-net-echo).

## Cargo features

| Feature | Default | Description |
|---|---|---|
| `auth` | no | Authentication (adds `sha2`). |

## License

Licensed under the [MIT license](LICENSE). Copyright (c) 2025-2026 Zetaohm LLC.
