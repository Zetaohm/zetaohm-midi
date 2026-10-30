//! The host-side reader of one USB Configuration descriptor set with USB-MIDI 2.0 and
//! USB-MIDI 1.0 functions (USB-MIDI v2.0 3.1, 5.2.2.1 and Appendix B; USB-MIDI v1.0 6.1 and
//! Appendix B; USB 2.0 9.4.3; REQ-USB-GEN-003, REQ-USB-MS1-030, REQ-USB-VEC1-014).
//!
//! - [`read_configuration`] takes the descriptor set from a caller slice and gives a
//!   [`FunctionInfo`] view. The view makes no copy and uses no heap (DEC-002): each result comes
//!   from an iterator over the caller slice.
//! - [`FunctionInfo::audio_control`] gives the AudioControl interfaces (the Ac reader). A
//!   configuration with no AudioControl interface is not an error: the reader reads the MIDI
//!   Streaming interfaces also then (REQ-USB-AC-005).
//! - [`FunctionInfo::settings`] gives each Alternate Setting of each interface descriptor with
//!   bInterfaceClass 0x01 (AUDIO) and bInterfaceSubClass 0x03 (MIDISTREAMING) as an
//!   [`AlternateSetting`]. An Alternate Setting ends at the next interface descriptor
//!   (REQ-USB-MS1-030).
//! - The reader selects the descriptor format of each Alternate Setting from the bcdMSC of its
//!   MS Interface Header, not from the Alternate Setting number ([`MsVersion`], REQ-USB-GEN-003,
//!   DEC-USB-001 choice U-01). 0x0100 is USB-MIDI 1.0 and 0x0200 is USB-MIDI 2.0 (exact values
//!   first). Another value with high byte 0x01 or 0x02 uses the format of that high byte and
//!   gives [`HostError::UnknownMinorVersion`]. Another high byte gives
//!   [`HostError::UnknownVersion`]: the reader does not read the other class-specific
//!   descriptors of that setting and continues with the next setting.
//! - A USB-MIDI 1.0 setting gives its Jacks, Elements, endpoints and cable maps through the Ms1
//!   reader. A USB-MIDI 2.0 setting gives its endpoints with their Group Terminal Block IDs
//!   ([`Ms2Endpoints`]); the Group Terminal Block set itself comes from the device by the
//!   request of the Request module.
//! - The reader is lenient (choice U-24): it gives each problem as a [`HostError`] value and
//!   continues. Nothing panics on received bytes. Multi-byte fields are little-endian (USB 2.0
//!   8.1).
//!
//! The module does not talk to a USB bus and does not start a transfer: the USB stack of the
//! caller gives the bytes (DEC-004, DEC-014).

use crate::ac::{audio_control_interfaces, AcInterfaceRead, AcInterfaces};
use crate::constants::{CS_INTERFACE, ENDPOINT, MS_HEADER};
use crate::ms1::{
    Element, Jack, Ms1Desc, Ms1Descriptors, Ms1Endpoint, Ms1Error, MsHeader, TransferEndpoint,
};
use crate::ms2::{
    read_block_ids, read_setting, Ms2Endpoint, Ms2Error, Ms2SettingInfo, MsInterface,
};
use crate::walk::{interfaces, Descriptors, InterfaceGroup, Interfaces, RawDesc, WalkError};

/// bDescriptorType of the standard Configuration descriptor (USB 2.0 Table 9-5).
pub const CONFIGURATION: u8 = 0x02;
/// bLength of the standard Configuration descriptor (USB 2.0 Table 9-10).
pub const CONFIGURATION_LEN: usize = 9;
/// The largest number of problems that one [`AlternateSetting`] or one [`FunctionInfo`] keeps.
pub const MAX_HOST_PROBLEMS: usize = 3;

/// A problem that the host reader found (choice U-24). The reader gives it as a value and
/// continues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    /// The slice does not start with a Configuration descriptor of 9 or more bytes. The reader
    /// reads the interfaces of the whole slice.
    #[error("no Configuration descriptor at the start of the slice")]
    MissingConfiguration,
    /// wTotalLength of the Configuration descriptor is not the length of the slice. The reader
    /// reads the first wTotalLength bytes, or the whole slice when it is shorter or wTotalLength
    /// is below 9.
    #[error("wTotalLength {declared} is not the slice length {available}")]
    TotalLength {
        /// wTotalLength as received.
        declared: u16,
        /// The bytes in the caller slice.
        available: usize,
    },
    /// A bLength below 2 or past the end stopped the descriptor walk (REQ-USB-MS1-006). The
    /// reader gives the interfaces before it.
    #[error("descriptor walk: {0}")]
    Walk(WalkError),
    /// A standard MIDI Streaming interface descriptor with bLength below 9.
    #[error("MIDI Streaming interface descriptor bLength below 9")]
    InterfaceTooShort,
    /// An Alternate Setting with no MS Interface Header: the reader cannot select its format and
    /// does not read its class-specific descriptors (REQ-USB-GEN-003).
    #[error("Alternate Setting with no MS Interface Header")]
    MissingHeader,
    /// An MS Interface Header with bLength below 7: the reader does not read the setting.
    #[error("MS Interface Header bLength below 7")]
    HeaderTooShort,
    /// A bcdMSC with high byte 0x01 or 0x02 and an unknown minor version. The reader uses the
    /// format of the high byte (REQ-USB-GEN-003, choice U-01).
    #[error("bcdMSC {0:#06x}: unknown minor version (REQ-USB-GEN-003)")]
    UnknownMinorVersion(u16),
    /// A bcdMSC with a high byte other than 0x01 and 0x02. The reader does not read the other
    /// class-specific descriptors of the setting (REQ-USB-GEN-003, choice U-01).
    #[error("bcdMSC {0:#06x}: unknown version, setting not read (REQ-USB-GEN-003)")]
    UnknownVersion(u16),
}

impl From<WalkError> for HostError {
    /// The host reader writes nothing, so `WalkError::Overflow` does not come from it; each
    /// walk error is kept as [`HostError::Walk`].
    fn from(e: WalkError) -> Self {
        HostError::Walk(e)
    }
}

/// The descriptor format of an Alternate Setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsFormat {
    /// USB-MIDI 1.0: Jacks, Elements, MS_GENERAL endpoints, USB-MIDI Event Packets.
    Midi1,
    /// USB-MIDI 2.0: MS_GENERAL_2_0 endpoints with Group Terminal Block IDs, UMP.
    Midi2,
}

/// The version of an Alternate Setting from the bcdMSC of its MS Interface Header
/// (REQ-USB-GEN-003, choice U-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsVersion {
    /// bcdMSC 0x0100 (MS_MIDI_1_0).
    Midi1,
    /// bcdMSC 0x0200 (MS_MIDI_2_0).
    Midi2,
    /// Another bcdMSC with high byte 0x01: USB-MIDI 1.0 format, unknown minor version.
    Midi1UnknownMinor(u16),
    /// Another bcdMSC with high byte 0x02: USB-MIDI 2.0 format, unknown minor version.
    Midi2UnknownMinor(u16),
    /// A bcdMSC with another high byte: unknown version, the setting is not read.
    Unknown(u16),
}

impl MsVersion {
    /// The version of a bcdMSC value: exact values first, then the high byte (choice U-01).
    pub const fn from_bcd_msc(bcd_msc: u16) -> Self {
        match bcd_msc {
            0x0100 => MsVersion::Midi1,
            0x0200 => MsVersion::Midi2,
            v => match v >> 8 {
                0x01 => MsVersion::Midi1UnknownMinor(v),
                0x02 => MsVersion::Midi2UnknownMinor(v),
                _ => MsVersion::Unknown(v),
            },
        }
    }

    /// The bcdMSC value.
    pub const fn bcd_msc(self) -> u16 {
        match self {
            MsVersion::Midi1 => 0x0100,
            MsVersion::Midi2 => 0x0200,
            MsVersion::Midi1UnknownMinor(v)
            | MsVersion::Midi2UnknownMinor(v)
            | MsVersion::Unknown(v) => v,
        }
    }

    /// The descriptor format, or `None` for an unknown version.
    pub const fn format(self) -> Option<MsFormat> {
        match self {
            MsVersion::Midi1 | MsVersion::Midi1UnknownMinor(_) => Some(MsFormat::Midi1),
            MsVersion::Midi2 | MsVersion::Midi2UnknownMinor(_) => Some(MsFormat::Midi2),
            MsVersion::Unknown(_) => None,
        }
    }

    /// The problem that this version gives, if any.
    pub const fn problem(self) -> Option<HostError> {
        match self {
            MsVersion::Midi1 | MsVersion::Midi2 => None,
            MsVersion::Midi1UnknownMinor(v) | MsVersion::Midi2UnknownMinor(v) => {
                Some(HostError::UnknownMinorVersion(v))
            }
            MsVersion::Unknown(v) => Some(HostError::UnknownVersion(v)),
        }
    }
}

/// A fixed list of at most [`MAX_HOST_PROBLEMS`] problems, and the count of all problems
/// found, also those that the list cannot keep (choice U-24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Problems {
    items: [Option<HostError>; MAX_HOST_PROBLEMS],
    count: usize,
}

impl Problems {
    fn push(&mut self, e: HostError) {
        if let Some(slot) = self.items.get_mut(self.count) {
            *slot = Some(e);
        }
        self.count = self.count.saturating_add(1);
    }

    fn iter(&self) -> impl Iterator<Item = HostError> + '_ {
        self.items.iter().flatten().copied()
    }
}

/// The standard Configuration descriptor (USB 2.0 Table 9-10; USB-MIDI v1.0 and v2.0 Table B-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConfigurationDescriptor {
    /// wTotalLength (offset 2, 2 bytes): the bytes of the whole descriptor set.
    pub total_length: u16,
    /// bNumInterfaces (offset 4).
    pub num_interfaces: u8,
    /// bConfigurationValue (offset 5).
    pub configuration_value: u8,
    /// iConfiguration (offset 6).
    pub string_index: u8,
    /// bmAttributes (offset 7).
    pub attributes: u8,
    /// MaxPower (offset 8), in units of 2 mA.
    pub max_power: u8,
}

impl ConfigurationDescriptor {
    /// Reads a Configuration descriptor: bDescriptorType 0x02, bLength 9 or more.
    pub fn read(d: &RawDesc<'_>) -> Option<Self> {
        if d.dtype() != CONFIGURATION || d.len() < CONFIGURATION_LEN {
            return None;
        }
        Some(ConfigurationDescriptor {
            total_length: d.u16_le(2)?,
            num_interfaces: d.byte(4)?,
            configuration_value: d.byte(5)?,
            string_index: d.byte(6)?,
            attributes: d.byte(7)?,
            max_power: d.byte(8)?,
        })
    }
}

/// The host view of one Configuration descriptor set (REQ-USB-MS1-030). It holds the caller
/// slice and gives each result through an iterator over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionInfo<'a> {
    /// The Configuration descriptor, if the slice starts with one.
    pub configuration: Option<ConfigurationDescriptor>,
    bytes: &'a [u8],
    problems: Problems,
}

/// Reads a Configuration descriptor set from a caller slice (REQ-USB-MS1-030).
///
/// The reader reads the first wTotalLength bytes of the slice. It reports a missing
/// Configuration descriptor, a wTotalLength that is not the slice length, and a walk error as
/// [`FunctionInfo::problems`], and continues (choice U-24).
pub fn read_configuration(bytes: &[u8]) -> FunctionInfo<'_> {
    let mut problems = Problems::default();
    let configuration = Descriptors::new(bytes)
        .next()
        .and_then(Result::ok)
        .and_then(|d| ConfigurationDescriptor::read(&d));
    let mut view = bytes;
    match configuration {
        None => problems.push(HostError::MissingConfiguration),
        Some(c) => {
            let declared = usize::from(c.total_length);
            if declared != bytes.len() {
                problems.push(HostError::TotalLength {
                    declared: c.total_length,
                    available: bytes.len(),
                });
            }
            if declared >= CONFIGURATION_LEN {
                view = bytes.get(..declared).unwrap_or(bytes);
            }
        }
    }
    if let Some(Err(e)) = Descriptors::new(view).find(Result::is_err) {
        problems.push(e.into());
    }
    FunctionInfo {
        configuration,
        bytes: view,
        problems,
    }
}

impl<'a> FunctionInfo<'a> {
    /// The bytes that the reader reads: the first wTotalLength bytes of the caller slice.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The problems of the descriptor set itself (choice U-24), at most
    /// [`MAX_HOST_PROBLEMS`].
    pub fn problems(&self) -> impl Iterator<Item = HostError> + '_ {
        self.problems.iter()
    }

    /// The number of problems of the descriptor set, also those above [`MAX_HOST_PROBLEMS`]
    /// that [`FunctionInfo::problems`] does not give.
    pub fn problem_count(&self) -> usize {
        self.problems.count
    }

    /// The AudioControl interfaces, in order (REQ-USB-AC-005). A configuration without one
    /// gives no item; it is not an error.
    pub fn audio_control(&self) -> AcInterfaces<'a> {
        audio_control_interfaces(self.bytes)
    }

    /// The first AudioControl interface whose header lists the MIDI Streaming interface
    /// `interface_number` in baInterfaceNr.
    pub fn audio_control_of(&self, interface_number: u8) -> Option<AcInterfaceRead<'a>> {
        self.audio_control().find(|ac| {
            ac.header
                .is_some_and(|h| h.interfaces.contains(&interface_number))
        })
    }

    /// Each Alternate Setting of each MIDI Streaming interface, in descriptor order
    /// (REQ-USB-MS1-030).
    pub fn settings(&self) -> AlternateSettings<'a> {
        AlternateSettings {
            groups: interfaces(self.bytes),
        }
    }

    /// The Alternate Settings of the MIDI Streaming interface `interface_number`.
    pub fn settings_of(&self, interface_number: u8) -> impl Iterator<Item = AlternateSetting<'a>> {
        self.settings()
            .filter(move |s| s.interface.interface_number == interface_number)
    }

    /// The bInterfaceNumber of each MIDI Streaming interface, each one time, in order.
    pub fn ms_interface_numbers(&self) -> MsInterfaceNumbers<'a> {
        MsInterfaceNumbers {
            settings: self.settings(),
            seen: [0; 32],
        }
    }
}

/// Iterator of [`FunctionInfo::settings`].
#[derive(Debug, Clone)]
pub struct AlternateSettings<'a> {
    groups: Interfaces<'a>,
}

impl<'a> Iterator for AlternateSettings<'a> {
    type Item = AlternateSetting<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.groups.by_ref().find_map(AlternateSetting::from_group)
    }
}

/// Iterator of [`FunctionInfo::ms_interface_numbers`].
#[derive(Debug, Clone)]
pub struct MsInterfaceNumbers<'a> {
    settings: AlternateSettings<'a>,
    seen: [u8; 32],
}

impl Iterator for MsInterfaceNumbers<'_> {
    type Item = u8;

    fn next(&mut self) -> Option<u8> {
        loop {
            let n = self.settings.next()?.interface.interface_number;
            let (byte, bit) = (usize::from(n / 8), 1u8 << (n % 8));
            let slot = self.seen.get_mut(byte)?;
            if *slot & bit == 0 {
                *slot |= bit;
                return Some(n);
            }
        }
    }
}

/// One Alternate Setting of a MIDI Streaming interface: the standard interface descriptor and
/// the descriptors after it up to the next interface descriptor (REQ-USB-MS1-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlternateSetting<'a> {
    /// The standard MIDI Streaming interface descriptor.
    pub interface: MsInterface,
    /// The values of the first MS Interface Header (bcdMSC, wTotalLength), if any.
    pub header: Option<MsHeader>,
    /// The version from bcdMSC, if the setting has a header (REQ-USB-GEN-003).
    pub version: Option<MsVersion>,
    group: InterfaceGroup<'a>,
    problems: Problems,
}

impl<'a> AlternateSetting<'a> {
    /// Reads the group of one interface descriptor. Gives `None` when the interface is not a
    /// MIDI Streaming interface (class 0x01, subclass 0x03).
    pub fn from_group(group: InterfaceGroup<'a>) -> Option<Self> {
        let mut problems = Problems::default();
        let d = &group.interface;
        let interface = match MsInterface::read(d)? {
            Ok(i) => i,
            Err(_) => {
                problems.push(HostError::InterfaceTooShort);
                MsInterface {
                    interface_number: d.byte(2).unwrap_or(0),
                    alternate_setting: d.byte(3).unwrap_or(0),
                    num_endpoints: d.byte(4).unwrap_or(0),
                    string_index: d.byte(8).unwrap_or(0),
                }
            }
        };
        let header_desc = Descriptors::new(group.body)
            .flatten()
            .find(|d| d.dtype() == CS_INTERFACE && d.subtype() == Some(MS_HEADER));
        let header = match header_desc {
            None => {
                problems.push(HostError::MissingHeader);
                None
            }
            Some(h) => match MsHeader::read(&h) {
                Ok(h) => Some(h),
                Err(_) => {
                    problems.push(HostError::HeaderTooShort);
                    None
                }
            },
        };
        let version = header.map(|h| MsVersion::from_bcd_msc(h.bcd_msc));
        if let Some(e) = version.and_then(MsVersion::problem) {
            problems.push(e);
        }
        Some(AlternateSetting {
            interface,
            header,
            version,
            group,
            problems,
        })
    }

    /// The descriptor format, or `None` when the reader does not read the setting (no header,
    /// or an unknown version).
    pub fn format(&self) -> Option<MsFormat> {
        self.version.and_then(MsVersion::format)
    }

    /// The problems of the setting (choice U-24): a short interface descriptor, a missing or
    /// short header, an unknown minor version or an unknown version. At most
    /// [`MAX_HOST_PROBLEMS`].
    pub fn problems(&self) -> impl Iterator<Item = HostError> + '_ {
        self.problems.iter()
    }

    /// The number of problems of the setting, also those above [`MAX_HOST_PROBLEMS`] that
    /// [`AlternateSetting::problems`] does not give.
    pub fn problem_count(&self) -> usize {
        self.problems.count
    }

    /// The interface group: the interface descriptor and the body after it.
    pub fn group(&self) -> InterfaceGroup<'a> {
        self.group
    }

    fn body_if(&self, format: MsFormat) -> &'a [u8] {
        if self.format() == Some(format) {
            self.group.body
        } else {
            &[]
        }
    }

    /// The lenient Ms1 reader over the setting body. It gives nothing when the setting does
    /// not have the USB-MIDI 1.0 format.
    pub fn ms1_descriptors(&self) -> Ms1Descriptors<'a> {
        Ms1Descriptors::new(self.body_if(MsFormat::Midi1))
    }

    /// The MIDI IN and MIDI OUT Jacks of a USB-MIDI 1.0 setting, with their types and sources.
    pub fn jacks(&self) -> impl Iterator<Item = Jack<'a>> {
        self.ms1_descriptors().filter_map(|d| match d {
            Ok(Ms1Desc::InJack(j)) => Some(Jack::In(j)),
            Ok(Ms1Desc::OutJack(j)) => Some(Jack::Out(j)),
            _ => None,
        })
    }

    /// The Elements of a USB-MIDI 1.0 setting.
    pub fn elements(&self) -> impl Iterator<Item = Element<'a>> {
        self.ms1_descriptors().filter_map(|d| match d {
            Ok(Ms1Desc::Element(e)) => Some(e),
            _ => None,
        })
    }

    /// The MIDI endpoints of a USB-MIDI 1.0 setting; [`Ms1Endpoint::cable_map`] gives the cable
    /// map of each.
    pub fn ms1_endpoints(&self) -> impl Iterator<Item = Ms1Endpoint<'a>> {
        self.ms1_descriptors().filter_map(|d| match d {
            Ok(Ms1Desc::Endpoint(e)) => Some(e),
            _ => None,
        })
    }

    /// The Transfer bulk endpoints of a USB-MIDI 1.0 setting.
    pub fn transfer_endpoints(&self) -> impl Iterator<Item = TransferEndpoint> + 'a {
        self.ms1_descriptors().filter_map(|d| match d {
            Ok(Ms1Desc::Transfer(e)) => Some(e),
            _ => None,
        })
    }

    /// The problems that the Ms1 reader found in a USB-MIDI 1.0 setting.
    pub fn ms1_problems(&self) -> impl Iterator<Item = Ms1Error> + 'a {
        self.ms1_descriptors().filter_map(Result::err)
    }

    /// The endpoints of a USB-MIDI 2.0 setting, each with its Group Terminal Block IDs. It gives
    /// nothing when the setting does not have the USB-MIDI 2.0 format.
    pub fn ms2_endpoints(&self) -> Ms2Endpoints<'a> {
        Ms2Endpoints {
            walk: Descriptors::new(self.body_if(MsFormat::Midi2)),
        }
    }

    /// The Group Terminal Block IDs of a USB-MIDI 2.0 setting: the IDs of each endpoint, in
    /// descriptor order (an ID can come more than once).
    pub fn block_ids(&self) -> impl Iterator<Item = u8> + 'a {
        self.ms2_endpoints().flatten().flat_map(|e| {
            let ids = e.blocks;
            (0..ids.len()).filter_map(move |i| ids.get(i).copied())
        })
    }

    /// The Ms2 reader result of a USB-MIDI 2.0 setting, with a fixed capacity of `E` endpoints.
    /// Gives `None` when the setting does not have the USB-MIDI 2.0 format.
    pub fn ms2_setting<const E: usize>(&self) -> Option<Ms2SettingInfo<E>> {
        if self.format() == Some(MsFormat::Midi2) {
            read_setting(self.group)
        } else {
            None
        }
    }
}

/// The endpoints of one USB-MIDI 2.0 Alternate Setting, each with the Group Terminal Block IDs
/// of the MS_GENERAL_2_0 descriptors after it (REQ-USB-MS2-011, REQ-USB-MS2-013).
///
/// Each item is an endpoint (with its problem in [`Ms2Endpoint::error`]) or an error: a
/// standard endpoint descriptor that is too short, or an MS_GENERAL_2_0 descriptor with no
/// endpoint before it. The iterator skips descriptors that it does not know.
#[derive(Debug, Clone)]
pub struct Ms2Endpoints<'a> {
    walk: Descriptors<'a>,
}

impl<'a> Ms2Endpoints<'a> {
    /// Gives the descriptors up to the next standard endpoint descriptor to `f`.
    fn until_endpoint(&mut self, mut f: impl FnMut(&RawDesc<'a>)) {
        loop {
            let mut look = self.walk.clone();
            match look.next() {
                Some(Ok(d)) if d.dtype() != ENDPOINT => {
                    self.walk = look;
                    f(&d);
                }
                _ => return,
            }
        }
    }
}

impl Iterator for Ms2Endpoints<'_> {
    type Item = Result<Ms2Endpoint, Ms2Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let d = self.walk.next()?.ok()?;
            if let Some(ep) = Ms2Endpoint::read(&d) {
                let mut ep = match ep {
                    Ok(ep) => ep,
                    Err(e) => {
                        self.until_endpoint(|_| {});
                        return Some(Err(e));
                    }
                };
                if let Err(e) = ep.packet_size() {
                    ep.error = Some(e);
                }
                self.until_endpoint(|d| match read_block_ids(d) {
                    Some(Ok(ids)) => {
                        for id in ids.iter() {
                            if ep.blocks.push(*id).is_err() {
                                ep.error.get_or_insert(Ms2Error::TooManyBlockIds);
                                break;
                            }
                        }
                    }
                    Some(Err(e)) => {
                        ep.error.get_or_insert(e);
                    }
                    None => {}
                });
                if ep.blocks.is_empty() {
                    ep.error.get_or_insert(Ms2Error::NoBlockIds);
                }
                return Some(Ok(ep));
            }
            if read_block_ids(&d).is_some() {
                return Some(Err(Ms2Error::ClassEndpointWithoutEndpoint));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::constants::EndpointDirection;
    use crate::constants::JackType;
    use crate::ms1::Pin;
    use crate::ms2::TransferType;
    use proptest::prelude::*;
    use std::vec::Vec;

    /// Table B-2 with wTotalLength 0x0065 (USB-MIDI v1.0 Appendix B).
    const B2_V1: [u8; 9] = [0x09, 0x02, 0x65, 0x00, 0x02, 0x01, 0x00, 0x80, 0x32];
    /// Tables B-3 and B-4: the AudioControl interface (18 bytes).
    const B3_B4: [u8; 18] = [
        0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, // B-3
        0x09, 0x24, 0x01, 0x00, 0x01, 0x09, 0x00, 0x01, 0x01, // B-4
    ];
    /// Tables B-5 to B-14: MIDI Streaming interface 1, Alternate Setting 0 (74 bytes).
    const B5_B14: [u8; 74] = [
        0x09, 0x04, 0x01, 0x00, 0x02, 0x01, 0x03, 0x00, 0x00, // B-5
        0x07, 0x24, 0x01, 0x00, 0x01, 0x41, 0x00, // B-6 header, bcdMSC 0x0100
        0x06, 0x24, 0x02, 0x01, 0x01, 0x00, // B-7 Embedded MIDI IN Jack 1
        0x06, 0x24, 0x02, 0x02, 0x02, 0x00, // B-8 External MIDI IN Jack 2
        0x09, 0x24, 0x03, 0x01, 0x03, 0x01, 0x02, 0x01, 0x00, // B-9 Embedded OUT Jack 3
        0x09, 0x24, 0x03, 0x02, 0x04, 0x01, 0x01, 0x01, 0x00, // B-10 External OUT Jack 4
        0x09, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00, // B-11 OUT endpoint 1
        0x05, 0x25, 0x01, 0x01, 0x01, // B-12 MS_GENERAL, Jack 1
        0x09, 0x05, 0x81, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00, // B-13 IN endpoint 1
        0x05, 0x25, 0x01, 0x01, 0x03, // B-14 MS_GENERAL, Jack 3
    ];
    /// Tables B-15 to B-20 (USB-MIDI v2.0 Appendix B): Alternate Setting 1 (40 bytes).
    const B15_B20: [u8; 40] = [
        0x09, 0x04, 0x01, 0x01, 0x02, 0x01, 0x03, 0x00, 0x00, // B-15
        0x07, 0x24, 0x01, 0x00, 0x02, 0x07, 0x00, // B-16 header, bcdMSC 0x0200
        0x07, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00, // B-17 bulk OUT endpoint 1
        0x05, 0x25, 0x02, 0x01, 0x01, // B-18 MS_GENERAL_2_0, block 1
        0x07, 0x05, 0x81, 0x03, 0x40, 0x00, 0x01, // B-19 interrupt IN endpoint 1
        0x05, 0x25, 0x02, 0x01, 0x01, // B-20 MS_GENERAL_2_0, block 1
    ];

    fn cat(parts: &[&[u8]]) -> Vec<u8> {
        parts.iter().flat_map(|p| p.iter().copied()).collect()
    }

    /// The 101 bytes of REQ-USB-VEC1-014.
    fn v1_config() -> Vec<u8> {
        cat(&[&B2_V1, &B3_B4, &B5_B14])
    }

    /// The USB-MIDI v2.0 example: a Configuration descriptor (wTotalLength 9 + 132 = 0x008D)
    /// and Tables B-3 to B-20.
    fn v2_config() -> Vec<u8> {
        let body = cat(&[&B3_B4, &B5_B14, &B15_B20]);
        assert_eq!(body.len(), 132);
        let total = (9 + body.len()) as u16;
        let [lo, hi] = total.to_le_bytes();
        cat(&[&[0x09, 0x02, lo, hi, 0x02, 0x01, 0x00, 0x80, 0x32], &body])
    }

    /// A configuration with a valid wTotalLength around `body`.
    fn config(body: &[u8]) -> Vec<u8> {
        let [lo, hi] = ((9 + body.len()) as u16).to_le_bytes();
        cat(&[&[0x09, 0x02, lo, hi, 0x01, 0x01, 0x00, 0x80, 0x32], body])
    }

    /// One MIDI Streaming setting: interface `n` Alternate Setting `alt`, a header with
    /// `bcd`, then `rest`.
    fn setting(n: u8, alt: u8, bcd: u16, rest: &[u8]) -> Vec<u8> {
        let [lo, hi] = bcd.to_le_bytes();
        cat(&[
            &[9, 4, n, alt, 1, 1, 3, 0, 0],
            &[7, 0x24, 1, lo, hi, 7, 0],
            rest,
        ])
    }

    /// Two Jacks and one MS_GENERAL endpoint (a USB-MIDI 1.0 body after the header).
    const MS1_REST: [u8; 26] = [
        0x06, 0x24, 0x02, 0x01, 0x01, 0x00, // Embedded MIDI IN Jack 1
        0x06, 0x24, 0x02, 0x02, 0x02, 0x00, // External MIDI IN Jack 2
        0x09, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00, // OUT endpoint 1
        0x05, 0x25, 0x01, 0x01, 0x01, // MS_GENERAL, Jack 1
    ];
    /// One USB-MIDI 2.0 endpoint with blocks 1 and 2 (a USB-MIDI 2.0 body after the header).
    const MS2_REST: [u8; 13] = [
        0x07, 0x05, 0x02, 0x02, 0x40, 0x00, 0x00, // bulk OUT endpoint 2
        0x06, 0x25, 0x02, 0x02, 0x01, 0x02, // MS_GENERAL_2_0, blocks 1 and 2
    ];

    #[test]
    fn reads_appendix_b_usb_midi_1_0_configuration() {
        // REQ-USB-VEC1-014: the 101 bytes of Table B-2 (wTotalLength 0x0065) and Tables B-3 to
        // B-14. REQ-USB-MS1-030: the interface by class and subclass, its setting with header,
        // Jacks and endpoints, through iterators. REQ-USB-AC-005: the AC header is read.
        let bytes = v1_config();
        assert_eq!(bytes.len(), 101);
        let f = read_configuration(&bytes);
        assert_eq!(f.problems().count(), 0);
        let c = f.configuration.unwrap();
        assert_eq!(
            (c.total_length, c.num_interfaces, c.configuration_value),
            (0x0065, 2, 1)
        );
        assert_eq!((c.string_index, c.attributes, c.max_power), (0, 0x80, 0x32));

        let acs: Vec<_> = f.audio_control().collect();
        assert_eq!(acs.len(), 1);
        let h = acs[0].header.unwrap();
        assert_eq!((h.bcd_adc, h.interfaces), (0x0100, &[1u8][..]));
        assert_eq!(f.audio_control_of(1), Some(acs[0]));
        assert_eq!(f.audio_control_of(2), None);

        assert_eq!(f.ms_interface_numbers().collect::<Vec<_>>(), [1]);
        let s: Vec<_> = f.settings().collect();
        assert_eq!(s.len(), 1);
        let s = s[0];
        assert_eq!(
            s.interface,
            MsInterface {
                interface_number: 1,
                alternate_setting: 0,
                num_endpoints: 2,
                string_index: 0
            }
        );
        assert_eq!(s.header, Some(MsHeader::new(0x0041)));
        assert_eq!(s.version, Some(MsVersion::Midi1));
        assert_eq!(s.format(), Some(MsFormat::Midi1));
        assert_eq!(s.problems().count(), 0);
        assert_eq!(s.ms1_problems().count(), 0);

        let jacks: Vec<_> = s.jacks().collect();
        let summary: Vec<_> = jacks.iter().map(|j| (j.id(), j.jack_type())).collect();
        assert_eq!(
            summary,
            [
                (1, JackType::Embedded),
                (2, JackType::External),
                (3, JackType::Embedded),
                (4, JackType::External)
            ]
        );
        assert!(matches!(jacks[0], Jack::In(_)) && matches!(jacks[1], Jack::In(_)));
        let sources: Vec<Vec<Pin>> = jacks
            .iter()
            .filter_map(|j| match j {
                Jack::Out(o) => Some(o.pins.iter().collect()),
                Jack::In(_) => None,
            })
            .collect();
        assert_eq!(sources, [[Pin::new(2, 1)], [Pin::new(1, 1)]]);
        assert_eq!(s.elements().count(), 0);
        assert_eq!(s.transfer_endpoints().count(), 0);

        let eps: Vec<_> = s.ms1_endpoints().collect();
        assert_eq!(eps.len(), 2);
        assert_eq!(
            (eps[0].direction, eps[0].number, eps[0].max_packet_size),
            (EndpointDirection::MidiOut, 1, 64)
        );
        assert_eq!(eps[0].direction.address(eps[0].number), 0x01);
        assert_eq!(eps[0].cable_map().cables(), 1);
        assert_eq!(eps[0].cable_map().jack(0), Some(1));
        assert_eq!(eps[1].direction.address(eps[1].number), 0x81);
        assert_eq!(eps[1].cable_map().jack(0), Some(3));
        assert_eq!(eps[1].cable_map().cable(3), Some(0));

        // A USB-MIDI 1.0 setting has no USB-MIDI 2.0 view.
        assert_eq!(s.ms2_endpoints().count(), 0);
        assert!(s.ms2_setting::<4>().is_none());
    }

    #[test]
    fn reads_appendix_b_usb_midi_2_0_example() {
        // Extra vector: the USB-MIDI v2.0 example, Tables B-3 to B-20 (132 bytes) after a
        // Configuration descriptor. REQ-USB-MS1-030 (each Alternate Setting ends at the next
        // interface descriptor), REQ-USB-GEN-003 (format by bcdMSC).
        let bytes = v2_config();
        assert_eq!(bytes.len(), 141);
        let f = read_configuration(&bytes);
        assert_eq!(f.problems().count(), 0);
        assert_eq!(f.configuration.unwrap().total_length, 0x008D);
        assert_eq!(f.ms_interface_numbers().collect::<Vec<_>>(), [1]);
        let s: Vec<_> = f.settings_of(1).collect();
        assert_eq!(s.len(), 2);
        assert_eq!(f.settings_of(0).count(), 0);

        // Alternate Setting 0 keeps only its own descriptors.
        assert_eq!(s[0].version, Some(MsVersion::Midi1));
        assert_eq!(s[0].jacks().count(), 4);
        assert_eq!(s[0].ms1_endpoints().count(), 2);
        assert_eq!(s[0].group().body.len(), B5_B14.len() - 9);

        let a1 = s[1];
        assert_eq!(a1.interface.alternate_setting, 1);
        assert_eq!(
            a1.header.map(|h| (h.bcd_msc, h.total_length)),
            Some((0x0200, 7))
        );
        assert_eq!(a1.version, Some(MsVersion::Midi2));
        assert_eq!(a1.problems().count(), 0);
        assert_eq!(a1.jacks().count(), 0);
        assert_eq!(a1.ms1_endpoints().count(), 0);
        let eps: Vec<_> = a1.ms2_endpoints().map(Result::unwrap).collect();
        assert_eq!(eps.len(), 2);
        assert_eq!(eps[0].address(), 0x01);
        assert_eq!(eps[0].direction, EndpointDirection::MidiOut);
        assert_eq!(
            (eps[0].transfer, eps[0].max_packet),
            (TransferType::Bulk, 64)
        );
        assert_eq!(eps[0].block_ids(), &[1]);
        assert_eq!(eps[1].address(), 0x81);
        assert_eq!(
            (eps[1].transfer, eps[1].interval),
            (TransferType::Interrupt, 1)
        );
        assert_eq!(eps[1].block_ids(), &[1]);
        assert!(eps.iter().all(|e| e.error.is_none()));
        assert_eq!(a1.block_ids().collect::<Vec<_>>(), [1, 1]);

        let info = a1.ms2_setting::<4>().unwrap();
        assert!(info.is_clean());
        assert_eq!(info.endpoints.as_slice(), eps.as_slice());
    }

    #[test]
    fn reads_ms_interface_without_audio_control() {
        // REQ-USB-AC-005, first clause: the reader reads a MIDI Streaming interface also when
        // the configuration has no AudioControl interface.
        let bytes = config(&B5_B14);
        let f = read_configuration(&bytes);
        assert_eq!(f.problems().count(), 0);
        assert_eq!(f.audio_control().count(), 0);
        assert_eq!(f.audio_control_of(1), None);
        let s: Vec<_> = f.settings().collect();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].problems().count(), 0);
        assert_eq!(s[0].jacks().count(), 4);
        assert_eq!(s[0].ms1_endpoints().count(), 2);
        // Also a USB-MIDI 2.0 only function.
        let bytes = config(&B15_B20);
        let f = read_configuration(&bytes);
        assert_eq!(f.audio_control().count(), 0);
        assert_eq!(f.settings().next().unwrap().block_ids().count(), 2);
    }

    #[test]
    fn version_from_bcd_msc_exact_then_high_byte() {
        // REQ-USB-GEN-003, choice U-01: exact 0x0100 and 0x0200 first; high byte 0x01 or 0x02
        // with an unknown-minor report; else unknown.
        let cases = [
            (0x0100, MsVersion::Midi1, Some(MsFormat::Midi1), None),
            (0x0200, MsVersion::Midi2, Some(MsFormat::Midi2), None),
            (
                0x0101,
                MsVersion::Midi1UnknownMinor(0x0101),
                Some(MsFormat::Midi1),
                Some(HostError::UnknownMinorVersion(0x0101)),
            ),
            (
                0x01FF,
                MsVersion::Midi1UnknownMinor(0x01FF),
                Some(MsFormat::Midi1),
                Some(HostError::UnknownMinorVersion(0x01FF)),
            ),
            (
                0x0210,
                MsVersion::Midi2UnknownMinor(0x0210),
                Some(MsFormat::Midi2),
                Some(HostError::UnknownMinorVersion(0x0210)),
            ),
            (
                0x0300,
                MsVersion::Unknown(0x0300),
                None,
                Some(HostError::UnknownVersion(0x0300)),
            ),
            (
                0x0000,
                MsVersion::Unknown(0x0000),
                None,
                Some(HostError::UnknownVersion(0x0000)),
            ),
            (
                0x0001,
                MsVersion::Unknown(0x0001),
                None,
                Some(HostError::UnknownVersion(0x0001)),
            ),
            (
                0x0002,
                MsVersion::Unknown(0x0002),
                None,
                Some(HostError::UnknownVersion(0x0002)),
            ),
        ];
        for (bcd, v, fmt, problem) in cases {
            assert_eq!(MsVersion::from_bcd_msc(bcd), v, "{bcd:#06x}");
            assert_eq!((v.format(), v.problem(), v.bcd_msc()), (fmt, problem, bcd));
        }
    }

    #[test]
    fn prop_version_round_trip() {
        // REQ-USB-GEN-003: every bcdMSC gives one version, and the version keeps the value.
        proptest!(|(bcd in any::<u16>())| {
            let v = MsVersion::from_bcd_msc(bcd);
            prop_assert_eq!(v.bcd_msc(), bcd);
            let expected = match bcd >> 8 {
                1 => Some(MsFormat::Midi1),
                2 => Some(MsFormat::Midi2),
                _ => None,
            };
            prop_assert_eq!(v.format(), expected);
            prop_assert_eq!(v.problem().is_none(), bcd == 0x0100 || bcd == 0x0200);
        });
    }

    #[test]
    fn format_comes_from_bcd_msc_not_alternate_setting() {
        // REQ-USB-GEN-003: Alternate Setting 0 with bcdMSC 0x0200 is read as USB-MIDI 2.0
        // (AMB-USB-MIDI-2-31), Alternate Setting 1 with bcdMSC 0x0100 as USB-MIDI 1.0.
        let bytes = config(&cat(&[
            &setting(1, 0, 0x0200, &MS2_REST),
            &setting(1, 1, 0x0100, &MS1_REST),
        ]));
        let f = read_configuration(&bytes);
        let s: Vec<_> = f.settings().collect();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].format(), Some(MsFormat::Midi2));
        assert_eq!(s[0].problems().count(), 0);
        assert_eq!(s[0].block_ids().collect::<Vec<_>>(), [1, 2]);
        assert_eq!(s[0].jacks().count(), 0);
        assert_eq!(s[1].format(), Some(MsFormat::Midi1));
        assert_eq!(s[1].jacks().count(), 2);
        assert_eq!(
            s[1].ms1_endpoints().next().unwrap().cable_map().jack(0),
            Some(1)
        );
        assert_eq!(s[1].ms2_endpoints().count(), 0);
    }

    #[test]
    fn setting_iterators_interface_numbers_and_problem_count() {
        // REQ-USB-MS1-030: the Elements and the Transfer endpoints of a USB-MIDI 1.0 setting.
        let element = [14, 0x24, 4, 5, 1, 1, 1, 1, 0, 0, 2, 0xFF, 0xFF, 0];
        let transfer = [9, 5, 0x82, 2, 64, 0, 0, 0, 0];
        let rest = cat(&[&MS1_REST, &element, &transfer]);
        let bytes = config(&cat(&[
            &setting(1, 0, 0x0110, &rest),
            &setting(9, 0, 0x0100, &MS1_REST),
        ]));
        let f = read_configuration(&bytes);
        let s = f.settings().next().unwrap();
        assert_eq!(s.elements().count(), 1);
        assert_eq!(s.transfer_endpoints().count(), 1);
        assert_eq!(s.problem_count(), 1, "the unknown minor version");
        // Each MIDI Streaming interface number once, also numbers above 7.
        assert_eq!(f.ms_interface_numbers().collect::<Vec<_>>(), [1, 9]);
        // A Configuration descriptor with more than 9 bytes is read.
        let mut long = config(&setting(1, 0, 0x0100, &MS1_REST));
        long[0] = 10;
        long.insert(9, 0);
        let [lo, hi] = (long.len() as u16).to_le_bytes();
        long[2] = lo;
        long[3] = hi;
        assert_eq!(read_configuration(&long).problem_count(), 0);
    }

    #[test]
    fn unknown_minor_version_reads_by_high_byte_and_reports() {
        // REQ-USB-GEN-003: high byte 0x01 or 0x02 with another minor version: the format of
        // the high byte, and an unknown-minor report.
        let bytes = config(&cat(&[
            &setting(1, 0, 0x0110, &MS1_REST),
            &setting(1, 1, 0x0201, &MS2_REST),
        ]));
        let f = read_configuration(&bytes);
        let s: Vec<_> = f.settings().collect();
        assert_eq!(s[0].version, Some(MsVersion::Midi1UnknownMinor(0x0110)));
        assert_eq!(
            s[0].problems().collect::<Vec<_>>(),
            [HostError::UnknownMinorVersion(0x0110)]
        );
        assert_eq!(s[0].jacks().count(), 2);
        assert_eq!(s[0].ms1_endpoints().count(), 1);
        assert_eq!(s[1].version, Some(MsVersion::Midi2UnknownMinor(0x0201)));
        assert_eq!(
            s[1].problems().collect::<Vec<_>>(),
            [HostError::UnknownMinorVersion(0x0201)]
        );
        assert_eq!(s[1].block_ids().collect::<Vec<_>>(), [1, 2]);
        assert!(s[1].ms2_setting::<2>().is_some());
    }

    #[test]
    fn setting_reader_skips_unknown_descriptors_and_reports_short_ones() {
        // REQ-USB-MS1-006: an unknown type, subtype 0x00 and a CS_GR_TRM_BLOCK (0x26) are
        // skipped; a Jack that is too short gives one problem, and the next Jack is read.
        let rest = cat(&[
            &[0x06, 0x24, 0x02, 0x01, 0x01, 0x00], // Embedded MIDI IN Jack 1
            &[0x03, 0x77, 0x00],                   // unknown bDescriptorType
            &[0x03, 0x24, 0x00],                   // subtype 0x00 (undefined)
            &[0x05, 0x26, 0x01, 0x00, 0x00],       // CS_GR_TRM_BLOCK
            &[0x04, 0x24, 0x02, 0x01],             // MIDI IN Jack, too short
            &[0x06, 0x24, 0x02, 0x02, 0x02, 0x00], // External MIDI IN Jack 2
        ]);
        let bytes = config(&setting(1, 0, 0x0100, &rest));
        let f = read_configuration(&bytes);
        let s = f.settings().next().unwrap();
        let ids: Vec<_> = s.jacks().map(|j| j.id()).collect();
        assert_eq!(ids, [1, 2]);
        // The skipped descriptors are `Other`; the short Jack gives one error.
        assert_eq!(s.ms1_problems().count(), 1);
        let kinds: Vec<_> = s
            .ms1_descriptors()
            .map(|d| match d {
                Ok(Ms1Desc::Header(_)) => 'h',
                Ok(Ms1Desc::InJack(_)) => 'j',
                Ok(Ms1Desc::Other(_)) => 'o',
                Ok(_) => '?',
                Err(_) => 'e',
            })
            .collect();
        assert_eq!(kinds, ['h', 'j', 'o', 'o', 'o', 'e', 'j']);
    }

    #[test]
    fn unknown_version_skips_setting_and_continues() {
        // REQ-USB-GEN-003: another high byte: report an unknown version, do not read the other
        // class-specific descriptors of the setting, continue with the next setting.
        let bytes = config(&cat(&[
            &setting(1, 0, 0x0300, &MS1_REST),
            &setting(1, 1, 0x0000, &MS2_REST),
            &setting(1, 2, 0x0200, &MS2_REST),
        ]));
        let f = read_configuration(&bytes);
        let s: Vec<_> = f.settings().collect();
        assert_eq!(s.len(), 3);
        for (i, bcd) in [(0, 0x0300), (1, 0x0000)] {
            assert_eq!(s[i].version, Some(MsVersion::Unknown(bcd)));
            assert_eq!(s[i].format(), None);
            assert_eq!(
                s[i].problems().collect::<Vec<_>>(),
                [HostError::UnknownVersion(bcd)]
            );
            assert_eq!(s[i].ms1_descriptors().count(), 0);
            assert_eq!(s[i].jacks().count(), 0);
            assert_eq!(s[i].ms2_endpoints().count(), 0);
            assert_eq!(s[i].block_ids().count(), 0);
            assert!(s[i].ms2_setting::<2>().is_none());
        }
        assert_eq!(s[2].version, Some(MsVersion::Midi2));
        assert_eq!(s[2].block_ids().collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn setting_without_header_is_not_read() {
        // REQ-USB-GEN-003: the format comes only from bcdMSC; with no header (or a short one)
        // the reader reports it and reads no class-specific descriptor.
        let bytes = config(&cat(&[&[9, 4, 1, 0, 1, 1, 3, 0, 0], &MS1_REST]));
        let f = read_configuration(&bytes);
        let s = f.settings().next().unwrap();
        assert_eq!((s.header, s.version, s.format()), (None, None, None));
        assert_eq!(s.problems().collect::<Vec<_>>(), [HostError::MissingHeader]);
        assert_eq!(s.jacks().count(), 0);
        let bytes = config(&cat(&[&[9, 4, 1, 0, 1, 1, 3, 0, 0], &[5, 0x24, 1, 0, 1]]));
        let f = read_configuration(&bytes);
        let s = f.settings().next().unwrap();
        assert_eq!(
            s.problems().collect::<Vec<_>>(),
            [HostError::HeaderTooShort]
        );
        assert_eq!(s.format(), None);
    }

    #[test]
    fn settings_end_at_next_interface_and_skip_other_classes() {
        // REQ-USB-MS1-030: only class 0x01 subclass 0x03 interfaces; a setting ends at the
        // next interface descriptor, also one of another class.
        let hid = [9, 4, 2, 0, 1, 3, 0, 0, 0, 6, 0x24, 2, 1, 9, 0];
        let bytes = config(&cat(&[
            &B3_B4,
            &setting(1, 0, 0x0100, &MS1_REST),
            &hid,
            &setting(3, 0, 0x0100, &MS1_REST[..12]),
            &setting(3, 1, 0x0200, &MS2_REST),
        ]));
        let f = read_configuration(&bytes);
        assert_eq!(f.problems().count(), 0);
        assert_eq!(f.ms_interface_numbers().collect::<Vec<_>>(), [1, 3]);
        let s: Vec<_> = f.settings().collect();
        assert_eq!(s.len(), 3);
        // The Jack of the HID interface body is not in setting 1/0.
        assert_eq!(s[0].jacks().count(), 2);
        assert!(s[0].jacks().all(|j| j.id() != 9));
        assert_eq!(s[1].jacks().count(), 2);
        assert_eq!(s[1].ms1_endpoints().count(), 0);
        assert_eq!(f.settings_of(3).count(), 2);
        assert_eq!(f.settings_of(2).count(), 0);
    }

    #[test]
    fn problem_list_counts_the_problems_it_cannot_keep() {
        // U-24: the list keeps the first MAX_HOST_PROBLEMS problems and counts all of them,
        // so that a lost problem shows.
        let mut p = Problems::default();
        assert_eq!((p.iter().count(), p.count), (0, 0));
        let all = [
            HostError::MissingConfiguration,
            HostError::InterfaceTooShort,
            HostError::MissingHeader,
            HostError::HeaderTooShort,
            HostError::UnknownVersion(0x0300),
        ];
        for e in all {
            p.push(e);
        }
        assert_eq!(p.iter().collect::<Vec<_>>(), all[..MAX_HOST_PROBLEMS]);
        assert_eq!(p.count, all.len());
        let bytes = v1_config();
        let f = read_configuration(&bytes);
        assert_eq!(f.problem_count(), 0);
        assert!(f
            .settings()
            .all(|s| s.problem_count() == s.problems().count()));
    }

    #[test]
    fn configuration_problems_are_values() {
        // U-24: a missing Configuration descriptor, a wTotalLength that is not the slice length
        // and a walk error are reported; the reader continues.
        let f = read_configuration(&B5_B14);
        assert_eq!(
            f.problems().collect::<Vec<_>>(),
            [HostError::MissingConfiguration]
        );
        assert_eq!(f.settings().count(), 1);

        // Extra bytes after wTotalLength are not read.
        let mut long = v1_config();
        long.extend_from_slice(&B15_B20);
        let f = read_configuration(&long);
        assert_eq!(
            f.problems().collect::<Vec<_>>(),
            [HostError::TotalLength {
                declared: 101,
                available: 141
            }]
        );
        assert_eq!(f.bytes().len(), 101);
        assert_eq!(f.settings().count(), 1);

        // A slice shorter than wTotalLength: the reader reads what is there.
        let bytes = v1_config();
        let short = &bytes[..bytes.len() - 3];
        let f = read_configuration(short);
        assert_eq!(
            f.problems().collect::<Vec<_>>(),
            [
                HostError::TotalLength {
                    declared: 101,
                    available: 98
                },
                HostError::Walk(WalkError::Truncated)
            ]
        );
        assert_eq!(f.problem_count(), 2);
        let s = f.settings().next().unwrap();
        assert_eq!(s.jacks().count(), 4);
        assert_eq!(s.ms1_endpoints().count(), 1);

        // A wTotalLength below 9: the whole slice is read.
        let mut bad = v1_config();
        bad[2] = 0;
        let f = read_configuration(&bad);
        assert_eq!(f.bytes().len(), 101);
        assert_eq!(f.settings().count(), 1);
    }

    #[test]
    fn short_interface_descriptor_is_reported() {
        // U-24: an MS interface descriptor with bLength 7 (class and subclass present).
        let bytes = config(&cat(&[&[7, 4, 1, 0, 1, 1, 3], &[7, 0x24, 1, 0, 1, 7, 0]]));
        let f = read_configuration(&bytes);
        let s = f.settings().next().unwrap();
        assert_eq!(s.interface.interface_number, 1);
        assert_eq!(
            s.problems().collect::<Vec<_>>(),
            [HostError::InterfaceTooShort]
        );
        assert_eq!(s.format(), Some(MsFormat::Midi1));
    }

    #[test]
    fn ms2_endpoint_problems_are_values() {
        // U-24: a class-specific endpoint descriptor before any endpoint, an endpoint with no
        // block ID, a too short endpoint descriptor.
        let rest = [
            5, 0x25, 2, 1, 1, // MS_GENERAL_2_0 before any endpoint
            7, 5, 0x01, 2, 0x40, 0, 0, // endpoint 1, no block ID
            6, 5, 0x82, 2, 0x40, 0, // endpoint too short
            5, 0x25, 2, 1, 3, // belongs to the short endpoint: dropped
            7, 5, 0x83, 2, 0x40, 0, 0, // endpoint 3
            5, 0x25, 2, 1, 4, // block 4
        ];
        let bytes = config(&setting(1, 1, 0x0200, &rest));
        let f = read_configuration(&bytes);
        let s = f.settings().next().unwrap();
        let items: Vec<_> = s.ms2_endpoints().collect();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0], Err(Ms2Error::ClassEndpointWithoutEndpoint));
        assert_eq!(items[1].unwrap().error, Some(Ms2Error::NoBlockIds));
        assert_eq!(items[2], Err(Ms2Error::EndpointTooShort));
        assert_eq!(items[3].unwrap().block_ids(), &[4]);
        assert_eq!(s.block_ids().collect::<Vec<_>>(), [4]);
    }

    #[test]
    fn ms1_endpoints_give_transfer_interval_and_usable_packet_size() {
        // REQ-USB-MS2-008, REQ-USB-MS2-009, REQ-USB-MS2-011 on Alternate Setting 0 (review
        // correctness M-2): the Ms1 reader of a USB-MIDI 1.0 setting gives the transfer type
        // and bInterval, rounds wMaxPacketSize down, and refuses a bLength below 7.
        let rest = [
            0x06, 0x24, 0x02, 0x01, 0x01, 0x00, // Embedded MIDI IN Jack 1
            0x09, 0x05, 0x01, 0x03, 0x42, 0x00, 0x02, 0x00, 0x00, // interrupt OUT 1, 66, 2
            0x05, 0x25, 0x01, 0x01, 0x01, // MS_GENERAL, Jack 1
            0x06, 0x05, 0x81, 0x03, 0x02, 0x00, // bLength 6: refused
            0x05, 0x25, 0x01, 0x01, 0x03, // its MS_GENERAL: skipped
        ];
        let bytes = config(&setting(1, 0, 0x0100, &rest));
        let f = read_configuration(&bytes);
        let s = f.settings().next().unwrap();
        let eps: Vec<_> = s.ms1_endpoints().collect();
        assert_eq!(eps.len(), 1);
        assert_eq!(
            (eps[0].transfer, eps[0].interval, eps[0].max_packet_size),
            (0x03, 2, 64)
        );
        assert_eq!(
            TransferType::from_attributes(eps[0].transfer),
            TransferType::Interrupt
        );
        let problems: Vec<_> = s.ms1_problems().collect();
        assert_eq!(
            problems,
            [Ms1Error::TooShort {
                dtype: ENDPOINT,
                subtype: 0x81
            }]
        );
    }

    fn exercise(bytes: &[u8]) {
        let f = read_configuration(bytes);
        let _ = f.problems().count();
        let _ = f.audio_control().count();
        let _ = f.ms_interface_numbers().count();
        for s in f.settings() {
            let _ = (
                s.problems().count(),
                s.jacks().count(),
                s.elements().count(),
            );
            let _ = (s.ms1_endpoints().count(), s.transfer_endpoints().count());
            let _ = (s.ms1_problems().count(), s.block_ids().count());
            let eps: Vec<_> = s.ms2_endpoints().flatten().collect();
            if let Some(info) = s.ms2_setting::<16>() {
                if eps.len() <= 16 {
                    assert_eq!(info.endpoints.as_slice(), eps.as_slice());
                }
            }
        }
    }

    #[test]
    fn prop_reader_never_panics() {
        // U-24: nothing panics on received bytes; the lazy endpoint reader agrees with
        // ms2::read_setting.
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..300))| {
            exercise(&bytes);
        });
        let base = v2_config();
        proptest!(|(at in 0..141usize, value in any::<u8>(), cut in 0..=141usize)| {
            let mut b = base.clone();
            b[at] = value;
            exercise(&b[..cut]);
        });
    }
}
