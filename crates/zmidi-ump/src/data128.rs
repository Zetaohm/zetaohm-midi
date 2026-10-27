//! Message Type 0x5: System Exclusive 8 and Mixed Data Set (M2-104-UM 7.8 to 7.10, Figures 85
//! to 87, Tables 19 to 21).
//!
//! - Format: one 128-bit packet. `mt=5` (bits 127..124), Group (123..120), Status (119..116),
//!   then a 4-bit field (115..112) whose meaning depends on the Status.
//! - System Exclusive 8 (7.8, Figure 85): Status 0x0 Complete, 0x1 Start, 0x2 Continue, 0x3
//!   End (Table 19). Number of Bytes counts the valid bytes from the Stream ID (1 to 14); 0x0 is
//!   not valid, and 0xF in an End UMP aborts the message (7.8.1). Unused bytes are zero. A
//!   Start or Continue with fewer than 14 bytes does not end the message (7.8 note).
//! - The 16-bit Manufacturer ID (7.10), Device ID and Sub-IDs are the first data bytes after
//!   the Stream ID (7.8). The caller puts them in the payload; [`ManufacturerId`] converts the
//!   MIDI 1.0 IDs.
//! - Mixed Data Set (7.9, Figure 86): Status 0x8 Header and 0x9 Payload, with a 4-bit MDS ID.
//!   One Header and its Payload UMPs make one Chunk; the last Payload is padded with zero bytes.
//!   The Number of Valid Bytes in This Chunk counts the 16 Header bytes and the bytes of each
//!   Payload UMP from its first byte, without the pad bytes (DEC-UMP-006).
//! - Neither message translates to MIDI 1.0, so other UMPs may come between their packets
//!   (7.8.1 note, 7.9.1 note).
//! - Message Type and Status are read-only. Reserved bits and unused bytes are zero on write and
//!   cleared on read (2.1.3). The structs have no public `from_bits` (DEC-UMP-002).

use core::ops::Deref;

use bitfield_struct::bitfield;

use crate::packet::{join_u128, put, split_u128, take, Group, MessageType, UmpError, UmpWords};

/// Data bytes after the Stream ID in one System Exclusive 8 packet (Figure 85).
const SX8_DATA: usize = 13;

/// Data bytes in one Mixed Data Set Payload packet (Figure 86).
const MDS_DATA: usize = 14;

/// Number of Valid Bytes of the Header alone (7.9.1).
const MDS_HEADER_BYTES: u16 = 16;

/// Status 0x8: Mixed Data Set Header (7.9).
const MDS_HEADER_STATUS: u128 = 0x8;

/// Status 0x9: Mixed Data Set Payload (7.9).
const MDS_PAYLOAD_STATUS: u128 = 0x9;

/// Message Type and Group bits of a packet.
const fn packet_header(group: Group, status: u128) -> u128 {
    ((MessageType::Data128.into_bits() as u128) << 124)
        | ((group.into_bits() as u128) << 120)
        | (status << 116)
}

/// A 16-bit Manufacturer ID, MfrID (7.10, Figure 87).
///
/// A 1-byte MIDI 1.0 ID `nn` is `0x00nn`. A 3-byte MIDI 1.0 ID `0x00 b2 b3` is
/// `0x8000 | b2 << 8 | b3`. The value 0x0000 is reserved (Table 20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ManufacturerId(u16);

impl ManufacturerId {
    /// Special ID 0x7D: Non-Commercial / Research, no public release (Table 20).
    pub const NON_COMMERCIAL: Self = Self(0x007D);
    /// Special ID 0x7E: Universal System Exclusive Non-Real Time (Table 20).
    pub const UNIVERSAL_NON_REAL_TIME: Self = Self(0x007E);
    /// Special ID 0x7F: Universal System Exclusive Real Time (Table 20).
    pub const UNIVERSAL_REAL_TIME: Self = Self(0x007F);

    /// The MfrID `value`. Gives `OutOfRange` for a value that no MIDI 1.0 ID gives: bit 15
    /// clear with a high byte other than 0 or a low byte above 0x7F, bit 15 set with bit 7 set,
    /// or the reserved 0x0000.
    pub const fn new(value: u16) -> Result<Self, UmpError> {
        let valid = if value & 0x8000 == 0 {
            value & 0xFF80 == 0 && value != 0
        } else {
            value & 0x0080 == 0
        };
        if valid {
            Ok(Self(value))
        } else {
            Err(UmpError::OutOfRange)
        }
    }

    /// The MfrID of a MIDI 1.0 Manufacturer ID: one byte 0x01 to 0x7F, or three bytes 0x00,
    /// b2, b3 with 7-bit b2 and b3 (7.10 example code). Gives `OutOfRange` for another length,
    /// a byte above 0x7F, a 1-byte ID of 0x00 or a 3-byte ID that does not start with 0x00.
    pub const fn from_midi1(bytes: &[u8]) -> Result<Self, UmpError> {
        match *bytes {
            [b1] if b1 != 0 && b1 <= 0x7F => Ok(Self(b1 as u16)),
            [0, b2, b3] if b2 <= 0x7F && b3 <= 0x7F => {
                Ok(Self(0x8000 | ((b2 as u16) << 8) | b3 as u16))
            }
            _ => Err(UmpError::OutOfRange),
        }
    }

    /// The three MIDI 1.0 ID bytes, as the 7.10 example code gives them: `[id, 0, 0]` for a
    /// 1-byte ID and `[0, b2, b3]` for a 3-byte ID. Use [`ManufacturerId::is_three_byte`] to
    /// know how many bytes to send.
    pub const fn to_midi1(self) -> [u8; 3] {
        if self.is_three_byte() {
            [0, ((self.0 & 0x7F00) >> 8) as u8, (self.0 & 0x007F) as u8]
        } else {
            [(self.0 & 0x007F) as u8, 0, 0]
        }
    }

    /// True when the ID is a 3-byte MIDI 1.0 ID (bit 15 set).
    pub const fn is_three_byte(self) -> bool {
        self.0 & 0x8000 != 0
    }

    /// The 16-bit MfrID value.
    pub const fn value(self) -> u16 {
        self.0
    }
}

/// Status of a System Exclusive 8 packet, bits 119..116 (Table 19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SysEx8Status {
    /// 0x0: Complete System Exclusive 8 Message in one UMP.
    Complete = 0x0,
    /// 0x1: System Exclusive 8 Start UMP.
    Start = 0x1,
    /// 0x2: System Exclusive 8 Continue UMP.
    Continue = 0x2,
    /// 0x3: System Exclusive 8 End UMP.
    End = 0x3,
}

impl SysEx8Status {
    /// The 4-bit status value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The status for a 4-bit `value`, or `None` for a value that Table 19 does not define.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0x0 => Some(Self::Complete),
            0x1 => Some(Self::Start),
            0x2 => Some(Self::Continue),
            0x3 => Some(Self::End),
            _ => None,
        }
    }
}

/// Number of Bytes 0xF: an End UMP that aborts the message (7.8.1).
const ABORT: u8 = 0xF;

/// One System Exclusive 8 packet (7.8, Figure 85).
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct SysEx8 {
    /// Message Type, 4 bits: 0x5.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 4 bits (Table 19). Read it with [`SysEx8::status`].
    #[bits(4, access = RO)]
    status_bits: u8,
    /// Number of Bytes, 4 bits: valid bytes from the Stream ID, 1 to 14; 0xF aborts (7.8.1).
    #[bits(4, access = RO)]
    pub number_of_bytes: u8,
    /// Stream ID, 8 bits.
    #[bits(8, access = RO)]
    pub stream_id: u8,
    /// 13 data bytes, 104 bits. Read them with [`SysEx8::data`].
    #[bits(104, access = RO)]
    data_bits: u128,
}

/// Mask of the header, the Stream ID and the first `count` data bytes.
const fn sx8_mask(count: usize) -> u128 {
    let mut mask = 0x00FF_FFFF << 104;
    let mut i = 0;
    while i < count {
        mask |= 0xFF << (96 - 8 * i);
        i += 1;
    }
    mask
}

impl SysEx8 {
    /// A packet with `status`, `stream_id` and the data bytes `data` after the Stream ID.
    /// Number of Bytes is `data.len() + 1`, and unused bytes are zero (7.8). Gives `OutOfRange`
    /// for more than 13 data bytes.
    pub const fn try_new(
        group: Group,
        status: SysEx8Status,
        stream_id: u8,
        data: &[u8],
    ) -> Result<Self, UmpError> {
        if data.len() > SX8_DATA {
            return Err(UmpError::OutOfRange);
        }
        let mut bits = packet_header(group, status.into_bits() as u128)
            | (((data.len() + 1) as u128) << 112)
            | ((stream_id as u128) << 104);
        let mut i = 0;
        while i < data.len() {
            bits |= (data[i] as u128) << (96 - 8 * i);
            i += 1;
        }
        Ok(Self(bits))
    }

    /// An End UMP that aborts the message of `stream_id`: Number of Bytes 0xF and every data
    /// byte zero, for data that is incomplete or of unknown quality (7.8.1). For valid data,
    /// end with `try_new(group, SysEx8Status::End, stream_id, &[])` (Number of Bytes 0x1).
    pub const fn abort(group: Group, stream_id: u8) -> Self {
        Self(
            packet_header(group, SysEx8Status::End.into_bits() as u128)
                | ((ABORT as u128) << 112)
                | ((stream_id as u128) << 104),
        )
    }

    /// The status of the packet (Table 19).
    pub const fn status(&self) -> SysEx8Status {
        match SysEx8Status::from_bits(self.status_bits()) {
            Some(status) => status,
            // Every constructor and `parse` keep the status at 0x0 to 0x3.
            None => SysEx8Status::Complete,
        }
    }

    /// True for an End UMP with Number of Bytes 0xF: an aborted message (7.8.1).
    pub const fn is_abort(&self) -> bool {
        self.number_of_bytes() == ABORT
    }

    /// The valid data bytes after the Stream ID: none for an abort.
    pub const fn data(&self) -> SysEx8Data {
        let mut bytes = [0u8; SX8_DATA];
        let len = if self.is_abort() {
            0
        } else {
            self.number_of_bytes().saturating_sub(1)
        };
        let mut i = 0;
        while i < len as usize {
            bytes[i] = (self.0 >> (96 - 8 * i)) as u8;
            i += 1;
        }
        SysEx8Data { bytes, len }
    }

    /// Decodes one System Exclusive 8 packet from the start of `words`. Gives `None` for short
    /// input, a Message Type other than 0x5, a Status other than 0x0 to 0x3, a Number of Bytes
    /// of 0, or 0xF in a UMP that is not an End. Unused bytes are cleared. Never panics.
    pub fn parse(words: &[u32]) -> Option<Self> {
        let bits = join_u128(take::<4>(words).ok()?);
        let packet = Self(bits);
        let status = SysEx8Status::from_bits(packet.status_bits())?;
        let count = packet.number_of_bytes();
        if !matches!(packet.message_type(), MessageType::Data128)
            || count == 0
            || (count == ABORT && !matches!(status, SysEx8Status::End))
            || (count > 14 && count != ABORT)
        {
            return None;
        }
        let data = if count == ABORT {
            0
        } else {
            count as usize - 1
        };
        Some(Self(bits & sx8_mask(data)))
    }
}

impl UmpWords for SysEx8 {
    const WORDS: usize = 4;

    fn from_words(words: &[u32]) -> Result<Self, UmpError> {
        take::<4>(words)?;
        Self::parse(words).ok_or(UmpError::UnexpectedMessage)
    }

    fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
        put(&split_u128(self.0), out)
    }
}

/// The 0 to 13 valid data bytes of one [`SysEx8`] packet. Derefs to a byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SysEx8Data {
    bytes: [u8; SX8_DATA],
    len: u8,
}

impl Deref for SysEx8Data {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

/// Iterator over the packets of one System Exclusive 8 message, in send order (7.8).
///
/// A payload of 0 to 13 bytes gives one Complete packet. A longer payload gives one Start
/// packet, zero or more Continue packets and one End packet, with 13 bytes in each packet but
/// the last. The payload starts with the Manufacturer ID (two bytes, high byte first), Device ID
/// and Sub-IDs, which the caller writes (7.8).
#[derive(Debug, Clone)]
pub struct SysEx8Packets<'a> {
    group: Group,
    stream_id: u8,
    rest: &'a [u8],
    started: bool,
    done: bool,
}

impl<'a> SysEx8Packets<'a> {
    /// The packets of `payload` with `stream_id` on `group`. A device with one stream uses
    /// Stream ID 0 (7.8).
    pub const fn new(group: Group, stream_id: u8, payload: &'a [u8]) -> Self {
        Self {
            group,
            stream_id,
            rest: payload,
            started: false,
            done: false,
        }
    }
}

impl Iterator for SysEx8Packets<'_> {
    type Item = SysEx8;

    fn next(&mut self) -> Option<SysEx8> {
        if self.done {
            return None;
        }
        let last = self.rest.len() <= SX8_DATA;
        let status = match (self.started, last) {
            (false, true) => SysEx8Status::Complete,
            (false, false) => SysEx8Status::Start,
            (true, false) => SysEx8Status::Continue,
            (true, true) => SysEx8Status::End,
        };
        let (chunk, rest) = self.rest.split_at(self.rest.len().min(SX8_DATA));
        self.rest = rest;
        self.started = true;
        self.done = last;
        SysEx8::try_new(self.group, status, self.stream_id, chunk).ok()
    }
}

/// Result of [`SysEx8Reassembler::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysEx8Event<'a> {
    /// A complete message: the data bytes after the Stream ID of all its packets.
    Complete(&'a [u8]),
    /// An End UMP with Number of Bytes 0xF aborted the message (7.8.1); its bytes are
    /// discarded.
    Aborted {
        /// Data bytes received before the abort.
        received: usize,
    },
    /// A Start or Complete of the same stream came before the End; the bytes are discarded.
    Incomplete {
        /// Data bytes received before the message ended.
        received: usize,
    },
    /// The message ended normally, but was longer than the capacity; its bytes are discarded.
    Overflow {
        /// Full data length of the message.
        length: usize,
    },
}

/// Reassembles the System Exclusive 8 messages of one Group and one Stream ID into a fixed
/// buffer of `N` bytes (7.8, 7.8.1). Needs no alloc.
#[derive(Debug, Clone)]
pub struct SysEx8Reassembler<const N: usize> {
    group: Group,
    stream_id: u8,
    buffer: [u8; N],
    /// Data bytes received in the current message; can be above `N` after an overflow.
    length: usize,
    in_message: bool,
}

impl<const N: usize> SysEx8Reassembler<N> {
    /// An empty reassembler for the messages of `stream_id` on `group`.
    pub const fn new(group: Group, stream_id: u8) -> Self {
        Self {
            group,
            stream_id,
            buffer: [0; N],
            length: 0,
            in_message: false,
        }
    }

    /// Clears the reassembler, so it holds no partial message.
    pub fn reset(&mut self) {
        self.length = 0;
        self.in_message = false;
    }

    /// Takes the words of one packet and gives an event when a message ends.
    ///
    /// - Packets that are not System Exclusive 8 packets of the Group and Stream ID give no
    ///   event: other messages may come between the packets (7.8.1 note).
    /// - A Start or Continue with fewer than 14 bytes does not end the message (7.8).
    /// - An End, or a Complete outside a message, gives [`SysEx8Event::Complete`], or
    ///   [`SysEx8Event::Overflow`] when the data is longer than `N`. An End with Number of
    ///   Bytes 0x1 and no data is a valid end; 0xF gives [`SysEx8Event::Aborted`] (7.8.1).
    /// - A Start before the End gives [`SysEx8Event::Incomplete`] and begins a new message. A
    ///   Complete before the End gives `Incomplete` and is not decoded: push it again.
    /// - A Continue or End outside a message gives no event.
    ///
    /// Never panics.
    pub fn push(&mut self, words: &[u32]) -> Option<SysEx8Event<'_>> {
        let packet = SysEx8::parse(words)?;
        if packet.group() != self.group || packet.stream_id() != self.stream_id {
            return None;
        }
        match (packet.status(), self.in_message) {
            (SysEx8Status::Complete, false) => {
                self.length = 0;
                self.append(&packet.data());
                self.finish()
            }
            (SysEx8Status::Complete, true) => {
                self.in_message = false;
                Some(SysEx8Event::Incomplete {
                    received: self.length,
                })
            }
            (SysEx8Status::Start, in_message) => {
                let event = in_message.then_some(SysEx8Event::Incomplete {
                    received: self.length,
                });
                self.length = 0;
                self.in_message = true;
                self.append(&packet.data());
                event
            }
            (SysEx8Status::Continue, true) => {
                self.append(&packet.data());
                None
            }
            (SysEx8Status::End, true) if packet.is_abort() => {
                self.in_message = false;
                Some(SysEx8Event::Aborted {
                    received: self.length,
                })
            }
            (SysEx8Status::End, true) => {
                self.append(&packet.data());
                self.finish()
            }
            (SysEx8Status::Continue | SysEx8Status::End, false) => None,
        }
    }

    fn append(&mut self, data: &[u8]) {
        for &byte in data {
            if let Some(slot) = self.buffer.get_mut(self.length) {
                *slot = byte;
            }
            self.length = self.length.saturating_add(1);
        }
    }

    fn finish(&mut self) -> Option<SysEx8Event<'_>> {
        self.in_message = false;
        if self.length > N {
            Some(SysEx8Event::Overflow {
                length: self.length,
            })
        } else {
            Some(SysEx8Event::Complete(&self.buffer[..self.length]))
        }
    }
}

/// The ID fields of a Mixed Data Set Header (7.9, Figure 86).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MdsIds {
    /// Manufacturer ID, 16 bits (7.10).
    pub manufacturer: ManufacturerId,
    /// Device ID, 16 bits; [`MdsIds::ALL_CALL`] for every device.
    pub device_id: u16,
    /// Sub-ID #1, 16 bits.
    pub sub_id1: u16,
    /// Sub-ID #2, 16 bits.
    pub sub_id2: u16,
}

impl MdsIds {
    /// Device ID 0xFFFF: all devices respond, as 0x7F in MIDI 1.0 (7.9).
    pub const ALL_CALL: u16 = 0xFFFF;
}

/// Mixed Data Set Header, Status 0x8 (7.9, Figure 86).
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct MdsHeader {
    /// Message Type, 4 bits: 0x5.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 4 bits: 0x8.
    #[bits(4, access = RO)]
    status_bits: u8,
    /// MDS ID, 4 bits.
    #[bits(4, access = RO)]
    pub mds_id: u8,
    /// Number of Valid Bytes in This Chunk, 16 bits, with the 16 Header bytes.
    #[bits(16, access = RO)]
    pub number_of_valid_bytes: u16,
    /// Number of Chunks in Mixed Data Set, 16 bits; 0 is unknown.
    #[bits(16, access = RO)]
    pub number_of_chunks: u16,
    /// Number of This Chunk, 16 bits, from 1; 0 ends a set of unknown validity (7.9.1).
    #[bits(16, access = RO)]
    pub number_of_this_chunk: u16,
    /// Manufacturer ID, 16 bits. Read it with [`MdsHeader::ids`].
    #[bits(16, access = RO)]
    manufacturer_bits: u16,
    /// Device ID, 16 bits.
    #[bits(16, access = RO)]
    pub device_id: u16,
    /// Sub-ID #1, 16 bits.
    #[bits(16, access = RO)]
    pub sub_id1: u16,
    /// Sub-ID #2, 16 bits.
    #[bits(16, access = RO)]
    pub sub_id2: u16,
}

impl MdsHeader {
    /// A Header for MDS ID `mds_id` (0 to 15). Gives `OutOfRange` for an MDS ID above 15, a
    /// Number of Valid Bytes below 16 (the Header alone), or a Number of This Chunk above a
    /// known (non-zero) Number of Chunks (7.9).
    pub const fn try_new(
        group: Group,
        mds_id: u8,
        number_of_valid_bytes: u16,
        number_of_chunks: u16,
        number_of_this_chunk: u16,
        ids: MdsIds,
    ) -> Result<Self, UmpError> {
        if mds_id > 0xF
            || number_of_valid_bytes < MDS_HEADER_BYTES
            || (number_of_chunks != 0 && number_of_this_chunk > number_of_chunks)
        {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            packet_header(group, MDS_HEADER_STATUS)
                | ((mds_id as u128) << 112)
                | ((number_of_valid_bytes as u128) << 96)
                | ((number_of_chunks as u128) << 80)
                | ((number_of_this_chunk as u128) << 64)
                | ((ids.manufacturer.value() as u128) << 48)
                | ((ids.device_id as u128) << 32)
                | ((ids.sub_id1 as u128) << 16)
                | ids.sub_id2 as u128,
        ))
    }

    /// The ID fields, or `None` when the received Manufacturer ID is not a valid MfrID.
    pub const fn ids(&self) -> Option<MdsIds> {
        match ManufacturerId::new(self.manufacturer_bits()) {
            Ok(manufacturer) => Some(MdsIds {
                manufacturer,
                device_id: self.device_id(),
                sub_id1: self.sub_id1(),
                sub_id2: self.sub_id2(),
            }),
            Err(_) => None,
        }
    }

    /// True for the last Chunk of a set: Number of This Chunk equals Number of Chunks, or is 0
    /// for a set of unknown validity (7.9, 7.9.1).
    pub const fn is_last_chunk(&self) -> bool {
        self.number_of_this_chunk() == self.number_of_chunks() || self.number_of_this_chunk() == 0
    }

    /// Reads a Header. Gives `None` for another Message Type or Status, or for a Number of
    /// Valid Bytes below 16 or too small for 2 prefix bytes in each Payload UMP (DEC-UMP-006).
    fn parse(words: &[u32]) -> Option<Self> {
        let bits = join_u128(take::<4>(words).ok()?);
        let valid = ((bits >> 96) as u16).checked_sub(MDS_HEADER_BYTES)? as usize;
        ((bits >> 124) as u8 == MessageType::Data128.into_bits()
            && (bits >> 116) & 0xF == MDS_HEADER_STATUS
            // The last Payload UMP holds at least its 2 prefix bytes.
            && valid % 16 != 1)
            .then_some(Self(bits))
    }
}

impl UmpWords for MdsHeader {
    const WORDS: usize = 4;

    fn from_words(words: &[u32]) -> Result<Self, UmpError> {
        take::<4>(words)?;
        Self::parse(words).ok_or(UmpError::UnexpectedMessage)
    }

    fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
        put(&split_u128(self.0), out)
    }
}

/// Mixed Data Set Payload, Status 0x9 (7.9, Figure 86).
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct MdsPayload {
    /// Message Type, 4 bits: 0x5.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 4 bits: 0x9.
    #[bits(4, access = RO)]
    status_bits: u8,
    /// MDS ID, 4 bits.
    #[bits(4, access = RO)]
    pub mds_id: u8,
    /// 14 data bytes, 112 bits. Read them with [`MdsPayload::data`].
    #[bits(112, access = RO)]
    data_bits: u128,
}

impl MdsPayload {
    /// A Payload for MDS ID `mds_id` (0 to 15) with 0 to 14 data bytes; the rest are zero pad
    /// bytes (7.9). Gives `OutOfRange` for an MDS ID above 15 or more than 14 bytes.
    pub const fn try_new(group: Group, mds_id: u8, data: &[u8]) -> Result<Self, UmpError> {
        if mds_id > 0xF || data.len() > MDS_DATA {
            return Err(UmpError::OutOfRange);
        }
        let mut bits = packet_header(group, MDS_PAYLOAD_STATUS) | ((mds_id as u128) << 112);
        let mut i = 0;
        while i < data.len() {
            bits |= (data[i] as u128) << (104 - 8 * i);
            i += 1;
        }
        Ok(Self(bits))
    }

    /// All 14 data bytes. The Header's Number of Valid Bytes tells how many of the last
    /// Payload's bytes are data and how many are pad bytes.
    pub const fn data(&self) -> [u8; MDS_DATA] {
        let mut bytes = [0u8; MDS_DATA];
        let mut i = 0;
        while i < MDS_DATA {
            bytes[i] = (self.0 >> (104 - 8 * i)) as u8;
            i += 1;
        }
        bytes
    }

    fn parse(words: &[u32]) -> Option<Self> {
        let bits = join_u128(take::<4>(words).ok()?);
        ((bits >> 124) as u8 == MessageType::Data128.into_bits()
            && (bits >> 116) & 0xF == MDS_PAYLOAD_STATUS)
            .then_some(Self(bits))
    }
}

impl UmpWords for MdsPayload {
    const WORDS: usize = 4;

    fn from_words(words: &[u32]) -> Result<Self, UmpError> {
        take::<4>(words)?;
        Self::parse(words).ok_or(UmpError::UnexpectedMessage)
    }

    fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
        put(&split_u128(self.0), out)
    }
}

/// Number of Valid Bytes of a Chunk with `payload` data bytes: 16 Header bytes, and 2 + data
/// bytes for each Payload UMP (DEC-UMP-006).
const fn chunk_valid_bytes(payload: usize) -> usize {
    MDS_HEADER_BYTES as usize + 2 * payload.div_ceil(MDS_DATA) + payload
}

/// Iterator over the packets of one Mixed Data Set Chunk: the Header, then Payload UMPs of 14
/// bytes with the last one padded with zero bytes (7.9, Figure 86). An empty payload gives a
/// Header-only Chunk with Number of Valid Bytes 16 (7.9.1).
#[derive(Debug, Clone)]
pub struct MdsChunkPackets<'a> {
    header: Option<MdsHeader>,
    group: Group,
    mds_id: u8,
    rest: &'a [u8],
}

impl<'a> MdsChunkPackets<'a> {
    /// The packets of one Chunk of MDS ID `mds_id`. For the last Chunk of a set with an
    /// unknown size, or of a set that ends early with valid data, give Number of Chunks equal
    /// to Number of This Chunk; for a set that ends early with data of unknown validity, give
    /// Number of This Chunk 0 (7.9, 7.9.1).
    ///
    /// Gives `OutOfRange` as [`MdsHeader::try_new`] does, or when the Number of Valid Bytes of
    /// the payload is above 0xFFFF.
    pub fn try_new(
        group: Group,
        mds_id: u8,
        number_of_chunks: u16,
        number_of_this_chunk: u16,
        ids: MdsIds,
        payload: &'a [u8],
    ) -> Result<Self, UmpError> {
        let valid = chunk_valid_bytes(payload.len());
        if valid > u16::MAX as usize {
            return Err(UmpError::OutOfRange);
        }
        let header = MdsHeader::try_new(
            group,
            mds_id,
            valid as u16,
            number_of_chunks,
            number_of_this_chunk,
            ids,
        )?;
        Ok(Self {
            header: Some(header),
            group,
            mds_id,
            rest: payload,
        })
    }
}

impl Iterator for MdsChunkPackets<'_> {
    type Item = Data128Message;

    fn next(&mut self) -> Option<Data128Message> {
        if let Some(header) = self.header.take() {
            return Some(Data128Message::MdsHeader(header));
        }
        if self.rest.is_empty() {
            return None;
        }
        let (chunk, rest) = self.rest.split_at(self.rest.len().min(MDS_DATA));
        self.rest = rest;
        MdsPayload::try_new(self.group, self.mds_id, chunk)
            .ok()
            .map(Data128Message::MdsPayload)
    }
}

/// One Message Type 0x5 packet (7.8, 7.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Data128Message {
    /// System Exclusive 8, Status 0x0 to 0x3.
    SysEx8(SysEx8),
    /// Mixed Data Set Header, Status 0x8.
    MdsHeader(MdsHeader),
    /// Mixed Data Set Payload, Status 0x9.
    MdsPayload(MdsPayload),
}

impl Data128Message {
    /// Decodes one Message Type 0x5 packet (four words) by its Status. Gives `None` for short
    /// input, another Message Type, a Status that 7.8 and 7.9 do not define, a System
    /// Exclusive 8 packet that [`SysEx8::parse`] rejects, or a Header with a Number of Valid
    /// Bytes that is not valid (DEC-UMP-006). Never panics.
    pub fn parse(words: &[u32]) -> Option<Self> {
        let w0 = *words.first()?;
        if w0 >> 28 != MessageType::Data128.into_bits() as u32 {
            return None;
        }
        match (w0 >> 20) & 0xF {
            0x0..=0x3 => SysEx8::parse(words).map(Self::SysEx8),
            0x8 => MdsHeader::parse(words).map(Self::MdsHeader),
            0x9 => MdsPayload::parse(words).map(Self::MdsPayload),
            _ => None,
        }
    }

    /// Words 0 to 3 of the packet.
    pub(crate) const fn into_words(self) -> [u32; 4] {
        split_u128(match self {
            Self::SysEx8(m) => m.0,
            Self::MdsHeader(m) => m.0,
            Self::MdsPayload(m) => m.0,
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec::Vec;

    use super::*;
    use proptest::prelude::*;

    fn g(n: u8) -> Group {
        Group::new(n).unwrap()
    }

    fn words<T: UmpWords>(m: &T) -> [u32; 4] {
        let mut out = [0u32; 4];
        m.to_words(&mut out).unwrap();
        out
    }

    fn ids() -> MdsIds {
        MdsIds {
            manufacturer: ManufacturerId::new(0x0043).unwrap(),
            device_id: MdsIds::ALL_CALL,
            sub_id1: 0x0102,
            sub_id2: 0x0304,
        }
    }

    /// Table 21: MIDI 1.0 Manufacturer IDs and their 16-bit MfrID. (The table prints MfrID_hi
    /// 0x80 and MfrID_lo 0x3b for imitone, copied from the row above; its MfrID 0x8213 is
    /// correct and is used here.)
    #[test]
    fn manufacturer_id_matches_table_21() {
        let cases: [(&[u8], u16); 9] = [
            (&[0x04], 0x0004),
            (&[0x09], 0x0009),
            (&[0x43], 0x0043),
            (&[0x00, 0x00, 0x3B], 0x803B),
            (&[0x00, 0x02, 0x13], 0x8213),
            (&[0x00, 0x02, 0x1D], 0x821D),
            (&[0x00, 0x20, 0x25], 0xA025),
            (&[0x00, 0x21, 0x09], 0xA109),
            (&[0x00, 0x21, 0x32], 0xA132),
        ];
        for (midi1, mfr) in cases {
            let id = ManufacturerId::from_midi1(midi1).unwrap();
            assert_eq!(id.value(), mfr);
            assert_eq!(ManufacturerId::new(mfr), Ok(id));
            assert_eq!(id.is_three_byte(), midi1.len() == 3);
            let bytes = id.to_midi1();
            let len = if id.is_three_byte() { 3 } else { 1 };
            assert_eq!(&bytes[..len], midi1);
            assert!(bytes[len..].iter().all(|&b| b == 0));
        }
    }

    /// Table 20 and Figure 87: the special IDs, and values that no MIDI 1.0 ID gives.
    #[test]
    fn manufacturer_id_special_and_invalid_values() {
        assert_eq!(ManufacturerId::NON_COMMERCIAL.value(), 0x007D);
        assert_eq!(ManufacturerId::UNIVERSAL_NON_REAL_TIME.value(), 0x007E);
        assert_eq!(ManufacturerId::UNIVERSAL_REAL_TIME.value(), 0x007F);
        assert_eq!(
            ManufacturerId::from_midi1(&[0x7E]),
            Ok(ManufacturerId::UNIVERSAL_NON_REAL_TIME)
        );
        for value in [0x0000, 0x0080, 0x00FF, 0x0100, 0x7F7F, 0x8080, 0xFFFF] {
            assert_eq!(
                ManufacturerId::new(value),
                Err(UmpError::OutOfRange),
                "{value:04X}"
            );
        }
        for bytes in [
            &[][..],
            &[0x00],
            &[0x80],
            &[0x00, 0x21],
            &[0x01, 0x21, 0x09],
            &[0x00, 0x80, 0x09],
            &[0x00, 0x21, 0x80],
            &[0x00, 0x21, 0x09, 0x00],
        ] {
            assert_eq!(ManufacturerId::from_midi1(bytes), Err(UmpError::OutOfRange));
        }
    }

    /// Table 19 status values.
    #[test]
    fn sysex8_status_matches_table_19() {
        for (bits, s) in [
            (0, SysEx8Status::Complete),
            (1, SysEx8Status::Start),
            (2, SysEx8Status::Continue),
            (3, SysEx8Status::End),
        ] {
            assert_eq!(s.into_bits(), bits);
            assert_eq!(SysEx8Status::from_bits(bits), Some(s));
        }
        assert!((4..=0xF).all(|b| SysEx8Status::from_bits(b).is_none()));
    }

    /// Figure 85: a Complete packet with the 16-bit Manufacturer ID, Device ID and Sub-ID as
    /// its first data bytes; Number of Bytes counts the Stream ID.
    #[test]
    fn sysex8_matches_figure_85() {
        let mfr = ManufacturerId::from_midi1(&[0x43])
            .unwrap()
            .value()
            .to_be_bytes();
        let payload = [mfr[0], mfr[1], 0x10, 0x4C];
        let m = SysEx8::try_new(g(1), SysEx8Status::Complete, 0, &payload).unwrap();
        assert_eq!(words(&m), [0x5105_0000, 0x4310_4C00, 0, 0]);
        assert_eq!(m.message_type(), MessageType::Data128);
        assert_eq!(
            (m.status(), m.number_of_bytes(), m.stream_id()),
            (SysEx8Status::Complete, 5, 0)
        );
        assert_eq!(&*m.data(), &payload);
        // 13 data bytes fill the packet: Number of Bytes 14.
        let full: Vec<u8> = (1..=13).collect();
        let m = SysEx8::try_new(g(15), SysEx8Status::Continue, 0xFF, &full).unwrap();
        assert_eq!(
            words(&m),
            [0x5F2E_FF01, 0x0203_0405, 0x0607_0809, 0x0A0B_0C0D]
        );
        assert_eq!(SysEx8::from_words(&words(&m)), Ok(m));
        assert_eq!(
            SysEx8::try_new(g(0), SysEx8Status::Start, 0, &[0; 14]),
            Err(UmpError::OutOfRange)
        );
        // Only the Stream ID: Number of Bytes 1.
        let m = SysEx8::try_new(g(0), SysEx8Status::Complete, 7, &[]).unwrap();
        assert_eq!(words(&m), [0x5001_0700, 0, 0, 0]);
    }

    /// 7.8.1: an End with all data zero and Number of Bytes 0x1 (valid) or 0xF (abort).
    #[test]
    fn sysex8_unexpected_end_matches_7_8_1() {
        let valid = SysEx8::try_new(g(0), SysEx8Status::End, 3, &[]).unwrap();
        assert_eq!(words(&valid), [0x5031_0300, 0, 0, 0]);
        assert!(!valid.is_abort());
        let abort = SysEx8::abort(g(0), 3);
        assert_eq!(words(&abort), [0x503F_0300, 0, 0, 0]);
        assert!(abort.is_abort());
        assert_eq!(abort.status(), SysEx8Status::End);
        assert!(abort.data().is_empty());
        // A received abort with data bytes reads with them cleared.
        let read = SysEx8::parse(&[0x503F_03AA, 1, 2, 3]).unwrap();
        assert_eq!(read, abort);
    }

    /// 7.8, 2.1.3: the parser rejects undefined Status and Number of Bytes values and clears
    /// unused bytes.
    #[test]
    fn sysex8_parse_rejects_and_clears() {
        for w0 in [
            0x5000_0000u32, // Number of Bytes 0.
            0x500F_0000,    // 0xF in a Complete.
            0x501F_0000,    // 0xF in a Start.
            0x502F_0000,    // 0xF in a Continue.
            0x500E_0000,    // Number of Bytes 14 is the largest valid count; 0xE is 14.
        ] {
            let parsed = SysEx8::parse(&[w0, 0, 0, 0]);
            if w0 == 0x500E_0000 {
                assert!(parsed.is_some());
            } else {
                assert_eq!(parsed, None, "{w0:08X}");
            }
        }
        for status in 4..=0xF {
            assert_eq!(SysEx8::parse(&[0x5001_0000 | status << 20, 0, 0, 0]), None);
        }
        assert_eq!(SysEx8::parse(&[0x3001_0000, 0, 0, 0]), None);
        assert_eq!(
            SysEx8::from_words(&[0x5001_0000, 0]),
            Err(UmpError::Truncated { needed: 4, got: 2 })
        );
        let read = SysEx8::parse(&[0x5003_07AA, 0xBBCC_DDEE, 1, 2]).unwrap();
        assert_eq!(words(&read), [0x5003_07AA, 0xBB00_0000, 0, 0]);
        assert_eq!(&*read.data(), &[0xAA, 0xBB]);
    }

    /// 7.8: the writer gives Complete for 13 bytes or fewer, else Start, Continue and End with
    /// 13 data bytes in each packet but the last.
    #[test]
    fn sysex8_packets_split_payload() {
        let short: Vec<_> = SysEx8Packets::new(g(2), 1, &[0xA1; 13]).collect();
        assert_eq!(short.len(), 1);
        assert_eq!(short[0].status(), SysEx8Status::Complete);
        assert_eq!(short[0].number_of_bytes(), 14);
        let empty: Vec<_> = SysEx8Packets::new(g(2), 1, &[]).collect();
        assert_eq!(empty.len(), 1);
        assert_eq!(empty[0].number_of_bytes(), 1);
        let payload: Vec<u8> = (0..30).collect();
        let packets: Vec<_> = SysEx8Packets::new(g(2), 9, &payload).collect();
        let shape: Vec<_> = packets
            .iter()
            .map(|p| (p.status(), p.number_of_bytes(), p.stream_id()))
            .collect();
        assert_eq!(
            shape,
            [
                (SysEx8Status::Start, 14, 9),
                (SysEx8Status::Continue, 14, 9),
                (SysEx8Status::End, 5, 9),
            ]
        );
        assert_eq!(words(&packets[2]), [0x5235_091A, 0x1B1C_1D00, 0, 0]);
    }

    fn push_all<const N: usize>(
        r: &mut SysEx8Reassembler<N>,
        packets: &[[u32; 4]],
    ) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for p in packets {
            if let Some(SysEx8Event::Complete(data)) = r.push(p) {
                out.push(data.to_vec());
            }
        }
        out
    }

    /// 7.8, 7.8.1: the reassembler joins one stream, ignores other streams and other UMPs,
    /// does not end at a short Start or Continue, and reports aborts and early ends.
    #[test]
    fn sysex8_reassembler_streams_and_ends() {
        let a: Vec<u8> = (0..40).collect();
        let b: Vec<u8> = (100..130).collect();
        let pa: Vec<_> = SysEx8Packets::new(g(0), 0, &a).map(|p| words(&p)).collect();
        let pb: Vec<_> = SysEx8Packets::new(g(0), 1, &b).map(|p| words(&p)).collect();
        // Two streams interleaved, with a Note On and a Timing Clock between (7.8.1 note).
        let mut mixed = Vec::new();
        for i in 0..pa.len().max(pb.len()) {
            mixed.extend(pa.get(i));
            mixed.push([0x4090_3C00, 0xFFFF_0000, 0, 0]);
            mixed.extend(pb.get(i));
            mixed.push([0x10F8_0000, 0, 0, 0]);
        }
        let mut r0 = SysEx8Reassembler::<64>::new(g(0), 0);
        let mut r1 = SysEx8Reassembler::<64>::new(g(0), 1);
        assert_eq!(push_all(&mut r0, &mixed), [a]);
        assert_eq!(push_all(&mut r1, &mixed), [b]);
        // A short Start and Continue do not end the message (7.8 note).
        let short = [
            words(&SysEx8::try_new(g(0), SysEx8Status::Start, 0, &[1, 2]).unwrap()),
            words(&SysEx8::try_new(g(0), SysEx8Status::Continue, 0, &[3]).unwrap()),
            words(&SysEx8::try_new(g(0), SysEx8Status::End, 0, &[]).unwrap()),
        ];
        assert_eq!(push_all(&mut r0, &short), [std::vec![1, 2, 3]]);
        // An abort discards the message.
        assert_eq!(r0.push(&short[0]), None);
        assert_eq!(
            r0.push(&words(&SysEx8::abort(g(0), 0))),
            Some(SysEx8Event::Aborted { received: 2 })
        );
        // An abort or End with no Start gives no event.
        assert_eq!(r0.push(&words(&SysEx8::abort(g(0), 0))), None);
        assert_eq!(r0.push(&short[2]), None);
        // A new Start before the End gives Incomplete and begins a new message.
        assert_eq!(r0.push(&short[0]), None);
        assert_eq!(
            r0.push(&short[0]),
            Some(SysEx8Event::Incomplete { received: 2 })
        );
        assert_eq!(push_all(&mut r0, &short[1..]), [std::vec![1, 2, 3]]);
        // A Complete before the End gives Incomplete; pushed again, it gives its data.
        let complete = words(&SysEx8::try_new(g(0), SysEx8Status::Complete, 0, &[9]).unwrap());
        assert_eq!(r0.push(&short[0]), None);
        assert_eq!(
            r0.push(&complete),
            Some(SysEx8Event::Incomplete { received: 2 })
        );
        assert_eq!(push_all(&mut r0, &[complete]), [std::vec![9]]);
        // reset discards a partial message.
        assert_eq!(r0.push(&short[0]), None);
        r0.reset();
        assert_eq!(r0.push(&short[2]), None);
        // Overflow.
        let mut small = SysEx8Reassembler::<16>::new(g(0), 0);
        let events: Vec<_> = pa
            .iter()
            .filter_map(|p| match small.push(p) {
                Some(SysEx8Event::Overflow { length }) => Some(length),
                Some(_) => Some(0),
                None => None,
            })
            .collect();
        assert_eq!(events, [40]);
        // Another Group gives no event.
        let mut other = SysEx8Reassembler::<64>::new(g(1), 0);
        assert!(push_all(&mut other, &pa).is_empty());
    }

    /// Figure 86: a Header with every field.
    #[test]
    fn mds_header_matches_figure_86() {
        let h = MdsHeader::try_new(g(2), 3, 0x0028, 2, 1, ids()).unwrap();
        assert_eq!(
            words(&h),
            [0x5283_0028, 0x0002_0001, 0x0043_FFFF, 0x0102_0304]
        );
        assert_eq!(
            (
                h.mds_id(),
                h.number_of_valid_bytes(),
                h.number_of_chunks(),
                h.number_of_this_chunk()
            ),
            (3, 0x28, 2, 1)
        );
        assert_eq!(h.ids(), Some(ids()));
        assert_eq!(h.message_type(), MessageType::Data128);
        assert!(!h.is_last_chunk());
        assert_eq!(MdsHeader::from_words(&words(&h)), Ok(h));
        // A Number of Valid Bytes below 16, or with fewer than 2 bytes for a Payload UMP, is
        // not valid (DEC-UMP-006); 18 is one Payload UMP with no data bytes.
        for valid in [0u32, 15, 17, 33] {
            assert_eq!(
                MdsHeader::from_words(&[0x5080_0000 | valid, 0x0001_0001, 0x0043_0000, 0]),
                Err(UmpError::UnexpectedMessage),
                "{valid}"
            );
        }
        assert!(MdsHeader::from_words(&[0x5080_0012, 0x0001_0001, 0x0043_0000, 0]).is_ok());
        // A received Manufacturer ID that is not a valid MfrID gives no ids.
        let bad = MdsHeader::from_words(&[0x5280_0010, 0x0001_0001, 0x0080_0000, 0]).unwrap();
        assert_eq!(bad.ids(), None);
        for (mds_id, valid, chunks, this) in [(16, 16, 1, 1), (0, 15, 1, 1), (0, 16, 2, 3)] {
            assert_eq!(
                MdsHeader::try_new(g(0), mds_id, valid, chunks, this, ids()),
                Err(UmpError::OutOfRange)
            );
        }
        // Number of Chunks 0 is unknown: any Number of This Chunk.
        assert!(MdsHeader::try_new(g(0), 0, 16, 0, 500, ids()).is_ok());
    }

    /// Figure 86: a Payload with 14 data bytes, and zero pad bytes after short data.
    #[test]
    fn mds_payload_matches_figure_86() {
        let data: Vec<u8> = (1..=14).collect();
        let p = MdsPayload::try_new(g(2), 3, &data).unwrap();
        assert_eq!(
            words(&p),
            [0x5293_0102, 0x0304_0506, 0x0708_090A, 0x0B0C_0D0E]
        );
        assert_eq!(p.data().as_slice(), data.as_slice());
        assert_eq!(p.mds_id(), 3);
        let p = MdsPayload::try_new(g(0), 15, &[0xAA]).unwrap();
        assert_eq!(words(&p), [0x509F_AA00, 0, 0, 0]);
        assert_eq!(MdsPayload::from_words(&words(&p)), Ok(p));
        assert_eq!(
            MdsPayload::try_new(g(0), 0, &[0; 15]),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            MdsPayload::try_new(g(0), 16, &[]),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            MdsPayload::from_words(&[0x5083_0000, 0, 0, 0]),
            Err(UmpError::UnexpectedMessage)
        );
    }

    /// 7.9: a Chunk is a Header and Payload UMPs, the last one padded; Number of Valid Bytes
    /// counts the Header and each Payload UMP without pad bytes (DEC-UMP-006).
    #[test]
    fn mds_chunk_packets_write_one_chunk() {
        let payload: Vec<u8> = (0..20).collect();
        let packets: Vec<_> = MdsChunkPackets::try_new(g(1), 4, 3, 2, ids(), &payload)
            .unwrap()
            .map(Data128Message::into_words)
            .collect();
        assert_eq!(
            packets,
            [
                [0x5184_0028, 0x0003_0002, 0x0043_FFFF, 0x0102_0304],
                [0x5194_0001, 0x0203_0405, 0x0607_0809, 0x0A0B_0C0D],
                [0x5194_0E0F, 0x1011_1213, 0, 0],
            ]
        );
        // 7.9.1: a Header-only Chunk has Number of Valid Bytes 16.
        let header_only: Vec<_> = MdsChunkPackets::try_new(g(1), 4, 3, 3, ids(), &[])
            .unwrap()
            .collect();
        let [Data128Message::MdsHeader(h)] = header_only.as_slice() else {
            panic!("an empty payload gives only the Header")
        };
        assert_eq!(h.number_of_valid_bytes(), 16);
        assert!(h.is_last_chunk());
        // The largest payload: 16 + 2 * 4095 + 57329 = 65535.
        let max = std::vec![0u8; 57_329];
        assert_eq!(
            MdsChunkPackets::try_new(g(0), 0, 1, 1, ids(), &max)
                .unwrap()
                .count(),
            1 + 4095
        );
        let too_long = std::vec![0u8; 57_330];
        assert!(MdsChunkPackets::try_new(g(0), 0, 1, 1, ids(), &too_long).is_err());
        assert!(MdsChunkPackets::try_new(g(0), 16, 1, 1, ids(), &[]).is_err());
    }

    /// Secondary check (DEC-001): the Mixed Data Set vectors of ni-midi2 v1.12.0
    /// (tests/mixed_data_set_test_data.cpp), which uses the same Number of Valid Bytes rule.
    #[test]
    fn mds_chunks_match_ni_midi2_vectors() {
        type Chunk<'a> = (u8, u16, u16, [u8; 3], [u16; 3], &'a [u8], &'a [[u32; 4]]);
        let mfr = |b: [u8; 3]| match b {
            [0, b2, b3] => ManufacturerId::from_midi1(&[0, b2, b3]).unwrap(),
            [b1, ..] => ManufacturerId::from_midi1(&[b1]).unwrap(),
        };
        let data20: Vec<u8> = (1..=20).collect();
        let a0: Vec<u8> = (0xA0..=0xAD).collect();
        let cases: [Chunk; 7] = [
            // Header-only chunk.
            (
                4,
                1,
                1,
                [0x7E, 0, 0],
                [0x1234, 5, 6],
                &[],
                &[[0x5084_0010, 0x0001_0001, 0x007E_1234, 0x0005_0006]],
            ),
            // One Payload packet.
            (
                0,
                1,
                1,
                [0, 0x21, 0x09],
                [0, 0, 0],
                &[0x11, 0x22, 0x33, 0x44, 0x55],
                &[
                    [0x5080_0017, 0x0001_0001, 0xA109_0000, 0],
                    [0x5090_1122, 0x3344_5500, 0, 0],
                ],
            ),
            // Two Payload packets.
            (
                0xF,
                1,
                1,
                [0x04, 0, 0],
                [0xFFFF, 0x0102, 0x0304],
                &data20,
                &[
                    [0x508F_0028, 0x0001_0001, 0x0004_FFFF, 0x0102_0304],
                    [0x509F_0102, 0x0304_0506, 0x0708_090A, 0x0B0C_0D0E],
                    [0x509F_0F10, 0x1112_1314, 0, 0],
                ],
            ),
            // Two chunks.
            (
                7,
                2,
                1,
                [0x7F, 0, 0],
                [1, 2, 3],
                &a0,
                &[
                    [0x5087_0020, 0x0002_0001, 0x007F_0001, 0x0002_0003],
                    [0x5097_A0A1, 0xA2A3_A4A5, 0xA6A7_A8A9, 0xAAAB_ACAD],
                ],
            ),
            (
                7,
                2,
                2,
                [0x7F, 0, 0],
                [1, 2, 3],
                &[0xAE, 0xAF, 0xB0],
                &[
                    [0x5087_0015, 0x0002_0002, 0x007F_0001, 0x0002_0003],
                    [0x5097_AEAF, 0xB000_0000, 0, 0],
                ],
            ),
            // Unknown number of chunks: the final chunk gives the count (7.9).
            (
                1,
                0,
                1,
                [0, 0x21, 0x1D],
                [0, 0, 0],
                &[0xDE, 0xAD, 0xBE, 0xEF],
                &[
                    [0x5081_0016, 0x0000_0001, 0xA11D_0000, 0],
                    [0x5091_DEAD, 0xBEEF_0000, 0, 0],
                ],
            ),
            (
                1,
                2,
                2,
                [0, 0x21, 0x1D],
                [0, 0, 0],
                &[0xCA, 0xFE],
                &[
                    [0x5081_0014, 0x0002_0002, 0xA11D_0000, 0],
                    [0x5091_CAFE, 0, 0, 0],
                ],
            ),
        ];
        for (mds_id, chunks, this, m, [device_id, sub_id1, sub_id2], payload, expected) in cases {
            let ids = MdsIds {
                manufacturer: mfr(m),
                device_id,
                sub_id1,
                sub_id2,
            };
            let out: Vec<_> = MdsChunkPackets::try_new(g(0), mds_id, chunks, this, ids, payload)
                .unwrap()
                .map(Data128Message::into_words)
                .collect();
            assert_eq!(out, expected);
        }
    }

    /// 7.9, 7.9.1: the last Chunk has Number of This Chunk equal to Number of Chunks (also
    /// for an unknown size or an early end with valid data), or 0 for data of unknown validity.
    #[test]
    fn mds_end_of_set_matches_7_9_1() {
        let last = MdsHeader::try_new(g(0), 1, 16, 5, 5, ids()).unwrap();
        assert!(last.is_last_chunk());
        let early_valid = MdsHeader::try_new(g(0), 1, 16, 3, 3, ids()).unwrap();
        assert!(early_valid.is_last_chunk());
        let unknown_validity = MdsHeader::try_new(g(0), 1, 16, 5, 0, ids()).unwrap();
        assert!(unknown_validity.is_last_chunk());
        assert_eq!(words(&unknown_validity)[1], 0x0005_0000);
        let streaming = MdsHeader::try_new(g(0), 1, 16, 0, 7, ids()).unwrap();
        assert!(!streaming.is_last_chunk());
    }

    /// Data128Message dispatch on Status; the Ump enum holds it.
    #[test]
    fn data128_message_parse_dispatches_on_status() {
        assert!(matches!(
            Data128Message::parse(&[0x5001_0000, 0, 0, 0]),
            Some(Data128Message::SysEx8(_))
        ));
        assert!(matches!(
            Data128Message::parse(&[0x5080_0010, 0, 0, 0]),
            Some(Data128Message::MdsHeader(_))
        ));
        assert!(matches!(
            Data128Message::parse(&[0x5090_0000, 0, 0, 0]),
            Some(Data128Message::MdsPayload(_))
        ));
        for status in [4u32, 5, 6, 7, 0xA, 0xF] {
            assert_eq!(
                Data128Message::parse(&[0x5000_0000 | status << 20, 0, 0, 0]),
                None
            );
        }
        assert_eq!(Data128Message::parse(&[0x5001_0000]), None);
        assert_eq!(Data128Message::parse(&[]), None);
    }

    /// Writers give packets that read back to the same values (DEC-UMP-003).
    #[test]
    fn prop_data128_writers_round_trip() {
        proptest!(|(grp in 0u8..16, stream in any::<u8>(), payload in prop::collection::vec(any::<u8>(), 0..80),
                    b2 in 0u8..0x80, b3 in 0u8..0x80, mds in 0u8..16, chunks in 1u16..100)| {
            let mut r = SysEx8Reassembler::<128>::new(g(grp), stream);
            let packets: Vec<_> = SysEx8Packets::new(g(grp), stream, &payload).map(|p| words(&p)).collect();
            prop_assert_eq!(packets.len(), payload.len().saturating_sub(1) / 13 + 1);
            prop_assert_eq!(push_all(&mut r, &packets), [payload.clone()]);
            let id = ManufacturerId::from_midi1(&[0, b2, b3]).unwrap();
            prop_assert_eq!(ManufacturerId::new(id.value()), Ok(id));
            prop_assert_eq!(id.to_midi1(), [0, b2, b3]);
            if b3 != 0 {
                let one = ManufacturerId::from_midi1(&[b3]).unwrap();
                prop_assert_eq!(one.to_midi1(), [b3, 0, 0]);
            }
            let chunk: Vec<_> = MdsChunkPackets::try_new(g(grp), mds, chunks, chunks, ids(), &payload).unwrap().collect();
            let Data128Message::MdsHeader(h) = chunk[0] else { panic!("Header first") };
            prop_assert_eq!(h.number_of_valid_bytes() as usize, 16 + 2 * payload.len().div_ceil(14) + payload.len());
            let mut data = Vec::new();
            for m in &chunk[1..] {
                let Data128Message::MdsPayload(p) = m else { panic!("Payload after Header") };
                prop_assert_eq!(p.mds_id(), mds);
                data.extend_from_slice(&p.data());
            }
            prop_assert_eq!(&data[..payload.len()], payload.as_slice());
            prop_assert!(data[payload.len()..].iter().all(|&b| b == 0));
            for m in chunk {
                prop_assert_eq!(Data128Message::parse(&m.into_words()), Some(m));
            }
        });
    }

    /// The parser and the reassembler never panic, and a parsed packet writes back to words
    /// that parse to the same packet.
    #[test]
    fn prop_data128_parse_never_panics_and_round_trips() {
        proptest!(|(w in any::<[u32; 4]>())| {
            let w0 = 0x5000_0000 | (w[0] & 0x0FFF_FFFF);
            let words = [w0, w[1], w[2], w[3]];
            if let Some(m) = Data128Message::parse(&words) {
                prop_assert_eq!(Data128Message::parse(&m.into_words()), Some(m));
            }
            let _ = Data128Message::parse(&w);
            let _ = Data128Message::parse(&w[..3]);
            let mut r = SysEx8Reassembler::<4>::new(g(((w0 >> 24) & 0xF) as u8), (w0 >> 8) as u8);
            let _ = r.push(&words);
            let _ = r.push(&w);
            let _ = r.push(&[]);
        });
    }
}
