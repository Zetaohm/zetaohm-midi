//! The Property Exchange engine of zmidi-ci-pe (M2-103-UM v1.2 sections 5 to 15): Property Exchange
//! with many Chunks, Flow Control and Subscriptions over the `CiDevice` of the program.
//!
//! - The program gives each received message to `CiDevice.receive` first, then calls
//!   `PeEngine.receive` with the same device; the engine reads the last event of the device.
//! - The start calls (reply, update, request, subscribe, unsubscribe) start a transaction; `send`
//!   writes its Chunks. The calls that queue a control message (reply_status, refresh, wait, abort,
//!   corrupt) need `write_pending`.
//! - The sizes are those of the C ABI (DEC-019 addendum of 2026-10-23): 4 transactions with 256
//!   header bytes, 4 subscriptions, 4 peers with a known ResourceList, 16 queued control messages.
//! - The encodings are "ASCII" and "Mcoded7"; the engine does not inflate "zlib+Mcoded7" (zlib
//!   window 0). The program encodes the Property Data that it sends.
//! - A program uses the engine or `CiDevice.pe_reply` for its replies, not both. There are no
//!   callbacks (DEC-019).

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use zmidi_ci::{ChunkEnd, Direction, Event, Muid, NakStatus};
use zmidi_ci_pe::header::{Command, EncodingName, Header, HeaderStr, Status};
use zmidi_ci_pe::resource_list::{CanSet, Encodings};
use zmidi_ci_pe::{
    CloseReason, EngineConfig, Outgoing, PeEngine as RsEngine, PeEvent as RsEvent, ResourceCaps,
    ResourceEntry, Role, SubscriptionEntry, SubscriptionError, TransactionError, TxnKind,
};

use crate::ci::CiDevice;
use crate::{invalid, rejected};

type Engine = RsEngine<'static, 4, 256, 4, 4, 16>;

/// Bytes of the JSON of one ResourceList entry (with its "schema").
const ENTRY_MAX: usize = 512;
/// Bytes of a message that the engine writes, without its Property Data: the MIDI-CI header, the
/// Chunk fields, the SysEx framing and the 256 header bytes.
const MESSAGE_MAX: usize = 22 + 2 + 256;
/// Characters of a decoded "resource", "resId" or "subscribeId" (M2-103 8.1, 8.2).
const ID_MAX: usize = 36;

/// One entry of the Resource table of a `PeEngine` (M2-103 Table 14).
#[pyclass(frozen, module = "zetaohm_midi._native")]
#[derive(Debug, Clone)]
pub struct PeResource {
    /// "resource": 1 to 36 characters A-Z, a-z, 0-9 with an optional "X-" prefix.
    #[pyo3(get)]
    resource: String,
    /// "canGet".
    #[pyo3(get)]
    can_get: bool,
    /// "canSet": "none", "full" or "partial".
    #[pyo3(get)]
    can_set: String,
    /// "canSubscribe".
    #[pyo3(get)]
    can_subscribe: bool,
    /// "requireResId".
    #[pyo3(get)]
    require_res_id: bool,
    /// "fcOnGet".
    #[pyo3(get)]
    fc_on_get: bool,
    /// "fcOnSet".
    #[pyo3(get)]
    fc_on_set: bool,
    /// "encodings": "ASCII", "Mcoded7", "zlib+Mcoded7".
    #[pyo3(get)]
    encodings: Vec<String>,
    /// "canPaginate".
    #[pyo3(get)]
    can_paginate: bool,
    /// The one "mediaType", or `None` for "application/json".
    #[pyo3(get)]
    media_type: Option<String>,
    /// "schema" as JSON text.
    schema: Option<Vec<u8>>,
    /// Write every Property, also the ones with their default value (REQ-PE-LIST-009).
    #[pyo3(get)]
    write_defaults: bool,
}

#[pymethods]
impl PeResource {
    /// A Resource entry. Raises `InvalidValueError` for a "resource" that M2-103 8.1 does not
    /// permit, a "canSet" or an encoding that M2-103 does not name, no encoding, or a "mediaType"
    /// that is not "type/subtype".
    #[new]
    #[pyo3(signature = (resource, can_get=true, can_set="none", can_subscribe=false,
        require_res_id=false, fc_on_get=false, fc_on_set=false, encodings=vec!["ASCII".to_string()],
        can_paginate=false, media_type=None, schema=None, write_defaults=false))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        resource: String,
        can_get: bool,
        can_set: &str,
        can_subscribe: bool,
        require_res_id: bool,
        fc_on_get: bool,
        fc_on_set: bool,
        encodings: Vec<String>,
        can_paginate: bool,
        media_type: Option<String>,
        schema: Option<Vec<u8>>,
        write_defaults: bool,
    ) -> PyResult<Self> {
        let r = Self {
            resource,
            can_get,
            can_set: can_set.to_string(),
            can_subscribe,
            require_res_id,
            fc_on_get,
            fc_on_set,
            encodings,
            can_paginate,
            media_type,
            schema,
            write_defaults,
        };
        if !resource_ok(&r.resource) {
            return Err(invalid(
                "a resource has 1 to 36 characters A-Z, a-z, 0-9 (M2-103 8.1)",
            ));
        }
        if r.media_type.as_deref().is_some_and(|m| !media_ok(m)) {
            return Err(invalid("a mediaType is \"type/subtype\" (M2-103 8.6.1)"));
        }
        r.caps()?;
        Ok(r)
    }

    /// "schema" as JSON text, or `None`.
    #[getter]
    fn schema<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.schema.as_deref().map(|s| PyBytes::new(py, s))
    }

    fn __repr__(&self) -> String {
        format!("PeResource(resource={:?})", self.resource)
    }
}

impl PeResource {
    /// The capabilities of the entry.
    fn caps(&self) -> PyResult<ResourceCaps> {
        let can_set = match self.can_set.as_str() {
            "none" => CanSet::None,
            "full" => CanSet::Full,
            "partial" => CanSet::Partial,
            _ => return Err(invalid("canSet is \"none\", \"full\" or \"partial\"")),
        };
        let mut encodings = Encodings {
            ascii: false,
            mcoded7: false,
            zlib_mcoded7: false,
        };
        for e in &self.encodings {
            match encoding(e)? {
                EncodingName::Ascii => encodings.ascii = true,
                EncodingName::Mcoded7 => encodings.mcoded7 = true,
                EncodingName::ZlibMcoded7 => encodings.zlib_mcoded7 = true,
            }
        }
        if self.encodings.is_empty() {
            return Err(invalid("a Resource has at least one encoding"));
        }
        Ok(ResourceCaps {
            can_get: self.can_get,
            can_set,
            can_subscribe: self.can_subscribe,
            require_res_id: self.require_res_id,
            fc_on_get: self.fc_on_get,
            fc_on_set: self.fc_on_set,
            encodings,
            can_paginate: self.can_paginate,
        })
    }
}

/// M2-103 8.1: 1 to 36 characters A-Z, a-z, 0-9 with an optional "X-" prefix.
fn resource_ok(r: &str) -> bool {
    let body = r.strip_prefix("X-").unwrap_or(r);
    r.len() <= ID_MAX && !body.is_empty() && body.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// RFC 6838 "type/subtype" with restricted-name characters (M2-103 8.6.1), at most 75 characters.
fn media_ok(m: &str) -> bool {
    let name = |s: &str| {
        !s.is_empty()
            && s.as_bytes()[0].is_ascii_alphanumeric()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&b))
    };
    m.len() <= 75 && matches!(m.split_once('/'), Some((t, st)) if name(t) && name(st))
}

/// The encoding of its M2-103 Table 12 name.
fn encoding(e: &str) -> PyResult<EncodingName> {
    match e {
        "ASCII" => Ok(EncodingName::Ascii),
        "Mcoded7" => Ok(EncodingName::Mcoded7),
        "zlib+Mcoded7" => Ok(EncodingName::ZlibMcoded7),
        _ => Err(invalid(
            "an encoding is \"ASCII\", \"Mcoded7\" or \"zlib+Mcoded7\"",
        )),
    }
}

fn peer(muid: u32) -> PyResult<Muid> {
    Muid::new(muid).map_err(|_| invalid("a MUID has 28 bits"))
}

fn direction(role: &str) -> PyResult<Direction> {
    match role {
        "initiator" => Ok(Direction::Initiator),
        "responder" => Ok(Direction::Responder),
        _ => Err(invalid("a role is \"initiator\" or \"responder\"")),
    }
}

/// The Python error of a refusal of the engine.
fn refuse(e: TransactionError) -> PyErr {
    use TransactionError as T;
    match e {
        T::Header(_)
        | T::NoMediaType
        | T::NoTotalCount
        | T::HeaderTooLarge
        | T::Size
        | T::InvalidWait
        | T::InvalidRef => invalid(e.to_string()),
        T::Refused(s) => rejected(format!(
            "refused by the ResourceList of the peer: status {}",
            s.0
        )),
        _ => rejected(e.to_string()),
    }
}

/// The decoded characters of a header text.
fn text(s: Option<HeaderStr<'_>>) -> Option<String> {
    s.map(|s| {
        let mut buf = [0u8; 4 * ID_MAX];
        s.decode_into(&mut buf).0.to_string()
    })
}

const fn role_name(d: Direction) -> &'static str {
    match d {
        Direction::Initiator => "initiator",
        Direction::Responder => "responder",
    }
}

const fn txn_name(k: TxnKind) -> &'static str {
    match k {
        TxnKind::Get => "get",
        TxnKind::Set => "set",
        TxnKind::Subscription => "subscription",
    }
}

const fn command_name(c: Command) -> &'static str {
    match c {
        Command::Start => "start",
        Command::Partial => "partial",
        Command::Full => "full",
        Command::Notify => "notify",
        Command::End => "end",
    }
}

const fn sub_error_name(e: SubscriptionError) -> &'static str {
    match e {
        SubscriptionError::Full => "full",
        SubscriptionError::NoSubscribeId => "no_subscribe_id",
        SubscriptionError::NotSuccess => "not_success",
        SubscriptionError::NotPermitted => "not_permitted",
        SubscriptionError::Invalid => "invalid",
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

/// One entry of the subscription table of a `PeEngine`.
#[pyclass(frozen, module = "zetaohm_midi._native")]
#[derive(Debug, Clone)]
pub struct PeSubscription {
    /// The MUID of the peer.
    #[pyo3(get)]
    muid: u32,
    /// The role of this Device: "initiator" (it sent the "start") or "responder".
    #[pyo3(get)]
    role: String,
    /// "subscribeId".
    #[pyo3(get)]
    subscribe_id: String,
    /// "resource".
    #[pyo3(get)]
    resource: String,
    /// "resId", or `None`.
    #[pyo3(get)]
    res_id: Option<String>,
}

#[pymethods]
impl PeSubscription {
    fn __repr__(&self) -> String {
        format!(
            "PeSubscription(muid={:#x}, role={:?}, resource={:?})",
            self.muid, self.role, self.resource
        )
    }
}

fn subscription(e: &SubscriptionEntry) -> PeSubscription {
    PeSubscription {
        muid: e.peer().value(),
        role: match e.role() {
            Role::Initiator => "initiator",
            Role::Responder => "responder",
        }
        .to_string(),
        subscribe_id: e.subscribe_id().to_string(),
        resource: e.resource().to_string(),
        res_id: e.res_id().map(str::to_string),
    }
}

/// One event of a `PeEngine`. It cannot change after it is made; the properties that the kind
/// does not use are `None` or empty.
#[pyclass(frozen, module = "zetaohm_midi._native")]
#[derive(Debug, Clone, Default)]
pub struct PeEvent {
    /// "message", "ready", "resend", "closed", "subscription_started", "subscription_ended" or
    /// "invalidated".
    #[pyo3(get)]
    kind: String,
    /// The MUID of the peer ("invalidated": the invalidated MUID).
    #[pyo3(get)]
    muid: Option<u32>,
    /// Request ID of the transaction.
    #[pyo3(get)]
    request_id: Option<u8>,
    /// The role of this Device: "initiator" or "responder".
    #[pyo3(get)]
    role: Option<String>,
    /// The kind of the transaction: "get", "set" or "subscription".
    #[pyo3(get)]
    txn: Option<String>,
    /// "message": "more", "complete" or "incomplete" (M2-103 5.3).
    #[pyo3(get)]
    end: Option<String>,
    /// "message": the encoding of the Property Data.
    #[pyo3(get)]
    encoding: Option<String>,
    /// "message": the "command" of the header.
    #[pyo3(get)]
    command: Option<String>,
    /// "message": the "status" of the header; "closed" with reason "refused": the status.
    #[pyo3(get)]
    status: Option<u16>,
    /// "resend": the number of the Chunk to send again.
    #[pyo3(get)]
    chunk: Option<u16>,
    /// "message": the index of the Resource in the Resource table.
    #[pyo3(get)]
    resource_index: Option<usize>,
    /// "message": the subscription entry of the message; "subscription_started": the new entry.
    #[pyo3(get)]
    subscription: Option<usize>,
    /// "message" of the reply to a "start": the new subscription entry.
    #[pyo3(get)]
    started: Option<usize>,
    /// "message" of the reply to a "start" with no new entry: why ("full", "no_subscribe_id",
    /// "not_success", "not_permitted", "invalid").
    #[pyo3(get)]
    start_error: Option<String>,
    /// "closed": "timeout", "terminated", "peer_timeout", "out_of_sequence", "nak", "bad_reply",
    /// "refused", "invalidated" or "resend_failed".
    #[pyo3(get)]
    reason: Option<String>,
    /// "closed" with reason "nak": the NAK Status Code.
    #[pyo3(get)]
    nak_code: Option<u8>,
    /// "closed" with reason "nak": the NAK Status Data.
    #[pyo3(get)]
    nak_data: Option<u8>,
    /// "invalidated": the number of removed subscriptions (see `take_ended`).
    #[pyo3(get)]
    ended: Option<usize>,
    /// "message": the decoded "resource"; "subscription_ended": the entry resource.
    #[pyo3(get)]
    resource: Option<String>,
    /// "message": the decoded "resId"; "subscription_ended": the entry resId.
    #[pyo3(get)]
    res_id: Option<String>,
    /// "message": the decoded "subscribeId"; "subscription_ended": the entry subscribeId.
    #[pyo3(get)]
    subscribe_id: Option<String>,
    /// "message": the Header Data (first Chunk only).
    header: Vec<u8>,
    /// "message": the decoded Property Data of this Chunk.
    data: Vec<u8>,
}

#[pymethods]
impl PeEvent {
    /// "message": the Header Data (first Chunk only).
    #[getter]
    fn header<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.header)
    }

    /// "message": the decoded Property Data of this Chunk.
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }

    fn __repr__(&self) -> String {
        format!(
            "PeEvent(kind={:?}, muid={:?}, request_id={:?})",
            self.kind, self.muid, self.request_id
        )
    }
}

/// The Python event of `e`; `raw` is the Header Data of the Chunk.
fn event(e: &RsEvent<'_, '_>, raw: &[u8]) -> PeEvent {
    let mut out = PeEvent::default();
    let txn = |peer: Muid, request_id: u8, role: Direction| {
        (
            Some(peer.value()),
            Some(request_id),
            Some(role_name(role).to_string()),
        )
    };
    match *e {
        RsEvent::Message(m) => {
            out.kind = "message".into();
            (out.muid, out.request_id, out.role) = txn(m.peer, m.request_id, m.role);
            out.txn = Some(txn_name(m.kind).into());
            out.end = Some(
                match m.end {
                    ChunkEnd::More => "more",
                    ChunkEnd::Complete => "complete",
                    ChunkEnd::Incomplete => "incomplete",
                }
                .into(),
            );
            out.encoding = Some(m.encoding.as_str().into());
            out.resource_index = m.resource;
            out.subscription = m.subscription;
            match m.started {
                Some(Ok(i)) => out.started = Some(i),
                Some(Err(e)) => out.start_error = Some(sub_error_name(e).into()),
                None => {}
            }
            if let Some(h) = m.header {
                out.header = raw.to_vec();
                out.status = h.status.map(|s| s.0);
                out.command = h.command.map(|c| command_name(c).into());
                out.resource = text(h.resource);
                out.res_id = text(h.res_id);
                out.subscribe_id = text(h.subscribe_id);
            }
            out.data = m.data.to_vec();
        }
        RsEvent::Ready {
            peer,
            request_id,
            role,
        } => {
            out.kind = "ready".into();
            (out.muid, out.request_id, out.role) = txn(peer, request_id, role);
        }
        RsEvent::Resend {
            peer,
            request_id,
            role,
            chunk,
        } => {
            out.kind = "resend".into();
            (out.muid, out.request_id, out.role) = txn(peer, request_id, role);
            out.chunk = Some(chunk);
        }
        RsEvent::Closed {
            peer,
            request_id,
            role,
            reason,
        } => {
            out.kind = "closed".into();
            (out.muid, out.request_id, out.role) = txn(peer, request_id, role);
            let r = match reason {
                CloseReason::Timeout => "timeout",
                CloseReason::Terminated => "terminated",
                CloseReason::PeerTimeout => "peer_timeout",
                CloseReason::OutOfSequence => "out_of_sequence",
                CloseReason::Nak(n) => {
                    let (c, d) = nak_bytes(n);
                    out.nak_code = Some(c);
                    out.nak_data = Some(d);
                    "nak"
                }
                CloseReason::BadReply(_) => "bad_reply",
                CloseReason::Refused(s) => {
                    out.status = Some(s.0);
                    "refused"
                }
                CloseReason::Invalidated => "invalidated",
                CloseReason::ResendFailed => "resend_failed",
            };
            out.reason = Some(r.into());
        }
        RsEvent::SubscriptionStarted { index } => {
            out.kind = "subscription_started".into();
            out.subscription = Some(index);
        }
        RsEvent::SubscriptionEnded(e) => {
            let s = subscription(&e);
            out.kind = "subscription_ended".into();
            out.muid = Some(s.muid);
            out.role = Some(s.role);
            out.txn = Some("subscription".into());
            out.resource = Some(s.resource);
            out.res_id = s.res_id;
            out.subscribe_id = Some(s.subscribe_id);
        }
        RsEvent::Invalidated { muid, ended } => {
            out.kind = "invalidated".into();
            out.muid = Some(muid.value());
            out.ended = Some(ended);
        }
    }
    out
}

/// The Property Exchange engine of zmidi-ci-pe over a `CiDevice` (4 transactions, 256 header bytes,
/// 4 subscriptions, 4 peers with a known ResourceList, 16 queued control messages).
#[pyclass(module = "zetaohm_midi._native")]
pub struct PeEngine {
    // Declared first, so it is dropped before the table and the texts that it borrows.
    eng: Engine,
    _table: Box<[ResourceEntry<'static>]>,
    _media: Box<[[&'static str; 1]]>,
    _resources: Vec<PeResource>,
}

/// Gives `&'static` to data in a box that the returned engine owns.
///
/// # Safety
/// The data outlives every use of the reference: the owner drops the engine first.
unsafe fn forever<T: ?Sized>(r: &T) -> &'static T {
    // SAFETY: caller contract above.
    unsafe { &*(r as *const T) }
}

#[pymethods]
impl PeEngine {
    /// An engine with the Resource table `resources`. Raises `InvalidValueError` for an entry
    /// that does not fit the ResourceList writer (more than 512 bytes of JSON).
    #[new]
    #[pyo3(signature = (resources=Vec::new()))]
    fn new(resources: Vec<PyRef<'_, PeResource>>) -> PyResult<Self> {
        let resources: Vec<PeResource> = resources.iter().map(|r| (**r).clone()).collect();
        let media: Box<[[&'static str; 1]]> = resources
            .iter()
            // SAFETY: the strings are in `resources`, which the engine owns and drops after `eng`.
            .map(|r| {
                [r.media_type
                    .as_deref()
                    .map_or("", |m| unsafe { forever(m) })]
            })
            .collect();
        let mut table = Vec::with_capacity(resources.len());
        for (i, r) in resources.iter().enumerate() {
            // SAFETY: as above.
            let mut e = ResourceEntry::new(unsafe { forever(r.resource.as_str()) });
            e.caps = r.caps()?;
            // SAFETY: as above.
            e.schema = r.schema.as_deref().map(|s| unsafe { forever(s) });
            e.write_defaults = r.write_defaults;
            if r.media_type.is_some() {
                // SAFETY: `media` is a box that the engine owns and drops after `eng`.
                e.media_types = unsafe { forever(&media[i]) };
            }
            table.push(e);
        }
        let table = table.into_boxed_slice();
        let eng = Engine::new(EngineConfig {
            // SAFETY: as above.
            resources: unsafe { forever(&*table) },
            zlib_window: 0,
        });
        let mut w = eng.resource_list::<ENTRY_MAX>();
        let mut sink = [0u8; 64];
        while !w.is_done() {
            w.write_next(&mut sink)
                .map_err(|e| invalid(format!("a Resource entry: {e}")))?;
        }
        Ok(Self {
            eng,
            _table: table,
            _media: media,
            _resources: resources,
        })
    }

    /// Gives the last event of `dev` (from `CiDevice.receive`) to the engine at `now_us`. Gives
    /// the engine event, or `None`. An event that the engine took is not given again. A refusal
    /// (for example a full queue: call `write_pending` first) raises and changes nothing.
    fn receive(&mut self, dev: &Bound<'_, CiDevice>, now_us: u64) -> PyResult<Option<PeEvent>> {
        let mut dev = dev.borrow_mut();
        let ev = match dev.last {
            Some(
                e @ (Event::PeChunk { .. }
                | Event::Ack { .. }
                | Event::Nak { .. }
                | Event::PeerInvalidated { .. }
                | Event::MuidChanged { .. }),
            ) => e,
            _ => return Ok(None),
        };
        let (raw, size) = match ev {
            Event::PeChunk { chunk, .. } => (chunk.header, chunk.data.len()),
            _ => (&[][..], 0),
        };
        let mut scratch = vec![0u8; size];
        let r = self
            .eng
            .receive(&mut dev.dev, &ev, now_us / 1000, &mut scratch)
            .map_err(refuse)?;
        dev.last = None;
        // The engine closed the request, and no other request came between.
        dev.end_reply();
        Ok(r.map(|r| event(&r, raw)))
    }

    /// Runs the deadlines at `now_us`. Gives one event, or `None`; call it until it gives `None`.
    fn poll(&mut self, dev: &Bound<'_, CiDevice>, now_us: u64) -> Option<PeEvent> {
        let mut dev = dev.borrow_mut();
        self.eng
            .poll(&mut dev.dev, now_us / 1000)
            .map(|r| event(&r, &[]))
    }

    /// Writes the next queued control message (status reply, ACK, NAK, the Get after a "notify").
    /// Gives it, or `None` when the queue is empty.
    fn write_pending<'py>(
        &mut self,
        py: Python<'py>,
        dev: &Bound<'_, CiDevice>,
        now_us: u64,
    ) -> PyResult<Option<Bound<'py, PyBytes>>> {
        let mut dev = dev.borrow_mut();
        let mut buf = [0u8; MESSAGE_MAX];
        match self
            .eng
            .write_pending(&mut dev.dev, now_us / 1000, &mut buf)
        {
            None => Ok(None),
            Some(Ok(n)) => Ok(Some(PyBytes::new(py, &buf[..n]))),
            Some(Err(e)) => Err(refuse(e)),
        }
    }

    /// Starts the reply to the complete inquiry `request_id` of `peer`: `status`, the encoding of
    /// the inquiry, `media_type` (`None` for JSON), "totalCount", "cacheTime" and the byte count of
    /// the encoded Property Data (`None` when not known). `send` writes the Chunks. A reply after
    /// the deadline raises `RejectedError` and closes the transaction.
    #[pyo3(signature = (dev, peer, request_id, status=200, length=Some(0), encoding="ASCII", media_type=None,
        total_count=None, cache_time=None, now_us=0))]
    #[allow(clippy::too_many_arguments)]
    fn reply(
        &mut self,
        dev: &Bound<'_, CiDevice>,
        peer: u32,
        request_id: u8,
        status: u16,
        length: Option<usize>,
        encoding: &str,
        media_type: Option<String>,
        total_count: Option<u32>,
        cache_time: Option<u32>,
        now_us: u64,
    ) -> PyResult<()> {
        let mut dev = dev.borrow_mut();
        let msg = Outgoing {
            header: Header {
                status: Some(Status(status)),
                media_type: media_type.as_deref().map(HeaderStr::Text),
                total_count,
                cache_time,
                ..Default::default()
            },
            encoding: self::encoding(encoding)?,
            json: media_type.is_none(),
            total_len: length,
        };
        self.eng
            .reply(
                &mut dev.dev,
                self::peer(peer)?,
                request_id,
                &msg,
                now_us / 1000,
            )
            .map_err(refuse)
    }

    /// Replies to the open inquiry `request_id` of `peer` with `status` and no Property Data, and
    /// closes the transaction. `write_pending` writes the reply.
    #[pyo3(signature = (dev, peer, request_id, status, now_us=0))]
    fn reply_status(
        &mut self,
        dev: &Bound<'_, CiDevice>,
        peer: u32,
        request_id: u8,
        status: u16,
        now_us: u64,
    ) -> PyResult<()> {
        let mut dev = dev.borrow_mut();
        self.eng
            .reply_status(
                &mut dev.dev,
                self::peer(peer)?,
                request_id,
                Status(status),
                now_us / 1000,
            )
            .map_err(refuse)
    }

    /// Starts a "partial", "full" or "notify" Subscription message for the subscription `index` in
    /// which this Device is the responder, with `length` bytes of encoded Property Data (`None`
    /// when not known). Gives its Request ID; `send` (role "initiator") writes the Chunks. The
    /// device needs the PE Capabilities of the peer first.
    #[pyo3(signature = (dev, index, command, length=Some(0), encoding="ASCII", now_us=0))]
    fn update(
        &mut self,
        dev: &Bound<'_, CiDevice>,
        index: usize,
        command: &str,
        length: Option<usize>,
        encoding: &str,
        now_us: u64,
    ) -> PyResult<u8> {
        let command = match command {
            "partial" => Command::Partial,
            "full" => Command::Full,
            "notify" => Command::Notify,
            _ => return Err(invalid("an update is \"partial\", \"full\" or \"notify\"")),
        };
        let mut dev = dev.borrow_mut();
        self.eng
            .update(
                &mut dev.dev,
                index,
                command,
                self::encoding(encoding)?,
                length,
                now_us / 1000,
            )
            .map(|i| i.request_id)
            .map_err(refuse)
    }

    /// The ResourceList JSON of the Resource table, for the reply to a Get of "ResourceList".
    fn resource_list<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let mut w = self.eng.resource_list::<ENTRY_MAX>();
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        while !w.is_done() {
            let n = w.write_next(&mut buf).map_err(|e| invalid(e.to_string()))?;
            out.extend_from_slice(&buf[..n]);
        }
        Ok(PyBytes::new(py, &out))
    }

    /// Starts a Get (`kind` "get") or Set ("set") inquiry to `peer` with the request Properties
    /// "resource", "resId", "mediaType", "offset" and "limit", "setPartial", the encoding and the
    /// byte count of the encoded Property Data (`None` when not known). `caps` are the capabilities
    /// of the Resource in the ResourceList of the peer, or `None`. Gives `(request_id,
    /// resource_list_missing)`; `send` writes the Chunks.
    #[pyo3(signature = (dev, peer, kind, resource, res_id=None, media_type=None, offset=None, limit=None,
        set_partial=false, encoding="ASCII", length=Some(0), caps=None, now_us=0))]
    #[allow(clippy::too_many_arguments)]
    fn request(
        &mut self,
        dev: &Bound<'_, CiDevice>,
        peer: u32,
        kind: &str,
        resource: &str,
        res_id: Option<&str>,
        media_type: Option<&str>,
        offset: Option<u32>,
        limit: Option<u32>,
        set_partial: bool,
        encoding: &str,
        length: Option<usize>,
        caps: Option<PyRef<'_, PeResource>>,
        now_us: u64,
    ) -> PyResult<(u8, bool)> {
        let kind = match kind {
            "get" => TxnKind::Get,
            "set" => TxnKind::Set,
            _ => return Err(invalid("a request is \"get\" or \"set\"")),
        };
        if offset.is_some() != limit.is_some() {
            return Err(invalid("offset and limit come together (M2-103 8.5)"));
        }
        let caps = caps.map(|c| c.caps()).transpose()?;
        let msg = Outgoing {
            header: Header {
                resource: Some(HeaderStr::Text(resource)),
                res_id: res_id.map(HeaderStr::Text),
                media_type: media_type.map(HeaderStr::Text),
                offset,
                limit,
                set_partial,
                ..Default::default()
            },
            encoding: self::encoding(encoding)?,
            json: media_type.is_none(),
            total_len: length,
        };
        let mut dev = dev.borrow_mut();
        self.eng
            .request(
                &mut dev.dev,
                self::peer(peer)?,
                kind,
                &msg,
                caps.as_ref(),
                now_us / 1000,
            )
            .map(|i| (i.request_id, i.resource_list_missing))
            .map_err(refuse)
    }

    /// Starts the Get of the ResourceList of `peer` and writes it. Gives `(request_id, message)`.
    #[pyo3(signature = (dev, peer, now_us=0))]
    fn request_resource_list<'py>(
        &mut self,
        py: Python<'py>,
        dev: &Bound<'_, CiDevice>,
        peer: u32,
        now_us: u64,
    ) -> PyResult<(u8, Bound<'py, PyBytes>)> {
        let mut dev = dev.borrow_mut();
        let mut buf = [0u8; MESSAGE_MAX];
        let (id, n) = self
            .eng
            .request_resource_list(&mut dev.dev, self::peer(peer)?, now_us / 1000, &mut buf)
            .map_err(refuse)?;
        Ok((id, PyBytes::new(py, &buf[..n])))
    }

    /// Starts a "start" Subscription message to `peer` for "resource" (and "resId") with the
    /// encoding of the updates. Gives `(request_id, resource_list_missing)`; `send` writes it with
    /// no data.
    #[pyo3(signature = (dev, peer, resource, res_id=None, encoding="ASCII", caps=None, now_us=0))]
    #[allow(clippy::too_many_arguments)]
    fn subscribe(
        &mut self,
        dev: &Bound<'_, CiDevice>,
        peer: u32,
        resource: &str,
        res_id: Option<&str>,
        encoding: &str,
        caps: Option<PyRef<'_, PeResource>>,
        now_us: u64,
    ) -> PyResult<(u8, bool)> {
        let enc = self::encoding(encoding)?;
        let get = Header {
            resource: Some(HeaderStr::Text(resource)),
            res_id: res_id.map(HeaderStr::Text),
            mutual_encoding: (enc != EncodingName::Ascii).then_some(enc),
            ..Default::default()
        };
        let caps = caps.map(|c| c.caps()).transpose()?;
        let mut dev = dev.borrow_mut();
        self.eng
            .subscribe(
                &mut dev.dev,
                self::peer(peer)?,
                &get,
                caps.as_ref(),
                now_us / 1000,
            )
            .map(|i| (i.request_id, i.resource_list_missing))
            .map_err(refuse)
    }

    /// Starts an "end" Subscription message for the subscription `index` and removes the entry.
    /// Gives the Request ID; `send` writes it with no data.
    #[pyo3(signature = (dev, index, now_us=0))]
    fn unsubscribe(
        &mut self,
        dev: &Bound<'_, CiDevice>,
        index: usize,
        now_us: u64,
    ) -> PyResult<u8> {
        let mut dev = dev.borrow_mut();
        self.eng
            .unsubscribe(&mut dev.dev, index, now_us / 1000)
            .map(|(i, _)| i.request_id)
            .map_err(refuse)
    }

    /// Asks again for the Property Data of the subscription `index`, after an update that the
    /// program can not apply (REQ-PE-SUB-016). `write_pending` writes the Get.
    fn refresh(&mut self, index: usize) -> PyResult<()> {
        self.eng.refresh(index).map_err(refuse)
    }

    /// The Property Data bytes that the next Chunk of the transaction (`peer`, `request_id`,
    /// `role`) takes.
    fn data_room(
        &self,
        dev: &Bound<'_, CiDevice>,
        peer: u32,
        request_id: u8,
        role: &str,
    ) -> PyResult<usize> {
        let dev = dev.borrow();
        self.eng
            .data_room(&dev.dev, self::peer(peer)?, request_id, direction(role)?)
            .map_err(refuse)
    }

    /// Writes the next Chunk of the transaction (`peer`, `request_id`, `role`) with `part` (at most
    /// the data room) as its encoded Property Data; `last` ends the message. Gives the message.
    #[pyo3(signature = (dev, peer, request_id, role, part, last, now_us=0))]
    #[allow(clippy::too_many_arguments)]
    fn send<'py>(
        &mut self,
        py: Python<'py>,
        dev: &Bound<'_, CiDevice>,
        peer: u32,
        request_id: u8,
        role: &str,
        part: &[u8],
        last: bool,
        now_us: u64,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let mut dev = dev.borrow_mut();
        let mut buf = vec![0u8; MESSAGE_MAX + part.len()];
        let n = self
            .eng
            .send(
                &mut dev.dev,
                self::peer(peer)?,
                request_id,
                direction(role)?,
                part,
                last,
                now_us / 1000,
                &mut buf,
            )
            .map_err(refuse)?;
        Ok(PyBytes::new(py, &buf[..n]))
    }

    /// Asks the peer to wait `ms` more milliseconds (1 to 12700) with a MIDI-CI ACK 0x10
    /// (REQ-PE-END-010). `write_pending` writes the ACK.
    fn wait(&mut self, peer: u32, request_id: u8, role: &str, ms: u64) -> PyResult<()> {
        self.eng
            .wait(self::peer(peer)?, request_id, direction(role)?, ms)
            .map_err(refuse)
    }

    /// Ends the transaction (`peer`, `request_id`, `role`) with a MIDI-CI NAK 0x20.
    /// `write_pending` writes the NAK.
    fn abort(
        &mut self,
        dev: &Bound<'_, CiDevice>,
        peer: u32,
        request_id: u8,
        role: &str,
    ) -> PyResult<()> {
        let mut dev = dev.borrow_mut();
        self.eng
            .abort(
                &mut dev.dev,
                self::peer(peer)?,
                request_id,
                direction(role)?,
            )
            .map_err(refuse)
    }

    /// The last received Chunk of the transaction is corrupt: with Flow Control the engine asks
    /// for it again with a NAK 0x12 (REQ-PE-FLOW-011). `write_pending` writes the NAK.
    fn corrupt(&mut self, peer: u32, request_id: u8, role: &str) -> PyResult<()> {
        self.eng
            .corrupt(self::peer(peer)?, request_id, direction(role)?)
            .map_err(refuse)
    }

    /// The subscription entry `index`, or `None` for an empty index.
    fn subscription(&self, index: usize) -> Option<PeSubscription> {
        self.eng.subscriptions().get(index).map(subscription)
    }

    /// The next subscription that an Invalidate MUID removed, or `None`.
    fn take_ended(&mut self) -> Option<PeSubscription> {
        self.eng.take_ended().map(|e| subscription(&e))
    }
}
