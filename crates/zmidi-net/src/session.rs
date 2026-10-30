//! The state machine for ONE peer of Network MIDI 2.0 (M2-124-UM v1.0.1 section 6.1 and the
//! sections on Ping 6.13, NAK 6.15, Bye 6.16, Session Reset 6.12 and UMP Data 7).
//!
//! - [`Session`] holds the [`SessionState`] of one peer address, one [`Handshake`], one
//!   [`DataStream`], the [`OpenPings`], the NAK 0x01 flags (REQ-NET-CMD-003) and a small queue
//!   of one-shot replies (Ping Reply, NAK, Retransmit Error, single Byes and Handshake
//!   replies; const generic `R`, default 4).
//! - [`Session::receive`] checks each received Command against Table 9 and the Command
//!   sections for the state (REQ-NET-SES-003, -004, -008) and against the direction of its
//!   Command Code for the role (REQ-NET-SES-009). A Command for which no section gives an
//!   action gets NAK 0x02 and no state change (N-15). The Session never writes a NAK for a
//!   NAK (N-29).
//! - The repeat and timeout engine uses [`SessionConfig`] (REQ-NET-SES-005 to -007, N-21,
//!   N-23, N-24, N-26). [`Session::poll`] runs the timeouts, [`Session::write_payload`] writes
//!   the due Commands, [`Session::next_deadline`] gives the time of the next work.
//! - [`Session::idle_reply`] answers a Command from an address with no entry, with no state
//!   (N-16): the Endpoint writes the [`Stateless`] reply once.
//! - Sans-IO (DEC-012): the caller gives the time as u64 ms, the buffers and the credentials.
//!   No clock, no I/O, no static, no alloc (DEC-002). Nothing panics on received bytes.
//!
//! Use by the Endpoint (one Session for each peer address):
//! 1. Client: [`Session::client`] then [`Session::connect`]. Host: for a Command from an
//!    address with no entry call [`Session::idle_reply`]; for [`IdleReply::Open`] put
//!    [`Session::host`] in a free place and call [`Session::open`] (check the table capacity
//!    before, REQ-NET-INV-035).
//! 2. For each received Command of the peer: [`Session::receive`].
//! 3. At [`Session::next_deadline`] (and after each receive): [`Session::poll`], then
//!    [`Session::write_payload`] until [`Written::more`] is false.
//! 4. Free the entry when [`Session::is_finished`] is true.

use crate::command::{
    write_empty, write_ping, write_ping_reply, Bye, ByeReason, Capabilities, Command, CommandCode,
    CommandError, Nak, NakReason, RetransmitError, Role, SessionState, MAX_BYE_TEXT,
};
use crate::handshake::{
    Challenge, Credentials, CredentialsError, Deadline, Env, Handshake, HandshakeConfig,
    HandshakeError, HandshakeEvent, Method, Outcome, PeerInfo, Repeat, Reply,
    DEFAULT_USERNAME_CAPACITY,
};
use crate::identity::Identity;
use crate::packet::{Header, PacketError, PayloadWriter};
use crate::stream::{
    DataStream, Delivery, RequestOutcome, Ring, StreamConfig, StreamError, StreamEvent, Written,
};

/// The smallest repeat interval, in ms (REQ-NET-SES-006, N-21).
pub const MIN_REPEAT_INTERVAL_MS: u64 = 300;
/// The largest repeat interval, in ms (REQ-NET-SES-006, N-21).
pub const MAX_REPEAT_INTERVAL_MS: u64 = 2000;
/// The default repeat interval, in ms (REQ-NET-SES-006, N-21).
pub const DEFAULT_REPEAT_INTERVAL_MS: u64 = 500;
/// The default Session Reset timeout, in ms (REQ-NET-RST-005, N-23).
pub const DEFAULT_RESET_TIMEOUT_MS: u64 = 3000;
/// The default Bye timeout, in ms (REQ-NET-BYE-007, N-24).
pub const DEFAULT_BYE_TIMEOUT_MS: u64 = 3000;
/// The default Ping interval, in ms (REQ-NET-PING-004, N-26).
pub const DEFAULT_PING_INTERVAL_MS: u64 = 2000;
/// The default Ping miss limit (REQ-NET-PING-009, N-26).
pub const DEFAULT_PING_MISS_LIMIT: u32 = 5;
/// The capacity of the Text Message of a Bye that this side repeats, in bytes. The Session
/// keeps the text for the repeats (REQ-NET-SES-005) in a fixed buffer (DEC-002).
pub const BYE_TEXT_CAPACITY: usize = 64;
/// The capacity of [`Events`].
pub const EVENTS_CAPACITY: usize = 6;

/// Error of the Session configuration and of the caller requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// The repeat interval is less than 300 ms or more than 2000 ms (REQ-NET-SES-006).
    #[error("repeat interval out of range (300 to 2000 ms)")]
    RepeatInterval,
    /// The stream configuration or a stream writer refused.
    #[error("stream: {0}")]
    Stream(#[from] StreamError),
    /// The Handshake configuration or a Handshake writer refused.
    #[error("handshake: {0}")]
    Handshake(#[from] HandshakeError),
    /// A Command writer refused.
    #[error("command: {0}")]
    Command(#[from] CommandError),
    /// The request is not allowed in this session state (REQ-NET-DATA-002, REQ-NET-RST-003,
    /// REQ-NET-BYE-006, REQ-NET-BYE-013).
    #[error("not allowed in state {0:?}")]
    WrongState(SessionState),
    /// The request is not allowed for the role of this Session.
    #[error("not allowed for this role")]
    WrongRole,
    /// The peer answered NAK 0x01 (Command Not Supported) for this Command Code in this
    /// session (REQ-NET-CMD-003).
    #[error("Command {0:?} not supported by the peer")]
    Refused(CommandCode),
    /// A caller Ping waits for the next UDP payload: call [`Session::write_payload`] first.
    #[error("a Ping waits to be written")]
    Busy,
}

/// The configuration of a [`Session`] (DEC-NET-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    /// The repeat interval of Invitation, Invitation with (User) Authentication, Session
    /// Reset and Bye, 300 to 2000 ms, default 500 ms (REQ-NET-SES-006, N-21).
    pub repeat_interval_ms: u64,
    /// The Session Reset timeout, from the reset request, default 3000 ms
    /// (REQ-NET-RST-005, N-23).
    pub reset_timeout_ms: u64,
    /// The Bye timeout, from the first Bye, default 3000 ms (REQ-NET-BYE-007, N-24).
    pub bye_timeout_ms: u64,
    /// The interval of the automatic Ping, default 2000 ms; 0 turns it off
    /// (REQ-NET-PING-004, N-26).
    pub ping_interval_ms: u64,
    /// The Ping miss limit, default 5; 0 turns the check off (REQ-NET-PING-009, N-26).
    pub ping_miss_limit: u32,
    /// NAK replies to bad Ping Reply Commands, default off (REQ-NET-PING-008, -010, N-28).
    pub nak_bad_ping_reply: bool,
    /// The UMP Data stream configuration; `stream.retransmit` is the Retransmit support
    /// value (REQ-NET-RTX-004, N-38).
    pub stream: StreamConfig,
    /// The Invitation timeouts and the failure limit (N-22, N-25, N-46).
    pub handshake: HandshakeConfig,
}

impl SessionConfig {
    /// The default values of DEC-NET-001.
    pub const DEFAULT: SessionConfig = SessionConfig {
        repeat_interval_ms: DEFAULT_REPEAT_INTERVAL_MS,
        reset_timeout_ms: DEFAULT_RESET_TIMEOUT_MS,
        bye_timeout_ms: DEFAULT_BYE_TIMEOUT_MS,
        ping_interval_ms: DEFAULT_PING_INTERVAL_MS,
        ping_miss_limit: DEFAULT_PING_MISS_LIMIT,
        nak_bad_ping_reply: false,
        stream: StreamConfig::DEFAULT,
        handshake: HandshakeConfig::DEFAULT,
    };

    /// Checks the values: the repeat interval from 300 to 2000 ms (REQ-NET-SES-006), the
    /// stream configuration and the Handshake configuration.
    pub fn check(&self) -> Result<(), SessionError> {
        if !(MIN_REPEAT_INTERVAL_MS..=MAX_REPEAT_INTERVAL_MS).contains(&self.repeat_interval_ms) {
            return Err(SessionError::RepeatInterval);
        }
        self.stream.check()?;
        self.handshake.check()?;
        Ok(())
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig::DEFAULT
    }
}

/// One open Ping: a Ping that this side wrote and that has no Ping Reply yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenPing {
    /// The Ping Id.
    pub id: u32,
    /// The time of the write, in ms.
    pub sent_at: u64,
}

/// The last P Pings written to one peer that wait for a reply (REQ-NET-PING-007, N-27), from
/// the oldest to the newest. P is 1 or more (checked at compile time).
#[derive(Debug, Clone)]
pub struct OpenPings<const P: usize = 8> {
    ring: Ring<OpenPing, P>,
}

impl<const P: usize> Default for OpenPings<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const P: usize> OpenPings<P> {
    const CHECK: () = assert!(P >= 1, "the open Ping table P must be 1 or more");

    /// An empty table.
    #[inline(always)]
    pub const fn new() -> Self {
        let () = Self::CHECK;
        OpenPings {
            ring: Ring::new(OpenPing { id: 0, sent_at: 0 }),
        }
    }

    /// The number of open Pings.
    pub const fn len(&self) -> usize {
        self.ring.len()
    }

    /// True when no Ping is open.
    pub const fn is_empty(&self) -> bool {
        self.ring.len() == 0
    }

    /// Closes all Pings.
    pub fn clear(&mut self) {
        self.ring.clear();
    }

    /// The open Pings, from the oldest to the newest.
    pub fn iter(&self) -> impl Iterator<Item = &OpenPing> {
        self.ring.iter()
    }

    /// Adds a written Ping; when the table is full, the oldest Ping leaves it.
    pub fn push(&mut self, id: u32, sent_at: u64) {
        if self.ring.len() == P {
            self.ring.drop_front(1);
        }
        let _ = self.ring.push_back(OpenPing { id, sent_at });
    }

    /// Closes the newest open Ping with Ping Id `id` and all the older open Pings, and gives
    /// it; `None` when no open Ping has this Id. The Id is used only to find the Ping
    /// (REQ-NET-PING-007).
    pub fn close(&mut self, id: u32) -> Option<OpenPing> {
        let i = self.ring.rposition(|p| p.id == id)?;
        let found = self.ring.get(i)?;
        self.ring.drop_front(i + 1);
        Some(found)
    }
}

/// Why a session went to Idle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseCause<'a> {
    /// The peer sent Bye: its Bye Reason (the raw value is `.0`) and Text Message
    /// (REQ-NET-BYE-008).
    ByeReceived(Bye<'a>),
    /// The Bye Reply of the peer ended the Pending Bye of this side (REQ-NET-BYE-011).
    ByeReply,
    /// The Bye timeout ended (REQ-NET-BYE-007).
    ByeTimeout,
    /// Host: the Host answer timeout ended (REQ-NET-SES-002).
    Expired,
    /// The Handshake ended the entry with one Bye of this reason and no Pending Bye (for
    /// example the authentication failure limit, REQ-NET-AUTH-022).
    Ended(ByeReason),
}

/// An event of the Session for the caller. Events of [`Session::receive`] can borrow the
/// received UDP payload (NAK and Bye Text Message).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent<'a> {
    /// Host: a new Client asks for a session and the caller decides with [`Session::accept`]
    /// or [`Session::reject`] (REQ-NET-INV-026). The identity is in [`Session::peer`].
    InvitationReceived {
        /// The Capabilities of the Invitation.
        capabilities: Capabilities,
    },
    /// Client: the first Invitation Reply: Pending (REQ-NET-INV-027).
    Pending,
    /// Client: the Host asks for authentication (REQ-NET-AUTH-034).
    AuthRequested(Challenge),
    /// Host: a failed authentication (REQ-NET-AUTH-022, -025).
    AuthFailed {
        /// The method that the Host asked for.
        method: Method,
        /// The failures of this entry.
        failures: u32,
        /// True when the Username was not found.
        username_not_found: bool,
    },
    /// A problem with the credentials of the caller (N-53).
    CredentialsError(CredentialsError),
    /// The session is established; the identity of the peer is in [`Session::peer`]
    /// (REQ-NET-INV-020).
    Established {
        /// The authentication method, if any.
        method: Option<Method>,
    },
    /// This side ends the session or the Invitation with a Bye of this reason and goes to
    /// Pending Bye (REQ-NET-BYE-006, -014).
    Ending(ByeReason),
    /// The session went to Idle (REQ-NET-BYE-007, -008, -011).
    Closed(CloseCause<'a>),
    /// The session was reset; the application resets its own state (REQ-NET-RST-010).
    Reset,
    /// This side started a Session Reset by itself (an unexpected Session Reset Reply,
    /// REQ-NET-RST-012): the session is in Pending Session Reset; [`SessionEvent::Reset`]
    /// follows with the Session Reset Reply of the peer.
    ResetStarted,
    /// A received NAK: NAK Reason (raw value `.reason.0`), NAK'ed Command Header and Text
    /// Message (REQ-NET-NAK-005).
    Nak(Nak<'a>),
    /// A Ping Reply for an open Ping, with the round-trip time (REQ-NET-PING-007).
    PingReply {
        /// The Ping Id.
        id: u32,
        /// The time from the write of the Ping to the receive of the reply, in ms.
        rtt_ms: u64,
    },
    /// A Ping Reply with no open Ping of this Id (REQ-NET-PING-008).
    UnexpectedPingReply(u32),
    /// The Ping miss limit ended the session (REQ-NET-PING-009); an [`SessionEvent::Ending`]
    /// with Bye Reason 0x04 follows.
    Timeout,
    /// An event of the UMP Data stream (data loss, Retransmit Error).
    Stream(StreamEvent),
}

/// The events of one call, at most [`EVENTS_CAPACITY`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Events<'a> {
    items: [Option<SessionEvent<'a>>; EVENTS_CAPACITY],
}

impl<'a> Events<'a> {
    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    fn push(&mut self, e: SessionEvent<'a>) {
        if let Some(slot) = self.items.iter_mut().find(|s| s.is_none()) {
            *slot = Some(e);
        }
    }

    /// The events, in order.
    pub fn iter(&self) -> impl Iterator<Item = &SessionEvent<'a>> {
        self.items.iter().flatten()
    }

    /// True when there is no event.
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(Option::is_none)
    }

    /// The number of events.
    pub fn len(&self) -> usize {
        self.iter().count()
    }
}

/// The result of [`Session::receive`] for one received Command.
#[derive(Debug, Clone, Copy, Default)]
pub struct Received<'a> {
    /// The events.
    pub events: Events<'a>,
    /// For a UMP Data Command in Established Session: the delivery of the Stream, which
    /// borrows the received payload (REQ-NET-DATA-008, -010).
    pub data: Option<Delivery<'a>>,
}

/// A reply with no session state, for an address with no entry: the Endpoint writes it once
/// (N-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stateless {
    /// Ping Reply with this Ping Id (REQ-NET-PING-005).
    PingReply(u32),
    /// Bye Reply (REQ-NET-BYE-009).
    ByeReply,
    /// Bye with this reason and no Text Message (REQ-NET-DATA-015, REQ-NET-RST-011, the
    /// Invitation sections).
    Bye(ByeReason),
    /// NAK with this reason and the received header word.
    Nak(NakReason, Header),
}

impl Stateless {
    /// Writes the Command; `role` is the role of this endpoint (Bye Reason check, N-30).
    pub fn write(&self, role: Role, w: &mut PayloadWriter<'_>) -> Result<(), CommandError> {
        match *self {
            Stateless::PingReply(id) => write_ping_reply(w, id),
            Stateless::ByeReply => write_empty(w, CommandCode::ByeReply),
            Stateless::Bye(r) => Bye::write(w, role, r, ""),
            Stateless::Nak(r, h) => Nak::write(w, r, h, ""),
        }
    }
}

/// A Host Handshake that a received Invitation started from Idle: give it to
/// [`Session::open`] of a new Host Session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opening<const U: usize = DEFAULT_USERNAME_CAPACITY> {
    hs: Handshake<U>,
    outcome: Outcome,
}

impl<const U: usize> Opening<U> {
    /// The Handshake of the new entry.
    pub const fn handshake(&self) -> &Handshake<U> {
        &self.hs
    }

    /// The Outcome that the new Session applies.
    pub const fn outcome(&self) -> &Outcome {
        &self.outcome
    }
}

/// The answer of [`Session::idle_reply`] for a Command from an address with no entry.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleReply<'a, const U: usize = DEFAULT_USERNAME_CAPACITY> {
    /// No reply.
    Ignore,
    /// Write this reply once; keep no entry (N-16). The events (for example a credentials
    /// error of the Handshake with its Bye) go to the caller.
    Send(Stateless, Events<'a>),
    /// Give this event to the caller, no reply (a NAK, REQ-NET-NAK-005).
    Event(SessionEvent<'a>),
    /// Host: an Invitation starts a session: put a new Host Session in a free place and call
    /// [`Session::open`].
    Open(Opening<U>),
}

/// A one-shot Command that waits for the next UDP payload (REQ-NET-DATA-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    PingReply(u32),
    Nak(NakReason, Header),
    RetransmitError(RetransmitError),
    Bye(ByeReason),
    Reply(Reply),
}

impl Item {
    // REQ-NET-BYE-013: Ping Reply and NAK only (Bye and Bye Reply are not in the queue).
    fn allowed_in_pending_bye(&self) -> bool {
        matches!(
            self,
            Item::PingReply(_) | Item::Nak(..) | Item::Reply(Reply::Nak(..))
        )
    }

    // The replies that stay when the session goes to Idle.
    fn allowed_in_idle(&self) -> bool {
        matches!(
            self,
            Item::PingReply(_)
                | Item::Nak(..)
                | Item::Bye(_)
                | Item::Reply(Reply::Nak(..) | Reply::Bye(_))
        )
    }
}

/// The FIFO of one-shot Commands, capacity R. A full queue drops the new Command, as a lost
/// UDP payload would.
#[derive(Debug, Clone)]
struct ReplyQueue<const R: usize> {
    ring: Ring<Item, R>,
    dropped: u32,
}

impl<const R: usize> ReplyQueue<R> {
    const CHECK: () = assert!(R >= 1, "the reply queue R must be 1 or more");

    const fn new() -> Self {
        let () = Self::CHECK;
        ReplyQueue {
            ring: Ring::new(Item::PingReply(0)),
            dropped: 0,
        }
    }

    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    fn push(&mut self, it: Item) {
        if self.ring.push_back(it).is_err() {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    fn get(&self, i: usize) -> Option<Item> {
        self.ring.get(i)
    }

    fn remove(&mut self, i: usize) {
        self.ring.remove(i);
    }

    fn retain(&mut self, keep: impl Fn(&Item) -> bool) {
        let mut i = 0;
        while let Some(it) = self.get(i) {
            if keep(&it) {
                i += 1;
            } else {
                self.remove(i);
            }
        }
    }

    fn contains(&self, it: Item) -> bool {
        self.ring.iter().any(|x| *x == it)
    }
}

/// The repeated invitation Command of the Handshake (REQ-NET-SES-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HsRepeat {
    reply: Reply,
    due: u64,
}

/// The Session Reset of this side (REQ-NET-RST-003 to -005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResetJob {
    started: u64,
    due: u64,
}

/// The Bye of this side in Pending Bye (REQ-NET-BYE-006, -007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ByeJob {
    reason: ByeReason,
    text: [u8; BYE_TEXT_CAPACITY],
    text_len: u8,
    sent_at: Option<u64>,
    due: u64,
}

impl ByeJob {
    fn text(&self) -> &str {
        core::str::from_utf8(self.text.get(..usize::from(self.text_len)).unwrap_or(&[]))
            .unwrap_or("")
    }
}

fn opposite(role: Role) -> Role {
    match role {
        Role::Host => Role::Client,
        Role::Client => Role::Host,
    }
}

// The Command Code of a repeated Handshake Command.
fn reply_code(r: Reply) -> CommandCode {
    match r {
        Reply::Invitation => CommandCode::Invitation,
        Reply::InvitationWithAuth => CommandCode::InvitationWithAuth,
        Reply::InvitationWithUserAuth => CommandCode::InvitationWithUserAuth,
        Reply::Accepted => CommandCode::InvitationAccepted,
        Reply::Pending => CommandCode::InvitationPending,
        Reply::AuthRequired { .. } => CommandCode::InvitationAuthRequired,
        Reply::UserAuthRequired { .. } => CommandCode::InvitationUserAuthRequired,
        Reply::Bye(_) => CommandCode::Bye,
        Reply::Nak(..) => CommandCode::Nak,
    }
}

fn is_overflow(e: &SessionError) -> bool {
    const O: PacketError = PacketError::Overflow;
    matches!(
        e,
        SessionError::Command(CommandError::Packet(O))
            | SessionError::Handshake(HandshakeError::Command(CommandError::Packet(O)))
            | SessionError::Stream(StreamError::Packet(O))
    )
}

// Ok(true): written; Ok(false): no room, `more` is set; Err: another error.
fn put(out: &mut Written, r: Result<(), SessionError>) -> Result<bool, SessionError> {
    match r {
        Ok(()) => {
            out.commands += 1;
            Ok(true)
        }
        Err(e) if is_overflow(&e) => {
            out.more = true;
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

// The Text Message rules of the Bye writer (6.16) and the capacity of the Session.
fn check_bye_text(text: &str) -> Result<(), SessionError> {
    if text.len() > BYE_TEXT_CAPACITY || text.len() > MAX_BYE_TEXT {
        return Err(CommandError::TextTooLong.into());
    }
    if text.starts_with('\u{FEFF}') || text.contains('\0') {
        return Err(CommandError::InvalidText.into());
    }
    Ok(())
}

fn map_event<'a>(e: HandshakeEvent) -> Option<SessionEvent<'a>> {
    Some(match e {
        HandshakeEvent::InvitationReceived { capabilities, .. } => {
            SessionEvent::InvitationReceived { capabilities }
        }
        HandshakeEvent::Established { method, .. } => SessionEvent::Established { method },
        HandshakeEvent::Pending { .. } => SessionEvent::Pending,
        HandshakeEvent::AuthRequested { challenge, .. } => SessionEvent::AuthRequested(challenge),
        HandshakeEvent::AuthFailed {
            method,
            failures,
            username_not_found,
        } => SessionEvent::AuthFailed {
            method,
            failures,
            username_not_found,
        },
        HandshakeEvent::Ended(r) => SessionEvent::Ending(r),
        HandshakeEvent::CredentialsError(c) => SessionEvent::CredentialsError(c),
        // Given as Closed(Expired) by `apply`.
        HandshakeEvent::Expired => return None,
    })
}

/// The state machine for one peer address (Host or Client role).
///
/// W, C and Q are the sizes of the [`DataStream`] buffers (receive window, send history in
/// words, Retransmit Request queue; N-01, N-40, N-41), P the number of open Pings (N-27), U
/// the Username buffer of the [`Handshake`] (N-52) and R the capacity of the one-shot reply
/// queue (NAK, Retransmit Error, Ping Reply, single Bye, Handshake replies).
#[derive(Debug, Clone)]
pub struct Session<
    const W: usize = 64,
    const C: usize = 256,
    const Q: usize = 4,
    const P: usize = 8,
    const U: usize = DEFAULT_USERNAME_CAPACITY,
    const R: usize = 4,
> {
    config: SessionConfig,
    core: SessionCore<W, C, Q, P, U, R>,
}

impl<
        const W: usize,
        const C: usize,
        const Q: usize,
        const P: usize,
        const U: usize,
        const R: usize,
    > Session<W, C, Q, P, U, R>
{
    fn new(role: Role, hs: Handshake<U>, config: SessionConfig) -> Result<Self, SessionError> {
        config.check()?;
        DataStream::<W, C, Q>::new(config.stream)?;
        Ok(Session {
            config,
            core: SessionCore::idle(role, hs),
        })
    }

    /// A Host Session in Idle; `config` is checked. Give it the [`Opening`] of
    /// [`Session::idle_reply`] with [`Session::open`].
    pub fn host(config: SessionConfig) -> Result<Self, SessionError> {
        Self::new(Role::Host, Handshake::host(), config)
    }

    /// A Client Session in Idle; `config` is checked. `capabilities` gives the
    /// authentication methods that the caller enables for this Host (REQ-NET-AUTH-013).
    /// Start it with [`Session::connect`].
    pub fn client(config: SessionConfig, capabilities: Capabilities) -> Result<Self, SessionError> {
        Self::new(Role::Client, Handshake::client(capabilities), config)
    }

    /// The configuration.
    pub const fn config(&self) -> &SessionConfig {
        &self.config
    }

    /// The answer to a Command from an address with no entry (Idle), with no state
    /// (REQ-NET-SES-001, N-16):
    /// - invitation-family Commands go to a new Handshake of `role` in Idle. A Host
    ///   Invitation that starts a session gives [`IdleReply::Open`]; the Endpoint checks the
    ///   table capacity before (Bye 0x40, REQ-NET-INV-035);
    /// - Ping: Ping Reply (REQ-NET-PING-005); Bye: Bye Reply (REQ-NET-BYE-009); Bye Reply:
    ///   nothing (REQ-NET-BYE-011); NAK: the NAK event, no reply (REQ-NET-NAK-005, N-29);
    /// - Ping Reply: no open Ping; NAK 0x02 with the option of REQ-NET-PING-008, else nothing
    ///   (REQ-NET-PING-010);
    /// - Session Reset and Session Reset Reply: Bye 0x05 (REQ-NET-RST-011);
    /// - UMP Data, Retransmit Request and Retransmit Error: Bye 0x05 when `first_in_payload`
    ///   (no earlier Command of the same UDP payload got a Bye 0x05), else nothing
    ///   (REQ-NET-DATA-015, N-16);
    /// - unknown Command Codes: nothing (the Endpoint writes NAK 0x01, REQ-NET-CMD-002).
    pub fn idle_reply<'a, Cr: Credentials + ?Sized>(
        now: u64,
        role: Role,
        header: Header,
        cmd: &Command<'a>,
        first_in_payload: bool,
        config: &SessionConfig,
        env: &mut Env<'_, Cr>,
    ) -> IdleReply<'a, U> {
        let not_established = Stateless::Bye(ByeReason::SESSION_NOT_ESTABLISHED);
        let send = |st| IdleReply::Send(st, Events::default());
        match *cmd {
            Command::Invitation(_) | Command::InvitationReply(_) | Command::Auth(_) => {
                let mut hs = match role {
                    Role::Host => Handshake::host(),
                    Role::Client => Handshake::client(Capabilities::default()),
                };
                let outcome =
                    hs.on_command(&config.handshake, now, SessionState::Idle, header, cmd, env);
                let mut ev = Events::default();
                for e in outcome.events.iter() {
                    if let Some(se) = map_event(*e) {
                        ev.push(se);
                    }
                }
                match (outcome.next, outcome.reply) {
                    (Some(s), _) if s != SessionState::Idle => {
                        IdleReply::Open(Opening { hs, outcome })
                    }
                    (_, Some(Reply::Bye(r))) => IdleReply::Send(Stateless::Bye(r), ev),
                    (_, Some(Reply::Nak(r, h))) => IdleReply::Send(Stateless::Nak(r, h), ev),
                    _ => IdleReply::Ignore,
                }
            }
            Command::Ping(id) => send(Stateless::PingReply(id)),
            Command::PingReply(_) => {
                if config.nak_bad_ping_reply {
                    send(Stateless::Nak(NakReason::COMMAND_NOT_EXPECTED, header))
                } else {
                    IdleReply::Ignore
                }
            }
            Command::Bye(_) => send(Stateless::ByeReply),
            Command::ByeReply | Command::Unknown(_) => IdleReply::Ignore,
            Command::Nak(n) => IdleReply::Event(SessionEvent::Nak(n)),
            Command::SessionReset | Command::SessionResetReply => send(not_established),
            Command::UmpData(_) | Command::RetransmitRequest(_) | Command::RetransmitError(_) => {
                if first_in_payload {
                    send(not_established)
                } else {
                    IdleReply::Ignore
                }
            }
        }
    }

    /// The earliest time at which [`Session::poll`] or [`Session::write_payload`] has work:
    /// a repeat, a timeout, a Ping or a stream timer. When Commands wait already, it gives
    /// the last time that the caller gave. `None`: nothing to do.
    pub fn next_deadline(&self) -> Option<u64> {
        self.core.next_deadline(&self.config)
    }

    /// True when [`Session::write_payload`] has Commands to write at `now`.
    pub fn wants_write(&self, now: u64) -> bool {
        self.core.wants_write(&self.config, now)
    }

    /// Host: starts the session of an [`IdleReply::Open`] in this Session (which is in Idle):
    /// the Session takes the Handshake and applies its Outcome (reply, state, events).
    pub fn open(&mut self, now: u64, opening: Opening<U>) -> Result<Events<'static>, SessionError> {
        self.core.open(now, opening)
    }

    /// Client: starts a session with the Host of this Session: Invitation in the next UDP
    /// payload, Pending Invitation, repeats and Invitation timeout (REQ-NET-INV-005, -009,
    /// -011). A new session starts with no NAK flags (REQ-NET-CMD-003).
    pub fn connect(&mut self, now: u64) -> Result<(), SessionError> {
        self.core.connect(&self.config, now)
    }

    // ----- received Commands -----

    /// Handles one received Command of the peer of this Session; `header` is its header word
    /// as received. The Endpoint checked the Signature, wrote NAK 0x01 for an unknown Command
    /// Code and NAK 0x03 for a malformed Command (it never gives a NAK for a NAK, N-29).
    ///
    /// The Session applies Table 9, the Command sections and the role direction
    /// (REQ-NET-SES-003, -004, -008, -009), Ping and Ping Reply (REQ-NET-PING-005, -007, -008,
    /// -010), NAK (REQ-NET-NAK-005, REQ-NET-CMD-003), Bye and Bye Reply (REQ-NET-BYE-008 to
    /// -011), Session Reset (REQ-NET-RST-006 to -008, -011, -012), UMP Data and Retransmit
    /// (REQ-NET-DATA-003, -015, REQ-NET-RTX-004, -007, -012), and gives the invitation-family
    /// Commands to the Handshake. The replies wait for [`Session::write_payload`].
    pub fn receive<'a, Cr: Credentials + ?Sized>(
        &mut self,
        now: u64,
        header: Header,
        cmd: &Command<'a>,
        env: &mut Env<'_, Cr>,
    ) -> Received<'a> {
        self.core.receive(&self.config, now, header, cmd, env)
    }

    // ----- caller requests -----

    /// Queues whole UMP messages for the peer in UMP Data Commands (REQ-NET-DATA-002). Only
    /// in Established Session; in Pending Session Reset the data is refused and not kept
    /// (REQ-NET-RST-006).
    pub fn send_ump(&mut self, words: &[u32]) -> Result<(), SessionError> {
        self.core.send_ump(&self.config, words)
    }

    /// Starts a Session Reset (REQ-NET-RST-003): Session Reset in the next UDP payload,
    /// repeats and timeout (REQ-NET-RST-004, -005), Pending Session Reset. Only in
    /// Established Session.
    pub fn reset(&mut self, now: u64) -> Result<(), SessionError> {
        self.core.reset(now)
    }

    /// Ends the session or the pending Invitation with a Bye of `reason` and `text` (at most
    /// [`BYE_TEXT_CAPACITY`] bytes), then Pending Bye (REQ-NET-BYE-006). In Pending
    /// Invitation and Authentication Required the Handshake checks the reason
    /// (REQ-NET-INV-010: Client 0x80; Host 0x40 to 0x45 and the general reasons). In other
    /// states the reason must be allowed for the role (N-30). In Idle and Pending Bye: an
    /// error.
    pub fn bye(
        &mut self,
        now: u64,
        reason: ByeReason,
        text: &str,
    ) -> Result<Events<'static>, SessionError> {
        self.core.bye(&self.config, now, reason, text)
    }

    /// Cancels the pending Invitation with the default Bye Reason (Client 0x80, Host 0x42;
    /// REQ-NET-INV-010, N-56).
    pub fn cancel(&mut self, now: u64) -> Result<Events<'static>, SessionError> {
        self.core.cancel(&self.config, now)
    }

    /// Host: the caller accepts the Client after [`SessionEvent::InvitationReceived`]
    /// (REQ-NET-INV-026): Accepted and Established Session.
    pub fn accept(&mut self, now: u64) -> Result<Events<'static>, SessionError> {
        self.core.accept(&self.config, now)
    }

    /// Host: the caller rejects the Client (REQ-NET-INV-026); `None` gives Bye 0x42 (N-56).
    pub fn reject(
        &mut self,
        now: u64,
        reason: Option<ByeReason>,
    ) -> Result<Events<'static>, SessionError> {
        self.core.reject(&self.config, now, reason)
    }

    /// Client: answers a challenge later, after [`Credentials::answer_now`] gave false
    /// (REQ-NET-AUTH-034, -035).
    pub fn answer<Cr: Credentials + ?Sized>(
        &mut self,
        now: u64,
        creds: &mut Cr,
    ) -> Events<'static> {
        self.core.answer(&self.config, now, creds)
    }

    /// Writes a Ping with Ping Id `id` in the next UDP payload (REQ-NET-PING-003). Not in
    /// Pending Bye (REQ-NET-BYE-013) and not after NAK 0x01 for Ping (REQ-NET-CMD-003).
    /// `Busy` while an earlier caller Ping waits to be written. A Ping changes no state.
    pub fn ping(&mut self, now: u64, id: u32) -> Result<(), SessionError> {
        self.core.ping(now, id)
    }

    /// Runs the timeouts at `now` (REQ-NET-SES-007): the Invitation, Pending and Host answer
    /// timeouts and the delayed authentication reply (Handshake), the Session Reset timeout
    /// (Bye 0x04, REQ-NET-RST-005), the Bye timeout (Idle, REQ-NET-BYE-007), the Ping miss
    /// limit (Bye 0x04, REQ-NET-PING-009) and the timers of the stream. Call it at
    /// [`Session::next_deadline`] and before [`Session::write_payload`].
    pub fn poll(&mut self, now: u64) -> Events<'static> {
        self.core.poll(&self.config, now)
    }

    /// Writes the Commands of this peer that are due at `now` into one UDP payload, in this
    /// order: Ping Reply first (REQ-NET-PING-006), Bye Reply and Session Reset Reply, the
    /// queued NAK, Retransmit Error, single Bye and Handshake replies (each once,
    /// REQ-NET-DATA-014), the repeated invitation Command, Session Reset or Bye
    /// (REQ-NET-SES-005), the Ping (REQ-NET-PING-004), then the UMP Data stream: Retransmit
    /// Request, resent Commands, new Commands (REQ-NET-RTX-015). In Pending Bye only Bye,
    /// Ping Reply, Bye Reply and NAK (REQ-NET-BYE-013). `identity` is the identity of this
    /// endpoint. When [`Written::more`] is true, send the payload and call it again.
    pub fn write_payload(
        &mut self,
        now: u64,
        identity: &Identity,
        w: &mut PayloadWriter<'_>,
    ) -> Result<Written, SessionError> {
        self.core.write_payload(&self.config, now, identity, w)
    }

    /// The role of this side.
    pub const fn role(&self) -> Role {
        self.core.role()
    }

    /// The session state of the peer (REQ-NET-SES-001).
    pub const fn state(&self) -> SessionState {
        self.core.state()
    }

    /// The identity of the peer, after the first Command that carries it.
    pub const fn peer(&self) -> Option<&PeerInfo> {
        self.core.peer()
    }

    /// The Handshake of this entry.
    pub const fn handshake(&self) -> &Handshake<U> {
        self.core.handshake()
    }

    /// The UMP Data stream.
    pub const fn stream(&self) -> &DataStream<W, C, Q> {
        self.core.stream()
    }

    /// The open Pings (REQ-NET-PING-007).
    pub const fn open_pings(&self) -> &OpenPings<P> {
        self.core.open_pings()
    }

    /// The Pings written after the last Ping Reply (REQ-NET-PING-009).
    pub const fn missed_pings(&self) -> u32 {
        self.core.missed_pings()
    }

    /// The one-shot replies that the full queue dropped.
    pub const fn dropped_replies(&self) -> u32 {
        self.core.dropped_replies()
    }

    /// True when the peer answered NAK 0x01 for `code` in this session (REQ-NET-CMD-003).
    pub fn is_refused(&self, code: CommandCode) -> bool {
        self.core.is_refused(code)
    }

    /// The Ping Id of the next automatic Ping; it increments by 1 for each automatic Ping.
    pub fn set_next_ping_id(&mut self, id: u32) {
        self.core.set_next_ping_id(id)
    }

    /// True when the Session is in Idle and has no Command to write: the Endpoint may free
    /// the entry.
    pub fn is_finished(&self) -> bool {
        self.core.is_finished()
    }
}

/// The state of one session with no configuration: the Endpoint keeps one for each entry and
/// one [`SessionConfig`] for all of them (OPT-009). [`Session`] is a core and its configuration.
#[derive(Debug, Clone)]
pub(crate) struct SessionCore<
    const W: usize = 64,
    const C: usize = 256,
    const Q: usize = 4,
    const P: usize = 8,
    const U: usize = DEFAULT_USERNAME_CAPACITY,
    const R: usize = 4,
> {
    role: Role,
    state: SessionState,
    hs: Handshake<U>,
    stream: DataStream<W, C, Q>,
    pings: OpenPings<P>,
    // Bit c: the peer answered NAK 0x01 for Command Code c (REQ-NET-CMD-003).
    flags: [u32; 8],
    queue: ReplyQueue<R>,
    // Bye Reply and Session Reset Reply Commands to write, one for each received Command
    // (REQ-NET-BYE-010, REQ-NET-RST-008).
    bye_replies: u16,
    reset_replies: u16,
    hs_repeat: Option<HsRepeat>,
    reset: Option<ResetJob>,
    // The start of the last Session Reset of this side (REQ-NET-RST-012).
    last_reset: Deadline,
    bye: Option<ByeJob>,
    ping_request: Option<u32>,
    next_ping_id: u32,
    last_ping_at: u64,
    missed: u32,
    // The last time that the caller gave.
    now: u64,
}

impl<
        const W: usize,
        const C: usize,
        const Q: usize,
        const P: usize,
        const U: usize,
        const R: usize,
    > SessionCore<W, C, Q, P, U, R>
{
    /// An Idle core from checked parts. It returns no `Result`, so that the compiler can
    /// build it in the place of the caller (the session table slot, OPT-003).
    #[inline(always)]
    fn idle(role: Role, hs: Handshake<U>) -> Self {
        SessionCore {
            role,
            state: SessionState::Idle,
            hs,
            // Built here, not passed by value: no copy of the stream on the stack (OPT-014).
            stream: DataStream::checked(),
            pings: OpenPings::new(),
            flags: [0; 8],
            queue: ReplyQueue::new(),
            bye_replies: 0,
            reset_replies: 0,
            hs_repeat: None,
            reset: None,
            last_reset: Deadline::NONE,
            bye: None,
            ping_request: None,
            next_ping_id: 0,
            last_ping_at: 0,
            missed: 0,
            now: 0,
        }
    }

    /// An Idle Host core for a configuration that the caller checked (`SessionConfig::check`).
    #[inline(always)]
    pub(crate) fn host_checked() -> Self {
        Self::idle(Role::Host, Handshake::host())
    }

    /// An Idle Client core for a configuration that the caller checked.
    #[inline(always)]
    pub(crate) fn client_checked(capabilities: Capabilities) -> Self {
        Self::idle(Role::Client, Handshake::client(capabilities))
    }

    /// The role of this side.
    pub(crate) const fn role(&self) -> Role {
        self.role
    }

    /// The session state of the peer (REQ-NET-SES-001).
    pub(crate) const fn state(&self) -> SessionState {
        self.state
    }

    /// The identity of the peer, after the first Command that carries it.
    pub(crate) const fn peer(&self) -> Option<&PeerInfo> {
        self.hs.peer()
    }

    /// The Handshake of this entry.
    pub(crate) const fn handshake(&self) -> &Handshake<U> {
        &self.hs
    }

    /// The UMP Data stream.
    pub(crate) const fn stream(&self) -> &DataStream<W, C, Q> {
        &self.stream
    }

    /// The open Pings (REQ-NET-PING-007).
    pub(crate) const fn open_pings(&self) -> &OpenPings<P> {
        &self.pings
    }

    /// The Pings written after the last Ping Reply (REQ-NET-PING-009).
    pub(crate) const fn missed_pings(&self) -> u32 {
        self.missed
    }

    /// The one-shot replies that the full queue dropped.
    pub(crate) const fn dropped_replies(&self) -> u32 {
        self.queue.dropped
    }

    /// True when the peer answered NAK 0x01 for `code` in this session (REQ-NET-CMD-003).
    pub(crate) fn is_refused(&self, code: CommandCode) -> bool {
        let c = code.into_bits();
        self.flags[usize::from(c >> 5)] & (1 << (c & 31)) != 0
    }

    fn refuse(&mut self, c: u8) {
        self.flags[usize::from(c >> 5)] |= 1 << (c & 31);
    }

    /// The Ping Id of the next automatic Ping; it increments by 1 for each automatic Ping.
    pub(crate) fn set_next_ping_id(&mut self, id: u32) {
        self.next_ping_id = id;
    }

    /// True when the Session is in Idle and has no Command to write: the Endpoint may free
    /// the entry.
    pub(crate) fn is_finished(&self) -> bool {
        self.state == SessionState::Idle
            && self.queue.ring.len() == 0
            && self.bye_replies == 0
            && self.reset_replies == 0
            && self.ping_request.is_none()
    }

    // ----- timers -----

    fn ping_enabled(&self, cfg: &SessionConfig) -> bool {
        cfg.ping_interval_ms > 0
            && matches!(
                self.state,
                SessionState::EstablishedSession | SessionState::PendingSessionReset
            )
            && !self.is_refused(CommandCode::Ping)
    }

    fn ping_due_at(&self, cfg: &SessionConfig) -> Option<u64> {
        self.ping_enabled(cfg)
            .then(|| self.last_ping_at.saturating_add(cfg.ping_interval_ms))
    }

    fn miss_limit_reached(&self, cfg: &SessionConfig) -> bool {
        self.state == SessionState::EstablishedSession
            && cfg.ping_miss_limit > 0
            && self.missed >= cfg.ping_miss_limit
    }

    fn auto_ping_due(&self, cfg: &SessionConfig, now: u64) -> bool {
        self.ping_due_at(cfg).is_some_and(|t| now >= t) && !self.miss_limit_reached(cfg)
    }

    fn reset_timed_out(&self, cfg: &SessionConfig, r: &ResetJob, now: u64) -> bool {
        now >= r.started.saturating_add(cfg.reset_timeout_ms)
    }

    fn bye_timed_out(&self, cfg: &SessionConfig, b: &ByeJob, now: u64) -> bool {
        b.sent_at
            .is_some_and(|s| now >= s.saturating_add(cfg.bye_timeout_ms))
    }

    fn hs_repeat_due(&self, now: u64) -> Option<Reply> {
        let r = self.hs_repeat?;
        let active = matches!(
            self.state,
            SessionState::PendingInvitation | SessionState::AuthenticationRequired
        );
        // No repeat at or after a Handshake timeout: poll ends it (REQ-NET-SES-007).
        let timed_out = self.hs.next_deadline(self.state).is_some_and(|d| now >= d);
        (active && now >= r.due && !timed_out && !self.is_refused(reply_code(r.reply)))
            .then_some(r.reply)
    }

    fn has_queued(&self) -> bool {
        self.queue.ring.len() > 0
            || self.bye_replies > 0
            || self.reset_replies > 0
            || self.ping_request.is_some()
    }

    /// The earliest time at which [`Session::poll`] or [`Session::write_payload`] has work:
    /// a repeat, a timeout, a Ping or a stream timer. When Commands wait already, it gives
    /// the last time that the caller gave. `None`: nothing to do.
    pub(crate) fn next_deadline(&self, cfg: &SessionConfig) -> Option<u64> {
        let mut d: Option<u64> = None;
        let mut add = |t: Option<u64>| {
            if let Some(t) = t {
                d = Some(d.map_or(t, |x| x.min(t)));
            }
        };
        if self.has_queued() {
            add(Some(self.now));
        }
        match self.state {
            SessionState::PendingInvitation | SessionState::AuthenticationRequired => {
                // No due time for a Command that the peer refused (REQ-NET-CMD-003).
                add(self
                    .hs_repeat
                    .filter(|r| !self.is_refused(reply_code(r.reply)))
                    .map(|r| r.due));
                add(self.hs.next_deadline(self.state));
            }
            SessionState::PendingSessionReset => {
                if let Some(r) = self.reset {
                    if !self.is_refused(CommandCode::SessionReset) {
                        add(Some(r.due));
                    }
                    add(Some(r.started.saturating_add(cfg.reset_timeout_ms)));
                }
                add(self.ping_due_at(cfg));
            }
            SessionState::PendingBye => {
                if let Some(b) = self.bye {
                    add(Some(b.due));
                    add(b.sent_at.map(|s| s.saturating_add(cfg.bye_timeout_ms)));
                }
            }
            SessionState::EstablishedSession => {
                add(self.ping_due_at(cfg));
                add(self.stream.next_deadline());
                if self.stream.wants_write(self.now) {
                    add(Some(self.now));
                }
            }
            SessionState::Idle => {}
        }
        d
    }

    /// True when [`Session::write_payload`] has Commands to write at `now`.
    pub(crate) fn wants_write(&self, cfg: &SessionConfig, now: u64) -> bool {
        if self.has_queued() || self.hs_repeat_due(now).is_some() {
            return true;
        }
        match self.state {
            SessionState::PendingSessionReset => {
                self.reset
                    .is_some_and(|r| now >= r.due && !self.reset_timed_out(cfg, &r, now))
                    || self.auto_ping_due(cfg, now)
            }
            SessionState::PendingBye => self
                .bye
                .is_some_and(|b| now >= b.due && !self.bye_timed_out(cfg, &b, now)),
            SessionState::EstablishedSession => {
                self.auto_ping_due(cfg, now) || self.stream.wants_write(now)
            }
            _ => false,
        }
    }

    // ----- writer -----

    fn write_item(
        &self,
        item: Item,
        identity: &Identity,
        w: &mut PayloadWriter<'_>,
    ) -> Result<(), SessionError> {
        match item {
            Item::PingReply(id) => write_ping_reply(w, id)?,
            Item::Nak(r, h) => Nak::write(w, r, h, "")?,
            Item::RetransmitError(e) => e.write(w)?,
            Item::Bye(r) => Bye::write(w, self.role, r, "")?,
            Item::Reply(r) => r.write(&self.hs, identity, w)?,
        }
        Ok(())
    }

    /// Host: starts the session of an [`IdleReply::Open`] in this Session (which is in Idle):
    /// the Session takes the Handshake and applies its Outcome (reply, state, events).
    pub(crate) fn open(
        &mut self,
        now: u64,
        opening: Opening<U>,
    ) -> Result<Events<'static>, SessionError> {
        self.now = now;
        if self.role != Role::Host || opening.hs.role() != Role::Host {
            return Err(SessionError::WrongRole);
        }
        if self.state != SessionState::Idle {
            return Err(SessionError::WrongState(self.state));
        }
        let mut ev = Events::default();
        self.hs = opening.hs;
        self.apply(now, opening.outcome, &mut ev);
        Ok(ev)
    }

    /// Client: starts a session with the Host of this Session: Invitation in the next UDP
    /// payload, Pending Invitation, repeats and Invitation timeout (REQ-NET-INV-005, -009,
    /// -011). A new session starts with no NAK flags (REQ-NET-CMD-003).
    pub(crate) fn connect(&mut self, cfg: &SessionConfig, now: u64) -> Result<(), SessionError> {
        self.now = now;
        if self.role != Role::Client {
            return Err(SessionError::WrongRole);
        }
        if self.state != SessionState::Idle {
            return Err(SessionError::WrongState(self.state));
        }
        let o = self.hs.start(&cfg.handshake, now);
        let mut ev = Events::default();
        self.apply(now, o, &mut ev);
        Ok(())
    }

    // ----- state changes -----

    // A new session: no flags, empty stream, no Pings (REQ-NET-CMD-003, REQ-NET-RST-009).
    fn start_session(&mut self) {
        self.flags = [0; 8];
        self.stream.reset();
        self.pings.clear();
        self.missed = 0;
        self.last_reset = Deadline::NONE;
        self.reset = None;
        self.bye = None;
    }

    fn establish(&mut self, now: u64) {
        self.state = SessionState::EstablishedSession;
        self.stream.reset();
        self.pings.clear();
        self.missed = 0;
        self.last_ping_at = now;
        self.reset = None;
        self.last_reset = Deadline::NONE;
    }

    fn enter_pending_bye(&mut self, now: u64, reason: ByeReason, text: &str) {
        let mut t = [0u8; BYE_TEXT_CAPACITY];
        let b = text.as_bytes();
        let n = b.len().min(BYE_TEXT_CAPACITY);
        t[..n].copy_from_slice(&b[..n]);
        self.state = SessionState::PendingBye;
        self.bye = Some(ByeJob {
            reason,
            text: t,
            text_len: n as u8,
            sent_at: None,
            due: now,
        });
        self.hs_repeat = None;
        self.reset = None;
        self.ping_request = None;
        self.reset_replies = 0;
        self.queue.retain(Item::allowed_in_pending_bye);
    }

    // REQ-NET-BYE-006, -014: Bye in the next payload and Pending Bye.
    fn end_session(&mut self, now: u64, reason: ByeReason, text: &str, ev: &mut Events<'_>) {
        self.enter_pending_bye(now, reason, text);
        ev.push(SessionEvent::Ending(reason));
    }

    fn go_idle(&mut self) {
        self.state = SessionState::Idle;
        self.bye = None;
        self.hs_repeat = None;
        self.reset = None;
        self.ping_request = None;
        self.reset_replies = 0;
        self.missed = 0;
        self.hs.clear();
        self.stream.reset();
        self.pings.clear();
        self.queue.retain(Item::allowed_in_idle);
    }

    fn start_reset(&mut self, now: u64) {
        self.state = SessionState::PendingSessionReset;
        self.reset = Some(ResetJob {
            started: now,
            due: now,
        });
        self.last_reset = Deadline::at(now);
    }

    /// Applies an Outcome of the Handshake: reply, repeat, state and events.
    fn apply(&mut self, now: u64, o: Outcome, ev: &mut Events<'_>) {
        let before = self.state;
        let mut expired = false;
        for e in o.events.iter() {
            expired |= matches!(e, HandshakeEvent::Expired);
            if let Some(se) = map_event(*e) {
                ev.push(se);
            }
        }
        if before == SessionState::Idle && o.next.is_some_and(|s| s != SessionState::Idle) {
            self.start_session();
        }
        match o.repeat {
            Repeat::Start(r) => self.hs_repeat = Some(HsRepeat { reply: r, due: now }),
            Repeat::Stop => self.hs_repeat = None,
            Repeat::Keep => {}
        }
        // A repeated reply is written by the repeat (first send at `now`).
        let once = if matches!(o.repeat, Repeat::Start(_)) {
            None
        } else {
            o.reply
        };
        match o.next {
            Some(SessionState::PendingBye) => {
                let reason = match o.reply {
                    Some(Reply::Bye(r)) => r,
                    _ => ByeReason::UNKNOWN,
                };
                self.enter_pending_bye(now, reason, "");
            }
            Some(SessionState::Idle) => {
                self.go_idle();
                if let Some(r) = once {
                    self.queue.push(Item::Reply(r));
                }
                if before != SessionState::Idle {
                    let cause = if expired {
                        CloseCause::Expired
                    } else {
                        match o.reply {
                            Some(Reply::Bye(r)) => CloseCause::Ended(r),
                            _ => CloseCause::Ended(ByeReason::UNKNOWN),
                        }
                    };
                    ev.push(SessionEvent::Closed(cause));
                }
            }
            Some(SessionState::EstablishedSession) => {
                self.establish(now);
                if let Some(r) = once {
                    self.queue.push(Item::Reply(r));
                }
            }
            Some(s) => {
                self.state = s;
                if let Some(r) = once {
                    self.queue.push(Item::Reply(r));
                }
            }
            None => {
                if let Some(r) = once {
                    self.queue.push(Item::Reply(r));
                }
            }
        }
    }

    // ----- received Commands -----

    /// Handles one received Command of the peer of this Session; `header` is its header word
    /// as received. The Endpoint checked the Signature, wrote NAK 0x01 for an unknown Command
    /// Code and NAK 0x03 for a malformed Command (it never gives a NAK for a NAK, N-29).
    ///
    /// The Session applies Table 9, the Command sections and the role direction
    /// (REQ-NET-SES-003, -004, -008, -009), Ping and Ping Reply (REQ-NET-PING-005, -007, -008,
    /// -010), NAK (REQ-NET-NAK-005, REQ-NET-CMD-003), Bye and Bye Reply (REQ-NET-BYE-008 to
    /// -011), Session Reset (REQ-NET-RST-006 to -008, -011, -012), UMP Data and Retransmit
    /// (REQ-NET-DATA-003, -015, REQ-NET-RTX-004, -007, -012), and gives the invitation-family
    /// Commands to the Handshake. The replies wait for [`Session::write_payload`].
    pub(crate) fn receive<'a, Cr: Credentials + ?Sized>(
        &mut self,
        cfg: &SessionConfig,
        now: u64,
        header: Header,
        cmd: &Command<'a>,
        env: &mut Env<'_, Cr>,
    ) -> Received<'a> {
        self.now = now;
        let mut out = Received::default();
        let ev = &mut out.events;
        // REQ-NET-SES-002: each Command of the Client starts the Host answer timeout again,
        // except Ping, Ping Reply and NAK, which change no state (REQ-NET-PING-005,
        // REQ-NET-NAK-005).
        if !matches!(
            cmd,
            Command::Ping(_) | Command::PingReply(_) | Command::Nak(_)
        ) {
            self.hs.touch(&cfg.handshake, now);
        }
        let code = CommandCode::from_bits(header.code);
        // REQ-NET-SES-009: a Command of the direction of this role is not valid here; the
        // Command sections give no action for it (REQ-NET-INV-019, -030, REQ-NET-AUTH-030).
        if !opposite(self.role).may_send(code.direction()) {
            self.queue
                .push(Item::Nak(NakReason::COMMAND_NOT_EXPECTED, header));
            return out;
        }
        let state = self.state;
        match *cmd {
            Command::Invitation(_) | Command::InvitationReply(_) | Command::Auth(_) => {
                let o = self
                    .hs
                    .on_command(&cfg.handshake, now, state, header, cmd, env);
                self.apply(now, o, ev);
            }
            Command::Nak(n) => self.on_nak(cfg, now, n, ev),
            Command::Ping(id) => self.queue.push(Item::PingReply(id)),
            Command::PingReply(id) => self.on_ping_reply(cfg, now, id, header, ev),
            Command::Bye(b) => {
                // REQ-NET-BYE-008, -009, -010.
                self.bye_replies = self.bye_replies.saturating_add(1);
                if state != SessionState::Idle {
                    self.go_idle();
                    ev.push(SessionEvent::Closed(CloseCause::ByeReceived(b)));
                }
            }
            Command::ByeReply => {
                // REQ-NET-BYE-011.
                if state == SessionState::PendingBye {
                    self.go_idle();
                    ev.push(SessionEvent::Closed(CloseCause::ByeReply));
                }
            }
            Command::SessionReset => match state {
                // REQ-NET-RST-008, -009, -010, N-11.
                SessionState::EstablishedSession | SessionState::PendingSessionReset => {
                    self.reset_replies = self.reset_replies.saturating_add(1);
                    self.stream.reset();
                    ev.push(SessionEvent::Reset);
                }
                // REQ-NET-RST-011 exception.
                SessionState::PendingBye => {}
                _ => self.single_bye_05(),
            },
            Command::SessionResetReply => match state {
                // REQ-NET-RST-007, -009, -010.
                SessionState::PendingSessionReset => {
                    self.reset = None;
                    self.stream.reset();
                    self.state = SessionState::EstablishedSession;
                    ev.push(SessionEvent::Reset);
                }
                // REQ-NET-RST-012, N-10.
                SessionState::EstablishedSession => {
                    let recent = self
                        .last_reset
                        .get()
                        .is_some_and(|t| now < t.saturating_add(cfg.reset_timeout_ms));
                    if !recent && !self.is_refused(CommandCode::SessionReset) {
                        self.start_reset(now);
                        ev.push(SessionEvent::ResetStarted);
                    }
                }
                SessionState::PendingBye => {}
                _ => self.single_bye_05(),
            },
            Command::UmpData(d) => match state {
                SessionState::EstablishedSession => {
                    out.data = Some(self.stream.on_ump_data(&cfg.stream, now, d));
                }
                // REQ-NET-RST-006, REQ-NET-DATA-015, N-09, N-17.
                SessionState::PendingSessionReset | SessionState::PendingBye => {}
                _ => self.not_established(now, ev),
            },
            Command::RetransmitRequest(req) => match state {
                SessionState::EstablishedSession => {
                    match self.stream.on_retransmit_request(&cfg.stream, req) {
                        RequestOutcome::Queued | RequestOutcome::Ignored(_) => {}
                        // REQ-NET-RTX-006.
                        RequestOutcome::Error(e) => self.queue.push(Item::RetransmitError(e)),
                        // REQ-NET-RTX-007.
                        RequestOutcome::NotSupported => self
                            .queue
                            .push(Item::Nak(NakReason::COMMAND_NOT_SUPPORTED, header)),
                    }
                }
                SessionState::PendingSessionReset | SessionState::PendingBye => {}
                _ => self.not_established(now, ev),
            },
            Command::RetransmitError(e) => match state {
                // REQ-NET-RTX-012: no state change.
                SessionState::EstablishedSession => {
                    ev.push(SessionEvent::Stream(self.stream.on_retransmit_error(e)));
                }
                SessionState::PendingSessionReset | SessionState::PendingBye => {}
                _ => self.not_established(now, ev),
            },
            // The Endpoint answers NAK 0x01 (REQ-NET-CMD-002).
            Command::Unknown(_) => {}
        }
        out
    }

    // REQ-NET-DATA-003, -015: Bye 0x05; Pending Bye in Pending Invitation and
    // Authentication Required; one Bye with no state in Idle.
    fn not_established(&mut self, now: u64, ev: &mut Events<'_>) {
        match self.state {
            SessionState::PendingInvitation | SessionState::AuthenticationRequired => {
                self.end_session(now, ByeReason::SESSION_NOT_ESTABLISHED, "", ev);
            }
            SessionState::Idle => {
                // One Bye for each UDP payload (N-16): one waits already.
                let bye = Item::Bye(ByeReason::SESSION_NOT_ESTABLISHED);
                if !self.queue.contains(bye) {
                    self.queue.push(bye);
                }
            }
            _ => {}
        }
    }

    // REQ-NET-RST-011: one Bye 0x05, no state change.
    fn single_bye_05(&mut self) {
        self.queue
            .push(Item::Bye(ByeReason::SESSION_NOT_ESTABLISHED));
    }

    // REQ-NET-NAK-005, REQ-NET-CMD-003, REQ-NET-RTX-011.
    fn on_nak<'a>(&mut self, cfg: &SessionConfig, now: u64, n: Nak<'a>, ev: &mut Events<'a>) {
        ev.push(SessionEvent::Nak(n));
        if n.reason == NakReason::COMMAND_NOT_SUPPORTED {
            let c = n.nak_header.code;
            self.refuse(c);
            if c == CommandCode::RetransmitRequest.into_bits() {
                if let Some(e) = self.stream.on_retransmit_nak() {
                    ev.push(SessionEvent::Stream(e));
                }
            }
            self.end_refused_repeat(cfg, now, c, ev);
        }
    }

    // NAK 0x01 for the Command that this side repeats: no suitable reply can come, so the
    // session ends at once (REQ-NET-CMD-003). Invitation family: Bye 0x80 (Client) and
    // Pending Bye; Session Reset: Bye 0x04 and Pending Bye.
    fn end_refused_repeat(&mut self, cfg: &SessionConfig, now: u64, code: u8, ev: &mut Events<'_>) {
        let pending_invitation = matches!(
            self.state,
            SessionState::PendingInvitation | SessionState::AuthenticationRequired
        );
        if pending_invitation
            && self
                .hs_repeat
                .is_some_and(|r| reply_code(r.reply).into_bits() == code)
        {
            self.hs_repeat = None;
            if let Ok(o) = self.hs.cancel(&cfg.handshake, now, self.state, None) {
                self.apply(now, o, ev);
            }
        } else if self.state == SessionState::PendingSessionReset
            && code == CommandCode::SessionReset.into_bits()
        {
            self.reset = None;
            self.end_session(now, ByeReason::TIMEOUT, "", ev);
        }
    }

    // REQ-NET-PING-007, -008, -010.
    fn on_ping_reply(
        &mut self,
        cfg: &SessionConfig,
        now: u64,
        id: u32,
        header: Header,
        ev: &mut Events<'_>,
    ) {
        match self.pings.close(id) {
            Some(p) => {
                self.missed = 0;
                ev.push(SessionEvent::PingReply {
                    id,
                    rtt_ms: now.saturating_sub(p.sent_at),
                });
            }
            None => {
                if cfg.nak_bad_ping_reply {
                    let reason = if self.pings.is_empty() {
                        NakReason::COMMAND_NOT_EXPECTED
                    } else {
                        NakReason::BAD_PING_REPLY
                    };
                    self.queue.push(Item::Nak(reason, header));
                }
                ev.push(SessionEvent::UnexpectedPingReply(id));
            }
        }
    }

    // ----- caller requests -----

    /// Queues whole UMP messages for the peer in UMP Data Commands (REQ-NET-DATA-002). Only
    /// in Established Session; in Pending Session Reset the data is refused and not kept
    /// (REQ-NET-RST-006).
    pub(crate) fn send_ump(
        &mut self,
        cfg: &SessionConfig,
        words: &[u32],
    ) -> Result<(), SessionError> {
        if self.state != SessionState::EstablishedSession {
            return Err(SessionError::WrongState(self.state));
        }
        if self.is_refused(CommandCode::UmpData) {
            return Err(SessionError::Refused(CommandCode::UmpData));
        }
        self.stream.send_ump(&cfg.stream, words)?;
        Ok(())
    }

    /// Starts a Session Reset (REQ-NET-RST-003): Session Reset in the next UDP payload,
    /// repeats and timeout (REQ-NET-RST-004, -005), Pending Session Reset. Only in
    /// Established Session.
    pub(crate) fn reset(&mut self, now: u64) -> Result<(), SessionError> {
        self.now = now;
        if self.state != SessionState::EstablishedSession {
            return Err(SessionError::WrongState(self.state));
        }
        if self.is_refused(CommandCode::SessionReset) {
            return Err(SessionError::Refused(CommandCode::SessionReset));
        }
        self.start_reset(now);
        Ok(())
    }

    /// Ends the session or the pending Invitation with a Bye of `reason` and `text` (at most
    /// [`BYE_TEXT_CAPACITY`] bytes), then Pending Bye (REQ-NET-BYE-006). In Pending
    /// Invitation and Authentication Required the Handshake checks the reason
    /// (REQ-NET-INV-010: Client 0x80; Host 0x40 to 0x45 and the general reasons). In other
    /// states the reason must be allowed for the role (N-30). In Idle and Pending Bye: an
    /// error.
    pub(crate) fn bye(
        &mut self,
        cfg: &SessionConfig,
        now: u64,
        reason: ByeReason,
        text: &str,
    ) -> Result<Events<'static>, SessionError> {
        self.now = now;
        check_bye_text(text)?;
        let mut ev = Events::default();
        match self.state {
            SessionState::PendingInvitation | SessionState::AuthenticationRequired => {
                let o = self
                    .hs
                    .cancel(&cfg.handshake, now, self.state, Some(reason))?;
                self.apply(now, o, &mut ev);
                if let Some(b) = self.bye.as_mut() {
                    let n = text.len();
                    b.text[..n].copy_from_slice(text.as_bytes());
                    b.text_len = n as u8;
                }
            }
            SessionState::EstablishedSession | SessionState::PendingSessionReset => {
                if !reason.is_defined() || !reason.allowed_for(self.role) {
                    return Err(CommandError::ReasonNotAllowed.into());
                }
                self.end_session(now, reason, text, &mut ev);
            }
            s => return Err(SessionError::WrongState(s)),
        }
        Ok(ev)
    }

    /// Cancels the pending Invitation with the default Bye Reason (Client 0x80, Host 0x42;
    /// REQ-NET-INV-010, N-56).
    pub(crate) fn cancel(
        &mut self,
        cfg: &SessionConfig,
        now: u64,
    ) -> Result<Events<'static>, SessionError> {
        self.now = now;
        let o = self.hs.cancel(&cfg.handshake, now, self.state, None)?;
        if o.is_empty() {
            return Err(SessionError::WrongState(self.state));
        }
        let mut ev = Events::default();
        self.apply(now, o, &mut ev);
        Ok(ev)
    }

    /// Host: the caller accepts the Client after [`SessionEvent::InvitationReceived`]
    /// (REQ-NET-INV-026): Accepted and Established Session.
    pub(crate) fn accept(
        &mut self,
        cfg: &SessionConfig,
        now: u64,
    ) -> Result<Events<'static>, SessionError> {
        self.now = now;
        if self.role != Role::Host {
            return Err(SessionError::WrongRole);
        }
        let o = self.hs.accept(&cfg.handshake, now, self.state);
        if o.is_empty() {
            return Err(SessionError::WrongState(self.state));
        }
        let mut ev = Events::default();
        self.apply(now, o, &mut ev);
        Ok(ev)
    }

    /// Host: the caller rejects the Client (REQ-NET-INV-026); `None` gives Bye 0x42 (N-56).
    pub(crate) fn reject(
        &mut self,
        cfg: &SessionConfig,
        now: u64,
        reason: Option<ByeReason>,
    ) -> Result<Events<'static>, SessionError> {
        self.now = now;
        if self.role != Role::Host {
            return Err(SessionError::WrongRole);
        }
        let o = self.hs.reject(&cfg.handshake, now, self.state, reason)?;
        if o.is_empty() {
            return Err(SessionError::WrongState(self.state));
        }
        let mut ev = Events::default();
        self.apply(now, o, &mut ev);
        Ok(ev)
    }

    /// Client: answers a challenge later, after [`Credentials::answer_now`] gave false
    /// (REQ-NET-AUTH-034, -035).
    pub(crate) fn answer<Cr: Credentials + ?Sized>(
        &mut self,
        cfg: &SessionConfig,
        now: u64,
        creds: &mut Cr,
    ) -> Events<'static> {
        self.now = now;
        let o = self.hs.answer(&cfg.handshake, now, self.state, creds);
        let mut ev = Events::default();
        self.apply(now, o, &mut ev);
        ev
    }

    /// Writes a Ping with Ping Id `id` in the next UDP payload (REQ-NET-PING-003). Not in
    /// Pending Bye (REQ-NET-BYE-013) and not after NAK 0x01 for Ping (REQ-NET-CMD-003).
    /// `Busy` while an earlier caller Ping waits to be written. A Ping changes no state.
    pub(crate) fn ping(&mut self, now: u64, id: u32) -> Result<(), SessionError> {
        self.now = now;
        if self.state == SessionState::PendingBye {
            return Err(SessionError::WrongState(self.state));
        }
        if self.is_refused(CommandCode::Ping) {
            return Err(SessionError::Refused(CommandCode::Ping));
        }
        if self.ping_request.is_some() {
            return Err(SessionError::Busy);
        }
        self.ping_request = Some(id);
        Ok(())
    }

    /// Runs the timeouts at `now` (REQ-NET-SES-007): the Invitation, Pending and Host answer
    /// timeouts and the delayed authentication reply (Handshake), the Session Reset timeout
    /// (Bye 0x04, REQ-NET-RST-005), the Bye timeout (Idle, REQ-NET-BYE-007), the Ping miss
    /// limit (Bye 0x04, REQ-NET-PING-009) and the timers of the stream. Call it at
    /// [`Session::next_deadline`] and before [`Session::write_payload`].
    pub(crate) fn poll(&mut self, cfg: &SessionConfig, now: u64) -> Events<'static> {
        self.now = now;
        let mut ev = Events::default();
        match self.state {
            SessionState::PendingInvitation | SessionState::AuthenticationRequired => {
                let o = self.hs.poll(&cfg.handshake, now, self.state);
                if !o.is_empty() {
                    self.apply(now, o, &mut ev);
                }
            }
            SessionState::PendingSessionReset => {
                if self
                    .reset
                    .is_some_and(|r| self.reset_timed_out(cfg, &r, now))
                {
                    self.reset = None;
                    self.end_session(now, ByeReason::TIMEOUT, "", &mut ev);
                }
            }
            SessionState::PendingBye => {
                if self.bye.is_some_and(|b| self.bye_timed_out(cfg, &b, now)) {
                    self.go_idle();
                    ev.push(SessionEvent::Closed(CloseCause::ByeTimeout));
                }
            }
            SessionState::EstablishedSession => {
                if self.miss_limit_reached(cfg) && self.ping_due_at(cfg).is_some_and(|t| now >= t) {
                    ev.push(SessionEvent::Timeout);
                    self.end_session(now, ByeReason::TIMEOUT, "", &mut ev);
                } else {
                    for _ in 0..EVENTS_CAPACITY {
                        match self.stream.poll(&cfg.stream, now) {
                            Some(e) => ev.push(SessionEvent::Stream(e)),
                            None => break,
                        }
                    }
                }
            }
            SessionState::Idle => {}
        }
        ev
    }

    /// Writes the Commands of this peer that are due at `now` into one UDP payload, in this
    /// order: Ping Reply first (REQ-NET-PING-006), Bye Reply and Session Reset Reply, the
    /// queued NAK, Retransmit Error, single Bye and Handshake replies (each once,
    /// REQ-NET-DATA-014), the repeated invitation Command, Session Reset or Bye
    /// (REQ-NET-SES-005), the Ping (REQ-NET-PING-004), then the UMP Data stream: Retransmit
    /// Request, resent Commands, new Commands (REQ-NET-RTX-015). In Pending Bye only Bye,
    /// Ping Reply, Bye Reply and NAK (REQ-NET-BYE-013). `identity` is the identity of this
    /// endpoint. When [`Written::more`] is true, send the payload and call it again.
    pub(crate) fn write_payload(
        &mut self,
        cfg: &SessionConfig,
        now: u64,
        identity: &Identity,
        w: &mut PayloadWriter<'_>,
    ) -> Result<Written, SessionError> {
        self.now = now;
        let mut out = Written::default();
        // Ping Replies (REQ-NET-PING-005, -006).
        let mut i = 0;
        while let Some(item) = self.queue.get(i) {
            if let Item::PingReply(_) = item {
                let r = self.write_item(item, identity, w);
                if matches!(r, Err(ref e) if !is_overflow(e)) {
                    self.queue.remove(i);
                }
                if !put(&mut out, r)? {
                    return Ok(out);
                }
                self.queue.remove(i);
            } else {
                i += 1;
            }
        }
        // Bye Reply, one for each received Bye (REQ-NET-BYE-010).
        while self.bye_replies > 0 {
            if !put(
                &mut out,
                write_empty(w, CommandCode::ByeReply).map_err(Into::into),
            )? {
                return Ok(out);
            }
            self.bye_replies -= 1;
        }
        // Session Reset Reply, one for each received Session Reset (REQ-NET-RST-008).
        while self.reset_replies > 0 {
            if !put(
                &mut out,
                write_empty(w, CommandCode::SessionResetReply).map_err(Into::into),
            )? {
                return Ok(out);
            }
            self.reset_replies -= 1;
        }
        // The other one-shot replies, in order.
        while let Some(item) = self.queue.get(0) {
            if self.state == SessionState::PendingBye && !item.allowed_in_pending_bye() {
                self.queue.remove(0);
                continue;
            }
            let r = self.write_item(item, identity, w);
            if matches!(r, Err(ref e) if !is_overflow(e)) {
                // A reply that can not be written is dropped (for example a Handshake reply
                // whose data the entry no longer keeps).
                self.queue.remove(0);
            }
            if !put(&mut out, r)? {
                return Ok(out);
            }
            self.queue.remove(0);
        }
        // The repeated Commands (REQ-NET-SES-005, -006).
        if let Some(reply) = self.hs_repeat_due(now) {
            if !put(
                &mut out,
                reply.write(&self.hs, identity, w).map_err(Into::into),
            )? {
                return Ok(out);
            }
            if let Some(r) = self.hs_repeat.as_mut() {
                r.due = now.saturating_add(cfg.repeat_interval_ms);
            }
        }
        if self.state == SessionState::PendingSessionReset {
            if let Some(job) = self.reset {
                // REQ-NET-RST-005: no last Session Reset at the timeout.
                if now >= job.due
                    && !self.reset_timed_out(cfg, &job, now)
                    && !self.is_refused(CommandCode::SessionReset)
                {
                    if !put(
                        &mut out,
                        write_empty(w, CommandCode::SessionReset).map_err(Into::into),
                    )? {
                        return Ok(out);
                    }
                    self.reset = Some(ResetJob {
                        due: now.saturating_add(cfg.repeat_interval_ms),
                        ..job
                    });
                }
            }
        }
        if self.state == SessionState::PendingBye {
            if let Some(job) = self.bye {
                if now >= job.due && !self.bye_timed_out(cfg, &job, now) {
                    let r = Bye::write(w, self.role, job.reason, job.text()).map_err(Into::into);
                    if !put(&mut out, r)? {
                        return Ok(out);
                    }
                    self.bye = Some(ByeJob {
                        sent_at: Some(job.sent_at.unwrap_or(now)),
                        due: now.saturating_add(cfg.repeat_interval_ms),
                        ..job
                    });
                }
            }
            return Ok(out);
        }
        // Ping (REQ-NET-PING-003, -004).
        if let Some(id) = self.ping_request {
            if !put(&mut out, write_ping(w, id).map_err(Into::into))? {
                return Ok(out);
            }
            self.ping_request = None;
            self.ping_written(id, now);
        }
        if self.auto_ping_due(cfg, now) {
            let id = self.next_ping_id;
            if !put(&mut out, write_ping(w, id).map_err(Into::into))? {
                return Ok(out);
            }
            self.next_ping_id = id.wrapping_add(1);
            self.ping_written(id, now);
        }
        // The UMP Data stream, in Established Session only (REQ-NET-DATA-002, REQ-NET-RST-006).
        if self.state == SessionState::EstablishedSession {
            let s = self.stream.write_data(&cfg.stream, now, w)?;
            out.commands += s.commands;
            out.more |= s.more;
        }
        Ok(out)
    }

    fn ping_written(&mut self, id: u32, now: u64) {
        self.pings.push(id, now);
        self.missed = self.missed.saturating_add(1);
        self.last_ping_at = now;
    }
}

/// A read-only view of one session of a [`crate::Host`] or [`crate::Client`]: its state and the
/// configuration of the instance. The instance keeps one configuration for all its sessions
/// (OPT-009), so it gives this view, not a `&Session`.
#[derive(Debug, Clone, Copy)]
pub struct SessionRef<
    'a,
    const W: usize = 64,
    const C: usize = 256,
    const Q: usize = 4,
    const P: usize = 8,
    const U: usize = DEFAULT_USERNAME_CAPACITY,
    const R: usize = 4,
> {
    core: &'a SessionCore<W, C, Q, P, U, R>,
    config: &'a SessionConfig,
}

impl<
        'a,
        const W: usize,
        const C: usize,
        const Q: usize,
        const P: usize,
        const U: usize,
        const R: usize,
    > SessionRef<'a, W, C, Q, P, U, R>
{
    pub(crate) const fn new(
        core: &'a SessionCore<W, C, Q, P, U, R>,
        config: &'a SessionConfig,
    ) -> Self {
        SessionRef { core, config }
    }

    /// The role (see [`Session::role`]).
    pub const fn role(&self) -> Role {
        self.core.role()
    }

    /// The session state (see [`Session::state`]).
    pub const fn state(&self) -> SessionState {
        self.core.state()
    }

    /// The configuration of the instance.
    pub const fn config(&self) -> &'a SessionConfig {
        self.config
    }

    /// The peer (see [`Session::peer`]).
    pub const fn peer(&self) -> Option<&'a PeerInfo> {
        self.core.peer()
    }

    /// The Handshake (see [`Session::handshake`]).
    pub const fn handshake(&self) -> &'a Handshake<U> {
        self.core.handshake()
    }

    /// The UMP Data path (see [`Session::stream`]).
    pub const fn stream(&self) -> &'a DataStream<W, C, Q> {
        self.core.stream()
    }

    /// The open Pings (see [`Session::open_pings`]).
    pub const fn open_pings(&self) -> &'a OpenPings<P> {
        self.core.open_pings()
    }

    /// The missed automatic Pings (see [`Session::missed_pings`]).
    pub const fn missed_pings(&self) -> u32 {
        self.core.missed_pings()
    }

    /// The dropped one-shot replies (see [`Session::dropped_replies`]).
    pub const fn dropped_replies(&self) -> u32 {
        self.core.dropped_replies()
    }

    /// True when the peer refused the Command (see [`Session::is_refused`]).
    pub fn is_refused(&self, code: CommandCode) -> bool {
        self.core.is_refused(code)
    }

    /// The earliest time with work (see [`Session::next_deadline`]).
    pub fn next_deadline(&self) -> Option<u64> {
        self.core.next_deadline(self.config)
    }

    /// True when there are Commands to write at `now` (see [`Session::wants_write`]).
    pub fn wants_write(&self, now: u64) -> bool {
        self.core.wants_write(self.config, now)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::command::{Invitation, RetransmitErrorReason, RetransmitRequest, SeqNum, UmpData};
    use crate::handshake::{FailureDelay, HostPolicy, PolicyKind, NONCE_RANDOM_LEN};
    use crate::packet::{command_packets, MaxPayload, PacketItem};
    use proptest::prelude::*;
    use std::string::String;
    use std::vec::Vec;

    #[test]
    fn default_open_pings_is_8() {
        // REQ-NET-PING-007: P = 8 by default, in the table and in the Session.
        let same = |a: core::any::TypeId, b: core::any::TypeId| a == b;
        assert!(same(
            core::any::TypeId::of::<OpenPings>(),
            core::any::TypeId::of::<OpenPings<8>>()
        ));
        assert!(same(
            core::any::TypeId::of::<Session>(),
            core::any::TypeId::of::<Session<64, 256, 4, 8>>()
        ));
    }

    /// OPT-047: the ring gives the same entries, in the same order, as a `Vec` with the old
    /// operations (push at the end, removal at the front, removal in the middle).
    #[test]
    fn prop_ring_matches_a_vec_model() {
        proptest!(|(ops in prop::collection::vec((0u8..6, any::<u16>()), 0..200))| {
            let mut ring: Ring<u16, 5> = Ring::new(0);
            let mut model: Vec<u16> = Vec::new();
            for (op, v) in ops {
                let i = usize::from(v) % 7;
                match op {
                    0 | 1 => {
                        let r = ring.push_back(v);
                        if model.len() < 5 {
                            prop_assert_eq!(r, Ok(()));
                            model.push(v);
                        } else {
                            prop_assert_eq!(r, Err(v));
                        }
                    }
                    2 => {
                        prop_assert_eq!(ring.pop_front(), (!model.is_empty()).then(|| model.remove(0)));
                    }
                    3 => {
                        ring.remove(i);
                        if i < model.len() {
                            model.remove(i);
                        }
                    }
                    4 => {
                        ring.drop_front(i);
                        model.drain(..i.min(model.len()));
                    }
                    _ => {
                        prop_assert_eq!(
                            ring.rposition(|x| x % 3 == 0),
                            model.iter().rposition(|x| x % 3 == 0)
                        );
                    }
                }
                prop_assert_eq!(ring.len(), model.len());
                prop_assert_eq!(ring.iter().copied().collect::<Vec<_>>(), model.clone());
                prop_assert_eq!(ring.get(i), model.get(i).copied());
            }
        });
    }
    use std::{format, vec};
    use SessionState::*;

    type S = Session;

    const NOTE: u32 = 0x2090_3C40; // MIDI 1.0 Note On, 1 word (MT 0x2)
    const NOTE2: u32 = 0x2080_3C00;
    const NOTE3: u32 = 0x2090_4040;

    /// Test credentials.
    #[derive(Default)]
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

    /// An owned copy of a SessionEvent, for the asserts.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Ev {
        Invitation,
        Pending,
        AuthRequested(Method),
        AuthFailed(u32),
        Credentials,
        Established(Option<Method>),
        Ending(u8),
        ClosedBye(u8, String),
        ClosedByeReply,
        ClosedByeTimeout,
        ClosedExpired,
        ClosedEnded(u8),
        Reset,
        ResetStarted,
        Nak(u8, u8, String),
        PingRtt(u32, u64),
        BadPingReply(u32),
        Timeout,
        Stream(StreamEvent),
    }

    fn text(b: &[u8]) -> String {
        String::from_utf8_lossy(b).into_owned()
    }

    fn own(e: &SessionEvent<'_>) -> Ev {
        match *e {
            SessionEvent::InvitationReceived { .. } => Ev::Invitation,
            SessionEvent::Pending => Ev::Pending,
            SessionEvent::AuthRequested(c) => Ev::AuthRequested(c.method),
            SessionEvent::AuthFailed { failures, .. } => Ev::AuthFailed(failures),
            SessionEvent::CredentialsError(_) => Ev::Credentials,
            SessionEvent::Established { method } => Ev::Established(method),
            SessionEvent::Ending(r) => Ev::Ending(r.0),
            SessionEvent::Closed(CloseCause::ByeReceived(b)) => {
                Ev::ClosedBye(b.reason.0, text(b.text.as_bytes()))
            }
            SessionEvent::Closed(CloseCause::ByeReply) => Ev::ClosedByeReply,
            SessionEvent::Closed(CloseCause::ByeTimeout) => Ev::ClosedByeTimeout,
            SessionEvent::Closed(CloseCause::Expired) => Ev::ClosedExpired,
            SessionEvent::Closed(CloseCause::Ended(r)) => Ev::ClosedEnded(r.0),
            SessionEvent::Reset => Ev::Reset,
            SessionEvent::ResetStarted => Ev::ResetStarted,
            SessionEvent::Nak(n) => Ev::Nak(n.reason.0, n.nak_header.code, text(n.text.as_bytes())),
            SessionEvent::PingReply { id, rtt_ms } => Ev::PingRtt(id, rtt_ms),
            SessionEvent::UnexpectedPingReply(id) => Ev::BadPingReply(id),
            SessionEvent::Timeout => Ev::Timeout,
            SessionEvent::Stream(s) => Ev::Stream(s),
        }
    }

    /// A configuration with no automatic Ping, no FEC and one idle Command.
    fn quiet() -> SessionConfig {
        SessionConfig {
            ping_interval_ms: 0,
            stream: StreamConfig {
                fec_repeats: 0,
                idle_further: 0,
                ..StreamConfig::DEFAULT
            },
            ..SessionConfig::DEFAULT
        }
    }

    /// One endpoint with one peer: a small model of the Endpoint module.
    struct Peer {
        role: Role,
        s: Option<S>,
        id: Identity,
        cfg: SessionConfig,
        creds: Creds,
        delay: FailureDelay,
        policy: HostPolicy,
        stateless: Vec<Stateless>,
        ev: Vec<Ev>,
        // Sequence Number, late mark, UMP words.
        data: Vec<(u16, bool, Vec<u32>)>,
        written: Vec<Vec<u8>>,
    }

    impl Peer {
        fn host(cfg: SessionConfig) -> Self {
            Peer {
                role: Role::Host,
                s: None,
                id: Identity::new("Host", "H1").unwrap(),
                cfg,
                creds: Creds::default(),
                delay: FailureDelay::new(0, 0),
                policy: HostPolicy::DEFAULT,
                stateless: Vec::new(),
                ev: Vec::new(),
                data: Vec::new(),
                written: Vec::new(),
            }
        }

        fn client(cfg: SessionConfig) -> Self {
            Peer {
                role: Role::Client,
                s: Some(S::client(cfg, Capabilities::default()).unwrap()),
                id: Identity::new("MyDev", "8shYe3h5").unwrap(),
                ..Peer::host(cfg)
            }
        }

        fn s(&mut self) -> &mut S {
            self.s.as_mut().expect("a session")
        }

        fn state(&self) -> SessionState {
            self.s.as_ref().map_or(Idle, |s| s.state())
        }

        fn take_ev(&mut self) -> Vec<Ev> {
            std::mem::take(&mut self.ev)
        }

        fn receive(&mut self, now: u64, p: &[u8]) {
            let Ok(items) = command_packets(p) else {
                return;
            };
            let mut first = true;
            for item in items {
                let PacketItem::Command(raw) = item else {
                    continue;
                };
                let Ok(cmd) = Command::parse(raw) else {
                    continue;
                };
                self.receive_cmd(now, raw.header, &cmd, &mut first);
            }
        }

        fn receive_cmd(&mut self, now: u64, header: Header, cmd: &Command<'_>, first: &mut bool) {
            let mut env = Env {
                creds: &mut self.creds,
                delay: Some(&mut self.delay),
                policy: self.policy,
            };
            match self.s.as_mut() {
                Some(s) => {
                    let r = s.receive(now, header, cmd, &mut env);
                    self.ev.extend(r.events.iter().map(own));
                    if let Some(d) = r.data {
                        if d.delivers() {
                            let words = d.messages().flat_map(|m| m.words().to_vec()).collect();
                            self.data.push((d.seq.0, d.is_late(), words));
                        }
                    }
                }
                None => {
                    match S::idle_reply(now, self.role, header, cmd, *first, &self.cfg, &mut env) {
                        IdleReply::Ignore => {}
                        IdleReply::Send(st, ev) => {
                            if st == Stateless::Bye(ByeReason::SESSION_NOT_ESTABLISHED) {
                                *first = false;
                            }
                            self.ev.extend(ev.iter().map(own));
                            self.stateless.push(st);
                        }
                        IdleReply::Event(e) => self.ev.push(own(&e)),
                        IdleReply::Open(o) => {
                            let mut s = S::host(self.cfg).unwrap();
                            let ev = s.open(now, o).unwrap();
                            self.ev.extend(ev.iter().map(own));
                            self.s = Some(s);
                        }
                    }
                }
            }
        }

        /// poll and write: the UDP payloads of this peer at `now`.
        fn tick(&mut self, now: u64) -> Vec<Vec<u8>> {
            let mut out = Vec::new();
            for st in std::mem::take(&mut self.stateless) {
                let mut buf = [0u8; 1400];
                let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
                st.write(self.role, &mut w).unwrap();
                let n = w.len();
                out.push(buf[..n].to_vec());
            }
            if let Some(s) = self.s.as_mut() {
                let ev = s.poll(now);
                self.ev.extend(ev.iter().map(own));
                for _ in 0..100 {
                    let mut buf = [0u8; 1400];
                    let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
                    let r = s.write_payload(now, &self.id, &mut w).unwrap();
                    let n = w.len();
                    if n > 0 {
                        out.push(buf[..n].to_vec());
                    } else {
                        assert!(!r.more);
                    }
                    if !r.more {
                        break;
                    }
                }
                if self.role == Role::Host && s.is_finished() {
                    self.s = None;
                }
            }
            self.written.extend(out.iter().cloned());
            out
        }
    }

    /// `a` writes at `now`; `keep` decides for each payload if it reaches `b`.
    fn send_if(
        now: u64,
        a: &mut Peer,
        b: &mut Peer,
        mut keep: impl FnMut(&[u8]) -> bool,
    ) -> Vec<Vec<u8>> {
        let ps = a.tick(now);
        for p in &ps {
            if keep(p) {
                b.receive(now, p);
            }
        }
        ps
    }

    fn send(now: u64, a: &mut Peer, b: &mut Peer) -> Vec<Vec<u8>> {
        send_if(now, a, b, |_| true)
    }

    fn round(now: u64, a: &mut Peer, b: &mut Peer) {
        send(now, a, b);
        send(now, b, a);
    }

    fn run(a: &mut Peer, b: &mut Peer, from: u64, to: u64, step: u64) {
        let mut t = from;
        while t <= to {
            round(t, a, b);
            t += step;
        }
    }

    fn codes(p: &[u8]) -> Vec<u8> {
        command_packets(p)
            .unwrap()
            .filter_map(|i| match i {
                PacketItem::Command(r) => Some(r.header.code),
                PacketItem::Truncated(_) => None,
            })
            .collect()
    }

    fn all_codes(ps: &[Vec<u8>]) -> Vec<u8> {
        ps.iter().flat_map(|p| codes(p)).collect()
    }

    fn headers(p: &[u8]) -> Vec<Header> {
        command_packets(p)
            .unwrap()
            .filter_map(|i| match i {
                PacketItem::Command(r) => Some(r.header),
                PacketItem::Truncated(_) => None,
            })
            .collect()
    }

    /// A UDP payload written by `f`.
    fn payload(f: impl FnOnce(&mut PayloadWriter<'_>)) -> Vec<u8> {
        let mut buf = [0u8; 1400];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        f(&mut w);
        let n = w.len();
        buf[..n].to_vec()
    }

    fn ping_pkt(id: u32) -> Vec<u8> {
        payload(|w| write_ping(w, id).unwrap())
    }

    fn data_pkt(seq: u16, words: &[u32]) -> Vec<u8> {
        payload(|w| UmpData::write(w, SeqNum(seq), words).unwrap())
    }

    fn empty_pkt(code: CommandCode) -> Vec<u8> {
        payload(|w| write_empty(w, code).unwrap())
    }

    fn nak_pkt(reason: NakReason, code: u8, text: &str) -> Vec<u8> {
        payload(|w| Nak::write(w, reason, Header::from_bytes([code, 0, 0, 0]), text).unwrap())
    }

    /// A Client and a Host in Established Session at time 0, events cleared.
    fn established(cfg: SessionConfig) -> (Peer, Peer) {
        let mut c = Peer::client(cfg);
        let mut h = Peer::host(cfg);
        c.s().connect(0).unwrap();
        round(0, &mut c, &mut h);
        assert_eq!(c.state(), EstablishedSession);
        assert_eq!(h.state(), EstablishedSession);
        c.take_ev();
        h.take_ev();
        (c, h)
    }

    // ----- configuration -----

    #[test]
    fn config_defaults_and_repeat_interval_range() {
        // REQ-NET-SES-006, -007, REQ-NET-RST-005, REQ-NET-BYE-007, REQ-NET-PING-004, -008,
        // -009, REQ-NET-RTX-004 (N-21, N-23, N-24, N-26, N-28, N-38).
        let c = SessionConfig::default();
        assert_eq!(c.repeat_interval_ms, 500);
        assert_eq!(c.reset_timeout_ms, 3000);
        assert_eq!(c.bye_timeout_ms, 3000);
        assert_eq!(c.ping_interval_ms, 2000);
        assert_eq!(c.ping_miss_limit, 5);
        assert!(!c.nak_bad_ping_reply);
        assert!(c.stream.retransmit);
        assert_eq!(c.check(), Ok(()));
        for (ms, ok) in [
            (299, false),
            (300, true),
            (2000, true),
            (2001, false),
            (0, false),
        ] {
            let c = SessionConfig {
                repeat_interval_ms: ms,
                ..SessionConfig::DEFAULT
            };
            assert_eq!(c.check().is_ok(), ok, "{ms}");
            assert_eq!(S::host(c).is_ok(), ok);
            assert_eq!(S::client(c, Capabilities::default()).is_ok(), ok);
        }
        let bad_stream = SessionConfig {
            stream: StreamConfig {
                idle_interval_ms: 301,
                ..StreamConfig::DEFAULT
            },
            ..SessionConfig::DEFAULT
        };
        assert_eq!(
            S::host(bad_stream).err(),
            Some(SessionError::Stream(StreamError::IdleInterval))
        );
    }

    // ----- session states -----

    #[test]
    fn host_and_client_establish_through_payloads() {
        // REQ-NET-SES-001, REQ-NET-SES-003 (Invitation in Idle, Accepted in Pending
        // Invitation).
        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        assert_eq!(c.state(), Idle);
        assert!(c.s().is_finished());
        c.s().connect(0).unwrap();
        assert_eq!(c.state(), PendingInvitation);
        assert_eq!(
            c.s().connect(0),
            Err(SessionError::WrongState(PendingInvitation))
        );
        assert_eq!(h.state(), Idle);
        let ps = send(0, &mut c, &mut h);
        assert_eq!(all_codes(&ps), [0x01]);
        assert_eq!(h.state(), EstablishedSession);
        assert_eq!(h.take_ev(), [Ev::Established(None)]);
        assert_eq!(h.s().peer().unwrap().name(), b"MyDev");
        let ps = send(0, &mut h, &mut c);
        assert_eq!(all_codes(&ps), [0x10]);
        assert_eq!(c.state(), EstablishedSession);
        assert_eq!(c.take_ev(), [Ev::Established(None)]);
        assert_eq!(c.s().peer().unwrap().name(), b"Host");
        assert_eq!(c.s().role(), Role::Client);
        assert_eq!(h.s().role(), Role::Host);
        // Nothing more to write.
        assert!(c.tick(1).is_empty());
        assert!(h.tick(1).is_empty());
        assert_eq!(
            S::host(quiet()).unwrap().connect(0),
            Err(SessionError::WrongRole)
        );
    }

    #[test]
    fn invitation_repeats_with_same_bytes_until_reply() {
        // REQ-NET-SES-005, REQ-NET-SES-006: one repeat interval between two sends, same
        // bytes, repeats stop at the reply.
        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        c.s().connect(0).unwrap();
        assert_eq!(c.s().next_deadline(), Some(0));
        let first = c.tick(0);
        assert_eq!(all_codes(&first), [0x01]);
        assert_eq!(c.s().next_deadline(), Some(500));
        assert!(!c.s().wants_write(499));
        assert!(c.tick(499).is_empty());
        assert!(c.s().wants_write(500));
        let second = c.tick(500);
        assert_eq!(second, first);
        // The second Invitation reaches the Host; the Accepted reply stops the repeats.
        h.receive(500, &second[0]);
        send(600, &mut h, &mut c);
        assert_eq!(c.state(), EstablishedSession);
        assert!(c.tick(1000).is_empty());
        assert!(c.tick(5000).is_empty());
    }

    #[test]
    fn invitation_survives_lost_payloads() {
        // REQ-NET-SES-005 with losses: every other payload in each direction is lost.
        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        c.s().connect(0).unwrap();
        // The first two Invitations and the first Accepted are lost.
        let (mut nc, mut nh) = (0, 0);
        let mut t = 0;
        while t <= 3000 && c.state() != EstablishedSession {
            send_if(t, &mut c, &mut h, |_| {
                nc += 1;
                nc > 2
            });
            send_if(t, &mut h, &mut c, |_| {
                nh += 1;
                nh > 1
            });
            t += 100;
        }
        assert_eq!(c.state(), EstablishedSession);
        assert_eq!(t, 1600, "established by the fourth Invitation at 1500 ms");
        assert_eq!(h.state(), EstablishedSession);
        // One Accepted for each received Invitation (REQ-NET-DATA-014).
        let invitations = all_codes(&c.written).iter().filter(|c| **c == 0x01).count();
        let accepted = all_codes(&h.written).iter().filter(|c| **c == 0x10).count();
        // The first two Invitations are lost: each of the others gets one Accepted.
        assert_eq!(accepted, invitations - 2);
    }

    #[test]
    fn invitation_timeout_sends_bye_80_then_bye_timeout_to_idle() {
        // REQ-NET-SES-007, REQ-NET-BYE-007, REQ-NET-BYE-014: Invitation timeout 10000 ms,
        // Bye 0x80, Bye repeats, Bye timeout 3000 ms, Idle with no further Command.
        let mut c = Peer::client(quiet());
        c.s().connect(0).unwrap();
        let mut ps = Vec::new();
        let mut t = 0;
        while t < 10_000 {
            ps.extend(c.tick(t));
            t += 100;
        }
        assert_eq!(all_codes(&ps), [0x01; 20]);
        let p = c.tick(10_000);
        assert_eq!(all_codes(&p), [0xF0]);
        assert_eq!(headers(&p[0])[0].csd1, 0x80);
        assert_eq!(c.state(), PendingBye);
        assert_eq!(c.take_ev(), [Ev::Ending(0x80)]);
        let mut byes = p.clone();
        let mut t = 10_100;
        while t < 13_000 {
            byes.extend(c.tick(t));
            t += 100;
        }
        assert_eq!(all_codes(&byes), [0xF0; 6]);
        assert!(byes.iter().all(|b| *b == p[0]));
        assert!(c.tick(13_000).is_empty());
        assert_eq!(c.state(), Idle);
        assert_eq!(c.take_ev(), [Ev::ClosedByeTimeout]);
        assert!(c.s().is_finished());
        assert_eq!(c.s().next_deadline(), None);
        assert!(c.tick(20_000).is_empty());
    }

    // ----- Bye -----

    #[test]
    fn bye_from_caller_with_text_and_bye_reply() {
        // REQ-NET-BYE-006, REQ-NET-BYE-008, REQ-NET-BYE-011.
        let (mut c, mut h) = established(quiet());
        assert_eq!(
            c.s().bye(10, ByeReason::TOO_MANY_SESSIONS, ""),
            Err(SessionError::Command(CommandError::ReasonNotAllowed))
        );
        let long = "x".repeat(BYE_TEXT_CAPACITY + 1);
        assert_eq!(
            c.s().bye(10, ByeReason::USER_TERMINATED, &long),
            Err(SessionError::Command(CommandError::TextTooLong))
        );
        let ev = c
            .s()
            .bye(10, ByeReason::USER_TERMINATED, "bye now")
            .unwrap();
        assert_eq!(ev.iter().map(own).collect::<Vec<_>>(), [Ev::Ending(1)]);
        assert_eq!(c.state(), PendingBye);
        assert_eq!(
            c.s().bye(10, ByeReason::USER_TERMINATED, ""),
            Err(SessionError::WrongState(PendingBye))
        );
        let ps = send(10, &mut c, &mut h);
        assert_eq!(all_codes(&ps), [0xF0]);
        assert_eq!(h.state(), Idle);
        assert_eq!(h.take_ev(), [Ev::ClosedBye(1, "bye now".into())]);
        let ps = send(10, &mut h, &mut c);
        assert_eq!(all_codes(&ps), [0xF1]);
        assert!(h.s.is_none(), "the Host entry is free");
        assert_eq!(c.state(), Idle);
        assert_eq!(c.take_ev(), [Ev::ClosedByeReply]);
        assert!(c.tick(600).is_empty());
        assert_eq!(
            c.s().bye(700, ByeReason::USER_TERMINATED, ""),
            Err(SessionError::WrongState(Idle))
        );
        // In Pending Session Reset: one Bye with the text, and no more Session Reset.
        let (mut c, _) = established(quiet());
        c.s().reset(0).unwrap();
        c.tick(0);
        let ev = c.s().bye(1, ByeReason::USER_TERMINATED, "x").unwrap();
        assert_eq!(ev.iter().map(own).collect::<Vec<_>>(), [Ev::Ending(1)]);
        assert_eq!(c.state(), PendingBye);
        let ps = c.tick(1);
        assert_eq!(all_codes(&ps), [0xF0]);
        assert_eq!(&ps[0][8..12], b"x\0\0\0");
        assert!(all_codes(&c.tick(500)).iter().all(|c| *c == 0xF0));
    }

    #[test]
    fn repeated_bye_gets_one_bye_reply_each() {
        // REQ-NET-BYE-010, REQ-NET-BYE-007, REQ-NET-BYE-009: the first Bye Reply is lost; the
        // repeat gets a Bye Reply from the Host, which has no entry any more.
        let (mut c, mut h) = established(quiet());
        c.s().bye(0, ByeReason::POWER_DOWN, "").unwrap();
        let b1 = send(0, &mut c, &mut h);
        let lost = h.tick(0);
        assert_eq!(all_codes(&lost), [0xF1]);
        assert!(h.s.is_none());
        assert_eq!(c.state(), PendingBye);
        assert!(c.tick(499).is_empty());
        let b2 = send(500, &mut c, &mut h);
        assert_eq!(b2, b1);
        let r2 = send(500, &mut h, &mut c);
        assert_eq!(all_codes(&r2), [0xF1]);
        assert_eq!(c.state(), Idle);
        assert_eq!(c.take_ev(), [Ev::ClosedByeReply]);
        // Two Byes, two Bye Replies, never a repeat by the Host.
        let replies = all_codes(&h.written).iter().filter(|c| **c == 0xF1).count();
        assert_eq!(replies, 2);
        assert!(h.tick(5000).is_empty());
        // Two Byes in one payload get two Bye Replies.
        let (mut c, _) = established(quiet());
        let mut two = payload(|w| Bye::write(w, Role::Host, ByeReason::UNKNOWN, "").unwrap());
        two.extend_from_slice(&two.clone()[4..]);
        c.receive(1, &two);
        assert_eq!(all_codes(&c.tick(1)), [0xF1, 0xF1]);
        assert!(c.tick(1000).is_empty());
    }

    #[test]
    fn received_bye_ends_each_active_state() {
        // REQ-NET-BYE-008: Pending Invitation, Established Session, Pending Session Reset and
        // Pending Bye.
        let bye = payload(|w| Bye::write(w, Role::Host, ByeReason::TIMEOUT, "t").unwrap());
        let mut c = Peer::client(quiet());
        c.s().connect(0).unwrap();
        c.receive(1, &bye);
        assert_eq!(c.state(), Idle);
        assert_eq!(c.take_ev(), [Ev::ClosedBye(4, "t".into())]);
        assert_eq!(all_codes(&c.tick(1)), [0xF1]);
        assert!(c.tick(600).is_empty(), "no Invitation repeat");

        let (mut c, _) = established(quiet());
        c.s().reset(0).unwrap();
        c.receive(1, &bye);
        assert_eq!(c.state(), Idle);
        assert_eq!(all_codes(&c.tick(1)), [0xF1]);
        assert!(c.tick(600).is_empty(), "no Session Reset repeat");

        let (mut c, mut h) = established(quiet());
        c.s().bye(0, ByeReason::USER_TERMINATED, "").unwrap();
        h.s().bye(0, ByeReason::USER_TERMINATED, "").unwrap();
        round(0, &mut c, &mut h);
        round(0, &mut c, &mut h);
        assert_eq!(c.state(), Idle);
        assert_eq!(h.state(), Idle);
        // The Bye of the Client arrives first: the Host answers it and drops its own Bye.
        assert_eq!(c.take_ev(), [Ev::ClosedByeReply]);
        assert_eq!(h.take_ev(), [Ev::ClosedBye(1, "".into())]);
        assert!(c.tick(5000).is_empty());
    }

    #[test]
    fn bye_reply_outside_pending_bye_is_ignored() {
        // REQ-NET-BYE-011.
        let (mut c, _) = established(quiet());
        c.receive(5, &empty_pkt(CommandCode::ByeReply));
        assert_eq!(c.state(), EstablishedSession);
        assert!(c.take_ev().is_empty());
        assert!(c.tick(5).is_empty());
        let mut c = Peer::client(quiet());
        c.s().connect(0).unwrap();
        c.tick(0);
        c.receive(5, &empty_pkt(CommandCode::ByeReply));
        assert_eq!(c.state(), PendingInvitation);
        assert!(c.tick(5).is_empty());
    }

    #[test]
    fn pending_bye_writes_only_bye_ping_reply_bye_reply_and_nak() {
        // REQ-NET-BYE-013, REQ-NET-DATA-015 (Pending Bye: discard, no second Bye).
        let cfg = SessionConfig {
            ping_interval_ms: 100,
            ..quiet()
        };
        let (mut c, _) = established(cfg);
        c.s().send_ump(&[NOTE]).unwrap();
        c.s().ping(0, 9).unwrap();
        c.s().bye(0, ByeReason::USER_TERMINATED, "").unwrap();
        assert_eq!(
            c.s().send_ump(&[NOTE]),
            Err(SessionError::WrongState(PendingBye))
        );
        assert_eq!(c.s().ping(0, 1), Err(SessionError::WrongState(PendingBye)));
        assert_eq!(c.s().reset(0), Err(SessionError::WrongState(PendingBye)));
        let mut p = ping_pkt(3);
        for extra in [
            data_pkt(0, &[NOTE]),
            empty_pkt(CommandCode::SessionReset),
            empty_pkt(CommandCode::SessionResetReply),
            payload(|w| {
                RetransmitRequest {
                    seq: SeqNum(0),
                    count: 1,
                }
                .write(w)
                .unwrap()
            }),
        ] {
            p.extend_from_slice(&extra[4..]);
        }
        c.receive(0, &p);
        let out = c.tick(0);
        assert_eq!(all_codes(&out), [0x21, 0xF0]);
        assert_eq!(c.state(), PendingBye);
        // Later: only the Bye repeats.
        let mut t = 100;
        while t < 3000 {
            for p in c.tick(t) {
                assert_eq!(codes(&p), [0xF0]);
            }
            t += 100;
        }
    }

    // ----- Session Reset -----

    #[test]
    fn session_reset_round_trip_resets_both_streams() {
        // REQ-NET-RST-003, -007, -008, -009, -010.
        let (mut c, mut h) = established(quiet());
        c.s().send_ump(&[NOTE]).unwrap();
        h.s().send_ump(&[NOTE2]).unwrap();
        run(&mut c, &mut h, 0, 400, 100);
        assert_eq!(h.data[0], (0, false, vec![NOTE]));
        assert_eq!(c.data[0], (0, false, vec![NOTE2]));
        assert!(c.s().stream().sender().next_seq().0 >= 1);
        // No reset before Established.
        let mut x = Peer::client(quiet());
        x.s().connect(0).unwrap();
        assert_eq!(
            x.s().reset(0),
            Err(SessionError::WrongState(PendingInvitation))
        );
        assert!(all_codes(&x.tick(0)).iter().all(|c| *c != 0x82));

        c.s().reset(1000).unwrap();
        assert_eq!(c.state(), PendingSessionReset);
        let ps = send(1000, &mut c, &mut h);
        assert_eq!(all_codes(&ps), [0x82]);
        assert_eq!(h.state(), EstablishedSession);
        assert_eq!(h.take_ev(), [Ev::Reset]);
        let ps = send(1000, &mut h, &mut c);
        assert_eq!(all_codes(&ps), [0x83]);
        assert_eq!(c.state(), EstablishedSession);
        assert_eq!(c.take_ev(), [Ev::Reset]);
        // Sequence Number 0 again in both directions, and taken as new.
        c.s().send_ump(&[NOTE3]).unwrap();
        h.s().send_ump(&[NOTE3]).unwrap();
        let pc = send(1100, &mut c, &mut h);
        let ph = send(1100, &mut h, &mut c);
        assert_eq!(headers(&pc[0])[0].csd(), 0);
        assert_eq!(headers(&ph[0])[0].csd(), 0);
        assert_eq!(h.data.last().unwrap(), &(0, false, vec![NOTE3]));
        assert_eq!(c.data.last().unwrap(), &(0, false, vec![NOTE3]));
        // No further Session Reset.
        assert!(all_codes(&c.tick(2000)).iter().all(|c| *c != 0x82));
        // REQ-NET-RST-008: two Session Resets in one payload get two replies, and no third.
        let (mut c, _) = established(quiet());
        let mut two = empty_pkt(CommandCode::SessionReset);
        two.extend_from_slice(&empty_pkt(CommandCode::SessionReset)[4..]);
        c.receive(10, &two);
        assert_eq!(all_codes(&c.tick(10)), [0x83, 0x83]);
        assert!(c.tick(1000).is_empty());
        // A repeat in a later payload gets one more.
        c.receive(1100, &empty_pkt(CommandCode::SessionReset));
        assert_eq!(all_codes(&c.tick(1100)), [0x83]);
    }

    #[test]
    fn session_reset_repeats_then_timeout_sends_bye_04() {
        // REQ-NET-RST-004, REQ-NET-RST-005, REQ-NET-SES-005, REQ-NET-SES-007,
        // REQ-NET-BYE-014.
        let (mut c, _) = established(quiet());
        c.s().reset(1000).unwrap();
        let mut ps = Vec::new();
        let mut t = 1000;
        while t < 4000 {
            ps.extend(c.tick(t));
            t += 100;
        }
        assert_eq!(all_codes(&ps), [0x82; 6]);
        assert!(ps.iter().all(|p| *p == ps[0]));
        assert_eq!(c.s().next_deadline(), Some(4000));
        let p = c.tick(4000);
        assert_eq!(
            all_codes(&p),
            [0xF0],
            "no last Session Reset at the timeout"
        );
        assert_eq!(headers(&p[0])[0].csd1, 0x04);
        assert_eq!(c.state(), PendingBye);
        assert_eq!(c.take_ev(), [Ev::Ending(4)]);
    }

    #[test]
    fn pending_reset_discards_data_and_retransmit_commands() {
        // REQ-NET-RST-006, REQ-NET-DATA-015, N-09.
        let (mut c, mut h) = established(quiet());
        c.s().send_ump(&[NOTE]).unwrap();
        c.s().reset(0).unwrap();
        assert_eq!(
            c.s().send_ump(&[NOTE]),
            Err(SessionError::WrongState(PendingSessionReset))
        );
        let mut p = data_pkt(5, &[NOTE2]);
        for extra in [
            payload(|w| {
                RetransmitRequest {
                    seq: SeqNum(0),
                    count: 1,
                }
                .write(w)
                .unwrap()
            }),
            payload(|w| {
                RetransmitError {
                    reason: RetransmitErrorReason::NOT_IN_BUFFER,
                    seq: SeqNum(0),
                }
                .write(w)
                .unwrap()
            }),
        ] {
            p.extend_from_slice(&extra[4..]);
        }
        c.receive(0, &p);
        assert!(c.data.is_empty());
        assert!(c.take_ev().is_empty());
        // Only the Session Reset: no UMP Data, no Bye 0x05, no Retransmit Request.
        let ps = send(0, &mut c, &mut h);
        assert_eq!(all_codes(&ps), [0x82]);
        send(0, &mut h, &mut c);
        assert_eq!(c.state(), EstablishedSession);
        // The data queued before the reset is gone; number 0 of the peer is new.
        assert!(c.tick(500).is_empty());
        c.receive(600, &data_pkt(0, &[NOTE3]));
        assert_eq!(c.data, [(0, false, vec![NOTE3])]);
    }

    #[test]
    fn session_reset_in_both_directions_at_once() {
        // REQ-NET-RST-008, N-11: reply, reset, stay in Pending Session Reset until the Reply.
        let (mut c, mut h) = established(quiet());
        c.s().reset(0).unwrap();
        h.s().reset(0).unwrap();
        let pc = c.tick(0);
        let ph = h.tick(0);
        for p in &pc {
            h.receive(0, p);
        }
        for p in &ph {
            c.receive(0, p);
        }
        assert_eq!(c.state(), PendingSessionReset);
        assert_eq!(h.state(), PendingSessionReset);
        assert_eq!(c.take_ev(), [Ev::Reset]);
        round(0, &mut c, &mut h);
        assert_eq!(c.state(), EstablishedSession);
        assert_eq!(h.state(), EstablishedSession);
        assert_eq!(c.take_ev(), [Ev::Reset]);
        assert_eq!(h.take_ev(), [Ev::Reset, Ev::Reset]);
        assert!(c.tick(100).is_empty());
        assert!(h.tick(100).is_empty());
    }

    #[test]
    fn session_reset_without_active_session_gets_bye_05() {
        // REQ-NET-RST-011: Bye 0x05 and no state change; ignored in Pending Bye.
        for code in [CommandCode::SessionReset, CommandCode::SessionResetReply] {
            let mut c = Peer::client(quiet());
            c.s().connect(0).unwrap();
            c.tick(0);
            c.receive(1, &empty_pkt(code));
            assert_eq!(c.state(), PendingInvitation);
            let p = c.tick(1);
            assert_eq!(all_codes(&p), [0xF0]);
            assert_eq!(headers(&p[0])[0].csd1, 0x05);
            assert_eq!(c.state(), PendingInvitation);
            assert!(c.tick(300).is_empty(), "the Bye is not repeated");

            let (mut c, _) = established(quiet());
            c.s().bye(0, ByeReason::USER_TERMINATED, "").unwrap();
            c.tick(0);
            c.receive(1, &empty_pkt(code));
            assert!(c.tick(1).is_empty());
            assert_eq!(c.state(), PendingBye);

            // No entry: one Bye 0x05, no entry kept.
            let mut h = Peer::host(quiet());
            h.receive(1, &empty_pkt(code));
            assert!(h.s.is_none());
            let p = h.tick(1);
            assert_eq!(all_codes(&p), [0xF0]);
            assert_eq!(headers(&p[0])[0].csd1, 0x05);
        }
    }

    #[test]
    fn late_session_reset_reply_is_ignored_then_starts_a_reset() {
        // REQ-NET-RST-012, N-10.
        let (mut c, mut h) = established(quiet());
        c.s().reset(1000).unwrap();
        round(1000, &mut c, &mut h);
        assert_eq!(c.state(), EstablishedSession);
        c.take_ev();
        // A reply to a repeat, inside the Session Reset timeout: ignored.
        c.receive(3999, &empty_pkt(CommandCode::SessionResetReply));
        assert_eq!(c.state(), EstablishedSession);
        assert!(c.tick(3999).is_empty());
        assert!(c.take_ev().is_empty());
        // After the timeout: this side starts a Session Reset.
        c.receive(4000, &empty_pkt(CommandCode::SessionResetReply));
        assert_eq!(c.state(), PendingSessionReset);
        assert_eq!(all_codes(&c.tick(4000)), [0x82]);
        // With no own reset at all: the same.
        let (_, mut h2) = established(quiet());
        h2.receive(10, &empty_pkt(CommandCode::SessionResetReply));
        assert_eq!(h2.state(), PendingSessionReset);
    }

    // ----- UMP Data -----

    #[test]
    fn ump_data_only_in_established_session() {
        // REQ-NET-DATA-002.
        let mut c = Peer::client(quiet());
        assert_eq!(c.s().send_ump(&[NOTE]), Err(SessionError::WrongState(Idle)));
        c.s().connect(0).unwrap();
        assert_eq!(
            c.s().send_ump(&[NOTE]),
            Err(SessionError::WrongState(PendingInvitation))
        );
        assert!(all_codes(&c.tick(0)).iter().all(|c| *c != 0xFF));
        let (mut c, mut h) = established(quiet());
        c.s().send_ump(&[NOTE, NOTE2]).unwrap();
        let ps = send(10, &mut c, &mut h);
        assert_eq!(all_codes(&ps), [0xFF]);
        assert_eq!(h.data, [(0, false, vec![NOTE, NOTE2])]);
        // Invalid UMP words: error from the stream.
        assert_eq!(
            c.s().send_ump(&[0x4000_0000]),
            Err(SessionError::Stream(StreamError::InvalidUmpData))
        );
    }

    #[test]
    fn ump_data_before_established_gets_bye_05_and_pending_bye() {
        // REQ-NET-DATA-003: Host in Pending Invitation and Client in Pending Invitation.
        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        h.policy = HostPolicy::new(PolicyKind::AskCaller);
        c.s().connect(0).unwrap();
        round(0, &mut c, &mut h);
        assert_eq!(h.state(), PendingInvitation);
        assert_eq!(h.take_ev(), [Ev::Invitation]);
        assert_eq!(c.take_ev(), [Ev::Pending]);
        h.receive(10, &data_pkt(0, &[NOTE]));
        assert_eq!(h.state(), PendingBye);
        assert_eq!(h.take_ev(), [Ev::Ending(5)]);
        let p = h.tick(10);
        assert_eq!(all_codes(&p), [0xF0]);
        assert_eq!(headers(&p[0])[0].csd1, 0x05);

        let mut c = Peer::client(quiet());
        c.s().connect(0).unwrap();
        c.tick(0);
        let p = payload(|w| {
            RetransmitRequest {
                seq: SeqNum(0),
                count: 0,
            }
            .write(w)
            .unwrap()
        });
        c.receive(10, &p);
        assert_eq!(c.state(), PendingBye);
        assert_eq!(headers(&c.tick(10)[0])[0].csd1, 0x05);
        // A Retransmit Error too.
        let mut c = Peer::client(quiet());
        c.s().connect(0).unwrap();
        c.tick(0);
        let p = payload(|w| {
            RetransmitError {
                reason: RetransmitErrorReason::NOT_IN_BUFFER,
                seq: SeqNum(0),
            }
            .write(w)
            .unwrap()
        });
        c.receive(10, &p);
        assert_eq!(c.state(), PendingBye);
        let out = c.tick(10);
        assert_eq!(all_codes(&out), [0xF0]);
        assert_eq!(headers(&out[0])[0].csd1, 0x05);
    }

    #[test]
    fn ump_data_with_no_entry_gets_one_bye_per_payload() {
        // REQ-NET-DATA-015, N-16.
        let mut h = Peer::host(quiet());
        let mut p = data_pkt(0, &[NOTE]);
        p.extend_from_slice(&data_pkt(1, &[NOTE])[4..]);
        p.extend_from_slice(&data_pkt(2, &[NOTE])[4..]);
        h.receive(0, &p);
        assert!(h.s.is_none());
        let out = h.tick(0);
        assert_eq!(all_codes(&out), [0xF0]);
        assert_eq!(headers(&out[0])[0].csd1, 0x05);
        h.receive(1, &p);
        assert_eq!(
            all_codes(&h.tick(1)),
            [0xF0],
            "one more for the next payload"
        );
        assert!(h.tick(1000).is_empty(), "no repeat");
    }

    #[test]
    fn one_shot_replies_are_written_once() {
        // REQ-NET-DATA-014: NAK, Retransmit Error, Ping Reply, Session Reset Reply, Bye
        // Reply and the Invitation Replies are not repeated by a timer.
        let (mut c, _) = established(quiet());
        let mut p = ping_pkt(7);
        for extra in [
            empty_pkt(CommandCode::SessionReset),
            payload(|w| {
                RetransmitRequest {
                    seq: SeqNum(40),
                    count: 1,
                }
                .write(w)
                .unwrap()
            }),
            payload(|w| {
                Invitation::write(
                    w,
                    &crate::identity::EndpointName::new("X").unwrap(),
                    &crate::identity::ProductInstanceId::new("Y").unwrap(),
                    Capabilities::default(),
                )
                .unwrap()
            }),
        ] {
            p.extend_from_slice(&extra[4..]);
        }
        c.receive(100, &p);
        // The Client gets an Invitation (C→H): NAK 0x02 (REQ-NET-SES-009).
        let out = c.tick(100);
        assert_eq!(all_codes(&out), [0x21, 0x83, 0x81, 0x8F]);
        let mut t = 200;
        while t < 6000 {
            assert!(c.tick(t).is_empty(), "{t}");
            t += 100;
        }
        // Invitation Reply: Pending is written once, not repeated by a timer.
        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        h.policy = HostPolicy::new(PolicyKind::AskCaller);
        c.s().connect(0).unwrap();
        send(0, &mut c, &mut h);
        assert_eq!(all_codes(&h.tick(0)), [0x11]);
        let mut t = 100;
        while t < 60_000 {
            assert!(h.tick(t).is_empty(), "{t}");
            t += 500;
        }
        assert!(h.tick(59_999).is_empty());
    }

    /// A Client with Authentication and a Host with a shared secret; the Host is in
    /// Authentication Required and has not written its challenge yet.
    #[cfg(feature = "auth")]
    fn authentication_required() -> (Peer, Peer) {
        let mut c = Peer::client(quiet());
        c.s = Some(
            S::client(
                quiet(),
                Capabilities {
                    auth: true,
                    user_auth: false,
                },
            )
            .unwrap(),
        );
        c.creds.secret = Some(b"secret".to_vec());
        let mut h = Peer::host(quiet());
        h.policy = HostPolicy::new(PolicyKind::SharedSecret);
        h.creds.random = Some([7; 16]);
        h.creds.secret = Some(b"secret".to_vec());
        c.s().connect(0).unwrap();
        send(0, &mut c, &mut h);
        assert_eq!(h.state(), AuthenticationRequired);
        c.take_ev();
        h.take_ev();
        (c, h)
    }

    #[cfg(feature = "auth")]
    #[test]
    fn authentication_required_rules() {
        let bye80 =
            payload(|w| Bye::write(w, Role::Client, ByeReason::INVITATION_CANCELED, "").unwrap());
        // REQ-NET-DATA-003: UMP Data or a Retransmit Request: Bye 0x05 and Pending Bye.
        let (_, mut h) = authentication_required();
        assert_eq!(all_codes(&h.tick(0)), [0x12]);
        h.receive(10, &data_pkt(0, &[NOTE]));
        assert_eq!(h.state(), PendingBye);
        assert_eq!(h.take_ev(), [Ev::Ending(5)]);
        let p = h.tick(10);
        assert_eq!(all_codes(&p), [0xF0]);
        assert_eq!(headers(&p[0])[0].csd1, 0x05);
        let (_, mut h) = authentication_required();
        h.receive(
            10,
            &payload(|w| {
                RetransmitRequest {
                    seq: SeqNum(0),
                    count: 1,
                }
                .write(w)
                .unwrap()
            }),
        );
        assert_eq!(h.state(), PendingBye);
        assert_eq!(headers(&h.tick(10)[0])[0].csd1, 0x05);

        // REQ-NET-BYE-008: a received Bye: Bye Reply, Idle, the closed event, no repeat.
        let (_, mut h) = authentication_required();
        h.tick(0);
        h.receive(10, &bye80);
        assert_eq!(h.state(), Idle);
        assert_eq!(h.take_ev(), [Ev::ClosedBye(0x80, "".into())]);
        assert_eq!(all_codes(&h.tick(10)), [0xF1]);
        assert!(h.s.is_none());
        assert!(h.tick(600).is_empty());

        // REQ-NET-BYE-006: a Bye from the caller of the Host, with its text.
        let (_, mut h) = authentication_required();
        h.tick(0);
        let ev = h.s().bye(1, ByeReason::USER_DID_NOT_ACCEPT, "x").unwrap();
        assert_eq!(ev.iter().map(own).collect::<Vec<_>>(), [Ev::Ending(0x42)]);
        assert_eq!(h.state(), PendingBye);
        let ps = h.tick(1);
        assert_eq!(all_codes(&ps), [0xF0]);
        assert_eq!(headers(&ps[0])[0].csd1, 0x42);
        assert_eq!(&ps[0][8..12], b"x\0\0\0");

        // REQ-NET-SES-002, REQ-NET-DATA-014: the challenge is not repeated by a timer; at the
        // Host answer timeout the entry goes with no Command.
        let (_, mut h) = authentication_required();
        assert_eq!(all_codes(&h.tick(0)), [0x12]);
        assert_eq!(h.s().next_deadline(), Some(60_000));
        assert!(h.tick(59_999).is_empty());
        assert!(h.tick(60_000).is_empty());
        assert!(h.s.is_none());
        assert_eq!(h.take_ev(), [Ev::ClosedExpired]);

        // REQ-NET-AUTH-017: a wrong digest gets a delayed reply; a Bye before it is due
        // discards it.
        let (mut c, mut h) = authentication_required();
        c.creds.secret = Some(b"wrong".to_vec());
        h.delay = FailureDelay::default();
        send(0, &mut h, &mut c);
        send(0, &mut c, &mut h);
        assert_eq!(h.take_ev(), [Ev::AuthFailed(1)]);
        assert!(h.tick(0).is_empty(), "the reply is due later");
        h.receive(1, &bye80);
        assert_eq!(all_codes(&h.tick(1)), [0xF1]);
        assert!(h.s.is_none());
        assert!(h.tick(10_000).is_empty(), "no delayed reply");

        // REQ-NET-AUTH-033: the Client keeps the CryptoNonce until it goes to Idle; after a
        // Bye it writes no Invitation with Authentication.
        let digest = |c: &Peer| {
            let mut buf = [0u8; 128];
            let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
            Reply::InvitationWithAuth.write(&c.s.as_ref().unwrap().core.hs, &c.id, &mut w)
        };
        let (mut c, mut h) = authentication_required();
        send(0, &mut h, &mut c);
        assert_eq!(digest(&c), Ok(()), "the nonce is kept");
        let bye42 =
            payload(|w| Bye::write(w, Role::Host, ByeReason::USER_DID_NOT_ACCEPT, "").unwrap());
        c.receive(1, &bye42);
        assert_eq!(c.state(), Idle);
        assert_eq!(digest(&c), Err(HandshakeError::NothingToWrite));
        assert_eq!(all_codes(&c.tick(1)), [0xF1]);
        assert!(c.tick(500).is_empty());
        assert!(c.tick(1000).is_empty());
    }

    // ----- Table 9 and directions -----

    #[test]
    fn established_session_takes_data_and_retransmit_commands() {
        // REQ-NET-SES-003 (UMP Data, Retransmit Request, Retransmit Error in Established
        // Session; Ping, Ping Reply, NAK and Bye in every state).
        let (mut c, _) = established(quiet());
        c.receive(1, &data_pkt(0, &[NOTE]));
        assert_eq!(c.data, [(0, false, vec![NOTE])]);
        let e = RetransmitError {
            reason: RetransmitErrorReason::UNKNOWN,
            seq: SeqNum(3),
        };
        c.receive(2, &payload(|w| e.write(w).unwrap()));
        assert_eq!(c.take_ev(), [Ev::Stream(StreamEvent::RetransmitError(e))]);
        assert_eq!(c.state(), EstablishedSession);
        for (i, state_peer) in [Idle, PendingInvitation, EstablishedSession]
            .iter()
            .enumerate()
        {
            let mut x = Peer::client(quiet());
            if *state_peer != Idle {
                x.s().connect(0).unwrap();
                x.tick(0);
            }
            if *state_peer == EstablishedSession {
                x = established(quiet()).0;
            }
            x.receive(5, &ping_pkt(i as u32));
            x.receive(5, &nak_pkt(NakReason::OTHER, 0x55, ""));
            assert_eq!(x.state(), *state_peer);
            assert_eq!(all_codes(&x.tick(5)), [0x21]);
        }
    }

    #[test]
    fn command_with_no_action_gets_nak_02_and_no_state_change() {
        // REQ-NET-SES-004 (N-15) and REQ-NET-SES-008.
        let (c, mut h) = established(quiet());
        h.s().bye(0, ByeReason::USER_TERMINATED, "").unwrap();
        h.tick(0);
        // Invitation at a Host in Pending Bye: no section gives an action.
        let inv = payload(|w| {
            Invitation::write(
                w,
                &c.id.name,
                &c.id.product_instance_id,
                Capabilities::default(),
            )
            .unwrap()
        });
        h.receive(1, &inv);
        assert_eq!(h.state(), PendingBye);
        let out = h.tick(1);
        assert_eq!(all_codes(&out), [0x8F]);
        assert_eq!(headers(&out[0])[0].csd1, 0x02);
    }

    #[test]
    fn ping_and_nak_do_not_restart_the_handshake_timeout() {
        // REQ-NET-SES-002: Ping, Ping Reply and NAK start or stop no timeout.
        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        h.policy = HostPolicy::new(PolicyKind::AskCaller);
        c.s().connect(0).unwrap();
        round(0, &mut c, &mut h);
        assert_eq!(h.state(), PendingInvitation);
        let deadline = h.s().next_deadline();
        assert!(deadline.is_some());
        h.receive(30_000, &ping_pkt(7));
        assert_eq!(all_codes(&h.tick(30_000)), [0x21]);
        assert_eq!(h.s().next_deadline(), deadline);
        h.receive(30_001, &nak_pkt(NakReason::OTHER, 0x20, ""));
        assert_eq!(h.s().next_deadline(), deadline);
        assert_eq!(h.state(), PendingInvitation);
    }

    #[test]
    fn host_invitation_in_established_keeps_sequence_numbers_and_buffers() {
        // REQ-NET-INV-018: an Invitation in Established Session changes no Sequence Number
        // and no send history.
        let (mut c, mut h) = established(quiet());
        h.s().send_ump(&[NOTE]).unwrap();
        send(0, &mut h, &mut c);
        assert_eq!(c.data, [(0, false, vec![NOTE])]);
        let next = h.s().stream().sender().next_seq();
        let inv = payload(|w| {
            Invitation::write(
                w,
                &c.id.name,
                &c.id.product_instance_id,
                Capabilities::default(),
            )
            .unwrap()
        });
        h.receive(10, &inv);
        assert_eq!(all_codes(&h.tick(10)), [0x10]);
        assert_eq!(h.s().stream().sender().next_seq(), next);
        // The send history still holds Sequence Number 0.
        let rr = payload(|w| {
            RetransmitRequest {
                seq: SeqNum(0),
                count: 1,
            }
            .write(w)
            .unwrap()
        });
        h.receive(20, &rr);
        let out = h.tick(20);
        assert_eq!(all_codes(&out), [0xFF]);
        assert_eq!(headers(&out[0])[0].csd(), 0);
    }

    #[test]
    fn session_reset_empties_fec_retransmit_buffer_and_open_requests() {
        // REQ-NET-RST-009 with FEC on, a sent Command in the history and an open request.
        let mut cfg = quiet();
        cfg.stream.fec_repeats = 2;
        let (mut c, mut h) = established(cfg);
        c.s().send_ump(&[NOTE]).unwrap();
        send(0, &mut c, &mut h);
        // Sequence Number 5 opens a Retransmit Request for 1 to 4 at the Host.
        h.receive(10, &data_pkt(5, &[NOTE2]));
        assert!(h.s().next_deadline().is_some());
        c.s().reset(100).unwrap();
        send(100, &mut c, &mut h);
        send(100, &mut h, &mut c);
        assert_eq!(c.state(), EstablishedSession);
        // No open Retransmit Request at the Host.
        assert_eq!(h.s().next_deadline(), None);
        // The retransmit buffer of the Client is empty: a request for 0 gets an error.
        let rr = payload(|w| {
            RetransmitRequest {
                seq: SeqNum(0),
                count: 1,
            }
            .write(w)
            .unwrap()
        });
        c.receive(200, &rr);
        assert_eq!(all_codes(&c.tick(200)), [0x81]);
        // The FEC buffer is empty: the first UMP Data has only its own Command.
        c.s().send_ump(&[NOTE3]).unwrap();
        let out = c.tick(300);
        assert_eq!(all_codes(&out), [0xFF]);
        assert_eq!(headers(&out[0])[0].csd(), 0);
    }

    #[test]
    fn host_invitation_in_established_and_pending_reset_gets_accepted() {
        // REQ-NET-SES-008, N-05: Accepted and the session state stays.
        let (mut c, mut h) = established(quiet());
        c.s().send_ump(&[NOTE]).unwrap();
        send(0, &mut c, &mut h);
        let inv = payload(|w| {
            Invitation::write(
                w,
                &c.id.name,
                &c.id.product_instance_id,
                Capabilities::default(),
            )
            .unwrap()
        });
        h.receive(10, &inv);
        assert_eq!(h.state(), EstablishedSession);
        assert_eq!(all_codes(&h.tick(10)), [0x10]);
        assert_eq!(h.s().stream().receiver().window().highest(), SeqNum(0));
        h.s().reset(20).unwrap();
        h.tick(20);
        h.receive(30, &inv);
        assert_eq!(h.state(), PendingSessionReset);
        assert_eq!(all_codes(&h.tick(30)), [0x10]);
        // UMP Data in Pending Session Reset is valid and discarded (REQ-NET-SES-008).
        h.receive(40, &data_pkt(1, &[NOTE]));
        assert_eq!(h.state(), PendingSessionReset);
        assert!(h.tick(40).is_empty());
    }

    #[test]
    fn direction_of_the_command_code_is_checked_for_the_role() {
        // REQ-NET-SES-009.
        let (mut c, mut h) = established(quiet());
        let inv = payload(|w| {
            Invitation::write(
                w,
                &h.id.name,
                &h.id.product_instance_id,
                Capabilities::default(),
            )
            .unwrap()
        });
        c.receive(1, &inv);
        let out = c.tick(1);
        assert_eq!(all_codes(&out), [0x8F]);
        assert_eq!(headers(&out[0])[0].csd1, 0x02);
        assert_eq!(c.state(), EstablishedSession);
        let acc = payload(|w| {
            crate::command::InvitationReply::write(w, false, &c.id.name, &c.id.product_instance_id)
                .unwrap()
        });
        h.receive(1, &acc);
        let out = h.tick(1);
        assert_eq!(all_codes(&out), [0x8F]);
        assert_eq!(headers(&out[0])[0].csd1, 0x02);
        assert_eq!(h.state(), EstablishedSession);
        // 0x12 (H→C) at a Host, 0x02 (C→H) at a Client.
        h.receive(2, &payload(|w| w.command(0x12, 0, 0, |_| Ok(())).unwrap()));
        assert_eq!(all_codes(&h.tick(2)), [0x8F]);
        c.receive(2, &payload(|w| w.command(0x02, 0, 0, |_| Ok(())).unwrap()));
        assert_eq!(all_codes(&c.tick(2)), [0x8F]);
        assert_eq!(c.state(), EstablishedSession);
    }

    // ----- NAK -----

    #[test]
    fn nak_gives_an_event_and_changes_no_state_or_timeout() {
        // REQ-NET-NAK-005, N-29 (no NAK for a NAK).
        let (mut c, _) = established(quiet());
        c.s().reset(0).unwrap();
        c.tick(0);
        c.receive(1000, &nak_pkt(NakReason::OTHER, 0x82, "busy"));
        assert_eq!(c.take_ev(), [Ev::Nak(0x00, 0x82, "busy".into())]);
        assert_eq!(c.state(), PendingSessionReset);
        // The Session Reset timeout stays at 3000.
        assert_eq!(c.s().next_deadline(), Some(500));
        let mut t = 500;
        while t < 3000 {
            assert_eq!(all_codes(&c.tick(t)), [0x82]);
            t += 500;
        }
        assert_eq!(headers(&c.tick(3000)[0])[0].csd1, 0x04);
        // An undefined NAK Reason is given with its raw value; no reply.
        let (mut c, _) = established(quiet());
        c.receive(
            1,
            &payload(|w| w.command(0x8F, 0x77, 0, |p| p.word(0x2000_0000)).unwrap()),
        );
        assert_eq!(c.take_ev(), [Ev::Nak(0x77, 0x20, "".into())]);
        assert!(c.tick(1).is_empty());
        assert_eq!(c.state(), EstablishedSession);
        // With no entry: the event, no reply.
        let mut h = Peer::host(quiet());
        h.receive(1, &nak_pkt(NakReason::COMMAND_MALFORMED, 0x20, ""));
        assert_eq!(h.take_ev(), [Ev::Nak(0x03, 0x20, "".into())]);
        assert!(h.tick(1).is_empty());
    }

    #[test]
    fn nak_01_flags_the_command_code_for_the_session() {
        // REQ-NET-CMD-003.
        let cfg = SessionConfig {
            ping_interval_ms: 1000,
            ping_miss_limit: 0,
            ..quiet()
        };
        let (mut c, mut h) = established(cfg);
        h.s = None; // the Host does not answer
        c.receive(1, &nak_pkt(NakReason::COMMAND_NOT_SUPPORTED, 0x20, ""));
        c.receive(1, &nak_pkt(NakReason::COMMAND_NOT_SUPPORTED, 0x82, ""));
        c.receive(1, &nak_pkt(NakReason::COMMAND_NOT_SUPPORTED, 0xFF, ""));
        c.receive(1, &nak_pkt(NakReason::COMMAND_NOT_SUPPORTED, 0xF0, ""));
        assert!(c.s().is_refused(CommandCode::Ping));
        assert!(!c.s().is_refused(CommandCode::ByeReply));
        assert_eq!(
            c.s().ping(2, 1),
            Err(SessionError::Refused(CommandCode::Ping))
        );
        assert_eq!(
            c.s().reset(2),
            Err(SessionError::Refused(CommandCode::SessionReset))
        );
        assert_eq!(
            c.s().send_ump(&[NOTE]),
            Err(SessionError::Refused(CommandCode::UmpData))
        );
        // No automatic Ping.
        let mut t = 0;
        while t <= 5000 {
            assert!(c.tick(t).is_empty());
            t += 250;
        }
        // A reply that a "shall" rule demands is still written; Bye is not flagged.
        c.receive(5000, &ping_pkt(1));
        assert_eq!(all_codes(&c.tick(5000)), [0x21]);
        c.s().bye(5000, ByeReason::USER_TERMINATED, "").unwrap();
        assert_eq!(all_codes(&c.tick(5000)), [0xF0]);
        // A new session starts with no flags.
        let (mut c, _) = established(cfg);
        c.receive(1, &nak_pkt(NakReason::COMMAND_NOT_SUPPORTED, 0x20, ""));
        c.receive(
            2,
            &payload(|w| Bye::write(w, Role::Host, ByeReason::UNKNOWN, "").unwrap()),
        );
        c.tick(2);
        assert_eq!(c.state(), Idle);
        c.s().connect(3).unwrap();
        assert!(!c.s().is_refused(CommandCode::Ping));
        assert_eq!(c.s().ping(3, 1), Ok(()));
    }

    // ----- review fixes -----

    #[test]
    fn nak_01_for_the_repeated_invitation_ends_at_once() {
        // E1: no suitable reply can come: Bye 0x80 and Pending Bye, no past due time.
        let mut c = Peer::client(quiet());
        c.s().connect(0).unwrap();
        assert_eq!(all_codes(&c.tick(0)), [0x01]);
        c.receive(1, &nak_pkt(NakReason::COMMAND_NOT_SUPPORTED, 0x01, ""));
        assert_eq!(
            c.take_ev(),
            [Ev::Nak(0x01, 0x01, "".into()), Ev::Ending(0x80)]
        );
        assert_eq!(c.state(), PendingBye);
        let p = c.tick(1);
        assert_eq!(all_codes(&p), [0xF0]);
        assert_eq!(headers(&p[0])[0].csd1, 0x80);
        assert!(c.s().next_deadline().is_some_and(|t| t > 1));
        // The Bye timeout ends it.
        c.tick(3001);
        assert_eq!(c.state(), Idle);
    }

    #[test]
    fn nak_01_for_the_repeated_session_reset_ends_with_bye_04() {
        // E1: Session Reset refused: Bye 0x04 and Pending Bye.
        let (mut c, _) = established(quiet());
        c.s().reset(0).unwrap();
        assert_eq!(all_codes(&c.tick(0)), [0x82]);
        c.receive(1, &nak_pkt(NakReason::COMMAND_NOT_SUPPORTED, 0x82, ""));
        assert_eq!(
            c.take_ev(),
            [Ev::Nak(0x01, 0x82, "".into()), Ev::Ending(0x04)]
        );
        assert_eq!(c.state(), PendingBye);
        let p = c.tick(1);
        assert_eq!(all_codes(&p), [0xF0]);
        assert_eq!(headers(&p[0])[0].csd1, 0x04);
        assert!(c.s().next_deadline().is_some_and(|t| t > 1));
    }

    #[test]
    fn second_caller_ping_before_the_write_is_busy() {
        // E3: the first Ping is not replaced.
        let (mut c, _) = established(quiet());
        assert_eq!(c.s().ping(1, 7), Ok(()));
        assert_eq!(c.s().ping(1, 8), Err(SessionError::Busy));
        let p = c.tick(1);
        assert_eq!(all_codes(&p), [0x20]);
        assert_eq!(&p[0][8..12], &7u32.to_be_bytes());
        assert_eq!(c.s().ping(2, 8), Ok(()));
    }

    #[test]
    fn ping_ping_reply_and_nak_do_not_restart_the_host_answer_timeout() {
        // E4: REQ-NET-PING-005 and REQ-NET-NAK-005 over the "each Command" of
        // REQ-NET-SES-002.
        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        h.policy = HostPolicy::new(PolicyKind::AskCaller);
        c.s().connect(0).unwrap();
        send(0, &mut c, &mut h);
        h.take_ev();
        h.tick(0);
        assert_eq!(h.state(), PendingInvitation);
        h.receive(20_000, &ping_pkt(1));
        h.receive(30_000, &payload(|w| write_ping_reply(w, 9).unwrap()));
        h.receive(40_000, &nak_pkt(NakReason::OTHER, 0x10, ""));
        h.tick(40_000);
        assert_eq!(h.s().next_deadline(), Some(60_000));
        h.take_ev();
        h.tick(60_000);
        assert!(h.s.is_none());
        assert_eq!(h.take_ev(), [Ev::ClosedExpired]);
    }

    #[test]
    fn unexpected_reset_reply_gives_reset_started() {
        // E7: REQ-NET-RST-012, a Session Reset that this side starts by itself.
        let (_, mut h) = established(quiet());
        h.receive(10, &empty_pkt(CommandCode::SessionResetReply));
        assert_eq!(h.state(), PendingSessionReset);
        assert_eq!(h.take_ev(), [Ev::ResetStarted]);
    }

    #[cfg(feature = "auth")]
    #[test]
    fn idle_reply_bye_keeps_the_handshake_events() {
        // E6: no random bytes for the challenge: Bye 0x45 and the credentials error.
        let mut h = Peer::host(quiet());
        h.policy = HostPolicy {
            pending_fallback: false,
            ..HostPolicy::new(PolicyKind::SharedSecret)
        };
        h.creds.random = None;
        let mut inv = Vec::from(*b"MIDI");
        inv.extend([0x01, 0x04, 0x02, 0x01]);
        inv.extend(*b"MyDev\0\0\08shYe3h5");
        h.receive(0, &inv);
        assert_eq!(h.take_ev(), [Ev::Credentials]);
        assert_eq!(h.stateless, [Stateless::Bye(ByeReason::NO_MATCHING_METHOD)]);
        assert!(h.s.is_none());
    }

    // ----- Ping -----

    #[test]
    fn automatic_ping_at_the_interval() {
        // REQ-NET-PING-004.
        let cfg = SessionConfig {
            ping_interval_ms: 2000,
            ping_miss_limit: 0,
            ..quiet()
        };
        let (mut c, _) = established(cfg);
        c.s().set_next_ping_id(100);
        assert_eq!(c.s().next_deadline(), Some(2000));
        let mut sent = Vec::new();
        let mut t = 0;
        while t <= 6100 {
            for p in c.tick(t) {
                let h = headers(&p);
                assert_eq!(h[0].code, 0x20);
                sent.push((t, u32::from_be_bytes(p[8..12].try_into().unwrap())));
            }
            t += 100;
        }
        assert_eq!(sent, [(2000, 100), (4000, 101), (6000, 102)]);
        // A Ping of the caller starts the interval again.
        c.s().ping(6500, 7).unwrap();
        c.tick(6500);
        assert_eq!(c.s().next_deadline(), Some(8500));
        // Also in Pending Session Reset.
        c.s().reset(7000).unwrap();
        c.tick(7000);
        assert_eq!(all_codes(&c.tick(8500)), [0x82, 0x20]);
        // Interval 0: no Ping.
        let (mut c, _) = established(quiet());
        let mut t = 0;
        while t <= 20_000 {
            assert!(c.tick(t).is_empty());
            t += 500;
        }
        assert_eq!(c.s().next_deadline(), None);
    }

    #[test]
    fn ping_gets_a_reply_in_every_state() {
        // REQ-NET-PING-005: any address, any state, every Ping Id, no state change.
        let id = 0xFFFF_FFFF;
        let mut h = Peer::host(quiet());
        h.receive(0, &ping_pkt(id));
        assert!(h.s.is_none());
        let out = h.tick(0);
        assert_eq!(all_codes(&out), [0x21]);
        assert_eq!(&out[0][8..12], &id.to_be_bytes());
        let mut c = Peer::client(quiet());
        c.s().connect(0).unwrap();
        c.tick(0);
        c.receive(1, &ping_pkt(5));
        assert_eq!(all_codes(&c.tick(1)), [0x21]);
        assert_eq!(c.state(), PendingInvitation);
        assert_eq!(c.s().next_deadline(), Some(500), "no timer change");
        let (mut c, _) = established(quiet());
        c.s().reset(0).unwrap();
        c.tick(0);
        c.receive(1, &ping_pkt(6));
        assert_eq!(all_codes(&c.tick(1)), [0x21]);
        assert_eq!(c.state(), PendingSessionReset);
    }

    #[test]
    fn ping_reply_is_written_first() {
        // REQ-NET-PING-006.
        let (mut c, _) = established(quiet());
        c.s().send_ump(&[NOTE]).unwrap();
        c.receive(1, &nak_pkt(NakReason::OTHER, 0x55, "")); // no reply
        c.receive(
            1,
            &payload(|w| {
                RetransmitRequest {
                    seq: SeqNum(40),
                    count: 1,
                }
                .write(w)
                .unwrap()
            }),
        );
        c.receive(1, &ping_pkt(42));
        let out = c.tick(1);
        assert_eq!(all_codes(&out), [0x21, 0x81, 0xFF]);
    }

    #[test]
    fn ping_reply_closes_open_pings_and_gives_rtt() {
        // REQ-NET-PING-007.
        let (mut c, mut h) = established(quiet());
        c.s().ping(100, 1).unwrap();
        send_if(100, &mut c, &mut h, |_| true);
        send_if(100, &mut h, &mut c, |_| false); // lost reply
        c.s().ping(200, 2).unwrap();
        send(200, &mut c, &mut h);
        assert_eq!(c.s().open_pings().len(), 2);
        assert_eq!(c.s().missed_pings(), 2);
        let ps = h.tick(250);
        c.receive(250, &ps[0]);
        assert_eq!(c.take_ev(), [Ev::PingRtt(2, 50)]);
        assert!(
            c.s().open_pings().is_empty(),
            "the older Ping 1 is closed too"
        );
        assert_eq!(c.s().missed_pings(), 0);
        // A later reply to Ping 1 has no open Ping.
        c.receive(300, &payload(|w| write_ping_reply(w, 1).unwrap()));
        assert_eq!(c.take_ev(), [Ev::BadPingReply(1)]);
    }

    #[test]
    fn open_pings_table_keeps_the_last_p() {
        // REQ-NET-PING-007, N-27.
        let mut t = OpenPings::<3>::new();
        for i in 0..5 {
            t.push(i, u64::from(i) * 10);
        }
        assert_eq!(t.iter().map(|p| p.id).collect::<Vec<_>>(), [2, 3, 4]);
        assert_eq!(t.close(1), None);
        assert_eq!(t.close(3), Some(OpenPing { id: 3, sent_at: 30 }));
        assert_eq!(t.iter().map(|p| p.id).collect::<Vec<_>>(), [4]);
        assert_eq!(t.close(4).map(|p| p.sent_at), Some(40));
        assert!(t.is_empty());
        assert_eq!(OpenPings::<8>::default().len(), 0);
    }

    #[test]
    fn bad_ping_reply_is_ignored_by_default() {
        // REQ-NET-PING-008.
        let (mut c, _) = established(quiet());
        c.receive(1, &payload(|w| write_ping_reply(w, 99).unwrap()));
        assert_eq!(c.take_ev(), [Ev::BadPingReply(99)]);
        assert!(c.tick(1).is_empty());
        assert_eq!(c.state(), EstablishedSession);
        c.s().ping(2, 5).unwrap();
        c.tick(2);
        c.receive(3, &payload(|w| write_ping_reply(w, 6).unwrap()));
        assert!(c.tick(3).is_empty());
        assert_eq!(c.s().open_pings().len(), 1);
    }

    #[test]
    fn bad_ping_reply_gets_a_nak_with_the_option() {
        // REQ-NET-PING-010, REQ-NET-PING-008 (option on).
        let cfg = SessionConfig {
            nak_bad_ping_reply: true,
            ..quiet()
        };
        let (mut c, _) = established(cfg);
        let reply = payload(|w| write_ping_reply(w, 99).unwrap());
        c.receive(1, &reply);
        let out = c.tick(1);
        assert_eq!(all_codes(&out), [0x8F]);
        assert_eq!(headers(&out[0])[0].csd1, 0x02);
        assert_eq!(&out[0][8..12], &reply[4..8], "the received header word");
        c.s().ping(2, 5).unwrap();
        c.tick(2);
        c.receive(3, &reply);
        let out = c.tick(3);
        assert_eq!(headers(&out[0])[0].csd1, 0x20);
        assert_eq!(&out[0][8..12], &reply[4..8]);
        assert_eq!(c.state(), EstablishedSession);
        // No entry: NAK 0x02.
        let mut h = Peer::host(cfg);
        h.receive(1, &reply);
        assert_eq!(headers(&h.tick(1)[0])[0].csd1, 0x02);
    }

    #[test]
    fn ping_miss_limit_ends_the_session_with_bye_04() {
        // REQ-NET-PING-009, REQ-NET-BYE-014.
        let cfg = SessionConfig {
            ping_interval_ms: 2000,
            ping_miss_limit: 5,
            ..quiet()
        };
        // With replies the session stays.
        let (mut c, mut h) = established(cfg);
        run(&mut c, &mut h, 0, 20_000, 100);
        assert_eq!(c.state(), EstablishedSession);
        assert_eq!(h.state(), EstablishedSession);
        assert!(c.take_ev().iter().all(|e| matches!(e, Ev::PingRtt(_, 0))));
        // No reply: five Pings, then Bye 0x04 at the next interval.
        let (mut c, mut h) = established(cfg);
        let mut t = 0;
        let mut pings = 0;
        while t < 12_000 {
            for p in send_if(t, &mut c, &mut h, |_| true) {
                pings += codes(&p).iter().filter(|c| **c == 0x20).count();
            }
            h.tick(t); // the replies are lost
            t += 100;
        }
        assert_eq!(pings, 5);
        assert_eq!(c.s().missed_pings(), 5);
        assert_eq!(c.state(), EstablishedSession);
        let out = c.tick(12_000);
        assert_eq!(all_codes(&out), [0xF0]);
        assert_eq!(headers(&out[0])[0].csd1, 0x04);
        assert_eq!(c.state(), PendingBye);
        assert_eq!(c.take_ev(), [Ev::Timeout, Ev::Ending(4)]);
    }

    // ----- Retransmit -----

    #[test]
    fn retransmit_recovers_a_lost_payload() {
        // REQ-NET-RTX-004 (resend), REQ-NET-RTX-012.
        let (mut c, mut h) = established(quiet());
        c.s().send_ump(&[NOTE]).unwrap();
        send(0, &mut c, &mut h);
        c.s().send_ump(&[NOTE2]).unwrap();
        send_if(1, &mut c, &mut h, |_| false);
        c.s().send_ump(&[NOTE3]).unwrap();
        send(2, &mut c, &mut h);
        assert_eq!(h.data.len(), 2);
        run(&mut h, &mut c, 3, 40, 1);
        assert!(all_codes(&h.written).contains(&0x80));
        assert_eq!(h.data[2], (1, true, vec![NOTE2]));
        assert_eq!(c.state(), EstablishedSession);
        assert_eq!(h.state(), EstablishedSession);
    }

    #[test]
    fn retransmit_off_gets_one_nak_01_for_each_request() {
        // REQ-NET-RTX-007, REQ-NET-RTX-004 (NAK when off), REQ-NET-CMD-003.
        let off = SessionConfig {
            stream: StreamConfig {
                retransmit: false,
                ..quiet().stream
            },
            ..quiet()
        };
        let mut c = Peer::client(off);
        let mut h = Peer::host(quiet());
        c.s().connect(0).unwrap();
        round(0, &mut c, &mut h);
        let req = payload(|w| {
            RetransmitRequest {
                seq: SeqNum(0),
                count: 1,
            }
            .write(w)
            .unwrap()
        });
        let mut two = req.clone();
        two.extend_from_slice(&req[4..]);
        c.receive(1, &two);
        let out = c.tick(1);
        assert_eq!(all_codes(&out), [0x8F, 0x8F]);
        for hd in headers(&out[0]) {
            assert_eq!(hd.csd1, 0x01);
        }
        assert_eq!(&out[0][8..12], &req[4..8]);
        // The receiver of the NAK writes no more requests and reports the loss.
        c.s().send_ump(&[NOTE]).unwrap();
        send(2, &mut c, &mut h);
        c.s().send_ump(&[NOTE2]).unwrap();
        send_if(3, &mut c, &mut h, |_| false);
        c.s().send_ump(&[NOTE3]).unwrap();
        send(4, &mut c, &mut h);
        run(&mut h, &mut c, 5, 40, 1);
        assert!(h.s().is_refused(CommandCode::RetransmitRequest));
        let cfg = h.s().config().stream;
        assert!(!h.s().stream().receiver().requests_enabled(&cfg));
        assert!(h
            .take_ev()
            .iter()
            .any(|e| matches!(e, Ev::Stream(StreamEvent::DataLoss { .. }))));
        assert_eq!(h.state(), EstablishedSession);
    }

    #[test]
    fn retransmit_error_written_or_received_keeps_the_state() {
        // REQ-NET-RTX-012, REQ-NET-RTX-004 (Retransmit Error).
        let (mut c, _) = established(quiet());
        c.receive(
            1,
            &payload(|w| {
                RetransmitRequest {
                    seq: SeqNum(9),
                    count: 1,
                }
                .write(w)
                .unwrap()
            }),
        );
        let out = c.tick(1);
        assert_eq!(all_codes(&out), [0x81]);
        assert_eq!(c.state(), EstablishedSession);
        let e = RetransmitError {
            reason: RetransmitErrorReason::NOT_IN_BUFFER,
            seq: SeqNum(0),
        };
        c.receive(2, &payload(|w| e.write(w).unwrap()));
        assert_eq!(c.state(), EstablishedSession);
        assert_eq!(c.take_ev(), [Ev::Stream(StreamEvent::RetransmitError(e))]);
    }

    // ----- Host decisions and idle replies -----

    #[test]
    fn host_caller_accepts_or_rejects() {
        // REQ-NET-INV-026 through the Session; REQ-NET-BYE-006 for the Host reject.
        let ask = HostPolicy::new(PolicyKind::AskCaller);
        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        h.policy = ask;
        c.s().connect(0).unwrap();
        round(0, &mut c, &mut h);
        assert_eq!(c.state(), PendingInvitation);
        assert_eq!(c.s().accept(1), Err(SessionError::WrongRole));
        let ev = h.s().accept(100).unwrap();
        assert_eq!(
            ev.iter().map(own).collect::<Vec<_>>(),
            [Ev::Established(None)]
        );
        assert_eq!(
            h.s().accept(100),
            Err(SessionError::WrongState(EstablishedSession))
        );
        round(100, &mut c, &mut h);
        assert_eq!(c.state(), EstablishedSession);

        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        h.policy = ask;
        c.s().connect(0).unwrap();
        round(0, &mut c, &mut h);
        h.s().reject(100, None).unwrap();
        assert_eq!(h.state(), PendingBye);
        round(100, &mut c, &mut h);
        round(100, &mut c, &mut h);
        assert_eq!(c.take_ev(), [Ev::Pending, Ev::ClosedBye(0x42, "".into())]);
        assert_eq!(h.take_ev(), [Ev::Invitation, Ev::ClosedByeReply]);
        assert!(h.s.is_none());
    }

    #[test]
    fn client_cancel_and_host_answer_timeout() {
        // REQ-NET-BYE-006 (pending Invitation, Bye 0x80), REQ-NET-SES-002 via the Session.
        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        h.policy = HostPolicy::new(PolicyKind::AskCaller);
        c.s().connect(0).unwrap();
        round(0, &mut c, &mut h);
        assert_eq!(
            c.s().bye(1, ByeReason::USER_TERMINATED, ""),
            Err(SessionError::Handshake(HandshakeError::ReasonNotAllowed))
        );
        let ev = c
            .s()
            .bye(1, ByeReason::INVITATION_CANCELED, "stop")
            .unwrap();
        assert_eq!(ev.iter().map(own).collect::<Vec<_>>(), [Ev::Ending(0x80)]);
        let ps = send(1, &mut c, &mut h);
        assert_eq!(headers(&ps[0])[0].csd1, 0x80);
        assert_eq!(&ps[0][8..12], b"stop");
        assert_eq!(
            h.take_ev(),
            [Ev::Invitation, Ev::ClosedBye(0x80, "stop".into())]
        );
        send(1, &mut h, &mut c);
        assert_eq!(c.state(), Idle);
        // Host answer timeout: Idle with no Command.
        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        h.policy = HostPolicy::new(PolicyKind::AskCaller);
        c.s().connect(0).unwrap();
        send(0, &mut c, &mut h);
        h.take_ev();
        h.tick(0);
        assert_eq!(h.s().next_deadline(), Some(60_000));
        assert!(h.tick(60_000).is_empty());
        assert!(h.s.is_none());
        assert_eq!(h.take_ev(), [Ev::ClosedExpired]);
        // cancel(): default reason.
        let mut c = Peer::client(quiet());
        c.s().connect(0).unwrap();
        c.s().cancel(1).unwrap();
        assert_eq!(headers(&c.tick(1)[0])[0].csd1, 0x80);
    }

    #[test]
    fn idle_reply_gives_the_stateless_replies() {
        // REQ-NET-BYE-009, REQ-NET-PING-005, REQ-NET-RST-011, REQ-NET-DATA-015,
        // REQ-NET-NAK-005, REQ-NET-SES-001 (no entry = Idle).
        let cfg = quiet();
        let mut creds = Creds::default();
        let mut env = Env::new(&mut creds);
        let h = |code: u8| Header::from_bytes([code, 0, 0, 0]);
        let ask = |role: Role, code: u8, cmd: Command<'static>, first: bool| {
            let mut creds = Creds::default();
            let mut env = Env::new(&mut creds);
            match S::idle_reply(0, role, h(code), &cmd, first, &cfg, &mut env) {
                IdleReply::Send(s, _) => Some(s),
                IdleReply::Ignore => None,
                other => panic!("{other:?}"),
            }
        };
        let bye05 = Some(Stateless::Bye(ByeReason::SESSION_NOT_ESTABLISHED));
        assert_eq!(
            ask(Role::Host, 0x20, Command::Ping(3), true),
            Some(Stateless::PingReply(3))
        );
        let bye = Command::Bye(Bye {
            reason: ByeReason::UNKNOWN,
            text: crate::packet::NetStr::from_field(&[]),
        });
        assert_eq!(
            ask(Role::Client, 0xF0, bye, true),
            Some(Stateless::ByeReply)
        );
        assert_eq!(ask(Role::Host, 0xF1, Command::ByeReply, true), None);
        assert_eq!(ask(Role::Host, 0x82, Command::SessionReset, false), bye05);
        assert_eq!(
            ask(Role::Host, 0x83, Command::SessionResetReply, true),
            bye05
        );
        let data = Command::RetransmitRequest(RetransmitRequest {
            seq: SeqNum(0),
            count: 0,
        });
        assert_eq!(ask(Role::Host, 0x80, data, true), bye05);
        assert_eq!(ask(Role::Host, 0x80, data, false), None);
        assert_eq!(ask(Role::Host, 0x21, Command::PingReply(1), true), None);
        // A Client gets an Invitation Reply: Accepted with no entry: Bye 0x06.
        let acc = Command::InvitationReply(crate::command::InvitationReply {
            pending: false,
            name: crate::packet::NetStr::from_field(b"H"),
            product_instance_id: crate::packet::NetStr::from_field(b"1"),
        });
        assert_eq!(
            ask(Role::Client, 0x10, acc, true),
            Some(Stateless::Bye(ByeReason::NO_PENDING_SESSION))
        );
        assert_eq!(
            ask(Role::Host, 0x10, acc, true),
            Some(Stateless::Nak(NakReason::COMMAND_NOT_EXPECTED, h(0x10)))
        );
        let nak = Command::Nak(Nak {
            reason: NakReason::OTHER,
            nak_header: h(0x20),
            text: crate::packet::NetStr::from_field(&[]),
        });
        assert!(matches!(
            S::idle_reply(0, Role::Host, h(0x8F), &nak, true, &cfg, &mut env),
            IdleReply::Event(SessionEvent::Nak(_))
        ));
        let inv = Command::Invitation(crate::command::Invitation {
            name: crate::packet::NetStr::from_field(b"C"),
            product_instance_id: crate::packet::NetStr::from_field(b"1"),
            capabilities: Capabilities::default(),
        });
        let IdleReply::Open(o) = S::idle_reply(0, Role::Host, h(0x01), &inv, true, &cfg, &mut env)
        else {
            panic!()
        };
        assert_eq!(o.outcome().next, Some(EstablishedSession));
        assert_eq!(o.handshake().role(), Role::Host);
        let mut cl = S::client(cfg, Capabilities::default()).unwrap();
        assert_eq!(cl.open(0, o).err(), Some(SessionError::WrongRole));
        // An empty name: Bye 0x07 once, no entry (N-20).
        let bad = Command::Invitation(crate::command::Invitation {
            name: crate::packet::NetStr::from_field(&[]),
            product_instance_id: crate::packet::NetStr::from_field(b"1"),
            capabilities: Capabilities::default(),
        });
        assert_eq!(
            ask(Role::Host, 0x01, bad, true),
            Some(Stateless::Bye(ByeReason::PROTOCOL_ERROR))
        );
        // The Stateless writer.
        let p = payload(|w| Stateless::ByeReply.write(Role::Host, w).unwrap());
        assert_eq!(codes(&p), [0xF1]);
    }

    #[cfg(not(feature = "auth"))]
    #[test]
    fn auth_policy_without_feature_gets_bye_45() {
        // N-06, DEC-013: the same API; no Client has a matching method.
        let mut c = Peer::client(quiet());
        let mut h = Peer::host(quiet());
        h.policy = HostPolicy::new(PolicyKind::SharedSecret);
        c.s().connect(0).unwrap();
        round(0, &mut c, &mut h);
        assert!(h.s.is_none());
        assert_eq!(c.state(), Idle);
        assert_eq!(c.take_ev(), [Ev::ClosedBye(0x45, "".into())]);
    }

    #[cfg(feature = "auth")]
    #[test]
    fn shared_secret_authentication_through_payloads() {
        // The Session applies the Handshake Outcomes of 6.7 and 6.9 (REQ-NET-AUTH-*).
        let mut c = Peer::client(quiet());
        c.s = Some(
            S::client(
                quiet(),
                Capabilities {
                    auth: true,
                    user_auth: false,
                },
            )
            .unwrap(),
        );
        c.creds.secret = Some(b"secret".to_vec());
        let mut h = Peer::host(quiet());
        h.policy = HostPolicy::new(PolicyKind::SharedSecret);
        h.creds.random = Some([7; 16]);
        h.creds.secret = Some(b"secret".to_vec());
        c.s().connect(0).unwrap();
        send(0, &mut c, &mut h);
        assert_eq!(h.state(), AuthenticationRequired);
        let ps = send(0, &mut h, &mut c);
        assert_eq!(all_codes(&ps), [0x12]);
        assert_eq!(c.take_ev(), [Ev::AuthRequested(Method::SharedSecret)]);
        assert_eq!(c.state(), PendingInvitation);
        // The Invitation with Authentication is lost once: repeated with the same bytes.
        let a1 = send_if(0, &mut c, &mut h, |_| false);
        assert_eq!(all_codes(&a1), [0x02]);
        let a2 = send(500, &mut c, &mut h);
        assert_eq!(a2, a1);
        send(500, &mut h, &mut c);
        assert_eq!(c.state(), EstablishedSession);
        assert_eq!(h.state(), EstablishedSession);
        assert_eq!(c.take_ev(), [Ev::Established(Some(Method::SharedSecret))]);
        assert_eq!(h.take_ev(), [Ev::Established(Some(Method::SharedSecret))]);
        // The repeat is answered by the Host again (REQ-NET-AUTH-029).
        h.receive(600, &a1[0]);
        assert_eq!(all_codes(&h.tick(600)), [0x10]);
    }

    #[cfg(feature = "auth")]
    #[test]
    fn failed_authentication_limit_ends_the_host_entry() {
        // REQ-NET-AUTH-022 through the Session: the Host entry goes to Idle with one Bye.
        let cfg = SessionConfig {
            handshake: HandshakeConfig {
                failure_limit: 1,
                ..HandshakeConfig::DEFAULT
            },
            ..quiet()
        };
        let mut c = Peer::client(cfg);
        c.s = Some(
            S::client(
                cfg,
                Capabilities {
                    auth: true,
                    user_auth: false,
                },
            )
            .unwrap(),
        );
        c.creds.secret = Some(b"wrong".to_vec());
        let mut h = Peer::host(cfg);
        h.policy = HostPolicy::new(PolicyKind::SharedSecret);
        h.creds.random = Some([9; 16]);
        h.creds.secret = Some(b"secret".to_vec());
        c.s().connect(0).unwrap();
        round(0, &mut c, &mut h);
        round(0, &mut c, &mut h);
        assert!(h.s.is_none());
        assert_eq!(
            h.take_ev(),
            [Ev::AuthFailed(1), Ev::Ending(0x43), Ev::ClosedEnded(0x43)]
        );
        assert_eq!(c.state(), Idle);
        assert!(c.take_ev().contains(&Ev::ClosedBye(0x43, "".into())));
    }

    #[test]
    fn prop_receive_never_panics() {
        // Nothing panics on received bytes; NAK is never answered with a NAK (N-29).
        proptest!(|(cmds in proptest::collection::vec(
            (any::<u8>(), 0u8..4, any::<u8>(), any::<u8>(), proptest::collection::vec(any::<u8>(), 0..16)),
            1..6), est in any::<bool>(), t in 0u64..100_000)| {
            let (mut c, mut h) = if est { established(SessionConfig::DEFAULT) } else {
                let mut c = Peer::client(SessionConfig::DEFAULT);
                c.s().connect(0).unwrap();
                (c, Peer::host(SessionConfig::DEFAULT))
            };
            let mut p = Vec::from(*b"MIDI");
            for (code, len, a, b, body) in &cmds {
                let mut body = body.clone();
                body.resize(usize::from(*len) * 4, 0);
                p.extend_from_slice(&[*code, *len, *a, *b]);
                p.extend_from_slice(&body);
            }
            c.receive(t, &p);
            h.receive(t, &p);
            for out in c.tick(t).iter().chain(h.tick(t).iter()) {
                prop_assert!(command_packets(out).is_ok());
            }
            if cmds.iter().all(|c| c.0 == 0x8F) {
                prop_assert!(!all_codes(&c.written).contains(&0x8F));
            }
            let _ = (c.tick(t + 5000), h.tick(t + 5000));
        });
    }

    #[test]
    fn events_list_has_a_fixed_capacity() {
        let mut e = Events::default();
        for _ in 0..EVENTS_CAPACITY + 2 {
            e.push(SessionEvent::Reset);
        }
        assert_eq!(e.len(), EVENTS_CAPACITY);
        assert!(!e.is_empty());
        let _ = format!("{:?}", e);
    }
}
