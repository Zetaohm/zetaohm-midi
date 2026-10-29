//! MIDI 1.0 Channel Voice Messages: Message Type 0x2 (M2-104-UM 7.3, Figures 36 to 44,
//! Appendix F Table 28).
//!
//! - Format: one 32-bit word. `mt=2` (bits 31..28), Group (27..24), opcode (23..20), Channel
//!   (19..16), then MIDI 1.0 bytes 2 and 3 (15..0).
//! - 3-byte MIDI 1.0 messages fill bytes 2 to 4; 2-byte messages (Program Change, Channel
//!   Pressure) fill bytes 2 and 3, and byte 4 is zero (Figures 36 and 37).
//! - The most significant bit of each data byte is reserved (`r` in Table 28): writers set it to
//!   zero, readers ignore it (2.1.3).
//! - A Note On with Velocity 0 stays a Note On here; translation rules are in another module.
//! - Message Type and opcode are read-only. Structs have no public `from_bits` (DEC-UMP-002).
//!   Group and Channel use the [`Group`] and [`Channel`] types (DEC-UMP-004).

use bitfield_struct::bitfield;

use crate::packet::{put, take, Channel, Group, MessageType, UmpError, UmpWords};

/// Opcode of a MIDI 1.0 Channel Voice message, bits 23..20 (Table 28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Midi1Opcode {
    /// 0x8: Note Off.
    NoteOff = 0x8,
    /// 0x9: Note On.
    NoteOn = 0x9,
    /// 0xA: Poly Pressure.
    PolyPressure = 0xA,
    /// 0xB: Control Change.
    ControlChange = 0xB,
    /// 0xC: Program Change.
    ProgramChange = 0xC,
    /// 0xD: Channel Pressure.
    ChannelPressure = 0xD,
    /// 0xE: Pitch Bend.
    PitchBend = 0xE,
}

impl Midi1Opcode {
    /// The 4-bit opcode.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The opcode for the low 4 bits of `value`, or `None` for an opcode Table 28 does not
    /// define.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value & 0xF {
            0x8 => Some(Self::NoteOff),
            0x9 => Some(Self::NoteOn),
            0xA => Some(Self::PolyPressure),
            0xB => Some(Self::ControlChange),
            0xC => Some(Self::ProgramChange),
            0xD => Some(Self::ChannelPressure),
            0xE => Some(Self::PitchBend),
            _ => None,
        }
    }
}

/// Channel Mode controller numbers: Control Change Index 120 to 127 (MIDI 1.0 Specification;
/// REQ-CV-MODE-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ChannelModeController {
    /// 120: All Sound Off.
    AllSoundOff = 120,
    /// 121: Reset All Controllers.
    ResetAllControllers = 121,
    /// 122: Local Control.
    LocalControl = 122,
    /// 123: All Notes Off.
    AllNotesOff = 123,
    /// 124: Omni Off.
    OmniOff = 124,
    /// 125: Omni On.
    OmniOn = 125,
    /// 126: Mono On.
    MonoOn = 126,
    /// 127: Poly On.
    PolyOn = 127,
}

impl ChannelModeController {
    /// The Channel Mode controller for `index`, or `None` for an index outside 120 to 127.
    pub const fn from_index(index: u8) -> Option<Self> {
        match index {
            120 => Some(Self::AllSoundOff),
            121 => Some(Self::ResetAllControllers),
            122 => Some(Self::LocalControl),
            123 => Some(Self::AllNotesOff),
            124 => Some(Self::OmniOff),
            125 => Some(Self::OmniOn),
            126 => Some(Self::MonoOn),
            127 => Some(Self::PolyOn),
            _ => None,
        }
    }

    /// The controller number, 120 to 127.
    pub const fn index(self) -> u8 {
        self as u8
    }
}

/// Word 0 with Message Type 0x2, `opcode`, `group` and `channel`.
const fn header(opcode: Midi1Opcode, group: Group, channel: Channel) -> u32 {
    ((MessageType::Midi1ChannelVoice.into_bits() as u32) << 28)
        | ((group.into_bits() as u32) << 24)
        | ((opcode.into_bits() as u32) << 20)
        | ((channel.into_bits() as u32) << 16)
}

/// Reads word 0 of a MIDI 1.0 Channel Voice packet with `opcode`, or gives an error. Clears the
/// bits in `reserved` (2.1.3).
fn decode(words: &[u32], opcode: Midi1Opcode, reserved: u32) -> Result<u32, UmpError> {
    let [w0] = take::<1>(words)?;
    let mt = MessageType::Midi1ChannelVoice.into_bits() as u32;
    if w0 >> 28 != mt || (w0 >> 20) & 0xF != opcode.into_bits() as u32 {
        return Err(UmpError::UnexpectedMessage);
    }
    Ok(w0 & !reserved)
}

/// MIDI 1.0 Note Off, opcode 0x8 (7.3.1, Figure 38, Table 28).
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi1NoteOff {
    /// Message Type, 4 bits: 0x2.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0x8.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Note Number (`kkkkkkk`), 7 bits.
    #[bits(7, access = RO)]
    pub note: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Velocity (`vvvvvvv`), 7 bits.
    #[bits(7, access = RO)]
    pub velocity: u8,
}

impl Midi1NoteOff {
    const RESERVED: u32 = 0x0000_8080;

    /// A Note Off message. Gives `OutOfRange` for a note or velocity above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        note: u8,
        velocity: u8,
    ) -> Result<Self, UmpError> {
        if note > 0x7F || velocity > 0x7F {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(Midi1Opcode::NoteOff, group, channel) | ((note as u32) << 8) | velocity as u32,
        ))
    }
}

/// MIDI 1.0 Note On, opcode 0x9 (7.3.2, Figure 39, Table 28).
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi1NoteOn {
    /// Message Type, 4 bits: 0x2.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0x9.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Note Number (`kkkkkkk`), 7 bits.
    #[bits(7, access = RO)]
    pub note: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Velocity (`vvvvvvv`); 0 stays a Note On, 7 bits.
    #[bits(7, access = RO)]
    pub velocity: u8,
}

impl Midi1NoteOn {
    const RESERVED: u32 = 0x0000_8080;

    /// A Note On message. Gives `OutOfRange` for a note or velocity above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        note: u8,
        velocity: u8,
    ) -> Result<Self, UmpError> {
        if note > 0x7F || velocity > 0x7F {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(Midi1Opcode::NoteOn, group, channel) | ((note as u32) << 8) | velocity as u32,
        ))
    }
}

/// MIDI 1.0 Poly Pressure, opcode 0xA (7.3.3, Figure 40, Table 28).
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi1PolyPressure {
    /// Message Type, 4 bits: 0x2.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0xA.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Note Number (`kkkkkkk`), 7 bits.
    #[bits(7, access = RO)]
    pub note: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Pressure (`ddddddd`), 7 bits.
    #[bits(7, access = RO)]
    pub pressure: u8,
}

impl Midi1PolyPressure {
    const RESERVED: u32 = 0x0000_8080;

    /// A Poly Pressure message. Gives `OutOfRange` for a note or pressure above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        note: u8,
        pressure: u8,
    ) -> Result<Self, UmpError> {
        if note > 0x7F || pressure > 0x7F {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(Midi1Opcode::PolyPressure, group, channel)
                | ((note as u32) << 8)
                | pressure as u32,
        ))
    }
}

/// MIDI 1.0 Control Change, opcode 0xB (7.3.4, Figure 41, Table 28).
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi1ControlChange {
    /// Message Type, 4 bits: 0x2.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0xB.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Index (`ccccccc`), 7 bits.
    #[bits(7, access = RO)]
    pub index: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Data (`ddddddd`), 7 bits.
    #[bits(7, access = RO)]
    pub data: u8,
}

impl Midi1ControlChange {
    const RESERVED: u32 = 0x0000_8080;

    /// A Control Change message. Gives `OutOfRange` for a index or data above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        index: u8,
        data: u8,
    ) -> Result<Self, UmpError> {
        if index > 0x7F || data > 0x7F {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(Midi1Opcode::ControlChange, group, channel)
                | ((index as u32) << 8)
                | data as u32,
        ))
    }
}

/// MIDI 1.0 Program Change, opcode 0xC (7.3.5, Figure 42, Table 28).
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi1ProgramChange {
    /// Message Type, 4 bits: 0x2.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0xC.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Program (`ppppppp`), 7 bits.
    #[bits(7, access = RO)]
    pub program: u8,
    /// Reserved (byte 4 of a 2-byte message), 8 bits.
    #[bits(8)]
    __: u8,
}

impl Midi1ProgramChange {
    const RESERVED: u32 = 0x0000_80FF;

    /// A Program Change message. Gives `OutOfRange` for a program above 127.
    pub const fn try_new(group: Group, channel: Channel, program: u8) -> Result<Self, UmpError> {
        if program > 0x7F {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(Midi1Opcode::ProgramChange, group, channel) | ((program as u32) << 8),
        ))
    }
}

/// MIDI 1.0 Channel Pressure, opcode 0xD (7.3.6, Figure 43, Table 28).
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi1ChannelPressure {
    /// Message Type, 4 bits: 0x2.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0xD.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Pressure (`ddddddd`), 7 bits.
    #[bits(7, access = RO)]
    pub pressure: u8,
    /// Reserved (byte 4 of a 2-byte message), 8 bits.
    #[bits(8)]
    __: u8,
}

impl Midi1ChannelPressure {
    const RESERVED: u32 = 0x0000_80FF;

    /// A Channel Pressure message. Gives `OutOfRange` for a pressure above 127.
    pub const fn try_new(group: Group, channel: Channel, pressure: u8) -> Result<Self, UmpError> {
        if pressure > 0x7F {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(Midi1Opcode::ChannelPressure, group, channel) | ((pressure as u32) << 8),
        ))
    }
}

/// MIDI 1.0 Pitch Bend, opcode 0xE (7.3.7, Figure 44, Table 28).
#[bitfield(u32, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi1PitchBend {
    /// Message Type, 4 bits: 0x2.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0xE.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Value LSB 7 bits (`ddddddd`), 7 bits.
    #[bits(7, access = RO)]
    pub lsb: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Value MSB 7 bits (`DDDDDDD`), 7 bits.
    #[bits(7, access = RO)]
    pub msb: u8,
}

impl Midi1PitchBend {
    const RESERVED: u32 = 0x0000_8080;

    /// A Pitch Bend message. Gives `OutOfRange` for a value above 16,383 (center 8,192).
    pub const fn try_new(group: Group, channel: Channel, value: u16) -> Result<Self, UmpError> {
        if value > 0x3FFF {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(Midi1Opcode::PitchBend, group, channel)
                | (((value & 0x7F) as u32) << 8)
                | ((value >> 7) as u32),
        ))
    }

    /// The 14-bit value: MSB 7 bits from byte 4, LSB 7 bits from byte 3.
    pub fn value(&self) -> u16 {
        ((self.msb() as u16) << 7) | self.lsb() as u16
    }
}

impl Midi1ControlChange {
    /// The Channel Mode controller when the Index is 120 to 127, otherwise `None`.
    pub const fn channel_mode(&self) -> Option<ChannelModeController> {
        ChannelModeController::from_index(self.index())
    }
}

macro_rules! midi1_words {
    ($($ty:ident => $opcode:ident),* $(,)?) => {$(
        impl UmpWords for $ty {
            const WORDS: usize = 1;

            fn from_words(words: &[u32]) -> Result<Self, UmpError> {
                decode(words, Midi1Opcode::$opcode, Self::RESERVED).map(Self)
            }

            fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
                put(&[self.0], out)
            }
        }
    )*};
}

midi1_words! {
    Midi1NoteOff => NoteOff,
    Midi1NoteOn => NoteOn,
    Midi1PolyPressure => PolyPressure,
    Midi1ControlChange => ControlChange,
    Midi1ProgramChange => ProgramChange,
    Midi1ChannelPressure => ChannelPressure,
    Midi1PitchBend => PitchBend,
}

/// One MIDI 1.0 Channel Voice message (Table 28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Midi1ChannelVoiceMessage {
    /// Note Off (0x8).
    NoteOff(Midi1NoteOff),
    /// Note On (0x9).
    NoteOn(Midi1NoteOn),
    /// Poly Pressure (0xA).
    PolyPressure(Midi1PolyPressure),
    /// Control Change (0xB).
    ControlChange(Midi1ControlChange),
    /// Program Change (0xC).
    ProgramChange(Midi1ProgramChange),
    /// Channel Pressure (0xD).
    ChannelPressure(Midi1ChannelPressure),
    /// Pitch Bend (0xE).
    PitchBend(Midi1PitchBend),
}

impl Midi1ChannelVoiceMessage {
    /// Decodes one MIDI 1.0 Channel Voice packet by its opcode. Gives `None` for short input, a
    /// Message Type other than 0x2, or an opcode Table 28 does not define. Never panics.
    pub fn parse(words: &[u32]) -> Option<Self> {
        let opcode = Midi1Opcode::from_bits((*words.first()? >> 20) as u8)?;
        match opcode {
            Midi1Opcode::NoteOff => Midi1NoteOff::from_words(words).ok().map(Self::NoteOff),
            Midi1Opcode::NoteOn => Midi1NoteOn::from_words(words).ok().map(Self::NoteOn),
            Midi1Opcode::PolyPressure => Midi1PolyPressure::from_words(words)
                .ok()
                .map(Self::PolyPressure),
            Midi1Opcode::ControlChange => Midi1ControlChange::from_words(words)
                .ok()
                .map(Self::ControlChange),
            Midi1Opcode::ProgramChange => Midi1ProgramChange::from_words(words)
                .ok()
                .map(Self::ProgramChange),
            Midi1Opcode::ChannelPressure => Midi1ChannelPressure::from_words(words)
                .ok()
                .map(Self::ChannelPressure),
            Midi1Opcode::PitchBend => Midi1PitchBend::from_words(words).ok().map(Self::PitchBend),
        }
    }

    /// The packet as one 32-bit word.
    pub(crate) const fn into_bits(self) -> u32 {
        match self {
            Self::NoteOff(m) => m.0,
            Self::NoteOn(m) => m.0,
            Self::PolyPressure(m) => m.0,
            Self::ControlChange(m) => m.0,
            Self::ProgramChange(m) => m.0,
            Self::ChannelPressure(m) => m.0,
            Self::PitchBend(m) => m.0,
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

    fn c(i: u8) -> Channel {
        Channel::new(i).unwrap()
    }

    #[test]
    fn midi1_opcode_matches_table_28() {
        for op in 0..=0xFu8 {
            match Midi1Opcode::from_bits(op) {
                Some(o) => {
                    assert!((0x8..=0xE).contains(&op));
                    assert_eq!(o.into_bits(), op);
                }
                None => assert!(!(0x8..=0xE).contains(&op)),
            }
        }
    }

    #[test]
    fn midi1_messages_match_table_28() {
        // Figure 36: the 3 MIDI 1.0 bytes are bytes 2 to 4. Note On ch 1 note 60 vel 100 on Group 1.
        let on = Midi1NoteOn::try_new(g(0), c(0), 60, 100).unwrap();
        assert_eq!(word(&on), 0x2090_3C64);
        assert_eq!(
            (
                on.message_type(),
                on.group(),
                on.opcode(),
                on.channel(),
                on.note(),
                on.velocity()
            ),
            (MessageType::Midi1ChannelVoice, g(0), 0x9, c(0), 60, 100)
        );
        assert_eq!(
            word(&Midi1NoteOff::try_new(g(1), c(2), 60, 0).unwrap()),
            0x2182_3C00
        );
        assert_eq!(
            word(&Midi1PolyPressure::try_new(g(0), c(15), 0x7F, 0x40).unwrap()),
            0x20AF_7F40
        );
        assert_eq!(
            word(&Midi1ControlChange::try_new(g(15), c(0), 7, 127).unwrap()),
            0x2FB0_077F
        );
        // Figure 37: 2-byte messages, byte 4 is zero.
        assert_eq!(
            word(&Midi1ProgramChange::try_new(g(0), c(9), 5).unwrap()),
            0x20C9_0500
        );
        assert_eq!(
            word(&Midi1ChannelPressure::try_new(g(0), c(0), 0x7F).unwrap()),
            0x20D0_7F00
        );
        // Pitch Bend: LSB in byte 3, MSB in byte 4; center 0x2000 = LSB 0x00, MSB 0x40.
        let pb = Midi1PitchBend::try_new(g(0), c(0), 0x2000).unwrap();
        assert_eq!(word(&pb), 0x20E0_0040);
        assert_eq!((pb.lsb(), pb.msb(), pb.value()), (0x00, 0x40, 0x2000));
        assert_eq!(
            word(&Midi1PitchBend::try_new(g(0), c(0), 0x3FFF).unwrap()),
            0x20E0_7F7F
        );
    }

    #[test]
    fn midi1_note_on_velocity_zero_stays_note_on() {
        let m = Midi1ChannelVoiceMessage::parse(&[0x2090_3C00]).unwrap();
        assert!(matches!(m, Midi1ChannelVoiceMessage::NoteOn(n) if n.velocity() == 0));
    }

    #[test]
    fn midi1_try_new_rejects_out_of_range() {
        assert_eq!(
            Midi1NoteOn::try_new(g(0), c(0), 128, 0),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            Midi1NoteOff::try_new(g(0), c(0), 0, 128),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            Midi1PolyPressure::try_new(g(0), c(0), 0, 200),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            Midi1ControlChange::try_new(g(0), c(0), 128, 0),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            Midi1ProgramChange::try_new(g(0), c(0), 128),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            Midi1ChannelPressure::try_new(g(0), c(0), 255),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            Midi1PitchBend::try_new(g(0), c(0), 0x4000),
            Err(UmpError::OutOfRange)
        );
    }

    #[test]
    fn midi1_reader_ignores_reserved_bits() {
        // Table 28 `r` bits and byte 4 of 2-byte messages are reserved (2.1.3).
        assert_eq!(
            word(&Midi1NoteOn::from_words(&[0x2090_BCE4]).unwrap()),
            0x2090_3C64
        );
        assert_eq!(
            word(&Midi1ProgramChange::from_words(&[0x20C9_85FF]).unwrap()),
            0x20C9_0500
        );
        assert_eq!(
            word(&Midi1PitchBend::from_words(&[0x20E0_80C0]).unwrap()),
            0x20E0_0040
        );
    }

    #[test]
    fn midi1_readers_give_each_field() {
        // REQ-CV1-002, -004, -005, -006, -007: each field read from a known word, with
        // different values in each field.
        let off = Midi1NoteOff::from_words(&[0x2182_3C05]).unwrap();
        assert_eq!(
            (off.group(), off.channel(), off.note(), off.velocity()),
            (g(1), c(2), 0x3C, 5)
        );
        let pp = Midi1PolyPressure::from_words(&[0x20A3_3D06]).unwrap();
        assert_eq!((pp.channel(), pp.note(), pp.pressure()), (c(3), 0x3D, 6));
        let cc = Midi1ControlChange::from_words(&[0x20B4_0708]).unwrap();
        assert_eq!((cc.channel(), cc.index(), cc.data()), (c(4), 7, 8));
        let pc = Midi1ProgramChange::from_words(&[0x20C5_0900]).unwrap();
        assert_eq!((pc.channel(), pc.program()), (c(5), 9));
        let cp = Midi1ChannelPressure::from_words(&[0x20D6_0A00]).unwrap();
        assert_eq!((cp.channel(), cp.pressure()), (c(6), 0x0A));
    }

    #[test]
    fn channel_mode_controllers() {
        // REQ-CV-MODE-001: 120 to 127.
        for i in 0..=127u8 {
            let cc = Midi1ControlChange::try_new(g(0), c(0), i, 0).unwrap();
            match cc.channel_mode() {
                Some(m) => {
                    assert!(i >= 120);
                    assert_eq!(m.index(), i);
                }
                None => assert!(i < 120),
            }
        }
        use ChannelModeController::*;
        for (i, name) in [
            (120, AllSoundOff),
            (121, ResetAllControllers),
            (122, LocalControl),
            (123, AllNotesOff),
            (124, OmniOff),
            (125, OmniOn),
            (126, MonoOn),
            (127, PolyOn),
        ] {
            assert_eq!(ChannelModeController::from_index(i), Some(name));
        }
        assert_eq!(ChannelModeController::from_index(128), None);
    }

    #[test]
    fn midi1_message_parse_dispatches_on_opcode() {
        let cases: [(u32, u8); 7] = [
            (0x2080_3C40, 0x8),
            (0x2090_3C40, 0x9),
            (0x20A0_3C40, 0xA),
            (0x20B0_0740, 0xB),
            (0x20C0_0500, 0xC),
            (0x20D0_4000, 0xD),
            (0x20E0_0040, 0xE),
        ];
        for (w, op) in cases {
            let m = Midi1ChannelVoiceMessage::parse(&[w]).unwrap();
            assert_eq!(m.into_bits(), w);
            assert_eq!((m.into_bits() >> 20) as u8 & 0xF, op);
        }
        for op in [0x0u32, 0x7, 0xF] {
            assert_eq!(
                Midi1ChannelVoiceMessage::parse(&[0x2000_0000 | (op << 20)]),
                None
            );
        }
        assert_eq!(Midi1ChannelVoiceMessage::parse(&[]), None);
        assert_eq!(
            Midi1NoteOn::from_words(&[0x2080_3C40]),
            Err(UmpError::UnexpectedMessage)
        );
    }

    #[test]
    fn prop_midi1_round_trip() {
        // DEC-UMP-003: every in-range value round-trips; out-of-range values are rejected.
        proptest!(|(gi in 0u8..16, ci in 0u8..16, a in any::<u8>(), b in any::<u8>(), v in any::<u16>())| {
            match Midi1NoteOn::try_new(g(gi), c(ci), a, b) {
                Ok(m) => {
                    prop_assert!(a <= 127 && b <= 127);
                    prop_assert_eq!(Midi1NoteOn::from_words(&[word(&m)]), Ok(m));
                    prop_assert_eq!((m.group(), m.channel(), m.note(), m.velocity()), (g(gi), c(ci), a, b));
                }
                Err(_) => prop_assert!(a > 127 || b > 127),
            }
            if let Ok(m) = Midi1ControlChange::try_new(g(gi), c(ci), a, b) {
                prop_assert_eq!(Midi1ControlChange::from_words(&[word(&m)]), Ok(m));
            }
            match Midi1PitchBend::try_new(g(gi), c(ci), v) {
                Ok(m) => {
                    prop_assert!(v <= 0x3FFF);
                    prop_assert_eq!(m.value(), v);
                    prop_assert_eq!(Midi1PitchBend::from_words(&[word(&m)]), Ok(m));
                }
                Err(_) => prop_assert!(v > 0x3FFF),
            }
        });
    }

    #[test]
    fn prop_midi1_reader_ignores_reserved_bits() {
        // 2.1.3: any pattern of reserved bits decodes to the cleared word, for every opcode.
        proptest!(|(w in any::<u32>(), op in 0x8u32..=0xE)| {
            let w = 0x2000_0000 | (w & 0x0F0F_FFFF) | (op << 20);
            let m = Midi1ChannelVoiceMessage::parse(&[w]).unwrap();
            let reserved = if op == 0xC || op == 0xD { 0x0000_80FF } else { 0x0000_8080 };
            prop_assert_eq!(m.into_bits(), w & !reserved);
        });
    }
}
