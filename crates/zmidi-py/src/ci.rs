//! A MIDI-CI Device with the one-chunk Property Exchange Responder (zmidi-ci, zmidi-ci-pe; M2-101,
//! M2-103), Profile Configuration (M2-101 7, M2-102) and Process Inquiry (M2-101 9): messages in,
//! replies and events out, and a call for each answer of the program.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use zmidi_ci::{
    Categories, ChannelBits, ChunkEnd, CiDevice as RsDevice, DataControl, DeviceConfig,
    DeviceError, DeviceId, DeviceIdentity, Direction, Event, Framing, ManufacturerId,
    MessageReportInquiry, MessageReportReply, Muid, NoteBits, PeChunk, PeKind, PiCapabilities,
    ProfileAddress, ProfileDetailsReply, ProfileEntry, ProfileId, ProfileReport,
    ProfileSpecificData, ReportKind, SystemBits,
};
use zmidi_ci_pe::header::{Header, HeaderKind, Status};
use zmidi_ci_pe::one_chunk::{respond, Resources};

use crate::{invalid, rejected};

type Device = RsDevice<'static, 4, 8, 4>;

/// The automatic replies of one message, as `(group, message)`.
type Replies<'py> = Vec<(u8, Bound<'py, PyBytes>)>;

/// The three bitmaps of a MIDI Message Report (M2-101 Tables 43, 45): System Messages, Channel
/// Controller Messages and Note Data Messages, one byte each.
type Bitmaps = (u8, u8, u8);

/// One event of a CI Device.
#[pyclass(frozen, module = "zetaohm_midi._native")]
#[derive(Debug, Clone)]
pub struct CiEvent {
    /// The kind: "discovered", "pe_get", "pe_get_reply", and so on (one for each zmidi-ci Event).
    #[pyo3(get)]
    kind: String,
    /// The MUID of the peer (the new MUID for "muid_changed"), when the event has one.
    #[pyo3(get)]
    muid: Option<u32>,
    /// Property Exchange: Request ID.
    #[pyo3(get)]
    request_id: Option<u8>,
    /// Property Exchange: Number of This Chunk.
    #[pyo3(get)]
    chunk_number: Option<u16>,
    /// Property Exchange: Number of Chunks in Message (0 when not known).
    #[pyo3(get)]
    chunk_total: Option<u16>,
    /// PE Get or Set: the decoded "resource".
    #[pyo3(get)]
    resource: Option<String>,
    /// PE Get or Set: the decoded "resId".
    #[pyo3(get)]
    res_id: Option<String>,
    /// Property Exchange: the Header Data.
    header: Vec<u8>,
    /// Property Exchange: the Property Data of this Chunk; Reply to Profile Details Inquiry: the
    /// Inquiry Target Data; Profile Specific Data: the data.
    data: Vec<u8>,
    /// Profile events and Inquiry: MIDI Message Report: the Device ID of the message (M2-101
    /// Table 4).
    #[pyo3(get)]
    device_id: Option<u8>,
    /// Profile events and Inquiry: MIDI Message Report: the UMP Group of the message; for
    /// "set_profile" and "profile_details_inquiry" the Group offset of the local Profile address.
    #[pyo3(get)]
    group: Option<u8>,
    /// Profile events except "profile_inquiry_reply": the Profile ID (M2-101 Table 19).
    profile: Option<[u8; 5]>,
    /// "profile_report": "added", "removed", "enabled" or "disabled" (M2-101 7.4, 7.5, 7.10,
    /// 7.11).
    #[pyo3(get)]
    report_kind: Option<String>,
    /// "set_profile": true for Set Profile On, false for Set Profile Off.
    #[pyo3(get)]
    on: Option<bool>,
    /// "set_profile" and "profile_report": the Number of Channels (requested, enabled or
    /// disabled).
    #[pyo3(get)]
    channels: Option<u16>,
    /// "profile_details_inquiry" and "profile_details_reply": the Inquiry Target.
    #[pyo3(get)]
    target: Option<u8>,
    /// "profile_details_reply": true when this Device did not ask for it (M2-101 7.7).
    #[pyo3(get)]
    notification: Option<bool>,
    /// "profile_inquiry_reply": the Currently Enabled Profiles (M2-101 Table 18).
    enabled_profiles: Vec<[u8; 5]>,
    /// "profile_inquiry_reply": the Currently Disabled Profiles.
    disabled_profiles: Vec<[u8; 5]>,
    /// "pi_capabilities": the MIDI Message Report bit of the Supported Features (M2-101 Table 42).
    #[pyo3(get)]
    midi_message_report: Option<bool>,
    /// "message_report": the Message Data Control (M2-101 Table 44).
    #[pyo3(get)]
    data_control: Option<u8>,
    /// "message_report": the requested bitmaps; "message_report_reply": the reported bitmaps.
    #[pyo3(get)]
    bitmaps: Option<Bitmaps>,
}

#[pymethods]
impl CiEvent {
    /// Property Exchange: the Header Data.
    #[getter]
    fn header<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.header)
    }

    /// The Property Data of a Chunk, the Inquiry Target Data or the Profile Specific Data.
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }

    /// The Profile ID (5 bytes) of a Profile event, or `None`.
    #[getter]
    fn profile<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.profile.map(|p| PyBytes::new(py, &p))
    }

    /// The Currently Enabled Profiles of a "profile_inquiry_reply", 5 bytes each.
    #[getter]
    fn enabled_profiles<'py>(&self, py: Python<'py>) -> Vec<Bound<'py, PyBytes>> {
        self.enabled_profiles
            .iter()
            .map(|p| PyBytes::new(py, p))
            .collect()
    }

    /// The Currently Disabled Profiles of a "profile_inquiry_reply", 5 bytes each.
    #[getter]
    fn disabled_profiles<'py>(&self, py: Python<'py>) -> Vec<Bound<'py, PyBytes>> {
        self.disabled_profiles
            .iter()
            .map(|p| PyBytes::new(py, p))
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "CiEvent(kind={:?}, muid={:?}, request_id={:?})",
            self.kind, self.muid, self.request_id
        )
    }
}

fn report_kind(k: ReportKind) -> &'static str {
    match k {
        ReportKind::Added => "added",
        ReportKind::Removed => "removed",
        ReportKind::Enabled => "enabled",
        ReportKind::Disabled => "disabled",
    }
}

fn event(e: &Event<'_>) -> CiEvent {
    let mut out = CiEvent {
        kind: String::new(),
        muid: None,
        request_id: None,
        chunk_number: None,
        chunk_total: None,
        resource: None,
        res_id: None,
        header: Vec::new(),
        data: Vec::new(),
        device_id: None,
        group: None,
        profile: None,
        report_kind: None,
        on: None,
        channels: None,
        target: None,
        notification: None,
        enabled_profiles: Vec::new(),
        disabled_profiles: Vec::new(),
        midi_message_report: None,
        data_control: None,
        bitmaps: None,
    };
    let (kind, muid) = match *e {
        Event::Discovered { muid, .. } => ("discovered", Some(muid)),
        Event::DiscoveryEnded => ("discovery_ended", None),
        Event::MuidChanged { new, .. } => ("muid_changed", Some(new)),
        Event::PeerInvalidated { muid } => ("peer_invalidated", Some(muid)),
        Event::EndpointInfo { muid, .. } => ("endpoint_info", Some(muid)),
        Event::Ack { muid, .. } => ("ack", Some(muid)),
        Event::Nak { muid, .. } => ("nak", Some(muid)),
        Event::ProfileInquiryReply {
            muid,
            device_id,
            group,
            reply,
        } => {
            out.device_id = Some(device_id.to_byte());
            out.group = Some(group);
            out.enabled_profiles = reply.enabled().map(ProfileId::to_bytes).collect();
            out.disabled_profiles = reply.disabled().map(ProfileId::to_bytes).collect();
            ("profile_inquiry_reply", Some(muid))
        }
        Event::ProfileReport {
            muid,
            device_id,
            group,
            kind,
            profile,
            channels,
        } => {
            out.device_id = Some(device_id.to_byte());
            out.group = Some(group);
            out.profile = Some(profile.to_bytes());
            out.channels = Some(channels);
            out.report_kind = Some(report_kind(kind).to_owned());
            ("profile_report", Some(muid))
        }
        Event::SetProfile {
            muid,
            address,
            profile,
            on,
            channels,
        } => {
            out.device_id = Some(address.device_id.to_byte());
            out.group = Some(address.group);
            out.profile = Some(profile.to_bytes());
            out.on = Some(on);
            out.channels = Some(channels);
            ("set_profile", Some(muid))
        }
        Event::ProfileDetailsInquiry {
            muid,
            address,
            profile,
            target,
        } => {
            out.device_id = Some(address.device_id.to_byte());
            out.group = Some(address.group);
            out.profile = Some(profile.to_bytes());
            out.target = Some(target);
            ("profile_details_inquiry", Some(muid))
        }
        Event::ProfileDetailsReply {
            muid,
            device_id,
            group,
            reply,
            notification,
        } => {
            out.device_id = Some(device_id.to_byte());
            out.group = Some(group);
            out.profile = Some(reply.profile.to_bytes());
            out.target = Some(reply.target);
            out.notification = Some(notification);
            out.data = reply.data.to_vec();
            ("profile_details_reply", Some(muid))
        }
        Event::ProfileSpecificData {
            muid,
            device_id,
            group,
            data,
        } => {
            out.device_id = Some(device_id.to_byte());
            out.group = Some(group);
            out.profile = Some(data.profile.to_bytes());
            out.data = data.data.to_vec();
            ("profile_specific_data", Some(muid))
        }
        Event::PeCapabilities { muid, .. } => ("pe_capabilities", Some(muid)),
        Event::PeChunk { muid, kind, chunk } => {
            out.request_id = Some(chunk.request_id);
            out.chunk_number = Some(chunk.number);
            out.chunk_total = Some(chunk.total);
            out.header = chunk.header.to_vec();
            out.data = chunk.data.to_vec();
            if matches!(kind, PeKind::GetInquiry | PeKind::SetInquiry) {
                if let Ok(h) = Header::parse(HeaderKind::Inquiry, chunk.header) {
                    let mut buf = [0u8; 64];
                    out.resource = h.resource.map(|r| r.decode_into(&mut buf).0.to_owned());
                    out.res_id = h.res_id.map(|r| r.decode_into(&mut buf).0.to_owned());
                }
            }
            let k = match kind {
                PeKind::GetInquiry => "pe_get",
                PeKind::SetInquiry => "pe_set",
                PeKind::Subscription => "pe_subscription",
                PeKind::GetReply => "pe_get_reply",
                PeKind::SetReply => "pe_set_reply",
                PeKind::SubscriptionReply => "pe_subscription_reply",
                PeKind::Notify => "pe_notify",
            };
            (k, Some(muid))
        }
        Event::PiCapabilities { muid, capabilities } => {
            out.midi_message_report = Some(capabilities.midi_message_report);
            ("pi_capabilities", Some(muid))
        }
        Event::MessageReport {
            muid,
            device_id,
            group,
            inquiry,
        } => {
            out.device_id = Some(device_id.to_byte());
            out.group = Some(group);
            out.data_control = Some(inquiry.data_control.to_byte());
            out.bitmaps = Some((
                inquiry.system.to_byte(),
                inquiry.channel.to_byte(),
                inquiry.note.to_byte(),
            ));
            ("message_report", Some(muid))
        }
        Event::MessageReportReply { muid, reply } => {
            out.bitmaps = Some((
                reply.system.to_byte(),
                reply.channel.to_byte(),
                reply.note.to_byte(),
            ));
            ("message_report_reply", Some(muid))
        }
        Event::MessageReportEnd { muid } => ("message_report_end", Some(muid)),
        Event::Rejected { .. } => ("rejected", None),
    };
    out.kind = kind.to_owned();
    out.muid = muid.map(Muid::value);
    out
}

fn xorshift(s: &mut u32) -> u32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    *s
}

fn device_error(e: DeviceError) -> PyErr {
    match e {
        DeviceError::InvalidConfig => invalid(e.to_string()),
        _ => rejected(e.to_string()),
    }
}

fn peer(muid: u32) -> PyResult<Muid> {
    Muid::new(muid).map_err(|_| invalid("a MUID has 28 bits"))
}

/// The Device ID `b`; a reserved value raises `InvalidValueError` (M2-101 Table 4).
fn device_id(b: u8) -> PyResult<DeviceId> {
    DeviceId::from_byte(b).map_err(|_| invalid("a reserved Device ID"))
}

/// The Profile ID of 5 bytes of 7 bits (M2-101 Table 19).
fn profile_id(b: &[u8]) -> PyResult<ProfileId> {
    let b: [u8; 5] = b
        .try_into()
        .map_err(|_| invalid("a Profile ID has 5 bytes"))?;
    ProfileId::from_bytes(b).map_err(|_| invalid("a Profile ID byte is over 0x7F"))
}

/// The Profile address of `device_id` and the Group offset `group` (0 to 15).
fn address(device_id: u8, group: u8) -> PyResult<ProfileAddress> {
    if group > 15 {
        return Err(invalid("a Group is 0 to 15"));
    }
    Ok(ProfileAddress {
        device_id: self::device_id(device_id)?,
        group,
    })
}

/// Checks a Number of Channels or a Maximum Number of Channels (14 bits).
fn channels(n: u16) -> PyResult<u16> {
    if n > 0x3FFF {
        return Err(invalid("a Number of Channels has 14 bits"));
    }
    Ok(n)
}

/// Checks a byte of 7 bits: an Inquiry Target, a bitmap or the Message Data Control.
fn u7(b: u8, what: &str) -> PyResult<u8> {
    if b > 0x7F {
        return Err(invalid(format!("{what} is over 0x7F")));
    }
    Ok(b)
}

/// The Inquiry: MIDI Message Report with `data_control` and `bitmaps` (M2-101 Table 43).
fn inquiry(data_control: u8, bitmaps: Bitmaps) -> PyResult<MessageReportInquiry> {
    Ok(MessageReportInquiry {
        data_control: DataControl::from_byte(u7(data_control, "the Message Data Control")?),
        system: SystemBits::from_byte(u7(bitmaps.0, "a bitmap")?),
        channel: ChannelBits::from_byte(u7(bitmaps.1, "a bitmap")?),
        note: NoteBits::from_byte(u7(bitmaps.2, "a bitmap")?),
    })
}

/// A Property Exchange request that waits for `pe_reply`: owned copies of its Header Data and
/// Property Data.
struct Pending {
    muid: Muid,
    kind: PeKind,
    request_id: u8,
    total: u16,
    number: u16,
    header: Box<[u8]>,
    data: Box<[u8]>,
}

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

/// A MIDI-CI Device (4 peers, 8 Profiles, 4 Property Exchange requests).
#[pyclass(module = "zetaohm_midi._native")]
pub struct CiDevice {
    // Declared first, so it is dropped before the texts that its configuration borrows.
    pub(crate) dev: Device,
    /// The event of the last `receive`, for `PeEngine.receive`; it points into `msg`, so it is
    /// declared (and dropped) before it.
    pub(crate) last: Option<Event<'static>>,
    /// A copy of the last received message.
    msg: Box<[u8]>,
    /// The peer and Request ID of a reply whose last Chunk the last `receive` gave. The next
    /// `receive` or a `PeEngine.receive` of the event frees the ID (REQ-PY-CI-007).
    ended: Option<(Muid, u8)>,
    rng: u32,
    pending: Option<Pending>,
    /// The ResourceList; `dev` holds no reference to it, `pe_reply` gives it to `respond`.
    list: Box<[u8]>,
    /// The Product Instance ID that the configuration of `dev` borrows.
    _pid: Option<Box<str>>,
}

#[pymethods]
impl CiDevice {
    /// A Device. `manufacturer`: the SysEx ID, 1 or 3 bytes. `revision`: 4 bytes. The other
    /// arguments are the fields of the zmidi-ci `DeviceConfig`. `seed` starts the MUID generator.
    #[new]
    #[pyo3(signature = (manufacturer, family=0, model=0, revision=b"\x00\x00\x00\x00".to_vec(), max_sysex_size=512,
        function_block=None, first_group=0, group_count=1, channels_in_use=1, product_instance_id=None,
        profiles=false, property_exchange=true, process_inquiry=false, sysex_framing=false,
        resource_list=b"[]".to_vec(), seed=1))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        manufacturer: Vec<u8>,
        family: u16,
        model: u16,
        revision: Vec<u8>,
        max_sysex_size: u32,
        function_block: Option<u8>,
        first_group: u8,
        group_count: u8,
        channels_in_use: u16,
        product_instance_id: Option<String>,
        profiles: bool,
        property_exchange: bool,
        process_inquiry: bool,
        sysex_framing: bool,
        resource_list: Vec<u8>,
        seed: u32,
    ) -> PyResult<Self> {
        let manufacturer = match manufacturer.as_slice() {
            [id] => ManufacturerId::OneByte(*id),
            [a, b, c] => ManufacturerId::ThreeByte([*a, *b, *c]),
            _ => return Err(invalid("a manufacturer ID has 1 or 3 bytes")),
        };
        let revision: [u8; 4] = revision
            .try_into()
            .map_err(|_| invalid("a revision has 4 bytes"))?;
        let pid: Option<Box<str>> = product_instance_id.map(String::into_boxed_str);
        // SAFETY: the box is owned by the returned object and dropped after `dev` (field order).
        let pid_ref: Option<&'static str> = pid.as_deref().map(|s| unsafe { &*(s as *const str) });
        let config = DeviceConfig {
            identity: DeviceIdentity {
                manufacturer,
                family,
                model,
                revision,
            },
            categories: Categories {
                profile_configuration: profiles,
                property_exchange,
                process_inquiry,
                ..Categories::default()
            },
            max_sysex_size,
            function_block,
            first_group,
            group_count,
            channels_in_use,
            product_instance_id: pid_ref,
            framing: if sysex_framing {
                Framing::SysEx
            } else {
                Framing::Bare
            },
            process_inquiry: PiCapabilities {
                midi_message_report: process_inquiry,
            },
        };
        let mut rng = seed | 1;
        let dev = Device::new(config, &mut || xorshift(&mut rng)).map_err(device_error)?;
        Ok(Self {
            dev,
            rng,
            pending: None,
            last: None,
            msg: Box::default(),
            ended: None,
            list: resource_list.into_boxed_slice(),
            _pid: pid,
        })
    }

    /// The MUID of the Device.
    #[getter]
    fn muid(&self) -> u32 {
        self.dev.muid().value()
    }

    /// Reads one MIDI-CI message `msg` received on `group` at `now_us`. Gives `(replies, event)`:
    /// the automatic replies as `(group, message)` and the event (or `None`). After a "pe_get",
    /// "pe_set" or "pe_subscription" event, answer with `pe_reply`, or give the event to
    /// `PeEngine.receive`. The device keeps a copy of the message for `PeEngine.receive`.
    fn receive<'py>(
        &mut self,
        py: Python<'py>,
        group: u8,
        msg: &[u8],
        now_us: u64,
    ) -> PyResult<(Replies<'py>, Option<CiEvent>)> {
        let mut rng = self.rng;
        let mut replies = Vec::new();
        self.end_reply();
        self.last = None;
        self.msg = msg.into();
        // SAFETY: the box is owned by `self` and replaced only after `last` is cleared above; `last`
        // is dropped before it (field order).
        let msg: &'static [u8] = unsafe { &*(&*self.msg as *const [u8]) };
        let ev = {
            let mut r = self
                .dev
                .receive(group, msg, now_us, &mut || xorshift(&mut rng));
            let mut buf = [0u8; Device::MAX_REPLY_LEN];
            while let Some(w) = r.write_next(&mut buf) {
                let (g, n) = w.map_err(|e| rejected(e.to_string()))?;
                replies.push((g, PyBytes::new(py, &buf[..n])));
            }
            r.event()
        };
        self.rng = rng;
        self.pending = None;
        self.last = ev;
        let out = ev.as_ref().map(event);
        if let Some(Event::PeChunk { muid, kind, chunk }) = ev {
            // The last Chunk of a reply ends the request of this Device (M2-101 Table 29).
            if matches!(
                kind,
                PeKind::GetReply | PeKind::SetReply | PeKind::SubscriptionReply
            ) && chunk.end() != ChunkEnd::More
            {
                self.ended = Some((muid, chunk.request_id));
            }
            if matches!(
                kind,
                PeKind::GetInquiry | PeKind::SetInquiry | PeKind::Subscription
            ) {
                self.pending = Some(Pending {
                    muid,
                    kind,
                    request_id: chunk.request_id,
                    total: chunk.total,
                    number: chunk.number,
                    header: chunk.header.into(),
                    data: chunk.data.into(),
                });
            }
        }
        Ok((replies, out))
    }

    /// Runs the timers at `now_us` and gives their event, or `None`.
    fn poll(&mut self, now_us: u64) -> Option<CiEvent> {
        self.dev.poll(now_us).as_ref().map(event)
    }

    /// The Discovery message for the output path `output_path_id` at `now_us`.
    #[pyo3(signature = (output_path_id=0, now_us=0))]
    fn discovery<'py>(
        &mut self,
        py: Python<'py>,
        output_path_id: u8,
        now_us: u64,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let mut buf = [0u8; 64];
        let n = self
            .dev
            .discovery(output_path_id, now_us, &mut buf)
            .map_err(device_error)?;
        Ok(PyBytes::new(py, &buf[..n]))
    }

    /// The Inquiry: Endpoint Information message to `peer`.
    fn endpoint_info_inquiry<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let mut buf = [0u8; 64];
        let n = self
            .dev
            .endpoint_info_inquiry(self::peer(peer)?, &mut buf)
            .map_err(device_error)?;
        Ok(PyBytes::new(py, &buf[..n]))
    }

    /// The Inquiry: Property Exchange Capabilities message to `peer`.
    fn pe_capabilities_inquiry<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let mut buf = [0u8; 64];
        let n = self
            .dev
            .pe_capabilities_inquiry(self::peer(peer)?, &mut buf)
            .map_err(device_error)?;
        Ok(PyBytes::new(py, &buf[..n]))
    }

    /// An Inquiry: Get Property Data with `header` to `peer`. Gives `(request_id, chunks)`.
    fn pe_get<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        header: &[u8],
    ) -> PyResult<(u8, Vec<Bound<'py, PyBytes>>)> {
        self.request(py, peer, PeKind::GetInquiry, header, &[])
    }

    /// An Inquiry: Set Property Data with `header` and `data` to `peer`. Gives
    /// `(request_id, chunks)`.
    fn pe_set<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        header: &[u8],
        data: &[u8],
    ) -> PyResult<(u8, Vec<Bound<'py, PyBytes>>)> {
        self.request(py, peer, PeKind::SetInquiry, header, data)
    }

    /// Frees the Request ID `request_id` of a `pe_get` or `pe_set` to `peer` that has no reply
    /// (a timeout or a NAK). The last Chunk of a reply frees the ID without this call. Not for a
    /// request of a `PeEngine`.
    fn pe_end(&mut self, peer: u32, request_id: u8) -> PyResult<()> {
        let peer = self::peer(peer)?;
        if request_id > 0x7F {
            return Err(invalid("a Request ID is 0 to 127"));
        }
        if self.ended == Some((peer, request_id)) {
            self.ended = None;
        }
        self.dev.pe_end(peer, request_id, Direction::Initiator);
        Ok(())
    }

    /// Answers the pending Property Exchange request with `status` (200: success) and, for a
    /// Get, `data`. Gives the reply Chunks. Raises `RejectedError` when no request is pending.
    #[pyo3(signature = (status, data=Vec::new()))]
    fn pe_reply<'py>(
        &mut self,
        py: Python<'py>,
        status: u16,
        data: Vec<u8>,
    ) -> PyResult<Vec<Bound<'py, PyBytes>>> {
        let p = self
            .pending
            .take()
            .ok_or_else(|| rejected("no pending Property Exchange request"))?;
        let chunk = PeChunk {
            request_id: p.request_id,
            header: &p.header,
            total: p.total,
            number: p.number,
            data: &p.data,
        };
        let split = data.len().max(self.list.len());
        let mut body = vec![0u8; split];
        let mut buf = vec![0u8; split + 64];
        let mut chunks = Vec::new();
        respond(
            &mut self.dev,
            &Event::PeChunk {
                muid: p.muid,
                kind: p.kind,
                chunk,
            },
            &self.list,
            &mut Answer {
                status,
                data: &data,
            },
            &mut body,
            &mut buf,
            |m| chunks.push(PyBytes::new(py, m)),
        );
        Ok(chunks)
    }

    // ----- Profile Configuration (M2-101 7, M2-102) -----

    /// Adds a local Profile `profile` (5 bytes) at `device_id` and the Group offset `group`, with
    /// `channels` and `max_channels` (14 bits each). Gives the Profile Added Report and, when
    /// `enabled`, the Profile Enabled Report as `(group, message)` (M2-101 7.4). Raises
    /// `RejectedError` for a Profile in the table or a full table.
    #[pyo3(signature = (device_id, group, profile, enabled=false, channels=0, max_channels=0))]
    #[allow(clippy::too_many_arguments)]
    fn add_profile<'py>(
        &mut self,
        py: Python<'py>,
        device_id: u8,
        group: u8,
        profile: &[u8],
        enabled: bool,
        channels: u16,
        max_channels: u16,
    ) -> PyResult<Replies<'py>> {
        let entry = ProfileEntry {
            address: address(device_id, group)?,
            profile: profile_id(profile)?,
            enabled,
            channels: self::channels(channels)?,
            max_channels: self::channels(max_channels)?,
        };
        let reports: Vec<ProfileReport> =
            self.dev.add_profile(entry).map_err(device_error)?.collect();
        self.reports(py, reports)
    }

    /// Removes the local Profile `profile` at `device_id` and `group`. Gives the Profile Disabled
    /// Report of an enabled Profile and the Profile Removed Report as `(group, message)` (M2-101
    /// 7.5). Raises `RejectedError` for a Profile that is not in the table.
    fn remove_profile<'py>(
        &mut self,
        py: Python<'py>,
        device_id: u8,
        group: u8,
        profile: &[u8],
    ) -> PyResult<Replies<'py>> {
        let (a, id) = (address(device_id, group)?, profile_id(profile)?);
        let reports: Vec<ProfileReport> = self
            .dev
            .remove_profile(a, id)
            .ok_or_else(|| rejected("the Profile is not in the table"))?
            .collect();
        self.reports(py, reports)
    }

    /// Sets the local Profile `profile` at `device_id` and `group` to `enabled` with `channels`
    /// Channels: the answer to a "set_profile" event (False refuses a Set Profile On), or a local
    /// change. Gives the Profile Enabled or Disabled Report as `(group, message)` (M2-101 7.10,
    /// 7.11). Raises `RejectedError` for a Profile that is not in the table.
    fn set_profile_state<'py>(
        &mut self,
        py: Python<'py>,
        device_id: u8,
        group: u8,
        profile: &[u8],
        enabled: bool,
        channels: u16,
    ) -> PyResult<(u8, Bound<'py, PyBytes>)> {
        let (a, id, n) = (
            address(device_id, group)?,
            profile_id(profile)?,
            self::channels(channels)?,
        );
        let report = self
            .dev
            .set_profile_state(a, id, enabled, n)
            .ok_or_else(|| rejected("the Profile is not in the table"))?;
        let mut out = self.reports(py, vec![report])?;
        out.pop()
            .ok_or_else(|| rejected("zmidi-ci wrote no report"))
    }

    /// The Reply to Profile Details Inquiry (M2-101 7.7, Table 23) for `profile` and the Inquiry
    /// Target `target` with the Inquiry Target Data `data`, to `peer` with `device_id`: the answer
    /// to a "profile_details_inquiry" event.
    fn profile_details_reply<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        device_id: u8,
        profile: &[u8],
        target: u8,
        data: &[u8],
    ) -> PyResult<Bound<'py, PyBytes>> {
        let (peer, d) = (self::peer(peer)?, self::device_id(device_id)?);
        let reply = ProfileDetailsReply {
            profile: profile_id(profile)?,
            target: u7(target, "an Inquiry Target")?,
            data,
        };
        write(py, data.len(), |b| {
            self.dev.profile_details_reply(peer, d, &reply, b)
        })
    }

    /// The Profile Specific Data message (M2-101 7.12, Table 28) for `profile` with `data` to
    /// `peer` (0x0FFFFFFF: Broadcast) with `device_id`.
    fn profile_specific_data<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        device_id: u8,
        profile: &[u8],
        data: &[u8],
    ) -> PyResult<Bound<'py, PyBytes>> {
        let (peer, d) = (self::peer(peer)?, self::device_id(device_id)?);
        let psd = ProfileSpecificData {
            profile: profile_id(profile)?,
            data,
        };
        let (me, framing) = (self.dev.muid(), self.dev.config().framing);
        write(py, data.len(), |b| {
            Ok(psd.write(false, d, me, peer, framing, b)?)
        })
    }

    /// The Profile Inquiry message (M2-101 7.2, Table 17) to `peer` for the address `device_id`.
    /// The reply gives a "profile_inquiry_reply" event.
    fn profile_inquiry<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        device_id: u8,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let (peer, d) = (self::peer(peer)?, self::device_id(device_id)?);
        write(py, 0, |b| self.dev.profile_inquiry(peer, d, b))
    }

    /// The Set Profile On message (M2-101 7.8, Table 24) for `profile` with `channels` Number of
    /// Channels Requested to `peer` with `device_id`. `max_channels` is the Maximum Number of
    /// Channels that the peer reported, or None. Raises `RejectedError` before a Profile Inquiry
    /// to the peer (M2-101 2.2).
    #[pyo3(signature = (peer, device_id, profile, channels, max_channels=None))]
    fn set_profile_on<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        device_id: u8,
        profile: &[u8],
        channels: u16,
        max_channels: Option<u16>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let (peer, d, id) = (
            self::peer(peer)?,
            self::device_id(device_id)?,
            profile_id(profile)?,
        );
        let n = self::channels(channels)?;
        let max = max_channels.map(self::channels).transpose()?;
        write(py, 0, |b| self.dev.set_profile_on(peer, d, id, n, max, b))
    }

    /// The Set Profile Off message (M2-101 7.9, Table 25) for `profile` to `peer` with
    /// `device_id`. Raises `RejectedError` before a Profile Inquiry to the peer.
    fn set_profile_off<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        device_id: u8,
        profile: &[u8],
    ) -> PyResult<Bound<'py, PyBytes>> {
        let (peer, d, id) = (
            self::peer(peer)?,
            self::device_id(device_id)?,
            profile_id(profile)?,
        );
        write(py, 0, |b| self.dev.set_profile_off(peer, d, id, b))
    }

    /// The Profile Details Inquiry message (M2-101 7.6, Table 22) for `profile` and the Inquiry
    /// Target `target` to `peer` with `device_id`; Message Version 2 peers only.
    fn profile_details_inquiry<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        device_id: u8,
        profile: &[u8],
        target: u8,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let (peer, d, id) = (
            self::peer(peer)?,
            self::device_id(device_id)?,
            profile_id(profile)?,
        );
        let t = u7(target, "an Inquiry Target")?;
        write(py, 0, |b| {
            self.dev.profile_details_inquiry(peer, d, id, t, b)
        })
    }

    // ----- Process Inquiry (M2-101 9) -----

    /// The Inquiry: Process Inquiry Capabilities message (M2-101 9.2, Table 40) to `peer`;
    /// Message Version 2 peers only. The reply gives a "pi_capabilities" event.
    fn pi_capabilities_inquiry<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let peer = self::peer(peer)?;
        write(py, 0, |b| self.dev.pi_capabilities_inquiry(peer, b))
    }

    /// The Inquiry: MIDI Message Report message (M2-101 9.5, Table 43) to `peer` with
    /// `device_id` (0x7F when System Messages are requested), the Message Data Control
    /// `data_control` and the `bitmaps` (system, channel, note). It opens the report.
    fn message_report_inquiry<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        device_id: u8,
        data_control: u8,
        bitmaps: Bitmaps,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let (peer, d) = (self::peer(peer)?, self::device_id(device_id)?);
        let q = inquiry(data_control, bitmaps)?;
        write(py, 0, |b| self.dev.message_report_inquiry(peer, d, &q, b))
    }

    /// True while a MIDI Message Report from `peer` is open: after `message_report_inquiry`,
    /// until End of MIDI Message Report.
    fn report_open(&self, peer: u32) -> PyResult<bool> {
        Ok(self.dev.report_open(self::peer(peer)?))
    }

    /// The Reply to MIDI Message Report (M2-101 9.6, Table 45) to `peer` with the Device ID
    /// `device_id` of the inquiry and the bitmaps that are in both `requested` (the bitmaps of the
    /// "message_report" event) and `supported`. Then the program sends the reported messages
    /// (M2-101 9.7) and `message_report_end`.
    fn message_report_reply<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        device_id: u8,
        requested: Bitmaps,
        supported: Bitmaps,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let (peer, d) = (self::peer(peer)?, self::device_id(device_id)?);
        let q = inquiry(0, requested)?;
        let c = inquiry(0, supported)?;
        let supported = MessageReportReply {
            system: c.system,
            channel: c.channel,
            note: c.note,
        };
        let reply = MessageReportReply::declare(&q, &supported);
        let (me, framing) = (self.dev.muid(), self.dev.config().framing);
        write(py, 0, |b| Ok(reply.write(d, me, peer, framing, b)?))
    }

    /// The End of MIDI Message Report message (M2-101 9.8, Table 46) to `peer` with the Device ID
    /// `device_id` of the inquiry.
    fn message_report_end<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        device_id: u8,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let (peer, d) = (self::peer(peer)?, self::device_id(device_id)?);
        write(py, 0, |b| self.dev.message_report_end(peer, d, b))
    }

    /// Stops a MIDI Message Report to `peer`: a NAK with Status Code 0x20 (the Responder stops)
    /// or, when `error`, 0x40 (M2-101 9.8.1).
    #[pyo3(signature = (peer, device_id, error=false))]
    fn abort_report<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        device_id: u8,
        error: bool,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let (peer, d) = (self::peer(peer)?, self::device_id(device_id)?);
        write(py, 0, |b| self.dev.abort_report(peer, d, error, b))
    }
}

/// Writes one message with `write` into a buffer of `Device::MAX_REPLY_LEN` bytes, or more for
/// `extra` bytes of data, and gives it.
fn write<'py>(
    py: Python<'py>,
    extra: usize,
    write: impl FnOnce(&mut [u8]) -> Result<usize, DeviceError>,
) -> PyResult<Bound<'py, PyBytes>> {
    let mut buf = vec![0u8; Device::MAX_REPLY_LEN.max(extra + 64)];
    let n = write(&mut buf).map_err(device_error)?;
    Ok(PyBytes::new(py, &buf[..n]))
}

impl CiDevice {
    /// Frees the Request ID of the reply that the last `receive` ended. The free waits until the
    /// event is used, so a request in that time gets another ID.
    pub(crate) fn end_reply(&mut self) {
        if let Some((peer, id)) = self.ended.take() {
            self.dev.pe_end(peer, id, Direction::Initiator);
        }
    }

    /// The Profile reports `reports` as `(group, message)`.
    fn reports<'py>(&self, py: Python<'py>, reports: Vec<ProfileReport>) -> PyResult<Replies<'py>> {
        let mut buf = [0u8; Device::MAX_REPLY_LEN];
        reports
            .iter()
            .map(|r| {
                let (g, n) = self.dev.profile_report(r, &mut buf).map_err(device_error)?;
                Ok((g, PyBytes::new(py, &buf[..n])))
            })
            .collect()
    }

    fn request<'py>(
        &mut self,
        py: Python<'py>,
        peer: u32,
        kind: PeKind,
        header: &[u8],
        data: &[u8],
    ) -> PyResult<(u8, Vec<Bound<'py, PyBytes>>)> {
        let peer = self::peer(peer)?;
        let (id, mut w) = self
            .dev
            .pe_request(peer, kind, header, Some(data.len()), None)
            .map_err(device_error)?;
        let mut chunks = Vec::new();
        let mut buf = vec![0u8; header.len() + data.len() + 64];
        let mut sent = 0;
        loop {
            let room = w.data_room().min(data.len() - sent);
            match w.write_next(&data[sent..sent + room], &mut buf) {
                Ok(n) => chunks.push(PyBytes::new(py, &buf[..n])),
                Err(e) => {
                    self.dev.pe_end(peer, id, Direction::Initiator);
                    return Err(rejected(e.to_string()));
                }
            }
            sent += room;
            if sent >= data.len() {
                break;
            }
        }
        Ok((id, chunks))
    }
}
