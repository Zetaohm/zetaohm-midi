//! MIDI-CI Property Exchange framing (M2-101-UM v1.2.1 section 8, Tables 33 and 34;
//! M2-103-UM v1.2 chunking rules, DEC-006, DEC-011 D-15).
//!
//! - Capabilities: Inquiry 0x30 and Reply 0x31 (8.3): Number of Simultaneous Requests and,
//!   from Message Version 2, the PE Major and Minor Version.
//! - Data messages 0x34-0x39 and 0x3F share one framing after the Destination MUID: Request ID
//!   (1 byte), Length of Following Header Data (2 bytes LSB first), Header Data, Number of
//!   Chunks in Message (2), Number of This Chunk (2), Length of Following Property Data (2),
//!   Property Data.
//! - Header Data is only in the first Chunk. Number of Chunks 0x0000 means "unknown"; Number of
//!   This Chunk 0x0000 marks a final Chunk whose data is not known to be usable (Table 29).
//! - The crate never joins Property Data (DEC-007): each Chunk is read or written on its own.
//!   Header Data meaning (JSON, M2-103) is in `zmidi-ci-pe`.

use crate::message::{
    need, start, u14, DeviceId, Framing, Header, Muid, ParseError, SubId2, WriteError,
};

/// Property Exchange Capabilities data (Tables 31, 32; REQ-CI-PEF-004 to REQ-CI-PEF-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeCapabilities {
    /// Number of Simultaneous Property Exchange Requests Supported.
    pub simultaneous: u8,
    /// Property Exchange Major Version; `None` in a version 1 message (written 0x00).
    pub major: Option<u8>,
    /// Property Exchange Minor Version; `None` in a version 1 message (written 0x00).
    pub minor: Option<u8>,
}

impl PeCapabilities {
    /// Reads the body of 0x30 or 0x31; every version value is accepted.
    pub fn read(header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        need(body, 1)?;
        let (major, minor) = if header.version.minor() >= 2 {
            need(body, 3)?;
            (Some(body[1]), Some(body[2]))
        } else {
            (None, None)
        };
        Ok(PeCapabilities {
            simultaneous: body[0],
            major,
            minor,
        })
    }

    fn write(
        &self,
        sub: SubId2,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let mut w = start(
            DeviceId::FunctionBlock,
            sub,
            source,
            destination,
            framing,
            out,
        );
        w.u7(self.simultaneous);
        w.u7(self.major.unwrap_or(0));
        w.u7(self.minor.unwrap_or(0));
        w.finish()
    }

    /// Writes Inquiry: Property Exchange Capabilities (0x30).
    pub fn write_inquiry(
        &self,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        self.write(SubId2::PeCapabilities, source, destination, framing, out)
    }

    /// Writes Reply to Property Exchange Capabilities (0x31).
    pub fn write_reply(
        &self,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        self.write(
            SubId2::PeCapabilitiesReply,
            source,
            destination,
            framing,
            out,
        )
    }
}

/// Property Exchange data messages (REQ-CI-PEF-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PeKind {
    /// 0x34 Inquiry: Get Property Data.
    GetInquiry,
    /// 0x35 Reply to Get Property Data.
    GetReply,
    /// 0x36 Inquiry: Set Property Data.
    SetInquiry,
    /// 0x37 Reply to Set Property Data.
    SetReply,
    /// 0x38 Subscription.
    Subscription,
    /// 0x39 Reply to Subscription.
    SubscriptionReply,
    /// 0x3F Notify.
    Notify,
}

impl PeKind {
    /// The kind of a Sub-ID#2; `None` for any other value, 0x30 and 0x31 included.
    pub const fn from_sub_id2(b: u8) -> Option<Self> {
        Some(match b {
            0x34 => PeKind::GetInquiry,
            0x35 => PeKind::GetReply,
            0x36 => PeKind::SetInquiry,
            0x37 => PeKind::SetReply,
            0x38 => PeKind::Subscription,
            0x39 => PeKind::SubscriptionReply,
            0x3F => PeKind::Notify,
            _ => return None,
        })
    }

    /// The Sub-ID#2 byte.
    pub const fn sub_id2(self) -> u8 {
        self.sub().to_byte()
    }

    const fn sub(self) -> SubId2 {
        match self {
            PeKind::GetInquiry => SubId2::GetPropertyData,
            PeKind::GetReply => SubId2::GetPropertyDataReply,
            PeKind::SetInquiry => SubId2::SetPropertyData,
            PeKind::SetReply => SubId2::SetPropertyDataReply,
            PeKind::Subscription => SubId2::Subscription,
            PeKind::SubscriptionReply => SubId2::SubscriptionReply,
            PeKind::Notify => SubId2::Notify,
        }
    }
}

/// End state of a Chunk (Table 29, REQ-CI-PEF-025).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkEnd {
    /// More Chunks follow.
    More,
    /// Final Chunk; the Property Data is complete.
    Complete,
    /// Final Chunk; the Property Data is not known to be complete or usable.
    Incomplete,
}

/// One Chunk of a Property Exchange data message (Table 33, REQ-CI-PEF-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeChunk<'a> {
    /// Request ID, 0-127.
    pub request_id: u8,
    /// Header Data (first Chunk only).
    pub header: &'a [u8],
    /// Number of Chunks in Message; 0 when unknown.
    pub total: u16,
    /// Number of This Chunk; 0 marks an incomplete final Chunk.
    pub number: u16,
    /// Property Data of this Chunk.
    pub data: &'a [u8],
}

/// Framing bytes after the 13-byte header, without Header Data or Property Data.
const CHUNK_FIXED: usize = 1 + 2 + 2 + 2 + 2;

impl<'a> PeChunk<'a> {
    /// Reads the framing of one Chunk (REQ-CI-PEF-001, REQ-CI-PEF-024).
    pub fn read(_header: &Header, body: &'a [u8]) -> Result<Self, ParseError> {
        need(body, 3)?;
        let hlen = u14(&body[1..3]) as usize;
        need(body, CHUNK_FIXED + hlen)?;
        let rest = &body[3 + hlen..];
        let total = u14(&rest[0..2]);
        let number = u14(&rest[2..4]);
        let dlen = u14(&rest[4..6]) as usize;
        need(rest, 6 + dlen)?;
        if number >= 2 && hlen != 0 {
            return Err(ParseError::InvalidValue);
        }
        Ok(PeChunk {
            request_id: body[0],
            header: &body[3..3 + hlen],
            total,
            number,
            data: &rest[6..6 + dlen],
        })
    }

    /// Writes one Chunk with Device ID 0x7F; Header Data only when `number` is 1. An Inquiry: Get
    /// Property Data or a Reply to Set Property Data other than Chunk 1 of 1 with no Property Data
    /// is an InvalidValue error.
    pub fn write(
        &self,
        kind: PeKind,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        // Header Data only in Chunk 1, or in a first Chunk that is also an incomplete end.
        if self.request_id > 0x7F || (self.number >= 2 && !self.header.is_empty()) {
            return Err(WriteError::InvalidValue);
        }
        // REQ-CI-PEF-007, -010 (Tables 33, 36): Inquiry: Get Property Data and Reply to Set
        // Property Data are one Chunk (0x0001 of 0x0001) with no Property Data.
        if matches!(kind, PeKind::GetInquiry | PeKind::SetReply)
            && (self.total != 1 || self.number != 1 || !self.data.is_empty())
        {
            return Err(WriteError::InvalidValue);
        }
        let hlen = u16::try_from(self.header.len()).map_err(|_| WriteError::InvalidValue)?;
        let dlen = u16::try_from(self.data.len()).map_err(|_| WriteError::InvalidValue)?;
        let mut w = start(
            DeviceId::FunctionBlock,
            kind.sub(),
            source,
            destination,
            framing,
            out,
        );
        w.u7(self.request_id);
        w.u14(hlen);
        w.bytes(self.header);
        w.u14(self.total);
        w.u14(self.number);
        w.u14(dlen);
        w.bytes(self.data);
        w.finish()
    }

    /// The end state of this Chunk (Table 29).
    pub fn end(&self) -> ChunkEnd {
        if self.number == 0 {
            ChunkEnd::Incomplete
        } else if self.total != 0 && self.number == self.total {
            ChunkEnd::Complete
        } else {
            ChunkEnd::More
        }
    }
}

/// Writes the Chunks of one Property Exchange message (REQ-CI-PEF-015 to REQ-CI-PEF-024).
#[derive(Debug, Clone)]
pub struct ChunkWriter<'h> {
    kind: PeKind,
    request_id: u8,
    header: &'h [u8],
    limit: usize,
    overhead: usize,
    /// Number of Chunks in Message for a known total; `None` for an unknown total.
    total: Option<u16>,
    next: u16,
    source: Muid,
    destination: Muid,
    framing: Framing,
}

impl<'h> ChunkWriter<'h> {
    /// Starts a message. `max_sysex_size` is the receiver's Receivable Maximum SysEx Message
    /// Size, `chunk_size` an optional smaller limit, `total_len` the whole Property Data
    /// length or `None` when unknown.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: PeKind,
        request_id: u8,
        header: &'h [u8],
        max_sysex_size: u32,
        chunk_size: Option<usize>,
        total_len: Option<usize>,
        source: Muid,
        destination: Muid,
        framing: Framing,
    ) -> Result<Self, WriteError> {
        if request_id > 0x7F {
            return Err(WriteError::InvalidValue);
        }
        let mut limit = max_sysex_size as usize;
        if let Some(c) = chunk_size {
            limit = limit.min(c);
        }
        let overhead = Header::LEN + CHUNK_FIXED + if framing == Framing::SysEx { 2 } else { 0 };
        let first = limit
            .checked_sub(overhead + header.len())
            .ok_or(WriteError::InvalidValue)?;
        let total = match total_len {
            None => None,
            Some(0) => Some(1),
            Some(n) => {
                let rest = n.saturating_sub(first);
                let room = limit - overhead;
                if room == 0 && rest > 0 || (first == 0 && rest == n && room == 0) {
                    return Err(WriteError::InvalidValue);
                }
                let extra = if rest == 0 { 0 } else { rest.div_ceil(room) };
                Some(
                    u16::try_from(1 + extra)
                        .ok()
                        .filter(|t| *t <= 0x3FFF)
                        .ok_or(WriteError::InvalidValue)?,
                )
            }
        };
        Ok(ChunkWriter {
            kind,
            request_id,
            header,
            limit,
            overhead,
            total,
            next: 1,
            source,
            destination,
            framing,
        })
    }

    /// Property Data bytes that the next Chunk takes.
    pub fn data_room(&self) -> usize {
        let h = if self.next == 1 { self.header.len() } else { 0 };
        self.limit.saturating_sub(self.overhead + h)
    }

    fn chunk(
        &mut self,
        part: &[u8],
        total: u16,
        number: u16,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        if part.len() > self.data_room() || self.next > 0x3FFF {
            return Err(WriteError::InvalidValue);
        }
        let header = if self.next == 1 { self.header } else { &[] };
        let n = PeChunk {
            request_id: self.request_id,
            header,
            total,
            number,
            data: part,
        }
        .write(self.kind, self.source, self.destination, self.framing, out)?;
        self.next += 1;
        Ok(n)
    }

    /// Writes the next Chunk with `part` (at most [`Self::data_room`] bytes). For a known
    /// total, the Chunk with the last number ends the message.
    pub fn write_next(&mut self, part: &[u8], out: &mut [u8]) -> Result<usize, WriteError> {
        match self.total {
            Some(t) if self.next > t => Err(WriteError::InvalidValue),
            Some(t) => self.chunk(part, t, self.next, out),
            None => self.chunk(part, 0, self.next, out),
        }
    }

    /// Writes the final Chunk with `part` (it can be empty, REQ-CI-PEF-023). `Complete`
    /// writes total = this number (REQ-CI-PEF-020, -021); `Incomplete` writes this number
    /// 0x0000 and total = the sequential count (REQ-CI-PEF-022).
    pub fn finish(
        &mut self,
        part: &[u8],
        end: ChunkEnd,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let count = self.next;
        match end {
            ChunkEnd::Complete => self.chunk(part, count, count, out),
            ChunkEnd::Incomplete => self.chunk(part, count, 0, out),
            ChunkEnd::More => Err(WriteError::InvalidValue),
        }
    }
}

/// Role of this Device in a Request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// This Device sent the inquiry.
    Initiator,
    /// This Device answers the inquiry.
    Responder,
}

/// Active Property Exchange Requests, capacity `N` (REQ-CI-PEF-026, -028, -030).
#[derive(Debug, Clone)]
pub struct RequestTable<const N: usize> {
    active: [Option<(Muid, u8, Direction)>; N],
}

#[allow(clippy::new_without_default)]
impl<const N: usize> RequestTable<N> {
    /// An empty table. `N` is at least 1.
    pub const fn new() -> Self {
        assert!(N >= 1 && N <= 127, "RequestTable capacity is 1 to 127");
        RequestTable { active: [None; N] }
    }

    /// Initiator: a Request ID that is not active to `peer`, marked active.
    pub fn allocate(&mut self, peer: Muid) -> Option<u8> {
        let slot = self.active.iter().position(Option::is_none)?;
        let id = (0u8..=0x7F).find(|id| !self.is_active(peer, *id, Direction::Initiator))?;
        self.active[slot] = Some((peer, id, Direction::Initiator));
        Some(id)
    }

    /// Responder: marks an inbound Request active. Overflow when full.
    pub fn open(&mut self, peer: Muid, request_id: u8) -> Result<(), WriteError> {
        if self.is_active(peer, request_id, Direction::Responder) {
            return Ok(());
        }
        let slot = self
            .active
            .iter_mut()
            .find(|s| s.is_none())
            .ok_or(WriteError::Overflow)?;
        *slot = Some((peer, request_id, Direction::Responder));
        Ok(())
    }

    /// Ends one Request.
    pub fn close(&mut self, peer: Muid, request_id: u8, direction: Direction) {
        for s in self.active.iter_mut() {
            if *s == Some((peer, request_id, direction)) {
                *s = None;
            }
        }
    }

    /// Ends every Request of `peer` (Invalidate MUID).
    pub fn close_peer(&mut self, peer: Muid) {
        for s in self.active.iter_mut() {
            if matches!(s, Some((m, _, _)) if *m == peer) {
                *s = None;
            }
        }
    }

    /// True when the Request is active.
    pub fn is_active(&self, peer: Muid, request_id: u8, direction: Direction) -> bool {
        self.active.contains(&Some((peer, request_id, direction)))
    }

    /// The capacity, reported as Number of Simultaneous Requests Supported.
    pub const fn capacity(&self) -> usize {
        N
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::message::Version;
    use proptest::prelude::*;
    use std::vec::Vec;

    const SRC: Muid = match Muid::new(0x0012_3456) {
        Ok(m) => m,
        Err(_) => panic!(),
    };
    const DST: Muid = match Muid::new(0x0065_4321) {
        Ok(m) => m,
        Err(_) => panic!(),
    };

    fn parse(buf: &[u8]) -> (Header, &[u8]) {
        Header::parse(buf).unwrap()
    }

    #[test]
    fn chunk_writer_at_its_limits() {
        // Data room: 512 - 13 header bytes - 9 Chunk bytes - the Header Data.
        let w = ChunkWriter::new(
            PeKind::GetReply,
            0x7F,
            b"{}",
            512,
            None,
            None,
            SRC,
            DST,
            Framing::Bare,
        )
        .unwrap();
        assert_eq!(w.data_room(), 512 - 13 - 9 - 2);
        // Request ID 0x7F is the largest.
        let mut out = [0u8; 64];
        let c = PeChunk {
            request_id: 0x7F,
            header: b"{}",
            total: 1,
            number: 1,
            data: &[],
        };
        assert!(c
            .write(PeKind::GetReply, SRC, DST, Framing::Bare, &mut out)
            .is_ok());
        // A known total: no Chunk after the last.
        let mut w = ChunkWriter::new(
            PeKind::GetReply,
            1,
            b"{}",
            512,
            None,
            Some(0),
            SRC,
            DST,
            Framing::Bare,
        )
        .unwrap();
        w.write_next(&[], &mut out).unwrap();
        assert_eq!(w.write_next(&[], &mut out), Err(WriteError::InvalidValue));
        // An unknown total: Chunk numbers up to 0x3FFF.
        let mut w = ChunkWriter::new(
            PeKind::GetReply,
            1,
            b"",
            512,
            None,
            None,
            SRC,
            DST,
            Framing::Bare,
        )
        .unwrap();
        for k in 1..=0x3FFF {
            assert!(w.write_next(&[], &mut out).is_ok(), "{k}");
        }
        assert_eq!(w.write_next(&[], &mut out), Err(WriteError::InvalidValue));
    }

    #[test]
    fn capabilities() {
        // Tables 31, 32; REQ-CI-PEF-004 to -006.
        let mut buf = [0u8; 32];
        let c = PeCapabilities {
            simultaneous: 4,
            major: None,
            minor: None,
        };
        let n = c.write_reply(SRC, DST, Framing::Bare, &mut buf).unwrap();
        assert_eq!(&buf[1..4], &[0x7F, 0x0D, 0x31]);
        assert_eq!(&buf[13..n], &[4, 0, 0]);
        let (h, body) = parse(&buf[..n]);
        assert_eq!(
            PeCapabilities::read(&h, body),
            Ok(PeCapabilities {
                simultaneous: 4,
                major: Some(0),
                minor: Some(0)
            })
        );
        buf[4] = 1;
        let (h, body) = parse(&buf[..n]);
        assert_eq!(PeCapabilities::read(&h, body), Ok(c));

        // Table 30: Inquiry: PE Capabilities, Sub-ID#2 0x30, the same fields (REQ-CI-PEF-004).
        let n = c.write_inquiry(SRC, DST, Framing::Bare, &mut buf).unwrap();
        assert_eq!(
            &buf[..n],
            &[
                0x7E, 0x7F, 0x0D, 0x30, 0x02, 0x56, 0x68, 0x48, 0x00, 0x21, 0x06, 0x15, 0x03, 4, 0,
                0
            ]
        );
        let (h, body) = parse(&buf[..n]);
        let back = PeCapabilities::read(&h, body).unwrap();
        assert_eq!(
            (back.simultaneous, back.major, back.minor),
            (4, Some(0), Some(0))
        );
        // REQ-CI-PEF-006: the reader accepts every version value; the writer writes the values
        // the caller gives (0x00 for None, above).
        buf[14] = 5;
        buf[15] = 7;
        let (h, body) = parse(&buf[..n]);
        assert_eq!(
            PeCapabilities::read(&h, body),
            Ok(PeCapabilities {
                simultaneous: 4,
                major: Some(5),
                minor: Some(7)
            })
        );
        let v = PeCapabilities {
            simultaneous: 1,
            major: Some(3),
            minor: Some(4),
        };
        let n = v.write_inquiry(SRC, DST, Framing::Bare, &mut buf).unwrap();
        assert_eq!(&buf[13..n], &[1, 3, 4]);
        // Version 2 needs the two version bytes.
        let (h, body) = parse(&buf[..n]);
        assert_eq!(
            PeCapabilities::read(&h, &body[..2]),
            Err(ParseError::TooShort)
        );
    }

    #[test]
    fn get_inquiry_bytes() {
        // Table 33, REQ-CI-PEF-007, REQ-CI-PEF-018.
        let mut buf = [0u8; 64];
        let header = br#"{"resource":"DeviceInfo"}"#;
        let c = PeChunk {
            request_id: 1,
            header,
            total: 1,
            number: 1,
            data: &[],
        };
        let n = c
            .write(PeKind::GetInquiry, SRC, DST, Framing::SysEx, &mut buf)
            .unwrap();
        assert_eq!(&buf[1..5], &[0x7E, 0x7F, 0x0D, 0x34]);
        assert_eq!(&buf[14..17], &[1, header.len() as u8, 0]);
        assert_eq!(&buf[17 + header.len()..n], &[1, 0, 1, 0, 0, 0, 0xF7]);
        let (h, body) = parse(&buf[..n]);
        let back = PeChunk::read(&h, body).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.end(), ChunkEnd::Complete);
    }

    #[test]
    fn every_kind_round_trip() {
        // REQ-CI-PEF-003, REQ-CI-PEF-007 to REQ-CI-PEF-013, REQ-CI-ADDR-002.
        let kinds = [
            (PeKind::GetInquiry, 0x34),
            (PeKind::GetReply, 0x35),
            (PeKind::SetInquiry, 0x36),
            (PeKind::SetReply, 0x37),
            (PeKind::Subscription, 0x38),
            (PeKind::SubscriptionReply, 0x39),
            (PeKind::Notify, 0x3F),
        ];
        let mut buf = [0u8; 64];
        for (kind, sub) in kinds {
            assert_eq!(kind.sub_id2(), sub);
            assert_eq!(PeKind::from_sub_id2(sub), Some(kind));
            // Tables 33 and 36: 0x34 and 0x37 are Chunk 1 of 1 with no Property Data.
            let fixed = matches!(kind, PeKind::GetInquiry | PeKind::SetReply);
            let c = PeChunk {
                request_id: 9,
                header: b"{}",
                total: if fixed { 1 } else { 0 },
                number: 1,
                data: if fixed { b"" } else { b"x" },
            };
            let n = c.write(kind, SRC, DST, Framing::Bare, &mut buf).unwrap();
            let mut want = std::vec![
                0x7E, 0x7F, 0x0D, sub, 0x02, 0x56, 0x68, 0x48, 0x00, 0x21, 0x06, 0x15, 0x03,
            ];
            // Request ID, header length 2, "{}".
            want.extend([9, 2, 0, b'{', b'}']);
            if fixed {
                want.extend([1, 0, 1, 0, 0, 0]);
            } else {
                want.extend([0, 0, 1, 0, 1, 0, b'x']);
            }
            assert_eq!(&buf[..n], &want[..], "{sub:#04x}");
            let (h, body) = parse(&buf[..n]);
            assert_eq!(PeChunk::read(&h, body), Ok(c));
        }
        assert_eq!(PeKind::from_sub_id2(0x30), None);
        assert_eq!(PeKind::from_sub_id2(0x3A), None);
    }

    #[test]
    fn get_inquiry_and_set_reply_are_one_empty_chunk() {
        // M2-101 Tables 33 and 36; REQ-CI-PEF-007, REQ-CI-PEF-010: Number of Chunks 01 00,
        // Number of This Chunk 01 00, Length of Following Property Data 00 00.
        let mut buf = [0u8; 64];
        let reply = PeChunk {
            request_id: 0x11,
            header: br#"{"status":200}"#,
            total: 1,
            number: 1,
            data: &[],
        };
        let n = reply
            .write(PeKind::SetReply, SRC, DST, Framing::SysEx, &mut buf)
            .unwrap();
        let mut want = std::vec![
            0xF0, 0x7E, 0x7F, 0x0D, 0x37, 0x02, 0x56, 0x68, 0x48, 0x00, 0x21, 0x06, 0x15, 0x03,
            0x11, 14, 0,
        ];
        want.extend_from_slice(br#"{"status":200}"#);
        want.extend([1, 0, 1, 0, 0, 0, 0xF7]);
        assert_eq!(&buf[..n], &want[..]);
        // Any other count or Property Data is an error, for both kinds.
        for kind in [PeKind::GetInquiry, PeKind::SetReply] {
            for (total, number, data) in [
                (0u16, 1u16, &b""[..]),
                (2, 1, b""),
                (1, 0, b""),
                (1, 1, b"x"),
            ] {
                let c = PeChunk {
                    total,
                    number,
                    data,
                    ..reply
                };
                assert_eq!(
                    c.write(kind, SRC, DST, Framing::Bare, &mut buf),
                    Err(WriteError::InvalidValue),
                    "{kind:?} {total} {number}"
                );
            }
            // Number of This Chunk 2 with no Header Data: also an error for these kinds.
            let c = PeChunk {
                header: &[],
                total: 2,
                number: 2,
                ..reply
            };
            assert_eq!(
                c.write(kind, SRC, DST, Framing::Bare, &mut buf),
                Err(WriteError::InvalidValue)
            );
            // The other kinds take the same values.
            let c = PeChunk {
                total: 0,
                data: b"x",
                ..reply
            };
            assert!(c
                .write(PeKind::SetInquiry, SRC, DST, Framing::Bare, &mut buf)
                .is_ok());
        }
        // A ChunkWriter of an unknown total gives the fixed values only with a Complete end.
        let mut w = ChunkWriter::new(
            PeKind::GetInquiry,
            2,
            b"{}",
            512,
            None,
            None,
            SRC,
            DST,
            Framing::Bare,
        )
        .unwrap();
        assert_eq!(w.write_next(&[], &mut buf), Err(WriteError::InvalidValue));
        let n = w.finish(&[], ChunkEnd::Complete, &mut buf).unwrap();
        assert_eq!(&buf[18..n], &[1, 0, 1, 0, 0, 0]);
    }

    #[test]
    fn chunk_end_table_29() {
        // REQ-CI-PEF-025.
        let c = |total, number| PeChunk {
            request_id: 0,
            header: &[],
            total,
            number,
            data: &[],
        };
        assert_eq!(c(3, 1).end(), ChunkEnd::More);
        assert_eq!(c(3, 3).end(), ChunkEnd::Complete);
        assert_eq!(c(0, 5).end(), ChunkEnd::More);
        assert_eq!(c(5, 5).end(), ChunkEnd::Complete);
        assert_eq!(c(5, 0).end(), ChunkEnd::Incomplete);
    }

    #[test]
    fn reader_rejects() {
        // REQ-CI-PEF-001, REQ-CI-PEF-024.
        let mut buf = [0u8; 64];
        let c = PeChunk {
            request_id: 2,
            header: &[],
            total: 2,
            number: 2,
            data: b"ab",
        };
        let n = c
            .write(PeKind::GetReply, SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        let (h, body) = parse(&buf[..n]);
        assert_eq!(
            PeChunk::read(&h, &body[..body.len() - 1]),
            Err(ParseError::TooShort)
        );
        // Bytes after the Property Data are ignored; both data fields are slices.
        let mut long = Vec::from(body);
        long.extend([0x11, 0x22, 0x33]);
        let back = PeChunk::read(&h, &long).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.data, b"ab");
        // A header length that the body does not hold.
        let mut short = Vec::from(body);
        short[1] = 0x7F;
        assert_eq!(PeChunk::read(&h, &short), Err(ParseError::TooShort));
        let mut bad = Vec::from(body);
        bad[1] = 1; // header length 1 in Chunk 2
        bad.insert(3, b'x');
        assert_eq!(PeChunk::read(&h, &bad), Err(ParseError::InvalidValue));
        let with_header = PeChunk { header: b"{}", ..c };
        assert_eq!(
            with_header.write(PeKind::GetReply, SRC, DST, Framing::Bare, &mut buf),
            Err(WriteError::InvalidValue)
        );
        let big_id = PeChunk {
            request_id: 0x80,
            ..c
        };
        assert_eq!(
            big_id.write(PeKind::GetReply, SRC, DST, Framing::Bare, &mut buf),
            Err(WriteError::InvalidValue)
        );
    }

    /// Writes `data` with a known total and reads the Chunks back.
    fn known(data: &[u8], header: &[u8], limit: u32) -> Vec<(u16, u16, Vec<u8>, usize)> {
        let mut w = ChunkWriter::new(
            PeKind::GetReply,
            7,
            header,
            limit,
            None,
            Some(data.len()),
            SRC,
            DST,
            Framing::Bare,
        )
        .unwrap();
        let mut out = Vec::new();
        let mut rest = data;
        let mut buf = [0u8; 600];
        loop {
            let take = w.data_room().min(rest.len());
            let n = w.write_next(&rest[..take], &mut buf).unwrap();
            assert!(n <= limit as usize);
            rest = &rest[take..];
            let (h, body) = parse(&buf[..n]);
            let c = PeChunk::read(&h, body).unwrap();
            assert_eq!(c.request_id, 7);
            out.push((c.total, c.number, c.data.to_vec(), c.header.len()));
            if c.end() == ChunkEnd::Complete {
                break;
            }
        }
        assert!(rest.is_empty());
        out
    }

    #[test]
    fn chunk_writer_known_total() {
        // REQ-CI-PEF-015, -017, -018, -024, -027.
        let data: Vec<u8> = (0..300).map(|i| (i % 128) as u8).collect();
        let chunks = known(&data, b"{\"status\":200}", 128);
        let t = chunks.len() as u16;
        assert!(t > 2);
        for (i, (total, number, _, hlen)) in chunks.iter().enumerate() {
            assert_eq!(*total, t);
            assert_eq!(*number, i as u16 + 1);
            assert_eq!(*hlen != 0, i == 0);
        }
        let joined: Vec<u8> = chunks.iter().flat_map(|c| c.2.clone()).collect();
        assert_eq!(joined, data);
        let empty = known(&[], b"{}", 128);
        assert_eq!(empty, [(1, 1, Vec::new(), 2)]);
    }

    #[test]
    fn chunk_writer_unknown_total_and_ends() {
        // REQ-CI-PEF-019 to -023.
        let mut buf = [0u8; 200];
        let mut w = ChunkWriter::new(
            PeKind::Subscription,
            3,
            b"{}",
            512,
            Some(100),
            None,
            SRC,
            DST,
            Framing::Bare,
        )
        .unwrap();
        let room = w.data_room();
        assert_eq!(room, 100 - 13 - 9 - 2);
        let n = w.write_next(&[1; 10], &mut buf).unwrap();
        let (h, body) = parse(&buf[..n]);
        let c = PeChunk::read(&h, body).unwrap();
        assert_eq!((c.total, c.number), (0, 1));
        let n = w.finish(&[], ChunkEnd::Complete, &mut buf).unwrap();
        let (h, body) = parse(&buf[..n]);
        let c = PeChunk::read(&h, body).unwrap();
        assert_eq!((c.total, c.number, c.data.len()), (2, 2, 0));
        assert_eq!(c.end(), ChunkEnd::Complete);

        let mut w = ChunkWriter::new(
            PeKind::GetReply,
            3,
            &[],
            512,
            None,
            Some(2000),
            SRC,
            DST,
            Framing::Bare,
        )
        .unwrap();
        // REQ-CI-PEF-021: a known total (5 Chunks of 2000 bytes at 512) ended early with complete
        // data gives Number of Chunks = Number of This Chunk.
        let mut early = w.clone();
        let n = early.write_next(&[0; 10], &mut buf).unwrap();
        let (h, body) = parse(&buf[..n]);
        assert_eq!(PeChunk::read(&h, body).unwrap().total, 5);
        let n = early.finish(&[0; 5], ChunkEnd::Complete, &mut buf).unwrap();
        let (h, body) = parse(&buf[..n]);
        let c = PeChunk::read(&h, body).unwrap();
        assert_eq!((c.total, c.number, c.data.len()), (2, 2, 5));
        assert_eq!(c.end(), ChunkEnd::Complete);
        let _ = w.write_next(&[0; 10], &mut buf).unwrap();
        let n = w.finish(&[0; 5], ChunkEnd::Incomplete, &mut buf).unwrap();
        let (h, body) = parse(&buf[..n]);
        let c = PeChunk::read(&h, body).unwrap();
        assert_eq!((c.total, c.number), (2, 0));
        assert_eq!(c.end(), ChunkEnd::Incomplete);
        assert_eq!(
            w.write_next(&[0; 600], &mut buf),
            Err(WriteError::InvalidValue)
        );
        assert!(ChunkWriter::new(
            PeKind::GetReply,
            3,
            &[0; 200],
            128,
            None,
            None,
            SRC,
            DST,
            Framing::Bare
        )
        .is_err());
    }

    #[test]
    fn incomplete_end_as_the_first_chunk() {
        // Code review finding 6: a first Chunk that is also an incomplete end keeps its header.
        let mut buf = [0u8; 128];
        let mut w = ChunkWriter::new(
            PeKind::GetReply,
            1,
            b"{\"status\":200}",
            512,
            None,
            None,
            SRC,
            DST,
            Framing::Bare,
        )
        .unwrap();
        let n = w.finish(b"ab", ChunkEnd::Incomplete, &mut buf).unwrap();
        let (h, body) = parse(&buf[..n]);
        let c = PeChunk::read(&h, body).unwrap();
        assert_eq!(
            (c.number, c.end(), c.header.len()),
            (0, ChunkEnd::Incomplete, 14)
        );
    }

    #[test]
    fn request_table() {
        // REQ-CI-PEF-026, -028, -030.
        let mut t = RequestTable::<3>::new();
        assert_eq!(t.capacity(), 3);
        assert_eq!(t.allocate(DST), Some(0));
        assert_eq!(t.allocate(DST), Some(1));
        assert_eq!(t.allocate(SRC), Some(0));
        assert_eq!(t.allocate(SRC), None);
        t.close(DST, 0, Direction::Initiator);
        assert_eq!(t.open(SRC, 0), Ok(()));
        assert!(t.is_active(SRC, 0, Direction::Responder));
        assert!(t.is_active(SRC, 0, Direction::Initiator));
        assert_eq!(t.open(DST, 9), Err(WriteError::Overflow));
        t.close_peer(SRC);
        assert!(!t.is_active(SRC, 0, Direction::Responder));
        assert_eq!(t.allocate(DST), Some(0));
        // REQ-CI-PEF-028: one Request ID is active with two peers at the same time.
        let mut t = RequestTable::<4>::new();
        assert_eq!(t.open(SRC, 5), Ok(()));
        assert_eq!(t.open(DST, 5), Ok(()));
        assert!(
            t.is_active(SRC, 5, Direction::Responder) && t.is_active(DST, 5, Direction::Responder)
        );
        t.close(SRC, 5, Direction::Responder);
        assert!(
            !t.is_active(SRC, 5, Direction::Responder) && t.is_active(DST, 5, Direction::Responder)
        );
    }

    #[test]
    fn prop_chunk_writer_round_trip() {
        proptest!(|(len in 0usize..2000, limit in 64u32..600, hlen in 0usize..20)| {
            let data: Vec<u8> = (0..len).map(|i| (i % 128) as u8).collect();
            let header = std::vec![b'h'; hlen];
            let chunks = known(&data, &header, limit);
            let joined: Vec<u8> = chunks.iter().flat_map(|c| c.2.clone()).collect();
            prop_assert_eq!(joined, data);
        });
    }

    #[test]
    fn prop_reader_never_panics() {
        proptest!(|(bytes in proptest::collection::vec(0u8..0x80, 0..40))| {
            let h = Header { device_id: DeviceId::FunctionBlock, sub_id2: 0x34, version: Version::CURRENT, source: SRC, destination: DST };
            let _ = PeChunk::read(&h, &bytes);
            let _ = PeCapabilities::read(&h, &bytes);
        });
    }
}
