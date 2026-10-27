//! The sans-IO MIDI-CI Device of one Function Block (M2-101-UM v1.2.1 sections 3-9,
//! Appendices F and H; DEC-009).
//!
//! - One [`CiDevice`] serves one Function Block (or USB Group Terminal Block) with its own MUID
//!   (3.2, REQ-CI-MUID-001). The caller gives time (`now_us`) and random numbers (a closure);
//!   the crate has no clock and no random source (DEC-009).
//! - [`CiDevice::receive`] takes one complete MIDI-CI message and gives an [`Event`] for the
//!   application and the automatic replies: Reply to Discovery, Reply to Endpoint Information,
//!   Reply to Profile Inquiry, Reply to PE and PI Capabilities, Invalidate MUID and NAKs.
//! - The Initiator writers fill a caller buffer. Replies that need application data (Profile
//!   state, Profile details, PE data, MIDI Message Report) are written after the event.
//! - The receive buffer is the caller's: a fixed buffer of at least 512 bytes, for example
//!   `zmidi_ump::SysEx7Reassembler<512>` (REQ-CI-LEN-001, REQ-CI-LEN-002).
//! - The Message Version is fixed (`Version::CURRENT`); a version change is a new build and
//!   so a new MUID at start (REQ-CI-MSG-008, REQ-CI-MSG-011).
//! - DEC-011: D-5 echo test, D-6 collision option, D-7 NAK codes, D-10 Device ID check,
//!   D-11 ACK/NAK size, D-12 Endpoint Information reply, D-14 peer Profile lists are the
//!   application's.

use crate::management::{
    Ack, Categories, Details, DeviceIdentity, Discovery, DiscoveryReply, EndpointInfoInquiry,
    EndpointInfoReply, EndpointInfoStatus, InvalidateMuid, Nak, NakStatus,
};
use crate::message::{
    Category, DeviceId, Framing, Header, Muid, ParseError, SubId2, Version, WriteError,
};
// DEC-017: some of these types are used only when their category feature is on.
#[cfg_attr(not(feature = "full"), allow(unused_imports))]
use crate::process::{
    MessageReportEnd, MessageReportInquiry, MessageReportReply, PiCapabilities,
    PiCapabilitiesInquiry, Protocol, ReportSource, ReportWriter,
};
#[cfg_attr(not(feature = "full"), allow(unused_imports))]
use crate::profile::{
    InquiryReplies, ProfileAddedReport, ProfileAddress, ProfileDetailsInquiry, ProfileDetailsReply,
    ProfileDisabledReport, ProfileEnabledReport, ProfileEntry, ProfileId, ProfileInquiry,
    ProfileInquiryReply, ProfileLevel, ProfileRemovedReport, ProfileReport, ProfileSpecificData,
    ProfileTable, SetProfileOff, SetProfileOn,
};
#[cfg_attr(not(feature = "full"), allow(unused_imports))]
use crate::property::{ChunkWriter, Direction, PeCapabilities, PeChunk, PeKind, RequestTable};

/// Discovery window: replies are accepted for at least 3 s (5.5, REQ-CI-DISC-011).
#[cfg(feature = "initiator")]
const DISCOVERY_WINDOW_US: u64 = 3_000_000;
/// Size used when a peer reported no Receivable Maximum SysEx Message Size (DEC-011 D-11).
const DEFAULT_MAX_SYSEX: u32 = 128;
/// Minimum Receivable Maximum SysEx Message Size (Appendix F, REQ-CI-LEN-001 to -003).
const MIN_SYSEX: u32 = 128;
const MIN_SYSEX_PROFILE_PE: u32 = 512;
/// Number of open Profile Details Inquiries that are remembered.
#[cfg(feature = "profiles")]
const DETAILS_PENDING: usize = 4;

/// Configuration of one Device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceConfig<'a> {
    /// Device identity for Discovery and Reply to Discovery.
    pub identity: DeviceIdentity,
    /// MIDI-CI Categories supported.
    pub categories: Categories,
    /// Receivable Maximum SysEx Message Size: at least 128, at least 512 with Profiles or PE.
    pub max_sysex_size: u32,
    /// Function Block number, `None` for a Device with no Function Block (REQ-CI-DISC-014).
    pub function_block: Option<u8>,
    /// First Group of the Function Block, 0-15.
    pub first_group: u8,
    /// Number of Groups of the Function Block, 1-16.
    pub group_count: u8,
    /// Channels in use on each Group: bit n is Channel n+1 (REQ-CI-MMR-010).
    pub channels_in_use: u16,
    /// Product Instance ID: at most 42 bytes of ASCII 32-126 (REQ-CI-EPI-007).
    pub product_instance_id: Option<&'a str>,
    /// Framing of written messages.
    pub framing: Framing,
    /// Process Inquiry features.
    pub process_inquiry: PiCapabilities,
}

/// Error of the Device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    /// The configuration breaks a rule of the specification.
    #[error("invalid configuration")]
    InvalidConfig,
    /// The MUID is not a discovered peer.
    #[error("unknown peer")]
    UnknownPeer,
    /// A Profile Inquiry (REQ-CI-GEN-003) or a PE Capabilities reply (REQ-CI-PEF-031) is
    /// needed first.
    #[error("inquiry needed first")]
    NotInquired,
    /// The message is newer than the peer's Message Version (REQ-CI-MSG-010).
    #[error("message not in the peer's version")]
    PeerVersion,
    /// More Channels than the Profile allows (REQ-CI-PRFR-021).
    #[error("too many channels")]
    TooManyChannels,
    /// No free Request ID or table entry.
    #[error("busy")]
    Busy,
    /// The message or Category is not supported.
    #[error("not supported")]
    NotSupported,
    /// A message writer failed.
    #[error("write: {0}")]
    Write(#[from] WriteError),
}

/// A discovered Device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    /// MUID of the peer.
    pub muid: Muid,
    /// Receivable Maximum SysEx Message Size of the peer.
    pub max_sysex_size: u32,
    /// Message Version of the peer's Discovery or Reply to Discovery.
    pub version: Version,
    /// Categories of the peer.
    pub categories: Categories,
    /// True after this Device sent a Profile Inquiry to the peer.
    pub profiles_inquired: bool,
    /// PE Capabilities of the peer, kept until the MUID is invalidated (REQ-CI-PEF-032).
    pub pe_capabilities: Option<PeCapabilities>,
    /// Discovery window in which the peer replied, and the Output Path ID of that reply, for
    /// REQ-CI-MUID-015.
    #[cfg(feature = "initiator")]
    window: u32,
    #[cfg(feature = "initiator")]
    path: Option<u8>,
}

/// Kind of a Profile report from another Device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    /// Profile Added Report.
    Added,
    /// Profile Removed Report.
    Removed,
    /// Profile Enabled Report.
    Enabled,
    /// Profile Disabled Report.
    Disabled,
}

/// An event for the application. `group` is the UMP Group of the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event<'m> {
    /// A Reply to Discovery (REQ-CI-DISC-003).
    Discovered { muid: Muid, reply: DiscoveryReply },
    /// The Discovery window ended.
    DiscoveryEnded,
    /// This Device has a new MUID (collision or Invalidate MUID); send a new Discovery.
    MuidChanged { old: Muid, new: Muid },
    /// A peer MUID was invalidated: discard its cached data (REQ-CI-INV-007).
    PeerInvalidated { muid: Muid },
    /// A Reply to Endpoint Information.
    EndpointInfo {
        muid: Muid,
        reply: EndpointInfoReply<'m>,
    },
    /// A MIDI-CI ACK.
    Ack {
        muid: Muid,
        device_id: DeviceId,
        ack: Ack<'m>,
    },
    /// A MIDI-CI NAK.
    Nak {
        muid: Muid,
        device_id: DeviceId,
        nak: Nak<'m>,
    },
    /// A Reply to Profile Inquiry.
    ProfileInquiryReply {
        muid: Muid,
        device_id: DeviceId,
        group: u8,
        reply: ProfileInquiryReply<'m>,
    },
    /// A Profile report (REQ-CI-PRFR-010 to -013).
    ProfileReport {
        muid: Muid,
        device_id: DeviceId,
        group: u8,
        kind: ReportKind,
        profile: ProfileId,
        channels: u16,
    },
    /// Set Profile On (`on`) or Off for a local Profile: call `set_profile_state`.
    SetProfile {
        muid: Muid,
        address: ProfileAddress,
        profile: ProfileId,
        on: bool,
        channels: u16,
    },
    /// Profile Details Inquiry for a target other than 0x00: call `profile_details_reply`.
    ProfileDetailsInquiry {
        muid: Muid,
        address: ProfileAddress,
        profile: ProfileId,
        target: u8,
    },
    /// Reply to Profile Details Inquiry; `notification` when not requested (REQ-CI-PRFR-016).
    ProfileDetailsReply {
        muid: Muid,
        device_id: DeviceId,
        group: u8,
        reply: ProfileDetailsReply<'m>,
        notification: bool,
    },
    /// Profile Specific Data.
    ProfileSpecificData {
        muid: Muid,
        device_id: DeviceId,
        group: u8,
        data: ProfileSpecificData<'m>,
    },
    /// Reply to PE Capabilities.
    PeCapabilities {
        muid: Muid,
        capabilities: PeCapabilities,
    },
    /// One Property Exchange Chunk for the application or `zmidi-ci-pe`.
    PeChunk {
        muid: Muid,
        kind: PeKind,
        chunk: PeChunk<'m>,
    },
    /// Reply to Process Inquiry Capabilities.
    PiCapabilities {
        muid: Muid,
        capabilities: PiCapabilities,
    },
    /// Inquiry: MIDI Message Report: call `message_report`.
    MessageReport {
        muid: Muid,
        device_id: DeviceId,
        group: u8,
        inquiry: MessageReportInquiry,
    },
    /// Reply to MIDI Message Report.
    MessageReportReply {
        muid: Muid,
        reply: MessageReportReply,
    },
    /// End of MIDI Message Report.
    MessageReportEnd { muid: Muid },
    /// A message that the reader rejected (REQ-CI-GEN-004).
    Rejected { error: ParseError },
}

/// `categories` with the categories that this build does not have cleared (DEC-017).
const fn compiled(mut c: Categories) -> Categories {
    c.profile_configuration &= cfg!(feature = "profiles");
    c.property_exchange &= cfg!(feature = "property-exchange");
    c.process_inquiry &= cfg!(feature = "process-inquiry");
    c
}

/// An automatic reply, written on demand.
#[derive(Debug, Clone, Copy)]
enum Reply {
    Discovery {
        destination: Muid,
        reply: DiscoveryReply,
    },
    /// The Information Data is the Product Instance ID of the configuration.
    EndpointInfo {
        destination: Muid,
        status: EndpointInfoStatus,
    },
    Invalidate {
        target: Muid,
    },
    Nak {
        destination: Muid,
        device_id: DeviceId,
        original: u8,
        status: NakStatus,
        details: Details,
        max: Option<u32>,
    },
    #[cfg(feature = "property-exchange")]
    PeCapabilities {
        destination: Muid,
        capabilities: PeCapabilities,
    },
    #[cfg(feature = "process-inquiry")]
    PiCapabilities {
        destination: Muid,
        capabilities: PiCapabilities,
    },
    #[cfg(feature = "profiles")]
    ProfileDetails {
        destination: Muid,
        entry: ProfileEntry,
    },
}

/// Result of [`CiDevice::receive`]: the event and the automatic replies.
#[derive(Debug)]
pub struct Received<'d, 'm, const PROFILES: usize> {
    event: Option<Event<'m>>,
    queue: [Option<Reply>; 2],
    /// Read only with the feature `profiles` (DEC-017).
    #[cfg_attr(not(feature = "profiles"), allow(dead_code))]
    profiles: Option<InquiryReplies<'d, PROFILES>>,
    source: Muid,
    group: u8,
    #[cfg_attr(not(feature = "profiles"), allow(dead_code))]
    first_group: u8,
    framing: Framing,
    /// Product Instance ID of the configuration, for the Reply to Endpoint Information.
    instance_id: &'d [u8],
}

impl<'m, const PROFILES: usize> Received<'_, 'm, PROFILES> {
    /// The event of the message, if any.
    pub fn event(&self) -> Option<Event<'m>> {
        self.event
    }

    /// Writes the next automatic reply; gives its Group and length. `None` after the last. An
    /// `out` of [`CiDevice::MAX_REPLY_LEN`] bytes is large enough for every automatic reply.
    pub fn write_next(&mut self, out: &mut [u8]) -> Option<Result<(u8, usize), WriteError>> {
        if let Some(r) = self.queue.iter_mut().find_map(Option::take) {
            return Some(self.write_reply(r, out).map(|n| (self.group, n)));
        }
        #[cfg(feature = "profiles")]
        {
            let replies = self.profiles.as_mut()?;
            let first = self.first_group;
            replies
                .write_next(out)
                .map(|r| r.map(|(g, n)| (first.saturating_add(g) & 0x0F, n)))
        }
        #[cfg(not(feature = "profiles"))]
        None
    }

    fn write_reply(&self, r: Reply, out: &mut [u8]) -> Result<usize, WriteError> {
        let (src, f) = (self.source, self.framing);
        match r {
            Reply::Discovery { destination, reply } => reply.write(src, destination, f, out),
            Reply::EndpointInfo {
                destination,
                status,
            } => EndpointInfoReply {
                status,
                data: self.instance_id,
            }
            .write(src, destination, f, out),
            Reply::Invalidate { target } => InvalidateMuid { target }.write(src, f, out),
            Reply::Nak {
                destination,
                device_id,
                original,
                status,
                details,
                max,
            } => Nak {
                original_sub_id2: original,
                status,
                details,
                text: &[],
            }
            .write_without_text(device_id, src, destination, max, f, out),
            #[cfg(feature = "property-exchange")]
            Reply::PeCapabilities {
                destination,
                capabilities,
            } => capabilities.write_reply(src, destination, f, out),
            #[cfg(feature = "process-inquiry")]
            Reply::PiCapabilities {
                destination,
                capabilities,
            } => capabilities.write(src, destination, f, out),
            #[cfg(feature = "profiles")]
            Reply::ProfileDetails { destination, entry } => {
                ProfileTable::<PROFILES>::details_reply(&entry, src, destination, f, out)
            }
        }
    }
}

/// The MIDI-CI Device of one Function Block. `PEERS`: peer table capacity; `PROFILES`:
/// local Profile capacity; `REQUESTS`: PE Request capacity, reported as the Number of
/// Simultaneous Property Exchange Requests Supported (1-127).
///
/// A `static` of `Option<CiDevice> = None` is in `.data` (its `None` is not all zero bits), so
/// its initial image also uses flash; a `core::mem::MaybeUninit` static (for example a
/// `static_cell::StaticCell`) is in `.bss` (OPT-013).
#[derive(Debug, Clone)]
pub struct CiDevice<'a, const PEERS: usize, const PROFILES: usize, const REQUESTS: usize> {
    config: DeviceConfig<'a>,
    muid: Muid,
    /// The MUID was used in a Transaction (DEC-011 D-6).
    used: bool,
    profiles: ProfileTable<PROFILES>,
    /// Only with the feature `property-exchange` (DEC-017 addendum of 2026-10-22).
    #[cfg(feature = "property-exchange")]
    requests: RequestTable<REQUESTS>,
    peers: [Option<Peer>; PEERS],
    /// Start of the open Discovery window. The window and the echo test are Initiator state
    /// (feature `initiator`, DEC-CI-001).
    #[cfg(feature = "initiator")]
    window_start: Option<u64>,
    #[cfg(feature = "initiator")]
    window: u32,
    /// Output Path IDs of the Discovery messages sent with this MUID, one bit each, for the
    /// echo test (DEC-011 D-5; one Discovery for each MIDI Out connection, REQ-CI-DISC-015).
    #[cfg(feature = "initiator")]
    sent_paths: u128,
    /// Output Path IDs sent in the open Discovery window.
    #[cfg(feature = "initiator")]
    window_paths: u128,
    /// Categories of the last Discovery sent, for the echo test.
    #[cfg(feature = "initiator")]
    sent_categories: Categories,
    /// Peer of the open MIDI Message Report Transaction as Initiator.
    #[cfg(feature = "process-inquiry")]
    report_peer: Option<Muid>,
    /// Open Profile Details Inquiries: peer, Profile, target (REQ-CI-PRFR-016).
    #[cfg(feature = "profiles")]
    details_pending: [Option<(Muid, ProfileId, u8)>; DETAILS_PENDING],
}

fn new_muid(old: Option<Muid>, random: &mut impl FnMut() -> u32) -> Muid {
    for _ in 0..64 {
        if let Some(m) = Muid::from_random(random(), old) {
            return m;
        }
    }
    // A random source that keeps failing: step from the old value (never reserved).
    let base = old.map_or(0, Muid::value);
    Muid::from_random(base.wrapping_add(1) % 0x0FFF_FF00, old).unwrap_or(Muid::BROADCAST)
}

impl<'a, const PEERS: usize, const PROFILES: usize, const REQUESTS: usize>
    CiDevice<'a, PEERS, PROFILES, REQUESTS>
{
    /// The longest automatic reply that [`Received::write_next`] writes, in bytes, with System
    /// Exclusive framing (REQ-CI-LEN-004): a reply buffer of this size never gives
    /// [`WriteError::Overflow`]. It is the larger of the Reply to Endpoint Information with a
    /// 42-byte Product Instance ID (Table 11: 2 + 13 + 3 + 42 = 60) and a Reply to Profile Inquiry
    /// with all `PROFILES` Profiles on one address (Table 18: 2 + 13 + 4 + 5 each). A Property
    /// Exchange reply is not automatic: its Chunks can use the peer's Receivable Maximum SysEx
    /// Message Size.
    pub const MAX_REPLY_LEN: usize = {
        let profiles = 19 + 5 * PROFILES;
        if profiles > 60 {
            profiles
        } else {
            60
        }
    };

    /// Makes a Device with a MUID from the caller's random numbers (REQ-CI-MUID-004, -005).
    pub fn new(
        config: DeviceConfig<'a>,
        random: &mut impl FnMut() -> u32,
    ) -> Result<Self, DeviceError> {
        let c = &config.categories;
        let min = if c.profile_configuration || c.property_exchange {
            MIN_SYSEX_PROFILE_PE
        } else {
            MIN_SYSEX
        };
        let pid_ok = config
            .product_instance_id
            .is_none_or(|p| crate::management::product_instance_id_check(p.as_bytes()));
        if config.max_sysex_size < min
            || config.max_sysex_size > 0x0FFF_FFFF
            || config.first_group > 15
            || config.group_count == 0
            || config.first_group as u16 + config.group_count as u16 > 16
            || config.function_block.is_some_and(|f| f > 0x7E)
            || !pid_ok
            || REQUESTS == 0
            || REQUESTS > 127
        {
            return Err(DeviceError::InvalidConfig);
        }
        Ok(CiDevice {
            config,
            muid: new_muid(None, random),
            used: false,
            profiles: ProfileTable::new(),
            #[cfg(feature = "property-exchange")]
            requests: RequestTable::new(),
            peers: [None; PEERS],
            #[cfg(feature = "initiator")]
            window_start: None,
            #[cfg(feature = "initiator")]
            window: 0,
            #[cfg(feature = "initiator")]
            sent_paths: 0,
            #[cfg(feature = "initiator")]
            window_paths: 0,
            #[cfg(feature = "initiator")]
            sent_categories: Categories {
                protocol_negotiation: false,
                profile_configuration: false,
                property_exchange: false,
                process_inquiry: false,
            },
            #[cfg(feature = "process-inquiry")]
            report_peer: None,
            #[cfg(feature = "profiles")]
            details_pending: [None; DETAILS_PENDING],
        })
    }

    /// The current MUID (REQ-CI-MUID-008).
    pub fn muid(&self) -> Muid {
        self.muid
    }

    /// The configuration.
    pub fn config(&self) -> &DeviceConfig<'a> {
        &self.config
    }

    /// The local Profiles.
    pub fn profiles(&self) -> &ProfileTable<PROFILES> {
        &self.profiles
    }

    /// The discovered peers.
    pub fn peers(&self) -> impl Iterator<Item = &Peer> {
        self.peers.iter().flatten()
    }

    /// A discovered peer.
    pub fn peer(&self, muid: Muid) -> Option<&Peer> {
        self.peers().find(|p| p.muid == muid)
    }

    fn peer_mut(&mut self, muid: Muid) -> Option<&mut Peer> {
        self.peers.iter_mut().flatten().find(|p| p.muid == muid)
    }

    #[inline(never)]
    fn peer_max(&self, muid: Muid) -> Option<u32> {
        self.peer(muid).map(|p| p.max_sysex_size).filter(|m| *m > 0)
    }

    fn store_peer(
        &mut self,
        muid: Muid,
        version: Version,
        categories: Categories,
        max: u32,
    ) -> Option<&mut Peer> {
        if let Some(i) = self
            .peers
            .iter()
            .position(|p| matches!(p, Some(p) if p.muid == muid))
        {
            let p = self.peers[i].as_mut()?;
            p.version = version;
            p.categories = categories;
            p.max_sysex_size = max;
            return Some(p);
        }
        let slot = self.peers.iter_mut().find(|p| p.is_none())?;
        *slot = Some(Peer {
            muid,
            max_sysex_size: max,
            version,
            categories,
            profiles_inquired: false,
            pe_capabilities: None,
            #[cfg(feature = "initiator")]
            window: 0,
            #[cfg(feature = "initiator")]
            path: None,
        });
        slot.as_mut()
    }

    fn remove_peer(&mut self, muid: Muid) {
        for p in self.peers.iter_mut() {
            if matches!(p, Some(x) if x.muid == muid) {
                *p = None;
            }
        }
        #[cfg(feature = "property-exchange")]
        self.requests.close_peer(muid);
        #[cfg(feature = "process-inquiry")]
        if self.report_peer == Some(muid) {
            self.report_peer = None;
        }
        #[cfg(feature = "profiles")]
        for d in self.details_pending.iter_mut() {
            if matches!(d, Some((m, _, _)) if *m == muid) {
                *d = None;
            }
        }
    }

    /// True when this build has Profiles and the configuration declares Profile Configuration
    /// (REQ-CI-TIER-004): the local Profile calls need both.
    fn profiles_on(&self) -> bool {
        compiled(self.config.categories).profile_configuration
    }

    /// Categories as written in Discovery: no Profile bit without Profiles (REQ-CI-PROF-018), and
    /// only the categories this build has (DEC-017).
    fn categories(&self) -> Categories {
        let mut c = compiled(self.config.categories);
        c.profile_configuration &= self.profiles.entries().next().is_some();
        c
    }

    #[cfg(feature = "initiator")]
    fn discovery_message(&self, output_path_id: u8) -> Discovery {
        Discovery {
            identity: self.config.identity,
            categories: self.categories(),
            max_sysex_size: self.config.max_sysex_size,
            output_path_id: Some(output_path_id),
        }
    }

    fn change_muid(&mut self, random: &mut impl FnMut() -> u32) -> Muid {
        let old = self.muid;
        self.muid = new_muid(Some(old), random);
        self.used = false;
        #[cfg(feature = "property-exchange")]
        {
            self.requests = RequestTable::new();
        }
        #[cfg(feature = "process-inquiry")]
        {
            self.report_peer = None;
        }
        #[cfg(feature = "profiles")]
        {
            self.details_pending = [None; DETAILS_PENDING];
        }
        #[cfg(feature = "initiator")]
        {
            self.window_start = None;
            self.sent_paths = 0;
        }
        // The peers discard their state for the old MUID (REQ-CI-INV-007): inquire again.
        for p in self.peers.iter_mut().flatten() {
            p.profiles_inquired = false;
            p.pe_capabilities = None;
        }
        old
    }

    /// Writes Discovery with `output_path_id` (0 for one connection; one ID per MIDI Out
    /// connection, kept by the caller) and opens the Discovery window (REQ-CI-DISC-001,
    /// -004, -011, -015 to -017).
    pub fn discovery(
        &mut self,
        output_path_id: u8,
        now_us: u64,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-CI-001: an Initiator call in a build with no Initiator role.
        #[cfg(not(feature = "initiator"))]
        {
            let _ = (output_path_id, now_us, out);
            Err(DeviceError::NotSupported)
        }
        #[cfg(feature = "initiator")]
        {
            let message = self.discovery_message(output_path_id);
            let n = message.write(self.muid, self.config.framing, out)?;
            self.expire_window(now_us);
            let bit = 1u128 << (output_path_id & 0x7F);
            // One window for the Discovery messages of all connections (REQ-CI-DISC-015); a new
            // Discovery on a path of the open window starts a new round.
            if self.window_start.is_none() || self.window_paths & bit != 0 {
                self.window = self.window.wrapping_add(1);
                self.window_paths = 0;
            }
            self.window_start = Some(now_us);
            self.window_paths |= bit;
            self.sent_paths |= bit;
            self.sent_categories = message.categories;
            Ok(n)
        }
    }

    /// Closes the Discovery window after 3 s (REQ-CI-DISC-011).
    fn expire_window(&mut self, now_us: u64) {
        #[cfg(not(feature = "initiator"))]
        let _ = now_us;
        #[cfg(feature = "initiator")]
        if self
            .window_start
            .is_some_and(|s| now_us.saturating_sub(s) >= DISCOVERY_WINDOW_US)
        {
            self.window_start = None;
            self.window_paths = 0;
        }
    }

    /// Ends the Discovery window after 3 s (REQ-CI-DISC-011).
    /// With no feature `initiator` there is no window: always `None` (DEC-CI-001).
    pub fn poll(&mut self, now_us: u64) -> Option<Event<'static>> {
        #[cfg(not(feature = "initiator"))]
        {
            let _ = now_us;
            None
        }
        #[cfg(feature = "initiator")]
        {
            let start = self.window_start?;
            if now_us.saturating_sub(start) >= DISCOVERY_WINDOW_US {
                self.window_start = None;
                return Some(Event::DiscoveryEnded);
            }
            None
        }
    }

    /// Processes one received MIDI-CI message on `group` (with or without F0/F7).
    pub fn receive<'d, 'm>(
        &'d mut self,
        group: u8,
        msg: &'m [u8],
        now_us: u64,
        random: &mut impl FnMut() -> u32,
    ) -> Received<'d, 'm, PROFILES> {
        self.expire_window(now_us);
        let (event, queue, inquiry) = match Header::parse(msg) {
            Err(error) => {
                let (event, queue) = self.rejected_header(msg, error);
                (event, queue, None)
            }
            Ok((h, body)) => self.dispatch(group, &h, body, random),
        };
        let offset = group.wrapping_sub(self.config.first_group) & 0x0F;
        #[cfg(feature = "profiles")]
        let profiles = inquiry.map(|(device_id, destination)| {
            self.profiles.inquiry_replies(
                device_id,
                offset,
                self.muid,
                destination,
                self.config.framing,
            )
        });
        #[cfg(not(feature = "profiles"))]
        let profiles = {
            let _ = (inquiry, offset);
            None
        };
        Received {
            event,
            queue,
            profiles,
            source: self.muid,
            group,
            first_group: self.config.first_group,
            framing: self.config.framing,
            instance_id: self.config.product_instance_id.unwrap_or("").as_bytes(),
        }
    }

    /// A message whose header the reader rejected. A message to another MUID is ignored
    /// (REQ-CI-ADDR-003); a reserved Device ID gets NAK 0x03 (DEC-011 D-10).
    #[allow(clippy::type_complexity)]
    fn rejected_header<'m>(
        &self,
        msg: &[u8],
        error: ParseError,
    ) -> (Option<Event<'m>>, [Option<Reply>; 2]) {
        let m = match msg.first() {
            Some(0xF0) => &msg[1..],
            _ => msg,
        };
        let muid = |at: usize| {
            m.get(at..at + 4)
                .and_then(|b| Muid::from_bytes([b[0], b[1], b[2], b[3]]).ok())
        };
        let rejected = (Some(Event::Rejected { error }), [None, None]);
        let (Some(source), Some(destination)) = (muid(5), muid(9)) else {
            return rejected;
        };
        if destination != self.muid && !destination.is_broadcast() || source == self.muid {
            return (None, [None, None]);
        }
        let sub = m[3];
        let no_nak = matches!(
            SubId2::from_byte(sub),
            Some(SubId2::Ack | SubId2::Nak | SubId2::InvalidateMuid)
        );
        if error != ParseError::ReservedDeviceId || no_nak {
            return rejected;
        }
        let nak = Reply::Nak {
            destination: source,
            device_id: DeviceId::FunctionBlock,
            original: sub,
            status: NakStatus::NotInUse,
            details: Details::Reserved,
            max: self.peer_max(source),
        };
        (Some(Event::Rejected { error }), [Some(nak), None])
    }

    #[allow(clippy::type_complexity)]
    fn dispatch<'m>(
        &mut self,
        group: u8,
        h: &Header,
        body: &'m [u8],
        random: &mut impl FnMut() -> u32,
    ) -> (
        Option<Event<'m>>,
        [Option<Reply>; 2],
        Option<(DeviceId, Muid)>,
    ) {
        let none = [None, None];
        // REQ-CI-ADDR-003: not for this Device.
        if h.destination != self.muid && !h.destination.is_broadcast() {
            return (None, none, None);
        }
        let src = h.source;
        let nak = |me: &Self, status: NakStatus, details: Details| -> [Option<Reply>; 2] {
            [
                Some(Reply::Nak {
                    destination: src,
                    device_id: h.device_id,
                    original: h.sub_id2,
                    status,
                    details,
                    max: me.peer_max(src),
                }),
                None,
            ]
        };
        // Never NAK an ACK, a NAK or an Invalidate MUID (REQ-CI-INV-004): no NAK loop.
        let is_ack_nak = matches!(
            SubId2::from_byte(h.sub_id2),
            Some(SubId2::Ack | SubId2::Nak | SubId2::InvalidateMuid)
        );
        // Messages from this MUID: echo, or a collision for Discovery (REQ-CI-MUID-012). An
        // Invalidate MUID of this MUID comes from the other Device of a collision (5.9.1
        // Option B: it sends with the duplicated MUID); the own echo has the old MUID.
        if src == self.muid && h.sub_id2 != SubId2::InvalidateMuid.to_byte() {
            if h.sub_id2 == SubId2::Discovery.to_byte() {
                return self.own_discovery(h, body, random);
            }
            return (None, none, None);
        }
        // DEC-CI-001: a Device with no Initiator role ignores a reply for an Initiator: no event
        // and no NAK.
        #[cfg(not(feature = "initiator"))]
        if matches!(
            SubId2::from_byte(h.sub_id2),
            Some(
                SubId2::DiscoveryReply
                    | SubId2::EndpointInfoReply
                    | SubId2::ProfileInquiryReply
                    | SubId2::PeCapabilitiesReply
                    | SubId2::ProcessInquiryCapabilitiesReply
                    | SubId2::MidiMessageReportReply
                    | SubId2::MidiMessageReportEnd
            )
        ) {
            return (None, none, None);
        }
        // DEC-CI-001 addendum of 2026-10-26: the same for the Profile messages that only an
        // Initiator uses (REQ-CI-PRFR-010 to -013, -016), when the build has Profiles.
        #[cfg(all(feature = "profiles", not(feature = "initiator")))]
        if matches!(
            SubId2::from_byte(h.sub_id2),
            Some(
                SubId2::ProfileDetailsReply
                    | SubId2::ProfileEnabled
                    | SubId2::ProfileDisabled
                    | SubId2::ProfileAdded
                    | SubId2::ProfileRemoved
            )
        ) {
            return (None, none, None);
        }
        // REQ-CI-MSG-015: reserved version bits.
        if h.version.has_reserved_bits() {
            let reply = if is_ack_nak {
                none
            } else {
                nak(self, NakStatus::VersionNotSupported, Details::Reserved)
            };
            return (None, reply, None);
        }
        let Some(sub) = SubId2::from_byte(h.sub_id2) else {
            return (
                None,
                nak(self, NakStatus::NotSupported, Details::Reserved),
                None,
            );
        };
        let c = compiled(self.config.categories);
        let supported = match sub.category() {
            Category::ProfileConfiguration => c.profile_configuration,
            Category::PropertyExchange => c.property_exchange,
            Category::ProcessInquiry => c.process_inquiry,
            _ => true,
        };
        if !supported {
            return (
                None,
                nak(self, NakStatus::NotSupported, Details::Reserved),
                None,
            );
        }
        // Appendix H, DEC-011 D-10: Device ID of each message (REQ-CI-ROUTE-001, REQ-CI-PEF-002).
        let fb = h.device_id == DeviceId::FunctionBlock;
        let id_ok = match sub {
            SubId2::Discovery
            | SubId2::DiscoveryReply
            | SubId2::EndpointInfo
            | SubId2::EndpointInfoReply
            | SubId2::InvalidateMuid
            | SubId2::ProcessInquiryCapabilities
            | SubId2::ProcessInquiryCapabilitiesReply => fb,
            s if s.category() == Category::PropertyExchange => {
                fb || h.device_id == DeviceId::Channel(0x0F)
            }
            _ => true,
        };
        if !id_ok {
            if is_ack_nak {
                return (None, none, None);
            }
            return (
                None,
                nak(self, NakStatus::NotInUse, Details::Reserved),
                None,
            );
        }
        match self.message(group, sub, h, body, random) {
            Ok(r) => {
                // DEC-011 D-6: every Transaction, Discovery included (4.1), uses the MUID.
                if sub != SubId2::InvalidateMuid {
                    self.used = true;
                }
                r
            }
            Err(error) => {
                let reply = if is_ack_nak {
                    none
                } else {
                    nak(self, NakStatus::Malformed, Details::Reserved)
                };
                (Some(Event::Rejected { error }), reply, None)
            }
        }
    }

    /// A Discovery with this MUID as Source: echo or collision (DEC-011 D-5, D-6).
    #[allow(clippy::type_complexity)]
    fn own_discovery<'m>(
        &mut self,
        h: &Header,
        body: &[u8],
        random: &mut impl FnMut() -> u32,
    ) -> (
        Option<Event<'m>>,
        [Option<Reply>; 2],
        Option<(DeviceId, Muid)>,
    ) {
        let received = Discovery::read(h, body).ok();
        // With no feature `initiator` this Device never sends Discovery, so a Discovery with its
        // MUID is never an echo.
        #[cfg(not(feature = "initiator"))]
        let echo = false;
        #[cfg(feature = "initiator")]
        let echo = match received.and_then(|d| d.output_path_id.map(|p| (d, p))) {
            Some((d, path)) => {
                self.sent_paths & (1 << path) != 0
                    && d == Discovery {
                        categories: self.sent_categories,
                        ..self.discovery_message(path)
                    }
            }
            None => false,
        };
        if echo {
            return (None, [None, None], None);
        }
        let option_a = !self.used;
        let old = self.change_muid(random);
        let new = self.muid;
        let reply = if option_a {
            // Option A: new MUID, then Reply to Discovery (REQ-CI-MUID-013).
            let path = received.and_then(|d| d.output_path_id).unwrap_or(0);
            Reply::Discovery {
                destination: old,
                reply: self.discovery_reply(path),
            }
        } else {
            // Option B: Invalidate the duplicated MUID (REQ-CI-MUID-014).
            Reply::Invalidate { target: old }
        };
        (
            Some(Event::MuidChanged { old, new }),
            [Some(reply), None],
            None,
        )
    }

    fn discovery_reply(&self, output_path_id: u8) -> DiscoveryReply {
        DiscoveryReply {
            identity: self.config.identity,
            categories: self.categories(),
            max_sysex_size: self.config.max_sysex_size,
            output_path_id: Some(output_path_id),
            function_block: self.config.function_block,
        }
    }

    #[cfg(feature = "profiles")]
    fn address(&self, h: &Header, group: u8) -> ProfileAddress {
        let group = match h.device_id {
            DeviceId::FunctionBlock => 0,
            _ => group.wrapping_sub(self.config.first_group) & 0x0F,
        };
        ProfileAddress {
            device_id: h.device_id,
            group,
        }
    }

    #[allow(clippy::type_complexity)]
    // `group` is used by the Profile and Process Inquiry arms only (DEC-017).
    #[cfg_attr(not(feature = "full"), allow(unused_variables))]
    fn message<'m>(
        &mut self,
        group: u8,
        sub: SubId2,
        h: &Header,
        body: &'m [u8],
        random: &mut impl FnMut() -> u32,
    ) -> Result<
        (
            Option<Event<'m>>,
            [Option<Reply>; 2],
            Option<(DeviceId, Muid)>,
        ),
        ParseError,
    > {
        let src = h.source;
        let none = [None, None];
        let one = |r: Reply| [Some(r), None];
        let nak = |me: &Self, status: NakStatus, details: Details| {
            one(Reply::Nak {
                destination: src,
                device_id: h.device_id,
                original: h.sub_id2,
                status,
                details,
                max: me.peer_max(src),
            })
        };
        let ev = |e: Event<'m>| Some(e);
        Ok(match sub {
            SubId2::Discovery => {
                let d = Discovery::read(h, body)?;
                self.store_peer(src, h.version, d.categories, d.max_sysex_size);
                // REQ-CI-DISC-002, -013: the received Output Path ID, 0 for version 1.
                let reply = self.discovery_reply(d.output_path_id.unwrap_or(0));
                (
                    None,
                    one(Reply::Discovery {
                        destination: src,
                        reply,
                    }),
                    None,
                )
            }
            #[cfg(feature = "initiator")]
            SubId2::DiscoveryReply => {
                let r = DiscoveryReply::read(h, body)?;
                let (open, window) = (self.window_start.is_some(), self.window);
                // REQ-CI-MUID-015: two replies with one MUID to one Discovery (same window and
                // Output Path ID). A peer that is not in a full peer table is not checked.
                let path = r.output_path_id;
                if open && matches!(self.peer(src), Some(p) if p.window == window && p.path == path)
                {
                    self.remove_peer(src);
                    return Ok((
                        ev(Event::PeerInvalidated { muid: src }),
                        one(Reply::Invalidate { target: src }),
                        None,
                    ));
                }
                if let Some(p) = self.store_peer(src, h.version, r.categories, r.max_sysex_size) {
                    if open {
                        p.window = window;
                        p.path = path;
                    }
                }
                (
                    ev(Event::Discovered {
                        muid: src,
                        reply: r,
                    }),
                    none,
                    None,
                )
            }
            SubId2::EndpointInfo => {
                let q = EndpointInfoInquiry::read(h, body)?;
                // REQ-CI-EPI-003, -006, -007; NAK 0x00 when there is no information (EPI-004).
                match (q.status, self.config.product_instance_id) {
                    (EndpointInfoStatus::ProductInstanceId, Some(_)) => (
                        None,
                        one(Reply::EndpointInfo {
                            destination: src,
                            status: q.status,
                        }),
                        None,
                    ),
                    _ => (None, nak(self, NakStatus::Nak, Details::Reserved), None),
                }
            }
            #[cfg(feature = "initiator")]
            SubId2::EndpointInfoReply => (
                ev(Event::EndpointInfo {
                    muid: src,
                    reply: EndpointInfoReply::read(h, body)?,
                }),
                none,
                None,
            ),
            SubId2::InvalidateMuid => {
                let target = InvalidateMuid::read(h, body)?.target;
                // REQ-CI-INV-004: no reply. REQ-CI-INV-005 to -007.
                if target == self.muid {
                    let old = self.change_muid(random);
                    return Ok((
                        ev(Event::MuidChanged {
                            old,
                            new: self.muid,
                        }),
                        none,
                        None,
                    ));
                }
                self.remove_peer(target);
                (ev(Event::PeerInvalidated { muid: target }), none, None)
            }
            SubId2::Ack => (
                ev(Event::Ack {
                    muid: src,
                    device_id: h.device_id,
                    ack: Ack::read(h, body)?,
                }),
                none,
                None,
            ),
            SubId2::Nak => {
                let nak = Nak::read(h, body)?;
                #[cfg(feature = "process-inquiry")]
                if self.report_peer == Some(src) {
                    self.report_peer = None; // REQ-CI-MMR-037
                }
                (
                    ev(Event::Nak {
                        muid: src,
                        device_id: h.device_id,
                        nak,
                    }),
                    none,
                    None,
                )
            }
            #[cfg(feature = "profiles")]
            SubId2::ProfileInquiry => {
                ProfileInquiry::read(h, body)?;
                (None, none, Some((h.device_id, src)))
            }
            #[cfg(all(feature = "profiles", feature = "initiator"))]
            SubId2::ProfileInquiryReply => {
                let reply = ProfileInquiryReply::read(h, body)?;
                (
                    ev(Event::ProfileInquiryReply {
                        muid: src,
                        device_id: h.device_id,
                        group,
                        reply,
                    }),
                    none,
                    None,
                )
            }
            #[cfg(feature = "profiles")]
            SubId2::SetProfileOn | SubId2::SetProfileOff => {
                let (profile, channels, on) = if sub == SubId2::SetProfileOn {
                    let m = SetProfileOn::read(h, body)?;
                    (m.profile, m.channels, true)
                } else {
                    (SetProfileOff::read(h, body)?.profile, 0, false)
                };
                let address = self.address(h, group);
                if self.profiles.find(address, profile).is_none() {
                    // REQ-CI-PRFR-025, -027; DEC-011 D-7.
                    return Ok((
                        None,
                        nak(
                            self,
                            NakStatus::ProfileNotSupported,
                            Details::Profile(profile.to_bytes()),
                        ),
                        None,
                    ));
                }
                (
                    ev(Event::SetProfile {
                        muid: src,
                        address,
                        profile,
                        on,
                        channels,
                    }),
                    none,
                    None,
                )
            }
            #[cfg(all(feature = "profiles", feature = "initiator"))]
            SubId2::ProfileEnabled
            | SubId2::ProfileDisabled
            | SubId2::ProfileAdded
            | SubId2::ProfileRemoved => {
                let (kind, profile, channels) = match sub {
                    SubId2::ProfileEnabled => {
                        let m = ProfileEnabledReport::read(h, body)?;
                        (ReportKind::Enabled, m.profile, m.channels)
                    }
                    SubId2::ProfileDisabled => {
                        let m = ProfileDisabledReport::read(h, body)?;
                        (ReportKind::Disabled, m.profile, m.channels)
                    }
                    SubId2::ProfileAdded => (
                        ReportKind::Added,
                        ProfileAddedReport::read(h, body)?.profile,
                        0,
                    ),
                    _ => (
                        ReportKind::Removed,
                        ProfileRemovedReport::read(h, body)?.profile,
                        0,
                    ),
                };
                (
                    ev(Event::ProfileReport {
                        muid: src,
                        device_id: h.device_id,
                        group,
                        kind,
                        profile,
                        channels,
                    }),
                    none,
                    None,
                )
            }
            #[cfg(feature = "profiles")]
            SubId2::ProfileDetailsInquiry => {
                let q = ProfileDetailsInquiry::read(h, body)?;
                let address = self.address(h, group);
                let Some(entry) = self.profiles.find(address, q.profile).copied() else {
                    // REQ-CI-PRFR-015.
                    return Ok((
                        None,
                        nak(
                            self,
                            NakStatus::ProfileNotSupported,
                            Details::Profile(q.profile.to_bytes()),
                        ),
                        None,
                    ));
                };
                if q.target == 0 {
                    (
                        None,
                        one(Reply::ProfileDetails {
                            destination: src,
                            entry,
                        }),
                        None,
                    )
                } else {
                    (
                        ev(Event::ProfileDetailsInquiry {
                            muid: src,
                            address,
                            profile: q.profile,
                            target: q.target,
                        }),
                        none,
                        None,
                    )
                }
            }
            #[cfg(all(feature = "profiles", feature = "initiator"))]
            SubId2::ProfileDetailsReply => {
                let reply = ProfileDetailsReply::read(h, body)?;
                let open = self.details_pending.iter_mut().find(|d| {
                    matches!(d, Some((m, p, t)) if *m == src && p.same_profile(reply.profile) && *t == reply.target)
                });
                let requested = open.map(|d| d.take()).is_some();
                let e = Event::ProfileDetailsReply {
                    muid: src,
                    device_id: h.device_id,
                    group,
                    reply,
                    notification: !requested,
                };
                (ev(e), none, None)
            }
            #[cfg(feature = "profiles")]
            SubId2::ProfileSpecificData => {
                let data = ProfileSpecificData::read(h, body)?;
                (
                    ev(Event::ProfileSpecificData {
                        muid: src,
                        device_id: h.device_id,
                        group,
                        data,
                    }),
                    none,
                    None,
                )
            }
            #[cfg(feature = "property-exchange")]
            SubId2::PeCapabilities => {
                PeCapabilities::read(h, body)?;
                let capabilities = PeCapabilities {
                    simultaneous: REQUESTS as u8,
                    major: None,
                    minor: None,
                };
                (
                    None,
                    one(Reply::PeCapabilities {
                        destination: src,
                        capabilities,
                    }),
                    None,
                )
            }
            #[cfg(all(feature = "property-exchange", feature = "initiator"))]
            SubId2::PeCapabilitiesReply => {
                let capabilities = PeCapabilities::read(h, body)?;
                if let Some(p) = self.peer_mut(src) {
                    p.pe_capabilities = Some(capabilities);
                }
                (
                    ev(Event::PeCapabilities {
                        muid: src,
                        capabilities,
                    }),
                    none,
                    None,
                )
            }
            #[cfg(feature = "property-exchange")]
            SubId2::GetPropertyData
            | SubId2::GetPropertyDataReply
            | SubId2::SetPropertyData
            | SubId2::SetPropertyDataReply
            | SubId2::Subscription
            | SubId2::SubscriptionReply
            | SubId2::Notify => {
                let chunk = PeChunk::read(h, body)?;
                let kind = PeKind::from_sub_id2(h.sub_id2).ok_or(ParseError::UnsupportedSubId)?;
                if matches!(kind, PeKind::GetInquiry | PeKind::SetInquiry)
                    && self.requests.open(src, chunk.request_id).is_err()
                {
                    let details = Details::PropertyExchange {
                        request_id: chunk.request_id,
                        chunk: chunk.number,
                    };
                    return Ok((None, nak(self, NakStatus::Error, details), None));
                }
                (
                    ev(Event::PeChunk {
                        muid: src,
                        kind,
                        chunk,
                    }),
                    none,
                    None,
                )
            }
            #[cfg(feature = "process-inquiry")]
            SubId2::ProcessInquiryCapabilities => {
                PiCapabilitiesInquiry::read(h, body)?;
                let capabilities = self.config.process_inquiry;
                (
                    None,
                    one(Reply::PiCapabilities {
                        destination: src,
                        capabilities,
                    }),
                    None,
                )
            }
            #[cfg(all(feature = "process-inquiry", feature = "initiator"))]
            SubId2::ProcessInquiryCapabilitiesReply => (
                ev(Event::PiCapabilities {
                    muid: src,
                    capabilities: PiCapabilities::read(h, body)?,
                }),
                none,
                None,
            ),
            #[cfg(feature = "process-inquiry")]
            SubId2::MidiMessageReport => {
                let inquiry = MessageReportInquiry::read(h, body)?;
                // REQ-CI-MMR-007, -010.
                if let DeviceId::Channel(c) = h.device_id {
                    if self.config.channels_in_use & (1 << c) == 0 {
                        return Ok((
                            None,
                            nak(self, NakStatus::NotInUse, Details::Reserved),
                            None,
                        ));
                    }
                }
                (
                    ev(Event::MessageReport {
                        muid: src,
                        device_id: h.device_id,
                        group,
                        inquiry,
                    }),
                    none,
                    None,
                )
            }
            #[cfg(all(feature = "process-inquiry", feature = "initiator"))]
            SubId2::MidiMessageReportReply => (
                ev(Event::MessageReportReply {
                    muid: src,
                    reply: MessageReportReply::read(h, body)?,
                }),
                none,
                None,
            ),
            #[cfg(all(feature = "process-inquiry", feature = "initiator"))]
            SubId2::MidiMessageReportEnd => {
                MessageReportEnd::read(h, body)?;
                if self.report_peer == Some(src) {
                    self.report_peer = None;
                }
                (ev(Event::MessageReportEnd { muid: src }), none, None)
            }
            // DEC-017: a category whose feature is off (the check in `dispatch` NAKs it first).
            #[allow(unreachable_patterns)]
            _ => (
                None,
                nak(self, NakStatus::NotSupported, Details::Reserved),
                None,
            ),
        })
    }

    fn known(&self, peer: Muid) -> Result<&Peer, DeviceError> {
        self.peer(peer).ok_or(DeviceError::UnknownPeer)
    }

    fn version_2(&self, peer: Muid) -> Result<(), DeviceError> {
        if self.known(peer)?.version.minor() < 2 {
            return Err(DeviceError::PeerVersion);
        }
        Ok(())
    }

    /// Writes Inquiry: Endpoint Information (REQ-CI-EPI-001); version 2 peers only.
    pub fn endpoint_info_inquiry(
        &mut self,
        peer: Muid,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-CI-001: an Initiator call in a build with no Initiator role.
        if !cfg!(feature = "initiator") {
            return Err(DeviceError::NotSupported);
        }
        self.version_2(peer)?;
        self.used = true;
        let q = EndpointInfoInquiry {
            status: EndpointInfoStatus::ProductInstanceId,
        };
        Ok(q.write(self.muid, peer, self.config.framing, out)?)
    }

    /// Invalidates this MUID: writes Invalidate MUID and takes a new MUID (REQ-CI-INV-001).
    pub fn invalidate(
        &mut self,
        random: &mut impl FnMut() -> u32,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        let n = InvalidateMuid { target: self.muid }.write(self.muid, self.config.framing, out)?;
        self.change_muid(random);
        Ok(n)
    }

    /// The Function Block moved or changed its number of Groups (REQ-CI-INV-002, -003,
    /// DEC-009): writes Invalidate MUID and takes a new MUID. Then call `discovery`.
    pub fn function_block_changed(
        &mut self,
        first_group: u8,
        group_count: u8,
        random: &mut impl FnMut() -> u32,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        if first_group > 15 || group_count == 0 || first_group as u16 + group_count as u16 > 16 {
            return Err(DeviceError::InvalidConfig);
        }
        let n = self.invalidate(random, out)?;
        self.config.first_group = first_group;
        self.config.group_count = group_count;
        Ok(n)
    }

    /// Writes Profile Inquiry and marks the peer as inquired (REQ-CI-PROF-015).
    pub fn profile_inquiry(
        &mut self,
        peer: Muid,
        device_id: DeviceId,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-017 addendum of 2026-10-22, DEC-CI-001: an Initiator call of a category or role that
        // this build does not have.
        if !cfg!(all(feature = "profiles", feature = "initiator")) {
            return Err(DeviceError::NotSupported);
        }
        let framing = self.config.framing;
        let me = self.muid;
        let p = self.peer_mut(peer).ok_or(DeviceError::UnknownPeer)?;
        let n = ProfileInquiry::write(device_id, me, peer, framing, out)?;
        p.profiles_inquired = true;
        self.used = true;
        Ok(n)
    }

    fn inquired(&self, peer: Muid) -> Result<(), DeviceError> {
        if !self.known(peer)?.profiles_inquired {
            return Err(DeviceError::NotInquired);
        }
        Ok(())
    }

    /// Writes Set Profile On (REQ-CI-PROF-025, REQ-CI-GEN-003, REQ-CI-PRFR-002, -021).
    /// `max_channels` is the Maximum Number of Channels that the peer reported, if any.
    #[allow(clippy::too_many_arguments)]
    pub fn set_profile_on(
        &mut self,
        peer: Muid,
        device_id: DeviceId,
        profile: ProfileId,
        channels: u16,
        max_channels: Option<u16>,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-017 addendum of 2026-10-22, DEC-CI-001: an Initiator call of a category or role that
        // this build does not have.
        if !cfg!(all(feature = "profiles", feature = "initiator")) {
            return Err(DeviceError::NotSupported);
        }
        self.inquired(peer)?;
        if max_channels.is_some_and(|m| channels > m) {
            return Err(DeviceError::TooManyChannels);
        }
        let profile = match profile {
            ProfileId::Standard {
                bank,
                number,
                version,
                ..
            } => ProfileId::Standard {
                bank,
                number,
                version,
                level: ProfileLevel::Highest,
            },
            m => m,
        };
        self.used = true;
        Ok(SetProfileOn { profile, channels }.write(
            device_id,
            self.muid,
            peer,
            self.config.framing,
            out,
        )?)
    }

    /// Writes Set Profile Off (REQ-CI-PROF-026, REQ-CI-GEN-003).
    pub fn set_profile_off(
        &mut self,
        peer: Muid,
        device_id: DeviceId,
        profile: ProfileId,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-017 addendum of 2026-10-22, DEC-CI-001: an Initiator call of a category or role that
        // this build does not have.
        if !cfg!(all(feature = "profiles", feature = "initiator")) {
            return Err(DeviceError::NotSupported);
        }
        self.inquired(peer)?;
        self.used = true;
        Ok(
            SetProfileOff { profile }.write(
                device_id,
                self.muid,
                peer,
                self.config.framing,
                out,
            )?,
        )
    }

    /// Writes Profile Details Inquiry; version 2 peers only.
    pub fn profile_details_inquiry(
        &mut self,
        peer: Muid,
        device_id: DeviceId,
        profile: ProfileId,
        target: u8,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-017 addendum of 2026-10-22, DEC-CI-001: an Initiator call of a category or role that
        // this build does not have.
        #[cfg(not(all(feature = "profiles", feature = "initiator")))]
        {
            let _ = (peer, device_id, profile, target, out);
            Err(DeviceError::NotSupported)
        }
        #[cfg(all(feature = "profiles", feature = "initiator"))]
        {
            self.version_2(peer)?;
            let n = ProfileDetailsInquiry { profile, target }.write(
                device_id,
                self.muid,
                peer,
                self.config.framing,
                out,
            )?;
            let slot = match self.details_pending.iter().position(Option::is_none) {
                Some(i) => i,
                None => {
                    self.details_pending.rotate_left(1);
                    DETAILS_PENDING - 1
                }
            };
            self.details_pending[slot] = Some((peer, profile, target));
            self.used = true;
            Ok(n)
        }
    }

    /// Writes Inquiry: Property Exchange Capabilities (REQ-CI-PEF-031).
    pub fn pe_capabilities_inquiry(
        &mut self,
        peer: Muid,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-017 addendum of 2026-10-22, DEC-CI-001: an Initiator call of a category or role that
        // this build does not have.
        if !cfg!(all(feature = "property-exchange", feature = "initiator")) {
            return Err(DeviceError::NotSupported);
        }
        self.known(peer)?;
        self.used = true;
        let c = PeCapabilities {
            simultaneous: REQUESTS as u8,
            major: None,
            minor: None,
        };
        Ok(c.write_inquiry(self.muid, peer, self.config.framing, out)?)
    }

    /// Allocates a Request ID and gives a ChunkWriter for a Get, Set or Subscription message
    /// (REQ-CI-PEF-031, -034, -036, -038, -042).
    pub fn pe_request<'h>(
        &mut self,
        peer: Muid,
        kind: PeKind,
        header: &'h [u8],
        total_len: Option<usize>,
        chunk_size: Option<usize>,
    ) -> Result<(u8, ChunkWriter<'h>), DeviceError> {
        // DEC-017 addendum of 2026-10-22, DEC-CI-001: an Initiator call of a category or role that
        // this build does not have.
        #[cfg(not(all(feature = "property-exchange", feature = "initiator")))]
        {
            let _ = (peer, kind, header, total_len, chunk_size);
            Err(DeviceError::NotSupported)
        }
        #[cfg(all(feature = "property-exchange", feature = "initiator"))]
        {
            if !matches!(
                kind,
                PeKind::GetInquiry | PeKind::SetInquiry | PeKind::Subscription
            ) {
                return Err(DeviceError::NotSupported);
            }
            let p = self.known(peer)?;
            if p.pe_capabilities.is_none() {
                return Err(DeviceError::NotInquired);
            }
            let max = p.max_sysex_size;
            let id = self.requests.allocate(peer).ok_or(DeviceError::Busy)?;
            self.used = true;
            let w = ChunkWriter::new(
                kind,
                id,
                header,
                max,
                chunk_size,
                total_len,
                self.muid,
                peer,
                self.config.framing,
            );
            match w {
                Ok(w) => Ok((id, w)),
                Err(e) => {
                    self.requests.close(peer, id, Direction::Initiator);
                    Err(e.into())
                }
            }
        }
    }

    /// Responder: a ChunkWriter for a reply with the Request ID of the inquiry
    /// (REQ-CI-PEF-035, -037, -041). Notify is refused (REQ-CI-PEF-042).
    #[allow(clippy::too_many_arguments)]
    pub fn pe_reply<'h>(
        &mut self,
        peer: Muid,
        kind: PeKind,
        request_id: u8,
        header: &'h [u8],
        total_len: Option<usize>,
        chunk_size: Option<usize>,
    ) -> Result<ChunkWriter<'h>, DeviceError> {
        // DEC-017 addendum of 2026-10-22: a call of a category that this build does not have.
        if !cfg!(feature = "property-exchange") {
            return Err(DeviceError::NotSupported);
        }
        if kind == PeKind::Notify || matches!(kind, PeKind::GetInquiry | PeKind::SetInquiry) {
            return Err(DeviceError::NotSupported);
        }
        let max = self.peer_max(peer).unwrap_or(DEFAULT_MAX_SYSEX);
        self.used = true;
        Ok(ChunkWriter::new(
            kind,
            request_id,
            header,
            max,
            chunk_size,
            total_len,
            self.muid,
            peer,
            self.config.framing,
        )?)
    }

    /// Ends a Property Exchange Request.
    pub fn pe_end(&mut self, peer: Muid, request_id: u8, direction: Direction) {
        #[cfg(feature = "property-exchange")]
        self.requests.close(peer, request_id, direction);
        #[cfg(not(feature = "property-exchange"))]
        let _ = (peer, request_id, direction);
    }

    /// Writes Inquiry: Process Inquiry Capabilities; version 2 peers only.
    pub fn pi_capabilities_inquiry(
        &mut self,
        peer: Muid,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-017 addendum of 2026-10-22, DEC-CI-001: an Initiator call of a category or role that
        // this build does not have.
        if !cfg!(all(feature = "process-inquiry", feature = "initiator")) {
            return Err(DeviceError::NotSupported);
        }
        self.version_2(peer)?;
        self.used = true;
        Ok(PiCapabilitiesInquiry::write(
            self.muid,
            peer,
            self.config.framing,
            out,
        )?)
    }

    /// Writes Inquiry: MIDI Message Report and opens the report Transaction (REQ-CI-MMR-037).
    pub fn message_report_inquiry(
        &mut self,
        peer: Muid,
        device_id: DeviceId,
        inquiry: &MessageReportInquiry,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-017 addendum of 2026-10-22, DEC-CI-001: an Initiator call of a category or role that
        // this build does not have.
        #[cfg(not(all(feature = "process-inquiry", feature = "initiator")))]
        {
            let _ = (peer, device_id, inquiry, out);
            Err(DeviceError::NotSupported)
        }
        #[cfg(all(feature = "process-inquiry", feature = "initiator"))]
        {
            self.version_2(peer)?;
            let n = inquiry.write(device_id, self.muid, peer, self.config.framing, out)?;
            self.report_peer = Some(peer);
            self.used = true;
            Ok(n)
        }
    }

    /// True while a MIDI Message Report from `peer` is open.
    pub fn report_open(&self, peer: Muid) -> bool {
        #[cfg(feature = "process-inquiry")]
        return self.report_peer == Some(peer);
        #[cfg(not(feature = "process-inquiry"))]
        {
            let _ = peer;
            false
        }
    }

    /// Writes an ACK with `text`; it fits the peer's maximum size, 128 when unknown.
    pub fn ack(
        &self,
        peer: Muid,
        device_id: DeviceId,
        ack: &Ack<'_>,
        text: &str,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // MIDI-CI ACK is a version 2 message (REQ-CI-MSG-010).
        if self.peer(peer).is_some_and(|p| p.version.minor() < 2) {
            return Err(DeviceError::PeerVersion);
        }
        Ok(ack.write(
            text,
            device_id,
            self.muid,
            peer,
            self.peer_max(peer),
            self.config.framing,
            out,
        )?)
    }

    /// Writes a NAK with `text`, with the size rule of `ack`.
    pub fn nak(
        &self,
        peer: Muid,
        device_id: DeviceId,
        nak: &Nak<'_>,
        text: &str,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        Ok(nak.write(
            text,
            device_id,
            self.muid,
            peer,
            self.peer_max(peer),
            self.config.framing,
            out,
        )?)
    }

    /// The application enabled, disabled or refused a Profile, or a local action changed it:
    /// gives the report to write (REQ-CI-PROF-027 to -032, REQ-CI-PRFR-028).
    pub fn set_profile_state(
        &mut self,
        address: ProfileAddress,
        profile: ProfileId,
        enabled: bool,
        channels: u16,
    ) -> Option<ProfileReport> {
        // DEC-017 addendum of 2026-10-22, REQ-CI-TIER-004: no Profiles in this build or in the
        // configuration.
        if !self.profiles_on() {
            return None;
        }
        self.profiles
            .set_enabled(address, profile, enabled, channels)
    }

    /// Adds a local Profile; gives the reports to write.
    pub fn add_profile(
        &mut self,
        entry: ProfileEntry,
    ) -> Result<impl Iterator<Item = ProfileReport>, DeviceError> {
        // DEC-017 addendum of 2026-10-22, REQ-CI-TIER-004: a call of a category that this build
        // or the configuration does not have.
        if !self.profiles_on() {
            return Err(DeviceError::NotSupported);
        }
        Ok(self.profiles.add(entry)?)
    }

    /// Removes a local Profile; gives the reports to write.
    pub fn remove_profile(
        &mut self,
        address: ProfileAddress,
        profile: ProfileId,
    ) -> Option<impl Iterator<Item = ProfileReport>> {
        // DEC-017 addendum of 2026-10-22, REQ-CI-TIER-004: no Profiles in this build or in the
        // configuration.
        if !self.profiles_on() {
            return None;
        }
        self.profiles.remove(address, profile)
    }

    /// Writes a Profile report to the Broadcast MUID; gives the UMP Group and the length.
    pub fn profile_report(
        &self,
        report: &ProfileReport,
        out: &mut [u8],
    ) -> Result<(u8, usize), DeviceError> {
        // DEC-017 addendum of 2026-10-22, REQ-CI-TIER-004: a call of a category that this build
        // or the configuration does not have.
        if !self.profiles_on() {
            return Err(DeviceError::NotSupported);
        }
        let (g, n) = report.write(self.muid, self.config.framing, out)?;
        Ok((self.config.first_group.saturating_add(g) & 0x0F, n))
    }

    /// Writes a Reply to Profile Details Inquiry with application data.
    pub fn profile_details_reply(
        &self,
        peer: Muid,
        device_id: DeviceId,
        reply: &ProfileDetailsReply,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-017 addendum of 2026-10-22, REQ-CI-TIER-004: a call of a category that this build
        // or the configuration does not have.
        if !self.profiles_on() {
            return Err(DeviceError::NotSupported);
        }
        Ok(reply.write(device_id, self.muid, peer, self.config.framing, out)?)
    }

    /// Writes the Reply to MIDI Message Report with the Device ID of the inquiry and gives
    /// the ReportWriter for the report messages (REQ-CI-MMR-012, -015, -016). After the last
    /// packet, write `message_report_end`.
    #[allow(clippy::too_many_arguments)]
    pub fn message_report<'s, S: ReportSource + ?Sized>(
        &self,
        peer: Muid,
        device_id: DeviceId,
        group: u8,
        inquiry: &MessageReportInquiry,
        supported: &MessageReportReply,
        protocol: Protocol,
        source: &'s S,
        out: &mut [u8],
    ) -> Result<(usize, ReportWriter<'s, S>), DeviceError> {
        // DEC-017 addendum of 2026-10-22: a call of a category that this build does not have.
        if !cfg!(feature = "process-inquiry") {
            return Err(DeviceError::NotSupported);
        }
        let bits = MessageReportReply::declare(inquiry, supported);
        let n = bits.write(device_id, self.muid, peer, self.config.framing, out)?;
        let c = &self.config;
        let w = ReportWriter::new(
            bits,
            inquiry.data_control,
            device_id,
            group,
            c.first_group,
            c.group_count,
            c.channels_in_use,
            protocol,
            source,
        );
        Ok((n, w))
    }

    /// Writes End of MIDI Message Report (REQ-CI-MMR-016, REQ-CI-MMR-035).
    pub fn message_report_end(
        &self,
        peer: Muid,
        device_id: DeviceId,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-017 addendum of 2026-10-22: a call of a category that this build does not have.
        if !cfg!(feature = "process-inquiry") {
            return Err(DeviceError::NotSupported);
        }
        Ok(MessageReportEnd::write(
            device_id,
            self.muid,
            peer,
            self.config.framing,
            out,
        )?)
    }

    /// Stops a report: NAK 0x20 (Responder terminates), or 0x40 for an error (REQ-CI-MMR-036).
    pub fn abort_report(
        &self,
        peer: Muid,
        device_id: DeviceId,
        error: bool,
        out: &mut [u8],
    ) -> Result<usize, DeviceError> {
        // DEC-017 addendum of 2026-10-22: a call of a category that this build does not have.
        if !cfg!(feature = "process-inquiry") {
            return Err(DeviceError::NotSupported);
        }
        let status = if error {
            NakStatus::Error
        } else {
            NakStatus::Terminate {
                by_initiator: false,
            }
        };
        let nak = Nak {
            original_sub_id2: SubId2::MidiMessageReport.to_byte(),
            status,
            details: Details::Reserved,
            text: &[],
        };
        self.nak(peer, device_id, &nak, "", out)
    }
}
