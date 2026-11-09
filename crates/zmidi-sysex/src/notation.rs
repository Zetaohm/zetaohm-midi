//! The Notation Information messages (M1 v4.2.1 pp. 54, 55; Universal Real Time Sub-ID#1 03H;
//! DEC-022). Feature `notation`.
//!
//! - Bar Marker (Sub-ID#2 01H): `F0 7F <device ID> 03 01 aa aa F7`, a 14-bit two's complement
//!   bar number sent LSB first: 00H 40H (-8192) is Not Running, -8191 to 0 are count-in bars, 1 to
//!   8190 are bar numbers and 7FH 3FH (8191) is Running with an unknown bar number (AMB-M1-52;
//!   REQ-NOT-001).
//! - Time Signature, Immediate (02H) and Delayed (42H): `F0 7F <device ID> 03 sub ln nn dd cc bb
//!   [nn dd ...] F7`. The format lines of p. 55 have one "bb" too many; the field list, ln = 4 and
//!   the SMF event FF 58 give `nn dd cc bb` (AMB-M1-50; REQ-NOT-002). ln is 4 + 2 × the number of
//!   additional pairs; the reader rejects another ln (AMB-M1-51; REQ-NOT-003).
//!
//! The field names follow the Standard MIDI File Time Signature (FF 58). When a message takes
//! effect (the next F8H or F1H, the next Bar Marker) is the application's.
//!
//! No alloc (DEC-002). Never panics.

use crate::universal::real_time::notation_information::{
    BAR_NUMBER, TIME_SIGNATURE_DELAYED, TIME_SIGNATURE_IMMEDIATE,
};
use crate::universal::{write_universal, RealTimeId, SysexError, Universal};

/// Sub-ID#1 03H: Notation Information.
const NOTATION: u8 = RealTimeId::NotationInformation.into_bits();
/// ln with no additional pairs: `nn dd cc bb` (p. 55).
const BASE_LEN: usize = 4;
/// The most additional pairs: ln is 7 bits, and 4 + 2 × 61 = 126 (AMB-M1-51).
const MAX_PAIRS: usize = 61;

/// The bar number of a Bar Marker, a 14-bit two's complement value (p. 54, AMB-M1-52;
/// REQ-NOT-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bar {
    /// -8192 (00H 40H): not running.
    NotRunning,
    /// -8191 to 0: a count-in bar; 0 is the last bar of the count-in.
    CountIn(i16),
    /// 1 to 8190: a bar number in the song.
    Number(u16),
    /// 8191 (7FH 3FH): running, but the bar number is not known (or is above 8190).
    Unknown,
}

impl Bar {
    /// The Bar of a signed value, -8192 to 8191; `None` outside that range.
    pub const fn from_value(value: i16) -> Option<Self> {
        Some(match value {
            -8192 => Self::NotRunning,
            -8191..=0 => Self::CountIn(value),
            1..=8190 => Self::Number(value as u16),
            8191 => Self::Unknown,
            _ => return None,
        })
    }

    /// The signed value of the Bar, the inverse of [`Bar::from_value`].
    pub const fn value(self) -> i16 {
        match self {
            Self::NotRunning => -8192,
            Self::CountIn(v) => v,
            Self::Number(n) => n as i16,
            Self::Unknown => 8191,
        }
    }
}

/// One Bar Marker message: the next MIDI clock (or MTC Quarter Frame) starts this bar
/// (REQ-NOT-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BarMarker {
    /// The Device ID; 7FH addresses all devices.
    pub device_id: u8,
    /// The bar number.
    pub bar: Bar,
}

impl BarMarker {
    /// Reads the message from a Universal header (Real Time, Sub-ID#1 03H, Sub-ID#2 01H). Gives
    /// `None` for another message, and [`SysexError::Length`] for data other than two bytes.
    pub fn from_universal(u: &Universal<'_>) -> Result<Option<Self>, SysexError> {
        if !u.real_time || u.sub_id1 != NOTATION || u.sub_id2 != Some(BAR_NUMBER) {
            return Ok(None);
        }
        let &[lsb, msb] = u.data else {
            return Err(SysexError::Length);
        };
        // The 14 bits to the top of an i16, then an arithmetic shift back: the sign of bit 13.
        let value = ((lsb as u16 | (msb as u16) << 7) << 2) as i16 >> 2;
        let bar = Bar::from_value(value).ok_or(SysexError::DataByte)?;
        Ok(Some(Self {
            device_id: u.device_id,
            bar,
        }))
    }

    /// Writes the whole message, F0H to F7H, into `out` and gives the length (REQ-SX-007). Gives
    /// [`SysexError::OutOfRange`] for a [`Bar::CountIn`] or [`Bar::Number`] value outside its
    /// range, or a Device ID above 7FH.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, SysexError> {
        let value = self.bar.value();
        if Bar::from_value(value) != Some(self.bar) {
            return Err(SysexError::OutOfRange);
        }
        let bits = value as u16 & 0x3FFF;
        write_universal(out, true, self.device_id, NOTATION, Some(BAR_NUMBER), |w| {
            w.bytes(&[(bits & 0x7F) as u8, (bits >> 7) as u8]);
            Ok(())
        })
    }
}

/// One Time Signature message, Immediate or Delayed; it borrows the input (REQ-NOT-002,
/// REQ-NOT-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimeSignature<'a> {
    /// The Device ID; 7FH addresses all devices.
    pub device_id: u8,
    /// `false`: Immediate (02H), it takes effect at once (or at the next MIDI clock); `true`:
    /// Delayed (42H), it takes effect at the next Bar Marker.
    pub delayed: bool,
    /// `nn`: the numerator.
    pub numerator: u8,
    /// `dd`: the denominator as a negative power of 2 (2 = quarter note, 3 = eighth note).
    pub denominator_power: u8,
    /// `cc`: MIDI clocks in a metronome click.
    pub clocks_per_click: u8,
    /// `bb`: notated 32nd notes in a MIDI quarter note (24 MIDI clocks).
    pub thirty_seconds_per_quarter: u8,
    /// The additional `nn dd` pairs of a compound time signature in the same bar, as bytes.
    pub additional: &'a [u8],
}

impl<'a> TimeSignature<'a> {
    /// Reads the message from a Universal header (Real Time, Sub-ID#1 03H, Sub-ID#2 02H or 42H).
    /// Gives `None` for another message, and [`SysexError::Length`] for an ln below 4, an odd ln,
    /// or an ln that differs from the number of bytes after it (AMB-M1-51).
    pub fn from_universal(u: &Universal<'a>) -> Result<Option<Self>, SysexError> {
        if !u.real_time || u.sub_id1 != NOTATION {
            return Ok(None);
        }
        let delayed = match u.sub_id2 {
            Some(TIME_SIGNATURE_IMMEDIATE) => false,
            Some(TIME_SIGNATURE_DELAYED) => true,
            _ => return Ok(None),
        };
        let &[ln, nn, dd, cc, bb, ref additional @ ..] = u.data else {
            return Err(SysexError::Length);
        };
        if ln as usize != BASE_LEN + additional.len() || !additional.len().is_multiple_of(2) {
            return Err(SysexError::Length);
        }
        Ok(Some(Self {
            device_id: u.device_id,
            delayed,
            numerator: nn,
            denominator_power: dd,
            clocks_per_click: cc,
            thirty_seconds_per_quarter: bb,
            additional,
        }))
    }

    /// The additional pairs, `(numerator, denominator_power)`.
    pub fn pairs(&self) -> impl Iterator<Item = (u8, u8)> + 'a {
        self.additional
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[nn, dd]| (nn, dd))
    }

    /// Writes the whole message, F0H to F7H, into `out` and gives the length; ln is 4 + 2 × the
    /// number of pairs (REQ-SX-007). Gives [`SysexError::OutOfRange`] for an odd number of
    /// additional bytes, more than 61 pairs, a field above 7FH or a Device ID above 7FH.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, SysexError> {
        let extra = self.additional.len();
        if !extra.is_multiple_of(2) || extra > 2 * MAX_PAIRS {
            return Err(SysexError::OutOfRange);
        }
        let sub = if self.delayed {
            TIME_SIGNATURE_DELAYED
        } else {
            TIME_SIGNATURE_IMMEDIATE
        };
        write_universal(out, true, self.device_id, NOTATION, Some(sub), |w| {
            w.byte((BASE_LEN + extra) as u8);
            w.value(self.numerator)?;
            w.value(self.denominator_power)?;
            w.value(self.clocks_per_click)?;
            w.value(self.thirty_seconds_per_quarter)?;
            for &b in self.additional {
                w.value(b)?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universal::Sysex;
    use proptest::prelude::*;

    fn universal(bytes: &[u8]) -> Universal<'_> {
        Universal::parse(&Sysex::parse(bytes).unwrap())
            .unwrap()
            .unwrap()
    }

    fn bar(bytes: &[u8]) -> Result<Option<BarMarker>, SysexError> {
        BarMarker::from_universal(&universal(bytes))
    }

    fn time(bytes: &[u8]) -> Result<Option<TimeSignature<'_>>, SysexError> {
        TimeSignature::from_universal(&universal(bytes))
    }

    #[test]
    fn bar_marker_bytes() {
        // REQ-NOT-001: p. 54 "F0 7F <device ID> 03 01 aa aa F7", the byte pairs of the list
        // (AMB-M1-52).
        for (aa, b) in [
            ([0x00, 0x40], Bar::NotRunning),
            ([0x01, 0x40], Bar::CountIn(-8191)),
            ([0x7F, 0x7F], Bar::CountIn(-1)),
            ([0x00, 0x00], Bar::CountIn(0)),
            ([0x01, 0x00], Bar::Number(1)),
            ([0x7E, 0x3F], Bar::Number(8190)),
            ([0x7F, 0x3F], Bar::Unknown),
        ] {
            let bytes = [0xF0, 0x7F, 0x7F, 0x03, 0x01, aa[0], aa[1], 0xF7];
            let m = BarMarker {
                device_id: 0x7F,
                bar: b,
            };
            assert_eq!(bar(&bytes), Ok(Some(m)), "{b:?}");
            let mut out = [0u8; 8];
            assert_eq!(m.write(&mut out), Ok(8));
            assert_eq!(out, bytes, "{b:?}");
        }
        // Wrong length, another Sub-ID#2 and Non-Real Time.
        assert_eq!(
            bar(&[0x7F, 0x7F, 0x03, 0x01, 0x00]),
            Err(SysexError::Length)
        );
        assert_eq!(
            bar(&[0x7F, 0x7F, 0x03, 0x01, 0, 0, 0]),
            Err(SysexError::Length)
        );
        assert_eq!(bar(&[0x7F, 0x7F, 0x03, 0x02, 0, 0]), Ok(None));
        assert_eq!(bar(&[0x7E, 0x7F, 0x03, 0x01, 0, 0]), Ok(None));
        // The writer refuses a value outside its variant, and a Device ID above 7FH.
        let mut out = [0u8; 8];
        for b in [
            Bar::CountIn(1),
            Bar::CountIn(-8192),
            Bar::Number(0),
            Bar::Number(8191),
            Bar::Number(40000),
        ] {
            let m = BarMarker {
                device_id: 0,
                bar: b,
            };
            assert_eq!(m.write(&mut out), Err(SysexError::OutOfRange), "{b:?}");
        }
        let m = BarMarker {
            device_id: 0x80,
            bar: Bar::Unknown,
        };
        assert_eq!(m.write(&mut out), Err(SysexError::OutOfRange));
        assert_eq!(
            BarMarker {
                device_id: 0,
                bar: Bar::Unknown
            }
            .write(&mut out[..7]),
            Err(SysexError::BufferTooSmall { needed: 8 })
        );
    }

    #[test]
    fn prop_bar_round_trip() {
        // REQ-NOT-001: each value -8192 to 8191 is one Bar and one byte pair; no other value is.
        proptest!(|(v in -8192i16..=8191, d in 0u8..0x80, w in any::<i16>())| {
            let b = Bar::from_value(v).unwrap();
            prop_assert_eq!(b.value(), v);
            let m = BarMarker { device_id: d, bar: b };
            let mut out = [0u8; 8];
            prop_assert_eq!(m.write(&mut out), Ok(8));
            prop_assert_eq!(bar(&out), Ok(Some(m)));
            prop_assert_eq!(Bar::from_value(w).is_some(), (-8192..=8191).contains(&w));
        });
    }

    #[test]
    fn time_signature_bytes() {
        // REQ-NOT-002: p. 55, the data ln nn dd cc bb [nn dd ...] (AMB-M1-50). 4/4, 24 clocks a
        // click, 8 32nd notes a quarter note, Immediate.
        let four = [
            0xF0, 0x7F, 0x7F, 0x03, 0x02, 0x04, 0x04, 0x02, 0x18, 0x08, 0xF7,
        ];
        let m = TimeSignature {
            device_id: 0x7F,
            delayed: false,
            numerator: 4,
            denominator_power: 2,
            clocks_per_click: 24,
            thirty_seconds_per_quarter: 8,
            additional: &[],
        };
        assert_eq!(time(&four), Ok(Some(m)));
        assert_eq!(m.pairs().count(), 0);
        let mut out = [0u8; 16];
        assert_eq!(m.write(&mut out), Ok(11));
        assert_eq!(&out[..11], &four);
        // 3/8 + 2/8 in one bar, Delayed, an eighth-note click: one additional pair.
        let compound = [
            0xF0, 0x7F, 0x10, 0x03, 0x42, 0x06, 0x03, 0x03, 0x0C, 0x08, 0x02, 0x03, 0xF7,
        ];
        let m = time(&compound).unwrap().unwrap();
        assert_eq!(
            (m.device_id, m.delayed, m.numerator, m.denominator_power),
            (0x10, true, 3, 3)
        );
        assert_eq!((m.clocks_per_click, m.thirty_seconds_per_quarter), (12, 8));
        assert!(m.pairs().eq([(2, 3)]));
        assert_eq!(m.write(&mut out), Ok(13));
        assert_eq!(&out[..13], &compound);
        // Another Sub-ID#2, the Bar Marker and Non-Real Time are not a Time Signature.
        assert_eq!(time(&[0x7F, 0x7F, 0x03, 0x03, 4, 4, 2, 24, 8]), Ok(None));
        assert_eq!(time(&[0x7F, 0x7F, 0x03, 0x01, 0, 0]), Ok(None));
        assert_eq!(time(&[0x7E, 0x7F, 0x03, 0x02, 4, 4, 2, 24, 8]), Ok(None));
    }

    #[test]
    fn time_signature_length_rules() {
        // REQ-NOT-003: ln = 4 + 2k; the reader rejects another ln (AMB-M1-51).
        let e = Err(SysexError::Length);
        assert_eq!(time(&[0x7F, 0x7F, 0x03, 0x02, 3, 4, 2, 24]), e);
        assert_eq!(time(&[0x7F, 0x7F, 0x03, 0x02, 3, 4, 2, 24, 8]), e);
        assert_eq!(time(&[0x7F, 0x7F, 0x03, 0x02, 4, 4, 2, 24, 8, 2, 3]), e);
        assert_eq!(time(&[0x7F, 0x7F, 0x03, 0x02, 5, 4, 2, 24, 8, 2]), e);
        assert_eq!(time(&[0x7F, 0x7F, 0x03, 0x02, 6, 4, 2, 24, 8]), e);
        assert_eq!(time(&[0x7F, 0x7F, 0x03, 0x42]), e);
        // The writer: pairs only, at most 61 (ln 7EH), 7-bit fields.
        let pairs = [0x01u8; 124];
        let mut m = TimeSignature {
            device_id: 0,
            delayed: true,
            numerator: 4,
            denominator_power: 2,
            clocks_per_click: 24,
            thirty_seconds_per_quarter: 8,
            additional: &pairs[..122],
        };
        let mut out = [0u8; 140];
        assert_eq!(m.write(&mut out), Ok(133));
        assert_eq!(out[5], 0x7E);
        assert_eq!(time(&out[..133]).unwrap().unwrap().pairs().count(), 61);
        let o = Err(SysexError::OutOfRange);
        m.additional = &pairs[..124];
        assert_eq!(m.write(&mut out), o);
        m.additional = &pairs[..3];
        assert_eq!(m.write(&mut out), o);
        m.additional = &[0x80, 0x01];
        assert_eq!(m.write(&mut out), o);
        m.additional = &[];
        m.numerator = 0x80;
        assert_eq!(m.write(&mut out), o);
        m.numerator = 4;
        m.device_id = 0x80;
        assert_eq!(m.write(&mut out), o);
    }

    #[test]
    fn prop_time_signature_round_trip() {
        // REQ-NOT-002, REQ-NOT-003: every valid message writes and reads back the same.
        proptest!(|(
            f in proptest::array::uniform5(0u8..0x80),
            delayed in any::<bool>(),
            pairs in (0usize..=61).prop_flat_map(|k| proptest::collection::vec(0u8..0x80, 2 * k)),
        )| {
            let m = TimeSignature {
                device_id: f[0],
                delayed,
                numerator: f[1],
                denominator_power: f[2],
                clocks_per_click: f[3],
                thirty_seconds_per_quarter: f[4],
                additional: &pairs,
            };
            let mut out = [0u8; 140];
            let n = m.write(&mut out).unwrap();
            prop_assert_eq!(n, 11 + pairs.len());
            prop_assert_eq!(time(&out[..n]), Ok(Some(m)));
            prop_assert_eq!(m.pairs().count(), pairs.len() / 2);
        });
    }
}
