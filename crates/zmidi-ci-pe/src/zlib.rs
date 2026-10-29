//! "zlib+Mcoded7" Property Data (M2-103-UM v1.2 6.1.6 Table 12, 6.2.2, 6.2.4), behind the Cargo
//! feature `zlib` (DEC-PE-004).
//!
//! - Reader: zlib (RFC 1950) inflate with the own streaming decoder of the private module
//!   `inflate`, with no alloc and no dependency (DEC-PE-004). The window is a caller buffer. A
//!   stream whose header declares a larger window gives `WindowTooLarge` (REQ-PE-ENC-009); the
//!   responder replies 413 to it (REQ-PE-STAT-011).
//! - Writer: the zlib header, stored deflate blocks (RFC 1951 3.2.4, BTYPE 00) and the Adler-32
//!   check, written into a caller buffer (REQ-PE-ENC-010). The writer does not make the data
//!   smaller.
//! - Order of the steps (6.2.2, 6.2.4): zlib, then Mcoded7, then Chunks. The reader reverses the
//!   order (REQ-PE-DATA-002). The data is streamed, never collected (DEC-007).
//!
//! Memory (measured on `thumbv7em-none-eabihf`, opt-level "s", 2026-10-18; OPT-010):
//! - [`ZlibInflater`] is 1072 bytes and [`ZlibMcoded7Decoder`] 1152. The window is a caller buffer
//!   of up to [`MAX_WINDOW`] (32768) bytes, as large as the window that the sender declares.
//! - A decoder made and used on the stack needs about 2.8 KB of stack
//!   (`scripts/stack-check.py --features zlib`). [`ZlibMcoded7Decoder::reset`] starts the next
//!   Data Set with the same decoder: a reset and a decode need about 600 bytes of stack.
//! - So the feature needs about 34 KB of RAM with a 32 KiB window. It is off by default.

use crate::inflate::Inflate;

use crate::encoding::{mcoded7_len, EncodingError, Mcoded7Decoder, Mcoded7Encoder};

/// Largest zlib window (RFC 1950 2.2: CINFO 7).
pub const MAX_WINDOW: usize = 32 * 1024;

/// The window size that the first byte (CMF) of a zlib stream declares: 2^(CINFO + 8). `None` when
/// the compression method is not 8 (deflate) or CINFO is above 7 (RFC 1950 2.2).
pub const fn window_size(cmf: u8) -> Option<usize> {
    if cmf & 0x0F != 8 || cmf >> 4 > 7 {
        None
    } else {
        Some(1 << ((cmf >> 4) + 8))
    }
}

/// Bytes read from the input and written to the output by one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    /// Input bytes consumed.
    pub consumed: usize,
    /// Output bytes written.
    pub written: usize,
}

/// Streaming zlib inflater with a caller window (REQ-PE-ENC-009). The decoder state is a fixed
/// size struct of about 1 KB (DEC-PE-004).
pub struct ZlibInflater<'w> {
    state: Inflate,
    window: &'w mut [u8],
    started: bool,
}

impl core::fmt::Debug for ZlibInflater<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ZlibInflater")
            .field("window", &self.window.len())
            .field("started", &self.started)
            .field("done", &self.state.is_done())
            .finish_non_exhaustive()
    }
}

impl<'w> ZlibInflater<'w> {
    /// A new inflater. It uses the largest power-of-two part of `window`, at most 32 KiB.
    pub fn new(window: &'w mut [u8]) -> Self {
        let len = window.len().min(MAX_WINDOW);
        let len = if len == 0 { 0 } else { 1 << len.ilog2() };
        ZlibInflater {
            state: Inflate::new(),
            window: &mut window[..len],
            started: false,
        }
    }

    /// Starts again for a new zlib stream with the same window, in place (OPT-010).
    pub fn reset(&mut self) {
        self.state = Inflate::new();
        self.started = false;
    }

    /// True after the end of the zlib stream and a correct Adler-32.
    pub fn is_done(&self) -> bool {
        self.state.is_done()
    }

    /// Inflates `input` into `out`. It stops when `out` is full; call it again (with the input not
    /// consumed, or with no input) to get the rest. Bytes after the end of the stream are
    /// consumed and ignored. Errors: `WindowTooLarge` when the header declares a window larger
    /// than the window buffer; `InvalidZlib` for data that is not a valid zlib stream.
    pub fn inflate(&mut self, input: &[u8], out: &mut [u8]) -> Result<Progress, EncodingError> {
        if !self.started {
            if let Some(&cmf) = input.first() {
                match window_size(cmf) {
                    None => return Err(EncodingError::InvalidZlib),
                    Some(w) if w > self.window.len() => return Err(EncodingError::WindowTooLarge),
                    Some(_) => self.started = true,
                }
            }
        }
        if !self.started {
            return Ok(Progress::default());
        }
        let step = self
            .state
            .inflate(input, self.window, out)
            .map_err(|_| EncodingError::InvalidZlib)?;
        let consumed = if self.state.is_done() {
            input.len() // bytes after the end of the stream are ignored
        } else {
            step.consumed
        };
        Ok(Progress {
            consumed,
            written: step.written,
        })
    }

    /// The end of the data: `InvalidZlib` when the stream did not end (data missing). Get all
    /// output with `inflate` before the call.
    pub fn finish(&mut self) -> Result<(), EncodingError> {
        if self.state.is_done() {
            Ok(())
        } else {
            Err(EncodingError::InvalidZlib)
        }
    }
}

/// Decoder of received "zlib+Mcoded7" Property Data: Mcoded7, then inflate (REQ-PE-DATA-002).
/// The Chunks of a Data Set are one stream.
#[derive(Debug)]
pub struct ZlibMcoded7Decoder<'w> {
    mcoded7: Mcoded7Decoder,
    inflater: ZlibInflater<'w>,
    buf: [u8; 64],
    start: usize,
    end: usize,
}

impl<'w> ZlibMcoded7Decoder<'w> {
    /// A new decoder with the window buffer `window` (see `ZlibInflater::new`).
    pub fn new(window: &'w mut [u8]) -> Self {
        ZlibMcoded7Decoder {
            mcoded7: Mcoded7Decoder::new(),
            inflater: ZlibInflater::new(window),
            buf: [0; 64],
            start: 0,
            end: 0,
        }
    }

    /// Starts again for the next "zlib+Mcoded7" Data Set with the same window, in place (see
    /// [`ZlibInflater::reset`]). Make the decoder once, for example in a `static`, and reset it
    /// for each Data Set (OPT-010).
    pub fn reset(&mut self) {
        self.mcoded7 = Mcoded7Decoder::new();
        self.inflater.reset();
        self.start = 0;
        self.end = 0;
    }

    /// Decodes `input` into `out`. It stops when `out` is full; call it again with the input not
    /// consumed, or with no input, to get the rest.
    pub fn decode(&mut self, input: &[u8], out: &mut [u8]) -> Result<Progress, EncodingError> {
        let mut p = Progress::default();
        loop {
            let r = self
                .inflater
                .inflate(&self.buf[self.start..self.end], &mut out[p.written..])?;
            self.start += r.consumed;
            p.written += r.written;
            if p.written == out.len() || self.start < self.end {
                break;
            }
            if self.inflater.is_done() {
                // Bytes after the end of the stream are ignored.
                p.consumed = input.len();
                break;
            }
            if p.consumed == input.len() {
                break;
            }
            // Mcoded7 gives at most one data byte for each input byte.
            let take = (input.len() - p.consumed).min(self.buf.len());
            self.end = self
                .mcoded7
                .decode(&input[p.consumed..p.consumed + take], &mut self.buf)?;
            self.start = 0;
            p.consumed += take;
        }
        Ok(p)
    }

    /// The end of the Data Set: `InvalidZlib` when the zlib stream did not end. Get all output
    /// with `decode` before the call.
    pub fn finish(&mut self) -> Result<(), EncodingError> {
        self.mcoded7.finish()?;
        self.inflater.finish()
    }
}

/// zlib header: CMF 0x78 (deflate, 32 KiB window), FLG 0x01 (no dictionary, FLEVEL 0).
const ZLIB_HEADER: [u8; 2] = [0x78, 0x01];
/// Largest stored block (RFC 1951 3.2.4: LEN is 16 bits).
const MAX_STORED: usize = 0xFFFF;

/// Running Adler-32 (RFC 1950 8.2).
#[derive(Debug, Clone, Copy)]
struct Adler32 {
    a: u32,
    b: u32,
}

impl Adler32 {
    const fn new() -> Self {
        Adler32 { a: 1, b: 0 }
    }

    fn update(&mut self, data: &[u8]) {
        for &x in data {
            self.a = (self.a + x as u32) % 65521;
            self.b = (self.b + self.a) % 65521;
        }
    }

    const fn value(&self) -> u32 {
        (self.b << 16) | self.a
    }
}

/// Encoder of "zlib+Mcoded7" Property Data: a zlib stream of stored blocks, then Mcoded7
/// (REQ-PE-ENC-010, REQ-PE-DATA-002).
#[derive(Debug, Clone)]
pub struct ZlibMcoded7Encoder {
    mcoded7: Mcoded7Encoder,
    adler: Adler32,
    started: bool,
}

impl Default for ZlibMcoded7Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ZlibMcoded7Encoder {
    /// A new encoder.
    pub const fn new() -> Self {
        ZlibMcoded7Encoder {
            mcoded7: Mcoded7Encoder::new(),
            adler: Adler32::new(),
            started: false,
        }
    }

    fn header_len(&self) -> usize {
        if self.started {
            0
        } else {
            ZLIB_HEADER.len()
        }
    }

    fn feed(&mut self, bytes: &[u8], out: &mut [u8], w: &mut usize) {
        // The caller checked the output size first, so this cannot overflow.
        if let Ok(n) = self.mcoded7.encode(bytes, &mut out[*w..]) {
            *w += n;
        }
    }

    fn start(&mut self, out: &mut [u8], w: &mut usize) {
        if !self.started {
            self.feed(&ZLIB_HEADER, out, w);
            self.started = true;
        }
    }

    /// Encodes `input` as stored blocks; complete Mcoded7 groups go to `out`. Gives the number of
    /// bytes written. On Overflow nothing is consumed.
    pub fn encode(&mut self, input: &[u8], out: &mut [u8]) -> Result<usize, EncodingError> {
        if input.is_empty() {
            return Ok(0);
        }
        let blocks = input.len().div_ceil(MAX_STORED);
        let z = self.header_len() + blocks * 5 + input.len();
        if out.len() < (self.mcoded7.pending() + z) / 7 * 8 {
            return Err(EncodingError::Overflow);
        }
        let mut w = 0;
        self.start(out, &mut w);
        for block in input.chunks(MAX_STORED) {
            let len = block.len() as u16;
            let [l0, l1] = len.to_le_bytes();
            self.feed(&[0x00, l0, l1, !l0, !l1], out, &mut w);
            self.feed(block, out, &mut w);
            self.adler.update(block);
        }
        Ok(w)
    }

    /// Writes the last stored block, the Adler-32 and the last Mcoded7 group. Gives the number of
    /// bytes written. On Overflow nothing is written.
    pub fn finish(&mut self, out: &mut [u8]) -> Result<usize, EncodingError> {
        let z = self.header_len() + 5 + 4;
        if out.len() < mcoded7_len(self.mcoded7.pending() + z) {
            return Err(EncodingError::Overflow);
        }
        let mut w = 0;
        self.start(out, &mut w);
        self.feed(&[0x01, 0x00, 0x00, 0xFF, 0xFF], out, &mut w);
        self.feed(&self.adler.value().to_be_bytes(), out, &mut w);
        w += self.mcoded7.finish(&mut out[w..])?;
        *self = Self::new();
        Ok(w)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;

    const TEXT: &[u8] = br#"{"name":"Zetaohm","x":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;
    /// Python zlib.compressobj(9, DEFLATED, 15) of TEXT.
    const Z15: [u8; 34] = [
        0x78, 0xDA, 0xAB, 0x56, 0xCA, 0x4B, 0xCC, 0x4D, 0x55, 0xB2, 0x52, 0x8A, 0x4A, 0x2D, 0x49,
        0xCC, 0xCF, 0xC8, 0x55, 0xD2, 0x51, 0xAA, 0x00, 0xF2, 0x12, 0x89, 0x04, 0x4A, 0xB5, 0x00,
        0xD5, 0x56, 0x16, 0xC2,
    ];

    fn inflate_all(
        z: &[u8],
        window: &mut [u8],
        piece: usize,
        out_piece: usize,
    ) -> Result<Vec<u8>, EncodingError> {
        let mut inf = ZlibInflater::new(window);
        let mut res = Vec::new();
        let mut buf = std::vec![0u8; out_piece.max(1)];
        for mut p in z.chunks(piece.max(1)) {
            loop {
                let r = inf.inflate(p, &mut buf)?;
                res.extend_from_slice(&buf[..r.written]);
                p = &p[r.consumed..];
                if p.is_empty() && r.written < buf.len() {
                    break;
                }
            }
        }
        inf.finish()?;
        Ok(res)
    }

    fn mcoded7(data: &[u8]) -> Vec<u8> {
        let mut e = Mcoded7Encoder::new();
        let mut out = std::vec![0u8; mcoded7_len(data.len())];
        let n = e.encode(data, &mut out).unwrap();
        let m = e.finish(&mut out[n..]).unwrap();
        out.truncate(n + m);
        out
    }

    /// Data of several shapes: random bytes, runs, a repeated pattern, text from a small
    /// alphabet. Lengths up to 100 KB, so that distances wrap in a 32 KiB window.
    fn shaped_data() -> impl Strategy<Value = Vec<u8>> {
        prop_oneof![
            proptest::collection::vec(any::<u8>(), 0..3000),
            (any::<u8>(), 0usize..100_000)
                .prop_map(|(s, n)| (0..n).map(|i| ((i / 97) as u8) ^ s).collect()),
            (
                proptest::collection::vec(any::<u8>(), 1..40),
                0usize..60_000
            )
                .prop_map(|(pat, n)| pat.iter().cycle().take(n).copied().collect()),
            (any::<u64>(), 0usize..50_000).prop_map(|(seed, n)| {
                let mut x = seed | 1;
                (0..n)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        let alphabet = b"  eetaoinshrdlu{}\":,0123";
                        alphabet[(x % alphabet.len() as u64) as usize]
                    })
                    .collect()
            }),
        ]
    }

    #[test]
    fn prop_inflate_matches_the_reference_at_every_level() {
        // DEC-PE-004: the own inflater gives the input back for streams of a reference
        // compressor at every level (stored, fixed and dynamic blocks), in any piece sizes.
        proptest!(ProptestConfig::with_cases(256), |(
            data in shaped_data(),
            level in 0u8..=10,
            piece in 1usize..400,
            out_piece in 1usize..700,
        )| {
            let z = miniz_oxide::deflate::compress_to_vec_zlib(&data, level);
            let mut w = std::vec![0u8; MAX_WINDOW];
            prop_assert_eq!(inflate_all(&z, &mut w, piece, out_piece).unwrap(), data);
        });
    }

    #[test]
    fn prop_changed_streams_agree_with_the_reference() {
        // DEC-PE-004: for a stream with changed bytes, the own inflater and the reference
        // decoder agree: both give the same output or both give an error. Never a panic.
        proptest!(ProptestConfig::with_cases(4000), |(
            data in proptest::collection::vec(0u8..8, 0..2000),
            level in 0u8..=10,
            flips in proptest::collection::vec((any::<proptest::sample::Index>(), 1u8..=255), 1..4),
            piece in 1usize..100,
        )| {
            let mut z = miniz_oxide::deflate::compress_to_vec_zlib(&data, level);
            for (i, x) in flips {
                let k = i.index(z.len());
                z[k] ^= x;
            }
            let mut w = std::vec![0u8; MAX_WINDOW];
            let ours = inflate_all(&z, &mut w, piece, 256);
            let reference = miniz_oxide::inflate::decompress_to_vec_zlib(&z);
            match (ours, reference) {
                (Ok(a), Ok(b)) => prop_assert_eq!(a, b),
                (Err(_), Err(_)) => {}
                (a, b) => prop_assert!(
                    false,
                    "own {:?}, reference {:?}",
                    a.map(|v| v.len()),
                    b.map(|v| v.len())
                ),
            }
        });
    }

    #[test]
    fn small_window_stream_decodes_with_its_window() {
        // A stream with CINFO 0 (a 256-byte window, RFC 1950 2.2) and stored blocks: a
        // 256-byte window buffer decodes it; a 128-byte one gives WindowTooLarge.
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let mut z = std::vec![0x08, 0x1D];
        assert_eq!((0x08u32 * 256 + 0x1D) % 31, 0);
        for (k, part) in data.chunks(300).enumerate() {
            let last = (k + 1) * 300 >= data.len();
            z.push(u8::from(last));
            let n = part.len() as u16;
            z.extend_from_slice(&n.to_le_bytes());
            z.extend_from_slice(&(!n).to_le_bytes());
            z.extend_from_slice(part);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &data {
            a = (a + u32::from(x)) % 65521;
            b = (b + a) % 65521;
        }
        z.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut w = [0u8; 256];
        assert_eq!(inflate_all(&z, &mut w, 7, 13).unwrap(), data);
        let mut w = [0u8; 128];
        assert_eq!(
            inflate_all(&z, &mut w, 7, 13),
            Err(EncodingError::WindowTooLarge)
        );
    }

    #[test]
    fn zlib_inflate_vectors() {
        // REQ-PE-ENC-009: Python zlib vectors, window 32 KiB (CMF 0x78) and 512 (CMF 0x18).
        let mut window = [0u8; MAX_WINDOW];
        assert_eq!(inflate_all(&Z15, &mut window, 34, 64).unwrap(), TEXT);
        assert_eq!(inflate_all(&Z15, &mut window, 1, 1).unwrap(), TEXT);
        let mut z9 = Z15;
        z9[..2].copy_from_slice(&[0x18, 0xD3]);
        let mut small = [0u8; 600];
        assert_eq!(inflate_all(&z9, &mut small, 5, 7).unwrap(), TEXT);
        assert_eq!(window_size(0x78), Some(32768));
        assert_eq!(window_size(0x18), Some(512));
        assert_eq!(window_size(0x88), None);
        assert_eq!(window_size(0x77), None);
    }

    #[test]
    fn zlib_window_too_large() {
        // REQ-PE-ENC-009, REQ-PE-STAT-011: a 32 KiB stream with a 16 KiB window buffer.
        let mut window = [0u8; 16 * 1024];
        assert_eq!(
            inflate_all(&Z15, &mut window, 34, 64),
            Err(EncodingError::WindowTooLarge)
        );
        let mut none: [u8; 0] = [];
        assert_eq!(
            inflate_all(&Z15, &mut none, 34, 64),
            Err(EncodingError::WindowTooLarge)
        );
    }

    #[test]
    fn zlib_bad_stream_errors() {
        // REQ-PE-ENC-009, REQ-PE-DATA-002: errors, never a panic.
        let mut window = [0u8; MAX_WINDOW];
        let mut bad = Z15;
        bad[33] ^= 1; // Adler-32
        assert_eq!(
            inflate_all(&bad, &mut window, 34, 64),
            Err(EncodingError::InvalidZlib)
        );
        assert_eq!(
            inflate_all(&Z15[..20], &mut window, 34, 64),
            Err(EncodingError::InvalidZlib)
        );
        assert_eq!(
            inflate_all(&[0x79, 0x9C, 0, 0], &mut window, 4, 64),
            Err(EncodingError::InvalidZlib)
        );
        let mut d = ZlibMcoded7Decoder::new(&mut window);
        let mut out = [0u8; 8];
        assert_eq!(d.decode(&[0x80], &mut out), Err(EncodingError::NotSevenBit));
    }

    #[test]
    fn prop_zlib_never_panics() {
        // REQ-PE-ENC-009: any input gives data or an error.
        proptest!(|(data in proptest::collection::vec(any::<u8>(), 0..200), piece in 1usize..40)| {
            let mut window = [0u8; 1024];
            let _ = inflate_all(&data, &mut window, piece, 13);
        });
    }

    #[test]
    fn zlib_stored_writer_vector() {
        // REQ-PE-ENC-010: header 78 01, one stored block, last empty block, Adler-32 of "abc",
        // then Mcoded7 (REQ-PE-DATA-002). Python zlib.decompress gives "abc" for these bytes.
        let mut e = ZlibMcoded7Encoder::new();
        let mut out = [0u8; 32];
        let n = e.encode(b"abc", &mut out).unwrap();
        let m = e.finish(&mut out[n..]).unwrap();
        let zlib = [
            0x78, 0x01, 0x00, 0x03, 0x00, 0xFC, 0xFF, 0x61, 0x62, 0x63, 0x01, 0x00, 0x00, 0xFF,
            0xFF, 0x02, 0x4D, 0x01, 0x27,
        ];
        assert_eq!(&out[..n + m], mcoded7(&zlib).as_slice());
        let mut small = [0u8; 4];
        assert_eq!(
            e.encode(b"abcdefgh", &mut small),
            Err(EncodingError::Overflow)
        );
        assert_eq!(e.finish(&mut small), Err(EncodingError::Overflow));
    }

    #[test]
    fn json_escape_comes_before_the_compression() {
        // REQ-PE-DATA-002: JSON data is escaped (6.1.1), then compressed (here one stored block of
        // the 14 escaped bytes), then Mcoded7; the reader gives the escaped text back.
        let mut text = [0u8; 32];
        let mut w = crate::json::JsonWriter::new(&mut text);
        w.begin_object().unwrap();
        w.name("t", 20).unwrap();
        w.string("\u{266a}").unwrap();
        w.end_object().unwrap();
        let n = w.finish().unwrap();
        let text = &text[..n];
        assert_eq!(text, br#"{"t":"\u266a"}"#);
        let mut e = ZlibMcoded7Encoder::new();
        let mut out = [0u8; 64];
        let mut k = e.encode(text, &mut out).unwrap();
        k += e.finish(&mut out[k..]).unwrap();
        // Python: zlib.decompress of these bytes gives the escaped text.
        let mut zlib = std::vec![0x78, 0x01, 0x00, 0x0E, 0x00, 0xF1, 0xFF];
        zlib.extend_from_slice(text);
        zlib.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF, 0x1D, 0xE7, 0x03, 0xFF]);
        assert_eq!(&out[..k], mcoded7(&zlib).as_slice());
        let mut window = [0u8; MAX_WINDOW];
        let mut d = ZlibMcoded7Decoder::new(&mut window);
        let mut back = [0u8; 32];
        let r = d.decode(&out[..k], &mut back).unwrap();
        d.finish().unwrap();
        assert_eq!(&back[..r.written], text);
    }

    #[test]
    fn reset_decodes_the_next_data_set_with_the_same_decoder() {
        // OPT-010: one decoder, reset between Data Sets: after a whole stream, in the middle of a
        // stream, and after a bad stream, the next stream decodes as with a new decoder.
        let a: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let b: Vec<u8> = (0..40_000u32).map(|i| (i / 7) as u8).collect();
        let za = mcoded7(&miniz_oxide::deflate::compress_to_vec_zlib(&a, 6));
        let zb = mcoded7(&miniz_oxide::deflate::compress_to_vec_zlib(&b, 6));
        let mut window = [0u8; MAX_WINDOW];
        let mut d = ZlibMcoded7Decoder::new(&mut window);
        let mut buf = std::vec![0u8; 256];
        let mut all =
            |d: &mut ZlibMcoded7Decoder<'_>, mut p: &[u8]| -> Result<Vec<u8>, EncodingError> {
                let mut got = Vec::new();
                loop {
                    let r = d.decode(p, &mut buf)?;
                    got.extend_from_slice(&buf[..r.written]);
                    p = &p[r.consumed..];
                    if p.is_empty() && r.written < buf.len() {
                        break;
                    }
                }
                d.finish().map(|()| got)
            };
        assert_eq!(all(&mut d, &za).unwrap(), a);
        d.reset();
        assert_eq!(all(&mut d, &zb).unwrap(), b);
        // Half of a stream, then a reset.
        d.reset();
        let _ = d.decode(&zb[..zb.len() / 2], &mut [0u8; 64]);
        d.reset();
        assert_eq!(all(&mut d, &za).unwrap(), a);
        // A bad stream (CINFO 15 in the zlib header), then a reset.
        d.reset();
        assert!(all(&mut d, &mcoded7(&[0xF8, 0x01, 0, 0])).is_err());
        d.reset();
        assert_eq!(all(&mut d, &zb).unwrap(), b);
    }

    #[test]
    fn prop_zlib_mcoded7_round_trip_in_pieces() {
        // REQ-PE-ENC-009, REQ-PE-ENC-010, REQ-PE-DATA-002: writer -> reader, and a reference
        // compressor (miniz_oxide with alloc, dev only) -> reader, in any Chunk sizes. The data is
        // larger than the window, so the window wraps.
        proptest!(|(seed in any::<u8>(), len in 0usize..70_000, piece in 1usize..300, out_piece in 1usize..500)| {
            let data: Vec<u8> = (0..len).map(|i| (i / 97) as u8 ^ seed).collect();
            let mut e = ZlibMcoded7Encoder::new();
            let mut enc = std::vec![0u8; mcoded7_len(len + 5 * (len.div_ceil(piece) + 1) + 6)];
            let mut n = 0;
            for p in data.chunks(piece) {
                n += e.encode(p, &mut enc[n..]).unwrap();
            }
            n += e.finish(&mut enc[n..]).unwrap();
            let reference = mcoded7(&miniz_oxide::deflate::compress_to_vec_zlib(&data, 6));
            for stream in [&enc[..n], &reference[..]] {
                let mut window = [0u8; MAX_WINDOW];
                let mut d = ZlibMcoded7Decoder::new(&mut window);
                let mut got = Vec::new();
                let mut buf = std::vec![0u8; out_piece];
                for mut p in stream.chunks(piece) {
                    loop {
                        let r = d.decode(p, &mut buf).unwrap();
                        got.extend_from_slice(&buf[..r.written]);
                        p = &p[r.consumed..];
                        if p.is_empty() && r.written < buf.len() {
                            break;
                        }
                    }
                }
                d.finish().unwrap();
                prop_assert_eq!(&got, &data);
            }
        });
    }
}
