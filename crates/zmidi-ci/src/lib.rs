//! MIDI Capability Inquiry (MIDI-CI, M2-101-UM v1.2.1), with the Profile rules of M2-102-U
//! and the Property Exchange framing of M2-103-UM.
//!
//! - The crate is `no_std` and its core API needs no `alloc` (DEC-002).
//! - The API is sans-IO (DEC-009): readers take one complete MIDI-CI message as a byte slice,
//!   writers fill a caller buffer, and the caller gives time and random numbers.
//! - Property Exchange semantics (JSON headers, Mcoded7, zlib) are in `zmidi-ci-pe` (DEC-006).
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![no_std]

pub mod device;
pub mod management;
pub mod message;
pub mod process;
pub mod profile;
pub mod property;

pub use device::{CiDevice, DeviceConfig, DeviceError, Event, Peer, Received, ReportKind};
pub use management::{
    Ack, AckStatus, Categories, Details, DeviceIdentity, Discovery, DiscoveryReply,
    EndpointInfoInquiry, EndpointInfoReply, EndpointInfoStatus, InvalidateMuid, ManufacturerId,
    Nak, NakStatus, StatusRange,
};
pub use message::{
    Category, DeviceId, Framing, Header, Muid, ParseError, SubId2, TextChars, Version, WriteError,
    Writer,
};
pub use process::{
    ChannelBits, ChannelMode, Controller, DataControl, MessageReportEnd, MessageReportInquiry,
    MessageReportReply, NoteBits, NoteState, PiCapabilities, PiCapabilitiesInquiry, Protocol,
    ReportPacket, ReportSource, ReportWriter, SystemBits,
};
pub use profile::{
    ChannelCounts, InquiryReplies, ProfileAddedReport, ProfileAddress, ProfileDetailsInquiry,
    ProfileDetailsReply, ProfileDisabledReport, ProfileEnabledReport, ProfileEntry, ProfileId,
    ProfileIds, ProfileInquiry, ProfileInquiryReply, ProfileLevel, ProfileRemovedReport,
    ProfileReport, ProfileSpecificData, ProfileTable, SetProfileOff, SetProfileOn,
};
pub use property::{
    ChunkEnd, ChunkWriter, Direction, PeCapabilities, PeChunk, PeKind, RequestTable,
};
