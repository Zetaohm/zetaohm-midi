//! MIDI Clip File constants and byte order (M2-116-U v1.0, sections 3 and 4, Table 5).
//!
//! - A MIDI Clip File starts with the 8-byte File Header "SMF2CLIP" (Table 5); after it there
//!   are only UMP messages.
//! - All data is big endian: each 32-bit UMP word is four bytes, most significant byte first.
//! - A Delta Clockstamp counts at most 1,048,575 ticks (20 bits, M2-104-UM 7.2.3.2).
//!
//! No alloc (DEC-002).

/// The File Header "SMF2CLIP" (Table 5).
pub const FILE_HEADER: [u8; 8] = *b"SMF2CLIP";

/// The MIDI Clip File name extension.
pub const EXTENSION: &str = ".midi2";

/// The largest Ticks Since Last Event of one Delta Clockstamp (20 bits).
pub const MAX_DELTA_TICKS: u32 = 0xF_FFFF;

/// The four bytes of a UMP word, most significant byte first.
pub const fn word_to_bytes(word: u32) -> [u8; 4] {
    word.to_be_bytes()
}

/// The UMP word of four bytes, most significant byte first.
pub const fn word_from_bytes(bytes: [u8; 4]) -> u32 {
    u32::from_be_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn file_header_matches_table_5() {
        assert_eq!(
            FILE_HEADER,
            [0x53, 0x4D, 0x46, 0x32, 0x43, 0x4C, 0x49, 0x50]
        );
        assert_eq!(EXTENSION, ".midi2");
        assert_eq!(MAX_DELTA_TICKS, 1_048_575);
    }

    #[test]
    fn words_are_big_endian() {
        assert_eq!(word_to_bytes(0x0040_0010), [0x00, 0x40, 0x00, 0x10]);
        assert_eq!(word_from_bytes([0xF0, 0x20, 0x00, 0x00]), 0xF020_0000);
    }

    #[test]
    fn prop_word_bytes_round_trip() {
        proptest!(|(w in any::<u32>())| {
            prop_assert_eq!(word_from_bytes(word_to_bytes(w)), w);
        });
    }
}
