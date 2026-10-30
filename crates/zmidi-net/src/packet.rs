//! UDP payload framing of Network MIDI 2.0 (M2-124-UM v1.0.1, sections 5.1 to 5.4).
//!
//! - Every UDP payload is the Signature "MIDI" (0x4D494449) and then one or more Command
//!   Packets (REQ-NET-PKT-001). A Command Packet is one header word (Command Code, Command
//!   Payload Length in words, Command Specific Data 1 and 2) and then the payload words
//!   (REQ-NET-PKT-012, -015). All numbers are big-endian (REQ-NET-PKT-005).
//! - The reader works on a payload slice of the caller; the writer works on a buffer of the
//!   caller (DEC-002, DEC-012). The module gives no meaning to the Command Codes.
//! - Nothing panics on received bytes.

/// The Signature at the start of each UDP payload: "MIDI" (5.2, REQ-NET-PKT-001).
pub const SIGNATURE: [u8; 4] = *b"MIDI";
/// The default and largest UDP payload size of the writer, in bytes (5.1.1, N-31).
pub const MAX_PAYLOAD: usize = 1400;

/// Error of the packet reader and writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PacketError {
    /// The UDP payload is shorter than 4 bytes or does not start with the Signature
    /// (REQ-NET-PKT-003).
    #[error("no Signature")]
    BadSignature,
    /// The Command Packet does not fit in the rest of the payload or of the caller buffer:
    /// write it in a new payload (REQ-NET-PKT-004).
    #[error("Command Packet does not fit")]
    Overflow,
    /// The Signature and this one Command Packet are longer than the maximum payload
    /// (REQ-NET-PKT-004).
    #[error("Command Packet too large")]
    TooLarge,
    /// A value out of range: a payload above 255 words, a maximum payload above 1400 bytes,
    /// or a string that starts with a Byte Order Mark (REQ-NET-PKT-007).
    #[error("invalid value")]
    InvalidValue,
    /// A payload with no Command Packet (REQ-NET-PKT-001).
    #[error("no Command Packet")]
    Empty,
}

/// The header word of a Command Packet (REQ-NET-PKT-012, -015).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Header {
    /// Command Code, bits 31 to 24.
    pub code: u8,
    /// Command Payload Length in 32-bit words, bits 23 to 16.
    pub len: u8,
    /// Command Specific Data 1, bits 15 to 8.
    pub csd1: u8,
    /// Command Specific Data 2, bits 7 to 0.
    pub csd2: u8,
}

impl Header {
    /// The header from its 4 bytes.
    pub const fn from_bytes(b: [u8; 4]) -> Self {
        Header {
            code: b[0],
            len: b[1],
            csd1: b[2],
            csd2: b[3],
        }
    }

    /// The 4 bytes of the header.
    pub const fn to_bytes(self) -> [u8; 4] {
        [self.code, self.len, self.csd1, self.csd2]
    }

    /// Command Specific Data as one 16-bit value (for example a Sequence Number).
    pub const fn csd(self) -> u16 {
        u16::from_be_bytes([self.csd1, self.csd2])
    }

    /// The header word.
    pub const fn word(self) -> u32 {
        u32::from_be_bytes(self.to_bytes())
    }
}

/// One received Command Packet: its header and its payload bytes (4 bytes for each word).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawCommand<'a> {
    /// The header.
    pub header: Header,
    /// The payload, `4 * header.len` bytes.
    pub payload: &'a [u8],
}

impl<'a> RawCommand<'a> {
    /// The payload word at `index`, big-endian.
    pub fn word(&self, index: usize) -> Option<u32> {
        let start = index.checked_mul(4)?;
        let b = self.payload.get(start..start.checked_add(4)?)?;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// The payload words, big-endian.
    pub fn words(&self) -> impl Iterator<Item = u32> + 'a {
        self.payload
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_be_bytes(*b))
    }

    /// The payload bytes of the words `start .. start + count`; `None` when they are not there.
    pub fn field(&self, start: usize, count: usize) -> Option<&'a [u8]> {
        let begin = start.checked_mul(4)?;
        let end = start.checked_add(count)?.checked_mul(4)?;
        self.payload.get(begin..end)
    }
}

/// One item of [`CommandPackets`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketItem<'a> {
    /// A complete Command Packet.
    Command(RawCommand<'a>),
    /// A Command Packet whose payload goes past the end of the UDP payload: the read stops
    /// here, and the endpoint sends NAK 0x03 with this header (REQ-NET-PKT-014, N-13).
    Truncated(Header),
}

/// The Command Packets of one UDP payload, in order (REQ-NET-PKT-001, REQ-NET-CMD-005). It
/// skips each Command Packet by its payload length, so an unknown Command Code needs no
/// knowledge here. 1 to 3 bytes after the last Command Packet are ignored (REQ-NET-PKT-014).
#[derive(Debug, Clone)]
pub struct CommandPackets<'a> {
    rest: &'a [u8],
}

/// Checks the Signature of a UDP payload and gives its Command Packets (REQ-NET-PKT-003).
/// `BadSignature` means: ignore the payload, send nothing and change no state; the caller
/// may report it as a diagnostic only.
pub fn command_packets(payload: &[u8]) -> Result<CommandPackets<'_>, PacketError> {
    match payload.split_first_chunk::<4>() {
        Some((sig, rest)) if *sig == SIGNATURE => Ok(CommandPackets { rest }),
        _ => Err(PacketError::BadSignature),
    }
}

impl<'a> Iterator for CommandPackets<'a> {
    type Item = PacketItem<'a>;

    fn next(&mut self) -> Option<PacketItem<'a>> {
        let (h, body) = self.rest.split_first_chunk::<4>()?;
        let header = Header::from_bytes(*h);
        let n = usize::from(header.len) * 4;
        match body.split_at_checked(n) {
            Some((payload, rest)) => {
                self.rest = rest;
                Some(PacketItem::Command(RawCommand { header, payload }))
            }
            None => {
                self.rest = &[];
                Some(PacketItem::Truncated(header))
            }
        }
    }
}

/// The text of a received string field: the bytes up to the first 0x00, or the whole field
/// (REQ-NET-PKT-007, -008, N-32). The bytes are not checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetStr<'a>(&'a [u8]);

impl<'a> NetStr<'a> {
    /// The string of a field of whole words.
    pub fn from_field(field: &'a [u8]) -> Self {
        let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
        NetStr(&field[..end])
    }

    /// The bytes.
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.0
    }

    /// True for an empty string.
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The text; an error for bytes that are not valid UTF-8.
    pub fn to_str(&self) -> Result<&'a str, core::str::Utf8Error> {
        core::str::from_utf8(self.0)
    }
}

/// The number of 32-bit words of a string field of `len` bytes (REQ-NET-PKT-008).
pub const fn string_words(len: usize) -> usize {
    len.div_ceil(4)
}

/// The maximum UDP payload size of the writer: 1400 bytes by default, a smaller value when
/// the caller sets it (REQ-NET-PKT-004, N-31).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxPayload(u16);

impl Default for MaxPayload {
    fn default() -> Self {
        MaxPayload(MAX_PAYLOAD as u16)
    }
}

impl MaxPayload {
    /// A maximum of `bytes`; `InvalidValue` above 1400 or below 8 (the Signature and one
    /// header word).
    pub const fn new(bytes: usize) -> Result<Self, PacketError> {
        if bytes > MAX_PAYLOAD || bytes < 8 {
            Err(PacketError::InvalidValue)
        } else {
            Ok(MaxPayload(bytes as u16))
        }
    }

    /// The value in bytes.
    pub const fn bytes(self) -> usize {
        self.0 as usize
    }
}

/// Writes the payload words of one Command Packet (see [`PayloadWriter::command`]).
#[derive(Debug)]
pub struct WordWriter<'w> {
    buf: &'w mut [u8],
    len: usize,
}

impl WordWriter<'_> {
    fn room(&mut self, n: usize) -> Result<&mut [u8], PacketError> {
        let end = self.len.checked_add(n).ok_or(PacketError::Overflow)?;
        let r = self
            .buf
            .get_mut(self.len..end)
            .ok_or(PacketError::Overflow)?;
        self.len = end;
        Ok(r)
    }

    /// Writes one word, big-endian.
    pub fn word(&mut self, w: u32) -> Result<(), PacketError> {
        self.room(4)?.copy_from_slice(&w.to_be_bytes());
        Ok(())
    }

    /// Writes bytes and 0x00 fill to the end of the last word.
    pub fn padded(&mut self, bytes: &[u8]) -> Result<(), PacketError> {
        let n = string_words(bytes.len()) * 4;
        let r = self.room(n)?;
        r[..bytes.len()].copy_from_slice(bytes);
        r[bytes.len()..].fill(0);
        Ok(())
    }

    /// Writes a UTF-8 string field in the smallest number of words, 0x00 fill
    /// (REQ-NET-PKT-007, -008). A string that starts with a Byte Order Mark is refused.
    pub fn string(&mut self, s: &str) -> Result<(), PacketError> {
        if s.starts_with('\u{FEFF}') {
            return Err(PacketError::InvalidValue);
        }
        self.padded(s.as_bytes())
    }

    /// Words written.
    pub const fn words(&self) -> usize {
        self.len / 4
    }
}

/// Writes one UDP payload into a caller buffer: the Signature and then Command Packets
/// (REQ-NET-PKT-001, -004, -012).
#[derive(Debug)]
pub struct PayloadWriter<'b> {
    buf: &'b mut [u8],
    len: usize,
    max: usize,
}

impl<'b> PayloadWriter<'b> {
    /// A writer into `buf` with the limit `max`.
    pub fn new(buf: &'b mut [u8], max: MaxPayload) -> Self {
        PayloadWriter {
            buf,
            len: 0,
            max: max.bytes(),
        }
    }

    /// True before the first Command Packet.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Bytes written so far (0 before the first Command Packet).
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Writes one Command Packet: the header with `code`, `csd1` and `csd2`, and the payload
    /// that `payload` writes; the Command Payload Length is the number of words written. On
    /// an error nothing is written. `Overflow`: the Command Packet does not fit in this
    /// payload, write it in a new one. `TooLarge`: it does not fit in an empty payload.
    pub fn command(
        &mut self,
        code: u8,
        csd1: u8,
        csd2: u8,
        payload: impl FnOnce(&mut WordWriter<'_>) -> Result<(), PacketError>,
    ) -> Result<(), PacketError> {
        let limit = self.max.min(self.buf.len());
        let start = if self.len == 0 { 4 } else { self.len };
        // An error for a packet that does not fit: in a new payload it can fit, unless this
        // payload is still empty.
        let no_fit = if self.len == 0 {
            PacketError::TooLarge
        } else {
            PacketError::Overflow
        };
        let Some(area) = self.buf.get_mut(start..limit) else {
            return Err(no_fit);
        };
        let Some((head, body)) = area.split_first_chunk_mut::<4>() else {
            return Err(no_fit);
        };
        let mut w = WordWriter { buf: body, len: 0 };
        match payload(&mut w) {
            Ok(()) => {}
            Err(PacketError::Overflow) => return Err(no_fit),
            Err(e) => return Err(e),
        }
        let words = w.words();
        let len = u8::try_from(words).map_err(|_| PacketError::InvalidValue)?;
        *head = Header {
            code,
            len,
            csd1,
            csd2,
        }
        .to_bytes();
        if self.len == 0 {
            self.buf[..4].copy_from_slice(&SIGNATURE);
        }
        self.len = start + 4 + words * 4;
        Ok(())
    }

    /// The length of the finished UDP payload; `Empty` when no Command Packet was written.
    pub fn finish(self) -> Result<usize, PacketError> {
        if self.len == 0 {
            Err(PacketError::Empty)
        } else {
            Ok(self.len)
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;

    /// Appendix A.1.1 (Figure 13): Invitation, UMP Endpoint Name "MyDev", Product Instance Id
    /// "8shYe3h5".
    const A11: [u8; 24] = [
        b'M', b'I', b'D', b'I', 0x01, 0x04, 0x02, 0x00, b'M', b'y', b'D', b'e', b'v', 0, 0, 0,
        b'8', b's', b'h', b'Y', b'e', b'3', b'h', b'5',
    ];
    /// Appendix A.1.2 (Figure 14): UMP Data, Sequence Number 16, one UMP word 0x10F80000.
    const A12: [u8; 12] = [
        b'M', b'I', b'D', b'I', 0xFF, 0x01, 0x00, 0x10, 0x10, 0xF8, 0x00, 0x00,
    ];

    fn items(p: &[u8]) -> Vec<PacketItem<'_>> {
        command_packets(p).unwrap().collect()
    }

    #[test]
    fn raw_command_word_and_field_never_overflow() {
        // L1: an index near usize::MAX gives None, no overflow panic.
        let c = RawCommand {
            header: Header::from_bytes([0x01, 1, 0, 0]),
            payload: &[1, 2, 3, 4],
        };
        assert_eq!(c.word(0), Some(0x0102_0304));
        assert_eq!(c.word(usize::MAX), None);
        assert_eq!(c.word(usize::MAX / 4), None);
        assert_eq!(c.field(0, 1), Some(&[1u8, 2, 3, 4][..]));
        assert_eq!(c.field(usize::MAX, 1), None);
        assert_eq!(c.field(1, usize::MAX), None);
        assert_eq!(c.field(usize::MAX / 4, 1), None);
    }

    #[test]
    fn appendix_a_framing() {
        // REQ-NET-PKT-001, -005, -008, -012, -015.
        let it = items(&A11);
        let [PacketItem::Command(c)] = it.as_slice() else {
            panic!("{it:?}")
        };
        assert_eq!(
            c.header,
            Header {
                code: 1,
                len: 4,
                csd1: 2,
                csd2: 0
            }
        );
        assert_eq!(
            NetStr::from_field(c.field(0, 2).unwrap()).to_str(),
            Ok("MyDev")
        );
        assert_eq!(
            NetStr::from_field(c.field(2, 2).unwrap()).to_str(),
            Ok("8shYe3h5")
        );
        let it = items(&A12);
        let [PacketItem::Command(c)] = it.as_slice() else {
            panic!("{it:?}")
        };
        assert_eq!((c.header.code, c.header.csd()), (0xFF, 16));
        assert_eq!(c.word(0), Some(0x10F8_0000));
        assert_eq!(c.words().collect::<Vec<_>>(), [0x10F8_0000]);
        // The writer gives the same bytes.
        let mut buf = [0u8; 64];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        w.command(0x01, 2, 0, |w| {
            w.string("MyDev")?;
            w.padded(b"8shYe3h5")
        })
        .unwrap();
        let n = w.finish().unwrap();
        assert_eq!(&buf[..n], &A11);
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        w.command(0xFF, 0x00, 0x10, |w| w.word(0x10F8_0000))
            .unwrap();
        let n = w.finish().unwrap();
        assert_eq!(&buf[..n], &A12);
    }

    #[test]
    fn signature_check() {
        // REQ-NET-PKT-003.
        assert_eq!(
            command_packets(b"MID").err(),
            Some(PacketError::BadSignature)
        );
        assert_eq!(
            command_packets(b"MIDX\x01\x00\x00\x00").err(),
            Some(PacketError::BadSignature)
        );
        assert_eq!(command_packets(b"").err(), Some(PacketError::BadSignature));
        assert_eq!(items(b"MIDI").len(), 0);
    }

    #[test]
    fn truncated_and_trailing_bytes() {
        // REQ-NET-PKT-014, REQ-NET-CMD-005 (unknown code 0x77 is skipped by its length),
        // REQ-NET-PKT-015 (no payload: header word only).
        let mut p = Vec::from(*b"MIDI");
        p.extend([0x77, 0x01, 0, 0, 1, 2, 3, 4]); // unknown, 1 word
        p.extend([0x20, 0x00, 0x05, 0x00]); // header only
        p.extend([0xAB, 0xCD]); // 2 trailing bytes
        let it = items(&p);
        assert_eq!(it.len(), 2);
        assert!(
            matches!(it[0], PacketItem::Command(c) if c.header.code == 0x77 && c.payload == [1, 2, 3, 4])
        );
        assert!(
            matches!(it[1], PacketItem::Command(c) if c.header.code == 0x20 && c.payload.is_empty())
        );
        let mut p = Vec::from(*b"MIDI");
        p.extend([0x20, 0x00, 0, 0]);
        p.extend([0xFF, 0x03, 0, 1, 0x10, 0, 0, 0]); // 3 words announced, 1 present
        p.extend([0x20, 0x00, 0, 0]);
        let it = items(&p);
        assert_eq!(it.len(), 2);
        assert_eq!(
            it[1],
            PacketItem::Truncated(Header {
                code: 0xFF,
                len: 3,
                csd1: 0,
                csd2: 1
            })
        );
    }

    #[test]
    fn strings() {
        // REQ-NET-PKT-007, -008, -009.
        let mut buf = [0u8; 32];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        assert_eq!(
            w.command(1, 0, 0, |w| w.string("\u{FEFF}x")),
            Err(PacketError::InvalidValue)
        );
        w.command(1, 0, 0, |w| w.string("abcd")).unwrap(); // no 0x00 for 4 bytes
        w.command(2, 0, 0, |w| w.string("")).unwrap(); // empty: zero words
        let n = w.finish().unwrap();
        assert_eq!(&buf[..n], b"MIDI\x01\x01\x00\x00abcd\x02\x00\x00\x00");
        // The reader: a BOM or bytes that are not UTF-8 are given, not refused.
        let s = NetStr::from_field(&[0xEF, 0xBB, 0xBF, b'a']);
        assert_eq!(s.to_str(), Ok("\u{FEFF}a"));
        let s = NetStr::from_field(&[0xFF, b'a', 0, 0]);
        assert_eq!(s.as_bytes(), [0xFF, b'a']);
        assert!(s.to_str().is_err());
        assert!(NetStr::from_field(&[]).is_empty());
        assert_eq!(string_words(5), 2);
        assert_eq!(string_words(0), 0);
    }

    #[test]
    fn writer_limits() {
        // REQ-NET-PKT-001 (no empty payload), REQ-NET-PKT-004 (1400 bytes, smaller limit,
        // next payload, too large).
        assert_eq!(MaxPayload::new(1401), Err(PacketError::InvalidValue));
        assert_eq!(MaxPayload::default().bytes(), 1400);
        let mut buf = [0u8; 2000];
        assert_eq!(
            PayloadWriter::new(&mut buf, MaxPayload::default()).finish(),
            Err(PacketError::Empty)
        );
        let max = MaxPayload::new(16).unwrap();
        let mut w = PayloadWriter::new(&mut buf, max);
        assert!(w.is_empty());
        w.command(0x20, 0, 0, |w| w.word(1)).unwrap(); // 4 + 8 = 12 bytes
        assert!(!w.is_empty());
        assert_eq!(
            w.command(0x20, 0, 0, |w| w.word(2)),
            Err(PacketError::Overflow)
        );
        w.command(0x21, 0, 0, |_| Ok(())).unwrap(); // header only fits: 16 bytes
        assert_eq!(w.finish(), Ok(16));
        let mut w = PayloadWriter::new(&mut buf, max);
        assert_eq!(
            w.command(0xFF, 0, 0, |w| (0..4).try_for_each(|i| w.word(i))),
            Err(PacketError::TooLarge)
        );
        assert!(w.is_empty());
        // 256 words is above the 8-bit length.
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        let r = w.command(0xFF, 0, 0, |w| (0..256).try_for_each(|i| w.word(i)));
        assert_eq!(r, Err(PacketError::InvalidValue));
        assert!(w.is_empty());
    }

    #[test]
    fn prop_reader_never_panics() {
        // Nothing panics on received bytes; each Command lies inside the payload.
        proptest!(|(data in proptest::collection::vec(any::<u8>(), 0..300))| {
            let mut p = Vec::from(*b"MIDI");
            p.extend(&data);
            let mut total = 4;
            for it in command_packets(&p).unwrap() {
                if let PacketItem::Command(c) = it {
                    total += 4 + c.payload.len();
                    prop_assert_eq!(c.payload.len(), usize::from(c.header.len) * 4);
                }
            }
            prop_assert!(total <= p.len());
        });
    }

    #[test]
    fn prop_writer_reader_round_trip() {
        proptest!(|(cmds in proptest::collection::vec((any::<u8>(), any::<u8>(), any::<u8>(), proptest::collection::vec(any::<u32>(), 0..20)), 1..10))| {
            let mut buf = [0u8; 1400];
            let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
            for (code, a, b, words) in &cmds {
                w.command(*code, *a, *b, |w| words.iter().try_for_each(|x| w.word(*x))).unwrap();
            }
            let n = w.finish().unwrap();
            let got: Vec<_> = command_packets(&buf[..n]).unwrap().collect();
            prop_assert_eq!(got.len(), cmds.len());
            for (g, (code, a, b, words)) in got.iter().zip(&cmds) {
                let PacketItem::Command(c) = g else { panic!() };
                prop_assert_eq!(c.header, Header { code: *code, len: words.len() as u8, csd1: *a, csd2: *b });
                prop_assert_eq!(c.words().collect::<Vec<_>>(), words.clone());
            }
        });
    }
}
