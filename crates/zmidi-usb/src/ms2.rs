//! The descriptors of one USB MIDI 2.0 Alternate Setting, and the standard MIDI Streaming
//! interface descriptor of both versions (USB-MIDI v2.0 sections 3.3, 5.2, 5.3 and Appendix B;
//! USB-MIDI v1.0 section 6.1.1).
//!
//! - [`MsInterface`]: the standard MIDI Streaming interface descriptor, 9 bytes (Table 5-1, v1.0
//!   Table 6-1; REQ-USB-MS2-002, REQ-USB-MS2-003).
//! - [`Ms2Header`]: the class-specific MS Interface Header of a USB MIDI 2.0 setting, 7 bytes,
//!   bcdMSC 0x0200, wTotalLength 0x0007 (Table 5-2; REQ-USB-MS2-004 to REQ-USB-MS2-006).
//! - [`EndpointConfig`]: one MIDI Streaming Data Endpoint of the writer: the standard 7-byte
//!   endpoint descriptor (Table 5-3) and the class-specific MS_GENERAL_2_0 descriptor of 4 + n
//!   bytes (Table 5-4) (REQ-USB-MS2-007 to REQ-USB-MS2-009, REQ-USB-MS2-012, REQ-USB-MS2-016).
//! - [`Ms2Setting`]: the writer of a full USB MIDI 2.0 Alternate Setting (REQ-USB-GEN-008).
//! - [`read_setting`] and [`Ms2Endpoint`]: the lenient readers (choice U-24): a problem is a
//!   value, and the reader continues (REQ-USB-MS2-011, REQ-USB-MS2-013).
//! - [`GtbIds`]: the Group Terminal Block IDs of one endpoint, at most 16 (AMB-USB-MIDI-2-24).
//!
//! Every 2-byte field is little-endian (AMB-USB-MIDI-2-25). A writer fills a caller buffer and
//! returns the number of bytes, or [`Ms2Error::Overflow`] with nothing written. A reader works on
//! a caller slice and never panics on received bytes. No alloc (DEC-002), no I/O and no USB stack
//! (DEC-004, DEC-014).

use core::ops::Deref;

use crate::constants::{
    AUDIO, CS_ENDPOINT, CS_INTERFACE, ENDPOINT, INTERFACE, MIDISTREAMING, MS_GENERAL_2_0,
    MS_HEADER, MS_MIDI_2_0,
};
use crate::walk::{DescWriter, Descriptors, InterfaceGroup, RawDesc, WalkError};

/// The one endpoint direction type of the crate ([`crate::constants::EndpointDirection`]),
/// also here for the modules that use the endpoint types of Ms2.
pub use crate::constants::EndpointDirection;

/// The largest number of Group Terminal Block IDs of one endpoint (AMB-USB-MIDI-2-24,
/// REQ-USB-MS2-012).
pub const MAX_BLOCK_IDS: usize = 16;
/// bLength of the standard MIDI Streaming interface descriptor (Table 5-1).
pub const INTERFACE_LEN: usize = 9;
/// bLength of the USB MIDI 2.0 MS Interface Header (Table 5-2).
pub const HEADER_LEN: usize = 7;
/// bLength of the USB MIDI 2.0 standard endpoint descriptor (Table 5-3).
pub const ENDPOINT_LEN: usize = 7;
/// The largest number of setting-level problems that [`Ms2SettingInfo`] keeps.
pub const MAX_PROBLEMS: usize = 8;

/// Error of the Ms2 writers and readers. Each variant names the broken rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Ms2Error {
    /// The caller buffer is too small; nothing is written.
    #[error("buffer too small")]
    Overflow,
    /// A descriptor walk error: a bLength below 2 or past the end (REQ-USB-MS1-006).
    #[error("descriptor walk: {0}")]
    Walk(WalkError),
    /// A USB MIDI 2.0 setting needs Alternate Setting 1 or more (REQ-USB-MS2-002).
    #[error("USB MIDI 2.0 setting with Alternate Setting 0")]
    AlternateSettingZero,
    /// A USB MIDI 2.0 setting has no endpoint (REQ-USB-GEN-008).
    #[error("USB MIDI 2.0 setting with no endpoint")]
    NoEndpoint,
    /// More endpoints than the capacity of the list (REQ-USB-GEN-008).
    #[error("too many endpoints")]
    TooManyEndpoints,
    /// An endpoint number of 0 or above 15 (REQ-USB-MS2-007).
    #[error("endpoint number {0} not in 1 to 15")]
    EndpointNumber(u8),
    /// Two endpoints of one Alternate Setting with this bEndpointAddress (REQ-USB-MS2-017,
    /// USB 2.0 9.6.6).
    #[error("two endpoints with bEndpointAddress {0:#04x}")]
    DuplicateEndpoint(u8),
    /// An interrupt endpoint with bInterval 0 (REQ-USB-MS2-008).
    #[error("interrupt endpoint with bInterval 0")]
    IntervalZero,
    /// A transfer type that is not bulk or interrupt (REQ-USB-MS2-008).
    #[error("transfer type not bulk or interrupt")]
    TransferType,
    /// A wMaxPacketSize below 4 or not a multiple of 4; for a reader, a value below 4 after
    /// it is rounded down to a multiple of 4 (REQ-USB-MS2-009).
    #[error("wMaxPacketSize {0} is below 4 or not a multiple of 4")]
    MaxPacketSize(u16),
    /// A wMaxPacketSize that USB 2.0 does not permit for the transfer type and the bus speed
    /// (REQ-USB-MS2-016).
    #[error("wMaxPacketSize {0} not permitted at this bus speed")]
    MaxPacketForSpeed(u16),
    /// An endpoint with no Group Terminal Block ID (n = 0 or no class-specific endpoint
    /// descriptor) (REQ-USB-MS2-012, REQ-USB-MS2-013).
    #[error("endpoint with no Group Terminal Block ID")]
    NoBlockIds,
    /// More than 16 Group Terminal Block IDs on one endpoint (REQ-USB-MS2-012).
    #[error("more than 16 Group Terminal Block IDs")]
    TooManyBlockIds,
    /// A standard MIDI Streaming interface descriptor with bLength below 9 (REQ-USB-MS2-003).
    #[error("interface descriptor bLength below 9")]
    InterfaceTooShort,
    /// An MS Interface Header with bLength below 7 (REQ-USB-MS2-006).
    #[error("MS Interface Header bLength below 7")]
    HeaderTooShort,
    /// A standard endpoint descriptor with bLength below 7 (REQ-USB-MS2-011).
    #[error("endpoint descriptor bLength below 7")]
    EndpointTooShort,
    /// An MS_GENERAL_2_0 descriptor with bLength below 4 + n (REQ-USB-MS2-013).
    #[error("MS_GENERAL_2_0 bLength below 4 + n")]
    ClassEndpointTooShort,
    /// An MS_GENERAL_2_0 descriptor with no standard endpoint descriptor before it
    /// (REQ-USB-MS2-013, AMB-USB-MIDI-2-40).
    #[error("MS_GENERAL_2_0 with no endpoint before it")]
    ClassEndpointWithoutEndpoint,
}

impl From<WalkError> for Ms2Error {
    fn from(e: WalkError) -> Self {
        match e {
            WalkError::Overflow => Ms2Error::Overflow,
            other => Ms2Error::Walk(other),
        }
    }
}

/// A list of fixed capacity `N` (no alloc, DEC-002). It dereferences to a slice. Two lists are
/// equal when their items are equal.
#[derive(Clone, Copy)]
pub struct FixedList<T, const N: usize> {
    items: [T; N],
    len: usize,
}

impl<T: Copy + Default, const N: usize> FixedList<T, N> {
    /// An empty list.
    pub fn new() -> Self {
        FixedList {
            items: [T::default(); N],
            len: 0,
        }
    }

    /// A list with the items of `items`, or `None` if there are more than `N`.
    pub fn from_slice(items: &[T]) -> Option<Self> {
        let mut l = Self::new();
        l.items.get_mut(..items.len())?.copy_from_slice(items);
        l.len = items.len();
        Some(l)
    }

    /// Adds an item at the end; gives the item back when the list is full.
    pub fn push(&mut self, item: T) -> Result<(), T> {
        match self.items.get_mut(self.len) {
            Some(slot) => {
                *slot = item;
                self.len += 1;
                Ok(())
            }
            None => Err(item),
        }
    }

    /// Removes all items.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// The capacity `N`.
    pub const fn capacity(&self) -> usize {
        N
    }

    /// The items.
    // `len` is never above `N`; the `min` lets the compiler remove the bound check of each
    // inlined copy (OPT-043: run-time USB class -1,352 B flash).
    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len.min(N)]
    }

    /// The items, mutable.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.items[..self.len.min(N)]
    }
}

impl<T: Copy + Default, const N: usize> Default for FixedList<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: PartialEq, const N: usize> PartialEq for FixedList<T, N> {
    fn eq(&self, other: &Self) -> bool {
        self.items[..self.len] == other.items[..other.len]
    }
}

impl<T: Eq, const N: usize> Eq for FixedList<T, N> {}

impl<T: core::fmt::Debug, const N: usize> core::fmt::Debug for FixedList<T, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.items[..self.len].iter())
            .finish()
    }
}

impl<T: Copy + Default, const N: usize> Deref for FixedList<T, N> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

/// The Group Terminal Block IDs of one endpoint, in descriptor order (baAssoGrpTrmBlkID,
/// Table 5-4), at most 16 (REQ-USB-MS2-012, REQ-USB-MS2-013).
pub type GtbIds = FixedList<u8, MAX_BLOCK_IDS>;

impl GtbIds {
    /// One block ID. The default block of choice U-09 is `GtbIds::one(1)`.
    pub fn one(id: u8) -> Self {
        let mut l = Self::new();
        l.items[0] = id;
        l.len = 1;
        l
    }
}

/// The transfer type of a MIDI endpoint (bmAttributes D1..0; 5.3, REQ-USB-MS2-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TransferType {
    /// Bulk, 0b10 (the default, choice U-06).
    #[default]
    Bulk,
    /// Interrupt, 0b11.
    Interrupt,
    /// Control (0b00) or isochronous (0b01): kept by the reader, refused by the writer.
    Other(u8),
}

impl TransferType {
    /// The transfer type of bmAttributes (D1..0; D7..2 are ignored).
    pub const fn from_attributes(attributes: u8) -> Self {
        match attributes & 0x03 {
            0b10 => TransferType::Bulk,
            0b11 => TransferType::Interrupt,
            other => TransferType::Other(other),
        }
    }

    /// bmAttributes with D7..2 set to 0.
    pub const fn attributes(self) -> u8 {
        match self {
            TransferType::Bulk => 0b10,
            TransferType::Interrupt => 0b11,
            TransferType::Other(b) => b & 0x03,
        }
    }
}

/// The bus speed of the function, from the caller (REQ-USB-MS2-016).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BusSpeed {
    /// Full speed (12 Mb/s).
    #[default]
    Full,
    /// High speed (480 Mb/s).
    High,
}

impl BusSpeed {
    /// The default wMaxPacketSize: 64 at full speed, 512 at high speed (choices U-06, U-25).
    pub const fn default_max_packet(self) -> u16 {
        match self {
            BusSpeed::Full => 64,
            BusSpeed::High => 512,
        }
    }
}

/// Checks a wMaxPacketSize of the writer (REQ-USB-MS2-009, REQ-USB-MS2-016; USB 2.0 5.7.3,
/// 5.8.3).
pub fn check_max_packet(
    transfer: TransferType,
    speed: BusSpeed,
    max_packet: u16,
) -> Result<(), Ms2Error> {
    if max_packet < 4 || !max_packet.is_multiple_of(4) {
        return Err(Ms2Error::MaxPacketSize(max_packet));
    }
    let ok = match (transfer, speed) {
        (TransferType::Bulk, BusSpeed::Full) => matches!(max_packet, 8 | 16 | 32 | 64),
        (TransferType::Bulk, BusSpeed::High) => max_packet == 512,
        (TransferType::Interrupt, BusSpeed::Full) => max_packet <= 64,
        (TransferType::Interrupt, BusSpeed::High) => max_packet <= 1024,
        (TransferType::Other(_), _) => return Err(Ms2Error::TransferType),
    };
    if ok {
        Ok(())
    } else {
        Err(Ms2Error::MaxPacketForSpeed(max_packet))
    }
}

/// The wMaxPacketSize that a reader uses: rounded down to a multiple of 4, and an error for a
/// result below 4 (REQ-USB-MS2-009).
pub const fn usable_max_packet(max_packet: u16) -> Result<u16, Ms2Error> {
    let v = max_packet & !3;
    if v < 4 {
        Err(Ms2Error::MaxPacketSize(max_packet))
    } else {
        Ok(v)
    }
}

/// The standard MIDI Streaming interface descriptor, 9 bytes (USB-MIDI v2.0 Table 5-1, v1.0
/// Table 6-1; REQ-USB-MS2-002, REQ-USB-MS2-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MsInterface {
    /// bInterfaceNumber, from the caller (the USB stack gives it).
    pub interface_number: u8,
    /// bAlternateSetting: 0 for the USB MIDI 1.0 setting, 1 or more for a USB MIDI 2.0 setting.
    pub alternate_setting: u8,
    /// bNumEndpoints: the standard endpoint descriptors that follow in the setting.
    pub num_endpoints: u8,
    /// iInterface: 0 means no string.
    pub string_index: u8,
}

impl MsInterface {
    /// Writes the 9 bytes; bInterfaceClass 0x01, bInterfaceSubClass 0x03, bInterfaceProtocol
    /// 0x00 (REQ-USB-MS2-002).
    pub fn write(&self, buf: &mut [u8]) -> Result<usize, Ms2Error> {
        let bytes = [
            INTERFACE_LEN as u8,
            INTERFACE,
            self.interface_number,
            self.alternate_setting,
            self.num_endpoints,
            AUDIO,
            MIDISTREAMING,
            0x00,
            self.string_index,
        ];
        let dst = buf.get_mut(..INTERFACE_LEN).ok_or(Ms2Error::Overflow)?;
        dst.copy_from_slice(&bytes);
        Ok(INTERFACE_LEN)
    }

    /// Reads a standard MIDI Streaming interface descriptor (REQ-USB-MS2-003).
    ///
    /// Gives `None` when the descriptor is not one: bDescriptorType not 0x04, or bInterfaceClass
    /// not 0x01 or bInterfaceSubClass not 0x03 (a descriptor too short to hold them is not
    /// identified). Any bInterfaceProtocol is accepted. A bLength below 9 gives
    /// [`Ms2Error::InterfaceTooShort`].
    pub fn read(d: &RawDesc<'_>) -> Option<Result<MsInterface, Ms2Error>> {
        if d.dtype() != INTERFACE || d.byte(5)? != AUDIO || d.byte(6)? != MIDISTREAMING {
            return None;
        }
        if d.len() < INTERFACE_LEN {
            return Some(Err(Ms2Error::InterfaceTooShort));
        }
        Some(Ok(MsInterface {
            interface_number: d.byte(2)?,
            alternate_setting: d.byte(3)?,
            num_endpoints: d.byte(4)?,
            string_index: d.byte(8)?,
        }))
    }
}

/// The class-specific MS Interface Header (Table 5-2, REQ-USB-MS2-005, REQ-USB-MS2-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ms2Header {
    /// bcdMSC: 0x0200 for a USB MIDI 2.0 setting, 0x0100 for the USB MIDI 1.0 setting.
    pub bcd_msc: u16,
}

impl Default for Ms2Header {
    fn default() -> Self {
        Ms2Header {
            bcd_msc: MS_MIDI_2_0,
        }
    }
}

impl Ms2Header {
    /// The 7 bytes of the USB MIDI 2.0 header: bcdMSC 0x0200, wTotalLength 0x0007
    /// (REQ-USB-MS2-005).
    pub const BYTES: [u8; HEADER_LEN] = [0x07, CS_INTERFACE, MS_HEADER, 0x00, 0x02, 0x07, 0x00];

    /// Writes the USB MIDI 2.0 header, 7 bytes (REQ-USB-MS2-005).
    pub fn write(buf: &mut [u8]) -> Result<usize, Ms2Error> {
        let dst = buf.get_mut(..HEADER_LEN).ok_or(Ms2Error::Overflow)?;
        dst.copy_from_slice(&Self::BYTES);
        Ok(HEADER_LEN)
    }

    /// Reads an MS Interface Header of either version (REQ-USB-MS2-006).
    ///
    /// Gives `None` when the descriptor is not one (type 0x24, subtype 0x01). A bLength below 7
    /// gives [`Ms2Error::HeaderTooShort`]. wTotalLength is not used.
    pub fn read(d: &RawDesc<'_>) -> Option<Result<Ms2Header, Ms2Error>> {
        if d.dtype() != CS_INTERFACE || d.subtype()? != MS_HEADER {
            return None;
        }
        if d.len() < HEADER_LEN {
            return Some(Err(Ms2Error::HeaderTooShort));
        }
        Some(Ok(Ms2Header {
            bcd_msc: d.u16_le(3)?,
        }))
    }
}

/// One MIDI Streaming Data Endpoint of a USB MIDI 2.0 setting, for the writer
/// (REQ-USB-MS2-007, REQ-USB-MS2-008, REQ-USB-MS2-009, REQ-USB-MS2-012, REQ-USB-MS2-016).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointConfig {
    /// MidiOut is host to device, MidiIn is device to host.
    pub direction: EndpointDirection,
    /// The endpoint number, 1 to 15 (bEndpointAddress D3..0).
    pub number: u8,
    /// Bulk (default) or interrupt.
    pub transfer: TransferType,
    /// wMaxPacketSize: a multiple of 4, permitted for the bus speed.
    pub max_packet: u16,
    /// bInterval of an interrupt endpoint (1 or more); the writer writes 0 for bulk.
    pub interval: u8,
    /// The associated Group Terminal Block IDs, 1 to 16, in descriptor order.
    pub blocks: GtbIds,
}

impl Default for EndpointConfig {
    /// Bulk MIDI OUT endpoint 1, 64 bytes, Group Terminal Block 1 (choices U-06, U-09).
    fn default() -> Self {
        EndpointConfig::bulk(EndpointDirection::MidiOut, 1, BusSpeed::Full)
    }
}

impl EndpointConfig {
    /// A bulk endpoint with the default wMaxPacketSize of `speed` and Group Terminal Block 1
    /// (choices U-06, U-09, U-25).
    pub fn bulk(direction: EndpointDirection, number: u8, speed: BusSpeed) -> Self {
        EndpointConfig {
            direction,
            number,
            transfer: TransferType::Bulk,
            max_packet: speed.default_max_packet(),
            interval: 0,
            blocks: GtbIds::one(1),
        }
    }

    /// An interrupt endpoint with Group Terminal Block 1.
    pub fn interrupt(
        direction: EndpointDirection,
        number: u8,
        max_packet: u16,
        interval: u8,
    ) -> Self {
        EndpointConfig {
            direction,
            number,
            transfer: TransferType::Interrupt,
            max_packet,
            interval,
            blocks: GtbIds::one(1),
        }
    }

    /// The same endpoint with the Group Terminal Block IDs `ids` (refused later by the writer
    /// if empty; `None` above 16).
    pub fn with_blocks(mut self, ids: &[u8]) -> Option<Self> {
        self.blocks = GtbIds::from_slice(ids)?;
        Some(self)
    }

    /// bEndpointAddress: D7 direction, D6..4 zero, D3..0 number.
    pub const fn address(&self) -> u8 {
        self.direction.address_bit() | (self.number & 0x0F)
    }

    /// The direction (for the Group Terminal Block checks).
    pub const fn direction(&self) -> EndpointDirection {
        self.direction
    }

    /// The Group Terminal Block IDs (for the Group Terminal Block checks).
    pub fn block_ids(&self) -> &[u8] {
        &self.blocks
    }

    /// The bytes that [`Self::write`] writes: 7 + 4 + n.
    pub fn len(&self) -> usize {
        ENDPOINT_LEN + 4 + self.blocks.len()
    }

    /// Never true: the descriptors have at least 11 bytes.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Checks every writer rule of the endpoint (REQ-USB-MS2-007, REQ-USB-MS2-008,
    /// REQ-USB-MS2-009, REQ-USB-MS2-012, REQ-USB-MS2-016).
    pub fn validate(&self, speed: BusSpeed) -> Result<(), Ms2Error> {
        if self.number == 0 || self.number > 15 {
            return Err(Ms2Error::EndpointNumber(self.number));
        }
        if let TransferType::Other(_) = self.transfer {
            return Err(Ms2Error::TransferType);
        }
        if self.transfer == TransferType::Interrupt && self.interval == 0 {
            return Err(Ms2Error::IntervalZero);
        }
        check_max_packet(self.transfer, speed, self.max_packet)?;
        if self.blocks.is_empty() {
            return Err(Ms2Error::NoBlockIds);
        }
        Ok(())
    }

    /// Writes the standard endpoint descriptor (7 bytes, no bRefresh, no bSynchAddress) and
    /// directly after it the MS_GENERAL_2_0 descriptor (4 + n bytes) (REQ-USB-MS2-007,
    /// REQ-USB-MS2-012). Returns the number of bytes.
    pub fn write(&self, speed: BusSpeed, buf: &mut [u8]) -> Result<usize, Ms2Error> {
        self.validate(speed)?;
        let len = self.len();
        if buf.len() < len {
            return Err(Ms2Error::Overflow);
        }
        let interval = match self.transfer {
            TransferType::Interrupt => self.interval,
            _ => 0,
        };
        let mut w = DescWriter::new(buf);
        w.u8(ENDPOINT_LEN as u8)?;
        w.u8(ENDPOINT)?;
        w.u8(self.address())?;
        w.u8(self.transfer.attributes())?;
        w.u16_le(self.max_packet)?;
        w.u8(interval)?;
        // The length of `blocks` is at most 16, so 4 + n fits in a byte.
        w.u8((4 + self.blocks.len()) as u8)?;
        w.u8(CS_ENDPOINT)?;
        w.u8(MS_GENERAL_2_0)?;
        w.u8(self.blocks.len() as u8)?;
        w.bytes(&self.blocks)?;
        Ok(w.len())
    }
}

/// The writer of one USB MIDI 2.0 Alternate Setting: the interface descriptor, the header, and
/// for each endpoint the standard and the class-specific endpoint descriptors (REQ-USB-MS2-002,
/// REQ-USB-MS2-004, REQ-USB-GEN-008). `E` is the capacity of the endpoint list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ms2Setting<const E: usize> {
    /// bInterfaceNumber of the MIDI Streaming interface.
    pub interface_number: u8,
    /// bAlternateSetting: 1 or more.
    pub alternate_setting: u8,
    /// iInterface: 0 means no string.
    pub string_index: u8,
    /// The bus speed of the function.
    pub speed: BusSpeed,
    /// The endpoints, at least one.
    pub endpoints: FixedList<EndpointConfig, E>,
}

impl<const E: usize> Ms2Setting<E> {
    /// A setting with no endpoint (add them with `endpoints.push`).
    pub fn empty(interface_number: u8, alternate_setting: u8, speed: BusSpeed) -> Self {
        Ms2Setting {
            interface_number,
            alternate_setting,
            string_index: 0,
            speed,
            endpoints: FixedList::new(),
        }
    }

    /// The default setting (choice U-06): one bulk MIDI OUT endpoint 1 and one bulk MIDI IN
    /// endpoint 1, with the default wMaxPacketSize of `speed` and Group Terminal Block 1.
    /// [`Ms2Error::TooManyEndpoints`] if `E` is below 2.
    pub fn new(
        interface_number: u8,
        alternate_setting: u8,
        speed: BusSpeed,
    ) -> Result<Self, Ms2Error> {
        let mut s = Self::empty(interface_number, alternate_setting, speed);
        for dir in [EndpointDirection::MidiOut, EndpointDirection::MidiIn] {
            s.endpoints
                .push(EndpointConfig::bulk(dir, 1, speed))
                .map_err(|_| Ms2Error::TooManyEndpoints)?;
        }
        Ok(s)
    }

    /// The standard interface descriptor of the setting.
    pub fn interface(&self) -> Result<MsInterface, Ms2Error> {
        Ok(MsInterface {
            interface_number: self.interface_number,
            alternate_setting: self.alternate_setting,
            num_endpoints: u8::try_from(self.endpoints.len())
                .map_err(|_| Ms2Error::TooManyEndpoints)?,
            string_index: self.string_index,
        })
    }

    /// Checks every writer rule of the setting: Alternate Setting 1 or more, at least one
    /// endpoint, each endpoint valid, and no two endpoints with the same bEndpointAddress
    /// (REQ-USB-MS2-017).
    pub fn validate(&self) -> Result<(), Ms2Error> {
        if self.alternate_setting == 0 {
            return Err(Ms2Error::AlternateSettingZero);
        }
        if self.endpoints.is_empty() {
            return Err(Ms2Error::NoEndpoint);
        }
        self.interface()?;
        for (i, ep) in self.endpoints.iter().enumerate() {
            ep.validate(self.speed)?;
            // Each endpoint of one Alternate Setting has its own address (REQ-USB-MS2-017).
            let earlier = self.endpoints.get(..i).unwrap_or(&[]);
            if earlier.iter().any(|e| e.address() == ep.address()) {
                return Err(Ms2Error::DuplicateEndpoint(ep.address()));
            }
        }
        Ok(())
    }

    /// The bytes that [`Self::write`] writes.
    pub fn len(&self) -> usize {
        INTERFACE_LEN + HEADER_LEN + self.endpoints.iter().map(|e| e.len()).sum::<usize>()
    }

    /// Never true.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Writes the setting; returns the number of bytes, or an error with nothing written.
    pub fn write(&self, buf: &mut [u8]) -> Result<usize, Ms2Error> {
        self.validate()?;
        if buf.len() < self.len() {
            return Err(Ms2Error::Overflow);
        }
        let mut n = self.interface()?.write(buf)?;
        n += Ms2Header::write(buf.get_mut(n..).ok_or(Ms2Error::Overflow)?)?;
        for ep in self.endpoints.iter() {
            n += ep.write(self.speed, buf.get_mut(n..).ok_or(Ms2Error::Overflow)?)?;
        }
        Ok(n)
    }
}

/// One MIDI Streaming Data Endpoint as the reader found it (REQ-USB-MS2-011, REQ-USB-MS2-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ms2Endpoint {
    /// From bEndpointAddress D7.
    pub direction: EndpointDirection,
    /// From bEndpointAddress D3..0.
    pub number: u8,
    /// From bmAttributes D1..0.
    pub transfer: TransferType,
    /// wMaxPacketSize as written in the descriptor; see [`Self::packet_size`].
    pub max_packet: u16,
    /// bInterval of an interrupt endpoint; 0 for any other endpoint (ignored).
    pub interval: u8,
    /// The Group Terminal Block IDs of the MS_GENERAL_2_0 descriptors after it.
    pub blocks: GtbIds,
    /// The first problem found for this endpoint, if any (U-24).
    pub error: Option<Ms2Error>,
}

impl Ms2Endpoint {
    /// Reads a standard endpoint descriptor of 7 or more bytes (REQ-USB-MS2-008,
    /// REQ-USB-MS2-011). `blocks` is empty. Gives `None` when bDescriptorType is not 0x05, and
    /// [`Ms2Error::EndpointTooShort`] for a bLength below 7.
    pub fn read(d: &RawDesc<'_>) -> Option<Result<Ms2Endpoint, Ms2Error>> {
        if d.dtype() != ENDPOINT {
            return None;
        }
        if d.len() < ENDPOINT_LEN {
            return Some(Err(Ms2Error::EndpointTooShort));
        }
        let address = d.byte(2)?;
        let transfer = TransferType::from_attributes(d.byte(3)?);
        let interval = match transfer {
            TransferType::Interrupt => d.byte(6)?,
            _ => 0,
        };
        Some(Ok(Ms2Endpoint {
            direction: EndpointDirection::from_address(address),
            number: address & 0x0F,
            transfer,
            max_packet: d.u16_le(4)?,
            interval,
            blocks: GtbIds::new(),
            error: None,
        }))
    }

    /// The wMaxPacketSize to use: rounded down to a multiple of 4; an error below 4
    /// (REQ-USB-MS2-009).
    pub const fn packet_size(&self) -> Result<u16, Ms2Error> {
        usable_max_packet(self.max_packet)
    }

    /// bEndpointAddress without D6..4.
    pub const fn address(&self) -> u8 {
        self.direction.address_bit() | self.number
    }

    /// The direction (for the Group Terminal Block checks).
    pub const fn direction(&self) -> EndpointDirection {
        self.direction
    }

    /// The Group Terminal Block IDs (for the Group Terminal Block checks).
    pub fn block_ids(&self) -> &[u8] {
        &self.blocks
    }
}

/// Reads an MS_GENERAL_2_0 class-specific endpoint descriptor (REQ-USB-MS2-013).
///
/// Gives `None` when the descriptor is not one (type 0x25, subtype 0x02). Gives
/// [`Ms2Error::NoBlockIds`] for n = 0, [`Ms2Error::ClassEndpointTooShort`] for a bLength below
/// 4 + n, and [`Ms2Error::TooManyBlockIds`] for n above 16.
pub fn read_block_ids(d: &RawDesc<'_>) -> Option<Result<GtbIds, Ms2Error>> {
    if d.dtype() != CS_ENDPOINT || d.subtype()? != MS_GENERAL_2_0 {
        return None;
    }
    let Some(n) = d.byte(3) else {
        return Some(Err(Ms2Error::ClassEndpointTooShort));
    };
    let n = usize::from(n);
    if n == 0 {
        return Some(Err(Ms2Error::NoBlockIds));
    }
    let Some(ids) = d.bytes().get(4..4 + n) else {
        return Some(Err(Ms2Error::ClassEndpointTooShort));
    };
    Some(GtbIds::from_slice(ids).ok_or(Ms2Error::TooManyBlockIds))
}

/// What the reader found in one USB MIDI 2.0 Alternate Setting. `E` is the capacity of the
/// endpoint list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ms2SettingInfo<const E: usize> {
    /// The standard interface descriptor.
    pub interface: MsInterface,
    /// The MS Interface Header, if the setting has one.
    pub header: Option<Ms2Header>,
    /// The endpoints in descriptor order, each with its block IDs and its error.
    pub endpoints: FixedList<Ms2Endpoint, E>,
    problems: [Option<Ms2Error>; MAX_PROBLEMS],
    problem_count: usize,
}

impl<const E: usize> Ms2SettingInfo<E> {
    fn report(&mut self, e: Ms2Error) {
        if let Some(slot) = self.problems.get_mut(self.problem_count) {
            *slot = Some(e);
        }
        self.problem_count += 1;
    }

    /// The setting-level problems (U-24), at most [`MAX_PROBLEMS`]; endpoint problems are in
    /// [`Ms2Endpoint::error`].
    pub fn problems(&self) -> impl Iterator<Item = Ms2Error> + '_ {
        self.problems.iter().flatten().copied()
    }

    /// The number of setting-level problems found, also those above [`MAX_PROBLEMS`].
    pub fn problem_count(&self) -> usize {
        self.problem_count
    }

    /// True when the setting and all its endpoints have no problem.
    pub fn is_clean(&self) -> bool {
        self.problem_count == 0 && self.endpoints.iter().all(|e| e.error.is_none())
    }

    /// The Group Terminal Block IDs of the setting: all the IDs of its class-specific
    /// endpoint descriptors, in descriptor order (an ID can come more than once)
    /// (REQ-USB-MS2-013).
    pub fn block_ids(&self) -> impl Iterator<Item = u8> + '_ {
        self.endpoints.iter().flat_map(|e| e.blocks.iter().copied())
    }

    /// True when an endpoint of the setting names the block `id`.
    pub fn has_block_id(&self, id: u8) -> bool {
        self.block_ids().any(|b| b == id)
    }
}

/// Reads one USB MIDI 2.0 Alternate Setting from a group of [`crate::walk::interfaces`]
/// (REQ-USB-MS2-003, REQ-USB-MS2-006, REQ-USB-MS2-011, REQ-USB-MS2-013, REQ-USB-GEN-008).
///
/// Gives `None` when the interface descriptor is not a MIDI Streaming interface. The reader is
/// lenient (U-24): it reports each problem as a value and continues. It associates each
/// MS_GENERAL_2_0 descriptor with the standard endpoint descriptor before it, skips
/// descriptors it does not know, and reports a setting with no endpoint as
/// [`Ms2Error::NoEndpoint`]. The caller selects USB MIDI 2.0 settings by the header bcdMSC.
pub fn read_setting<const E: usize>(group: InterfaceGroup<'_>) -> Option<Ms2SettingInfo<E>> {
    let mut info = Ms2SettingInfo {
        interface: MsInterface::default(),
        header: None,
        endpoints: FixedList::new(),
        problems: [None; MAX_PROBLEMS],
        problem_count: 0,
    };
    match MsInterface::read(&group.interface)? {
        Ok(i) => info.interface = i,
        Err(e) => {
            info.report(e);
            let d = &group.interface;
            info.interface = MsInterface {
                interface_number: d.byte(2).unwrap_or(0),
                alternate_setting: d.byte(3).unwrap_or(0),
                num_endpoints: d.byte(4).unwrap_or(0),
                string_index: d.byte(8).unwrap_or(0),
            };
        }
    }

    enum Current {
        None,
        Kept(usize),
        Dropped,
    }
    let mut current = Current::None;
    for item in Descriptors::new(group.body) {
        let d = match item {
            Ok(d) => d,
            Err(e) => {
                info.report(Ms2Error::from(e));
                break;
            }
        };
        if let Some(h) = Ms2Header::read(&d) {
            match h {
                Ok(h) if info.header.is_none() => info.header = Some(h),
                Ok(_) => {}
                Err(e) => info.report(e),
            }
        } else if let Some(ep) = Ms2Endpoint::read(&d) {
            match ep {
                Ok(mut ep) => {
                    if let Err(e) = ep.packet_size() {
                        ep.error = Some(e);
                    }
                    match info.endpoints.push(ep) {
                        Ok(()) => current = Current::Kept(info.endpoints.len() - 1),
                        Err(_) => {
                            info.report(Ms2Error::TooManyEndpoints);
                            current = Current::Dropped;
                        }
                    }
                }
                Err(e) => {
                    info.report(e);
                    current = Current::Dropped;
                }
            }
        } else if let Some(ids) = read_block_ids(&d) {
            match current {
                Current::None => info.report(Ms2Error::ClassEndpointWithoutEndpoint),
                Current::Dropped => {}
                Current::Kept(i) => {
                    if let Some(ep) = info.endpoints.as_mut_slice().get_mut(i) {
                        match ids {
                            Ok(ids) => {
                                for id in ids.iter() {
                                    if ep.blocks.push(*id).is_err() {
                                        ep.error.get_or_insert(Ms2Error::TooManyBlockIds);
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                ep.error.get_or_insert(e);
                            }
                        }
                    }
                }
            }
        }
    }
    for ep in info.endpoints.as_mut_slice() {
        if ep.blocks.is_empty() {
            ep.error.get_or_insert(Ms2Error::NoBlockIds);
        }
    }
    if info.endpoints.is_empty() && !matches!(current, Current::Dropped) {
        info.report(Ms2Error::NoEndpoint);
    }
    Some(info)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::walk::interfaces;
    use proptest::prelude::*;
    use std::vec::Vec;

    /// Tables B-15 to B-20 (USB-MIDI v2.0 Appendix B): Alternate Setting 1 of the example.
    const B15: [u8; 9] = [0x09, 0x04, 0x01, 0x01, 0x02, 0x01, 0x03, 0x00, 0x00];
    const B16: [u8; 7] = [0x07, 0x24, 0x01, 0x00, 0x02, 0x07, 0x00];
    const B17: [u8; 7] = [0x07, 0x05, 0x01, 0x02, 0x40, 0x00, 0x00];
    const B18: [u8; 5] = [0x05, 0x25, 0x02, 0x01, 0x01];
    const B19: [u8; 7] = [0x07, 0x05, 0x81, 0x03, 0x40, 0x00, 0x01];
    const B20: [u8; 5] = [0x05, 0x25, 0x02, 0x01, 0x01];

    fn desc(bytes: &[u8]) -> RawDesc<'_> {
        Descriptors::new(bytes).next().unwrap().unwrap()
    }

    fn example_setting() -> Ms2Setting<4> {
        let mut s = Ms2Setting::<4>::empty(1, 1, BusSpeed::Full);
        s.endpoints
            .push(EndpointConfig::bulk(
                EndpointDirection::MidiOut,
                1,
                BusSpeed::Full,
            ))
            .unwrap();
        s.endpoints
            .push(EndpointConfig::interrupt(
                EndpointDirection::MidiIn,
                1,
                64,
                1,
            ))
            .unwrap();
        s
    }

    fn example_bytes() -> Vec<u8> {
        [&B15[..], &B16, &B17, &B18, &B19, &B20].concat()
    }

    #[test]
    fn interface_descriptor_table_b15_round_trip() {
        // REQ-USB-VEC2-007, REQ-USB-MS2-002, REQ-USB-MS2-003.
        let i = MsInterface {
            interface_number: 1,
            alternate_setting: 1,
            num_endpoints: 2,
            string_index: 0,
        };
        let mut buf = [0u8; 9];
        assert_eq!(i.write(&mut buf), Ok(9));
        assert_eq!(buf, B15);
        assert_eq!(MsInterface::read(&desc(&B15)), Some(Ok(i)));
    }

    #[test]
    fn interface_writer_alternate_setting_zero_for_midi1() {
        // REQ-USB-MS2-002: the same 9 bytes, Alternate Setting 0 for the USB MIDI 1.0 setting,
        // with a string index from the caller.
        let i = MsInterface {
            interface_number: 1,
            alternate_setting: 0,
            num_endpoints: 2,
            string_index: 5,
        };
        let mut buf = [0u8; 9];
        i.write(&mut buf).unwrap();
        assert_eq!(buf, [0x09, 0x04, 0x01, 0x00, 0x02, 0x01, 0x03, 0x00, 0x05]);
        let mut small = [0xAAu8; 8];
        assert_eq!(i.write(&mut small), Err(Ms2Error::Overflow));
        assert_eq!(small, [0xAA; 8]);
    }

    #[test]
    fn interface_reader_is_lenient() {
        // REQ-USB-MS2-003: any bInterfaceProtocol, bLength 9 or more; below 9 is an error;
        // not class 0x01 subclass 0x03 is not identified.
        let mut long = [0u8; 11];
        long[..9].copy_from_slice(&B15);
        long[0] = 11;
        long[7] = 0x42;
        let i = MsInterface::read(&desc(&long)).unwrap().unwrap();
        assert_eq!(
            (i.interface_number, i.alternate_setting, i.num_endpoints),
            (1, 1, 2)
        );
        let mut short = B15;
        short[0] = 8;
        assert_eq!(
            MsInterface::read(&desc(&short[..8])),
            Some(Err(Ms2Error::InterfaceTooShort))
        );
        let mut ac = B15;
        ac[6] = 0x01;
        assert_eq!(MsInterface::read(&desc(&ac)), None);
        assert_eq!(MsInterface::read(&desc(&B16)), None);
    }

    #[test]
    fn header_table_b16_round_trip() {
        // REQ-USB-VEC2-008, REQ-USB-MS2-005, REQ-USB-MS2-006.
        let mut buf = [0u8; 7];
        assert_eq!(Ms2Header::write(&mut buf), Ok(7));
        assert_eq!(buf, B16);
        assert_eq!(
            Ms2Header::read(&desc(&B16)),
            Some(Ok(Ms2Header { bcd_msc: 0x0200 }))
        );
        assert_eq!(Ms2Header::write(&mut [0u8; 6]), Err(Ms2Error::Overflow));
    }

    #[test]
    fn header_reader_accepts_any_total_length_and_longer_blength() {
        // REQ-USB-MS2-006: bLength 7 or more, bcdMSC little-endian, wTotalLength not used.
        let h1 = [0x07, 0x24, 0x01, 0x00, 0x01, 0x41, 0x00];
        assert_eq!(
            Ms2Header::read(&desc(&h1)),
            Some(Ok(Ms2Header { bcd_msc: 0x0100 }))
        );
        let h = [0x08, 0x24, 0x01, 0x00, 0x02, 0xFF, 0xFF, 0x00];
        assert_eq!(
            Ms2Header::read(&desc(&h)),
            Some(Ok(Ms2Header { bcd_msc: 0x0200 }))
        );
        let short = [0x06, 0x24, 0x01, 0x00, 0x02, 0x07];
        assert_eq!(
            Ms2Header::read(&desc(&short)),
            Some(Err(Ms2Error::HeaderTooShort))
        );
        assert_eq!(Ms2Header::read(&desc(&[0x06, 0x24, 0x02, 1, 1, 0])), None);
    }

    #[test]
    fn bulk_out_endpoint_tables_b17_b18_round_trip() {
        // REQ-USB-VEC2-009, REQ-USB-MS2-007, REQ-USB-MS2-008, REQ-USB-MS2-012.
        let ep = EndpointConfig::bulk(EndpointDirection::MidiOut, 1, BusSpeed::Full);
        let mut buf = [0u8; 12];
        assert_eq!(ep.write(BusSpeed::Full, &mut buf), Ok(12));
        assert_eq!(&buf[..7], &B17);
        assert_eq!(&buf[7..], &B18);
        let r = Ms2Endpoint::read(&desc(&B17)).unwrap().unwrap();
        assert_eq!(
            (r.direction, r.number, r.transfer, r.max_packet, r.interval),
            (EndpointDirection::MidiOut, 1, TransferType::Bulk, 64, 0)
        );
        assert_eq!(read_block_ids(&desc(&B18)), Some(Ok(GtbIds::one(1))));
    }

    #[test]
    fn interrupt_in_endpoint_tables_b19_b20_round_trip() {
        // REQ-USB-VEC2-010, REQ-USB-MS2-007, REQ-USB-MS2-008, REQ-USB-MS2-012.
        let ep = EndpointConfig::interrupt(EndpointDirection::MidiIn, 1, 64, 1);
        let mut buf = [0u8; 12];
        assert_eq!(ep.write(BusSpeed::Full, &mut buf), Ok(12));
        assert_eq!(&buf[..7], &B19);
        assert_eq!(&buf[7..], &B20);
        let r = Ms2Endpoint::read(&desc(&B19)).unwrap().unwrap();
        assert_eq!(
            (r.direction, r.number, r.transfer, r.max_packet, r.interval),
            (EndpointDirection::MidiIn, 1, TransferType::Interrupt, 64, 1)
        );
        assert_eq!(read_block_ids(&desc(&B20)), Some(Ok(GtbIds::one(1))));
    }

    #[test]
    fn endpoint_writer_refuses_bad_number() {
        // REQ-USB-MS2-007: endpoint number 1 to 15.
        let mut buf = [0u8; 32];
        for n in [0u8, 16, 0x81] {
            let ep = EndpointConfig::bulk(EndpointDirection::MidiIn, n, BusSpeed::Full);
            assert_eq!(
                ep.write(BusSpeed::Full, &mut buf),
                Err(Ms2Error::EndpointNumber(n))
            );
        }
        let ep = EndpointConfig::bulk(EndpointDirection::MidiIn, 15, BusSpeed::Full);
        ep.write(BusSpeed::Full, &mut buf).unwrap();
        assert_eq!(buf[2], 0x8F);
    }

    #[test]
    fn endpoint_writer_bulk_interval_zero_and_interrupt_interval() {
        // REQ-USB-MS2-008: bulk writes bInterval 0; interrupt refuses 0; bmAttributes D7..2 = 0.
        let mut ep = EndpointConfig::bulk(EndpointDirection::MidiOut, 2, BusSpeed::Full);
        ep.interval = 9;
        let mut buf = [0u8; 12];
        ep.write(BusSpeed::Full, &mut buf).unwrap();
        assert_eq!((buf[3], buf[6]), (0x02, 0));
        let ep = EndpointConfig::interrupt(EndpointDirection::MidiIn, 2, 8, 0);
        assert_eq!(
            ep.write(BusSpeed::Full, &mut buf),
            Err(Ms2Error::IntervalZero)
        );
        let mut ep = EndpointConfig::bulk(EndpointDirection::MidiOut, 2, BusSpeed::Full);
        ep.transfer = TransferType::Other(1);
        assert_eq!(
            ep.write(BusSpeed::Full, &mut buf),
            Err(Ms2Error::TransferType)
        );
    }

    #[test]
    fn endpoint_reader_accepts_both_transfers_and_ignores_bulk_interval() {
        // REQ-USB-MS2-008, REQ-USB-MS2-011: 9-byte form, D6..4 and bmAttributes D7..2 and
        // bRefresh/bSynchAddress ignored; bulk bInterval ignored.
        let nine = [0x09, 0x05, 0xF3, 0xFE, 0x00, 0x02, 0x07, 0x05, 0x06];
        let r = Ms2Endpoint::read(&desc(&nine)).unwrap().unwrap();
        assert_eq!(
            (r.direction, r.number, r.transfer, r.max_packet, r.interval),
            (EndpointDirection::MidiIn, 3, TransferType::Bulk, 512, 0)
        );
        let short = [0x06, 0x05, 0x01, 0x02, 0x40, 0x00];
        assert_eq!(
            Ms2Endpoint::read(&desc(&short)),
            Some(Err(Ms2Error::EndpointTooShort))
        );
        assert_eq!(Ms2Endpoint::read(&desc(&B18)), None);
    }

    #[test]
    fn max_packet_multiple_of_four_and_speed_limits() {
        // REQ-USB-MS2-009, REQ-USB-MS2-016.
        use BusSpeed::*;
        use TransferType::*;
        for v in [8u16, 16, 32, 64] {
            assert_eq!(check_max_packet(Bulk, Full, v), Ok(()));
        }
        assert_eq!(
            check_max_packet(Bulk, Full, 48),
            Err(Ms2Error::MaxPacketForSpeed(48))
        );
        assert_eq!(
            check_max_packet(Bulk, Full, 512),
            Err(Ms2Error::MaxPacketForSpeed(512))
        );
        assert_eq!(check_max_packet(Bulk, High, 512), Ok(()));
        assert_eq!(
            check_max_packet(Bulk, High, 64),
            Err(Ms2Error::MaxPacketForSpeed(64))
        );
        assert_eq!(check_max_packet(Interrupt, Full, 4), Ok(()));
        assert_eq!(
            check_max_packet(Interrupt, Full, 68),
            Err(Ms2Error::MaxPacketForSpeed(68))
        );
        assert_eq!(check_max_packet(Interrupt, High, 1024), Ok(()));
        assert_eq!(
            check_max_packet(Interrupt, High, 1028),
            Err(Ms2Error::MaxPacketForSpeed(1028))
        );
        for v in [0u16, 2, 6, 10] {
            assert_eq!(
                check_max_packet(Interrupt, High, v),
                Err(Ms2Error::MaxPacketSize(v))
            );
        }
        let mut ep = EndpointConfig::bulk(EndpointDirection::MidiOut, 1, Full);
        ep.max_packet = 30;
        assert_eq!(
            ep.write(Full, &mut [0u8; 16]),
            Err(Ms2Error::MaxPacketSize(30))
        );
        // Defaults (U-06, U-25).
        assert_eq!(
            EndpointConfig::bulk(EndpointDirection::MidiIn, 1, Full).max_packet,
            64
        );
        assert_eq!(
            EndpointConfig::bulk(EndpointDirection::MidiIn, 1, High).max_packet,
            512
        );
    }

    #[test]
    fn reader_rounds_max_packet_down() {
        // REQ-USB-MS2-009: rounded down to a multiple of 4, an error below 4.
        assert_eq!(usable_max_packet(64), Ok(64));
        assert_eq!(usable_max_packet(63), Ok(60));
        assert_eq!(usable_max_packet(4), Ok(4));
        assert_eq!(usable_max_packet(3), Err(Ms2Error::MaxPacketSize(3)));
        assert_eq!(usable_max_packet(0), Err(Ms2Error::MaxPacketSize(0)));
        let mut bytes = example_bytes();
        bytes[9 + 7 + 4] = 2; // OUT endpoint wMaxPacketSize 2
        let g = interfaces(&bytes).next().unwrap();
        let info = read_setting::<4>(g).unwrap();
        assert_eq!(info.endpoints[0].error, Some(Ms2Error::MaxPacketSize(2)));
        assert_eq!(info.endpoints[1].error, None);
    }

    #[test]
    fn class_endpoint_writes_block_ids_in_caller_order() {
        // REQ-USB-MS2-012: 4 + n bytes, IDs from offset 4 in caller order; n = 0 and n > 16
        // refused.
        let ep = EndpointConfig::bulk(EndpointDirection::MidiOut, 1, BusSpeed::Full)
            .with_blocks(&[3, 1, 2])
            .unwrap();
        let mut buf = [0u8; 14];
        assert_eq!(ep.write(BusSpeed::Full, &mut buf), Ok(14));
        assert_eq!(&buf[7..], &[0x07, 0x25, 0x02, 0x03, 3, 1, 2]);
        let all: Vec<u8> = (1..=16).collect();
        let ep16 = ep.with_blocks(&all).unwrap();
        assert_eq!(ep16.write(BusSpeed::Full, &mut [0u8; 27]), Ok(27));
        assert!(ep.with_blocks(&[0u8; 17]).is_none());
        let empty = ep.with_blocks(&[]).unwrap();
        assert_eq!(
            empty.write(BusSpeed::Full, &mut buf),
            Err(Ms2Error::NoBlockIds)
        );
        let mut small = [0xAAu8; 13];
        assert_eq!(
            ep.write(BusSpeed::Full, &mut small),
            Err(Ms2Error::Overflow)
        );
        assert_eq!(small, [0xAA; 13]);
    }

    #[test]
    fn class_endpoint_reader_errors() {
        // REQ-USB-MS2-013: n = 0 or bLength below 4 + n is an error.
        assert_eq!(
            read_block_ids(&desc(&[0x04, 0x25, 0x02, 0x00])),
            Some(Err(Ms2Error::NoBlockIds))
        );
        assert_eq!(
            read_block_ids(&desc(&[0x05, 0x25, 0x02, 0x02, 0x01])),
            Some(Err(Ms2Error::ClassEndpointTooShort))
        );
        assert_eq!(
            read_block_ids(&desc(&[0x03, 0x25, 0x02])),
            Some(Err(Ms2Error::ClassEndpointTooShort))
        );
        assert_eq!(read_block_ids(&desc(&[0x05, 0x25, 0x01, 0x01, 0x01])), None);
        let mut big = [0u8; 21];
        big[..4].copy_from_slice(&[21, 0x25, 0x02, 17]);
        assert_eq!(
            read_block_ids(&desc(&big)),
            Some(Err(Ms2Error::TooManyBlockIds))
        );
        // A longer bLength is accepted; the IDs are the first n bytes from offset 4.
        assert_eq!(
            read_block_ids(&desc(&[0x07, 0x25, 0x02, 0x02, 4, 5, 9])),
            Some(Ok(GtbIds::from_slice(&[4, 5]).unwrap()))
        );
    }

    #[test]
    fn setting_writer_gives_appendix_b_bytes() {
        // REQ-USB-VEC2-007 to REQ-USB-VEC2-010, REQ-USB-MS2-002, REQ-USB-MS2-004: interface,
        // header only (no Jack, no Element), then each endpoint pair; bNumEndpoints = 2.
        let s = example_setting();
        let mut buf = [0u8; 64];
        let n = s.write(&mut buf).unwrap();
        assert_eq!(&buf[..n], &example_bytes()[..]);
        assert_eq!(n, s.len());
        let mut small = [0xAAu8; 39];
        assert_eq!(s.write(&mut small), Err(Ms2Error::Overflow));
        assert_eq!(small, [0xAA; 39]);
    }

    #[test]
    fn setting_default_is_bulk_out_and_bulk_in() {
        // REQ-USB-GEN-008, choice U-06.
        let s = Ms2Setting::<2>::new(1, 1, BusSpeed::High).unwrap();
        let e = s.endpoints.as_slice();
        assert_eq!(e.len(), 2);
        assert_eq!(
            (e[0].direction, e[0].transfer, e[0].max_packet),
            (EndpointDirection::MidiOut, TransferType::Bulk, 512)
        );
        assert_eq!(
            (e[1].direction, e[1].address()),
            (EndpointDirection::MidiIn, 0x81)
        );
        assert_eq!(e[1].block_ids(), &[1]);
        let mut buf = [0u8; 64];
        let n = s.write(&mut buf).unwrap();
        assert_eq!(n, 9 + 7 + 2 * 12);
        assert_eq!(&buf[28..33], &[0x07, 0x05, 0x81, 0x02, 0x00]);
        assert_eq!(buf[33..35], [0x02, 0x00]);
        assert_eq!(
            Ms2Setting::<1>::new(1, 1, BusSpeed::Full),
            Err(Ms2Error::TooManyEndpoints)
        );
    }

    #[test]
    fn setting_writer_refuses_no_endpoint_and_alternate_setting_zero() {
        // REQ-USB-GEN-008, REQ-USB-MS2-002.
        let mut buf = [0u8; 64];
        let s = Ms2Setting::<2>::empty(1, 1, BusSpeed::Full);
        assert_eq!(s.write(&mut buf), Err(Ms2Error::NoEndpoint));
        let mut s = example_setting();
        s.alternate_setting = 0;
        assert_eq!(s.write(&mut buf), Err(Ms2Error::AlternateSettingZero));
        let mut s = example_setting();
        s.endpoints.as_mut_slice()[1].number = 0;
        assert_eq!(s.write(&mut buf), Err(Ms2Error::EndpointNumber(0)));
    }

    // REQ-USB-MS2-017: two endpoints with the same bEndpointAddress in one USB MIDI 2.0
    // setting are refused, and nothing is written; the same number in the other direction is
    // another address and is accepted.
    #[test]
    fn setting_writer_refuses_two_endpoints_with_one_address() {
        let mut s = Ms2Setting::<3>::empty(1, 1, BusSpeed::Full);
        for ids in [[1u8], [2]] {
            let ep = EndpointConfig::bulk(EndpointDirection::MidiOut, 1, BusSpeed::Full)
                .with_blocks(&ids)
                .unwrap();
            s.endpoints.push(ep).unwrap();
        }
        assert_eq!(s.validate(), Err(Ms2Error::DuplicateEndpoint(0x01)));
        let mut buf = [0xAAu8; 64];
        assert_eq!(s.write(&mut buf), Err(Ms2Error::DuplicateEndpoint(0x01)));
        assert!(buf.iter().all(|&b| b == 0xAA));
        // The same for two MIDI IN endpoints 1 (interrupt and interrupt).
        let mut s = example_setting();
        s.endpoints.as_mut_slice()[0] =
            EndpointConfig::interrupt(EndpointDirection::MidiIn, 1, 64, 1);
        assert_eq!(s.validate(), Err(Ms2Error::DuplicateEndpoint(0x81)));
        // OUT 1 and IN 1 are two addresses (Appendix B).
        assert_eq!(example_setting().validate(), Ok(()));
    }

    #[test]
    fn setting_reader_reads_appendix_b() {
        // REQ-USB-VEC2-007 to REQ-USB-VEC2-010, REQ-USB-MS2-003, REQ-USB-MS2-006,
        // REQ-USB-MS2-011, REQ-USB-MS2-013.
        let bytes = example_bytes();
        let g = interfaces(&bytes).next().unwrap();
        let info = read_setting::<4>(g).unwrap();
        assert!(info.is_clean());
        assert_eq!(info.interface.num_endpoints, 2);
        assert_eq!(info.header, Some(Ms2Header { bcd_msc: 0x0200 }));
        assert_eq!(info.endpoints.len(), 2);
        assert_eq!(info.endpoints[0].direction(), EndpointDirection::MidiOut);
        assert_eq!(info.endpoints[1].direction(), EndpointDirection::MidiIn);
        assert_eq!(info.endpoints[1].interval, 1);
        assert_eq!(info.endpoints[1].block_ids(), &[1]);
        assert_eq!(info.block_ids().collect::<Vec<_>>(), [1, 1]);
        assert!(info.has_block_id(1) && !info.has_block_id(2));
    }

    #[test]
    fn setting_reader_associates_block_ids_and_continues_after_errors() {
        // REQ-USB-MS2-013: each MS_GENERAL_2_0 goes to the endpoint before it; an error is
        // reported for that endpoint and the reader continues; the block IDs of the setting are
        // all the IDs.
        let bytes = [
            &B15[..],
            &B16,
            &[0x06, 0x24, 0x02, 0x01, 0x01, 0x00], // a Jack: skipped
            &B17,
            &[0x04, 0x25, 0x02, 0x00], // n = 0
            &[0x07, 0x05, 0x82, 0x02, 0x40, 0x00, 0x00],
            &[0x06, 0x25, 0x02, 0x02, 0x02, 0x03],
            &[0x05, 0x26, 0x01, 0x05, 0x00], // inline block header: skipped
        ]
        .concat();
        let g = interfaces(&bytes).next().unwrap();
        let info = read_setting::<4>(g).unwrap();
        assert_eq!(info.problem_count(), 0);
        assert_eq!(info.endpoints[0].error, Some(Ms2Error::NoBlockIds));
        assert_eq!(info.endpoints[1].error, None);
        assert_eq!(info.endpoints[1].number, 2);
        assert_eq!(info.endpoints[1].block_ids(), &[2, 3]);
        assert_eq!(info.block_ids().collect::<Vec<_>>(), [2, 3]);
        assert!(!info.is_clean());
    }

    #[test]
    fn setting_reader_reports_orphan_class_endpoint_and_missing_one() {
        // REQ-USB-MS2-013 (AMB-USB-MIDI-2-40): MS_GENERAL_2_0 with no endpoint before it.
        let bytes = [&B15[..], &B16, &B18, &B17].concat();
        let g = interfaces(&bytes).next().unwrap();
        let info = read_setting::<4>(g).unwrap();
        assert_eq!(
            info.problems().collect::<Vec<_>>(),
            [Ms2Error::ClassEndpointWithoutEndpoint]
        );
        assert_eq!(info.endpoints[0].error, Some(Ms2Error::NoBlockIds));
    }

    #[test]
    fn setting_reader_reports_no_endpoint() {
        // REQ-USB-GEN-008: a setting with no endpoint is an error for the reader.
        let bytes = [&B15[..], &B16].concat();
        let g = interfaces(&bytes).next().unwrap();
        let info = read_setting::<4>(g).unwrap();
        assert_eq!(info.problems().collect::<Vec<_>>(), [Ms2Error::NoEndpoint]);
    }

    #[test]
    fn setting_reader_capacity_and_non_ms_interface() {
        // REQ-USB-GEN-008: fixed capacity; extra endpoints are reported, not stored.
        let bytes = example_bytes();
        let g = interfaces(&bytes).next().unwrap();
        let info = read_setting::<1>(g).unwrap();
        assert_eq!(info.endpoints.len(), 1);
        assert_eq!(
            info.problems().collect::<Vec<_>>(),
            [Ms2Error::TooManyEndpoints]
        );
        // REQ-USB-MS2-003: an AudioControl interface is not read.
        let ac = [0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00];
        assert!(read_setting::<2>(interfaces(&ac).next().unwrap()).is_none());
    }

    #[test]
    fn prop_endpoint_write_read_round_trip() {
        // REQ-USB-MS2-007, REQ-USB-MS2-008, REQ-USB-MS2-011, REQ-USB-MS2-012, REQ-USB-MS2-013.
        proptest!(|(number in 1u8..=15, is_in: bool, interrupt: bool, mp in 1u16..=16,
                    interval in 1u8..=255, ids in proptest::collection::vec(any::<u8>(), 1..=16))| {
            let direction = if is_in { EndpointDirection::MidiIn } else { EndpointDirection::MidiOut };
            let ep = if interrupt {
                EndpointConfig::interrupt(direction, number, mp * 4, interval)
            } else {
                EndpointConfig::bulk(direction, number, BusSpeed::Full)
            }.with_blocks(&ids).unwrap();
            let mut buf = [0u8; 32];
            let n = ep.write(BusSpeed::Full, &mut buf).unwrap();
            prop_assert_eq!(n, 11 + ids.len());
            let mut d = Descriptors::new(&buf[..n]);
            let r = Ms2Endpoint::read(&d.next().unwrap().unwrap()).unwrap().unwrap();
            prop_assert_eq!(r.direction, ep.direction);
            prop_assert_eq!(r.number, number);
            prop_assert_eq!(r.transfer, ep.transfer);
            prop_assert_eq!(r.max_packet, ep.max_packet);
            prop_assert_eq!(r.interval, if interrupt { interval } else { 0 });
            let b = read_block_ids(&d.next().unwrap().unwrap()).unwrap().unwrap();
            prop_assert_eq!(b.as_slice(), &ids[..]);
        });
    }

    #[test]
    fn prop_readers_never_panic() {
        // U-24: the readers never panic on received bytes.
        proptest!(|(body in proptest::collection::vec(any::<u8>(), 0..200))| {
            let bytes = [&B15[..], &body].concat();
            for g in interfaces(&bytes) {
                if let Some(info) = read_setting::<4>(g) {
                    prop_assert!(info.endpoints.len() <= 4);
                    let _ = info.block_ids().count();
                }
            }
            for d in Descriptors::new(&body).flatten() {
                let _ = (MsInterface::read(&d), Ms2Header::read(&d), Ms2Endpoint::read(&d), read_block_ids(&d));
            }
        });
    }

    #[test]
    fn ms2_values_at_their_limits() {
        // FixedList: capacity, equality by the used items, Debug of the used items.
        let mut a = GtbIds::one(1);
        assert_eq!(a.capacity(), MAX_BLOCK_IDS);
        let b = GtbIds::one(2);
        assert_ne!(a, b);
        a.push(2).unwrap();
        assert_ne!(a, GtbIds::one(1));
        assert_eq!(std::format!("{a:?}"), "[1, 2]");
        // bmAttributes D1..0 of a kept transfer type.
        assert_eq!(TransferType::Other(0x05).attributes(), 0x01);
        // An interface of another class with subclass 0x03 is no MIDI Streaming interface.
        let other = [0x09, 0x04, 0x01, 0x01, 0x02, 0xFF, 0x03, 0x00, 0x00];
        let d = crate::walk::Descriptors::new(&other)
            .next()
            .unwrap()
            .unwrap();
        assert!(MsInterface::read(&d).is_none());
        // The writer fills a buffer of exactly its length.
        let s = Ms2Setting::<2>::new(1, 1, BusSpeed::Full).unwrap();
        assert!(!s.is_empty());
        assert!(!s.endpoints[0].is_empty());
        let mut buf = std::vec![0u8; s.len()];
        assert_eq!(s.write(&mut buf), Ok(s.len()));
    }

    #[test]
    fn setting_reader_keeps_the_first_header_and_counts_problems() {
        // REQ-USB-MS2-003: the first class header of the setting; each short one is a problem.
        let short = [0x06, 0x24, 0x01, 0x00, 0x02, 0x07];
        let second = [0x07, 0x24, 0x01, 0x00, 0x01, 0x07, 0x00];
        let base = [&B15[..], &B16].concat();
        let n0 = read_setting::<4>(interfaces(&base).next().unwrap())
            .unwrap()
            .problem_count();
        let bytes = [&B15[..], &short, &B16, &second, &short].concat();
        let g = interfaces(&bytes).next().unwrap();
        let info = read_setting::<4>(g).unwrap();
        assert_eq!(info.header.map(|h| h.bcd_msc), Some(0x0200));
        assert_eq!(info.problem_count(), n0 + 2);
        assert_eq!(info.problems().count(), n0 + 2);
    }
}
