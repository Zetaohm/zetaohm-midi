//! The C ABI of a zmidi-ci Device (DEC-019): Discovery and Endpoint Information, the Initiator role,
//! Profile Configuration (M2-101 7, M2-102), Property Exchange with the zmidi-ci-pe one-chunk
//! Responder (M2-101 8, M2-103) and Process Inquiry (M2-101 9). The program gives the storage, the
//! messages and the output buffers; the library gives events, and the program answers with separate
//! calls (for example [`zmidi_ci_pe_reply`] or `zmidi_ci_set_profile_state`). There is no callback
//! (DEC-019 rule 2).
//!
//! Features (REQ-FFI-CI-009, as DEC-017 and DEC-CI-001 do for zmidi-ci): `ci-device` is the Device
//! with no category and no Initiator role; `ci-initiator`, `ci-profiles`, `ci-property-exchange` and
//! `ci-process-inquiry` add the role and the categories; `ci` is all of them. The header guards the
//! calls of each with `ZMIDI_CI_DEVICE`, `ZMIDI_CI_INITIATOR`, `ZMIDI_CI_PROFILES`,
//! `ZMIDI_CI_PROPERTY_EXCHANGE` and `ZMIDI_CI_PROCESS_INQUIRY`; `ZMIDI_CI` selects all of them.

use core::mem::{align_of, size_of};

use zmidi_ci::{
    Categories, CiDevice, DeviceConfig, DeviceIdentity, Event, Framing, ManufacturerId, Muid,
    PeKind, PiCapabilities,
};

#[cfg(any(
    feature = "ci-initiator",
    feature = "ci-profiles",
    feature = "ci-process-inquiry"
))]
use zmidi_ci::{DeviceError, WriteError};

#[cfg(any(feature = "ci-profiles", feature = "ci-process-inquiry"))]
use zmidi_ci::DeviceId;

#[cfg(feature = "ci-profiles")]
use zmidi_ci::{
    ProfileAddress, ProfileDetailsReply, ProfileEntry, ProfileId, ProfileReport,
    ProfileSpecificData, ReportKind,
};

#[cfg(feature = "ci-process-inquiry")]
use zmidi_ci::{
    ChannelBits, DataControl, MessageReportInquiry, MessageReportReply, NoteBits, SystemBits,
};

#[cfg(feature = "ci-property-exchange")]
use zmidi_ci_pe::header::{Header, HeaderKind, Status};
#[cfg(feature = "ci-property-exchange")]
use zmidi_ci_pe::one_chunk::{respond, Resources, MAX_ID};

use crate::status::ZmidiStatus;

/// Peers, Profiles and open Property Exchange requests of the C device (REQ-FFI-CI-006).
pub(crate) type Device = CiDevice<'static, 4, 8, 4>;

/// The state in the storage of a [`ZmidiCi`].
pub(crate) struct State {
    pub(crate) dev: Device,
    /// xorshift32 state for MUIDs (DEC-019 rule 2).
    rng: u32,
    /// ResourceList of the configuration.
    #[cfg(feature = "ci-property-exchange")]
    list: &'static [u8],
    /// The event of the last [`zmidi_ci_receive`]; it points into that message (DEC-019 rule 3).
    /// [`zmidi_ci_pe_reply`] removes a Property Exchange request that it answers.
    #[cfg(any(
        feature = "ci-profiles",
        feature = "ci-property-exchange",
        feature = "ci-process-inquiry"
    ))]
    pub(crate) last: Option<Event<'static>>,
    /// The peer and Request ID of a reply whose last Chunk the last [`zmidi_ci_receive`] gave. The
    /// next [`zmidi_ci_receive`] or `zmidi_pe_receive` frees the ID (REQ-FFI-CI-021).
    #[cfg(all(feature = "ci-initiator", feature = "ci-property-exchange"))]
    pub(crate) ended: Option<(Muid, u8)>,
}

#[cfg(all(feature = "ci-initiator", feature = "ci-property-exchange"))]
impl State {
    /// Frees the Request ID of the reply that the last [`zmidi_ci_receive`] ended
    /// (REQ-FFI-CI-021). The free waits until the event is used: a request in that time gets
    /// another ID, so a `zmidi_pe_receive` of the event does not free the ID of a new request.
    pub(crate) fn end_reply(&mut self) {
        if let Some((peer, id)) = self.ended.take() {
            self.dev.pe_end(peer, id, zmidi_ci::Direction::Initiator);
        }
    }
}

/// Storage words of a [`ZmidiCi`] on a 64-bit target.
#[cfg(target_pointer_width = "64")]
pub const ZMIDI_CI_WORDS: usize = 68;
/// Storage words of a [`ZmidiCi`] on a 32-bit or 16-bit target.
#[cfg(not(target_pointer_width = "64"))]
pub const ZMIDI_CI_WORDS: usize = 62;

/// Storage of one MIDI-CI Device. The program gives it; [`zmidi_ci_init`] sets it up. 16-byte
/// alignment: the Device holds a `u128`.
#[repr(C, align(16))]
pub struct ZmidiCi {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_CI_WORDS],
}

const _: () = assert!(size_of::<State>() <= size_of::<ZmidiCi>());
const _: () = assert!(align_of::<State>() <= align_of::<ZmidiCi>());

/// Bytes of the longest automatic reply record: `CiDevice::MAX_REPLY_LEN` and 3 bytes of record
/// header.
pub const ZMIDI_CI_REPLY_MAX: usize = 63;
/// The most automatic replies to one message: 2 queued replies, one Reply to Profile Inquiry for
/// each Profile entry and one for the Function Block.
pub const ZMIDI_CI_REPLY_COUNT: usize = 11;
/// Bytes of a reply buffer of [`zmidi_ci_receive`] that holds all replies of one message.
pub const ZMIDI_CI_REPLY_BUFFER: usize = 693;
/// Bytes of a decoded "resource" or "resId" with its NUL (M2-103 8.1, 8.2: 36 characters).
pub const ZMIDI_PE_ID_LEN: usize = 37;
/// Bytes of an output buffer that holds the Profile reports of one Profile table call: two
/// records of at most [`ZMIDI_CI_REPLY_MAX`] bytes (M2-101 7.4 to 7.11: Disabled then Removed, or
/// Added then Enabled).
pub const ZMIDI_CI_REPORT_BUFFER: usize = 126;

const _: () = assert!(ZMIDI_CI_REPLY_MAX == Device::MAX_REPLY_LEN + 3);
const _: () = assert!(ZMIDI_CI_REPLY_BUFFER == ZMIDI_CI_REPLY_MAX * ZMIDI_CI_REPLY_COUNT);
const _: () = assert!(ZMIDI_CI_REPORT_BUFFER == 2 * ZMIDI_CI_REPLY_MAX);
#[cfg(feature = "ci-property-exchange")]
const _: () = assert!(ZMIDI_PE_ID_LEN == MAX_ID + 1);

/// The configuration of a C device (M2-101 Discovery, Endpoint Information).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiCiConfig {
    /// Manufacturer SysEx ID: `{id, 0, 0}` for a one-byte ID, `{0x00, b2, b3}` for a three-byte ID.
    pub manufacturer: [u8; 3],
    /// Device Family, 14 bits.
    pub family: u16,
    /// Device Family Model Number, 14 bits.
    pub model: u16,
    /// Software Revision Level, 4 bytes of 7 bits.
    pub revision: [u8; 4],
    /// Receivable Maximum SysEx Message Size: at least 128, at least 512 with Profiles or PE.
    pub max_sysex_size: u32,
    /// True when the Device is a Function Block.
    pub has_function_block: bool,
    /// Function Block number, 0 to 0x7E, when `has_function_block`.
    pub function_block: u8,
    /// First Group of the Function Block, 0 to 15.
    pub first_group: u8,
    /// Number of Groups, 1 to 16.
    pub group_count: u8,
    /// Channels in use on each Group: bit n is Channel n+1.
    pub channels_in_use: u16,
    /// Product Instance ID as a NUL-terminated ASCII string (at most 42 characters), or null. It
    /// stays valid while the device is used.
    pub product_instance_id: *const u8,
    /// Profile Configuration supported (needs the feature `ci-profiles`).
    pub profiles: bool,
    /// Property Exchange supported (needs the feature `ci-property-exchange`).
    pub property_exchange: bool,
    /// Process Inquiry supported, with MIDI Message Report (needs the feature
    /// `ci-process-inquiry`).
    pub process_inquiry: bool,
    /// True: messages with 0xF0/0xF7 (MIDI 1.0 byte stream); false: SysEx7 payload only.
    pub sysex_framing: bool,
    /// ResourceList JSON for a Get of "ResourceList", or null. It stays valid while the device is
    /// used.
    pub resource_list: *const u8,
    /// Bytes of `resource_list`.
    pub resource_list_len: usize,
}

/// The kind of an event: one value for each zmidi-ci Event; Property Exchange Chunks by kind.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiCiEventKind {
    /// No event.
    None = 0,
    /// Reply to Discovery from a new peer.
    Discovered = 1,
    /// The Discovery window ended.
    DiscoveryEnded = 2,
    /// This Device took a new MUID.
    MuidChanged = 3,
    /// A peer was invalidated.
    PeerInvalidated = 4,
    /// Reply to Endpoint Information.
    EndpointInfo = 5,
    /// ACK.
    Ack = 6,
    /// NAK.
    Nak = 7,
    /// Reply to Profile Inquiry: the IDs with `zmidi_ci_profile_ids`.
    ProfileInquiryReply = 8,
    /// Profile Added, Removed, Enabled or Disabled Report.
    ProfileReport = 9,
    /// Set Profile On or Off to this Device: answer with `zmidi_ci_set_profile_state`.
    SetProfile = 10,
    /// Inquiry: Profile Details to this Device: answer with `zmidi_ci_profile_details_reply`.
    ProfileDetailsInquiry = 11,
    /// Reply to Profile Details.
    ProfileDetailsReply = 12,
    /// Profile Specific Data.
    ProfileSpecificData = 13,
    /// Reply to Property Exchange Capabilities.
    PeCapabilities = 14,
    /// Inquiry: Get Property Data: answer with [`zmidi_ci_pe_reply`].
    PeGet = 15,
    /// Inquiry: Set Property Data: answer with [`zmidi_ci_pe_reply`].
    PeSet = 16,
    /// Subscription: the one-chunk Responder answers it with 405 in [`zmidi_ci_pe_reply`].
    PeSubscription = 17,
    /// Reply to Get Property Data.
    PeGetReply = 18,
    /// Reply to Set Property Data.
    PeSetReply = 19,
    /// Reply to Subscription.
    PeSubscriptionReply = 20,
    /// Notify.
    PeNotify = 21,
    /// Reply to Process Inquiry Capabilities.
    PiCapabilities = 22,
    /// Inquiry: MIDI Message Report to this Device: answer with
    /// `zmidi_ci_message_report_reply`.
    MessageReport = 23,
    /// Reply to MIDI Message Report.
    MessageReportReply = 24,
    /// End of MIDI Message Report.
    MessageReportEnd = 25,
    /// A message that zmidi-ci rejected.
    Rejected = 26,
}

/// One event. The pointers point into the message of the program. The Profile and Process
/// Inquiry fields of an event come from `zmidi_ci_profile_event` and `zmidi_ci_process_event`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiCiEvent {
    /// The kind of the event.
    pub kind: ZmidiCiEventKind,
    /// The MUID of the peer (the new MUID for MuidChanged), 0 when the event has none.
    pub muid: u32,
    /// Property Exchange: Request ID.
    pub request_id: u8,
    /// Property Exchange: Number of Chunks in Message (0 when not known).
    pub chunk_total: u16,
    /// Property Exchange: Number of This Chunk.
    pub chunk_number: u16,
    /// PeGet, PeSet: decoded "resource", NUL-terminated.
    pub resource: [u8; ZMIDI_PE_ID_LEN],
    /// PeGet, PeSet: decoded "resId", NUL-terminated.
    pub res_id: [u8; ZMIDI_PE_ID_LEN],
    /// Property Exchange: Header Data.
    pub header: *const u8,
    /// Bytes of `header`.
    pub header_len: usize,
    /// Property Exchange: Property Data of this Chunk.
    pub data: *const u8,
    /// Bytes of `data`.
    pub data_len: usize,
}

impl ZmidiCiEvent {
    const NONE: Self = Self {
        kind: ZmidiCiEventKind::None,
        muid: 0,
        request_id: 0,
        chunk_total: 0,
        chunk_number: 0,
        resource: [0; ZMIDI_PE_ID_LEN],
        res_id: [0; ZMIDI_PE_ID_LEN],
        header: core::ptr::null(),
        header_len: 0,
        data: core::ptr::null(),
        data_len: 0,
    };
}

fn xorshift(s: &mut u32) -> u32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    *s
}

/// Appends one record `[group][len lo][len hi][bytes]` to `out`; false when it does not fit.
pub(crate) fn put(out: &mut [u8], at: &mut usize, group: u8, msg: &[u8]) -> bool {
    let end = *at + 3 + msg.len();
    let Some(dst) = out.get_mut(*at..end) else {
        return false;
    };
    dst[0] = group;
    dst[1] = msg.len() as u8;
    dst[2] = (msg.len() >> 8) as u8;
    dst[3..].copy_from_slice(msg);
    *at = end;
    true
}

#[cfg(any(
    feature = "ci-initiator",
    feature = "ci-profiles",
    feature = "ci-process-inquiry"
))]
fn device_error(e: DeviceError) -> ZmidiStatus {
    match e {
        DeviceError::NotSupported => ZmidiStatus::NotSupported,
        DeviceError::InvalidConfig => ZmidiStatus::InvalidArgument,
        DeviceError::Write(WriteError::Overflow) => ZmidiStatus::BufferTooSmall,
        DeviceError::Write(_) => ZmidiStatus::InvalidArgument,
        _ => ZmidiStatus::Rejected,
    }
}

/// Fills `ev` from `event`.
fn fill(ev: &mut ZmidiCiEvent, event: &Event<'_>) {
    use ZmidiCiEventKind as K;
    let (kind, muid) = match *event {
        Event::Discovered { muid, .. } => (K::Discovered, Some(muid)),
        Event::DiscoveryEnded => (K::DiscoveryEnded, None),
        Event::MuidChanged { new, .. } => (K::MuidChanged, Some(new)),
        Event::PeerInvalidated { muid } => (K::PeerInvalidated, Some(muid)),
        Event::EndpointInfo { muid, .. } => (K::EndpointInfo, Some(muid)),
        Event::Ack { muid, .. } => (K::Ack, Some(muid)),
        Event::Nak { muid, .. } => (K::Nak, Some(muid)),
        Event::ProfileInquiryReply { muid, .. } => (K::ProfileInquiryReply, Some(muid)),
        Event::ProfileReport { muid, .. } => (K::ProfileReport, Some(muid)),
        Event::SetProfile { muid, .. } => (K::SetProfile, Some(muid)),
        Event::ProfileDetailsInquiry { muid, .. } => (K::ProfileDetailsInquiry, Some(muid)),
        Event::ProfileDetailsReply { muid, .. } => (K::ProfileDetailsReply, Some(muid)),
        Event::ProfileSpecificData { muid, .. } => (K::ProfileSpecificData, Some(muid)),
        Event::PeCapabilities { muid, .. } => (K::PeCapabilities, Some(muid)),
        Event::PeChunk { muid, kind, chunk } => {
            ev.request_id = chunk.request_id;
            ev.chunk_total = chunk.total;
            ev.chunk_number = chunk.number;
            ev.header = chunk.header.as_ptr();
            ev.header_len = chunk.header.len();
            ev.data = chunk.data.as_ptr();
            ev.data_len = chunk.data.len();
            let k = match kind {
                PeKind::GetInquiry => K::PeGet,
                PeKind::SetInquiry => K::PeSet,
                PeKind::Subscription => K::PeSubscription,
                PeKind::GetReply => K::PeGetReply,
                PeKind::SetReply => K::PeSetReply,
                PeKind::SubscriptionReply => K::PeSubscriptionReply,
                PeKind::Notify => K::PeNotify,
            };
            #[cfg(feature = "ci-property-exchange")]
            if matches!(kind, PeKind::GetInquiry | PeKind::SetInquiry) {
                if let Ok(h) = Header::parse(HeaderKind::Inquiry, chunk.header) {
                    if let Some(r) = h.resource {
                        r.decode_into(&mut ev.resource[..MAX_ID]);
                    }
                    if let Some(r) = h.res_id {
                        r.decode_into(&mut ev.res_id[..MAX_ID]);
                    }
                }
            }
            (k, Some(muid))
        }
        Event::PiCapabilities { muid, .. } => (K::PiCapabilities, Some(muid)),
        Event::MessageReport { muid, .. } => (K::MessageReport, Some(muid)),
        Event::MessageReportReply { muid, .. } => (K::MessageReportReply, Some(muid)),
        Event::MessageReportEnd { muid } => (K::MessageReportEnd, Some(muid)),
        Event::Rejected { .. } => (K::Rejected, None),
    };
    ev.kind = kind;
    ev.muid = muid.map_or(0, Muid::value);
}

/// Gives the state of `ci`, or `None` for a null pointer.
///
/// # Safety
/// `ci` is null or was set up by [`zmidi_ci_init`].
pub(crate) unsafe fn state<'a>(ci: *mut ZmidiCi) -> Option<&'a mut State> {
    // SAFETY: caller contract above.
    unsafe { ci.cast::<State>().as_mut() }
}

/// Gives the bytes of a NUL-terminated string, or `None` for a string over `max` bytes.
///
/// # Safety
/// `p` points to a NUL-terminated string.
pub(crate) unsafe fn c_str(p: *const u8, max: usize) -> Option<&'static [u8]> {
    let mut n = 0;
    // SAFETY: caller contract above: the string ends with a NUL.
    while unsafe { *p.add(n) } != 0 {
        n += 1;
        if n > max {
            return None;
        }
    }
    // SAFETY: caller contract above: the bytes outlive the device.
    Some(unsafe { core::slice::from_raw_parts(p, n) })
}

/// Gives the `len` bytes at `p`: empty for a length of 0, `None` for a null pointer with a length.
///
/// # Safety
/// `p` points to `len` bytes or `len` is 0.
#[cfg(any(feature = "ci-profiles", feature = "ci-property-exchange"))]
pub(crate) unsafe fn bytes<'a>(p: *const u8, len: usize) -> Option<&'a [u8]> {
    if len == 0 {
        Some(&[])
    } else if p.is_null() {
        None
    } else {
        // SAFETY: caller contract above.
        Some(unsafe { core::slice::from_raw_parts(p, len) })
    }
}

/// Sets up `ci` from `config` with MUIDs from the random `seed`. A category of `config` that the
/// build does not have gives not-supported (REQ-FFI-CI-009).
///
/// # Safety
/// `ci` points to writable storage of a [`ZmidiCi`]; `config` is valid; its strings stay valid
/// while the device is used.
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_init(
    ci: *mut ZmidiCi,
    config: *const ZmidiCiConfig,
    seed: u32,
) -> ZmidiStatus {
    if ci.is_null() || config.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let c = unsafe { &*config };
    if (c.profiles && !cfg!(feature = "ci-profiles"))
        || (c.property_exchange && !cfg!(feature = "ci-property-exchange"))
        || (c.process_inquiry && !cfg!(feature = "ci-process-inquiry"))
    {
        return ZmidiStatus::NotSupported;
    }
    let manufacturer = if c.manufacturer[0] == 0 {
        ManufacturerId::ThreeByte(c.manufacturer)
    } else {
        ManufacturerId::OneByte(c.manufacturer[0])
    };
    let product_instance_id = if c.product_instance_id.is_null() {
        None
    } else {
        // SAFETY: caller contract above.
        match unsafe { c_str(c.product_instance_id, 42) }.map(core::str::from_utf8) {
            Some(Ok(s)) => Some(s),
            _ => return ZmidiStatus::InvalidArgument,
        }
    };
    let config = DeviceConfig {
        identity: DeviceIdentity {
            manufacturer,
            family: c.family,
            model: c.model,
            revision: c.revision,
        },
        categories: Categories {
            profile_configuration: c.profiles,
            property_exchange: c.property_exchange,
            process_inquiry: c.process_inquiry,
            ..Categories::default()
        },
        max_sysex_size: c.max_sysex_size,
        function_block: c.has_function_block.then_some(c.function_block),
        first_group: c.first_group,
        group_count: c.group_count,
        channels_in_use: c.channels_in_use,
        product_instance_id,
        framing: if c.sysex_framing {
            Framing::SysEx
        } else {
            Framing::Bare
        },
        process_inquiry: PiCapabilities {
            midi_message_report: c.process_inquiry,
        },
    };
    #[cfg(feature = "ci-property-exchange")]
    let list = if c.resource_list.is_null() {
        &[][..]
    } else {
        // SAFETY: caller contract above: the bytes outlive the device.
        unsafe { core::slice::from_raw_parts(c.resource_list, c.resource_list_len) }
    };
    let mut rng = seed | 1;
    let Ok(dev) = Device::new(config, &mut || xorshift(&mut rng)) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe {
        ci.cast::<State>().write(State {
            dev,
            rng,
            #[cfg(feature = "ci-property-exchange")]
            list,
            #[cfg(any(
                feature = "ci-profiles",
                feature = "ci-property-exchange",
                feature = "ci-process-inquiry"
            ))]
            last: None,
            #[cfg(all(feature = "ci-initiator", feature = "ci-property-exchange"))]
            ended: None,
        })
    };
    ZmidiStatus::Ok
}

/// Gives the MUID of `ci`, or 0 for a null pointer.
///
/// # Safety
/// `ci` is null or was set up by [`zmidi_ci_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_muid(ci: *mut ZmidiCi) -> u32 {
    // SAFETY: caller contract above.
    unsafe { state(ci) }.map_or(0, |s| s.dev.muid().value())
}

/// Gives the size in bytes of the Rust state of a [`ZmidiCi`] (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_ci_size() -> usize {
    size_of::<State>()
}

/// Reads one MIDI-CI message `msg` (`len` bytes, in the framing of the configuration) received on
/// `group` at `now_us`, writes the automatic replies as records into `out` (`capacity` bytes) with
/// their byte count in `out_len`, and the event into `ev`. Keep `msg` valid while you use the event
/// (its pointers, `zmidi_ci_profile_event`, `zmidi_ci_process_event`) and until the reply call of a
/// PeGet, PeSet or PeSubscription event ([`zmidi_ci_pe_reply`]).
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `msg` points to `len` bytes; `out` points to `capacity`
/// writable bytes; `out_len` and `ev` are writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_receive(
    ci: *mut ZmidiCi,
    group: u8,
    msg: *const u8,
    len: usize,
    now_us: u64,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
    ev: *mut ZmidiCiEvent,
) -> ZmidiStatus {
    if msg.is_null() || out.is_null() || out_len.is_null() || ev.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { state(ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    if capacity < ZMIDI_CI_REPLY_BUFFER {
        return ZmidiStatus::BufferTooSmall;
    }
    // SAFETY: caller contract above: the message stays valid while the event is used.
    let msg: &'static [u8] = unsafe { core::slice::from_raw_parts(msg, len) };
    // SAFETY: caller contract above.
    let out = unsafe { core::slice::from_raw_parts_mut(out, capacity) };
    #[cfg(all(feature = "ci-initiator", feature = "ci-property-exchange"))]
    s.end_reply();
    let mut e = ZmidiCiEvent::NONE;
    let mut rng = s.rng;
    let mut at = 0;
    let mut fits = true;
    let event = {
        let mut r = s
            .dev
            .receive(group, msg, now_us, &mut || xorshift(&mut rng));
        let mut buf = [0u8; Device::MAX_REPLY_LEN];
        while let Some(w) = r.write_next(&mut buf) {
            match w {
                Ok((g, n)) => fits &= put(out, &mut at, g, &buf[..n]),
                Err(_) => fits = false,
            }
        }
        r.event()
    };
    s.rng = rng;
    if let Some(event) = &event {
        fill(&mut e, event);
    }
    // REQ-FFI-CI-021: the last Chunk of a reply ends the request of this Device (M2-101 Table 29).
    #[cfg(all(feature = "ci-initiator", feature = "ci-property-exchange"))]
    if let Some(Event::PeChunk {
        muid,
        kind: PeKind::GetReply | PeKind::SetReply | PeKind::SubscriptionReply,
        chunk,
    }) = event
    {
        if chunk.end() != zmidi_ci::ChunkEnd::More {
            s.ended = Some((muid, chunk.request_id));
        }
    }
    #[cfg(any(
        feature = "ci-profiles",
        feature = "ci-property-exchange",
        feature = "ci-process-inquiry"
    ))]
    {
        s.last = event;
    }
    // SAFETY: caller contract above.
    unsafe {
        out_len.write(at);
        ev.write(e);
    }
    if fits {
        ZmidiStatus::Ok
    } else {
        ZmidiStatus::BufferTooSmall
    }
}

/// Runs the timers of `ci` at `now_us` and writes the event they give into `ev` (kind None when
/// there is none).
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `ev` is writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_poll(
    ci: *mut ZmidiCi,
    now_us: u64,
    ev: *mut ZmidiCiEvent,
) -> ZmidiStatus {
    if ev.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { state(ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let mut e = ZmidiCiEvent::NONE;
    if let Some(event) = s.dev.poll(now_us) {
        fill(&mut e, &event);
    }
    // SAFETY: caller contract above.
    unsafe { ev.write(e) };
    ZmidiStatus::Ok
}

/// Writes one message from `write` as a record on `group` into `out` (`capacity` bytes) and its
/// length into `out_len`.
///
/// # Safety
/// `ci` is null or was set up by [`zmidi_ci_init`]; `out` points to `capacity` writable bytes;
/// `out_len` is writable.
#[cfg(any(
    feature = "ci-initiator",
    feature = "ci-profiles",
    feature = "ci-process-inquiry"
))]
unsafe fn write_one(
    ci: *mut ZmidiCi,
    group: u8,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
    write: impl FnOnce(&mut Device, &mut [u8]) -> Result<usize, DeviceError>,
) -> ZmidiStatus {
    if out.is_null() || out_len.is_null() || group > 15 {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { state(ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let out = unsafe { core::slice::from_raw_parts_mut(out, capacity) };
    let Some(body) = out.get_mut(3..) else {
        return ZmidiStatus::BufferTooSmall;
    };
    match write(&mut s.dev, body) {
        Ok(n) => {
            out[0] = group;
            out[1] = n as u8;
            out[2] = (n >> 8) as u8;
            // SAFETY: caller contract above.
            unsafe { out_len.write(3 + n) };
            ZmidiStatus::Ok
        }
        Err(e) => device_error(e),
    }
}

/// Gives the MUID `peer`, or `None` when it has more than 28 bits.
#[cfg(any(
    feature = "ci-initiator",
    feature = "ci-profiles",
    feature = "ci-process-inquiry"
))]
fn muid(peer: u32) -> Option<Muid> {
    Muid::new(peer).ok()
}

/// Writes Discovery for the output path `output_path_id` at `now_us` as a record into `out`.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `out` points to `capacity` writable bytes; `out_len` is
/// writable.
#[cfg(feature = "ci-initiator")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_discovery(
    ci: *mut ZmidiCi,
    output_path_id: u8,
    now_us: u64,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, 0, out, capacity, out_len, |d, b| {
            d.discovery(output_path_id, now_us, b)
        })
    }
}

/// Writes Inquiry: Endpoint Information to `peer` as a record into `out`.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `out` points to `capacity` writable bytes; `out_len` is
/// writable.
#[cfg(feature = "ci-initiator")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_endpoint_info_inquiry(
    ci: *mut ZmidiCi,
    peer: u32,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    let Some(peer) = muid(peer) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, 0, out, capacity, out_len, |d, b| {
            d.endpoint_info_inquiry(peer, b)
        })
    }
}

/// Writes Inquiry: Property Exchange Capabilities to `peer` as a record into `out`.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `out` points to `capacity` writable bytes; `out_len` is
/// writable.
#[cfg(all(feature = "ci-initiator", feature = "ci-property-exchange"))]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_pe_capabilities_inquiry(
    ci: *mut ZmidiCi,
    peer: u32,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    let Some(peer) = muid(peer) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, 0, out, capacity, out_len, |d, b| {
            d.pe_capabilities_inquiry(peer, b)
        })
    }
}

/// Writes a Property Exchange request of `kind` with `header` and `data` as Chunk records.
///
/// # Safety
/// As [`zmidi_ci_pe_get`]; `data` is valid.
#[cfg(all(feature = "ci-initiator", feature = "ci-property-exchange"))]
#[allow(clippy::too_many_arguments)]
unsafe fn pe_request(
    ci: *mut ZmidiCi,
    peer: u32,
    kind: PeKind,
    header: *const u8,
    header_len: usize,
    data: &[u8],
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
    request_id: *mut u8,
) -> ZmidiStatus {
    if header.is_null() || out.is_null() || out_len.is_null() || request_id.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    let Some(peer) = muid(peer) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { state(ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let (header, out) = unsafe {
        (
            core::slice::from_raw_parts(header, header_len),
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let (id, mut w) = match s.dev.pe_request(peer, kind, header, Some(data.len()), None) {
        Ok(x) => x,
        Err(e) => return device_error(e),
    };
    let mut at = 0;
    let mut sent = 0;
    loop {
        let room = w.data_room().min(data.len() - sent);
        let Some(chunk) = out.get_mut(at + 3..) else {
            s.dev.pe_end(peer, id, zmidi_ci::Direction::Initiator);
            return ZmidiStatus::BufferTooSmall;
        };
        match w.write_next(&data[sent..sent + room], chunk) {
            Ok(n) => {
                out[at] = 0;
                out[at + 1] = n as u8;
                out[at + 2] = (n >> 8) as u8;
                at += 3 + n;
            }
            Err(_) => {
                s.dev.pe_end(peer, id, zmidi_ci::Direction::Initiator);
                return ZmidiStatus::BufferTooSmall;
            }
        }
        sent += room;
        if sent >= data.len() {
            break;
        }
    }
    // SAFETY: caller contract above.
    unsafe {
        out_len.write(at);
        request_id.write(id);
    }
    ZmidiStatus::Ok
}

/// Writes Inquiry: Get Property Data with the Header Data `header` to `peer` as Chunk records into
/// `out`, and the Request ID into `request_id`.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `header` points to `header_len` bytes; `out` points to
/// `capacity` writable bytes; `out_len` and `request_id` are writable.
#[cfg(all(feature = "ci-initiator", feature = "ci-property-exchange"))]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_pe_get(
    ci: *mut ZmidiCi,
    peer: u32,
    header: *const u8,
    header_len: usize,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
    request_id: *mut u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe {
        pe_request(
            ci,
            peer,
            PeKind::GetInquiry,
            header,
            header_len,
            &[],
            out,
            capacity,
            out_len,
            request_id,
        )
    }
}

/// Writes Inquiry: Set Property Data with `header` and `data` to `peer` as Chunk records into
/// `out`, and the Request ID into `request_id`.
///
/// # Safety
/// As [`zmidi_ci_pe_get`]; `data` points to `data_len` bytes.
#[cfg(all(feature = "ci-initiator", feature = "ci-property-exchange"))]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_pe_set(
    ci: *mut ZmidiCi,
    peer: u32,
    header: *const u8,
    header_len: usize,
    data: *const u8,
    data_len: usize,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
    request_id: *mut u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(data) = (unsafe { bytes(data, data_len) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        pe_request(
            ci,
            peer,
            PeKind::SetInquiry,
            header,
            header_len,
            data,
            out,
            capacity,
            out_len,
            request_id,
        )
    }
}

/// Frees the Request ID `request_id` of a [`zmidi_ci_pe_get`] or [`zmidi_ci_pe_set`] to `peer`
/// that has no reply: a timeout or a NAK (REQ-FFI-CI-022). The last Chunk of a reply frees the ID
/// without this call (REQ-FFI-CI-021). Do not call it for a request of a `ZmidiPe`.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`].
#[cfg(all(feature = "ci-initiator", feature = "ci-property-exchange"))]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_pe_end(
    ci: *mut ZmidiCi,
    peer: u32,
    request_id: u8,
) -> ZmidiStatus {
    let Some(peer) = muid(peer) else {
        return ZmidiStatus::InvalidArgument;
    };
    if request_id > 0x7F {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { state(ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    if s.ended == Some((peer, request_id)) {
        s.ended = None;
    }
    s.dev
        .pe_end(peer, request_id, zmidi_ci::Direction::Initiator);
    ZmidiStatus::Ok
}

/// Bytes of a buffer of [`zmidi_ci_pe_reply`] for `n` bytes of Property Data (REQ-FFI-CI-005).
#[cfg(feature = "ci-property-exchange")]
const fn reply_need(n: usize) -> usize {
    2 * n + 64
}

/// Answers the pending PeGet, PeSet or PeSubscription event with `status` (200: success) and, for
/// a Get, the Property Data `data`. The reply Chunks go as records into `out`; `scratch` holds the
/// Property Data and the Chunk. Both buffers need 2 x max(`data_len`, ResourceList length) + 64
/// bytes.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; the message of the event is still valid; `data` points to
/// `data_len` bytes (or is null when `data_len` is 0); `scratch` and `out` point to their writable
/// bytes; `out_len` is writable.
#[cfg(feature = "ci-property-exchange")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_pe_reply(
    ci: *mut ZmidiCi,
    status: u16,
    data: *const u8,
    data_len: usize,
    scratch: *mut u8,
    scratch_len: usize,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    /// The answer of the program, given to `respond` as its Resources.
    struct Answer<'a> {
        status: u16,
        data: &'a [u8],
    }

    impl Resources for Answer<'_> {
        fn get(&mut self, _: &[u8], _: Option<&[u8]>, out: &mut [u8]) -> Result<usize, Status> {
            if self.status != Status::OK.0 {
                return Err(Status(self.status));
            }
            let n = self.data.len().min(out.len());
            out[..n].copy_from_slice(&self.data[..n]);
            Ok(n)
        }

        fn set(&mut self, _: &[u8], _: Option<&[u8]>, _: &[u8]) -> Result<(), Status> {
            if self.status == Status::OK.0 {
                Ok(())
            } else {
                Err(Status(self.status))
            }
        }
    }

    if (data.is_null() && data_len > 0) || scratch.is_null() || out.is_null() || out_len.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { state(ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Some(event @ Event::PeChunk { kind, .. }) = s.last else {
        return ZmidiStatus::NoPending;
    };
    if !matches!(
        kind,
        PeKind::GetInquiry | PeKind::SetInquiry | PeKind::Subscription
    ) {
        return ZmidiStatus::NoPending;
    }
    let split = data_len.max(s.list.len());
    if scratch_len < reply_need(split) || capacity < reply_need(split) {
        return ZmidiStatus::BufferTooSmall;
    }
    s.last = None;
    // SAFETY: caller contract above.
    let (data, scratch, out) = unsafe {
        (
            bytes(data, data_len).unwrap_or(&[]),
            core::slice::from_raw_parts_mut(scratch, scratch_len),
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let (body, chunk_buf) = scratch.split_at_mut(split);
    let mut at = 0;
    let mut fits = true;
    respond(
        &mut s.dev,
        &event,
        s.list,
        &mut Answer { status, data },
        body,
        chunk_buf,
        |m| fits &= put(out, &mut at, 0, m),
    );
    // SAFETY: caller contract above.
    unsafe { out_len.write(at) };
    if fits {
        ZmidiStatus::Ok
    } else {
        ZmidiStatus::BufferTooSmall
    }
}

// ---------------------------------------------------------------------------------------------
// Profile Configuration (M2-101 7, M2-102): REQ-FFI-CI-010 to REQ-FFI-CI-012.

/// The kind of a Profile report in a [`ZmidiCiProfileEvent`] (M2-101 7.4, 7.5, 7.10, 7.11).
#[cfg(feature = "ci-profiles")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiCiProfileReport {
    /// The event is not a Profile report.
    None = 0,
    /// Profile Added Report (0x26).
    Added = 1,
    /// Profile Removed Report (0x27).
    Removed = 2,
    /// Profile Enabled Report (0x24).
    Enabled = 3,
    /// Profile Disabled Report (0x25).
    Disabled = 4,
}

/// One local Profile for `zmidi_ci_add_profile` (M2-101 7.1 to 7.3).
#[cfg(feature = "ci-profiles")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiCiProfile {
    /// Device ID of the address: Channel 0x00 to 0x0F, Group 0x7E, Function Block 0x7F.
    pub device_id: u8,
    /// Group offset of the address in the Function Block, 0 to 15 (0 for 0x7F).
    pub group: u8,
    /// Profile ID, 5 bytes (Table 19).
    pub profile: [u8; 5],
    /// True when the Profile is enabled now.
    pub enabled: bool,
    /// Number of Channels enabled (Manager and Members), 14 bits.
    pub channels: u16,
    /// Maximum Number of Channels, 14 bits.
    pub max_channels: u16,
}

/// The Profile fields of the last event (M2-101 7). The pointers point into the message.
#[cfg(feature = "ci-profiles")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiCiProfileEvent {
    /// Device ID of the message (the address of the Profile).
    pub device_id: u8,
    /// UMP Group of the message; for SetProfile and ProfileDetailsInquiry the Group offset of the
    /// local Profile address.
    pub group: u8,
    /// Profile ID, 5 bytes (zero for ProfileInquiryReply).
    pub profile: [u8; 5],
    /// ProfileReport: the kind of the report.
    pub report: ZmidiCiProfileReport,
    /// SetProfile: true for Set Profile On, false for Set Profile Off.
    pub on: bool,
    /// SetProfile, ProfileReport: Number of Channels (Requested, Enabled or Disabled).
    pub channels: u16,
    /// ProfileDetailsInquiry, ProfileDetailsReply: Inquiry Target.
    pub target: u8,
    /// ProfileDetailsReply: true when this Device did not ask for it (M2-101 7.7).
    pub notification: bool,
    /// ProfileInquiryReply: number of Currently Enabled Profiles.
    pub enabled_count: usize,
    /// ProfileInquiryReply: number of Currently Disabled Profiles Supported.
    pub disabled_count: usize,
    /// ProfileDetailsReply: Inquiry Target Data; ProfileSpecificData: Profile Specific Data.
    pub data: *const u8,
    /// Bytes of `data`.
    pub data_len: usize,
}

/// Gives the Device ID of the byte `b`, or `None` for a reserved value (M2-101 Table 4).
#[cfg(any(feature = "ci-profiles", feature = "ci-process-inquiry"))]
fn device_id(b: u8) -> Option<DeviceId> {
    DeviceId::from_byte(b).ok()
}

/// Reads the 5 bytes of a Profile ID at `p`, or `None` for a null pointer or an invalid ID.
///
/// # Safety
/// `p` is null or points to 5 bytes.
#[cfg(feature = "ci-profiles")]
unsafe fn profile_id(p: *const u8) -> Option<ProfileId> {
    if p.is_null() {
        return None;
    }
    let mut b = [0u8; 5];
    // SAFETY: caller contract above.
    unsafe { core::ptr::copy_nonoverlapping(p, b.as_mut_ptr(), 5) };
    ProfileId::from_bytes(b).ok()
}

/// Gives the Profile address of `device_id` and the Group offset `group`.
#[cfg(feature = "ci-profiles")]
fn address(device_id: u8, group: u8) -> Option<ProfileAddress> {
    if group > 15 {
        return None;
    }
    Some(ProfileAddress {
        device_id: self::device_id(device_id)?,
        group,
    })
}

/// Writes `reports` as records into `out` (at least [`ZMIDI_CI_REPORT_BUFFER`] bytes).
#[cfg(feature = "ci-profiles")]
fn put_reports(
    dev: &Device,
    reports: impl Iterator<Item = ProfileReport>,
    out: &mut [u8],
) -> Result<usize, ZmidiStatus> {
    let mut at = 0;
    let mut buf = [0u8; Device::MAX_REPLY_LEN];
    for r in reports {
        let (g, n) = dev.profile_report(&r, &mut buf).map_err(device_error)?;
        if !put(out, &mut at, g, &buf[..n]) {
            return Err(ZmidiStatus::BufferTooSmall);
        }
    }
    Ok(at)
}

/// Checks the output buffer of a Profile table call and gives the state and the buffer.
///
/// # Safety
/// As `zmidi_ci_add_profile`.
#[cfg(feature = "ci-profiles")]
unsafe fn table_call<'a>(
    ci: *mut ZmidiCi,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
    need: usize,
) -> Result<(&'a mut State, &'a mut [u8]), ZmidiStatus> {
    if out.is_null() || out_len.is_null() {
        return Err(ZmidiStatus::InvalidArgument);
    }
    // SAFETY: caller contract above.
    let s = unsafe { state(ci) }.ok_or(ZmidiStatus::InvalidArgument)?;
    if capacity < need {
        return Err(ZmidiStatus::BufferTooSmall);
    }
    // SAFETY: caller contract above.
    Ok((s, unsafe { core::slice::from_raw_parts_mut(out, capacity) }))
}

/// Adds the local Profile `profile` to the table of `ci` (M2-101 7.4, REQ-CI-PROF-019) and writes
/// the Profile Added Report, and the Profile Enabled Report for an enabled Profile, as records into
/// `out` (at least [`ZMIDI_CI_REPORT_BUFFER`] bytes). A Profile that is in the table, or a full
/// table, gives rejected.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `profile` is valid; `out` points to `capacity` writable
/// bytes; `out_len` is writable.
#[cfg(feature = "ci-profiles")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_add_profile(
    ci: *mut ZmidiCi,
    profile: *const ZmidiCiProfile,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    if profile.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let p = unsafe { &*profile };
    let (Some(address), Ok(id)) = (
        address(p.device_id, p.group),
        ProfileId::from_bytes(p.profile),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    if p.channels > 0x3FFF || p.max_channels > 0x3FFF {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (s, out) = match unsafe { table_call(ci, out, capacity, out_len, ZMIDI_CI_REPORT_BUFFER) } {
        Ok(x) => x,
        Err(e) => return e,
    };
    let entry = ProfileEntry {
        address,
        profile: id,
        enabled: p.enabled,
        channels: p.channels,
        max_channels: p.max_channels,
    };
    let reports = match s.dev.add_profile(entry) {
        Ok(r) => r,
        Err(DeviceError::NotSupported) => return ZmidiStatus::NotSupported,
        Err(_) => return ZmidiStatus::Rejected,
    };
    match put_reports(&s.dev, reports, out) {
        Ok(n) => {
            // SAFETY: caller contract above.
            unsafe { out_len.write(n) };
            ZmidiStatus::Ok
        }
        Err(e) => e,
    }
}

/// Removes the local Profile `profile` (5 bytes) at `device_id` and the Group offset `group`
/// (M2-101 7.5, REQ-CI-PROF-021) and writes the Profile Disabled Report for an enabled Profile and
/// the Profile Removed Report as records into `out` (at least [`ZMIDI_CI_REPORT_BUFFER`] bytes). A
/// Profile that is not in the table gives rejected.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `profile` points to 5 bytes; `out` points to `capacity`
/// writable bytes; `out_len` is writable.
#[cfg(feature = "ci-profiles")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_remove_profile(
    ci: *mut ZmidiCi,
    device_id: u8,
    group: u8,
    profile: *const u8,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(address), Some(id)) = (address(device_id, group), unsafe { profile_id(profile) })
    else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let (s, out) = match unsafe { table_call(ci, out, capacity, out_len, ZMIDI_CI_REPORT_BUFFER) } {
        Ok(x) => x,
        Err(e) => return e,
    };
    let Some(reports) = s.dev.remove_profile(address, id) else {
        return ZmidiStatus::Rejected;
    };
    match put_reports(&s.dev, reports, out) {
        Ok(n) => {
            // SAFETY: caller contract above.
            unsafe { out_len.write(n) };
            ZmidiStatus::Ok
        }
        Err(e) => e,
    }
}

/// Sets the local Profile `profile` (5 bytes) at `device_id` and `group` to `enabled` with
/// `channels` Channels and writes the Profile Enabled or Disabled Report as a record into `out` (at
/// least [`ZMIDI_CI_REPLY_MAX`] bytes): the answer to a SetProfile event, or the report of a local
/// change (M2-101 7.10, 7.11, REQ-CI-PROF-027 to -032). To refuse a Set Profile On, give
/// `enabled` false. A Profile that is not in the table gives rejected.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `profile` points to 5 bytes; `out` points to `capacity`
/// writable bytes; `out_len` is writable.
#[cfg(feature = "ci-profiles")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_set_profile_state(
    ci: *mut ZmidiCi,
    device_id: u8,
    group: u8,
    profile: *const u8,
    enabled: bool,
    channels: u16,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(address), Some(id)) = (address(device_id, group), unsafe { profile_id(profile) })
    else {
        return ZmidiStatus::InvalidArgument;
    };
    if channels > 0x3FFF {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (s, out) = match unsafe { table_call(ci, out, capacity, out_len, ZMIDI_CI_REPLY_MAX) } {
        Ok(x) => x,
        Err(e) => return e,
    };
    let Some(report) = s.dev.set_profile_state(address, id, enabled, channels) else {
        return ZmidiStatus::Rejected;
    };
    match put_reports(&s.dev, core::iter::once(report), out) {
        Ok(n) => {
            // SAFETY: caller contract above.
            unsafe { out_len.write(n) };
            ZmidiStatus::Ok
        }
        Err(e) => e,
    }
}

/// Writes Reply to Profile Details Inquiry (M2-101 7.7, Table 23) for the Profile `profile` (5
/// bytes) and the Inquiry Target `target` with the Inquiry Target Data `data` to `peer` as a
/// record on `group`: the answer to a ProfileDetailsInquiry event.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `profile` points to 5 bytes; `data` points to `data_len`
/// bytes (or `data_len` is 0); `out` points to `capacity` writable bytes; `out_len` is writable.
#[cfg(feature = "ci-profiles")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_profile_details_reply(
    ci: *mut ZmidiCi,
    peer: u32,
    group: u8,
    device_id: u8,
    profile: *const u8,
    target: u8,
    data: *const u8,
    data_len: usize,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(peer), Some(d), Some(profile), Some(data)) = (
        muid(peer),
        self::device_id(device_id),
        unsafe { profile_id(profile) },
        unsafe { bytes(data, data_len) },
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    if target > 0x7F {
        return ZmidiStatus::InvalidArgument;
    }
    let reply = ProfileDetailsReply {
        profile,
        target,
        data,
    };
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, group, out, capacity, out_len, |dev, b| {
            dev.profile_details_reply(peer, d, &reply, b)
        })
    }
}

/// Writes Profile Specific Data (M2-101 7.12, Table 28) for the Profile `profile` (5 bytes) with
/// `data` to `peer` (0x0FFFFFFF: Broadcast, at most 512 bytes) as a record on `group`.
///
/// # Safety
/// As [`zmidi_ci_profile_details_reply`].
#[cfg(feature = "ci-profiles")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_profile_specific_data(
    ci: *mut ZmidiCi,
    peer: u32,
    group: u8,
    device_id: u8,
    profile: *const u8,
    data: *const u8,
    data_len: usize,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(peer), Some(d), Some(profile), Some(data)) = (
        muid(peer),
        self::device_id(device_id),
        unsafe { profile_id(profile) },
        unsafe { bytes(data, data_len) },
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    let psd = ProfileSpecificData { profile, data };
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, group, out, capacity, out_len, |dev, b| {
            let (me, framing) = (dev.muid(), dev.config().framing);
            Ok(psd.write(false, d, me, peer, framing, b)?)
        })
    }
}

/// Writes Profile Inquiry (M2-101 7.2, Table 17) to `peer` for the address `device_id` as a record
/// on `group` (REQ-CI-PROF-015). The Reply to Profile Inquiry gives a ProfileInquiryReply event.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `out` points to `capacity` writable bytes; `out_len` is
/// writable.
#[cfg(all(feature = "ci-profiles", feature = "ci-initiator"))]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_profile_inquiry(
    ci: *mut ZmidiCi,
    peer: u32,
    group: u8,
    device_id: u8,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    let (Some(peer), Some(d)) = (muid(peer), self::device_id(device_id)) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, group, out, capacity, out_len, |dev, b| {
            dev.profile_inquiry(peer, d, b)
        })
    }
}

/// Writes Set Profile On (M2-101 7.8, Table 24) for the Profile `profile` (5 bytes) with `channels`
/// Number of Channels Requested to `peer` as a record on `group`. `max_channels` is the Maximum
/// Number of Channels that the peer reported, or 0 when not known. Before a Profile Inquiry to the
/// peer (M2-101 2.2, REQ-CI-GEN-003) it gives rejected.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `profile` points to 5 bytes; `out` points to `capacity`
/// writable bytes; `out_len` is writable.
#[cfg(all(feature = "ci-profiles", feature = "ci-initiator"))]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_set_profile_on(
    ci: *mut ZmidiCi,
    peer: u32,
    group: u8,
    device_id: u8,
    profile: *const u8,
    channels: u16,
    max_channels: u16,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(peer), Some(d), Some(profile)) = (muid(peer), self::device_id(device_id), unsafe {
        profile_id(profile)
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    if channels > 0x3FFF {
        return ZmidiStatus::InvalidArgument;
    }
    let max = (max_channels != 0).then_some(max_channels);
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, group, out, capacity, out_len, |dev, b| {
            dev.set_profile_on(peer, d, profile, channels, max, b)
        })
    }
}

/// Writes Set Profile Off (M2-101 7.9, Table 25) for the Profile `profile` (5 bytes) to `peer` as a
/// record on `group`. Before a Profile Inquiry to the peer it gives rejected.
///
/// # Safety
/// As [`zmidi_ci_set_profile_on`].
#[cfg(all(feature = "ci-profiles", feature = "ci-initiator"))]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_set_profile_off(
    ci: *mut ZmidiCi,
    peer: u32,
    group: u8,
    device_id: u8,
    profile: *const u8,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(peer), Some(d), Some(profile)) = (muid(peer), self::device_id(device_id), unsafe {
        profile_id(profile)
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, group, out, capacity, out_len, |dev, b| {
            dev.set_profile_off(peer, d, profile, b)
        })
    }
}

/// Writes Profile Details Inquiry (M2-101 7.6, Table 22) for the Profile `profile` (5 bytes) and
/// the Inquiry Target `target` to `peer` as a record on `group`; version 2 peers only.
///
/// # Safety
/// As [`zmidi_ci_set_profile_on`].
#[cfg(all(feature = "ci-profiles", feature = "ci-initiator"))]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_profile_details_inquiry(
    ci: *mut ZmidiCi,
    peer: u32,
    group: u8,
    device_id: u8,
    profile: *const u8,
    target: u8,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(peer), Some(d), Some(profile)) = (muid(peer), self::device_id(device_id), unsafe {
        profile_id(profile)
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    if target > 0x7F {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, group, out, capacity, out_len, |dev, b| {
            dev.profile_details_inquiry(peer, d, profile, target, b)
        })
    }
}

/// Writes the Profile fields of the last event of [`zmidi_ci_receive`] into `pe`. An event that is
/// not a Profile event gives no-pending.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; the message of the event is still valid; `pe` is writable.
#[cfg(feature = "ci-profiles")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_profile_event(
    ci: *mut ZmidiCi,
    pe: *mut ZmidiCiProfileEvent,
) -> ZmidiStatus {
    if pe.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { state(ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let mut p = ZmidiCiProfileEvent {
        device_id: 0,
        group: 0,
        profile: [0; 5],
        report: ZmidiCiProfileReport::None,
        on: false,
        channels: 0,
        target: 0,
        notification: false,
        enabled_count: 0,
        disabled_count: 0,
        data: core::ptr::null(),
        data_len: 0,
    };
    match s.last {
        Some(Event::ProfileInquiryReply {
            device_id,
            group,
            reply,
            ..
        }) => {
            p.device_id = device_id.to_byte();
            p.group = group;
            p.enabled_count = reply.enabled().count();
            p.disabled_count = reply.disabled().count();
        }
        Some(Event::ProfileReport {
            device_id,
            group,
            kind,
            profile,
            channels,
            ..
        }) => {
            p.device_id = device_id.to_byte();
            p.group = group;
            p.profile = profile.to_bytes();
            p.channels = channels;
            p.report = match kind {
                ReportKind::Added => ZmidiCiProfileReport::Added,
                ReportKind::Removed => ZmidiCiProfileReport::Removed,
                ReportKind::Enabled => ZmidiCiProfileReport::Enabled,
                ReportKind::Disabled => ZmidiCiProfileReport::Disabled,
            };
        }
        Some(Event::SetProfile {
            address,
            profile,
            on,
            channels,
            ..
        }) => {
            p.device_id = address.device_id.to_byte();
            p.group = address.group;
            p.profile = profile.to_bytes();
            p.on = on;
            p.channels = channels;
        }
        Some(Event::ProfileDetailsInquiry {
            address,
            profile,
            target,
            ..
        }) => {
            p.device_id = address.device_id.to_byte();
            p.group = address.group;
            p.profile = profile.to_bytes();
            p.target = target;
        }
        Some(Event::ProfileDetailsReply {
            device_id,
            group,
            reply,
            notification,
            ..
        }) => {
            p.device_id = device_id.to_byte();
            p.group = group;
            p.profile = reply.profile.to_bytes();
            p.target = reply.target;
            p.notification = notification;
            p.data = reply.data.as_ptr();
            p.data_len = reply.data.len();
        }
        Some(Event::ProfileSpecificData {
            device_id,
            group,
            data,
            ..
        }) => {
            p.device_id = device_id.to_byte();
            p.group = group;
            p.profile = data.profile.to_bytes();
            p.data = data.data.as_ptr();
            p.data_len = data.data.len();
        }
        _ => return ZmidiStatus::NoPending,
    }
    // SAFETY: caller contract above.
    unsafe { pe.write(p) };
    ZmidiStatus::Ok
}

/// Copies the Profile IDs of the last ProfileInquiryReply event, 5 bytes each (M2-101 Table 18):
/// the Currently Enabled Profiles when `enabled`, else the Currently Disabled Profiles Supported.
/// `count` gets the number of IDs; `out` (`capacity` bytes) needs 5 x `count` bytes, else nothing
/// is copied and the status is buffer-too-small. An event that is not a ProfileInquiryReply gives
/// no-pending.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; the message of the event is still valid; `out` points to
/// `capacity` writable bytes (or `capacity` is 0); `count` is writable.
#[cfg(feature = "ci-profiles")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_profile_ids(
    ci: *mut ZmidiCi,
    enabled: bool,
    out: *mut u8,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if count.is_null() || (out.is_null() && capacity > 0) {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { state(ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Some(Event::ProfileInquiryReply { reply, .. }) = s.last else {
        return ZmidiStatus::NoPending;
    };
    let ids = if enabled {
        reply.enabled()
    } else {
        reply.disabled()
    };
    let n = ids.clone().count();
    // SAFETY: caller contract above.
    unsafe { count.write(n) };
    if capacity < 5 * n {
        return ZmidiStatus::BufferTooSmall;
    }
    if n > 0 {
        // SAFETY: caller contract above; `out` is not null when `capacity` is at least 5.
        let out = unsafe { core::slice::from_raw_parts_mut(out, capacity) };
        for (dst, id) in out.as_chunks_mut::<5>().0.iter_mut().zip(ids) {
            *dst = id.to_bytes();
        }
    }
    ZmidiStatus::Ok
}

// ---------------------------------------------------------------------------------------------
// Process Inquiry (M2-101 9): REQ-FFI-CI-013, REQ-FFI-CI-014.

/// The bitmaps of a MIDI Message Report (M2-101 Tables 43, 45): one byte each, reserved bits zero.
#[cfg(feature = "ci-process-inquiry")]
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ZmidiCiReport {
    /// Message Data Control (Table 44): 0x00 Begin and End only, 0x01 non-default values, 0x7F
    /// full report. Not used for the supported set of `zmidi_ci_message_report_reply`.
    pub data_control: u8,
    /// System Messages: D0 MTC Quarter Frame, D1 Song Position, D2 Song Select.
    pub system: u8,
    /// Channel Controller Messages: D0 Pitchbend, D1 Control Change, D2 RPN, D3 NRPN, D4 Program
    /// Change, D5 Channel Pressure.
    pub channel: u8,
    /// Note Data Messages: D0 Notes, D1 Poly Pressure, D2 Per-Note Pitchbend, D3 Registered
    /// Per-Note Controller, D4 Assignable Per-Note Controller.
    pub note: u8,
}

/// The Process Inquiry fields of the last event (M2-101 9).
#[cfg(feature = "ci-process-inquiry")]
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiCiProcessEvent {
    /// MessageReport: Device ID of the inquiry (0 for the other events).
    pub device_id: u8,
    /// MessageReport: UMP Group of the inquiry (0 for the other events).
    pub group: u8,
    /// PiCapabilities: D0 of Process Inquiry Supported Features, MIDI Message Report (Table 42).
    pub midi_message_report: bool,
    /// MessageReport: the requested bitmaps and Message Data Control; MessageReportReply: the
    /// bitmaps that the peer reports (`data_control` 0).
    pub report: ZmidiCiReport,
}

#[cfg(feature = "ci-process-inquiry")]
impl ZmidiCiReport {
    /// The inquiry of these bitmaps, or `None` for a byte over 0x7F.
    fn inquiry(&self) -> Option<MessageReportInquiry> {
        if (self.data_control | self.system | self.channel | self.note) > 0x7F {
            return None;
        }
        Some(MessageReportInquiry {
            data_control: DataControl::from_byte(self.data_control),
            system: SystemBits::from_byte(self.system),
            channel: ChannelBits::from_byte(self.channel),
            note: NoteBits::from_byte(self.note),
        })
    }
}

/// Writes Inquiry: Process Inquiry Capabilities (M2-101 9.2, Table 40) to `peer` as a record;
/// version 2 peers only. The reply gives a PiCapabilities event.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `out` points to `capacity` writable bytes; `out_len` is
/// writable.
#[cfg(all(feature = "ci-process-inquiry", feature = "ci-initiator"))]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_pi_capabilities_inquiry(
    ci: *mut ZmidiCi,
    peer: u32,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    let Some(peer) = muid(peer) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, 0, out, capacity, out_len, |dev, b| {
            dev.pi_capabilities_inquiry(peer, b)
        })
    }
}

/// Writes Inquiry: MIDI Message Report (M2-101 9.5, Table 43) with the bitmaps of `inquiry` to
/// `peer` for `device_id` (0x7F when System Messages are requested) as a record on `group`, and
/// opens the report Transaction; version 2 peers only.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `inquiry` is valid; `out` points to `capacity` writable
/// bytes; `out_len` is writable.
#[cfg(all(feature = "ci-process-inquiry", feature = "ci-initiator"))]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_message_report_inquiry(
    ci: *mut ZmidiCi,
    peer: u32,
    group: u8,
    device_id: u8,
    inquiry: *const ZmidiCiReport,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    if inquiry.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let q = unsafe { &*inquiry };
    let (Some(peer), Some(d), Some(q)) = (muid(peer), self::device_id(device_id), q.inquiry())
    else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, group, out, capacity, out_len, |dev, b| {
            dev.message_report_inquiry(peer, d, &q, b)
        })
    }
}

/// Gives true while a MIDI Message Report from `peer` is open: after
/// [`zmidi_ci_message_report_inquiry`], until End of MIDI Message Report.
///
/// # Safety
/// `ci` is null or was set up by [`zmidi_ci_init`].
#[cfg(all(feature = "ci-process-inquiry", feature = "ci-initiator"))]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_report_open(ci: *mut ZmidiCi, peer: u32) -> bool {
    // SAFETY: caller contract above.
    match (unsafe { state(ci) }, muid(peer)) {
        (Some(s), Some(peer)) => s.dev.report_open(peer),
        _ => false,
    }
}

/// Writes Reply to MIDI Message Report (M2-101 9.6, Table 45) to `peer` as a record on `group`,
/// with the Device ID `device_id` of the inquiry and the bitmaps that are in both `requested` (the
/// inquiry) and `supported` (REQ-CI-MMR-013): the answer to a MessageReport event. Then the program
/// sends the declared messages with their current values (M2-101 9.7) and calls
/// [`zmidi_ci_message_report_end`].
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `requested` and `supported` are valid; `out` points to
/// `capacity` writable bytes; `out_len` is writable.
#[cfg(feature = "ci-process-inquiry")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_message_report_reply(
    ci: *mut ZmidiCi,
    peer: u32,
    group: u8,
    device_id: u8,
    requested: *const ZmidiCiReport,
    supported: *const ZmidiCiReport,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    if requested.is_null() || supported.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (q, c) = unsafe { (&*requested, &*supported) };
    let (Some(peer), Some(d), Some(q), Some(c)) = (
        muid(peer),
        self::device_id(device_id),
        q.inquiry(),
        c.inquiry(),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    let supported = MessageReportReply {
        system: c.system,
        channel: c.channel,
        note: c.note,
    };
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, group, out, capacity, out_len, |dev, b| {
            let (me, framing) = (dev.muid(), dev.config().framing);
            let reply = MessageReportReply::declare(&q, &supported);
            Ok(reply.write(d, me, peer, framing, b)?)
        })
    }
}

/// Writes End of MIDI Message Report (M2-101 9.8, Table 46) to `peer` with the Device ID
/// `device_id` of the inquiry as a record on `group`.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `out` points to `capacity` writable bytes; `out_len` is
/// writable.
#[cfg(feature = "ci-process-inquiry")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_message_report_end(
    ci: *mut ZmidiCi,
    peer: u32,
    group: u8,
    device_id: u8,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    let (Some(peer), Some(d)) = (muid(peer), self::device_id(device_id)) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, group, out, capacity, out_len, |dev, b| {
            dev.message_report_end(peer, d, b)
        })
    }
}

/// Stops a MIDI Message Report to `peer`: writes a NAK with Status Code 0x20 (Responder terminates)
/// or, when `error`, 0x40 (M2-101 9.8.1, REQ-CI-MMR-036) as a record on `group`.
///
/// # Safety
/// As [`zmidi_ci_message_report_end`].
#[cfg(feature = "ci-process-inquiry")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_abort_report(
    ci: *mut ZmidiCi,
    peer: u32,
    group: u8,
    device_id: u8,
    error: bool,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> ZmidiStatus {
    let (Some(peer), Some(d)) = (muid(peer), self::device_id(device_id)) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_one(ci, group, out, capacity, out_len, |dev, b| {
            dev.abort_report(peer, d, error, b)
        })
    }
}

/// Writes the Process Inquiry fields of the last event of [`zmidi_ci_receive`] into `pe`. An event
/// that is not a Process Inquiry event gives no-pending.
///
/// # Safety
/// `ci` was set up by [`zmidi_ci_init`]; `pe` is writable.
#[cfg(feature = "ci-process-inquiry")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_ci_process_event(
    ci: *mut ZmidiCi,
    pe: *mut ZmidiCiProcessEvent,
) -> ZmidiStatus {
    if pe.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(s) = (unsafe { state(ci) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let mut p = ZmidiCiProcessEvent {
        device_id: 0,
        group: 0,
        midi_message_report: false,
        report: ZmidiCiReport::default(),
    };
    match s.last {
        Some(Event::PiCapabilities { capabilities, .. }) => {
            p.midi_message_report = capabilities.midi_message_report;
        }
        Some(Event::MessageReport {
            device_id,
            group,
            inquiry,
            ..
        }) => {
            p.device_id = device_id.to_byte();
            p.group = group;
            p.report = ZmidiCiReport {
                data_control: inquiry.data_control.to_byte(),
                system: inquiry.system.to_byte(),
                channel: inquiry.channel.to_byte(),
                note: inquiry.note.to_byte(),
            };
        }
        Some(Event::MessageReportReply { reply, .. }) => {
            p.report = ZmidiCiReport {
                data_control: 0,
                system: reply.system.to_byte(),
                channel: reply.channel.to_byte(),
                note: reply.note.to_byte(),
            };
        }
        Some(Event::MessageReportEnd { .. }) => {}
        _ => return ZmidiStatus::NoPending,
    }
    // SAFETY: caller contract above.
    unsafe { pe.write(p) };
    ZmidiStatus::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REQ-FFI-CI-001: the MUID generator is xorshift32 (13, 17, 5) over the seed of the program.
    #[test]
    fn muid_generator_is_xorshift32() {
        let mut s = 0x1234_5678;
        let got: [u32; 4] = core::array::from_fn(|_| xorshift(&mut s));
        assert_eq!(got, [0x8798_5aa5, 0x155b_24a3, 0x4820_f4c4, 0x81b3_ac98]);
    }

    /// REQ-FFI-ABI-001: each zmidi-ci Device error has its status.
    #[cfg(any(
        feature = "ci-initiator",
        feature = "ci-profiles",
        feature = "ci-process-inquiry"
    ))]
    #[test]
    fn device_errors_map_to_statuses() {
        use ZmidiStatus as S;
        assert_eq!(device_error(DeviceError::NotSupported), S::NotSupported);
        assert_eq!(device_error(DeviceError::InvalidConfig), S::InvalidArgument);
        assert_eq!(
            device_error(DeviceError::Write(WriteError::Overflow)),
            S::BufferTooSmall
        );
        assert_eq!(
            device_error(DeviceError::Write(WriteError::InvalidValue)),
            S::InvalidArgument
        );
        assert_eq!(device_error(DeviceError::UnknownPeer), S::Rejected);
    }

    /// REQ-FFI-CI-020: a bitmap or Message Data Control byte over 0x7F gives no inquiry, also when
    /// two bytes have the same high bit.
    #[cfg(feature = "ci-process-inquiry")]
    #[test]
    fn report_bytes_over_0x7f_are_refused() {
        let r = |data_control, system, channel, note| ZmidiCiReport {
            data_control,
            system,
            channel,
            note,
        };
        assert!(r(0x7F, 0x7F, 0x7F, 0x7F).inquiry().is_some());
        for bad in [
            r(0x80, 0, 0, 0),
            r(0, 0x80, 0, 0),
            r(0, 0, 0x80, 0),
            r(0, 0, 0, 0x80),
            r(0x80, 0x80, 0, 0),
            r(0, 0x80, 0x80, 0),
            r(0, 0, 0x80, 0x80),
        ] {
            assert!(bad.inquiry().is_none(), "{bad:?}");
        }
    }
}
