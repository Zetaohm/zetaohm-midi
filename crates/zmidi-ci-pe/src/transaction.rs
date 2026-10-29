//! The Property Exchange transaction engine (M2-103-UM v1.2 sections 5.3, 7.4, 8.6.2, 9, 11,
//! 12, 14 and 15).
//!
//! - Sans-IO: the engine reads no clock and does no I/O. The caller gives the clock time in
//!   milliseconds with each call, gives the zmidi-ci Device events to [`PeEngine::receive`], and
//!   sends the bytes that the engine writes into the caller buffers.
//! - Fixed memory (DEC-002): `T` open transactions with a header buffer of `H` bytes each, a
//!   subscription table of `S` entries and `P` peers whose ResourceList is known.
//! - The Property Data is streamed one Chunk at a time (DEC-007). The engine decodes "ASCII" and
//!   "Mcoded7". For "zlib+Mcoded7" it decodes the Mcoded7 step and gives the zlib stream, which
//!   the caller inflates (module Zlib); the engine checks the zlib window (413).
//! - The caller encodes the Property Data that it sends (modules Encoding and Zlib) and gives
//!   one Chunk of encoded bytes to [`PeEngine::send`] at a time.
//! - The readings of DEC-PE-002: statuses that M2-103 does not give, Chunk gaps (NAK 0x21),
//!   timeouts, retransmit, and the JUCE NAK 0x00 that answers an ACK (DEC-010 D-2, D-3).
//! - Nothing panics on received bytes.

use zmidi_ci::{
    Ack, AckStatus, ChunkEnd, CiDevice, Details, DeviceError, DeviceId, Direction, Event, Framing,
    Muid, Nak, NakStatus, PeChunk, PeKind, WriteError,
};

use crate::encoding::Mcoded7Decoder;
use crate::header::{
    Command, EncodingName, Header, HeaderError, HeaderKind, HeaderStr, Status, StatusClass,
};
use crate::json::JsonError;
use crate::resource_list::{Operation, ResourceCaps, ResourceEntry, ResourceListWriter};
use crate::subscription::{
    check_send, start_header, Role, SubscriptionEntry, SubscriptionError, SubscriptionInput,
    SubscriptionTable,
};

/// The timeout of a transaction: 3 seconds after its last Chunk (REQ-PE-END-008, -009, -011).
pub const TIMEOUT_MS: u64 = 3_000;
/// The wait of a Flow Control receiver for the next Chunk (REQ-PE-FLOW-014).
pub const FLOW_RECEIVER_WAIT_MS: u64 = 12_000;
/// Number of times a Flow Control sender sends a Chunk again (REQ-PE-FLOW-013).
pub const RETRANSMITS: u8 = 3;
/// The largest wait of a MIDI-CI ACK 0x10: 0x7F x 100 ms (REQ-PE-END-010).
pub const MAX_WAIT_MS: u64 = 12_700;
/// Notify status 100 adds this time (REQ-PE-END-012).
const NOTIFY_WAIT_MS: u64 = 3_000;
/// MIDI-CI header and fixed Chunk fields (M2-101 Table 33).
const CHUNK_OVERHEAD: usize = 13 + 9;
/// Size used when a peer reported no Receivable Maximum SysEx Message Size.
const DEFAULT_MAX_SYSEX: u32 = 128;
/// The default capacity `Q` of the control message queue of [`PeEngine`] (OPT-006).
pub const DEFAULT_QUEUE: usize = 16;
/// Original Transaction Sub-ID#2 of a MIDI-CI ACK (END-016: a NAK that answers an ACK).
const SUB_ID2_ACK: u8 = 0x7D;

/// What the engine needs from the zmidi-ci Device; `CiDevice` implements it.
pub trait PeHost {
    /// The MUID of this Device.
    fn muid(&self) -> Muid;
    /// The framing of written messages.
    fn framing(&self) -> Framing;
    /// Receivable Maximum SysEx Message Size of `peer`; 128 when unknown.
    fn peer_max_sysex(&self, peer: Muid) -> u32;
    /// Number of Simultaneous Property Exchange Requests Supported of `peer`; `None` before
    /// the Property Exchange Capabilities exchange.
    fn peer_simultaneous(&self, peer: Muid) -> Option<u8>;
    /// Allocates a Request ID for a new inquiry to `peer`.
    fn allocate(&mut self, peer: Muid, kind: PeKind) -> Result<u8, DeviceError>;
    /// Frees a Request ID.
    fn end(&mut self, peer: Muid, request_id: u8, direction: Direction);
    /// Writes a MIDI-CI ACK to `peer`.
    fn write_ack(&self, peer: Muid, ack: &Ack<'_>, out: &mut [u8]) -> Result<usize, DeviceError>;
    /// Writes a MIDI-CI NAK to `peer`.
    fn write_nak(&self, peer: Muid, nak: &Nak<'_>, out: &mut [u8]) -> Result<usize, DeviceError>;
}

impl<const PE: usize, const PR: usize, const RQ: usize> PeHost for CiDevice<'_, PE, PR, RQ> {
    fn muid(&self) -> Muid {
        CiDevice::muid(self)
    }

    fn framing(&self) -> Framing {
        self.config().framing
    }

    fn peer_max_sysex(&self, peer: Muid) -> u32 {
        self.peer(peer)
            .map(|p| p.max_sysex_size)
            .filter(|m| *m >= DEFAULT_MAX_SYSEX)
            .unwrap_or(DEFAULT_MAX_SYSEX)
    }

    fn peer_simultaneous(&self, peer: Muid) -> Option<u8> {
        self.peer(peer)
            .and_then(|p| p.pe_capabilities)
            .map(|c| c.simultaneous)
    }

    fn allocate(&mut self, peer: Muid, kind: PeKind) -> Result<u8, DeviceError> {
        self.pe_request(peer, kind, &[], None, None)
            .map(|(id, _)| id)
    }

    fn end(&mut self, peer: Muid, request_id: u8, direction: Direction) {
        self.pe_end(peer, request_id, direction);
    }

    fn write_ack(&self, peer: Muid, ack: &Ack<'_>, out: &mut [u8]) -> Result<usize, DeviceError> {
        self.ack(peer, DeviceId::FunctionBlock, ack, "", out)
    }

    fn write_nak(&self, peer: Muid, nak: &Nak<'_>, out: &mut [u8]) -> Result<usize, DeviceError> {
        self.nak(peer, DeviceId::FunctionBlock, nak, "", out)
    }
}

/// Error of the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransactionError {
    /// The zmidi-ci Device refused (unknown peer, no PE Capabilities, no free Request ID).
    #[error("device: {0}")]
    Device(#[from] DeviceError),
    /// A message writer failed (the output buffer is too small).
    #[error("write: {0}")]
    Write(#[from] WriteError),
    /// The header writer refused the header.
    #[error("header: {0}")]
    Header(#[from] HeaderError),
    /// A subscription rule refused the message.
    #[error("subscription: {0}")]
    Subscription(#[from] SubscriptionError),
    /// No open transaction has this peer, Request ID and role.
    #[error("no such transaction")]
    UnknownTransaction,
    /// All `T` transaction slots are in use.
    #[error("no free transaction slot")]
    Full,
    /// The peer's Number of Simultaneous Requests is reached (REQ-PE-GEN-003).
    #[error("peer has its maximum of open requests")]
    PeerBusy,
    /// The deadline passed; nothing was sent and the transaction is closed
    /// (REQ-PE-END-008, -009).
    #[error("deadline passed")]
    Late,
    /// The call does not fit the state of the transaction (for example, a Chunk while the
    /// Flow Control ACK of the Chunk before has not come).
    #[error("not permitted in this state")]
    State,
    /// The ResourceList of the peer does not permit the request; the status it would get.
    #[error("refused by the peer's ResourceList: {0:?}")]
    Refused(Status),
    /// Property Data that is not JSON needs a "mediaType" (REQ-PE-ENC-002).
    #[error("mediaType needed")]
    NoMediaType,
    /// A reply for a Resource with "canPaginate" needs "totalCount" (REQ-PE-TXN-007).
    #[error("totalCount needed")]
    NoTotalCount,
    /// The header does not fit the header buffer or the first Chunk.
    #[error("header too large")]
    HeaderTooLarge,
    /// A Chunk part larger than the room of the Chunk, or a scratch buffer smaller than the
    /// received Property Data.
    #[error("buffer size")]
    Size,
    /// The queue of control messages is full: call [`PeEngine::write_pending`] first. Nothing
    /// was changed.
    #[error("pending queue full")]
    QueueFull,
    /// A wait of 0 or more than 12.7 seconds (REQ-PE-END-010).
    #[error("invalid wait")]
    InvalidWait,
    /// A "$ref" value that is not "midi+jsonschema://<id>" (REQ-PE-LIST-010).
    #[error("invalid $ref")]
    InvalidRef,
}

/// The kind of a transaction, by its inquiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxnKind {
    /// Inquiry: Get Property Data (0x34) and its reply (0x35).
    Get,
    /// Inquiry: Set Property Data (0x36) and its reply (0x37).
    Set,
    /// Subscription (0x38) and Reply to Subscription (0x39).
    Subscription,
}

impl TxnKind {
    /// The kind of the inquiry message.
    pub const fn inquiry(self) -> PeKind {
        match self {
            TxnKind::Get => PeKind::GetInquiry,
            TxnKind::Set => PeKind::SetInquiry,
            TxnKind::Subscription => PeKind::Subscription,
        }
    }

    /// The kind of the reply message.
    pub const fn reply(self) -> PeKind {
        match self {
            TxnKind::Get => PeKind::GetReply,
            TxnKind::Set => PeKind::SetReply,
            TxnKind::Subscription => PeKind::SubscriptionReply,
        }
    }

    const fn of(kind: PeKind) -> Option<(TxnKind, bool)> {
        Some(match kind {
            PeKind::GetInquiry => (TxnKind::Get, true),
            PeKind::GetReply => (TxnKind::Get, false),
            PeKind::SetInquiry => (TxnKind::Set, true),
            PeKind::SetReply => (TxnKind::Set, false),
            PeKind::Subscription => (TxnKind::Subscription, true),
            PeKind::SubscriptionReply => (TxnKind::Subscription, false),
            PeKind::Notify => return None,
        })
    }

    fn header_kind(self) -> HeaderKind {
        match self {
            TxnKind::Subscription => HeaderKind::Subscription,
            _ => HeaderKind::Inquiry,
        }
    }
}

/// Why a transaction closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// A deadline of this Device passed (REQ-PE-END-008, -009, -013, REQ-PE-FLOW-013, -014).
    Timeout,
    /// The peer ended it: NAK 0x20 or Notify 144 (REQ-PE-END-003, -005, -006).
    Terminated,
    /// The peer timed out: NAK 0x42 or Notify 408 (REQ-PE-END-014, -015).
    PeerTimeout,
    /// A Chunk came out of sequence; this Device sent NAK 0x21 (REQ-PE-STAT-008).
    OutOfSequence,
    /// The peer sent another NAK for the transaction.
    Nak(NakStatus),
    /// The Header Data of the reply is not valid (REQ-PE-STAT-009).
    BadReply(HeaderError),
    /// This Device sent its reply as a status after an error (REQ-PE-STAT-008, -011).
    Refused(Status),
    /// An Invalidate MUID ended it.
    Invalidated,
    /// A Flow Control resend that the sender can not do (REQ-PE-FLOW-012).
    ResendFailed,
}

/// One received Chunk of a Property Exchange message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message<'m, 's> {
    /// The other Device.
    pub peer: Muid,
    /// Request ID of the transaction.
    pub request_id: u8,
    /// The role of this Device in the transaction.
    pub role: Direction,
    /// The kind of the transaction.
    pub kind: TxnKind,
    /// The Header Data, in the first Chunk only.
    pub header: Option<Header<'m>>,
    /// Index of the Resource in the Resource table (an inquiry to this Device); `None` for
    /// "ResourceList" when the table has no entry for it, and for replies.
    pub resource: Option<usize>,
    /// The encoding of the Property Data.
    pub encoding: EncodingName,
    /// The Property Data of this Chunk: decoded "ASCII" and "Mcoded7" data, or the zlib
    /// stream of "zlib+Mcoded7".
    pub data: &'s [u8],
    /// `More`, or the end of the message (`Incomplete`: the data is not known to be usable,
    /// REQ-PE-STAT-008).
    pub end: ChunkEnd,
    /// The subscription table entry of a Subscription message, or of a Get that the engine
    /// sent after a "notify" (REQ-PE-SUB-014).
    pub subscription: Option<usize>,
    /// For the reply to a "start": the new subscription table entry, or why there is none
    /// (REQ-PE-SUB-006).
    pub started: Option<Result<usize, SubscriptionError>>,
}

/// An event of the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum PeEvent<'m, 's> {
    /// A received Chunk.
    Message(Message<'m, 's>),
    /// Flow Control: the ACK of the last Chunk came; send the next Chunk (REQ-PE-FLOW-002).
    Ready {
        /// The other Device.
        peer: Muid,
        /// Request ID.
        request_id: u8,
        /// The role of this Device.
        role: Direction,
    },
    /// Flow Control: send this Chunk again with [`PeEngine::send`] (REQ-PE-FLOW-012, -013).
    Resend {
        /// The other Device.
        peer: Muid,
        /// Request ID.
        request_id: u8,
        /// The role of this Device.
        role: Direction,
        /// The number of the Chunk to send again.
        chunk: u16,
    },
    /// A transaction closed.
    Closed {
        /// The other Device.
        peer: Muid,
        /// Request ID.
        request_id: u8,
        /// The role of this Device.
        role: Direction,
        /// Why.
        reason: CloseReason,
    },
    /// This Device accepted a "start" and replied 200 (REQ-PE-SUB-007).
    SubscriptionStarted {
        /// The new subscription table entry.
        index: usize,
    },
    /// A subscription ended by an "end" from the peer (REQ-PE-SUB-018).
    SubscriptionEnded(SubscriptionEntry),
    /// An Invalidate MUID closed the transactions of a peer, or of this Device (`muid` is then
    /// the old MUID). `ended` subscriptions wait in [`PeEngine::take_ended`]
    /// (REQ-PE-SUB-019).
    Invalidated {
        /// The invalidated MUID.
        muid: Muid,
        /// Number of removed subscriptions.
        ended: usize,
    },
}

/// A message that the engine sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outgoing<'h> {
    /// The Header Data Properties.
    pub header: Header<'h>,
    /// The encoding of the Property Data that the caller sends; for a Get inquiry, the
    /// encoding that it asks for the reply (REQ-PE-ENC-004, REQ-PE-HDR-008).
    pub encoding: EncodingName,
    /// False when the Property Data is not JSON: a "mediaType" is then needed
    /// (REQ-PE-ENC-002).
    pub json: bool,
    /// The length of the encoded Property Data, `None` when not known.
    pub total_len: Option<usize>,
}

impl<'h> Outgoing<'h> {
    /// A JSON message in "ASCII" with `len` bytes of Property Data.
    pub fn json(header: Header<'h>, len: usize) -> Self {
        Outgoing {
            header,
            encoding: EncodingName::Ascii,
            json: true,
            total_len: Some(len),
        }
    }
}

/// The result of a new request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestInfo {
    /// The Request ID of the new transaction.
    pub request_id: u8,
    /// True when this Device does not have the ResourceList of the peer yet
    /// (REQ-PE-RES-003).
    pub resource_list_missing: bool,
}

/// Configuration of the engine.
#[derive(Debug, Clone, Copy)]
pub struct EngineConfig<'r> {
    /// The Resource table of this Device as responder.
    pub resources: &'r [ResourceEntry<'r>],
    /// The zlib window buffer of the caller: 0 when it does not inflate "zlib+Mcoded7"
    /// (REQ-PE-ENC-009, REQ-PE-STAT-011).
    pub zlib_window: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// This Device sends the Chunks of its message.
    Sending,
    /// This Device receives Chunks (the inquiry, or the reply).
    Receiving,
    /// Responder: the inquiry is complete; the caller gives the reply.
    Replying,
}

#[derive(Debug, Clone, Copy)]
struct Txn<const H: usize> {
    peer: Muid,
    request_id: u8,
    role: Direction,
    kind: TxnKind,
    phase: Phase,
    flow: bool,
    encoding: EncodingName,
    decoder: Mcoded7Decoder,
    zlib_checked: bool,
    /// The decoder state before the last good Chunk, for a resend (REQ-PE-FLOW-011).
    prev_decoder: Mcoded7Decoder,
    prev_zlib_checked: bool,
    /// Number of the last good received Chunk.
    last_good: u16,
    /// Number of the next Chunk to send.
    next: u16,
    /// Number of Chunks of the message that this Device sends; 0 when not known.
    total: u16,
    /// Flow Control: waits for the ACK of Chunk `next - 1`.
    await_ack: bool,
    retransmits: u8,
    /// The deadline of the present wait.
    deadline: u64,
    resource: Option<usize>,
    resource_list: bool,
    can_paginate: bool,
    /// Initiator of a "start": the start header is in `hdr`.
    sub_start: bool,
    subscription: Option<usize>,
    hdr: [u8; H],
    hdr_len: usize,
}

/// The fields that name a transaction: `rx` keeps them after it changes the `Txn` in its slot,
/// so it does not copy the whole `Txn` (H + 72 bytes; OPT-034).
#[derive(Clone, Copy)]
struct TxnKey {
    peer: Muid,
    request_id: u8,
    role: Direction,
    kind: TxnKind,
}

impl TxnKey {
    fn of<const H: usize>(t: &Txn<H>) -> Self {
        TxnKey {
            peer: t.peer,
            request_id: t.request_id,
            role: t.role,
            kind: t.kind,
        }
    }
}

/// Puts `p` in the first free entry of the control message queue; a full queue drops it (the
/// callers check the room first).
fn push_to(pending: &mut [Option<Pending>], p: Pending) {
    if let Some(slot) = pending.iter_mut().find(|s| s.is_none()) {
        *slot = Some(p);
    }
}

#[derive(Debug, Clone, Copy)]
enum Pending {
    Status {
        peer: Muid,
        request_id: u8,
        kind: TxnKind,
        status: Status,
        subscription: Option<usize>,
    },
    Ack {
        peer: Muid,
        request_id: u8,
        kind: TxnKind,
        status: AckStatus,
        chunk: u16,
    },
    Nak {
        peer: Muid,
        request_id: u8,
        kind: TxnKind,
        status: NakStatus,
        chunk: u16,
    },
    /// A Get for a subscription after a "notify" (REQ-PE-SUB-014, -016).
    Refresh { index: usize },
}

/// The "message" Property of a reply with a status other than 200 (REQ-PE-STAT-006).
pub fn default_message(status: Status) -> &'static str {
    match status.0 {
        201 => "Accepted",
        341 => "Resource currently unavailable",
        342 => "Bad data or unexpected end of data",
        343 => "Too many requests",
        400 => "Bad request",
        403 => "Not authorized",
        404 => "Resource not found",
        405 => "Resource not allowed",
        406 => "Flow Control not supported",
        407 => "Flow Control is required",
        413 => "Payload too large",
        415 => "Unsupported media type or encoding",
        445 => "Invalid version of data",
        _ => "Internal device error",
    }
}

/// The header of a JSON Schema request for a "$ref" value "midi+jsonschema://<id>#fragment":
/// gives the "resId" and the fragment (REQ-PE-LIST-010). Use the "resId" with
/// "resource":"JSONSchema".
pub fn schema_ref(value: &str) -> Result<(&str, Option<&str>), TransactionError> {
    let rest = value
        .strip_prefix("midi+jsonschema://")
        .ok_or(TransactionError::InvalidRef)?;
    let (id, fragment) = match rest.split_once('#') {
        Some((id, f)) => (id, Some(f)),
        None => (rest, None),
    };
    // REQ-PE-HDR-007: 1 to 36 characters, letters, digits and "_".
    if id.is_empty() || id.len() > 36 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(TransactionError::InvalidRef);
    }
    Ok((id, fragment))
}

/// The reply status for a header that the parser refuses (REQ-PE-STAT-009, -012).
fn status_of(e: HeaderError) -> Status {
    match e {
        HeaderError::Invalid("mutualEncoding") | HeaderError::Invalid("mediaType") => {
            Status::UNSUPPORTED_MEDIA
        }
        _ => Status::BAD_REQUEST,
    }
}

/// The window that a zlib CMF byte declares (RFC 1950 2.2), `None` when not valid.
const fn zlib_window(cmf: u8) -> Option<usize> {
    if cmf & 0x0F != 8 || cmf >> 4 > 7 {
        None
    } else {
        Some(1 << ((cmf >> 4) + 8))
    }
}

fn header_error(e: HeaderError) -> TransactionError {
    match e {
        HeaderError::Json(JsonError::Overflow) => TransactionError::HeaderTooLarge,
        e => TransactionError::Header(e),
    }
}

/// The Property Exchange transaction engine: `T` transactions with `H` header bytes each, `S`
/// subscriptions, `P` peers whose ResourceList is known and a queue of `Q` control messages
/// (at least 3: one received event queues up to 3). A full queue gives
/// [`TransactionError::QueueFull`] and changes nothing.
///
/// Small devices (OPT-018): a firmware that calls [`PeEngine::write_pending`] until it gives
/// `None` after each `receive`, `poll` and request call has an empty queue before each of them,
/// and one of them queues at most 3 messages, so `Q = 4` is enough. Each entry is 16 bytes on
/// `thumbv7em`, so `Q = 4` uses 192 bytes less than the default. With a smaller `Q`, a
/// `receive` before the queue is written gives `QueueFull` sooner.
///
/// In a `static`, `Option<PeEngine> = None` is not all zero bits, so the linker puts the
/// static in `.data`, and its initial image also uses flash (OPT-013). A static of
/// `core::mem::MaybeUninit` (for example a `static_cell::StaticCell`) is in `.bss` and uses no
/// flash.
#[derive(Debug)]
pub struct PeEngine<
    'r,
    const T: usize,
    const H: usize,
    const S: usize,
    const P: usize,
    const Q: usize = DEFAULT_QUEUE,
> {
    config: EngineConfig<'r>,
    txns: [Option<Txn<H>>; T],
    subs: SubscriptionTable<S>,
    lists: [Option<Muid>; P],
    pending: [Option<Pending>; Q],
}

impl<'r, const T: usize, const H: usize, const S: usize, const P: usize, const Q: usize>
    PeEngine<'r, T, H, S, P, Q>
{
    const CHECK: () = assert!(Q >= 3, "the control message queue needs 3 entries or more");

    /// A new engine.
    pub fn new(config: EngineConfig<'r>) -> Self {
        let () = Self::CHECK;
        PeEngine {
            config,
            txns: [None; T],
            subs: SubscriptionTable::new(),
            lists: [None; P],
            pending: [None; Q],
        }
    }

    /// The ResourceList writer of this Device's Resource table, with an entry buffer of `E`
    /// bytes. With `S` = 0 every entry has "canSubscribe" false (REQ-PE-SUB-022).
    pub fn resource_list<const E: usize>(&self) -> ResourceListWriter<'r, E> {
        let w = ResourceListWriter::new(self.config.resources);
        if S == 0 {
            w.without_subscriptions()
        } else {
            w
        }
    }

    /// The subscription table.
    pub fn subscriptions(&self) -> &SubscriptionTable<S> {
        &self.subs
    }

    /// True when this Device has the ResourceList of `peer` (REQ-PE-RES-003).
    pub fn has_resource_list(&self, peer: Muid) -> bool {
        self.lists.contains(&Some(peer))
    }

    /// Number of open transactions.
    pub fn open(&self) -> usize {
        self.txns.iter().flatten().count()
    }

    /// The next subscription that an Invalidate MUID removed (REQ-PE-SUB-019).
    pub fn take_ended(&mut self) -> Option<SubscriptionEntry> {
        self.subs.take_ended()
    }

    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    fn find(&self, peer: Muid, request_id: u8, role: Direction) -> Option<usize> {
        self.txns.iter().position(
            |t| matches!(t, Some(t) if t.peer == peer && t.request_id == request_id && t.role == role),
        )
    }

    /// The transaction that an ACK, a NAK or a Notify from `peer` is for (REQ-PE-END-001).
    /// `orig` is the Original Transaction Sub-ID#2 (`None` for a Notify). `role` is the role of
    /// this Device when the message gives it; with no role, an initiator transaction comes
    /// first. Both Devices can use the same Request ID for their own requests.
    fn find_for(
        &self,
        peer: Muid,
        request_id: u8,
        orig: Option<u8>,
        role: Option<Direction>,
    ) -> Option<usize> {
        let pick = |r: Direction| {
            self.txns.iter().position(|t| {
                matches!(t, Some(t) if t.peer == peer
                    && t.request_id == request_id
                    && t.role == r
                    && orig.is_none_or(|o| t.kind.inquiry().sub_id2() == o))
            })
        };
        match role {
            Some(r) => pick(r),
            None => pick(Direction::Initiator).or_else(|| pick(Direction::Responder)),
        }
    }

    /// The role of the sender of the Property Data of a transaction with this Original
    /// Transaction Sub-ID#2: the responder of a Get, the initiator of a Set or a Subscription.
    fn data_sender(orig: u8) -> Direction {
        if orig == TxnKind::Get.inquiry().sub_id2() {
            Direction::Responder
        } else {
            Direction::Initiator
        }
    }

    fn free_pending(&self) -> usize {
        self.pending.iter().filter(|p| p.is_none()).count()
    }

    /// `QueueFull` when the control message queue has no free entry: a caller request that
    /// queues a message then changes nothing (no message is lost).
    fn queue_room(&self) -> Result<(), TransactionError> {
        if self.free_pending() == 0 {
            return Err(TransactionError::QueueFull);
        }
        Ok(())
    }

    fn free_slot(&self) -> Option<usize> {
        self.txns.iter().position(Option::is_none)
    }

    fn push(&mut self, p: Pending) {
        push_to(&mut self.pending, p);
    }

    /// Frees slot `i` in place and ends its request in the device. Gives the key only: a moved
    /// `Txn<H>` costs H + 72 bytes in the frame of each caller (GitHub issue #1).
    fn close<D: PeHost>(&mut self, host: &mut D, i: usize) -> Option<TxnKey> {
        let slot = self.txns.get_mut(i)?;
        let t = TxnKey::of(slot.as_ref()?);
        *slot = None;
        host.end(t.peer, t.request_id, t.role);
        Some(t)
    }

    fn closed<D: PeHost>(
        &mut self,
        host: &mut D,
        i: usize,
        reason: CloseReason,
    ) -> Option<PeEvent<'static, 'static>> {
        self.close(host, i).map(|t| PeEvent::Closed {
            peer: t.peer,
            request_id: t.request_id,
            role: t.role,
            reason,
        })
    }

    /// A status reply to an inquiry with no transaction slot.
    fn refuse<D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        request_id: u8,
        kind: TxnKind,
        status: Status,
    ) {
        self.push(Pending::Status {
            peer,
            request_id,
            kind,
            status,
            subscription: None,
        });
        host.end(peer, request_id, Direction::Responder);
    }

    fn nak(&mut self, t: &Txn<H>, status: NakStatus, chunk: u16) {
        self.push(Pending::Nak {
            peer: t.peer,
            request_id: t.request_id,
            kind: t.kind,
            status,
            chunk,
        });
    }

    fn terminate(&mut self, t: &Txn<H>) {
        self.nak(
            t,
            NakStatus::Terminate {
                by_initiator: t.role == Direction::Initiator,
            },
            0,
        );
    }

    /// Handles one zmidi-ci Device event: Property Exchange Chunks, ACK, NAK, Invalidate MUID
    /// and a new MUID. Other events give `None`. `scratch` takes the decoded Property Data of
    /// a Chunk; it must be at least as long as the Property Data of the Chunk.
    pub fn receive<'m, 's, D: PeHost>(
        &mut self,
        host: &mut D,
        event: &Event<'m>,
        now: u64,
        scratch: &'s mut [u8],
    ) -> Result<Option<PeEvent<'m, 's>>, TransactionError> {
        // The checks come first, so that an error changes nothing. One event queues at most 3
        // control messages.
        if let Event::PeChunk { chunk, .. } = event {
            if scratch.len() < chunk.data.len() {
                return Err(TransactionError::Size);
            }
        }
        if matches!(
            event,
            Event::PeChunk { .. } | Event::Ack { .. } | Event::Nak { .. }
        ) && self.free_pending() < 3
        {
            return Err(TransactionError::QueueFull);
        }
        match *event {
            Event::PeChunk { muid, kind, chunk } => match TxnKind::of(kind) {
                Some((k, true)) => self.on_inquiry(host, muid, k, chunk, now, scratch),
                Some((k, false)) => self.on_reply(host, muid, k, chunk, now, scratch),
                None => Ok(self.on_notify(host, muid, chunk)),
            },
            Event::Ack { muid, ack, .. } => Ok(self.on_ack(muid, &ack, now)),
            Event::Nak { muid, nak, .. } => Ok(self.on_nak(host, muid, &nak)),
            Event::PeerInvalidated { muid } => Ok(Some(self.invalidate(host, muid, false))),
            Event::MuidChanged { old, .. } => Ok(Some(self.invalidate(host, old, true))),
            _ => Ok(None),
        }
    }

    /// Closes the transactions and removes the subscriptions of `muid` (a peer), or of this
    /// Device (`own`), with no message (REQ-PE-SUB-019).
    fn invalidate<D: PeHost>(
        &mut self,
        host: &mut D,
        muid: Muid,
        own: bool,
    ) -> PeEvent<'static, 'static> {
        for i in 0..T {
            if self.txns[i].is_some_and(|t| own || t.peer == muid) {
                self.close(host, i);
            }
        }
        // The ended entries wait in their places for `take_ended` (OPT-004).
        let ended = if own {
            self.subs.end_all()
        } else {
            self.subs.end_peer(muid)
        };
        if !own {
            for l in self.lists.iter_mut() {
                if *l == Some(muid) {
                    *l = None;
                }
            }
        } else {
            self.lists = [None; P];
        }
        PeEvent::Invalidated { muid, ended }
    }

    fn on_inquiry<'m, 's, D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        kind: TxnKind,
        chunk: PeChunk<'m>,
        now: u64,
        scratch: &'s mut [u8],
    ) -> Result<Option<PeEvent<'m, 's>>, TransactionError> {
        let rid = chunk.request_id;
        if let Some(i) = self.find(peer, rid, Direction::Responder) {
            if self.txns[i].is_some_and(|t| t.phase == Phase::Receiving && t.kind == kind) {
                return self.rx(host, i, chunk, None, now, scratch);
            }
            return Ok(None);
        }
        if chunk.number != 1 {
            // A later Chunk (or a final Chunk 0x0000) of a transaction that is closed, for
            // example after an early reply (REQ-PE-FLOW-007, -008): ignore it.
            host.end(peer, rid, Direction::Responder);
            return Ok(None);
        }
        if chunk.header.len() > H {
            self.refuse(host, peer, rid, kind, Status::PAYLOAD_TOO_LARGE); // REQ-PE-STAT-011
            return Ok(None);
        }
        let header = match Header::parse(kind.header_kind(), chunk.header) {
            Ok(h) => h,
            Err(e) => {
                self.refuse(host, peer, rid, kind, status_of(e)); // REQ-PE-STAT-009, -012
                return Ok(None);
            }
        };
        if kind == TxnKind::Subscription {
            return self.on_subscription(host, peer, header, chunk, now, scratch);
        }
        let resources = self.config.resources;
        let res = header.resource.unwrap_or(HeaderStr::Text(""));
        let index = resources.iter().position(|e| res.eq_str(e.resource));
        let list_get = kind == TxnKind::Get && res.eq_str("ResourceList");
        let mut can_paginate = false;
        match index {
            // REQ-PE-GEN-001: never 404 or 405 for ResourceList.
            None if list_get => {}
            None => {
                self.refuse(host, peer, rid, kind, Status::NOT_FOUND); // REQ-PE-STAT-010
                return Ok(None);
            }
            Some(ix) => {
                let e = &resources[ix];
                can_paginate = e.caps.can_paginate;
                let op = if kind == TxnKind::Get {
                    Operation::Get
                } else {
                    Operation::Set
                };
                let mut status = if list_get {
                    Ok(())
                } else {
                    e.caps.check(op, &header)
                };
                if status.is_ok() && kind == TxnKind::Set && !Self::media_ok(e, &header) {
                    status = Err(Status::UNSUPPORTED_MEDIA); // REQ-PE-STAT-012
                }
                if let Err(s) = status {
                    // REQ-PE-FLOW-007, -008: the reply after Chunk 1; later Chunks are
                    // ignored.
                    self.refuse(host, peer, rid, kind, s);
                    return Ok(None);
                }
            }
        }
        // A Device that does not inflate can not take zlib data; a Get asks for a zlib reply,
        // which the Resource "encodings" permit or refuse (415 above).
        if kind == TxnKind::Set
            && header.encoding() == EncodingName::ZlibMcoded7
            && self.config.zlib_window == 0
        {
            self.refuse(host, peer, rid, kind, Status::UNSUPPORTED_MEDIA);
            return Ok(None);
        }
        let Some(slot) = self.free_slot() else {
            self.refuse(host, peer, rid, kind, Status::TOO_MANY_REQUESTS); // REQ-PE-GEN-004
            return Ok(None);
        };
        self.txns[slot] = Some(Txn {
            can_paginate,
            resource: index,
            ..self.new_txn(
                peer,
                rid,
                Direction::Responder,
                kind,
                Phase::Receiving,
                &header,
            )
        });
        self.rx(host, slot, chunk, Some(header), now, scratch)
    }

    fn media_ok(e: &ResourceEntry<'_>, header: &Header<'_>) -> bool {
        match &header.media_type {
            None => true,
            Some(m) if e.media_types.is_empty() => m.eq_str("application/json"),
            Some(m) => e.media_types.iter().any(|t| m.eq_ignore_case(t)),
        }
    }

    fn new_txn(
        &self,
        peer: Muid,
        request_id: u8,
        role: Direction,
        kind: TxnKind,
        phase: Phase,
        header: &Header<'_>,
    ) -> Txn<H> {
        Txn {
            peer,
            request_id,
            role,
            kind,
            phase,
            flow: header.flow_control,
            encoding: header.encoding(),
            decoder: Mcoded7Decoder::new(),
            zlib_checked: false,
            prev_decoder: Mcoded7Decoder::new(),
            prev_zlib_checked: false,
            last_good: 0,
            next: 1,
            total: 0,
            await_ack: false,
            retransmits: 0,
            deadline: 0,
            resource: None,
            resource_list: false,
            can_paginate: false,
            sub_start: false,
            subscription: None,
            hdr: [0; H],
            hdr_len: 0,
        }
    }

    fn on_subscription<'m, 's, D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        header: Header<'m>,
        chunk: PeChunk<'m>,
        now: u64,
        scratch: &'s mut [u8],
    ) -> Result<Option<PeEvent<'m, 's>>, TransactionError> {
        let rid = chunk.request_id;
        let kind = TxnKind::Subscription;
        let ok = |index| Pending::Status {
            peer,
            request_id: rid,
            kind,
            status: Status::OK,
            subscription: index,
        };
        match header.command {
            Some(Command::Start) => {
                let res = header.resource.unwrap_or(HeaderStr::Text(""));
                let Some(e) = self
                    .config
                    .resources
                    .iter()
                    .find(|e| res.eq_str(e.resource))
                else {
                    self.refuse(host, peer, rid, kind, Status::NOT_FOUND); // REQ-PE-STAT-010
                    return Ok(None);
                };
                let caps = e.caps;
                match self.subs.accept_start(peer, &header, &caps) {
                    Ok(index) => {
                        self.push(ok(Some(index)));
                        Ok(Some(PeEvent::SubscriptionStarted { index }))
                    }
                    Err(s) => {
                        self.refuse(host, peer, rid, kind, s); // REQ-PE-SUB-007
                        Ok(None)
                    }
                }
            }
            _ => match self.subs.receive(peer, &header) {
                // REQ-PE-SUB-015, -017: an "end" gets 200; its Property Data is ignored.
                Ok(SubscriptionInput::Ended(entry)) => {
                    self.push(ok(None));
                    Ok(Some(PeEvent::SubscriptionEnded(entry)))
                }
                Ok(SubscriptionInput::EndUnknown) => {
                    self.push(ok(None));
                    Ok(None)
                }
                Ok(SubscriptionInput::Update {
                    index,
                    command: Command::Notify,
                }) => {
                    // REQ-PE-SUB-013, -014: the data is ignored; a Get follows.
                    self.push(ok(None));
                    self.push(Pending::Refresh { index });
                    Ok(Some(PeEvent::Message(Message {
                        peer,
                        request_id: rid,
                        role: Direction::Responder,
                        kind,
                        header: Some(header),
                        resource: None,
                        encoding: header.encoding(),
                        data: &[],
                        end: ChunkEnd::Complete,
                        subscription: Some(index),
                        started: None,
                    })))
                }
                Ok(SubscriptionInput::Update { index, .. }) => {
                    let Some(slot) = self.free_slot() else {
                        self.refuse(host, peer, rid, kind, Status::TOO_MANY_REQUESTS);
                        return Ok(None);
                    };
                    self.txns[slot] = Some(Txn {
                        subscription: Some(index),
                        ..self.new_txn(
                            peer,
                            rid,
                            Direction::Responder,
                            kind,
                            Phase::Receiving,
                            &header,
                        )
                    });
                    self.rx(host, slot, chunk, Some(header), now, scratch)
                }
                Err(s) => {
                    self.refuse(host, peer, rid, kind, s); // REQ-PE-SUB-015
                    Ok(None)
                }
            },
        }
    }

    fn on_reply<'m, 's, D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        kind: TxnKind,
        chunk: PeChunk<'m>,
        now: u64,
        scratch: &'s mut [u8],
    ) -> Result<Option<PeEvent<'m, 's>>, TransactionError> {
        let Some(i) = self.find(peer, chunk.request_id, Direction::Initiator) else {
            return Ok(None);
        };
        // The fields are read in the slot: a copy of `Txn<H>` costs H + 72 bytes of stack on each
        // reply path (GitHub issue #1).
        let Some(t) = self.txns[i].as_ref() else {
            return Ok(None);
        };
        if t.kind != kind {
            return Ok(None);
        }
        let (sub_start, resource_list) = (t.sub_start, t.resource_list);
        let first = t.last_good == 0 && chunk.number <= 1;
        if t.phase == Phase::Sending {
            if !first {
                return Ok(None);
            }
            // REQ-PE-FLOW-008: a reply before the last Chunk stops the transmission.
            if let Some(t) = self.txns[i].as_mut() {
                t.phase = Phase::Receiving;
                t.await_ack = false;
            }
        }
        if !first {
            return self.rx(host, i, chunk, None, now, scratch);
        }
        let header = match Header::parse(HeaderKind::Reply, chunk.header) {
            Ok(h) => h,
            Err(e) => return Ok(self.closed(host, i, CloseReason::BadReply(e))),
        };
        let class = header.status.map_or(StatusClass::Unknown, Status::class);
        let started = sub_start.then(|| self.start_reply(i, peer, &header, class));
        if resource_list && class == StatusClass::Success {
            self.mark_list(peer);
        }
        if let Some(t) = self.txns[i].as_mut() {
            t.encoding = header.encoding();
        }
        let mut ev = self.rx(host, i, chunk, Some(header), now, scratch)?;
        // REQ-PE-STAT-004, REQ-PE-GEN-004: a reply that is not a success ends the transaction.
        if class != StatusClass::Success && self.txns[i].is_some() {
            self.close(host, i);
        }
        if let Some(PeEvent::Message(m)) = ev.as_mut() {
            m.started = started;
        }
        Ok(ev)
    }

    /// The reply to a Subscription "start" of this Device (the subscriber): the new entry
    /// (REQ-PE-SUB-*). Not inlined: only a subscriber runs it, and its second `Header` (the
    /// stored "start") would add to the stack of each reply path (GitHub issue #1).
    #[inline(never)]
    fn start_reply(
        &mut self,
        i: usize,
        peer: Muid,
        header: &Header<'_>,
        class: StatusClass,
    ) -> Result<usize, SubscriptionError> {
        if class != StatusClass::Success {
            return Err(SubscriptionError::NotSuccess);
        }
        let Some(t) = self.txns[i].as_ref() else {
            return Err(SubscriptionError::Invalid);
        };
        match Header::parse(HeaderKind::Subscription, &t.hdr[..t.hdr_len]) {
            Ok(start) => self.subs.add_started(peer, &start, header),
            Err(_) => Err(SubscriptionError::Invalid),
        }
    }

    fn mark_list(&mut self, peer: Muid) {
        if self.has_resource_list(peer) {
            return;
        }
        if let Some(slot) = self.lists.iter_mut().find(|s| s.is_none()) {
            *slot = Some(peer);
        } else if P > 0 {
            self.lists[0] = Some(peer);
        }
    }

    /// One received Chunk of an open transaction: sequence, decode, Flow Control ACK, end.
    fn rx<'m, 's, D: PeHost>(
        &mut self,
        host: &mut D,
        i: usize,
        chunk: PeChunk<'m>,
        header: Option<Header<'m>>,
        now: u64,
        scratch: &'s mut [u8],
    ) -> Result<Option<PeEvent<'m, 's>>, TransactionError> {
        let Some(cur) = self.txns[i].as_ref() else {
            return Ok(None);
        };
        let expected = cur.last_good + 1;
        if chunk.number != 0 && chunk.number != expected {
            let t = *cur;
            if t.flow {
                if chunk.number == t.last_good {
                    // REQ-PE-FLOW-015: the same Chunk again: ACK it again.
                    self.push(Pending::Ack {
                        peer: t.peer,
                        request_id: t.request_id,
                        kind: t.kind,
                        status: AckStatus::FlowControlSendNext,
                        chunk: t.last_good,
                    });
                } else if chunk.number > expected {
                    self.nak(&t, NakStatus::FlowControlResend, t.last_good); // REQ-PE-FLOW-011
                }
                return Ok(None);
            }
            // REQ-PE-STAT-008: NAK 0x21 at once and close (DEC-PE-002).
            self.nak(&t, NakStatus::ChunksOutOfSequence, chunk.number);
            return Ok(self.closed(host, i, CloseReason::OutOfSequence));
        }
        if scratch.len() < chunk.data.len() {
            return Err(TransactionError::Size);
        }
        // The transaction is changed in its slot: a copy of `Txn<H>` costs H + 72 bytes twice.
        let zlib_max = self.config.zlib_window;
        let Some(t) = self.txns[i].as_mut() else {
            return Ok(None);
        };
        let (prev_decoder, prev_zlib) = (t.decoder, t.zlib_checked);
        let n = match t.encoding {
            EncodingName::Ascii => {
                scratch[..chunk.data.len()].copy_from_slice(chunk.data);
                Some(chunk.data.len())
            }
            _ => t.decoder.decode(chunk.data, scratch).ok(),
        };
        let mut bad = n.is_none();
        let n = n.unwrap_or(0);
        if t.encoding == EncodingName::ZlibMcoded7 && !t.zlib_checked && n > 0 {
            match zlib_window(scratch[0]) {
                Some(w) if w <= zlib_max => t.zlib_checked = true,
                Some(_) => return Ok(self.bad_data(host, i, Status::PAYLOAD_TOO_LARGE)),
                None => bad = true,
            }
        }
        if bad {
            return Ok(self.bad_data(host, i, Status::BAD_DATA)); // REQ-PE-STAT-008
        }
        let end = chunk.end();
        let last = end != ChunkEnd::More;
        if chunk.number != 0 {
            t.last_good = chunk.number;
        }
        t.prev_decoder = prev_decoder;
        t.prev_zlib_checked = prev_zlib;
        // A wait of an ACK 0x10 counts until the next Chunk (REQ-PE-END-008, -009).
        t.deadline = now + TIMEOUT_MS;
        // REQ-PE-FLOW-003, -004, -009: an ACK 0x11 for each Chunk of the Property Data stream
        // (the reply of a Get; the inquiry of a Set or a Subscription), except the last Chunk
        // of a Set, which the reply answers.
        let stream = (t.kind == TxnKind::Get) == (t.role == Direction::Initiator);
        let set_end = last && t.kind != TxnKind::Get;
        if t.flow && chunk.number != 0 && stream && !set_end {
            push_to(
                &mut self.pending,
                Pending::Ack {
                    peer: t.peer,
                    request_id: t.request_id,
                    kind: t.kind,
                    status: AckStatus::FlowControlSendNext,
                    chunk: chunk.number,
                },
            );
            if !last {
                t.deadline = now + FLOW_RECEIVER_WAIT_MS; // REQ-PE-FLOW-014
            }
        }
        let msg = Message {
            peer: t.peer,
            request_id: t.request_id,
            role: t.role,
            kind: t.kind,
            header,
            resource: t.resource,
            encoding: t.encoding,
            data: &scratch[..n],
            end,
            subscription: t.subscription,
            started: None,
        };
        let t = TxnKey::of(t);
        if last {
            match (t.role, t.kind) {
                (Direction::Initiator, _) => {
                    self.close(host, i);
                }
                (Direction::Responder, TxnKind::Subscription) => {
                    // REQ-PE-SUB-015, -016: 200, also when the caller can not apply it.
                    let status = if end == ChunkEnd::Incomplete {
                        Status::BAD_DATA
                    } else {
                        Status::OK
                    };
                    self.push(Pending::Status {
                        peer: t.peer,
                        request_id: t.request_id,
                        kind: t.kind,
                        status,
                        subscription: None,
                    });
                    self.close(host, i);
                }
                (Direction::Responder, TxnKind::Set) if end == ChunkEnd::Incomplete => {
                    // REQ-PE-STAT-008.
                    self.push(Pending::Status {
                        peer: t.peer,
                        request_id: t.request_id,
                        kind: t.kind,
                        status: Status::BAD_DATA,
                        subscription: None,
                    });
                    self.close(host, i);
                }
                (Direction::Responder, _) => {
                    if let Some(t) = self.txns[i].as_mut() {
                        t.phase = Phase::Replying; // REQ-PE-END-008
                    }
                }
            }
        }
        Ok(Some(PeEvent::Message(msg)))
    }

    /// Property Data that can not be decoded (REQ-PE-STAT-008, -011).
    fn bad_data<D: PeHost>(
        &mut self,
        host: &mut D,
        i: usize,
        status: Status,
    ) -> Option<PeEvent<'static, 'static>> {
        let t = self.txns[i]?;
        if t.role == Direction::Responder {
            self.push(Pending::Status {
                peer: t.peer,
                request_id: t.request_id,
                kind: t.kind,
                status,
                subscription: None,
            });
            self.closed(host, i, CloseReason::Refused(status))
        } else {
            self.nak(&t, NakStatus::Terminate { by_initiator: true }, 0); // REQ-PE-STAT-013
            self.closed(host, i, CloseReason::Refused(status))
        }
    }

    fn on_notify<D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        chunk: PeChunk<'_>,
    ) -> Option<PeEvent<'static, 'static>> {
        let header = Header::parse(HeaderKind::Reply, chunk.header).ok()?;
        let i = self.find_for(peer, chunk.request_id, None, None)?;
        match header.status?.0 {
            100 => {
                if let Some(t) = self.txns[i].as_mut() {
                    t.deadline += NOTIFY_WAIT_MS; // REQ-PE-END-012
                }
                None
            }
            144 => self.closed(host, i, CloseReason::Terminated), // REQ-PE-END-006
            408 => self.closed(host, i, CloseReason::PeerTimeout), // REQ-PE-END-015
            _ => None,
        }
    }

    fn pe_details(details: Details) -> Option<(u8, u16)> {
        match details {
            Details::PropertyExchange { request_id, chunk } => Some((request_id, chunk)),
            _ => None,
        }
    }

    fn on_ack(&mut self, peer: Muid, ack: &Ack<'_>, now: u64) -> Option<PeEvent<'static, 'static>> {
        let (rid, chunk) = Self::pe_details(ack.details)?; // REQ-PE-END-001
        match ack.status {
            AckStatus::TimeoutWait(w) => {
                // REQ-PE-END-011, REQ-PE-FLOW-014.
                let i = self.find_for(peer, rid, Some(ack.original_sub_id2), None)?;
                if let Some(t) = self.txns[i].as_mut() {
                    t.deadline += u64::from(w) * 100;
                }
                None
            }
            AckStatus::FlowControlSendNext => {
                // REQ-PE-FLOW-002: the ACK of the last Chunk sent; any other number is ignored.
                let role = Self::data_sender(ack.original_sub_id2);
                let i = self.find_for(peer, rid, Some(ack.original_sub_id2), Some(role))?;
                let t = self.txns[i]?;
                if t.phase != Phase::Sending || !t.await_ack || chunk + 1 != t.next {
                    return None;
                }
                let t = self.txns[i].as_mut()?;
                t.await_ack = false;
                t.retransmits = 0;
                t.deadline = now + TIMEOUT_MS; // REQ-PE-FLOW-014
                Some(PeEvent::Ready {
                    peer,
                    request_id: rid,
                    role: t.role,
                })
            }
            _ => None,
        }
    }

    fn on_nak<D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        nak: &Nak<'_>,
    ) -> Option<PeEvent<'static, 'static>> {
        if nak.original_sub_id2 == SUB_ID2_ACK {
            return None; // REQ-PE-END-016
        }
        let (rid, chunk) = Self::pe_details(nak.details)?; // REQ-PE-END-001
        let orig = nak.original_sub_id2;
        // The role of this Device, where the NAK gives it.
        let role = match nak.status {
            // NAK 0x20 Status Data 0x01 and NAK 0x42 come from the initiator.
            NakStatus::Terminate { by_initiator: true } | NakStatus::Timeout => {
                Some(Direction::Responder)
            }
            NakStatus::Terminate {
                by_initiator: false,
            } => Some(Direction::Initiator),
            NakStatus::FlowControlResend => Some(Self::data_sender(orig)),
            _ => None,
        };
        let i = self.find_for(peer, rid, Some(orig), role)?;
        let t = self.txns[i]?;
        // A Set initiator after its last Chunk can still send that Chunk again.
        let resend_last = t.phase == Phase::Receiving
            && t.role == Direction::Initiator
            && t.kind != TxnKind::Get
            && t.last_good == 0;
        match nak.status {
            NakStatus::Terminate { .. } => self.closed(host, i, CloseReason::Terminated),
            NakStatus::Timeout => self.closed(host, i, CloseReason::PeerTimeout),
            NakStatus::FlowControlResend
                if t.flow && (t.phase == Phase::Sending || resend_last) =>
            {
                // REQ-PE-FLOW-012: only the last Chunk sent can be sent again.
                if chunk + 2 == t.next {
                    let t = self.txns[i].as_mut()?;
                    t.next = chunk + 1;
                    t.await_ack = false;
                    t.phase = Phase::Sending;
                    Some(PeEvent::Resend {
                        peer,
                        request_id: rid,
                        role: t.role,
                        chunk: chunk + 1,
                    })
                } else {
                    self.terminate(&t);
                    self.closed(host, i, CloseReason::ResendFailed)
                }
            }
            NakStatus::FlowControlResend => None,
            status => self.closed(host, i, CloseReason::Nak(status)),
        }
    }

    /// Handles the deadlines at clock time `now`; call it often. Gives one event for each
    /// call; call again until it gives `None`.
    pub fn poll<D: PeHost>(&mut self, host: &mut D, now: u64) -> Option<PeEvent<'static, 'static>> {
        if self.free_pending() == 0 {
            return None; // write the pending messages first
        }
        let i =
            (0..T).find(|&i| self.txns[i].is_some_and(|t| t.deadline != 0 && now > t.deadline))?;
        let t = self.txns[i]?;
        match t.phase {
            Phase::Sending if t.flow && t.await_ack && t.retransmits < RETRANSMITS => {
                // REQ-PE-FLOW-013: send the Chunk again.
                let t = self.txns[i].as_mut()?;
                t.retransmits += 1;
                t.next -= 1;
                t.await_ack = false;
                t.deadline = now + TIMEOUT_MS;
                Some(PeEvent::Resend {
                    peer: t.peer,
                    request_id: t.request_id,
                    role: t.role,
                    chunk: t.next,
                })
            }
            // Nothing was sent: the peer does not know the transaction.
            Phase::Sending if t.next == 1 && t.role == Direction::Initiator => {
                self.closed(host, i, CloseReason::Timeout)
            }
            Phase::Sending => {
                // REQ-PE-FLOW-013, REQ-PE-END-009: the transaction ends with NAK 0x20.
                self.terminate(&t);
                self.closed(host, i, CloseReason::Timeout)
            }
            Phase::Receiving if t.role == Direction::Initiator => {
                // REQ-PE-END-013, -014, REQ-PE-GEN-005.
                self.nak(&t, NakStatus::Timeout, 0);
                self.closed(host, i, CloseReason::Timeout)
            }
            Phase::Receiving => {
                // REQ-PE-END-009, REQ-PE-FLOW-014.
                self.terminate(&t);
                self.closed(host, i, CloseReason::Timeout)
            }
            // REQ-PE-END-008: no reply is sent after the deadline.
            Phase::Replying => self.closed(host, i, CloseReason::Timeout),
        }
    }

    /// Writes the next pending control message (status replies, ACK, NAK, a Get after a
    /// "notify") into `out`. Call it until it gives `None`; send each message.
    pub fn write_pending<D: PeHost>(
        &mut self,
        host: &mut D,
        now: u64,
        out: &mut [u8],
    ) -> Option<Result<usize, TransactionError>> {
        // A Get after a "notify" waits in the queue while the peer has its maximum of open
        // requests or no slot is free (REQ-PE-SUB-014).
        let k = (0..Q).find(|&k| match self.pending[k] {
            None => false,
            Some(Pending::Refresh { index }) => self.can_refresh(host, index),
            Some(_) => true,
        })?;
        let p = self.pending[k].take()?;
        Some(self.write_one(host, p, now, out))
    }

    fn can_refresh<D: PeHost>(&self, host: &D, index: usize) -> bool {
        let Some(e) = self.subs.get(index) else {
            return true; // write_one gives the error
        };
        let limit = host.peer_simultaneous(e.peer()).unwrap_or(1).max(1) as usize;
        let open = self
            .txns
            .iter()
            .flatten()
            .filter(|t| t.peer == e.peer() && t.role == Direction::Initiator)
            .count();
        self.free_slot().is_some() && open < limit
    }

    fn write_one<D: PeHost>(
        &mut self,
        host: &mut D,
        p: Pending,
        now: u64,
        out: &mut [u8],
    ) -> Result<usize, TransactionError> {
        match p {
            Pending::Status {
                peer,
                request_id,
                kind,
                status,
                subscription,
            } => {
                let id = subscription.and_then(|i| self.subs.get(i));
                let header = Header {
                    status: Some(status),
                    message: (status != Status::OK)
                        .then(|| HeaderStr::Text(default_message(status))),
                    subscribe_id: id.map(|e| HeaderStr::Text(e.subscribe_id())),
                    ..Default::default()
                };
                let mut buf = [0u8; 160];
                let n = header.write(HeaderKind::Reply, &mut buf)?;
                Ok(PeChunk {
                    request_id,
                    header: &buf[..n],
                    total: 1,
                    number: 1,
                    data: &[],
                }
                .write(kind.reply(), host.muid(), peer, host.framing(), out)?)
            }
            Pending::Ack {
                peer,
                request_id,
                kind,
                status,
                chunk,
            } => Ok(host.write_ack(
                peer,
                &Ack {
                    original_sub_id2: kind.inquiry().sub_id2(),
                    status,
                    details: Details::PropertyExchange { request_id, chunk },
                    text: &[],
                },
                out,
            )?),
            Pending::Nak {
                peer,
                request_id,
                kind,
                status,
                chunk,
            } => Ok(host.write_nak(
                peer,
                &Nak {
                    original_sub_id2: kind.inquiry().sub_id2(),
                    status,
                    details: Details::PropertyExchange { request_id, chunk },
                    text: &[],
                },
                out,
            )?),
            Pending::Refresh { index } => {
                let entry = *self
                    .subs
                    .get(index)
                    .ok_or(TransactionError::UnknownTransaction)?;
                let msg = Outgoing {
                    header: entry.get_header(),
                    encoding: entry.get_header().encoding(),
                    json: true,
                    total_len: Some(0),
                };
                let info = self.request(host, entry.peer(), TxnKind::Get, &msg, None, now)?;
                if let Some(i) = self.find(entry.peer(), info.request_id, Direction::Initiator) {
                    if let Some(t) = self.txns[i].as_mut() {
                        t.subscription = Some(index);
                    }
                }
                self.send(
                    host,
                    entry.peer(),
                    info.request_id,
                    Direction::Initiator,
                    &[],
                    true,
                    now,
                    out,
                )
            }
        }
    }

    /// Starts a new inquiry to `peer` (REQ-PE-GEN-003, REQ-PE-RES-003). `caps` are the
    /// capabilities of the Resource in the ResourceList of the peer, when known: a request
    /// that they do not permit is refused, and "flowControl":true is set where they give
    /// "fcOnSet":true (REQ-PE-FLOW-008). Send the Chunks with [`Self::send`].
    pub fn request<D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        kind: TxnKind,
        msg: &Outgoing<'_>,
        caps: Option<&ResourceCaps>,
        now: u64,
    ) -> Result<RequestInfo, TransactionError> {
        let mut header = msg.header;
        let has_data = msg.total_len.is_some_and(|n| n > 0);
        let op = match kind {
            TxnKind::Get => Some(Operation::Get),
            TxnKind::Set => Some(Operation::Set),
            TxnKind::Subscription => match header.command {
                Some(Command::Start) => {
                    check_send(Role::Initiator, Command::Start, has_data)?;
                    self.subs.can_start()?;
                    Some(Operation::Subscribe)
                }
                Some(c @ (Command::Partial | Command::Full | Command::Notify)) => {
                    check_send(Role::Responder, c, has_data)?;
                    None
                }
                Some(Command::End) if has_data => {
                    return Err(SubscriptionError::NotPermitted.into())
                }
                _ => None,
            },
        };
        if let Some(c) = caps {
            if kind == TxnKind::Set && c.fc_on_set {
                header.flow_control = true; // REQ-PE-FLOW-008
            }
            if let Some(op) = op {
                c.check(op, &header).map_err(TransactionError::Refused)?;
            }
        }
        if !msg.json && header.media_type.is_none() {
            return Err(TransactionError::NoMediaType); // REQ-PE-ENC-002
        }
        if msg.encoding != EncodingName::Ascii {
            header.mutual_encoding = Some(msg.encoding); // REQ-PE-ENC-004, REQ-PE-HDR-008
        }
        // REQ-PE-GEN-003.
        let limit = host
            .peer_simultaneous(peer)
            .ok_or(DeviceError::NotInquired)?
            .max(1) as usize;
        let open = self
            .txns
            .iter()
            .flatten()
            .filter(|t| t.peer == peer && t.role == Direction::Initiator)
            .count();
        if open >= limit {
            return Err(TransactionError::PeerBusy);
        }
        let slot = self.free_slot().ok_or(TransactionError::Full)?;
        let mut t = self.new_txn(peer, 0, Direction::Initiator, kind, Phase::Sending, &header);
        t.hdr_len = header
            .write(kind.header_kind(), &mut t.hdr)
            .map_err(header_error)?;
        t.total = self.total_chunks(host, peer, t.hdr_len, msg.total_len)?;
        t.request_id = host.allocate(peer, kind.inquiry())?;
        let res = header.resource.unwrap_or(HeaderStr::Text(""));
        t.resource_list = kind == TxnKind::Get && res.eq_str("ResourceList");
        t.sub_start = kind == TxnKind::Subscription && header.command == Some(Command::Start);
        t.deadline = now + TIMEOUT_MS;
        let info = RequestInfo {
            request_id: t.request_id,
            resource_list_missing: !t.resource_list && !self.has_resource_list(peer),
        };
        self.txns[slot] = Some(t);
        Ok(info)
    }

    /// Starts the request for the ResourceList of `peer` and writes it (REQ-PE-LIST-002,
    /// REQ-PE-RES-003).
    pub fn request_resource_list<D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        now: u64,
        out: &mut [u8],
    ) -> Result<(u8, usize), TransactionError> {
        let header = Header {
            resource: Some(HeaderStr::Text("ResourceList")),
            ..Default::default()
        };
        let info = self.request(
            host,
            peer,
            TxnKind::Get,
            &Outgoing::json(header, 0),
            None,
            now,
        )?;
        let n = self.send(
            host,
            peer,
            info.request_id,
            Direction::Initiator,
            &[],
            true,
            now,
            out,
        )?;
        Ok((info.request_id, n))
    }

    /// Starts a "start" Subscription message with the request Properties of `get`
    /// (REQ-PE-SUB-006). Send it with [`Self::send`] and no data.
    pub fn subscribe<D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        get: &Header<'_>,
        caps: Option<&ResourceCaps>,
        now: u64,
    ) -> Result<RequestInfo, TransactionError> {
        let msg = Outgoing::json(start_header(get), 0);
        self.request(host, peer, TxnKind::Subscription, &msg, caps, now)
    }

    /// Starts a "partial", "full" or "notify" Subscription message for the entry `index`, in
    /// which this Device is the responder (REQ-PE-SUB-008 to -013). Send the Property Data with
    /// [`Self::send`].
    pub fn update<D: PeHost>(
        &mut self,
        host: &mut D,
        index: usize,
        command: Command,
        encoding: EncodingName,
        total_len: Option<usize>,
        now: u64,
    ) -> Result<RequestInfo, TransactionError> {
        let entry = *self
            .subs
            .get(index)
            .ok_or(TransactionError::UnknownTransaction)?;
        let msg = Outgoing {
            header: entry.update_header(command)?,
            encoding,
            json: true,
            total_len,
        };
        self.request(host, entry.peer(), TxnKind::Subscription, &msg, None, now)
    }

    /// Starts an "end" Subscription message for the entry `index` and removes the entry
    /// (REQ-PE-SUB-017, -018). Send it with [`Self::send`] and no data.
    pub fn unsubscribe<D: PeHost>(
        &mut self,
        host: &mut D,
        index: usize,
        now: u64,
    ) -> Result<(RequestInfo, SubscriptionEntry), TransactionError> {
        let entry = *self
            .subs
            .get(index)
            .ok_or(TransactionError::UnknownTransaction)?;
        let msg = Outgoing::json(entry.end_header(), 0);
        let info = self.request(host, entry.peer(), TxnKind::Subscription, &msg, None, now)?;
        self.subs.remove(index);
        Ok((info, entry))
    }

    /// Asks for the Property Data of subscription `index` again, after an update that the
    /// caller can not apply (REQ-PE-SUB-016). The Get is written by [`Self::write_pending`].
    pub fn refresh(&mut self, index: usize) -> Result<(), TransactionError> {
        self.subs
            .get(index)
            .ok_or(TransactionError::UnknownTransaction)?;
        self.queue_room()?;
        self.push(Pending::Refresh { index });
        Ok(())
    }

    fn total_chunks<D: PeHost>(
        &self,
        host: &D,
        peer: Muid,
        hdr_len: usize,
        total_len: Option<usize>,
    ) -> Result<u16, TransactionError> {
        let room = self.room(host, peer);
        let first = room
            .checked_sub(hdr_len)
            .ok_or(TransactionError::HeaderTooLarge)?;
        Ok(match total_len {
            None => 0,
            Some(n) if n <= first => 1,
            Some(_) if room == 0 => return Err(TransactionError::Size),
            Some(n) => u16::try_from(1 + (n - first).div_ceil(room))
                .ok()
                .filter(|t| *t <= 0x3FFF)
                .ok_or(TransactionError::Size)?,
        })
    }

    fn room<D: PeHost>(&self, host: &D, peer: Muid) -> usize {
        let framing = if host.framing() == Framing::SysEx {
            2
        } else {
            0
        };
        (host.peer_max_sysex(peer) as usize).saturating_sub(CHUNK_OVERHEAD + framing)
    }

    /// The Property Data bytes that the next Chunk of the transaction takes.
    pub fn data_room<D: PeHost>(
        &self,
        host: &D,
        peer: Muid,
        request_id: u8,
        role: Direction,
    ) -> Result<usize, TransactionError> {
        let t = self.txn(peer, request_id, role)?;
        let h = if t.next == 1 { t.hdr_len } else { 0 };
        Ok(self.room(host, peer).saturating_sub(h))
    }

    fn txn(
        &self,
        peer: Muid,
        request_id: u8,
        role: Direction,
    ) -> Result<&Txn<H>, TransactionError> {
        self.find(peer, request_id, role)
            .and_then(|i| self.txns[i].as_ref())
            .ok_or(TransactionError::UnknownTransaction)
    }

    /// Starts the reply to an inquiry when it is complete (REQ-PE-END-008, REQ-PE-HDR-008,
    /// REQ-PE-STAT-006, REQ-PE-TXN-007). Send the Chunks with [`Self::send`]. A reply after the
    /// deadline gives `Late` and closes the transaction.
    pub fn reply<D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        request_id: u8,
        msg: &Outgoing<'_>,
        now: u64,
    ) -> Result<(), TransactionError> {
        let i = self
            .find(peer, request_id, Direction::Responder)
            .ok_or(TransactionError::UnknownTransaction)?;
        let t = self.txns[i].ok_or(TransactionError::UnknownTransaction)?;
        if t.phase != Phase::Replying || t.kind == TxnKind::Subscription {
            return Err(TransactionError::State);
        }
        if now > t.deadline {
            self.close(host, i);
            return Err(TransactionError::Late);
        }
        let mut header = msg.header;
        let status = header.status.unwrap_or(Status::OK);
        header.status = Some(status);
        if status != Status::OK && header.message.is_none() {
            header.message = Some(HeaderStr::Text(default_message(status)));
        }
        let success = status.class() == StatusClass::Success;
        if t.kind == TxnKind::Get && success {
            if t.can_paginate && header.total_count.is_none() {
                return Err(TransactionError::NoTotalCount); // REQ-PE-TXN-007
            }
            // REQ-PE-HDR-008: the reply uses the encoding of the inquiry.
            if msg.encoding != t.encoding {
                return Err(TransactionError::State);
            }
            header.mutual_encoding = (t.encoding != EncodingName::Ascii).then_some(t.encoding);
            if !msg.json && header.media_type.is_none() {
                return Err(TransactionError::NoMediaType); // REQ-PE-ENC-002
            }
        }
        let mut hdr = [0u8; H];
        let hdr_len = header
            .write(HeaderKind::Reply, &mut hdr)
            .map_err(header_error)?;
        let len = if t.kind == TxnKind::Get {
            msg.total_len
        } else {
            Some(0)
        };
        let total = self.total_chunks(host, peer, hdr_len, len)?;
        let t = self.txns[i]
            .as_mut()
            .ok_or(TransactionError::UnknownTransaction)?;
        t.hdr = hdr;
        t.hdr_len = hdr_len;
        t.total = total;
        t.phase = Phase::Sending;
        t.next = 1;
        Ok(())
    }

    /// Replies to an open inquiry with a status and no Property Data, and closes the
    /// transaction: for example 413 for Property Data that the caller refuses as too large
    /// (REQ-PE-STAT-011), or 342 for data that it can not decode (REQ-PE-STAT-008).
    pub fn reply_status<D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        request_id: u8,
        status: Status,
        now: u64,
    ) -> Result<(), TransactionError> {
        let i = self
            .find(peer, request_id, Direction::Responder)
            .ok_or(TransactionError::UnknownTransaction)?;
        let t = self.txns[i].ok_or(TransactionError::UnknownTransaction)?;
        if t.phase == Phase::Sending || t.kind == TxnKind::Subscription {
            return Err(TransactionError::State);
        }
        if now > t.deadline {
            self.close(host, i);
            return Err(TransactionError::Late);
        }
        self.queue_room()?;
        self.push(Pending::Status {
            peer,
            request_id,
            kind: t.kind,
            status,
            subscription: None,
        });
        self.close(host, i);
        Ok(())
    }

    /// Writes the next Chunk of the message of this Device into `out`, with `part` as its
    /// Property Data (at most [`Self::data_room`] bytes); `last` ends the message
    /// (REQ-PE-END-009, REQ-PE-FLOW-002, -006). After the last Chunk of a reply, the
    /// transaction closes (REQ-PE-FLOW-004).
    #[allow(clippy::too_many_arguments)]
    pub fn send<D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        request_id: u8,
        role: Direction,
        part: &[u8],
        last: bool,
        now: u64,
        out: &mut [u8],
    ) -> Result<usize, TransactionError> {
        let i = self
            .find(peer, request_id, role)
            .ok_or(TransactionError::UnknownTransaction)?;
        let t = self.txns[i].ok_or(TransactionError::UnknownTransaction)?;
        if t.phase != Phase::Sending || t.await_ack {
            return Err(TransactionError::State);
        }
        if now > t.deadline && t.deadline != 0 {
            // REQ-PE-END-009. An inquiry that was not sent needs no NAK.
            if !(t.next == 1 && role == Direction::Initiator) {
                self.terminate(&t);
            }
            self.close(host, i);
            return Err(TransactionError::Late);
        }
        if part.len() > self.data_room(host, peer, request_id, role)? {
            return Err(TransactionError::Size);
        }
        let number = t.next;
        if t.total != 0 && number > t.total {
            return Err(TransactionError::State);
        }
        let total = if t.total != 0 && !(last && number < t.total) {
            t.total
        } else if last {
            number
        } else {
            0
        };
        if t.total != 0 && number == t.total && !last {
            return Err(TransactionError::State);
        }
        let header: &[u8] = if number == 1 {
            &t.hdr[..t.hdr_len]
        } else {
            &[]
        };
        let kind = match role {
            Direction::Initiator => t.kind.inquiry(),
            Direction::Responder => t.kind.reply(),
        };
        let n = PeChunk {
            request_id,
            header,
            total,
            number,
            data: part,
        }
        .write(kind, host.muid(), peer, host.framing(), out)?;
        let t = self.txns[i]
            .as_mut()
            .ok_or(TransactionError::UnknownTransaction)?;
        t.next = number + 1;
        t.deadline = now + TIMEOUT_MS;
        if last {
            match role {
                Direction::Initiator => {
                    t.phase = Phase::Receiving; // waits for the reply
                    t.await_ack = false;
                }
                Direction::Responder => {
                    self.close(host, i); // REQ-PE-FLOW-004
                }
            }
        } else if t.flow {
            t.await_ack = true; // REQ-PE-FLOW-002, -013
        }
        Ok(n)
    }

    /// Asks the peer to wait `ms` more milliseconds with a MIDI-CI ACK 0x10, rounded up to
    /// 100 ms (REQ-PE-END-010). The deadline of this Device grows by the same time.
    pub fn wait(
        &mut self,
        peer: Muid,
        request_id: u8,
        role: Direction,
        ms: u64,
    ) -> Result<(), TransactionError> {
        if ms == 0 || ms > MAX_WAIT_MS {
            return Err(TransactionError::InvalidWait);
        }
        let units = ms.div_ceil(100);
        let i = self
            .find(peer, request_id, role)
            .ok_or(TransactionError::UnknownTransaction)?;
        self.queue_room()?;
        let t = self.txns[i]
            .as_mut()
            .ok_or(TransactionError::UnknownTransaction)?;
        t.deadline += units * 100;
        let (kind, last) = (t.kind, t.last_good);
        self.push(Pending::Ack {
            peer,
            request_id,
            kind,
            status: AckStatus::TimeoutWait(units as u8),
            chunk: last,
        });
        Ok(())
    }

    /// Ends a transaction with a MIDI-CI NAK 0x20: Status Data 0x01 as initiator, 0x00 as
    /// responder (REQ-PE-END-002, -004, REQ-PE-STAT-013).
    pub fn abort<D: PeHost>(
        &mut self,
        host: &mut D,
        peer: Muid,
        request_id: u8,
        role: Direction,
    ) -> Result<(), TransactionError> {
        let i = self
            .find(peer, request_id, role)
            .ok_or(TransactionError::UnknownTransaction)?;
        let t = self.txns[i].ok_or(TransactionError::UnknownTransaction)?;
        self.queue_room()?;
        self.terminate(&t);
        self.close(host, i);
        Ok(())
    }

    /// The caller found the last received Chunk corrupt: with Flow Control, the engine asks for
    /// it again with a NAK 0x12 and does not ACK it (REQ-PE-FLOW-011).
    pub fn corrupt(
        &mut self,
        peer: Muid,
        request_id: u8,
        role: Direction,
    ) -> Result<(), TransactionError> {
        let i = self
            .find(peer, request_id, role)
            .ok_or(TransactionError::UnknownTransaction)?;
        let mut t = self.txns[i].ok_or(TransactionError::UnknownTransaction)?;
        if !t.flow || t.last_good == 0 {
            return Err(TransactionError::State);
        }
        let bad = t.last_good;
        let is_ack = |p: &Option<Pending>| {
            matches!(p, Some(Pending::Ack { peer: m, request_id: r, chunk, status: AckStatus::FlowControlSendNext, .. })
                if *m == peer && *r == request_id && *chunk == bad)
        };
        // The removed ACK gives a free entry for the NAK.
        if self.free_pending() == 0 && !self.pending.iter().any(is_ack) {
            return Err(TransactionError::QueueFull);
        }
        for p in self.pending.iter_mut() {
            if matches!(p, Some(Pending::Ack { peer: m, request_id: r, chunk, status: AckStatus::FlowControlSendNext, .. })
                if *m == peer && *r == request_id && *chunk == bad)
            {
                *p = None;
            }
        }
        t.last_good = bad - 1;
        t.decoder = t.prev_decoder;
        t.zlib_checked = t.prev_zlib_checked;
        if t.phase == Phase::Replying {
            t.phase = Phase::Receiving;
        }
        self.txns[i] = Some(t);
        self.nak(&t, NakStatus::FlowControlResend, t.last_good);
        Ok(())
    }
}
