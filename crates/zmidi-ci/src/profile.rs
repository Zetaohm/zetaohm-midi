//! MIDI-CI Profile Configuration (M2-101-UM v1.2.1 section 7, Sub-ID#2 0x20-0x29, 0x2F;
//! M2-102-U v1.1 sections 2.2-2.5).
//!
//! - A Profile ID is 5 bytes: Standard (Byte 1 0x7E, Bank, Number, Version, Level) or
//!   Manufacturer Specific (SysEx ID in Bytes 1-3, Info in Bytes 4-5) (7.1, M2-102 2.2).
//! - The Device ID of a Profile message is its address: Channel 0x00-0x0F, Group 0x7E,
//!   Function Block 0x7F (7.2, REQ-CI-PRFR-003). A Multi-Channel Profile is addressed to its
//!   Manager Channel (REQ-CI-PRFR-022).
//! - The four reports (Added, Removed, Enabled, Disabled) go to the Broadcast MUID.
//! - [`ProfileTable`] is the responder state of the local Profiles: it answers Profile Inquiry
//!   in the order of REQ-CI-PRFR-009 and gives the reports for each change (DEC-002: fixed
//!   capacity). The application decides whether a Profile can be enabled (DEC-011 D-14).

use crate::management::ManufacturerId;
use crate::message::{
    need, start, u14, DeviceId, Emit, Framing, Header, Muid, ParseError, SubId2, WriteError,
};

fn read_id(body: &[u8]) -> Result<ProfileId, ParseError> {
    need(body, 5)?;
    ProfileId::from_bytes([body[0], body[1], body[2], body[3], body[4]])
}

/// Profile Level, Byte 5 of a Standard Profile ID (M2-102 2.2.1, REQ-CI-PRFR-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileLevel {
    /// 0x00.
    Partial,
    /// 0x01.
    Minimum,
    /// 0x02-0x7E: Minimum Required plus Extended Feature Sets.
    Extended(u8),
    /// 0x7F.
    Highest,
}

impl ProfileLevel {
    /// Reads the Level byte.
    pub fn from_byte(b: u8) -> Result<Self, ParseError> {
        Ok(match b {
            0x00 => ProfileLevel::Partial,
            0x01 => ProfileLevel::Minimum,
            0x02..=0x7E => ProfileLevel::Extended(b),
            0x7F => ProfileLevel::Highest,
            _ => return Err(ParseError::InvalidByte),
        })
    }

    /// The Level byte.
    pub const fn to_byte(self) -> u8 {
        match self {
            ProfileLevel::Partial => 0,
            ProfileLevel::Minimum => 1,
            ProfileLevel::Extended(v) => v & 0x7F,
            ProfileLevel::Highest => 0x7F,
        }
    }
}

/// A 5-byte Profile ID (Table 17, REQ-CI-PROF-001, REQ-CI-PROF-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileId {
    /// Standard Defined Profile (Byte 1 0x7E).
    Standard {
        bank: u8,
        number: u8,
        version: u8,
        level: ProfileLevel,
    },
    /// Manufacturer Specific Profile.
    Manufacturer { id: ManufacturerId, info: [u8; 2] },
}

impl ProfileId {
    /// Reads 5 bytes.
    pub fn from_bytes(b: [u8; 5]) -> Result<Self, ParseError> {
        if b.iter().any(|x| *x > 0x7F) {
            return Err(ParseError::InvalidByte);
        }
        Ok(if b[0] == 0x7E {
            ProfileId::Standard {
                bank: b[1],
                number: b[2],
                version: b[3],
                level: ProfileLevel::from_byte(b[4])?,
            }
        } else {
            ProfileId::Manufacturer {
                id: ManufacturerId::from_bytes([b[0], b[1], b[2]]),
                info: [b[3], b[4]],
            }
        })
    }

    /// True for the same Profile. A Standard Profile is matched without its Level: Set
    /// Profile On carries Level 0x7F, "highest possible" (M2-102 2.2.1, REQ-CI-PRFR-002).
    pub fn same_profile(self, other: ProfileId) -> bool {
        match (self, other) {
            (
                ProfileId::Standard {
                    bank,
                    number,
                    version,
                    ..
                },
                ProfileId::Standard {
                    bank: b,
                    number: n,
                    version: v,
                    ..
                },
            ) => bank == b && number == n && version == v,
            (a, b) => a == b,
        }
    }

    /// The 5 bytes.
    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    pub fn to_bytes(self) -> [u8; 5] {
        match self {
            ProfileId::Standard {
                bank,
                number,
                version,
                level,
            } => [0x7E, bank, number, version, level.to_byte()],
            ProfileId::Manufacturer { id, info } => {
                let m = id.to_bytes();
                [m[0], m[1], m[2], info[0], info[1]]
            }
        }
    }
}

/// Profile Inquiry, 0x20 (REQ-CI-PROF-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileInquiry;

impl ProfileInquiry {
    /// Reads the message; it has no data.
    pub fn read(_header: &Header, _body: &[u8]) -> Result<Self, ParseError> {
        Ok(ProfileInquiry)
    }

    /// Writes the message to `device_id`.
    pub fn write(
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        start(
            device_id,
            SubId2::ProfileInquiry,
            source,
            destination,
            framing,
            out,
        )
        .finish()
    }
}

/// Iterator over a list of Profile IDs in received bytes.
#[derive(Debug, Clone)]
pub struct ProfileIds<'a> {
    bytes: &'a [u8],
}

impl Iterator for ProfileIds<'_> {
    type Item = ProfileId;

    fn next(&mut self) -> Option<ProfileId> {
        while self.bytes.len() >= 5 {
            let (id, rest) = self.bytes.split_at(5);
            self.bytes = rest;
            if let Ok(p) = read_id(id) {
                return Some(p);
            }
        }
        None
    }
}

/// Reply to Profile Inquiry, 0x21 (Table 19, REQ-CI-PROF-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileInquiryReply<'a> {
    enabled: &'a [u8],
    disabled: &'a [u8],
}

impl<'a> ProfileInquiryReply<'a> {
    /// Reads the message.
    pub fn read(_header: &Header, body: &'a [u8]) -> Result<Self, ParseError> {
        need(body, 2)?;
        let n_en = u14(body) as usize * 5;
        need(body, 2 + n_en + 2)?;
        let n_dis = u14(&body[2 + n_en..]) as usize * 5;
        need(body, 4 + n_en + n_dis)?;
        Ok(ProfileInquiryReply {
            enabled: &body[2..2 + n_en],
            disabled: &body[4 + n_en..4 + n_en + n_dis],
        })
    }

    /// The Currently Enabled Profiles.
    pub fn enabled(&self) -> ProfileIds<'a> {
        ProfileIds {
            bytes: self.enabled,
        }
    }

    /// The Currently Disabled Profiles.
    pub fn disabled(&self) -> ProfileIds<'a> {
        ProfileIds {
            bytes: self.disabled,
        }
    }

    /// Writes the reply with Device ID `device_id` (the address whose Profiles it lists).
    pub fn write(
        enabled: impl IntoIterator<Item = ProfileId>,
        disabled: impl IntoIterator<Item = ProfileId>,
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let mut w = start(
            device_id,
            SubId2::ProfileInquiryReply,
            source,
            destination,
            framing,
            out,
        );
        write_list(&mut w, enabled);
        write_list(&mut w, disabled);
        w.finish()
    }
}

fn write_list(w: &mut Emit<'_>, list: impl IntoIterator<Item = ProfileId>) {
    let at = w.len();
    w.u14(0);
    let mut n: u32 = 0;
    for id in list {
        w.bytes(&id.to_bytes());
        n += 1;
    }
    // More than 0x3FFF IDs: InvalidValue.
    w.patch_u14(at, u16::try_from(n).unwrap_or(u16::MAX));
}

/// The Profiles of `entries` at address `a` (any Function Block address for a Function Block)
/// with the enabled flag `enabled`, as a counted list of Table 33 or 34 (REQ-CI-PRFR-007).
#[inline(never)]
fn write_table_list(
    w: &mut Emit<'_>,
    entries: &[Option<ProfileEntry>],
    a: ProfileAddress,
    enabled: bool,
) {
    let at = w.len();
    w.u14(0);
    let mut n: u16 = 0;
    for e in entries.iter().flatten() {
        let here = match a.device_id {
            DeviceId::FunctionBlock => e.address.device_id == DeviceId::FunctionBlock,
            _ => e.address == a,
        };
        if here && e.enabled == enabled {
            w.bytes(&e.profile.to_bytes());
            n += 1;
        }
    }
    w.patch_u14(at, n);
}

fn read_id_count(header: &Header, body: &[u8]) -> Result<(ProfileId, u16), ParseError> {
    let profile = read_id(body)?;
    let channels = if header.version.minor() >= 2 {
        need(body, 7)?;
        u14(&body[5..7])
    } else {
        0
    };
    Ok((profile, channels))
}

/// Set Profile On, 0x22 (Table 20, REQ-CI-PROF-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetProfileOn {
    /// The Profile.
    pub profile: ProfileId,
    /// Number of Channels (14 bits); 0 in a version 1 message.
    pub channels: u16,
}

impl SetProfileOn {
    /// Reads the message; a version 1 message has no count.
    pub fn read(header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        read_id_count(header, body).map(|(profile, channels)| SetProfileOn { profile, channels })
    }
}

/// Profile Enabled Report, 0x24 (Table 22, REQ-CI-PROF-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileEnabledReport {
    /// The Profile.
    pub profile: ProfileId,
    /// Number of Channels (14 bits); 0 in a version 1 message.
    pub channels: u16,
}

impl ProfileEnabledReport {
    /// Reads the message; a version 1 message has no count.
    pub fn read(header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        read_id_count(header, body)
            .map(|(profile, channels)| ProfileEnabledReport { profile, channels })
    }
}

/// Profile Disabled Report, 0x25 (Table 23, REQ-CI-PROF-012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileDisabledReport {
    /// The Profile.
    pub profile: ProfileId,
    /// Number of Channels (14 bits); 0 in a version 1 message.
    pub channels: u16,
}

impl ProfileDisabledReport {
    /// Reads the message; a version 1 message has no count.
    pub fn read(header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        read_id_count(header, body)
            .map(|(profile, channels)| ProfileDisabledReport { profile, channels })
    }
}

#[allow(clippy::too_many_arguments)]
fn write_id_count(
    sub: SubId2,
    profile: ProfileId,
    channels: u16,
    device_id: DeviceId,
    source: Muid,
    destination: Muid,
    framing: Framing,
    out: &mut [u8],
) -> Result<usize, WriteError> {
    let channels = if matches!(device_id, DeviceId::Channel(_)) {
        channels
    } else {
        0
    };
    let mut w = start(device_id, sub, source, destination, framing, out);
    w.bytes(&profile.to_bytes());
    w.u14(channels);
    w.finish()
}

impl SetProfileOn {
    /// Writes the message; Number of Channels Requested is 0 for Device ID 0x7E or 0x7F.
    pub fn write(
        &self,
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        write_id_count(
            SubId2::SetProfileOn,
            self.profile,
            self.channels,
            device_id,
            source,
            destination,
            framing,
            out,
        )
    }
}

impl ProfileEnabledReport {
    /// Writes the report to the Broadcast MUID; the count is 0 for Device ID 0x7E or 0x7F.
    pub fn write(
        &self,
        device_id: DeviceId,
        source: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        write_id_count(
            SubId2::ProfileEnabled,
            self.profile,
            self.channels,
            device_id,
            source,
            Muid::BROADCAST,
            framing,
            out,
        )
    }
}

impl ProfileDisabledReport {
    /// Writes the report to the Broadcast MUID; the count is 0 for Device ID 0x7E or 0x7F.
    pub fn write(
        &self,
        device_id: DeviceId,
        source: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        write_id_count(
            SubId2::ProfileDisabled,
            self.profile,
            self.channels,
            device_id,
            source,
            Muid::BROADCAST,
            framing,
            out,
        )
    }
}

/// Set Profile Off, 0x23 (Table 21, REQ-CI-PROF-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetProfileOff {
    /// The Profile.
    pub profile: ProfileId,
}

impl SetProfileOff {
    /// Reads the message; the 2 reserved bytes are ignored.
    pub fn read(_header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        Ok(SetProfileOff {
            profile: read_id(body)?,
        })
    }

    /// Writes the message with the 2 reserved bytes 0x00.
    pub fn write(
        &self,
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let mut w = start(
            device_id,
            SubId2::SetProfileOff,
            source,
            destination,
            framing,
            out,
        );
        w.bytes(&self.profile.to_bytes());
        w.u14(0);
        w.finish()
    }
}

/// Profile Added Report, 0x26 (Table 24, REQ-CI-PROF-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileAddedReport {
    /// The Profile.
    pub profile: ProfileId,
}

impl ProfileAddedReport {
    /// Reads the message.
    pub fn read(_header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        Ok(ProfileAddedReport {
            profile: read_id(body)?,
        })
    }

    /// Writes the report to the Broadcast MUID.
    pub fn write(
        &self,
        device_id: DeviceId,
        source: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let mut w = start(
            device_id,
            SubId2::ProfileAdded,
            source,
            Muid::BROADCAST,
            framing,
            out,
        );
        w.bytes(&self.profile.to_bytes());
        w.finish()
    }
}

/// Profile Removed Report, 0x27 (Table 25, REQ-CI-PROF-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileRemovedReport {
    /// The Profile.
    pub profile: ProfileId,
}

impl ProfileRemovedReport {
    /// Reads the message.
    pub fn read(_header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        Ok(ProfileRemovedReport {
            profile: read_id(body)?,
        })
    }

    /// Writes the report to the Broadcast MUID.
    pub fn write(
        &self,
        device_id: DeviceId,
        source: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let mut w = start(
            device_id,
            SubId2::ProfileRemoved,
            source,
            Muid::BROADCAST,
            framing,
            out,
        );
        w.bytes(&self.profile.to_bytes());
        w.finish()
    }
}

/// Profile Details Inquiry, 0x28 (Table 26, REQ-CI-PROF-007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileDetailsInquiry {
    /// The Profile.
    pub profile: ProfileId,
    /// Inquiry Target: 0x00-0x3F registered, 0x40-0x7F Profile specific.
    pub target: u8,
}

impl ProfileDetailsInquiry {
    /// Reads the message.
    pub fn read(_header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        need(body, 6)?;
        Ok(ProfileDetailsInquiry {
            profile: read_id(body)?,
            target: body[5],
        })
    }

    /// Writes the message.
    pub fn write(
        &self,
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let mut w = start(
            device_id,
            SubId2::ProfileDetailsInquiry,
            source,
            destination,
            framing,
            out,
        );
        w.bytes(&self.profile.to_bytes());
        w.u7(self.target);
        w.finish()
    }
}

/// Reply to Profile Details Inquiry, 0x29 (Table 27, REQ-CI-PROF-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileDetailsReply<'a> {
    /// The Profile.
    pub profile: ProfileId,
    /// Inquiry Target of the inquiry (REQ-CI-PROF-024).
    pub target: u8,
    /// Inquiry Target Data, uninterpreted (REQ-CI-PRFR-018).
    pub data: &'a [u8],
}

impl<'a> ProfileDetailsReply<'a> {
    /// Reads the message.
    pub fn read(_header: &Header, body: &'a [u8]) -> Result<Self, ParseError> {
        need(body, 8)?;
        let len = u14(&body[6..8]) as usize;
        need(body, 8 + len)?;
        Ok(ProfileDetailsReply {
            profile: read_id(body)?,
            target: body[5],
            data: &body[8..8 + len],
        })
    }

    /// Writes the message.
    pub fn write(
        &self,
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let len = u16::try_from(self.data.len()).map_err(|_| WriteError::InvalidValue)?;
        let mut w = start(
            device_id,
            SubId2::ProfileDetailsReply,
            source,
            destination,
            framing,
            out,
        );
        w.bytes(&self.profile.to_bytes());
        w.u7(self.target);
        w.u14(len);
        w.bytes(self.data);
        w.finish()
    }
}

/// Inquiry Target 0x00 data: Number of MIDI Channels (M2-102 2.5.1, REQ-CI-PRFR-017).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelCounts {
    /// Channels in use (Manager and Members), 14 bits.
    pub in_use: u16,
    /// Maximum Number of Channels, 14 bits.
    pub maximum: u16,
}

impl ChannelCounts {
    /// Reads Inquiry Target Data; at least 4 bytes.
    pub fn read(data: &[u8]) -> Result<Self, ParseError> {
        need(data, 4)?;
        if data[..4].iter().any(|b| *b > 0x7F) {
            return Err(ParseError::InvalidByte);
        }
        Ok(ChannelCounts {
            in_use: u14(&data[0..2]),
            maximum: u14(&data[2..4]),
        })
    }

    /// The 4 data bytes.
    pub fn to_bytes(self) -> [u8; 4] {
        [
            (self.in_use & 0x7F) as u8,
            ((self.in_use >> 7) & 0x7F) as u8,
            (self.maximum & 0x7F) as u8,
            ((self.maximum >> 7) & 0x7F) as u8,
        ]
    }
}

/// Largest message to the Broadcast MUID without a chunking mechanism (REQ-CI-MUID-009).
const BROADCAST_LIMIT: usize = 512;

/// Profile Specific Data, 0x2F (Table 28, REQ-CI-PROF-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileSpecificData<'a> {
    /// The Profile.
    pub profile: ProfileId,
    /// Profile Specific Data.
    pub data: &'a [u8],
}

impl<'a> ProfileSpecificData<'a> {
    /// Reads the message.
    pub fn read(_header: &Header, body: &'a [u8]) -> Result<Self, ParseError> {
        need(body, 9)?;
        let len = (body[5] as usize)
            | (body[6] as usize) << 7
            | (body[7] as usize) << 14
            | (body[8] as usize) << 21;
        need(body, 9 + len)?;
        Ok(ProfileSpecificData {
            profile: read_id(body)?,
            data: &body[9..9 + len],
        })
    }

    /// Writes the message. To the Broadcast MUID a message above 512 bytes is an
    /// InvalidValue error unless `profile_defines_chunking` (REQ-CI-PROF-014).
    #[allow(clippy::too_many_arguments)]
    pub fn write(
        &self,
        profile_defines_chunking: bool,
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let total =
            Header::LEN + 9 + self.data.len() + if framing == Framing::SysEx { 2 } else { 0 };
        if destination.is_broadcast() && !profile_defines_chunking && total > BROADCAST_LIMIT {
            return Err(WriteError::InvalidValue);
        }
        let len = u32::try_from(self.data.len()).map_err(|_| WriteError::InvalidValue)?;
        let mut w = start(
            device_id,
            SubId2::ProfileSpecificData,
            source,
            destination,
            framing,
            out,
        );
        w.bytes(&self.profile.to_bytes());
        w.u28(len);
        w.bytes(self.data);
        w.finish()
    }
}

/// Address of a local Profile: Device ID and Group offset in the Function Block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProfileAddress {
    /// Channel (Manager Channel for a Multi-Channel Profile), Group or Function Block.
    pub device_id: DeviceId,
    /// Group offset in the Function Block, 0-15; 0 for a Function Block Profile.
    pub group: u8,
}

/// One local Profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileEntry {
    /// Address.
    pub address: ProfileAddress,
    /// Profile ID.
    pub profile: ProfileId,
    /// Enabled flag.
    pub enabled: bool,
    /// Number of Channels enabled (Manager plus Members), 14 bits.
    pub channels: u16,
    /// Maximum Number of Channels, 14 bits.
    pub max_channels: u16,
}

/// A report that the caller sends to the Broadcast MUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileReport {
    /// Profile Added Report.
    Added(ProfileEntry),
    /// Profile Removed Report.
    Removed(ProfileEntry),
    /// Profile Enabled Report.
    Enabled(ProfileEntry),
    /// Profile Disabled Report.
    Disabled(ProfileEntry),
}

impl ProfileReport {
    /// Writes the report; gives the Group offset of the entry and the length.
    pub fn write(
        &self,
        source: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<(u8, usize), WriteError> {
        let (ProfileReport::Added(e)
        | ProfileReport::Removed(e)
        | ProfileReport::Enabled(e)
        | ProfileReport::Disabled(e)) = self;
        let d = e.address.device_id;
        let n = match self {
            ProfileReport::Added(_) => {
                ProfileAddedReport { profile: e.profile }.write(d, source, framing, out)
            }
            ProfileReport::Removed(_) => {
                ProfileRemovedReport { profile: e.profile }.write(d, source, framing, out)
            }
            ProfileReport::Enabled(_) => ProfileEnabledReport {
                profile: e.profile,
                channels: e.channels,
            }
            .write(d, source, framing, out),
            ProfileReport::Disabled(_) => ProfileDisabledReport {
                profile: e.profile,
                channels: e.channels,
            }
            .write(d, source, framing, out),
        }?;
        Ok((e.address.group, n))
    }
}

/// Responder state of the local Profiles, capacity `N`.
#[derive(Debug, Clone)]
pub struct ProfileTable<const N: usize> {
    entries: [Option<ProfileEntry>; N],
}

#[allow(clippy::new_without_default)]
impl<const N: usize> ProfileTable<N> {
    /// An empty table.
    pub const fn new() -> Self {
        ProfileTable { entries: [None; N] }
    }

    fn index(&self, address: ProfileAddress, profile: ProfileId) -> Option<usize> {
        self.entries.iter().position(
            |e| matches!(e, Some(e) if e.address == address && e.profile.same_profile(profile)),
        )
    }

    /// Adds a Profile; gives Added, then Enabled when the entry is enabled
    /// (REQ-CI-PROF-019, REQ-CI-PROF-020). Overflow when full, InvalidValue when present.
    pub fn add(
        &mut self,
        entry: ProfileEntry,
    ) -> Result<impl Iterator<Item = ProfileReport>, WriteError> {
        if self.index(entry.address, entry.profile).is_some() {
            return Err(WriteError::InvalidValue);
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|e| e.is_none())
            .ok_or(WriteError::Overflow)?;
        *slot = Some(entry);
        Ok([
            Some(ProfileReport::Added(entry)),
            entry.enabled.then_some(ProfileReport::Enabled(entry)),
        ]
        .into_iter()
        .flatten())
    }

    /// Removes a Profile; gives Disabled when it was enabled, then Removed
    /// (REQ-CI-PROF-021, REQ-CI-PROF-022). `None` when not found.
    pub fn remove(
        &mut self,
        address: ProfileAddress,
        profile: ProfileId,
    ) -> Option<impl Iterator<Item = ProfileReport>> {
        let i = self.index(address, profile)?;
        let e = self.entries[i].take()?;
        Some(
            [
                e.enabled.then_some(ProfileReport::Disabled(e)),
                Some(ProfileReport::Removed(e)),
            ]
            .into_iter()
            .flatten(),
        )
    }

    /// Sets the state after the application enabled or disabled a Profile, or gives the
    /// unchanged state after it refused a request (REQ-CI-PROF-027 to REQ-CI-PROF-032).
    /// `channels` is the number enabled, or disabled for a disable. `None` when not found.
    pub fn set_enabled(
        &mut self,
        address: ProfileAddress,
        profile: ProfileId,
        enabled: bool,
        channels: u16,
    ) -> Option<ProfileReport> {
        let i = self.index(address, profile)?;
        let e = self.entries[i].as_mut()?;
        e.enabled = enabled;
        if enabled {
            e.channels = channels;
            Some(ProfileReport::Enabled(*e))
        } else {
            e.channels = 0;
            Some(ProfileReport::Disabled(ProfileEntry { channels, ..*e }))
        }
    }

    /// Finds a Profile at an address.
    pub fn find(&self, address: ProfileAddress, profile: ProfileId) -> Option<&ProfileEntry> {
        self.index(address, profile)
            .and_then(|i| self.entries[i].as_ref())
    }

    /// The entries.
    pub fn entries(&self) -> impl Iterator<Item = &ProfileEntry> + Clone {
        self.entries.iter().flatten()
    }

    fn has(&self, address: ProfileAddress) -> bool {
        self.entries().any(|e| e.address == address)
    }

    /// Replies to a Profile Inquiry to `device_id` received on Group offset `group`
    /// (REQ-CI-PROF-016, REQ-CI-PROF-017, REQ-CI-PRFR-007 to REQ-CI-PRFR-009).
    pub fn inquiry_replies(
        &self,
        device_id: DeviceId,
        group: u8,
        source: Muid,
        destination: Muid,
        framing: Framing,
    ) -> InquiryReplies<'_, N> {
        let step = match device_id {
            DeviceId::FunctionBlock => Step::Channels(0),
            other => Step::One(ProfileAddress {
                device_id: other,
                group,
            }),
        };
        InquiryReplies {
            table: self,
            step,
            group,
            source,
            destination,
            framing,
        }
    }

    /// Writes the Reply to Profile Details Inquiry for Inquiry Target 0x00
    /// (REQ-CI-PRFR-017, REQ-CI-PRFR-019); channels in use are 0 when disabled.
    pub fn details_reply(
        entry: &ProfileEntry,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let counts = ChannelCounts {
            in_use: if entry.enabled { entry.channels } else { 0 },
            maximum: entry.max_channels,
        };
        let data = counts.to_bytes();
        ProfileDetailsReply {
            profile: entry.profile,
            target: 0x00,
            data: &data,
        }
        .write(entry.address.device_id, source, destination, framing, out)
    }
}

#[derive(Debug, Clone, Copy)]
enum Step {
    /// Next Channel slot to check: group * 16 + channel.
    Channels(u16),
    /// Next Group offset to check.
    Groups(u8),
    Last,
    One(ProfileAddress),
    Done,
}

/// Lazy iterator over the Reply to Profile Inquiry messages of one inquiry.
#[derive(Debug, Clone)]
pub struct InquiryReplies<'t, const N: usize> {
    table: &'t ProfileTable<N>,
    step: Step,
    group: u8,
    source: Muid,
    destination: Muid,
    framing: Framing,
}

impl<const N: usize> InquiryReplies<'_, N> {
    fn next_address(&mut self) -> Option<ProfileAddress> {
        loop {
            match self.step {
                Step::Channels(slot) if slot < 256 => {
                    self.step = Step::Channels(slot + 1);
                    let a = ProfileAddress {
                        device_id: DeviceId::Channel((slot % 16) as u8),
                        group: (slot / 16) as u8,
                    };
                    if self.table.has(a) {
                        return Some(a);
                    }
                }
                Step::Channels(_) => self.step = Step::Groups(0),
                Step::Groups(g) if g < 16 => {
                    self.step = Step::Groups(g + 1);
                    let a = ProfileAddress {
                        device_id: DeviceId::Group,
                        group: g,
                    };
                    if self.table.has(a) {
                        return Some(a);
                    }
                }
                Step::Groups(_) => self.step = Step::Last,
                Step::Last => {
                    self.step = Step::Done;
                    return Some(ProfileAddress {
                        device_id: DeviceId::FunctionBlock,
                        group: self.group,
                    });
                }
                Step::One(a) => {
                    self.step = Step::Done;
                    return Some(a);
                }
                Step::Done => return None,
            }
        }
    }

    /// Writes the next reply; gives its Group offset and length. `None` after the last.
    pub fn write_next(&mut self, out: &mut [u8]) -> Option<Result<(u8, usize), WriteError>> {
        let a = self.next_address()?;
        // The bytes of ProfileInquiryReply::write, with one list writer for both lists (one
        // copy of the code, not one for each filter).
        let mut w = start(
            a.device_id,
            SubId2::ProfileInquiryReply,
            self.source,
            self.destination,
            self.framing,
            out,
        );
        write_table_list(&mut w, &self.table.entries, a, true);
        write_table_list(&mut w, &self.table.entries, a, false);
        Some(w.finish().map(|n| (a.group, n)))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::message::Version;
    use proptest::prelude::*;
    use std::vec::Vec;

    const SRC: Muid = match Muid::new(0x0012_3456) {
        Ok(m) => m,
        Err(_) => panic!(),
    };
    const DST: Muid = match Muid::new(0x0065_4321) {
        Ok(m) => m,
        Err(_) => panic!(),
    };
    const GM: ProfileId = ProfileId::Standard {
        bank: 0x21,
        number: 0x00,
        version: 0x01,
        level: ProfileLevel::Highest,
    };
    const DRAW: ProfileId = ProfileId::Standard {
        bank: 0x22,
        number: 0x01,
        version: 0x01,
        level: ProfileLevel::Minimum,
    };

    fn parse(buf: &[u8]) -> (Header, &[u8]) {
        Header::parse(buf).unwrap()
    }

    #[test]
    fn profile_id_forms() {
        // 7.1, M2-102 2.2; REQ-CI-PROF-001, REQ-CI-PROF-002, REQ-CI-PRFR-001.
        assert_eq!(GM.to_bytes(), [0x7E, 0x21, 0x00, 0x01, 0x7F]);
        assert_eq!(ProfileId::from_bytes(GM.to_bytes()), Ok(GM));
        let m1 = ProfileId::Manufacturer {
            id: ManufacturerId::OneByte(0x41),
            info: [1, 2],
        };
        assert_eq!(m1.to_bytes(), [0x41, 0, 0, 1, 2]);
        assert_eq!(ProfileId::from_bytes([0x41, 9, 9, 1, 2]), Ok(m1));
        let m3 = ProfileId::Manufacturer {
            id: ManufacturerId::ThreeByte([0, 0x21, 0x09]),
            info: [3, 4],
        };
        assert_eq!(ProfileId::from_bytes(m3.to_bytes()), Ok(m3));
        assert_eq!(ProfileLevel::from_byte(0x05), Ok(ProfileLevel::Extended(5)));
        // M2-102 2.2.1; REQ-CI-PRFR-001: each Level value, both ways.
        for (b, level) in [
            (0x00, ProfileLevel::Partial),
            (0x01, ProfileLevel::Minimum),
            (0x02, ProfileLevel::Extended(0x02)),
            (0x7E, ProfileLevel::Extended(0x7E)),
            (0x7F, ProfileLevel::Highest),
        ] {
            assert_eq!(ProfileLevel::from_byte(b), Ok(level));
            assert_eq!(level.to_byte(), b);
        }
        assert_eq!(ProfileLevel::from_byte(0x80), Err(ParseError::InvalidByte));
        assert_eq!(
            ProfileId::from_bytes([0x7E, 0x21, 0, 1, 0x00]),
            Ok(ProfileId::Standard {
                bank: 0x21,
                number: 0,
                version: 1,
                level: ProfileLevel::Partial
            })
        );
        assert_eq!(
            ProfileId::from_bytes([0x7E, 0x80, 0, 0, 0]),
            Err(ParseError::InvalidByte)
        );
    }

    #[test]
    fn prop_profile_id_round_trip() {
        proptest!(|(b in proptest::array::uniform5(0u8..0x80))| {
            let id = ProfileId::from_bytes(b).unwrap();
            let back = ProfileId::from_bytes(id.to_bytes()).unwrap();
            prop_assert_eq!(back, id);
        });
    }

    #[test]
    fn inquiry_reply_bytes() {
        // Table 19; REQ-CI-PROF-004.
        let mut buf = [0u8; 64];
        let n = ProfileInquiryReply::write(
            [GM],
            [DRAW],
            DeviceId::Channel(2),
            SRC,
            DST,
            Framing::Bare,
            &mut buf,
        )
        .unwrap();
        assert_eq!(&buf[1..4], &[0x02, 0x0D, 0x21]);
        assert_eq!(
            &buf[13..n],
            &[1, 0, 0x7E, 0x21, 0, 1, 0x7F, 1, 0, 0x7E, 0x22, 1, 1, 1]
        );
        let (h, body) = parse(&buf[..n]);
        let r = ProfileInquiryReply::read(&h, body).unwrap();
        assert_eq!(r.enabled().collect::<Vec<_>>(), [GM]);
        assert_eq!(r.disabled().collect::<Vec<_>>(), [DRAW]);
        assert_eq!(
            ProfileInquiryReply::read(&h, &body[..8]),
            Err(ParseError::TooShort)
        );
        assert_eq!(
            ProfileInquiryReply::read(&h, &body[..body.len() - 1]),
            Err(ParseError::TooShort)
        );
        // REQ-CI-PROF-004: bytes after the last Profile ID are ignored.
        let mut long = body.to_vec();
        long.extend([0x7E, 0x21, 0, 1, 0x7F, 0x33]);
        let r = ProfileInquiryReply::read(&h, &long).unwrap();
        assert_eq!(r.enabled().collect::<Vec<_>>(), [GM]);
        assert_eq!(r.disabled().collect::<Vec<_>>(), [DRAW]);
    }

    #[test]
    fn profile_values_at_their_limits() {
        // A Manufacturer Specific Profile is the same only with the same bytes.
        let m = |info| ProfileId::Manufacturer {
            id: ManufacturerId::OneByte(0x41),
            info,
        };
        assert!(m([1, 2]).same_profile(m([1, 2])));
        assert!(!m([1, 2]).same_profile(m([1, 3])));
        // 7-bit bytes: 0x7F is the largest.
        assert_eq!(
            ChannelCounts::read(&[0x7F; 4]),
            Ok(ChannelCounts {
                in_use: 0x3FFF,
                maximum: 0x3FFF
            })
        );
        assert_eq!(
            ChannelCounts {
                in_use: 0x81,
                maximum: 0x102
            }
            .to_bytes(),
            [0x01, 0x01, 0x02, 0x02]
        );
        // The 4 length bytes, LSB first: a length in byte 3 or 4 is larger than the data.
        let mut buf = [0u8; 64];
        let n =
            ProfileInquiry::write(DeviceId::Channel(0), SRC, DST, Framing::Bare, &mut buf).unwrap();
        let (h, _) = parse(&buf[..n]);
        for len in [[0, 0, 1, 0], [0, 0, 0, 1]] {
            let mut body = std::vec::Vec::from(GM.to_bytes());
            body.extend(len);
            body.extend([1, 2, 3]);
            assert_eq!(
                ProfileSpecificData::read(&h, &body).map(|_| ()),
                Err(ParseError::TooShort),
                "{len:?}"
            );
        }
        // To the Broadcast MUID with SysEx framing: 512 bytes with F0 and F7.
        let big = [0x11u8; 500];
        for (len, ok) in [(488, true), (489, false)] {
            let r = ProfileSpecificData {
                profile: GM,
                data: &big[..len],
            }
            .write(
                false,
                DeviceId::Channel(0),
                SRC,
                Muid::BROADCAST,
                Framing::SysEx,
                &mut [0u8; 600],
            );
            assert_eq!(r.is_ok(), ok, "{len}");
        }
    }

    #[test]
    fn profile_inquiry_message() {
        // Table 18; REQ-CI-PROF-003, REQ-CI-PRFR-003, REQ-CI-PRFR-006.
        let mut buf = [0u8; 32];
        for d in [
            DeviceId::Channel(9),
            DeviceId::Group,
            DeviceId::FunctionBlock,
        ] {
            let n = ProfileInquiry::write(d, SRC, DST, Framing::Bare, &mut buf).unwrap();
            assert_eq!((n, buf[3]), (13, 0x20));
            let (h, body) = parse(&buf[..n]);
            assert_eq!(h.device_id, d);
            assert_eq!(ProfileInquiry::read(&h, body), Ok(ProfileInquiry));
        }
        // Device ID 0x10 to 0x7D: the reserved Device ID rule (REQ-CI-MSG-005).
        for b in [0x10, 0x20, 0x7D] {
            buf[1] = b;
            assert_eq!(
                Header::parse(&buf[..13]),
                Err(ParseError::ReservedDeviceId),
                "{b:#04x}"
            );
        }
    }

    #[test]
    fn table_find_unknown_profile() {
        // REQ-CI-PRFR-014, -015, -025, -027: nothing found, so the Device sends NAK 0x04.
        let mut t = ProfileTable::<2>::new();
        let e = entry(DeviceId::Channel(0), 0, GM, false);
        let _ = t.add(e).unwrap();
        assert!(t.find(e.address, GM).is_some());
        assert!(t.find(e.address, DRAW).is_none());
        let highest = ProfileId::Standard {
            bank: 0x21,
            number: 0x00,
            version: 0x01,
            level: ProfileLevel::Highest,
        };
        let minimum = ProfileId::Standard {
            bank: 0x21,
            number: 0x00,
            version: 0x01,
            level: ProfileLevel::Minimum,
        };
        assert!(
            highest.same_profile(minimum),
            "Level is not part of the match"
        );
        assert!(t.find(e.address, minimum).is_some());
        assert!(t
            .find(
                ProfileAddress {
                    device_id: DeviceId::Channel(1),
                    group: 0
                },
                GM
            )
            .is_none());
        assert!(t.set_enabled(e.address, DRAW, true, 1).is_none());
    }

    #[test]
    fn set_on_off_and_reports() {
        // Tables 20-25; REQ-CI-PROF-005, -006, -009 to -012.
        let mut buf = [0u8; 64];
        let on = SetProfileOn {
            profile: GM,
            channels: 3,
        };
        let n = on
            .write(DeviceId::Channel(0), SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        let (h, body) = parse(&buf[..n]);
        assert_eq!(SetProfileOn::read(&h, body), Ok(on));
        let n = on
            .write(DeviceId::Group, SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        assert_eq!(&buf[n - 2..n], &[0, 0]);
        // Table 20; REQ-CI-PROF-009: Sub-ID#2 0x22, the Profile ID, then the count LSB first;
        // 0x0000 for Device ID 0x7F.
        let wide = SetProfileOn {
            profile: GM,
            channels: 0x0081,
        };
        let n = wide
            .write(DeviceId::Channel(0), SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        assert_eq!(buf[3], 0x22);
        assert_eq!(&buf[13..n], &[0x7E, 0x21, 0x00, 0x01, 0x7F, 0x01, 0x01]);
        let (h, body) = parse(&buf[..n]);
        assert_eq!(SetProfileOn::read(&h, body), Ok(wide));
        let n = wide
            .write(DeviceId::FunctionBlock, SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        assert_eq!(&buf[n - 2..n], &[0, 0]);
        let n = SetProfileOff { profile: GM }
            .write(DeviceId::FunctionBlock, SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        // Table 21; REQ-CI-PROF-010: Profile ID and 2 reserved bytes 0x00 0x00.
        assert_eq!(n, 13 + 7);
        assert_eq!(&buf[..4], &[0x7E, 0x7F, 0x0D, 0x23]);
        assert_eq!(&buf[13..n], &[0x7E, 0x21, 0x00, 0x01, 0x7F, 0x00, 0x00]);
        // The reader ignores the value of the reserved bytes.
        buf[18] = 0x12;
        buf[19] = 0x34;
        let (h, body) = parse(&buf[..n]);
        assert_eq!(
            SetProfileOff::read(&h, body),
            Ok(SetProfileOff { profile: GM })
        );
        let n = ProfileEnabledReport {
            profile: GM,
            channels: 2,
        }
        .write(DeviceId::Channel(1), SRC, Framing::Bare, &mut buf)
        .unwrap();
        assert_eq!(&buf[9..13], &[0x7F; 4]);
        let (h, body) = parse(&buf[..n]);
        assert_eq!(ProfileEnabledReport::read(&h, body).unwrap().channels, 2);
        // Tables 22, 23, 25; REQ-CI-PROF-006, -011, -012: Sub-ID#2, the Device ID, the Broadcast
        // MUID, the Profile ID and, for Enabled and Disabled, the count (0 for 0x7E and 0x7F).
        let src = [0x56, 0x68, 0x48, 0x00];
        let id = [0x7E, 0x21, 0x00, 0x01, 0x7F];
        for (d, b, count) in [
            (DeviceId::Channel(1), 0x01, [3, 0]),
            (DeviceId::Group, 0x7E, [0, 0]),
            (DeviceId::FunctionBlock, 0x7F, [0, 0]),
        ] {
            for sub in [0x24u8, 0x25, 0x27] {
                let n = match sub {
                    0x24 => ProfileEnabledReport {
                        profile: GM,
                        channels: 3,
                    }
                    .write(d, SRC, Framing::Bare, &mut buf),
                    0x25 => ProfileDisabledReport {
                        profile: GM,
                        channels: 3,
                    }
                    .write(d, SRC, Framing::Bare, &mut buf),
                    _ => {
                        ProfileRemovedReport { profile: GM }.write(d, SRC, Framing::Bare, &mut buf)
                    }
                }
                .unwrap();
                let mut want = std::vec![0x7E, b, 0x0D, sub, 0x02];
                want.extend(src);
                want.extend([0x7F; 4]);
                want.extend(id);
                if sub != 0x27 {
                    want.extend(count);
                }
                assert_eq!(&buf[..n], &want[..], "{sub:#04x} {d:?}");
                let (h, body) = parse(&buf[..n]);
                let read = match sub {
                    0x24 => ProfileEnabledReport::read(&h, body).map(|r| (r.profile, r.channels)),
                    0x25 => ProfileDisabledReport::read(&h, body).map(|r| (r.profile, r.channels)),
                    _ => ProfileRemovedReport::read(&h, body).map(|r| (r.profile, 0)),
                };
                let channels = if sub == 0x27 { 0 } else { u16::from(count[0]) };
                assert_eq!(read, Ok((GM, channels)));
            }
        }
        let n = ProfileAddedReport { profile: DRAW }
            .write(DeviceId::Group, SRC, Framing::Bare, &mut buf)
            .unwrap();
        let (h, body) = parse(&buf[..n]);
        assert_eq!(ProfileAddedReport::read(&h, body).unwrap().profile, DRAW);
        // Table 24; REQ-CI-PROF-005: Sub-ID#2 0x26, the Broadcast MUID and the Profile ID.
        let n = ProfileAddedReport { profile: GM }
            .write(DeviceId::Group, SRC, Framing::Bare, &mut buf)
            .unwrap();
        let mut want = std::vec![0x7E, 0x7E, 0x0D, 0x26, 0x02];
        want.extend(src);
        want.extend([0x7F; 4]);
        want.extend(id);
        assert_eq!(&buf[..n], &want[..]);
        // Version 1: no count.
        let mut v1 = [0u8; 64];
        let n = on
            .write(DeviceId::Channel(0), SRC, DST, Framing::Bare, &mut v1)
            .unwrap();
        v1[4] = 0x01;
        let (h, body) = parse(&v1[..n - 2]);
        assert_eq!(SetProfileOn::read(&h, body).unwrap().channels, 0);
    }

    #[test]
    fn details_and_specific_data() {
        // Tables 26-28; REQ-CI-PROF-007, -008, -013, -014, REQ-CI-PRFR-017.
        let mut buf = [0u8; 700];
        let q = ProfileDetailsInquiry {
            profile: GM,
            target: 0,
        };
        let n = q
            .write(DeviceId::Channel(0), SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        let (h, body) = parse(&buf[..n]);
        assert_eq!(ProfileDetailsInquiry::read(&h, body), Ok(q));
        // Table 26; REQ-CI-PROF-007: Sub-ID#2 0x28, the Profile ID, then the Inquiry Target.
        for target in [0x00, 0x40] {
            let n = ProfileDetailsInquiry {
                profile: GM,
                target,
            }
            .write(DeviceId::Channel(0), SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
            assert_eq!(
                &buf[..n],
                &[
                    0x7E, 0x00, 0x0D, 0x28, 0x02, 0x56, 0x68, 0x48, 0x00, 0x21, 0x06, 0x15, 0x03,
                    0x7E, 0x21, 0x00, 0x01, 0x7F, target
                ]
            );
            let (h, body) = parse(&buf[..n]);
            assert_eq!(
                ProfileDetailsInquiry::read(&h, body).unwrap().target,
                target
            );
        }
        let data = ChannelCounts {
            in_use: 2,
            maximum: 200,
        }
        .to_bytes();
        let r = ProfileDetailsReply {
            profile: GM,
            target: 0,
            data: &data,
        };
        let n = r
            .write(DeviceId::Channel(0), SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        // Table 27; REQ-CI-PROF-008, REQ-CI-PRFR-017: Profile ID, target 0, length 4 (LSB
        // first), in use 2 and maximum 200 (0x48 0x01), LSB first.
        assert_eq!(
            &buf[3..n],
            &[
                0x29, 0x02, 0x56, 0x68, 0x48, 0x00, 0x21, 0x06, 0x15, 0x03, 0x7E, 0x21, 0x00, 0x01,
                0x7F, 0x00, 0x04, 0x00, 2, 0, 0x48, 0x01
            ]
        );
        let (h, body) = parse(&buf[..n]);
        let back = ProfileDetailsReply::read(&h, body).unwrap();
        assert_eq!(
            ChannelCounts::read(back.data),
            Ok(ChannelCounts {
                in_use: 2,
                maximum: 200
            })
        );
        assert_eq!(ChannelCounts::read(&data[..3]), Err(ParseError::TooShort));
        // The reader ignores Inquiry Target Data bytes after the fourth.
        assert_eq!(
            ChannelCounts::read(&[2, 0, 0x48, 0x01, 0x11, 0x22]),
            Ok(ChannelCounts {
                in_use: 2,
                maximum: 200
            })
        );
        // REQ-CI-PROF-008: shorter than the length needs: rejected; later bytes: ignored.
        assert_eq!(
            ProfileDetailsReply::read(&h, &body[..body.len() - 1]),
            Err(ParseError::TooShort)
        );
        let mut long = body.to_vec();
        long.extend([0x55, 0x66]);
        assert_eq!(ProfileDetailsReply::read(&h, &long), Ok(back));
        // REQ-CI-PRFR-018: another target gives its data as raw bytes.
        let other = ProfileDetailsReply {
            profile: GM,
            target: 0x40,
            data: &[0x01, 0x7F, 0x00],
        };
        let n = other
            .write(DeviceId::Group, SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        assert_eq!(&buf[18..n], &[0x40, 0x03, 0x00, 0x01, 0x7F, 0x00]);
        let (h, body) = parse(&buf[..n]);
        let r = ProfileDetailsReply::read(&h, body).unwrap();
        assert_eq!((r.target, r.data), (0x40, &[0x01, 0x7F, 0x00][..]));

        let big = [0x11u8; 500];
        let s = ProfileSpecificData {
            profile: GM,
            data: &big,
        };
        assert_eq!(
            s.write(
                false,
                DeviceId::Channel(0),
                SRC,
                Muid::BROADCAST,
                Framing::Bare,
                &mut buf
            ),
            Err(WriteError::InvalidValue)
        );
        assert!(s
            .write(
                true,
                DeviceId::Channel(0),
                SRC,
                Muid::BROADCAST,
                Framing::Bare,
                &mut buf
            )
            .is_ok());
        // REQ-CI-MUID-009, REQ-CI-PROF-014: to the Broadcast MUID, 512 bytes pass and 513 bytes
        // do not.
        for (len, ok) in [(490, true), (491, false)] {
            let s = ProfileSpecificData {
                profile: GM,
                data: &big[..len],
            };
            let r = s.write(
                false,
                DeviceId::Channel(0),
                SRC,
                Muid::BROADCAST,
                Framing::Bare,
                &mut buf,
            );
            if ok {
                assert_eq!(r, Ok(512));
            } else {
                assert_eq!(r, Err(WriteError::InvalidValue));
            }
        }
        let n = s
            .write(
                false,
                DeviceId::Channel(0),
                SRC,
                DST,
                Framing::Bare,
                &mut buf,
            )
            .unwrap();
        // Table 28; REQ-CI-PROF-013: length 500 in 4 bytes, LSB first (0x74 0x03 0x00 0x00).
        assert_eq!(
            &buf[13..22],
            &[0x7E, 0x21, 0x00, 0x01, 0x7F, 0x74, 0x03, 0x00, 0x00]
        );
        let (h, body) = parse(&buf[..n]);
        let back = ProfileSpecificData::read(&h, body).unwrap();
        assert_eq!((back.profile, back.data), (GM, &big[..]));
        assert_eq!(
            ProfileSpecificData::read(&h, &body[..body.len() - 1]),
            Err(ParseError::TooShort)
        );
        let mut long = body.to_vec();
        long.extend([0x01, 0x02]);
        assert_eq!(ProfileSpecificData::read(&h, &long), Ok(back));
    }

    fn entry(device_id: DeviceId, group: u8, profile: ProfileId, enabled: bool) -> ProfileEntry {
        ProfileEntry {
            address: ProfileAddress { device_id, group },
            profile,
            enabled,
            channels: 1,
            max_channels: 1,
        }
    }

    #[test]
    fn table_reports() {
        // REQ-CI-PROF-019 to -022, -027, -029 to -032.
        let mut t = ProfileTable::<4>::new();
        let e = entry(DeviceId::Channel(0), 0, GM, true);
        let r: Vec<_> = t.add(e).unwrap().collect();
        assert_eq!(r, [ProfileReport::Added(e), ProfileReport::Enabled(e)]);
        assert!(t.add(e).is_err());
        let r = t.set_enabled(e.address, GM, false, 1).unwrap();
        let off = ProfileEntry {
            enabled: false,
            channels: 1,
            ..e
        };
        assert_eq!(r, ProfileReport::Disabled(off));
        assert!(!t.find(e.address, GM).unwrap().enabled);
        assert_eq!(t.find(e.address, GM).unwrap().channels, 0);
        // REQ-CI-PROF-029: Set Profile On that the application cannot do: the entry stays
        // disabled and the Device sends a Disabled Report.
        let r = t.set_enabled(e.address, GM, false, 1).unwrap();
        assert_eq!(r, ProfileReport::Disabled(off));
        assert!(!t.find(e.address, GM).unwrap().enabled);
        // REQ-CI-PROF-027, REQ-CI-PRFR-024: Set Profile On that the application does: enabled,
        // with an Enabled Report and its count.
        let r = t.set_enabled(e.address, GM, true, 2).unwrap();
        let on = ProfileEntry {
            enabled: true,
            channels: 2,
            ..e
        };
        assert_eq!(r, ProfileReport::Enabled(on));
        assert_eq!(t.find(e.address, GM), Some(&on));
        // REQ-CI-PROF-032: Set Profile Off that the application cannot do: still enabled, with an
        // Enabled Report.
        let r = t.set_enabled(e.address, GM, true, 2).unwrap();
        assert_eq!(r, ProfileReport::Enabled(on));
        assert!(t.find(e.address, GM).unwrap().enabled);
        // REQ-CI-PROF-022: removing an enabled Profile gives Disabled, then Removed.
        let r: Vec<_> = t.remove(e.address, GM).unwrap().collect();
        assert_eq!(r, [ProfileReport::Disabled(on), ProfileReport::Removed(on)]);
        assert!(t.find(e.address, GM).is_none());
        // A disabled Profile gives Removed only.
        let _ = t.add(entry(DeviceId::Group, 0, DRAW, false)).unwrap();
        let r: Vec<_> = t
            .remove(
                ProfileAddress {
                    device_id: DeviceId::Group,
                    group: 0,
                },
                DRAW,
            )
            .unwrap()
            .collect();
        assert_eq!(
            r,
            [ProfileReport::Removed(entry(
                DeviceId::Group,
                0,
                DRAW,
                false
            ))]
        );
        let mut full = ProfileTable::<1>::new();
        let _ = full.add(e).unwrap();
        assert_eq!(
            full.add(entry(DeviceId::Group, 0, DRAW, false)).err(),
            Some(WriteError::Overflow)
        );
        let mut buf = [0u8; 32];
        let (g, n) = ProfileReport::Enabled(entry(DeviceId::Group, 3, DRAW, true))
            .write(SRC, Framing::Bare, &mut buf)
            .unwrap();
        assert_eq!(g, 3);
        assert_eq!(&buf[n - 2..n], &[0, 0]);
        // REQ-CI-PRFR-023: the Device ID of the Profile in Enabled and Disabled Reports.
        for (d, b) in [
            (DeviceId::Channel(4), 0x04),
            (DeviceId::Channel(15), 0x0F),
            (DeviceId::Group, 0x7E),
            (DeviceId::FunctionBlock, 0x7F),
        ] {
            let e = ProfileEntry {
                channels: 2,
                ..entry(d, 1, DRAW, true)
            };
            for (r, sub) in [
                (ProfileReport::Enabled(e), 0x24),
                (ProfileReport::Disabled(e), 0x25),
            ] {
                let (g, n) = r.write(SRC, Framing::Bare, &mut buf).unwrap();
                assert_eq!((g, buf[1], buf[3]), (1, b, sub));
                let count = if b < 0x10 { 2 } else { 0 };
                assert_eq!(&buf[n - 2..n], &[count, 0]);
            }
        }
    }

    #[test]
    fn inquiry_reply_order_for_function_block() {
        // REQ-CI-PRFR-009: Channels, then Groups, then 0x7F.
        let mut t = ProfileTable::<8>::new();
        let _ = t
            .add(entry(DeviceId::FunctionBlock, 0, DRAW, false))
            .unwrap();
        let _ = t.add(entry(DeviceId::Group, 1, DRAW, true)).unwrap();
        let _ = t.add(entry(DeviceId::Channel(5), 1, GM, true)).unwrap();
        let _ = t.add(entry(DeviceId::Channel(3), 0, GM, false)).unwrap();
        let mut it = t.inquiry_replies(DeviceId::FunctionBlock, 0, SRC, DST, Framing::Bare);
        let mut buf = [0u8; 64];
        let mut seen = Vec::new();
        while let Some(r) = it.write_next(&mut buf) {
            let (g, n) = r.unwrap();
            let (h, body) = parse(&buf[..n]);
            let reply = ProfileInquiryReply::read(&h, body).unwrap();
            seen.push((
                g,
                h.device_id,
                reply.enabled().count(),
                reply.disabled().count(),
            ));
        }
        assert_eq!(
            seen,
            [
                (0, DeviceId::Channel(3), 0, 1),
                (1, DeviceId::Channel(5), 1, 0),
                (1, DeviceId::Group, 1, 0),
                (0, DeviceId::FunctionBlock, 0, 1),
            ]
        );
        // REQ-CI-PROF-017: empty lists for an address with no Profile.
        let mut it = t.inquiry_replies(DeviceId::Channel(9), 0, SRC, DST, Framing::Bare);
        let (_, n) = it.write_next(&mut buf).unwrap().unwrap();
        assert_eq!(&buf[13..n], &[0, 0, 0, 0]);
        assert!(it.write_next(&mut buf).is_none());
    }

    /// The write of `write_next` before the one list writer `write_table_list`: the oracle of
    /// `prop_inquiry_replies_match_generic_writer`.
    fn write_next_generic<const N: usize>(
        it: &mut InquiryReplies<'_, N>,
        out: &mut [u8],
    ) -> Option<Result<(u8, usize), WriteError>> {
        let a = it.next_address()?;
        let at = |e: &&ProfileEntry| match a.device_id {
            DeviceId::FunctionBlock => e.address.device_id == DeviceId::FunctionBlock,
            _ => e.address == a,
        };
        let entries = it.table.entries().filter(at);
        let enabled = entries.clone().filter(|e| e.enabled).map(|e| e.profile);
        let disabled = entries.filter(|e| !e.enabled).map(|e| e.profile);
        Some(
            ProfileInquiryReply::write(
                enabled,
                disabled,
                a.device_id,
                it.source,
                it.destination,
                it.framing,
                out,
            )
            .map(|n| (a.group, n)),
        )
    }

    #[test]
    fn prop_inquiry_replies_match_generic_writer() {
        // REQ-CI-PRFR-007 to -009, REQ-CI-PROF-016, -017: the same replies, Groups, lengths and
        // errors as ProfileInquiryReply::write with the filters, for random tables, inquiry
        // addresses, framings and buffer sizes.
        let dev = prop_oneof![
            (0u8..16).prop_map(DeviceId::Channel),
            Just(DeviceId::Group),
            Just(DeviceId::FunctionBlock),
        ];
        let e = (dev.clone(), 0u8..4, 0u8..3, any::<bool>()).prop_map(|(d, g, p, on)| {
            let profile = ProfileId::Standard {
                bank: 0x21 + p,
                number: p,
                version: 1,
                level: ProfileLevel::Highest,
            };
            entry(d, g, profile, on)
        });
        let framing = prop_oneof![Just(Framing::Bare), Just(Framing::SysEx)];
        proptest!(|(entries in proptest::collection::vec(e, 0..8), q in dev, g in 0u8..4, framing in framing, room in 0usize..48)| {
            let mut t = ProfileTable::<8>::new();
            for e in entries {
                let _ = t.add(e);
            }
            let mut a = t.inquiry_replies(q, g, SRC, DST, framing);
            let mut b = t.inquiry_replies(q, g, SRC, DST, framing);
            loop {
                let mut x = [0u8; 48];
                let mut y = [0u8; 48];
                let ra = a.write_next(&mut x[..room]);
                let rb = write_next_generic(&mut b, &mut y[..room]);
                prop_assert_eq!(ra, rb);
                match ra {
                    None => break,
                    Some(Ok((_, n))) => prop_assert_eq!(&x[..n], &y[..n]),
                    Some(Err(_)) => {}
                }
            }
        });
    }

    #[test]
    fn details_reply_target_zero() {
        // REQ-CI-PRFR-019: channels in use 0 when disabled.
        let mut buf = [0u8; 32];
        let e = ProfileEntry {
            channels: 4,
            max_channels: 8,
            ..entry(DeviceId::Channel(0), 0, GM, false)
        };
        let n = ProfileTable::<1>::details_reply(&e, SRC, DST, Framing::Bare, &mut buf).unwrap();
        let (h, body) = parse(&buf[..n]);
        let r = ProfileDetailsReply::read(&h, body).unwrap();
        assert_eq!(
            ChannelCounts::read(r.data),
            Ok(ChannelCounts {
                in_use: 0,
                maximum: 8
            })
        );
    }

    #[test]
    fn prop_readers_never_panic() {
        proptest!(|(bytes in proptest::collection::vec(0u8..0x80, 0..40), v in 0u8..4)| {
            let h = Header { device_id: DeviceId::Group, sub_id2: 0x21, version: Version::from_byte(v).unwrap(), source: SRC, destination: DST };
            let _ = ProfileInquiryReply::read(&h, &bytes).map(|r| (r.enabled().count(), r.disabled().count()));
            let _ = SetProfileOn::read(&h, &bytes);
            let _ = SetProfileOff::read(&h, &bytes);
            let _ = ProfileDetailsInquiry::read(&h, &bytes);
            let _ = ProfileDetailsReply::read(&h, &bytes);
            let _ = ProfileSpecificData::read(&h, &bytes);
        });
    }
}
