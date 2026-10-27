//! UMP packet layer (M2-104-UM 2.1.1 to 2.1.4).
//!
//! - The Message Type (MT) is the most significant 4 bits of word 0 and fixes the packet size
//!   (Table 4), reserved values included.
//! - Packets are exchanged as 32-bit words, word 0 first. Inside each word the most significant
//!   bit comes first, as in the specification diagrams (2.1.1).
//! - Each message type stores its packet in one integer (`u32`, `u64` or `u128`); word 0 is the
//!   most significant 32 bits (DEC-UMP-001).

use thiserror::Error;

/// UMP Message Type, bits 31..28 of word 0 (M2-104-UM 2.1.4, Table 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MessageType {
    /// 0x0, 32 bits: Utility Messages (Groupless).
    Utility = 0x0,
    /// 0x1, 32 bits: System Real Time and System Common Messages (except System Exclusive).
    System = 0x1,
    /// 0x2, 32 bits: MIDI 1.0 Channel Voice Messages.
    Midi1ChannelVoice = 0x2,
    /// 0x3, 64 bits: Data Messages (including System Exclusive).
    Data64 = 0x3,
    /// 0x4, 64 bits: MIDI 2.0 Channel Voice Messages.
    Midi2ChannelVoice = 0x4,
    /// 0x5, 128 bits: Data Messages.
    Data128 = 0x5,
    /// 0x6, 32 bits: Reserved.
    Reserved6 = 0x6,
    /// 0x7, 32 bits: Reserved.
    Reserved7 = 0x7,
    /// 0x8, 64 bits: Reserved.
    Reserved8 = 0x8,
    /// 0x9, 64 bits: Reserved.
    Reserved9 = 0x9,
    /// 0xA, 64 bits: Reserved.
    ReservedA = 0xA,
    /// 0xB, 96 bits: Reserved.
    ReservedB = 0xB,
    /// 0xC, 96 bits: Reserved.
    ReservedC = 0xC,
    /// 0xD, 128 bits: Flex Data Messages.
    FlexData = 0xD,
    /// 0xE, 128 bits: Reserved.
    ReservedE = 0xE,
    /// 0xF, 128 bits: UMP Stream Messages (Groupless).
    UmpStream = 0xF,
}

impl MessageType {
    /// The 4-bit value of this Message Type.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Message Type for the low 4 bits of `value`. Every 4-bit value has a variant.
    pub const fn from_bits(value: u8) -> Self {
        match value & 0xF {
            0x0 => Self::Utility,
            0x1 => Self::System,
            0x2 => Self::Midi1ChannelVoice,
            0x3 => Self::Data64,
            0x4 => Self::Midi2ChannelVoice,
            0x5 => Self::Data128,
            0x6 => Self::Reserved6,
            0x7 => Self::Reserved7,
            0x8 => Self::Reserved8,
            0x9 => Self::Reserved9,
            0xA => Self::ReservedA,
            0xB => Self::ReservedB,
            0xC => Self::ReservedC,
            0xD => Self::FlexData,
            0xE => Self::ReservedE,
            _ => Self::UmpStream,
        }
    }

    /// True when [`crate::Ump::parse`] decodes this Message Type in this build: its Cargo
    /// feature is on (DEC-017). False for a reserved Message Type and for a family whose feature
    /// is off; [`crate::Ump::parse`] gives both as [`crate::Ump::Unknown`].
    // Each arm depends on a Cargo feature, so a `matches!` is right only for one feature set.
    #[allow(clippy::match_like_matches_macro)]
    pub const fn is_decoded(self) -> bool {
        match self {
            Self::Utility => cfg!(feature = "utility"),
            Self::System => cfg!(feature = "system-common"),
            Self::Midi1ChannelVoice => cfg!(feature = "channel-voice1"),
            Self::Midi2ChannelVoice => cfg!(feature = "channel-voice2"),
            Self::Data64 => cfg!(feature = "sysex7"),
            Self::Data128 => cfg!(feature = "data128"),
            Self::FlexData => cfg!(feature = "flex-data"),
            Self::UmpStream => cfg!(feature = "ump-stream"),
            _ => false,
        }
    }

    /// Packet size in 32-bit words (Table 4).
    pub const fn word_count(self) -> usize {
        match self {
            Self::Utility
            | Self::System
            | Self::Midi1ChannelVoice
            | Self::Reserved6
            | Self::Reserved7 => 1,
            Self::Data64
            | Self::Midi2ChannelVoice
            | Self::Reserved8
            | Self::Reserved9
            | Self::ReservedA => 2,
            Self::ReservedB | Self::ReservedC => 3,
            Self::Data128 | Self::FlexData | Self::ReservedE | Self::UmpStream => 4,
        }
    }
}

/// UMP Group: bits 27..24 of word 0 in every message that has a Group (M2-104-UM 2.1.2, DEC-UMP-004).
///
/// Stores the packet value 0 to 15. [`Group::number`] gives the user-facing number 1 to 16. A
/// `Group` is a distinct type, so it cannot be used where a [`Channel`] is expected:
///
/// ```compile_fail
/// fn takes_group(_: zmidi_ump::Group) {}
/// takes_group(zmidi_ump::Channel::new(0).unwrap());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Group(u8);

impl Group {
    /// The Group with packet value `index` (0 to 15). Gives `OutOfRange` for other values.
    pub const fn new(index: u8) -> Result<Self, UmpError> {
        if index > 0xF {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(index))
    }

    /// The Group with user-facing number `number` (1 to 16). Gives `OutOfRange` for other values.
    pub const fn from_number(number: u8) -> Result<Self, UmpError> {
        if number == 0 || number > 16 {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(number - 1))
    }

    /// The packet value, 0 to 15.
    pub const fn index(self) -> u8 {
        self.0
    }

    /// The user-facing number, 1 to 16.
    pub const fn number(self) -> u8 {
        self.0 + 1
    }

    /// The 4-bit field value.
    pub const fn into_bits(self) -> u8 {
        self.0
    }

    /// The Group for the low 4 bits of `value`. Every 4-bit value is valid.
    pub const fn from_bits(value: u8) -> Self {
        Self(value & 0xF)
    }
}

/// MIDI Channel: the 4-bit Channel field of a channel-addressed message (M2-104-UM 2.1.2, DEC-UMP-004).
///
/// Stores the packet value 0 to 15. [`Channel::number`] gives the user-facing number 1 to 16. A
/// `Channel` is a distinct type, so it cannot be used where a [`Group`] is expected:
///
/// ```compile_fail
/// fn takes_channel(_: zmidi_ump::Channel) {}
/// takes_channel(zmidi_ump::Group::new(0).unwrap());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Channel(u8);

impl Channel {
    /// The Channel with packet value `index` (0 to 15). Gives `OutOfRange` for other values.
    pub const fn new(index: u8) -> Result<Self, UmpError> {
        if index > 0xF {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(index))
    }

    /// The Channel with user-facing number `number` (1 to 16). Gives `OutOfRange` for other values.
    pub const fn from_number(number: u8) -> Result<Self, UmpError> {
        if number == 0 || number > 16 {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(number - 1))
    }

    /// The packet value, 0 to 15.
    pub const fn index(self) -> u8 {
        self.0
    }

    /// The user-facing number, 1 to 16.
    pub const fn number(self) -> u8 {
        self.0 + 1
    }

    /// The 4-bit field value.
    pub const fn into_bits(self) -> u8 {
        self.0
    }

    /// The Channel for the low 4 bits of `value`. Every 4-bit value is valid.
    pub const fn from_bits(value: u8) -> Self {
        Self(value & 0xF)
    }
}

/// Errors of this crate. No variant allocates (DEC-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UmpError {
    /// The input holds fewer words than the packet needs.
    #[error("input holds {got} words, the packet needs {needed}")]
    Truncated { needed: usize, got: usize },
    /// The caller buffer holds fewer words than the packet needs.
    #[error("buffer holds {got} words, the packet needs {needed}")]
    BufferTooSmall { needed: usize, got: usize },
    /// A constructor value is outside the range the specification allows.
    #[error("value is outside the range the specification allows")]
    OutOfRange,
    /// The words hold a different Message Type or Status than the type being decoded.
    #[error("the words hold a different message")]
    UnexpectedMessage,
}

/// Conversion between a message and its 32-bit words, word 0 first (DEC-UMP-001, DEC-UMP-002).
pub trait UmpWords: Sized {
    /// Number of 32-bit words in the packet.
    const WORDS: usize;

    /// Decode the message from the start of `words`. Checks Message Type and Status and clears
    /// reserved bits. Never panics.
    fn from_words(words: &[u32]) -> Result<Self, UmpError>;

    /// Write one complete packet of `WORDS` words into the start of `out` (2.1). Returns the
    /// number of words written. Never panics.
    fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError>;
}

/// Word 0 and word 1 of a 64-bit packet; word 0 is the most significant (2.1.1, DEC-UMP-001).
pub(crate) const fn split_u64(packet: u64) -> [u32; 2] {
    [(packet >> 32) as u32, packet as u32]
}

/// A 64-bit packet from word 0 and word 1; word 0 is the most significant.
pub(crate) const fn join_u64(words: [u32; 2]) -> u64 {
    ((words[0] as u64) << 32) | words[1] as u64
}

/// Words 0 to 3 of a 128-bit packet; word 0 is the most significant.
pub(crate) const fn split_u128(packet: u128) -> [u32; 4] {
    [
        (packet >> 96) as u32,
        (packet >> 64) as u32,
        (packet >> 32) as u32,
        packet as u32,
    ]
}

/// A 128-bit packet from words 0 to 3; word 0 is the most significant.
pub(crate) const fn join_u128(words: [u32; 4]) -> u128 {
    ((words[0] as u128) << 96)
        | ((words[1] as u128) << 64)
        | ((words[2] as u128) << 32)
        | words[3] as u128
}

/// The first `N` words of `words`, or `Truncated`.
pub(crate) fn take<const N: usize>(words: &[u32]) -> Result<[u32; N], UmpError> {
    match words.get(..N) {
        Some(head) => {
            let mut out = [0; N];
            out.copy_from_slice(head);
            Ok(out)
        }
        None => Err(UmpError::Truncated {
            needed: N,
            got: words.len(),
        }),
    }
}

/// Copy `words` into the start of `out`, or `BufferTooSmall`.
pub(crate) fn put(words: &[u32], out: &mut [u32]) -> Result<usize, UmpError> {
    match out.get_mut(..words.len()) {
        Some(head) => {
            head.copy_from_slice(words);
            Ok(words.len())
        }
        None => Err(UmpError::BufferTooSmall {
            needed: words.len(),
            got: out.len(),
        }),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;

    #[test]
    fn message_type_word_count_matches_table_4() {
        // Table 4: sizes in bits for MT 0x0..=0xF.
        let bits = [
            32, 32, 32, 64, 64, 128, 32, 32, 64, 64, 64, 96, 96, 128, 128, 128,
        ];
        for (mt, expected) in bits.iter().enumerate() {
            let t = MessageType::from_bits(mt as u8);
            assert_eq!(t.into_bits(), mt as u8);
            assert_eq!(t.word_count() * 32, *expected, "MT {mt:#x}");
        }
    }

    #[test]
    fn message_type_from_bits_ignores_high_bits() {
        assert_eq!(MessageType::from_bits(0xFD), MessageType::FlexData);
    }

    #[test]
    fn storage_integer_word_order_is_msb_first() {
        // 2.1.1: word 0 is sent first and holds the most significant bits of the packet.
        assert_eq!(split_u64(0x4090_3C00_8000_0000), [0x4090_3C00, 0x8000_0000]);
        assert_eq!(join_u64([0x4090_3C00, 0x8000_0000]), 0x4090_3C00_8000_0000);
        let p = 0xD050_0105_0000_0001_0000_0002_0000_0003u128;
        assert_eq!(split_u128(p), [0xD050_0105, 1, 2, 3]);
        assert_eq!(join_u128(split_u128(p)), p);
    }

    #[test]
    fn take_reports_truncated_input() {
        assert_eq!(take::<2>(&[1, 2, 3]), Ok([1, 2]));
        assert_eq!(
            take::<4>(&[1]),
            Err(UmpError::Truncated { needed: 4, got: 1 })
        );
        assert_eq!(
            take::<1>(&[]),
            Err(UmpError::Truncated { needed: 1, got: 0 })
        );
    }

    #[test]
    fn put_reports_small_buffer() {
        let mut out = [0u32; 2];
        assert_eq!(put(&[7, 8], &mut out), Ok(2));
        assert_eq!(out, [7, 8]);
        assert_eq!(
            put(&[1, 2, 3], &mut out),
            Err(UmpError::BufferTooSmall { needed: 3, got: 2 })
        );
    }

    #[test]
    fn group_and_channel_ranges() {
        // 2.1.2: 4-bit fields, value 0 to 15, shown to users as 1 to 16 (REQ-UMP-008).
        for i in 0..=15u8 {
            let g = Group::new(i).unwrap();
            assert_eq!((g.index(), g.number(), g.into_bits()), (i, i + 1, i));
            assert_eq!(Group::from_number(i + 1), Ok(g));
            assert_eq!(Group::from_bits(i), g);
            let c = Channel::new(i).unwrap();
            assert_eq!((c.index(), c.number(), c.into_bits()), (i, i + 1, i));
            assert_eq!(Channel::from_number(i + 1), Ok(c));
        }
        assert_eq!(Group::new(16), Err(UmpError::OutOfRange));
        assert_eq!(Channel::new(16), Err(UmpError::OutOfRange));
        for n in [0, 17, u8::MAX] {
            assert_eq!(Group::from_number(n), Err(UmpError::OutOfRange));
            assert_eq!(Channel::from_number(n), Err(UmpError::OutOfRange));
        }
        assert_eq!(Group::from_bits(0xF3), Group::new(3).unwrap());
        assert_eq!(Channel::from_bits(0xA5), Channel::new(5).unwrap());
    }

    #[test]
    fn prop_group_and_channel_constructors_accept_spec_range() {
        // DEC-UMP-003, DEC-UMP-004.
        proptest!(|(v in any::<u8>())| {
            prop_assert_eq!(Group::new(v).is_ok(), v <= 15);
            prop_assert_eq!(Channel::new(v).is_ok(), v <= 15);
            prop_assert_eq!(Group::from_number(v).is_ok(), (1..=16).contains(&v));
            if let Ok(c) = Channel::from_number(v) {
                prop_assert_eq!(c.number(), v);
                prop_assert_eq!(c.index() + 1, v);
            }
            prop_assert_eq!(Group::from_bits(v).index(), v & 0xF);
        });
    }
}
