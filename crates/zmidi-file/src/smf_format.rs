//! Standard MIDI File 1.0 byte rules and value types (RP-001, revised February 1996).
//!
//! - A file is a header chunk (`MThd`) and one or more track chunks (`MTrk`). Each chunk is a
//!   4-character type and a 32-bit length, most significant byte first, that does not count the
//!   8 bytes of type and length (Chunks, p. 3).
//! - A variable-length quantity has 7 bits in each byte, most significant bits first; bit 7 is set
//!   in each byte except the last. The largest value is 0x0FFFFFFF, four bytes (Conventions, p. 2).
//! - The division is ticks per quarter-note (bit 15 = 0) or an SMPTE format and ticks per frame
//!   (bit 15 = 1; Header Chunks, p. 4).
//! - Meta-events are `FF <type> <length> <data>`; the types of RP-001 are on pages 8 to 10.
//! - A channel message is also a UMP MIDI 1.0 Channel Voice word (Message Type 0x2, M2-104-UM 7.3,
//!   DEC-021).
//!
//! No alloc (DEC-002). Nothing here panics on any input.

use thiserror::Error;

/// The chunk type of the header chunk (Header Chunks, p. 4).
pub const MTHD: [u8; 4] = *b"MThd";

/// The chunk type of a track chunk (Track Chunks, p. 5).
pub const MTRK: [u8; 4] = *b"MTrk";

/// The largest value of a variable-length quantity (Conventions, p. 2).
pub const MAX_VLQ: u32 = 0x0FFF_FFFF;

/// The tempo when a file has no Set Tempo: 500,000 microseconds per quarter-note, 120 beats per
/// minute (Formats 0, 1, and 2, p. 5; Calculating Delta Times, p. 14).
pub const DEFAULT_TEMPO: u32 = 500_000;

/// Error of the SMF reader and of the decode functions of this module (thiserror 2, no_std).
/// The writer module `smf_writer` has its own error enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
pub enum SmfError {
    /// The data does not start with an `MThd` chunk.
    #[error("not a Standard MIDI File: no MThd chunk")]
    NoHeader,
    /// The `MThd` chunk has less than 6 data bytes.
    #[error("MThd chunk shorter than 6 bytes")]
    BadHeaderLength,
    /// An SMPTE format other than -24, -25, -29 or -30.
    #[error("bad SMPTE format in the division")]
    BadDivision,
    /// A chunk, an event or a variable-length quantity cut by the end of the data.
    #[error("Standard MIDI File truncated")]
    Truncated,
    /// A variable-length quantity with a fifth byte.
    #[error("variable-length quantity longer than four bytes")]
    VlqTooLong,
    /// A data byte where an event starts, with no running status in effect.
    #[error("data byte with no running status")]
    NoRunningStatus,
    /// A status byte that does not start an event (0xF1 to 0xF6, 0xF8 to 0xFE).
    #[error("status byte {0:#04X} does not start an event")]
    UnexpectedStatus(u8),
    /// A data byte of a channel message above 0x7F.
    #[error("data byte above 0x7F")]
    BadDataByte,
    /// A channel message or End of Track while a sysex message in packets is open.
    #[error("event inside a sysex message in packets")]
    SysExInterrupted,
    /// A track chunk with no End of Track.
    #[error("track chunk with no End of Track")]
    NoEndOfTrack,
    /// An event after End of Track in the same track chunk.
    #[error("data after End of Track")]
    DataAfterEndOfTrack,
}

/// Reads the variable-length quantity at the start of `bytes`. Gives the value and the number of
/// bytes (1 to 4). A longer form than necessary (for example `80 00`) is accepted when it has at
/// most four bytes (AMB-RP-001-01).
pub fn read_vlq(bytes: &[u8]) -> Result<(u32, usize), SmfError> {
    let mut value = 0u32;
    for i in 0..4 {
        let b = *bytes.get(i).ok_or(SmfError::Truncated)?;
        value = (value << 7) | u32::from(b & 0x7F);
        if b & 0x80 == 0 {
            return Ok((value, i + 1));
        }
    }
    Err(SmfError::VlqTooLong)
}

/// The shortest variable-length form of `value`: the bytes (first `len` used) and `len`.
/// None for a value above [`MAX_VLQ`].
pub const fn write_vlq(value: u32) -> Option<([u8; 4], usize)> {
    if value > MAX_VLQ {
        return None;
    }
    let mut len = 1;
    while len < 4 && value >> (7 * len) != 0 {
        len += 1;
    }
    let mut out = [0u8; 4];
    let mut i = 0;
    while i < len {
        let shift = 7 * (len - 1 - i);
        let more = if i + 1 < len { 0x80 } else { 0 };
        out[i] = ((value >> shift) as u8 & 0x7F) | more;
        i += 1;
    }
    Some((out, len))
}

/// The `<format>` word of the header chunk (Header Chunks, p. 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileFormat {
    /// 0: one multi-channel track.
    Single,
    /// 1: one or more simultaneous tracks of a sequence.
    MultiTrack,
    /// 2: one or more sequentially independent single-track patterns.
    MultiSequence,
    /// Any other value. The reader accepts it (Formats 0, 1, and 2, p. 5); the writer rejects it.
    Other(u16),
}

impl FileFormat {
    /// The format of a `<format>` word.
    pub const fn from_word(word: u16) -> Self {
        match word {
            0 => Self::Single,
            1 => Self::MultiTrack,
            2 => Self::MultiSequence,
            w => Self::Other(w),
        }
    }

    /// The `<format>` word.
    pub const fn to_word(self) -> u16 {
        match self {
            Self::Single => 0,
            Self::MultiTrack => 1,
            Self::MultiSequence => 2,
            Self::Other(w) => w,
        }
    }
}

/// The SMPTE format of a time-code division: frames per second (Header Chunks, p. 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SmpteFormat {
    /// -24: 24 frames per second.
    Fps24,
    /// -25: 25 frames per second.
    Fps25,
    /// -29: 30 drop frame.
    Fps29,
    /// -30: 30 frames per second.
    Fps30,
}

impl SmpteFormat {
    /// The two's complement value in the high byte of the division.
    pub const fn to_i8(self) -> i8 {
        match self {
            Self::Fps24 => -24,
            Self::Fps25 => -25,
            Self::Fps29 => -29,
            Self::Fps30 => -30,
        }
    }

    /// The format of a high byte read as `i8` (AMB-RP-001-02). None for another value.
    pub const fn from_i8(value: i8) -> Option<Self> {
        match value {
            -24 => Some(Self::Fps24),
            -25 => Some(Self::Fps25),
            -29 => Some(Self::Fps29),
            -30 => Some(Self::Fps30),
            _ => None,
        }
    }
}

/// The `<division>` word of the header chunk: the meaning of the delta-times (Header Chunks, p. 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Division {
    /// Bit 15 = 0: ticks per quarter-note, 15 bits (0 to 0x7FFF).
    TicksPerQuarterNote(u16),
    /// Bit 15 = 1: SMPTE format (high byte) and ticks per frame (low byte, 8 bits).
    Smpte {
        /// Frames per second.
        format: SmpteFormat,
        /// The resolution within a frame.
        ticks_per_frame: u8,
    },
}

impl Division {
    /// The division of a `<division>` word. `BadDivision` for an SMPTE format other than -24,
    /// -25, -29 or -30.
    pub const fn from_word(word: u16) -> Result<Self, SmfError> {
        if word & 0x8000 == 0 {
            return Ok(Self::TicksPerQuarterNote(word));
        }
        match SmpteFormat::from_i8((word >> 8) as u8 as i8) {
            Some(format) => Ok(Self::Smpte {
                format,
                ticks_per_frame: word as u8,
            }),
            None => Err(SmfError::BadDivision),
        }
    }

    /// The `<division>` word. None for ticks per quarter-note above 0x7FFF.
    pub const fn to_word(self) -> Option<u16> {
        match self {
            Self::TicksPerQuarterNote(t) if t > 0x7FFF => None,
            Self::TicksPerQuarterNote(t) => Some(t),
            Self::Smpte {
                format,
                ticks_per_frame,
            } => Some(((format.to_i8() as u8 as u16) << 8) | ticks_per_frame as u16),
        }
    }
}

/// A MIDI 1.0 channel message (Track Chunks, p. 6): status 0x80 to 0xEF and one or two data
/// bytes 0x00 to 0x7F. Program Change (0xC0) and Channel Pressure (0xD0) have one data byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelMessage {
    status: u8,
    data: [u8; 2],
}

impl ChannelMessage {
    /// A checked channel message. `UnexpectedStatus` for a status outside 0x80 to 0xEF,
    /// `BadDataByte` for a used data byte above 0x7F. The second data byte of a message with one
    /// data byte is not used and is kept as 0.
    pub const fn new(status: u8, data1: u8, data2: u8) -> Result<Self, SmfError> {
        if status < 0x80 || status > 0xEF {
            return Err(SmfError::UnexpectedStatus(status));
        }
        let two = Self::data_len_of(status) == 2;
        if data1 > 0x7F || (two && data2 > 0x7F) {
            return Err(SmfError::BadDataByte);
        }
        Ok(Self {
            status,
            data: [data1, if two { data2 } else { 0 }],
        })
    }

    /// The number of data bytes of a channel status (1 for 0xC0 to 0xDF, else 2).
    pub(crate) const fn data_len_of(status: u8) -> usize {
        match status & 0xF0 {
            0xC0 | 0xD0 => 1,
            _ => 2,
        }
    }

    /// The status byte.
    pub const fn status(&self) -> u8 {
        self.status
    }

    /// The channel, 0 to 15.
    pub const fn channel(&self) -> u8 {
        self.status & 0x0F
    }

    /// The number of data bytes, 1 or 2.
    pub const fn data_len(&self) -> usize {
        Self::data_len_of(self.status)
    }

    /// The data bytes (one or two).
    pub fn data(&self) -> &[u8] {
        &self.data[..self.data_len()]
    }

    /// The UMP MIDI 1.0 Channel Voice word (Message Type 0x2, M2-104-UM 7.3) with `group` (its
    /// low 4 bits). The unused byte of a message with one data byte is 0.
    pub const fn to_ump(&self, group: u8) -> u32 {
        0x2000_0000
            | ((group as u32 & 0x0F) << 24)
            | ((self.status as u32) << 16)
            | ((self.data[0] as u32) << 8)
            | self.data[1] as u32
    }

    /// The channel message of a UMP MIDI 1.0 Channel Voice word. None for another Message Type,
    /// a status outside 0x80 to 0xEF or a used data byte above 0x7F. The unused byte of a
    /// message with one data byte is ignored.
    pub const fn from_ump(word: u32) -> Option<Self> {
        if word >> 28 != 0x2 {
            return None;
        }
        match Self::new((word >> 16) as u8, (word >> 8) as u8, word as u8) {
            Ok(m) => Some(m),
            Err(_) => None,
        }
    }
}

/// The text meta-events, types 0x01 to 0x0F (Meta-Events, p. 8 and 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextKind {
    /// 0x01 Text Event.
    Text,
    /// 0x02 Copyright Notice.
    Copyright,
    /// 0x03 Sequence/Track Name.
    TrackName,
    /// 0x04 Instrument Name.
    InstrumentName,
    /// 0x05 Lyric.
    Lyric,
    /// 0x06 Marker.
    Marker,
    /// 0x07 Cue Point.
    CuePoint,
    /// 0x08 to 0x0F: reserved text types.
    Reserved(u8),
}

impl TextKind {
    /// The kind of a meta-event type. None outside 0x01 to 0x0F.
    pub const fn from_byte(kind: u8) -> Option<Self> {
        Some(match kind {
            0x01 => Self::Text,
            0x02 => Self::Copyright,
            0x03 => Self::TrackName,
            0x04 => Self::InstrumentName,
            0x05 => Self::Lyric,
            0x06 => Self::Marker,
            0x07 => Self::CuePoint,
            0x08..=0x0F => Self::Reserved(kind),
            _ => return None,
        })
    }

    /// The meta-event type byte.
    pub const fn to_byte(self) -> u8 {
        match self {
            Self::Text => 0x01,
            Self::Copyright => 0x02,
            Self::TrackName => 0x03,
            Self::InstrumentName => 0x04,
            Self::Lyric => 0x05,
            Self::Marker => 0x06,
            Self::CuePoint => 0x07,
            Self::Reserved(k) => k,
        }
    }
}

/// SMPTE Offset, `FF 54 05 hr mn se fr ff` (Meta-Events, p. 9 and 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SmpteOffset {
    /// The hour byte as in MIDI Time Code: `0rrhhhhh`, rate bits and hours (8 bits).
    pub hour: u8,
    /// Minutes (8 bits).
    pub minute: u8,
    /// Seconds (8 bits).
    pub second: u8,
    /// Frames (8 bits).
    pub frame: u8,
    /// Fractional frames in 100ths of a frame (8 bits).
    pub fraction: u8,
}

impl SmpteOffset {
    /// The hours, the low 5 bits of the hour byte.
    pub const fn hours(&self) -> u8 {
        self.hour & 0x1F
    }

    /// The rate bits `rr` of the hour byte (0 = 24, 1 = 25, 2 = 30 drop, 3 = 30 frames per
    /// second, as in MIDI Time Code).
    pub const fn rate(&self) -> u8 {
        (self.hour >> 5) & 0x03
    }
}

/// Time Signature, `FF 58 04 nn dd cc bb` (Meta-Events, p. 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeSignature {
    /// `nn`: the numerator as notated.
    pub numerator: u8,
    /// `dd`: the denominator as a power of two (2 = quarter-note, 3 = eighth-note).
    pub denominator_power: u8,
    /// `cc`: MIDI clocks in a metronome click.
    pub clocks_per_click: u8,
    /// `bb`: notated 32nd-notes in a MIDI quarter-note (24 MIDI clocks).
    pub thirty_seconds_per_quarter: u8,
}

impl TimeSignature {
    /// The time signature when a file has none: 4/4, 24 MIDI clocks a click, 8 32nd-notes a
    /// quarter-note (Formats 0, 1, and 2, p. 5; example p. 12).
    pub const DEFAULT: Self = Self {
        numerator: 4,
        denominator_power: 2,
        clocks_per_click: 24,
        thirty_seconds_per_quarter: 8,
    };
}

/// Key Signature, `FF 59 02 sf mi` (Meta-Events, p. 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeySignature {
    /// `sf`: -7 (7 flats) to 7 (7 sharps), 0 = key of C.
    pub sharps_flats: i8,
    /// `mi`: false = major key (0), true = minor key (1).
    pub minor: bool,
}

/// The manufacturer ID at the start of a Sequencer-Specific meta-event (Meta-Events, p. 10):
/// one byte, or three bytes when the first byte is 0x00.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManufacturerId {
    /// A one-byte ID (not 0x00).
    One(u8),
    /// A three-byte ID; the first byte is 0x00.
    Three([u8; 3]),
}

impl ManufacturerId {
    /// The number of ID bytes, 1 or 3. An ID is never empty, so there is no `is_empty`.
    #[allow(clippy::len_without_is_empty)]
    pub const fn len(&self) -> usize {
        match self {
            Self::One(_) => 1,
            Self::Three(_) => 3,
        }
    }

    const fn is_valid(&self) -> bool {
        match self {
            Self::One(b) => *b != 0,
            Self::Three(b) => b[0] == 0,
        }
    }
}

/// True for the meta-event types that RP-001 defines: 0x00, 0x01 to 0x0F, 0x20, 0x2F, 0x51,
/// 0x54, 0x58, 0x59 and 0x7F.
pub const fn is_known_meta_kind(kind: u8) -> bool {
    matches!(
        kind,
        0x00..=0x0F | 0x20 | 0x2F | 0x51 | 0x54 | 0x58 | 0x59 | 0x7F
    )
}

/// A meta-event of RP-001 with its value (Meta-Events, p. 8 to 10). Text and data borrow the
/// file bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetaEvent<'a> {
    /// `FF 00 02 ssss`: Sequence Number.
    SequenceNumber(u16),
    /// `FF 01..0F len text`: a text event.
    Text {
        /// The text type.
        kind: TextKind,
        /// The text bytes (printable ASCII for interchange; the caller handles other codes).
        text: &'a [u8],
    },
    /// `FF 20 01 cc`: MIDI Channel Prefix, channel 0 to 15.
    ChannelPrefix(u8),
    /// `FF 2F 00`: End of Track.
    EndOfTrack,
    /// `FF 51 03 tttttt`: Set Tempo, microseconds per quarter-note (24 bits).
    SetTempo(u32),
    /// `FF 54 05 hr mn se fr ff`: SMPTE Offset.
    SmpteOffset(SmpteOffset),
    /// `FF 58 04 nn dd cc bb`: Time Signature.
    TimeSignature(TimeSignature),
    /// `FF 59 02 sf mi`: Key Signature.
    KeySignature(KeySignature),
    /// `FF 7F len data`: Sequencer-Specific, with its manufacturer ID and the bytes after it.
    SequencerSpecific {
        /// The manufacturer ID.
        manufacturer: ManufacturerId,
        /// The bytes after the ID.
        data: &'a [u8],
    },
}

impl<'a> MetaEvent<'a> {
    /// The value of a meta-event of type `kind` with `data`. None for an unknown type, for data
    /// shorter than defined, and for a value out of range (REQ-SMF-META-008). When the data is
    /// longer than defined, only the defined bytes are used (Track Chunks, p. 7).
    pub fn decode(kind: u8, data: &'a [u8]) -> Option<Self> {
        let event = match (kind, data) {
            (0x00, [a, b, ..]) => Self::SequenceNumber(u16::from_be_bytes([*a, *b])),
            (0x01..=0x0F, text) => Self::Text {
                kind: TextKind::from_byte(kind)?,
                text,
            },
            (0x20, [c, ..]) => Self::ChannelPrefix(*c),
            (0x2F, _) => Self::EndOfTrack,
            (0x51, [a, b, c, ..]) => Self::SetTempo(u32::from_be_bytes([0, *a, *b, *c])),
            (0x54, [hour, minute, second, frame, fraction, ..]) => Self::SmpteOffset(SmpteOffset {
                hour: *hour,
                minute: *minute,
                second: *second,
                frame: *frame,
                fraction: *fraction,
            }),
            (0x58, [nn, dd, cc, bb, ..]) => Self::TimeSignature(TimeSignature {
                numerator: *nn,
                denominator_power: *dd,
                clocks_per_click: *cc,
                thirty_seconds_per_quarter: *bb,
            }),
            (0x59, [sf, mi, ..]) if *mi <= 1 => Self::KeySignature(KeySignature {
                sharps_flats: *sf as i8,
                minor: *mi == 1,
            }),
            (0x7F, [0, b, c, rest @ ..]) => Self::SequencerSpecific {
                manufacturer: ManufacturerId::Three([0, *b, *c]),
                data: rest,
            },
            (0x7F, [id, rest @ ..]) if *id != 0 => Self::SequencerSpecific {
                manufacturer: ManufacturerId::One(*id),
                data: rest,
            },
            _ => return None,
        };
        event.is_valid().then_some(event)
    }

    /// The meta-event type byte.
    pub const fn kind(&self) -> u8 {
        match self {
            Self::SequenceNumber(_) => 0x00,
            Self::Text { kind, .. } => kind.to_byte(),
            Self::ChannelPrefix(_) => 0x20,
            Self::EndOfTrack => 0x2F,
            Self::SetTempo(_) => 0x51,
            Self::SmpteOffset(_) => 0x54,
            Self::TimeSignature(_) => 0x58,
            Self::KeySignature(_) => 0x59,
            Self::SequencerSpecific { .. } => 0x7F,
        }
    }

    /// The number of data bytes the writer writes: the defined length, or the text and data
    /// length of a text or Sequencer-Specific event.
    pub const fn data_len(&self) -> usize {
        match self {
            Self::SequenceNumber(_) => 2,
            Self::Text { text, .. } => text.len(),
            Self::ChannelPrefix(_) => 1,
            Self::EndOfTrack => 0,
            Self::SetTempo(_) => 3,
            Self::SmpteOffset(_) => 5,
            Self::TimeSignature(_) => 4,
            Self::KeySignature(_) => 2,
            Self::SequencerSpecific { manufacturer, data } => manufacturer.len() + data.len(),
        }
    }

    /// True when each value is in range: text type 0x01 to 0x0F, MIDI Channel Prefix 0 to 15,
    /// Set Tempo at most 24 bits, Time Signature denominator power at most 31, Key Signature
    /// `sf` -7 to 7, a manufacturer ID of one nonzero byte or of three bytes that start with 0x00.
    pub const fn is_valid(&self) -> bool {
        match self {
            Self::Text { kind, .. } => matches!(kind.to_byte(), 0x01..=0x0F),
            Self::ChannelPrefix(c) => *c <= 15,
            Self::SetTempo(t) => *t <= 0x00FF_FFFF,
            Self::TimeSignature(ts) => ts.denominator_power <= 31,
            Self::KeySignature(ks) => ks.sharps_flats >= -7 && ks.sharps_flats <= 7,
            Self::SequencerSpecific { manufacturer, .. } => manufacturer.is_valid(),
            _ => true,
        }
    }

    /// Writes the data bytes (not `FF`, the type or the length) into `out` and gives their
    /// number. None when `out` is shorter than [`MetaEvent::data_len`]. The values are written as
    /// they are; the writer checks [`MetaEvent::is_valid`] first.
    pub fn write_data(&self, out: &mut [u8]) -> Option<usize> {
        let len = self.data_len();
        let out = out.get_mut(..len)?;
        match self {
            Self::SequenceNumber(n) => out.copy_from_slice(&n.to_be_bytes()),
            Self::Text { text, .. } => out.copy_from_slice(text),
            Self::ChannelPrefix(c) => out[0] = *c,
            Self::EndOfTrack => {}
            Self::SetTempo(t) => out.copy_from_slice(&t.to_be_bytes()[1..]),
            Self::SmpteOffset(o) => {
                out.copy_from_slice(&[o.hour, o.minute, o.second, o.frame, o.fraction])
            }
            Self::TimeSignature(ts) => out.copy_from_slice(&[
                ts.numerator,
                ts.denominator_power,
                ts.clocks_per_click,
                ts.thirty_seconds_per_quarter,
            ]),
            Self::KeySignature(ks) => out.copy_from_slice(&[ks.sharps_flats as u8, ks.minor as u8]),
            Self::SequencerSpecific { manufacturer, data } => {
                let n = manufacturer.len();
                match manufacturer {
                    ManufacturerId::One(b) => out[0] = *b,
                    ManufacturerId::Three(b) => out[..3].copy_from_slice(b),
                }
                out[n..].copy_from_slice(data);
            }
        }
        Some(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn values_at_their_limits_and_each_name() {
        // Ticks per quarter-note: 15 bits.
        assert_eq!(
            Division::TicksPerQuarterNote(0x7FFF).to_word(),
            Some(0x7FFF)
        );
        assert_eq!(Division::TicksPerQuarterNote(0x8000).to_word(), None);
        // The text kinds 0x01 to 0x07 (Meta-Events, p. 8 to 9).
        for (b, k) in [
            (0x01, TextKind::Text),
            (0x02, TextKind::Copyright),
            (0x03, TextKind::TrackName),
            (0x04, TextKind::InstrumentName),
            (0x05, TextKind::Lyric),
            (0x06, TextKind::Marker),
            (0x07, TextKind::CuePoint),
        ] {
            assert_eq!(TextKind::from_byte(b), Some(k), "{b:#04x}");
        }
        // The hour byte 0rrhhhhh: rate 1 (25 frames per second), 11 hours.
        let o = SmpteOffset {
            hour: 0x2B,
            minute: 0,
            second: 0,
            frame: 0,
            fraction: 0,
        };
        assert_eq!((o.hours(), o.rate()), (11, 1));
        // A one-byte manufacturer ID of 0 is not valid (0 starts the three-byte form).
        let bad = MetaEvent::SequencerSpecific {
            manufacturer: ManufacturerId::One(0),
            data: &[],
        };
        assert!(!bad.is_valid());
        let three = MetaEvent::SequencerSpecific {
            manufacturer: ManufacturerId::Three([1, 2, 3]),
            data: &[],
        };
        assert!(!three.is_valid());
    }

    /// Conventions (p. 2): the table of variable-length quantities.
    const VLQ_TABLE: [(u32, &[u8]); 12] = [
        (0x0000_0000, &[0x00]),
        (0x0000_0040, &[0x40]),
        (0x0000_007F, &[0x7F]),
        (0x0000_0080, &[0x81, 0x00]),
        (0x0000_2000, &[0xC0, 0x00]),
        (0x0000_3FFF, &[0xFF, 0x7F]),
        (0x0000_4000, &[0x81, 0x80, 0x00]),
        (0x0010_0000, &[0xC0, 0x80, 0x00]),
        (0x001F_FFFF, &[0xFF, 0xFF, 0x7F]),
        (0x0020_0000, &[0x81, 0x80, 0x80, 0x00]),
        (0x0800_0000, &[0xC0, 0x80, 0x80, 0x00]),
        (0x0FFF_FFFF, &[0xFF, 0xFF, 0xFF, 0x7F]),
    ];

    #[test]
    fn vlq_table_page_2_round_trip() {
        for (value, bytes) in VLQ_TABLE {
            let (out, len) = write_vlq(value).unwrap();
            assert_eq!(&out[..len], bytes, "write {value:#X}");
            assert_eq!(read_vlq(bytes), Ok((value, bytes.len())), "read {value:#X}");
        }
    }

    #[test]
    fn vlq_reader_accepts_long_form_rejects_fifth_byte() {
        assert_eq!(read_vlq(&[0x80, 0x00]), Ok((0, 2)));
        assert_eq!(read_vlq(&[0x80, 0x80, 0x80, 0x7F, 0x55]), Ok((0x7F, 4)));
        assert_eq!(
            read_vlq(&[0x81, 0x80, 0x80, 0x80, 0x00]),
            Err(SmfError::VlqTooLong)
        );
        assert_eq!(read_vlq(&[0x81, 0x80]), Err(SmfError::Truncated));
        assert_eq!(read_vlq(&[]), Err(SmfError::Truncated));
    }

    #[test]
    fn vlq_writer_rejects_above_max() {
        assert_eq!(write_vlq(MAX_VLQ + 1), None);
        assert_eq!(write_vlq(u32::MAX), None);
        assert!(write_vlq(MAX_VLQ).is_some());
    }

    #[test]
    fn prop_vlq_round_trip() {
        proptest!(|(v in 0..=MAX_VLQ)| {
            let (out, len) = write_vlq(v).unwrap();
            prop_assert_eq!(read_vlq(&out[..len]), Ok((v, len)));
            // Shortest form: no leading 0x80 byte.
            prop_assert!(len == 1 || out[0] != 0x80);
        });
    }

    #[test]
    fn division_metrical_and_smpte() {
        assert_eq!(
            Division::from_word(0x0060),
            Ok(Division::TicksPerQuarterNote(96))
        );
        assert_eq!(Division::TicksPerQuarterNote(96).to_word(), Some(0x0060));
        assert_eq!(Division::TicksPerQuarterNote(0x8000).to_word(), None);
        // 25 frames per second, 40 units a frame: milliseconds (p. 4).
        let ms = Division::Smpte {
            format: SmpteFormat::Fps25,
            ticks_per_frame: 40,
        };
        assert_eq!(ms.to_word(), Some(0xE728));
        assert_eq!(Division::from_word(0xE728), Ok(ms));
        for f in [
            SmpteFormat::Fps24,
            SmpteFormat::Fps25,
            SmpteFormat::Fps29,
            SmpteFormat::Fps30,
        ] {
            assert_eq!(SmpteFormat::from_i8(f.to_i8()), Some(f));
        }
        // The high byte of each SMPTE format: -24, -25, -29, -30 in two's complement (p. 4).
        for (f, high) in [
            (SmpteFormat::Fps24, 0xE8),
            (SmpteFormat::Fps25, 0xE7),
            (SmpteFormat::Fps29, 0xE3),
            (SmpteFormat::Fps30, 0xE2),
        ] {
            let d = Division::Smpte {
                format: f,
                ticks_per_frame: 4,
            };
            assert_eq!(d.to_word(), Some(high << 8 | 4), "{f:?}");
            assert_eq!(Division::from_word(high << 8 | 4), Ok(d), "{f:?}");
        }
    }

    #[test]
    fn division_e250_example() {
        // Bit resolution of thirty-frame time code (p. 4).
        let d = Division::Smpte {
            format: SmpteFormat::Fps30,
            ticks_per_frame: 80,
        };
        assert_eq!(Division::from_word(0xE250), Ok(d));
        assert_eq!(d.to_word(), Some(0xE250));
    }

    #[test]
    fn division_rejects_bad_smpte_format() {
        assert_eq!(Division::from_word(0xE650), Err(SmfError::BadDivision)); // -26
        assert_eq!(Division::from_word(0x8000), Err(SmfError::BadDivision));
    }

    #[test]
    fn default_tempo_and_time_signature() {
        assert_eq!(DEFAULT_TEMPO, 500_000);
        // Page 14: 6144 ticks at 96 ticks a quarter-note and 500,000 us = 32,000 ms.
        assert_eq!(6144u64 * u64::from(DEFAULT_TEMPO) / 96 / 1000, 32_000);
        assert_eq!(
            TimeSignature::DEFAULT,
            TimeSignature {
                numerator: 4,
                denominator_power: 2,
                clocks_per_click: 24,
                thirty_seconds_per_quarter: 8
            }
        );
    }

    #[test]
    fn channel_message_to_ump_word() {
        let m = ChannelMessage::new(0x92, 48, 96).unwrap();
        assert_eq!(m.to_ump(0), 0x2092_3060);
        assert_eq!(m.to_ump(0x15), 0x2592_3060);
        assert_eq!(m.channel(), 2);
        assert_eq!(m.data(), &[48, 96]);
        let pc = ChannelMessage::new(0xC1, 46, 0x7F).unwrap();
        assert_eq!(pc.data(), &[46]);
        assert_eq!(pc.to_ump(3), 0x23C1_2E00);
        assert_eq!(ChannelMessage::from_ump(0x23C1_2E55), Some(pc));
        assert_eq!(ChannelMessage::from_ump(0x4092_3060), None);
        assert_eq!(ChannelMessage::from_ump(0x20F2_3060), None);
        assert_eq!(ChannelMessage::from_ump(0x2092_8060), None);
        assert_eq!(
            ChannelMessage::new(0xF1, 0, 0),
            Err(SmfError::UnexpectedStatus(0xF1))
        );
        assert_eq!(
            ChannelMessage::new(0x90, 0x80, 0),
            Err(SmfError::BadDataByte)
        );
    }

    #[test]
    fn prop_channel_message_ump_round_trip() {
        proptest!(|(s in 0x80u8..=0xEF, d1 in 0u8..=0x7F, d2 in 0u8..=0x7F, g in 0u8..16)| {
            let m = ChannelMessage::new(s, d1, d2).unwrap();
            let w = m.to_ump(g);
            prop_assert_eq!((w >> 24) & 0x0F, u32::from(g));
            prop_assert_eq!(ChannelMessage::from_ump(w), Some(m));
        });
    }

    #[test]
    fn meta_events_round_trip() {
        let events = [
            MetaEvent::SequenceNumber(0x1234),
            MetaEvent::Text {
                kind: TextKind::TrackName,
                text: b"Piano",
            },
            MetaEvent::Text {
                kind: TextKind::Reserved(0x0F),
                text: b"",
            },
            MetaEvent::ChannelPrefix(15),
            MetaEvent::EndOfTrack,
            MetaEvent::SetTempo(500_000),
            MetaEvent::SmpteOffset(SmpteOffset {
                hour: 0x61,
                minute: 2,
                second: 3,
                frame: 4,
                fraction: 99,
            }),
            MetaEvent::TimeSignature(TimeSignature::DEFAULT),
            MetaEvent::KeySignature(KeySignature {
                sharps_flats: -7,
                minor: true,
            }),
            MetaEvent::SequencerSpecific {
                manufacturer: ManufacturerId::Three([0, 0x21, 0x09]),
                data: &[1, 2],
            },
            MetaEvent::SequencerSpecific {
                manufacturer: ManufacturerId::One(0x43),
                data: &[],
            },
        ];
        for e in events {
            let mut buf = [0u8; 16];
            let n = e.write_data(&mut buf).unwrap();
            assert_eq!(n, e.data_len());
            assert!(is_known_meta_kind(e.kind()));
            assert_eq!(MetaEvent::decode(e.kind(), &buf[..n]), Some(e), "{e:?}");
        }
        // The type and data bytes of each meta-event (p. 8 to 10); Set Tempo 500,000 = 07 A1 20
        // (p. 12).
        let text = |kind| MetaEvent::Text { kind, text: b"ab" };
        let bytes: [(MetaEvent<'_>, u8, &[u8]); 15] = [
            (MetaEvent::SequenceNumber(0x1234), 0x00, &[0x12, 0x34]),
            (text(TextKind::Text), 0x01, b"ab"),
            (text(TextKind::Copyright), 0x02, b"ab"),
            (text(TextKind::TrackName), 0x03, b"ab"),
            (text(TextKind::InstrumentName), 0x04, b"ab"),
            (text(TextKind::Lyric), 0x05, b"ab"),
            (text(TextKind::Marker), 0x06, b"ab"),
            (text(TextKind::CuePoint), 0x07, b"ab"),
            (MetaEvent::ChannelPrefix(15), 0x20, &[0x0F]),
            (MetaEvent::SetTempo(500_000), 0x51, &[0x07, 0xA1, 0x20]),
            (
                MetaEvent::SmpteOffset(SmpteOffset {
                    hour: 0x61,
                    minute: 2,
                    second: 3,
                    frame: 4,
                    fraction: 99,
                }),
                0x54,
                &[0x61, 2, 3, 4, 99],
            ),
            (
                MetaEvent::TimeSignature(TimeSignature::DEFAULT),
                0x58,
                &[4, 2, 24, 8],
            ),
            (
                MetaEvent::KeySignature(KeySignature {
                    sharps_flats: -7,
                    minor: true,
                }),
                0x59,
                &[0xF9, 0x01],
            ),
            (
                MetaEvent::SequencerSpecific {
                    manufacturer: ManufacturerId::Three([0, 0x21, 0x09]),
                    data: &[1, 2],
                },
                0x7F,
                &[0x00, 0x21, 0x09, 1, 2],
            ),
            (
                MetaEvent::SequencerSpecific {
                    manufacturer: ManufacturerId::One(0x43),
                    data: &[],
                },
                0x7F,
                &[0x43],
            ),
        ];
        for (e, kind, data) in bytes {
            let mut buf = [0u8; 16];
            let n = e.write_data(&mut buf).unwrap();
            assert_eq!((e.kind(), &buf[..n]), (kind, data), "{e:?}");
        }
        // Key Signature major with sf 7 (07 00), and the reserved text type 0x08: written and read.
        let major = MetaEvent::KeySignature(KeySignature {
            sharps_flats: 7,
            minor: false,
        });
        let mut buf = [0u8; 4];
        assert_eq!(major.write_data(&mut buf), Some(2));
        assert_eq!(buf[..2], [0x07, 0x00]);
        assert_eq!(MetaEvent::decode(0x59, &[0x07, 0x00]), Some(major));
        let reserved = MetaEvent::Text {
            kind: TextKind::Reserved(0x08),
            text: b"x",
        };
        assert_eq!(reserved.kind(), 0x08);
        assert_eq!(MetaEvent::decode(0x08, b"x"), Some(reserved));
        let mut short = [0u8; 2];
        assert_eq!(MetaEvent::SetTempo(1).write_data(&mut short), None);
        let s = MetaEvent::SmpteOffset(SmpteOffset {
            hour: 0x61,
            minute: 0,
            second: 0,
            frame: 0,
            fraction: 0,
        });
        if let MetaEvent::SmpteOffset(o) = s {
            assert_eq!((o.rate(), o.hours()), (3, 1));
        }
    }

    #[test]
    fn time_signature_6_8_example() {
        // FF 58 04 06 03 24 08: 6/8, 36 MIDI clocks a click, 8 32nd-notes (p. 10).
        assert_eq!(
            MetaEvent::decode(0x58, &[0x06, 0x03, 0x24, 0x08]),
            Some(MetaEvent::TimeSignature(TimeSignature {
                numerator: 6,
                denominator_power: 3,
                clocks_per_click: 36,
                thirty_seconds_per_quarter: 8
            }))
        );
    }

    #[test]
    fn reader_decodes_longer_known_meta() {
        assert_eq!(
            MetaEvent::decode(0x51, &[0x07, 0xA1, 0x20, 0xAA, 0xBB]),
            Some(MetaEvent::SetTempo(500_000))
        );
        assert_eq!(MetaEvent::decode(0x2F, &[1]), Some(MetaEvent::EndOfTrack));
    }

    #[test]
    fn meta_decode_rejects_short_or_out_of_range() {
        // The limits are accepted: sf 7 and -7, denominator power 31.
        assert!(MetaEvent::decode(0x59, &[7, 0]).is_some());
        assert!(MetaEvent::decode(0x59, &[0xF9, 1]).is_some());
        assert!(MetaEvent::decode(0x58, &[4, 31, 24, 8]).is_some());
        assert_eq!(MetaEvent::decode(0x51, &[0x07, 0xA1]), None);
        assert_eq!(MetaEvent::decode(0x00, &[1]), None);
        assert_eq!(MetaEvent::decode(0x20, &[16]), None);
        assert_eq!(MetaEvent::decode(0x59, &[8, 0]), None);
        assert_eq!(MetaEvent::decode(0x59, &[0xF9, 2]), None);
        assert_eq!(MetaEvent::decode(0x58, &[4, 32, 24, 8]), None);
        assert_eq!(MetaEvent::decode(0x7F, &[0, 1]), None);
        assert_eq!(MetaEvent::decode(0x7F, &[]), None);
        assert_eq!(MetaEvent::decode(0x21, &[0]), None); // unknown type
        assert!(!is_known_meta_kind(0x21));
        assert!(!MetaEvent::SetTempo(0x0100_0000).is_valid());
        assert!(!MetaEvent::Text {
            kind: TextKind::Reserved(0x10),
            text: b""
        }
        .is_valid());
    }

    #[test]
    fn prop_meta_decode_never_panics() {
        proptest!(|(kind in any::<u8>(), data in proptest::collection::vec(any::<u8>(), 0..12))| {
            if let Some(e) = MetaEvent::decode(kind, &data) {
                prop_assert_eq!(e.kind(), kind);
                prop_assert!(e.is_valid());
            }
        });
    }
}
