//! Property Data encodings (M2-103-UM v1.2 sections 6.1.5 to 6.1.7 and 6.2; Table 12).
//!
//! - "ASCII": the bytes go unchanged and must be 7-bit (REQ-PE-ENC-006, REQ-PE-DATA-001).
//! - "Mcoded7": each group of 7 bytes becomes 8 bytes: one byte with bit 7 of the 7 bytes
//!   (bit 7 of the first byte in bit 6), then the low 7 bits of each byte. A last group of n
//!   bytes becomes n + 1 bytes (REQ-PE-ENC-008). Encoder and decoder are streams with a state of
//!   at most 8 bytes (DEC-007); a group can cross a Chunk boundary.
//! - "zlib+Mcoded7" is in the `zlib` module (Cargo feature `zlib`, DEC-PE-001): this module gives
//!   `Unsupported` for it.

use crate::header::EncodingName;

/// Error of an encoder or decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EncodingError {
    /// The caller output buffer is too small.
    #[error("output buffer overflow")]
    Overflow,
    /// A byte above 0x7F for "ASCII", or a received byte with bit 7 set (REQ-PE-ENC-006).
    #[error("byte not 7-bit")]
    NotSevenBit,
    /// A zlib stream whose header declares a window larger than the window buffer
    /// (REQ-PE-ENC-009; the responder replies 413).
    #[error("zlib window too large")]
    WindowTooLarge,
    /// Data that is not a valid zlib stream, or a stream that does not end (REQ-PE-ENC-009).
    #[error("invalid zlib data")]
    InvalidZlib,
    /// The encoding is not supported here: "zlib+Mcoded7" uses the `zlib` module (Cargo feature
    /// `zlib`, DEC-PE-001).
    #[error("encoding not supported")]
    Unsupported,
}

/// Size of the Mcoded7 form of `n` bytes.
pub const fn mcoded7_len(n: usize) -> usize {
    n / 7 * 8 + if n.is_multiple_of(7) { 0 } else { n % 7 + 1 }
}

/// Streaming Mcoded7 encoder (REQ-PE-ENC-008).
#[derive(Debug, Clone, Default)]
pub struct Mcoded7Encoder {
    group: [u8; 7],
    n: usize,
}

impl Mcoded7Encoder {
    /// A new encoder.
    pub const fn new() -> Self {
        Mcoded7Encoder {
            group: [0; 7],
            n: 0,
        }
    }

    fn emit(group: &[u8], out: &mut [u8]) -> Result<usize, EncodingError> {
        let len = group.len() + 1;
        let o = out.get_mut(..len).ok_or(EncodingError::Overflow)?;
        let mut head = 0u8;
        for (i, b) in group.iter().enumerate() {
            head |= (b >> 7) << (6 - i);
            o[i + 1] = b & 0x7F;
        }
        o[0] = head;
        Ok(len)
    }

    /// Encodes `input`; each complete group of 7 bytes is written to `out`, a rest waits for
    /// more input. Gives the number of bytes written. On Overflow nothing is consumed.
    pub fn encode(&mut self, input: &[u8], out: &mut [u8]) -> Result<usize, EncodingError> {
        let total = self.n + input.len();
        if out.len() < total / 7 * 8 {
            return Err(EncodingError::Overflow);
        }
        let mut written = 0;
        for &b in input {
            self.group[self.n] = b;
            self.n += 1;
            if self.n == 7 {
                written += Self::emit(&self.group, &mut out[written..])?;
                self.n = 0;
            }
        }
        Ok(written)
    }

    /// Number of input bytes that wait for a complete group.
    #[cfg(feature = "zlib")]
    pub(crate) const fn pending(&self) -> usize {
        self.n
    }

    /// Writes the last group of 1 to 6 bytes, if any; gives the number of bytes written.
    pub fn finish(&mut self, out: &mut [u8]) -> Result<usize, EncodingError> {
        if self.n == 0 {
            return Ok(0);
        }
        let n = Self::emit(&self.group[..self.n], out)?;
        self.n = 0;
        Ok(n)
    }
}

/// Streaming Mcoded7 decoder (REQ-PE-ENC-008, REQ-PE-DATA-003).
#[derive(Debug, Clone, Copy, Default)]
pub struct Mcoded7Decoder {
    head: u8,
    /// Bytes of the present group so far, header byte included.
    n: usize,
}

impl Mcoded7Decoder {
    /// A new decoder.
    pub const fn new() -> Self {
        Mcoded7Decoder { head: 0, n: 0 }
    }

    /// Decodes `input` into `out`; gives the number of data bytes written. A byte with bit 7
    /// set is an error. On Overflow nothing is consumed.
    pub fn decode(&mut self, input: &[u8], out: &mut [u8]) -> Result<usize, EncodingError> {
        if input.iter().any(|b| *b > 0x7F) {
            return Err(EncodingError::NotSevenBit);
        }
        // Data bytes in `input`: every byte that is not a group header byte. The first header
        // byte is at index (8 - n) % 8, then one in each 8 (OPT-030: no loop).
        let first = (8 - self.n) % 8;
        let heads = match input.len().checked_sub(first) {
            Some(rest) if rest > 0 => (rest - 1) / 8 + 1,
            _ => 0,
        };
        if out.len() < input.len() - heads {
            return Err(EncodingError::Overflow);
        }
        let (mut n, mut head) = (self.n, self.head);
        let mut w = 0;
        for &b in input {
            if n == 0 {
                head = b;
            } else {
                // Bit 7 - n of the header byte is bit 7 of this byte.
                if let Some(slot) = out.get_mut(w) {
                    *slot = b | ((head << n) & 0x80);
                }
                w += 1;
            }
            n = (n + 1) % 8;
        }
        self.n = n;
        self.head = head;
        Ok(w)
    }

    /// The end of the data. A last group of one byte (a header byte with no data) gives no
    /// data and is ignored, as JUCE, MIDI 2.0 Workbench and ktmidi do (AMB-M2-103-31). The
    /// unused low bits of a short group's header byte are ignored.
    pub fn finish(&mut self) -> Result<(), EncodingError> {
        self.n = 0;
        Ok(())
    }
}

/// Encoder of the Property Data of one message by its encoding (REQ-PE-DATA-001, -003).
#[derive(Debug, Clone)]
pub enum DataEncoder {
    /// "ASCII": bytes unchanged, each 7-bit.
    Ascii,
    /// "Mcoded7".
    Mcoded7(Mcoded7Encoder),
}

impl DataEncoder {
    /// An encoder for `encoding`; `Unsupported` for "zlib+Mcoded7" (see the `zlib` module).
    pub fn new(encoding: EncodingName) -> Result<Self, EncodingError> {
        match encoding {
            EncodingName::Ascii => Ok(DataEncoder::Ascii),
            EncodingName::Mcoded7 => Ok(DataEncoder::Mcoded7(Mcoded7Encoder::new())),
            EncodingName::ZlibMcoded7 => Err(EncodingError::Unsupported),
        }
    }

    /// Encodes the next part of the Property Data into `out`. For "ASCII" a byte above 0x7F
    /// is an error and nothing is sent (REQ-PE-ENC-006).
    pub fn encode(&mut self, input: &[u8], out: &mut [u8]) -> Result<usize, EncodingError> {
        match self {
            DataEncoder::Ascii => {
                if input.iter().any(|b| *b > 0x7F) {
                    return Err(EncodingError::NotSevenBit);
                }
                out.get_mut(..input.len())
                    .ok_or(EncodingError::Overflow)?
                    .copy_from_slice(input);
                Ok(input.len())
            }
            DataEncoder::Mcoded7(e) => e.encode(input, out),
        }
    }

    /// Writes the end of the encoded data.
    pub fn finish(&mut self, out: &mut [u8]) -> Result<usize, EncodingError> {
        match self {
            DataEncoder::Ascii => Ok(0),
            DataEncoder::Mcoded7(e) => e.finish(out),
        }
    }
}

/// Decoder of received Property Data by its encoding; the Chunks of a Data Set are one stream
/// (REQ-PE-DATA-003).
#[derive(Debug, Clone)]
pub enum DataDecoder {
    /// "ASCII": bytes unchanged.
    Ascii,
    /// "Mcoded7".
    Mcoded7(Mcoded7Decoder),
}

impl DataDecoder {
    /// A decoder for `encoding`; `Unsupported` for "zlib+Mcoded7" (see the `zlib` module).
    pub fn new(encoding: EncodingName) -> Result<Self, EncodingError> {
        match encoding {
            EncodingName::Ascii => Ok(DataDecoder::Ascii),
            EncodingName::Mcoded7 => Ok(DataDecoder::Mcoded7(Mcoded7Decoder::new())),
            EncodingName::ZlibMcoded7 => Err(EncodingError::Unsupported),
        }
    }

    /// Decodes the Property Data of one Chunk into `out`.
    pub fn decode(&mut self, input: &[u8], out: &mut [u8]) -> Result<usize, EncodingError> {
        match self {
            DataDecoder::Ascii => {
                out.get_mut(..input.len())
                    .ok_or(EncodingError::Overflow)?
                    .copy_from_slice(input);
                Ok(input.len())
            }
            DataDecoder::Mcoded7(d) => d.decode(input, out),
        }
    }

    /// The end of the Data Set.
    pub fn finish(&mut self) -> Result<(), EncodingError> {
        match self {
            DataDecoder::Ascii => Ok(()),
            DataDecoder::Mcoded7(d) => d.finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;

    fn encode_all(data: &[u8], piece: usize) -> Vec<u8> {
        let mut e = Mcoded7Encoder::new();
        let mut out = Vec::new();
        for p in data.chunks(piece.max(1)) {
            let mut buf = [0u8; 64];
            let n = e.encode(p, &mut buf).unwrap();
            out.extend_from_slice(&buf[..n]);
        }
        let mut buf = [0u8; 8];
        let n = e.finish(&mut buf).unwrap();
        out.extend_from_slice(&buf[..n]);
        out
    }

    fn decode_all(data: &[u8], piece: usize) -> Result<Vec<u8>, EncodingError> {
        let mut d = Mcoded7Decoder::new();
        let mut out = Vec::new();
        for p in data.chunks(piece.max(1)) {
            let mut buf = [0u8; 64];
            let n = d.decode(p, &mut buf)?;
            out.extend_from_slice(&buf[..n]);
        }
        d.finish()?;
        Ok(out)
    }

    #[test]
    fn mcoded7_vector() {
        // M2-103 6.1.7 / Table 13: AAAAaaaa BBBBbbbb ... GGGGgggg gives a header byte with the
        // seven top bits, then the seven low parts.
        let data = [0x81, 0x02, 0x83, 0x04, 0x85, 0x06, 0x87];
        assert_eq!(
            encode_all(&data, 7),
            [0b0101_0101, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]
        );
        // A last group of 2 bytes: 3 bytes, unused header bits 0.
        assert_eq!(encode_all(&[0xFF, 0x01], 7), [0b0100_0000, 0x7F, 0x01]);
        assert_eq!(mcoded7_len(7), 8);
        assert_eq!(mcoded7_len(9), 11);
        assert_eq!(mcoded7_len(0), 0);
    }

    #[test]
    fn mcoded7_decoder_rules() {
        // REQ-PE-ENC-008: groups across Chunks; a last group of 1 byte is ignored; unused
        // header bits are ignored; a byte with bit 7 set is an error.
        assert_eq!(
            decode_all(&[0b0100_0001, 0x7F, 0x01], 1),
            Ok(std::vec![0xFF, 0x01])
        );
        assert_eq!(decode_all(&[0x00, 0x01, 0x00], 2).map(|v| v.len()), Ok(2));
        assert_eq!(
            decode_all(&[0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x7F], 3),
            Ok(std::vec![1, 2, 3, 4, 5, 6, 7])
        );
        assert_eq!(decode_all(&[0x80], 1), Err(EncodingError::NotSevenBit));
    }

    #[test]
    fn data_encoder_ascii_and_overflow() {
        // REQ-PE-ENC-006, REQ-PE-DATA-001; zlib is in the zlib module.
        let mut a = DataEncoder::new(EncodingName::Ascii).unwrap();
        let mut buf = [0u8; 4];
        assert_eq!(a.encode(b"ab", &mut buf), Ok(2));
        assert_eq!(
            a.encode(&[0x41, 0xC3], &mut buf),
            Err(EncodingError::NotSevenBit)
        );
        assert_eq!(a.encode(b"abcde", &mut buf), Err(EncodingError::Overflow));
        assert_eq!(
            DataEncoder::new(EncodingName::ZlibMcoded7).err(),
            Some(EncodingError::Unsupported)
        );
        assert_eq!(
            DataDecoder::new(EncodingName::ZlibMcoded7).err(),
            Some(EncodingError::Unsupported)
        );
        let mut m = DataEncoder::new(EncodingName::Mcoded7).unwrap();
        let mut small = [0u8; 7];
        assert_eq!(
            m.encode(&[0u8; 7], &mut small),
            Err(EncodingError::Overflow)
        );
        let mut d = DataDecoder::new(EncodingName::Ascii).unwrap();
        assert_eq!(d.decode(b"xyz", &mut buf), Ok(3));
        d.finish().unwrap();
    }

    #[test]
    fn prop_mcoded7_round_trip_in_pieces() {
        // REQ-PE-ENC-008, REQ-PE-DATA-003: Chunk boundaries anywhere.
        proptest!(|(data in proptest::collection::vec(any::<u8>(), 0..100), a in 1usize..20, b in 1usize..20)| {
            let enc = encode_all(&data, a);
            prop_assert_eq!(enc.len(), mcoded7_len(data.len()));
            prop_assert!(enc.iter().all(|x| *x < 0x80));
            prop_assert_eq!(decode_all(&enc, b).unwrap(), data);
        });
    }

    #[test]
    fn prop_decoder_needs_exactly_the_data_bytes() {
        // REQ-PE-ENC-008: after any first piece, `decode` writes every byte that is not a group
        // header byte, and gives Overflow (with no state change) for an `out` one byte shorter.
        proptest!(|(data in proptest::collection::vec(0u8..0x80, 0..60), cut in 0usize..60)| {
            let cut = cut.min(data.len());
            let (a, b) = data.split_at(cut);
            let mut d = Mcoded7Decoder::new();
            let mut buf = [0u8; 64];
            d.decode(a, &mut buf).unwrap();
            let heads = (0..b.len()).filter(|i| (cut + i) % 8 == 0).count();
            let need = b.len() - heads;
            if need > 0 {
                let before = d;
                prop_assert_eq!(d.decode(b, &mut buf[..need - 1]), Err(EncodingError::Overflow));
                prop_assert_eq!((d.n, d.head), (before.n, before.head));
            }
            prop_assert_eq!(d.decode(b, &mut buf[..need]), Ok(need));
        });
    }

    #[test]
    fn prop_decoder_never_panics() {
        proptest!(|(data in proptest::collection::vec(0u8..0x80, 0..40), p in 1usize..9)| {
            let _ = decode_all(&data, p);
        });
    }
}
