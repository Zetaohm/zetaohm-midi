//! System Common and System Real Time Messages: Message Type 0x1 (M2-104-UM 7.6, Figure 83,
//! Table 17, Appendix F Table 27).
//!
//! - Format: one 32-bit word. `mt=1` (bits 31..28), Group (27..24), the MIDI 1.0 status byte
//!   (23..16), then MIDI 1.0 bytes 2 and 3 (15..0).
//! - The messages carry the same data as in the MIDI 1.0 Specification. Each data byte has a
//!   reserved most significant bit. Unused bytes are Reserved: writers set them to zero and
//!   readers do not use them (7.6, 2.1.3).
//! - Status values 0xF0, 0xF4, 0xF5, 0xF7, 0xF9 and 0xFD are reserved (Table 17);
//!   [`SystemMessage::parse`] gives `None` for them.
//! - Song Position Pointer data is LSB before MSB, as in the MIDI 1.0 Protocol (Table 17 note).
//! - Message Type and Status are read-only. Structs have no public `from_bits` (DEC-UMP-002).

use bitfield_struct::bitfield;

use crate::packet::{put, take, Group, MessageType, UmpError, UmpWords};

/// Status byte of a System Common or System Real Time message, bits 23..16 (Table 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SystemStatus {
    /// 0xF1: MIDI Time Code.
    MidiTimeCode = 0xF1,
    /// 0xF2: Song Position Pointer.
    SongPositionPointer = 0xF2,
    /// 0xF3: Song Select.
    SongSelect = 0xF3,
    /// 0xF6: Tune Request.
    TuneRequest = 0xF6,
    /// 0xF8: Timing Clock.
    TimingClock = 0xF8,
    /// 0xFA: Start.
    Start = 0xFA,
    /// 0xFB: Continue.
    Continue = 0xFB,
    /// 0xFC: Stop.
    Stop = 0xFC,
    /// 0xFE: Active Sensing.
    ActiveSensing = 0xFE,
    /// 0xFF: Reset.
    Reset = 0xFF,
}

impl SystemStatus {
    /// The status byte.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The System status for `value`, or `None` for a reserved status (0xF0, 0xF4, 0xF5, 0xF7,
    /// 0xF9, 0xFD) or a byte that is not a System Common or Real Time status.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0xF1 => Some(Self::MidiTimeCode),
            0xF2 => Some(Self::SongPositionPointer),
            0xF3 => Some(Self::SongSelect),
            0xF6 => Some(Self::TuneRequest),
            0xF8 => Some(Self::TimingClock),
            0xFA => Some(Self::Start),
            0xFB => Some(Self::Continue),
            0xFC => Some(Self::Stop),
            0xFE => Some(Self::ActiveSensing),
            0xFF => Some(Self::Reset),
            _ => None,
        }
    }
}

/// Word 0 with Message Type 0x1 and Status bits for `status`.
const fn header(status: SystemStatus) -> u32 {
    ((MessageType::System.into_bits() as u32) << 28) | ((status.into_bits() as u32) << 16)
}

/// Reads word 0 of a System packet with `status`, or gives an error. Clears the bits in
/// `reserved` (2.1.3).
fn decode(words: &[u32], status: SystemStatus, reserved: u32) -> Result<u32, UmpError> {
    let [w0] = take::<1>(words)?;
    let mt = MessageType::System.into_bits() as u32;
    if w0 >> 28 != mt || (w0 >> 16) & 0xFF != status.into_bits() as u32 {
        return Err(UmpError::UnexpectedMessage);
    }
    Ok(w0 & !reserved)
}

/// MIDI Time Code, status 0xF1: byte 3 is `0nnndddd` (Table 17).
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct MidiTimeCode {
    /// Message Type, 4 bits: 0x1.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 8 bits: 0xF1.
    #[bits(8, access = RO)]
    pub status: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Time code type (`nnn`), 3 bits.
    #[bits(3, access = RO)]
    pub time_code_type: u8,
    /// Value (`dddd`), 4 bits.
    #[bits(4, access = RO)]
    pub value: u8,
    /// Reserved, 8 bits.
    #[bits(8)]
    __: u8,
}

impl MidiTimeCode {
    const RESERVED: u32 = 0x0000_80FF;

    /// A MIDI Time Code message. Gives `OutOfRange` for a type above 7 or a value above 15.
    pub const fn try_new(group: Group, time_code_type: u8, value: u8) -> Result<Self, UmpError> {
        if time_code_type > 7 || value > 15 {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(SystemStatus::MidiTimeCode)
                | ((group.into_bits() as u32) << 24)
                | ((time_code_type as u32) << 12)
                | ((value as u32) << 8),
        ))
    }
}

/// Song Position Pointer, status 0xF2: a 14-bit position, LSB 7 bits in byte 3 and MSB 7 bits
/// in byte 4 (Table 17 note).
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct SongPositionPointer {
    /// Message Type, 4 bits: 0x1.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 8 bits: 0xF2.
    #[bits(8, access = RO)]
    pub status: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Position LSB (`lllllll`), 7 bits.
    #[bits(7, access = RO)]
    pub position_lsb: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Position MSB (`mmmmmmm`), 7 bits.
    #[bits(7, access = RO)]
    pub position_msb: u8,
}

impl SongPositionPointer {
    const RESERVED: u32 = 0x0000_8080;

    /// A Song Position Pointer message. Gives `OutOfRange` for a position above 16,383.
    pub const fn try_new(group: Group, position: u16) -> Result<Self, UmpError> {
        if position > 0x3FFF {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(SystemStatus::SongPositionPointer)
                | ((group.into_bits() as u32) << 24)
                | (((position & 0x7F) as u32) << 8)
                | ((position >> 7) as u32),
        ))
    }

    /// The 14-bit position: MSB 7 bits from byte 4, LSB 7 bits from byte 3.
    pub fn position(&self) -> u16 {
        ((self.position_msb() as u16) << 7) | self.position_lsb() as u16
    }
}

/// Song Select, status 0xF3: a 7-bit song number in byte 3.
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct SongSelect {
    /// Message Type, 4 bits: 0x1.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 8 bits: 0xF3.
    #[bits(8, access = RO)]
    pub status: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Song (`sssssss`), 7 bits.
    #[bits(7, access = RO)]
    pub song: u8,
    /// Reserved, 8 bits.
    #[bits(8)]
    __: u8,
}

impl SongSelect {
    const RESERVED: u32 = 0x0000_80FF;

    /// A Song Select message. Gives `OutOfRange` for a song above 127.
    pub const fn try_new(group: Group, song: u8) -> Result<Self, UmpError> {
        if song > 0x7F {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(SystemStatus::SongSelect)
                | ((group.into_bits() as u32) << 24)
                | ((song as u32) << 8),
        ))
    }
}

/// Tune Request, status 0xF6. Bytes 3 and 4 are reserved.
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct TuneRequest {
    /// Message Type, 4 bits: 0x1.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 8 bits: 0xF6.
    #[bits(8, access = RO)]
    pub status: u8,
    /// Reserved, 16 bits.
    #[bits(16)]
    __: u16,
}

/// Timing Clock, status 0xF8. Bytes 3 and 4 are reserved.
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct TimingClock {
    /// Message Type, 4 bits: 0x1.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 8 bits: 0xF8.
    #[bits(8, access = RO)]
    pub status: u8,
    /// Reserved, 16 bits.
    #[bits(16)]
    __: u16,
}

/// Start, status 0xFA. Bytes 3 and 4 are reserved.
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Start {
    /// Message Type, 4 bits: 0x1.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 8 bits: 0xFA.
    #[bits(8, access = RO)]
    pub status: u8,
    /// Reserved, 16 bits.
    #[bits(16)]
    __: u16,
}

/// Continue, status 0xFB. Bytes 3 and 4 are reserved.
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Continue {
    /// Message Type, 4 bits: 0x1.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 8 bits: 0xFB.
    #[bits(8, access = RO)]
    pub status: u8,
    /// Reserved, 16 bits.
    #[bits(16)]
    __: u16,
}

/// Stop, status 0xFC. Bytes 3 and 4 are reserved.
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Stop {
    /// Message Type, 4 bits: 0x1.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 8 bits: 0xFC.
    #[bits(8, access = RO)]
    pub status: u8,
    /// Reserved, 16 bits.
    #[bits(16)]
    __: u16,
}

/// Active Sensing, status 0xFE. Bytes 3 and 4 are reserved.
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct ActiveSensing {
    /// Message Type, 4 bits: 0x1.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 8 bits: 0xFE.
    #[bits(8, access = RO)]
    pub status: u8,
    /// Reserved, 16 bits.
    #[bits(16)]
    __: u16,
}

/// Reset, status 0xFF. Bytes 3 and 4 are reserved.
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Reset {
    /// Message Type, 4 bits: 0x1.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 8 bits: 0xFF.
    #[bits(8, access = RO)]
    pub status: u8,
    /// Reserved, 16 bits.
    #[bits(16)]
    __: u16,
}

impl TuneRequest {
    /// A Tune Request on `group`.
    pub const fn new(group: Group) -> Self {
        Self(header(SystemStatus::TuneRequest) | ((group.into_bits() as u32) << 24))
    }
}

impl TimingClock {
    /// A Timing Clock on `group`.
    pub const fn new(group: Group) -> Self {
        Self(header(SystemStatus::TimingClock) | ((group.into_bits() as u32) << 24))
    }
}

impl Start {
    /// A Start on `group`.
    pub const fn new(group: Group) -> Self {
        Self(header(SystemStatus::Start) | ((group.into_bits() as u32) << 24))
    }
}

impl Continue {
    /// A Continue on `group`.
    pub const fn new(group: Group) -> Self {
        Self(header(SystemStatus::Continue) | ((group.into_bits() as u32) << 24))
    }
}

impl Stop {
    /// A Stop on `group`.
    pub const fn new(group: Group) -> Self {
        Self(header(SystemStatus::Stop) | ((group.into_bits() as u32) << 24))
    }
}

impl ActiveSensing {
    /// An Active Sensing on `group`.
    pub const fn new(group: Group) -> Self {
        Self(header(SystemStatus::ActiveSensing) | ((group.into_bits() as u32) << 24))
    }
}

impl Reset {
    /// A Reset on `group`.
    pub const fn new(group: Group) -> Self {
        Self(header(SystemStatus::Reset) | ((group.into_bits() as u32) << 24))
    }
}

/// Reserved bits of a message with no data bytes: bytes 3 and 4.
const NO_DATA_RESERVED: u32 = 0x0000_FFFF;

macro_rules! system_words {
    ($($ty:ident => $status:ident, $reserved:expr);* $(;)?) => {$(
        impl UmpWords for $ty {
            const WORDS: usize = 1;

            fn from_words(words: &[u32]) -> Result<Self, UmpError> {
                decode(words, SystemStatus::$status, $reserved).map(Self)
            }

            fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
                put(&[self.0], out)
            }
        }
    )*};
}

system_words! {
    MidiTimeCode => MidiTimeCode, MidiTimeCode::RESERVED;
    SongPositionPointer => SongPositionPointer, SongPositionPointer::RESERVED;
    SongSelect => SongSelect, SongSelect::RESERVED;
    TuneRequest => TuneRequest, NO_DATA_RESERVED;
    TimingClock => TimingClock, NO_DATA_RESERVED;
    Start => Start, NO_DATA_RESERVED;
    Continue => Continue, NO_DATA_RESERVED;
    Stop => Stop, NO_DATA_RESERVED;
    ActiveSensing => ActiveSensing, NO_DATA_RESERVED;
    Reset => Reset, NO_DATA_RESERVED;
}

/// One System Common or System Real Time message (Table 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemMessage {
    /// MIDI Time Code (0xF1).
    MidiTimeCode(MidiTimeCode),
    /// Song Position Pointer (0xF2).
    SongPositionPointer(SongPositionPointer),
    /// Song Select (0xF3).
    SongSelect(SongSelect),
    /// Tune Request (0xF6).
    TuneRequest(TuneRequest),
    /// Timing Clock (0xF8).
    TimingClock(TimingClock),
    /// Start (0xFA).
    Start(Start),
    /// Continue (0xFB).
    Continue(Continue),
    /// Stop (0xFC).
    Stop(Stop),
    /// Active Sensing (0xFE).
    ActiveSensing(ActiveSensing),
    /// Reset (0xFF).
    Reset(Reset),
}

impl SystemMessage {
    /// Decodes one System packet by its status byte. Gives `None` for short input, a Message
    /// Type other than 0x1, or a reserved status. Never panics.
    pub fn parse(words: &[u32]) -> Option<Self> {
        let status = SystemStatus::from_bits((*words.first()? >> 16) as u8)?;
        match status {
            SystemStatus::MidiTimeCode => {
                MidiTimeCode::from_words(words).ok().map(Self::MidiTimeCode)
            }
            SystemStatus::SongPositionPointer => SongPositionPointer::from_words(words)
                .ok()
                .map(Self::SongPositionPointer),
            SystemStatus::SongSelect => SongSelect::from_words(words).ok().map(Self::SongSelect),
            SystemStatus::TuneRequest => TuneRequest::from_words(words).ok().map(Self::TuneRequest),
            SystemStatus::TimingClock => TimingClock::from_words(words).ok().map(Self::TimingClock),
            SystemStatus::Start => Start::from_words(words).ok().map(Self::Start),
            SystemStatus::Continue => Continue::from_words(words).ok().map(Self::Continue),
            SystemStatus::Stop => Stop::from_words(words).ok().map(Self::Stop),
            SystemStatus::ActiveSensing => ActiveSensing::from_words(words)
                .ok()
                .map(Self::ActiveSensing),
            SystemStatus::Reset => Reset::from_words(words).ok().map(Self::Reset),
        }
    }

    /// The packet as one 32-bit word.
    pub(crate) const fn into_bits(self) -> u32 {
        match self {
            Self::MidiTimeCode(m) => m.0,
            Self::SongPositionPointer(m) => m.0,
            Self::SongSelect(m) => m.0,
            Self::TuneRequest(m) => m.0,
            Self::TimingClock(m) => m.0,
            Self::Start(m) => m.0,
            Self::Continue(m) => m.0,
            Self::Stop(m) => m.0,
            Self::ActiveSensing(m) => m.0,
            Self::Reset(m) => m.0,
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;

    fn word<T: UmpWords>(m: &T) -> u32 {
        let mut out = [0u32; 1];
        assert_eq!(m.to_words(&mut out), Ok(1));
        out[0]
    }

    fn g(i: u8) -> Group {
        Group::new(i).unwrap()
    }

    #[test]
    fn system_status_matches_table_17() {
        let defined = [0xF1, 0xF2, 0xF3, 0xF6, 0xF8, 0xFA, 0xFB, 0xFC, 0xFE, 0xFF];
        for b in 0..=0xFFu8 {
            match SystemStatus::from_bits(b) {
                Some(s) => {
                    assert!(defined.contains(&b), "{b:#x}");
                    assert_eq!(s.into_bits(), b);
                }
                None => assert!(!defined.contains(&b), "{b:#x}"),
            }
        }
        for reserved in [0xF0, 0xF4, 0xF5, 0xF7, 0xF9, 0xFD] {
            assert_eq!(SystemStatus::from_bits(reserved), None);
        }
    }

    #[test]
    fn midi_time_code_matches_table_27() {
        // Byte 3 = 0nnndddd: type 3, value 5 gives 0x35.
        let m = MidiTimeCode::try_new(g(2), 3, 5).unwrap();
        assert_eq!(word(&m), 0x12F1_3500);
        assert_eq!(
            (m.message_type(), m.group(), m.status()),
            (MessageType::System, g(2), 0xF1)
        );
        assert_eq!((m.time_code_type(), m.value()), (3, 5));
        assert_eq!(MidiTimeCode::from_words(&[0x12F1_3500]), Ok(m));
        assert_eq!(MidiTimeCode::try_new(g(0), 8, 0), Err(UmpError::OutOfRange));
        assert_eq!(
            MidiTimeCode::try_new(g(0), 0, 16),
            Err(UmpError::OutOfRange)
        );
    }

    #[test]
    fn song_position_pointer_matches_table_27() {
        // Table 17 note: LSB before MSB. 0x1234 = MSB 0x24, LSB 0x34.
        let m = SongPositionPointer::try_new(g(0), 0x1234).unwrap();
        assert_eq!(word(&m), 0x10F2_3424);
        assert_eq!(
            (m.position_lsb(), m.position_msb(), m.position()),
            (0x34, 0x24, 0x1234)
        );
        assert_eq!(
            word(&SongPositionPointer::try_new(g(15), 0x3FFF).unwrap()),
            0x1FF2_7F7F
        );
        assert_eq!(
            SongPositionPointer::try_new(g(0), 0x4000),
            Err(UmpError::OutOfRange)
        );
    }

    #[test]
    fn song_select_matches_table_27() {
        let m = SongSelect::try_new(g(1), 0x7F).unwrap();
        assert_eq!(word(&m), 0x11F3_7F00);
        assert_eq!(m.song(), 0x7F);
        assert_eq!(SongSelect::try_new(g(1), 0x80), Err(UmpError::OutOfRange));
    }

    #[test]
    fn no_data_messages_match_table_27() {
        assert_eq!(word(&TuneRequest::new(g(0))), 0x10F6_0000);
        assert_eq!(word(&TimingClock::new(g(3))), 0x13F8_0000);
        assert_eq!(word(&Start::new(g(4))), 0x14FA_0000);
        assert_eq!(word(&Continue::new(g(5))), 0x15FB_0000);
        assert_eq!(word(&Stop::new(g(6))), 0x16FC_0000);
        assert_eq!(word(&ActiveSensing::new(g(7))), 0x17FE_0000);
        assert_eq!(word(&Reset::new(g(15))), 0x1FFF_0000);
        assert_eq!(TimingClock::new(g(3)).group().number(), 4);
        assert_eq!(
            TimingClock::new(g(3)).with_group(g(9)),
            TimingClock::new(g(9))
        );
    }

    #[test]
    fn system_reader_ignores_reserved_bits() {
        // 7.6: unused bytes are Reserved; 2.1.3: readers do not use them.
        assert_eq!(
            TimingClock::from_words(&[0x13F8_FFFF]),
            Ok(TimingClock::new(g(3)))
        );
        let mtc = MidiTimeCode::from_words(&[0x12F1_B5FF]).unwrap();
        assert_eq!(word(&mtc), 0x12F1_3500);
        let spp = SongPositionPointer::from_words(&[0x10F2_B4A4]).unwrap();
        assert_eq!(word(&spp), 0x10F2_3424);
        let ss = SongSelect::from_words(&[0x11F3_FFFF]).unwrap();
        assert_eq!(word(&ss), 0x11F3_7F00);
    }

    #[test]
    fn system_from_words_rejects_other_messages() {
        assert_eq!(
            TimingClock::from_words(&[0x13FA_0000]),
            Err(UmpError::UnexpectedMessage)
        );
        assert_eq!(
            TimingClock::from_words(&[0x23F8_0000]),
            Err(UmpError::UnexpectedMessage)
        );
        assert_eq!(
            TimingClock::from_words(&[]),
            Err(UmpError::Truncated { needed: 1, got: 0 })
        );
        assert_eq!(
            Reset::new(g(0)).to_words(&mut []),
            Err(UmpError::BufferTooSmall { needed: 1, got: 0 })
        );
    }

    #[test]
    fn system_message_parse_dispatches_on_status() {
        assert!(matches!(
            SystemMessage::parse(&[0x12F1_3500]),
            Some(SystemMessage::MidiTimeCode(_))
        ));
        assert!(matches!(
            SystemMessage::parse(&[0x10F2_3424]),
            Some(SystemMessage::SongPositionPointer(m)) if m.position() == 0x1234
        ));
        assert!(matches!(
            SystemMessage::parse(&[0x11F3_0500]),
            Some(SystemMessage::SongSelect(_))
        ));
        assert!(matches!(
            SystemMessage::parse(&[0x10F6_0000]),
            Some(SystemMessage::TuneRequest(_))
        ));
        assert!(matches!(
            SystemMessage::parse(&[0x10F8_0000]),
            Some(SystemMessage::TimingClock(_))
        ));
        assert!(matches!(
            SystemMessage::parse(&[0x10FA_0000]),
            Some(SystemMessage::Start(_))
        ));
        assert!(matches!(
            SystemMessage::parse(&[0x10FB_0000]),
            Some(SystemMessage::Continue(_))
        ));
        assert!(matches!(
            SystemMessage::parse(&[0x10FC_0000]),
            Some(SystemMessage::Stop(_))
        ));
        assert!(matches!(
            SystemMessage::parse(&[0x10FE_0000]),
            Some(SystemMessage::ActiveSensing(_))
        ));
        assert!(matches!(
            SystemMessage::parse(&[0x10FF_0000]),
            Some(SystemMessage::Reset(_))
        ));
        for reserved in [0xF0u32, 0xF4, 0xF5, 0xF7, 0xF9, 0xFD, 0x90] {
            assert_eq!(
                SystemMessage::parse(&[0x1000_0000 | (reserved << 16)]),
                None
            );
        }
        assert_eq!(SystemMessage::parse(&[]), None);
        let m = SystemMessage::parse(&[0x1AF8_0000]).unwrap();
        assert_eq!(m.into_bits(), 0x1AF8_0000);
    }

    #[test]
    fn prop_system_data_round_trip() {
        // DEC-UMP-003: every in-range value round-trips; out-of-range values are rejected.
        proptest!(|(gi in 0u8..16, t in any::<u8>(), v in any::<u8>(), pos in any::<u16>(), song in any::<u8>())| {
            let grp = g(gi);
            match MidiTimeCode::try_new(grp, t, v) {
                Ok(m) => {
                    prop_assert!(t <= 7 && v <= 15);
                    prop_assert_eq!(MidiTimeCode::from_words(&[word(&m)]), Ok(m));
                    prop_assert_eq!((m.time_code_type(), m.value(), m.group()), (t, v, grp));
                }
                Err(e) => {
                    prop_assert!(t > 7 || v > 15);
                    prop_assert_eq!(e, UmpError::OutOfRange);
                }
            }
            match SongPositionPointer::try_new(grp, pos) {
                Ok(m) => {
                    prop_assert!(pos <= 0x3FFF);
                    prop_assert_eq!(m.position(), pos);
                    prop_assert_eq!(SongPositionPointer::from_words(&[word(&m)]), Ok(m));
                }
                Err(_) => prop_assert!(pos > 0x3FFF),
            }
            match SongSelect::try_new(grp, song) {
                Ok(m) => {
                    prop_assert!(song <= 0x7F);
                    prop_assert_eq!(SongSelect::from_words(&[word(&m)]), Ok(m));
                }
                Err(_) => prop_assert!(song > 0x7F),
            }
        });
    }

    #[test]
    fn prop_system_reader_ignores_reserved_bits() {
        // 2.1.3: for every defined status, any pattern of reserved bits decodes to the cleared word.
        proptest!(|(w in any::<u32>(), si in 0usize..10)| {
            let status = [0xF1u32, 0xF2, 0xF3, 0xF6, 0xF8, 0xFA, 0xFB, 0xFC, 0xFE, 0xFF][si];
            let w = 0x1000_0000 | (w & 0x0F00_FFFF) | (status << 16);
            let m = SystemMessage::parse(&[w]).unwrap();
            let reserved = match status {
                0xF1 | 0xF3 => 0x0000_80FF,
                0xF2 => 0x0000_8080,
                _ => 0x0000_FFFF,
            };
            prop_assert_eq!(m.into_bits(), w & !reserved);
            prop_assert_eq!(SystemMessage::parse(&[w & !reserved]), Some(m));
        });
    }
}
