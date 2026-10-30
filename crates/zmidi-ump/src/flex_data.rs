//! Flex Data Messages: Message Type 0xD (M2-104-UM 7.5, Figures 69 to 82, Table 16).
//!
//! - Format: one 128-bit packet. `mt=D` (bits 127..124), Group (123..120), Format (119..118),
//!   Address (117..116), Channel (115..112), Status Bank (111..104), Status (103..96), then
//!   12 data bytes (Figure 69).
//! - Format: 0 Complete, 1 Start, 2 Continue, 3 End. A message is one Complete UMP, or Start,
//!   zero or more Continue and End, with at most 32 UMPs (7.5.1).
//! - Address: 0 Channel (the Channel field is the destination), 1 Group (the Channel field is
//!   reserved and 0); 2 and 3 are reserved (7.5.1).
//! - Status Bank 0x00: Set Tempo, Set Time Signature, Set Metronome (Format 0 and Address 1
//!   only), Set Key Signature and Set Chord Name. Status Banks 0x01 and 0x02: text in the Text
//!   Messages Common Format, UTF-8 without a Byte Order Mark, 12 bytes in each UMP; 0x00 bytes
//!   end the text and occur only in an End or Complete UMP (7.5.9).
//! - Lyrics and Ruby: a Complete UMP filled with 0x00 is a melisma; 0x00 in a Continue UMP also
//!   marks a melisma. 7.5.10 calls Continue "Format = 0x1"; 7.5.1 defines Continue as 0x2, which
//!   this module uses (REQ-FLEX-014).
//! - The caller gives NFC text; the writers do not normalize it (1.8, DEC-UMP-005).
//! - Reserved bits are zero on write and cleared on read (2.1.3). The structs have no public
//!   `from_bits` (DEC-UMP-002).

use core::ops::Deref;

use bitfield_struct::bitfield;

use crate::packet::{
    join_u128, put, split_u128, take, Channel, Group, MessageType, UmpError, UmpWords,
};

/// Format of a Flex Data packet, bits 119..118 (Figure 69).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FlexFormat {
    /// 0: Complete Flex Data Message in one UMP.
    Complete = 0,
    /// 1: Flex Data Message Start UMP.
    Start = 1,
    /// 2: Flex Data Message Continue UMP.
    Continue = 2,
    /// 3: Flex Data Message End UMP.
    End = 3,
}

impl FlexFormat {
    /// The 2-bit Format value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Format for the low 2 bits of `value`. Every 2-bit value is defined.
    pub const fn from_bits(value: u8) -> Self {
        match value & 0b11 {
            0 => Self::Complete,
            1 => Self::Start,
            2 => Self::Continue,
            _ => Self::End,
        }
    }
}

/// Address of a Flex Data packet, bits 117..116 (Figure 69). 2 and 3 are reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FlexAddress {
    /// 0: the message goes to the Channel in the Channel field.
    Channel = 0,
    /// 1: the message goes to the Group; the Channel field is reserved.
    Group = 1,
}

impl FlexAddress {
    /// The 2-bit Address value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Address for a 2-bit `value`, or `None` for the reserved 2 and 3.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Channel),
            1 => Some(Self::Group),
            _ => None,
        }
    }
}

/// Status Bank of a Flex Data packet, bits 111..104 (7.5.1). 0x03 to 0xFF are reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FlexStatusBank {
    /// 0x00: Setup and Performance Events (no text).
    Setup = 0x00,
    /// 0x01: Metadata Text.
    MetadataText = 0x01,
    /// 0x02: Performance Text Events (including lyrics).
    PerformanceText = 0x02,
}

impl FlexStatusBank {
    /// The Status Bank byte.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Status Bank for `value`, or `None` for a reserved value.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::Setup),
            0x01 => Some(Self::MetadataText),
            0x02 => Some(Self::PerformanceText),
            _ => None,
        }
    }
}

/// Status values of Status Bank 0x00 (7.5.3 to 7.5.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SetupStatus {
    /// 0x00: Set Tempo.
    SetTempo = 0x00,
    /// 0x01: Set Time Signature.
    SetTimeSignature = 0x01,
    /// 0x02: Set Metronome.
    SetMetronome = 0x02,
    /// 0x05: Set Key Signature.
    SetKeySignature = 0x05,
    /// 0x06: Set Chord Name.
    SetChordName = 0x06,
}

impl SetupStatus {
    /// The Status byte.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Setup status for `value`, or `None` for a value that 7.5 does not define.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::SetTempo),
            0x01 => Some(Self::SetTimeSignature),
            0x02 => Some(Self::SetMetronome),
            0x05 => Some(Self::SetKeySignature),
            0x06 => Some(Self::SetChordName),
            _ => None,
        }
    }
}

/// Status values of Status Bank 0x01, Metadata Text (Table 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MetadataTextStatus {
    /// 0x00: Unknown Metadata Text Event.
    Unknown = 0x00,
    /// 0x01: Project Name. Address should be Group.
    ProjectName = 0x01,
    /// 0x02: Composition (Song) Name.
    CompositionName = 0x02,
    /// 0x03: MIDI Clip Name. Address should be Group.
    MidiClipName = 0x03,
    /// 0x04: Copyright Notice.
    CopyrightNotice = 0x04,
    /// 0x05: Composer Name.
    ComposerName = 0x05,
    /// 0x06: Lyricist Name.
    LyricistName = 0x06,
    /// 0x07: Arranger Name.
    ArrangerName = 0x07,
    /// 0x08: Publisher Name.
    PublisherName = 0x08,
    /// 0x09: Primary Performer Name.
    PrimaryPerformerName = 0x09,
    /// 0x0A: Accompanying Performer Name.
    AccompanyingPerformerName = 0x0A,
    /// 0x0B: Recording/Concert Date, in ISO 8601 form (7.5.9.2).
    RecordingDate = 0x0B,
    /// 0x0C: Recording/Concert Location.
    RecordingLocation = 0x0C,
}

impl MetadataTextStatus {
    /// The Status byte.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Metadata Text status for `value`, or `None` for 0x0D to 0xFF.
    pub const fn from_bits(value: u8) -> Option<Self> {
        Some(match value {
            0x00 => Self::Unknown,
            0x01 => Self::ProjectName,
            0x02 => Self::CompositionName,
            0x03 => Self::MidiClipName,
            0x04 => Self::CopyrightNotice,
            0x05 => Self::ComposerName,
            0x06 => Self::LyricistName,
            0x07 => Self::ArrangerName,
            0x08 => Self::PublisherName,
            0x09 => Self::PrimaryPerformerName,
            0x0A => Self::AccompanyingPerformerName,
            0x0B => Self::RecordingDate,
            0x0C => Self::RecordingLocation,
            _ => return None,
        })
    }
}

/// Status values of Status Bank 0x02, Performance Text Events (Table 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PerformanceTextStatus {
    /// 0x00: Unknown Performance Text Event.
    Unknown = 0x00,
    /// 0x01: Lyrics (7.5.10).
    Lyrics = 0x01,
    /// 0x02: Lyrics Language, a BCP 47 tag (7.5.11).
    LyricsLanguage = 0x02,
    /// 0x03: Ruby (7.5.12).
    Ruby = 0x03,
    /// 0x04: Ruby Language, a BCP 47 tag (7.5.13).
    RubyLanguage = 0x04,
}

impl PerformanceTextStatus {
    /// The Status byte.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Performance Text status for `value`, or `None` for 0x05 to 0xFF.
    pub const fn from_bits(value: u8) -> Option<Self> {
        Some(match value {
            0x00 => Self::Unknown,
            0x01 => Self::Lyrics,
            0x02 => Self::LyricsLanguage,
            0x03 => Self::Ruby,
            0x04 => Self::RubyLanguage,
            _ => return None,
        })
    }
}

/// Status Bank and Status of a text message (Table 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlexTextStatus {
    /// Status Bank 0x01.
    Metadata(MetadataTextStatus),
    /// Status Bank 0x02.
    Performance(PerformanceTextStatus),
}

impl FlexTextStatus {
    const fn bank(self) -> FlexStatusBank {
        match self {
            Self::Metadata(_) => FlexStatusBank::MetadataText,
            Self::Performance(_) => FlexStatusBank::PerformanceText,
        }
    }

    const fn status(self) -> u8 {
        match self {
            Self::Metadata(s) => s.into_bits(),
            Self::Performance(s) => s.into_bits(),
        }
    }

    const fn from_parts(bank: u8, status: u8) -> Option<Self> {
        match FlexStatusBank::from_bits(bank) {
            Some(FlexStatusBank::MetadataText) => match MetadataTextStatus::from_bits(status) {
                Some(s) => Some(Self::Metadata(s)),
                None => None,
            },
            Some(FlexStatusBank::PerformanceText) => {
                match PerformanceTextStatus::from_bits(status) {
                    Some(s) => Some(Self::Performance(s)),
                    None => None,
                }
            }
            _ => None,
        }
    }

    /// True for Lyrics and Ruby, which have melisma events (7.5.10.1, 7.5.12.1).
    const fn has_melisma(self) -> bool {
        matches!(
            self,
            Self::Performance(PerformanceTextStatus::Lyrics | PerformanceTextStatus::Ruby)
        )
    }
}

/// Destination of a Flex Data message: a Channel, or the whole Group (7.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlexDestination {
    /// Address 0: the Channel in the Channel field.
    Channel(Channel),
    /// Address 1: the Group; the Channel field is 0.
    Group,
}

impl FlexDestination {
    /// Address and Channel bits (117..112).
    const fn bits(self) -> u128 {
        match self {
            Self::Channel(c) => (c.into_bits() as u128) << 112,
            Self::Group => 1 << 116,
        }
    }

    /// The destination of a packet, or `None` for a reserved Address.
    const fn from_packet(bits: u128) -> Option<Self> {
        match FlexAddress::from_bits(((bits >> 116) & 0b11) as u8) {
            Some(FlexAddress::Channel) => Some(Self::Channel(Channel::from_bits(
                ((bits >> 112) & 0xF) as u8,
            ))),
            Some(FlexAddress::Group) => Some(Self::Group),
            None => None,
        }
    }
}

/// Message Type, Group, Format, Address, Channel, Status Bank and Status bits.
const fn header(
    group: Group,
    format: FlexFormat,
    destination: FlexDestination,
    bank: FlexStatusBank,
    status: u8,
) -> u128 {
    ((MessageType::FlexData.into_bits() as u128) << 124)
        | ((group.into_bits() as u128) << 120)
        | ((format.into_bits() as u128) << 118)
        | destination.bits()
        | ((bank.into_bits() as u128) << 104)
        | ((status as u128) << 96)
}

/// Header of a Complete, Group-addressed Setup message.
const fn setup_header(group: Group, status: SetupStatus) -> u128 {
    header(
        group,
        FlexFormat::Complete,
        FlexDestination::Group,
        FlexStatusBank::Setup,
        status.into_bits(),
    )
}

/// Mask of the Message Type, Group, Format, Address, Channel, Status Bank and Status bits.
const HEADER: u128 = 0xFFFF_FFFF << 96;

/// Reads a Complete Setup packet with `status` from `words`. A message that is Group-addressed
/// only (`group_only`) needs Address 1, and its Channel field is cleared. Clears every bit
/// outside `valid`.
fn decode(
    words: &[u32],
    status: SetupStatus,
    group_only: bool,
    valid: u128,
) -> Result<u128, UmpError> {
    let bits = join_u128(take::<4>(words)?);
    let fixed = (bits >> 124) as u8 == MessageType::FlexData.into_bits()
        && (bits >> 118) & 0b11 == FlexFormat::Complete.into_bits() as u128
        && (bits >> 104) as u8 == FlexStatusBank::Setup.into_bits()
        && (bits >> 96) as u8 == status.into_bits();
    let destination = FlexDestination::from_packet(bits);
    match destination {
        Some(FlexDestination::Group) if fixed => Ok(bits & valid & !(0xF << 112)),
        Some(FlexDestination::Channel(_)) if fixed && !group_only => Ok(bits & valid),
        _ => Err(UmpError::UnexpectedMessage),
    }
}

/// Set Tempo, Bank 0x00 Status 0x00 (7.5.3, Figure 70): the number of 10 ns units per quarter
/// note. Format Complete and Address Group.
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct SetTempo {
    /// Message Type, 4 bits: 0xD.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Format, Address, Channel, Status Bank and Status: 24 bits.
    #[bits(24, access = RO)]
    header_bits: u32,
    /// Number of 10 nanosecond units per quarter note, 32 bits.
    pub ten_ns_per_quarter_note: u32,
    /// Reserved, 64 bits.
    __: u64,
}

impl SetTempo {
    const VALID: u128 = HEADER | (0xFFFF_FFFF << 64);

    /// A Set Tempo for `group` with `ten_ns_per_quarter_note` units of 10 ns per quarter note.
    /// Every value is valid (the slowest tempo is 1.39698 bpm).
    pub const fn new(group: Group, ten_ns_per_quarter_note: u32) -> Self {
        Self(setup_header(group, SetupStatus::SetTempo) | ((ten_ns_per_quarter_note as u128) << 64))
    }
}

/// Set Time Signature, Bank 0x00 Status 0x01 (7.5.4, Figure 71). Format Complete and Address
/// Group.
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct SetTimeSignature {
    /// Message Type, 4 bits: 0xD.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Format, Address, Channel, Status Bank and Status: 24 bits.
    #[bits(24, access = RO)]
    header_bits: u32,
    /// Numerator: beats in a bar, 8 bits.
    #[bits(8, access = RO)]
    pub numerator: u8,
    /// Denominator as a negative power of 2 (2 is a quarter note, 3 an eighth note), 8 bits;
    /// 0 is a non-standard denominator.
    pub denominator: u8,
    /// Number of 1/32 notes in 24 MIDI Clocks, 8 bits.
    pub thirty_second_notes: u8,
    /// Reserved, 72 bits.
    #[bits(72)]
    __: u128,
}

impl SetTimeSignature {
    const VALID: u128 = HEADER | (0xFF_FFFF << 72);

    /// A Set Time Signature. Gives `OutOfRange` for a Numerator of 0. 7.5.4 gives the range 1
    /// to 256 for an 8-bit field and defines no code for 256, so the Numerator is 1 to 255.
    pub const fn try_new(
        group: Group,
        numerator: u8,
        denominator: u8,
        thirty_second_notes: u8,
    ) -> Result<Self, UmpError> {
        if numerator == 0 {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            setup_header(group, SetupStatus::SetTimeSignature)
                | ((numerator as u128) << 88)
                | ((denominator as u128) << 80)
                | ((thirty_second_notes as u128) << 72),
        ))
    }
}

/// Set Metronome, Bank 0x00 Status 0x02 (7.5.5, Figure 72). Format Complete and Address Group.
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct SetMetronome {
    /// Message Type, 4 bits: 0xD.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Format, Address, Channel, Status Bank and Status: 24 bits.
    #[bits(24, access = RO)]
    header_bits: u32,
    /// Number of MIDI Clocks per Primary Click, 8 bits.
    pub clocks_per_primary_click: u8,
    /// Bar Accent Part 1, 8 bits.
    #[bits(8, access = RO)]
    pub bar_accent_part1: u8,
    /// Bar Accent Part 2, 8 bits.
    #[bits(8, access = RO)]
    pub bar_accent_part2: u8,
    /// Bar Accent Part 3, 8 bits.
    #[bits(8, access = RO)]
    pub bar_accent_part3: u8,
    /// Number of Subdivision Clicks 1, 8 bits; 0 is no subdivision.
    pub subdivision_clicks1: u8,
    /// Number of Subdivision Clicks 2, 8 bits; 0 is no subdivision.
    pub subdivision_clicks2: u8,
    /// Reserved, 48 bits.
    #[bits(48)]
    __: u64,
}

impl SetMetronome {
    const VALID: u128 = HEADER | (0xFFFF_FFFF << 64) | (0xFFFF << 48);

    /// A Set Metronome for a bar of `beats_in_bar` beats, which the message does not carry.
    /// Gives `OutOfRange` when the three Bar Accent parts do not sum to `beats_in_bar` (7.5.5);
    /// an undivided bar has Part 1 equal to the beats and Parts 2 and 3 zero.
    pub const fn try_new(
        group: Group,
        clocks_per_primary_click: u8,
        bar_accents: [u8; 3],
        subdivision_clicks: [u8; 2],
        beats_in_bar: u16,
    ) -> Result<Self, UmpError> {
        let sum = bar_accents[0] as u16 + bar_accents[1] as u16 + bar_accents[2] as u16;
        if sum != beats_in_bar {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            setup_header(group, SetupStatus::SetMetronome)
                | ((clocks_per_primary_click as u128) << 88)
                | ((bar_accents[0] as u128) << 80)
                | ((bar_accents[1] as u128) << 72)
                | ((bar_accents[2] as u128) << 64)
                | ((subdivision_clicks[0] as u128) << 56)
                | ((subdivision_clicks[1] as u128) << 48),
        ))
    }
}

/// A 4-bit two's complement value as a signed value.
const fn signed4(bits: u8) -> i8 {
    ((bits << 4) as i8) >> 4
}

/// A signed value as 4-bit two's complement bits.
const fn bits4(value: i8) -> u128 {
    (value as u8 & 0xF) as u128
}

/// Set Key Signature, Bank 0x00 Status 0x05 (7.5.7, Figure 74). Format Complete; Channel or
/// Group address.
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct SetKeySignature {
    /// Message Type, 4 bits: 0xD.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Format, Address, Channel, Status Bank and Status: 24 bits.
    #[bits(24, access = RO)]
    header_bits: u32,
    /// Sharps/Flats, 4 bits two's complement. Read it with `sharps_flats()`.
    #[bits(4, access = RO)]
    sharps_flats_bits: u8,
    /// Tonic Note, 4 bits: 0 unknown, 1 to 7 A to G.
    #[bits(4, access = RO)]
    pub tonic_note: u8,
    /// Reserved, 88 bits.
    #[bits(88)]
    __: u128,
}

impl SetKeySignature {
    const VALID: u128 = HEADER | (0xFF << 88);

    /// A Set Key Signature with `sharps_flats` (positive sharps, negative flats, -8 unknown or
    /// non-standard) and `tonic_note` (0 unknown, 1 to 7 A to G). Gives `OutOfRange` for a
    /// count outside -8 to 7 or a Tonic Note of 8 to 15.
    pub const fn try_new(
        group: Group,
        destination: FlexDestination,
        sharps_flats: i8,
        tonic_note: u8,
    ) -> Result<Self, UmpError> {
        if sharps_flats < -8 || sharps_flats > 7 || tonic_note > 7 {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(
                group,
                FlexFormat::Complete,
                destination,
                FlexStatusBank::Setup,
                SetupStatus::SetKeySignature.into_bits(),
            ) | (bits4(sharps_flats) << 92)
                | ((tonic_note as u128) << 88),
        ))
    }

    /// The Sharps/Flats count, -8 to 7.
    pub const fn sharps_flats(&self) -> i8 {
        signed4(self.sharps_flats_bits())
    }

    /// The destination of the message.
    pub const fn destination(&self) -> FlexDestination {
        match FlexDestination::from_packet(self.0) {
            Some(d) => d,
            None => FlexDestination::Group,
        }
    }
}

/// One chord alteration (7.5.8, Figure 75).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ChordAlteration {
    /// Alteration Type: 0 none, 1 add, 2 subtract, 3 raise, 4 lower; 5 to 15 are reserved.
    pub kind: u8,
    /// Degree of the chord that is altered (1 the root, 3 the third), 4 bits.
    pub degree: u8,
}

/// The field values of a Set Chord Name message (7.5.8, Figure 75, Tables 13 to 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ChordName {
    /// Tonic Sharps/Flats: 2 double sharp, 1 sharp, 0 natural, -1 flat, -2 double flat.
    pub tonic_sharps_flats: i8,
    /// Chord Tonic: 0 unknown (No Chord), 1 to 7 A to G.
    pub chord_tonic: u8,
    /// Chord Type (Table 14): 0x00 No Chord to 0x1B 7 Suspended 4th.
    pub chord_type: u8,
    /// Alterations 1 to 4 of the chord.
    pub alterations: [ChordAlteration; 4],
    /// Bass Sharps/Flats: as the Tonic, or -8 for a Bass Note that is the Chord Tonic.
    pub bass_sharps_flats: i8,
    /// Bass Note: 0 the Chord Tonic, 1 to 7 A to G.
    pub bass_note: u8,
    /// Bass Chord Type: 0x00 No Bass Chord, else as the Chord Type.
    pub bass_chord_type: u8,
    /// Alterations 1 and 2 of the bass chord.
    pub bass_alterations: [ChordAlteration; 2],
}

impl ChordName {
    /// True when no field holds a reserved value (Tables 13 to 15, 7.5.8).
    const fn is_valid(&self) -> bool {
        let mut ok = self.tonic_sharps_flats >= -2
            && self.tonic_sharps_flats <= 2
            && (self.bass_sharps_flats == -8
                || (self.bass_sharps_flats >= -2 && self.bass_sharps_flats <= 2))
            && self.chord_tonic <= 7
            && self.bass_note <= 7
            && self.chord_type <= 0x1B
            && self.bass_chord_type <= 0x1B
            // -8 and Bass Note 0 both mean "same as the Chord Tonic" and go together.
            && (self.bass_sharps_flats == -8) == (self.bass_note == 0);
        let mut i = 0;
        while i < 6 {
            let a = if i < 4 {
                self.alterations[i]
            } else {
                self.bass_alterations[i - 4]
            };
            ok = ok && a.kind <= 4 && a.degree <= 15;
            i += 1;
        }
        ok
    }

    /// Words 1 to 3 of the message (Figure 75).
    const fn into_bits(self) -> u128 {
        const fn alt(a: ChordAlteration) -> u128 {
            ((a.kind as u128) << 4) | a.degree as u128
        }
        (bits4(self.tonic_sharps_flats) << 92)
            | ((self.chord_tonic as u128) << 88)
            | ((self.chord_type as u128) << 80)
            | (alt(self.alterations[0]) << 72)
            | (alt(self.alterations[1]) << 64)
            | (alt(self.alterations[2]) << 56)
            | (alt(self.alterations[3]) << 48)
            | (bits4(self.bass_sharps_flats) << 28)
            | ((self.bass_note as u128) << 24)
            | ((self.bass_chord_type as u128) << 16)
            | (alt(self.bass_alterations[0]) << 8)
            | alt(self.bass_alterations[1])
    }

    const fn from_bits(bits: u128) -> Self {
        const fn alt(byte: u128) -> ChordAlteration {
            ChordAlteration {
                kind: ((byte >> 4) & 0xF) as u8,
                degree: (byte & 0xF) as u8,
            }
        }
        Self {
            tonic_sharps_flats: signed4(((bits >> 92) & 0xF) as u8),
            chord_tonic: ((bits >> 88) & 0xF) as u8,
            chord_type: (bits >> 80) as u8,
            alterations: [
                alt(bits >> 72),
                alt(bits >> 64),
                alt(bits >> 56),
                alt(bits >> 48),
            ],
            bass_sharps_flats: signed4(((bits >> 28) & 0xF) as u8),
            bass_note: ((bits >> 24) & 0xF) as u8,
            bass_chord_type: (bits >> 16) as u8,
            bass_alterations: [alt(bits >> 8), alt(bits)],
        }
    }
}

/// Set Chord Name, Bank 0x00 Status 0x06 (7.5.8, Figure 75). Format Complete; Channel or Group
/// address. Read the fields with [`SetChordName::chord`].
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct SetChordName {
    /// Message Type, 4 bits: 0xD.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Format, Address, Channel, Status Bank and Status: 24 bits.
    #[bits(24, access = RO)]
    header_bits: u32,
    /// Chord fields of words 1 to 3: 96 bits.
    #[bits(96, access = RO)]
    chord_bits: u128,
}

impl SetChordName {
    /// Every bit but the 16 reserved bits of word 2.
    const VALID: u128 = !(0xFFFF << 32);

    /// A Set Chord Name. Gives `OutOfRange` for a reserved value: a Tonic Sharps/Flats outside
    /// -2 to 2, a Bass Sharps/Flats outside -2 to 2 and not -8, a Chord Tonic or Bass Note of
    /// 8 to 15, a Chord Type or Bass Chord Type above 0x1B, an Alteration Type of 5 to 15 or a
    /// Degree above 15. A Bass Sharps/Flats of -8 needs Bass Note 0, and Bass Note 0 needs -8.
    pub const fn try_new(
        group: Group,
        destination: FlexDestination,
        chord: ChordName,
    ) -> Result<Self, UmpError> {
        if !chord.is_valid() {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(
                group,
                FlexFormat::Complete,
                destination,
                FlexStatusBank::Setup,
                SetupStatus::SetChordName.into_bits(),
            ) | chord.into_bits(),
        ))
    }

    /// The chord fields, or `None` when a received field holds a reserved value.
    pub const fn chord(&self) -> Option<ChordName> {
        let chord = ChordName::from_bits(self.chord_bits());
        if chord.is_valid() {
            Some(chord)
        } else {
            None
        }
    }

    /// The destination of the message.
    pub const fn destination(&self) -> FlexDestination {
        match FlexDestination::from_packet(self.0) {
            Some(d) => d,
            None => FlexDestination::Group,
        }
    }
}

macro_rules! setup_words {
    ($($ty:ident => $status:ident, $group_only:expr);* $(;)?) => {$(
        impl UmpWords for $ty {
            const WORDS: usize = 4;

            fn from_words(words: &[u32]) -> Result<Self, UmpError> {
                decode(words, SetupStatus::$status, $group_only, Self::VALID).map(Self)
            }

            fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
                put(&split_u128(self.0), out)
            }
        }
    )*};
}

setup_words! {
    SetTempo => SetTempo, true;
    SetTimeSignature => SetTimeSignature, true;
    SetMetronome => SetMetronome, true;
    SetKeySignature => SetKeySignature, false;
    SetChordName => SetChordName, false;
}

/// Text bytes in one UMP (Figure 77).
const CHUNK: usize = 12;

/// Largest text: 32 UMPs of 12 bytes (7.5.1).
const MAX_TEXT: usize = 32 * CHUNK;

/// One text UMP of the Text Messages Common Format (7.5.9, Figure 77).
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct FlexTextPacket {
    /// Message Type, 4 bits: 0xD.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Format, 2 bits. Read it with `format()`.
    #[bits(2, access = RO)]
    format_bits: FlexFormat,
    /// Address and Channel, 6 bits. Read them with `destination()`.
    #[bits(6, access = RO)]
    destination_bits: u8,
    /// Status Bank and Status, 16 bits. Read them with `text_status()`.
    #[bits(16, access = RO)]
    status_bits: u16,
    /// 12 text bytes, 96 bits.
    #[bits(96, access = RO)]
    data: u128,
}

impl FlexTextPacket {
    fn from_parts(
        group: Group,
        format: FlexFormat,
        destination: FlexDestination,
        status: FlexTextStatus,
        text: &[u8],
    ) -> Self {
        let mut bits = header(group, format, destination, status.bank(), status.status());
        for (i, &b) in text.iter().take(CHUNK).enumerate() {
            bits |= (b as u128) << (88 - 8 * i);
        }
        Self(bits)
    }

    /// Reads a text packet, or `None` for another message, a reserved Address or an undefined
    /// Status Bank or Status. Clears the Channel field of a Group-addressed packet, and the
    /// bytes after the first 0x00 in a Start, End or Complete UMP (7.5.9).
    fn parse(words: &[u32]) -> Option<Self> {
        let bits = join_u128(take::<4>(words).ok()?);
        if (bits >> 124) as u8 != MessageType::FlexData.into_bits() {
            return None;
        }
        FlexTextStatus::from_parts((bits >> 104) as u8, (bits >> 96) as u8)?;
        let mut clean = bits & HEADER;
        if let FlexDestination::Group = FlexDestination::from_packet(bits)? {
            clean &= !(0xF << 112);
        }
        let packet = Self(clean);
        let continue_ = matches!(packet.format(), FlexFormat::Continue);
        let mut i = 0;
        while i < CHUNK {
            let b = (bits >> (88 - 8 * i)) as u8;
            if b == 0 && !continue_ {
                break;
            }
            clean |= (b as u128) << (88 - 8 * i);
            i += 1;
        }
        Some(Self(clean))
    }

    /// Data byte `i` (0 to 11).
    const fn byte(&self, i: usize) -> u8 {
        (self.data() >> (88 - 8 * i)) as u8
    }

    /// The Format of the packet.
    pub const fn format(&self) -> FlexFormat {
        self.format_bits()
    }

    /// The Address and Channel of the packet.
    pub const fn destination(&self) -> FlexDestination {
        match FlexDestination::from_packet(self.0) {
            Some(d) => d,
            None => FlexDestination::Group,
        }
    }

    /// The Status Bank and Status of the packet.
    pub const fn text_status(&self) -> FlexTextStatus {
        let s = self.status_bits();
        match FlexTextStatus::from_parts((s >> 8) as u8, s as u8) {
            Some(t) => t,
            None => FlexTextStatus::Metadata(MetadataTextStatus::Unknown),
        }
    }

    /// The text bytes up to the first 0x00 byte.
    pub fn text(&self) -> FlexTextBytes {
        let mut bytes = [0u8; CHUNK];
        let mut len = 0;
        while len < CHUNK && self.byte(len) != 0 {
            bytes[len] = self.byte(len);
            len += 1;
        }
        FlexTextBytes {
            bytes,
            len: len as u8,
        }
    }

    /// True for a Lyrics or Ruby Complete UMP filled with 0x00: a melisma (7.5.10.1, 7.5.12.1).
    pub fn is_melisma(&self) -> bool {
        matches!(self.format(), FlexFormat::Complete)
            && self.text_status().has_melisma()
            && self.data() == 0
    }

    /// True when this is a Lyrics or Ruby Continue UMP that holds 0x00, which marks a melisma
    /// (7.5.10, 7.5.12).
    fn continue_melisma(&self) -> bool {
        matches!(self.format(), FlexFormat::Continue)
            && self.text_status().has_melisma()
            && (0..CHUNK).any(|i| self.byte(i) == 0)
    }
}

impl UmpWords for FlexTextPacket {
    const WORDS: usize = 4;

    fn from_words(words: &[u32]) -> Result<Self, UmpError> {
        take::<4>(words)?;
        Self::parse(words).ok_or(UmpError::UnexpectedMessage)
    }

    fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
        put(&split_u128(self.0), out)
    }
}

/// The 0 to 12 text bytes of one [`FlexTextPacket`]. Derefs to a byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlexTextBytes {
    bytes: [u8; CHUNK],
    len: u8,
}

impl Deref for FlexTextBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

/// `n` ASCII digits at the start of `s`, with their value.
fn digits(s: &[u8], n: usize) -> Option<(u32, &[u8])> {
    let (head, rest) = (s.get(..n)?, &s[n..]);
    let mut v = 0;
    for &b in head {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (b - b'0') as u32;
    }
    Some((v, rest))
}

/// True for an ISO 8601 calendar date (`YYYY-MM-DD` or `YYYYMMDD`) with an optional time
/// (`Thh:mm[:ss[.f]]` or `Thhmm[ss[.f]]`) and an optional zone (`Z`, `+hh[:mm]`, `-hh[:mm]`)
/// (7.5.9.2).
fn is_iso8601_date(text: &str) -> bool {
    fn date(s: &[u8]) -> Option<(bool, &[u8])> {
        let (_, s) = digits(s, 4)?;
        let extended = s.first() == Some(&b'-');
        let s = if extended { &s[1..] } else { s };
        let (month, s) = digits(s, 2)?;
        let s = if extended { s.strip_prefix(b"-")? } else { s };
        let (day, s) = digits(s, 2)?;
        ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some((extended, s))
    }
    fn time(s: &[u8], extended: bool) -> Option<&[u8]> {
        let (hour, s) = digits(s, 2)?;
        let s = if extended { s.strip_prefix(b":")? } else { s };
        let (minute, mut s) = digits(s, 2)?;
        if hour > 24 || minute > 59 {
            return None;
        }
        let colon = if extended {
            s.strip_prefix(b":")
        } else {
            Some(s)
        };
        if let Some((second, rest)) = colon.and_then(|c| digits(c, 2)) {
            if second > 60 {
                return None;
            }
            s = rest;
            if let Some(f) = s.strip_prefix(b".").or_else(|| s.strip_prefix(b",")) {
                let n = f.iter().take_while(|b| b.is_ascii_digit()).count();
                if n == 0 {
                    return None;
                }
                s = &f[n..];
            }
        }
        match s.first() {
            None => Some(s),
            Some(b'Z') => Some(&s[1..]),
            Some(b'+' | b'-') => {
                let (zh, z) = digits(&s[1..], 2)?;
                let z = if extended {
                    z.strip_prefix(b":").unwrap_or(z)
                } else {
                    z
                };
                let z = match digits(z, 2) {
                    Some((zm, rest)) if zm <= 59 => rest,
                    Some(_) => return None,
                    None => z,
                };
                (zh <= 14).then_some(z)
            }
            Some(_) => None,
        }
    }
    let Some((extended, rest)) = date(text.as_bytes()) else {
        return false;
    };
    match rest.first() {
        None => true,
        Some(b'T') => time(&rest[1..], extended).is_some_and(<[u8]>::is_empty),
        Some(_) => false,
    }
}

/// True for a BCP 47 language tag shape: ASCII letters, digits and single hyphens between
/// subtags (7.5.11).
fn is_language_tag(text: &str) -> bool {
    !text.is_empty()
        && text
            .split('-')
            .all(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Iterator over the text UMPs of one Flex Data text message, in send order (7.5.1, 7.5.9).
///
/// Text of 12 bytes or fewer gives one Complete packet. Longer text gives Start, zero or more
/// Continue and End packets, with 12 bytes in each packet but the last, which is padded with
/// 0x00.
#[derive(Debug, Clone)]
pub struct FlexTextPackets<'a> {
    group: Group,
    destination: FlexDestination,
    status: FlexTextStatus,
    rest: &'a [u8],
    started: bool,
    done: bool,
}

impl<'a> FlexTextPackets<'a> {
    /// The Project Name packets of `text` on `group`, to the Address Group (7.5.9.1,
    /// REQ-FLEX-011). Use [`Self::try_new`] for another Address.
    pub fn project_name(group: Group, text: &'a str) -> Result<Self, UmpError> {
        let status = FlexTextStatus::Metadata(MetadataTextStatus::ProjectName);
        Self::try_new(group, FlexDestination::Group, status, text)
    }

    /// The MIDI Clip Name packets of `text` on `group`, to the Address Group (7.5.9.1,
    /// REQ-FLEX-011). Use [`Self::try_new`] for another Address.
    pub fn midi_clip_name(group: Group, text: &'a str) -> Result<Self, UmpError> {
        let status = FlexTextStatus::Metadata(MetadataTextStatus::MidiClipName);
        Self::try_new(group, FlexDestination::Group, status, text)
    }

    /// The packets of `text` with `status` to `destination` on `group`.
    ///
    /// Gives `OutOfRange` for text that is empty, longer than 384 bytes (32 UMPs, 7.5.1), or
    /// holds a Byte Order Mark (U+FEFF, 7.5.9) or a 0x00 byte (it would end the text, and
    /// 0x00 must not occur in a Start UMP, 7.5.10). A Recording/Concert Date must be an ISO
    /// 8601 calendar date with an optional time (7.5.9.2). A Lyrics Language or Ruby Language
    /// must be a BCP 47 tag of ASCII letters, digits and hyphens (7.5.11, 7.5.13).
    ///
    /// The caller gives NFC text (1.8, DEC-UMP-005). Project Name and MIDI Clip Name should go
    /// to the Group (Table 16).
    pub fn try_new(
        group: Group,
        destination: FlexDestination,
        status: FlexTextStatus,
        text: &'a str,
    ) -> Result<Self, UmpError> {
        let valid = match status {
            FlexTextStatus::Metadata(MetadataTextStatus::RecordingDate) => is_iso8601_date(text),
            FlexTextStatus::Performance(
                PerformanceTextStatus::LyricsLanguage | PerformanceTextStatus::RubyLanguage,
            ) => is_language_tag(text),
            _ => true,
        };
        if !valid || text.is_empty() || text.len() > MAX_TEXT || text.contains(['\0', '\u{FEFF}']) {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self {
            group,
            destination,
            status,
            rest: text.as_bytes(),
            started: false,
            done: false,
        })
    }

    /// A melisma: one Lyrics or Ruby Complete UMP filled with 0x00 (7.5.10.1, 7.5.12.1). Gives
    /// `OutOfRange` for another status.
    pub fn melisma(
        group: Group,
        destination: FlexDestination,
        status: PerformanceTextStatus,
    ) -> Result<Self, UmpError> {
        let status = FlexTextStatus::Performance(status);
        if !status.has_melisma() {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self {
            group,
            destination,
            status,
            rest: &[],
            started: false,
            done: false,
        })
    }
}

impl Iterator for FlexTextPackets<'_> {
    type Item = FlexTextPacket;

    fn next(&mut self) -> Option<FlexTextPacket> {
        if self.done {
            return None;
        }
        let last = self.rest.len() <= CHUNK;
        let format = match (self.started, last) {
            (false, true) => FlexFormat::Complete,
            (false, false) => FlexFormat::Start,
            (true, false) => FlexFormat::Continue,
            (true, true) => FlexFormat::End,
        };
        let (head, rest) = self.rest.split_at(self.rest.len().min(CHUNK));
        self.rest = rest;
        self.started = true;
        self.done = last;
        Some(FlexTextPacket::from_parts(
            self.group,
            format,
            self.destination,
            self.status,
            head,
        ))
    }
}

/// A complete text from [`FlexTextReassembler`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlexText<'a> {
    status: FlexTextStatus,
    destination: FlexDestination,
    bytes: &'a [u8],
    melisma: bool,
}

impl<'a> FlexText<'a> {
    /// The Status Bank and Status of the text.
    pub const fn text_status(&self) -> FlexTextStatus {
        self.status
    }

    /// The destination of the text.
    pub const fn destination(&self) -> FlexDestination {
        self.destination
    }

    /// The text bytes, without 0x00 bytes.
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The text, or `None` when the bytes are not valid UTF-8.
    pub fn as_str(&self) -> Option<&'a str> {
        core::str::from_utf8(self.bytes).ok()
    }

    /// True when a Lyrics or Ruby Continue UMP of the text held 0x00, which marks a melisma
    /// (7.5.10, 7.5.12).
    pub const fn has_melisma(&self) -> bool {
        self.melisma
    }
}

/// Result of [`FlexTextReassembler::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexTextEvent<'a> {
    /// A complete text.
    Complete(FlexText<'a>),
    /// A Lyrics or Ruby Complete UMP filled with 0x00: the previous syllable continues
    /// (7.5.10.1, 7.5.12.1).
    Melisma {
        /// Lyrics or Ruby.
        status: FlexTextStatus,
        /// The destination of the melisma.
        destination: FlexDestination,
    },
    /// A text ended before its End UMP; its bytes are discarded.
    Incomplete {
        /// Text bytes received before the text ended.
        received: usize,
    },
    /// A text that ended normally but was longer than the capacity; its bytes are discarded.
    Overflow {
        /// Full text length.
        length: usize,
    },
}

/// Reassembles the Flex Data text messages of one Group into a fixed buffer of `N` bytes
/// (7.5.1, 7.5.9). A capacity of 384 holds the largest text. Needs no alloc.
#[derive(Debug, Clone)]
pub struct FlexTextReassembler<const N: usize> {
    group: Group,
    buffer: [u8; N],
    length: usize,
    current: Option<(FlexTextStatus, FlexDestination)>,
    melisma: bool,
}

impl<const N: usize> FlexTextReassembler<N> {
    /// An empty reassembler for the text messages of `group`.
    pub const fn new(group: Group) -> Self {
        Self {
            group,
            buffer: [0; N],
            length: 0,
            current: None,
            melisma: false,
        }
    }

    /// Clears the reassembler, so it holds no partial text.
    pub fn reset(&mut self) {
        self.length = 0;
        self.current = None;
        self.melisma = false;
    }

    /// Takes the words of one packet and gives an event when a text ends.
    ///
    /// - Packets that are not Flex Data text UMPs of the Group give no event, so System Real
    ///   Time, JR Clock and other messages may come between the UMPs of a text (7.5.2).
    /// - A Start begins a text and a Continue extends it. An End or Complete gives the text, or
    ///   [`FlexTextEvent::Overflow`] when it is longer than `N`. A Lyrics or Ruby Complete UMP
    ///   filled with 0x00 gives [`FlexTextEvent::Melisma`].
    /// - A Start or Complete before the End of the current text ends it with
    ///   [`FlexTextEvent::Incomplete`]. A Start also begins a new text; a Complete is not
    ///   decoded, and the caller pushes it again.
    /// - A Continue or End with another status or destination ends the current text with
    ///   `Incomplete`. A Continue or End with no Start gives no event.
    ///
    /// Never panics.
    pub fn push(&mut self, words: &[u32]) -> Option<FlexTextEvent<'_>> {
        let packet = FlexTextPacket::parse(words)?;
        if packet.group() != self.group {
            return None;
        }
        let id = (packet.text_status(), packet.destination());
        match (packet.format(), self.current) {
            (FlexFormat::Complete, None) => {
                if packet.is_melisma() {
                    return Some(FlexTextEvent::Melisma {
                        status: id.0,
                        destination: id.1,
                    });
                }
                self.length = 0;
                self.melisma = false;
                self.append(&packet);
                Some(self.finish(id))
            }
            (FlexFormat::Complete, Some(_)) => self.interrupt(),
            (FlexFormat::Start, current) => {
                let event = current.map(|_| FlexTextEvent::Incomplete {
                    received: self.length,
                });
                self.length = 0;
                self.melisma = false;
                self.current = Some(id);
                self.append(&packet);
                event
            }
            (FlexFormat::Continue | FlexFormat::End, None) => None,
            (format, Some(current)) => {
                if current != id {
                    return self.interrupt();
                }
                self.melisma |= packet.continue_melisma();
                self.append(&packet);
                if matches!(format, FlexFormat::End) {
                    self.current = None;
                    Some(self.finish(id))
                } else {
                    None
                }
            }
        }
    }

    fn interrupt(&mut self) -> Option<FlexTextEvent<'_>> {
        self.current = None;
        Some(FlexTextEvent::Incomplete {
            received: self.length,
        })
    }

    fn append(&mut self, packet: &FlexTextPacket) {
        for i in 0..CHUNK {
            let b = packet.byte(i);
            if b == 0 {
                continue;
            }
            if let Some(slot) = self.buffer.get_mut(self.length) {
                *slot = b;
            }
            self.length = self.length.saturating_add(1);
        }
    }

    fn finish(&self, id: (FlexTextStatus, FlexDestination)) -> FlexTextEvent<'_> {
        if self.length > N {
            return FlexTextEvent::Overflow {
                length: self.length,
            };
        }
        FlexTextEvent::Complete(FlexText {
            status: id.0,
            destination: id.1,
            bytes: &self.buffer[..self.length],
            melisma: self.melisma,
        })
    }
}

/// One Flex Data message (7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlexDataMessage {
    /// Set Tempo (0x00/0x00).
    SetTempo(SetTempo),
    /// Set Time Signature (0x00/0x01).
    SetTimeSignature(SetTimeSignature),
    /// Set Metronome (0x00/0x02).
    SetMetronome(SetMetronome),
    /// Set Key Signature (0x00/0x05).
    SetKeySignature(SetKeySignature),
    /// Set Chord Name (0x00/0x06).
    SetChordName(SetChordName),
    /// One text UMP of Status Bank 0x01 or 0x02.
    Text(FlexTextPacket),
}

impl FlexDataMessage {
    /// Decodes one Flex Data packet (four words) by its Status Bank and Status. Gives `None`
    /// for short input, a Message Type other than 0xD, a reserved Address, an undefined Status
    /// Bank or Status, a Setup message whose Format is not Complete, or a Set Tempo, Set Time
    /// Signature or Set Metronome that is not Group-addressed. Reserved bits are cleared.
    /// Never panics.
    pub fn parse(words: &[u32]) -> Option<Self> {
        let w0 = *words.first()?;
        if w0 >> 28 != MessageType::FlexData.into_bits() as u32 {
            return None;
        }
        match FlexStatusBank::from_bits((w0 >> 8) as u8)? {
            FlexStatusBank::Setup => Some(match SetupStatus::from_bits(w0 as u8)? {
                SetupStatus::SetTempo => Self::SetTempo(SetTempo::from_words(words).ok()?),
                SetupStatus::SetTimeSignature => {
                    Self::SetTimeSignature(SetTimeSignature::from_words(words).ok()?)
                }
                SetupStatus::SetMetronome => {
                    Self::SetMetronome(SetMetronome::from_words(words).ok()?)
                }
                SetupStatus::SetKeySignature => {
                    Self::SetKeySignature(SetKeySignature::from_words(words).ok()?)
                }
                SetupStatus::SetChordName => {
                    Self::SetChordName(SetChordName::from_words(words).ok()?)
                }
            }),
            _ => FlexTextPacket::parse(words).map(Self::Text),
        }
    }

    /// Words 0 to 3 of the packet.
    pub(crate) const fn into_words(self) -> [u32; 4] {
        split_u128(match self {
            Self::SetTempo(m) => m.0,
            Self::SetTimeSignature(m) => m.0,
            Self::SetMetronome(m) => m.0,
            Self::SetKeySignature(m) => m.0,
            Self::SetChordName(m) => m.0,
            Self::Text(m) => m.0,
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::string::String;
    use std::vec::Vec;

    use super::*;
    use proptest::prelude::*;

    fn words<T: UmpWords>(m: &T) -> [u32; 4] {
        let mut out = [0u32; 4];
        m.to_words(&mut out).unwrap();
        out
    }

    fn group(n: u8) -> Group {
        Group::new(n).unwrap()
    }

    fn channel(n: u8) -> FlexDestination {
        FlexDestination::Channel(Channel::new(n).unwrap())
    }

    fn packet_words(packets: FlexTextPackets) -> Vec<[u32; 4]> {
        packets.map(|p| words(&p)).collect()
    }

    /// Pushes every packet and gives the texts of the Complete events.
    fn push_texts<const N: usize>(
        r: &mut FlexTextReassembler<N>,
        packets: &[[u32; 4]],
    ) -> Vec<String> {
        let mut out = Vec::new();
        for p in packets {
            if let Some(FlexTextEvent::Complete(t)) = r.push(p) {
                out.push(String::from(t.as_str().unwrap()));
            }
        }
        out
    }

    const COPYRIGHT: FlexTextStatus = FlexTextStatus::Metadata(MetadataTextStatus::CopyrightNotice);
    const LYRICS: FlexTextStatus = FlexTextStatus::Performance(PerformanceTextStatus::Lyrics);

    /// Figure 69, Tables 9 to 11: Format, Address and Status Bank values.
    #[test]
    fn flex_format_address_and_bank_values() {
        for (bits, f) in [
            (0, FlexFormat::Complete),
            (1, FlexFormat::Start),
            (2, FlexFormat::Continue),
            (3, FlexFormat::End),
        ] {
            assert_eq!(f.into_bits(), bits);
            assert_eq!(FlexFormat::from_bits(bits), f);
        }
        assert_eq!(FlexAddress::from_bits(0), Some(FlexAddress::Channel));
        assert_eq!(FlexAddress::from_bits(1), Some(FlexAddress::Group));
        assert_eq!(FlexAddress::Group.into_bits(), 1);
        assert_eq!(FlexAddress::from_bits(2), None);
        assert_eq!(FlexAddress::from_bits(3), None);
        assert_eq!(FlexStatusBank::from_bits(0), Some(FlexStatusBank::Setup));
        assert_eq!(
            FlexStatusBank::from_bits(2).map(FlexStatusBank::into_bits),
            Some(2)
        );
        assert!((3..=0xFF).all(|b| FlexStatusBank::from_bits(b).is_none()));
        for s in [0x00, 0x01, 0x02, 0x05, 0x06] {
            assert_eq!(
                SetupStatus::from_bits(s).map(SetupStatus::into_bits),
                Some(s)
            );
        }
        assert!([0x03, 0x04, 0x07, 0xFF]
            .iter()
            .all(|&s| SetupStatus::from_bits(s).is_none()));
    }

    /// Table 16: Status values of Status Banks 0x01 and 0x02; the rest are reserved.
    #[test]
    fn flex_text_status_values_match_table_16() {
        for s in 0x00..=0x0C {
            assert_eq!(MetadataTextStatus::from_bits(s).unwrap().into_bits(), s);
        }
        assert_eq!(
            MetadataTextStatus::from_bits(0x0B),
            Some(MetadataTextStatus::RecordingDate)
        );
        assert!((0x0D..=0xFF).all(|s| MetadataTextStatus::from_bits(s).is_none()));
        for s in 0x00..=0x04 {
            assert_eq!(PerformanceTextStatus::from_bits(s).unwrap().into_bits(), s);
        }
        // REQ-FLEX-010: each value has its name in Table 16.
        for (b, name) in [
            (0x00, PerformanceTextStatus::Unknown),
            (0x01, PerformanceTextStatus::Lyrics),
            (0x02, PerformanceTextStatus::LyricsLanguage),
            (0x03, PerformanceTextStatus::Ruby),
            (0x04, PerformanceTextStatus::RubyLanguage),
        ] {
            assert_eq!(PerformanceTextStatus::from_bits(b), Some(name));
        }
        assert!((0x05..=0xFF).all(|s| PerformanceTextStatus::from_bits(s).is_none()));
        // A packet with a reserved text Status is not decoded.
        assert_eq!(
            FlexDataMessage::parse(&[0xD010_010D, 0x4100_0000, 0, 0]),
            None
        );
        assert_eq!(
            FlexDataMessage::parse(&[0xD010_0205, 0x4100_0000, 0, 0]),
            None
        );
        assert_eq!(
            FlexDataMessage::parse(&[0xD010_0301, 0x4100_0000, 0, 0]),
            None
        );
    }

    /// Figure 70: Set Tempo, 120 bpm is 50 000 000 units of 10 ns.
    #[test]
    fn set_tempo_matches_figure_70() {
        let m = SetTempo::new(group(0), 50_000_000);
        assert_eq!(words(&m), [0xD010_0000, 0x02FA_F080, 0, 0]);
        assert_eq!(m.message_type(), MessageType::FlexData);
        assert_eq!(m.ten_ns_per_quarter_note(), 50_000_000);
        let m = SetTempo::new(group(5), u32::MAX);
        assert_eq!(words(&m), [0xD510_0000, 0xFFFF_FFFF, 0, 0]);
        assert_eq!(m.group(), group(5));
        assert_eq!(SetTempo::from_words(&words(&m)), Ok(m));
    }

    /// Figure 71: Set Time Signature 4/4 with eight 1/32 notes in 24 MIDI Clocks.
    #[test]
    fn set_time_signature_matches_figure_71() {
        let m = SetTimeSignature::try_new(group(0), 4, 2, 8).unwrap();
        assert_eq!(words(&m), [0xD010_0001, 0x0402_0800, 0, 0]);
        assert_eq!(
            (m.numerator(), m.denominator(), m.thirty_second_notes()),
            (4, 2, 8)
        );
        // 0 is a non-standard denominator.
        let m = SetTimeSignature::try_new(group(1), 255, 0, 8).unwrap();
        assert_eq!(words(&m), [0xD110_0001, 0xFF00_0800, 0, 0]);
        // 7.5.4 gives 1 to 256; the 8-bit field has no code for 256 or 0.
        assert_eq!(
            SetTimeSignature::try_new(group(0), 0, 2, 8),
            Err(UmpError::OutOfRange)
        );
    }

    /// Figure 73: the four example Set Metronome messages.
    #[test]
    fn set_metronome_matches_figure_73() {
        // Clocks, Bar Accents, Subdivision Clicks, beats in the bar, words.
        type Case = (u8, [u8; 3], [u8; 2], u16, [u32; 4]);
        let cases: [Case; 4] = [
            // 4/4, accent every 4th beat, no subdivisions.
            (24, [4, 0, 0], [0, 0], 4, [0xD010_0002, 0x1804_0000, 0, 0]),
            // 6/8, dotted quarter clicks, accent every 2nd beat, 1/8 triplets.
            (
                36,
                [2, 0, 0],
                [3, 0],
                2,
                [0xD010_0002, 0x2402_0000, 0x0300_0000, 0],
            ),
            // 5/4, 3 + 2 accent, 1/8 notes.
            (
                24,
                [3, 2, 0],
                [2, 0],
                5,
                [0xD010_0002, 0x1803_0200, 0x0200_0000, 0],
            ),
            // 3/4, accent every 3rd beat, 1/8 notes and 1/8 triplets.
            (
                24,
                [3, 0, 0],
                [2, 3],
                3,
                [0xD010_0002, 0x1803_0000, 0x0203_0000, 0],
            ),
        ];
        for (clocks, accents, subdivisions, beats, expected) in cases {
            let m = SetMetronome::try_new(group(0), clocks, accents, subdivisions, beats).unwrap();
            assert_eq!(words(&m), expected);
            assert_eq!(m.clocks_per_primary_click(), clocks);
            assert_eq!(
                [
                    m.bar_accent_part1(),
                    m.bar_accent_part2(),
                    m.bar_accent_part3()
                ],
                accents
            );
            assert_eq!(
                [m.subdivision_clicks1(), m.subdivision_clicks2()],
                subdivisions
            );
            assert_eq!(SetMetronome::from_words(&expected), Ok(m));
        }
    }

    /// 7.5.5: the Bar Accent parts sum to the beats in the bar; three fields allow at most
    /// three parts.
    #[test]
    fn set_metronome_rejects_bad_bar_accents() {
        let bad = [
            ([4, 0, 0], 3),
            ([3, 3, 2], 9),
            ([0, 0, 0], 4),
            ([255, 255, 255], 764),
        ];
        for (accents, beats) in bad {
            assert_eq!(
                SetMetronome::try_new(group(0), 24, accents, [0, 0], beats),
                Err(UmpError::OutOfRange)
            );
        }
        // 9/4 as 3 + 3 + 3 and as 4 + 4 + 1 (7.5.5).
        assert!(SetMetronome::try_new(group(0), 24, [3, 3, 3], [0, 0], 9).is_ok());
        assert!(SetMetronome::try_new(group(0), 24, [4, 4, 1], [0, 0], 9).is_ok());
        assert!(SetMetronome::try_new(group(0), 24, [255, 255, 255], [0, 0], 765).is_ok());
    }

    /// Figure 74, Table 12: Set Key Signature with Channel or Group address.
    #[test]
    fn set_key_signature_matches_figure_74() {
        // One sharp, Tonic D, to Channel 4 of Group 2.
        let m = SetKeySignature::try_new(group(2), channel(3), 1, 4).unwrap();
        assert_eq!(words(&m), [0xD203_0005, 0x1400_0000, 0, 0]);
        assert_eq!((m.sharps_flats(), m.tonic_note()), (1, 4));
        assert_eq!(m.destination(), channel(3));
        // Four flats, Tonic D (D flat), to the Group.
        let m = SetKeySignature::try_new(group(0), FlexDestination::Group, -4, 4).unwrap();
        assert_eq!(words(&m), [0xD010_0005, 0xC400_0000, 0, 0]);
        assert_eq!(m.sharps_flats(), -4);
        assert_eq!(m.destination(), FlexDestination::Group);
        // -8 (binary 1000) is unknown or non-standard.
        let m = SetKeySignature::try_new(group(0), FlexDestination::Group, -8, 0).unwrap();
        assert_eq!(words(&m)[1], 0x8000_0000);
        assert_eq!(m.sharps_flats(), -8);
        for (sf, tonic) in [(8, 0), (-9, 0), (0, 8), (0, 15)] {
            assert_eq!(
                SetKeySignature::try_new(group(0), FlexDestination::Group, sf, tonic),
                Err(UmpError::OutOfRange)
            );
        }
    }

    fn chord(tonic_sf: i8, tonic: u8, kind: u8, bass_sf: i8, bass: u8) -> ChordName {
        ChordName {
            tonic_sharps_flats: tonic_sf,
            chord_tonic: tonic,
            chord_type: kind,
            bass_sharps_flats: bass_sf,
            bass_note: bass,
            ..ChordName::default()
        }
    }

    /// Figure 76: the three example Set Chord Name messages.
    #[test]
    fn set_chord_name_matches_figure_76() {
        // B flat minor.
        let b_flat_min = chord(-1, 2, 0x07, -8, 0);
        // D major over F sharp.
        let d_maj_f_sharp = chord(0, 4, 0x01, 1, 6);
        // C major 7th, sharp 11th.
        let mut c_maj7_sharp11 = chord(0, 3, 0x03, -8, 0);
        c_maj7_sharp11.alterations[0] = ChordAlteration {
            kind: 3,
            degree: 11,
        };
        let cases = [
            (b_flat_min, [0xD010_0006, 0xF207_0000, 0, 0x8000_0000]),
            (d_maj_f_sharp, [0xD010_0006, 0x0401_0000, 0, 0x1600_0000]),
            (c_maj7_sharp11, [0xD010_0006, 0x0303_3B00, 0, 0x8000_0000]),
        ];
        for (c, expected) in cases {
            let m = SetChordName::try_new(group(0), FlexDestination::Group, c).unwrap();
            assert_eq!(words(&m), expected);
            assert_eq!(m.chord(), Some(c));
            assert_eq!(SetChordName::from_words(&expected), Ok(m));
        }
        // Every field position, to Channel 16.
        let full = ChordName {
            tonic_sharps_flats: 2,
            chord_tonic: 7,
            chord_type: 0x1B,
            alterations: [
                ChordAlteration { kind: 1, degree: 2 },
                ChordAlteration { kind: 2, degree: 3 },
                ChordAlteration { kind: 3, degree: 4 },
                ChordAlteration {
                    kind: 4,
                    degree: 15,
                },
            ],
            bass_sharps_flats: -2,
            bass_note: 5,
            bass_chord_type: 0x01,
            bass_alterations: [
                ChordAlteration { kind: 1, degree: 9 },
                ChordAlteration { kind: 2, degree: 5 },
            ],
        };
        let m = SetChordName::try_new(group(3), channel(15), full).unwrap();
        assert_eq!(
            words(&m),
            [0xD30F_0006, 0x271B_1223, 0x344F_0000, 0xE501_1925]
        );
        assert_eq!(m.chord(), Some(full));
        assert_eq!(m.destination(), channel(15));
    }

    /// Tables 13 to 15 and 7.5.8: reserved chord values are rejected on write and give no
    /// chord on read.
    #[test]
    fn set_chord_name_rejects_reserved_values() {
        let ok = chord(0, 3, 0x01, -8, 0);
        let mut bad = [ok; 12];
        bad[0].tonic_sharps_flats = 3;
        bad[1].tonic_sharps_flats = -3;
        bad[2].bass_sharps_flats = -7;
        bad[3].chord_tonic = 8;
        bad[4].chord_type = 0x1C;
        bad[5].bass_chord_type = 0xFF;
        bad[6].alterations[2].kind = 5;
        bad[7].bass_alterations[1].degree = 16;
        bad[8].bass_note = 8;
        // -8 needs Bass Note 0, and Bass Note 0 needs -8.
        bad[9] = chord(0, 3, 0x01, -8, 2);
        bad[10] = chord(0, 3, 0x01, 0, 0);
        bad[11].alterations[0].degree = 16;
        for c in bad {
            assert_eq!(
                SetChordName::try_new(group(0), FlexDestination::Group, c),
                Err(UmpError::OutOfRange),
                "{c:?}"
            );
        }
        let received = SetChordName::from_words(&[0xD010_0006, 0x081C_0000, 0, 0]).unwrap();
        assert_eq!(received.chord(), None);
    }

    /// Figure 78: the two example Copyright Notice messages, written and read.
    #[test]
    fn flex_text_matches_figure_78() {
        let packets = packet_words(
            FlexTextPackets::try_new(group(0), FlexDestination::Group, COPYRIGHT, "©2022 AMEI")
                .unwrap(),
        );
        assert_eq!(
            packets,
            [[0xD010_0104, 0xC2A9_3230, 0x3232_2041, 0x4D45_4900]]
        );
        let text = "Copyright 2022 MIDI Association Publishing";
        let packets = packet_words(
            FlexTextPackets::try_new(group(0), FlexDestination::Group, COPYRIGHT, text).unwrap(),
        );
        assert_eq!(
            packets,
            [
                [0xD050_0104, 0x436F_7079, 0x7269_6768, 0x7420_3230],
                [0xD090_0104, 0x3232_204D, 0x4944_4920, 0x4173_736F],
                [0xD090_0104, 0x6369_6174, 0x696F_6E20, 0x5075_626C],
                [0xD0D0_0104, 0x6973_6869, 0x6E67_0000, 0x0000_0000],
            ]
        );
        let p = FlexTextPacket::from_words(&packets[3]).unwrap();
        assert_eq!(p.format(), FlexFormat::End);
        assert_eq!(p.text_status(), COPYRIGHT);
        assert_eq!(p.destination(), FlexDestination::Group);
        assert_eq!(&*p.text(), b"ishing");
        let mut r = FlexTextReassembler::<384>::new(group(0));
        assert_eq!(push_texts(&mut r, &packets), [text]);
        let mut events = Vec::new();
        let complete = [0xD010_0104, 0xC2A9_3230, 0x3232_2041, 0x4D45_4900];
        if let Some(FlexTextEvent::Complete(t)) = r.push(&complete) {
            assert_eq!(t.text_status(), COPYRIGHT);
            assert_eq!(t.destination(), FlexDestination::Group);
            assert_eq!(t.as_bytes(), "©2022 AMEI".as_bytes());
            assert!(!t.has_melisma());
            events.push(());
        }
        assert_eq!(events.len(), 1);
    }

    /// 7.5.1, 7.5.9, 1.8: at most 32 UMPs; no empty text, Byte Order Mark or 0x00 byte.
    #[test]
    fn flex_text_writer_rejects_invalid_text() {
        let max = "a".repeat(384);
        let packets = packet_words(
            FlexTextPackets::try_new(group(0), FlexDestination::Group, COPYRIGHT, &max).unwrap(),
        );
        assert_eq!(packets.len(), 32);
        assert_eq!(packets[0][0] >> 22 & 0b11, 1);
        assert!(packets[1..31].iter().all(|p| p[0] >> 22 & 0b11 == 2));
        assert_eq!(packets[31][0] >> 22 & 0b11, 3);
        // Exactly 12 bytes is one Complete UMP; 13 bytes is a Start and an End.
        let twelve = packet_words(
            FlexTextPackets::try_new(group(0), FlexDestination::Group, COPYRIGHT, "abcdefghijkl")
                .unwrap(),
        );
        assert_eq!(twelve.len(), 1);
        assert_eq!(twelve[0][0] >> 22 & 0b11, 0);
        let thirteen = packet_words(
            FlexTextPackets::try_new(group(0), FlexDestination::Group, COPYRIGHT, "abcdefghijklm")
                .unwrap(),
        );
        assert_eq!(thirteen.len(), 2);
        assert_eq!(thirteen[1], [0xD0D0_0104, 0x6D00_0000, 0, 0]);
        let too_long = "a".repeat(385);
        for text in [
            "",
            too_long.as_str(),
            "\u{FEFF}Name",
            "Na\u{FEFF}me",
            "a\0b",
        ] {
            assert_eq!(
                FlexTextPackets::try_new(group(0), FlexDestination::Group, COPYRIGHT, text)
                    .map(|_| ()),
                Err(UmpError::OutOfRange)
            );
        }
    }

    /// 1.8, DEC-UMP-005: the writer sends the caller's UTF-8 bytes unchanged and does not
    /// normalize them; the reader gives them back as a string.
    #[test]
    fn flex_text_writes_caller_text_unchanged() {
        let nfc = "Caf\u{E9} \u{30AC}";
        let nfd = "Cafe\u{301}";
        let mut r = FlexTextReassembler::<384>::new(group(0));
        for text in [nfc, nfd] {
            let packets = packet_words(
                FlexTextPackets::try_new(group(0), FlexDestination::Group, COPYRIGHT, text)
                    .unwrap(),
            );
            assert_eq!(push_texts(&mut r, &packets), [text]);
        }
        let p = FlexTextPackets::try_new(group(0), FlexDestination::Group, COPYRIGHT, nfd)
            .unwrap()
            .next()
            .unwrap();
        assert_eq!(&*p.text(), nfd.as_bytes());
    }

    /// Table 16: Project Name and MIDI Clip Name go to the Group (Address 1, Channel 0); the
    /// caller may give a Channel.
    #[test]
    fn flex_text_project_and_clip_name_use_group_address() {
        for status in [
            MetadataTextStatus::ProjectName,
            MetadataTextStatus::MidiClipName,
        ] {
            let status = FlexTextStatus::Metadata(status);
            let p = FlexTextPackets::try_new(group(1), FlexDestination::Group, status, "Song")
                .unwrap()
                .next()
                .unwrap();
            assert_eq!(words(&p)[0] & 0x00FF_FF00, 0x0010_0100);
            let p = FlexTextPackets::try_new(group(1), channel(2), status, "Song")
                .unwrap()
                .next()
                .unwrap();
            assert_eq!(words(&p)[0] & 0x00FF_FF00, 0x0002_0100);
        }
        // The Project Name and MIDI Clip Name writers use the Address Group.
        let first = |p: Result<FlexTextPackets<'_>, UmpError>| words(&p.unwrap().next().unwrap());
        assert_eq!(
            first(FlexTextPackets::project_name(group(1), "Song")),
            [0xD110_0101, 0x536F_6E67, 0, 0]
        );
        assert_eq!(
            first(FlexTextPackets::midi_clip_name(group(1), "Song")),
            [0xD110_0103, 0x536F_6E67, 0, 0]
        );
        // A Group-addressed packet reads with its reserved Channel field cleared.
        let p = FlexTextPacket::from_words(&[0xD01F_0101, 0x4100_0000, 0, 0]).unwrap();
        assert_eq!(words(&p)[0], 0xD010_0101);
    }

    /// 7.5.9.2: Recording/Concert Date text is an ISO 8601 date with an optional time.
    #[test]
    fn flex_recording_date_needs_iso8601() {
        let date = FlexTextStatus::Metadata(MetadataTextStatus::RecordingDate);
        let try_date =
            |text| FlexTextPackets::try_new(group(0), FlexDestination::Group, date, text).is_ok();
        for good in [
            "2023-10-27",
            "20231027",
            "2023-10-27T14:30",
            "2023-10-27T14:30:15Z",
            "2023-10-27T14:30:15.25+09:00",
            "2023-10-27T14:30-05",
            "20231027T143015",
        ] {
            assert!(try_date(good), "{good}");
        }
        for bad in [
            "2023-13-01",
            "2023-10-32",
            "2023-1027",
            "27/10/2023",
            "2023-10-27 14:30",
            "2023-10-27T25:00",
            "2023-10-27T14:61",
            "2023-10-27T14:30:15.",
            "2023-10-27T14:30Q",
            "October 2023",
        ] {
            assert!(!try_date(bad), "{bad}");
        }
        // Other statuses take any text.
        assert!(
            FlexTextPackets::try_new(group(0), FlexDestination::Group, COPYRIGHT, "27/10").is_ok()
        );
    }

    /// 7.5.11, 7.5.13: Lyrics Language and Ruby Language hold a BCP 47 tag in ASCII.
    #[test]
    fn flex_language_tags_are_bcp47_ascii() {
        for (status, byte) in [
            (PerformanceTextStatus::LyricsLanguage, 0x02),
            (PerformanceTextStatus::RubyLanguage, 0x04),
        ] {
            let status = FlexTextStatus::Performance(status);
            let try_tag = |text| {
                FlexTextPackets::try_new(group(0), channel(0), status, text).map(packet_words)
            };
            assert_eq!(
                try_tag("ja-JP"),
                Ok(std::vec![[0xD000_0200 | byte, 0x6A61_2D4A, 0x5000_0000, 0]])
            );
            assert!(try_tag("en").is_ok());
            assert!(try_tag("zh-Hant-TW").is_ok());
            for bad in ["en_US", "en--US", "-en", "en-", "\u{65E5}\u{672C}", "en US"] {
                assert_eq!(try_tag(bad), Err(UmpError::OutOfRange), "{bad}");
            }
        }
    }

    /// 7.5.10, 7.5.12: one syllable per message; Space ends a word, CR a line and CR LF a
    /// paragraph; a Start UMP never holds 0x00.
    #[test]
    fn flex_lyrics_and_ruby_syllables() {
        let ruby = FlexTextStatus::Performance(PerformanceTextStatus::Ruby);
        let syllables = ["Hel", "lo ", "world\r", "sixty-four\r\n"];
        let mut r = FlexTextReassembler::<384>::new(group(0));
        for status in [LYRICS, ruby] {
            for s in syllables {
                let packets = packet_words(
                    FlexTextPackets::try_new(group(0), channel(0), status, s).unwrap(),
                );
                assert_eq!(packets.len(), 1);
                assert_eq!(push_texts(&mut r, &packets), [s]);
            }
            // A long syllable spans a Start and an End; the Start has no 0x00 byte.
            let long = "Supercalifragilistic ";
            let packets =
                packet_words(FlexTextPackets::try_new(group(0), channel(0), status, long).unwrap());
            assert_eq!(packets.len(), 2);
            let start = FlexTextPacket::from_words(&packets[0]).unwrap();
            assert_eq!(start.format(), FlexFormat::Start);
            assert_eq!(start.text().len(), 12);
            assert_eq!(push_texts(&mut r, &packets), [long]);
        }
    }

    /// 7.5.10.1, 7.5.12.1: a Lyrics or Ruby Complete UMP filled with 0x00 is a melisma, and
    /// 0x00 in a Continue UMP marks one (7.5.10 calls Continue Format 0x1; it is 0x2).
    #[test]
    fn flex_lyrics_and_ruby_melisma() {
        let mut r = FlexTextReassembler::<384>::new(group(0));
        for (status, byte) in [
            (PerformanceTextStatus::Lyrics, 0x01),
            (PerformanceTextStatus::Ruby, 0x03),
        ] {
            let packets = packet_words(
                FlexTextPackets::melisma(group(0), FlexDestination::Group, status).unwrap(),
            );
            let expected = 0xD010_0200 | byte;
            assert_eq!(packets, [[expected, 0, 0, 0]]);
            let p = FlexTextPacket::from_words(&packets[0]).unwrap();
            assert!(p.is_melisma());
            assert_eq!(
                r.push(&packets[0]),
                Some(FlexTextEvent::Melisma {
                    status: FlexTextStatus::Performance(status),
                    destination: FlexDestination::Group,
                })
            );
            // Start "la", Continue with 0x00, End " ": the text has a melisma.
            let s = expected & !0x00F0_0000;
            let words = [
                [s | 0x0050_0000, 0x6C61_6C61, 0x6C61_6C61, 0x6C61_6C61],
                [s | 0x0090_0000, 0x6C61_0000, 0, 0],
                [s | 0x00D0_0000, 0x2000_0000, 0, 0],
            ];
            assert_eq!(r.push(&words[0]), None);
            assert_eq!(r.push(&words[1]), None);
            let Some(FlexTextEvent::Complete(t)) = r.push(&words[2]) else {
                panic!("End must complete the text")
            };
            assert!(t.has_melisma());
            assert_eq!(t.as_str(), Some("lalalalalalala "));
        }
        // A Copyright Notice filled with 0x00 is not a melisma.
        assert!(!FlexTextPacket::from_words(&[0xD010_0104, 0, 0, 0])
            .unwrap()
            .is_melisma());
        for status in [
            PerformanceTextStatus::Unknown,
            PerformanceTextStatus::LyricsLanguage,
            PerformanceTextStatus::RubyLanguage,
        ] {
            assert_eq!(
                FlexTextPackets::melisma(group(0), FlexDestination::Group, status).map(|_| ()),
                Err(UmpError::OutOfRange)
            );
        }
    }

    /// 7.5.2: other messages may come between the UMPs of a text; a new Start or Complete
    /// ends an unfinished text.
    #[test]
    fn flex_reassembler_interleaving_and_errors() {
        let text = "Copyright 2022 MIDI Association Publishing";
        let packets = packet_words(
            FlexTextPackets::try_new(group(0), FlexDestination::Group, COPYRIGHT, text).unwrap(),
        );
        let mut r = FlexTextReassembler::<384>::new(group(0));
        // Timing Clock, JR Clock and a packet of another Group between the UMPs.
        let mut mixed = Vec::new();
        for p in &packets {
            mixed.push(*p);
            mixed.push([0x10F8_0000, 0, 0, 0]);
            mixed.push([0x0010_1234, 0, 0, 0]);
            mixed.push([0xD150_0104, 0x4141_4141, 0x4141_4141, 0x4141_4141]);
        }
        assert_eq!(push_texts(&mut r, &mixed), [text]);
        // A Start before the End gives Incomplete and begins a new text.
        assert_eq!(r.push(&packets[0]), None);
        assert_eq!(r.push(&packets[1]), None);
        assert_eq!(
            r.push(&packets[0]),
            Some(FlexTextEvent::Incomplete { received: 24 })
        );
        assert_eq!(push_texts(&mut r, &packets[1..]), [text]);
        // A Complete before the End gives Incomplete; pushed again, it gives its text.
        let complete = [0xD010_0104, 0x4142_4300, 0, 0];
        assert_eq!(r.push(&packets[0]), None);
        assert_eq!(
            r.push(&complete),
            Some(FlexTextEvent::Incomplete { received: 12 })
        );
        assert_eq!(push_texts(&mut r, &[complete]), ["ABC"]);
        // A Continue with another status ends the text.
        assert_eq!(r.push(&packets[0]), None);
        assert_eq!(
            r.push(&[0xD090_0105, 0x4100_0000, 0, 0]),
            Some(FlexTextEvent::Incomplete { received: 12 })
        );
        // A Continue or End with no Start gives no event.
        assert_eq!(r.push(&packets[1]), None);
        assert_eq!(r.push(&packets[3]), None);
        // reset discards a partial text.
        assert_eq!(r.push(&packets[0]), None);
        r.reset();
        assert_eq!(r.push(&packets[3]), None);
        // A text longer than the capacity gives Overflow.
        let mut small = FlexTextReassembler::<16>::new(group(0));
        let events: Vec<_> = packets
            .iter()
            .filter_map(|p| small.push(p).map(|e| format_event(&e)))
            .collect();
        assert_eq!(events, ["Overflow 42"]);
    }

    fn format_event(e: &FlexTextEvent) -> String {
        match e {
            FlexTextEvent::Overflow { length } => std::format!("Overflow {length}"),
            other => std::format!("{other:?}"),
        }
    }

    /// 7.5.1, 2.1.3: the parser rejects reserved Address values, a Setup message that is not
    /// Complete, and Set Tempo, Set Time Signature or Set Metronome that is not
    /// Group-addressed; it clears reserved bits.
    #[test]
    fn flex_parse_rejects_and_clears_reserved() {
        for w0 in [
            0xD020_0005u32, // Address 2.
            0xD030_0104,    // Address 3.
            0xD050_0000,    // Set Tempo as a Start UMP.
            0xD000_0000,    // Set Tempo to a Channel.
            0xD003_0001,    // Set Time Signature to a Channel.
            0xD000_0002,    // Set Metronome to a Channel.
            0xD010_0003,    // Undefined Setup Status.
            0xD010_0300,    // Reserved Status Bank.
            0xE010_0000,    // Message Type 0xE.
        ] {
            assert_eq!(FlexDataMessage::parse(&[w0, 0, 0, 0]), None, "{w0:08X}");
        }
        assert_eq!(FlexDataMessage::parse(&[0xD010_0000, 0, 0]), None);
        assert_eq!(
            SetTempo::from_words(&[0xD010_0000, 0, 0]),
            Err(UmpError::Truncated { needed: 4, got: 3 })
        );
        // Key Signature and Chord Name may go to a Channel.
        assert!(matches!(
            FlexDataMessage::parse(&[0xD00F_0005, 0x1400_0000, 0, 0]),
            Some(FlexDataMessage::SetKeySignature(_))
        ));
        let cleared = [
            ([0xD01F_0000, 5, 0xFFFF_FFFF, 1], [0xD010_0000, 5, 0, 0]),
            (
                [0xD010_0001, 0x0402_08FF, 1, 1],
                [0xD010_0001, 0x0402_0800, 0, 0],
            ),
            (
                [0xD010_0002, 0x1804_0000, 0x0000_FFFF, 7],
                [0xD010_0002, 0x1804_0000, 0, 0],
            ),
            (
                [0xD003_0005, 0x14FF_FFFF, 1, 1],
                [0xD003_0005, 0x1400_0000, 0, 0],
            ),
            (
                [0xD010_0006, 0x0401_0000, 0x0000_FFFF, 0x1600_0000],
                [0xD010_0006, 0x0401_0000, 0, 0x1600_0000],
            ),
            // Text bytes after the first 0x00 of a Complete UMP are cleared.
            (
                [0xD01F_0104, 0x4100_4200, 0, 0x4300_0000],
                [0xD010_0104, 0x4100_0000, 0, 0],
            ),
            // A Continue UMP keeps every byte (0x00 there marks a melisma).
            (
                [0xD090_0201, 0x4100_4200, 0, 0x4300_0000],
                [0xD090_0201, 0x4100_4200, 0, 0x4300_0000],
            ),
        ];
        for (input, expected) in cleared {
            let m = FlexDataMessage::parse(&input).unwrap();
            assert_eq!(m.into_words(), expected, "{input:08X?}");
        }
    }

    /// Writers give valid fields and the reader gives them back (DEC-UMP-003).
    #[test]
    fn prop_flex_setup_round_trip() {
        proptest!(|(g in 0u8..16, ch in 0u8..16, tempo in any::<u32>(), num in 1u8..=255,
                    den in any::<u8>(), n32 in any::<u8>(), sf in -8i8..8, tonic in 0u8..8,
                    accents in any::<[u8; 3]>(), subs in any::<[u8; 2]>())| {
            let m = SetTempo::new(group(g), tempo);
            prop_assert_eq!(FlexDataMessage::parse(&words(&m)), Some(FlexDataMessage::SetTempo(m)));
            prop_assert_eq!(m.ten_ns_per_quarter_note(), tempo);
            let m = SetTimeSignature::try_new(group(g), num, den, n32).unwrap();
            prop_assert_eq!(SetTimeSignature::from_words(&words(&m)), Ok(m));
            prop_assert_eq!((m.numerator(), m.denominator(), m.thirty_second_notes()), (num, den, n32));
            let beats = accents.iter().map(|&a| a as u16).sum();
            let m = SetMetronome::try_new(group(g), 24, accents, subs, beats).unwrap();
            prop_assert_eq!(SetMetronome::from_words(&words(&m)), Ok(m));
            let m = SetKeySignature::try_new(group(g), channel(ch), sf, tonic).unwrap();
            prop_assert_eq!(SetKeySignature::from_words(&words(&m)), Ok(m));
            prop_assert_eq!((m.sharps_flats(), m.tonic_note(), m.destination()), (sf, tonic, channel(ch)));
            prop_assert_eq!(m.group(), group(g));
        });
    }

    /// Any text of 1 to 384 bytes without 0x00 or a Byte Order Mark round trips.
    #[test]
    fn prop_flex_text_round_trip() {
        proptest!(|(text in "[^\u{0}\u{FEFF}]{1,96}", g in 0u8..16, ch in 0u8..16)| {
            prop_assume!(text.len() <= 384);
            let packets = packet_words(FlexTextPackets::try_new(group(g), channel(ch), COPYRIGHT, &text).unwrap());
            prop_assert_eq!(packets.len(), text.len().div_ceil(12));
            prop_assert!(packets.iter().all(|p| p[0] >> 28 == 0xD && (p[0] >> 24) as u8 & 0xF == g));
            let mut r = FlexTextReassembler::<384>::new(group(g));
            prop_assert_eq!(push_texts(&mut r, &packets), [text]);
        });
    }

    /// The parser and the reassembler never panic, and a parsed message writes back to words
    /// that parse to the same message.
    #[test]
    fn prop_flex_parse_never_panics_and_round_trips() {
        proptest!(|(w in any::<[u32; 4]>(), bank in 0u32..4, status in 0u32..8)| {
            let w0 = 0xD000_0000 | (w[0] & 0x0FFF_0000) | (bank << 8) | status;
            let words = [w0, w[1], w[2], w[3]];
            if let Some(m) = FlexDataMessage::parse(&words) {
                let out = m.into_words();
                prop_assert_eq!(out[0] >> 8 & 0xFF, bank);
                prop_assert_eq!(FlexDataMessage::parse(&out), Some(m));
            }
            let _ = FlexDataMessage::parse(&w);
            let _ = FlexDataMessage::parse(&w[..2]);
            let mut r = FlexTextReassembler::<8>::new(group(((w0 >> 24) & 0xF) as u8));
            let _ = r.push(&words);
            let _ = r.push(&w);
            let _ = r.push(&[]);
        });
    }
}
