//! MIDI 2.0 Channel Voice Messages: Message Type 0x4 (M2-104-UM 7.4, Figures 45 to 68,
//! Table 8, Appendix A, Appendix F Table 30).
//!
//! - Format: two 32-bit words, stored in a `u64` with word 0 as the most significant 32 bits
//!   (DEC-UMP-001). Word 0: `mt=4` (bits 31..28), Group (27..24), opcode (23..20), Channel
//!   (19..16), a 16-bit index field (15..0). Word 1: a 32-bit data field.
//! - Every `r` bit and reserved field is written as zero and ignored on read (2.1.3).
//! - Opcode 0x7 is not defined; [`Midi2ChannelVoiceMessage::parse`] gives `None` for it.
//! - A Note On with Velocity 0 is a Note On, not a Note Off (7.4.2).
//! - Attribute Type 0x00: the writer sets Attribute Data to 0x0000 and the reader ignores it
//!   (Table 8); `from_words` clears it, so equal messages compare equal.
//! - Message Type and opcode are read-only. Structs have no public `from_bits` (DEC-UMP-002).
//!   Group and Channel use the [`Group`] and [`Channel`] types (DEC-UMP-004).

use bitfield_struct::bitfield;

use crate::packet::{
    join_u64, put, split_u64, take, Channel, Group, MessageType, UmpError, UmpWords,
};

/// Opcode of a MIDI 2.0 Channel Voice message, bits 23..20 (Table 30).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Midi2Opcode {
    /// 0x0: Registered Per-Note Controller.
    RegisteredPerNoteController = 0x0,
    /// 0x1: Assignable Per-Note Controller.
    AssignablePerNoteController = 0x1,
    /// 0x2: Registered Controller (RPN).
    RegisteredController = 0x2,
    /// 0x3: Assignable Controller (NRPN).
    AssignableController = 0x3,
    /// 0x4: Relative Registered Controller.
    RelativeRegisteredController = 0x4,
    /// 0x5: Relative Assignable Controller.
    RelativeAssignableController = 0x5,
    /// 0x6: Per-Note Pitch Bend.
    PerNotePitchBend = 0x6,
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
    /// 0xF: Per-Note Management.
    PerNoteManagement = 0xF,
}

impl Midi2Opcode {
    /// The 4-bit opcode.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The opcode for the low 4 bits of `value`, or `None` for 0x7, which Table 30 does not
    /// define.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value & 0xF {
            0x0 => Some(Self::RegisteredPerNoteController),
            0x1 => Some(Self::AssignablePerNoteController),
            0x2 => Some(Self::RegisteredController),
            0x3 => Some(Self::AssignableController),
            0x4 => Some(Self::RelativeRegisteredController),
            0x5 => Some(Self::RelativeAssignableController),
            0x6 => Some(Self::PerNotePitchBend),
            0x8 => Some(Self::NoteOff),
            0x9 => Some(Self::NoteOn),
            0xA => Some(Self::PolyPressure),
            0xB => Some(Self::ControlChange),
            0xC => Some(Self::ProgramChange),
            0xD => Some(Self::ChannelPressure),
            0xE => Some(Self::PitchBend),
            0xF => Some(Self::PerNoteManagement),
            _ => None,
        }
    }
}

/// Attribute Type of a MIDI 2.0 Note On or Note Off (Table 8). 0x04 to 0xFF are reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AttributeType {
    /// 0x00: No Attribute Data.
    None = 0x00,
    /// 0x01: Manufacturer Specific.
    ManufacturerSpecific = 0x01,
    /// 0x02: Profile Specific.
    ProfileSpecific = 0x02,
    /// 0x03: Pitch 7.9 (7.4.15.3).
    Pitch7_9 = 0x03,
}

impl AttributeType {
    /// The 8-bit Attribute Type.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Attribute Type for `value`, or `None` for a reserved value (0x04 to 0xFF).
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::None),
            0x01 => Some(Self::ManufacturerSpecific),
            0x02 => Some(Self::ProfileSpecific),
            0x03 => Some(Self::Pitch7_9),
            _ => None,
        }
    }
}

/// Q7.9 unsigned fixed-point pitch in HCUs (7.4.15.3): 7 integer bits (the pitch of that Note
/// Number in 12-tone equal temperament, A=440 at Note Number 69) and 9 fraction bits
/// (1/512 HCU).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Pitch7_9(u16);

impl Pitch7_9 {
    /// The pitch for its 16-bit Attribute Data value.
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// The 16-bit Attribute Data value.
    pub const fn into_bits(self) -> u16 {
        self.0
    }

    /// The 7-bit integer part, in HCUs.
    pub const fn note(self) -> u8 {
        (self.0 >> 9) as u8
    }

    /// The 9-bit fraction of one HCU.
    pub const fn fraction(self) -> u16 {
        self.0 & 0x1FF
    }
}

/// Q7.25 unsigned fixed-point pitch in HCUs: the data of Registered Per-Note Controller #3
/// Pitch 7.25 (7.4.15.2). 7 integer bits, 25 fraction bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Pitch7_25(u32);

impl Pitch7_25 {
    /// The pitch for its 32-bit data value.
    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    /// The 32-bit data value.
    pub const fn into_bits(self) -> u32 {
        self.0
    }

    /// The 7-bit integer part, in HCUs.
    pub const fn note(self) -> u8 {
        (self.0 >> 25) as u8
    }

    /// The 25-bit fraction of one HCU.
    pub const fn fraction(self) -> u32 {
        self.0 & 0x01FF_FFFF
    }
}

/// The Attribute Type and Attribute Data of a Note On or Note Off as one value (Table 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoteAttribute {
    /// 0x00: No Attribute Data. The writer sets the data to 0x0000.
    None,
    /// 0x01: Manufacturer Specific data.
    ManufacturerSpecific(u16),
    /// 0x02: Profile Specific data.
    ProfileSpecific(u16),
    /// 0x03: Pitch 7.9.
    Pitch(Pitch7_9),
    /// A reserved Attribute Type (0x04 to 0xFF) as received. Constructors reject it.
    Reserved {
        /// The received Attribute Type.
        attribute_type: u8,
        /// The received Attribute Data.
        data: u16,
    },
}

impl NoteAttribute {
    /// Attribute Type and Attribute Data to write, or `OutOfRange` for a reserved type.
    const fn to_fields(self) -> Result<(u8, u16), UmpError> {
        match self {
            Self::None => Ok((0x00, 0)),
            Self::ManufacturerSpecific(d) => Ok((0x01, d)),
            Self::ProfileSpecific(d) => Ok((0x02, d)),
            Self::Pitch(p) => Ok((0x03, p.into_bits())),
            Self::Reserved { .. } => Err(UmpError::OutOfRange),
        }
    }

    /// The attribute for a received Attribute Type and Attribute Data.
    const fn from_fields(attribute_type: u8, data: u16) -> Self {
        match attribute_type {
            0x00 => Self::None,
            0x01 => Self::ManufacturerSpecific(data),
            0x02 => Self::ProfileSpecific(data),
            0x03 => Self::Pitch(Pitch7_9::from_bits(data)),
            _ => Self::Reserved {
                attribute_type,
                data,
            },
        }
    }
}

/// Registered Per-Note Controller indexes that Appendix A (Table 22) defines. Other indexes are
/// reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RegisteredPerNoteControllerIndex {
    /// 1: Modulation.
    Modulation = 1,
    /// 2: Breath.
    Breath = 2,
    /// 3: Pitch 7.25 (7.4.15.2).
    Pitch7_25 = 3,
    /// 7: Volume.
    Volume = 7,
    /// 8: Balance.
    Balance = 8,
    /// 10: Pan.
    Pan = 10,
    /// 11: Expression.
    Expression = 11,
    /// 70: Sound Controller 1 (default: Sound Variation).
    SoundController1 = 70,
    /// 71: Sound Controller 2 (default: Timbre/Harmonic Intensity).
    SoundController2 = 71,
    /// 72: Sound Controller 3 (default: Release Time).
    SoundController3 = 72,
    /// 73: Sound Controller 4 (default: Attack Time).
    SoundController4 = 73,
    /// 74: Sound Controller 5 (default: Brightness).
    SoundController5 = 74,
    /// 75: Sound Controller 6 (default: Decay Time).
    SoundController6 = 75,
    /// 76: Sound Controller 7 (default: Vibrato Rate).
    SoundController7 = 76,
    /// 77: Sound Controller 8 (default: Vibrato Depth).
    SoundController8 = 77,
    /// 78: Sound Controller 9 (default: Vibrato Delay).
    SoundController9 = 78,
    /// 79: Sound Controller 10.
    SoundController10 = 79,
    /// 91: Effects 1 Depth (default: Reverb Send Level).
    Effects1Depth = 91,
    /// 92: Effects 2 Depth.
    Effects2Depth = 92,
    /// 93: Effects 3 Depth (default: Chorus Send Level).
    Effects3Depth = 93,
    /// 94: Effects 4 Depth.
    Effects4Depth = 94,
    /// 95: Effects 5 Depth.
    Effects5Depth = 95,
}

impl RegisteredPerNoteControllerIndex {
    /// The controller for `index`, or `None` for an index that Table 22 reserves.
    pub const fn from_index(index: u8) -> Option<Self> {
        Some(match index {
            1 => Self::Modulation,
            2 => Self::Breath,
            3 => Self::Pitch7_25,
            7 => Self::Volume,
            8 => Self::Balance,
            10 => Self::Pan,
            11 => Self::Expression,
            70 => Self::SoundController1,
            71 => Self::SoundController2,
            72 => Self::SoundController3,
            73 => Self::SoundController4,
            74 => Self::SoundController5,
            75 => Self::SoundController6,
            76 => Self::SoundController7,
            77 => Self::SoundController8,
            78 => Self::SoundController9,
            79 => Self::SoundController10,
            91 => Self::Effects1Depth,
            92 => Self::Effects2Depth,
            93 => Self::Effects3Depth,
            94 => Self::Effects4Depth,
            95 => Self::Effects5Depth,
            _ => return None,
        })
    }

    /// The 8-bit controller index.
    pub const fn index(self) -> u8 {
        self as u8
    }
}

/// Data of the Registered Controller for Sensitivity of Per-Note Pitch Bend (Bank 0, Index 7;
/// 7.4.13, Figure 68): a 7.25 fixed-point unsigned interval in units of 100 cents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PerNotePitchBendSensitivity(u32);

impl PerNotePitchBendSensitivity {
    /// Registered Controller bank of this message.
    pub const BANK: u8 = 0;
    /// Registered Controller index of this message.
    pub const INDEX: u8 = 7;

    /// The sensitivity for the 32-bit Data of the Registered Controller.
    pub const fn from_data(data: u32) -> Self {
        Self(data)
    }

    /// The 32-bit Data for the Registered Controller.
    pub const fn into_data(self) -> u32 {
        self.0
    }

    /// The 7-bit integer part: the number of 100-cent units.
    pub const fn semitones(self) -> u8 {
        (self.0 >> 25) as u8
    }

    /// The 25-bit fraction of one 100-cent unit.
    pub const fn fraction(self) -> u32 {
        self.0 & 0x01FF_FFFF
    }
}

/// The defined most significant `bits` bits of a 32-bit controller data value, for data that
/// the specification defines only in its most significant bits (7.4.7.x). The undefined low
/// bits are ignored. `bits` above 32 is treated as 32; 0 gives 0.
pub const fn msb_value(data: u32, bits: u32) -> u32 {
    match bits {
        0 => 0,
        1..=31 => data >> (32 - bits),
        _ => data,
    }
}

/// The 64-bit packet with Message Type 0x4, `opcode`, `group` and `channel`.
const fn header(opcode: Midi2Opcode, group: Group, channel: Channel) -> u64 {
    (((MessageType::Midi2ChannelVoice.into_bits() as u64) << 28)
        | ((group.into_bits() as u64) << 24)
        | ((opcode.into_bits() as u64) << 20)
        | ((channel.into_bits() as u64) << 16))
        << 32
}

/// Gives `OutOfRange` when `value` does not fit in 7 bits.
const fn check7(value: u8) -> Result<u64, UmpError> {
    if value > 0x7F {
        Err(UmpError::OutOfRange)
    } else {
        Ok(value as u64)
    }
}

/// Reads a MIDI 2.0 Channel Voice packet with `opcode`, or gives an error. Clears the bits in
/// `reserved` (2.1.3).
fn decode(words: &[u32], opcode: Midi2Opcode, reserved: u64) -> Result<u64, UmpError> {
    let w = take::<2>(words)?;
    let mt = MessageType::Midi2ChannelVoice.into_bits() as u32;
    if w[0] >> 28 != mt || (w[0] >> 20) & 0xF != opcode.into_bits() as u32 {
        return Err(UmpError::UnexpectedMessage);
    }
    Ok(join_u64(w) & !reserved)
}

/// MIDI 2.0 Note Off, opcode 0x8 (7.4.1, Figure 46, Table 30).
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi2NoteOff {
    /// Message Type, 4 bits: 0x4.
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
    /// Note Number, 7 bits.
    #[bits(7, access = RO)]
    pub note: u8,
    /// Attribute Type, 8 bits (Table 8).
    #[bits(8, access = RO)]
    pub attribute_type: u8,
    /// Velocity, 16 bits: 0x0000 to 0xFFFF.
    pub velocity: u16,
    /// Attribute Data, 16 bits.
    #[bits(16, access = RO)]
    pub attribute_data: u16,
}

/// MIDI 2.0 Note On, opcode 0x9 (7.4.2, Figure 47, Table 30). Velocity 0 is a Note On.
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi2NoteOn {
    /// Message Type, 4 bits: 0x4.
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
    /// Note Number, 7 bits.
    #[bits(7, access = RO)]
    pub note: u8,
    /// Attribute Type, 8 bits (Table 8).
    #[bits(8, access = RO)]
    pub attribute_type: u8,
    /// Velocity, 16 bits: 0x0000 to 0xFFFF.
    pub velocity: u16,
    /// Attribute Data, 16 bits.
    #[bits(16, access = RO)]
    pub attribute_data: u16,
}

/// MIDI 2.0 Poly Pressure, opcode 0xA (7.4.3, Figure 48).
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi2PolyPressure {
    /// Message Type, 4 bits: 0x4.
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
    /// Note Number, 7 bits.
    #[bits(7, access = RO)]
    pub note: u8,
    /// Reserved, 8 bits.
    #[bits(8)]
    __: u8,
    /// Pressure, 32 bits.
    pub pressure: u32,
}

/// MIDI 2.0 Registered Per-Note Controller, opcode 0x0 (7.4.4, Figure 49, Appendix A).
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct RegisteredPerNoteController {
    /// Message Type, 4 bits: 0x4.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0x0.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Note Number, 7 bits.
    #[bits(7, access = RO)]
    pub note: u8,
    /// Controller Index, 8 bits.
    #[bits(8, access = RO)]
    pub controller: u8,
    /// Data, 32 bits.
    pub data: u32,
}

/// MIDI 2.0 Assignable Per-Note Controller, opcode 0x1 (7.4.4, Figure 50).
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct AssignablePerNoteController {
    /// Message Type, 4 bits: 0x4.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0x1.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Note Number, 7 bits.
    #[bits(7, access = RO)]
    pub note: u8,
    /// Controller Index, 8 bits.
    #[bits(8, access = RO)]
    pub controller: u8,
    /// Data, 32 bits.
    pub data: u32,
}

/// MIDI 2.0 Per-Note Management, opcode 0xF (7.4.5, Figure 51). D=0 with S=0 is valid and
/// has no defined function.
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct PerNoteManagement {
    /// Message Type, 4 bits: 0x4.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0xF.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Note Number, 7 bits.
    #[bits(7, access = RO)]
    pub note: u8,
    /// Reserved option flags, 6 bits.
    #[bits(6)]
    __: u8,
    /// D: Detach Per-Note Controllers from previously received Notes, 1 bit.
    pub detach: bool,
    /// S: Reset (Set) Per-Note Controllers to default values, 1 bit.
    pub reset: bool,
    /// Reserved, 32 bits.
    #[bits(32)]
    __: u32,
}

/// MIDI 2.0 Control Change, opcode 0xB (7.4.6, Figure 52).
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi2ControlChange {
    /// Message Type, 4 bits: 0x4.
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
    /// Index, 7 bits.
    #[bits(7, access = RO)]
    pub index: u8,
    /// Reserved, 8 bits.
    #[bits(8)]
    __: u8,
    /// Data, 32 bits.
    pub data: u32,
}

/// MIDI 2.0 Registered Controller (RPN), opcode 0x2 (7.4.7, Figure 55).
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct RegisteredController {
    /// Message Type, 4 bits: 0x4.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0x2.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Bank, 7 bits.
    #[bits(7, access = RO)]
    pub bank: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Index, 7 bits.
    #[bits(7, access = RO)]
    pub index: u8,
    /// Data, 32 bits.
    pub data: u32,
}

/// MIDI 2.0 Assignable Controller (NRPN), opcode 0x3 (7.4.7, Figure 56).
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct AssignableController {
    /// Message Type, 4 bits: 0x4.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0x3.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Bank, 7 bits.
    #[bits(7, access = RO)]
    pub bank: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Index, 7 bits.
    #[bits(7, access = RO)]
    pub index: u8,
    /// Data, 32 bits.
    pub data: u32,
}

/// MIDI 2.0 Relative Registered Controller, opcode 0x4 (7.4.8, Figure 62).
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct RelativeRegisteredController {
    /// Message Type, 4 bits: 0x4.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0x4.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Bank, 7 bits.
    #[bits(7, access = RO)]
    pub bank: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Index, 7 bits.
    #[bits(7, access = RO)]
    pub index: u8,
    /// Data, 32-bit two's complement relative value.
    pub data: i32,
}

/// MIDI 2.0 Relative Assignable Controller, opcode 0x5 (7.4.8, Figure 63).
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct RelativeAssignableController {
    /// Message Type, 4 bits: 0x4.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0x5.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Bank, 7 bits.
    #[bits(7, access = RO)]
    pub bank: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Index, 7 bits.
    #[bits(7, access = RO)]
    pub index: u8,
    /// Data, 32-bit two's complement relative value.
    pub data: i32,
}

/// MIDI 2.0 Program Change, opcode 0xC (7.4.9, Figure 64). When Bank Valid (B) is 0, Bank MSB
/// and Bank LSB are zero; other option flags are reserved and zero.
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct Midi2ProgramChange {
    /// Message Type, 4 bits: 0x4.
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
    /// Reserved, 8 bits.
    #[bits(8)]
    __: u8,
    /// Reserved option flags, 7 bits.
    #[bits(7)]
    __: u8,
    /// B: Bank Valid, 1 bit.
    #[bits(1, access = RO)]
    pub bank_valid: bool,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Program, 7 bits.
    #[bits(7, access = RO)]
    pub program: u8,
    /// Reserved, 8 bits.
    #[bits(8)]
    __: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Bank MSB, 7 bits.
    #[bits(7, access = RO)]
    pub bank_msb: u8,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Bank LSB, 7 bits.
    #[bits(7, access = RO)]
    pub bank_lsb: u8,
}

/// MIDI 2.0 Channel Pressure, opcode 0xD (7.4.10, Figure 65). Every 32-bit value is valid, so
/// `new()` gives a valid message; set Group, Channel and pressure with the setters.
#[bitfield(u64, order = Msb, conversion = false, hash = true)]
#[derive(PartialEq, Eq)]
pub struct Midi2ChannelPressure {
    /// Message Type, 4 bits: 0x4.
    #[bits(4, default = MessageType::Midi2ChannelVoice, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0xD.
    #[bits(4, default = 0xD, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 16 bits.
    #[bits(16)]
    __: u16,
    /// Pressure, 32 bits.
    pub pressure: u32,
}

/// MIDI 2.0 Pitch Bend, opcode 0xE (7.4.11, Figure 66): unsigned bipolar value, center
/// 0x80000000. `new()` gives a valid message with value 0; set the fields with the setters.
#[bitfield(u64, order = Msb, conversion = false, hash = true)]
#[derive(PartialEq, Eq)]
pub struct Midi2PitchBend {
    /// Message Type, 4 bits: 0x4.
    #[bits(4, default = MessageType::Midi2ChannelVoice, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0xE.
    #[bits(4, default = 0xE, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 16 bits.
    #[bits(16)]
    __: u16,
    /// Pitch Bend value, 32 bits, center 0x80000000.
    pub value: u32,
}

/// MIDI 2.0 Per-Note Pitch Bend, opcode 0x6 (7.4.12, Figure 67): unsigned bipolar value, center
/// 0x80000000.
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct PerNotePitchBend {
    /// Message Type, 4 bits: 0x4.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Opcode, 4 bits: 0x6.
    #[bits(4, access = RO)]
    pub opcode: u8,
    /// Channel, 4 bits.
    #[bits(4)]
    pub channel: Channel,
    /// Reserved, 1 bit.
    #[bits(1)]
    __: u8,
    /// Note Number, 7 bits.
    #[bits(7, access = RO)]
    pub note: u8,
    /// Reserved, 8 bits.
    #[bits(8)]
    __: u8,
    /// Pitch Bend value, 32 bits, center 0x80000000.
    pub value: u32,
}

const NOTE_RESERVED: u64 = 0x0000_8000_0000_0000;
const NOTE_BYTE4_RESERVED: u64 = 0x0000_80FF_0000_0000;
const BANK_INDEX_RESERVED: u64 = 0x0000_8080_0000_0000;

impl Midi2NoteOff {
    const RESERVED: u64 = NOTE_RESERVED;

    /// A Note Off. Gives `OutOfRange` for a Note Number above 127 or a reserved attribute.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        note: u8,
        velocity: u16,
        attribute: NoteAttribute,
    ) -> Result<Self, UmpError> {
        match note_fields(note, attribute) {
            Ok(f) => Ok(Self(
                header(Midi2Opcode::NoteOff, group, channel) | f | ((velocity as u64) << 16),
            )),
            Err(e) => Err(e),
        }
    }

    /// The Attribute Type and Attribute Data as one value.
    pub const fn attribute(&self) -> NoteAttribute {
        NoteAttribute::from_fields(self.attribute_type(), self.attribute_data())
    }
}

impl Midi2NoteOn {
    const RESERVED: u64 = NOTE_RESERVED;

    /// A Note On. Gives `OutOfRange` for a Note Number above 127 or a reserved attribute.
    /// Velocity 0 is accepted and stays a Note On.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        note: u8,
        velocity: u16,
        attribute: NoteAttribute,
    ) -> Result<Self, UmpError> {
        match note_fields(note, attribute) {
            Ok(f) => Ok(Self(
                header(Midi2Opcode::NoteOn, group, channel) | f | ((velocity as u64) << 16),
            )),
            Err(e) => Err(e),
        }
    }

    /// The Attribute Type and Attribute Data as one value.
    pub const fn attribute(&self) -> NoteAttribute {
        NoteAttribute::from_fields(self.attribute_type(), self.attribute_data())
    }
}

/// Note Number, Attribute Type and Attribute Data bits of a Note On or Note Off.
const fn note_fields(note: u8, attribute: NoteAttribute) -> Result<u64, UmpError> {
    let note = match check7(note) {
        Ok(n) => n,
        Err(e) => return Err(e),
    };
    match attribute.to_fields() {
        Ok((t, d)) => Ok((note << 40) | ((t as u64) << 32) | d as u64),
        Err(e) => Err(e),
    }
}

/// Clears the Attribute Data of a Note On or Note Off with Attribute Type 0x00, which the
/// reader ignores (Table 8).
const fn normalize_note(bits: u64) -> u64 {
    if (bits >> 32) & 0xFF == 0 {
        bits & !0xFFFF
    } else {
        bits
    }
}

impl Midi2PolyPressure {
    const RESERVED: u64 = NOTE_BYTE4_RESERVED;

    /// A Poly Pressure. Gives `OutOfRange` for a Note Number above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        note: u8,
        pressure: u32,
    ) -> Result<Self, UmpError> {
        match check7(note) {
            Ok(n) => Ok(Self(
                header(Midi2Opcode::PolyPressure, group, channel) | (n << 40) | pressure as u64,
            )),
            Err(e) => Err(e),
        }
    }
}

impl RegisteredPerNoteController {
    const RESERVED: u64 = NOTE_RESERVED;

    /// A Registered Per-Note Controller. Gives `OutOfRange` for a Note Number above 127 or a
    /// controller index that Appendix A reserves.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        note: u8,
        controller: u8,
        data: u32,
    ) -> Result<Self, UmpError> {
        if RegisteredPerNoteControllerIndex::from_index(controller).is_none() {
            return Err(UmpError::OutOfRange);
        }
        match check7(note) {
            Ok(n) => Ok(Self(
                header(Midi2Opcode::RegisteredPerNoteController, group, channel)
                    | (n << 40)
                    | ((controller as u64) << 32)
                    | data as u64,
            )),
            Err(e) => Err(e),
        }
    }
}

impl AssignablePerNoteController {
    const RESERVED: u64 = NOTE_RESERVED;

    /// An Assignable Per-Note Controller. Gives `OutOfRange` for a Note Number above 127; every
    /// controller index 0 to 255 is valid.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        note: u8,
        controller: u8,
        data: u32,
    ) -> Result<Self, UmpError> {
        match check7(note) {
            Ok(n) => Ok(Self(
                header(Midi2Opcode::AssignablePerNoteController, group, channel)
                    | (n << 40)
                    | ((controller as u64) << 32)
                    | data as u64,
            )),
            Err(e) => Err(e),
        }
    }
}

impl PerNoteManagement {
    const RESERVED: u64 = 0x0000_80FC_FFFF_FFFF;

    /// A Per-Note Management. Gives `OutOfRange` for a Note Number above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        note: u8,
        detach: bool,
        reset: bool,
    ) -> Result<Self, UmpError> {
        match check7(note) {
            Ok(n) => Ok(Self(
                header(Midi2Opcode::PerNoteManagement, group, channel)
                    | (n << 40)
                    | ((detach as u64) << 33)
                    | ((reset as u64) << 32),
            )),
            Err(e) => Err(e),
        }
    }
}

impl Midi2ControlChange {
    const RESERVED: u64 = NOTE_BYTE4_RESERVED;

    /// A Control Change. Gives `OutOfRange` for an Index above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        index: u8,
        data: u32,
    ) -> Result<Self, UmpError> {
        match check7(index) {
            Ok(i) => Ok(Self(
                header(Midi2Opcode::ControlChange, group, channel) | (i << 40) | data as u64,
            )),
            Err(e) => Err(e),
        }
    }
}

/// Bank and Index bits of a Registered, Assignable or Relative Controller.
const fn bank_index(bank: u8, index: u8) -> Result<u64, UmpError> {
    if bank > 0x7F || index > 0x7F {
        return Err(UmpError::OutOfRange);
    }
    Ok(((bank as u64) << 40) | ((index as u64) << 32))
}

impl RegisteredController {
    const RESERVED: u64 = BANK_INDEX_RESERVED;

    /// A Registered Controller (RPN). Gives `OutOfRange` for a Bank or Index above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        bank: u8,
        index: u8,
        data: u32,
    ) -> Result<Self, UmpError> {
        match bank_index(bank, index) {
            Ok(b) => Ok(Self(
                header(Midi2Opcode::RegisteredController, group, channel) | b | data as u64,
            )),
            Err(e) => Err(e),
        }
    }
}

impl AssignableController {
    const RESERVED: u64 = BANK_INDEX_RESERVED;

    /// An Assignable Controller (NRPN). Gives `OutOfRange` for a Bank or Index above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        bank: u8,
        index: u8,
        data: u32,
    ) -> Result<Self, UmpError> {
        match bank_index(bank, index) {
            Ok(b) => Ok(Self(
                header(Midi2Opcode::AssignableController, group, channel) | b | data as u64,
            )),
            Err(e) => Err(e),
        }
    }
}

impl RelativeRegisteredController {
    const RESERVED: u64 = BANK_INDEX_RESERVED;

    /// A Relative Registered Controller. Gives `OutOfRange` for a Bank or Index above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        bank: u8,
        index: u8,
        data: i32,
    ) -> Result<Self, UmpError> {
        match bank_index(bank, index) {
            Ok(b) => Ok(Self(
                header(Midi2Opcode::RelativeRegisteredController, group, channel)
                    | b
                    | (data as u32) as u64,
            )),
            Err(e) => Err(e),
        }
    }
}

impl RelativeAssignableController {
    const RESERVED: u64 = BANK_INDEX_RESERVED;

    /// A Relative Assignable Controller. Gives `OutOfRange` for a Bank or Index above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        bank: u8,
        index: u8,
        data: i32,
    ) -> Result<Self, UmpError> {
        match bank_index(bank, index) {
            Ok(b) => Ok(Self(
                header(Midi2Opcode::RelativeAssignableController, group, channel)
                    | b
                    | (data as u32) as u64,
            )),
            Err(e) => Err(e),
        }
    }
}

impl Midi2ProgramChange {
    const RESERVED: u64 = 0x0000_FFFE_80FF_8080;

    /// A Program Change. `bank` is `(Bank MSB, Bank LSB)`: `Some` sets Bank Valid to 1, `None`
    /// sets it to 0 and the bank fields to zero (7.4.9). Gives `OutOfRange` for a value above
    /// 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        program: u8,
        bank: Option<(u8, u8)>,
    ) -> Result<Self, UmpError> {
        let p = match check7(program) {
            Ok(p) => p,
            Err(e) => return Err(e),
        };
        let b = match bank {
            None => 0,
            Some((msb, lsb)) => {
                if msb > 0x7F || lsb > 0x7F {
                    return Err(UmpError::OutOfRange);
                }
                (1 << 32) | ((msb as u64) << 8) | lsb as u64
            }
        };
        Ok(Self(
            header(Midi2Opcode::ProgramChange, group, channel) | (p << 24) | b,
        ))
    }
}

/// Clears the bank fields of a Program Change with Bank Valid 0 (7.4.9).
const fn normalize_program_change(bits: u64) -> u64 {
    if (bits >> 32) & 1 == 0 {
        bits & !0x7F7F
    } else {
        bits
    }
}

impl Midi2ChannelPressure {
    const RESERVED: u64 = 0x0000_FFFF_0000_0000;
}

impl Midi2PitchBend {
    const RESERVED: u64 = 0x0000_FFFF_0000_0000;
}

impl PerNotePitchBend {
    const RESERVED: u64 = NOTE_BYTE4_RESERVED;

    /// A Per-Note Pitch Bend. Gives `OutOfRange` for a Note Number above 127.
    pub const fn try_new(
        group: Group,
        channel: Channel,
        note: u8,
        value: u32,
    ) -> Result<Self, UmpError> {
        match check7(note) {
            Ok(n) => Ok(Self(
                header(Midi2Opcode::PerNotePitchBend, group, channel) | (n << 40) | value as u64,
            )),
            Err(e) => Err(e),
        }
    }
}

const fn identity(bits: u64) -> u64 {
    bits
}

macro_rules! midi2_words {
    ($($ty:ident => $opcode:ident, $normalize:ident);* $(;)?) => {$(
        impl UmpWords for $ty {
            const WORDS: usize = 2;

            fn from_words(words: &[u32]) -> Result<Self, UmpError> {
                decode(words, Midi2Opcode::$opcode, Self::RESERVED).map(|b| Self($normalize(b)))
            }

            fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
                put(&split_u64(self.0), out)
            }
        }
    )*};
}

midi2_words! {
    RegisteredPerNoteController => RegisteredPerNoteController, identity;
    AssignablePerNoteController => AssignablePerNoteController, identity;
    RegisteredController => RegisteredController, identity;
    AssignableController => AssignableController, identity;
    RelativeRegisteredController => RelativeRegisteredController, identity;
    RelativeAssignableController => RelativeAssignableController, identity;
    PerNotePitchBend => PerNotePitchBend, identity;
    Midi2NoteOff => NoteOff, normalize_note;
    Midi2NoteOn => NoteOn, normalize_note;
    Midi2PolyPressure => PolyPressure, identity;
    Midi2ControlChange => ControlChange, identity;
    Midi2ProgramChange => ProgramChange, normalize_program_change;
    Midi2ChannelPressure => ChannelPressure, identity;
    Midi2PitchBend => PitchBend, identity;
    PerNoteManagement => PerNoteManagement, identity;
}

/// One MIDI 2.0 Channel Voice message (Table 30).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Midi2ChannelVoiceMessage {
    /// Registered Per-Note Controller (0x0).
    RegisteredPerNoteController(RegisteredPerNoteController),
    /// Assignable Per-Note Controller (0x1).
    AssignablePerNoteController(AssignablePerNoteController),
    /// Registered Controller (0x2).
    RegisteredController(RegisteredController),
    /// Assignable Controller (0x3).
    AssignableController(AssignableController),
    /// Relative Registered Controller (0x4).
    RelativeRegisteredController(RelativeRegisteredController),
    /// Relative Assignable Controller (0x5).
    RelativeAssignableController(RelativeAssignableController),
    /// Per-Note Pitch Bend (0x6).
    PerNotePitchBend(PerNotePitchBend),
    /// Note Off (0x8).
    NoteOff(Midi2NoteOff),
    /// Note On (0x9).
    NoteOn(Midi2NoteOn),
    /// Poly Pressure (0xA).
    PolyPressure(Midi2PolyPressure),
    /// Control Change (0xB).
    ControlChange(Midi2ControlChange),
    /// Program Change (0xC).
    ProgramChange(Midi2ProgramChange),
    /// Channel Pressure (0xD).
    ChannelPressure(Midi2ChannelPressure),
    /// Pitch Bend (0xE).
    PitchBend(Midi2PitchBend),
    /// Per-Note Management (0xF).
    PerNoteManagement(PerNoteManagement),
}

impl Midi2ChannelVoiceMessage {
    /// Decodes one MIDI 2.0 Channel Voice packet (two words) by its opcode. Gives `None` for
    /// short input, a Message Type other than 0x4, or opcode 0x7. Never panics.
    pub fn parse(words: &[u32]) -> Option<Self> {
        use Midi2Opcode as O;
        let opcode = Midi2Opcode::from_bits((*words.first()? >> 20) as u8)?;
        Some(match opcode {
            O::RegisteredPerNoteController => Self::RegisteredPerNoteController(
                RegisteredPerNoteController::from_words(words).ok()?,
            ),
            O::AssignablePerNoteController => Self::AssignablePerNoteController(
                AssignablePerNoteController::from_words(words).ok()?,
            ),
            O::RegisteredController => {
                Self::RegisteredController(RegisteredController::from_words(words).ok()?)
            }
            O::AssignableController => {
                Self::AssignableController(AssignableController::from_words(words).ok()?)
            }
            O::RelativeRegisteredController => Self::RelativeRegisteredController(
                RelativeRegisteredController::from_words(words).ok()?,
            ),
            O::RelativeAssignableController => Self::RelativeAssignableController(
                RelativeAssignableController::from_words(words).ok()?,
            ),
            O::PerNotePitchBend => {
                Self::PerNotePitchBend(PerNotePitchBend::from_words(words).ok()?)
            }
            O::NoteOff => Self::NoteOff(Midi2NoteOff::from_words(words).ok()?),
            O::NoteOn => Self::NoteOn(Midi2NoteOn::from_words(words).ok()?),
            O::PolyPressure => Self::PolyPressure(Midi2PolyPressure::from_words(words).ok()?),
            O::ControlChange => Self::ControlChange(Midi2ControlChange::from_words(words).ok()?),
            O::ProgramChange => Self::ProgramChange(Midi2ProgramChange::from_words(words).ok()?),
            O::ChannelPressure => {
                Self::ChannelPressure(Midi2ChannelPressure::from_words(words).ok()?)
            }
            O::PitchBend => Self::PitchBend(Midi2PitchBend::from_words(words).ok()?),
            O::PerNoteManagement => {
                Self::PerNoteManagement(PerNoteManagement::from_words(words).ok()?)
            }
        })
    }

    /// The message of the words of a MIDI 2.0 Channel Voice packet that the crate wrote with
    /// opcode 0x2, 0x3 or 0x8 to 0xE, with no checks: the translator of D.3 gives only these
    /// (DEC-018). Another opcode gives a Pitch Bend; the callers never pass one.
    pub(crate) const fn from_translated(words: [u32; 2]) -> Self {
        let bits = join_u64(words);
        match (words[0] >> 20) & 0xF {
            0x2 => Self::RegisteredController(RegisteredController(bits)),
            0x3 => Self::AssignableController(AssignableController(bits)),
            0x8 => Self::NoteOff(Midi2NoteOff(bits)),
            0x9 => Self::NoteOn(Midi2NoteOn(bits)),
            0xA => Self::PolyPressure(Midi2PolyPressure(bits)),
            0xB => Self::ControlChange(Midi2ControlChange(bits)),
            0xC => Self::ProgramChange(Midi2ProgramChange(bits)),
            0xD => Self::ChannelPressure(Midi2ChannelPressure(bits)),
            _ => Self::PitchBend(Midi2PitchBend(bits)),
        }
    }

    /// The packet as two 32-bit words, word 0 first.
    pub(crate) const fn into_words(self) -> [u32; 2] {
        split_u64(match self {
            Self::RegisteredPerNoteController(m) => m.0,
            Self::AssignablePerNoteController(m) => m.0,
            Self::RegisteredController(m) => m.0,
            Self::AssignableController(m) => m.0,
            Self::RelativeRegisteredController(m) => m.0,
            Self::RelativeAssignableController(m) => m.0,
            Self::PerNotePitchBend(m) => m.0,
            Self::NoteOff(m) => m.0,
            Self::NoteOn(m) => m.0,
            Self::PolyPressure(m) => m.0,
            Self::ControlChange(m) => m.0,
            Self::ProgramChange(m) => m.0,
            Self::ChannelPressure(m) => m.0,
            Self::PitchBend(m) => m.0,
            Self::PerNoteManagement(m) => m.0,
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;

    fn words<T: UmpWords>(m: &T) -> [u32; 2] {
        let mut out = [0u32; 2];
        assert_eq!(m.to_words(&mut out), Ok(2));
        out
    }

    fn g(i: u8) -> Group {
        Group::new(i).unwrap()
    }

    fn c(i: u8) -> Channel {
        Channel::new(i).unwrap()
    }

    #[test]
    fn midi2_opcode_matches_table_30() {
        for op in 0..=0xFu8 {
            match Midi2Opcode::from_bits(op) {
                Some(o) => assert_eq!(o.into_bits(), op),
                None => assert_eq!(op, 0x7),
            }
        }
    }

    #[test]
    fn midi2_note_messages_match_figures_46_47() {
        // Table 30: rkkkkkkk | Attribute Type | Velocity (16) | Attribute Data (16).
        let on = Midi2NoteOn::try_new(g(0), c(0), 60, 0xC000, NoteAttribute::None).unwrap();
        assert_eq!(words(&on), [0x4090_3C00, 0xC000_0000]);
        assert_eq!(
            (
                on.message_type(),
                on.opcode(),
                on.note(),
                on.velocity(),
                on.attribute()
            ),
            (
                MessageType::Midi2ChannelVoice,
                0x9,
                60,
                0xC000,
                NoteAttribute::None
            )
        );
        let pitch = Pitch7_9::from_bits((61 << 9) | 256);
        let off =
            Midi2NoteOff::try_new(g(3), c(15), 0x7F, 0xFFFF, NoteAttribute::Pitch(pitch)).unwrap();
        assert_eq!(words(&off), [0x438F_7F03, 0xFFFF_7B00]);
        assert_eq!(off.attribute(), NoteAttribute::Pitch(pitch));
        assert_eq!((pitch.note(), pitch.fraction()), (61, 256));
        let m = Midi2NoteOn::try_new(
            g(0),
            c(0),
            1,
            2,
            NoteAttribute::ManufacturerSpecific(0xABCD),
        )
        .unwrap();
        assert_eq!(words(&m), [0x4090_0101, 0x0002_ABCD]);
        let p = Midi2NoteOn::try_new(g(0), c(0), 1, 2, NoteAttribute::ProfileSpecific(5)).unwrap();
        assert_eq!(words(&p), [0x4090_0102, 0x0002_0005]);
    }

    #[test]
    fn midi2_note_on_velocity_zero_stays_note_on() {
        // 7.4.2: Velocity 0 is a Note On.
        let m = Midi2ChannelVoiceMessage::parse(&[0x4090_3C00, 0x0000_0000]).unwrap();
        assert!(matches!(m, Midi2ChannelVoiceMessage::NoteOn(n) if n.velocity() == 0));
    }

    #[test]
    fn midi2_attribute_type_matches_table_8() {
        for t in 0..=0xFFu8 {
            match AttributeType::from_bits(t) {
                Some(a) => assert_eq!(a.into_bits(), t),
                None => assert!(t >= 0x04),
            }
        }
        // Type 0x00: the reader ignores Attribute Data; 0x04 and above are reserved.
        let m = Midi2NoteOn::from_words(&[0x4090_3C00, 0x8000_1234]).unwrap();
        assert_eq!(m.attribute(), NoteAttribute::None);
        assert_eq!(words(&m), [0x4090_3C00, 0x8000_0000]);
        let r = Midi2NoteOn::from_words(&[0x4090_3C04, 0x8000_1234]).unwrap();
        assert_eq!(
            r.attribute(),
            NoteAttribute::Reserved {
                attribute_type: 4,
                data: 0x1234
            }
        );
        let reserved = NoteAttribute::Reserved {
            attribute_type: 4,
            data: 0,
        };
        assert_eq!(
            Midi2NoteOn::try_new(g(0), c(0), 60, 1, reserved),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            Midi2NoteOff::try_new(g(0), c(0), 60, 1, reserved),
            Err(UmpError::OutOfRange)
        );
    }

    #[test]
    fn midi2_controller_messages_match_table_30() {
        assert_eq!(
            words(&Midi2PolyPressure::try_new(g(0), c(1), 60, 0x8000_0000).unwrap()),
            [0x40A1_3C00, 0x8000_0000]
        );
        let rpnc = RegisteredPerNoteController::try_new(g(0), c(0), 60, 3, 0x7800_0000).unwrap();
        assert_eq!(words(&rpnc), [0x4000_3C03, 0x7800_0000]);
        assert_eq!(rpnc.controller(), 3);
        assert_eq!(
            words(&AssignablePerNoteController::try_new(g(0), c(0), 60, 0xFF, 1).unwrap()),
            [0x4010_3CFF, 0x0000_0001]
        );
        assert_eq!(
            words(&Midi2ControlChange::try_new(g(1), c(0), 7, 0xFFFF_FFFF).unwrap()),
            [0x41B0_0700, 0xFFFF_FFFF]
        );
        assert_eq!(
            words(&RegisteredController::try_new(g(0), c(0), 0, 7, 0x0600_0000).unwrap()),
            [0x4020_0007, 0x0600_0000]
        );
        assert_eq!(
            words(&AssignableController::try_new(g(0), c(0), 0x7F, 0x7F, 5).unwrap()),
            [0x4030_7F7F, 0x0000_0005]
        );
        let rel = RelativeRegisteredController::try_new(g(0), c(0), 1, 2, -1).unwrap();
        assert_eq!(words(&rel), [0x4040_0102, 0xFFFF_FFFF]);
        assert_eq!(rel.data(), -1);
        assert_eq!(
            words(&RelativeAssignableController::try_new(g(0), c(0), 1, 2, i32::MIN).unwrap()),
            [0x4050_0102, 0x8000_0000]
        );
    }

    #[test]
    fn midi2_per_note_management_matches_figure_51() {
        // Byte 4: 6 reserved option flags, then D, then S.
        let m = PerNoteManagement::try_new(g(0), c(0), 60, true, false).unwrap();
        assert_eq!(words(&m), [0x40F0_3C02, 0]);
        assert_eq!(
            words(&PerNoteManagement::try_new(g(0), c(0), 60, false, true).unwrap()),
            [0x40F0_3C01, 0]
        );
        // D=0 and S=0 is a valid message with no defined function.
        let none = PerNoteManagement::from_words(&[0x40F0_3C00, 0]).unwrap();
        assert!(!none.detach() && !none.reset());
    }

    #[test]
    fn midi2_program_change_matches_figure_64() {
        let with_bank = Midi2ProgramChange::try_new(g(0), c(0), 5, Some((1, 2))).unwrap();
        assert_eq!(words(&with_bank), [0x40C0_0001, 0x0500_0102]);
        assert!(with_bank.bank_valid());
        assert_eq!(
            (
                with_bank.program(),
                with_bank.bank_msb(),
                with_bank.bank_lsb()
            ),
            (5, 1, 2)
        );
        // B = 0: the bank fields are zero (7.4.9), and the reader clears them.
        let no_bank = Midi2ProgramChange::try_new(g(0), c(0), 5, None).unwrap();
        assert_eq!(words(&no_bank), [0x40C0_0000, 0x0500_0000]);
        assert_eq!(
            Midi2ProgramChange::from_words(&[0x40C0_0000, 0x0500_0102]),
            Ok(no_bank)
        );
        assert_eq!(
            Midi2ProgramChange::try_new(g(0), c(0), 128, None),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            Midi2ProgramChange::try_new(g(0), c(0), 0, Some((0, 128))),
            Err(UmpError::OutOfRange)
        );
    }

    #[test]
    fn midi2_pressure_and_pitch_bend_match_figures_65_to_67() {
        let cp = Midi2ChannelPressure::new()
            .with_group(g(2))
            .with_channel(c(3))
            .with_pressure(0x1234_5678);
        assert_eq!(words(&cp), [0x42D3_0000, 0x1234_5678]);
        let pb = Midi2PitchBend::new().with_value(0x8000_0000);
        assert_eq!(words(&pb), [0x40E0_0000, 0x8000_0000]);
        assert_eq!(words(&Midi2PitchBend::default()), [0x40E0_0000, 0]);
        assert_eq!(
            words(&PerNotePitchBend::try_new(g(0), c(0), 60, 0x8000_0000).unwrap()),
            [0x4060_3C00, 0x8000_0000]
        );
    }

    #[test]
    fn midi2_try_new_rejects_out_of_range() {
        assert_eq!(
            Midi2NoteOn::try_new(g(0), c(0), 128, 0, NoteAttribute::None),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            Midi2PolyPressure::try_new(g(0), c(0), 128, 0),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            Midi2ControlChange::try_new(g(0), c(0), 128, 0),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            RegisteredController::try_new(g(0), c(0), 128, 0, 0),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            AssignableController::try_new(g(0), c(0), 0, 128, 0),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            PerNotePitchBend::try_new(g(0), c(0), 200, 0),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            PerNoteManagement::try_new(g(0), c(0), 128, false, false),
            Err(UmpError::OutOfRange)
        );
        // Appendix A: index 0, 4 and 96 are reserved for Registered Per-Note Controllers.
        for reserved in [0u8, 4, 9, 12, 69, 80, 90, 96, 255] {
            assert_eq!(
                RegisteredPerNoteController::try_new(g(0), c(0), 60, reserved, 0),
                Err(UmpError::OutOfRange)
            );
        }
    }

    #[test]
    fn registered_per_note_controller_index_matches_table_22() {
        let defined: std::vec::Vec<u8> = [1u8, 2, 3, 7, 8, 10, 11]
            .into_iter()
            .chain(70..=79)
            .chain(91..=95)
            .collect();
        for i in 0..=255u8 {
            match RegisteredPerNoteControllerIndex::from_index(i) {
                Some(x) => {
                    assert!(defined.contains(&i), "{i}");
                    assert_eq!(x.index(), i);
                }
                None => assert!(!defined.contains(&i), "{i}"),
            }
        }
    }

    #[test]
    fn fixed_point_pitch_and_sensitivity() {
        // 7.4.15.2: Pitch 7.25, integer 69 (A=440) plus half an HCU.
        let p = Pitch7_25::from_bits((69 << 25) | (1 << 24));
        assert_eq!(
            (p.note(), p.fraction(), p.into_bits()),
            (69, 1 << 24, (69 << 25) | (1 << 24))
        );
        // 7.4.13.3: 0x0C000000 is 6 units of 100 cents.
        let s = PerNotePitchBendSensitivity::from_data(0x0C00_0000);
        assert_eq!(
            (s.semitones(), s.fraction(), s.into_data()),
            (6, 0, 0x0C00_0000)
        );
        assert_eq!(
            (
                PerNotePitchBendSensitivity::BANK,
                PerNotePitchBendSensitivity::INDEX
            ),
            (0, 7)
        );
        let half = PerNotePitchBendSensitivity::from_data(0x0300_0000);
        assert_eq!((half.semitones(), half.fraction()), (1, 1 << 24));
    }

    #[test]
    fn msb_value_ignores_undefined_low_bits() {
        // 7.4.7.x: a 7-bit value in the MSBs of a 32-bit data field.
        assert_eq!(msb_value(0xFE00_0000, 7), 0x7F);
        assert_eq!(msb_value(0xFE01_2345, 7), 0x7F);
        assert_eq!(msb_value(0x0600_0000, 7), 0x03);
        assert_eq!(msb_value(0x1234_5678, 32), 0x1234_5678);
        assert_eq!(msb_value(0x1234_5678, 40), 0x1234_5678);
        assert_eq!(msb_value(0xFFFF_FFFF, 0), 0);
    }

    #[test]
    fn midi2_reader_ignores_reserved_bits() {
        let m = Midi2ControlChange::from_words(&[0x40B0_87FF, 1]).unwrap();
        assert_eq!(words(&m), [0x40B0_0700, 1]);
        let pb = Midi2PitchBend::from_words(&[0x40E0_FFFF, 2]).unwrap();
        assert_eq!(words(&pb), [0x40E0_0000, 2]);
        let pc = Midi2ProgramChange::from_words(&[0x40C0_FFFF, 0xFFFF_FFFF]).unwrap();
        assert_eq!(words(&pc), [0x40C0_0001, 0x7F00_7F7F]);
        let pnm = Midi2ChannelVoiceMessage::parse(&[0x40F0_BCFF, 0xFFFF_FFFF]).unwrap();
        assert_eq!(pnm.into_words(), [0x40F0_3C03, 0]);
    }

    #[test]
    fn midi2_from_words_checks_header_and_length() {
        assert_eq!(
            Midi2NoteOn::from_words(&[0x4080_3C00, 0]),
            Err(UmpError::UnexpectedMessage)
        );
        assert_eq!(
            Midi2NoteOn::from_words(&[0x2090_3C00, 0]),
            Err(UmpError::UnexpectedMessage)
        );
        assert_eq!(
            Midi2NoteOn::from_words(&[0x4090_3C00]),
            Err(UmpError::Truncated { needed: 2, got: 1 })
        );
        assert_eq!(
            Midi2PitchBend::new().to_words(&mut [0u32; 1]),
            Err(UmpError::BufferTooSmall { needed: 2, got: 1 })
        );
    }

    #[test]
    fn midi2_message_parse_dispatches_on_opcode() {
        for op in 0x0..=0xFu32 {
            let w = [0x4000_0000 | (op << 20) | 0x0000_0101, 0x8000_0000];
            match Midi2ChannelVoiceMessage::parse(&w) {
                Some(m) => {
                    assert_ne!(op, 0x7);
                    assert_eq!((m.into_words()[0] >> 20) & 0xF, op);
                }
                None => assert!(op == 0x7 || op == 0x0, "opcode {op:#x}"),
            }
        }
        assert_eq!(Midi2ChannelVoiceMessage::parse(&[0x4090_3C00]), None);
        assert_eq!(Midi2ChannelVoiceMessage::parse(&[]), None);
    }

    #[test]
    fn prop_midi2_round_trip() {
        // DEC-UMP-003: in-range values round-trip; the 7-bit fields reject values above 127.
        proptest!(|(gi in 0u8..16, ci in 0u8..16, a in any::<u8>(), b in any::<u8>(), v16 in any::<u16>(), d in any::<u32>(), r in any::<i32>())| {
            match Midi2NoteOn::try_new(g(gi), c(ci), a, v16, NoteAttribute::ManufacturerSpecific(v16)) {
                Ok(m) => {
                    prop_assert!(a <= 127);
                    prop_assert_eq!(Midi2NoteOn::from_words(&words(&m)), Ok(m));
                    prop_assert_eq!((m.group(), m.channel(), m.note(), m.velocity()), (g(gi), c(ci), a, v16));
                }
                Err(_) => prop_assert!(a > 127),
            }
            match RegisteredController::try_new(g(gi), c(ci), a, b, d) {
                Ok(m) => {
                    prop_assert!(a <= 127 && b <= 127);
                    prop_assert_eq!(RegisteredController::from_words(&words(&m)), Ok(m));
                    prop_assert_eq!((m.bank(), m.index(), m.data()), (a, b, d));
                }
                Err(_) => prop_assert!(a > 127 || b > 127),
            }
            if let Ok(m) = RelativeAssignableController::try_new(g(gi), c(ci), a, b, r) {
                prop_assert_eq!(m.data(), r);
                prop_assert_eq!(RelativeAssignableController::from_words(&words(&m)), Ok(m));
            }
            if let Ok(m) = Midi2ProgramChange::try_new(g(gi), c(ci), a, Some((b, a))) {
                prop_assert_eq!(Midi2ProgramChange::from_words(&words(&m)), Ok(m));
            }
            let pb = Midi2PitchBend::new().with_group(g(gi)).with_channel(c(ci)).with_value(d);
            prop_assert_eq!(Midi2PitchBend::from_words(&words(&pb)), Ok(pb));
        });
    }

    #[test]
    fn prop_midi2_reader_ignores_reserved_bits() {
        // 2.1.3: any reserved bits decode to the same message as the cleared packet.
        proptest!(|(w0 in any::<u32>(), w1 in any::<u32>(), op in 0x8u32..=0xF)| {
            let w = [0x4000_0000 | (w0 & 0x0F0F_FFFF) | (op << 20), w1];
            let m = Midi2ChannelVoiceMessage::parse(&w).unwrap();
            let cleared = m.into_words();
            prop_assert_eq!(Midi2ChannelVoiceMessage::parse(&cleared), Some(m));
            prop_assert_eq!(cleared[0] >> 16, w[0] >> 16);
        });
    }
}
