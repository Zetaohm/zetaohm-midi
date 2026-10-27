//! BLE-MIDI v1.0 timestamps and the header and timestamp bytes (sections 7 and 9).
//!
//! - A timestamp is 13 bits of milliseconds, 0 to 8191 (9).
//! - Header byte: bit 7 = 1, bit 6 = 0 (reserved; the reader ignores it, AMB-BLE-MIDI-04),
//!   bits 5-0 = timestampHigh, the most significant 6 bits (7, Header Byte table).
//! - Timestamp byte: bit 7 = 1, bits 6-0 = timestampLow, the least significant 7 bits (7,
//!   Timestamp Byte table).
//! - The timestamp is timestampHigh * 128 + timestampLow (7, 9).

/// A 13-bit BLE-MIDI timestamp in milliseconds (0 to 8191), in the sender's clock domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Timestamp(u16);

impl Timestamp {
    /// The largest timestamp: 8191 ms.
    pub const MAX: Timestamp = Timestamp(0x1FFF);

    /// The timestamp of a time in milliseconds: its low 13 bits (the time modulo 8192).
    pub const fn from_millis(ms: u64) -> Self {
        Timestamp((ms & 0x1FFF) as u16)
    }

    /// The timestamp of `high` (6 bits) and `low` (7 bits); higher bits are ignored.
    pub const fn from_parts(high: u8, low: u8) -> Self {
        Timestamp(((high as u16 & 0x3F) << 7) | (low as u16 & 0x7F))
    }

    /// The value in milliseconds, 0 to 8191.
    pub const fn millis(self) -> u16 {
        self.0
    }

    /// timestampHigh: the most significant 6 bits.
    pub const fn high(self) -> u8 {
        (self.0 >> 7) as u8
    }

    /// timestampLow: the least significant 7 bits.
    pub const fn low(self) -> u8 {
        (self.0 & 0x7F) as u8
    }

    /// The header byte of a packet whose first time is this timestamp: 1, 0, timestampHigh.
    pub const fn header_byte(self) -> u8 {
        0x80 | self.high()
    }

    /// The timestamp byte: 1, timestampLow.
    pub const fn timestamp_byte(self) -> u8 {
        0x80 | self.low()
    }
}

/// True when `byte` can be a header byte (bit 7 set). Bit 6 is not checked.
pub const fn is_header(byte: u8) -> bool {
    byte & 0x80 != 0
}

/// timestampHigh of a header byte; bit 6 is ignored.
pub const fn header_high(byte: u8) -> u8 {
    byte & 0x3F
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn header_and_timestamp_bytes_of_section_7() {
        let t = Timestamp::from_parts(0x2A, 0x55);
        assert_eq!(t.millis(), 0x2A * 128 + 0x55);
        assert_eq!(t.header_byte(), 0b1010_1010);
        assert_eq!(t.timestamp_byte(), 0b1101_0101);
        assert_eq!(Timestamp::MAX.millis(), 8191);
        assert_eq!(Timestamp::MAX.header_byte(), 0xBF);
        assert_eq!(Timestamp::MAX.timestamp_byte(), 0xFF);
    }

    #[test]
    fn reader_ignores_header_bit_6() {
        assert!(is_header(0xC5));
        assert_eq!(header_high(0xC5), 0x05);
        assert!(!is_header(0x45));
    }

    #[test]
    fn prop_timestamp_is_the_time_modulo_8192() {
        proptest!(|(ms in any::<u64>())| {
            let t = Timestamp::from_millis(ms);
            prop_assert_eq!(u64::from(t.millis()), ms % 8192);
            prop_assert_eq!(Timestamp::from_parts(t.high(), t.low()), t);
            prop_assert_eq!(t.header_byte() & 0xC0, 0x80);
            prop_assert_eq!(t.timestamp_byte() & 0x80, 0x80);
        });
    }
}
