//! A Network MIDI 2.0 (UDP, M2-124-UM v1.0.1) Host and Client on a `std` UDP socket, with the
//! sans-IO `zmidi-net` crate (DEC-012).
//!
//! [`Node`] owns one UDP socket, one `zmidi-net` [`Host`] or [`Client`] and the clock, and runs the
//! main loop of the `zmidi-net` endpoint documentation:
//! 1. for each received UDP payload: `receive`, then each stateless reply (`write_next`);
//! 2. at the next deadline and after each receive: `poll`, then `write_pending` until it gives
//!    nothing.
//!
//! The binaries `host` (sends each received UMP back) and `client` (connects, sends UMPs and
//! checks the echo) use it. `tests/loopback.rs` runs both on localhost. [`discovery`] announces
//! the Host with DNS-SD and finds it from the Client.

pub mod discovery;

use std::io::ErrorKind;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use zmidi_net::session::SessionError;
use zmidi_net::stream::StreamError;
use zmidi_net::{
    Client, ClientConfig, EndpointError, EventKind, Host, HostConfig, Identity, NoCredentials,
    SessionEvent,
};

/// The largest UDP payload that this example reads.
const MAX_UDP: usize = 1500;

/// An event of the endpoint, copied out of the received payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeEvent {
    /// The session with the peer is open.
    Established(SocketAddr),
    /// UMP messages from the peer, each as its words.
    Data(SocketAddr, Vec<Vec<u32>>),
    /// The session with the peer is closed.
    Closed(SocketAddr),
    /// Any other event, as text.
    Other(SocketAddr, String),
}

/// The common part of a `zmidi-net` Host and Client that the loop uses.
pub trait Endpoint {
    /// Reads one received UDP payload: the events and the stateless replies.
    fn rx(&mut self, now: u64, from: SocketAddr, payload: &[u8]) -> Output;
    /// Runs the timeouts and writes the pending payloads.
    fn tx(&mut self, now: u64) -> Output;
    /// The time of the next call to `tx`.
    fn next_deadline(&self) -> Option<u64>;
    /// Queues whole UMP messages for `peer`.
    fn send_ump(&mut self, peer: SocketAddr, words: &[u32]) -> Result<(), EndpointError>;
}

/// The events and the UDP payloads to send of one step.
#[derive(Debug, Default)]
pub struct Output {
    pub events: Vec<NodeEvent>,
    pub payloads: Vec<(SocketAddr, Vec<u8>)>,
}

fn own(peer: SocketAddr, kind: &EventKind<'_>) -> NodeEvent {
    match kind {
        EventKind::Session(SessionEvent::Established { .. }) => NodeEvent::Established(peer),
        EventKind::Session(SessionEvent::Closed(_)) => NodeEvent::Closed(peer),
        EventKind::Data(d) => {
            NodeEvent::Data(peer, d.messages().map(|m| m.words().to_vec()).collect())
        }
        other => NodeEvent::Other(peer, format!("{other:?}")),
    }
}

macro_rules! endpoint {
    ($t:ty) => {
        impl Endpoint for $t {
            fn rx(&mut self, now: u64, from: SocketAddr, payload: &[u8]) -> Output {
                let mut out = Output::default();
                let mut creds = NoCredentials;
                let mut r = self.receive(now, from, payload, &mut creds);
                for e in r.by_ref() {
                    out.events.push(own(e.peer, &e.kind));
                }
                let mut buf = [0u8; MAX_UDP];
                while let Some((to, n)) = r.write_next(&mut buf) {
                    out.payloads.push((to, buf[..n].to_vec()));
                }
                out
            }

            fn tx(&mut self, now: u64) -> Output {
                let mut out = Output::default();
                for e in self.poll(now) {
                    out.events.push(own(e.peer, &e.kind));
                }
                let mut buf = [0u8; MAX_UDP];
                while let Some((to, n)) = self.write_pending(now, &mut buf) {
                    out.payloads.push((to, buf[..n].to_vec()));
                }
                out
            }

            fn next_deadline(&self) -> Option<u64> {
                <$t>::next_deadline(self)
            }

            fn send_ump(&mut self, peer: SocketAddr, words: &[u32]) -> Result<(), EndpointError> {
                <$t>::send_ump(self, peer, words)
            }
        }
    };
}
endpoint!(Host);
endpoint!(Client);

/// One UDP socket, one endpoint and the clock.
pub struct Node<E> {
    pub socket: UdpSocket,
    pub endpoint: E,
    start: Instant,
}

impl<E: Endpoint> Node<E> {
    pub fn new(socket: UdpSocket, endpoint: E) -> Self {
        Node {
            socket,
            endpoint,
            start: Instant::now(),
        }
    }

    /// The clock of the endpoint, in ms.
    pub fn now(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Waits at most `max_wait` (less when a deadline comes first) for one UDP payload, gives it
    /// to the endpoint, runs `tx` and sends every payload. Gives the events.
    pub fn step(&mut self, max_wait: Duration) -> std::io::Result<Vec<NodeEvent>> {
        let now = self.now();
        let wait = match self.endpoint.next_deadline() {
            Some(d) => Duration::from_millis(d.saturating_sub(now)).min(max_wait),
            None => max_wait,
        };
        // A zero timeout is an error for `set_read_timeout`.
        self.socket
            .set_read_timeout(Some(wait.max(Duration::from_millis(1))))?;
        let mut events = Vec::new();
        let mut buf = [0u8; MAX_UDP];
        match self.socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                let out = self.endpoint.rx(self.now(), from, &buf[..n]);
                self.send(&out)?;
                events.extend(out.events);
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(e) => return Err(e),
        }
        let out = self.endpoint.tx(self.now());
        self.send(&out)?;
        events.extend(out.events);
        Ok(events)
    }

    fn send(&self, out: &Output) -> std::io::Result<()> {
        for (to, p) in &out.payloads {
            self.socket.send_to(p, to)?;
        }
        Ok(())
    }
}

/// A Host on `bind` (for example `0.0.0.0:5673`) that accepts each Client.
pub fn host(bind: &str, name: &str) -> Result<Node<Host>, Box<dyn std::error::Error>> {
    let identity = Identity::new(name, "zetaohm-midi-host")?;
    let host = Host::new(HostConfig::new(identity))?;
    Ok(Node::new(UdpSocket::bind(bind)?, host))
}

/// A Client on a free local port.
pub fn client(name: &str) -> Result<Node<Client>, Box<dyn std::error::Error>> {
    let identity = Identity::new(name, "zetaohm-midi-client")?;
    let client = Client::new(ClientConfig::new(identity))?;
    Ok(Node::new(UdpSocket::bind("0.0.0.0:0")?, client))
}

/// Host loop: sends each received UMP back to its peer, until `stop` gives true.
pub fn run_echo_host(
    node: &mut Node<Host>,
    mut log: impl FnMut(&NodeEvent),
    mut stop: impl FnMut() -> bool,
) -> std::io::Result<()> {
    while !stop() {
        for e in node.step(Duration::from_millis(50))? {
            log(&e);
            if let NodeEvent::Data(peer, msgs) = &e {
                for m in msgs {
                    if let Err(err) = node.endpoint.send_ump(*peer, m) {
                        log(&NodeEvent::Other(*peer, format!("echo refused: {err:?}")));
                    }
                }
            }
        }
    }
    Ok(())
}

/// The UMPs that the client sends: Note On and Note Off, MIDI 2.0 Channel Voice (Message Type
/// 0x4, M2-104 7.4.1, 7.4.2), Group 0, Channel 0, notes 60 to 60 + count - 1, velocity 0x8000.
pub fn test_notes(count: u8) -> Vec<[u32; 2]> {
    (0..count)
        .flat_map(|i| {
            let note = u32::from(60 + i);
            [
                [0x4090_0000 | note << 8, 0x8000_0000],
                [0x4080_0000 | note << 8, 0x0000_0000],
            ]
        })
        .collect()
}

/// Client run: connects to `host`, sends `umps` when the session is open, and waits up to
/// `timeout` for the same UMPs to come back. Gives the received UMPs.
pub fn run_echo_client(
    node: &mut Node<Client>,
    host: SocketAddr,
    umps: &[[u32; 2]],
    timeout: Duration,
    mut log: impl FnMut(&NodeEvent),
) -> Result<Vec<Vec<u32>>, Box<dyn std::error::Error>> {
    let end = Instant::now() + timeout;
    let now = node.now();
    node.endpoint.connect(now, host)?;
    let mut established = false;
    let mut next = 0;
    let mut got = Vec::new();
    while Instant::now() < end && got.len() < umps.len() {
        // Queue what fits; a full send queue (256 words) empties at the next step.
        while established && next < umps.len() {
            match node.endpoint.send_ump(host, &umps[next]) {
                Ok(()) => next += 1,
                Err(EndpointError::Session(SessionError::Stream(StreamError::Full))) => break,
                Err(e) => return Err(e.into()),
            }
        }
        for e in node.step(Duration::from_millis(20))? {
            log(&e);
            match e {
                NodeEvent::Established(peer) if peer == host => established = true,
                NodeEvent::Data(peer, msgs) if peer == host => got.extend(msgs),
                _ => {}
            }
        }
    }
    Ok(got)
}

/// Client end: sends Bye (User Terminated, M2-124 6.16) to `host` and waits up to `timeout`
/// for the Bye Reply (6.17). Gives true when the session closed.
pub fn bye(
    node: &mut Node<Client>,
    host: SocketAddr,
    timeout: Duration,
    mut log: impl FnMut(&NodeEvent),
) -> Result<bool, Box<dyn std::error::Error>> {
    let now = node.now();
    node.endpoint
        .bye(now, host, zmidi_net::ByeReason::USER_TERMINATED, "")?;
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        for e in node.step(Duration::from_millis(20))? {
            log(&e);
            if e == NodeEvent::Closed(host) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
