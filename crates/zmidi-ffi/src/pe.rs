//! The C ABI of the zmidi-ci-pe Property Exchange engine (DEC-019 addendum of 2026-10-23): Property
//! Exchange with many Chunks, Flow Control and Subscriptions (M2-103-UM v1.2 sections 5 to 15) over
//! the `CiDevice` of a [`ZmidiCi`].
//!
//! - The program gives the storage [`ZmidiPe`] and a Resource table of at most
//!   [`ZMIDI_PE_RESOURCES_MAX`] entries. The engine points into its own storage, so the program does
//!   not move or copy a [`ZmidiPe`] after [`zmidi_pe_init`]; a call with a moved storage gives the
//!   invalid-argument status.
//! - The program gives each received message to `zmidi_ci_receive` first, then calls
//!   [`zmidi_pe_receive`]. The engine reads the last event of the [`ZmidiCi`].
//! - The start calls (reply, update, request, subscribe, unsubscribe) start a transaction;
//!   [`zmidi_pe_send`] writes its Chunks. The calls that queue a control message (reply status,
//!   refresh, wait, abort, corrupt) need [`zmidi_pe_write_pending`].
//! - The clock is in microseconds, as in the ci calls; the engine gets milliseconds.
//! - The encodings are "ASCII" and "Mcoded7"; the engine does not inflate "zlib+Mcoded7" (zlib
//!   window 0, REQ-PE-ENC-009). The program encodes the Property Data that it sends.
//! - A program uses the engine or the one-chunk Responder (`zmidi_ci_pe_reply`), not both.
//! - Output messages are records `[group][len lo][len hi][bytes]` on Group 0, as the records of
//!   `zmidi_ci_pe_reply`. There are no callbacks (DEC-019 rule 2).

use core::mem::{align_of, size_of};
use core::ptr::{addr_of, addr_of_mut};

use zmidi_ci::{ChunkEnd, DeviceError, Direction, Event, Muid, NakStatus, WriteError};
use zmidi_ci_pe::header::{Command, EncodingName, Header, HeaderStr, Status};
use zmidi_ci_pe::resource_list::{CanSet, Encodings};
use zmidi_ci_pe::{
    CloseReason, EngineConfig, Outgoing, PeEngine, PeEvent, ResourceCaps, ResourceEntry, Role,
    SubscriptionEntry, SubscriptionError, TransactionError, TxnKind,
};

use crate::ci::{self, ZmidiCi, ZMIDI_PE_ID_LEN};
use crate::status::ZmidiStatus;

/// The engine of the C ABI: 4 transactions with 256 header bytes each, 4 subscriptions, 4 peers
/// with a known ResourceList and 16 queued control messages (REQ-FFI-PE-001).
type Engine = PeEngine<'static, 4, 256, 4, 4, 16>;

/// Most entries of the Resource table (REQ-FFI-PE-009).
pub const ZMIDI_PE_RESOURCES_MAX: usize = 16;

/// Bytes of an output buffer of [`zmidi_pe_write_pending`]: a record header, the MIDI-CI header
/// and Chunk fields with the SysEx framing, and the 256 header bytes of a Get after a "notify".
pub const ZMIDI_PE_PENDING_MAX: usize = 3 + 2 + 22 + 256;

/// Bytes of the JSON of one ResourceList entry (with its "schema"); a longer entry gives the
/// invalid-argument status at init.
const ENTRY_MAX: usize = 512;

/// The state in the storage of a [`ZmidiPe`].
struct PeState {
    eng: Engine,
    /// The Resource table of the engine; the engine points into it.
    table: [ResourceEntry<'static>; ZMIDI_PE_RESOURCES_MAX],
    /// The one "mediaType" of each entry; the entries point into it.
    media: [[&'static str; 1]; ZMIDI_PE_RESOURCES_MAX],
    /// The address of the storage at init.
    home: usize,
    /// The reason of the last refusal (REQ-FFI-PE-008).
    error: ZmidiPeError,
    /// The status of a refusal by the ResourceList of the peer.
    refused: u16,
}

/// Storage words of a [`ZmidiPe`] on a 64-bit target.
#[cfg(target_pointer_width = "64")]
pub const ZMIDI_PE_WORDS: usize = 488;
/// Storage words of a [`ZmidiPe`] on a 32-bit or 16-bit target (wasm32 aligns `u128` to 16 bytes:
/// 2936 bytes there, 2928 on thumbv7em).
#[cfg(not(target_pointer_width = "64"))]
pub const ZMIDI_PE_WORDS: usize = 368;

/// Storage of one Property Exchange engine. The program gives it; [`zmidi_pe_init`] sets it up.
/// The program does not move or copy it after init. 16-byte alignment: the engine holds a `u128`.
#[repr(C, align(16))]
pub struct ZmidiPe {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_PE_WORDS],
}

const _: () = assert!(size_of::<PeState>() <= size_of::<ZmidiPe>());
const _: () = assert!(align_of::<PeState>() <= align_of::<ZmidiPe>());

/// One entry of the Resource table (M2-103 Table 14, REQ-FFI-PE-009). The texts are
/// NUL-terminated and stay valid while the engine is used.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiPeResource {
    /// "resource": 1 to 36 characters A-Z, a-z, 0-9 with an optional "X-" prefix.
    pub resource: *const u8,
    /// "canGet".
    pub can_get: bool,
    /// "canSet": 0 "none", 1 "full", 2 "partial".
    pub can_set: u8,
    /// "canSubscribe".
    pub can_subscribe: bool,
    /// "requireResId".
    pub require_res_id: bool,
    /// "fcOnGet".
    pub fc_on_get: bool,
    /// "fcOnSet".
    pub fc_on_set: bool,
    /// "canPaginate".
    pub can_paginate: bool,
    /// Write every Property, also the ones with their default value (REQ-PE-LIST-009).
    pub write_defaults: bool,
    /// "encodings": bit n is the [`ZmidiPeEncoding`] of value n; at least one bit.
    pub encodings: u8,
    /// The one "mediaType" ("type/subtype", at most 75 characters), or null for
    /// "application/json".
    pub media_type: *const u8,
    /// "schema" as JSON text, or null.
    pub schema: *const u8,
    /// Bytes of `schema`.
    pub schema_len: usize,
}

/// The request Properties that the program gives to [`zmidi_pe_request`] and
/// [`zmidi_pe_subscribe`] (M2-103 8). The texts are NUL-terminated and stay valid during the call.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiPeHeader {
    /// "resource".
    pub resource: *const u8,
    /// "resId", or null.
    pub res_id: *const u8,
    /// "mediaType" of the Property Data of a Set, or null for JSON.
    pub media_type: *const u8,
    /// True when the request has "offset" and "limit" (REQ-PE-TXN-006).
    pub has_offset: bool,
    /// "offset".
    pub offset: u32,
    /// "limit", 1 and up.
    pub limit: u32,
    /// "setPartial".
    pub set_partial: bool,
    /// The [`ZmidiPeEncoding`] of the Property Data that the program sends; for a Get, the
    /// encoding that it asks for the reply.
    pub encoding: u8,
    /// Bytes of the encoded Property Data that the program sends (0 for a Get); `SIZE_MAX` when
    /// not known.
    pub length: usize,
}

/// The kind of a [`ZmidiPeEvent`] (one for each `PeEvent` of zmidi-ci-pe).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiPeEventKind {
    /// No event.
    None = 0,
    /// A received Chunk of a message.
    Message = 1,
    /// Flow Control: send the next Chunk (REQ-PE-FLOW-002).
    Ready = 2,
    /// Flow Control: send the Chunk `chunk` again (REQ-PE-FLOW-012, -013).
    Resend = 3,
    /// A transaction closed; `reason` tells why.
    Closed = 4,
    /// This Device accepted a "start" and replied 200; `subscription` is the new entry.
    SubscriptionStarted = 5,
    /// The peer ended a subscription with "end"; the entry fields are in the event.
    SubscriptionEnded = 6,
    /// An Invalidate MUID closed the transactions of `muid`; `ended` subscriptions wait in
    /// [`zmidi_pe_take_ended`].
    Invalidated = 7,
}

/// The role of this Device in a transaction or a subscription.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiPeRole {
    /// This Device sent the inquiry or the "start".
    Initiator = 0,
    /// This Device answers.
    Responder = 1,
}

/// The kind of a transaction.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiPeTxnKind {
    /// Get Property Data and its reply.
    Get = 0,
    /// Set Property Data and its reply.
    Set = 1,
    /// Subscription and its reply.
    Subscription = 2,
}

/// The end of a received Chunk (M2-103 5.3).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiPeChunkEnd {
    /// More Chunks follow.
    More = 0,
    /// The last Chunk: the Property Data is complete.
    Complete = 1,
    /// The last Chunk: the Property Data is not known to be usable (REQ-PE-STAT-008).
    Incomplete = 2,
}

/// The encoding of Property Data (M2-103 Table 12).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiPeEncoding {
    /// "ASCII".
    Ascii = 0,
    /// "Mcoded7".
    Mcoded7 = 1,
    /// "zlib+Mcoded7": the event gives the zlib stream.
    ZlibMcoded7 = 2,
}

/// The "command" of a Subscription message (M2-103 11).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiPeCommand {
    /// No "command".
    None = 0,
    /// "start".
    Start = 1,
    /// "partial".
    Partial = 2,
    /// "full".
    Full = 3,
    /// "notify".
    Notify = 4,
    /// "end".
    End = 5,
}

/// Why a transaction closed.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiPeCloseReason {
    /// The event is not a close.
    None = 0,
    /// A deadline of this Device passed.
    Timeout = 1,
    /// The peer ended it: NAK 0x20 or Notify 144.
    Terminated = 2,
    /// The peer timed out: NAK 0x42 or Notify 408.
    PeerTimeout = 3,
    /// A Chunk came out of sequence; this Device sent NAK 0x21.
    OutOfSequence = 4,
    /// The peer sent another NAK: `nak_code` and `nak_data`.
    Nak = 5,
    /// The Header Data of the reply is not valid.
    BadReply = 6,
    /// This Device replied with the status `status` after an error.
    Refused = 7,
    /// An Invalidate MUID ended it.
    Invalidated = 8,
    /// A Flow Control resend that this Device can not do.
    ResendFailed = 9,
}

/// Why the reply to a "start" gave no subscription (REQ-PE-SUB-006).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiPeSubscriptionError {
    /// No error.
    None = 0,
    /// The subscription table is full.
    Full = 1,
    /// The reply has no "subscribeId".
    NoSubscribeId = 2,
    /// The reply has no status from 200 to 299.
    NotSuccess = 3,
    /// A command or Property Data that the role does not permit.
    NotPermitted = 4,
    /// A header value is longer than the entry buffers, or a Property is missing.
    Invalid = 5,
}

/// One event of the engine (REQ-FFI-PE-002, REQ-FFI-PE-010). The fields that the kind does not
/// use are 0, -1 or empty.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiPeEvent {
    /// The kind; None when there is no event.
    pub kind: ZmidiPeEventKind,
    /// The MUID of the peer (Invalidated: the invalidated MUID).
    pub muid: u32,
    /// Request ID of the transaction.
    pub request_id: u8,
    /// The role of this Device.
    pub role: ZmidiPeRole,
    /// The kind of the transaction.
    pub txn: ZmidiPeTxnKind,
    /// Message: the end of this Chunk.
    pub end: ZmidiPeChunkEnd,
    /// Message: the encoding of the Property Data.
    pub encoding: ZmidiPeEncoding,
    /// Message: the "command" of the header.
    pub command: ZmidiPeCommand,
    /// Message: the "status" of the header, 0 when absent. Closed with Refused: the status.
    pub status: u16,
    /// Resend: the number of the Chunk to send again.
    pub chunk: u16,
    /// Message: the index of the Resource in the Resource table, or -1.
    pub resource_index: i32,
    /// Message: the subscription entry of the message, or -1. SubscriptionStarted: the new entry.
    pub subscription: i32,
    /// Message of the reply to a "start": the new subscription entry, or -1.
    pub started: i32,
    /// Message of the reply to a "start" with no new entry: why.
    pub start_error: ZmidiPeSubscriptionError,
    /// Closed: why.
    pub reason: ZmidiPeCloseReason,
    /// Closed with Nak: the NAK Status Code.
    pub nak_code: u8,
    /// Closed with Nak: the NAK Status Data.
    pub nak_data: u8,
    /// Invalidated: the number of removed subscriptions.
    pub ended: u32,
    /// Message: the Header Data (first Chunk only), in the message of the program.
    pub header: *const u8,
    /// Bytes of `header`.
    pub header_len: usize,
    /// Message: the decoded Property Data of this Chunk, in the scratch buffer of the program.
    pub data: *const u8,
    /// Bytes of `data`.
    pub data_len: usize,
    /// Message: the decoded "resource" (NUL-terminated). SubscriptionEnded: the entry resource.
    pub resource: [u8; ZMIDI_PE_ID_LEN],
    /// Message: the decoded "resId". SubscriptionEnded: the entry resId.
    pub res_id: [u8; ZMIDI_PE_ID_LEN],
    /// Message: the decoded "subscribeId". SubscriptionEnded: the entry subscribeId.
    pub subscribe_id: [u8; ZMIDI_PE_ID_LEN],
}

impl ZmidiPeEvent {
    const NONE: ZmidiPeEvent = ZmidiPeEvent {
        kind: ZmidiPeEventKind::None,
        muid: 0,
        request_id: 0,
        role: ZmidiPeRole::Initiator,
        txn: ZmidiPeTxnKind::Get,
        end: ZmidiPeChunkEnd::More,
        encoding: ZmidiPeEncoding::Ascii,
        command: ZmidiPeCommand::None,
        status: 0,
        chunk: 0,
        resource_index: -1,
        subscription: -1,
        started: -1,
        start_error: ZmidiPeSubscriptionError::None,
        reason: ZmidiPeCloseReason::None,
        nak_code: 0,
        nak_data: 0,
        ended: 0,
        header: core::ptr::null(),
        header_len: 0,
        data: core::ptr::null(),
        data_len: 0,
        resource: [0; ZMIDI_PE_ID_LEN],
        res_id: [0; ZMIDI_PE_ID_LEN],
        subscribe_id: [0; ZMIDI_PE_ID_LEN],
    };
}

/// A copy of one subscription table entry (REQ-FFI-PE-007).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiPeSubscription {
    /// The MUID of the peer.
    pub muid: u32,
    /// The role of this Device.
    pub role: ZmidiPeRole,
    /// "subscribeId" (NUL-terminated).
    pub subscribe_id: [u8; ZMIDI_PE_ID_LEN],
    /// "resource" (NUL-terminated).
    pub resource: [u8; ZMIDI_PE_ID_LEN],
    /// "resId" (NUL-terminated, empty when absent).
    pub res_id: [u8; ZMIDI_PE_ID_LEN],
}

/// The reason of the last refusal of the engine (REQ-FFI-PE-008): one value for each
/// `TransactionError` of zmidi-ci-pe, and the refusals of the C ABI.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiPeError {
    /// No refusal.
    None = 0,
    /// The zmidi-ci Device refused (unknown peer, no PE Capabilities, no free Request ID, a call
    /// that the build does not support).
    Device = 1,
    /// A message writer failed: the output buffer is too small.
    Write = 2,
    /// The header writer refused the header.
    Header = 3,
    /// A subscription rule refused the message.
    Subscription = 4,
    /// No open transaction has this peer, Request ID and role, or no subscription this index.
    UnknownTransaction = 5,
    /// All transaction slots are in use.
    Full = 6,
    /// The peer has its maximum of open requests.
    PeerBusy = 7,
    /// The deadline passed; the transaction is closed.
    Late = 8,
    /// The call does not fit the state of the transaction.
    State = 9,
    /// The ResourceList of the peer does not permit the request; the status it would get is
    /// given by [`zmidi_pe_last_error`].
    Refused = 10,
    /// Property Data that is not JSON needs a "mediaType".
    NoMediaType = 11,
    /// A reply for a Resource with "canPaginate" needs "totalCount".
    NoTotalCount = 12,
    /// The header does not fit the header buffer or the first Chunk.
    HeaderTooLarge = 13,
    /// A Chunk part larger than the room of the Chunk, or a scratch buffer smaller than the
    /// received Property Data.
    Size = 14,
    /// The queue of control messages is full: call [`zmidi_pe_write_pending`] first.
    QueueFull = 15,
    /// A wait of 0 or more than 12.7 seconds.
    InvalidWait = 16,
    /// A "$ref" value that is not "midi+jsonschema://<id>".
    InvalidRef = 17,
}

/// The reason and the status of a refusal (REQ-FFI-PE-008).
fn refusal(e: &TransactionError) -> (ZmidiPeError, ZmidiStatus) {
    use TransactionError as T;
    use ZmidiPeError as E;
    match e {
        T::Device(DeviceError::NotSupported) => (E::Device, ZmidiStatus::NotSupported),
        T::Device(DeviceError::Write(WriteError::Overflow)) => {
            (E::Device, ZmidiStatus::BufferTooSmall)
        }
        T::Device(_) => (E::Device, ZmidiStatus::Rejected),
        T::Write(_) => (E::Write, ZmidiStatus::BufferTooSmall),
        T::Header(_) => (E::Header, ZmidiStatus::InvalidArgument),
        T::Subscription(_) => (E::Subscription, ZmidiStatus::Rejected),
        T::UnknownTransaction => (E::UnknownTransaction, ZmidiStatus::Rejected),
        T::Full => (E::Full, ZmidiStatus::Rejected),
        T::PeerBusy => (E::PeerBusy, ZmidiStatus::Rejected),
        T::Late => (E::Late, ZmidiStatus::Rejected),
        T::State => (E::State, ZmidiStatus::Rejected),
        T::Refused(_) => (E::Refused, ZmidiStatus::Rejected),
        T::NoMediaType => (E::NoMediaType, ZmidiStatus::InvalidArgument),
        T::NoTotalCount => (E::NoTotalCount, ZmidiStatus::InvalidArgument),
        T::HeaderTooLarge => (E::HeaderTooLarge, ZmidiStatus::InvalidArgument),
        T::Size => (E::Size, ZmidiStatus::BufferTooSmall),
        T::QueueFull => (E::QueueFull, ZmidiStatus::Rejected),
        T::InvalidWait => (E::InvalidWait, ZmidiStatus::InvalidArgument),
        T::InvalidRef => (E::InvalidRef, ZmidiStatus::InvalidArgument),
    }
}

/// Gives the status of a refusal and keeps its reason in `s`.
fn refuse(s: &mut PeState, e: TransactionError) -> ZmidiStatus {
    if let TransactionError::Refused(st) = e {
        s.refused = st.0;
    }
    let (error, status) = refusal(&e);
    s.error = error;
    status
}

/// Gives the state of `pe` with its error cleared, or `None` for a null or moved storage.
///
/// # Safety
/// `pe` is null or was set up by [`zmidi_pe_init`].
unsafe fn pe_state<'a>(pe: *mut ZmidiPe) -> Option<&'a mut PeState> {
    // SAFETY: caller contract above.
    let s = unsafe { pe.cast::<PeState>().as_mut() }?;
    if s.home != pe as usize {
        return None;
    }
    s.error = ZmidiPeError::None;
    s.refused = 0;
    Some(s)
}

/// Gives the states of `pe` and `ci`.
///
/// # Safety
/// `pe` is null or was set up by [`zmidi_pe_init`]; `ci` is null or was set up by `zmidi_ci_init`.
unsafe fn states<'a>(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
) -> Option<(&'a mut PeState, &'a mut ci::State)> {
    // SAFETY: caller contract above.
    unsafe { Some((pe_state(pe)?, ci::state(ci)?)) }
}

/// Milliseconds of the engine from microseconds of the program.
const fn ms(now_us: u64) -> u64 {
    now_us / 1000
}

fn direction(role: u8) -> Option<Direction> {
    match role {
        0 => Some(Direction::Initiator),
        1 => Some(Direction::Responder),
        _ => None,
    }
}

fn encoding(e: u8) -> Option<EncodingName> {
    match e {
        0 => Some(EncodingName::Ascii),
        1 => Some(EncodingName::Mcoded7),
        2 => Some(EncodingName::ZlibMcoded7),
        _ => None,
    }
}

/// Gives the text of a NUL-terminated string of at most `max` bytes, `Some(None)` for null, and
/// `None` for a string that is too long or not UTF-8.
///
/// # Safety
/// `p` is null or points to a NUL-terminated string.
unsafe fn text(p: *const u8, max: usize) -> Option<Option<&'static str>> {
    if p.is_null() {
        return Some(None);
    }
    // SAFETY: caller contract above.
    let b = unsafe { ci::c_str(p, max) }?;
    core::str::from_utf8(b).ok().map(Some)
}

/// The capabilities of `r`, or `None` when a value is out of range.
fn caps(r: &ZmidiPeResource) -> Option<ResourceCaps> {
    let can_set = match r.can_set {
        0 => CanSet::None,
        1 => CanSet::Full,
        2 => CanSet::Partial,
        _ => return None,
    };
    if r.encodings == 0 || r.encodings > 7 {
        return None;
    }
    Some(ResourceCaps {
        can_get: r.can_get,
        can_set,
        can_subscribe: r.can_subscribe,
        require_res_id: r.require_res_id,
        fc_on_get: r.fc_on_get,
        fc_on_set: r.fc_on_set,
        encodings: Encodings {
            ascii: r.encodings & 1 != 0,
            mcoded7: r.encodings & 2 != 0,
            zlib_mcoded7: r.encodings & 4 != 0,
        },
        can_paginate: r.can_paginate,
    })
}

/// Gives the capabilities at `p`: `Some(None)` for null, `None` for values out of range.
///
/// # Safety
/// `p` is null or points to a valid [`ZmidiPeResource`].
#[cfg(feature = "ci-initiator")]
unsafe fn caps_at(p: *const ZmidiPeResource) -> Option<Option<ResourceCaps>> {
    // SAFETY: caller contract above.
    match unsafe { p.as_ref() } {
        None => Some(None),
        Some(r) => caps(r).map(Some),
    }
}

/// Copies the decoded characters of `s` into `out` with a NUL.
fn decode(s: Option<HeaderStr<'_>>, out: &mut [u8; ZMIDI_PE_ID_LEN]) {
    if let Some(s) = s {
        s.decode_into(&mut out[..ZMIDI_PE_ID_LEN - 1]);
    }
}

/// Copies `t` into `out` with a NUL, cut at the size of `out`.
fn copy_text(t: &str, out: &mut [u8; ZMIDI_PE_ID_LEN]) {
    let n = t.len().min(ZMIDI_PE_ID_LEN - 1);
    out[..n].copy_from_slice(&t.as_bytes()[..n]);
}

fn index(i: Option<usize>) -> i32 {
    i.map_or(-1, |i| i as i32)
}

const fn role_of(d: Direction) -> ZmidiPeRole {
    match d {
        Direction::Initiator => ZmidiPeRole::Initiator,
        Direction::Responder => ZmidiPeRole::Responder,
    }
}

const fn txn_of(k: TxnKind) -> ZmidiPeTxnKind {
    match k {
        TxnKind::Get => ZmidiPeTxnKind::Get,
        TxnKind::Set => ZmidiPeTxnKind::Set,
        TxnKind::Subscription => ZmidiPeTxnKind::Subscription,
    }
}

const fn encoding_of(e: EncodingName) -> ZmidiPeEncoding {
    match e {
        EncodingName::Ascii => ZmidiPeEncoding::Ascii,
        EncodingName::Mcoded7 => ZmidiPeEncoding::Mcoded7,
        EncodingName::ZlibMcoded7 => ZmidiPeEncoding::ZlibMcoded7,
    }
}

const fn command_of(c: Option<Command>) -> ZmidiPeCommand {
    match c {
        None => ZmidiPeCommand::None,
        Some(Command::Start) => ZmidiPeCommand::Start,
        Some(Command::Partial) => ZmidiPeCommand::Partial,
        Some(Command::Full) => ZmidiPeCommand::Full,
        Some(Command::Notify) => ZmidiPeCommand::Notify,
        Some(Command::End) => ZmidiPeCommand::End,
    }
}

const fn sub_error_of(e: SubscriptionError) -> ZmidiPeSubscriptionError {
    match e {
        SubscriptionError::Full => ZmidiPeSubscriptionError::Full,
        SubscriptionError::NoSubscribeId => ZmidiPeSubscriptionError::NoSubscribeId,
        SubscriptionError::NotSuccess => ZmidiPeSubscriptionError::NotSuccess,
        SubscriptionError::NotPermitted => ZmidiPeSubscriptionError::NotPermitted,
        SubscriptionError::Invalid => ZmidiPeSubscriptionError::Invalid,
    }
}

/// The NAK Status Code and Status Data (M2-101 Table 16).
const fn nak_bytes(n: NakStatus) -> (u8, u8) {
    match n {
        NakStatus::Nak => (0x00, 0),
        NakStatus::NotSupported => (0x01, 0),
        NakStatus::VersionNotSupported => (0x02, 0),
        NakStatus::NotInUse => (0x03, 0),
        NakStatus::ProfileNotSupported => (0x04, 0),
        NakStatus::FlowControlResend => (0x12, 0),
        NakStatus::Terminate { by_initiator } => (0x20, by_initiator as u8),
        NakStatus::ChunksOutOfSequence => (0x21, 0),
        NakStatus::Error => (0x40, 0),
        NakStatus::Malformed => (0x41, 0),
        NakStatus::Timeout => (0x42, 0),
        NakStatus::Busy(t) => (0x43, t),
        NakStatus::Unknown { code, data } => (code, data),
    }
}

/// Fills `ev` with the fields of a subscription entry.
fn fill_entry(
    e: &SubscriptionEntry,
    muid: &mut u32,
    role: &mut ZmidiPeRole,
    texts: [&mut [u8; ZMIDI_PE_ID_LEN]; 3],
) {
    *muid = e.peer().value();
    *role = match e.role() {
        Role::Initiator => ZmidiPeRole::Initiator,
        Role::Responder => ZmidiPeRole::Responder,
    };
    let [id, res, res_id] = texts;
    copy_text(e.subscribe_id(), id);
    copy_text(e.resource(), res);
    copy_text(e.res_id().unwrap_or(""), res_id);
}

/// Fills `ev` from `event`; `raw` is the Header Data of the Chunk in the message of the program.
fn fill(ev: &mut ZmidiPeEvent, event: &PeEvent<'_, '_>, raw: &[u8]) {
    use ZmidiPeEventKind as K;
    match *event {
        PeEvent::Message(m) => {
            ev.kind = K::Message;
            ev.muid = m.peer.value();
            ev.request_id = m.request_id;
            ev.role = role_of(m.role);
            ev.txn = txn_of(m.kind);
            ev.end = match m.end {
                ChunkEnd::More => ZmidiPeChunkEnd::More,
                ChunkEnd::Complete => ZmidiPeChunkEnd::Complete,
                ChunkEnd::Incomplete => ZmidiPeChunkEnd::Incomplete,
            };
            ev.encoding = encoding_of(m.encoding);
            ev.resource_index = index(m.resource);
            ev.subscription = index(m.subscription);
            match m.started {
                Some(Ok(i)) => ev.started = i as i32,
                Some(Err(e)) => ev.start_error = sub_error_of(e),
                None => {}
            }
            if let Some(h) = m.header {
                ev.header = raw.as_ptr();
                ev.header_len = raw.len();
                ev.status = h.status.map_or(0, |s| s.0);
                ev.command = command_of(h.command);
                decode(h.resource, &mut ev.resource);
                decode(h.res_id, &mut ev.res_id);
                decode(h.subscribe_id, &mut ev.subscribe_id);
            }
            ev.data = m.data.as_ptr();
            ev.data_len = m.data.len();
        }
        PeEvent::Ready {
            peer,
            request_id,
            role,
        } => {
            ev.kind = K::Ready;
            ev.muid = peer.value();
            ev.request_id = request_id;
            ev.role = role_of(role);
        }
        PeEvent::Resend {
            peer,
            request_id,
            role,
            chunk,
        } => {
            ev.kind = K::Resend;
            ev.muid = peer.value();
            ev.request_id = request_id;
            ev.role = role_of(role);
            ev.chunk = chunk;
        }
        PeEvent::Closed {
            peer,
            request_id,
            role,
            reason,
        } => {
            use ZmidiPeCloseReason as R;
            ev.kind = K::Closed;
            ev.muid = peer.value();
            ev.request_id = request_id;
            ev.role = role_of(role);
            ev.reason = match reason {
                CloseReason::Timeout => R::Timeout,
                CloseReason::Terminated => R::Terminated,
                CloseReason::PeerTimeout => R::PeerTimeout,
                CloseReason::OutOfSequence => R::OutOfSequence,
                CloseReason::Nak(n) => {
                    (ev.nak_code, ev.nak_data) = nak_bytes(n);
                    R::Nak
                }
                CloseReason::BadReply(_) => R::BadReply,
                CloseReason::Refused(s) => {
                    ev.status = s.0;
                    R::Refused
                }
                CloseReason::Invalidated => R::Invalidated,
                CloseReason::ResendFailed => R::ResendFailed,
            };
        }
        PeEvent::SubscriptionStarted { index } => {
            ev.kind = K::SubscriptionStarted;
            ev.subscription = index as i32;
        }
        PeEvent::SubscriptionEnded(e) => {
            ev.kind = K::SubscriptionEnded;
            ev.txn = ZmidiPeTxnKind::Subscription;
            fill_entry(
                &e,
                &mut ev.muid,
                &mut ev.role,
                [&mut ev.subscribe_id, &mut ev.resource, &mut ev.res_id],
            );
        }
        PeEvent::Invalidated { muid, ended } => {
            ev.kind = K::Invalidated;
            ev.muid = muid.value();
            ev.ended = ended as u32;
        }
    }
}

/// Writes one message from `write` as a record on Group 0 into `out` (`capacity` bytes) and its
/// length into `out_len`.
///
/// # Safety
/// `out` points to `capacity` writable bytes; `out_len` is writable.
unsafe fn record(
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
    write: impl FnOnce(&mut [u8]) -> Result<usize, ZmidiStatus>,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let out = unsafe { core::slice::from_raw_parts_mut(out, capacity) };
    let Some(body) = out.get_mut(3..) else {
        return ZmidiStatus::BufferTooSmall;
    };
    match write(body) {
        Ok(n) => {
            out[0] = 0;
            out[1] = n as u8;
            out[2] = (n >> 8) as u8;
            // SAFETY: caller contract above.
            unsafe { out_len.write(3 + n) };
            ZmidiStatus::Ok
        }
        Err(e) => e,
    }
}

/// Gives the size in bytes of the Rust state of a [`ZmidiPe`] (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_pe_size() -> usize {
    size_of::<PeState>()
}

/// Sets up `pe` with the Resource table `resources` of `count` entries (REQ-FFI-PE-009). A null
/// pointer, more than [`ZMIDI_PE_RESOURCES_MAX`] entries, or an entry that is not valid gives
/// invalid-argument and changes nothing.
///
/// # Safety
/// `pe` points to writable storage of a [`ZmidiPe`]; `resources` points to `count` entries (or is
/// null when `count` is 0); their texts stay valid while the engine is used.
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_init(
    pe: *mut ZmidiPe,
    resources: *const ZmidiPeResource,
    count: usize,
) -> ZmidiStatus {
    if pe.is_null() || (resources.is_null() && count > 0) || count > ZMIDI_PE_RESOURCES_MAX {
        return ZmidiStatus::InvalidArgument;
    }
    let src: &[ZmidiPeResource] = if count == 0 {
        &[]
    } else {
        // SAFETY: caller contract above.
        unsafe { core::slice::from_raw_parts(resources, count) }
    };
    let st = pe.cast::<PeState>();
    let mut media: [[&'static str; 1]; ZMIDI_PE_RESOURCES_MAX] = [[""]; ZMIDI_PE_RESOURCES_MAX];
    let mut table = [ResourceEntry::new(""); ZMIDI_PE_RESOURCES_MAX];
    for (i, r) in src.iter().enumerate() {
        // SAFETY: caller contract above: the texts are NUL-terminated.
        let (Some(Some(name)), Some(mt)) = (unsafe { text(r.resource, 36) }, unsafe {
            text(r.media_type, 75)
        }) else {
            return ZmidiStatus::InvalidArgument;
        };
        let Some(c) = caps(r) else {
            return ZmidiStatus::InvalidArgument;
        };
        // SAFETY: caller contract above.
        let Some(schema) = (unsafe { ci::bytes(r.schema, r.schema_len) }) else {
            return ZmidiStatus::InvalidArgument;
        };
        let mut e = ResourceEntry::new(name);
        e.caps = c;
        e.schema = (!r.schema.is_null()).then_some(schema);
        e.write_defaults = r.write_defaults;
        if let Some(m) = mt {
            media[i] = [m];
        }
        table[i] = e;
    }
    // SAFETY: `st` points to writable storage of a PeState (size and alignment asserted above).
    // The engine and the entries point into the storage, which the program does not move.
    unsafe {
        addr_of_mut!((*st).media).write(media);
        for (i, e) in table.iter_mut().enumerate().take(count) {
            if src[i].media_type.is_null() {
                continue;
            }
            let m: &'static [&'static str; 1] = &*addr_of!((*st).media[i]);
            e.media_types = m;
        }
        addr_of_mut!((*st).table).write(table);
        let entries: &'static [ResourceEntry<'static>] = core::slice::from_raw_parts(
            addr_of!((*st).table).cast::<ResourceEntry<'static>>(),
            count,
        );
        let eng = Engine::new(EngineConfig {
            resources: entries,
            zlib_window: 0,
        });
        // The ResourceList writer checks every entry: the "resource" characters (M2-103 8.1), the
        // "mediaType" form and the size of the JSON (REQ-FFI-PE-009).
        let mut w = eng.resource_list::<ENTRY_MAX>();
        let mut sink = [0u8; 64];
        while !w.is_done() {
            if w.write_next(&mut sink).is_err() {
                return ZmidiStatus::InvalidArgument;
            }
        }
        addr_of_mut!((*st).eng).write(eng);
        addr_of_mut!((*st).home).write(pe as usize);
        addr_of_mut!((*st).error).write(ZmidiPeError::None);
        addr_of_mut!((*st).refused).write(0);
    }
    ZmidiStatus::Ok
}

/// Gives the last event of `ci` (from `zmidi_ci_receive`) to the engine at `now_us` and writes the
/// engine event into `ev` (kind None when there is none) (REQ-FFI-PE-002). The decoded Property
/// Data goes into `scratch`. An event that the engine took is not given again. A refusal changes
/// nothing: for example, call [`zmidi_pe_write_pending`] after a full queue and call again.
///
/// # Safety
/// `pe` and `ci` were set up by their init functions; the message of the ci event is still valid;
/// `scratch` points to `scratch_len` writable bytes (or is null when `scratch_len` is 0); `ev` is
/// writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_receive(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    now_us: u64,
    scratch: *mut u8,
    scratch_len: usize,
    ev: *mut ZmidiPeEvent,
) -> ZmidiStatus {
    if ev.is_null() || (scratch.is_null() && scratch_len > 0) {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let mut e = ZmidiPeEvent::NONE;
    let event = match c.last {
        Some(
            event @ (Event::PeChunk { .. }
            | Event::Ack { .. }
            | Event::Nak { .. }
            | Event::PeerInvalidated { .. }
            | Event::MuidChanged { .. }),
        ) => event,
        _ => {
            // SAFETY: caller contract above.
            unsafe { ev.write(e) };
            return ZmidiStatus::Ok;
        }
    };
    let raw: &[u8] = match event {
        Event::PeChunk { chunk, .. } => chunk.header,
        _ => &[],
    };
    let scratch: &mut [u8] = if scratch_len == 0 {
        &mut []
    } else {
        // SAFETY: caller contract above.
        unsafe { core::slice::from_raw_parts_mut(scratch, scratch_len) }
    };
    match s.eng.receive(&mut c.dev, &event, ms(now_us), scratch) {
        Ok(r) => {
            c.last = None;
            // REQ-FFI-CI-021: the engine closed the request, and no other request came between.
            #[cfg(feature = "ci-initiator")]
            c.end_reply();
            if let Some(r) = r {
                fill(&mut e, &r, raw);
            }
            // SAFETY: caller contract above.
            unsafe { ev.write(e) };
            ZmidiStatus::Ok
        }
        Err(err) => {
            // SAFETY: caller contract above.
            unsafe { ev.write(e) };
            refuse(s, err)
        }
    }
}

/// Runs the deadlines of the engine at `now_us` and writes one event into `ev` (kind None when
/// there is none); call it until it gives None (REQ-FFI-PE-003).
///
/// # Safety
/// `pe` and `ci` were set up by their init functions; `ev` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_poll(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    now_us: u64,
    ev: *mut ZmidiPeEvent,
) -> ZmidiStatus {
    if ev.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let mut e = ZmidiPeEvent::NONE;
    if let Some(r) = s.eng.poll(&mut c.dev, ms(now_us)) {
        fill(&mut e, &r, &[]);
    }
    // SAFETY: caller contract above.
    unsafe { ev.write(e) };
    ZmidiStatus::Ok
}

/// Writes the next queued control message (status reply, ACK, NAK, the Get after a "notify") as a
/// record into `out` (REQ-FFI-PE-003). With no queued message it gives no-pending. `capacity`
/// must be at least [`ZMIDI_PE_PENDING_MAX`] bytes.
///
/// # Safety
/// `pe` and `ci` were set up by their init functions; `out` points to `capacity` writable bytes;
/// `out_len` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_write_pending(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    now_us: u64,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || out_len.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // The engine takes the message from its queue before it writes it.
    if capacity < ZMIDI_PE_PENDING_MAX {
        return ZmidiStatus::BufferTooSmall;
    }
    // SAFETY: caller contract above.
    unsafe {
        out_len.write(0);
        record(out, capacity, out_len, |body| {
            match s.eng.write_pending(&mut c.dev, ms(now_us), body) {
                None => Err(ZmidiStatus::NoPending),
                Some(Ok(n)) => Ok(n),
                Some(Err(e)) => Err(refuse(s, e)),
            }
        })
    }
}

/// Starts the reply to the complete inquiry `request_id` of `peer` (REQ-FFI-PE-004): `status`
/// (200: success), the [`ZmidiPeEncoding`] `encoding` (the encoding of the inquiry), `media_type`
/// (null for JSON), `total_count` and `cache_time` (-1 when absent) and `data_len` bytes of
/// encoded Property Data (`SIZE_MAX` when not known). [`zmidi_pe_send`] writes the Chunks. A reply
/// after the deadline gives rejected and closes the transaction.
///
/// # Safety
/// `pe` and `ci` were set up by their init functions; `media_type` is null or a NUL-terminated
/// string.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_pe_reply(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    peer: u32,
    request_id: u8,
    status: u16,
    encoding_value: u8,
    media_type: *const u8,
    total_count: i64,
    cache_time: i64,
    data_len: usize,
    now_us: u64,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let (Ok(peer), Some(enc), Some(mt)) = (
        Muid::new(peer),
        encoding(encoding_value),
        // SAFETY: caller contract above.
        unsafe { text(media_type, 75) },
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    let opt = |v: i64| u32::try_from(v).ok();
    if (total_count != -1 && opt(total_count).is_none())
        || (cache_time != -1 && opt(cache_time).is_none())
    {
        return ZmidiStatus::InvalidArgument;
    }
    let msg = Outgoing {
        header: Header {
            status: Some(Status(status)),
            media_type: mt.map(HeaderStr::Text),
            total_count: opt(total_count),
            cache_time: opt(cache_time),
            ..Default::default()
        },
        encoding: enc,
        json: mt.is_none(),
        total_len: (data_len != usize::MAX).then_some(data_len),
    };
    match s.eng.reply(&mut c.dev, peer, request_id, &msg, ms(now_us)) {
        Ok(()) => ZmidiStatus::Ok,
        Err(e) => refuse(s, e),
    }
}

/// Replies to the open inquiry `request_id` of `peer` with `status` and no Property Data, and
/// closes the transaction (REQ-FFI-PE-004). [`zmidi_pe_write_pending`] writes the reply.
///
/// # Safety
/// `pe` and `ci` were set up by their init functions.
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_reply_status(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    peer: u32,
    request_id: u8,
    status: u16,
    now_us: u64,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Ok(peer) = Muid::new(peer) else {
        return ZmidiStatus::InvalidArgument;
    };
    match s
        .eng
        .reply_status(&mut c.dev, peer, request_id, Status(status), ms(now_us))
    {
        Ok(()) => ZmidiStatus::Ok,
        Err(e) => refuse(s, e),
    }
}

/// Starts a "partial", "full" or "notify" ([`ZmidiPeCommand`]) Subscription message for the
/// subscription `index` in which this Device is the responder, with `data_len` bytes of encoded
/// Property Data (`SIZE_MAX` when not known), and writes its Request ID (REQ-FFI-PE-004).
/// [`zmidi_pe_send`] writes the Chunks. A responder-only build (no `ci-initiator`) sends it too
/// (M2-101 8.11, DEC-CI-001).
///
/// # Safety
/// `pe` and `ci` were set up by their init functions; `request_id` is writable.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_pe_update(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    index: usize,
    command: u8,
    encoding_value: u8,
    data_len: usize,
    now_us: u64,
    request_id: *mut u8,
) -> ZmidiStatus {
    if request_id.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let cmd = match command {
        2 => Command::Partial,
        3 => Command::Full,
        4 => Command::Notify,
        _ => return ZmidiStatus::InvalidArgument,
    };
    let Some(enc) = encoding(encoding_value) else {
        return ZmidiStatus::InvalidArgument;
    };
    let len = (data_len != usize::MAX).then_some(data_len);
    match s.eng.update(&mut c.dev, index, cmd, enc, len, ms(now_us)) {
        Ok(info) => {
            // SAFETY: caller contract above.
            unsafe { request_id.write(info.request_id) };
            ZmidiStatus::Ok
        }
        Err(e) => refuse(s, e),
    }
}

/// Writes the ResourceList JSON of the Resource table into `out` (`capacity` bytes) and its length
/// into `out_len`, for the reply to a Get of "ResourceList" (REQ-FFI-PE-004).
///
/// # Safety
/// `pe` was set up by [`zmidi_pe_init`]; `out` points to `capacity` writable bytes; `out_len` is
/// writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_resource_list(
    pe: *mut ZmidiPe,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || out_len.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { pe_state(pe) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let out = unsafe { core::slice::from_raw_parts_mut(out, capacity) };
    let mut w = s.eng.resource_list::<ENTRY_MAX>();
    let mut at = 0;
    while !w.is_done() {
        match w.write_next(&mut out[at..]) {
            Ok(0) if !w.is_done() => return ZmidiStatus::BufferTooSmall,
            Ok(n) => at += n,
            Err(_) => return ZmidiStatus::BufferTooSmall,
        }
    }
    // SAFETY: caller contract above.
    unsafe { out_len.write(at) };
    ZmidiStatus::Ok
}

/// Gives the Header of `h` with its encoding, its length and its JSON flag, or `None` for a value
/// that is not valid.
///
/// # Safety
/// `h` is valid; its texts are null or NUL-terminated.
#[cfg(feature = "ci-initiator")]
unsafe fn header_of(h: &ZmidiPeHeader) -> Option<(Header<'static>, EncodingName, Option<usize>)> {
    // SAFETY: caller contract above.
    let (Some(Some(resource)), Some(res_id), Some(mt)) = (
        unsafe { text(h.resource, 36) },
        unsafe { text(h.res_id, 36) },
        unsafe { text(h.media_type, 75) },
    ) else {
        return None;
    };
    let header = Header {
        resource: Some(HeaderStr::Text(resource)),
        res_id: res_id.map(HeaderStr::Text),
        media_type: mt.map(HeaderStr::Text),
        offset: h.has_offset.then_some(h.offset),
        limit: h.has_offset.then_some(h.limit),
        set_partial: h.set_partial,
        ..Default::default()
    };
    let len = (h.length != usize::MAX).then_some(h.length);
    Some((header, encoding(h.encoding)?, len))
}

/// Starts a Get (`kind` 0) or Set (`kind` 1) inquiry to `peer` from `header` (REQ-FFI-PE-005).
/// `caps` are the capabilities of the Resource in the ResourceList of the peer, or null: a request
/// that they do not permit gives rejected with the status in [`zmidi_pe_last_error`]. It writes the
/// Request ID and whether this Device has no ResourceList of the peer yet. [`zmidi_pe_send`] writes
/// the Chunks.
///
/// # Safety
/// `pe` and `ci` were set up by their init functions; `header` is valid; `caps` is null or valid;
/// `request_id` and `list_missing` are writable.
#[cfg(feature = "ci-initiator")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_pe_request(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    peer: u32,
    kind: u8,
    header: *const ZmidiPeHeader,
    caps: *const ZmidiPeResource,
    now_us: u64,
    request_id: *mut u8,
    list_missing: *mut bool,
) -> ZmidiStatus {
    if request_id.is_null() || list_missing.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let Some(h) = (unsafe { header.as_ref() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let kind = match kind {
        0 => TxnKind::Get,
        1 => TxnKind::Set,
        _ => return ZmidiStatus::InvalidArgument,
    };
    // SAFETY: caller contract above.
    let (Ok(peer), Some((hdr, enc, len)), Some(caps)) =
        (Muid::new(peer), unsafe { header_of(h) }, unsafe {
            caps_at(caps)
        })
    else {
        return ZmidiStatus::InvalidArgument;
    };
    let msg = Outgoing {
        json: hdr.media_type.is_none(),
        header: hdr,
        encoding: enc,
        total_len: len,
    };
    match s
        .eng
        .request(&mut c.dev, peer, kind, &msg, caps.as_ref(), ms(now_us))
    {
        Ok(info) => {
            // SAFETY: caller contract above.
            unsafe {
                request_id.write(info.request_id);
                list_missing.write(info.resource_list_missing);
            }
            ZmidiStatus::Ok
        }
        Err(e) => refuse(s, e),
    }
}

/// Starts the Get of the ResourceList of `peer`, writes it as a record into `out` and writes its
/// Request ID (REQ-FFI-PE-005).
///
/// # Safety
/// `pe` and `ci` were set up by their init functions; `out` points to `capacity` writable bytes;
/// `out_len` and `request_id` are writable.
#[cfg(feature = "ci-initiator")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_pe_request_resource_list(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    peer: u32,
    now_us: u64,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
    request_id: *mut u8,
) -> ZmidiStatus {
    if out.is_null() || out_len.is_null() || request_id.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Ok(peer) = Muid::new(peer) else {
        return ZmidiStatus::InvalidArgument;
    };
    // The engine opens the transaction before it writes the Chunk: a Chunk with no data needs the
    // MIDI-CI header, the Chunk fields, the framing and the header of 25 bytes.
    if capacity < 3 + 2 + 22 + 32 {
        return ZmidiStatus::BufferTooSmall;
    }
    // SAFETY: caller contract above.
    unsafe {
        record(out, capacity, out_len, |body| {
            match s
                .eng
                .request_resource_list(&mut c.dev, peer, ms(now_us), body)
            {
                Ok((id, n)) => {
                    request_id.write(id);
                    Ok(n)
                }
                Err(e) => Err(refuse(s, e)),
            }
        })
    }
}

/// Starts a "start" Subscription message to `peer` with the request Properties of `header`
/// (REQ-FFI-PE-005, REQ-PE-SUB-006) and writes its Request ID and whether this Device has no
/// ResourceList of the peer yet. [`zmidi_pe_send`] writes it with no data.
///
/// # Safety
/// `pe` and `ci` were set up by their init functions; `header` is valid; `caps` is null or valid;
/// `request_id` and `list_missing` are writable.
#[cfg(feature = "ci-initiator")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_pe_subscribe(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    peer: u32,
    header: *const ZmidiPeHeader,
    caps: *const ZmidiPeResource,
    now_us: u64,
    request_id: *mut u8,
    list_missing: *mut bool,
) -> ZmidiStatus {
    if request_id.is_null() || list_missing.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let Some(h) = (unsafe { header.as_ref() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let (Ok(peer), Some((hdr, enc, _)), Some(caps)) =
        (Muid::new(peer), unsafe { header_of(h) }, unsafe {
            caps_at(caps)
        })
    else {
        return ZmidiStatus::InvalidArgument;
    };
    let hdr = Header {
        mutual_encoding: (enc != EncodingName::Ascii).then_some(enc),
        ..hdr
    };
    match s
        .eng
        .subscribe(&mut c.dev, peer, &hdr, caps.as_ref(), ms(now_us))
    {
        Ok(info) => {
            // SAFETY: caller contract above.
            unsafe {
                request_id.write(info.request_id);
                list_missing.write(info.resource_list_missing);
            }
            ZmidiStatus::Ok
        }
        Err(e) => refuse(s, e),
    }
}

/// Starts an "end" Subscription message for the subscription `index`, removes the entry and
/// writes the Request ID (REQ-FFI-PE-005). [`zmidi_pe_send`] writes it with no data.
///
/// # Safety
/// `pe` and `ci` were set up by their init functions; `request_id` is writable.
#[cfg(feature = "ci-initiator")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_unsubscribe(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    index: usize,
    now_us: u64,
    request_id: *mut u8,
) -> ZmidiStatus {
    if request_id.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    match s.eng.unsubscribe(&mut c.dev, index, ms(now_us)) {
        Ok((info, _)) => {
            // SAFETY: caller contract above.
            unsafe { request_id.write(info.request_id) };
            ZmidiStatus::Ok
        }
        Err(e) => refuse(s, e),
    }
}

/// Asks again for the Property Data of the subscription `index`, after an update that the program
/// can not apply (REQ-FFI-PE-005, REQ-PE-SUB-016). [`zmidi_pe_write_pending`] writes the Get.
///
/// # Safety
/// `pe` was set up by [`zmidi_pe_init`].
#[cfg(feature = "ci-initiator")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_refresh(pe: *mut ZmidiPe, index: usize) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { pe_state(pe) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    match s.eng.refresh(index) {
        Ok(()) => ZmidiStatus::Ok,
        Err(e) => refuse(s, e),
    }
}

/// Writes into `room` the Property Data bytes that the next Chunk of the transaction
/// (`peer`, `request_id`, `role`: 0 initiator, 1 responder) takes (REQ-FFI-PE-006).
///
/// # Safety
/// `pe` and `ci` were set up by their init functions; `room` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_data_room(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    peer: u32,
    request_id: u8,
    role: u8,
    room: *mut usize,
) -> ZmidiStatus {
    if room.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let (Ok(peer), Some(role)) = (Muid::new(peer), direction(role)) else {
        return ZmidiStatus::InvalidArgument;
    };
    match s.eng.data_room(&c.dev, peer, request_id, role) {
        Ok(n) => {
            // SAFETY: caller contract above.
            unsafe { room.write(n) };
            ZmidiStatus::Ok
        }
        Err(e) => refuse(s, e),
    }
}

/// Writes the next Chunk of the transaction (`peer`, `request_id`, `role`) as a record into `out`,
/// with `part` (`part_len` bytes of encoded Property Data, at most the data room) as its Property
/// Data; `last` ends the message (REQ-FFI-PE-006). A refusal changes nothing.
///
/// # Safety
/// `pe` and `ci` were set up by their init functions; `part` points to `part_len` bytes (or is null
/// when `part_len` is 0); `out` points to `capacity` writable bytes; `out_len` is writable.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_pe_send(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    peer: u32,
    request_id: u8,
    role: u8,
    part: *const u8,
    part_len: usize,
    last: bool,
    now_us: u64,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || out_len.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let (Ok(peer), Some(role), Some(part)) = (Muid::new(peer), direction(role), unsafe {
        ci::bytes(part, part_len)
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        record(out, capacity, out_len, |body| {
            s.eng
                .send(
                    &mut c.dev,
                    peer,
                    request_id,
                    role,
                    part,
                    last,
                    ms(now_us),
                    body,
                )
                .map_err(|e| refuse(s, e))
        })
    }
}

/// Asks the peer to wait `wait_ms` more milliseconds (1 to 12700) with a MIDI-CI ACK 0x10
/// (REQ-FFI-PE-006, REQ-PE-END-010). [`zmidi_pe_write_pending`] writes the ACK.
///
/// # Safety
/// `pe` was set up by [`zmidi_pe_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_wait(
    pe: *mut ZmidiPe,
    peer: u32,
    request_id: u8,
    role: u8,
    wait_ms: u32,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { pe_state(pe) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let (Ok(peer), Some(role)) = (Muid::new(peer), direction(role)) else {
        return ZmidiStatus::InvalidArgument;
    };
    match s.eng.wait(peer, request_id, role, u64::from(wait_ms)) {
        Ok(()) => ZmidiStatus::Ok,
        Err(e) => refuse(s, e),
    }
}

/// Ends the transaction (`peer`, `request_id`, `role`) with a MIDI-CI NAK 0x20 (REQ-FFI-PE-006).
/// [`zmidi_pe_write_pending`] writes the NAK.
///
/// # Safety
/// `pe` and `ci` were set up by their init functions.
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_abort(
    pe: *mut ZmidiPe,
    ci: *mut ZmidiCi,
    peer: u32,
    request_id: u8,
    role: u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some((s, c)) = (unsafe { states(pe, ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let (Ok(peer), Some(role)) = (Muid::new(peer), direction(role)) else {
        return ZmidiStatus::InvalidArgument;
    };
    match s.eng.abort(&mut c.dev, peer, request_id, role) {
        Ok(()) => ZmidiStatus::Ok,
        Err(e) => refuse(s, e),
    }
}

/// The last received Chunk of the transaction (`peer`, `request_id`, `role`) is corrupt: with Flow
/// Control the engine asks for it again with a NAK 0x12 (REQ-FFI-PE-006, REQ-PE-FLOW-011).
/// [`zmidi_pe_write_pending`] writes the NAK.
///
/// # Safety
/// `pe` was set up by [`zmidi_pe_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_corrupt(
    pe: *mut ZmidiPe,
    peer: u32,
    request_id: u8,
    role: u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { pe_state(pe) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let (Ok(peer), Some(role)) = (Muid::new(peer), direction(role)) else {
        return ZmidiStatus::InvalidArgument;
    };
    match s.eng.corrupt(peer, request_id, role) {
        Ok(()) => ZmidiStatus::Ok,
        Err(e) => refuse(s, e),
    }
}

/// Copies `e` into `out`.
///
/// # Safety
/// `out` is writable.
unsafe fn write_entry(e: &SubscriptionEntry, out: *mut ZmidiPeSubscription) {
    let mut c = ZmidiPeSubscription {
        muid: 0,
        role: ZmidiPeRole::Initiator,
        subscribe_id: [0; ZMIDI_PE_ID_LEN],
        resource: [0; ZMIDI_PE_ID_LEN],
        res_id: [0; ZMIDI_PE_ID_LEN],
    };
    fill_entry(
        e,
        &mut c.muid,
        &mut c.role,
        [&mut c.subscribe_id, &mut c.resource, &mut c.res_id],
    );
    // SAFETY: caller contract above.
    unsafe { out.write(c) };
}

/// Copies the subscription entry `index` into `out` (REQ-FFI-PE-007). An empty index or an index
/// outside the table gives no-pending.
///
/// # Safety
/// `pe` was set up by [`zmidi_pe_init`]; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_subscription(
    pe: *mut ZmidiPe,
    index: usize,
    out: *mut ZmidiPeSubscription,
) -> ZmidiStatus {
    if out.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { pe_state(pe) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    match s.eng.subscriptions().get(index) {
        Some(e) => {
            // SAFETY: caller contract above.
            unsafe { write_entry(e, out) };
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::NoPending,
    }
}

/// Copies the next subscription that an Invalidate MUID removed into `out` (REQ-FFI-PE-007,
/// REQ-PE-SUB-019). With none it gives no-pending.
///
/// # Safety
/// `pe` was set up by [`zmidi_pe_init`]; `out` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_take_ended(
    pe: *mut ZmidiPe,
    out: *mut ZmidiPeSubscription,
) -> ZmidiStatus {
    if out.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { pe_state(pe) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    match s.eng.take_ended() {
        Some(e) => {
            // SAFETY: caller contract above.
            unsafe { write_entry(&e, out) };
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::NoPending,
    }
}

/// Gives the reason of the refusal of the last engine call (None when it did its work), and
/// writes into `status` (when not null) the status of a refusal by the ResourceList of the peer
/// (REQ-FFI-PE-008). A null or moved `pe` gives None.
///
/// # Safety
/// `pe` is null or was set up by [`zmidi_pe_init`]; `status` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_pe_last_error(pe: *const ZmidiPe, status: *mut u16) -> ZmidiPeError {
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { pe.cast::<PeState>().as_ref() }) else {
        return ZmidiPeError::None;
    };
    if s.home != pe as usize {
        return ZmidiPeError::None;
    }
    if !status.is_null() {
        // SAFETY: caller contract above.
        unsafe { status.write(s.refused) };
    }
    s.error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_map_to_reasons_and_statuses() {
        // REQ-FFI-PE-008: each refusal of the engine, also those that the C calls can not reach
        // ("$ref", "mediaType"), gives its reason and status.
        use TransactionError as T;
        use ZmidiPeError as E;
        use ZmidiStatus as S;
        let cases = [
            (
                T::Device(DeviceError::NotSupported),
                E::Device,
                S::NotSupported,
            ),
            (
                T::Device(DeviceError::Write(WriteError::Overflow)),
                E::Device,
                S::BufferTooSmall,
            ),
            (T::UnknownTransaction, E::UnknownTransaction, S::Rejected),
            (T::Full, E::Full, S::Rejected),
            (T::PeerBusy, E::PeerBusy, S::Rejected),
            (T::Late, E::Late, S::Rejected),
            (T::State, E::State, S::Rejected),
            (T::Refused(Status(405)), E::Refused, S::Rejected),
            (T::NoMediaType, E::NoMediaType, S::InvalidArgument),
            (T::NoTotalCount, E::NoTotalCount, S::InvalidArgument),
            (T::HeaderTooLarge, E::HeaderTooLarge, S::InvalidArgument),
            (T::Size, E::Size, S::BufferTooSmall),
            (T::QueueFull, E::QueueFull, S::Rejected),
            (T::InvalidWait, E::InvalidWait, S::InvalidArgument),
            (T::InvalidRef, E::InvalidRef, S::InvalidArgument),
        ];
        for (e, error, status) in cases {
            assert_eq!(refusal(&e), (error, status), "{e:?}");
        }
    }
}
