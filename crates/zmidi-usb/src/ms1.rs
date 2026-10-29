//! USB-MIDI 1.0 MIDI Streaming descriptors of Alternate Setting 0 and the Jack topology of one
//! USB-MIDI 1.0 function (USB-MIDI v1.0 sections 3, 4 and 6, Appendix B; the Appendix B vectors
//! are the same in USB-MIDI v2.0).
//!
//! - [`MsHeader`]: the 7-byte MS Interface Header, bcdMSC 0x0100 (REQ-USB-MS1-003). The writer
//!   puts in wTotalLength the header, all Jacks and all standard and class-specific endpoint
//!   descriptors of the setting (DEC-USB-001 choice U-10, REQ-USB-MS1-004): 0x0041 for
//!   Appendix B.
//! - [`MidiInJack`] (6 bytes) and [`MidiOutJack`] (7+2p bytes, the AMB-USB-MIDI-1-11 correction
//!   of Table 6-4) (REQ-USB-MS1-010, REQ-USB-MS1-011).
//! - [`Element`]: read only; the writer writes no Element (choice U-13, REQ-USB-TOPO-012).
//! - [`Ms1Endpoint`]: the 9-byte standard bulk endpoint descriptor and the class-specific
//!   MS_GENERAL descriptor of 4+n bytes directly after it (REQ-USB-MS1-018, REQ-USB-MS1-021).
//!   The writer writes no Transfer endpoint (choice U-14); the reader gives one as a
//!   [`TransferEndpoint`] (REQ-USB-TOPO-014).
//! - [`Ms1Topology`]: a fixed-capacity set of Jacks and endpoints with the writer checks
//!   (REQ-USB-TOPO-*) and the writer of the setting body (header, Jacks, endpoints;
//!   REQ-USB-MS1-002). The standard MIDI Streaming interface descriptor before it belongs to
//!   the Ms2 module. The cable-count form (choice U-26) is built by the Device module.
//! - [`CableMap`]: Cable Number n of an endpoint is the Jack at position n of its baAssocJackID
//!   list (REQ-USB-EVT-017).
//! - [`Ms1Descriptors`]: the lenient reader (choice U-24). It gives each descriptor that it
//!   reads, and reports each problem as an error item after it, then continues.
//!
//! Writers fill a caller buffer and return the number of bytes, or [`Ms1Error::Overflow`] with
//! nothing written. Readers never panic on received bytes. Multi-byte fields are little-endian
//! (USB 2.0 8.1, AMB-USB-MIDI-1-31). No alloc (DEC-002), no I/O and no USB stack (DEC-004,
//! DEC-014).

use core::num::NonZeroU8;

use crate::constants::{
    EndpointDirection, JackType, CS_ENDPOINT, CS_INTERFACE, ELEMENT, ENDPOINT, MIDI_IN_JACK,
    MIDI_OUT_JACK, MS_GENERAL, MS_HEADER, MS_MIDI_1_0,
};
use crate::walk::{DescWriter, Descriptors, RawDesc, WalkError};

/// bLength of the MS Interface Header (Table 6-2).
pub const HEADER_LEN: usize = 7;
/// bLength of the MIDI IN Jack descriptor (Table 6-3).
pub const IN_JACK_LEN: usize = 6;
/// bLength of the standard endpoint descriptor of Alternate Setting 0 (Table 6-6).
pub const ENDPOINT_LEN: usize = 9;
/// The largest number of Input Pins of a MIDI OUT Jack in the writer: 7 + 2 x 124 = 255
/// (REQ-USB-TOPO-007).
pub const MAX_OUT_JACK_PINS: usize = 124;
/// The largest number of Embedded Jacks of one MIDI endpoint (3.2.1, REQ-USB-TOPO-010).
pub const MAX_ENDPOINT_JACKS: usize = 16;
/// bmAttributes of a MIDI endpoint of Alternate Setting 0: bulk, no synchronization
/// (REQ-USB-MS1-018).
pub const BULK: u8 = 0x02;
/// bmAttributes D1..0 of an interrupt endpoint (USB 2.0 Table 9-13). The reader accepts it on
/// Alternate Setting 0 (REQ-USB-MS2-008); the writer never writes it.
pub const INTERRUPT: u8 = 0x03;
/// The smallest bLength of a standard endpoint descriptor that the reader accepts
/// (REQ-USB-MS2-011).
pub const MIN_ENDPOINT_LEN: usize = 7;

/// A topology rule that a function breaks (USB-MIDI v1.0 3.1 to 3.4, 6.1.2, 6.2.2).
///
/// The writer returns it as an error. The reader gives the reader-side ones as error items
/// and continues (choice U-24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TopologyError {
    /// An Entity ID of 0x00 (REQ-USB-TOPO-004).
    #[error("Entity ID 0x00 (REQ-USB-TOPO-004)")]
    ZeroId,
    /// Two Entities use this ID (REQ-USB-TOPO-004).
    #[error("Entity ID {0} used two times (REQ-USB-TOPO-004)")]
    DuplicateId(u8),
    /// A bJackType that is not EMBEDDED or EXTERNAL in the writer (REQ-USB-MS1-009).
    #[error("bJackType {0:#04x} is not EMBEDDED or EXTERNAL (REQ-USB-MS1-009)")]
    JackType(u8),
    /// A MIDI OUT Jack with no Input Pin in the writer (REQ-USB-TOPO-007).
    #[error("MIDI OUT Jack {0} has no Input Pin (REQ-USB-TOPO-007)")]
    NoInputPin(u8),
    /// A MIDI OUT Jack with more than 124 Input Pins (REQ-USB-TOPO-007).
    #[error("MIDI OUT Jack {0} has more than 124 Input Pins (REQ-USB-TOPO-007)")]
    TooManyInputPins(u8),
    /// A baSourceID that is not a MIDI IN Jack or an Element of the function
    /// (REQ-USB-TOPO-006).
    #[error("Jack {jack}: baSourceID {source_id} is not a MIDI IN Jack (REQ-USB-TOPO-006)")]
    BadSourceId {
        /// The MIDI OUT Jack.
        jack: u8,
        /// The baSourceID.
        source_id: u8,
    },
    /// A baSourcePin of 0 or above the number of Output Pins of the source (REQ-USB-TOPO-005).
    #[error(
        "Jack {jack}: baSourcePin {pin} is not an Output Pin of the source (REQ-USB-TOPO-005)"
    )]
    BadSourcePin {
        /// The MIDI OUT Jack.
        jack: u8,
        /// The baSourcePin.
        pin: u8,
    },
    /// A function with no MIDI endpoint (REQ-USB-TOPO-001).
    #[error("no MIDI endpoint (REQ-USB-TOPO-001)")]
    NoEndpoint,
    /// A MIDI endpoint (this endpoint number) with no Embedded Jack (REQ-USB-TOPO-001).
    #[error("MIDI endpoint {0} has no Embedded Jack (REQ-USB-TOPO-001)")]
    EndpointWithoutJack(u8),
    /// A MIDI endpoint (this endpoint number) with more than 16 Embedded Jacks
    /// (REQ-USB-TOPO-010).
    #[error("MIDI endpoint {0} has more than 16 Embedded Jacks (REQ-USB-TOPO-010)")]
    TooManyJacks(u8),
    /// A baAssocJackID that names no Embedded Jack of the correct direction: a MIDI OUT
    /// endpoint lists Embedded MIDI IN Jacks, a MIDI IN endpoint Embedded MIDI OUT Jacks
    /// (REQ-USB-MS1-022).
    #[error("endpoint {endpoint}: Jack {jack} is not an Embedded Jack of the correct direction (REQ-USB-MS1-022)")]
    WrongJack {
        /// The endpoint number.
        endpoint: u8,
        /// The baAssocJackID.
        jack: u8,
    },
    /// An Embedded Jack in two endpoint lists, or two times in one list (REQ-USB-MS1-022).
    #[error("Embedded Jack {0} is in more than one endpoint list (REQ-USB-MS1-022)")]
    JackInTwoEndpoints(u8),
    /// An Embedded Jack in no endpoint list (3.2.1, REQ-USB-MS1-022).
    #[error("Embedded Jack {0} is in no endpoint list (REQ-USB-MS1-022)")]
    JackWithoutEndpoint(u8),
    /// Two MIDI endpoints with the same direction and endpoint number.
    #[error("two MIDI endpoints use endpoint number {0} in one direction")]
    DuplicateEndpoint(u8),
    /// An Element with no Input Pin (reader, REQ-USB-TOPO-013).
    #[error("Element {0} has no Input Pin (REQ-USB-TOPO-013)")]
    ElementNoInputPin(u8),
    /// An Element with no Output Pin (reader, REQ-USB-TOPO-013).
    #[error("Element {0} has no Output Pin (REQ-USB-TOPO-013)")]
    ElementNoOutputPin(u8),
    /// The fixed capacity of the [`Ms1Topology`] is full.
    #[error("topology capacity full")]
    Full,
}

/// Error of the Ms1 writers and readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Ms1Error {
    /// The caller buffer is too small; nothing is written.
    #[error("buffer too small")]
    Overflow,
    /// A topology rule is broken.
    #[error(transparent)]
    Topology(#[from] TopologyError),
    /// An endpoint number of 0 or above 15 (REQ-USB-MS2-007).
    #[error("endpoint number {0} is not 1 to 15 (REQ-USB-MS2-007)")]
    EndpointNumber(u8),
    /// A wMaxPacketSize below 4 or not a multiple of 4 (REQ-USB-MS2-009).
    #[error("wMaxPacketSize {0} is below 4 or not a multiple of 4 (REQ-USB-MS2-009)")]
    MaxPacketSize(u16),
    /// The descriptors of the setting are longer than wTotalLength can give (REQ-USB-MS1-004).
    #[error("descriptors longer than 0xFFFF bytes (REQ-USB-MS1-004)")]
    TooLong,
    /// A descriptor shorter than its fixed fields (reader).
    #[error("descriptor type {dtype:#04x} subtype {subtype:#04x} too short")]
    TooShort {
        /// bDescriptorType.
        dtype: u8,
        /// bDescriptorSubtype (0 when the descriptor has none).
        subtype: u8,
    },
    /// A class-specific MS_GENERAL endpoint descriptor with bLength less than 4+n
    /// (REQ-USB-MS1-021).
    #[error("MS_GENERAL endpoint descriptor with bLength less than 4+n (REQ-USB-MS1-021)")]
    EndpointTooShort,
    /// A class-specific endpoint descriptor of a subtype other than MS_GENERAL after a standard
    /// endpoint descriptor of Alternate Setting 0.
    #[error("class-specific endpoint subtype {0:#04x} is not MS_GENERAL")]
    EndpointSubtype(u8),
    /// The walk by bLength stopped (REQ-USB-MS1-006).
    #[error("descriptor walk: {0}")]
    Walk(WalkError),
}

impl From<WalkError> for Ms1Error {
    fn from(e: WalkError) -> Self {
        match e {
            WalkError::Overflow => Ms1Error::Overflow,
            other => Ms1Error::Walk(other),
        }
    }
}

/// Gives `Overflow` when `out` has less than `len` bytes, so that a writer writes nothing.
fn need(out: &[u8], len: usize) -> Result<(), Ms1Error> {
    if out.len() < len {
        Err(Ms1Error::Overflow)
    } else {
        Ok(())
    }
}

fn string_byte(s: Option<NonZeroU8>) -> u8 {
    s.map_or(0, NonZeroU8::get)
}

fn check_jack_type(t: JackType) -> Result<(), TopologyError> {
    match t {
        JackType::Embedded | JackType::External => Ok(()),
        other => Err(TopologyError::JackType(other.into_bits())),
    }
}

fn too_short(d: &RawDesc<'_>) -> Ms1Error {
    Ms1Error::TooShort {
        dtype: d.dtype(),
        subtype: d.subtype().unwrap_or(0),
    }
}

/// The MS Interface Header Descriptor of Alternate Setting 0 (Table 6-2, Table B-6;
/// REQ-USB-MS1-003, REQ-USB-MS1-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MsHeader {
    /// bcdMSC (offset 3, 2 bytes). The writer always writes 0x0100.
    pub bcd_msc: u16,
    /// wTotalLength (offset 5, 2 bytes).
    pub total_length: u16,
}

impl MsHeader {
    /// A USB-MIDI 1.0 header with this wTotalLength.
    pub const fn new(total_length: u16) -> Self {
        MsHeader {
            bcd_msc: MS_MIDI_1_0,
            total_length,
        }
    }

    /// Writes the 7 bytes with bcdMSC 0x0100 (REQ-USB-MS1-003).
    pub fn write(&self, out: &mut [u8]) -> Result<usize, Ms1Error> {
        need(out, HEADER_LEN)?;
        let mut w = DescWriter::new(out);
        w.bytes(&[HEADER_LEN as u8, CS_INTERFACE, MS_HEADER])?;
        w.u16_le(MS_MIDI_1_0)?;
        w.u16_le(self.total_length)?;
        Ok(w.len())
    }

    /// Reads a header: CS_INTERFACE, MS_HEADER, bLength 7 or more. Any wTotalLength is accepted.
    pub fn read(d: &RawDesc<'_>) -> Result<Self, Ms1Error> {
        match (d.u16_le(3), d.u16_le(5)) {
            (Some(bcd_msc), Some(total_length)) => Ok(MsHeader {
                bcd_msc,
                total_length,
            }),
            _ => Err(too_short(d)),
        }
    }
}

/// A MIDI IN Jack (Table 6-3; REQ-USB-MS1-010). An Embedded MIDI IN Jack takes data from a
/// MIDI OUT endpoint; an External MIDI IN Jack is a physical input. It has one Output Pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MidiInJack {
    /// bJackType (offset 3).
    pub jack_type: JackType,
    /// bJackID (offset 4), 1 to 255.
    pub id: u8,
    /// iJack (offset 5): None is 0x00, no string (REQ-USB-MS1-012).
    pub string: Option<NonZeroU8>,
}

impl MidiInJack {
    /// A Jack with no string.
    pub const fn new(jack_type: JackType, id: u8) -> Self {
        MidiInJack {
            jack_type,
            id,
            string: None,
        }
    }

    /// Writes the 6 bytes. Refuses an ID of 0 and a type other than EMBEDDED or EXTERNAL.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, Ms1Error> {
        check_jack_type(self.jack_type)?;
        if self.id == 0 {
            return Err(TopologyError::ZeroId.into());
        }
        need(out, IN_JACK_LEN)?;
        let mut w = DescWriter::new(out);
        w.bytes(&[
            IN_JACK_LEN as u8,
            CS_INTERFACE,
            MIDI_IN_JACK,
            self.jack_type.into_bits(),
            self.id,
            string_byte(self.string),
        ])?;
        Ok(w.len())
    }

    /// Reads a MIDI IN Jack of bLength 6 or more. Any bJackType is given (REQ-USB-MS1-009).
    pub fn read(d: &RawDesc<'_>) -> Result<Self, Ms1Error> {
        match (d.byte(3), d.byte(4), d.byte(5)) {
            (Some(t), Some(id), Some(s)) => Ok(MidiInJack {
                jack_type: JackType::from_bits(t),
                id,
                string: NonZeroU8::new(s),
            }),
            _ => Err(too_short(d)),
        }
    }
}

/// One Input Pin connection: baSourceID and baSourcePin (3.1, REQ-USB-TOPO-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Pin {
    /// baSourceID: the Entity whose Output Pin connects to this Input Pin.
    pub source_id: u8,
    /// baSourcePin: the Output Pin of the source, from 1.
    pub source_pin: u8,
}

impl Pin {
    /// A connection to Output Pin `source_pin` of Entity `source_id`.
    pub const fn new(source_id: u8, source_pin: u8) -> Self {
        Pin {
            source_id,
            source_pin,
        }
    }
}

/// The Input Pins of a MIDI OUT Jack or an Element: a caller slice (writer) or the
/// descriptor bytes (reader). Input Pin i is at index i - 1.
#[derive(Debug, Clone, Copy)]
pub struct Pins<'a>(PinsRepr<'a>);

#[derive(Debug, Clone, Copy)]
enum PinsRepr<'a> {
    List(&'a [Pin]),
    /// baSourceID, baSourcePin pairs; an even number of bytes.
    Raw(&'a [u8]),
}

impl<'a> Pins<'a> {
    /// The pins of a caller slice.
    pub const fn new(pins: &'a [Pin]) -> Self {
        Pins(PinsRepr::List(pins))
    }

    fn raw(bytes: &'a [u8]) -> Self {
        Pins(PinsRepr::Raw(bytes))
    }

    /// The number of Input Pins.
    pub fn len(&self) -> usize {
        match self.0 {
            PinsRepr::List(l) => l.len(),
            PinsRepr::Raw(b) => b.len() / 2,
        }
    }

    /// True when there is no Input Pin.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The connection of the Input Pin at `index` (Input Pin `index` + 1).
    pub fn get(&self, index: usize) -> Option<Pin> {
        match self.0 {
            PinsRepr::List(l) => l.get(index).copied(),
            PinsRepr::Raw(b) => {
                let at = index.checked_mul(2)?;
                Some(Pin::new(*b.get(at)?, *b.get(at + 1)?))
            }
        }
    }

    /// The connections in Input Pin order.
    pub fn iter(&self) -> impl Iterator<Item = Pin> + 'a {
        let p = *self;
        (0..p.len()).filter_map(move |i| p.get(i))
    }
}

impl PartialEq for Pins<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().eq(other.iter())
    }
}

impl Eq for Pins<'_> {}

/// A MIDI OUT Jack with p Input Pins (Table 6-4 with the AMB-USB-MIDI-1-11 correction;
/// REQ-USB-MS1-011). An Embedded MIDI OUT Jack sends data to a MIDI IN endpoint; an External
/// MIDI OUT Jack is a physical output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiOutJack<'a> {
    /// bJackType (offset 3).
    pub jack_type: JackType,
    /// bJackID (offset 4), 1 to 255.
    pub id: u8,
    /// bNrInputPins (offset 5) and the baSourceID, baSourcePin pairs (offset 6 on).
    pub pins: Pins<'a>,
    /// iJack (offset 6+2p): None is 0x00, no string (REQ-USB-MS1-012).
    pub string: Option<NonZeroU8>,
}

impl<'a> MidiOutJack<'a> {
    /// A Jack with these Input Pins and no string.
    pub const fn new(jack_type: JackType, id: u8, pins: &'a [Pin]) -> Self {
        MidiOutJack {
            jack_type,
            id,
            pins: Pins::new(pins),
            string: None,
        }
    }

    /// bLength: 7 + 2p.
    pub fn len(&self) -> usize {
        7 + 2 * self.pins.len()
    }

    /// Never true: the descriptor has at least 7 bytes.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Writes the 7+2p bytes. Refuses an ID of 0, a type other than EMBEDDED or EXTERNAL,
    /// and p = 0 or p above 124 (REQ-USB-TOPO-007). The source checks need the whole function
    /// and are in [`Ms1Topology::check`].
    pub fn write(&self, out: &mut [u8]) -> Result<usize, Ms1Error> {
        check_jack_type(self.jack_type)?;
        if self.id == 0 {
            return Err(TopologyError::ZeroId.into());
        }
        let p = self.pins.len();
        if p == 0 {
            return Err(TopologyError::NoInputPin(self.id).into());
        }
        if p > MAX_OUT_JACK_PINS {
            return Err(TopologyError::TooManyInputPins(self.id).into());
        }
        let len = self.len();
        need(out, len)?;
        let mut w = DescWriter::new(out);
        w.bytes(&[
            len as u8,
            CS_INTERFACE,
            MIDI_OUT_JACK,
            self.jack_type.into_bits(),
            self.id,
            p as u8,
        ])?;
        for pin in self.pins.iter() {
            w.bytes(&[pin.source_id, pin.source_pin])?;
        }
        w.u8(string_byte(self.string))?;
        Ok(w.len())
    }

    /// Reads a MIDI OUT Jack. It accepts p = 0 (REQ-USB-TOPO-007), any bJackType
    /// (REQ-USB-MS1-009) and any source (REQ-USB-TOPO-006). bLength less than 7+2p is an
    /// error.
    pub fn read(d: &RawDesc<'a>) -> Result<Self, Ms1Error> {
        let b = d.bytes();
        let (Some(t), Some(id), Some(p)) = (d.byte(3), d.byte(4), d.byte(5)) else {
            return Err(too_short(d));
        };
        let end = 6 + 2 * usize::from(p);
        let (Some(pins), Some(s)) = (b.get(6..end), b.get(end)) else {
            return Err(too_short(d));
        };
        Ok(MidiOutJack {
            jack_type: JackType::from_bits(t),
            id,
            pins: Pins::raw(pins),
            string: NonZeroU8::new(*s),
        })
    }
}

/// A MIDI Jack of a topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jack<'a> {
    /// A MIDI IN Jack.
    In(MidiInJack),
    /// A MIDI OUT Jack.
    Out(MidiOutJack<'a>),
}

impl Jack<'_> {
    /// bJackID.
    pub fn id(&self) -> u8 {
        match self {
            Jack::In(j) => j.id,
            Jack::Out(j) => j.id,
        }
    }

    /// bJackType.
    pub fn jack_type(&self) -> JackType {
        match self {
            Jack::In(j) => j.jack_type,
            Jack::Out(j) => j.jack_type,
        }
    }

    /// True for an Embedded Jack.
    pub fn is_embedded(&self) -> bool {
        self.jack_type() == JackType::Embedded
    }

    /// bLength of its descriptor.
    pub fn len(&self) -> usize {
        match self {
            Jack::In(_) => IN_JACK_LEN,
            Jack::Out(j) => j.len(),
        }
    }

    /// Never true.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Writes its descriptor.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, Ms1Error> {
        match self {
            Jack::In(j) => j.write(out),
            Jack::Out(j) => j.write(out),
        }
    }
}

/// bmElementCaps bits D0 to D11 (6.1.2.4; REQ-USB-MS1-016). The reader ignores D12 and higher.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ElementCaps(u16);

impl ElementCaps {
    /// D0 CUSTOM UNDEFINED.
    pub const CUSTOM_UNDEFINED: u16 = 1 << 0;
    /// D1 MIDI CLOCK.
    pub const MIDI_CLOCK: u16 = 1 << 1;
    /// D2 MIDI TIME CODE (MTC).
    pub const MTC: u16 = 1 << 2;
    /// D3 MIDI MACHINE CONTROL (MMC).
    pub const MMC: u16 = 1 << 3;
    /// D4 GM1.
    pub const GM1: u16 = 1 << 4;
    /// D5 GM2.
    pub const GM2: u16 = 1 << 5;
    /// D6 GS.
    pub const GS: u16 = 1 << 6;
    /// D7 XG.
    pub const XG: u16 = 1 << 7;
    /// D8 EFX.
    pub const EFX: u16 = 1 << 8;
    /// D9 MIDI Patch Bay.
    pub const MIDI_PATCH_BAY: u16 = 1 << 9;
    /// D10 DLS1.
    pub const DLS1: u16 = 1 << 10;
    /// D11 DLS2.
    pub const DLS2: u16 = 1 << 11;
    /// All defined bits, D0 to D11.
    pub const DEFINED: u16 = 0x0FFF;

    /// The defined bits of `bits`; D12 and higher are dropped.
    pub const fn from_bits(bits: u16) -> Self {
        ElementCaps(bits & Self::DEFINED)
    }

    /// The bits D0 to D11.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// True when every bit of `mask` is set.
    pub const fn contains(self, mask: u16) -> bool {
        self.0 & mask == mask
    }
}

/// A MIDI Element, read only (Table 6-5; REQ-USB-MS1-013 to REQ-USB-MS1-016, choice U-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Element<'a> {
    /// bElementID (offset 3).
    pub id: u8,
    /// bNrInputPins p (offset 4) and the baSourceID, baSourcePin pairs (offset 5 on).
    pub pins: Pins<'a>,
    /// bNrOutputPins (offset 5+2p).
    pub output_pins: u8,
    /// bInTerminalLink (offset 6+2p): None is 0x00, no link (REQ-USB-MS1-014).
    pub in_terminal_link: Option<NonZeroU8>,
    /// bOutTerminalLink (offset 7+2p): None is 0x00, no link (REQ-USB-MS1-014).
    pub out_terminal_link: Option<NonZeroU8>,
    /// bElCapsSize n (offset 8+2p), as read.
    pub caps_size: u8,
    /// bmElementCaps (offset 9+2p), least significant byte first; absent bytes read as 0
    /// (REQ-USB-MS1-015).
    pub caps: ElementCaps,
    /// iElement (offset 9+2p+n): None is 0x00 or absent, no string (REQ-USB-MS1-012).
    pub string: Option<NonZeroU8>,
}

impl<'a> Element<'a> {
    /// Reads an Element. The fields up to bElCapsSize must be present; the capability bytes and
    /// iElement are read from the bytes that are present (REQ-USB-MS1-015).
    pub fn read(d: &RawDesc<'a>) -> Result<Self, Ms1Error> {
        let b = d.bytes();
        let (Some(id), Some(p)) = (d.byte(3), d.byte(4)) else {
            return Err(too_short(d));
        };
        let q_at = 5 + 2 * usize::from(p);
        let (Some(pins), Some(fixed)) = (b.get(5..q_at), b.get(q_at..q_at + 4)) else {
            return Err(too_short(d));
        };
        let n = usize::from(fixed[3]);
        let caps_at = q_at + 4;
        let lo = b.get(caps_at).copied().filter(|_| n >= 1).unwrap_or(0);
        let hi = b.get(caps_at + 1).copied().filter(|_| n >= 2).unwrap_or(0);
        Ok(Element {
            id,
            pins: Pins::raw(pins),
            output_pins: fixed[0],
            in_terminal_link: NonZeroU8::new(fixed[1]),
            out_terminal_link: NonZeroU8::new(fixed[2]),
            caps_size: fixed[3],
            caps: ElementCaps::from_bits(u16::from_le_bytes([lo, hi])),
            string: b.get(caps_at + n).copied().and_then(NonZeroU8::new),
        })
    }
}

/// A MIDI endpoint of Alternate Setting 0: the 9-byte standard bulk endpoint descriptor and the
/// class-specific MS_GENERAL descriptor directly after it (Tables 6-6 and 6-7;
/// REQ-USB-MS1-018, REQ-USB-MS1-021).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ms1Endpoint<'a> {
    /// bEndpointAddress D7.
    pub direction: EndpointDirection,
    /// bEndpointAddress D3..0, 1 to 15.
    pub number: u8,
    /// wMaxPacketSize. The reader gives the value of the descriptor rounded down to a multiple
    /// of 4, and an error item after the endpoint when the result is below 4
    /// (REQ-USB-MS2-009).
    pub max_packet_size: u16,
    /// bmAttributes D1..0, the transfer type: [`BULK`] or [`INTERRUPT`] (REQ-USB-MS2-008). The
    /// reader gives the value of the descriptor; the writer always writes [`BULK`] (Table 6-6)
    /// and ignores this field.
    pub transfer: u8,
    /// bInterval of an interrupt endpoint; 0 for any other endpoint (REQ-USB-MS2-008). The
    /// writer always writes 0 and ignores this field.
    pub interval: u8,
    /// baAssocJackID: the Embedded Jacks in Cable Number order (REQ-USB-EVT-017). The reader
    /// gives at most the first 16 (REQ-USB-TOPO-010).
    pub jacks: &'a [u8],
}

impl<'a> Ms1Endpoint<'a> {
    /// A bulk endpoint with these Embedded Jacks.
    pub const fn new(
        direction: EndpointDirection,
        number: u8,
        max_packet_size: u16,
        jacks: &'a [u8],
    ) -> Self {
        Ms1Endpoint {
            direction,
            number,
            max_packet_size,
            transfer: BULK,
            interval: 0,
            jacks,
        }
    }

    /// bEndpointAddress: D7 direction, D3..0 number.
    pub const fn address(&self) -> u8 {
        self.direction.address(self.number)
    }

    /// True for an interrupt endpoint (bmAttributes D1..0 = 3).
    pub const fn is_interrupt(&self) -> bool {
        self.transfer & 0x03 == INTERRUPT
    }

    /// The bytes of the two descriptors: 9 + 4 + n.
    pub fn len(&self) -> usize {
        ENDPOINT_LEN + 4 + self.jacks.len()
    }

    /// Never true.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The cable map of this endpoint (REQ-USB-EVT-017).
    pub fn cable_map(&self) -> CableMap<'a> {
        CableMap::new(self.jacks)
    }

    fn check(&self) -> Result<(), Ms1Error> {
        if self.number == 0 || self.number > 15 {
            return Err(Ms1Error::EndpointNumber(self.number));
        }
        if self.max_packet_size < 4 || !self.max_packet_size.is_multiple_of(4) {
            return Err(Ms1Error::MaxPacketSize(self.max_packet_size));
        }
        if self.jacks.is_empty() {
            return Err(TopologyError::EndpointWithoutJack(self.number).into());
        }
        if self.jacks.len() > MAX_ENDPOINT_JACKS {
            return Err(TopologyError::TooManyJacks(self.number).into());
        }
        Ok(())
    }

    /// Writes the standard endpoint descriptor (9 bytes: bulk, bInterval, bRefresh and
    /// bSynchAddress 0) and the MS_GENERAL descriptor (4+n bytes). Refuses an endpoint number
    /// outside 1 to 15, a bad wMaxPacketSize and n outside 1 to 16.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, Ms1Error> {
        self.check()?;
        need(out, self.len())?;
        let mut w = DescWriter::new(out);
        w.bytes(&[
            ENDPOINT_LEN as u8,
            ENDPOINT,
            self.direction.address(self.number),
            BULK,
        ])?;
        w.u16_le(self.max_packet_size)?;
        w.bytes(&[0, 0, 0])?;
        w.bytes(&[
            4 + self.jacks.len() as u8,
            CS_ENDPOINT,
            MS_GENERAL,
            self.jacks.len() as u8,
        ])?;
        w.bytes(self.jacks)?;
        Ok(w.len())
    }
}

/// A Transfer bulk endpoint that the reader found: a standard endpoint descriptor with no
/// class-specific descriptor directly after it (6.2.3, 6.2.4; REQ-USB-TOPO-014). The writer
/// never writes one (choice U-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransferEndpoint {
    /// Transfer IN (D7 = 1, `MidiIn`) or Transfer OUT (D7 = 0, `MidiOut`).
    pub direction: EndpointDirection,
    /// bEndpointAddress D3..0.
    pub number: u8,
    /// wMaxPacketSize, as written in the descriptor (the endpoint carries no MIDI data, so
    /// REQ-USB-MS2-009 does not apply).
    pub max_packet_size: u16,
    /// bmAttributes D1..0, the transfer type.
    pub transfer: u8,
    /// bInterval of an interrupt endpoint; 0 for any other endpoint.
    pub interval: u8,
}

/// The Cable Numbers of one MIDI endpoint: Cable Number n is the Jack at position n of
/// baAssocJackID (3.2.1, 5.1; REQ-USB-EVT-017, AMB-USB-MIDI-1-08).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CableMap<'a> {
    jacks: &'a [u8],
}

impl<'a> CableMap<'a> {
    /// The map of a baAssocJackID list; only the first 16 Jacks get a Cable Number.
    pub fn new(jacks: &'a [u8]) -> Self {
        CableMap {
            jacks: jacks.get(..MAX_ENDPOINT_JACKS).unwrap_or(jacks),
        }
    }

    /// The number of cables: bNumEmbMIDIJack.
    pub fn cables(&self) -> u8 {
        self.jacks.len() as u8
    }

    /// The Jack ID of Cable Number `cable`.
    pub fn jack(&self, cable: u8) -> Option<u8> {
        self.jacks.get(usize::from(cable)).copied()
    }

    /// The Cable Number of Jack `jack_id`.
    pub fn cable(&self, jack_id: u8) -> Option<u8> {
        self.jacks
            .iter()
            .position(|&j| j == jack_id)
            .map(|p| p as u8)
    }
}

/// Where the device side sends the data of an Embedded MIDI OUT Jack (REQ-USB-EVT-017).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CableRoute {
    /// The number of the MIDI IN endpoint that lists the Jack.
    pub endpoint: u8,
    /// The Cable Number: the position of the Jack in that list.
    pub cable: u8,
}

/// A 256-bit set of IDs.
#[derive(Debug, Clone, Copy, Default)]
struct IdSet([u32; 8]);

impl IdSet {
    /// Adds `id`; false when it was already there.
    fn insert(&mut self, id: u8) -> bool {
        let (w, b) = (usize::from(id >> 5), 1u32 << (id & 31));
        let fresh = self.0[w] & b == 0;
        self.0[w] |= b;
        fresh
    }

    fn contains(&self, id: u8) -> bool {
        self.0[usize::from(id >> 5)] & (1u32 << (id & 31)) != 0
    }
}

/// The Jacks and MIDI endpoints of Alternate Setting 0 of one function, with fixed capacities:
/// `J` Jacks and `E` endpoints (DEC-002). It holds the writer checks (REQ-USB-TOPO-*) and
/// writes the setting body after the standard MIDI Streaming interface descriptor: the header,
/// the Jacks and the endpoints (REQ-USB-MS1-002).
#[derive(Debug, Clone)]
pub struct Ms1Topology<'a, const J: usize, const E: usize = 2> {
    jacks: [Option<Jack<'a>>; J],
    n_jacks: usize,
    endpoints: [Option<Ms1Endpoint<'a>>; E],
    n_endpoints: usize,
}

impl<'a, const J: usize, const E: usize> Default for Ms1Topology<'a, J, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, const J: usize, const E: usize> Ms1Topology<'a, J, E> {
    /// An empty topology.
    pub const fn new() -> Self {
        Ms1Topology {
            jacks: [None; J],
            n_jacks: 0,
            endpoints: [None; E],
            n_endpoints: 0,
        }
    }

    /// Adds a Jack. The Jacks are written in this order, except that the Embedded Jacks of
    /// each direction take the order of the baAssocJackID lists (REQ-USB-MS1-023).
    pub fn add_jack(&mut self, jack: Jack<'a>) -> Result<(), TopologyError> {
        let slot = self
            .jacks
            .get_mut(self.n_jacks)
            .ok_or(TopologyError::Full)?;
        *slot = Some(jack);
        self.n_jacks += 1;
        Ok(())
    }

    /// Adds a MIDI endpoint. The endpoints are written in this order.
    pub fn add_endpoint(&mut self, ep: Ms1Endpoint<'a>) -> Result<(), TopologyError> {
        let slot = self
            .endpoints
            .get_mut(self.n_endpoints)
            .ok_or(TopologyError::Full)?;
        *slot = Some(ep);
        self.n_endpoints += 1;
        Ok(())
    }

    /// The Jacks in the order they were added.
    pub fn jacks(&self) -> impl Iterator<Item = &Jack<'a>> + '_ {
        self.jacks.iter().flatten()
    }

    /// The MIDI endpoints in the order they were added.
    pub fn endpoints(&self) -> impl Iterator<Item = &Ms1Endpoint<'a>> + '_ {
        self.endpoints.iter().flatten()
    }

    /// The number of MIDI endpoints: bNumEndpoints of Alternate Setting 0 (no Transfer
    /// endpoint, choice U-14).
    pub fn num_endpoints(&self) -> u8 {
        self.n_endpoints as u8
    }

    /// The Jack with this ID.
    pub fn jack(&self, id: u8) -> Option<&Jack<'a>> {
        self.jacks().find(|j| j.id() == id)
    }

    /// The bytes that [`Self::write`] writes, which is also wTotalLength (choice U-10,
    /// REQ-USB-MS1-004): the header, all Jacks and all endpoint descriptors.
    pub fn len(&self) -> usize {
        HEADER_LEN
            + self.jacks().map(Jack::len).sum::<usize>()
            + self.endpoints().map(Ms1Endpoint::len).sum::<usize>()
    }

    /// Never true: the header is always written.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The Cable Number route of an Embedded MIDI OUT Jack: the MIDI IN endpoint whose list
    /// has it and its position there (REQ-USB-EVT-017). The device side sends the data of the
    /// Jack only there.
    pub fn send_route(&self, jack_id: u8) -> Option<CableRoute> {
        self.route(EndpointDirection::MidiIn, jack_id)
    }

    /// The Cable Number route of an Embedded MIDI IN Jack on its MIDI OUT endpoint.
    pub fn receive_route(&self, jack_id: u8) -> Option<CableRoute> {
        self.route(EndpointDirection::MidiOut, jack_id)
    }

    fn route(&self, dir: EndpointDirection, jack_id: u8) -> Option<CableRoute> {
        self.endpoints()
            .filter(|e| e.direction == dir)
            .find_map(|e| {
                e.cable_map().cable(jack_id).map(|cable| CableRoute {
                    endpoint: e.number,
                    cable,
                })
            })
    }

    /// The Embedded Jack that a baAssocJackID of an endpoint in `dir` must name.
    fn is_embedded_for(&self, dir: EndpointDirection, id: u8) -> bool {
        matches!(
            (dir, self.jack(id)),
            (EndpointDirection::MidiOut, Some(j @ Jack::In(_)))
                | (EndpointDirection::MidiIn, Some(j @ Jack::Out(_))) if j.is_embedded()
        )
    }

    /// Checks every writer rule of the topology (REQ-USB-TOPO-001, -004 to -008, -010,
    /// REQ-USB-MS1-009, REQ-USB-MS1-022) and the length (REQ-USB-MS1-004).
    pub fn check(&self) -> Result<(), Ms1Error> {
        let mut ids = IdSet::default();
        for j in self.jacks() {
            check_jack_type(j.jack_type())?;
            let id = j.id();
            if id == 0 {
                return Err(TopologyError::ZeroId.into());
            }
            if !ids.insert(id) {
                return Err(TopologyError::DuplicateId(id).into());
            }
            if let Jack::Out(o) = j {
                if o.pins.is_empty() {
                    return Err(TopologyError::NoInputPin(id).into());
                }
                if o.pins.len() > MAX_OUT_JACK_PINS {
                    return Err(TopologyError::TooManyInputPins(id).into());
                }
                for pin in o.pins.iter() {
                    // Only a MIDI IN Jack can be a source: the writer writes no Element
                    // (choice U-13), and a MIDI OUT Jack has no Output Pin.
                    if !matches!(self.jack(pin.source_id), Some(Jack::In(_))) {
                        return Err(TopologyError::BadSourceId {
                            jack: id,
                            source_id: pin.source_id,
                        }
                        .into());
                    }
                    // A MIDI IN Jack has one Output Pin, number 1 (REQ-USB-TOPO-005).
                    if pin.source_pin != 1 {
                        return Err(TopologyError::BadSourcePin {
                            jack: id,
                            pin: pin.source_pin,
                        }
                        .into());
                    }
                }
            }
        }
        if self.n_endpoints == 0 {
            return Err(TopologyError::NoEndpoint.into());
        }
        let mut listed = IdSet::default();
        let mut addresses = IdSet::default();
        for e in self.endpoints() {
            e.check()?;
            if !addresses.insert(e.direction.address(e.number)) {
                return Err(TopologyError::DuplicateEndpoint(e.number).into());
            }
            for &id in e.jacks {
                if !self.is_embedded_for(e.direction, id) {
                    return Err(TopologyError::WrongJack {
                        endpoint: e.number,
                        jack: id,
                    }
                    .into());
                }
                if !listed.insert(id) {
                    return Err(TopologyError::JackInTwoEndpoints(id).into());
                }
            }
        }
        if let Some(j) = self
            .jacks()
            .find(|j| j.is_embedded() && !listed.contains(j.id()))
        {
            return Err(TopologyError::JackWithoutEndpoint(j.id()).into());
        }
        if self.len() > usize::from(u16::MAX) {
            return Err(Ms1Error::TooLong);
        }
        Ok(())
    }

    /// The `k`-th Jack ID of the concatenated baAssocJackID lists of the endpoints in `dir`.
    fn nth_assoc(&self, dir: EndpointDirection, k: usize) -> Option<u8> {
        self.endpoints()
            .filter(|e| e.direction == dir)
            .flat_map(|e| e.jacks.iter().copied())
            .nth(k)
    }

    /// The Jacks in the order that [`Self::write`] writes them: the order in which they were
    /// added, but the slots of the Embedded Jacks of one direction take those Jacks in
    /// baAssocJackID order (REQ-USB-MS1-023). A slot with no Jack (only in a topology that
    /// [`Self::check`] refuses) gives [`TopologyError::JackWithoutEndpoint`].
    pub fn jacks_in_write_order(&self) -> impl Iterator<Item = Result<&Jack<'a>, Ms1Error>> + '_ {
        self.jacks()
            .scan((0usize, 0usize), move |(k_in, k_out), j| {
                let jack = match j {
                    Jack::In(_) if j.is_embedded() => {
                        *k_in += 1;
                        self.nth_assoc(EndpointDirection::MidiOut, *k_in - 1)
                            .and_then(|id| self.jack(id))
                    }
                    Jack::Out(_) if j.is_embedded() => {
                        *k_out += 1;
                        self.nth_assoc(EndpointDirection::MidiIn, *k_out - 1)
                            .and_then(|id| self.jack(id))
                    }
                    _ => Some(j),
                };
                // The check makes the lists and the Embedded Jacks match one to one.
                Some(jack.ok_or(TopologyError::JackWithoutEndpoint(j.id()).into()))
            })
    }

    /// Checks the topology, then writes the MS Interface Header (wTotalLength per choice U-10),
    /// one descriptor for each Jack (REQ-USB-TOPO-008), and each endpoint as its standard
    /// descriptor directly followed by its MS_GENERAL descriptor (REQ-USB-MS1-002). No Element
    /// and no Transfer endpoint is written (choices U-13, U-14).
    ///
    /// The Jacks keep the order in which they were added, but the slots of the Embedded Jacks
    /// of one direction take those Jacks in baAssocJackID order, so the n-th Jack ID of the
    /// lists is the n-th Embedded Jack of that direction (REQ-USB-MS1-023).
    pub fn write(&self, out: &mut [u8]) -> Result<usize, Ms1Error> {
        self.check()?;
        let total = self.len();
        need(out, total)?;
        let mut at = MsHeader::new(total as u16).write(out)?;
        for jack in self.jacks_in_write_order() {
            let jack = jack?;
            let dst = out.get_mut(at..).ok_or(Ms1Error::Overflow)?;
            at += jack.write(dst)?;
        }
        for e in self.endpoints() {
            let dst = out.get_mut(at..).ok_or(Ms1Error::Overflow)?;
            at += e.write(dst)?;
        }
        Ok(at)
    }
}

/// One descriptor of Alternate Setting 0 that [`Ms1Descriptors`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ms1Desc<'a> {
    /// The MS Interface Header.
    Header(MsHeader),
    /// A MIDI IN Jack.
    InJack(MidiInJack),
    /// A MIDI OUT Jack.
    OutJack(MidiOutJack<'a>),
    /// A MIDI Element.
    Element(Element<'a>),
    /// A MIDI endpoint: a standard endpoint with its MS_GENERAL descriptor.
    Endpoint(Ms1Endpoint<'a>),
    /// A Transfer bulk endpoint (REQ-USB-TOPO-014).
    Transfer(TransferEndpoint),
    /// Any other descriptor, given as it is.
    Other(RawDesc<'a>),
}

/// Up to 3 problems of one descriptor, given after it.
#[derive(Debug, Clone, Copy, Default)]
struct Pending {
    items: [Option<Ms1Error>; 3],
}

impl Pending {
    fn push(&mut self, e: Ms1Error) {
        if let Some(slot) = self.items.iter_mut().find(|s| s.is_none()) {
            *slot = Some(e);
        }
    }

    fn pop(&mut self) -> Option<Ms1Error> {
        let first = self.items[0].take()?;
        self.items.rotate_left(1);
        Some(first)
    }
}

/// The lenient reader of the descriptors of Alternate Setting 0 (choice U-24): the body after
/// the standard MIDI Streaming interface descriptor (for example [`crate::walk::InterfaceGroup`]
/// `body`).
///
/// Each item is a descriptor, or a problem as an error after the descriptor it belongs to; the
/// reader then continues. It reports an Entity ID of 0x00 or used two times (REQ-USB-TOPO-004),
/// an Element with no Input or Output Pin (REQ-USB-TOPO-013) and a list of more than 16 Jacks
/// (REQ-USB-TOPO-010, the endpoint keeps the first 16). For each endpoint it gives the
/// transfer type and the bInterval of an interrupt endpoint (REQ-USB-MS2-008), a
/// wMaxPacketSize rounded down to a multiple of 4 with an error item below 4
/// (REQ-USB-MS2-009), and an error in place of an endpoint with a bLength below 7
/// (REQ-USB-MS2-011; its class-specific descriptor is skipped). It accepts any jack type
/// (REQ-USB-MS1-009), any source (REQ-USB-TOPO-006) and any baAssocJackID (REQ-USB-MS1-022).
/// A walk error ends the iteration.
#[derive(Debug, Clone)]
pub struct Ms1Descriptors<'a> {
    walk: Descriptors<'a>,
    ids: IdSet,
    pending: Pending,
}

impl<'a> Ms1Descriptors<'a> {
    /// A reader over `body`.
    pub fn new(body: &'a [u8]) -> Self {
        Ms1Descriptors {
            walk: Descriptors::new(body),
            ids: IdSet::default(),
            pending: Pending::default(),
        }
    }

    fn entity_id(&mut self, id: u8) {
        if id == 0 {
            self.pending.push(TopologyError::ZeroId.into());
        } else if !self.ids.insert(id) {
            self.pending.push(TopologyError::DuplicateId(id).into());
        }
    }

    fn endpoint(&mut self, d: &RawDesc<'a>) -> Result<Ms1Desc<'a>, Ms1Error> {
        let mut look = self.walk.clone();
        let cs = match look.next() {
            Some(Ok(cs)) if cs.dtype() == CS_ENDPOINT => Some(cs),
            _ => None,
        };
        let fields = (d.byte(2), d.byte(3), d.u16_le(4), d.byte(6));
        let (Some(address), Some(attributes), Some(raw_packet), Some(raw_interval)) = fields else {
            // A bLength below 7 (REQ-USB-MS2-011): the endpoint and its class-specific
            // descriptor are not used.
            if cs.is_some() {
                self.walk = look;
            }
            return Err(too_short(d));
        };
        let direction = EndpointDirection::from_address(address);
        let number = address & 0x0F;
        let transfer = attributes & 0x03;
        // bInterval only for an interrupt endpoint (REQ-USB-MS2-008).
        let interval = if transfer == INTERRUPT {
            raw_interval
        } else {
            0
        };
        let Some(cs) = cs else {
            return Ok(Ms1Desc::Transfer(TransferEndpoint {
                direction,
                number,
                max_packet_size: raw_packet,
                transfer,
                interval,
            }));
        };
        self.walk = look;
        match cs.subtype() {
            Some(MS_GENERAL) => {}
            Some(other) => return Err(Ms1Error::EndpointSubtype(other)),
            None => return Err(Ms1Error::EndpointTooShort),
        }
        let n = usize::from(cs.byte(3).ok_or(Ms1Error::EndpointTooShort)?);
        let all = cs.bytes().get(4..4 + n).ok_or(Ms1Error::EndpointTooShort)?;
        if n > MAX_ENDPOINT_JACKS {
            self.pending
                .push(TopologyError::TooManyJacks(number).into());
        }
        // The usable packet size: rounded down to a multiple of 4, an error below 4
        // (REQ-USB-MS2-009).
        let max_packet_size = raw_packet & !3;
        if max_packet_size < 4 {
            self.pending.push(Ms1Error::MaxPacketSize(raw_packet));
        }
        Ok(Ms1Desc::Endpoint(Ms1Endpoint {
            direction,
            number,
            max_packet_size,
            transfer,
            interval,
            jacks: all.get(..MAX_ENDPOINT_JACKS).unwrap_or(all),
        }))
    }

    fn read(&mut self, d: RawDesc<'a>) -> Result<Ms1Desc<'a>, Ms1Error> {
        match (d.dtype(), d.subtype()) {
            (CS_INTERFACE, Some(MS_HEADER)) => MsHeader::read(&d).map(Ms1Desc::Header),
            (CS_INTERFACE, Some(MIDI_IN_JACK)) => {
                let j = MidiInJack::read(&d)?;
                self.entity_id(j.id);
                Ok(Ms1Desc::InJack(j))
            }
            (CS_INTERFACE, Some(MIDI_OUT_JACK)) => {
                let j = MidiOutJack::read(&d)?;
                self.entity_id(j.id);
                Ok(Ms1Desc::OutJack(j))
            }
            (CS_INTERFACE, Some(ELEMENT)) => {
                let e = Element::read(&d)?;
                self.entity_id(e.id);
                if e.pins.is_empty() {
                    self.pending
                        .push(TopologyError::ElementNoInputPin(e.id).into());
                }
                if e.output_pins == 0 {
                    self.pending
                        .push(TopologyError::ElementNoOutputPin(e.id).into());
                }
                Ok(Ms1Desc::Element(e))
            }
            (ENDPOINT, _) => self.endpoint(&d),
            _ => Ok(Ms1Desc::Other(d)),
        }
    }
}

impl<'a> Iterator for Ms1Descriptors<'a> {
    type Item = Result<Ms1Desc<'a>, Ms1Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(e) = self.pending.pop() {
            return Some(Err(e));
        }
        match self.walk.next()? {
            Ok(d) => Some(self.read(d)),
            Err(e) => Some(Err(Ms1Error::Walk(e))),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;

    const EMB: JackType = JackType::Embedded;
    const EXT: JackType = JackType::External;

    /// Tables B-6 to B-14: Alternate Setting 0 of the Appendix B function after the standard
    /// interface descriptor (65 bytes).
    const APPENDIX_B: [u8; 65] = [
        0x07, 0x24, 0x01, 0x00, 0x01, 0x41, 0x00, // B-6 header
        0x06, 0x24, 0x02, 0x01, 0x01, 0x00, // B-7 Embedded MIDI IN Jack 1
        0x06, 0x24, 0x02, 0x02, 0x02, 0x00, // B-8 External MIDI IN Jack 2
        0x09, 0x24, 0x03, 0x01, 0x03, 0x01, 0x02, 0x01, 0x00, // B-9 Embedded MIDI OUT Jack 3
        0x09, 0x24, 0x03, 0x02, 0x04, 0x01, 0x01, 0x01, 0x00, // B-10 External MIDI OUT Jack 4
        0x09, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00, // B-11 OUT endpoint 1
        0x05, 0x25, 0x01, 0x01, 0x01, // B-12 MS_GENERAL, Jack 1
        0x09, 0x05, 0x81, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00, // B-13 IN endpoint 1
        0x05, 0x25, 0x01, 0x01, 0x03, // B-14 MS_GENERAL, Jack 3
    ];

    const PIN_2: [Pin; 1] = [Pin::new(2, 1)];
    const PIN_1: [Pin; 1] = [Pin::new(1, 1)];
    const OUT_JACKS: [u8; 1] = [1];
    const IN_JACKS: [u8; 1] = [3];

    /// The Appendix B topology.
    fn appendix_b() -> Ms1Topology<'static, 4> {
        let mut t = Ms1Topology::new();
        t.add_jack(Jack::In(MidiInJack::new(EMB, 1))).unwrap();
        t.add_jack(Jack::In(MidiInJack::new(EXT, 2))).unwrap();
        t.add_jack(Jack::Out(MidiOutJack::new(EMB, 3, &PIN_2)))
            .unwrap();
        t.add_jack(Jack::Out(MidiOutJack::new(EXT, 4, &PIN_1)))
            .unwrap();
        t.add_endpoint(Ms1Endpoint::new(
            EndpointDirection::MidiOut,
            1,
            64,
            &OUT_JACKS,
        ))
        .unwrap();
        t.add_endpoint(Ms1Endpoint::new(
            EndpointDirection::MidiIn,
            1,
            64,
            &IN_JACKS,
        ))
        .unwrap();
        t
    }

    fn write_vec<const J: usize, const E: usize>(t: &Ms1Topology<'_, J, E>) -> Vec<u8> {
        let mut buf = [0u8; 1024];
        let n = t.write(&mut buf).unwrap();
        buf[..n].to_vec()
    }

    fn read_all(bytes: &[u8]) -> Vec<Result<Ms1Desc<'_>, Ms1Error>> {
        Ms1Descriptors::new(bytes).collect()
    }

    fn one(bytes: &[u8]) -> RawDesc<'_> {
        Descriptors::new(bytes).next().unwrap().unwrap()
    }

    #[test]
    fn writes_appendix_b_alternate_setting_0() {
        // REQ-USB-MS1-002 (order: header, Jacks, each endpoint and its MS_GENERAL directly
        // after), REQ-USB-MS1-004 (wTotalLength 0x0041), REQ-USB-TOPO-008 (one descriptor per
        // Jack, no other).
        let t = appendix_b();
        assert_eq!(t.len(), 0x41);
        assert_eq!(t.num_endpoints(), 2);
        assert_eq!(write_vec(&t), APPENDIX_B);
        let jack_descs = Descriptors::new(&APPENDIX_B)
            .flatten()
            .filter(|d| {
                d.dtype() == CS_INTERFACE
                    && matches!(d.subtype(), Some(MIDI_IN_JACK | MIDI_OUT_JACK | ELEMENT))
            })
            .count();
        assert_eq!(jack_descs, 4);
    }

    #[test]
    fn writer_overflow_writes_nothing() {
        // DEC-002: Overflow with nothing written.
        let t = appendix_b();
        let mut buf = [0xAAu8; 64];
        assert_eq!(t.write(&mut buf), Err(Ms1Error::Overflow));
        assert!(buf.iter().all(|&b| b == 0xAA));
        let mut small = [0xAAu8; 8];
        let ep = Ms1Endpoint::new(EndpointDirection::MidiIn, 1, 64, &IN_JACKS);
        assert_eq!(ep.write(&mut small), Err(Ms1Error::Overflow));
        assert_eq!(
            MsHeader::new(7).write(&mut small[..6]),
            Err(Ms1Error::Overflow)
        );
        assert!(small.iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn header_table_b6_bytes() {
        // REQ-USB-MS1-003, REQ-USB-VEC1-004: Table B-6.
        let mut buf = [0u8; 7];
        assert_eq!(MsHeader::new(0x41).write(&mut buf), Ok(7));
        assert_eq!(buf, [0x07, 0x24, 0x01, 0x00, 0x01, 0x41, 0x00]);
        let h = MsHeader::read(&one(&buf)).unwrap();
        assert_eq!(
            h,
            MsHeader {
                bcd_msc: 0x0100,
                total_length: 0x0041
            }
        );
        assert!(MsHeader::read(&one(&[6, 0x24, 1, 0, 1, 0x41])).is_err());
    }

    #[test]
    fn total_length_counts_endpoint_descriptors() {
        // REQ-USB-MS1-004, choice U-10: 7 + 6 + 6 + 9 + 9 + 9 + 5 + 9 + 5 = 0x41, not the
        // Table 6-2 value 0x25.
        let t = appendix_b();
        let bytes = write_vec(&t);
        assert_eq!(u16::from_le_bytes([bytes[5], bytes[6]]), 0x0041);
        assert_eq!(
            usize::from(u16::from_le_bytes([bytes[5], bytes[6]])),
            bytes.len()
        );
    }

    #[test]
    fn in_jack_tables_b7_b8() {
        // REQ-USB-MS1-010, REQ-USB-VEC1-005, REQ-USB-VEC1-006.
        let mut buf = [0u8; 6];
        MidiInJack::new(EMB, 1).write(&mut buf).unwrap();
        assert_eq!(buf, [0x06, 0x24, 0x02, 0x01, 0x01, 0x00]);
        assert_eq!(MidiInJack::read(&one(&buf)), Ok(MidiInJack::new(EMB, 1)));
        MidiInJack::new(EXT, 2).write(&mut buf).unwrap();
        assert_eq!(buf, [0x06, 0x24, 0x02, 0x02, 0x02, 0x00]);
        assert_eq!(MidiInJack::read(&one(&buf)), Ok(MidiInJack::new(EXT, 2)));
    }

    #[test]
    fn out_jack_tables_b9_b10() {
        // REQ-USB-MS1-011, REQ-USB-VEC1-007, REQ-USB-VEC1-008: bLength 7+2p, iJack at 6+2p
        // (AMB-USB-MIDI-1-11).
        let mut buf = [0u8; 9];
        let j3 = MidiOutJack::new(EMB, 3, &PIN_2);
        assert_eq!(j3.write(&mut buf), Ok(9));
        assert_eq!(buf, [0x09, 0x24, 0x03, 0x01, 0x03, 0x01, 0x02, 0x01, 0x00]);
        assert_eq!(MidiOutJack::read(&one(&buf)), Ok(j3));
        let j4 = MidiOutJack::new(EXT, 4, &PIN_1);
        j4.write(&mut buf).unwrap();
        assert_eq!(buf, [0x09, 0x24, 0x03, 0x02, 0x04, 0x01, 0x01, 0x01, 0x00]);
        assert_eq!(MidiOutJack::read(&one(&buf)), Ok(j4));
    }

    #[test]
    fn out_jack_with_three_pins_layout() {
        // REQ-USB-MS1-011, REQ-USB-TOPO-006: one baSourceID, baSourcePin pair for each pin.
        let pins = [Pin::new(1, 1), Pin::new(2, 1), Pin::new(5, 1)];
        let mut j = MidiOutJack::new(EXT, 9, &pins);
        j.string = NonZeroU8::new(4);
        let mut buf = [0u8; 13];
        assert_eq!(j.write(&mut buf), Ok(13));
        assert_eq!(buf, [13, 0x24, 0x03, 0x02, 9, 3, 1, 1, 2, 1, 5, 1, 4]);
        let r = MidiOutJack::read(&one(&buf)).unwrap();
        assert_eq!(r, j);
        assert_eq!(r.pins.get(2), Some(Pin::new(5, 1)));
        assert_eq!(r.pins.get(3), None);
        // bLength less than 7+2p is an error.
        let mut short = buf;
        short[0] = 12;
        assert!(MidiOutJack::read(&one(&short[..12])).is_err());
    }

    #[test]
    fn jack_string_index() {
        // REQ-USB-MS1-012: iJack is the caller index, 0x00 by default; the reader reads 0 as
        // no string.
        let mut buf = [0u8; 6];
        let mut j = MidiInJack::new(EMB, 1);
        j.write(&mut buf).unwrap();
        assert_eq!(buf[5], 0);
        assert_eq!(MidiInJack::read(&one(&buf)).unwrap().string, None);
        j.string = NonZeroU8::new(7);
        j.write(&mut buf).unwrap();
        assert_eq!(buf[5], 7);
        assert_eq!(
            MidiInJack::read(&one(&buf)).unwrap().string,
            NonZeroU8::new(7)
        );
        let e = Element::read(&one(&[13, 0x24, 4, 5, 1, 1, 1, 1, 0, 0, 1, 0x02, 0])).unwrap();
        assert_eq!(e.string, None);
        // iInterface: 0 is no string, another index is kept.
        for s in [0u8, 3] {
            let i = crate::ms2::MsInterface::read(&one(&[9, 4, 1, 0, 2, 1, 3, 0, s]))
                .unwrap()
                .unwrap();
            assert_eq!(i.string_index, s);
        }
    }

    #[test]
    fn jack_type_writer_refuses_undefined_reader_keeps_unknown() {
        // REQ-USB-MS1-009: the writer never writes 0x00; the reader gives other values and
        // continues.
        let mut buf = [0u8; 16];
        for t in [JackType::Undefined, JackType::Unknown(7)] {
            assert_eq!(
                MidiInJack::new(t, 1).write(&mut buf),
                Err(TopologyError::JackType(t.into_bits()).into())
            );
            assert!(MidiOutJack::new(t, 1, &PIN_1).write(&mut buf).is_err());
        }
        let bytes = [
            6, 0x24, 2, 0x00, 1, 0, 6, 0x24, 2, 0x07, 2, 0, 6, 0x24, 2, 1, 3, 0,
        ];
        let r = read_all(&bytes);
        assert_eq!(r.len(), 3);
        assert!(matches!(
            r[0],
            Ok(Ms1Desc::InJack(MidiInJack {
                jack_type: JackType::Undefined,
                ..
            }))
        ));
        assert!(matches!(
            r[1],
            Ok(Ms1Desc::InJack(MidiInJack {
                jack_type: JackType::Unknown(7),
                ..
            }))
        ));
        assert!(matches!(
            r[2],
            Ok(Ms1Desc::InJack(MidiInJack {
                jack_type: JackType::Embedded,
                id: 3,
                ..
            }))
        ));
        let mut t2: Ms1Topology<'_, 5> = Ms1Topology::new();
        for j in appendix_b().jacks() {
            t2.add_jack(*j).unwrap();
        }
        for e in appendix_b().endpoints() {
            t2.add_endpoint(*e).unwrap();
        }
        t2.add_jack(Jack::In(MidiInJack::new(JackType::Undefined, 9)))
            .unwrap();
        assert_eq!(t2.check(), Err(TopologyError::JackType(0).into()));
    }

    #[test]
    fn element_reader_layout() {
        // REQ-USB-MS1-013, REQ-USB-TOPO-012: 10+2p+n bytes; p = 2, q = 1, n = 2.
        let bytes = [
            14, 0x24, 0x04, 7, // bElementID 7
            2, 1, 1, 2, 1, // two pins: Jack 1 pin 1, Jack 2 pin 1
            1, 5, 6, // bNrOutputPins 1, bInTerminalLink 5, bOutTerminalLink 6
            2, 0x03, // bElCapsSize 2, caps low byte (needs a second byte, see below)
        ];
        // The array above is 14 bytes with bLength 14 but n = 2 needs 16 bytes: the high
        // capability byte and iElement are absent and read as 0 (REQ-USB-MS1-015).
        let e = Element::read(&one(&bytes)).unwrap();
        assert_eq!(e.id, 7);
        assert_eq!(e.pins.len(), 2);
        assert_eq!(e.pins.get(1), Some(Pin::new(2, 1)));
        assert_eq!(e.output_pins, 1);
        assert_eq!(e.in_terminal_link, NonZeroU8::new(5));
        assert_eq!(e.out_terminal_link, NonZeroU8::new(6));
        assert_eq!(e.caps_size, 2);
        assert_eq!(e.caps.bits(), 0x0003);
        assert_eq!(e.string, None);
        let full = [16, 0x24, 0x04, 7, 2, 1, 1, 2, 1, 1, 5, 6, 2, 0x03, 0x0F, 9];
        let e = Element::read(&one(&full)).unwrap();
        assert_eq!(e.caps.bits(), 0x0F03);
        assert_eq!(e.string, NonZeroU8::new(9));
        // Too short for the fixed fields.
        assert!(Element::read(&one(&[8, 0x24, 4, 7, 2, 1, 1, 2])).is_err());
    }

    #[test]
    fn element_terminal_links_zero_is_no_link() {
        // REQ-USB-MS1-014.
        let e = Element::read(&one(&[12, 0x24, 4, 5, 1, 1, 1, 1, 0, 0, 0, 0])).unwrap();
        assert_eq!((e.in_terminal_link, e.out_terminal_link), (None, None));
    }

    #[test]
    fn element_caps_size_any() {
        // REQ-USB-MS1-015: size 0 gives no bit; size 1 gives D0..D7; size 3 reads 2 bytes.
        let e0 = Element::read(&one(&[11, 0x24, 4, 5, 1, 1, 1, 1, 0, 0, 0])).unwrap();
        assert_eq!(e0.caps.bits(), 0);
        // n = 0: the byte after bElCapsSize is iElement, not a capability.
        let e0s = Element::read(&one(&[12, 0x24, 4, 5, 1, 1, 1, 1, 0, 0, 0, 0xFF])).unwrap();
        assert_eq!(e0s.caps.bits(), 0);
        assert_eq!(e0s.string, NonZeroU8::new(0xFF));
        let e1 = Element::read(&one(&[13, 0x24, 4, 5, 1, 1, 1, 1, 0, 0, 1, 0x82, 3])).unwrap();
        assert_eq!(e1.caps.bits(), 0x82);
        assert_eq!(e1.string, NonZeroU8::new(3));
        let e3 = Element::read(&one(&[
            15, 0x24, 4, 5, 1, 1, 1, 1, 0, 0, 3, 0x01, 0x0F, 0xFF, 2,
        ]))
        .unwrap();
        assert_eq!(e3.caps.bits(), 0x0F01);
        assert_eq!(e3.string, NonZeroU8::new(2));
    }

    #[test]
    fn element_caps_bits_d0_to_d11() {
        // REQ-USB-MS1-016: D0..D11 read, D12 and higher ignored.
        let e = Element::read(&one(&[14, 0x24, 4, 5, 1, 1, 1, 1, 0, 0, 2, 0xFF, 0xFF, 0])).unwrap();
        assert_eq!(e.caps.bits(), 0x0FFF);
        let names = [
            ElementCaps::CUSTOM_UNDEFINED,
            ElementCaps::MIDI_CLOCK,
            ElementCaps::MTC,
            ElementCaps::MMC,
            ElementCaps::GM1,
            ElementCaps::GM2,
            ElementCaps::GS,
            ElementCaps::XG,
            ElementCaps::EFX,
            ElementCaps::MIDI_PATCH_BAY,
            ElementCaps::DLS1,
            ElementCaps::DLS2,
        ];
        for (i, m) in names.iter().enumerate() {
            assert_eq!(*m, 1u16 << i);
            assert!(e.caps.contains(*m));
        }
        assert_eq!(ElementCaps::from_bits(0xF000).bits(), 0);
    }

    #[test]
    fn endpoint_tables_b11_to_b14() {
        // REQ-USB-MS1-018, REQ-USB-MS1-021, REQ-USB-VEC1-009 to REQ-USB-VEC1-012.
        let mut buf = [0u8; 14];
        let out = Ms1Endpoint::new(EndpointDirection::MidiOut, 1, 64, &OUT_JACKS);
        assert_eq!(out.write(&mut buf), Ok(14));
        assert_eq!(
            buf[..9],
            [0x09, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(buf[9..], [0x05, 0x25, 0x01, 0x01, 0x01]);
        assert_eq!(read_all(&buf), [Ok(Ms1Desc::Endpoint(out))]);
        let inp = Ms1Endpoint::new(EndpointDirection::MidiIn, 1, 64, &IN_JACKS);
        inp.write(&mut buf).unwrap();
        assert_eq!(
            buf[..9],
            [0x09, 0x05, 0x81, 0x02, 0x40, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(buf[9..], [0x05, 0x25, 0x01, 0x01, 0x03]);
        assert_eq!(read_all(&buf), [Ok(Ms1Desc::Endpoint(inp))]);
    }

    #[test]
    fn endpoint_writer_refuses_bad_values() {
        // REQ-USB-MS1-018 (endpoint number per REQ-USB-MS2-007, packet size per
        // REQ-USB-MS2-009), REQ-USB-MS1-021.
        let mut buf = [0u8; 64];
        let ep = |n, mps, j| Ms1Endpoint::new(EndpointDirection::MidiOut, n, mps, j);
        assert_eq!(
            ep(0, 64, &OUT_JACKS).write(&mut buf),
            Err(Ms1Error::EndpointNumber(0))
        );
        assert_eq!(
            ep(16, 64, &OUT_JACKS).write(&mut buf),
            Err(Ms1Error::EndpointNumber(16))
        );
        assert_eq!(
            ep(1, 62, &OUT_JACKS).write(&mut buf),
            Err(Ms1Error::MaxPacketSize(62))
        );
        assert_eq!(
            ep(1, 0, &OUT_JACKS).write(&mut buf),
            Err(Ms1Error::MaxPacketSize(0))
        );
        assert_eq!(ep(15, 512, &OUT_JACKS).write(&mut buf), Ok(14));
        assert_eq!(&buf[2..6], &[0x0F, 0x02, 0x00, 0x02]);
    }

    #[test]
    fn endpoint_reader_short_cs_descriptor_is_error() {
        // REQ-USB-MS1-021: bLength less than 4+n.
        let bytes = [
            9, 5, 0x81, 2, 64, 0, 0, 0, 0, 5, 0x25, 1, 2, 3, 6, 0x24, 2, 1, 4, 0,
        ];
        let r = read_all(&bytes);
        assert_eq!(r[0], Err(Ms1Error::EndpointTooShort));
        assert!(matches!(r[1], Ok(Ms1Desc::InJack(_))));
        // A 7-byte standard endpoint (USB MIDI 2.0 form) is also read.
        let r = read_all(&[7, 5, 0x02, 2, 64, 0, 0, 5, 0x25, 1, 1, 1]);
        assert!(matches!(
            r[0],
            Ok(Ms1Desc::Endpoint(Ms1Endpoint { number: 2, .. }))
        ));
    }

    #[test]
    fn endpoint_reader_gives_transfer_interval_and_usable_packet_size() {
        // REQ-USB-MS2-008, REQ-USB-MS2-009, REQ-USB-MS2-011 on Alternate Setting 0: the reader
        // gives the transfer type and bInterval of an interrupt endpoint, rounds
        // wMaxPacketSize down to a multiple of 4, and refuses a bLength below 7.
        // Interrupt IN 1, wMaxPacketSize 66 (used: 64), bInterval 4.
        let r = read_all(&[9, 5, 0x81, 3, 66, 0, 4, 0, 0, 5, 0x25, 1, 1, 3]);
        let Ok(Ms1Desc::Endpoint(e)) = r[0] else {
            panic!("{r:?}")
        };
        assert_eq!(
            (e.transfer, e.interval, e.max_packet_size),
            (INTERRUPT, 4, 64)
        );
        assert!(e.is_interrupt());
        assert_eq!(e.address(), 0x81);
        assert_eq!(r.len(), 1);
        // Bulk: bInterval is ignored (0).
        let r = read_all(&[9, 5, 0x01, 2, 64, 0, 9, 0, 0, 5, 0x25, 1, 1, 1]);
        let Ok(Ms1Desc::Endpoint(e)) = r[0] else {
            panic!("{r:?}")
        };
        assert_eq!((e.transfer, e.interval, e.max_packet_size), (BULK, 0, 64));
        // wMaxPacketSize 2: rounded down to 0, an error item after the endpoint.
        let r = read_all(&[9, 5, 0x01, 2, 2, 0, 0, 0, 0, 5, 0x25, 1, 1, 1]);
        assert!(matches!(r[0], Ok(Ms1Desc::Endpoint(_))));
        assert_eq!(r[1], Err(Ms1Error::MaxPacketSize(2)));
        // bLength 6 (the case of the review, correctness M-2): an error, and the class-specific
        // descriptor after it is not given as another descriptor.
        let r = read_all(&[6, 5, 0x81, 3, 2, 0, 5, 0x25, 1, 1, 3]);
        assert_eq!(
            r,
            [Err(Ms1Error::TooShort {
                dtype: ENDPOINT,
                subtype: 0x81
            })]
        );
        // A Transfer endpoint also gives its transfer type and bInterval.
        let r = read_all(&[7, 5, 0x82, 3, 8, 0, 10]);
        assert_eq!(
            r,
            [Ok(Ms1Desc::Transfer(TransferEndpoint {
                direction: EndpointDirection::MidiIn,
                number: 2,
                max_packet_size: 8,
                transfer: INTERRUPT,
                interval: 10,
            }))]
        );
    }

    // REQ-USB-MS2-017: the USB-MIDI 1.0 writer refuses two MIDI endpoints with the same
    // bEndpointAddress in Alternate Setting 0, and writes nothing.
    #[test]
    fn writer_refuses_two_endpoints_with_one_address() {
        let mut t = appendix_b();
        let ids = [1u8];
        t.endpoints[0] = Some(Ms1Endpoint::new(EndpointDirection::MidiIn, 1, 64, &[3]));
        t.endpoints[1] = Some(Ms1Endpoint::new(EndpointDirection::MidiIn, 1, 64, &ids));
        // The first error is the address, before the Jack direction check of the second list.
        assert_eq!(t.check(), Err(TopologyError::DuplicateEndpoint(1).into()));
        let mut buf = [0xAAu8; 128];
        assert_eq!(
            t.write(&mut buf),
            Err(TopologyError::DuplicateEndpoint(1).into())
        );
        assert!(buf.iter().all(|&b| b == 0xAA));
        // OUT 1 and IN 1 are two addresses (Appendix B).
        assert_eq!(appendix_b().check(), Ok(()));
    }

    #[test]
    fn endpoint_lists_embedded_jacks_of_correct_direction() {
        // REQ-USB-MS1-022: OUT endpoint lists Embedded MIDI IN Jacks, IN endpoint lists
        // Embedded MIDI OUT Jacks, each in one endpoint only.
        let wrong_dir = [3u8];
        let mut t = appendix_b();
        t.endpoints[0] = Some(Ms1Endpoint::new(
            EndpointDirection::MidiOut,
            1,
            64,
            &wrong_dir,
        ));
        assert_eq!(
            t.check(),
            Err(TopologyError::WrongJack {
                endpoint: 1,
                jack: 3
            }
            .into())
        );
        let external = [2u8];
        let mut t = appendix_b();
        t.endpoints[0] = Some(Ms1Endpoint::new(
            EndpointDirection::MidiOut,
            1,
            64,
            &external,
        ));
        assert_eq!(
            t.check(),
            Err(TopologyError::WrongJack {
                endpoint: 1,
                jack: 2
            }
            .into())
        );
        let unknown = [1u8, 99];
        let mut t = appendix_b();
        t.endpoints[0] = Some(Ms1Endpoint::new(
            EndpointDirection::MidiOut,
            1,
            64,
            &unknown,
        ));
        assert_eq!(
            t.check(),
            Err(TopologyError::WrongJack {
                endpoint: 1,
                jack: 99
            }
            .into())
        );
        let twice = [1u8, 1];
        let mut t = appendix_b();
        t.endpoints[0] = Some(Ms1Endpoint::new(EndpointDirection::MidiOut, 1, 64, &twice));
        assert_eq!(t.check(), Err(TopologyError::JackInTwoEndpoints(1).into()));
        let mut t: Ms1Topology<'_, 4, 3> = Ms1Topology::new();
        for j in appendix_b().jacks() {
            t.add_jack(*j).unwrap();
        }
        for e in appendix_b().endpoints() {
            t.add_endpoint(*e).unwrap();
        }
        t.add_endpoint(Ms1Endpoint::new(
            EndpointDirection::MidiOut,
            2,
            64,
            &OUT_JACKS,
        ))
        .unwrap();
        assert_eq!(t.check(), Err(TopologyError::JackInTwoEndpoints(1).into()));
        // An Embedded Jack in no list.
        let mut t = appendix_b();
        t.endpoints[1] = None;
        t.n_endpoints = 1;
        assert_eq!(t.check(), Err(TopologyError::JackWithoutEndpoint(3).into()));
        // The reader accepts other Jack IDs and gives them.
        let r = read_all(&[9, 5, 1, 2, 64, 0, 0, 0, 0, 6, 0x25, 1, 2, 4, 200]);
        assert!(matches!(
            r[0],
            Ok(Ms1Desc::Endpoint(Ms1Endpoint {
                jacks: &[4, 200],
                ..
            }))
        ));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn embedded_jacks_written_in_assoc_order() {
        // REQ-USB-MS1-023: the n-th Jack ID of the list is the n-th Embedded Jack of that
        // direction in descriptor order, whatever order the caller adds them in.
        let pins_a = [Pin::new(10, 1)];
        let pins_b = [Pin::new(11, 1)];
        let out_list = [2u8, 1];
        let in_list = [21u8, 20];
        let mut t: Ms1Topology<'_, 6> = Ms1Topology::new();
        t.add_jack(Jack::In(MidiInJack::new(EMB, 1))).unwrap();
        t.add_jack(Jack::In(MidiInJack::new(EMB, 2))).unwrap();
        t.add_jack(Jack::In(MidiInJack::new(EXT, 10))).unwrap();
        t.add_jack(Jack::In(MidiInJack::new(EXT, 11))).unwrap();
        t.add_jack(Jack::Out(MidiOutJack::new(EMB, 20, &pins_a)))
            .unwrap();
        t.add_jack(Jack::Out(MidiOutJack::new(EMB, 21, &pins_b)))
            .unwrap();
        t.add_endpoint(Ms1Endpoint::new(
            EndpointDirection::MidiOut,
            1,
            64,
            &out_list,
        ))
        .unwrap();
        t.add_endpoint(Ms1Endpoint::new(EndpointDirection::MidiIn, 2, 64, &in_list))
            .unwrap();
        let bytes = write_vec(&t);
        let mut emb_in = Vec::new();
        let mut emb_out = Vec::new();
        for d in Ms1Descriptors::new(&bytes).flatten() {
            match d {
                Ms1Desc::InJack(j) if j.jack_type == EMB => emb_in.push(j.id),
                Ms1Desc::OutJack(j) if j.jack_type == EMB => emb_out.push(j.id),
                _ => {}
            }
        }
        assert_eq!(emb_in, out_list);
        assert_eq!(emb_out, in_list);
        assert_eq!(
            usize::from(u16::from_le_bytes([bytes[5], bytes[6]])),
            bytes.len()
        );
    }

    #[test]
    fn needs_one_endpoint_and_one_jack_per_endpoint() {
        // REQ-USB-TOPO-001.
        let mut t: Ms1Topology<'_, 4> = Ms1Topology::new();
        t.add_jack(Jack::In(MidiInJack::new(EXT, 2))).unwrap();
        assert_eq!(t.check(), Err(TopologyError::NoEndpoint.into()));
        let mut buf = [0u8; 64];
        assert_eq!(t.write(&mut buf), Err(TopologyError::NoEndpoint.into()));
        let mut t = appendix_b();
        t.endpoints[1] = Some(Ms1Endpoint::new(EndpointDirection::MidiIn, 1, 64, &[]));
        assert_eq!(t.check(), Err(TopologyError::EndpointWithoutJack(1).into()));
    }

    #[test]
    fn entity_ids_nonzero_and_unique() {
        // REQ-USB-TOPO-004: the writer refuses 0x00 and duplicates; the reader reports both and
        // still gives the descriptors.
        let mut t = appendix_b();
        t.jacks[1] = Some(Jack::In(MidiInJack::new(EXT, 0)));
        assert_eq!(t.check(), Err(TopologyError::ZeroId.into()));
        let mut t = appendix_b();
        t.jacks[1] = Some(Jack::In(MidiInJack::new(EXT, 1)));
        assert_eq!(t.check(), Err(TopologyError::DuplicateId(1).into()));
        assert_eq!(
            MidiInJack::new(EMB, 0).write(&mut [0; 6]),
            Err(TopologyError::ZeroId.into())
        );
        let bytes = [
            6, 0x24, 2, 1, 0, 0, // Jack ID 0
            6, 0x24, 2, 2, 5, 0, // Jack 5
            12, 0x24, 4, 5, 1, 1, 1, 1, 0, 0, 0, 0, // Element 5: duplicate
        ];
        let r = read_all(&bytes);
        assert_eq!(r.len(), 5);
        assert!(matches!(
            r[0],
            Ok(Ms1Desc::InJack(MidiInJack { id: 0, .. }))
        ));
        assert_eq!(r[1], Err(TopologyError::ZeroId.into()));
        assert!(matches!(
            r[2],
            Ok(Ms1Desc::InJack(MidiInJack { id: 5, .. }))
        ));
        assert!(matches!(r[3], Ok(Ms1Desc::Element(Element { id: 5, .. }))));
        assert_eq!(r[4], Err(TopologyError::DuplicateId(5).into()));
    }

    #[test]
    fn source_pin_must_be_output_pin_of_source() {
        // REQ-USB-TOPO-005: a MIDI IN Jack has one Output Pin, number 1.
        for pin in [0u8, 2] {
            let pins = [Pin::new(2, pin)];
            let mut t = appendix_b();
            t.jacks[2] = Some(Jack::Out(MidiOutJack::new(EMB, 3, &pins)));
            assert_eq!(
                t.check(),
                Err(TopologyError::BadSourcePin { jack: 3, pin }.into())
            );
        }
    }

    #[test]
    fn source_must_be_midi_in_jack() {
        // REQ-USB-TOPO-006: the writer refuses a source that is a MIDI OUT Jack or no Entity;
        // the reader accepts any source.
        for src in [4u8, 77] {
            let pins = [Pin::new(src, 1)];
            let mut t = appendix_b();
            t.jacks[2] = Some(Jack::Out(MidiOutJack::new(EMB, 3, &pins)));
            assert_eq!(
                t.check(),
                Err(TopologyError::BadSourceId {
                    jack: 3,
                    source_id: src
                }
                .into())
            );
        }
        let r = read_all(&[9, 0x24, 3, 1, 3, 1, 77, 9, 0]);
        let Ok(Ms1Desc::OutJack(j)) = r[0] else {
            panic!()
        };
        assert_eq!(j.pins.get(0), Some(Pin::new(77, 9)));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn out_jack_input_pin_count() {
        // REQ-USB-TOPO-007: writer 1 to 124 pins; the reader also accepts 0.
        let mut buf = [0u8; 300];
        assert_eq!(
            MidiOutJack::new(EMB, 3, &[]).write(&mut buf),
            Err(TopologyError::NoInputPin(3).into())
        );
        let many = [Pin::new(1, 1); 125];
        assert_eq!(
            MidiOutJack::new(EMB, 3, &many).write(&mut buf),
            Err(TopologyError::TooManyInputPins(3).into())
        );
        assert_eq!(
            MidiOutJack::new(EMB, 3, &many[..124]).write(&mut buf),
            Ok(255)
        );
        assert_eq!(buf[0], 255);
        let mut t = appendix_b();
        t.jacks[2] = Some(Jack::Out(MidiOutJack::new(EMB, 3, &[])));
        assert_eq!(t.check(), Err(TopologyError::NoInputPin(3).into()));
        let r = read_all(&[7, 0x24, 3, 1, 3, 0, 0]);
        let Ok(Ms1Desc::OutJack(j)) = r[0] else {
            panic!()
        };
        assert!(j.pins.is_empty());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn at_most_16_jacks_per_endpoint() {
        // REQ-USB-TOPO-010: writer error; the reader reports and keeps the first 16.
        let list: [u8; 17] = core::array::from_fn(|i| i as u8 + 1);
        let mut buf = [0u8; 64];
        let ep = Ms1Endpoint::new(EndpointDirection::MidiOut, 1, 64, &list);
        assert_eq!(
            ep.write(&mut buf),
            Err(TopologyError::TooManyJacks(1).into())
        );
        assert_eq!(
            Ms1Endpoint {
                jacks: &list[..16],
                ..ep
            }
            .write(&mut buf),
            Ok(29)
        );
        let mut bytes = Vec::from([9u8, 5, 1, 2, 64, 0, 0, 0, 0, 21, 0x25, 1, 17]);
        bytes.extend_from_slice(&list);
        let r = read_all(&bytes);
        assert_eq!(r.len(), 2);
        let Ok(Ms1Desc::Endpoint(e)) = r[0] else {
            panic!()
        };
        assert_eq!(e.jacks, &list[..16]);
        assert_eq!(e.cable_map().cables(), 16);
        assert_eq!(r[1], Err(TopologyError::TooManyJacks(1).into()));
    }

    #[test]
    fn function_without_external_jacks() {
        // REQ-USB-TOPO-011: no External Jack is valid; the reader accepts an External MIDI OUT
        // Jack whose source is an Element.
        let mut t: Ms1Topology<'_, 2> = Ms1Topology::new();
        let pins = [Pin::new(1, 1)];
        t.add_jack(Jack::In(MidiInJack::new(EMB, 1))).unwrap();
        t.add_jack(Jack::Out(MidiOutJack::new(EMB, 2, &pins)))
            .unwrap();
        let (o, i) = ([1u8], [2u8]);
        t.add_endpoint(Ms1Endpoint::new(EndpointDirection::MidiOut, 1, 64, &o))
            .unwrap();
        t.add_endpoint(Ms1Endpoint::new(EndpointDirection::MidiIn, 1, 64, &i))
            .unwrap();
        let bytes = write_vec(&t);
        assert_eq!(bytes.len(), 7 + 6 + 9 + 14 + 14);
        assert!(read_all(&bytes).iter().all(Result::is_ok));
        let r = read_all(&[
            12, 0x24, 4, 5, 1, 1, 1, 1, 0, 0, 0, 0, // Element 5
            9, 0x24, 3, 2, 6, 1, 5, 1, 0, // External OUT Jack 6 from Element 5
        ]);
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(Result::is_ok));
    }

    #[test]
    fn writer_writes_no_element_reader_reads_them() {
        // REQ-USB-TOPO-012 (choice U-13), REQ-USB-TOPO-008.
        let bytes = write_vec(&appendix_b());
        assert!(Descriptors::new(&bytes)
            .flatten()
            .all(|d| !(d.dtype() == CS_INTERFACE && d.subtype() == Some(ELEMENT))));
        let r = read_all(&[12, 0x24, 4, 5, 1, 1, 1, 1, 0, 0, 0, 0]);
        assert!(matches!(
            r[..],
            [Ok(Ms1Desc::Element(Element { id: 5, .. }))]
        ));
    }

    #[test]
    fn element_with_no_pins_is_reported() {
        // REQ-USB-TOPO-013: p = 0 and q = 0 reported, descriptor still given.
        let r = read_all(&[10, 0x24, 4, 5, 0, 0, 0, 0, 0, 0]);
        assert_eq!(r.len(), 3);
        assert!(matches!(
            r[0],
            Ok(Ms1Desc::Element(Element {
                id: 5,
                output_pins: 0,
                ..
            }))
        ));
        assert_eq!(r[1], Err(TopologyError::ElementNoInputPin(5).into()));
        assert_eq!(r[2], Err(TopologyError::ElementNoOutputPin(5).into()));
    }

    #[test]
    fn transfer_endpoint_read_never_written() {
        // REQ-USB-TOPO-014 (choice U-14): a standard endpoint with no class-specific descriptor
        // after it is a Transfer endpoint; the writer puts MS_GENERAL after each endpoint.
        let bytes = [
            9, 5, 0x82, 2, 64, 0, 0, 0, 0, // Transfer IN 2
            9, 5, 0x03, 2, 64, 0, 0, 0, 0, // Transfer OUT 3, at the end
        ];
        let r = read_all(&bytes);
        assert_eq!(
            r,
            [
                Ok(Ms1Desc::Transfer(TransferEndpoint {
                    direction: EndpointDirection::MidiIn,
                    number: 2,
                    max_packet_size: 64,
                    transfer: BULK,
                    interval: 0,
                })),
                Ok(Ms1Desc::Transfer(TransferEndpoint {
                    direction: EndpointDirection::MidiOut,
                    number: 3,
                    max_packet_size: 64,
                    transfer: BULK,
                    interval: 0,
                })),
            ]
        );
        let written = write_vec(&appendix_b());
        let d: Vec<_> = Descriptors::new(&written).flatten().collect();
        for (i, x) in d.iter().enumerate() {
            if x.dtype() == ENDPOINT {
                assert_eq!(d[i + 1].dtype(), CS_ENDPOINT);
            }
        }
        assert!(read_all(&written)
            .iter()
            .all(|x| !matches!(x, Ok(Ms1Desc::Transfer(_)))));
    }

    #[test]
    fn reads_appendix_b_alternate_setting_0() {
        // REQ-USB-MS1-003, REQ-USB-MS1-010, REQ-USB-MS1-011, REQ-USB-MS1-021: the reader gives
        // the Appendix B descriptors back.
        let r: Vec<_> = read_all(&APPENDIX_B)
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            r,
            [
                Ms1Desc::Header(MsHeader::new(0x41)),
                Ms1Desc::InJack(MidiInJack::new(EMB, 1)),
                Ms1Desc::InJack(MidiInJack::new(EXT, 2)),
                Ms1Desc::OutJack(MidiOutJack::new(EMB, 3, &PIN_2)),
                Ms1Desc::OutJack(MidiOutJack::new(EXT, 4, &PIN_1)),
                Ms1Desc::Endpoint(Ms1Endpoint::new(
                    EndpointDirection::MidiOut,
                    1,
                    64,
                    &OUT_JACKS
                )),
                Ms1Desc::Endpoint(Ms1Endpoint::new(
                    EndpointDirection::MidiIn,
                    1,
                    64,
                    &IN_JACKS
                )),
            ]
        );
    }

    #[test]
    fn interface_table_b5_bytes() {
        // REQ-USB-VEC1-003: Table B-5 with interface 1, two MIDI endpoints, no string, from the
        // writer and the reader of the crate (Ms2); bNumEndpoints from the topology.
        let t = appendix_b();
        let i = crate::ms2::MsInterface {
            interface_number: 1,
            alternate_setting: 0,
            num_endpoints: t.num_endpoints(),
            string_index: 0,
        };
        let mut b = [0u8; 9];
        assert_eq!(i.write(&mut b), Ok(9));
        assert_eq!(b, [0x09, 0x04, 0x01, 0x00, 0x02, 0x01, 0x03, 0x00, 0x00]);
        assert_eq!(crate::ms2::MsInterface::read(&one(&b)), Some(Ok(i)));
        let mut all = Vec::from(b);
        all.extend(write_vec(&t));
        let g = crate::walk::interfaces(&all).next().unwrap();
        assert_eq!(g.body, APPENDIX_B);
    }

    #[test]
    fn cable_number_is_position_in_assoc_list() {
        // REQ-USB-EVT-017: cable n = position n of baAssocJackID; cables = bNumEmbMIDIJack;
        // an Embedded MIDI OUT Jack is sent only on the MIDI IN endpoint that lists it.
        let m = CableMap::new(&[5, 9, 1]);
        assert_eq!(m.cables(), 3);
        assert_eq!(
            (m.jack(0), m.jack(1), m.jack(2), m.jack(3)),
            (Some(5), Some(9), Some(1), None)
        );
        assert_eq!((m.cable(9), m.cable(4)), (Some(1), None));
        let t = appendix_b();
        assert_eq!(
            t.send_route(3),
            Some(CableRoute {
                endpoint: 1,
                cable: 0
            })
        );
        assert_eq!(t.send_route(1), None);
        assert_eq!(
            t.receive_route(1),
            Some(CableRoute {
                endpoint: 1,
                cable: 0
            })
        );
        let list = [1u8, 5, 9, 13];
        let ep = Ms1Endpoint::new(EndpointDirection::MidiOut, 1, 64, &list);
        assert_eq!(ep.cable_map().cable(13), Some(3));
    }

    #[test]
    fn capacity_full() {
        let mut t: Ms1Topology<'_, 1, 1> = Ms1Topology::new();
        t.add_jack(Jack::In(MidiInJack::new(EMB, 1))).unwrap();
        assert_eq!(
            t.add_jack(Jack::In(MidiInJack::new(EMB, 2))),
            Err(TopologyError::Full)
        );
        t.add_endpoint(Ms1Endpoint::new(
            EndpointDirection::MidiOut,
            1,
            64,
            &OUT_JACKS,
        ))
        .unwrap();
        assert_eq!(
            t.add_endpoint(Ms1Endpoint::new(
                EndpointDirection::MidiIn,
                1,
                64,
                &IN_JACKS
            )),
            Err(TopologyError::Full)
        );
    }

    /// The cable-count form of choice U-26 (REQ-USB-MS1-027), as the Device module builds it:
    /// (is a MIDI OUT Jack, jack type, Jack ID, index in `pins`).
    fn cable_count(
        n_out: usize,
        n_in: usize,
        pins: &mut [[Pin; 1]; 32],
        lists: &mut ([u8; 16], [u8; 16]),
    ) -> Vec<(bool, JackType, u8, usize)> {
        let mut plan = Vec::new();
        let mut id = 1u8;
        let mut pi = 0;
        for k in 0..n_out.max(n_in) {
            let emb_in = id;
            if k < n_out {
                plan.push((false, EMB, id, 0));
                lists.0[k] = id;
                id += 1;
            }
            if k < n_in {
                plan.push((false, EXT, id, 0));
                pins[pi] = [Pin::new(id, 1)];
                id += 1;
                plan.push((true, EMB, id, pi));
                lists.1[k] = id;
                pi += 1;
                id += 1;
            }
            if k < n_out {
                pins[pi] = [Pin::new(emb_in, 1)];
                plan.push((true, EXT, id, pi));
                pi += 1;
                id += 1;
            }
        }
        plan
    }

    #[test]
    fn prop_cable_count_topology_round_trip() {
        // REQ-USB-MS1-004, REQ-USB-MS1-023, REQ-USB-EVT-017, choice U-26: every cable-count
        // form passes the checks, wTotalLength is the written length, the reader gives the
        // same endpoints back, and Jack IDs 4k+1..4k+4 for equal counts.
        proptest!(|(n_out in 0usize..=16, n_in in 0usize..=16)| {
            prop_assume!(n_out + n_in > 0);
            let mut pins = [[Pin::default(); 1]; 32];
            let mut lists = ([0u8; 16], [0u8; 16]);
            let plan = cable_count(n_out, n_in, &mut pins, &mut lists);
            let mut t: Ms1Topology<'_, 64> = Ms1Topology::new();
            for &(is_out, jt, id, pi) in &plan {
                let jack = if is_out {
                    Jack::Out(MidiOutJack::new(jt, id, &pins[pi]))
                } else {
                    Jack::In(MidiInJack::new(jt, id))
                };
                t.add_jack(jack).unwrap();
            }
            if n_out > 0 {
                t.add_endpoint(Ms1Endpoint::new(EndpointDirection::MidiOut, 1, 64, &lists.0[..n_out])).unwrap();
            }
            if n_in > 0 {
                t.add_endpoint(Ms1Endpoint::new(EndpointDirection::MidiIn, 1, 64, &lists.1[..n_in])).unwrap();
            }
            let bytes = write_vec(&t);
            prop_assert_eq!(usize::from(u16::from_le_bytes([bytes[5], bytes[6]])), bytes.len());
            let eps: Vec<_> = read_all(&bytes).into_iter().filter_map(|d| match d {
                Ok(Ms1Desc::Endpoint(e)) => Some(e),
                _ => None,
            }).collect();
            let want: Vec<_> = t.endpoints().copied().collect();
            prop_assert_eq!(eps, want);
            prop_assert!(read_all(&bytes).iter().all(Result::is_ok));
            if n_out == n_in {
                for k in 0..n_out as u8 {
                    prop_assert_eq!(t.receive_route(4 * k + 1), Some(CableRoute { endpoint: 1, cable: k }));
                    prop_assert_eq!(t.send_route(4 * k + 3), Some(CableRoute { endpoint: 1, cable: k }));
                }
            }
        });
    }

    #[test]
    fn prop_reader_never_panics() {
        // Readers never panic on received bytes (U-24).
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..300))| {
            let mut n = 0;
            for item in Ms1Descriptors::new(&bytes) {
                n += 1;
                if let Ok(Ms1Desc::Endpoint(e)) = item {
                    prop_assert!(e.jacks.len() <= 16);
                    let _ = e.cable_map().jack(3);
                }
                if let Ok(Ms1Desc::OutJack(j)) = item {
                    prop_assert_eq!(j.pins.iter().count(), j.pins.len());
                }
            }
            prop_assert!(n <= bytes.len() * 4 + 1);
        });
    }

    #[test]
    fn prop_jack_round_trip() {
        // REQ-USB-MS1-010, REQ-USB-MS1-011: write then read gives the same Jack.
        proptest!(|(id in 1u8..=255, ext in any::<bool>(), s in any::<u8>(),
                    raw in proptest::collection::vec(any::<(u8, u8)>(), 1..=124))| {
            let t = if ext { EXT } else { EMB };
            let pins: Vec<Pin> = raw.iter().map(|&(a, b)| Pin::new(a, b)).collect();
            let mut j = MidiOutJack::new(t, id, &pins);
            j.string = NonZeroU8::new(s);
            let mut buf = [0u8; 255];
            let n = j.write(&mut buf).unwrap();
            prop_assert_eq!(n, 7 + 2 * pins.len());
            prop_assert_eq!(MidiOutJack::read(&one(&buf[..n])).unwrap(), j);
            let mut i = MidiInJack::new(t, id);
            i.string = NonZeroU8::new(s);
            let n = i.write(&mut buf).unwrap();
            prop_assert_eq!(MidiInJack::read(&one(&buf[..n])).unwrap(), i);
        });
    }
}
