//! The System Exclusive base (M1 v4.2.1 System Exclusive pp. 34, 35; Table VII p. T-8; Table VIIa
//! pp. T-9, T-10; DEC-022). Always built.
//!
//! - [`Sysex::parse`] takes one whole message, with or without the F0H start and the F7H end
//!   (REQ-SX-001), and gives its ID class (Table VII, [`SysexId`]) with the bytes after the ID
//!   (REQ-SX-002). A data byte above 7FH, an F0H or F7H inside, or no ID byte is an error.
//! - [`Universal::parse`] reads the Universal header: the Device ID, Sub-ID#1 and, only for the
//!   families of [`has_sub_id2`], the Sub-ID#2 (AMB-M1-29; REQ-SX-004).
//! - [`addressed`] is the Device ID test: 7FH ([`ALL_DEVICES`]) or the own Device ID (p. 35,
//!   AMB-M1-30; REQ-SX-005).
//! - Each writer writes one whole message, F0H to F7H, into a caller buffer and gives its length,
//!   or [`SysexError::BufferTooSmall`] with the needed length; the buffer content is then not
//!   defined (REQ-SX-007). [`write_manufacturer`] rejects the IDs that M1 reserves (AMB-M1-58).
//!
//! No alloc (DEC-002). Never panics.

use zmidi_ump::ManufacturerId;

/// Start of System Exclusive, SOX (Table VII).
pub(crate) const SOX: u8 = 0xF0;
/// End of System Exclusive, EOX (Table VII).
pub(crate) const EOX: u8 = 0xF7;
/// Non-Commercial ID (Table VII).
const NON_COMMERCIAL: u8 = 0x7D;
/// Universal Non-Real Time ID (Table VII).
const NON_REAL_TIME: u8 = 0x7E;
/// Universal Real Time ID (Table VII).
const REAL_TIME: u8 = 0x7F;
/// The MfrID of the 3-byte ID 00H 00H 00H, which M1 reserves (p. 34).
const RESERVED_THREE_BYTE: u16 = 0x8000;

/// The Device ID that addresses all devices ("all call", p. 35); RP-003 and RP-024 suggest it for
/// the General MIDI messages (REQ-SX-005).
pub const ALL_DEVICES: u8 = 0x7F;

/// The error of every reader and writer of the crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum SysexError {
    /// No ID byte after F0H (REQ-SX-001).
    #[error("no ID byte")]
    NoId,
    /// A data byte above 7FH, or F0H or F7H inside the message (REQ-SX-001).
    #[error("a data byte above 7FH")]
    DataByte,
    /// The message is too short, or its length is not one that the format allows.
    #[error("a length that the format does not allow")]
    Length,
    /// A writer value outside the range of its field (REQ-SX-007).
    #[error("a value out of the range of its field")]
    OutOfRange,
    /// The caller buffer is too small for the message (REQ-SX-007).
    #[error("the buffer is too small: {needed} bytes needed")]
    BufferTooSmall {
        /// The length of the whole message, F0H to F7H.
        needed: usize,
    },
    /// The checksum of a dump does not match (MIDI Tuning).
    #[error("the checksum does not match")]
    Checksum,
}

/// The ID class of a message: the first data byte (Table VII, REQ-SX-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SysexId {
    /// 00H (with two more bytes) or 01H to 7CH: a Manufacturer ID.
    Manufacturer(ManufacturerId),
    /// 7DH: Non-Commercial, not for products released to the public.
    NonCommercial,
    /// 7EH: Universal Non-Real Time.
    NonRealTime,
    /// 7FH: Universal Real Time.
    RealTime,
}

/// One System Exclusive message; it borrows the input bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sysex<'a> {
    /// The ID class.
    pub id: SysexId,
    /// The bytes after the ID, without F7H.
    pub data: &'a [u8],
}

impl<'a> Sysex<'a> {
    /// Reads one whole message, with or without F0H at the start and F7H at the end (REQ-SX-001).
    /// Gives [`SysexError::DataByte`] for a byte above 7FH (so also for F0H or F7H inside),
    /// [`SysexError::NoId`] for no ID byte, and [`SysexError::Length`] for a 3-byte Manufacturer
    /// ID with fewer than three bytes. Never panics.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, SysexError> {
        let body = strip(bytes);
        if body.iter().any(|&b| b > 0x7F) {
            return Err(SysexError::DataByte);
        }
        let (&first, rest) = body.split_first().ok_or(SysexError::NoId)?;
        let (id, data) = match first {
            0 => {
                let &[b2, b3, ..] = rest else {
                    return Err(SysexError::Length);
                };
                (manufacturer(&[0, b2, b3])?, &rest[2..])
            }
            NON_COMMERCIAL => (SysexId::NonCommercial, rest),
            NON_REAL_TIME => (SysexId::NonRealTime, rest),
            REAL_TIME => (SysexId::RealTime, rest),
            b => (manufacturer(&[b])?, rest),
        };
        Ok(Self { id, data })
    }
}

/// The input without F0H at the start and without F7H at the end, where they are.
fn strip(bytes: &[u8]) -> &[u8] {
    let bytes = match bytes {
        [SOX, rest @ ..] => rest,
        _ => bytes,
    };
    match bytes {
        [rest @ .., EOX] => rest,
        _ => bytes,
    }
}

fn manufacturer(id: &[u8]) -> Result<SysexId, SysexError> {
    ManufacturerId::from_midi1(id)
        .map(SysexId::Manufacturer)
        .map_err(|_| SysexError::DataByte)
}

/// Writes F0H, the Manufacturer ID, `data` and F7H into `out` and gives the length (REQ-SX-003,
/// REQ-SX-007). Gives [`SysexError::OutOfRange`] for the reserved 3-byte ID 00H 00H 00H (p. 34,
/// AMB-M1-58; the 1-byte 00H is not a `ManufacturerId`) and for the Universal IDs 7EH and 7FH
/// (use [`Universal::write`]); [`SysexError::DataByte`] for a data byte above 7FH.
pub fn write_manufacturer(
    id: ManufacturerId,
    data: &[u8],
    out: &mut [u8],
) -> Result<usize, SysexError> {
    let v = id.value();
    if v == RESERVED_THREE_BYTE || v == NON_REAL_TIME as u16 || v == REAL_TIME as u16 {
        return Err(SysexError::OutOfRange);
    }
    let mut w = Out::new(out);
    w.byte(SOX);
    let bytes = id.to_midi1();
    w.bytes(if id.is_three_byte() {
        &bytes
    } else {
        &bytes[..1]
    });
    w.data(data)?;
    w.finish()
}

/// True when a received Device ID addresses this device: 7FH ([`ALL_DEVICES`]) or the own Device
/// ID `own` (p. 35, AMB-M1-30; REQ-SX-005).
pub const fn addressed(device_id: u8, own: u8) -> bool {
    device_id == ALL_DEVICES || device_id == own
}

/// The Sub-ID#1 values of Universal Non-Real Time (Table VIIa p. T-9, REQ-SX-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum NonRealTimeId {
    /// 01H: Sample Dump Header.
    SampleDumpHeader = 0x01,
    /// 02H: Sample Data Packet.
    SampleDataPacket = 0x02,
    /// 03H: Sample Dump Request.
    SampleDumpRequest = 0x03,
    /// 04H: MIDI Time Code (set-up).
    MidiTimeCode = 0x04,
    /// 05H: Sample Dump Extensions.
    SampleDumpExtensions = 0x05,
    /// 06H: General Information.
    GeneralInformation = 0x06,
    /// 07H: File Dump.
    FileDump = 0x07,
    /// 08H: MIDI Tuning Standard.
    MidiTuning = 0x08,
    /// 09H: General MIDI.
    GeneralMidi = 0x09,
    /// 7BH: End Of File.
    EndOfFile = 0x7B,
    /// 7CH: Wait.
    Wait = 0x7C,
    /// 7DH: Cancel.
    Cancel = 0x7D,
    /// 7EH: NAK.
    Nak = 0x7E,
    /// 7FH: ACK.
    Ack = 0x7F,
}

impl NonRealTimeId {
    /// The 7-bit Sub-ID#1 value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Sub-ID#1 of a 7-bit value, or `None` for a value that Table VIIa does not define.
    pub const fn from_bits(value: u8) -> Option<Self> {
        Some(match value {
            0x01 => Self::SampleDumpHeader,
            0x02 => Self::SampleDataPacket,
            0x03 => Self::SampleDumpRequest,
            0x04 => Self::MidiTimeCode,
            0x05 => Self::SampleDumpExtensions,
            0x06 => Self::GeneralInformation,
            0x07 => Self::FileDump,
            0x08 => Self::MidiTuning,
            0x09 => Self::GeneralMidi,
            0x7B => Self::EndOfFile,
            0x7C => Self::Wait,
            0x7D => Self::Cancel,
            0x7E => Self::Nak,
            0x7F => Self::Ack,
            _ => return None,
        })
    }
}

/// The Sub-ID#1 values of Universal Real Time (Table VIIa p. T-10, REQ-SX-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RealTimeId {
    /// 01H: MIDI Time Code.
    MidiTimeCode = 0x01,
    /// 02H: MIDI Show Control.
    MidiShowControl = 0x02,
    /// 03H: Notation Information.
    NotationInformation = 0x03,
    /// 04H: Device Control.
    DeviceControl = 0x04,
    /// 05H: Real Time MTC Cueing.
    MtcCueing = 0x05,
    /// 06H: MIDI Machine Control Commands.
    MmcCommand = 0x06,
    /// 07H: MIDI Machine Control Responses.
    MmcResponse = 0x07,
    /// 08H: MIDI Tuning Standard.
    MidiTuning = 0x08,
}

impl RealTimeId {
    /// The 7-bit Sub-ID#1 value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Sub-ID#1 of a 7-bit value, or `None` for a value that Table VIIa does not define.
    pub const fn from_bits(value: u8) -> Option<Self> {
        Some(match value {
            0x01 => Self::MidiTimeCode,
            0x02 => Self::MidiShowControl,
            0x03 => Self::NotationInformation,
            0x04 => Self::DeviceControl,
            0x05 => Self::MtcCueing,
            0x06 => Self::MmcCommand,
            0x07 => Self::MmcResponse,
            0x08 => Self::MidiTuning,
            _ => return None,
        })
    }
}

/// True for the families that carry a Sub-ID#2: Non-Real Time 04H to 09H and Real Time 01H, 03H,
/// 04H, 05H and 08H (AMB-M1-29, REQ-SX-004). In the other families the byte after Sub-ID#1 is
/// data: a sample or packet number, the MSC command format or the first MMC command.
pub const fn has_sub_id2(real_time: bool, sub_id1: u8) -> bool {
    if real_time {
        matches!(sub_id1, 0x01 | 0x03 | 0x04 | 0x05 | 0x08)
    } else {
        matches!(sub_id1, 0x04..=0x09)
    }
}

/// The Sub-ID#2 values of the Universal Non-Real Time families of Table VIIa (p. T-9), one module
/// for each Sub-ID#1 (REQ-SX-004). Later documents add values; the part modules name them.
pub mod non_real_time {
    /// Sub-ID#1 04H: MIDI Time Code set-up.
    pub mod midi_time_code {
        /// 00H: Special.
        pub const SPECIAL: u8 = 0x00;
        /// 01H: Punch In Points.
        pub const PUNCH_IN_POINTS: u8 = 0x01;
        /// 02H: Punch Out Points.
        pub const PUNCH_OUT_POINTS: u8 = 0x02;
        /// 03H: Delete Punch In Point.
        pub const DELETE_PUNCH_IN_POINT: u8 = 0x03;
        /// 04H: Delete Punch Out Point.
        pub const DELETE_PUNCH_OUT_POINT: u8 = 0x04;
        /// 05H: Event Start Point.
        pub const EVENT_START_POINT: u8 = 0x05;
        /// 06H: Event Stop Point.
        pub const EVENT_STOP_POINT: u8 = 0x06;
        /// 07H: Event Start Points with additional information.
        pub const EVENT_START_POINTS_INFO: u8 = 0x07;
        /// 08H: Event Stop Points with additional information.
        pub const EVENT_STOP_POINTS_INFO: u8 = 0x08;
        /// 09H: Delete Event Start Point.
        pub const DELETE_EVENT_START_POINT: u8 = 0x09;
        /// 0AH: Delete Event Stop Point.
        pub const DELETE_EVENT_STOP_POINT: u8 = 0x0A;
        /// 0BH: Cue Points.
        pub const CUE_POINTS: u8 = 0x0B;
        /// 0CH: Cue Points with additional information.
        pub const CUE_POINTS_INFO: u8 = 0x0C;
        /// 0DH: Delete Cue Point.
        pub const DELETE_CUE_POINT: u8 = 0x0D;
        /// 0EH: Event Name in additional information.
        pub const EVENT_NAME_INFO: u8 = 0x0E;
    }

    /// Sub-ID#1 05H: Sample Dump Extensions.
    pub mod sample_dump_extensions {
        /// 01H: Multiple Loop Points.
        pub const MULTIPLE_LOOP_POINTS: u8 = 0x01;
        /// 02H: Loop Points Request.
        pub const LOOP_POINTS_REQUEST: u8 = 0x02;
    }

    /// Sub-ID#1 06H: General Information.
    pub mod general_information {
        /// 01H: Identity Request.
        pub const IDENTITY_REQUEST: u8 = 0x01;
        /// 02H: Identity Reply.
        pub const IDENTITY_REPLY: u8 = 0x02;
    }

    /// Sub-ID#1 07H: File Dump.
    pub mod file_dump {
        /// 01H: Header.
        pub const HEADER: u8 = 0x01;
        /// 02H: Data Packet.
        pub const DATA_PACKET: u8 = 0x02;
        /// 03H: Request.
        pub const REQUEST: u8 = 0x03;
    }

    /// Sub-ID#1 08H: MIDI Tuning Standard.
    pub mod midi_tuning {
        /// 00H: Bulk Dump Request.
        pub const BULK_DUMP_REQUEST: u8 = 0x00;
        /// 01H: Bulk Dump Reply.
        pub const BULK_DUMP_REPLY: u8 = 0x01;
    }

    /// Sub-ID#1 09H: General MIDI.
    pub mod general_midi {
        /// 01H: General MIDI System On.
        pub const SYSTEM_ON: u8 = 0x01;
        /// 02H: General MIDI System Off.
        pub const SYSTEM_OFF: u8 = 0x02;
    }
}

/// The Sub-ID#2 values of the Universal Real Time families of Table VIIa (p. T-10), one module for
/// each Sub-ID#1 (REQ-SX-004). Later documents add values; the part modules name them.
pub mod real_time {
    /// Sub-ID#1 01H: MIDI Time Code.
    pub mod midi_time_code {
        /// 01H: Full Message.
        pub const FULL_MESSAGE: u8 = 0x01;
        /// 02H: User Bits.
        pub const USER_BITS: u8 = 0x02;
    }

    /// Sub-ID#1 03H: Notation Information.
    pub mod notation_information {
        /// 01H: Bar Number.
        pub const BAR_NUMBER: u8 = 0x01;
        /// 02H: Time Signature (Immediate).
        pub const TIME_SIGNATURE_IMMEDIATE: u8 = 0x02;
        /// 42H: Time Signature (Delayed).
        pub const TIME_SIGNATURE_DELAYED: u8 = 0x42;
    }

    /// Sub-ID#1 04H: Device Control.
    pub mod device_control {
        /// 01H: Master Volume.
        pub const MASTER_VOLUME: u8 = 0x01;
        /// 02H: Master Balance.
        pub const MASTER_BALANCE: u8 = 0x02;
    }

    /// Sub-ID#1 05H: Real Time MTC Cueing; 03H, 04H, 09H, 0AH and 0DH are reserved.
    pub mod mtc_cueing {
        /// 00H: Special.
        pub const SPECIAL: u8 = 0x00;
        /// 01H: Punch In Points.
        pub const PUNCH_IN_POINTS: u8 = 0x01;
        /// 02H: Punch Out Points.
        pub const PUNCH_OUT_POINTS: u8 = 0x02;
        /// 05H: Event Start Points.
        pub const EVENT_START_POINTS: u8 = 0x05;
        /// 06H: Event Stop Points.
        pub const EVENT_STOP_POINTS: u8 = 0x06;
        /// 07H: Event Start Points with additional information.
        pub const EVENT_START_POINTS_INFO: u8 = 0x07;
        /// 08H: Event Stop Points with additional information.
        pub const EVENT_STOP_POINTS_INFO: u8 = 0x08;
        /// 0BH: Cue Points.
        pub const CUE_POINTS: u8 = 0x0B;
        /// 0CH: Cue Points with additional information.
        pub const CUE_POINTS_INFO: u8 = 0x0C;
        /// 0EH: Event Name in additional information.
        pub const EVENT_NAME_INFO: u8 = 0x0E;
    }

    /// Sub-ID#1 08H: MIDI Tuning Standard.
    pub mod midi_tuning {
        /// 02H: Note Change.
        pub const NOTE_CHANGE: u8 = 0x02;
    }
}

/// The Universal header and its data; it borrows the input bytes (Table VIIa note 1, REQ-SX-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Universal<'a> {
    /// `true` for Real Time (7FH), `false` for Non-Real Time (7EH).
    pub real_time: bool,
    /// The Device ID, 00H to 7FH (REQ-SX-005).
    pub device_id: u8,
    /// Sub-ID#1.
    pub sub_id1: u8,
    /// Sub-ID#2, only in the families of [`has_sub_id2`].
    pub sub_id2: Option<u8>,
    /// The bytes after the header, without F7H.
    pub data: &'a [u8],
}

impl<'a> Universal<'a> {
    /// Reads the Universal header from the data of a Non-Real Time or Real Time message. Gives
    /// `None` for a Manufacturer or Non-Commercial message, and [`SysexError::Length`] for a
    /// message that ends inside the header. Never panics.
    pub fn parse(sysex: &Sysex<'a>) -> Result<Option<Self>, SysexError> {
        let real_time = match sysex.id {
            SysexId::NonRealTime => false,
            SysexId::RealTime => true,
            _ => return Ok(None),
        };
        let &[device_id, sub_id1, ref rest @ ..] = sysex.data else {
            return Err(SysexError::Length);
        };
        let (sub_id2, data) = if has_sub_id2(real_time, sub_id1) {
            let (&s, data) = rest.split_first().ok_or(SysexError::Length)?;
            (Some(s), data)
        } else {
            (None, rest)
        };
        Ok(Some(Self {
            real_time,
            device_id,
            sub_id1,
            sub_id2,
            data,
        }))
    }

    /// Writes the whole message, F0H to F7H, into `out` and gives the length (REQ-SX-007). Gives
    /// [`SysexError::OutOfRange`] for a header value above 7FH or a Sub-ID#2 against the
    /// [`has_sub_id2`] rule, and [`SysexError::DataByte`] for a data byte above 7FH.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, SysexError> {
        write_universal(
            out,
            self.real_time,
            self.device_id,
            self.sub_id1,
            self.sub_id2,
            |w| w.data(self.data),
        )
    }
}

/// A writer into a caller buffer. It counts the length past the end of the buffer, so that
/// [`SysexError::BufferTooSmall`] gives the needed length (REQ-SX-007).
pub(crate) struct Out<'b> {
    buf: &'b mut [u8],
    len: usize,
}

impl<'b> Out<'b> {
    pub(crate) fn new(buf: &'b mut [u8]) -> Self {
        Self { buf, len: 0 }
    }

    pub(crate) fn byte(&mut self, b: u8) {
        if let Some(x) = self.buf.get_mut(self.len) {
            *x = b;
        }
        self.len += 1;
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
    }

    /// A 7-bit field value: [`SysexError::OutOfRange`] above 7FH.
    pub(crate) fn value(&mut self, v: u8) -> Result<(), SysexError> {
        if v > 0x7F {
            return Err(SysexError::OutOfRange);
        }
        self.byte(v);
        Ok(())
    }

    /// Data bytes: [`SysexError::DataByte`] for a byte above 7FH.
    pub(crate) fn data(&mut self, data: &[u8]) -> Result<(), SysexError> {
        if data.iter().any(|&b| b > 0x7F) {
            return Err(SysexError::DataByte);
        }
        self.bytes(data);
        Ok(())
    }

    /// The bytes written after F0H, for a checksum.
    #[cfg(feature = "tuning")]
    pub(crate) fn written(&self) -> &[u8] {
        let end = self.len.min(self.buf.len());
        self.buf.get(1..end).unwrap_or(&[])
    }

    /// Writes F7H and gives the length, or the needed length.
    pub(crate) fn finish(mut self) -> Result<usize, SysexError> {
        self.byte(EOX);
        if self.len > self.buf.len() {
            Err(SysexError::BufferTooSmall { needed: self.len })
        } else {
            Ok(self.len)
        }
    }
}

/// Writes a Universal message: F0H, the class ID, the Device ID, Sub-ID#1, the Sub-ID#2 when the
/// family has one, the body and F7H.
pub(crate) fn write_universal(
    out: &mut [u8],
    real_time: bool,
    device_id: u8,
    sub_id1: u8,
    sub_id2: Option<u8>,
    body: impl FnOnce(&mut Out<'_>) -> Result<(), SysexError>,
) -> Result<usize, SysexError> {
    if sub_id2.is_some() != has_sub_id2(real_time, sub_id1) {
        return Err(SysexError::OutOfRange);
    }
    let mut w = Out::new(out);
    w.byte(SOX);
    w.byte(if real_time { REAL_TIME } else { NON_REAL_TIME });
    w.value(device_id)?;
    w.value(sub_id1)?;
    if let Some(s) = sub_id2 {
        w.value(s)?;
    }
    body(&mut w)?;
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn framing_with_and_without_f0_f7() {
        // REQ-SX-001: the same message with and without F0H and F7H.
        let forms: [&[u8]; 4] = [
            &[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7],
            &[0x7E, 0x7F, 0x06, 0x01],
            &[0xF0, 0x7E, 0x7F, 0x06, 0x01],
            &[0x7E, 0x7F, 0x06, 0x01, 0xF7],
        ];
        for f in forms {
            let s = Sysex::parse(f).unwrap();
            assert_eq!(s.id, SysexId::NonRealTime);
            assert_eq!(s.data, &[0x7F, 0x06, 0x01]);
        }
        assert_eq!(Sysex::parse(&[]), Err(SysexError::NoId));
        assert_eq!(Sysex::parse(&[0xF0, 0xF7]), Err(SysexError::NoId));
        // A byte above 7FH, F0H or F7H inside.
        assert_eq!(Sysex::parse(&[0x43, 0x80]), Err(SysexError::DataByte));
        assert_eq!(
            Sysex::parse(&[0xF0, 0x43, 0xF0, 0x01, 0xF7]),
            Err(SysexError::DataByte)
        );
        assert_eq!(
            Sysex::parse(&[0xF0, 0x43, 0xF7, 0x01, 0xF7]),
            Err(SysexError::DataByte)
        );
    }

    #[test]
    fn prop_reader_never_panics() {
        // REQ-SX-001: any bytes give a message or an error.
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..40))| {
            if let Ok(s) = Sysex::parse(&bytes) {
                let _ = Universal::parse(&s);
            }
        });
    }

    #[test]
    fn ids_follow_table_vii() {
        // REQ-SX-002: Table VII (p. T-8) and the ID rules of p. 34.
        let s = Sysex::parse(&[0x43, 0x10, 0x4C]).unwrap();
        assert_eq!(
            s.id,
            SysexId::Manufacturer(ManufacturerId::from_midi1(&[0x43]).unwrap())
        );
        assert_eq!(s.data, &[0x10, 0x4C]);
        let s = Sysex::parse(&[0x00, 0x20, 0x29, 0x02]).unwrap();
        assert_eq!(
            s.id,
            SysexId::Manufacturer(ManufacturerId::from_midi1(&[0x00, 0x20, 0x29]).unwrap())
        );
        assert_eq!(s.data, &[0x02]);
        assert_eq!(Sysex::parse(&[0x00, 0x20]), Err(SysexError::Length));
        assert_eq!(Sysex::parse(&[0x7D, 1]).unwrap().id, SysexId::NonCommercial);
        assert_eq!(Sysex::parse(&[0x7E]).unwrap().id, SysexId::NonRealTime);
        assert_eq!(Sysex::parse(&[0x7F]).unwrap().id, SysexId::RealTime);
        // A reader gives the reserved 00H 00H 00H as it is (AMB-M1-58).
        let s = Sysex::parse(&[0x00, 0x00, 0x00, 0x05]).unwrap();
        assert!(matches!(s.id, SysexId::Manufacturer(m) if m.value() == 0x8000));
    }

    #[test]
    fn writer_rejects_reserved_manufacturer_ids() {
        // REQ-SX-003: p. 34 "00 and 00 00 00 are not to be used" (AMB-M1-58).
        let mut out = [0u8; 16];
        let zero3 = ManufacturerId::from_midi1(&[0, 0, 0]).unwrap();
        assert_eq!(
            write_manufacturer(zero3, &[1], &mut out),
            Err(SysexError::OutOfRange)
        );
        for u in [
            ManufacturerId::UNIVERSAL_NON_REAL_TIME,
            ManufacturerId::UNIVERSAL_REAL_TIME,
        ] {
            assert_eq!(
                write_manufacturer(u, &[1], &mut out),
                Err(SysexError::OutOfRange)
            );
        }
        let id = ManufacturerId::from_midi1(&[0x43]).unwrap();
        assert_eq!(write_manufacturer(id, &[0x10, 0x4C], &mut out), Ok(5));
        assert_eq!(&out[..5], &[0xF0, 0x43, 0x10, 0x4C, 0xF7]);
        let id = ManufacturerId::from_midi1(&[0x00, 0x20, 0x29]).unwrap();
        assert_eq!(write_manufacturer(id, &[2], &mut out), Ok(6));
        assert_eq!(&out[..6], &[0xF0, 0x00, 0x20, 0x29, 0x02, 0xF7]);
        assert_eq!(
            write_manufacturer(id, &[0x80], &mut out),
            Err(SysexError::DataByte)
        );
    }

    #[test]
    fn universal_header_follows_table_viia() {
        // REQ-SX-004: the header, with a Sub-ID#2 only in the families of AMB-M1-29.
        let s = Sysex::parse(&[0xF0, 0x7E, 0x10, 0x06, 0x02, 0x43, 0xF7]).unwrap();
        let u = Universal::parse(&s).unwrap().unwrap();
        assert_eq!(
            (u.real_time, u.device_id, u.sub_id1, u.sub_id2, u.data),
            (false, 0x10, 0x06, Some(0x02), &[0x43][..])
        );
        // Sample Dump Request (03H) and ACK (7FH): the byte after Sub-ID#1 is data.
        let s = Sysex::parse(&[0x7E, 0x00, 0x03, 0x05, 0x00]).unwrap();
        let u = Universal::parse(&s).unwrap().unwrap();
        assert_eq!((u.sub_id2, u.data), (None, &[0x05, 0x00][..]));
        // MMC (Real Time 06H): the first command follows Sub-ID#1.
        let s = Sysex::parse(&[0x7F, 0x7F, 0x06, 0x02]).unwrap();
        let u = Universal::parse(&s).unwrap().unwrap();
        assert_eq!((u.real_time, u.sub_id2, u.data), (true, None, &[0x02][..]));
        for (rt, id, has) in [
            (false, 0x03, false),
            (false, 0x04, true),
            (false, 0x09, true),
            (false, 0x0A, false),
            (false, 0x7F, false),
            (true, 0x01, true),
            (true, 0x02, false),
            (true, 0x03, true),
            (true, 0x05, true),
            (true, 0x06, false),
            (true, 0x07, false),
            (true, 0x08, true),
        ] {
            assert_eq!(has_sub_id2(rt, id), has, "{rt} {id:02X}");
        }
        // A header that ends early; a manufacturer message is not Universal.
        assert_eq!(
            Universal::parse(&Sysex::parse(&[0x7E, 0x7F]).unwrap()),
            Err(SysexError::Length)
        );
        assert_eq!(
            Universal::parse(&Sysex::parse(&[0x7E, 0x7F, 0x06]).unwrap()),
            Err(SysexError::Length)
        );
        assert_eq!(
            Universal::parse(&Sysex::parse(&[0x43, 1]).unwrap()),
            Ok(None)
        );
        // Table VIIa names.
        for v in 0..=0x7Fu8 {
            if let Some(n) = NonRealTimeId::from_bits(v) {
                assert_eq!(n.into_bits(), v);
            }
            if let Some(r) = RealTimeId::from_bits(v) {
                assert_eq!(r.into_bits(), v);
            }
        }
        assert_eq!(
            NonRealTimeId::from_bits(0x09),
            Some(NonRealTimeId::GeneralMidi)
        );
        assert_eq!(NonRealTimeId::from_bits(0x0D), None);
        assert_eq!(RealTimeId::from_bits(0x04), Some(RealTimeId::DeviceControl));
        assert_eq!(RealTimeId::from_bits(0x09), None);
        // Writer round trip.
        let mut out = [0u8; 8];
        let n = u.write(&mut out).unwrap();
        assert_eq!(&out[..n], &[0xF0, 0x7F, 0x7F, 0x06, 0x02, 0xF7]);
        let bad = Universal {
            sub_id2: Some(1),
            ..u
        };
        assert_eq!(bad.write(&mut out), Err(SysexError::OutOfRange));
    }

    #[test]
    fn device_id_all_call_and_own() {
        // REQ-SX-005: p. 35, AMB-M1-30.
        assert!(addressed(ALL_DEVICES, 0x10));
        assert!(addressed(0x10, 0x10));
        assert!(!addressed(0x11, 0x10));
        assert_eq!(ALL_DEVICES, 0x7F);
    }

    #[test]
    fn writers_report_overflow() {
        // REQ-SX-007: the needed length of the whole message, and a value out of range.
        let u = Universal {
            real_time: false,
            device_id: 0x7F,
            sub_id1: 0x06,
            sub_id2: Some(0x01),
            data: &[],
        };
        let mut out = [0u8; 6];
        assert_eq!(u.write(&mut out), Ok(6));
        assert_eq!(
            u.write(&mut out[..5]),
            Err(SysexError::BufferTooSmall { needed: 6 })
        );
        assert_eq!(
            u.write(&mut []),
            Err(SysexError::BufferTooSmall { needed: 6 })
        );
        let id = ManufacturerId::from_midi1(&[0x43]).unwrap();
        assert_eq!(
            write_manufacturer(id, &[1, 2, 3], &mut out[..4]),
            Err(SysexError::BufferTooSmall { needed: 6 })
        );
        let big = Universal {
            device_id: 0x80,
            ..u
        };
        assert_eq!(big.write(&mut out), Err(SysexError::OutOfRange));
        let data = Universal { data: &[0x80], ..u };
        assert_eq!(data.write(&mut out), Err(SysexError::DataByte));
    }
}
