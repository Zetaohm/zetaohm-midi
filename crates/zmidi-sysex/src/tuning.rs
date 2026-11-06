//! The MIDI Tuning messages (M1 v4.2.1 pp. 47 to 51; the updated MIDI Tuning document with CA-020
//! and CA-021; Universal Sub-ID#1 08H; DEC-022). Feature `tuning`.
//!
//! - [`FrequencyWord`]: the semitone below the frequency (7 bits) and the fraction above it in
//!   1/16384 of 100 cents (14 bits, MSB first); 7FH 7FH 7FH is "no change" (REQ-TUN-001).
//! - Non-Real Time: 00H Bulk Tuning Dump Request and 03H with a bank (REQ-TUN-002); 01H Bulk
//!   Tuning Dump and 04H Key-Based Tuning Dump (REQ-TUN-003); 05H and 06H Scale/Octave Tuning
//!   Dumps, 1-byte and 2-byte format (REQ-TUN-006).
//! - Real Time 02H Single Note Tuning Change; 07H with a bank in both forms (AMB-MTS-06;
//!   REQ-TUN-005). 08H and 09H Scale/Octave Tuning 1-byte and 2-byte forms in both forms, with the
//!   3-byte channel field: `hh` channels 1 to 7, `gg` 8 to 14, `ff` bits 0 and 1 channels 15 and
//!   16, and 5 reserved bits (AMB-MTS-07; REQ-TUN-007).
//! - The checksum of a dump is the XOR of all bytes after F0H and before it, AND 7FH. Each writer
//!   writes it. The reader rejects a bad checksum of 04H, 05H and 06H, and gives the result of
//!   the check for 01H (AMB-M1-46, AMB-MTS-02; REQ-TUN-004).
//!
//! No alloc (DEC-002). Never panics.

use crate::universal::{write_universal, NonRealTimeId, Out, SysexError, Universal};

/// Sub-ID#1 08H: MIDI Tuning Standard.
const MIDI_TUNING: u8 = NonRealTimeId::MidiTuning.into_bits();
/// The Sub-ID#2 values of the MIDI Tuning messages.
const DUMP_REQUEST: u8 = 0x00;
const BULK_DUMP: u8 = 0x01;
const NOTE_CHANGE: u8 = 0x02;
const DUMP_REQUEST_BANK: u8 = 0x03;
const KEY_DUMP: u8 = 0x04;
const SCALE_DUMP_1: u8 = 0x05;
const SCALE_DUMP_2: u8 = 0x06;
const NOTE_CHANGE_BANK: u8 = 0x07;
const SCALE_OCTAVE_1: u8 = 0x08;
const SCALE_OCTAVE_2: u8 = 0x09;
/// The length of a tuning name.
const NAME: usize = 16;
/// The number of keys of a key dump.
const KEYS: usize = 128;

/// The 3-byte frequency word (REQ-TUN-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrequencyWord {
    /// The equal-tempered semitone below the frequency, 0 to 127 (MIDI note numbers).
    pub semitone: u8,
    /// The fraction of 100 cents above the semitone, in 1/16384 units, 0 to 3FFFH.
    pub fraction: u16,
}

impl FrequencyWord {
    /// 7FH 7FH 7FH: "no change", not a frequency.
    pub const NO_CHANGE: Self = Self {
        semitone: 0x7F,
        fraction: 0x3FFF,
    };

    /// The word of three bytes; [`SysexError::DataByte`] for a byte above 7FH.
    pub const fn from_bytes(bytes: [u8; 3]) -> Result<Self, SysexError> {
        let [s, f1, f2] = bytes;
        if (s | f1 | f2) > 0x7F {
            return Err(SysexError::DataByte);
        }
        Ok(Self {
            semitone: s,
            fraction: (f1 as u16) << 7 | f2 as u16,
        })
    }

    /// The three bytes: the semitone, then the fraction MSB first.
    pub const fn to_bytes(self) -> [u8; 3] {
        [
            self.semitone & 0x7F,
            (self.fraction >> 7) as u8 & 0x7F,
            self.fraction as u8 & 0x7F,
        ]
    }

    /// True for the "no change" word 7FH 7FH 7FH.
    pub const fn is_no_change(self) -> bool {
        self.semitone == 0x7F && self.fraction == 0x3FFF
    }

    const fn valid(self) -> bool {
        self.semitone <= 0x7F && self.fraction <= 0x3FFF
    }
}

/// The 12 scale/octave offsets from C to B (REQ-TUN-006, REQ-TUN-007).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScaleOffsets {
    /// 1-byte format: cents, -64 to +63 (the byte minus 64).
    OneByte([i8; 12]),
    /// 2-byte format: the signed offset from 8192 of a 14-bit value sent MSB first, -8192 to
    /// +8191, in steps of 100/8192 cent (AMB-MTS-08).
    TwoByte([i16; 12]),
}

impl ScaleOffsets {
    fn read(two_byte: bool, bytes: &[u8]) -> Self {
        if two_byte {
            let mut o = [0i16; 12];
            for (x, &[msb, lsb]) in o.iter_mut().zip(bytes.as_chunks::<2>().0) {
                *x = ((msb as i16) << 7 | lsb as i16) - 8192;
            }
            Self::TwoByte(o)
        } else {
            let mut o = [0i8; 12];
            for (x, b) in o.iter_mut().zip(bytes) {
                *x = *b as i8 - 64;
            }
            Self::OneByte(o)
        }
    }

    fn write(&self, w: &mut Out<'_>) -> Result<(), SysexError> {
        match self {
            Self::OneByte(o) => {
                for &x in o {
                    if !(-64..=63).contains(&x) {
                        return Err(SysexError::OutOfRange);
                    }
                    w.byte((x + 64) as u8);
                }
            }
            Self::TwoByte(o) => {
                for &x in o {
                    if !(-8192..=8191).contains(&x) {
                        return Err(SysexError::OutOfRange);
                    }
                    let v = (x + 8192) as u16;
                    w.bytes(&[(v >> 7) as u8, (v & 0x7F) as u8]);
                }
            }
        }
        Ok(())
    }

    const fn two_byte(&self) -> bool {
        matches!(self, Self::TwoByte(_))
    }
}

/// One MIDI Tuning message; it borrows the message bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuningMessage<'a> {
    /// 00H Bulk Tuning Dump Request, or 03H with a bank (REQ-TUN-002).
    DumpRequest {
        /// The Device ID.
        device_id: u8,
        /// The bank of 03H, `None` for 00H.
        bank: Option<u8>,
        /// The tuning program, 0 to 127.
        program: u8,
    },
    /// 01H Bulk Tuning Dump, or 04H Key-Based Tuning Dump with a bank (REQ-TUN-003).
    KeyDump {
        /// The Device ID.
        device_id: u8,
        /// The bank of 04H, `None` for 01H.
        bank: Option<u8>,
        /// The tuning program, 0 to 127.
        program: u8,
        /// The 16-byte tuning name.
        name: &'a [u8; 16],
        /// The 128 frequency words, key 0 first.
        words: FrequencyWords<'a>,
        /// The result of the checksum check. Always `true` for 04H, which the reader rejects
        /// when the check fails; 01H gives it to the caller (AMB-M1-46, REQ-TUN-004).
        checksum_ok: bool,
    },
    /// Real Time 02H Single Note Tuning Change, or 07H with a bank in either form
    /// (REQ-TUN-005).
    NoteChange {
        /// The header type: `true` for Real Time (7FH).
        real_time: bool,
        /// The Device ID.
        device_id: u8,
        /// The bank of 07H, `None` for 02H.
        bank: Option<u8>,
        /// The tuning program, 0 to 127.
        program: u8,
        /// The changes, 1 to 127 sets of key and frequency word.
        changes: NoteChanges<'a>,
    },
    /// 05H (1-byte) or 06H (2-byte) Scale/Octave Tuning Dump (REQ-TUN-006).
    ScaleDump {
        /// The Device ID.
        device_id: u8,
        /// The bank, 0 to 127.
        bank: u8,
        /// The tuning program, 0 to 127.
        program: u8,
        /// The 16-byte tuning name.
        name: &'a [u8; 16],
        /// The 12 offsets.
        offsets: ScaleOffsets,
    },
    /// 08H (1-byte) or 09H (2-byte) Scale/Octave Tuning, in either form (REQ-TUN-007).
    ScaleOctave {
        /// The header type: `true` for Real Time (7FH).
        real_time: bool,
        /// The Device ID.
        device_id: u8,
        /// The channels: bit n is the channel with packet value n.
        channels: u16,
        /// The 12 offsets.
        offsets: ScaleOffsets,
    },
}

impl<'a> TuningMessage<'a> {
    /// Reads the message from a Universal header (Sub-ID#1 08H). Gives `None` for another
    /// message, or a Sub-ID#2 that the class does not define; [`SysexError::Length`] for a wrong
    /// length or a count that does not match the sets; [`SysexError::Checksum`] for a bad checksum
    /// of 04H, 05H or 06H.
    pub fn from_universal(u: &Universal<'a>) -> Result<Option<Self>, SysexError> {
        if u.sub_id1 != MIDI_TUNING {
            return Ok(None);
        }
        let Some(sub) = u.sub_id2 else {
            return Ok(None);
        };
        let (rt, d, device_id) = (u.real_time, u.data, u.device_id);
        let m = match (rt, sub) {
            (false, DUMP_REQUEST) => {
                let &[program] = d else {
                    return Err(SysexError::Length);
                };
                Self::DumpRequest {
                    device_id,
                    bank: None,
                    program,
                }
            }
            (false, DUMP_REQUEST_BANK) => {
                let &[bank, program] = d else {
                    return Err(SysexError::Length);
                };
                Self::DumpRequest {
                    device_id,
                    bank: Some(bank),
                    program,
                }
            }
            (false, BULK_DUMP | KEY_DUMP) => {
                let banked = sub == KEY_DUMP;
                let head = usize::from(banked) + 1;
                if d.len() != head + NAME + 3 * KEYS + 1 {
                    return Err(SysexError::Length);
                }
                let ok = checksum_ok(u);
                if banked && !ok {
                    return Err(SysexError::Checksum);
                }
                Self::KeyDump {
                    device_id,
                    bank: banked.then(|| d[0]),
                    program: d[head - 1],
                    name: name(&d[head..])?,
                    words: FrequencyWords {
                        bytes: &d[head + NAME..d.len() - 1],
                    },
                    checksum_ok: ok,
                }
            }
            (true, NOTE_CHANGE) | (_, NOTE_CHANGE_BANK) => {
                let banked = sub == NOTE_CHANGE_BANK;
                let head = usize::from(banked) + 2;
                let Some(&count) = d.get(head - 1) else {
                    return Err(SysexError::Length);
                };
                if count == 0 || d.len() != head + 4 * count as usize {
                    return Err(SysexError::Length);
                }
                Self::NoteChange {
                    real_time: rt,
                    device_id,
                    bank: banked.then(|| d[0]),
                    program: d[head - 2],
                    changes: NoteChanges { bytes: &d[head..] },
                }
            }
            (false, SCALE_DUMP_1 | SCALE_DUMP_2) => {
                let two = sub == SCALE_DUMP_2;
                let size = if two { 24 } else { 12 };
                if d.len() != 2 + NAME + size + 1 {
                    return Err(SysexError::Length);
                }
                if !checksum_ok(u) {
                    return Err(SysexError::Checksum);
                }
                Self::ScaleDump {
                    device_id,
                    bank: d[0],
                    program: d[1],
                    name: name(&d[2..])?,
                    offsets: ScaleOffsets::read(two, &d[2 + NAME..d.len() - 1]),
                }
            }
            (_, SCALE_OCTAVE_1 | SCALE_OCTAVE_2) => {
                let two = sub == SCALE_OCTAVE_2;
                let size = if two { 24 } else { 12 };
                let &[ff, gg, hh, ref rest @ ..] = d else {
                    return Err(SysexError::Length);
                };
                if rest.len() != size {
                    return Err(SysexError::Length);
                }
                // The 5 reserved bits of ff are ignored (AMB-MTS-07).
                let channels = hh as u16 | (gg as u16) << 7 | ((ff & 0x03) as u16) << 14;
                Self::ScaleOctave {
                    real_time: rt,
                    device_id,
                    channels,
                    offsets: ScaleOffsets::read(two, rest),
                }
            }
            _ => return Ok(None),
        };
        Ok(Some(m))
    }
}

fn name(bytes: &[u8]) -> Result<&[u8; 16], SysexError> {
    bytes
        .get(..NAME)
        .and_then(|n| n.try_into().ok())
        .ok_or(SysexError::Length)
}

/// True when the last data byte is the checksum of the bytes after F0H and before it.
fn checksum_ok(u: &Universal<'_>) -> bool {
    let Some((&sum, body)) = u.data.split_last() else {
        return false;
    };
    let id = if u.real_time { 0x7F } else { 0x7E };
    let head = id ^ u.device_id ^ u.sub_id1 ^ u.sub_id2.unwrap_or(0);
    (head ^ checksum(body)) & 0x7F == sum
}

/// The XOR of `bytes`, AND 7FH: the checksum of a dump over the bytes after F0H and before the
/// checksum (REQ-TUN-004).
pub const fn checksum(bytes: &[u8]) -> u8 {
    let mut x = 0;
    let mut i = 0;
    while i < bytes.len() {
        x ^= bytes[i];
        i += 1;
    }
    x & 0x7F
}

/// The 128 frequency words of a key dump, key 0 first (REQ-TUN-003).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrequencyWords<'a> {
    bytes: &'a [u8],
}

impl Iterator for FrequencyWords<'_> {
    type Item = FrequencyWord;

    fn next(&mut self) -> Option<FrequencyWord> {
        let (&[s, f1, f2], rest) = self.bytes.split_first_chunk::<3>()?;
        self.bytes = rest;
        Some(FrequencyWord {
            semitone: s,
            fraction: (f1 as u16) << 7 | f2 as u16,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.bytes.len() / 3;
        (n, Some(n))
    }
}

impl ExactSizeIterator for FrequencyWords<'_> {}

/// The sets of key and frequency word of a Single Note Tuning Change (REQ-TUN-005).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteChanges<'a> {
    bytes: &'a [u8],
}

impl Iterator for NoteChanges<'_> {
    type Item = (u8, FrequencyWord);

    fn next(&mut self) -> Option<(u8, FrequencyWord)> {
        let (&[k, s, f1, f2], rest) = self.bytes.split_first_chunk::<4>()?;
        self.bytes = rest;
        let word = FrequencyWord {
            semitone: s,
            fraction: (f1 as u16) << 7 | f2 as u16,
        };
        Some((k, word))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.bytes.len() / 4;
        (n, Some(n))
    }
}

impl ExactSizeIterator for NoteChanges<'_> {}

/// Writes 00H Bulk Tuning Dump Request, or 03H with `bank`, into `out` and gives the length
/// (REQ-TUN-002, REQ-SX-007). Gives [`SysexError::OutOfRange`] for a value above 7FH.
pub fn write_dump_request(
    device_id: u8,
    bank: Option<u8>,
    program: u8,
    out: &mut [u8],
) -> Result<usize, SysexError> {
    let sub = if bank.is_some() {
        DUMP_REQUEST_BANK
    } else {
        DUMP_REQUEST
    };
    write_universal(out, false, device_id, MIDI_TUNING, Some(sub), |w| {
        if let Some(b) = bank {
            w.value(b)?;
        }
        w.value(program)
    })
}

/// Writes 01H Bulk Tuning Dump, or 04H Key-Based Tuning Dump with `bank`, with the checksum,
/// into `out` and gives the length (REQ-TUN-003, REQ-TUN-004). "No change" is allowed for any
/// key. Gives [`SysexError::OutOfRange`] for a value outside its field and
/// [`SysexError::DataByte`] for a name byte above 7FH.
pub fn write_key_dump(
    device_id: u8,
    bank: Option<u8>,
    program: u8,
    name: &[u8; 16],
    words: &[FrequencyWord; 128],
    out: &mut [u8],
) -> Result<usize, SysexError> {
    let sub = if bank.is_some() { KEY_DUMP } else { BULK_DUMP };
    write_universal(out, false, device_id, MIDI_TUNING, Some(sub), |w| {
        if let Some(b) = bank {
            w.value(b)?;
        }
        w.value(program)?;
        w.data(name)?;
        for word in words {
            if !word.valid() {
                return Err(SysexError::OutOfRange);
            }
            w.bytes(&word.to_bytes());
        }
        w.byte(checksum(w.written()));
        Ok(())
    })
}

/// Writes a Single Note Tuning Change into `out` and gives the length (REQ-TUN-005): Real Time
/// 02H without `bank`, or 07H with `bank` in the form of `real_time`. Gives
/// [`SysexError::OutOfRange`] for 1 to 127 sets not given, a Non-Real Time message without a
/// bank, or a value outside its field.
pub fn write_note_change(
    real_time: bool,
    device_id: u8,
    bank: Option<u8>,
    program: u8,
    changes: &[(u8, FrequencyWord)],
    out: &mut [u8],
) -> Result<usize, SysexError> {
    if changes.is_empty() || changes.len() > 127 || (!real_time && bank.is_none()) {
        return Err(SysexError::OutOfRange);
    }
    let sub = if bank.is_some() {
        NOTE_CHANGE_BANK
    } else {
        NOTE_CHANGE
    };
    write_universal(out, real_time, device_id, MIDI_TUNING, Some(sub), |w| {
        if let Some(b) = bank {
            w.value(b)?;
        }
        w.value(program)?;
        w.byte(changes.len() as u8);
        for &(key, word) in changes {
            if !word.valid() {
                return Err(SysexError::OutOfRange);
            }
            w.value(key)?;
            w.bytes(&word.to_bytes());
        }
        Ok(())
    })
}

/// Writes a Scale/Octave Tuning Dump, 05H for [`ScaleOffsets::OneByte`] or 06H for
/// [`ScaleOffsets::TwoByte`], with the checksum, into `out` and gives the length (REQ-TUN-006,
/// REQ-TUN-004). Gives [`SysexError::OutOfRange`] for a value outside its field.
pub fn write_scale_dump(
    device_id: u8,
    bank: u8,
    program: u8,
    name: &[u8; 16],
    offsets: &ScaleOffsets,
    out: &mut [u8],
) -> Result<usize, SysexError> {
    let sub = if offsets.two_byte() {
        SCALE_DUMP_2
    } else {
        SCALE_DUMP_1
    };
    write_universal(out, false, device_id, MIDI_TUNING, Some(sub), |w| {
        w.value(bank)?;
        w.value(program)?;
        w.data(name)?;
        offsets.write(w)?;
        w.byte(checksum(w.written()));
        Ok(())
    })
}

/// Writes a Scale/Octave Tuning message, 08H for [`ScaleOffsets::OneByte`] or 09H for
/// [`ScaleOffsets::TwoByte`], in the form of `real_time`, into `out` and gives the length
/// (REQ-TUN-007). Bit n of `channels` is the channel with packet value n; the 5 reserved bits of
/// the channel field are written as 0. Gives [`SysexError::OutOfRange`] for an offset outside its
/// field.
pub fn write_scale_octave(
    real_time: bool,
    device_id: u8,
    channels: u16,
    offsets: &ScaleOffsets,
    out: &mut [u8],
) -> Result<usize, SysexError> {
    let sub = if offsets.two_byte() {
        SCALE_OCTAVE_2
    } else {
        SCALE_OCTAVE_1
    };
    write_universal(out, real_time, device_id, MIDI_TUNING, Some(sub), |w| {
        w.bytes(&[
            (channels >> 14) as u8,
            (channels >> 7) as u8 & 0x7F,
            channels as u8 & 0x7F,
        ]);
        offsets.write(w)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universal::Sysex;
    use proptest::prelude::*;

    fn read(bytes: &[u8]) -> Result<Option<TuningMessage<'_>>, SysexError> {
        let s = Sysex::parse(bytes).unwrap();
        TuningMessage::from_universal(&Universal::parse(&s).unwrap().unwrap())
    }

    const NAME_BYTES: &[u8; 16] = b"Just intonation ";

    #[test]
    fn frequency_word_examples() {
        // REQ-TUN-001: the examples of the MIDI Tuning document (section 1).
        let w = |b| FrequencyWord::from_bytes(b).unwrap();
        assert_eq!(
            w([0x00, 0x00, 0x00]),
            FrequencyWord {
                semitone: 0,
                fraction: 0
            }
        );
        assert_eq!(w([0x3C, 0x00, 0x00]).semitone, 60); // middle C
        assert_eq!(
            w([0x45, 0x00, 0x00]),
            FrequencyWord {
                semitone: 69,
                fraction: 0
            }
        ); // A-440
        assert_eq!(
            w([0x44, 0x7F, 0x7F]),
            FrequencyWord {
                semitone: 68,
                fraction: 0x3FFF
            }
        );
        assert_eq!(w([0x7F, 0x7F, 0x7E]).fraction, 0x3FFE); // top of range
        assert!(w([0x7F, 0x7F, 0x7F]).is_no_change());
        assert!(!w([0x7F, 0x7F, 0x7E]).is_no_change());
        assert_eq!(FrequencyWord::NO_CHANGE.to_bytes(), [0x7F, 0x7F, 0x7F]);
        assert_eq!(
            FrequencyWord::from_bytes([0x80, 0, 0]),
            Err(SysexError::DataByte)
        );
    }

    #[test]
    fn prop_frequency_word_round_trip() {
        // REQ-TUN-001: every 7-bit byte triple is a word, and back.
        proptest!(|(s in 0u8..0x80, f1 in 0u8..0x80, f2 in 0u8..0x80)| {
            let w = FrequencyWord::from_bytes([s, f1, f2]).unwrap();
            prop_assert_eq!(w.to_bytes(), [s, f1, f2]);
            prop_assert_eq!(w.fraction, (f1 as u16) << 7 | f2 as u16);
        });
    }

    #[test]
    fn dump_requests_with_and_without_bank() {
        // REQ-TUN-002: "F0 7E <device ID> 08 00 tt F7" and "F0 7E <device ID> 08 03 bb tt F7"
        // (AMB-MTS-01: the bank form has a Device ID).
        let mut out = [0u8; 8];
        let n = write_dump_request(0x10, None, 5, &mut out).unwrap();
        assert_eq!(&out[..n], &[0xF0, 0x7E, 0x10, 0x08, 0x00, 0x05, 0xF7]);
        assert_eq!(
            read(&out[..n]),
            Ok(Some(TuningMessage::DumpRequest {
                device_id: 0x10,
                bank: None,
                program: 5
            }))
        );
        let n = write_dump_request(0x10, Some(2), 127, &mut out).unwrap();
        assert_eq!(&out[..n], &[0xF0, 0x7E, 0x10, 0x08, 0x03, 0x02, 0x7F, 0xF7]);
        assert_eq!(
            read(&out[..n]),
            Ok(Some(TuningMessage::DumpRequest {
                device_id: 0x10,
                bank: Some(2),
                program: 127
            }))
        );
        assert_eq!(read(&[0x7E, 0x10, 0x08, 0x00]), Err(SysexError::Length));
        assert_eq!(read(&[0x7E, 0x10, 0x08, 0x03, 1]), Err(SysexError::Length));
        assert_eq!(
            write_dump_request(0x10, Some(0x80), 0, &mut out),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            write_dump_request(0x10, None, 0x80, &mut out),
            Err(SysexError::OutOfRange)
        );
        // A Real Time dump request is not defined: raw.
        assert_eq!(read(&[0x7F, 0x10, 0x08, 0x00, 5]), Ok(None));
    }

    fn words() -> [FrequencyWord; 128] {
        core::array::from_fn(|k| {
            if k == 127 {
                FrequencyWord::NO_CHANGE
            } else {
                FrequencyWord {
                    semitone: k as u8,
                    fraction: (k as u16) * 64,
                }
            }
        })
    }

    #[test]
    fn key_based_dumps_round_trip() {
        // REQ-TUN-003: 01H and 04H, 128 words in key order, "no change" for any key.
        let mut out = [0u8; 420];
        let ws = words();
        for bank in [None, Some(3)] {
            let n = write_key_dump(0x7F, bank, 9, NAME_BYTES, &ws, &mut out).unwrap();
            assert_eq!(n, 6 + usize::from(bank.is_some()) + 1 + 16 + 384 + 1);
            let Some(TuningMessage::KeyDump {
                device_id,
                bank: b,
                program,
                name,
                words,
                checksum_ok,
            }) = read(&out[..n]).unwrap()
            else {
                panic!("not a key dump");
            };
            assert_eq!((device_id, b, program, name), (0x7F, bank, 9, NAME_BYTES));
            assert!(checksum_ok);
            assert_eq!(words.len(), 128);
            assert!(words.eq(ws.iter().copied()));
        }
        // A wrong length.
        let n = write_key_dump(0x7F, None, 9, NAME_BYTES, &ws, &mut out).unwrap();
        let mut short = out[..n].to_vec();
        short.remove(30);
        assert_eq!(read(&short), Err(SysexError::Length));
        let bad = FrequencyWord {
            semitone: 0,
            fraction: 0x4000,
        };
        let mut ws2 = ws;
        ws2[5] = bad;
        assert_eq!(
            write_key_dump(0x7F, None, 9, NAME_BYTES, &ws2, &mut out),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            write_key_dump(0x7F, None, 9, NAME_BYTES, &ws, &mut out[..100]),
            Err(SysexError::BufferTooSmall { needed: 408 })
        );
    }

    #[test]
    fn checksum_rules() {
        // REQ-TUN-004: the XOR of all bytes after F0H and before the checksum, AND 7FH; 04H, 05H
        // and 06H are rejected with a bad checksum, 01H gives the result (AMB-M1-46,
        // AMB-MTS-02).
        assert_eq!(checksum(&[0x7E, 0x00, 0x08, 0x05]), 0x7E ^ 0x08 ^ 0x05);
        assert_eq!(checksum(&[0xFF]), 0x7F);
        let mut out = [0u8; 420];
        let offsets = ScaleOffsets::OneByte([0; 12]);
        let n = write_scale_dump(0x00, 1, 2, NAME_BYTES, &offsets, &mut out).unwrap();
        assert_eq!(out[n - 2], checksum(&out[1..n - 2]));
        out[n - 2] ^= 1;
        assert_eq!(read(&out[..n]), Err(SysexError::Checksum));
        let ws = words();
        let n = write_key_dump(0x00, Some(1), 2, NAME_BYTES, &ws, &mut out).unwrap();
        assert_eq!(out[n - 2], checksum(&out[1..n - 2]));
        out[n - 2] ^= 1;
        assert_eq!(read(&out[..n]), Err(SysexError::Checksum));
        let n = write_key_dump(0x00, None, 2, NAME_BYTES, &ws, &mut out).unwrap();
        assert_eq!(out[n - 2], checksum(&out[1..n - 2]));
        out[n - 2] ^= 1;
        let Some(TuningMessage::KeyDump { checksum_ok, .. }) = read(&out[..n]).unwrap() else {
            panic!("not a key dump");
        };
        assert!(!checksum_ok);
    }

    #[test]
    fn single_note_changes() {
        // REQ-TUN-005: "F0 7F <device ID> 08 02 tt ll [kk xx yy zz] F7", 8 + 4 × ll bytes; 07H
        // with a bank in both forms (AMB-MTS-06).
        let a440 = FrequencyWord {
            semitone: 0x45,
            fraction: 0,
        };
        let changes = [(69u8, a440), (70, FrequencyWord::NO_CHANGE)];
        let mut out = [0u8; 32];
        let n = write_note_change(true, 0x7F, None, 0, &changes, &mut out).unwrap();
        assert_eq!(n, 8 + 4 * 2);
        assert_eq!(
            &out[..n],
            &[
                0xF0, 0x7F, 0x7F, 0x08, 0x02, 0x00, 0x02, 69, 0x45, 0, 0, 70, 0x7F, 0x7F, 0x7F,
                0xF7
            ]
        );
        let Some(TuningMessage::NoteChange {
            real_time,
            bank,
            changes: c,
            ..
        }) = read(&out[..n]).unwrap()
        else {
            panic!("not a note change");
        };
        assert_eq!((real_time, bank), (true, None));
        assert!(c.eq(changes.iter().copied()));
        for rt in [true, false] {
            let n = write_note_change(rt, 0x7F, Some(4), 1, &changes, &mut out).unwrap();
            assert_eq!(out[1], if rt { 0x7F } else { 0x7E });
            let Some(TuningMessage::NoteChange {
                real_time, bank, ..
            }) = read(&out[..n]).unwrap()
            else {
                panic!("not a note change");
            };
            assert_eq!((real_time, bank), (rt, Some(4)));
        }
        // The count does not match, a count of 0, and the Non-Real Time 02H.
        assert_eq!(
            read(&[0x7F, 0x7F, 0x08, 0x02, 0, 2, 69, 0x45, 0, 0]),
            Err(SysexError::Length)
        );
        assert_eq!(
            read(&[0x7F, 0x7F, 0x08, 0x02, 0, 0]),
            Err(SysexError::Length)
        );
        assert_eq!(
            read(&[0x7E, 0x7F, 0x08, 0x02, 0, 1, 69, 0x45, 0, 0]),
            Ok(None)
        );
        assert_eq!(
            write_note_change(true, 0x7F, None, 0, &[], &mut out),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            write_note_change(false, 0x7F, None, 0, &changes, &mut out),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            write_note_change(true, 0x7F, None, 0, &[(0x80, a440)], &mut out),
            Err(SysexError::OutOfRange)
        );
    }

    #[test]
    fn scale_octave_dumps() {
        // REQ-TUN-006: 05H "00H means -64 Cent, 40H +/- 0, 7FH +63"; 06H "00H 00H means -100
        // cents, 40H 00H 0 cents, 7FH 7FH +100 cents" (AMB-MTS-08).
        let mut out = [0u8; 64];
        let mut one = [0i8; 12];
        one[0] = -64;
        one[11] = 63;
        let o = ScaleOffsets::OneByte(one);
        let n = write_scale_dump(0x7F, 0, 1, NAME_BYTES, &o, &mut out).unwrap();
        assert_eq!(n, 6 + 2 + 16 + 12 + 1);
        assert_eq!((out[23], out[24], out[34]), (0x00, 0x40, 0x7F));
        assert_eq!(
            read(&out[..n]),
            Ok(Some(TuningMessage::ScaleDump {
                device_id: 0x7F,
                bank: 0,
                program: 1,
                name: NAME_BYTES,
                offsets: o
            }))
        );
        let mut two = [0i16; 12];
        two[0] = -8192;
        two[11] = 8191;
        let o = ScaleOffsets::TwoByte(two);
        let n = write_scale_dump(0x7F, 0, 1, NAME_BYTES, &o, &mut out).unwrap();
        assert_eq!(n, 6 + 2 + 16 + 24 + 1);
        assert_eq!(&out[23..27], &[0x00, 0x00, 0x40, 0x00]);
        assert_eq!(&out[45..47], &[0x7F, 0x7F]);
        assert_eq!(
            read(&out[..n]),
            Ok(Some(TuningMessage::ScaleDump {
                device_id: 0x7F,
                bank: 0,
                program: 1,
                name: NAME_BYTES,
                offsets: o
            }))
        );
        one[0] = 64;
        assert_eq!(
            write_scale_dump(
                0x7F,
                0,
                1,
                NAME_BYTES,
                &ScaleOffsets::OneByte(one),
                &mut out
            ),
            Err(SysexError::OutOfRange)
        );
        two[0] = 8192;
        assert_eq!(
            write_scale_dump(
                0x7F,
                0,
                1,
                NAME_BYTES,
                &ScaleOffsets::TwoByte(two),
                &mut out
            ),
            Err(SysexError::OutOfRange)
        );
    }

    #[test]
    fn scale_octave_forms_and_channels() {
        // REQ-TUN-007: "F0 7F <device ID> 08 08 ff gg hh [ss] ... F7": hh channels 1 to 7, gg 8
        // to 14, ff bits 0 and 1 channels 15 and 16; the 5 reserved bits are written as 0 and
        // ignored on reading (AMB-MTS-07).
        let mut out = [0u8; 40];
        let o = ScaleOffsets::OneByte([5; 12]);
        for (rt, id) in [(true, 0x7F), (false, 0x7E)] {
            let n = write_scale_octave(rt, 0x01, 0b1100_0000_1000_0001, &o, &mut out).unwrap();
            assert_eq!(n, 6 + 3 + 12);
            assert_eq!(
                &out[..9],
                &[0xF0, id, 0x01, 0x08, 0x08, 0x03, 0x01, 0x01, 69]
            );
            assert_eq!(
                read(&out[..n]),
                Ok(Some(TuningMessage::ScaleOctave {
                    real_time: rt,
                    device_id: 0x01,
                    channels: 0b1100_0000_1000_0001,
                    offsets: o
                }))
            );
        }
        let n = write_scale_octave(true, 0x01, 0xFFFF, &o, &mut out).unwrap();
        assert_eq!(&out[5..8], &[0x03, 0x7F, 0x7F]);
        out[5] = 0x7F; // reserved bits set by a sender
        let Some(TuningMessage::ScaleOctave { channels, .. }) = read(&out[..n]).unwrap() else {
            panic!("not a scale/octave message");
        };
        assert_eq!(channels, 0xFFFF);
        let o2 = ScaleOffsets::TwoByte([-1; 12]);
        let n = write_scale_octave(false, 0x01, 1, &o2, &mut out).unwrap();
        assert_eq!((n, out[4]), (6 + 3 + 24, 0x09));
        assert_eq!(&out[8..10], &[0x3F, 0x7F]);
        assert_eq!(
            read(&out[..n]),
            Ok(Some(TuningMessage::ScaleOctave {
                real_time: false,
                device_id: 0x01,
                channels: 1,
                offsets: o2
            }))
        );
        assert_eq!(read(&out[..n - 2]), Err(SysexError::Length));
    }
}
