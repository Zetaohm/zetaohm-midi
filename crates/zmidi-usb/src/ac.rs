//! AudioControl interface descriptors of one USB MIDI function, for USB-MIDI v1.0 and v2.0
//! (USB-MIDI v1.0 B.3, USB-MIDI v2.0 5.1 and B.2, USB Audio 1.0 4.3.1 and 4.3.2; DEC-014).
//!
//! - [`StandardAcInterface`] is the Standard AC Interface Descriptor of 9 bytes: class 0x01
//!   AUDIO, subclass 0x01 AUDIO_CONTROL, Alternate Setting 0, no endpoint (REQ-USB-AC-002).
//! - [`AcHeader`] is the Class-specific AC Interface Header Descriptor of 8+n bytes, where n is
//!   the number of MIDIStreaming interfaces: bcdADC 0x0100, wTotalLength equal to its own
//!   bLength, the list baInterfaceNr (REQ-USB-AC-003, REQ-USB-AC-004). The list has a const
//!   generic capacity (DEC-002).
//! - [`write_audio_control`] writes the AudioControl interface of one function: the standard
//!   descriptor, then the header, and no Terminal or Unit descriptor (REQ-USB-AC-001,
//!   REQ-USB-AC-004). The Device module calls it one time for each function, before the
//!   MIDIStreaming interface, and not for each Alternate Setting. It is the same for a function
//!   with USB-MIDI 1.0 and for a function with only USB MIDI 2.0 Alternate Settings.
//! - The writers fill a caller buffer and return the number of bytes. If the buffer is too
//!   small, they return [`AcError::Overflow`] and write nothing. Multi-byte fields are
//!   little-endian (USB 2.0 8.1, AMB-USB-MIDI-1-31, AMB-USB-MIDI-2-25).
//! - The readers are lenient (DEC-USB-001 choice U-24): [`audio_control_interfaces`] gives each
//!   AudioControl interface of a configuration with its problems as values and continues. A
//!   configuration without an AudioControl interface is not an error (REQ-USB-AC-005). The
//!   header view gives bcdADC and baInterfaceNr as a view on the caller slice, with no copy.
//!   Nothing panics on received bytes.
//!
//! No alloc (DEC-002), synchronous and sans-IO with no USB stack dependency (DEC-004, DEC-014).

use crate::constants::{AC_HEADER, AUDIO, AUDIOCONTROL, CS_INTERFACE, INTERFACE};
use crate::walk::{interfaces, DescWriter, Descriptors, InterfaceGroup, Interfaces, WalkError};

/// bcdADC that the writer writes: USB Audio 1.0 (USB-MIDI v1.0 Table B-4, REQ-USB-AC-003).
pub const BCD_ADC: u16 = 0x0100;
/// The largest number of MIDIStreaming interfaces in one header: bLength 8+n fits in one byte.
pub const MAX_IN_COLLECTION: usize = 255 - 8;

/// Error of the AudioControl writers and readers. Each variant names the rule that the data
/// broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AcError {
    /// The caller buffer is too small; nothing is written.
    #[error("buffer too small")]
    Overflow,
    /// A header with no MIDIStreaming interface: n = 0 (REQ-USB-AC-003).
    #[error("bInCollection is 0")]
    NoStreamingInterface,
    /// More interface numbers than the capacity of the list, or than 247 (bLength 8+n is one
    /// byte).
    #[error("too many interface numbers")]
    TooManyInterfaces,
    /// The descriptor is not a Standard AC Interface Descriptor: bLength below 9, a type other
    /// than INTERFACE, or a class and subclass other than AUDIO and AUDIO_CONTROL
    /// (REQ-USB-AC-002).
    #[error("not a Standard AC Interface Descriptor")]
    NotAudioControl,
    /// The descriptor is not a Class-specific AC Interface Header Descriptor: a type other than
    /// CS_INTERFACE or a subtype other than HEADER (REQ-USB-AC-003).
    #[error("not an AC Interface Header Descriptor")]
    NotHeader,
    /// The bLength of a header is less than 8+bInCollection (REQ-USB-AC-005).
    #[error("AC header bLength {len} is less than 8 + bInCollection {in_collection}")]
    HeaderTooShort {
        /// bLength as received.
        len: u8,
        /// bInCollection as received (0 when bLength is below 8).
        in_collection: u8,
    },
    /// The AudioControl interface has no Class-specific AC Interface Header Descriptor
    /// (USB-MIDI v1.0 B.3.2: the class-specific descriptor is always headed by a header).
    #[error("no AC Interface Header Descriptor")]
    MissingHeader,
    /// A bLength below 2, or a descriptor that goes past the end of the slice, stopped the
    /// read or the descriptor walk (REQ-USB-MS1-006). Never `WalkError::Overflow`: that is
    /// [`AcError::Overflow`].
    #[error("descriptor walk: {0}")]
    Walk(WalkError),
}

impl From<WalkError> for AcError {
    fn from(e: WalkError) -> Self {
        match e {
            WalkError::Overflow => AcError::Overflow,
            other => AcError::Walk(other),
        }
    }
}

/// The Standard AC Interface Descriptor (USB-MIDI v1.0 B.3.1 Table B-3, USB-MIDI v2.0 B.2.1
/// Table B-3; REQ-USB-AC-002).
///
/// The writer writes bAlternateSetting 0, bNumEndpoints 0, bInterfaceClass 0x01 AUDIO,
/// bInterfaceSubClass 0x01 AUDIO_CONTROL and bInterfaceProtocol 0. The reader does not check
/// bAlternateSetting, bNumEndpoints and bInterfaceProtocol (U-24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StandardAcInterface {
    /// bInterfaceNumber (offset 2), from the USB stack. 8 bits.
    pub interface_number: u8,
    /// iInterface (offset 8): the string index, 0 for no string (the default). 8 bits.
    pub string_index: u8,
}

impl StandardAcInterface {
    /// bLength of the descriptor.
    pub const LEN: usize = 9;

    /// The descriptor of interface `interface_number`, with no string (REQ-USB-AC-002).
    pub const fn new(interface_number: u8) -> Self {
        StandardAcInterface {
            interface_number,
            string_index: 0,
        }
    }

    /// The same descriptor with the string index `string_index`.
    pub const fn with_string(mut self, string_index: u8) -> Self {
        self.string_index = string_index;
        self
    }

    /// The 9 bytes of the descriptor (REQ-USB-AC-002).
    pub const fn to_bytes(&self) -> [u8; 9] {
        [
            Self::LEN as u8,
            INTERFACE,
            self.interface_number,
            0x00, // bAlternateSetting
            0x00, // bNumEndpoints
            AUDIO,
            AUDIOCONTROL,
            0x00, // bInterfaceProtocol
            self.string_index,
        ]
    }

    /// Writes the 9 bytes into `out` and returns 9. If `out` is too small, returns
    /// [`AcError::Overflow`] and writes nothing.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, AcError> {
        let dst = out.get_mut(..Self::LEN).ok_or(AcError::Overflow)?;
        dst.copy_from_slice(&self.to_bytes());
        Ok(Self::LEN)
    }

    /// Reads a Standard AC Interface Descriptor from `desc` (the descriptor from its bLength
    /// on). Bytes after bLength are not read.
    pub fn read(desc: &[u8]) -> Result<Self, AcError> {
        let len = usize::from(*desc.first().ok_or(AcError::Walk(WalkError::Truncated))?);
        if len > desc.len() {
            return Err(AcError::Walk(WalkError::Truncated));
        }
        let d = &desc[..len];
        if len < Self::LEN || d[1] != INTERFACE || d[5] != AUDIO || d[6] != AUDIOCONTROL {
            return Err(AcError::NotAudioControl);
        }
        Ok(StandardAcInterface {
            interface_number: d[2],
            string_index: d[8],
        })
    }
}

/// True when `desc` is a standard interface descriptor of class AUDIO and subclass
/// AUDIO_CONTROL.
pub fn is_audio_control(desc: &[u8]) -> bool {
    StandardAcInterface::read(desc).is_ok()
}

/// The Class-specific AC Interface Header Descriptor to write (USB-MIDI v1.0 B.3.2 Table B-4,
/// USB-MIDI v2.0 5.1 item 4 and B.2.2; REQ-USB-AC-003, REQ-USB-AC-004).
///
/// `N` is the capacity of the list baInterfaceNr. The list has 1 to `N` interface numbers
/// (at most [`MAX_IN_COLLECTION`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AcHeader<const N: usize> {
    interfaces: [u8; N],
    count: usize,
}

impl<const N: usize> AcHeader<N> {
    /// A header with one MIDIStreaming interface, the default (REQ-USB-AC-003). Returns
    /// [`AcError::TooManyInterfaces`] when `N` is 0.
    pub fn single(ms_interface: u8) -> Result<Self, AcError> {
        Self::new(&[ms_interface])
    }

    /// A header with the interface numbers `interfaces`, in this order. Returns
    /// [`AcError::NoStreamingInterface`] for an empty list (REQ-USB-AC-003) and
    /// [`AcError::TooManyInterfaces`] for more than `N` or more than [`MAX_IN_COLLECTION`].
    pub fn new(interfaces: &[u8]) -> Result<Self, AcError> {
        if interfaces.is_empty() {
            return Err(AcError::NoStreamingInterface);
        }
        if interfaces.len() > N || interfaces.len() > MAX_IN_COLLECTION {
            return Err(AcError::TooManyInterfaces);
        }
        let mut list = [0u8; N];
        list[..interfaces.len()].copy_from_slice(interfaces);
        Ok(AcHeader {
            interfaces: list,
            count: interfaces.len(),
        })
    }

    /// baInterfaceNr(1) to baInterfaceNr(n).
    pub fn interfaces(&self) -> &[u8] {
        &self.interfaces[..self.count]
    }

    /// bLength: 8+n. It is also wTotalLength (REQ-USB-AC-004).
    pub fn len(&self) -> usize {
        8 + self.count
    }

    /// Never true: a header has at least 9 bytes.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Writes the 8+n bytes into `out` and returns 8+n. wTotalLength is equal to bLength: the
    /// header is the first and only class-specific AC descriptor (REQ-USB-AC-004). If `out` is
    /// too small, returns [`AcError::Overflow`] and writes nothing.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, AcError> {
        let len = self.len();
        if out.len() < len {
            return Err(AcError::Overflow);
        }
        let mut w = DescWriter::new(out);
        w.u8(len as u8)?;
        w.u8(CS_INTERFACE)?;
        w.u8(AC_HEADER)?;
        w.u16_le(BCD_ADC)?;
        w.u16_le(len as u16)?;
        w.u8(self.count as u8)?;
        w.bytes(self.interfaces())?;
        Ok(w.len())
    }
}

/// A received Class-specific AC Interface Header Descriptor: a view on the caller slice
/// (REQ-USB-AC-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcHeaderView<'a> {
    /// bLength.
    pub len: u8,
    /// bcdADC (offset 3), little-endian. The reader does not check it (U-24).
    pub bcd_adc: u16,
    /// wTotalLength (offset 5), little-endian. The reader does not check it (U-24).
    pub total_length: u16,
    /// baInterfaceNr(1) to baInterfaceNr(bInCollection), with no copy.
    pub interfaces: &'a [u8],
}

impl<'a> AcHeaderView<'a> {
    /// Reads a header from `desc` (the descriptor from its bLength on). Returns
    /// [`AcError::NotHeader`] for another descriptor and [`AcError::HeaderTooShort`] when
    /// bLength is less than 8+bInCollection (REQ-USB-AC-005).
    pub fn read(desc: &'a [u8]) -> Result<Self, AcError> {
        let len = *desc.first().ok_or(AcError::Walk(WalkError::Truncated))?;
        let d = desc
            .get(..usize::from(len))
            .ok_or(AcError::Walk(WalkError::Truncated))?;
        if d.get(1) != Some(&CS_INTERFACE) || d.get(2) != Some(&AC_HEADER) {
            return Err(AcError::NotHeader);
        }
        let too_short = |in_collection| AcError::HeaderTooShort { len, in_collection };
        let in_collection = *d.get(7).ok_or(too_short(0))?;
        let interfaces = d
            .get(8..8 + usize::from(in_collection))
            .ok_or(too_short(in_collection))?;
        Ok(AcHeaderView {
            len,
            bcd_adc: u16::from_le_bytes([d[3], d[4]]),
            total_length: u16::from_le_bytes([d[5], d[6]]),
            interfaces,
        })
    }
}

/// Writes the AudioControl interface of one USB MIDI function: the Standard AC Interface
/// Descriptor, then the AC Interface Header Descriptor, and nothing else (REQ-USB-AC-001,
/// REQ-USB-AC-004). Returns the number of bytes, 17+n. If `out` is too small, returns
/// [`AcError::Overflow`] and writes nothing.
///
/// Call it one time for each function, before the MIDIStreaming interface, for a USB-MIDI 1.0
/// function and for a function with only USB MIDI 2.0 Alternate Settings.
pub fn write_audio_control<const N: usize>(
    interface: &StandardAcInterface,
    header: &AcHeader<N>,
    out: &mut [u8],
) -> Result<usize, AcError> {
    let total = audio_control_len(header);
    if out.len() < total {
        return Err(AcError::Overflow);
    }
    let n = interface.write(out)?;
    let rest = out.get_mut(n..).ok_or(AcError::Overflow)?;
    Ok(n + header.write(rest)?)
}

/// The number of bytes that [`write_audio_control`] writes: 9 + 8 + n.
pub fn audio_control_len<const N: usize>(header: &AcHeader<N>) -> usize {
    StandardAcInterface::LEN + header.len()
}

/// One AudioControl interface of a configuration, as read (REQ-USB-AC-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcInterfaceRead<'a> {
    /// The Standard AC Interface Descriptor.
    pub interface: StandardAcInterface,
    /// The first AC Interface Header Descriptor that the reader could read, if any.
    pub header: Option<AcHeaderView<'a>>,
    /// The first problem that the reader found, if any (U-24): a header that is too short,
    /// a walk error, or [`AcError::MissingHeader`].
    pub problem: Option<AcError>,
}

impl<'a> AcInterfaceRead<'a> {
    /// Reads one interface group. Returns `None` when the group is not an AudioControl
    /// interface. The reader reads the first AC Interface Header Descriptor and skips each
    /// other descriptor by its bLength (REQ-USB-AC-005).
    pub fn from_group(group: &InterfaceGroup<'a>) -> Option<Self> {
        let interface = StandardAcInterface::read(group.interface.bytes()).ok()?;
        let mut header = None;
        let mut problem = None;
        for item in Descriptors::new(group.body) {
            match item {
                Ok(d) => {
                    if header.is_some()
                        || d.dtype() != CS_INTERFACE
                        || d.subtype() != Some(AC_HEADER)
                    {
                        continue;
                    }
                    match AcHeaderView::read(d.bytes()) {
                        Ok(h) => header = Some(h),
                        Err(e) => {
                            problem.get_or_insert(e);
                        }
                    }
                }
                Err(e) => {
                    problem.get_or_insert(AcError::from(e));
                }
            }
        }
        if header.is_none() {
            problem.get_or_insert(AcError::MissingHeader);
        }
        Some(AcInterfaceRead {
            interface,
            header,
            problem,
        })
    }
}

/// The AudioControl interfaces of a configuration descriptor set, in order (REQ-USB-AC-005).
/// A configuration without one gives no item; it is not an error.
pub fn audio_control_interfaces(config: &[u8]) -> AcInterfaces<'_> {
    AcInterfaces {
        groups: interfaces(config),
    }
}

/// Iterator of [`audio_control_interfaces`].
#[derive(Debug, Clone)]
pub struct AcInterfaces<'a> {
    groups: Interfaces<'a>,
}

impl<'a> AcInterfaces<'a> {
    /// The walk error that ended the iteration, if any.
    pub fn error(&self) -> Option<WalkError> {
        self.groups.error()
    }
}

impl<'a> Iterator for AcInterfaces<'a> {
    type Item = AcInterfaceRead<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.groups
            .by_ref()
            .find_map(|g| AcInterfaceRead::from_group(&g))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;

    /// USB-MIDI v1.0 Table B-3 (and v2.0 Table B-3).
    const TABLE_B3: [u8; 9] = [0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00];
    /// USB-MIDI v1.0 Table B-4 (and v2.0 Table B-4).
    const TABLE_B4: [u8; 9] = [0x09, 0x24, 0x01, 0x00, 0x01, 0x09, 0x00, 0x01, 0x01];
    /// A MIDIStreaming interface, Alternate Setting 0.
    const MS_ALT0: [u8; 9] = [9, 4, 1, 0, 2, 1, 3, 0, 0];

    #[test]
    fn table_b3_vector_write_and_read() {
        // REQ-USB-VEC1-001, REQ-USB-AC-002.
        let mut buf = [0xAAu8; 12];
        assert_eq!(StandardAcInterface::new(0).write(&mut buf), Ok(9));
        assert_eq!(buf[..9], TABLE_B3);
        assert_eq!(buf[9..], [0xAA; 3]);
        assert_eq!(
            StandardAcInterface::read(&TABLE_B3),
            Ok(StandardAcInterface {
                interface_number: 0,
                string_index: 0
            })
        );
    }

    #[test]
    fn table_b4_vector_write_and_read() {
        // REQ-USB-VEC1-002, REQ-USB-AC-003, REQ-USB-AC-004.
        let h = AcHeader::<4>::single(1).unwrap();
        let mut buf = [0u8; 9];
        assert_eq!(h.write(&mut buf), Ok(9));
        assert_eq!(buf, TABLE_B4);
        let v = AcHeaderView::read(&TABLE_B4).unwrap();
        assert_eq!(v.len, 9);
        assert_eq!(v.bcd_adc, 0x0100);
        assert_eq!(v.total_length, 0x0009);
        assert_eq!(v.interfaces, &[1]);
        assert_eq!(AcHeader::<4>::new(v.interfaces), Ok(h));
    }

    #[test]
    fn standard_interface_fields_from_caller() {
        // REQ-USB-AC-002: interface number and string index from the caller; fixed fields.
        let d = StandardAcInterface::new(3).with_string(5);
        assert_eq!(d.to_bytes(), [9, 0x04, 3, 0, 0, 0x01, 0x01, 0, 5]);
        assert_eq!(StandardAcInterface::read(&d.to_bytes()), Ok(d));
        assert_eq!(StandardAcInterface::new(7).string_index, 0);
    }

    #[test]
    fn standard_interface_reader_rejects_other_descriptors() {
        // REQ-USB-AC-002: only class AUDIO, subclass AUDIO_CONTROL, 9 bytes or more.
        assert_eq!(
            StandardAcInterface::read(&MS_ALT0),
            Err(AcError::NotAudioControl)
        );
        assert_eq!(
            StandardAcInterface::read(&[8, 4, 0, 0, 0, 1, 1, 0]),
            Err(AcError::NotAudioControl)
        );
        assert_eq!(
            StandardAcInterface::read(&TABLE_B4),
            Err(AcError::NotAudioControl)
        );
        assert_eq!(
            StandardAcInterface::read(&TABLE_B3[..8]),
            Err(AcError::Walk(WalkError::Truncated))
        );
        assert_eq!(
            StandardAcInterface::read(&[]),
            Err(AcError::Walk(WalkError::Truncated))
        );
        assert!(is_audio_control(&TABLE_B3));
        // Lenient (U-24): a Status Interrupt endpoint count is accepted.
        let mut with_ep = TABLE_B3;
        with_ep[4] = 1;
        assert!(is_audio_control(&with_ep));
    }

    #[test]
    fn header_with_several_interfaces() {
        // REQ-USB-AC-003: bLength 8+n, bInCollection n, list in caller order.
        let h = AcHeader::<4>::new(&[1, 2, 5]).unwrap();
        let mut buf = [0u8; 11];
        assert_eq!(h.write(&mut buf), Ok(11));
        assert_eq!(buf, [11, 0x24, 0x01, 0x00, 0x01, 11, 0x00, 3, 1, 2, 5]);
        assert_eq!(AcHeaderView::read(&buf).unwrap().interfaces, &[1, 2, 5]);
    }

    #[test]
    fn header_rejects_zero_and_too_many_interfaces() {
        // REQ-USB-AC-003: n = 0 is rejected; the list has a fixed capacity.
        assert_eq!(AcHeader::<4>::new(&[]), Err(AcError::NoStreamingInterface));
        assert_eq!(
            AcHeader::<2>::new(&[1, 2, 3]),
            Err(AcError::TooManyInterfaces)
        );
        assert_eq!(AcHeader::<0>::single(1), Err(AcError::TooManyInterfaces));
        let many = [1u8; 248];
        assert_eq!(AcHeader::<300>::new(&many), Err(AcError::TooManyInterfaces));
        let h = AcHeader::<300>::new(&many[..247]).unwrap();
        let mut buf = [0u8; 255];
        assert_eq!(h.write(&mut buf), Ok(255));
        assert_eq!(buf[0], 255);
    }

    #[test]
    fn total_length_equals_blength_and_no_other_descriptor() {
        // REQ-USB-AC-004: wTotalLength = bLength; the header is the only class-specific AC
        // descriptor; no Terminal or Unit descriptor.
        let list = [1u8, 2, 3, 4, 5, 6, 7, 8];
        for n in 1..=list.len() {
            let h = AcHeader::<8>::new(&list[..n]).unwrap();
            let mut buf = [0u8; 64];
            let len = write_audio_control(&StandardAcInterface::new(0), &h, &mut buf).unwrap();
            let descs: Vec<_> = Descriptors::new(&buf[..len]).map(|d| d.unwrap()).collect();
            assert_eq!(descs.len(), 2);
            let header = descs[1];
            assert_eq!(header.u16_le(5), Some(header.len() as u16));
            assert_eq!(header.len(), 8 + n);
        }
    }

    #[test]
    fn audio_control_interface_order_and_length() {
        // REQ-USB-AC-001: standard AC descriptor first, then the header, one time.
        let h = AcHeader::<1>::single(1).unwrap();
        let ac = StandardAcInterface::new(0);
        let mut buf = [0u8; 18];
        assert_eq!(audio_control_len(&h), 18);
        assert_eq!(write_audio_control(&ac, &h, &mut buf), Ok(18));
        assert_eq!(buf[..9], TABLE_B3);
        assert_eq!(buf[9..], TABLE_B4);
        // The same bytes for a function with only USB MIDI 2.0 Alternate Settings: the AC
        // interface does not depend on the Alternate Settings.
        let read: Vec<_> = audio_control_interfaces(&buf).collect();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].interface, ac);
        assert_eq!(read[0].header.unwrap().interfaces, &[1]);
        assert_eq!(read[0].problem, None);
    }

    #[test]
    fn writers_write_nothing_on_overflow() {
        // REQ-USB-AC-001, REQ-USB-AC-002, REQ-USB-AC-003: Overflow and nothing written.
        let h = AcHeader::<2>::new(&[1, 2]).unwrap();
        let ac = StandardAcInterface::new(0);
        let mut buf = [0xEEu8; 18];
        assert_eq!(
            write_audio_control(&ac, &h, &mut buf),
            Err(AcError::Overflow)
        );
        assert_eq!(buf, [0xEE; 18]);
        assert_eq!(h.write(&mut buf[..9]), Err(AcError::Overflow));
        assert_eq!(ac.write(&mut buf[..8]), Err(AcError::Overflow));
        assert_eq!(buf, [0xEE; 18]);
    }

    #[test]
    fn reader_accepts_configuration_without_audio_control() {
        // REQ-USB-AC-005: no AudioControl interface is not an error.
        let mut cfg = Vec::new();
        cfg.extend_from_slice(&[9, 2, 25, 0, 1, 1, 0, 0x80, 0x32]);
        cfg.extend_from_slice(&MS_ALT0);
        cfg.extend_from_slice(&[7, 0x24, 1, 0, 1, 7, 0]);
        let mut it = audio_control_interfaces(&cfg);
        assert_eq!(it.next(), None);
        assert_eq!(it.error(), None);
    }

    #[test]
    fn reader_skips_other_class_specific_descriptors() {
        // REQ-USB-AC-005: other class-specific AC descriptors and endpoints are skipped by
        // bLength; the header gives bcdADC and baInterfaceNr.
        let mut cfg = Vec::new();
        cfg.extend_from_slice(&TABLE_B3);
        cfg.extend_from_slice(&[12, 0x24, 0x02, 1, 1, 1, 0, 2, 3, 0, 0, 0]); // Input Terminal
        cfg.extend_from_slice(&[0x0A, 0x24, 0x01, 0x10, 0x01, 0x0A, 0x00, 2, 1, 2]);
        cfg.extend_from_slice(&[7, 0x05, 0x83, 0x03, 8, 0, 10]); // Status Interrupt endpoint
        cfg.extend_from_slice(&MS_ALT0);
        let read: Vec<_> = audio_control_interfaces(&cfg).collect();
        assert_eq!(read.len(), 1);
        let h = read[0].header.unwrap();
        assert_eq!(h.bcd_adc, 0x0110);
        assert_eq!(h.interfaces, &[1, 2]);
        assert_eq!(read[0].problem, None);
    }

    #[test]
    fn reader_reports_short_header_and_continues() {
        // REQ-USB-AC-005: bLength < 8+bInCollection gives an error for that descriptor; U-24:
        // the reader continues.
        let short = [9u8, 0x24, 0x01, 0x00, 0x01, 0x09, 0x00, 0x02, 0x01];
        assert_eq!(
            AcHeaderView::read(&short),
            Err(AcError::HeaderTooShort {
                len: 9,
                in_collection: 2
            })
        );
        assert_eq!(
            AcHeaderView::read(&[5, 0x24, 0x01, 0x00, 0x01]),
            Err(AcError::HeaderTooShort {
                len: 5,
                in_collection: 0
            })
        );
        assert_eq!(AcHeaderView::read(&TABLE_B3), Err(AcError::NotHeader));
        assert_eq!(
            AcHeaderView::read(&TABLE_B4[..8]),
            Err(AcError::Walk(WalkError::Truncated))
        );

        let mut cfg = Vec::new();
        cfg.extend_from_slice(&TABLE_B3);
        cfg.extend_from_slice(&short);
        cfg.extend_from_slice(&MS_ALT0);
        cfg.extend_from_slice(&[9, 4, 2, 0, 0, 1, 1, 0, 0]); // second AC interface, no header
        let read: Vec<_> = audio_control_interfaces(&cfg).collect();
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].header, None);
        assert_eq!(
            read[0].problem,
            Some(AcError::HeaderTooShort {
                len: 9,
                in_collection: 2
            })
        );
        assert_eq!(read[1].interface.interface_number, 2);
        assert_eq!(read[1].problem, Some(AcError::MissingHeader));
    }

    #[test]
    fn reader_reports_walk_error() {
        // REQ-USB-AC-005, U-24: a bad bLength after the header ends the walk; the header read
        // before it stays, and the iterator gives the walk error.
        let mut cfg = Vec::new();
        cfg.extend_from_slice(&TABLE_B3);
        cfg.extend_from_slice(&TABLE_B4);
        cfg.extend_from_slice(&[1, 0x24]);
        let read: Vec<_> = audio_control_interfaces(&cfg).collect();
        assert_eq!(read.len(), 1);
        assert!(read[0].header.is_some());
        assert_eq!(read[0].problem, None);
        let mut it = audio_control_interfaces(&cfg);
        it.by_ref().for_each(drop);
        assert_eq!(it.error(), Some(WalkError::LengthTooSmall));
    }

    #[test]
    fn prop_header_round_trip() {
        // REQ-USB-AC-003, REQ-USB-AC-004.
        proptest!(|(list in proptest::collection::vec(any::<u8>(), 1..=16), iface in any::<u8>(), s in any::<u8>())| {
            let h = AcHeader::<16>::new(&list).unwrap();
            let ac = StandardAcInterface::new(iface).with_string(s);
            let mut buf = [0u8; 64];
            let len = write_audio_control(&ac, &h, &mut buf).unwrap();
            prop_assert_eq!(len, 17 + list.len());
            let read: Vec<_> = audio_control_interfaces(&buf[..len]).collect();
            prop_assert_eq!(read.len(), 1);
            prop_assert_eq!(read[0].interface, ac);
            let v = read[0].header.unwrap();
            prop_assert_eq!(v.interfaces, &list[..]);
            prop_assert_eq!(usize::from(v.total_length), 8 + list.len());
            prop_assert_eq!(v.bcd_adc, BCD_ADC);
        });
    }

    #[test]
    fn prop_readers_never_panic() {
        // REQ-USB-AC-005, U-24: nothing panics on received bytes.
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..200))| {
            let _ = StandardAcInterface::read(&bytes);
            let _ = AcHeaderView::read(&bytes);
            for r in audio_control_interfaces(&bytes) {
                if let Some(h) = r.header {
                    prop_assert!(usize::from(h.len) >= 8 + h.interfaces.len());
                }
            }
        });
    }
}
