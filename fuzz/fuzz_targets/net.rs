//! Network MIDI 2.0 UDP payloads from a peer: `Host::receive` and `Client::receive` with
//! authentication on, their replies, the timeouts and the pending writes.
#![no_main]

use core::net::SocketAddr;

use libfuzzer_sys::fuzz_target;
use zmidi_net::{Client, ClientConfig, Credentials, Host, HostConfig, Identity, SessionConfig};

/// Fixed credentials: a nonce, a shared secret and one user, so that both authentication
/// methods run.
struct Creds;

impl Credentials for Creds {
    fn random_nonce(&mut self) -> Option<[u8; 16]> {
        Some([7; 16])
    }
    fn shared_secret(&mut self) -> Option<&[u8]> {
        Some(b"secret")
    }
    fn password(&mut self, username: &[u8]) -> Option<&[u8]> {
        (username == b"user").then_some(&b"password"[..])
    }
    fn user(&mut self) -> Option<(&[u8], &[u8])> {
        Some((b"user", b"password"))
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let session = SessionConfig {
        ping_interval_ms: if mode & 1 == 0 { 0 } else { 2000 },
        ..SessionConfig::DEFAULT
    };
    let mut host: Host<2> = Host::new(HostConfig {
        session,
        ..HostConfig::new(Identity::new("Host", "H1").unwrap())
    })
    .unwrap();
    let mut client: Client = Client::new(ClientConfig {
        session,
        ..ClientConfig::new(Identity::new("MyDev", "8shYe3h5").unwrap())
    })
    .unwrap();
    let peer = SocketAddr::from(([10, 0, 0, 2], 5004));
    let _ = client.connect(0, peer);
    let mut creds = Creds;
    let mut buf = [0u8; 1400];
    let mut now = 0u64;
    for (i, part) in zmidi_fuzz::parts(rest).enumerate() {
        now += 700 * (i as u64 % 9);
        // Half of the payloads get the "MIDI" signature, so the fuzzer reaches the Commands.
        let mut payload = Vec::with_capacity(part.len() + 4);
        if part.first().is_some_and(|b| b & 1 == 1) {
            payload.extend_from_slice(b"MIDI");
        }
        payload.extend_from_slice(part);
        if (mode >> 1) & 1 == (i as u8) & 1 {
            let mut r = host.receive(now, peer, &payload, &mut creds);
            for _ in r.by_ref().take(1000) {}
            for _ in 0..1000 {
                if r.write_next(&mut buf).is_none() {
                    break;
                }
            }
        } else {
            let mut r = client.receive(now, peer, &payload, &mut creds);
            for _ in r.by_ref().take(1000) {}
            for _ in 0..1000 {
                if r.write_next(&mut buf).is_none() {
                    break;
                }
            }
        }
        for _ in host.poll(now).take(1000) {}
        for _ in client.poll(now).take(1000) {}
        for _ in 0..1000 {
            if host.write_pending(now, &mut buf).is_none() {
                break;
            }
        }
        for _ in 0..1000 {
            if client.write_pending(now, &mut buf).is_none() {
                break;
            }
        }
    }
});
