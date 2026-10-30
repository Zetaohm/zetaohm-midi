//! The Host and Client instances of Network MIDI 2.0 over UDP (M2-124-UM v1.0.1): the
//! receive loop, the session table, the stateless replies and the round-robin writer.
//!
//! - [`Host`] holds at most `S` Client sessions (default 4, N-42, REQ-NET-GEN-001);
//!   [`Client`] holds at most `M` Host sessions (default 1, N-43, REQ-NET-GEN-002). Each
//!   session entry is one [`Session`] of the Session module, keyed by the peer address
//!   (IP address and UDP port, REQ-NET-GEN-004).
//! - Sans-IO (DEC-002, DEC-012): the caller owns the UDP socket and the clock; it gives the
//!   time as u64 ms, each peer address as a [`SocketAddr`] and each buffer. No static, no
//!   alloc: several Host and Client instances can run in one process (REQ-NET-TOPO-006).
//! - The receive loop ([`Host::receive`], [`Client::receive`]) checks the Signature
//!   (REQ-NET-PKT-003), writes NAK 0x01 for an unknown Command Code (REQ-NET-CMD-002) and
//!   NAK 0x03 for a malformed Command (REQ-NET-NAK-004, REQ-NET-PKT-014, REQ-NET-DATA-006),
//!   never a NAK for a NAK (REQ-NET-NAK-010), gives each other Command to the Session of its
//!   address, and answers an address with no entry with [`Session::idle_reply`] (N-16).
//! - A multicast address or the IPv4 broadcast address is not a peer address
//!   (REQ-NET-PKT-002).
//! - The Credentials trait and the public API are the same with and without the Cargo
//!   feature `auth` (DEC-013, N-52). The Host holds one failure delay for all its Clients
//!   (N-45).
//!
//! The main loop of the caller:
//! 1. For each received UDP payload: `receive(now, from, payload, creds)`; read the
//!    [`Received`] events, then write each stateless reply with [`Received::write_next`].
//! 2. At [`Host::next_deadline`] and after each receive: [`Host::poll`] (read the events),
//!    then [`Host::write_pending`] until it gives `None`.

use core::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};

use crate::command::{
    write_ping, ByeReason, Capabilities, Command, CommandCode, CommandError, NakReason, Role,
    SessionState,
};
use crate::dns::{DnsError, HostRecords, Hostname, ServiceInstanceName};
use crate::handshake::{
    Credentials, Deadline, Env, FailureDelay, HostPolicy, DEFAULT_USERNAME_CAPACITY,
};
use crate::identity::Identity;
use crate::packet::{
    command_packets, CommandPackets, Header, MaxPayload, PacketError, PacketItem, PayloadWriter,
};
use crate::session::{
    Events, IdleReply, Opening, Session, SessionConfig, SessionCore, SessionError, SessionEvent,
    SessionRef, Stateless,
};
use crate::stream::{Delivery, Ring, StreamError};

/// The default capacity of the session table of a Host (N-42, REQ-NET-GEN-001).
pub const DEFAULT_HOST_SESSIONS: usize = 4;
/// The default number of sessions of a Client (N-43, REQ-NET-GEN-002).
pub const DEFAULT_CLIENT_SESSIONS: usize = 1;
/// The default first delay after a failed authentication, in ms (N-45).
pub const DEFAULT_FAILURE_DELAY_INITIAL_MS: u64 = 100;
/// The default largest delay after a failed authentication, in ms (N-45).
pub const DEFAULT_FAILURE_DELAY_MAX_MS: u64 = 3200;
/// The default capacity `O` of the queue of stateless Commands (NAK, idle replies, Pings to
/// addresses with no session) and of the queue of events that wait for [`Host::poll`]. A full
/// stateless queue drops the new Command, as a lost UDP payload would; a full event queue drops
/// the event and counts it. `O` is a const generic of [`Host`] and [`Client`], 1 or more
/// (OPT-008).
pub const STATELESS_CAPACITY: usize = 16;
/// The number of open Pings to addresses with no session that the endpoint keeps for the
/// round-trip time.
pub const STATELESS_PINGS: usize = 4;
// The events of one received Command: the Session events, a data delivery and a diagnostic.
const SINK_CAPACITY: usize = crate::session::EVENTS_CAPACITY + 4;

/// Error of the endpoint requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    /// A multicast address or the IPv4 broadcast address as a peer address
    /// (REQ-NET-PKT-002).
    #[error("multicast or broadcast peer address")]
    InvalidAddress,
    /// No session entry for this address.
    #[error("no session with this address")]
    NoSession,
    /// Client: a session with this Host address is not in Idle (REQ-NET-GEN-002).
    #[error("a session with this address is in state {0:?}")]
    Busy(SessionState),
    /// Client: all `M` session places are in use.
    #[error("session table full")]
    TableFull,
    /// The Session refused the request or the configuration.
    #[error("session: {0}")]
    Session(#[from] SessionError),
    /// A DNS-SD value is not valid.
    #[error("dns: {0}")]
    Dns(#[from] DnsError),
}

/// A diagnostic of the receive loop: no state changed for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Diagnostic {
    /// The UDP payload is shorter than 4 bytes or has no Signature; it was ignored
    /// (REQ-NET-PKT-003).
    BadSignature,
    /// The source address is a multicast or broadcast address; the payload was ignored
    /// (REQ-NET-PKT-002).
    InvalidPeerAddress,
    /// Host: an Invitation from a new address with no free place in the table; the reply is
    /// Bye 0x40 (REQ-NET-INV-035).
    TableFull,
    /// The endpoint answers a Command with this NAK (REQ-NET-CMD-002, REQ-NET-NAK-004).
    NakSent {
        /// The NAK Reason.
        reason: NakReason,
        /// The header of the refused Command, as received.
        header: Header,
    },
}

/// What happened, for one peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind<'p> {
    /// An event of the Session of the peer (or of the idle rules for an address with no
    /// session).
    Session(SessionEvent<'p>),
    /// UMP Data for the caller; the messages borrow the received payload (REQ-NET-DATA-008,
    /// -010).
    Data(Delivery<'p>),
    /// A diagnostic.
    Diagnostic(Diagnostic),
}

/// An event with the address of its peer (REQ-NET-GEN-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event<'p> {
    /// The IP address and UDP port of the peer.
    pub peer: SocketAddr,
    /// The event.
    pub kind: EventKind<'p>,
}

impl<'p> Event<'p> {
    fn session(peer: SocketAddr, e: SessionEvent<'p>) -> Self {
        Event {
            peer,
            kind: EventKind::Session(e),
        }
    }

    fn diagnostic(peer: SocketAddr, d: Diagnostic) -> Self {
        Event {
            peer,
            kind: EventKind::Diagnostic(d),
        }
    }
}

/// `InvalidAddress` for a multicast address (IPv4 or IPv6, also IPv4-mapped) or the IPv4
/// broadcast address 255.255.255.255 (REQ-NET-PKT-002).
pub fn check_peer(addr: SocketAddr) -> Result<(), EndpointError> {
    let v4_bad = |a: Ipv4Addr| a.is_multicast() || a.is_broadcast();
    let bad = match addr.ip() {
        IpAddr::V4(a) => v4_bad(a),
        IpAddr::V6(a) => a.is_multicast() || a.to_ipv4_mapped().is_some_and(v4_bad),
    };
    if bad {
        Err(EndpointError::InvalidAddress)
    } else {
        Ok(())
    }
}

/// The configuration of a [`Host`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostConfig {
    /// The UMP Endpoint Name and Product Instance Id of this Host: written in each Invitation
    /// Reply and in the TXT record (REQ-NET-DISC-013, REQ-NET-TOPO-005).
    pub identity: Identity,
    /// The policy for new Clients (REQ-NET-INV-012).
    pub policy: HostPolicy,
    /// The configuration of each session.
    pub session: SessionConfig,
    /// The largest UDP payload that the Host writes (REQ-NET-PKT-004, N-31).
    pub max_payload: MaxPayload,
    /// The first delay after a failed authentication, in ms; 0 turns the delay off (N-45).
    pub failure_delay_initial_ms: u64,
    /// The largest delay after failed authentications, in ms (N-45).
    pub failure_delay_max_ms: u64,
    /// The first Ping Id of the Ping counter (REQ-NET-PING-003).
    pub first_ping_id: u32,
}

impl HostConfig {
    /// A configuration with `identity` and the default values.
    pub fn new(identity: Identity) -> Self {
        HostConfig {
            identity,
            policy: HostPolicy::DEFAULT,
            session: SessionConfig::DEFAULT,
            max_payload: MaxPayload::default(),
            failure_delay_initial_ms: DEFAULT_FAILURE_DELAY_INITIAL_MS,
            failure_delay_max_ms: DEFAULT_FAILURE_DELAY_MAX_MS,
            first_ping_id: 0,
        }
    }
}

/// The configuration of a [`Client`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientConfig {
    /// The UMP Endpoint Name and Product Instance Id written in each Invitation
    /// (REQ-NET-INV-006).
    pub identity: Identity,
    /// The authentication methods that the caller enables (REQ-NET-AUTH-013).
    pub capabilities: Capabilities,
    /// The configuration of each session.
    pub session: SessionConfig,
    /// The largest UDP payload that the Client writes (REQ-NET-PKT-004, N-31).
    pub max_payload: MaxPayload,
    /// The first Ping Id of the Ping counter (REQ-NET-PING-003).
    pub first_ping_id: u32,
}

impl ClientConfig {
    /// A configuration with `identity` and the default values (no authentication method).
    pub fn new(identity: Identity) -> Self {
        ClientConfig {
            identity,
            capabilities: Capabilities {
                auth: false,
                user_auth: false,
            },
            session: SessionConfig::DEFAULT,
            max_payload: MaxPayload::default(),
            first_ping_id: 0,
        }
    }
}

// ----- internal queues -----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Out {
    Reply(Stateless),
    Ping(u32),
}

const NO_ADDR: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));

/// The FIFO of stateless Commands.
#[derive(Debug, Clone)]
struct OutQueue<const O: usize> {
    ring: Ring<(SocketAddr, Out), O>,
    dropped: u32,
}

impl<const O: usize> OutQueue<O> {
    const fn new() -> Self {
        OutQueue {
            ring: Ring::new((NO_ADDR, Out::Ping(0))),
            dropped: 0,
        }
    }

    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    fn push(&mut self, addr: SocketAddr, o: Out) {
        if self.ring.push_back((addr, o)).is_err() {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    fn front(&self) -> Option<(SocketAddr, Out)> {
        self.ring.get(0)
    }

    fn pop(&mut self) {
        self.ring.drop_front(1);
    }
}

/// The open Pings to addresses with no session (REQ-NET-PING-003).
#[derive(Debug, Clone)]
struct StatelessPings {
    ring: Ring<(SocketAddr, u32, u64), STATELESS_PINGS>,
}

impl StatelessPings {
    const fn new() -> Self {
        StatelessPings {
            ring: Ring::new((NO_ADDR, 0, 0)),
        }
    }

    fn push(&mut self, addr: SocketAddr, id: u32, now: u64) {
        if self.ring.len() == STATELESS_PINGS {
            self.ring.drop_front(1);
        }
        let _ = self.ring.push_back((addr, id, now));
    }

    // The round-trip time of the open Ping, which is closed.
    fn close(&mut self, addr: SocketAddr, id: u32, now: u64) -> Option<u64> {
        let i = self.ring.rposition(|p| p.0 == addr && p.1 == id)?;
        let sent = self.ring.get(i)?.2;
        self.ring.remove(i);
        Some(now.saturating_sub(sent))
    }
}

/// The events that wait for [`Host::poll`] (events of the polls that `write_pending` did).
#[derive(Debug, Clone)]
struct Backlog<const O: usize> {
    items: [Option<(SocketAddr, SessionEvent<'static>)>; O],
    head: usize,
    len: usize,
    dropped: u32,
}

impl<const O: usize> Backlog<O> {
    const fn new() -> Self {
        Backlog {
            items: [None; O],
            head: 0,
            len: 0,
            dropped: 0,
        }
    }

    fn push(&mut self, addr: SocketAddr, e: SessionEvent<'static>) {
        if self.len == O {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.items[(self.head + self.len) % O] = Some((addr, e));
        self.len += 1;
    }

    fn pop(&mut self) -> Option<(SocketAddr, SessionEvent<'static>)> {
        if self.len == 0 {
            return None;
        }
        let it = self.items[self.head].take();
        self.head = (self.head + 1) % O;
        self.len -= 1;
        it
    }
}

/// The events of one received Command.
#[derive(Debug)]
struct Sink<'p> {
    items: [Option<Event<'p>>; SINK_CAPACITY],
    read: usize,
    write: usize,
    dropped: u32,
}

impl<'p> Sink<'p> {
    const fn new() -> Self {
        Sink {
            items: [None; SINK_CAPACITY],
            read: 0,
            write: 0,
            dropped: 0,
        }
    }

    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    fn push(&mut self, e: Event<'p>) {
        match self.items.get_mut(self.write) {
            Some(slot) => {
                *slot = Some(e);
                self.write += 1;
            }
            None => self.dropped = self.dropped.saturating_add(1),
        }
    }

    fn pop(&mut self) -> Option<Event<'p>> {
        let e = self.items.get_mut(self.read).and_then(Option::take);
        if e.is_some() {
            self.read += 1;
        } else {
            self.read = 0;
            self.write = 0;
        }
        e
    }
}

// ----- the shared core of Host and Client -----

#[derive(Debug, Clone)]
struct Entry<const W: usize, const C: usize, const Q: usize, const P: usize, const U: usize> {
    addr: SocketAddr,
    session: SessionCore<W, C, Q, P, U>,
    // The time of the last poll of this entry.
    polled: Deadline,
}

/// The session table and the queues of one Host or Client instance.
#[derive(Debug, Clone)]
struct Core<
    const N: usize,
    const W: usize,
    const C: usize,
    const Q: usize,
    const P: usize,
    const U: usize,
    const O: usize,
> {
    role: Role,
    identity: Identity,
    config: SessionConfig,
    capabilities: Capabilities,
    policy: HostPolicy,
    // One delay for the whole Host (N-45).
    delay: FailureDelay,
    max: MaxPayload,
    entries: [Option<Entry<W, C, Q, P, U>>; N],
    out: OutQueue<O>,
    pings: StatelessPings,
    next_ping_id: u32,
    backlog: Backlog<O>,
    rr: usize,
    now: u64,
    dropped_events: u32,
    write_errors: u32,
}

impl<
        const N: usize,
        const W: usize,
        const C: usize,
        const Q: usize,
        const P: usize,
        const U: usize,
        const O: usize,
    > Core<N, W, C, Q, P, U, O>
{
    const CHECK: () = {
        assert!(N >= 1, "the session table must have 1 place or more");
        assert!(O >= 1, "the queues must have 1 place or more");
    };

    /// The session configuration of the instance, checked: `max_command_words` is limited to
    /// the largest UMP Data Command that fits in one UDP payload. The Signature and one header
    /// word take 8 bytes (REQ-NET-PKT-004); a maximum payload with no room for one UMP word is
    /// refused.
    fn check_config(
        config: SessionConfig,
        max: MaxPayload,
    ) -> Result<SessionConfig, EndpointError> {
        let () = Self::CHECK;
        let mut config = config;
        let words = (max.bytes().saturating_sub(8) / 4).min(crate::command::MAX_UMP_DATA_WORDS);
        if words == 0 {
            return Err(SessionError::Stream(StreamError::CommandWords).into());
        }
        config.stream.max_command_words = config.stream.max_command_words.min(words);
        config.check()?;
        Ok(config)
    }

    /// A Core from a configuration of [`Core::check_config`]. It returns no `Result`, so that
    /// the compiler can build it in the place of the caller (OPT-003).
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn build(
        role: Role,
        identity: Identity,
        config: SessionConfig,
        capabilities: Capabilities,
        policy: HostPolicy,
        delay: FailureDelay,
        max: MaxPayload,
        first_ping_id: u32,
    ) -> Self {
        Core {
            role,
            identity,
            config,
            capabilities,
            policy,
            delay,
            max,
            entries: core::array::from_fn(|_| None),
            out: OutQueue::new(),
            pings: StatelessPings::new(),
            next_ping_id: first_ping_id,
            backlog: Backlog::new(),
            rr: 0,
            now: 0,
            dropped_events: 0,
            write_errors: 0,
        }
    }

    fn find(&self, addr: SocketAddr) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.as_ref().is_some_and(|e| e.addr == addr))
    }

    fn session(&self, addr: SocketAddr) -> Option<SessionRef<'_, W, C, Q, P, U>> {
        self.find(addr)
            .and_then(|i| self.entries[i].as_ref())
            .map(|e| SessionRef::new(&e.session, &self.config))
    }

    // The session of `addr` and the configuration of the instance (OPT-009: one copy for all
    // sessions).
    fn entry_mut(
        &mut self,
        addr: SocketAddr,
    ) -> Result<(&mut SessionCore<W, C, Q, P, U>, &SessionConfig), EndpointError> {
        check_peer(addr)?;
        let i = self.find(addr).ok_or(EndpointError::NoSession)?;
        let config = &self.config;
        self.entries[i]
            .as_mut()
            .map(|e| (&mut e.session, config))
            .ok_or(EndpointError::NoSession)
    }

    fn sessions(&self) -> impl Iterator<Item = (SocketAddr, SessionRef<'_, W, C, Q, P, U>)> {
        let config = &self.config;
        self.entries
            .iter()
            .flatten()
            .map(move |e| (e.addr, SessionRef::new(&e.session, config)))
    }

    // A session that is not in Idle uses one place (REQ-NET-GEN-001).
    fn active(&self) -> usize {
        self.sessions()
            .filter(|(_, s)| s.state() != SessionState::Idle)
            .count()
    }

    // A free place: an empty one, else one whose session is finished: in Idle with no
    // Command to write (REQ-NET-GEN-001). An Idle entry that still owes a Command (for
    // example a Bye Reply) is not free.
    fn free_slot(&mut self) -> Option<usize> {
        if let Some(i) = self.entries.iter().position(Option::is_none) {
            return Some(i);
        }
        let i = self
            .entries
            .iter()
            .position(|e| e.as_ref().is_some_and(|e| e.session.is_finished()))?;
        self.entries[i] = None;
        Some(i)
    }

    // Frees the entry when its Session is finished (Session contract step 4).
    fn tidy(&mut self, i: usize) {
        if let Some(slot) = self.entries.get_mut(i) {
            if slot.as_ref().is_some_and(|e| e.session.is_finished()) {
                *slot = None;
            }
        }
    }

    fn tidy_addr(&mut self, addr: SocketAddr) {
        if let Some(i) = self.find(addr) {
            self.tidy(i);
        }
    }

    // ----- caller requests -----

    fn connect(&mut self, now: u64, addr: SocketAddr) -> Result<(), EndpointError> {
        check_peer(addr)?;
        self.now = now;
        let i = match self.find(addr) {
            Some(i) => i,
            None => {
                let i = self.free_slot().ok_or(EndpointError::TableFull)?;
                // Set up in the slot: no Session is built on the stack and copied (OPT-003).
                // `config` was checked in `Core::new`.
                self.entries[i] = Some(Entry {
                    addr,
                    session: SessionCore::client_checked(self.capabilities),
                    polled: Deadline::NONE,
                });
                i
            }
        };
        let Some(e) = self.entries[i].as_mut() else {
            return Err(EndpointError::NoSession);
        };
        let state = e.session.state();
        if state != SessionState::Idle {
            return Err(EndpointError::Busy(state));
        }
        let r = e.session.connect(&self.config, now);
        self.tidy(i);
        Ok(r?)
    }

    fn ping(&mut self, now: u64, addr: SocketAddr, id: Option<u32>) -> Result<u32, EndpointError> {
        check_peer(addr)?;
        self.now = now;
        let from_counter = id.is_none();
        let id = id.unwrap_or(self.next_ping_id);
        match self.find(addr).and_then(|i| self.entries[i].as_mut()) {
            Some(e) => e.session.ping(now, id)?,
            None => {
                if self.out.ring.len() == O {
                    return Err(EndpointError::TableFull);
                }
                self.out.push(addr, Out::Ping(id));
            }
        }
        // The counter increments by 1 (modulo 2^32) for each Ping that takes its Id from
        // the counter (REQ-NET-PING-003).
        if from_counter {
            self.next_ping_id = id.wrapping_add(1);
        }
        Ok(id)
    }

    fn send_ump(&mut self, addr: SocketAddr, words: &[u32]) -> Result<(), EndpointError> {
        let (s, cfg) = self.entry_mut(addr)?;
        Ok(s.send_ump(cfg, words)?)
    }

    fn bye(
        &mut self,
        now: u64,
        addr: SocketAddr,
        reason: ByeReason,
        text: &str,
    ) -> Result<Events<'static>, EndpointError> {
        self.now = now;
        let (s, cfg) = self.entry_mut(addr)?;
        let r = s.bye(cfg, now, reason, text);
        self.tidy_addr(addr);
        Ok(r?)
    }

    fn cancel(&mut self, now: u64, addr: SocketAddr) -> Result<Events<'static>, EndpointError> {
        self.now = now;
        let (s, cfg) = self.entry_mut(addr)?;
        let r = s.cancel(cfg, now);
        self.tidy_addr(addr);
        Ok(r?)
    }

    fn reset(&mut self, now: u64, addr: SocketAddr) -> Result<(), EndpointError> {
        self.now = now;
        let (s, _) = self.entry_mut(addr)?;
        Ok(s.reset(now)?)
    }

    fn accept(&mut self, now: u64, addr: SocketAddr) -> Result<Events<'static>, EndpointError> {
        self.now = now;
        let (s, cfg) = self.entry_mut(addr)?;
        let r = s.accept(cfg, now);
        self.tidy_addr(addr);
        Ok(r?)
    }

    fn reject(
        &mut self,
        now: u64,
        addr: SocketAddr,
        reason: Option<ByeReason>,
    ) -> Result<Events<'static>, EndpointError> {
        self.now = now;
        let (s, cfg) = self.entry_mut(addr)?;
        let r = s.reject(cfg, now, reason);
        self.tidy_addr(addr);
        Ok(r?)
    }

    fn answer<Cr: Credentials + ?Sized>(
        &mut self,
        now: u64,
        addr: SocketAddr,
        creds: &mut Cr,
    ) -> Result<Events<'static>, EndpointError> {
        self.now = now;
        let (s, cfg) = self.entry_mut(addr)?;
        let ev = s.answer(cfg, now, creds);
        self.tidy_addr(addr);
        Ok(ev)
    }

    // ----- receive -----

    fn nak(&mut self, from: SocketAddr, reason: NakReason, header: Header, sink: &mut Sink<'_>) {
        self.out
            .push(from, Out::Reply(Stateless::Nak(reason, header)));
        sink.push(Event::diagnostic(
            from,
            Diagnostic::NakSent { reason, header },
        ));
    }

    // NAK 0x03, except for a NAK (REQ-NET-NAK-004, -010).
    fn malformed(&mut self, from: SocketAddr, header: Header, sink: &mut Sink<'_>) {
        if header.code != CommandCode::Nak.into_bits() {
            self.nak(from, NakReason::COMMAND_MALFORMED, header, sink);
        }
    }

    fn process<'p, Cr: Credentials + ?Sized>(
        &mut self,
        creds: &mut Cr,
        now: u64,
        from: SocketAddr,
        item: PacketItem<'p>,
        first: &mut bool,
        sink: &mut Sink<'p>,
    ) {
        let raw = match item {
            // REQ-NET-PKT-014, N-13.
            PacketItem::Truncated(h) => return self.malformed(from, h, sink),
            PacketItem::Command(raw) => raw,
        };
        let header = raw.header;
        let cmd = match Command::parse(raw) {
            Ok(Command::Unknown(_)) => {
                // REQ-NET-CMD-002: in each state, also with no session.
                return self.nak(from, NakReason::COMMAND_NOT_SUPPORTED, header, sink);
            }
            Ok(c) => c,
            // REQ-NET-NAK-004, REQ-NET-INV-032, -034.
            Err(_) => return self.malformed(from, header, sink),
        };
        // REQ-NET-DATA-006: a UMP Data payload that ends inside a message.
        let cut = match cmd {
            Command::UmpData(d) => {
                let mut m = d.messages();
                m.by_ref().for_each(drop);
                m.truncated()
            }
            _ => false,
        };
        // The NAK 0x03 for a cut UMP Data Command is for an entry in Established Session
        // only (below, with the delivery); in other states the Command takes the usual rules
        // (Bye 0x05 or discard, REQ-NET-DATA-015, REQ-NET-RST-006).
        let slot = self.find(from);
        let delay = match self.role {
            Role::Host => Some(&mut self.delay),
            Role::Client => None,
        };
        let mut env = Env {
            creds,
            delay,
            policy: self.policy,
        };
        if let Some(i) = slot {
            let Some(e) = self.entries[i].as_mut() else {
                return;
            };
            let r = e.session.receive(&self.config, now, header, &cmd, &mut env);
            for ev in r.events.iter() {
                sink.push(Event::session(from, *ev));
            }
            if let Some(d) = r.data {
                let messages = d.messages().next().is_some();
                if (d.delivers() && messages) || d.gap.is_some() || d.lost.is_some() {
                    sink.push(Event {
                        peer: from,
                        kind: EventKind::Data(d),
                    });
                }
                // One NAK: a FEC repeat of the same Command is skipped (REQ-NET-DATA-006).
                if cut && d.delivers() {
                    self.nak(from, NakReason::COMMAND_MALFORMED, header, sink);
                }
            }
            self.tidy(i);
            return;
        }
        // An address with no entry: Idle (REQ-NET-GEN-004, N-16).
        if let Command::PingReply(id) = cmd {
            if let Some(rtt_ms) = self.pings.close(from, id, now) {
                sink.push(Event::session(from, SessionEvent::PingReply { id, rtt_ms }));
                return;
            }
        }
        let reply = Session::<W, C, Q, P, U>::idle_reply(
            now,
            self.role,
            header,
            &cmd,
            *first,
            &self.config,
            &mut env,
        );
        match reply {
            IdleReply::Ignore => {}
            IdleReply::Send(st, ev) => {
                for e in ev.iter() {
                    sink.push(Event::session(from, *e));
                }
                // One Bye 0x05 for each UDP payload (N-16).
                if st == Stateless::Bye(ByeReason::SESSION_NOT_ESTABLISHED) {
                    *first = false;
                }
                self.out.push(from, Out::Reply(st));
            }
            IdleReply::Event(e) => sink.push(Event::session(from, e)),
            IdleReply::Open(o) => self.open(now, from, o, sink),
        }
    }

    // Host: a new session in a free place, else Bye 0x40 (REQ-NET-GEN-001, REQ-NET-INV-035).
    fn open<'p>(&mut self, now: u64, from: SocketAddr, o: Opening<U>, sink: &mut Sink<'p>) {
        if self.role != Role::Host {
            return;
        }
        let Some(i) = self.free_slot() else {
            self.out.push(
                from,
                Out::Reply(Stateless::Bye(ByeReason::TOO_MANY_SESSIONS)),
            );
            sink.push(Event::diagnostic(from, Diagnostic::TableFull));
            return;
        };
        // Set up in the slot: no Session is built on the stack and copied (OPT-003).
        // `config` was checked in `Core::new`.
        let e = self.entries[i].insert(Entry {
            addr: from,
            session: SessionCore::host_checked(),
            polled: Deadline::NONE,
        });
        let Ok(ev) = e.session.open(now, o) else {
            self.entries[i] = None;
            return;
        };
        for e in ev.iter() {
            sink.push(Event::session(from, *e));
        }
        self.tidy(i);
    }

    // ----- poll and write -----

    fn poll_entry(&mut self, i: usize, now: u64) -> Option<(SocketAddr, Events<'static>)> {
        let e = self.entries.get_mut(i)?.as_mut()?;
        let ev = e.session.poll(&self.config, now);
        e.polled = Deadline::at(now);
        let addr = e.addr;
        self.tidy(i);
        Some((addr, ev))
    }

    fn next_deadline(&self) -> Option<u64> {
        let mut d = (self.out.ring.len() > 0).then_some(self.now);
        for (_, s) in self.sessions() {
            if let Some(t) = s.next_deadline() {
                d = Some(d.map_or(t, |x| x.min(t)));
            }
        }
        d
    }

    // The stateless Commands of the address at the front of the queue, in one UDP payload.
    fn write_out(&mut self, now: u64, buf: &mut [u8]) -> Option<(SocketAddr, usize)> {
        loop {
            let (addr, _) = self.out.front()?;
            let mut w = PayloadWriter::new(&mut *buf, self.max);
            while let Some((a, item)) = self.out.front() {
                if a != addr {
                    break;
                }
                let r = match item {
                    Out::Reply(st) => st.write(self.role, &mut w),
                    Out::Ping(id) => write_ping(&mut w, id),
                };
                match r {
                    Ok(()) => {
                        if let Out::Ping(id) = item {
                            self.pings.push(addr, id, now);
                        }
                        self.out.pop();
                    }
                    // No room: the next UDP payload.
                    Err(CommandError::Packet(PacketError::Overflow)) => break,
                    Err(_) => {
                        self.write_errors = self.write_errors.saturating_add(1);
                        self.out.pop();
                    }
                }
            }
            let n = w.len();
            if n > 0 {
                return Some((addr, n));
            }
        }
    }

    fn write_pending(&mut self, now: u64, buf: &mut [u8]) -> Option<(SocketAddr, usize)> {
        self.now = now;
        if let Some(r) = self.write_out(now, buf) {
            return Some(r);
        }
        for k in 0..N {
            let i = (self.rr + k) % N;
            // Poll before write (Session contract); the events wait for `poll`.
            if self.entries[i]
                .as_ref()
                .is_some_and(|e| e.polled.get().is_none_or(|t| t < now))
            {
                if let Some((addr, ev)) = self.poll_entry(i, now) {
                    for e in ev.iter() {
                        self.backlog.push(addr, *e);
                    }
                }
            }
            let Some(e) = self.entries[i].as_mut() else {
                continue;
            };
            if !e.session.wants_write(&self.config, now) {
                continue;
            }
            let addr = e.addr;
            let mut w = PayloadWriter::new(&mut *buf, self.max);
            if e.session
                .write_payload(&self.config, now, &self.identity, &mut w)
                .is_err()
            {
                self.write_errors = self.write_errors.saturating_add(1);
            }
            let n = w.len();
            self.tidy(i);
            if n > 0 {
                // Round robin: the next call starts at the next entry.
                self.rr = (i + 1) % N;
                return Some((addr, n));
            }
        }
        None
    }
}

// ----- Received -----

/// The result of `receive` for one UDP payload. It is an iterator of the events (it reads
/// one Command at a time); the data deliveries borrow the payload. Then
/// [`Received::write_next`] writes each stateless reply (NAK, idle replies, Bye 0x40), as the
/// zmidi-ci `Received` does. The Commands that the caller does not read are processed by
/// `write_next` or on drop; their events are then lost (counted by `dropped_events`).
#[derive(Debug)]
pub struct Received<
    'e,
    'p,
    Cr: Credentials + ?Sized,
    const N: usize,
    const W: usize,
    const C: usize,
    const Q: usize,
    const P: usize,
    const U: usize,
    const O: usize,
> {
    core: &'e mut Core<N, W, C, Q, P, U, O>,
    creds: &'e mut Cr,
    now: u64,
    from: SocketAddr,
    items: Option<CommandPackets<'p>>,
    first: bool,
    sink: Sink<'p>,
}

impl<
        'e,
        'p,
        Cr: Credentials + ?Sized,
        const N: usize,
        const W: usize,
        const C: usize,
        const Q: usize,
        const P: usize,
        const U: usize,
        const O: usize,
    > Received<'e, 'p, Cr, N, W, C, Q, P, U, O>
{
    fn new(
        core: &'e mut Core<N, W, C, Q, P, U, O>,
        creds: &'e mut Cr,
        now: u64,
        from: SocketAddr,
        payload: &'p [u8],
    ) -> Self {
        core.now = now;
        let mut sink = Sink::new();
        let items = if check_peer(from).is_err() {
            sink.push(Event::diagnostic(from, Diagnostic::InvalidPeerAddress));
            None
        } else {
            match command_packets(payload) {
                Ok(it) => Some(it),
                // REQ-NET-PKT-003: ignore the payload, a diagnostic only.
                Err(_) => {
                    sink.push(Event::diagnostic(from, Diagnostic::BadSignature));
                    None
                }
            }
        };
        Received {
            core,
            creds,
            now,
            from,
            items,
            first: true,
            sink,
        }
    }

    /// The source address of the payload.
    pub fn from(&self) -> SocketAddr {
        self.from
    }

    /// Writes the next stateless reply UDP payload into `buf` and gives its destination and
    /// length; `None` after the last. It first processes the Commands that the caller did
    /// not read.
    pub fn write_next(&mut self, buf: &mut [u8]) -> Option<(SocketAddr, usize)> {
        self.finish();
        self.core.write_out(self.now, buf)
    }

    fn finish(&mut self) {
        while self.next().is_some() {
            self.core.dropped_events = self.core.dropped_events.saturating_add(1);
        }
    }
}

impl<
        'p,
        Cr: Credentials + ?Sized,
        const N: usize,
        const W: usize,
        const C: usize,
        const Q: usize,
        const P: usize,
        const U: usize,
        const O: usize,
    > Iterator for Received<'_, 'p, Cr, N, W, C, Q, P, U, O>
{
    type Item = Event<'p>;

    fn next(&mut self) -> Option<Event<'p>> {
        loop {
            if let Some(e) = self.sink.pop() {
                return Some(e);
            }
            self.core.dropped_events = self
                .core
                .dropped_events
                .saturating_add(core::mem::take(&mut self.sink.dropped));
            let Some(item) = self.items.as_mut()?.next() else {
                self.items = None;
                return None;
            };
            self.core.process(
                self.creds,
                self.now,
                self.from,
                item,
                &mut self.first,
                &mut self.sink,
            );
        }
    }
}

impl<
        Cr: Credentials + ?Sized,
        const N: usize,
        const W: usize,
        const C: usize,
        const Q: usize,
        const P: usize,
        const U: usize,
        const O: usize,
    > Drop for Received<'_, '_, Cr, N, W, C, Q, P, U, O>
{
    fn drop(&mut self) {
        self.finish();
    }
}

// ----- Polled -----

/// The events of `poll`: an iterator that polls one session at a time. The sessions that
/// the caller does not reach are polled on drop; their events wait for the next `poll`.
#[derive(Debug)]
pub struct Polled<
    'e,
    const N: usize,
    const W: usize,
    const C: usize,
    const Q: usize,
    const P: usize,
    const U: usize,
    const O: usize,
> {
    core: &'e mut Core<N, W, C, Q, P, U, O>,
    now: u64,
    next_entry: usize,
    current: Option<(SocketAddr, Events<'static>, usize)>,
}

impl<
        const N: usize,
        const W: usize,
        const C: usize,
        const Q: usize,
        const P: usize,
        const U: usize,
        const O: usize,
    > Iterator for Polled<'_, N, W, C, Q, P, U, O>
{
    type Item = Event<'static>;

    fn next(&mut self) -> Option<Event<'static>> {
        loop {
            if let Some((addr, e)) = self.core.backlog.pop() {
                return Some(Event::session(addr, e));
            }
            if let Some((addr, ev, pos)) = self.current.as_mut() {
                if let Some(e) = ev.iter().nth(*pos) {
                    *pos += 1;
                    return Some(Event::session(*addr, *e));
                }
                self.current = None;
            }
            if self.next_entry >= N {
                return None;
            }
            let i = self.next_entry;
            self.next_entry += 1;
            if let Some((addr, ev)) = self.core.poll_entry(i, self.now) {
                if !ev.is_empty() {
                    self.current = Some((addr, ev, 0));
                }
            }
        }
    }
}

impl<
        const N: usize,
        const W: usize,
        const C: usize,
        const Q: usize,
        const P: usize,
        const U: usize,
        const O: usize,
    > Drop for Polled<'_, N, W, C, Q, P, U, O>
{
    fn drop(&mut self) {
        if let Some((addr, ev, pos)) = self.current.take() {
            for e in ev.iter().skip(pos) {
                self.core.backlog.push(addr, *e);
            }
        }
        while self.next_entry < N {
            let i = self.next_entry;
            self.next_entry += 1;
            if let Some((addr, ev)) = self.core.poll_entry(i, self.now) {
                for e in ev.iter() {
                    self.core.backlog.push(addr, *e);
                }
            }
        }
    }
}

// ----- Host and Client -----

/// The API that Host and Client share.
macro_rules! shared_api {
    () => {
        /// The identity of this instance.
        pub fn identity(&self) -> &Identity {
            &self.core.identity
        }

        /// Reads one received UDP payload from `from` (REQ-NET-GEN-004). The result gives
        /// the events (iterate it) and then the stateless replies
        /// ([`Received::write_next`]). See the module documentation for the checks.
        pub fn receive<'e, 'p, Cr: Credentials + ?Sized>(
            &'e mut self,
            now: u64,
            from: SocketAddr,
            payload: &'p [u8],
            creds: &'e mut Cr,
        ) -> Received<'e, 'p, Cr, N, W, C, Q, P, U, O> {
            Received::new(&mut self.core, creds, now, from, payload)
        }

        /// Runs the timeouts of each session at `now` and gives their events (with the
        /// events of the polls that [`Self::write_pending`] did). Call it at
        /// [`Self::next_deadline`] and before [`Self::write_pending`].
        pub fn poll(&mut self, now: u64) -> Polled<'_, N, W, C, Q, P, U, O> {
            self.core.now = now;
            Polled {
                core: &mut self.core,
                now,
                next_entry: 0,
                current: None,
            }
        }

        /// The earliest time at which `poll` or `write_pending` has work; `None`: nothing
        /// to do.
        pub fn next_deadline(&self) -> Option<u64> {
            self.core.next_deadline()
        }

        /// Writes the next UDP payload into `buf`: first the stateless Commands, then the
        /// sessions in turn (round robin). Gives the destination and the length, `None` when
        /// nothing waits. Call it until `None`.
        pub fn write_pending(&mut self, now: u64, buf: &mut [u8]) -> Option<(SocketAddr, usize)> {
            self.core.write_pending(now, buf)
        }

        /// Writes a Ping to `addr` in any session state, also with no session
        /// (REQ-NET-PING-003); `None` takes the Ping Id from the counter of this instance,
        /// which increments by 1 for each Ping. Gives the Ping Id. A Ping changes no session
        /// state. The Session refuses a Ping in Pending Bye (REQ-NET-BYE-013) and after NAK
        /// 0x01 for Ping (REQ-NET-CMD-003).
        pub fn ping(
            &mut self,
            now: u64,
            addr: SocketAddr,
            id: Option<u32>,
        ) -> Result<u32, EndpointError> {
            self.core.ping(now, addr, id)
        }

        /// Queues whole UMP messages for the peer `addr` (REQ-NET-DATA-002).
        pub fn send_ump(&mut self, addr: SocketAddr, words: &[u32]) -> Result<(), EndpointError> {
            self.core.send_ump(addr, words)
        }

        /// Ends the session with `addr` with a Bye (REQ-NET-BYE-006).
        pub fn bye(
            &mut self,
            now: u64,
            addr: SocketAddr,
            reason: ByeReason,
            text: &str,
        ) -> Result<Events<'static>, EndpointError> {
            self.core.bye(now, addr, reason, text)
        }

        /// Cancels the pending Invitation with `addr` (REQ-NET-INV-010).
        pub fn cancel(
            &mut self,
            now: u64,
            addr: SocketAddr,
        ) -> Result<Events<'static>, EndpointError> {
            self.core.cancel(now, addr)
        }

        /// Starts a Session Reset with `addr` (REQ-NET-RST-003).
        pub fn reset(&mut self, now: u64, addr: SocketAddr) -> Result<(), EndpointError> {
            self.core.reset(now, addr)
        }

        /// The session of `addr`, if any, as a read-only [`SessionRef`]: the instance keeps one
        /// configuration for all its sessions (OPT-009).
        pub fn session(&self, addr: SocketAddr) -> Option<SessionRef<'_, W, C, Q, P, U>> {
            self.core.session(addr)
        }

        /// The session entries with their peer addresses, as read-only [`SessionRef`] views.
        pub fn sessions(
            &self,
        ) -> impl Iterator<Item = (SocketAddr, SessionRef<'_, W, C, Q, P, U>)> {
            self.core.sessions()
        }

        /// The number of sessions that are not in Idle: each uses one place of the table
        /// (REQ-NET-GEN-001).
        pub fn active_sessions(&self) -> usize {
            self.core.active()
        }

        /// The events that the instance dropped (not read from [`Received`], full queues).
        pub fn dropped_events(&self) -> u32 {
            self.core
                .dropped_events
                .saturating_add(self.core.backlog.dropped)
        }

        /// The stateless Commands that the full queue dropped.
        pub fn dropped_replies(&self) -> u32 {
            self.core.out.dropped
        }
    };
}

/// A Network MIDI 2.0 Host: one local UDP port, one UMP Endpoint, at most `S` Client
/// sessions (REQ-NET-GEN-001, -004). W, C, Q, P and U are the buffer sizes of each
/// [`Session`] (receive window, send history in words, Retransmit Request queue, open Pings,
/// Username capacity); O is the capacity of the stateless Command queue and of the event
/// queue ([`STATELESS_CAPACITY`]).
#[derive(Debug, Clone)]
pub struct Host<
    const S: usize = DEFAULT_HOST_SESSIONS,
    const W: usize = 64,
    const C: usize = 256,
    const Q: usize = 4,
    const P: usize = 8,
    const U: usize = DEFAULT_USERNAME_CAPACITY,
    const O: usize = STATELESS_CAPACITY,
> {
    core: Core<S, W, C, Q, P, U, O>,
}

/// A Host with the default sizes.
pub type DefaultHost = Host;

impl<
        const N: usize,
        const W: usize,
        const C: usize,
        const Q: usize,
        const P: usize,
        const U: usize,
        const O: usize,
    > Host<N, W, C, Q, P, U, O>
{
    /// A Host with `config`; the session configuration is checked. The Host keeps its own
    /// policy and its own failure delay (REQ-NET-GEN-001, N-45). See [`Host::init`] for a Host
    /// in a `static`.
    pub fn new(config: HostConfig) -> Result<Self, EndpointError> {
        let session =
            Core::<N, W, C, Q, P, U, O>::check_config(config.session, config.max_payload)?;
        Ok(Self::build(config, session))
    }

    /// Writes a Host with `config` into `slot` and gives it (see [`Host::new`]). The Host is
    /// built in `slot`, not on the stack and then copied, so the stack needs no room for it
    /// (OPT-003). On an error `slot` does not change.
    ///
    /// In a `static`, `Option<Host> = None` is not all zero bits, so the linker puts the
    /// static in `.data`, and its initial image also uses flash (OPT-013). A static of
    /// `core::mem::MaybeUninit`, for example a `static_cell::StaticCell<Option<Host>>`, is in
    /// `.bss` and uses no flash: `Host::init(CELL.init(None), config)`.
    pub fn init(slot: &mut Option<Self>, config: HostConfig) -> Result<&mut Self, EndpointError> {
        let session =
            Core::<N, W, C, Q, P, U, O>::check_config(config.session, config.max_payload)?;
        Ok(slot.insert(Self::build(config, session)))
    }

    #[inline(always)]
    fn build(config: HostConfig, session: SessionConfig) -> Self {
        Host {
            core: Core::build(
                Role::Host,
                config.identity,
                session,
                Capabilities::default(),
                config.policy,
                FailureDelay::new(config.failure_delay_initial_ms, config.failure_delay_max_ms),
                config.max_payload,
                config.first_ping_id,
            ),
        }
    }

    shared_api!();

    /// Accepts the Client `addr` after [`SessionEvent::InvitationReceived`]
    /// (REQ-NET-INV-026).
    pub fn accept(&mut self, now: u64, addr: SocketAddr) -> Result<Events<'static>, EndpointError> {
        self.core.accept(now, addr)
    }

    /// Rejects the Client `addr`; `None` gives Bye 0x42 (REQ-NET-INV-026, N-56).
    pub fn reject(
        &mut self,
        now: u64,
        addr: SocketAddr,
        reason: Option<ByeReason>,
    ) -> Result<Events<'static>, EndpointError> {
        self.core.reject(now, addr, reason)
    }

    /// The policy for new Clients.
    pub fn policy(&self) -> &HostPolicy {
        &self.core.policy
    }

    /// The DNS-SD PTR, SRV and TXT records of this Host, with the same identity that the
    /// Host writes in each Invitation Reply (REQ-NET-DISC-013, REQ-NET-TOPO-005, N-60).
    pub fn host_records(
        &self,
        instance_name: &str,
        port: u16,
        hostname: &str,
    ) -> Result<HostRecords, EndpointError> {
        Ok(HostRecords::new(
            ServiceInstanceName::new(instance_name)?,
            self.core.identity,
            port,
            Hostname::new(hostname)?,
        ))
    }
}

/// A Network MIDI 2.0 Client: one local UDP port, at most `M` sessions with different Host
/// addresses (REQ-NET-GEN-002). W, C, Q, P and U are the buffer sizes of each [`Session`]; O
/// is the capacity of the stateless Command queue and of the event queue
/// ([`STATELESS_CAPACITY`]).
#[derive(Debug, Clone)]
pub struct Client<
    const M: usize = DEFAULT_CLIENT_SESSIONS,
    const W: usize = 64,
    const C: usize = 256,
    const Q: usize = 4,
    const P: usize = 8,
    const U: usize = DEFAULT_USERNAME_CAPACITY,
    const O: usize = STATELESS_CAPACITY,
> {
    core: Core<M, W, C, Q, P, U, O>,
}

/// A Client with the default sizes.
pub type DefaultClient = Client;

impl<
        const N: usize,
        const W: usize,
        const C: usize,
        const Q: usize,
        const P: usize,
        const U: usize,
        const O: usize,
    > Client<N, W, C, Q, P, U, O>
{
    /// A Client with `config`; the session configuration is checked. See [`Client::init`] for
    /// a Client in a `static`.
    pub fn new(config: ClientConfig) -> Result<Self, EndpointError> {
        let session =
            Core::<N, W, C, Q, P, U, O>::check_config(config.session, config.max_payload)?;
        Ok(Self::build(config, session))
    }

    /// Writes a Client with `config` into `slot` and gives it (see [`Client::new`]). The Client
    /// is built in `slot`, not on the stack and then copied (OPT-003). On an error `slot` does
    /// not change. For a `static`, see the note on [`Host::init`] (OPT-013).
    pub fn init(slot: &mut Option<Self>, config: ClientConfig) -> Result<&mut Self, EndpointError> {
        let session =
            Core::<N, W, C, Q, P, U, O>::check_config(config.session, config.max_payload)?;
        Ok(slot.insert(Self::build(config, session)))
    }

    #[inline(always)]
    fn build(config: ClientConfig, session: SessionConfig) -> Self {
        Client {
            core: Core::build(
                Role::Client,
                config.identity,
                session,
                config.capabilities,
                HostPolicy::DEFAULT,
                FailureDelay::new(0, 0),
                config.max_payload,
                config.first_ping_id,
            ),
        }
    }

    shared_api!();

    /// Starts a session with the Host at `addr`: the Invitation goes in the next UDP payload
    /// of [`Client::write_pending`] and the session goes to Pending Invitation
    /// (REQ-NET-INV-005, -006). It needs no DNS-SD data (REQ-NET-DISC-001). `Busy` when the
    /// session with `addr` is not in Idle, `TableFull` when all `M` places are in use; no
    /// Invitation then (REQ-NET-GEN-002).
    pub fn connect(&mut self, now: u64, addr: SocketAddr) -> Result<(), EndpointError> {
        self.core.connect(now, addr)
    }

    /// Answers the challenge of the Host `addr` later, after [`Credentials::answer_now`]
    /// gave false (REQ-NET-AUTH-034, -035).
    pub fn answer<Cr: Credentials + ?Sized>(
        &mut self,
        now: u64,
        addr: SocketAddr,
        creds: &mut Cr,
    ) -> Result<Events<'static>, EndpointError> {
        self.core.answer(now, addr, creds)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::command::{Bye, InvitationReply, Nak};
    use crate::handshake::{Method, PolicyKind, NONCE_RANDOM_LEN};
    use crate::packet::RawCommand;
    use crate::session::CloseCause;
    use crate::stream::StreamConfig;
    use proptest::prelude::*;
    use std::boxed::Box;
    use std::collections::VecDeque;
    use std::format;
    use std::string::String;
    use std::vec;
    use std::vec::Vec;

    const NOTE: u32 = 0x2090_3C40; // MIDI 1.0 Note On, 1 word (MT 0x2)
    const NOTE2: u32 = 0x2080_3C00;

    /// Appendix A.1.1 (Figure 13): Invitation, "MyDev", "8shYe3h5".
    const A11: [u8; 24] = [
        b'M', b'I', b'D', b'I', 0x01, 0x04, 0x02, 0x00, b'M', b'y', b'D', b'e', b'v', 0, 0, 0,
        b'8', b's', b'h', b'Y', b'e', b'3', b'h', b'5',
    ];

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    fn midi(bytes: &[u8]) -> Vec<u8> {
        let mut p = Vec::from(*b"MIDI");
        p.extend_from_slice(bytes);
        p
    }

    /// Test credentials.
    #[derive(Default, Clone)]
    struct Creds {
        random: Option<[u8; NONCE_RANDOM_LEN]>,
        secret: Option<Vec<u8>>,
    }

    impl Credentials for Creds {
        fn random_nonce(&mut self) -> Option<[u8; NONCE_RANDOM_LEN]> {
            self.random
        }
        fn shared_secret(&mut self) -> Option<&[u8]> {
            self.secret.as_deref()
        }
        fn password(&mut self, _username: &[u8]) -> Option<&[u8]> {
            None
        }
        fn user(&mut self) -> Option<(&[u8], &[u8])> {
            None
        }
    }

    /// An owned copy of an Event, for the asserts.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Ev {
        Invitation,
        Pending,
        Established(Option<Method>),
        Ending(u8),
        ClosedBye(u8),
        ClosedByeReply,
        Closed(String),
        Ping(u32),
        Nak(u8, u8),
        Data(Vec<u32>, bool),
        Diag(Diagnostic),
        Other(String),
    }

    fn own(e: &Event<'_>) -> (SocketAddr, Ev) {
        let ev = match e.kind {
            EventKind::Session(s) => match s {
                SessionEvent::InvitationReceived { .. } => Ev::Invitation,
                SessionEvent::Pending => Ev::Pending,
                SessionEvent::Established { method } => Ev::Established(method),
                SessionEvent::Ending(r) => Ev::Ending(r.0),
                SessionEvent::Closed(CloseCause::ByeReceived(b)) => Ev::ClosedBye(b.reason.0),
                SessionEvent::Closed(CloseCause::ByeReply) => Ev::ClosedByeReply,
                SessionEvent::Closed(c) => Ev::Closed(format!("{c:?}")),
                SessionEvent::PingReply { id, .. } => Ev::Ping(id),
                SessionEvent::Nak(n) => Ev::Nak(n.reason.0, n.nak_header.code),
                other => Ev::Other(format!("{other:?}")),
            },
            EventKind::Data(d) => Ev::Data(
                d.messages().flat_map(|m| m.words().to_vec()).collect(),
                d.is_late(),
            ),
            EventKind::Diagnostic(x) => Ev::Diag(x),
        };
        (e.peer, ev)
    }

    type Log = Vec<(SocketAddr, Ev)>;
    type Wire = Vec<(SocketAddr, Vec<u8>)>;

    /// The caller loop of one instance, for the tests.
    trait Ep {
        fn rx(&mut self, now: u64, from: SocketAddr, p: &[u8], creds: &mut Creds) -> (Log, Wire);
        fn tx(&mut self, now: u64) -> (Log, Wire);
    }

    macro_rules! impl_ep {
        ($t:ident) => {
            impl<
                    const N: usize,
                    const W: usize,
                    const C: usize,
                    const Q: usize,
                    const P: usize,
                    const U: usize,
                    const O: usize,
                > Ep for $t<N, W, C, Q, P, U, O>
            {
                fn rx(
                    &mut self,
                    now: u64,
                    from: SocketAddr,
                    p: &[u8],
                    creds: &mut Creds,
                ) -> (Log, Wire) {
                    let mut log = Vec::new();
                    let mut out = Vec::new();
                    let mut r = self.receive(now, from, p, creds);
                    for e in r.by_ref() {
                        log.push(own(&e));
                    }
                    let mut buf = [0u8; 1400];
                    while let Some((a, n)) = r.write_next(&mut buf) {
                        out.push((a, buf[..n].to_vec()));
                    }
                    (log, out)
                }

                fn tx(&mut self, now: u64) -> (Log, Wire) {
                    let log = self.poll(now).map(|e| own(&e)).collect();
                    let mut out = Vec::new();
                    let mut buf = [0u8; 1400];
                    for _ in 0..1000 {
                        match self.write_pending(now, &mut buf) {
                            Some((a, n)) => out.push((a, buf[..n].to_vec())),
                            None => break,
                        }
                    }
                    (log, out)
                }
            }
        };
    }
    impl_ep!(Host);
    impl_ep!(Client);

    fn host_id() -> Identity {
        Identity::new("Host", "H1").unwrap()
    }

    fn client_id() -> Identity {
        Identity::new("MyDev", "8shYe3h5").unwrap()
    }

    /// No automatic Ping, so that the asserts see only the test traffic.
    fn quiet() -> SessionConfig {
        SessionConfig {
            ping_interval_ms: 0,
            ..SessionConfig::DEFAULT
        }
    }

    fn host_cfg() -> HostConfig {
        HostConfig {
            session: quiet(),
            ..HostConfig::new(host_id())
        }
    }

    fn client_cfg() -> ClientConfig {
        ClientConfig {
            session: quiet(),
            ..ClientConfig::new(client_id())
        }
    }

    struct Node {
        addr: SocketAddr,
        c: Client,
        creds: Creds,
        log: Log,
    }

    type DropRule = Box<dyn FnMut(SocketAddr, SocketAddr, &[u8]) -> bool>;

    /// One Host<2> and its Clients on a fake network with a fake clock.
    struct World {
        now: u64,
        haddr: SocketAddr,
        host: Host<2>,
        hcreds: Creds,
        hlog: Log,
        nodes: Vec<Node>,
        drop: DropRule,
        dropped: usize,
        trace: Vec<(SocketAddr, SocketAddr, Vec<u8>)>,
    }

    impl World {
        fn new(cfg: HostConfig) -> Self {
            World {
                now: 1000,
                haddr: addr("10.0.0.1:5673"),
                host: Host::new(cfg).unwrap(),
                hcreds: Creds::default(),
                hlog: Vec::new(),
                nodes: Vec::new(),
                drop: Box::new(|_, _, _| false),
                dropped: 0,
                trace: Vec::new(),
            }
        }

        fn add(&mut self, a: &str, cfg: ClientConfig) -> usize {
            self.nodes.push(Node {
                addr: addr(a),
                c: Client::new(cfg).unwrap(),
                creds: Creds::default(),
                log: Vec::new(),
            });
            self.nodes.len() - 1
        }

        fn connect(&mut self, i: usize) {
            let (now, h) = (self.now, self.haddr);
            self.nodes[i].c.connect(now, h).unwrap();
        }

        fn step(&mut self) {
            let now = self.now;
            let mut wire: VecDeque<(SocketAddr, SocketAddr, Vec<u8>)> = VecDeque::new();
            let (log, out) = self.host.tx(now);
            self.hlog.extend(log);
            wire.extend(out.into_iter().map(|(d, b)| (self.haddr, d, b)));
            for n in self.nodes.iter_mut() {
                let (log, out) = n.c.tx(now);
                n.log.extend(log);
                wire.extend(out.into_iter().map(|(d, b)| (n.addr, d, b)));
            }
            while let Some((s, d, b)) = wire.pop_front() {
                if (self.drop)(s, d, &b) {
                    self.dropped += 1;
                    continue;
                }
                self.trace.push((s, d, b.clone()));
                if d == self.haddr {
                    let (log, out) = self.host.rx(now, s, &b, &mut self.hcreds);
                    self.hlog.extend(log);
                    wire.extend(out.into_iter().map(|(x, y)| (self.haddr, x, y)));
                } else if let Some(n) = self.nodes.iter_mut().find(|n| n.addr == d) {
                    let (log, out) = n.c.rx(now, s, &b, &mut n.creds);
                    n.log.extend(log);
                    wire.extend(out.into_iter().map(|(x, y)| (n.addr, x, y)));
                }
            }
            self.now += 1;
        }

        fn run(&mut self, ms: u64) {
            for _ in 0..ms {
                self.step();
            }
        }

        fn host_has(&self, peer: SocketAddr, ev: &Ev) -> bool {
            self.hlog.iter().any(|(a, e)| *a == peer && e == ev)
        }

        fn node_has(&self, i: usize, ev: &Ev) -> bool {
            self.nodes[i].log.iter().any(|(_, e)| e == ev)
        }

        fn host_data(&self, peer: SocketAddr) -> Vec<u32> {
            data_of(&self.hlog, peer)
        }
    }

    fn data_of(log: &Log, peer: SocketAddr) -> Vec<u32> {
        log.iter()
            .filter(|(a, _)| *a == peer)
            .filter_map(|(_, e)| match e {
                Ev::Data(w, _) => Some(w.clone()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn commands(p: &[u8]) -> Vec<RawCommand<'_>> {
        command_packets(p)
            .unwrap()
            .filter_map(|i| match i {
                PacketItem::Command(c) => Some(c),
                PacketItem::Truncated(_) => None,
            })
            .collect()
    }

    fn established_world(clients: usize) -> World {
        let mut w = World::new(host_cfg());
        for k in 0..clients {
            let i = w.add(&format!("10.0.0.{}:6000", k + 2), client_cfg());
            w.connect(i);
        }
        w.run(20);
        for k in 0..clients {
            assert!(
                w.node_has(k, &Ev::Established(None)),
                "{:?}",
                w.nodes[k].log
            );
        }
        w
    }

    // ----- REQ-NET-INV-005, -006, REQ-NET-DISC-001 -----

    #[test]
    fn connect_writes_the_invitation_of_appendix_a11_to_the_address() {
        // REQ-NET-INV-005, -006, REQ-NET-DISC-001: an address only, no DNS-SD data.
        let mut c: Client = Client::new(ClientConfig::new(client_id())).unwrap();
        let h = addr("192.168.1.20:5673");
        c.connect(0, h).unwrap();
        assert_eq!(
            c.session(h).unwrap().state(),
            SessionState::PendingInvitation
        );
        let mut buf = [0u8; 1400];
        let (to, n) = c.write_pending(0, &mut buf).unwrap();
        assert_eq!(to, h);
        assert_eq!(&buf[..n], &A11);
        assert_eq!(c.write_pending(0, &mut buf), None);
    }

    #[test]
    fn each_invitation_repeat_holds_the_caller_identity() {
        // REQ-NET-INV-006: the repeats carry the same name and id.
        let mut c: Client = Client::new(ClientConfig::new(client_id())).unwrap();
        let h = addr("192.168.1.20:5673");
        c.connect(0, h).unwrap();
        let mut sent = 0;
        let mut buf = [0u8; 1400];
        for now in 0..2000 {
            let _ = c.poll(now).count();
            while let Some((to, n)) = c.write_pending(now, &mut buf) {
                assert_eq!(to, h);
                assert_eq!(&buf[..n], &A11);
                sent += 1;
            }
        }
        assert!(sent >= 4, "{sent}");
    }

    // ----- REQ-NET-GEN-002 -----

    #[test]
    fn client_refuses_a_second_session_with_one_host_address() {
        let mut c: Client = Client::new(client_cfg()).unwrap();
        let h = addr("10.0.0.1:5673");
        c.connect(0, h).unwrap();
        let mut buf = [0u8; 1400];
        while c.write_pending(0, &mut buf).is_some() {}
        assert_eq!(
            c.connect(1, h),
            Err(EndpointError::Busy(SessionState::PendingInvitation))
        );
        // No Invitation for the refused request.
        assert_eq!(c.write_pending(1, &mut buf), None);
    }

    #[test]
    fn client_sessions_are_limited_to_m_host_addresses() {
        let mut c: Client<2> = Client::new(client_cfg()).unwrap();
        c.connect(0, addr("10.0.0.1:5673")).unwrap();
        c.connect(0, addr("10.0.0.9:5673")).unwrap();
        assert_eq!(
            c.connect(0, addr("10.0.0.7:5673")),
            Err(EndpointError::TableFull)
        );
        let mut buf = [0u8; 1400];
        let mut to = Vec::new();
        while let Some((a, _)) = c.write_pending(0, &mut buf) {
            to.push(a);
        }
        to.sort();
        assert_eq!(to, [addr("10.0.0.1:5673"), addr("10.0.0.9:5673")]);
        // The default Client has one place.
        let mut d: Client = Client::new(client_cfg()).unwrap();
        d.connect(0, addr("10.0.0.1:5673")).unwrap();
        assert_eq!(
            d.connect(0, addr("10.0.0.9:5673")),
            Err(EndpointError::TableFull)
        );
    }

    #[test]
    fn client_reconnects_after_the_session_ends() {
        // REQ-NET-GEN-002: only a session that is not in Idle blocks a new one.
        let mut w = established_world(1);
        let (now, h) = (w.now, w.haddr);
        w.nodes[0]
            .c
            .bye(now, h, ByeReason::USER_TERMINATED, "")
            .unwrap();
        w.run(10);
        assert!(w.node_has(0, &Ev::ClosedByeReply));
        assert!(w.nodes[0].c.session(h).is_none());
        w.nodes[0].log.clear();
        w.connect(0);
        w.run(10);
        assert!(w.node_has(0, &Ev::Established(None)));
    }

    // ----- REQ-NET-CMD-002 -----

    #[test]
    fn unknown_command_code_gets_nak_01_with_no_session() {
        // Test vector of REQ-NET-CMD-002.
        let mut h: Host = Host::new(host_cfg()).unwrap();
        let p = midi(&[0x55, 0x02, 0x12, 0x34, 1, 2, 3, 4, 5, 6, 7, 8]);
        let (log, out) = h.rx(0, addr("10.0.0.2:6000"), &p, &mut Creds::default());
        assert_eq!(
            out,
            [(
                addr("10.0.0.2:6000"),
                midi(&[0x8F, 0x01, 0x01, 0x00, 0x55, 0x02, 0x12, 0x34])
            )]
        );
        assert!(matches!(
            log[..],
            [(_, Ev::Diag(Diagnostic::NakSent { reason, .. }))] if reason == NakReason::COMMAND_NOT_SUPPORTED
        ));
        // A Client does the same.
        let mut c: Client = Client::new(client_cfg()).unwrap();
        let (_, out) = c.rx(0, addr("10.0.0.1:5673"), &p, &mut Creds::default());
        assert_eq!(
            out[0].1,
            midi(&[0x8F, 0x01, 0x01, 0x00, 0x55, 0x02, 0x12, 0x34])
        );
    }

    #[test]
    fn unknown_command_code_gets_nak_01_in_a_session() {
        let mut w = established_world(1);
        let c = w.nodes[0].addr;
        let p = midi(&[0x55, 0x02, 0x12, 0x34, 1, 2, 3, 4, 5, 6, 7, 8]);
        let (_, out) = w.host.rx(w.now, c, &p, &mut Creds::default());
        assert_eq!(
            out,
            [(c, midi(&[0x8F, 0x01, 0x01, 0x00, 0x55, 0x02, 0x12, 0x34]))]
        );
        assert_eq!(
            w.host.session(c).unwrap().state(),
            SessionState::EstablishedSession
        );
    }

    // ----- REQ-NET-NAK-004 -----

    #[test]
    fn short_and_truncated_commands_get_nak_03() {
        let mut h: Host = Host::new(host_cfg()).unwrap();
        let from = addr("10.0.0.2:6000");
        let mut cr = Creds::default();
        // Ping with pl = 0.
        let (_, out) = h.rx(0, from, &midi(&[0x20, 0, 0, 0]), &mut cr);
        assert_eq!(
            out,
            [(
                from,
                midi(&[0x8F, 0x01, 0x03, 0x00, 0x20, 0x00, 0x00, 0x00])
            )]
        );
        // Invitation whose name goes past the payload (csd1 > pl, REQ-NET-INV-032).
        let (_, out) = h.rx(
            0,
            from,
            &midi(&[0x01, 0x01, 0x02, 0x00, b'a', 0, 0, 0]),
            &mut cr,
        );
        assert_eq!(
            out[0].1,
            midi(&[0x8F, 0x01, 0x03, 0x00, 0x01, 0x01, 0x02, 0x00])
        );
        assert_eq!(h.active_sessions(), 0);
        // A Command Packet that goes past the payload (REQ-NET-PKT-014): the Commands
        // before it are kept (the Ping gets its reply), then NAK 0x03.
        let p = midi(&[0x20, 0x01, 0, 0, 0, 0, 0, 7, 0x20, 0x05, 0, 0, 1, 2, 3, 4]);
        let (_, out) = h.rx(0, from, &p, &mut cr);
        assert_eq!(
            out,
            [(
                from,
                midi(&[0x21, 0x01, 0, 0, 0, 0, 0, 7, 0x8F, 0x01, 0x03, 0x00, 0x20, 0x05, 0, 0])
            )]
        );
    }

    #[test]
    fn invitation_reply_with_bad_lengths_gets_nak_03() {
        // REQ-NET-INV-034 at a Client in Pending Invitation.
        let mut c: Client = Client::new(client_cfg()).unwrap();
        let h = addr("10.0.0.1:5673");
        c.connect(0, h).unwrap();
        let _ = c.tx(0);
        let (_, out) = c.rx(
            1,
            h,
            &midi(&[0x10, 0x00, 0x01, 0x00]),
            &mut Creds::default(),
        );
        assert_eq!(
            out,
            [(h, midi(&[0x8F, 0x01, 0x03, 0x00, 0x10, 0x00, 0x01, 0x00]))]
        );
        assert_eq!(
            c.session(h).unwrap().state(),
            SessionState::PendingInvitation
        );
    }

    #[test]
    fn retransmit_commands_with_pl_0_get_nak_03_and_long_ump_data_gets_none() {
        // REQ-NET-RTX-001, REQ-NET-RTX-014: pl 0 is Command Malformed.
        let mut w = established_world(1);
        let c = w.nodes[0].addr;
        for code in [0x80u8, 0x81] {
            let (_, out) = w
                .host
                .rx(w.now, c, &midi(&[code, 0, 0, 0]), &mut Creds::default());
            assert_eq!(
                out,
                [(c, midi(&[0x8F, 0x01, 0x03, 0x00, code, 0x00, 0x00, 0x00]))],
                "{code:#04x}"
            );
        }
        // REQ-NET-DATA-001: pl 65 is accepted with no NAK.
        let mut p = vec![0xFF, 65, 0x00, 0x00];
        for _ in 0..65 {
            p.extend(NOTE.to_be_bytes());
        }
        let (log, out) = w.host.rx(w.now, c, &midi(&p), &mut Creds::default());
        assert_eq!(data_of(&log, c), [NOTE; 65]);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn ump_data_cut_inside_a_message_is_delivered_with_one_nak_03() {
        // REQ-NET-DATA-006 with REQ-NET-NAK-004: whole messages first, one NAK, the repeat
        // of the same Command gets no second NAK.
        let mut w = established_world(1);
        let c = w.nodes[0].addr;
        // NOTE, then the first word of a 2-word message (MT 0x4).
        let p = midi(&[
            0xFF, 0x02, 0x00, 0x00, 0x20, 0x90, 0x3C, 0x40, 0x40, 0x90, 0x3C, 0x00,
        ]);
        let (log, out) = w.host.rx(w.now, c, &p, &mut Creds::default());
        assert!(log.contains(&(c, Ev::Data(vec![NOTE], false))), "{log:?}");
        assert_eq!(
            out,
            [(c, midi(&[0x8F, 0x01, 0x03, 0x00, 0xFF, 0x02, 0x00, 0x00]))]
        );
        let (log, out) = w.host.rx(w.now, c, &p, &mut Creds::default());
        assert!(out.is_empty(), "{out:?}");
        assert!(data_of(&log, c).is_empty());
        // With no session the cut check does not apply: UMP Data from an address with no
        // entry gets Bye 0x05 (REQ-NET-DATA-015), no NAK 0x03.
        let other = addr("10.0.0.99:6000");
        let (log, out) = w.host.rx(w.now, other, &p, &mut Creds::default());
        assert!(data_of(&log, other).is_empty());
        assert_eq!(out, [(other, midi(&[0xF0, 0x00, 0x05, 0x00]))]);
    }

    // ----- review fixes -----

    #[test]
    fn cut_ump_data_in_pending_session_reset_is_discarded() {
        // E2: the cut check is for Established Session only; in Pending Session Reset the
        // UMP Data is discarded (REQ-NET-RST-006): no NAK 0x03, no data.
        let mut w = established_world(1);
        let c = w.nodes[0].addr;
        w.host.reset(w.now, c).unwrap();
        let p = midi(&[
            0xFF, 0x02, 0x00, 0x00, 0x20, 0x90, 0x3C, 0x40, 0x40, 0x90, 0x3C, 0x00,
        ]);
        let (log, out) = w.host.rx(w.now, c, &p, &mut Creds::default());
        assert!(out.is_empty(), "{out:?}");
        assert!(data_of(&log, c).is_empty());
        assert!(
            !log.iter().any(|(_, e)| matches!(e, Ev::Diag(_))),
            "{log:?}"
        );
    }

    #[test]
    fn idle_entry_that_owes_a_bye_reply_is_not_a_free_place() {
        // E5: Host<1>: the Bye Reply of the first Client is written before its place is
        // used again.
        let mut h: Host<1> = Host::new(host_cfg()).unwrap();
        let mut cr = Creds::default();
        let a = addr("10.0.0.2:6000");
        let b = addr("10.0.0.3:6000");
        h.rx(0, a, &A11, &mut cr);
        assert_eq!(h.active_sessions(), 1);
        h.tx(0);
        h.rx(1, a, &midi(&[0xF0, 0x00, 0x00, 0x00]), &mut cr);
        assert_eq!(h.active_sessions(), 0);
        let (log, out) = h.rx(1, b, &A11, &mut cr);
        assert_eq!(out, [(b, midi(&[0xF0, 0, 0x40, 0]))]);
        assert!(log.contains(&(b, Ev::Diag(Diagnostic::TableFull))));
        let (_, out) = h.tx(1);
        assert_eq!(out, [(a, midi(&[0xF1, 0, 0, 0]))]);
        let (log, out) = h.rx(2, b, &A11, &mut cr);
        assert!(out.is_empty(), "{out:?}");
        assert!(log.contains(&(b, Ev::Established(None))));
        // Client<1>: the same rule for connect.
        let mut w = established_world(1);
        let now = w.now;
        let (haddr, other) = (w.haddr, addr("10.0.0.50:5673"));
        w.host
            .bye(now, w.nodes[0].addr, ByeReason::USER_TERMINATED, "")
            .unwrap();
        let (_, out) = w.host.tx(now);
        let n = &mut w.nodes[0];
        for (_, p) in out {
            n.c.rx(now, haddr, &p, &mut n.creds);
        }
        assert_eq!(n.c.active_sessions(), 0);
        assert_eq!(n.c.connect(now, other), Err(EndpointError::TableFull));
        let (_, out) = n.c.tx(now);
        assert_eq!(out, [(haddr, midi(&[0xF1, 0, 0, 0]))]);
        assert_eq!(n.c.connect(now, other), Ok(()));
    }

    #[cfg(feature = "auth")]
    #[test]
    fn idle_bye_45_gives_the_credentials_error_event() {
        // E6: the Handshake events of an idle reply reach the caller.
        let mut h: Host = Host::new(HostConfig {
            policy: HostPolicy {
                pending_fallback: false,
                ..HostPolicy::new(PolicyKind::SharedSecret)
            },
            ..host_cfg()
        })
        .unwrap();
        let from = addr("10.0.0.2:6000");
        let mut inv = A11;
        inv[7] = 0x01; // Capabilities: shared secret
        let (log, out) = h.rx(0, from, &inv, &mut Creds::default());
        assert_eq!(out, [(from, midi(&[0xF0, 0, 0x45, 0]))]);
        assert!(log
            .iter()
            .any(|(a, e)| *a == from
                && matches!(e, Ev::Other(s) if s.starts_with("CredentialsError"))));
    }

    #[test]
    fn caller_ping_waiting_to_be_written_is_busy() {
        // E3 through the endpoint: the counter does not move for a refused Ping.
        let mut w = established_world(1);
        let c = w.nodes[0].addr;
        assert_eq!(w.host.ping(w.now, c, None), Ok(0));
        assert_eq!(
            w.host.ping(w.now, c, None),
            Err(EndpointError::Session(SessionError::Busy))
        );
        w.host.tx(w.now);
        assert_eq!(w.host.ping(w.now, c, None), Ok(1));
    }

    #[test]
    fn init_writes_the_same_instance_as_new_into_the_slot() {
        // OPT-003: `init` builds in the slot; the result is the same as `new`.
        let mut slot: Option<Host<2>> = None;
        let h = Host::<2>::init(&mut slot, host_cfg()).unwrap();
        assert_eq!(h.identity(), &host_id());
        assert_eq!(h.active_sessions(), 0);
        let made = Host::<2>::new(host_cfg()).unwrap();
        let held = slot.as_ref().unwrap();
        assert_eq!(held.identity(), made.identity());
        assert_eq!(held.policy(), made.policy());
        assert_eq!(held.next_deadline(), made.next_deadline());

        let mut slot: Option<Client> = None;
        let c = Client::init(&mut slot, client_cfg()).unwrap();
        let hp = addr("10.0.0.1:5673");
        c.connect(0, hp).unwrap();
        assert_eq!(
            c.session(hp).unwrap().state(),
            SessionState::PendingInvitation
        );

        // A configuration that `new` refuses: `init` refuses it and `slot` does not change.
        let bad = HostConfig {
            max_payload: MaxPayload::new(11).unwrap(),
            ..host_cfg()
        };
        let mut slot: Option<Host<1>> = None;
        assert!(Host::<1>::init(&mut slot, bad).is_err());
        assert!(slot.is_none());
        let mut slot = Some(Host::<1>::new(host_cfg()).unwrap());
        assert!(Host::<1>::init(&mut slot, bad).is_err());
        assert_eq!(slot.as_ref().unwrap().identity(), &host_id());
    }

    #[test]
    fn command_size_follows_the_maximum_payload() {
        // S3: (max payload - 8) / 4 words, at most 64; no room for one word is refused.
        let words = |bytes: usize| {
            let mut c: Client = Client::new(ClientConfig {
                max_payload: MaxPayload::new(bytes).unwrap(),
                ..client_cfg()
            })
            .unwrap();
            let h = addr("10.0.0.1:5673");
            c.connect(0, h).unwrap();
            c.session(h).unwrap().config().stream.max_command_words
        };
        assert_eq!(words(1400), 64);
        assert_eq!(words(264), 64);
        assert_eq!(words(100), 23);
        assert_eq!(words(12), 1);
        assert!(matches!(
            Host::<1>::new(HostConfig {
                max_payload: MaxPayload::new(11).unwrap(),
                ..host_cfg()
            }),
            Err(EndpointError::Session(SessionError::Stream(
                crate::stream::StreamError::CommandWords
            )))
        ));
        // Data through a small payload: every Command fits, nothing is lost.
        let mut w = World::new(HostConfig {
            max_payload: MaxPayload::new(40).unwrap(),
            ..host_cfg()
        });
        let i = w.add(
            "10.0.0.2:6000",
            ClientConfig {
                max_payload: MaxPayload::new(40).unwrap(),
                ..client_cfg()
            },
        );
        w.connect(i);
        w.run(20);
        let c = w.nodes[i].addr;
        let data: Vec<u32> = (0..20).map(|k| NOTE + k).collect();
        w.nodes[i].c.send_ump(w.haddr, &data).unwrap();
        w.run(5);
        assert_eq!(w.host_data(c), data);
        assert!(w.trace.iter().all(|(_, _, p)| p.len() <= 40));
    }

    // ----- REQ-NET-NAK-010 -----

    #[test]
    fn extra_words_and_reserved_bits_get_no_nak() {
        let mut h: Host = Host::new(host_cfg()).unwrap();
        let from = addr("10.0.0.2:6000");
        let mut cr = Creds::default();
        // Ping with 2 payload words and Command Specific Data not zero.
        let p = midi(&[0x20, 0x02, 0xAB, 0xCD, 0, 0, 0, 9, 0xEE, 0xEE, 0xEE, 0xEE]);
        let (log, out) = h.rx(0, from, &p, &mut cr);
        assert_eq!(out, [(from, midi(&[0x21, 0x01, 0, 0, 0, 0, 0, 9]))]);
        assert!(log.is_empty(), "{log:?}");
        // REQ-NET-PKT-011: the reader goes on with the next Command Packet.
        let p = midi(&[
            0x20, 0x02, 0, 0, 0, 0, 0, 9, 0xEE, 0xEE, 0xEE, 0xEE, 0x20, 0x01, 0, 0, 0, 0, 0, 10,
        ]);
        let (_, out) = h.rx(0, from, &p, &mut cr);
        assert_eq!(
            out,
            [(
                from,
                midi(&[0x21, 0x01, 0, 0, 0, 0, 0, 9, 0x21, 0x01, 0, 0, 0, 0, 0, 10])
            )]
        );
    }

    #[test]
    fn payload_length_rules() {
        // REQ-NET-PKT-004: the reader takes a payload longer than 1400 bytes (1432 here).
        let mut h: Host = Host::new(host_cfg()).unwrap();
        let from = addr("10.0.0.2:6000");
        let mut cr = Creds::default();
        let mut body = std::vec![0x20u8, 0xFF, 0, 0, 0, 0, 0, 1];
        body.extend([0u8; 254 * 4]);
        body.extend([0x20u8, 100, 0, 0, 0, 0, 0, 2]);
        body.extend([0u8; 99 * 4]);
        let p = midi(&body);
        assert_eq!(p.len(), 1432);
        let (_, out) = h.rx(0, from, &p, &mut cr);
        assert_eq!(
            out,
            [(
                from,
                midi(&[0x21, 0x01, 0, 0, 0, 0, 0, 1, 0x21, 0x01, 0, 0, 0, 0, 0, 2])
            )]
        );
        // A Command Packet that does not fit in the rest of the payload goes in a new payload:
        // with a maximum of 12 bytes, each Ping Reply has its own payload.
        let mut h: Host = Host::new(HostConfig {
            max_payload: MaxPayload::new(12).unwrap(),
            ..host_cfg()
        })
        .unwrap();
        let p = midi(&[0x20, 0x01, 0, 0, 0, 0, 0, 1, 0x20, 0x01, 0, 0, 0, 0, 0, 2]);
        let (_, out) = h.rx(0, from, &p, &mut cr);
        assert_eq!(
            out,
            [
                (from, midi(&[0x21, 0x01, 0, 0, 0, 0, 0, 1])),
                (from, midi(&[0x21, 0x01, 0, 0, 0, 0, 0, 2]))
            ]
        );
    }

    #[test]
    fn a_malformed_nak_gets_no_nak() {
        let mut h: Host = Host::new(host_cfg()).unwrap();
        let from = addr("10.0.0.2:6000");
        let mut cr = Creds::default();
        // NAK with pl = 0.
        let (log, out) = h.rx(0, from, &midi(&[0x8F, 0x00, 0x01, 0x00]), &mut cr);
        assert!(out.is_empty() && log.is_empty(), "{out:?} {log:?}");
        // NAK cut by the end of the payload.
        let (log, out) = h.rx(
            0,
            from,
            &midi(&[0x8F, 0x05, 0x01, 0x00, 0, 0, 0, 0]),
            &mut cr,
        );
        assert!(out.is_empty() && log.is_empty(), "{out:?} {log:?}");
    }

    // ----- REQ-NET-PKT-003 (diagnostic) -----

    #[test]
    fn a_payload_with_no_signature_is_a_diagnostic_only() {
        let mut h: Host = Host::new(host_cfg()).unwrap();
        let from = addr("10.0.0.2:6000");
        for p in [&b"MIDX\x20\x01\x00\x00\x00\x00\x00\x01"[..], b"MID", b""] {
            let (log, out) = h.rx(0, from, p, &mut Creds::default());
            assert_eq!(log, [(from, Ev::Diag(Diagnostic::BadSignature))]);
            assert!(out.is_empty());
        }
        assert_eq!(h.next_deadline(), None);
    }

    // ----- REQ-NET-PKT-002 -----

    #[test]
    fn multicast_and_broadcast_peer_addresses_are_errors() {
        let bad = [
            "224.0.0.251:5353",
            "239.1.2.3:5673",
            "255.255.255.255:5673",
            "[ff02::fb]:5353",
            "[::ffff:224.0.0.1]:5673",
            "[::ffff:255.255.255.255]:5673",
        ];
        let mut c: Client = Client::new(client_cfg()).unwrap();
        let mut h: Host = Host::new(host_cfg()).unwrap();
        for a in bad {
            let a = addr(a);
            assert_eq!(check_peer(a), Err(EndpointError::InvalidAddress));
            assert_eq!(c.connect(0, a), Err(EndpointError::InvalidAddress));
            assert_eq!(c.ping(0, a, None), Err(EndpointError::InvalidAddress));
            assert_eq!(h.ping(0, a, Some(1)), Err(EndpointError::InvalidAddress));
            assert_eq!(h.send_ump(a, &[NOTE]), Err(EndpointError::InvalidAddress));
            assert_eq!(
                h.bye(0, a, ByeReason::POWER_DOWN, ""),
                Err(EndpointError::InvalidAddress)
            );
            // A payload from such an address is ignored.
            let (log, out) = h.rx(0, a, &A11, &mut Creds::default());
            assert_eq!(log, [(a, Ev::Diag(Diagnostic::InvalidPeerAddress))]);
            assert!(out.is_empty());
        }
        assert_eq!(h.active_sessions(), 0);
        let mut buf = [0u8; 64];
        assert_eq!(c.write_pending(0, &mut buf), None);
        assert_eq!(h.write_pending(0, &mut buf), None);
        for good in ["10.0.0.1:5673", "192.168.255.255:5673", "[fe80::1]:5673"] {
            assert_eq!(check_peer(addr(good)), Ok(()));
        }
    }

    // ----- REQ-NET-GEN-001, REQ-NET-INV-035 -----

    #[test]
    fn host_holds_s_clients_and_answers_a_new_one_with_bye_40() {
        let mut w = established_world(2);
        assert_eq!(w.host.active_sessions(), 2);
        let i = w.add("10.0.0.4:6000", client_cfg());
        w.connect(i);
        w.run(10);
        let third = w.nodes[i].addr;
        assert!(w.host_has(third, &Ev::Diag(Diagnostic::TableFull)));
        assert!(w.node_has(i, &Ev::ClosedBye(0x40)), "{:?}", w.nodes[i].log);
        // The Bye bytes: code 0xF0, pl 0, Bye Reason 0x40.
        assert!(w
            .trace
            .iter()
            .any(|(s, d, b)| *s == w.haddr && *d == third && *b == midi(&[0xF0, 0, 0x40, 0])));
        assert_eq!(w.host.active_sessions(), 2);
        // The first two sessions still work.
        let (now, h) = (w.now, w.haddr);
        w.nodes[0].c.send_ump(h, &[NOTE]).unwrap();
        w.run(5);
        assert_eq!(w.host_data(w.nodes[0].addr), [NOTE]);
        // A free place after a Bye: the third Client gets a session.
        w.nodes[1]
            .c
            .bye(now, h, ByeReason::USER_TERMINATED, "")
            .unwrap();
        w.run(10);
        assert_eq!(w.host.active_sessions(), 1);
        w.nodes[i].log.clear();
        w.connect(i);
        w.run(10);
        assert!(w.node_has(i, &Ev::Established(None)));
        assert_eq!(w.host.active_sessions(), 2);
    }

    #[test]
    fn default_host_table_has_four_places() {
        // REQ-NET-GEN-001, N-42.
        let mut h: Host = Host::new(host_cfg()).unwrap();
        let mut cr = Creds::default();
        for k in 0..5 {
            let from = addr(&format!("10.0.0.{}:6000", 10 + k));
            let (_, out) = h.rx(0, from, &A11, &mut cr);
            if k < 4 {
                assert!(out.is_empty(), "{k}");
            } else {
                assert_eq!(out, [(from, midi(&[0xF0, 0, 0x40, 0]))]);
            }
        }
        assert_eq!(h.active_sessions(), 4);
    }

    #[test]
    fn authentication_invitation_from_idle_gets_bye_41_also_when_full() {
        // REQ-NET-INV-035 second sentence, REQ-NET-AUTH-027.
        let mut h: Host<1> = Host::new(host_cfg()).unwrap();
        let mut cr = Creds::default();
        let from = addr("10.0.0.20:6000");
        for full in [false, true] {
            for code in [0x02u8, 0x03] {
                let (_, out) = h.rx(0, from, &midi(&[code, 0, 0, 0]), &mut cr);
                assert_eq!(out, [(from, midi(&[0xF0, 0, 0x41, 0]))], "{code} {full}");
            }
            if !full {
                let (_, out) = h.rx(0, addr("10.0.0.21:6000"), &A11, &mut cr);
                assert!(out.is_empty());
                assert_eq!(h.active_sessions(), 1);
            }
        }
    }

    #[test]
    fn each_host_keeps_its_own_policy() {
        // REQ-NET-GEN-001: own authentication settings for each Host instance.
        let ask = HostConfig {
            policy: HostPolicy::new(PolicyKind::AskCaller),
            ..host_cfg()
        };
        let mut a: Host = Host::new(ask).unwrap();
        let mut b: Host = Host::new(host_cfg()).unwrap();
        let from = addr("10.0.0.2:6000");
        let mut cr = Creds::default();
        let (la, _) = a.rx(0, from, &A11, &mut cr);
        let (lb, _) = b.rx(0, from, &A11, &mut cr);
        assert!(la.contains(&(from, Ev::Invitation)));
        assert!(lb.contains(&(from, Ev::Established(None))));
        assert_eq!(a.policy().kind, PolicyKind::AskCaller);
        assert_eq!(b.policy().kind, PolicyKind::Accept);
    }

    #[test]
    fn host_accepts_or_rejects_after_pending() {
        // REQ-NET-INV-026 through the Endpoint.
        let mut w = World::new(HostConfig {
            policy: HostPolicy::new(PolicyKind::AskCaller),
            ..host_cfg()
        });
        let i = w.add("10.0.0.2:6000", client_cfg());
        let j = w.add("10.0.0.3:6000", client_cfg());
        w.connect(i);
        w.connect(j);
        w.run(5);
        let (ci, cj) = (w.nodes[i].addr, w.nodes[j].addr);
        assert!(w.host_has(ci, &Ev::Invitation) && w.host_has(cj, &Ev::Invitation));
        assert!(w.node_has(i, &Ev::Pending));
        let now = w.now;
        w.host.accept(now, ci).unwrap();
        w.host.reject(now, cj, None).unwrap();
        assert_eq!(
            w.host.accept(now, addr("10.0.0.50:1")).unwrap_err(),
            EndpointError::NoSession
        );
        w.run(10);
        assert!(w.node_has(i, &Ev::Established(None)));
        assert!(w.node_has(j, &Ev::ClosedBye(0x42)));
        assert_eq!(w.host.active_sessions(), 1);
    }

    // ----- REQ-NET-GEN-004 -----

    #[test]
    fn sessions_are_keyed_by_ip_address_and_port() {
        let mut w = World::new(host_cfg());
        let i = w.add("10.0.0.2:6000", client_cfg());
        let j = w.add("10.0.0.2:6001", client_cfg());
        w.connect(i);
        w.connect(j);
        w.run(10);
        assert!(w.node_has(i, &Ev::Established(None)));
        assert!(w.node_has(j, &Ev::Established(None)));
        assert_eq!(w.host.sessions().count(), 2);
        let (ci, cj) = (w.nodes[i].addr, w.nodes[j].addr);
        w.host.send_ump(ci, &[NOTE]).unwrap();
        w.host.send_ump(cj, &[NOTE2]).unwrap();
        w.run(5);
        assert_eq!(data_of(&w.nodes[i].log, w.haddr), [NOTE]);
        assert_eq!(data_of(&w.nodes[j].log, w.haddr), [NOTE2]);
        // Each payload of the Host goes to the address of its session.
        for (s, d, b) in &w.trace {
            if *s == w.haddr {
                for c in commands(b) {
                    if c.header.code == 0xFF && c.header.len > 0 {
                        let word = c.word(0).unwrap();
                        assert_eq!(*d, if word == NOTE { ci } else { cj });
                    }
                }
            }
        }
    }

    // ----- REQ-NET-PING-003 -----

    #[test]
    fn ping_to_any_address_uses_the_counter() {
        let mut c: Client = Client::new(ClientConfig {
            first_ping_id: 0xFFFF_FFFF,
            ..client_cfg()
        })
        .unwrap();
        let h = addr("10.0.0.1:5673");
        assert_eq!(c.ping(0, h, None), Ok(0xFFFF_FFFF));
        assert_eq!(c.ping(0, h, None), Ok(0));
        assert_eq!(c.ping(0, h, Some(77)), Ok(77));
        assert_eq!(c.ping(0, h, None), Ok(1));
        // No session: no entry is made.
        assert!(c.session(h).is_none());
        let mut buf = [0u8; 1400];
        let (to, n) = c.write_pending(0, &mut buf).unwrap();
        assert_eq!(to, h);
        let ids: Vec<u32> = commands(&buf[..n])
            .iter()
            .map(|c| {
                assert_eq!(c.header.code, 0x20);
                c.word(0).unwrap()
            })
            .collect();
        assert_eq!(ids, [0xFFFF_FFFF, 0, 77, 1]);
        // The Ping Reply of the Host gives the round-trip time.
        let mut host: Host = Host::new(host_cfg()).unwrap();
        let (_, replies) = host.rx(5, addr("10.0.0.2:6000"), &buf[..n], &mut Creds::default());
        let mut cr = Creds::default();
        let got: Vec<(u32, u64)> = c
            .receive(9, h, &replies[0].1, &mut cr)
            .filter_map(|e| match e.kind {
                EventKind::Session(SessionEvent::PingReply { id, rtt_ms }) => Some((id, rtt_ms)),
                _ => None,
            })
            .collect();
        assert_eq!(got, [(0xFFFF_FFFF, 9), (0, 9), (77, 9), (1, 9)]);
        assert!(host.sessions().next().is_none());
    }

    #[test]
    fn queue_capacity_follows_o() {
        // OPT-008: O sets the capacity of the stateless Command queue. With O = 2, a third Ping
        // to an address with no session is refused (TableFull) until write_pending sends the
        // queue.
        let mut h: Host<1, 64, 256, 4, 8, 128, 2> = Host::new(host_cfg()).unwrap();
        let mut buf = [0u8; 1500];
        assert!(h.ping(0, addr("10.0.0.2:6000"), Some(1)).is_ok());
        assert!(h.ping(0, addr("10.0.0.3:6000"), Some(2)).is_ok());
        assert_eq!(
            h.ping(0, addr("10.0.0.4:6000"), Some(3)),
            Err(EndpointError::TableFull)
        );
        let mut sent = 0;
        while h.write_pending(0, &mut buf).is_some() {
            sent += 1;
        }
        assert_eq!(sent, 2);
        assert!(h.ping(0, addr("10.0.0.4:6000"), Some(3)).is_ok());
    }

    #[test]
    fn ping_in_any_state_does_not_change_the_state() {
        let mut c: Client = Client::new(client_cfg()).unwrap();
        let h = addr("10.0.0.1:5673");
        c.connect(0, h).unwrap();
        assert_eq!(c.ping(0, h, Some(5)), Ok(5));
        assert_eq!(
            c.session(h).unwrap().state(),
            SessionState::PendingInvitation
        );
        let mut w = established_world(1);
        let (now, haddr) = (w.now, w.haddr);
        w.nodes[0].c.ping(now, haddr, Some(42)).unwrap();
        w.host.ping(now, w.nodes[0].addr, None).unwrap();
        w.run(3);
        assert!(w.node_has(0, &Ev::Ping(42)));
        assert!(w.host_has(w.nodes[0].addr, &Ev::Ping(0)));
        assert_eq!(
            w.nodes[0].c.session(haddr).unwrap().state(),
            SessionState::EstablishedSession
        );
        // In Pending Bye the Session refuses (REQ-NET-BYE-013).
        let now = w.now;
        w.nodes[0]
            .c
            .bye(now, haddr, ByeReason::USER_TERMINATED, "")
            .unwrap();
        assert_eq!(
            w.nodes[0].c.ping(now, haddr, None),
            Err(EndpointError::Session(SessionError::WrongState(
                SessionState::PendingBye
            )))
        );
    }

    // ----- REQ-NET-DISC-013, REQ-NET-TOPO-005 -----

    fn txt_value<'a>(rdata: &'a [u8], key: &str) -> &'a [u8] {
        let mut rest = rdata;
        while let Some((&len, tail)) = rest.split_first() {
            let (s, r) = tail.split_at(usize::from(len));
            if let Some(v) = s
                .strip_prefix(key.as_bytes())
                .and_then(|v| v.strip_prefix(b"="))
            {
                return v;
            }
            rest = r;
        }
        panic!("no key {key}");
    }

    #[test]
    fn txt_record_and_invitation_replies_hold_the_same_identity() {
        // REQ-NET-DISC-013: Accepted and Pending.
        for policy in [PolicyKind::Accept, PolicyKind::AskCaller] {
            let mut h: Host = Host::new(HostConfig {
                policy: HostPolicy::new(policy),
                ..host_cfg()
            })
            .unwrap();
            let rec = h.host_records("Studio Host", 5673, "studio.local").unwrap();
            let mut buf = [0u8; 600];
            let set = rec.write(&mut buf).unwrap();
            let name = txt_value(set.txt.rdata, "UMPEndpointName").to_vec();
            let id = txt_value(set.txt.rdata, "ProductInstanceId").to_vec();
            assert_eq!(name, b"Host");
            assert_eq!(id, b"H1");
            let from = addr("10.0.0.2:6000");
            let _ = h.rx(0, from, &A11, &mut Creds::default());
            let (_, out) = h.tx(0);
            let raw = commands(&out[0].1)[0];
            let r = InvitationReply::read(&raw, policy == PolicyKind::AskCaller).unwrap();
            assert_eq!(r.name.as_bytes(), name);
            assert_eq!(r.product_instance_id.as_bytes(), id);
        }
        let h: Host = Host::new(host_cfg()).unwrap();
        assert!(matches!(
            h.host_records("", 5673, "studio.local"),
            Err(EndpointError::Dns(_))
        ));
    }

    #[test]
    fn host_and_client_of_one_endpoint_write_the_same_identity() {
        // REQ-NET-TOPO-005: one identity value for a Host and a Client.
        let id = Identity::new("Synth", "SN-0042").unwrap();
        let mut w = World::new(HostConfig::new(id));
        let i = w.add("10.0.0.2:6000", ClientConfig::new(id));
        w.connect(i);
        w.run(5);
        let rec = w.host.host_records("Synth", 5673, "synth.local").unwrap();
        let mut buf = [0u8; 600];
        let set = rec.write(&mut buf).unwrap();
        let name = txt_value(set.txt.rdata, "UMPEndpointName");
        let pid = txt_value(set.txt.rdata, "ProductInstanceId");
        let mut seen = 0;
        for (_, _, b) in &w.trace {
            for c in commands(b) {
                let (n, p) = match c.header.code {
                    0x01 => {
                        let v = crate::command::Invitation::read(&c).unwrap();
                        (v.name, v.product_instance_id)
                    }
                    0x10 | 0x11 => {
                        let v = InvitationReply::read(&c, c.header.code == 0x11).unwrap();
                        (v.name, v.product_instance_id)
                    }
                    _ => continue,
                };
                assert_eq!(n.as_bytes(), name);
                assert_eq!(p.as_bytes(), pid);
                seen += 1;
            }
        }
        assert!(seen >= 2, "{seen}");
    }

    // ----- REQ-NET-TOPO-006 -----

    #[test]
    fn several_hosts_and_clients_run_side_by_side() {
        let h1a = addr("10.0.0.1:5673");
        let h2a = addr("10.0.0.1:5674");
        let ca = addr("10.0.0.2:6000");
        let mut h1: Host = Host::new(HostConfig::new(Identity::new("One", "1").unwrap())).unwrap();
        let mut h2: Host = Host::new(HostConfig::new(Identity::new("Two", "2").unwrap())).unwrap();
        let mut c: Client<2> = Client::new(client_cfg()).unwrap();
        c.connect(0, h1a).unwrap();
        c.connect(0, h2a).unwrap();
        let mut cr = Creds::default();
        for now in 0..5 {
            let (_, out) = c.tx(now);
            for (to, b) in out {
                let h = if to == h1a { &mut h1 } else { &mut h2 };
                let _ = h.rx(now, ca, &b, &mut cr);
            }
            for (ha, h) in [(h1a, &mut h1), (h2a, &mut h2)] {
                let (_, out) = h.tx(now);
                for (to, b) in out {
                    assert_eq!(to, ca);
                    let _ = c.rx(now, ha, &b, &mut cr);
                }
            }
        }
        let peer = |a| {
            c.session(a)
                .and_then(|s| s.peer())
                .map(|p| p.name().to_vec())
        };
        assert_eq!(peer(h1a), Some(b"One".to_vec()));
        assert_eq!(peer(h2a), Some(b"Two".to_vec()));
        assert_eq!(h1.active_sessions(), 1);
        assert_eq!(h2.active_sessions(), 1);
        assert_eq!(h1.identity().name.as_str(), "One");
        assert_eq!(h2.identity().name.as_str(), "Two");
    }

    // ----- end to end -----

    #[test]
    fn end_to_end_data_ping_and_bye() {
        let mut w = World::new(HostConfig::new(host_id()));
        let i = w.add("10.0.0.2:6000", ClientConfig::new(client_id()));
        w.connect(i);
        w.run(10);
        let (h, c) = (w.haddr, w.nodes[i].addr);
        assert!(w.host_has(c, &Ev::Established(None)));
        assert!(w.node_has(i, &Ev::Established(None)));
        w.nodes[i].c.send_ump(h, &[NOTE, NOTE2]).unwrap();
        w.host.send_ump(c, &[NOTE2]).unwrap();
        w.run(500);
        assert_eq!(w.host_data(c), [NOTE, NOTE2]);
        assert_eq!(data_of(&w.nodes[i].log, h), [NOTE2]);
        // The automatic Ping of the default configuration and its replies.
        w.run(2000);
        assert!(w.nodes[i].log.iter().any(|(_, e)| matches!(e, Ev::Ping(_))));
        assert!(w.hlog.iter().any(|(_, e)| matches!(e, Ev::Ping(_))));
        // Bye from the Host.
        let now = w.now;
        let ev = w.host.bye(now, c, ByeReason::POWER_DOWN, "off").unwrap();
        assert!(ev
            .iter()
            .any(|e| *e == SessionEvent::Ending(ByeReason::POWER_DOWN)));
        w.run(10);
        assert!(w.node_has(i, &Ev::ClosedBye(0x02)));
        assert!(w.host_has(c, &Ev::ClosedByeReply));
        assert_eq!(w.host.sessions().count(), 0);
        assert_eq!(w.nodes[i].c.sessions().count(), 0);
        assert_eq!(w.host.next_deadline(), None);
        assert_eq!(w.host.dropped_events(), 0);
        assert_eq!(w.host.dropped_replies(), 0);
    }

    fn is_data_with_words(b: &[u8]) -> bool {
        commands(b)
            .iter()
            .any(|c| c.header.code == 0xFF && c.header.len > 0)
    }

    #[test]
    fn lost_payload_is_recovered_by_fec() {
        // REQ-NET-FEC-001 through the Endpoint.
        let mut w = established_world(1);
        let mut lost = false;
        w.drop = Box::new(move |_, _, b| {
            if !lost && is_data_with_words(b) {
                lost = true;
                return true;
            }
            false
        });
        let h = w.haddr;
        w.nodes[0].c.send_ump(h, &[NOTE]).unwrap();
        w.run(2);
        w.nodes[0].c.send_ump(h, &[NOTE2]).unwrap();
        w.run(2);
        assert_eq!(w.dropped, 1);
        assert_eq!(w.host_data(w.nodes[0].addr), [NOTE, NOTE2]);
    }

    #[test]
    fn lost_payload_is_recovered_by_retransmit() {
        // REQ-NET-RTX-005, -009 through the Endpoint: no FEC, a Retransmit Request.
        let no_fec = SessionConfig {
            stream: StreamConfig {
                fec_repeats: 0,
                ..StreamConfig::DEFAULT
            },
            ..quiet()
        };
        let mut w = World::new(HostConfig {
            session: no_fec,
            ..host_cfg()
        });
        let i = w.add(
            "10.0.0.2:6000",
            ClientConfig {
                session: no_fec,
                ..client_cfg()
            },
        );
        w.connect(i);
        w.run(5);
        let mut lost = false;
        w.drop = Box::new(move |_, _, b| {
            if !lost && is_data_with_words(b) {
                lost = true;
                return true;
            }
            false
        });
        let h = w.haddr;
        w.nodes[i].c.send_ump(h, &[NOTE]).unwrap();
        w.run(2);
        w.nodes[i].c.send_ump(h, &[NOTE2]).unwrap();
        w.run(100);
        assert_eq!(w.dropped, 1);
        let c = w.nodes[i].addr;
        assert!(w.host_has(c, &Ev::Data(vec![NOTE2], false)), "{:?}", w.hlog);
        assert!(w.host_has(c, &Ev::Data(vec![NOTE], true)), "{:?}", w.hlog);
        assert!(w
            .trace
            .iter()
            .any(|(s, _, b)| *s == w.haddr && commands(b).iter().any(|c| c.header.code == 0x80)));
    }

    #[test]
    fn write_pending_serves_the_sessions_in_turn() {
        // Round robin: each session with data gets one payload before the next round.
        let mut w = established_world(2);
        let (a, b) = (w.nodes[0].addr, w.nodes[1].addr);
        let now = w.now;
        for _ in 0..3 {
            w.host.send_ump(a, &[NOTE]).unwrap();
            w.host.send_ump(b, &[NOTE2]).unwrap();
        }
        let _ = w.host.poll(now).count();
        let mut buf = [0u8; 1400];
        let mut to = Vec::new();
        while let Some((d, _)) = w.host.write_pending(now, &mut buf) {
            to.push(d);
        }
        assert_eq!(to.len(), 2);
        assert_ne!(to[0], to[1]);
    }

    #[test]
    fn polled_events_are_not_lost_when_the_iterator_is_dropped() {
        // A Client whose Invitation times out: the event waits for the next poll.
        let mut c: Client = Client::new(client_cfg()).unwrap();
        let h = addr("10.0.0.1:5673");
        c.connect(0, h).unwrap();
        let t = 20_000;
        drop(c.poll(t));
        let log: Vec<_> = c.poll(t).map(|e| own(&e).1).collect();
        // Invitation timeout: Bye 0x80 and Pending Bye (REQ-NET-INV-011).
        assert_eq!(log, [Ev::Ending(0x80)]);
        // The entry in Pending Bye still uses the one place.
        assert_eq!(
            c.connect(t, addr("10.0.0.9:5673")),
            Err(EndpointError::TableFull)
        );
    }

    #[test]
    fn host_cancel_and_reset_reach_the_session() {
        let mut w = established_world(1);
        let c = w.nodes[0].addr;
        let now = w.now;
        w.host.reset(now, c).unwrap();
        w.run(5);
        assert!(w.host_has(c, &Ev::Other(format!("{:?}", SessionEvent::Reset))));
        assert!(matches!(
            w.host.cancel(w.now, c),
            Err(EndpointError::Session(_))
        ));
        assert_eq!(
            w.host.reset(w.now, addr("10.9.9.9:1")),
            Err(EndpointError::NoSession)
        );
    }

    #[test]
    fn invalid_session_configuration_is_refused() {
        let bad = SessionConfig {
            repeat_interval_ms: 10,
            ..SessionConfig::DEFAULT
        };
        assert!(matches!(
            Host::<4>::new(HostConfig {
                session: bad,
                ..host_cfg()
            }),
            Err(EndpointError::Session(SessionError::RepeatInterval))
        ));
        assert!(matches!(
            Client::<1>::new(ClientConfig {
                session: bad,
                ..client_cfg()
            }),
            Err(EndpointError::Session(SessionError::RepeatInterval))
        ));
    }

    #[test]
    fn small_writer_buffer_and_max_payload_are_used() {
        // REQ-NET-PKT-004 limit from the configuration.
        let mut c: Client = Client::new(ClientConfig {
            max_payload: MaxPayload::new(16).unwrap(),
            ..client_cfg()
        })
        .unwrap();
        let h = addr("10.0.0.1:5673");
        for _ in 0..3 {
            c.ping(0, h, None).unwrap();
        }
        let mut buf = [0u8; 1400];
        let mut n_payloads = 0;
        while let Some((_, n)) = c.write_pending(0, &mut buf) {
            assert!(n <= 16);
            n_payloads += 1;
        }
        assert_eq!(n_payloads, 3);
    }

    #[test]
    fn ping_gets_a_reply_when_the_session_table_is_full() {
        // REQ-NET-PING-005: a Host with one place and one session answers a Ping from a new
        // address.
        let mut h: Host<1> = Host::new(host_cfg()).unwrap();
        let mut cr = Creds::default();
        let (a, b) = (addr("10.0.0.2:6000"), addr("10.0.0.3:6000"));
        h.rx(0, a, &A11, &mut cr);
        h.tx(0);
        assert_eq!(h.active_sessions(), 1);
        let (_, out) = h.rx(1, b, &midi(&[0x20, 0x01, 0, 0, 0, 0, 0, 5]), &mut cr);
        assert_eq!(out, [(b, midi(&[0x21, 0x01, 0, 0, 0, 0, 0, 5]))]);
        assert_eq!(h.active_sessions(), 1);
    }

    #[cfg(feature = "auth")]
    #[test]
    fn failures_of_all_clients_use_one_counter() {
        // REQ-NET-AUTH-026 (N-45): n counts the failed authentications of the Host, from every
        // Client address.
        let mut w = World::new(HostConfig {
            policy: HostPolicy::new(PolicyKind::SharedSecret),
            ..host_cfg()
        });
        w.hcreds = Creds {
            random: Some([7; NONCE_RANDOM_LEN]),
            secret: Some(b"s3cret".to_vec()),
        };
        let shared = ClientConfig {
            capabilities: Capabilities {
                auth: true,
                user_auth: false,
            },
            ..client_cfg()
        };
        let i = w.add("10.0.0.2:6000", shared);
        let j = w.add("10.0.0.3:6000", shared);
        w.nodes[i].creds.secret = Some(b"wrong".to_vec());
        w.nodes[j].creds.secret = Some(b"bad".to_vec());
        w.connect(i);
        w.connect(j);
        w.run(50);
        let failed = |a: SocketAddr| {
            w.hlog
                .iter()
                .filter(|(x, e)| {
                    *x == a && matches!(e, Ev::Other(s) if s.starts_with("AuthFailed"))
                })
                .count() as u32
        };
        let (fi, fj) = (failed(w.nodes[i].addr), failed(w.nodes[j].addr));
        assert!(fi >= 1 && fj >= 1, "{fi} {fj}");
        assert_eq!(w.host.core.delay.failures(), fi + fj);
    }

    #[cfg(feature = "auth")]
    #[test]
    fn user_auth_required_holds_the_identity() {
        // REQ-NET-DISC-013, REQ-NET-TOPO-005: Invitation Reply: User Authentication Required
        // holds the identity of the TXT record.
        let mut w = World::new(HostConfig {
            policy: HostPolicy::new(PolicyKind::UserList),
            ..host_cfg()
        });
        w.hcreds = Creds {
            random: Some([7; NONCE_RANDOM_LEN]),
            secret: None,
        };
        let i = w.add(
            "10.0.0.2:6000",
            ClientConfig {
                capabilities: Capabilities {
                    auth: false,
                    user_auth: true,
                },
                ..client_cfg()
            },
        );
        w.connect(i);
        w.run(5);
        let rec = w.host.host_records("Studio", 5673, "studio.local").unwrap();
        let mut buf = [0u8; 600];
        let set = rec.write(&mut buf).unwrap();
        let mut seen = 0;
        for (s, _, b) in &w.trace {
            for cmd in commands(b) {
                if *s == w.haddr && cmd.header.code == 0x13 {
                    let r = crate::auth::UserAuthRequired::read(&cmd).unwrap();
                    assert_eq!(
                        r.name.as_bytes(),
                        txt_value(set.txt.rdata, "UMPEndpointName")
                    );
                    assert_eq!(
                        r.product_instance_id.as_bytes(),
                        txt_value(set.txt.rdata, "ProductInstanceId")
                    );
                    seen += 1;
                }
            }
        }
        assert!(seen >= 1, "{seen}");
    }

    #[cfg(feature = "auth")]
    #[test]
    fn end_to_end_shared_secret_session() {
        // REQ-NET-AUTH through the Endpoint; the Host keeps its own failure delay (N-45).
        let mut w = World::new(HostConfig {
            policy: HostPolicy::new(PolicyKind::SharedSecret),
            ..host_cfg()
        });
        w.hcreds = Creds {
            random: Some([7; NONCE_RANDOM_LEN]),
            secret: Some(b"s3cret".to_vec()),
        };
        let i = w.add(
            "10.0.0.2:6000",
            ClientConfig {
                capabilities: Capabilities {
                    auth: true,
                    user_auth: false,
                },
                ..client_cfg()
            },
        );
        w.nodes[i].creds.secret = Some(b"s3cret".to_vec());
        // A second Client with a wrong secret.
        let j = w.add(
            "10.0.0.3:6000",
            ClientConfig {
                capabilities: Capabilities {
                    auth: true,
                    user_auth: false,
                },
                ..client_cfg()
            },
        );
        w.nodes[j].creds.secret = Some(b"wrong".to_vec());
        w.connect(i);
        w.connect(j);
        w.run(50);
        let c = w.nodes[i].addr;
        assert!(
            w.node_has(i, &Ev::Established(Some(Method::SharedSecret))),
            "{:?}",
            w.nodes[i].log
        );
        assert!(w.host_has(c, &Ev::Established(Some(Method::SharedSecret))));
        assert!(!w.node_has(j, &Ev::Established(Some(Method::SharedSecret))));
        assert!(w.hlog.iter().any(|(a, e)| *a == w.nodes[j].addr
            && matches!(e, Ev::Other(s) if s.starts_with("AuthFailed"))));
        // REQ-NET-DISC-013: Authentication Required holds the identity of the TXT record.
        let rec = w.host.host_records("Studio", 5673, "studio.local").unwrap();
        let mut buf = [0u8; 600];
        let set = rec.write(&mut buf).unwrap();
        let mut seen = 0;
        for (s, _, b) in &w.trace {
            for cmd in commands(b) {
                if *s == w.haddr && cmd.header.code == 0x12 {
                    let r = crate::auth::AuthRequired::read(&cmd).unwrap();
                    assert_eq!(
                        r.name.as_bytes(),
                        txt_value(set.txt.rdata, "UMPEndpointName")
                    );
                    assert_eq!(
                        r.product_instance_id.as_bytes(),
                        txt_value(set.txt.rdata, "ProductInstanceId")
                    );
                    seen += 1;
                }
            }
        }
        assert!(seen >= 2, "{seen}");
        let h = w.haddr;
        w.nodes[i].c.send_ump(h, &[NOTE]).unwrap();
        w.run(3);
        assert_eq!(w.host_data(c), [NOTE]);
    }

    // ----- never panics -----

    #[test]
    fn prop_receive_never_panics() {
        proptest!(|(body in proptest::collection::vec(any::<u8>(), 0..120), port in 1u16..4, now in 0u64..10_000)| {
            let mut h: Host<2> = Host::new(host_cfg()).unwrap();
            let mut c: Client = Client::new(client_cfg()).unwrap();
            let from = SocketAddr::from(([10, 0, 0, 2], port));
            c.connect(0, from).unwrap();
            let p = midi(&body);
            let _ = h.rx(now, from, &p, &mut Creds::default());
            let _ = h.rx(now, from, &body, &mut Creds::default());
            let _ = c.rx(now, from, &p, &mut Creds::default());
            let _ = h.tx(now + 1);
            let _ = c.tx(now + 1);
        });
    }

    #[test]
    fn prop_received_dropped_early_still_answers() {
        // A Received that the caller drops processes all Commands (no state is left half
        // done); the replies wait for write_pending.
        proptest!(|(id in any::<u32>())| {
            let mut h: Host = Host::new(host_cfg()).unwrap();
            let from = addr("10.0.0.2:6000");
            let b = id.to_be_bytes();
            let p = midi(&[0x20, 1, 0, 0, b[0], b[1], b[2], b[3]]);
            drop(h.receive(0, from, &p, &mut Creds::default()));
            let mut buf = [0u8; 64];
            let (to, n) = h.write_pending(0, &mut buf).unwrap();
            prop_assert_eq!(to, from);
            prop_assert_eq!(&buf[..n], &midi(&[0x21, 1, 0, 0, b[0], b[1], b[2], b[3]])[..]);
        });
    }

    #[test]
    fn nak_and_bye_types_are_the_command_module_types() {
        // The stateless replies use the Command writers: a Bye 0x05 to an address with no
        // session for UMP Data, once for each UDP payload (N-16).
        let mut h: Host = Host::new(host_cfg()).unwrap();
        let from = addr("10.0.0.2:6000");
        let p = midi(&[0xFF, 0, 0, 1, 0xFF, 0, 0, 2, 0xFF, 0, 0, 3]);
        let (_, out) = h.rx(0, from, &p, &mut Creds::default());
        assert_eq!(out.len(), 1);
        let cmds = commands(&out[0].1);
        assert_eq!(cmds.len(), 1);
        assert_eq!(
            Bye::read(&cmds[0]).reason,
            ByeReason::SESSION_NOT_ESTABLISHED
        );
        let _ = Nak::read;
    }
}
