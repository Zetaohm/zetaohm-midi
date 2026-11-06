//! MIDI Machine Control, the transport tier (RP-013 v1.0; DEC-022). Feature `mmc-transport`.
//!
//! - The command message `F0 7F <device ID> 06 <commands> F7` and the response message
//!   `F0 7F <device ID> 07 <responses> F7` (p. 1; REQ-MMC-001). The string is read lazily with
//!   [`Commands`] and [`InfoFields`]; a writer rejects a string longer than [`MAX_STRING`]
//!   (AMB-RP-013-03). A response gives its source Device ID as received; a Controlled Device ID is
//!   00H to 7EH, so the caller treats a source 7FH as not valid (AMB-RP-013-04).
//! - The length rule of Message Length (p. 5; REQ-MMC-002) for every name, known or not: commands
//!   01H to 3FH and 78H to 7FH have no data, 40H to 77H a count byte; Information Fields 01H to
//!   1FH have 5 data bytes, 20H to 3FH 2, 40H to 77H a count byte, 78H to 7FH none. One or two
//!   00H prefixes extend a name, and its last byte gives the length.
//! - The commands of Guideline Minimum Set 1 and PLAY and PAUSE (p. 14; AMB-RP-013-02;
//!   REQ-MMC-003), the Standard Time Code and its short form (pp. 8, 9; REQ-MMC-004), the MOVE
//!   form conversion (p. 32; REQ-MMC-005) and the Information Fields SELECTED TIME CODE and GP0 /
//!   LOCATE POINT (pp. 44, 47; REQ-MMC-006).
//! - A reader keeps the raw counts with a validity check, and ignores the reserved bits; a writer
//!   rejects a value out of range and writes the reserved bits as 0 (AMB-RP-013-07,
//!   AMB-RP-013-21).
//!
//! Segmentation, the other sets and a responder are the later part `mmc`. No alloc (DEC-002).
//! Never panics.

use crate::mtc::MtcTime;
use crate::universal::{write_universal, Out, RealTimeId, SysexError, Universal};

/// Sub-ID#1 06H: MMC command.
const COMMAND: u8 = RealTimeId::MmcCommand.into_bits();
/// Sub-ID#1 07H: MMC response.
const RESPONSE: u8 = RealTimeId::MmcResponse.into_bits();

/// The largest command or response string of one message, in bytes (p. 1; REQ-MMC-001).
pub const MAX_STRING: usize = 48;

/// The command names of the transport tier (p. 14; REQ-MMC-003).
pub mod command {
    /// 01H STOP.
    pub const STOP: u8 = 0x01;
    /// 02H PLAY.
    pub const PLAY: u8 = 0x02;
    /// 03H DEFERRED PLAY.
    pub const DEFERRED_PLAY: u8 = 0x03;
    /// 04H FAST FORWARD.
    pub const FAST_FORWARD: u8 = 0x04;
    /// 05H REWIND.
    pub const REWIND: u8 = 0x05;
    /// 06H RECORD STROBE.
    pub const RECORD_STROBE: u8 = 0x06;
    /// 07H RECORD EXIT.
    pub const RECORD_EXIT: u8 = 0x07;
    /// 09H PAUSE.
    pub const PAUSE: u8 = 0x09;
    /// 0DH MMC RESET.
    pub const MMC_RESET: u8 = 0x0D;
    /// 40H WRITE.
    pub const WRITE: u8 = 0x40;
    /// 44H LOCATE.
    pub const LOCATE: u8 = 0x44;
    /// 4CH MOVE.
    pub const MOVE: u8 = 0x4C;
}

/// The Information Field names of the transport tier (pp. 44, 47; REQ-MMC-006).
pub mod field {
    /// 01H SELECTED TIME CODE: time code with status, not negative.
    pub const SELECTED_TIME_CODE: u8 = 0x01;
    /// 08H GP0 / LOCATE POINT: time code with subframes, signed.
    pub const GP0: u8 = 0x08;
}

/// LOCATE sub-command 00H: \[I/F\].
const LOCATE_FIELD: u8 = 0x00;
/// LOCATE sub-command 01H: \[TARGET\].
const LOCATE_TARGET: u8 = 0x01;

/// The short name of a time code field: 01H to 1FH give the name plus 20H (p. 9; REQ-MMC-004).
pub const fn short_name(field: u8) -> Option<u8> {
    match field {
        0x01..=0x1F => Some(field + 0x20),
        _ => None,
    }
}

/// The time code field of a short name: 21H to 3FH give the name minus 20H (p. 9; REQ-MMC-004).
pub const fn full_name(short: u8) -> Option<u8> {
    match short {
        0x21..=0x3F => Some(short - 0x20),
        _ => None,
    }
}

/// The status byte `0evdnxxx` of a time code (p. 9; REQ-MMC-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TimeCodeStatus {
    /// e: the estimated code flag.
    pub estimated: bool,
    /// v: the invalid code flag (ignore it when e is 1).
    pub invalid: bool,
    /// d: video field 1.
    pub video_field1: bool,
    /// n: the reader has had no time code at its input after power up or an MMC RESET.
    pub no_time_code: bool,
}

impl TimeCodeStatus {
    /// The flags of a status byte; the reserved bits are ignored (AMB-RP-013-07).
    pub const fn from_byte(byte: u8) -> Self {
        Self {
            estimated: byte & 0x40 != 0,
            invalid: byte & 0x20 != 0,
            video_field1: byte & 0x10 != 0,
            no_time_code: byte & 0x08 != 0,
        }
    }

    /// The status byte, with the reserved bits `xxx` as 000 (p. 9).
    pub const fn to_byte(self) -> u8 {
        (self.estimated as u8) << 6
            | (self.invalid as u8) << 5
            | (self.video_field1 as u8) << 4
            | (self.no_time_code as u8) << 3
    }
}

/// The final byte of a time code: bit i of the frames byte tells the form (p. 8; REQ-MMC-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FinalByte {
    /// i = 0, {ff}: fractional frames, 0 to 99 (7 bits).
    Subframes(u8),
    /// i = 1, {st}: the code status.
    Status(TimeCodeStatus),
}

impl FinalByte {
    /// The bit i of the frames byte.
    const I: u8 = 0x20;

    const fn read(fr: u8, ff: u8) -> Self {
        if fr & Self::I != 0 {
            Self::Status(TimeCodeStatus::from_byte(ff))
        } else {
            Self::Subframes(ff & 0x7F)
        }
    }

    /// The bit i and the final byte.
    const fn bytes(self) -> Result<(u8, u8), SysexError> {
        match self {
            Self::Subframes(s) if s > 99 => Err(SysexError::OutOfRange),
            Self::Subframes(s) => Ok((0, s)),
            Self::Status(st) => Ok((Self::I, st.to_byte())),
        }
    }
}

/// The Standard Time Code `hr mn sc fr {ff|st}` (pp. 8, 9; REQ-MMC-004, REQ-MMC-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StandardTimeCode {
    /// The time with the time code type of the hours byte; a reader gives the raw counts.
    pub time: MtcTime,
    /// c: the color frame flag of the minutes byte.
    pub color_frame: bool,
    /// k: the blank flag of the seconds byte: the field has had no time code since power up or
    /// an MMC RESET.
    pub blank: bool,
    /// g: the sign of the frames byte, where signed time code is permitted.
    pub negative: bool,
    /// The final byte.
    pub final_byte: FinalByte,
}

impl StandardTimeCode {
    /// Reads the 5 bytes; each count is masked to its width (AMB-RP-013-21).
    pub const fn from_bytes(bytes: [u8; 5]) -> Self {
        let [hr, mn, sc, fr, ff] = bytes;
        Self {
            time: MtcTime::raw(hr, mn, sc, fr),
            color_frame: mn & 0x40 != 0,
            blank: sc & 0x40 != 0,
            negative: fr & 0x40 != 0,
            final_byte: FinalByte::read(fr, ff),
        }
    }

    /// The 5 bytes. Gives [`SysexError::OutOfRange`] for subframes above 99 or a time that is
    /// not valid; when the blank flag is set, the time is written as zeroes (p. 8).
    pub const fn to_bytes(&self) -> Result<[u8; 5], SysexError> {
        let (i, ff) = match self.final_byte.bytes() {
            Ok(b) => b,
            Err(e) => return Err(e),
        };
        let t = if self.blank {
            MtcTime::raw(self.time.hours_byte() & 0x60, 0, 0, 0)
        } else if self.time.is_valid() {
            self.time
        } else {
            return Err(SysexError::OutOfRange);
        };
        Ok([
            t.hours_byte(),
            (self.color_frame as u8) << 6 | t.minutes,
            (self.blank as u8) << 6 | t.seconds,
            (self.negative as u8) << 6 | i | t.frames,
            ff,
        ])
    }

    /// The short form: the frames, the sign and the final byte (p. 9).
    pub const fn short(&self) -> ShortTimeCode {
        ShortTimeCode {
            frames: self.time.frames,
            negative: self.negative,
            final_byte: self.final_byte,
        }
    }

    /// Applies a short form to this last full value: the hours, minutes, seconds and their flags
    /// stay (p. 9).
    pub const fn with_short(self, short: ShortTimeCode) -> Self {
        let mut t = self;
        t.time.frames = short.frames;
        t.negative = short.negative;
        t.final_byte = short.final_byte;
        t
    }

    /// The MOVE rule from {st} to {ff}: bit i 0 and subframes 00; an {ff} value stays as it is
    /// (p. 32; REQ-MMC-005).
    pub const fn to_subframes_form(self) -> Self {
        let mut t = self;
        if let FinalByte::Status(_) = t.final_byte {
            t.final_byte = FinalByte::Subframes(0);
        }
        t
    }

    /// The MOVE rule from {ff} to {st}: bit i 1, e 0, v 0, d as the caller gives and n 1; an {st}
    /// value stays as it is (p. 32; REQ-MMC-005).
    pub const fn to_status_form(self, video_field1: bool) -> Self {
        let mut t = self;
        if let FinalByte::Subframes(_) = t.final_byte {
            t.final_byte = FinalByte::Status(TimeCodeStatus {
                estimated: false,
                invalid: false,
                video_field1,
                no_time_code: true,
            });
        }
        t
    }
}

/// The Standard Short Time Code `fr {ff|st}` (p. 9; REQ-MMC-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShortTimeCode {
    /// The frames (5 bits).
    pub frames: u8,
    /// g: the sign.
    pub negative: bool,
    /// The final byte.
    pub final_byte: FinalByte,
}

impl ShortTimeCode {
    /// Reads the 2 bytes; the frames are masked to 5 bits.
    pub const fn from_bytes(bytes: [u8; 2]) -> Self {
        let [fr, ff] = bytes;
        Self {
            frames: fr & 0x1F,
            negative: fr & 0x40 != 0,
            final_byte: FinalByte::read(fr, ff),
        }
    }

    /// The 2 bytes. Gives [`SysexError::OutOfRange`] for frames above 29 or subframes above 99.
    pub const fn to_bytes(&self) -> Result<[u8; 2], SysexError> {
        if self.frames > 29 {
            return Err(SysexError::OutOfRange);
        }
        match self.final_byte.bytes() {
            Ok((i, ff)) => Ok([(self.negative as u8) << 6 | i | self.frames, ff]),
            Err(e) => Err(e),
        }
    }
}

/// The data length that the last byte of a name gives (p. 5).
enum Length {
    Fixed(usize),
    Count,
}

const fn command_length(name: u8) -> Length {
    match name {
        0x40..=0x77 => Length::Count,
        _ => Length::Fixed(0),
    }
}

const fn field_length(name: u8) -> Length {
    match name {
        0x01..=0x1F => Length::Fixed(5),
        0x20..=0x3F => Length::Fixed(2),
        0x40..=0x77 => Length::Count,
        _ => Length::Fixed(0),
    }
}

/// One name with its data from the front of a string: (name, data, rest), or `None` when the
/// string is cut short (REQ-MMC-002).
fn split(bytes: &[u8], length: fn(u8) -> Length) -> Option<(&[u8], &[u8], &[u8])> {
    let prefixes = bytes.iter().take(2).take_while(|&&b| b == 0).count();
    let (name, rest) = bytes.split_at_checked(prefixes + 1)?;
    let (data, rest) = match length(name[prefixes]) {
        Length::Fixed(n) => rest.split_at_checked(n)?,
        Length::Count => {
            let (&count, rest) = rest.split_first()?;
            rest.split_at_checked(count as usize)?
        }
    };
    Some((name, data, rest))
}

/// Writes a raw name and its data by the length rule; [`SysexError::Length`] when the data does
/// not fit the rule or the name is not one byte with up to two 00H prefixes.
fn write_named(
    w: &mut Out<'_>,
    name: &[u8],
    data: &[u8],
    length: fn(u8) -> Length,
) -> Result<(), SysexError> {
    let Some((&last, prefixes)) = name.split_last() else {
        return Err(SysexError::Length);
    };
    if prefixes.len() > 2 || prefixes.iter().any(|&b| b != 0) || last == 0 && prefixes.len() < 2 {
        return Err(SysexError::Length);
    }
    w.data(name)?;
    match length(last) {
        Length::Fixed(n) if n == data.len() => {}
        Length::Count if data.len() <= 0x7F => w.byte(data.len() as u8),
        _ => return Err(SysexError::Length),
    }
    w.data(data)
}

/// One MMC command (pp. 20 to 32; REQ-MMC-003).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MmcCommand<'a> {
    /// 01H STOP.
    Stop,
    /// 02H PLAY.
    Play,
    /// 03H DEFERRED PLAY.
    DeferredPlay,
    /// 04H FAST FORWARD.
    FastForward,
    /// 05H REWIND.
    Rewind,
    /// 06H RECORD STROBE.
    RecordStrobe,
    /// 07H RECORD EXIT.
    RecordExit,
    /// 09H PAUSE.
    Pause,
    /// 0DH MMC RESET.
    Reset,
    /// 40H WRITE: the name and data pairs of the Information Fields.
    Write(InfoFields<'a>),
    /// 44H LOCATE \[I/F\]: sub-command 00H and a register name, 08H (GP0) to 0FH (GP7).
    LocateField {
        /// The register name.
        register: u8,
    },
    /// 44H LOCATE \[TARGET\]: sub-command 01H and a time with subframes ({ff}).
    LocateTarget(StandardTimeCode),
    /// 4CH MOVE: the destination and source Information Field names, 01H to 1FH.
    Move {
        /// The destination field.
        destination: u8,
        /// The source field.
        source: u8,
    },
    /// Any other command, or a known one with data that does not fit it: the name with its 00H
    /// extension prefixes, and the data after the count byte.
    Other {
        /// The name bytes.
        name: &'a [u8],
        /// The data bytes.
        data: &'a [u8],
    },
}

impl<'a> MmcCommand<'a> {
    fn read(name: &'a [u8], data: &'a [u8]) -> Self {
        use command::*;
        match (name, data) {
            ([STOP], _) => Self::Stop,
            ([PLAY], _) => Self::Play,
            ([DEFERRED_PLAY], _) => Self::DeferredPlay,
            ([FAST_FORWARD], _) => Self::FastForward,
            ([REWIND], _) => Self::Rewind,
            ([RECORD_STROBE], _) => Self::RecordStrobe,
            ([RECORD_EXIT], _) => Self::RecordExit,
            ([PAUSE], _) => Self::Pause,
            ([MMC_RESET], _) => Self::Reset,
            ([WRITE], d) => Self::Write(InfoFields::new(d)),
            // A count above the defined length: the defined bytes, the rest skipped (p. 5).
            ([LOCATE], &[LOCATE_FIELD, register, ..]) if register != 0 => {
                Self::LocateField { register }
            }
            ([LOCATE], &[LOCATE_TARGET, hr, mn, sc, fr, ff, ..]) => {
                Self::LocateTarget(StandardTimeCode::from_bytes([hr, mn, sc, fr, ff]))
            }
            ([MOVE], &[destination, source, ..]) if destination != 0 && source != 0 => Self::Move {
                destination,
                source,
            },
            _ => Self::Other { name, data },
        }
    }

    fn write(&self, w: &mut Out<'_>) -> Result<(), SysexError> {
        use command::*;
        let name = match self {
            Self::Stop => STOP,
            Self::Play => PLAY,
            Self::DeferredPlay => DEFERRED_PLAY,
            Self::FastForward => FAST_FORWARD,
            Self::Rewind => REWIND,
            Self::RecordStrobe => RECORD_STROBE,
            Self::RecordExit => RECORD_EXIT,
            Self::Pause => PAUSE,
            Self::Reset => MMC_RESET,
            Self::Write(fields) => {
                return write_named(w, &[WRITE], fields.as_bytes(), command_length)
            }
            Self::LocateField { register } => {
                if !matches!(register, 0x08..=0x0F) {
                    return Err(SysexError::OutOfRange);
                }
                w.bytes(&[LOCATE, 2, LOCATE_FIELD, *register]);
                return Ok(());
            }
            Self::LocateTarget(target) => {
                if !matches!(target.final_byte, FinalByte::Subframes(_)) {
                    return Err(SysexError::OutOfRange);
                }
                w.bytes(&[LOCATE, 6, LOCATE_TARGET]);
                w.bytes(&target.to_bytes()?);
                return Ok(());
            }
            Self::Move {
                destination,
                source,
            } => {
                if !matches!(destination, 0x01..=0x1F) || !matches!(source, 0x01..=0x1F) {
                    return Err(SysexError::OutOfRange);
                }
                w.bytes(&[MOVE, 2, *destination, *source]);
                return Ok(());
            }
            Self::Other { name, data } => return write_named(w, name, data, command_length),
        };
        w.byte(name);
        Ok(())
    }
}

/// One Information Field (pp. 8, 9, 44, 47; REQ-MMC-004, REQ-MMC-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoField<'a> {
    /// A time code field, 01H to 1FH, with 5 data bytes. SELECTED TIME CODE (01H) is {st} and not
    /// negative; GP0 (08H) is {ff} and signed.
    TimeCode {
        /// The field name.
        field: u8,
        /// The time code.
        code: StandardTimeCode,
    },
    /// The short form of a time code field: the name 21H to 3FH, with 2 data bytes.
    Short {
        /// The field name of the full form, 01H to 1FH.
        field: u8,
        /// The short time code.
        code: ShortTimeCode,
    },
    /// Any other field: the name with its 00H extension prefixes, and its data.
    Other {
        /// The name bytes.
        name: &'a [u8],
        /// The data bytes, after the count byte when the name has one.
        data: &'a [u8],
    },
}

/// The form rules of the transport tier fields (pp. 44, 47; REQ-MMC-006).
const fn check_form(field: u8, negative: bool, final_byte: FinalByte) -> Result<(), SysexError> {
    let ok = match (field, final_byte) {
        (field::SELECTED_TIME_CODE, FinalByte::Status(_)) => !negative,
        (field::SELECTED_TIME_CODE, _) => false,
        (field::GP0, f) => matches!(f, FinalByte::Subframes(_)),
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(SysexError::OutOfRange)
    }
}

impl<'a> InfoField<'a> {
    fn read(name: &'a [u8], data: &'a [u8]) -> Self {
        match (name, data) {
            (&[field @ 0x01..=0x1F], &[hr, mn, sc, fr, ff]) => Self::TimeCode {
                field,
                code: StandardTimeCode::from_bytes([hr, mn, sc, fr, ff]),
            },
            (&[short @ 0x21..=0x3F], &[fr, ff]) => Self::Short {
                field: short - 0x20,
                code: ShortTimeCode::from_bytes([fr, ff]),
            },
            _ => Self::Other { name, data },
        }
    }

    fn write(&self, w: &mut Out<'_>) -> Result<(), SysexError> {
        match self {
            Self::TimeCode { field, code } => {
                if short_name(*field).is_none() {
                    return Err(SysexError::OutOfRange);
                }
                check_form(*field, code.negative, code.final_byte)?;
                w.byte(*field);
                w.bytes(&code.to_bytes()?);
            }
            Self::Short { field, code } => {
                let Some(short) = short_name(*field) else {
                    return Err(SysexError::OutOfRange);
                };
                check_form(*field, code.negative, code.final_byte)?;
                w.byte(short);
                w.bytes(&code.to_bytes()?);
            }
            Self::Other { name, data } => write_named(w, name, data, field_length)?,
        }
        Ok(())
    }
}

/// The reader of a command string (REQ-MMC-001, REQ-MMC-002). A string cut short gives
/// [`SysexError::Length`] once, then the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commands<'a> {
    bytes: &'a [u8],
}

impl<'a> Commands<'a> {
    /// The bytes not read yet.
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

impl<'a> Iterator for Commands<'a> {
    type Item = Result<MmcCommand<'a>, SysexError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bytes.is_empty() {
            return None;
        }
        let Some((name, data, rest)) = split(self.bytes, command_length) else {
            self.bytes = &[];
            return Some(Err(SysexError::Length));
        };
        self.bytes = rest;
        Some(Ok(MmcCommand::read(name, data)))
    }
}

/// The reader of an Information Field string: a response, or the data of WRITE (REQ-MMC-002,
/// REQ-MMC-006). A string cut short gives [`SysexError::Length`] once, then the end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoFields<'a> {
    bytes: &'a [u8],
}

impl<'a> InfoFields<'a> {
    /// A reader over the name and data pairs, for example the data of a WRITE command.
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// The bytes not read yet.
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

impl<'a> Iterator for InfoFields<'a> {
    type Item = Result<InfoField<'a>, SysexError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.bytes.is_empty() {
            return None;
        }
        let Some((name, data, rest)) = split(self.bytes, field_length) else {
            self.bytes = &[];
            return Some(Err(SysexError::Length));
        };
        self.bytes = rest;
        Some(Ok(InfoField::read(name, data)))
    }
}

/// An MMC message (p. 1; REQ-MMC-001).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MmcMessage<'a> {
    /// Sub-ID#1 06H: commands to a device, a group or all devices.
    Commands {
        /// The destination Device ID.
        device_id: u8,
        /// The command string.
        commands: Commands<'a>,
    },
    /// Sub-ID#1 07H: responses from a device.
    Responses {
        /// The source Device ID; 7FH is not valid (AMB-RP-013-04).
        device_id: u8,
        /// The response string.
        fields: InfoFields<'a>,
    },
}

impl<'a> MmcMessage<'a> {
    /// Reads the message from a Universal header (Real Time, Sub-ID#1 06H or 07H). Gives `None`
    /// for another message and [`SysexError::Length`] for an empty string.
    pub fn from_universal(u: &Universal<'a>) -> Result<Option<Self>, SysexError> {
        if !u.real_time || !matches!(u.sub_id1, COMMAND | RESPONSE) {
            return Ok(None);
        }
        if u.data.is_empty() {
            return Err(SysexError::Length);
        }
        let device_id = u.device_id;
        Ok(Some(if u.sub_id1 == COMMAND {
            Self::Commands {
                device_id,
                commands: Commands { bytes: u.data },
            }
        } else {
            Self::Responses {
                device_id,
                fields: InfoFields { bytes: u.data },
            }
        }))
    }
}

/// Checks the string length: not empty and at most [`MAX_STRING`] bytes (p. 1).
fn check_string(start: usize, w: &Out<'_>) -> Result<(), SysexError> {
    match w.len() - start {
        1..=MAX_STRING => Ok(()),
        _ => Err(SysexError::Length),
    }
}

/// Writes the whole command message `F0 7F <device ID> 06 <commands> F7` into `out` and gives
/// the length (REQ-MMC-001, REQ-MMC-003, REQ-SX-007).
pub fn write_commands(
    out: &mut [u8],
    device_id: u8,
    commands: &[MmcCommand<'_>],
) -> Result<usize, SysexError> {
    write_universal(out, true, device_id, COMMAND, None, |w| {
        let start = w.len();
        for c in commands {
            c.write(w)?;
        }
        check_string(start, w)
    })
}

/// Writes the whole response message `F0 7F <device ID> 07 <responses> F7` into `out` and gives
/// the length (REQ-MMC-001, REQ-MMC-006, REQ-SX-007). The source Device ID is 00H to 7EH
/// (AMB-RP-013-04).
pub fn write_responses(
    out: &mut [u8],
    device_id: u8,
    fields: &[InfoField<'_>],
) -> Result<usize, SysexError> {
    if device_id == 0x7F {
        return Err(SysexError::OutOfRange);
    }
    write_universal(out, true, device_id, RESPONSE, None, |w| {
        let start = w.len();
        for f in fields {
            f.write(w)?;
        }
        check_string(start, w)
    })
}

/// Writes the name and data pairs of `fields` into `out`, for the data of a WRITE command, and
/// gives the length (REQ-MMC-003, REQ-MMC-006).
pub fn write_info_fields(out: &mut [u8], fields: &[InfoField<'_>]) -> Result<usize, SysexError> {
    let capacity = out.len();
    let mut w = Out::new(out);
    for f in fields {
        f.write(&mut w)?;
    }
    match w.len() {
        n if n > capacity => Err(SysexError::BufferTooSmall { needed: n }),
        n => Ok(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mtc::TimeCodeType::*;
    use crate::universal::Sysex;

    fn message(bytes: &[u8]) -> Result<Option<MmcMessage<'_>>, SysexError> {
        let s = Sysex::parse(bytes)?;
        let u = Universal::parse(&s)?.ok_or(SysexError::NoId)?;
        MmcMessage::from_universal(&u)
    }

    fn string(bytes: &[u8]) -> Commands<'_> {
        Commands { bytes }
    }

    fn time(h: u8, m: u8, s: u8, f: u8, ff: u8) -> StandardTimeCode {
        StandardTimeCode {
            time: MtcTime::new(h, m, s, f, Fps30).unwrap(),
            color_frame: false,
            blank: false,
            negative: false,
            final_byte: FinalByte::Subframes(ff),
        }
    }

    #[test]
    fn mmc_command_and_response_strings() {
        // REQ-MMC-001: p. 1 "F0 7F <device_ID> <mcc> <commands...> F7" with <mcc> 06 and 07.
        let bytes = [0xF0, 0x7F, 0x05, 0x06, 0x01, 0x02, 0xF7];
        let Ok(Some(MmcMessage::Commands {
            device_id,
            commands,
        })) = message(&bytes)
        else {
            panic!("not a command message");
        };
        assert_eq!(device_id, 0x05);
        let mut c = commands;
        assert_eq!(c.next(), Some(Ok(MmcCommand::Stop)));
        assert_eq!(c.as_bytes(), &[0x02]);
        assert_eq!(c.next(), Some(Ok(MmcCommand::Play)));
        assert_eq!(c.next(), None);
        // Several commands in one message.
        let mut out = [0u8; 16];
        let cmds = [MmcCommand::Stop, MmcCommand::Rewind, MmcCommand::Reset];
        assert_eq!(write_commands(&mut out, 0x7F, &cmds), Ok(8));
        assert_eq!(&out[..8], &[0xF0, 0x7F, 0x7F, 0x06, 0x01, 0x05, 0x0D, 0xF7]);
        // A response from device 01H.
        let f = [InfoField::TimeCode {
            field: field::GP0,
            code: time(1, 2, 3, 4, 50),
        }];
        assert_eq!(write_responses(&mut out, 0x01, &f), Ok(11));
        assert_eq!(
            &out[..11],
            &[0xF0, 0x7F, 0x01, 0x07, 0x08, 0x61, 2, 3, 4, 50, 0xF7]
        );
        let Ok(Some(MmcMessage::Responses { device_id, fields })) = message(&out[..11]) else {
            panic!("not a response message");
        };
        assert_eq!(device_id, 0x01);
        assert!(fields.eq(f.iter().map(|&x| Ok(x))));
        // The string length: at most 48 bytes, never empty (AMB-RP-013-03).
        let mut big = [0u8; 64];
        assert_eq!(
            write_commands(
                &mut big,
                0x7F,
                &core::array::from_fn::<_, 48, _>(|_| MmcCommand::Stop)
            ),
            Ok(53)
        );
        assert_eq!(
            write_commands(
                &mut big,
                0x7F,
                &core::array::from_fn::<_, 49, _>(|_| MmcCommand::Stop)
            ),
            Err(SysexError::Length)
        );
        assert_eq!(write_commands(&mut big, 0x7F, &[]), Err(SysexError::Length));
        assert_eq!(
            message(&[0xF0, 0x7F, 0x7F, 0x06, 0xF7]),
            Err(SysexError::Length)
        );
        assert_eq!(
            write_commands(&mut out[..4], 0x7F, &cmds),
            Err(SysexError::BufferTooSmall { needed: 8 })
        );
        // The source of a response is 00H to 7EH (AMB-RP-013-04).
        assert_eq!(
            write_responses(&mut out, 0x7F, &f),
            Err(SysexError::OutOfRange)
        );
        // Another Real Time family is not MMC.
        assert_eq!(message(&[0x7F, 0x7F, 0x01, 0x01, 0, 0, 0, 0]), Ok(None));
    }

    #[test]
    fn mmc_length_rule_and_extensions() {
        // REQ-MMC-002: p. 5 Message Length, for known and unknown names.
        let mut c = string(&[
            0x20, // unknown, 0 data bytes
            0x50, 0x02, 0x11, 0x22, // unknown, a count
            0x7A, // handshake, 0 data bytes
            0x00, 0x01, // extended name, last byte 01H: 0 data bytes
            0x00, 0x00, 0x45, 0x01, 0x33, // two prefixes, a count
            0x44, 0x03, 0x00, 0x08, 0x55, // LOCATE [I/F] with a longer count: 55H skipped
            0x02,
        ]);
        let o =
            |name: &'static [u8], data: &'static [u8]| Some(Ok(MmcCommand::Other { name, data }));
        assert_eq!(c.next(), o(&[0x20], &[]));
        assert_eq!(c.next(), o(&[0x50], &[0x11, 0x22]));
        assert_eq!(c.next(), o(&[0x7A], &[]));
        assert_eq!(c.next(), o(&[0x00, 0x01], &[]));
        assert_eq!(c.next(), o(&[0x00, 0x00, 0x45], &[0x33]));
        assert_eq!(
            c.next(),
            Some(Ok(MmcCommand::LocateField { register: 0x08 }))
        );
        assert_eq!(c.next(), Some(Ok(MmcCommand::Play)));
        assert_eq!(c.next(), None);
        // A string cut short: the error once, then the end.
        let mut c = string(&[0x01, 0x40, 0x05, 0x01]);
        assert_eq!(c.next(), Some(Ok(MmcCommand::Stop)));
        assert_eq!(c.next(), Some(Err(SysexError::Length)));
        assert_eq!(c.next(), None);
        assert_eq!(string(&[0x00, 0x00]).next(), Some(Err(SysexError::Length)));
        // Information Fields: 5, 2, a count and none.
        let mut f = InfoFields::new(&[
            0x02, 0x61, 0, 0, 0x20, 0, // 02H, 5 bytes
            0x20, 1, 2, // 20H, 2 bytes, no full form
            0x48, 0x01, 0x7F, // a count
            0x7F, // handshake
            0x00, 0x05, 1, 2, 3, 4, 5, // an extended time code name
        ]);
        let Some(Ok(InfoField::TimeCode { field: 0x02, code })) = f.next() else {
            panic!("not a time code");
        };
        assert_eq!(
            code.final_byte,
            FinalByte::Status(TimeCodeStatus::default())
        );
        assert_eq!(
            f.next(),
            Some(Ok(InfoField::Other {
                name: &[0x20],
                data: &[1, 2]
            }))
        );
        assert_eq!(
            f.next(),
            Some(Ok(InfoField::Other {
                name: &[0x48],
                data: &[0x7F]
            }))
        );
        assert_eq!(
            f.next(),
            Some(Ok(InfoField::Other {
                name: &[0x7F],
                data: &[]
            }))
        );
        assert_eq!(
            f.next(),
            Some(Ok(InfoField::Other {
                name: &[0x00, 0x05],
                data: &[1, 2, 3, 4, 5]
            }))
        );
        assert_eq!(f.next(), None);
        assert_eq!(
            InfoFields::new(&[0x01, 0, 0]).next(),
            Some(Err(SysexError::Length))
        );
        // The writer applies the same rule to a raw name.
        let mut out = [0u8; 16];
        let raw = |name, data| [MmcCommand::Other { name, data }];
        assert_eq!(
            write_commands(&mut out, 0, &raw(&[0x00, 0x45], &[1, 2])),
            Ok(10)
        );
        assert_eq!(&out[4..9], &[0x00, 0x45, 0x02, 1, 2]);
        assert_eq!(
            write_commands(&mut out, 0, &raw(&[0x20], &[1])),
            Err(SysexError::Length)
        );
        assert_eq!(
            write_commands(&mut out, 0, &raw(&[0x01, 0x20], &[])),
            Err(SysexError::Length)
        );
        assert_eq!(
            write_commands(&mut out, 0, &raw(&[], &[])),
            Err(SysexError::Length)
        );
        assert_eq!(
            write_commands(&mut out, 0, &raw(&[0x00], &[])),
            Err(SysexError::Length)
        );
    }

    #[test]
    fn mmc_transport_commands_round_trip() {
        // REQ-MMC-003: every command of the tier, written and read back (pp. 20 to 32).
        let data = [0x08, 0x61, 0x00, 0x00, 0x00, 0x00];
        let cmds = [
            MmcCommand::Stop,
            MmcCommand::Play,
            MmcCommand::DeferredPlay,
            MmcCommand::FastForward,
            MmcCommand::Rewind,
            MmcCommand::RecordStrobe,
            MmcCommand::RecordExit,
            MmcCommand::Pause,
            MmcCommand::Reset,
            MmcCommand::Write(InfoFields::new(&data)),
            MmcCommand::LocateField { register: 0x08 },
            MmcCommand::LocateTarget(time(1, 2, 3, 4, 5)),
            MmcCommand::Move {
                destination: 0x08,
                source: 0x01,
            },
        ];
        let mut out = [0u8; 48];
        let n = write_commands(&mut out, 0x7F, &cmds).unwrap();
        assert_eq!(
            &out[4..n - 1],
            &[
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x09,
                0x0D, // the commands without data
                0x40, 0x06, 0x08, 0x61, 0x00, 0x00, 0x00, 0x00, // WRITE <count> GP0 <data>
                0x44, 0x02, 0x00, 0x08, // LOCATE [I/F] GP0
                0x44, 0x06, 0x01, 0x61, 0x02, 0x03, 0x04,
                0x05, // LOCATE [TARGET] hr mn sc fr ff
                0x4C, 0x02, 0x08, 0x01, // MOVE GP0 SELECTED TIME CODE
            ]
        );
        let Ok(Some(MmcMessage::Commands { commands, .. })) = message(&out[..n]) else {
            panic!("not a command message");
        };
        let mut read = commands;
        for c in &cmds {
            assert_eq!(read.next().as_ref(), Some(&Ok(c.clone())));
        }
        assert_eq!(read.next(), None);
        // The WRITE data reads as Information Fields.
        let mut w = InfoFields::new(&data);
        assert!(matches!(
            w.next(),
            Some(Ok(InfoField::TimeCode { field: 0x08, .. }))
        ));
        // Writer checks.
        let bad = |c| write_commands(&mut [0u8; 32], 0x7F, &[c]);
        assert_eq!(
            bad(MmcCommand::LocateField { register: 0x07 }),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            bad(MmcCommand::LocateField { register: 0x10 }),
            Err(SysexError::OutOfRange)
        );
        let st = time(1, 2, 3, 4, 0).to_status_form(false);
        assert_eq!(
            bad(MmcCommand::LocateTarget(st)),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            bad(MmcCommand::Move {
                destination: 0x20,
                source: 1
            }),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            bad(MmcCommand::Move {
                destination: 1,
                source: 0
            }),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            write_commands(&mut [0u8; 8], 0x80, &[MmcCommand::Stop]),
            Err(SysexError::OutOfRange)
        );
        // Data that does not fit a known command is raw.
        let mut c = string(&[0x44, 0x01, 0x02, 0x4C, 0x01, 0x08, 0x44, 0x02, 0x00, 0x00]);
        assert_eq!(
            c.next(),
            Some(Ok(MmcCommand::Other {
                name: &[0x44],
                data: &[0x02]
            }))
        );
        assert_eq!(
            c.next(),
            Some(Ok(MmcCommand::Other {
                name: &[0x4C],
                data: &[0x08]
            }))
        );
        assert_eq!(
            c.next(),
            Some(Ok(MmcCommand::Other {
                name: &[0x44],
                data: &[0, 0]
            }))
        );
    }

    #[test]
    fn standard_time_code_bytes() {
        // REQ-MMC-004: pp. 8, 9. Example 2A (p. 76): 01:02:03:06 at 30 frames is
        // "61 02 03 26 00", the status form with no flag.
        let t = StandardTimeCode::from_bytes([0x61, 0x02, 0x03, 0x26, 0x00]);
        assert_eq!(t.time, MtcTime::new(1, 2, 3, 6, Fps30).unwrap());
        assert_eq!(t.final_byte, FinalByte::Status(TimeCodeStatus::default()));
        assert!(!t.color_frame && !t.blank && !t.negative);
        assert_eq!(t.to_bytes(), Ok([0x61, 0x02, 0x03, 0x26, 0x00]));
        // Every flag, the subframes form and the 25 frame type.
        let t = StandardTimeCode::from_bytes([0x37, 0x7B, 0x3B, 0x58, 0x63]);
        assert_eq!(t.time, MtcTime::new(23, 59, 59, 24, Fps25).unwrap());
        assert!(t.color_frame && !t.blank && t.negative);
        assert_eq!(t.final_byte, FinalByte::Subframes(99));
        assert_eq!(t.to_bytes(), Ok([0x37, 0x7B, 0x3B, 0x58, 0x63]));
        // Status flags; the reserved bits xxx and bit 7 are ignored, and written as 0.
        let st = TimeCodeStatus::from_byte(0xFF);
        assert_eq!(
            st,
            TimeCodeStatus {
                estimated: true,
                invalid: true,
                video_field1: true,
                no_time_code: true
            }
        );
        assert_eq!(st.to_byte(), 0x78);
        for (b, f) in [(0x40, 0), (0x20, 1), (0x10, 2), (0x08, 3)] {
            let s = TimeCodeStatus::from_byte(b);
            assert_eq!(
                [s.estimated, s.invalid, s.video_field1, s.no_time_code]
                    .iter()
                    .position(|&x| x),
                Some(f)
            );
        }
        // The blank flag: the time is written as zeroes, the type stays.
        let mut b = time(1, 2, 3, 4, 5);
        b.blank = true;
        assert_eq!(b.to_bytes(), Ok([0x60, 0x00, 0x40, 0x00, 0x05]));
        // The writer range rules (AMB-RP-013-21); the reader keeps raw counts.
        assert_eq!(
            time(1, 2, 3, 4, 100).to_bytes(),
            Err(SysexError::OutOfRange)
        );
        let raw = StandardTimeCode::from_bytes([0x1F, 0x3F, 0x3F, 0x1F, 0x7F]);
        assert!(!raw.time.is_valid());
        assert_eq!(raw.final_byte, FinalByte::Subframes(0x7F));
        assert_eq!(raw.to_bytes(), Err(SysexError::OutOfRange));
        let drop = StandardTimeCode::from_bytes([0x40, 0x01, 0x00, 0x00, 0x00]);
        assert_eq!(drop.to_bytes(), Err(SysexError::OutOfRange));
    }

    #[test]
    fn short_time_code_names() {
        // REQ-MMC-004: the example of p. 9: GENERATOR TIME CODE 06H has the short name 26H.
        assert_eq!(
            (short_name(0x06), full_name(0x26)),
            (Some(0x26), Some(0x06))
        );
        assert_eq!(
            (short_name(0x01), short_name(0x1F)),
            (Some(0x21), Some(0x3F))
        );
        assert_eq!((short_name(0x00), short_name(0x20)), (None, None));
        assert_eq!(
            (full_name(0x20), full_name(0x40), full_name(0x21)),
            (None, None, Some(0x01))
        );
        // The short form is fr and the final byte of the full form.
        let full = StandardTimeCode::from_bytes([0x61, 0x42, 0x03, 0x46, 0x10]);
        let short = full.short();
        assert_eq!(short.to_bytes(), Ok([0x46, 0x10]));
        assert_eq!(ShortTimeCode::from_bytes([0x46, 0x10]), short);
        // The controller keeps hours, minutes, seconds and their flags from the last full value.
        let next = full.with_short(ShortTimeCode::from_bytes([0x27, 0x08]));
        assert_eq!(next.to_bytes(), Ok([0x61, 0x42, 0x03, 0x27, 0x08]));
        assert_eq!(
            ShortTimeCode {
                frames: 30,
                ..short
            }
            .to_bytes(),
            Err(SysexError::OutOfRange)
        );
        // A short field reads with its full name, and writes back with its short name.
        let mut f = InfoFields::new(&[0x21, 0x26, 0x00]);
        let got = f.next().unwrap().unwrap();
        assert_eq!(
            got,
            InfoField::Short {
                field: 0x01,
                code: ShortTimeCode::from_bytes([0x26, 0])
            }
        );
        let mut out = [0u8; 4];
        assert_eq!(write_info_fields(&mut out, &[got]), Ok(3));
        assert_eq!(&out[..3], &[0x21, 0x26, 0x00]);
        let bad = InfoField::Short {
            field: 0x20,
            code: short,
        };
        assert_eq!(
            write_info_fields(&mut out, &[bad]),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            write_info_fields(&mut out[..2], &[got]),
            Err(SysexError::BufferTooSmall { needed: 3 })
        );
    }

    #[test]
    fn time_code_form_conversion() {
        // REQ-MMC-005: p. 32 MOVE (a) {st} to {ff}: i = 0, subframes 00; (b) {ff} to {st}:
        // i = 1, e = 0, v = 0, d as determined, n = 1. The other flags stay.
        let mut st = StandardTimeCode::from_bytes([0x61, 0x42, 0x43, 0x66, 0x70]);
        assert!(st.color_frame && st.blank && st.negative);
        let ff = st.to_subframes_form();
        assert_eq!(ff.final_byte, FinalByte::Subframes(0));
        assert_eq!(
            (ff.time, ff.color_frame, ff.blank, ff.negative),
            (st.time, true, true, true)
        );
        let back = ff.to_status_form(true);
        let want = TimeCodeStatus {
            estimated: false,
            invalid: false,
            video_field1: true,
            no_time_code: true,
        };
        assert_eq!(back.final_byte, FinalByte::Status(want));
        assert_eq!(
            (back.time, back.color_frame, back.blank, back.negative),
            (st.time, true, true, true)
        );
        // The same form stays as it is: subframes are kept, status flags are kept.
        let sub = time(0, 0, 1, 2, 42);
        assert_eq!(sub.to_subframes_form(), sub);
        st.blank = false;
        assert_eq!(st.to_status_form(false), st);
        assert_eq!(
            time(0, 0, 1, 2, 42).to_status_form(false).to_bytes(),
            Ok([0x60, 0, 1, 0x22, 0x08])
        );
    }

    #[test]
    fn mmc_transport_fields_round_trip() {
        // REQ-MMC-006: SELECTED TIME CODE {st} not negative (p. 44); GP0 {ff} signed (p. 47).
        let selected = time(10, 20, 30, 15, 0).to_status_form(true);
        let mut gp0 = time(0, 0, 5, 0, 50);
        gp0.negative = true;
        let fields = [
            InfoField::TimeCode {
                field: field::SELECTED_TIME_CODE,
                code: selected,
            },
            InfoField::TimeCode {
                field: field::GP0,
                code: gp0,
            },
            InfoField::Other {
                name: &[0x48],
                data: &[0x01],
            },
        ];
        let mut out = [0u8; 32];
        let n = write_responses(&mut out, 0x10, &fields).unwrap();
        assert_eq!(
            &out[4..n - 1],
            &[
                0x01, 0x6A, 0x14, 0x1E, 0x2F, 0x18, 0x08, 0x60, 0x00, 0x05, 0x40, 0x32, 0x48, 0x01,
                0x01
            ]
        );
        let Ok(Some(MmcMessage::Responses { fields: read, .. })) = message(&out[..n]) else {
            panic!("not a response message");
        };
        assert!(read.eq(fields.iter().map(|&f| Ok(f))));
        // The form rules of the writer.
        let one = |f| write_info_fields(&mut [0u8; 8], &[f]);
        let mut neg = selected;
        neg.negative = true;
        assert_eq!(
            one(InfoField::TimeCode {
                field: 0x01,
                code: neg
            }),
            Err(SysexError::OutOfRange)
        );
        let sub = time(1, 0, 0, 0, 0);
        assert_eq!(
            one(InfoField::TimeCode {
                field: 0x01,
                code: sub
            }),
            Err(SysexError::OutOfRange)
        );
        let st = sub.to_status_form(false);
        assert_eq!(
            one(InfoField::TimeCode {
                field: 0x08,
                code: st
            }),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            one(InfoField::Short {
                field: 0x01,
                code: neg.short()
            }),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            one(InfoField::TimeCode {
                field: 0x20,
                code: sub
            }),
            Err(SysexError::OutOfRange)
        );
        // Another time code field takes either form.
        assert_eq!(
            one(InfoField::TimeCode {
                field: 0x06,
                code: st
            }),
            Ok(6)
        );
        assert_eq!(
            one(InfoField::TimeCode {
                field: 0x06,
                code: sub
            }),
            Ok(6)
        );
    }
}
