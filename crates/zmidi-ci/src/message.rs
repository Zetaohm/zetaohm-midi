//! MIDI-CI common message layer (M2-101-UM v1.2.1 sections 3, 4, 5.1-5.4, Appendix E,
//! Tables 4 and 5).
//!
//! - Every MIDI-CI message is a Universal System Exclusive message: `0x7E`, Device ID,
//!   Sub-ID#1 `0x0D`, Sub-ID#2, Message Version/Format, Source MUID, Destination MUID
//!   (Table 5). A MUID is 28 bits in four 7-bit bytes, least significant byte first.
//! - In a MIDI 1.0 byte stream the message has `0xF0` and `0xF7` (REQ-CI-MSG-002); in UMP
//!   System Exclusive 7 packets it has neither (REQ-CI-MSG-003). Readers accept both forms.
//! - The writer always writes Message Version/Format 0x02 (REQ-CI-MSG-006). A received
//!   0x00 is read as 0x01 (DEC-011 D-8); bits 6-5 are reserved (DEC-011 D-9).
//! - Message Text is ASCII with the escapes of ECMA-404 (M2-101 4.6, REQ-CI-TXT-*).
//!
//! Readers are zero-copy over a byte slice and never panic; writers write into a caller
//! buffer and return an Overflow error when it is too small (DEC-002).

/// Error of the readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The message is not a MIDI-CI message (no `0x7E` .. `0x0D`).
    #[error("not a MIDI-CI message")]
    NotMidiCi,
    /// Fewer bytes than the format needs.
    #[error("message too short")]
    TooShort,
    /// A Device ID from 0x10 to 0x7D (REQ-CI-MSG-005).
    #[error("reserved Device ID")]
    ReservedDeviceId,
    /// A data byte with bit 7 set.
    #[error("data byte with bit 7 set")]
    InvalidByte,
    /// A Sub-ID#2 that the crate does not support (REQ-CI-SUBID-001).
    #[error("unsupported Sub-ID#2")]
    UnsupportedSubId,
    /// A field value outside its range.
    #[error("field value out of range")]
    InvalidValue,
}

/// Error of the writers and checked constructors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// The caller buffer is full.
    #[error("buffer overflow")]
    Overflow,
    /// A value that does not fit its field.
    #[error("value out of range")]
    InvalidValue,
}

/// A MIDI-CI MUID (M2-101 3.2): 28 bits, at most 0x0FFFFFFF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Muid(u32);

impl Muid {
    /// The Broadcast MUID (REQ-CI-MUID-007).
    pub const BROADCAST: Muid = Muid(0x0FFF_FFFF);
    /// First value of the reserved range 0x0FFFFF00-0x0FFFFFFE (REQ-CI-MUID-006).
    const RESERVED_START: u32 = 0x0FFF_FF00;

    /// Checked constructor (REQ-CI-MUID-003).
    pub const fn new(value: u32) -> Result<Self, WriteError> {
        if value > 0x0FFF_FFFF {
            Err(WriteError::InvalidValue)
        } else {
            Ok(Muid(value))
        }
    }

    /// The 28-bit value.
    pub const fn value(self) -> u32 {
        self.0
    }

    /// Reads four 7-bit bytes, least significant byte first.
    pub fn from_bytes(bytes: [u8; 4]) -> Result<Self, ParseError> {
        let mut v = 0u32;
        for (i, b) in bytes.iter().enumerate() {
            if *b > 0x7F {
                return Err(ParseError::InvalidByte);
            }
            v |= (*b as u32) << (7 * i);
        }
        Ok(Muid(v))
    }

    /// The four 7-bit bytes, least significant byte first.
    pub const fn to_bytes(self) -> [u8; 4] {
        [
            (self.0 & 0x7F) as u8,
            ((self.0 >> 7) & 0x7F) as u8,
            ((self.0 >> 14) & 0x7F) as u8,
            ((self.0 >> 21) & 0x7F) as u8,
        ]
    }

    /// True for the Broadcast MUID.
    pub const fn is_broadcast(self) -> bool {
        self.0 == Self::BROADCAST.0
    }

    /// A Device MUID from a caller random number (REQ-CI-MUID-006, REQ-CI-MUID-011).
    ///
    /// Gives `None` for the reserved range, the Broadcast MUID or a value equal to `old`;
    /// the caller then tries another random number.
    pub fn from_random(random: u32, old: Option<Muid>) -> Option<Muid> {
        let v = random & 0x0FFF_FFFF;
        if v >= Self::RESERVED_START || Some(Muid(v)) == old {
            None
        } else {
            Some(Muid(v))
        }
    }
}

/// The Device ID byte (Table 5, REQ-CI-ADDR-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceId {
    /// MIDI Channel 0-15 (Channels 1-16).
    Channel(u8),
    /// The whole Group (0x7E).
    Group,
    /// The whole Function Block (0x7F).
    FunctionBlock,
}

impl DeviceId {
    /// Reads the byte; 0x10-0x7D are reserved (REQ-CI-MSG-005).
    pub fn from_byte(byte: u8) -> Result<Self, ParseError> {
        match byte {
            0x00..=0x0F => Ok(DeviceId::Channel(byte)),
            0x7E => Ok(DeviceId::Group),
            0x7F => Ok(DeviceId::FunctionBlock),
            0x80..=0xFF => Err(ParseError::InvalidByte),
            _ => Err(ParseError::ReservedDeviceId),
        }
    }

    /// The byte. A Channel above 15 is written as its low 4 bits; constructors check it.
    pub const fn to_byte(self) -> u8 {
        match self {
            DeviceId::Channel(c) => c & 0x0F,
            DeviceId::Group => 0x7E,
            DeviceId::FunctionBlock => 0x7F,
        }
    }
}

/// Message Category (Table 4, REQ-CI-MSG-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// 0x1, deprecated.
    ProtocolNegotiation,
    /// 0x2.
    ProfileConfiguration,
    /// 0x3.
    PropertyExchange,
    /// 0x4.
    ProcessInquiry,
    /// 0x7.
    Management,
    /// 0x0, 0x5, 0x6: the nibble.
    Reserved(u8),
}

impl Category {
    /// The Category from the high nibble of a Sub-ID#2 byte.
    pub const fn from_sub_id2(byte: u8) -> Self {
        match (byte >> 4) & 0x07 {
            0x1 => Category::ProtocolNegotiation,
            0x2 => Category::ProfileConfiguration,
            0x3 => Category::PropertyExchange,
            0x4 => Category::ProcessInquiry,
            0x7 => Category::Management,
            n => Category::Reserved(n),
        }
    }
}

/// Sub-ID#2 of every supported MIDI-CI message (Appendix E, REQ-CI-SUBID-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SubId2 {
    ProfileInquiry = 0x20,
    ProfileInquiryReply = 0x21,
    SetProfileOn = 0x22,
    SetProfileOff = 0x23,
    ProfileEnabled = 0x24,
    ProfileDisabled = 0x25,
    ProfileAdded = 0x26,
    ProfileRemoved = 0x27,
    ProfileDetailsInquiry = 0x28,
    ProfileDetailsReply = 0x29,
    ProfileSpecificData = 0x2F,
    PeCapabilities = 0x30,
    PeCapabilitiesReply = 0x31,
    GetPropertyData = 0x34,
    GetPropertyDataReply = 0x35,
    SetPropertyData = 0x36,
    SetPropertyDataReply = 0x37,
    Subscription = 0x38,
    SubscriptionReply = 0x39,
    Notify = 0x3F,
    ProcessInquiryCapabilities = 0x40,
    ProcessInquiryCapabilitiesReply = 0x41,
    MidiMessageReport = 0x42,
    MidiMessageReportReply = 0x43,
    MidiMessageReportEnd = 0x44,
    Discovery = 0x70,
    DiscoveryReply = 0x71,
    EndpointInfo = 0x72,
    EndpointInfoReply = 0x73,
    Ack = 0x7D,
    InvalidateMuid = 0x7E,
    Nak = 0x7F,
}

impl SubId2 {
    /// Reads a Sub-ID#2; `None` for reserved, unlisted and Protocol Negotiation values.
    pub const fn from_byte(byte: u8) -> Option<Self> {
        use SubId2::*;
        Some(match byte {
            0x20 => ProfileInquiry,
            0x21 => ProfileInquiryReply,
            0x22 => SetProfileOn,
            0x23 => SetProfileOff,
            0x24 => ProfileEnabled,
            0x25 => ProfileDisabled,
            0x26 => ProfileAdded,
            0x27 => ProfileRemoved,
            0x28 => ProfileDetailsInquiry,
            0x29 => ProfileDetailsReply,
            0x2F => ProfileSpecificData,
            0x30 => PeCapabilities,
            0x31 => PeCapabilitiesReply,
            0x34 => GetPropertyData,
            0x35 => GetPropertyDataReply,
            0x36 => SetPropertyData,
            0x37 => SetPropertyDataReply,
            0x38 => Subscription,
            0x39 => SubscriptionReply,
            0x3F => Notify,
            0x40 => ProcessInquiryCapabilities,
            0x41 => ProcessInquiryCapabilitiesReply,
            0x42 => MidiMessageReport,
            0x43 => MidiMessageReportReply,
            0x44 => MidiMessageReportEnd,
            0x70 => Discovery,
            0x71 => DiscoveryReply,
            0x72 => EndpointInfo,
            0x73 => EndpointInfoReply,
            0x7D => Ack,
            0x7E => InvalidateMuid,
            0x7F => Nak,
            _ => return None,
        })
    }

    /// The byte.
    pub const fn to_byte(self) -> u8 {
        self as u8
    }

    /// The Category of the message.
    pub const fn category(self) -> Category {
        Category::from_sub_id2(self as u8)
    }
}

/// The Message Version/Format byte (Table 5, REQ-CI-MSG-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version(u8);

impl Version {
    /// Version 0x02 (MIDI-CI 1.2), written in every message (REQ-CI-MSG-006).
    pub const CURRENT: Version = Version(0x02);

    /// Reads the byte; 0x00 is read as 0x01 (DEC-011 D-8, REQ-CI-MSG-014).
    pub const fn from_byte(byte: u8) -> Result<Self, ParseError> {
        if byte > 0x7F {
            Err(ParseError::InvalidByte)
        } else if byte == 0 {
            Ok(Version(0x01))
        } else {
            Ok(Version(byte))
        }
    }

    /// The minor version, bits 4-0 (DEC-011 D-9).
    pub const fn minor(self) -> u8 {
        self.0 & 0x1F
    }

    /// True when the reserved bits 6-5 are set (REQ-CI-MSG-015 answers with NAK 0x02).
    pub const fn has_reserved_bits(self) -> bool {
        self.0 & 0x60 != 0
    }

    /// The byte.
    pub const fn to_byte(self) -> u8 {
        self.0
    }
}

/// Framing of a message (REQ-CI-MSG-002, REQ-CI-MSG-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Framing {
    /// UMP System Exclusive 7 payload: no `0xF0`/`0xF7`.
    #[default]
    Bare,
    /// MIDI 1.0 byte stream: `0xF0` .. `0xF7`.
    SysEx,
}

/// The common header of a MIDI-CI message (Table 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Device ID (address), 7 bits.
    pub device_id: DeviceId,
    /// Sub-ID#2 byte as received, 7 bits.
    pub sub_id2: u8,
    /// Message Version/Format, 7 bits.
    pub version: Version,
    /// Source MUID, 28 bits.
    pub source: Muid,
    /// Destination MUID, 28 bits.
    pub destination: Muid,
}

impl Header {
    /// Header length without framing bytes.
    pub const LEN: usize = 13;

    /// Reads a message and gives the header and the body after the Destination MUID.
    pub fn parse(msg: &[u8]) -> Result<(Header, &[u8]), ParseError> {
        let msg = match msg.first() {
            Some(0xF0) => {
                let inner = &msg[1..];
                match inner.last() {
                    Some(0xF7) => &inner[..inner.len() - 1],
                    _ => inner,
                }
            }
            _ => msg,
        };
        if msg.len() < 4 || msg[0] != 0x7E || msg[2] != 0x0D {
            return Err(ParseError::NotMidiCi);
        }
        if msg.len() < Self::LEN {
            return Err(ParseError::TooShort);
        }
        if high_bit(msg) {
            return Err(ParseError::InvalidByte);
        }
        let device_id = DeviceId::from_byte(msg[1])?;
        let header = Header {
            device_id,
            sub_id2: msg[3],
            version: Version::from_byte(msg[4])?,
            source: Muid::from_bytes([msg[5], msg[6], msg[7], msg[8]])?,
            destination: Muid::from_bytes([msg[9], msg[10], msg[11], msg[12]])?,
        };
        Ok((header, &msg[Self::LEN..]))
    }

    /// Starts a message in `out`: framing start and the 13 header bytes, with
    /// Version::CURRENT whatever `self.version` holds (REQ-CI-MSG-007).
    pub fn write<'b>(&self, framing: Framing, out: &'b mut [u8]) -> Result<Writer<'b>, WriteError> {
        let mut w = Writer::new(framing, out)?;
        w.u7(0x7E)?;
        w.u7(self.device_id.to_byte())?;
        w.u7(0x0D)?;
        w.u7(self.sub_id2)?;
        w.u7(Version::CURRENT.to_byte())?;
        w.u28(self.source.value())?;
        w.u28(self.destination.value())?;
        Ok(w)
    }
}

/// A cursor over a caller buffer for message writers.
#[derive(Debug)]
pub struct Writer<'b> {
    buf: &'b mut [u8],
    pos: usize,
    framing: Framing,
}

#[allow(clippy::len_without_is_empty)]
impl<'b> Writer<'b> {
    /// Starts a writer; SysEx framing writes `0xF0`.
    pub fn new(framing: Framing, buf: &'b mut [u8]) -> Result<Self, WriteError> {
        let mut w = Writer {
            buf,
            pos: 0,
            framing,
        };
        if framing == Framing::SysEx {
            w.raw(0xF0)?;
        }
        Ok(w)
    }

    fn raw(&mut self, b: u8) -> Result<(), WriteError> {
        let slot = self.buf.get_mut(self.pos).ok_or(WriteError::Overflow)?;
        *slot = b;
        self.pos += 1;
        Ok(())
    }

    /// Writes one 7-bit byte (REQ-CI-GEN-001).
    pub fn u7(&mut self, v: u8) -> Result<(), WriteError> {
        if v > 0x7F {
            return Err(WriteError::InvalidValue);
        }
        self.raw(v)
    }

    /// Writes a 14-bit value, LSB first.
    pub fn u14(&mut self, v: u16) -> Result<(), WriteError> {
        if v > 0x3FFF {
            return Err(WriteError::InvalidValue);
        }
        self.u7((v & 0x7F) as u8)?;
        self.u7((v >> 7) as u8)
    }

    /// Writes a 28-bit value, LSB first.
    pub fn u28(&mut self, v: u32) -> Result<(), WriteError> {
        if v > 0x0FFF_FFFF {
            return Err(WriteError::InvalidValue);
        }
        for i in 0..4 {
            self.u7(((v >> (7 * i)) & 0x7F) as u8)?;
        }
        Ok(())
    }

    /// Copies 7-bit bytes.
    pub fn bytes(&mut self, data: &[u8]) -> Result<(), WriteError> {
        if data.iter().any(|b| *b > 0x7F) {
            return Err(WriteError::InvalidValue);
        }
        let end = self
            .pos
            .checked_add(data.len())
            .ok_or(WriteError::Overflow)?;
        self.buf
            .get_mut(self.pos..end)
            .ok_or(WriteError::Overflow)?
            .copy_from_slice(data);
        self.pos = end;
        Ok(())
    }

    /// Bytes written so far, framing start included.
    pub fn len(&self) -> usize {
        self.pos
    }

    /// Ends the message (SysEx framing writes `0xF7`) and gives the length.
    pub fn finish(mut self) -> Result<usize, WriteError> {
        if self.framing == Framing::SysEx {
            self.raw(0xF7)?;
        }
        Ok(self.pos)
    }
}

/// An error when `body` has fewer than `n` bytes.
pub(crate) fn need(body: &[u8], n: usize) -> Result<(), ParseError> {
    if body.len() < n {
        Err(ParseError::TooShort)
    } else {
        Ok(())
    }
}

/// A 14-bit value from two 7-bit bytes, LSB first.
pub(crate) fn u14(b: &[u8]) -> u16 {
    (b[0] as u16) | ((b[1] as u16) << 7)
}

/// A 28-bit value from four 7-bit bytes, LSB first.
pub(crate) fn u28(b: &[u8]) -> u32 {
    (b[0] as u32) | ((b[1] as u32) << 7) | ((b[2] as u32) << 14) | ((b[3] as u32) << 21)
}

/// Starts a message of this crate: framing and header with Version::CURRENT (REQ-CI-MSG-007).
pub(crate) fn start<'b>(
    device_id: DeviceId,
    sub_id2: SubId2,
    source: Muid,
    destination: Muid,
    framing: Framing,
    out: &'b mut [u8],
) -> Emit<'b> {
    let mut w = Emit {
        buf: out,
        pos: 0,
        framing,
        error: None,
    };
    if framing == Framing::SysEx {
        w.raw(0xF0);
    }
    let (s, d) = (le28(source.value()), le28(destination.value()));
    w.bytes(&[
        0x7E,
        device_id.to_byte(),
        0x0D,
        sub_id2.to_byte(),
        Version::CURRENT.to_byte(),
        s[0],
        s[1],
        s[2],
        s[3],
        d[0],
        d[1],
        d[2],
        d[3],
    ]);
    w
}

/// `v` as two 7-bit bytes, LSB first. A value over 0x3FFF gives a last byte over 0x7F, so the 7-bit
/// check of [`Emit::bytes`] gives [`WriteError::InvalidValue`] for it.
pub(crate) const fn le14(v: u16) -> [u8; 2] {
    [
        (v & 0x7F) as u8,
        if v >> 7 > 0xFF { 0xFF } else { (v >> 7) as u8 },
    ]
}

/// `v` as four 7-bit bytes, LSB first. A value over 0x0FFF_FFFF gives a last byte over 0x7F
/// (see [`le14`]).
pub(crate) const fn le28(v: u32) -> [u8; 4] {
    [
        (v & 0x7F) as u8,
        ((v >> 7) & 0x7F) as u8,
        ((v >> 14) & 0x7F) as u8,
        if v >> 21 > 0xFF {
            0xFF
        } else {
            (v >> 21) as u8
        },
    ]
}

/// The writer of the messages of this crate (OPT-049, step 4). It keeps the first
/// error and gives it at [`Emit::finish`], so a message writer has no `Result` for each field; the
/// bytes and the errors are those of [`Writer`]: [`WriteError::InvalidValue`] for a value out of
/// range, [`WriteError::Overflow`] when `out` is too small (the first of the two that happens).
#[derive(Debug)]
pub(crate) struct Emit<'b> {
    buf: &'b mut [u8],
    pos: usize,
    framing: Framing,
    error: Option<WriteError>,
}

impl Emit<'_> {
    /// Keeps `e` when it is the first error.
    pub(crate) fn fail(&mut self, e: WriteError) {
        self.error.get_or_insert(e);
    }

    fn raw(&mut self, b: u8) {
        match self.buf.get_mut(self.pos) {
            Some(slot) => *slot = b,
            None => self.fail(WriteError::Overflow),
        }
        self.pos += 1;
    }

    /// One 7-bit byte.
    pub(crate) fn u7(&mut self, v: u8) {
        self.bytes(&[v]);
    }

    /// 14 bits, LSB first.
    #[inline(never)]
    pub(crate) fn u14(&mut self, v: u16) {
        self.bytes(&le14(v));
    }

    /// 28 bits, LSB first.
    pub(crate) fn u28(&mut self, v: u32) {
        self.bytes(&le28(v));
    }

    /// 7-bit bytes.
    pub(crate) fn bytes(&mut self, data: &[u8]) {
        if high_bit(data) {
            self.fail(WriteError::InvalidValue);
        }
        let end = self.pos.saturating_add(data.len());
        match self.buf.get_mut(self.pos..end) {
            Some(dst) => dst.copy_from_slice(data),
            None => self.fail(WriteError::Overflow),
        }
        self.pos = end;
    }

    /// Bytes written so far (also past the end of `out` after an overflow).
    pub(crate) fn len(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    /// Writes `v` as 14 bits at `at`, a place written before.
    pub(crate) fn patch_u14(&mut self, at: usize, v: u16) {
        if v > 0x3FFF {
            self.fail(WriteError::InvalidValue);
        }
        if let Some([lo, hi]) = self
            .buf
            .get_mut(at..at + 2)
            .and_then(|s| <&mut [u8; 2]>::try_from(s).ok())
        {
            *lo = (v & 0x7F) as u8;
            *hi = (v >> 7) as u8 & 0x7F;
        }
    }

    /// Ends the message (`0xF7` with SysEx framing) and gives its length or the first error.
    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    pub(crate) fn finish(mut self) -> Result<usize, WriteError> {
        if self.framing == Framing::SysEx {
            self.raw(0xF7);
        }
        match self.error {
            Some(e) => Err(e),
            None => Ok(self.pos),
        }
    }
}

/// Writes `text` as Message Text (REQ-CI-TEXT-001, REQ-CI-TXT-001, -003, -005).
///
/// At most `limit` bytes are written; the text stops at a character boundary. Gives
/// the number of bytes written and `true` when the text was truncated.
pub fn escape_text(
    text: &str,
    limit: usize,
    w: &mut Writer<'_>,
) -> Result<(usize, bool), WriteError> {
    let mut result = Ok(());
    let r = escape_each(text, limit, |b| {
        if result.is_ok() {
            result = w.bytes(b);
        }
    });
    result.map(|()| r)
}

/// The escape of [`escape_text`]: gives the escaped bytes of each character to `f`. Gives the
/// number of bytes and `true` when the text was truncated at `limit`.
pub(crate) fn escape_each(text: &str, limit: usize, mut f: impl FnMut(&[u8])) -> (usize, bool) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut written = 0;
    for ch in text.chars() {
        let mut tmp = [0u8; 12];
        let n = match ch {
            '\\' => {
                tmp[..2].copy_from_slice(b"\\\\");
                2
            }
            '\n' | ' '..='~' => {
                tmp[0] = ch as u8;
                1
            }
            c if (c as u32) < 0x20 || c == '\u{7F}' => 0,
            c => {
                let mut units = [0u16; 2];
                let mut n = 0;
                for u in c.encode_utf16(&mut units).iter() {
                    tmp[n..n + 2].copy_from_slice(b"\\u");
                    for k in 0..4 {
                        tmp[n + 2 + k] = HEX[((u >> (12 - 4 * k)) & 0xF) as usize];
                    }
                    n += 6;
                }
                n
            }
        };
        if written + n > limit {
            return (written, true);
        }
        f(&tmp[..n]);
        written += n;
    }
    (written, false)
}

/// Iterator over received Message Text that gives characters (REQ-CI-TXT-001, -002,
/// -004, -005). A bad escape gives U+FFFD.
#[derive(Debug, Clone)]
pub struct TextChars<'a> {
    bytes: &'a [u8],
}

impl<'a> TextChars<'a> {
    /// Iterates over raw Message Text bytes.
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        TextChars { bytes }
    }

    fn hex4(&self, at: usize) -> Option<u16> {
        let s = self.bytes.get(at..at + 4)?;
        let mut v = 0u16;
        for b in s {
            v = (v << 4) | (*b as char).to_digit(16)? as u16;
        }
        Some(v)
    }
}

impl Iterator for TextChars<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        loop {
            let (&b, rest) = self.bytes.split_first()?;
            if b != b'\\' {
                self.bytes = rest;
                if b < 0x20 && b != 0x0A {
                    continue;
                }
                return Some(if b < 0x80 {
                    b as char
                } else {
                    char::REPLACEMENT_CHARACTER
                });
            }
            let Some(&e) = rest.first() else {
                self.bytes = rest;
                return Some(char::REPLACEMENT_CHARACTER);
            };
            let simple = match e {
                b'"' => Some('"'),
                b'\\' => Some('\\'),
                b'/' => Some('/'),
                b'b' => Some('\u{8}'),
                b'f' => Some('\u{C}'),
                b'n' => Some('\n'),
                b'r' => Some('\r'),
                b't' => Some('\t'),
                _ => None,
            };
            if let Some(c) = simple {
                self.bytes = &self.bytes[2..];
                if (c as u32) < 0x20 && c != '\n' {
                    continue;
                }
                return Some(c);
            }
            if e != b'u' {
                self.bytes = &self.bytes[2..];
                return Some(char::REPLACEMENT_CHARACTER);
            }
            let Some(hi) = self.hex4(2) else {
                self.bytes = &self.bytes[2..];
                return Some(char::REPLACEMENT_CHARACTER);
            };
            if (0xD800..0xDC00).contains(&hi) {
                if self.bytes.get(6..8) == Some(b"\\u") {
                    if let Some(lo) = self.hex4(8).filter(|lo| (0xDC00..0xE000).contains(lo)) {
                        self.bytes = &self.bytes[12..];
                        let c = 0x10000 + (((hi as u32) - 0xD800) << 10) + ((lo as u32) - 0xDC00);
                        return Some(char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER));
                    }
                }
                self.bytes = &self.bytes[6..];
                return Some(char::REPLACEMENT_CHARACTER);
            }
            self.bytes = &self.bytes[6..];
            let c = char::from_u32(hi as u32).unwrap_or(char::REPLACEMENT_CHARACTER);
            if (c as u32) < 0x20 && c != '\n' {
                continue;
            }
            return Some(c);
        }
    }
}

/// True when a byte of `data` is over 0x7F; 4 bytes at a time with no early exit (a PE Chunk has
/// hundreds of bytes).
pub(crate) fn high_bit(data: &[u8]) -> bool {
    let (words, rest) = data.as_chunks::<4>();
    let high = words.iter().fold(0u32, |a, w| a | u32::from_ne_bytes(*w))
        | rest.iter().fold(0u32, |a, b| a | u32::from(*b));
    high & 0x8080_8080 != 0
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::string::String;
    use std::vec::Vec;

    /// The word check of `Header::parse` and `Emit::bytes` finds a byte over 0x7F at each place
    /// and in each length, and accepts 7-bit data.
    #[test]
    fn prop_high_bit_matches_each_byte_check() {
        proptest!(|(data in proptest::collection::vec(0u8..0x80, 0..40), at in any::<usize>(), b in 0x80u8..=0xFF)| {
            prop_assert!(!high_bit(&data));
            let mut bad = data.clone();
            if !bad.is_empty() {
                let i = at % bad.len();
                bad[i] = b;
                prop_assert!(high_bit(&bad));
            }
        });
    }

    #[test]
    fn muid_bytes_lsb_first() {
        // M2-101 3.2: 28-bit value, four 7-bit bytes, LSB first.
        let m = Muid::new(0x0123_4567).unwrap();
        assert_eq!(m.to_bytes(), [0x67, 0x0A, 0x0D, 0x09]);
        assert_eq!(Muid::from_bytes(m.to_bytes()), Ok(m));
        assert_eq!(Muid::BROADCAST.to_bytes(), [0x7F; 4]);
        // REQ-CI-MUID-007: [0x7F; 4] reads as the Broadcast MUID.
        let b = Muid::from_bytes([0x7F; 4]).unwrap();
        assert_eq!(
            (b, b.value(), b.is_broadcast()),
            (Muid::BROADCAST, 0x0FFF_FFFF, true)
        );
        assert!(!Muid::new(0x0FFF_FFFE).unwrap().is_broadcast());
        assert_eq!(Muid::new(0x1000_0000), Err(WriteError::InvalidValue));
        assert_eq!(
            Muid::from_bytes([0x80, 0, 0, 0]),
            Err(ParseError::InvalidByte)
        );
    }

    #[test]
    fn muid_from_random_avoids_reserved_and_old() {
        // REQ-CI-MUID-006, REQ-CI-MUID-011.
        assert_eq!(Muid::from_random(0x0FFF_FF00, None), None);
        assert_eq!(Muid::from_random(0x0FFF_FF80, None), None);
        assert_eq!(Muid::from_random(0x0FFF_FFFE, None), None);
        assert_eq!(Muid::from_random(0x0FFF_FFFF, None), None);
        assert_eq!(Muid::from_random(0x1FFF_FFFE, None), None);
        assert_eq!(Muid::from_random(0xFFFF_FFFF, None), None);
        assert_eq!(
            Muid::from_random(0x0FFF_FEFF, None),
            Some(Muid(0x0FFF_FEFF))
        );
        assert_eq!(Muid::from_random(0x1000_0005, None), Some(Muid(5)));
        assert_eq!(Muid::from_random(5, Some(Muid(5))), None);
    }

    #[test]
    fn prop_muid_round_trip() {
        proptest!(|(v in 0u32..=0x0FFF_FFFF)| {
            let m = Muid::new(v).unwrap();
            prop_assert_eq!(Muid::from_bytes(m.to_bytes()), Ok(m));
        });
    }

    #[test]
    fn device_id_values() {
        // Table 5; REQ-CI-MSG-005.
        assert_eq!(DeviceId::from_byte(0x05), Ok(DeviceId::Channel(5)));
        // REQ-CI-ADDR-001: 0x00-0x0F Channels 1-16, 0x7E Group, 0x7F Function Block, both ways.
        assert_eq!(DeviceId::from_byte(0x00), Ok(DeviceId::Channel(0)));
        assert_eq!(DeviceId::from_byte(0x0F), Ok(DeviceId::Channel(15)));
        assert_eq!(DeviceId::from_byte(0x80), Err(ParseError::InvalidByte));
        for (id, b) in [
            (DeviceId::Channel(0), 0x00),
            (DeviceId::Channel(9), 0x09),
            (DeviceId::Channel(15), 0x0F),
            (DeviceId::Group, 0x7E),
            (DeviceId::FunctionBlock, 0x7F),
        ] {
            assert_eq!(id.to_byte(), b);
            assert_eq!(DeviceId::from_byte(b), Ok(id));
        }
        assert_eq!(DeviceId::from_byte(0x7E), Ok(DeviceId::Group));
        assert_eq!(DeviceId::from_byte(0x7F), Ok(DeviceId::FunctionBlock));
        for b in 0x10..=0x7D {
            assert_eq!(DeviceId::from_byte(b), Err(ParseError::ReservedDeviceId));
        }
    }

    #[test]
    fn sub_id2_map() {
        // Appendix E; REQ-CI-SUBID-001.
        for b in 0u8..=0x7F {
            let reserved = matches!(b,
                0x00..=0x1F | 0x2A..=0x2E | 0x32..=0x33 | 0x3A..=0x3E | 0x45..=0x6F | 0x74..=0x7C);
            match SubId2::from_byte(b) {
                Some(s) => {
                    assert!(!reserved, "{b:#x}");
                    assert_eq!(s.to_byte(), b);
                }
                None => assert!(reserved, "{b:#x}"),
            }
        }
        assert_eq!(SubId2::Discovery.category(), Category::Management);
        assert_eq!(SubId2::Notify.category(), Category::PropertyExchange);
        // Table 4; REQ-CI-MSG-001: the high nibble of each Sub-ID#2.
        for (b, c) in [
            (0x05, Category::Reserved(0)),
            (0x10, Category::ProtocolNegotiation),
            (0x1F, Category::ProtocolNegotiation),
            (0x20, Category::ProfileConfiguration),
            (0x2F, Category::ProfileConfiguration),
            (0x30, Category::PropertyExchange),
            (0x3F, Category::PropertyExchange),
            (0x40, Category::ProcessInquiry),
            (0x44, Category::ProcessInquiry),
            (0x55, Category::Reserved(5)),
            (0x60, Category::Reserved(6)),
            (0x70, Category::Management),
            (0x7F, Category::Management),
        ] {
            assert_eq!(Category::from_sub_id2(b), c, "{b:#04x}");
        }
        assert_eq!(
            SubId2::ProfileInquiry.category(),
            Category::ProfileConfiguration
        );
        assert_eq!(
            SubId2::MidiMessageReport.category(),
            Category::ProcessInquiry
        );
    }

    #[test]
    fn version_rules() {
        // DEC-011 D-8, D-9; REQ-CI-MSG-009, REQ-CI-MSG-014.
        assert_eq!(Version::from_byte(0).unwrap().minor(), 1);
        assert_eq!(Version::from_byte(0x22).unwrap().minor(), 2);
        assert!(Version::from_byte(0x22).unwrap().has_reserved_bits());
        assert!(!Version::CURRENT.has_reserved_bits());
        assert_eq!(Version::CURRENT.to_byte(), 0x02);
        // REQ-CI-MSG-009: bit 6 and bit 5 are reserved; bits 4-0 are the minor version.
        let v = |b| Version::from_byte(b).unwrap();
        assert!(v(0x42).has_reserved_bits());
        assert_eq!(v(0x42).minor(), 0x02);
        assert!(v(0x60).has_reserved_bits());
        assert_eq!(v(0x60).minor(), 0x00);
        assert!(!v(0x1F).has_reserved_bits());
        assert_eq!(v(0x1F).minor(), 0x1F);
        assert_eq!(v(0x12).minor(), 0x12);
        assert_eq!(v(0x01).minor(), 0x01);
        assert_eq!(Version::from_byte(0x80), Err(ParseError::InvalidByte));
    }

    const DISCOVERY_HEADER: [u8; 13] = [
        0x7E, 0x7F, 0x0D, 0x70, 0x02, 0x01, 0x02, 0x03, 0x04, 0x7F, 0x7F, 0x7F, 0x7F,
    ];

    #[test]
    fn header_parse_both_framings() {
        // REQ-CI-MSG-002, REQ-CI-MSG-003, REQ-CI-MSG-004.
        let (h, body) = Header::parse(&DISCOVERY_HEADER).unwrap();
        assert_eq!(h.device_id, DeviceId::FunctionBlock);
        assert_eq!(h.sub_id2, 0x70);
        // REQ-CI-MSG-012: the version is read with the header, before the message data.
        assert_eq!(h.version, Version::CURRENT);
        assert_eq!(h.source.value(), 0x01 | 0x02 << 7 | 0x03 << 14 | 0x04 << 21);
        assert!(h.destination.is_broadcast());
        assert!(body.is_empty());
        let mut framed = [0u8; 16];
        framed[0] = 0xF0;
        framed[1..14].copy_from_slice(&DISCOVERY_HEADER);
        framed[14] = 0x55;
        framed[15] = 0xF7;
        let (h2, body2) = Header::parse(&framed).unwrap();
        assert_eq!(h2, h);
        assert_eq!(body2, &[0x55]);
    }

    #[test]
    fn header_parse_rejects() {
        assert_eq!(
            Header::parse(&[0x7E, 0x7F, 0x0C, 0x70]),
            Err(ParseError::NotMidiCi)
        );
        // REQ-CI-MSG-004: only a message that starts with 0x7E and has Sub-ID#1 0x0D.
        let mut m = DISCOVERY_HEADER;
        m[0] = 0x7D;
        assert_eq!(Header::parse(&m), Err(ParseError::NotMidiCi));
        m[0] = 0x7F;
        assert_eq!(Header::parse(&m), Err(ParseError::NotMidiCi));
        let mut framed = [0u8; 15];
        framed[0] = 0xF0;
        framed[1..14].copy_from_slice(&m);
        framed[14] = 0xF7;
        assert_eq!(Header::parse(&framed), Err(ParseError::NotMidiCi));
        m = DISCOVERY_HEADER;
        m[2] = 0x0E;
        assert_eq!(Header::parse(&m), Err(ParseError::NotMidiCi));
        assert_eq!(
            Header::parse(&DISCOVERY_HEADER[..12]),
            Err(ParseError::TooShort)
        );
        let mut m = DISCOVERY_HEADER;
        m[1] = 0x20;
        assert_eq!(Header::parse(&m), Err(ParseError::ReservedDeviceId));
        m = DISCOVERY_HEADER;
        m[6] = 0x80;
        assert_eq!(Header::parse(&m), Err(ParseError::InvalidByte));
    }

    #[test]
    fn prop_header_parse_never_panics() {
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..40))| {
            let _ = Header::parse(&bytes);
        });
    }

    #[test]
    fn header_write_round_trip() {
        // REQ-CI-MSG-004: 0x7E and 0x0D in each message; REQ-CI-MSG-007: version 0x02 also for
        // a header read with a lower version.
        let (mut h, _) = Header::parse(&DISCOVERY_HEADER).unwrap();
        h.version = Version::from_byte(0x01).unwrap();
        let mut buf = [0u8; 15];
        let n = h.write(Framing::SysEx, &mut buf).unwrap().finish().unwrap();
        assert_eq!(n, 15);
        assert_eq!(buf[0], 0xF0);
        assert_eq!(&buf[1..14], &DISCOVERY_HEADER);
        assert_eq!(buf[14], 0xF7);
        // REQ-CI-MSG-003: in UMP, no 0xF0 and no 0xF7.
        let mut bare = [0u8; 13];
        let n = h.write(Framing::Bare, &mut bare).unwrap().finish().unwrap();
        assert_eq!((n, bare), (13, DISCOVERY_HEADER));
        let mut w = start(
            DeviceId::Group,
            SubId2::Ack,
            h.source,
            h.destination,
            Framing::Bare,
            &mut buf,
        );
        w.u7(0x55);
        assert_eq!(w.finish(), Ok(14));
        assert_eq!(
            &buf[..14],
            &[0x7E, 0x7E, 0x0D, 0x7D, 0x02, 0x01, 0x02, 0x03, 0x04, 0x7F, 0x7F, 0x7F, 0x7F, 0x55]
        );
        let mut small = [0u8; 12];
        assert_eq!(
            h.write(Framing::Bare, &mut small).err(),
            Some(WriteError::Overflow)
        );
    }

    #[test]
    fn values_at_their_limits() {
        // 7-bit values: 0x7F is the largest.
        assert!(Version::from_byte(0x7F).is_ok());
        assert_eq!(Version::from_byte(0x80), Err(ParseError::InvalidByte));
        let mut buf = [0u8; 8];
        let mut w = Writer::new(Framing::SysEx, &mut buf).unwrap();
        assert_eq!(w.len(), 1, "the framing start");
        w.bytes(&[0x7F, 0x00]).unwrap();
        assert_eq!(w.len(), 3);
        // A MIDI-CI start with no more bytes: too short, not "not MIDI-CI".
        assert_eq!(
            Header::parse(&[0x7E, 0x7F, 0x0D, 0x70]),
            Err(ParseError::TooShort)
        );
        // 28 bits from four 7-bit bytes, LSB first.
        assert_eq!(u28(&[1, 2, 3, 4]), 1 | 2 << 7 | 3 << 14 | 4 << 21);
        // A value over the range gives a last byte over 0x7F, which the 7-bit check refuses.
        assert!(le14(0x8000)[1] > 0x7F);
        assert!(le28(0x2000_0000)[3] > 0x7F);
        // patch_u14 takes 0x3FFF.
        let mut buf = [0u8; 32];
        let mut e = start(
            DeviceId::FunctionBlock,
            SubId2::Ack,
            Muid::new(1).unwrap(),
            Muid::BROADCAST,
            Framing::Bare,
            &mut buf,
        );
        let at = e.len();
        e.u14(0);
        e.patch_u14(at, 0x3FFF);
        assert_eq!(e.finish(), Ok(15));
        assert_eq!(buf[13..15], [0x7F, 0x7F]);
        // Text: a space is written, and read back from "\u0020"; "\u000A" is a new line and
        // "\u0001" is dropped.
        assert_eq!(escape("a b", 100).0, "a b");
        let s: String = TextChars::new(b"a\\u0020b\\u000Ac\\u0001d").collect();
        assert_eq!(s, "a b\ncd");
        // A byte over 0x7F is not a character of the text.
        let s: String = TextChars::new(&[b'a', 0x80]).collect();
        assert_eq!(s, "a\u{FFFD}");
    }

    #[test]
    fn writer_rejects_bit7() {
        // REQ-CI-GEN-001.
        let mut buf = [0u8; 8];
        let mut w = Writer::new(Framing::Bare, &mut buf).unwrap();
        assert_eq!(w.u7(0x80), Err(WriteError::InvalidValue));
        assert_eq!(w.bytes(&[0x01, 0xFF]), Err(WriteError::InvalidValue));
        w.u14(0x3FFF).unwrap();
        assert_eq!(w.u14(0x4000), Err(WriteError::InvalidValue));
        assert_eq!(w.finish(), Ok(2));
    }

    fn escape(s: &str, limit: usize) -> (String, bool) {
        let mut buf = [0u8; 256];
        let mut w = Writer::new(Framing::Bare, &mut buf).unwrap();
        let (n, t) = escape_text(s, limit, &mut w).unwrap();
        (String::from(core::str::from_utf8(&buf[..n]).unwrap()), t)
    }

    #[test]
    fn text_escape_writer() {
        // REQ-CI-TEXT-001, REQ-CI-TXT-001, -003, -005.
        assert_eq!(escape("a\\b/c\n", 100).0, "a\\\\b/c\n");
        assert_eq!(escape("é", 100).0, "\\u00E9");
        assert_eq!(escape("𝄞", 100).0, "\\uD834\\uDD1E");
        assert_eq!(escape("a\tb", 100).0, "ab");
        assert_eq!(escape("abé", 7), (String::from("ab"), true));
    }

    #[test]
    fn text_chars_reader() {
        // REQ-CI-TXT-001, -002, -004, -005.
        let s: String = TextChars::new(b"a\\/b/c\\\\\\u00e9\\u00E9\\uD834\\uDD1E\x01\n").collect();
        assert_eq!(s, "a/b/c\\éé𝄞\n");
        // ECMA-404 escapes: \" and \n are kept; \b, \f, \r and \t are control characters,
        // which the reader drops (REQ-CI-TXT-004).
        let esc: String = TextChars::new(br#"\"q\" a\nb\tc\rd\be\ff"#).collect();
        assert_eq!(esc, "\"q\" a\nbcdef");
        let esc: Vec<char> = TextChars::new(br#"\b\f\r\t\"\\"#).collect();
        assert_eq!(esc, ['"', '\\']);
        let bad: String = TextChars::new(b"\\q\\u12\\uD800x\\").collect();
        assert_eq!(bad, "\u{FFFD}\u{FFFD}12\u{FFFD}x\u{FFFD}");
    }

    #[test]
    fn prop_text_round_trip() {
        proptest!(|(s in "\\PC{0,20}")| {
            let mut buf = [0u8; 512];
            let mut w = Writer::new(Framing::Bare, &mut buf).unwrap();
            let (n, t) = escape_text(&s, 512, &mut w).unwrap();
            prop_assert!(!t);
            let back: String = TextChars::new(&buf[..n]).collect();
            let expected: String =
                s.chars().filter(|c| *c == '\n' || !((*c as u32) < 0x20 || *c == '\u{7F}')).collect();
            prop_assert_eq!(back, expected);
        });
    }

    #[test]
    fn prop_text_chars_never_panics() {
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..40))| {
            let _ = TextChars::new(&bytes).count();
        });
    }
}
