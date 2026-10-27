//! zmidi-net: Network MIDI 2.0 (M2-124). A Host and its Clients exchange UDP payloads through
//! buffers; the sessions open before the first mark. The benches measure the UMP Data path: the
//! Client queues and writes 64 notes, the Host reads them. The scale bench reads the same payload
//! from the last of 3 sessions (OPT-029 step 3).

use core::hint::black_box;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};

use zmidi_net::{
    Client, ClientConfig, EventKind, Host, HostConfig, Identity, NoCredentials, SessionConfig,
    SessionEvent,
};

use super::cv1;
use crate::bench;

const MTU: usize = 1400;
const CLIENTS: usize = 4;
const SESSIONS: usize = 3;

static mut HOST1: Option<Host<1>> = None;
static mut HOST4: Option<Host<SESSIONS>> = None;
static mut CLIENT: [Option<Client<1>>; CLIENTS] = [const { None }; CLIENTS];

fn addr(last: u8) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, last)), 5673)
}

/// No automatic Ping, so that only the bench traffic is on the wire.
fn quiet() -> SessionConfig {
    SessionConfig {
        ping_interval_ms: 0,
        ..SessionConfig::DEFAULT
    }
}

/// Moves every pending payload between the Host and the Clients until all are quiet; gives the
/// number of Established events at the Clients.
fn settle<const N: usize>(
    host: &mut Host<N>,
    haddr: SocketAddr,
    clients: &mut [(SocketAddr, &mut Client<1>)],
) -> u32 {
    let mut est = 0;
    let mut buf = [0u8; MTU];
    let mut reply = [0u8; MTU];
    let mut creds = NoCredentials;
    for _ in 0..16 {
        let mut moved = false;
        for _ in host.poll(1000) {}
        while let Some((to, n)) = host.write_pending(1000, &mut buf) {
            moved = true;
            if let Some((_, c)) = clients.iter_mut().find(|(a, _)| *a == to) {
                let mut r = c.receive(1000, haddr, &buf[..n], &mut creds);
                for e in r.by_ref() {
                    if matches!(e.kind, EventKind::Session(SessionEvent::Established { .. })) {
                        est += 1;
                    }
                }
                while r.write_next(&mut reply).is_some() {}
            }
        }
        for (a, c) in clients.iter_mut() {
            for _ in c.poll(1000) {}
            while let Some((_, n)) = c.write_pending(1000, &mut buf) {
                moved = true;
                let mut r = host.receive(1000, *a, &buf[..n], &mut creds);
                for _ in r.by_ref() {}
                while r.write_next(&mut reply).is_some() {}
            }
        }
        if !moved {
            break;
        }
    }
    est
}

/// Counts the UMP messages of the Data events of one payload received at the Host.
fn host_read<const N: usize>(host: &mut Host<N>, from: SocketAddr, p: &[u8]) -> u32 {
    let mut n = 0;
    let mut creds = NoCredentials;
    let mut r = host.receive(1000, from, p, &mut creds);
    for e in r.by_ref() {
        if let EventKind::Data(d) = e.kind {
            for m in d.messages() {
                black_box(&m);
                n += 1;
            }
        }
    }
    let mut b = [0u8; 64];
    while r.write_next(&mut b).is_some() {}
    n
}

pub fn run() {
    let hcfg = HostConfig {
        session: quiet(),
        ..HostConfig::new(Identity::new("Host", "H1").unwrap())
    };
    let ccfg = ClientConfig {
        session: quiet(),
        ..ClientConfig::new(Identity::new("Dev", "D1").unwrap())
    };
    // SAFETY: the probe has one thread and no interrupt; nothing else uses the statics.
    let (host1, host4, clients) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(HOST1),
            &mut *core::ptr::addr_of_mut!(HOST4),
            &mut *core::ptr::addr_of_mut!(CLIENT),
        )
    };
    let host1 = Host::init(host1, hcfg).unwrap();
    let host4 = Host::init(host4, hcfg).unwrap();
    let mut cl = [None, None, None, None];
    for (k, slot) in clients.iter_mut().enumerate() {
        cl[k] = Some(Client::init(slot, ccfg).unwrap() as *mut Client<1>);
    }
    // SAFETY: four distinct statics.
    let clients: [&mut Client<1>; CLIENTS] = cl.map(|c| unsafe { &mut *c.unwrap() });
    let [c0, c1, c2, c3] = clients;

    // Client 0 opens a session with host1 (address 1); clients 1 to 3 open sessions with host4
    // (address 9).
    let (a1, a4) = (addr(1), addr(9));
    c0.connect(1000, a1).unwrap();
    assert_eq!(settle(host1, a1, &mut [(addr(2), &mut *c0)]), 1);
    for c in [&mut *c1, &mut *c2, &mut *c3] {
        c.connect(1000, a4).unwrap();
    }
    let mut more = [
        (addr(3), &mut *c1),
        (addr(4), &mut *c2),
        (addr(5), &mut *c3),
    ];
    assert_eq!(settle(host4, a4, &mut more), 3);

    // M2-124 UMP Data Command: the Client writes the 64 notes into one UDP payload.
    let mut payload = [0u8; MTU];
    let mut len = 0;
    bench("net_client_send_note", 64, 1, || {
        for u in cv1().chunks(1) {
            let _ = c0.send_ump(a1, u);
        }
        let mut k = 0;
        while let Some((_, n)) = c0.write_pending(1000, &mut payload) {
            len = n;
            k += 1;
        }
        k
    });
    let p = black_box(&payload[..len]);
    bench("net_host_receive_note", 64, 64, || {
        host_read(host1, addr(2), p)
    });

    // The same payload from the last of 3 sessions of host4: the sequence numbers of each session
    // start at the same value, so the payload of client 0 is valid for client 3 too.
    bench("net_host3_receive_note_last_session", 64, 64, || {
        host_read(host4, addr(5), p)
    });
}
