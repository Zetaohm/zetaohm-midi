//! The Device Control messages (M1 v4.2.1 p. 57; CA-025; Universal Real Time Sub-ID#1 04H;
//! DEC-022). Feature `device-control`.
//!
//! - Master Volume (Sub-ID#2 01H) and Master Balance (02H): `F0 7F <device ID> 04 0n ll mm F7`,
//!   a 14-bit value sent LSB first. Volume 0 is off; balance 0 is hard left, 3FFFH hard right and
//!   2000H the center (AMB-M1-53; REQ-DC-001).
//! - Master Fine Tuning (03H): a 14-bit value, LSB first, read as the signed offset from 8192
//!   where 8192 steps are 100 cents from A440, the scale of Channel Fine Tuning (AMB-CA-025-02;
//!   REQ-DC-002).
//! - Master Coarse Tuning (04H): an LSB and an MSB; the MSB minus 64 is the offset in semitones
//!   from A440. The writer writes LSB 0 and the reader ignores the LSB (AMB-CA-025-01;
//!   REQ-DC-003).
//!
//! No alloc (DEC-002). Never panics.

use crate::universal::real_time::device_control::{MASTER_BALANCE, MASTER_VOLUME};
use crate::universal::{write_universal, RealTimeId, SysexError, Universal};

/// Sub-ID#1 04H: Device Control.
const DEVICE_CONTROL: u8 = RealTimeId::DeviceControl.into_bits();
/// Sub-ID#2 03H: Master Fine Tuning (CA-025).
const MASTER_FINE_TUNING: u8 = 0x03;
/// Sub-ID#2 04H: Master Coarse Tuning (CA-025).
const MASTER_COARSE_TUNING: u8 = 0x04;

/// The value of one Device Control message, by its Sub-ID#2 (REQ-DC-001 to REQ-DC-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceControlValue {
    /// 01H: Master Volume, 14 bits; 0 is off.
    MasterVolume(u16),
    /// 02H: Master Balance, 14 bits; 0 is hard left, 2000H the center, 3FFFH hard right.
    MasterBalance(u16),
    /// 03H: Master Fine Tuning, 14 bits ([`fine_tuning_offset`]).
    MasterFineTuning(u16),
    /// 04H: Master Coarse Tuning, the MSB, 7 bits ([`coarse_tuning_semitones`]).
    MasterCoarseTuning(u8),
}

/// One Device Control message (REQ-DC-001 to REQ-DC-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceControl {
    /// The Device ID; 7FH addresses all devices.
    pub device_id: u8,
    /// The value.
    pub value: DeviceControlValue,
}

impl DeviceControl {
    /// Reads the message from a Universal header (Real Time, Sub-ID#1 04H). Gives `None` for
    /// another message or another Sub-ID#2, and [`SysexError::Length`] for data other than two
    /// bytes.
    pub fn from_universal(u: &Universal<'_>) -> Result<Option<Self>, SysexError> {
        if !u.real_time || u.sub_id1 != DEVICE_CONTROL {
            return Ok(None);
        }
        let Some(sub) = u
            .sub_id2
            .filter(|s| (MASTER_VOLUME..=MASTER_COARSE_TUNING).contains(s))
        else {
            return Ok(None);
        };
        let &[lsb, msb] = u.data else {
            return Err(SysexError::Length);
        };
        let v = lsb as u16 | (msb as u16) << 7;
        let value = match sub {
            MASTER_VOLUME => DeviceControlValue::MasterVolume(v),
            MASTER_BALANCE => DeviceControlValue::MasterBalance(v),
            MASTER_FINE_TUNING => DeviceControlValue::MasterFineTuning(v),
            _ => DeviceControlValue::MasterCoarseTuning(msb),
        };
        Ok(Some(Self {
            device_id: u.device_id,
            value,
        }))
    }

    /// Writes the whole message, F0H to F7H, into `out` and gives the length (REQ-SX-007). Gives
    /// [`SysexError::OutOfRange`] for a value above its width or a Device ID above 7FH.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, SysexError> {
        let (sub, v) = match self.value {
            DeviceControlValue::MasterVolume(v) => (MASTER_VOLUME, v),
            DeviceControlValue::MasterBalance(v) => (MASTER_BALANCE, v),
            DeviceControlValue::MasterFineTuning(v) => (MASTER_FINE_TUNING, v),
            DeviceControlValue::MasterCoarseTuning(m) => (MASTER_COARSE_TUNING, (m as u16) << 7),
        };
        let (lsb, msb) = (v & 0x7F, v >> 7);
        if lsb > 0x7F || msb > 0x7F {
            return Err(SysexError::OutOfRange);
        }
        write_universal(out, true, self.device_id, DEVICE_CONTROL, Some(sub), |w| {
            w.bytes(&[lsb as u8, msb as u8]);
            Ok(())
        })
    }
}

/// The signed offset of a 14-bit Master Fine Tuning value from 8192, -8192 to +8191, where 8192
/// steps are 100 cents from A440 (CA-025, AMB-CA-025-02; REQ-DC-002). Bits above 14 are ignored.
pub const fn fine_tuning_offset(value: u16) -> i16 {
    (value & 0x3FFF) as i16 - 8192
}

/// The Master Coarse Tuning MSB minus 64: the offset in semitones from A440, -64 to +63 (CA-025;
/// REQ-DC-003). Bit 7 is ignored.
pub const fn coarse_tuning_semitones(msb: u8) -> i8 {
    (msb & 0x7F) as i8 - 64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universal::Sysex;

    fn read(bytes: &[u8]) -> Result<Option<DeviceControl>, SysexError> {
        let s = Sysex::parse(bytes).unwrap();
        DeviceControl::from_universal(&Universal::parse(&s).unwrap().unwrap())
    }

    fn round_trip(value: DeviceControlValue, bytes: [u8; 8]) {
        let m = DeviceControl {
            device_id: 0x7F,
            value,
        };
        assert_eq!(read(&bytes), Ok(Some(m)), "{value:?}");
        let mut out = [0u8; 8];
        assert_eq!(m.write(&mut out), Ok(8));
        assert_eq!(out, bytes, "{value:?}");
    }

    #[test]
    fn master_volume_and_balance_bytes() {
        // REQ-DC-001: p. 57 "F0 7F <device ID> 04 01 vv vv F7", LSB first (AMB-M1-53).
        use DeviceControlValue::*;
        round_trip(
            MasterVolume(0x3FFF),
            [0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x7F, 0x7F, 0xF7],
        );
        round_trip(
            MasterVolume(0),
            [0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x00, 0x00, 0xF7],
        );
        round_trip(
            MasterVolume(0x0081),
            [0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x01, 0x01, 0xF7],
        );
        round_trip(
            MasterBalance(0x2000),
            [0xF0, 0x7F, 0x7F, 0x04, 0x02, 0x00, 0x40, 0xF7],
        );
        // Wrong length, other Sub-ID#2, Non-Real Time.
        assert_eq!(
            read(&[0x7F, 0x7F, 0x04, 0x01, 0x7F]),
            Err(SysexError::Length)
        );
        assert_eq!(
            read(&[0x7F, 0x7F, 0x04, 0x01, 0, 0, 0]),
            Err(SysexError::Length)
        );
        assert_eq!(read(&[0x7F, 0x7F, 0x04, 0x05, 0, 0]), Ok(None));
        assert_eq!(read(&[0x7E, 0x7F, 0x04, 0x01, 0, 0]), Ok(None));
        let mut out = [0u8; 8];
        let big = DeviceControl {
            device_id: 0,
            value: MasterVolume(0x4000),
        };
        assert_eq!(big.write(&mut out), Err(SysexError::OutOfRange));
        let dev = DeviceControl {
            device_id: 0x80,
            value: MasterVolume(0),
        };
        assert_eq!(dev.write(&mut out), Err(SysexError::OutOfRange));
    }

    #[test]
    fn master_fine_tuning_ca_025_table() {
        // REQ-DC-002: the CA-025 table (p. 2): 00 00 = -100 cents, 00 40 = A440, 7F 7F = +99.988.
        use DeviceControlValue::MasterFineTuning;
        round_trip(
            MasterFineTuning(0),
            [0xF0, 0x7F, 0x7F, 0x04, 0x03, 0x00, 0x00, 0xF7],
        );
        round_trip(
            MasterFineTuning(0x2000),
            [0xF0, 0x7F, 0x7F, 0x04, 0x03, 0x00, 0x40, 0xF7],
        );
        round_trip(
            MasterFineTuning(0x3FFF),
            [0xF0, 0x7F, 0x7F, 0x04, 0x03, 0x7F, 0x7F, 0xF7],
        );
        assert_eq!(fine_tuning_offset(0x0000), -8192);
        assert_eq!(fine_tuning_offset(0x2000), 0);
        assert_eq!(fine_tuning_offset(0x3FFF), 8191);
        assert_eq!(fine_tuning_offset(0xC000), -8192);
    }

    #[test]
    fn master_coarse_tuning_ca_025_table() {
        // REQ-DC-003: the CA-025 table (p. 2): MSB 00 = -64 semitones, 40 = A440, 7F = +63; the
        // writer writes LSB 0, the reader ignores the LSB (AMB-CA-025-01).
        use DeviceControlValue::MasterCoarseTuning;
        round_trip(
            MasterCoarseTuning(0x40),
            [0xF0, 0x7F, 0x7F, 0x04, 0x04, 0x00, 0x40, 0xF7],
        );
        round_trip(
            MasterCoarseTuning(0x7F),
            [0xF0, 0x7F, 0x7F, 0x04, 0x04, 0x00, 0x7F, 0xF7],
        );
        assert_eq!(
            read(&[0x7F, 0x7F, 0x04, 0x04, 0x33, 0x40]),
            Ok(Some(DeviceControl {
                device_id: 0x7F,
                value: MasterCoarseTuning(0x40)
            }))
        );
        assert_eq!(coarse_tuning_semitones(0x00), -64);
        assert_eq!(coarse_tuning_semitones(0x40), 0);
        assert_eq!(coarse_tuning_semitones(0x7F), 63);
        let mut out = [0u8; 8];
        let big = DeviceControl {
            device_id: 0,
            value: MasterCoarseTuning(0x80),
        };
        assert_eq!(big.write(&mut out), Err(SysexError::OutOfRange));
    }
}
