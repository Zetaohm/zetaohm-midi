//! Utility Messages: Message Type 0x0 (M2-104-UM 7.2, Figure 26, Table 26).
//!
//! - Format: one 32-bit word. `mt=0` (bits 31..28), reserved (27..24), Status (23..20),
//!   data or reserved (19..0).
//! - Utility Messages are Groupless (version history 1.1; Table 26 shows bits 27..24 as
//!   reserved). The API has no Group, and writers set bits 27..24 to zero.
//! - Status values (Table 26): 0x0 NOOP, 0x1 JR Clock, 0x2 JR Timestamp, 0x3 Delta Clockstamp
//!   Ticks Per Quarter Note, 0x4 Delta Clockstamp. Other values are not defined in this
//!   version; [`UtilityMessage::parse`] gives `None` for them.
//! - Readers ignore reserved bits (2.1.3): `from_words` clears them, so a word with reserved
//!   bits set decodes to the same value as the word with them cleared (DEC-UMP-002).
//! - Message Type and Status are read-only. Structs have no public `from_bits` (DEC-UMP-002).

use bitfield_struct::bitfield;

use crate::packet::{put, take, MessageType, UmpError, UmpWords};

/// Status of a Utility Message, bits 23..20 (Table 26).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum UtilityStatus {
    /// 0x0: NOOP (7.2.1).
    Noop = 0x0,
    /// 0x1: JR Clock (7.2.2.1).
    JrClock = 0x1,
    /// 0x2: JR Timestamp (7.2.2.2).
    JrTimestamp = 0x2,
    /// 0x3: Delta Clockstamp Ticks Per Quarter Note (7.2.3.1).
    DeltaClockstampTpq = 0x3,
    /// 0x4: Delta Clockstamp (7.2.3.2).
    DeltaClockstamp = 0x4,
}

impl UtilityStatus {
    /// The 4-bit value of this Status.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Status for the low 4 bits of `value`, or `None` for a value Table 26 does not define.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value & 0xF {
            0x0 => Some(Self::Noop),
            0x1 => Some(Self::JrClock),
            0x2 => Some(Self::JrTimestamp),
            0x3 => Some(Self::DeltaClockstampTpq),
            0x4 => Some(Self::DeltaClockstamp),
            _ => None,
        }
    }
}

/// Reads word 0 of a Utility packet with Status `status`, or gives an error.
/// Clears the bits in `reserved` (2.1.3).
fn decode(words: &[u32], status: UtilityStatus, reserved: u32) -> Result<u32, UmpError> {
    let [w0] = take::<1>(words)?;
    let mt = MessageType::Utility.into_bits() as u32;
    if w0 >> 28 != mt || (w0 >> 20) & 0xF != status.into_bits() as u32 {
        return Err(UmpError::UnexpectedMessage);
    }
    Ok(w0 & !reserved)
}

/// NOOP: no operation (7.2.1, Figure 27). Status 0x0; bits 19..0 are 0x00000.
///
/// The reader treats bits 19..0 as reserved: a NOOP with non-zero bits there decodes as NOOP.
#[bitfield(u32, order = Msb, conversion = false, hash = true)]
#[derive(PartialEq, Eq)]
pub struct Noop {
    /// Message Type, 4 bits: 0x0.
    #[bits(4, default = MessageType::Utility, access = RO)]
    pub message_type: MessageType,
    /// Reserved (Groupless), 4 bits.
    #[bits(4)]
    __: u8,
    /// Status, 4 bits: 0x0.
    #[bits(4, default = 0x0, access = RO)]
    pub status: u8,
    /// 0x00000, 20 bits.
    #[bits(20)]
    __: u32,
}

impl Noop {
    const RESERVED: u32 = 0x0F0F_FFFF;

    /// The packet as one 32-bit word.
    pub(crate) const fn into_bits(self) -> u32 {
        self.0
    }
}

/// JR Clock: the current time of the Sender (7.2.2.1, Figure 29). Status 0x1.
#[bitfield(u32, order = Msb, conversion = false, hash = true)]
#[derive(PartialEq, Eq)]
pub struct JrClock {
    /// Message Type, 4 bits: 0x0.
    #[bits(4, default = MessageType::Utility, access = RO)]
    pub message_type: MessageType,
    /// Reserved (Groupless), 4 bits.
    #[bits(4)]
    __: u8,
    /// Status, 4 bits: 0x1.
    #[bits(4, default = 0x1, access = RO)]
    pub status: u8,
    /// Reserved, 4 bits.
    #[bits(4)]
    __: u8,
    /// Sender Clock Time, 16 bits, in ticks of 1/31250 s (32 µs). Wraps every 2.09712 s.
    pub time: u16,
}

impl JrClock {
    const RESERVED: u32 = 0x0F0F_0000;

    /// The packet as one 32-bit word.
    pub(crate) const fn into_bits(self) -> u32 {
        self.0
    }
}

/// JR Timestamp: the time of the following message(s) (7.2.2.2, Figure 31). Status 0x2.
#[bitfield(u32, order = Msb, conversion = false, hash = true)]
#[derive(PartialEq, Eq)]
pub struct JrTimestamp {
    /// Message Type, 4 bits: 0x0.
    #[bits(4, default = MessageType::Utility, access = RO)]
    pub message_type: MessageType,
    /// Reserved (Groupless), 4 bits.
    #[bits(4)]
    __: u8,
    /// Status, 4 bits: 0x2.
    #[bits(4, default = 0x2, access = RO)]
    pub status: u8,
    /// Reserved, 4 bits.
    #[bits(4)]
    __: u8,
    /// Sender Clock Timestamp, 16 bits, in ticks of 1/31250 s (32 µs).
    pub timestamp: u16,
}

impl JrTimestamp {
    const RESERVED: u32 = 0x0F0F_0000;

    /// The packet as one 32-bit word.
    pub(crate) const fn into_bits(self) -> u32 {
        self.0
    }
}

/// Delta Clockstamp Ticks Per Quarter Note (DCTPQ) (7.2.3.1, Figure 33). Status 0x3.
///
/// Number of Ticks Per Quarter Note is 1 to 65,535; 0 is Reserved. [`Self::try_new`] rejects 0.
/// The reader gives the value as received, 0 included.
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct DeltaClockstampTpq {
    /// Message Type, 4 bits: 0x0.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Reserved (Groupless), 4 bits.
    #[bits(4)]
    __: u8,
    /// Status, 4 bits: 0x3.
    #[bits(4, access = RO)]
    pub status: u8,
    /// Reserved, 4 bits.
    #[bits(4)]
    __: u8,
    /// Number of Ticks Per Quarter Note, 16 bits: 1 to 65,535 (0 = Reserved).
    #[bits(16, access = RO)]
    pub ticks_per_quarter_note: u16,
}

impl DeltaClockstampTpq {
    const RESERVED: u32 = 0x0F0F_0000;

    /// A DCTPQ message. Gives `OutOfRange` for 0, which is Reserved (7.2.3.1).
    pub const fn try_new(ticks_per_quarter_note: u16) -> Result<Self, UmpError> {
        if ticks_per_quarter_note == 0 {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(0x0030_0000 | ticks_per_quarter_note as u32))
    }

    /// The packet as one 32-bit word.
    pub(crate) const fn into_bits(self) -> u32 {
        self.0
    }
}

/// Delta Clockstamp (DC): ticks since last event (7.2.3.2, Figure 34). Status 0x4.
///
/// [`Self::try_new`] and `from_words` are the only ways to get a value (DEC-UMP-002). There is
/// no public `from_bits`:
///
/// ```compile_fail
/// let m = zmidi_ump::DeltaClockstamp::from_bits(0x0050_0000);
/// ```
///
/// and no setter for the read-only fields:
///
/// ```compile_fail
/// let m = zmidi_ump::DeltaClockstamp::try_new(1).unwrap().with_status(0x5);
/// ```
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct DeltaClockstamp {
    /// Message Type, 4 bits: 0x0.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Reserved (Groupless), 4 bits.
    #[bits(4)]
    __: u8,
    /// Status, 4 bits: 0x4.
    #[bits(4, access = RO)]
    pub status: u8,
    /// Number of Ticks Since Last Event, 20 bits: 0 to 1,048,575.
    #[bits(20, access = RO)]
    pub ticks: u32,
}

impl DeltaClockstamp {
    const RESERVED: u32 = 0x0F00_0000;

    /// Largest value of the 20-bit ticks field.
    const MAX_TICKS: u32 = 0xF_FFFF;

    /// A DC message. Gives `OutOfRange` for a value above 1,048,575 (20 bits).
    pub const fn try_new(ticks: u32) -> Result<Self, UmpError> {
        if ticks > Self::MAX_TICKS {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(0x0040_0000 | ticks))
    }

    /// The packet as one 32-bit word.
    pub(crate) const fn into_bits(self) -> u32 {
        self.0
    }
}

macro_rules! utility_words {
    ($($ty:ident => $status:ident),* $(,)?) => {$(
        impl UmpWords for $ty {
            const WORDS: usize = 1;

            fn from_words(words: &[u32]) -> Result<Self, UmpError> {
                decode(words, UtilityStatus::$status, Self::RESERVED).map(Self)
            }

            fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
                put(&[self.0], out)
            }
        }
    )*};
}

utility_words! {
    Noop => Noop,
    JrClock => JrClock,
    JrTimestamp => JrTimestamp,
    DeltaClockstampTpq => DeltaClockstampTpq,
    DeltaClockstamp => DeltaClockstamp,
}

/// One Utility Message (Table 26).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UtilityMessage {
    /// NOOP (Status 0x0).
    Noop(Noop),
    /// JR Clock (Status 0x1).
    JrClock(JrClock),
    /// JR Timestamp (Status 0x2).
    JrTimestamp(JrTimestamp),
    /// Delta Clockstamp Ticks Per Quarter Note (Status 0x3).
    DeltaClockstampTpq(DeltaClockstampTpq),
    /// Delta Clockstamp (Status 0x4).
    DeltaClockstamp(DeltaClockstamp),
}

impl UtilityMessage {
    /// Decodes one Utility packet by its Status. Gives `None` for short input, a Message Type
    /// other than 0x0, or a Status that Table 26 does not define. Never panics.
    pub fn parse(words: &[u32]) -> Option<Self> {
        let status = UtilityStatus::from_bits((*words.first()? >> 20) as u8)?;
        match status {
            UtilityStatus::Noop => Noop::from_words(words).ok().map(Self::Noop),
            UtilityStatus::JrClock => JrClock::from_words(words).ok().map(Self::JrClock),
            UtilityStatus::JrTimestamp => {
                JrTimestamp::from_words(words).ok().map(Self::JrTimestamp)
            }
            UtilityStatus::DeltaClockstampTpq => DeltaClockstampTpq::from_words(words)
                .ok()
                .map(Self::DeltaClockstampTpq),
            UtilityStatus::DeltaClockstamp => DeltaClockstamp::from_words(words)
                .ok()
                .map(Self::DeltaClockstamp),
        }
    }

    /// The packet as one 32-bit word.
    pub(crate) const fn into_bits(self) -> u32 {
        match self {
            Self::Noop(m) => m.into_bits(),
            Self::JrClock(m) => m.into_bits(),
            Self::JrTimestamp(m) => m.into_bits(),
            Self::DeltaClockstampTpq(m) => m.into_bits(),
            Self::DeltaClockstamp(m) => m.into_bits(),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;

    /// Word written by `to_words` for a message.
    fn word<T: UmpWords>(m: &T) -> u32 {
        let mut out = [0u32; 1];
        assert_eq!(m.to_words(&mut out), Ok(1));
        out[0]
    }

    #[test]
    fn utility_status_matches_table_26() {
        for (bits, status) in [
            (0x0, UtilityStatus::Noop),
            (0x1, UtilityStatus::JrClock),
            (0x2, UtilityStatus::JrTimestamp),
            (0x3, UtilityStatus::DeltaClockstampTpq),
            (0x4, UtilityStatus::DeltaClockstamp),
        ] {
            assert_eq!(UtilityStatus::from_bits(bits), Some(status));
            assert_eq!(status.into_bits(), bits);
        }
        for bits in 0x5..=0xF {
            assert_eq!(UtilityStatus::from_bits(bits), None);
        }
    }

    #[test]
    fn noop_matches_figure_27() {
        let m = Noop::new();
        assert_eq!(m.message_type(), MessageType::Utility);
        assert_eq!(m.status(), 0x0);
        assert_eq!(word(&m), 0x0000_0000);
        assert_eq!(Noop::default(), m);
        assert_eq!(Noop::from_words(&[0x0000_0000]), Ok(m));
    }

    #[test]
    fn jr_clock_matches_figure_29() {
        let m = JrClock::new().with_time(0xABCD);
        assert_eq!(m.message_type(), MessageType::Utility);
        assert_eq!(m.status(), 0x1);
        assert_eq!(m.time(), 0xABCD);
        assert_eq!(word(&JrClock::new()), 0x0010_0000);
        assert_eq!(word(&m), 0x0010_ABCD);
        assert_eq!(JrClock::from_words(&[0x0010_ABCD]), Ok(m));
    }

    #[test]
    fn jr_clock_time_round_trip() {
        for t in [0, 1, 0x7FFF, 0x8000, u16::MAX] {
            let m = JrClock::new().with_time(t);
            assert_eq!(JrClock::from_words(&[word(&m)]).map(|m| m.time()), Ok(t));
        }
    }

    #[test]
    fn jr_timestamp_matches_figure_31() {
        let m = JrTimestamp::new().with_timestamp(0x1234);
        assert_eq!(m.message_type(), MessageType::Utility);
        assert_eq!(m.status(), 0x2);
        assert_eq!(word(&JrTimestamp::new()), 0x0020_0000);
        assert_eq!(word(&m), 0x0020_1234);
        for t in [0, 1, 0x8000, u16::MAX] {
            let m = JrTimestamp::new().with_timestamp(t);
            assert_eq!(
                JrTimestamp::from_words(&[word(&m)]).map(|m| m.timestamp()),
                Ok(t)
            );
        }
    }

    #[test]
    fn dctpq_matches_figure_33() {
        let m = DeltaClockstampTpq::try_new(960).unwrap();
        assert_eq!(m.message_type(), MessageType::Utility);
        assert_eq!(m.status(), 0x3);
        assert_eq!(m.ticks_per_quarter_note(), 960);
        assert_eq!(word(&m), 0x0030_03C0);
        for t in [1, 24, 96, 480, 960, u16::MAX] {
            let m = DeltaClockstampTpq::try_new(t).unwrap();
            assert_eq!(
                DeltaClockstampTpq::from_words(&[word(&m)]).map(|m| m.ticks_per_quarter_note()),
                Ok(t)
            );
        }
    }

    #[test]
    fn dctpq_rejects_reserved_zero() {
        // 7.2.3.1: 0 = Reserved. The writer rejects it; the reader gives it as received.
        assert_eq!(DeltaClockstampTpq::try_new(0), Err(UmpError::OutOfRange));
        let m = DeltaClockstampTpq::from_words(&[0x0030_0000]).unwrap();
        assert_eq!(m.ticks_per_quarter_note(), 0);
    }

    #[test]
    fn delta_clockstamp_matches_figure_34() {
        let m = DeltaClockstamp::try_new(0xF_FFFF).unwrap();
        assert_eq!(m.message_type(), MessageType::Utility);
        assert_eq!(m.status(), 0x4);
        assert_eq!(m.ticks(), 1_048_575);
        assert_eq!(word(&m), 0x004F_FFFF);
        for t in [0, 1, 0x8_0000, DeltaClockstamp::MAX_TICKS] {
            let m = DeltaClockstamp::try_new(t).unwrap();
            assert_eq!(
                DeltaClockstamp::from_words(&[word(&m)]).map(|m| m.ticks()),
                Ok(t)
            );
        }
    }

    #[test]
    fn delta_clockstamp_rejects_out_of_range() {
        assert_eq!(
            DeltaClockstamp::try_new(0x10_0000),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            DeltaClockstamp::try_new(u32::MAX),
            Err(UmpError::OutOfRange)
        );
    }

    #[test]
    fn utility_reader_ignores_reserved_bits() {
        // 2.1.3: readers do not depend on reserved bits; bits 27..24 were the Group before 1.1.
        assert_eq!(Noop::from_words(&[0x0F0F_FFFF]), Ok(Noop::new()));
        let clock = JrClock::from_words(&[0x0513_1234]).unwrap();
        assert_eq!(clock, JrClock::new().with_time(0x1234));
        assert_eq!(word(&clock), 0x0010_1234);
        let ts = JrTimestamp::from_words(&[0x0F2F_1234]).unwrap();
        assert_eq!(word(&ts), 0x0020_1234);
        let tpq = DeltaClockstampTpq::from_words(&[0x0A3C_0060]).unwrap();
        assert_eq!(word(&tpq), 0x0030_0060);
        let dc = DeltaClockstamp::from_words(&[0x0F4F_FFFF]).unwrap();
        assert_eq!(word(&dc), 0x004F_FFFF);
    }

    #[test]
    fn utility_writer_sets_reserved_bits_to_zero() {
        // 2.1.3 and Groupless: bits 27..24 and every reserved field are written as zero.
        assert_eq!(word(&Noop::new()) & Noop::RESERVED, 0);
        assert_eq!(
            word(&JrClock::new().with_time(u16::MAX)) & JrClock::RESERVED,
            0
        );
        assert_eq!(
            word(&JrTimestamp::new().with_timestamp(u16::MAX)) & JrTimestamp::RESERVED,
            0
        );
        let tpq = DeltaClockstampTpq::try_new(u16::MAX).unwrap();
        assert_eq!(word(&tpq) & DeltaClockstampTpq::RESERVED, 0);
        let dc = DeltaClockstamp::try_new(DeltaClockstamp::MAX_TICKS).unwrap();
        assert_eq!(word(&dc) & DeltaClockstamp::RESERVED, 0);
    }

    #[test]
    fn utility_from_words_rejects_other_messages() {
        assert_eq!(
            JrClock::from_words(&[0x0020_1234]),
            Err(UmpError::UnexpectedMessage)
        );
        assert_eq!(
            Noop::from_words(&[0x1000_0000]),
            Err(UmpError::UnexpectedMessage)
        );
        assert_eq!(
            DeltaClockstamp::from_words(&[0x0030_0001]),
            Err(UmpError::UnexpectedMessage)
        );
    }

    #[test]
    fn utility_from_words_and_to_words_check_lengths() {
        assert_eq!(
            JrClock::from_words(&[]),
            Err(UmpError::Truncated { needed: 1, got: 0 })
        );
        assert_eq!(
            JrClock::new().to_words(&mut []),
            Err(UmpError::BufferTooSmall { needed: 1, got: 0 })
        );
        // Extra words after the packet are not read or written.
        let mut out = [0xFFFF_FFFF; 2];
        assert_eq!(JrClock::new().to_words(&mut out), Ok(1));
        assert_eq!(out, [0x0010_0000, 0xFFFF_FFFF]);
        assert_eq!(
            JrClock::from_words(&[0x0010_0001, 0x4000_0000]).map(|m| m.time()),
            Ok(1)
        );
    }

    #[test]
    fn utility_message_parse_dispatches_on_status() {
        assert!(matches!(
            UtilityMessage::parse(&[0x0000_0000]),
            Some(UtilityMessage::Noop(_))
        ));
        assert!(matches!(
            UtilityMessage::parse(&[0x0010_0001]),
            Some(UtilityMessage::JrClock(m)) if m.time() == 1
        ));
        assert!(matches!(
            UtilityMessage::parse(&[0x0020_0002]),
            Some(UtilityMessage::JrTimestamp(m)) if m.timestamp() == 2
        ));
        assert!(matches!(
            UtilityMessage::parse(&[0x0030_0060]),
            Some(UtilityMessage::DeltaClockstampTpq(m)) if m.ticks_per_quarter_note() == 96
        ));
        assert!(matches!(
            UtilityMessage::parse(&[0x0040_0010]),
            Some(UtilityMessage::DeltaClockstamp(m)) if m.ticks() == 16
        ));
        for status in 0x5..=0xFu32 {
            assert_eq!(UtilityMessage::parse(&[status << 20]), None);
        }
        assert_eq!(UtilityMessage::parse(&[]), None);
        assert_eq!(UtilityMessage::parse(&[0x2010_0000]), None);
    }

    /// Message Type 0x0 word with Status `status` and any other bits.
    fn utility_word(status: u32) -> impl Strategy<Value = u32> {
        any::<u32>().prop_map(move |w| (w & 0x0F0F_FFFF) | (status << 20))
    }

    /// DEC-UMP-003: every 16-bit time round-trips through the words (REQ-UTIL-002).
    #[test]
    fn prop_jr_clock_round_trip() {
        proptest!(|(t in any::<u16>())| {
            let m = JrClock::new().with_time(t);
            prop_assert_eq!(word(&m), 0x0010_0000 | t as u32);
            prop_assert_eq!(JrClock::from_words(&[word(&m)]), Ok(m));
        });
    }

    /// Every 16-bit timestamp round-trips through the words (REQ-UTIL-003).
    #[test]
    fn prop_jr_timestamp_round_trip() {
        proptest!(|(t in any::<u16>())| {
            let m = JrTimestamp::new().with_timestamp(t);
            prop_assert_eq!(word(&m), 0x0020_0000 | t as u32);
            prop_assert_eq!(JrTimestamp::from_words(&[word(&m)]), Ok(m));
        });
    }

    /// `try_new` accepts exactly 1 to 65,535 (7.2.3.1, REQ-UTIL-004).
    #[test]
    fn prop_dctpq_try_new_accepts_spec_range() {
        proptest!(|(t in any::<u16>())| {
            match DeltaClockstampTpq::try_new(t) {
                Ok(m) => {
                    prop_assert!(t != 0);
                    prop_assert_eq!(word(&m), 0x0030_0000 | t as u32);
                    prop_assert_eq!(DeltaClockstampTpq::from_words(&[word(&m)]), Ok(m));
                }
                Err(e) => {
                    prop_assert_eq!(t, 0);
                    prop_assert_eq!(e, UmpError::OutOfRange);
                }
            }
        });
    }

    /// `try_new` accepts exactly 0 to 1,048,575 (7.2.3.2, REQ-UTIL-005).
    #[test]
    fn prop_delta_clockstamp_try_new_accepts_spec_range() {
        proptest!(|(t in any::<u32>())| {
            match DeltaClockstamp::try_new(t) {
                Ok(m) => {
                    prop_assert!(t <= 0xF_FFFF);
                    prop_assert_eq!(word(&m), 0x0040_0000 | t);
                    prop_assert_eq!(DeltaClockstamp::from_words(&[word(&m)]), Ok(m));
                }
                Err(e) => {
                    prop_assert!(t > 0xF_FFFF);
                    prop_assert_eq!(e, UmpError::OutOfRange);
                }
            }
        });
    }

    /// Any pattern of reserved bits decodes to the same value as the cleared word, and the
    /// writer gives the cleared word (2.1.3, REQ-UMP-003, REQ-UMP-004).
    #[test]
    fn prop_utility_reader_ignores_reserved_bits() {
        proptest!(|(noop in utility_word(0x0), clock in utility_word(0x1), ts in utility_word(0x2), tpq in utility_word(0x3), dc in utility_word(0x4))| {
                let n = Noop::from_words(&[noop]).unwrap();
                prop_assert_eq!(word(&n), noop & !Noop::RESERVED);
                let c = JrClock::from_words(&[clock]).unwrap();
                prop_assert_eq!(c, JrClock::from_words(&[clock & !JrClock::RESERVED]).unwrap());
                prop_assert_eq!(word(&c), clock & !JrClock::RESERVED);
                let t = JrTimestamp::from_words(&[ts]).unwrap();
                prop_assert_eq!(word(&t), ts & !JrTimestamp::RESERVED);
                let q = DeltaClockstampTpq::from_words(&[tpq]).unwrap();
                prop_assert_eq!(word(&q), tpq & !DeltaClockstampTpq::RESERVED);
                let d = DeltaClockstamp::from_words(&[dc]).unwrap();
                prop_assert_eq!(word(&d), dc & !DeltaClockstamp::RESERVED);
        });
    }
}
