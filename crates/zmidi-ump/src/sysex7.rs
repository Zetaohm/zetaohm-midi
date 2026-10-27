//! System Exclusive (7-bit) Messages: Message Type 0x3 (M2-104-UM 7.7, Figure 84, Table 18).
//!
//! - Format: one 64-bit packet. `mt=3` (bits 63..60), Group (59..56), Status (55..52), Number
//!   of Bytes (51..48), then six data bytes. Each data byte has a reserved most significant bit
//!   and 7 data bits (Figure 84).
//! - The payload is the MIDI 1.0 System Exclusive data without the 0xF0 and 0xF7 bytes. The
//!   Manufacturer ID (or 0x7D, or a Universal System Exclusive ID), Device ID and Sub-IDs are
//!   the first data bytes (7.7).
//! - Status (Table 18): 0x0 Complete, 0x1 Start, 0x2 Continue, 0x3 End. A message is one
//!   Complete UMP, or one Start UMP, zero or more Continue UMPs and one End UMP (7.7).
//!   Status values 0x4 to 0xF are not defined; [`SysEx7::parse`] gives `None` for them.
//! - Number of Bytes is 0 to 6. Unused bytes are reserved and set to zero (7.7). A Start or
//!   Continue with fewer than 6 bytes does not end the message (7.7 note).
//! - Within a Group, only Continue UMPs and System Real Time messages may come between a Start
//!   and its End. Any other UMP on the same Group ends the message (7.7.1).
//!   [`SysEx7Reassembler`] applies this rule.
//! - Message Type is read-only. The struct has no public `from_bits` (DEC-UMP-002).

use core::ops::Deref;

use bitfield_struct::bitfield;

use crate::packet::{join_u64, put, split_u64, take, Group, MessageType, UmpError, UmpWords};

/// Largest Number of Bytes in one packet (7.7).
const MAX_BYTES: usize = 6;

/// Status of a System Exclusive (7-bit) packet, bits 55..52 (Table 18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SysEx7Status {
    /// 0x0: Complete System Exclusive Message in one UMP.
    Complete = 0x0,
    /// 0x1: System Exclusive Start UMP.
    Start = 0x1,
    /// 0x2: System Exclusive Continue UMP.
    Continue = 0x2,
    /// 0x3: System Exclusive End UMP.
    End = 0x3,
}

impl SysEx7Status {
    /// The 4-bit status value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The status for a 4-bit `value`, or `None` for 0x4 to 0xF, which Table 18 does not
    /// define.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0x0 => Some(Self::Complete),
            0x1 => Some(Self::Start),
            0x2 => Some(Self::Continue),
            0x3 => Some(Self::End),
            _ => None,
        }
    }
}

/// One System Exclusive (7-bit) packet (Figure 84). [`SysEx7::try_new`] is the only
/// constructor.
#[bitfield(u64, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct SysEx7 {
    /// Message Type, 4 bits: 0x3.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Group, 4 bits.
    #[bits(4)]
    pub group: Group,
    /// Status, 4 bits (Table 18). Read it with [`SysEx7::status`].
    #[bits(4, access = RO)]
    status_bits: u8,
    /// Number of Bytes, 4 bits: 0 to 6 valid data bytes.
    #[bits(4, access = RO)]
    pub number_of_bytes: u8,
    /// Six data bytes, 48 bits: each is a reserved bit and 7 data bits. Read them with
    /// [`SysEx7::data`].
    #[bits(48, access = RO)]
    data_bits: u64,
}

/// Mask of the bits that are valid when the packet holds `count` data bytes: the header and the
/// 7 low bits of each of the first `count` bytes.
/// One shift, not one for each byte: a 64-bit shift by a variable amount is a call or a long
/// sequence on a 32-bit target (OPT-030). `count` above 6 gives the mask of 6.
const fn valid_mask(count: u8) -> u64 {
    let count = if count as usize > MAX_BYTES {
        MAX_BYTES as u32
    } else {
        count as u32
    };
    // The data bytes of the first `count` positions: bits 47 down to 48 - 8 * count.
    let keep = if count == 0 {
        0
    } else {
        u64::MAX << (48 - 8 * count)
    };
    0xFFFF_0000_0000_0000 | (0x7F7F_7F7F_7F7F & keep)
}

impl SysEx7 {
    /// A packet with `status` and the data bytes `data`. Number of Bytes is `data.len()`, and
    /// unused bytes are zero (7.7).
    ///
    /// # Errors
    /// [`UmpError::OutOfRange`] when `data` holds more than 6 bytes or a byte above 0x7F.
    pub const fn try_new(
        group: Group,
        status: SysEx7Status,
        data: &[u8],
    ) -> Result<Self, UmpError> {
        if data.len() > MAX_BYTES {
            return Err(UmpError::OutOfRange);
        }
        let mut bits = (MessageType::Data64.into_bits() as u64) << 60
            | (group.into_bits() as u64) << 56
            | (status.into_bits() as u64) << 52
            | (data.len() as u64) << 48;
        let mut i = 0;
        while i < data.len() {
            if data[i] > 0x7F {
                return Err(UmpError::OutOfRange);
            }
            bits |= (data[i] as u64) << (40 - 8 * i);
            i += 1;
        }
        Ok(Self(bits))
    }

    /// The status of the packet (Table 18).
    pub const fn status(&self) -> SysEx7Status {
        match SysEx7Status::from_bits(self.status_bits()) {
            Some(status) => status,
            // Every constructor and `parse` keep the status at 0x0 to 0x3.
            None => SysEx7Status::Complete,
        }
    }

    /// The valid data bytes of the packet: `number_of_bytes` of them.
    pub const fn data(&self) -> SysEx7Data {
        let count = self.number_of_bytes();
        // The bytes after `count` are zero: the 32-bit halves with constant shifts (OPT-030).
        let bits = self.0 & valid_mask(count);
        let (w0, w1) = ((bits >> 32) as u32, bits as u32);
        let bytes = [
            (w0 >> 8) as u8,
            w0 as u8,
            (w1 >> 24) as u8,
            (w1 >> 16) as u8,
            (w1 >> 8) as u8,
            w1 as u8,
        ];
        SysEx7Data { bytes, len: count }
    }

    /// Decodes one System Exclusive (7-bit) packet from the start of `words`. Gives `None` for
    /// short input, a Message Type other than 0x3, a Status of 0x4 to 0xF or a Number of Bytes
    /// above 6. Reserved bits and unused bytes are cleared. Never panics.
    pub fn parse(words: &[u32]) -> Option<Self> {
        let bits = join_u64(take::<2>(words).ok()?);
        let packet = Self(bits);
        if !matches!(packet.message_type(), MessageType::Data64)
            || SysEx7Status::from_bits(packet.status_bits()).is_none()
            || packet.number_of_bytes() as usize > MAX_BYTES
        {
            return None;
        }
        Some(Self(bits & valid_mask(packet.number_of_bytes())))
    }

    /// Word 0 and word 1 of the packet.
    pub(crate) const fn into_words(self) -> [u32; 2] {
        split_u64(self.0)
    }
}

impl UmpWords for SysEx7 {
    const WORDS: usize = 2;

    fn from_words(words: &[u32]) -> Result<Self, UmpError> {
        take::<2>(words)?;
        Self::parse(words).ok_or(UmpError::UnexpectedMessage)
    }

    fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
        put(&self.into_words(), out)
    }
}

/// The 0 to 6 valid data bytes of one [`SysEx7`] packet. Derefs to a byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SysEx7Data {
    bytes: [u8; MAX_BYTES],
    len: u8,
}

impl Deref for SysEx7Data {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

/// Iterator over the packets of one System Exclusive (7-bit) message, in send order (7.7).
///
/// A payload of 0 to 6 bytes gives one Complete packet. A longer payload gives one Start
/// packet, zero or more Continue packets and one End packet, with 6 bytes in each packet but
/// the last.
#[derive(Debug, Clone)]
pub struct SysEx7Packets<'a> {
    group: Group,
    rest: &'a [u8],
    started: bool,
    done: bool,
}

impl<'a> SysEx7Packets<'a> {
    /// The packets for `payload` (without 0xF0 and 0xF7) on `group`.
    ///
    /// # Errors
    /// [`UmpError::OutOfRange`] when a payload byte is above 0x7F.
    pub fn try_new(group: Group, payload: &'a [u8]) -> Result<Self, UmpError> {
        if payload.iter().any(|&b| b > 0x7F) {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self {
            group,
            rest: payload,
            started: false,
            done: false,
        })
    }
}

impl Iterator for SysEx7Packets<'_> {
    type Item = SysEx7;

    fn next(&mut self) -> Option<SysEx7> {
        if self.done {
            return None;
        }
        let last = self.rest.len() <= MAX_BYTES;
        let status = match (self.started, last) {
            (false, true) => SysEx7Status::Complete,
            (false, false) => SysEx7Status::Start,
            (true, false) => SysEx7Status::Continue,
            (true, true) => SysEx7Status::End,
        };
        let (chunk, rest) = self.rest.split_at(self.rest.len().min(MAX_BYTES));
        self.rest = rest;
        self.started = true;
        self.done = last;
        SysEx7::try_new(self.group, status, chunk).ok()
    }
}

/// Result of [`SysEx7Reassembler::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysEx7Event<'a> {
    /// A complete message from a Complete packet, or from a Start to its End: the payload
    /// without 0xF0 and 0xF7.
    Complete(&'a [u8]),
    /// Another UMP on the same Group ended the message after its Start and before its End
    /// (7.7.1). `received` is the number of payload bytes received; they are discarded.
    Terminated {
        /// Payload bytes received before the message ended.
        received: usize,
    },
    /// The message ended normally, but its payload was longer than the capacity. `length` is
    /// the full payload length; the bytes are discarded.
    Overflow {
        /// Full payload length of the message.
        length: usize,
    },
}

/// Reassembles System Exclusive (7-bit) messages of one Group into a fixed buffer of `N` bytes
/// (7.7, 7.7.1). Needs no alloc.
#[derive(Debug, Clone)]
pub struct SysEx7Reassembler<const N: usize> {
    group: Group,
    buffer: [u8; N],
    /// Payload bytes received in the current message; can be above `N` after an overflow.
    length: usize,
    in_message: bool,
}

impl<const N: usize> SysEx7Reassembler<N> {
    /// An empty reassembler for the packets of `group`.
    pub const fn new(group: Group) -> Self {
        Self {
            group,
            buffer: [0; N],
            length: 0,
            in_message: false,
        }
    }

    /// Clears the reassembler, so it holds no partial message.
    pub fn reset(&mut self) {
        self.length = 0;
        self.in_message = false;
    }

    /// Takes the words of one packet and gives an event when a message ends.
    ///
    /// - Packets on other Groups and groupless packets (Message Types 0x0 and 0xF) give no
    ///   event, and System Real Time on the same Group gives no event (7.7.1).
    /// - A Start or Continue with fewer than 6 bytes does not end the message (7.7).
    /// - An End, or a Complete outside a message, gives [`SysEx7Event::Complete`], or
    ///   [`SysEx7Event::Overflow`] when the payload is longer than `N`.
    /// - Any other UMP on the same Group after a Start gives [`SysEx7Event::Terminated`]
    ///   (7.7.1). When that UMP is a Start, a new message starts with it. When it is a Complete,
    ///   the event is `Terminated`, and the Complete is not decoded: push it again to read it.
    /// - A Continue or End outside a message, and empty input, give no event.
    ///
    /// Never panics.
    pub fn push(&mut self, words: &[u32]) -> Option<SysEx7Event<'_>> {
        let w0 = *words.first()?;
        let message_type = MessageType::from_bits((w0 >> 28) as u8);
        if matches!(message_type, MessageType::Utility | MessageType::UmpStream)
            || Group::from_bits((w0 >> 24) as u8 & 0xF) != self.group
        {
            return None;
        }
        if matches!(message_type, MessageType::System) && (w0 >> 16) as u8 >= 0xF8 {
            return None;
        }
        let packet = if matches!(message_type, MessageType::Data64) {
            SysEx7::parse(words)
        } else {
            None
        };
        let Some(packet) = packet else {
            return self.terminate();
        };
        match (packet.status(), self.in_message) {
            (SysEx7Status::Complete, false) => {
                self.length = 0;
                self.append(&packet.data());
                self.finish()
            }
            (SysEx7Status::Complete, true) => self.terminate(),
            (SysEx7Status::Start, in_message) => {
                let event = if in_message { Some(self.length) } else { None };
                self.length = 0;
                self.in_message = true;
                self.append(&packet.data());
                event.map(|received| SysEx7Event::Terminated { received })
            }
            (SysEx7Status::Continue, true) => {
                self.append(&packet.data());
                None
            }
            (SysEx7Status::End, true) => {
                self.append(&packet.data());
                self.finish()
            }
            (SysEx7Status::Continue | SysEx7Status::End, false) => None,
        }
    }

    fn append(&mut self, data: &[u8]) {
        for &byte in data {
            if let Some(slot) = self.buffer.get_mut(self.length) {
                *slot = byte;
            }
            self.length = self.length.saturating_add(1);
        }
    }

    fn finish(&mut self) -> Option<SysEx7Event<'_>> {
        self.in_message = false;
        if self.length > N {
            Some(SysEx7Event::Overflow {
                length: self.length,
            })
        } else {
            Some(SysEx7Event::Complete(&self.buffer[..self.length]))
        }
    }

    fn terminate(&mut self) -> Option<SysEx7Event<'_>> {
        if !self.in_message {
            return None;
        }
        self.in_message = false;
        Some(SysEx7Event::Terminated {
            received: self.length,
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec::Vec;

    use super::*;
    use proptest::prelude::*;

    fn g(n: u8) -> Group {
        Group::new(n).unwrap()
    }

    fn words(p: SysEx7) -> [u32; 2] {
        p.into_words()
    }

    /// Figure 84: mt=3, group, status, # of bytes, then six data bytes.
    #[test]
    fn sysex7_matches_figure_84() {
        let p = SysEx7::try_new(
            g(0x5),
            SysEx7Status::Start,
            &[0x7E, 0x7F, 0x06, 0x01, 0x12, 0x34],
        )
        .unwrap();
        assert_eq!(words(p), [0x3516_7E7F, 0x0601_1234]);
        assert_eq!(p.message_type(), MessageType::Data64);
        assert_eq!(p.group(), g(5));
        assert_eq!(p.status(), SysEx7Status::Start);
        assert_eq!(p.number_of_bytes(), 6);
        assert_eq!(&*p.data(), &[0x7E, 0x7F, 0x06, 0x01, 0x12, 0x34]);
    }

    /// M2-104-UM D.3.7 example: 43 12 00 07 in one Complete UMP with # bytes 4; unused bytes zero.
    #[test]
    fn sysex7_d_3_7_example_and_unused_bytes_zero() {
        let p = SysEx7::try_new(g(0), SysEx7Status::Complete, &[0x43, 0x12, 0x00, 0x07]).unwrap();
        assert_eq!(words(p), [0x3004_4312, 0x0007_0000]);
        assert_eq!(&*p.data(), &[0x43, 0x12, 0x00, 0x07]);
        let empty = SysEx7::try_new(g(0xF), SysEx7Status::End, &[]).unwrap();
        assert_eq!(words(empty), [0x3F30_0000, 0x0000_0000]);
        assert!(empty.data().is_empty());
    }

    /// Table 18: the four status values and their conversions.
    #[test]
    fn sysex7_status_matches_table_18() {
        for (bits, status) in [
            (0x0, SysEx7Status::Complete),
            (0x1, SysEx7Status::Start),
            (0x2, SysEx7Status::Continue),
            (0x3, SysEx7Status::End),
        ] {
            assert_eq!(status.into_bits(), bits);
            assert_eq!(SysEx7Status::from_bits(bits), Some(status));
        }
        for bits in 0x4..=0xF {
            assert_eq!(SysEx7Status::from_bits(bits), None);
        }
    }

    #[test]
    fn sysex7_try_new_rejects_bad_data() {
        assert_eq!(
            SysEx7::try_new(g(0), SysEx7Status::Complete, &[0; 7]),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            SysEx7::try_new(g(0), SysEx7Status::Complete, &[0x80]),
            Err(UmpError::OutOfRange)
        );
    }

    /// 7.7: a reserved status or a Number of Bytes above 6 is not a System Exclusive (7-bit)
    /// packet; Ump keeps it as unknown.
    #[test]
    fn sysex7_parse_rejects_undefined_status_and_count() {
        assert_eq!(SysEx7::parse(&[0x3040_0000, 0]), None);
        assert_eq!(SysEx7::parse(&[0x3007_0000, 0]), None);
        assert_eq!(SysEx7::parse(&[0x2000_0000, 0]), None);
        assert_eq!(SysEx7::parse(&[0x3000_0000]), None);
        assert_eq!(
            SysEx7::from_words(&[0x3000_0000]),
            Err(UmpError::Truncated { needed: 2, got: 1 })
        );
        assert_eq!(
            SysEx7::from_words(&[0x30F0_0000, 0]),
            Err(UmpError::UnexpectedMessage)
        );
    }

    /// 7.7, 2.1.3: the reader clears reserved bits and the bytes after Number of Bytes.
    #[test]
    fn sysex7_reader_clears_reserved_bits_and_unused_bytes() {
        let p = SysEx7::parse(&[0x3002_C1FF, 0xFFFF_FFFF]).unwrap();
        assert_eq!(&*p.data(), &[0x41, 0x7F]);
        assert_eq!(words(p), [0x3002_417F, 0x0000_0000]);
    }

    #[test]
    fn sysex7_to_words_round_trip() {
        let p = SysEx7::try_new(g(3), SysEx7Status::Continue, &[1, 2, 3]).unwrap();
        let mut out = [0u32; 2];
        assert_eq!(p.to_words(&mut out), Ok(2));
        assert_eq!(SysEx7::from_words(&out), Ok(p));
        assert_eq!(
            p.to_words(&mut out[..1]),
            Err(UmpError::BufferTooSmall { needed: 2, got: 1 })
        );
    }

    fn split(payload: &[u8]) -> Vec<SysEx7> {
        SysEx7Packets::try_new(g(1), payload).unwrap().collect()
    }

    /// 7.7: a short message is one Complete UMP; an empty payload is one Complete with 0 bytes.
    #[test]
    fn packets_short_payload_is_one_complete() {
        for len in [0usize, 1, 6] {
            let payload: Vec<u8> = (0..len as u8).collect();
            let packets = split(&payload);
            assert_eq!(packets.len(), 1);
            assert_eq!(packets[0].status(), SysEx7Status::Complete);
            assert_eq!(&*packets[0].data(), &payload[..]);
        }
    }

    /// 7.7: Start, zero or more Continue, End; 6 bytes in each packet but the last.
    #[test]
    fn packets_long_payload_is_start_continue_end() {
        let statuses = |len: usize| -> Vec<(SysEx7Status, u8)> {
            let payload: Vec<u8> = (0..len).map(|i| i as u8 & 0x7F).collect();
            split(&payload)
                .iter()
                .map(|p| (p.status(), p.number_of_bytes()))
                .collect()
        };
        use SysEx7Status::*;
        assert_eq!(statuses(7), [(Start, 6), (End, 1)]);
        assert_eq!(statuses(12), [(Start, 6), (End, 6)]);
        assert_eq!(statuses(13), [(Start, 6), (Continue, 6), (End, 1)]);
        assert_eq!(
            statuses(20),
            [(Start, 6), (Continue, 6), (Continue, 6), (End, 2)]
        );
    }

    #[test]
    fn packets_reject_byte_above_7f() {
        assert_eq!(
            SysEx7Packets::try_new(g(0), &[0x7E, 0xF7]).err(),
            Some(UmpError::OutOfRange)
        );
    }

    fn push_all<const N: usize>(
        r: &mut SysEx7Reassembler<N>,
        packets: &[[u32; 2]],
    ) -> Vec<Option<Vec<u8>>> {
        packets
            .iter()
            .map(|w| match r.push(w) {
                Some(SysEx7Event::Complete(d)) => Some(d.to_vec()),
                Some(other) => panic!("unexpected event {other:?}"),
                None => None,
            })
            .collect()
    }

    /// 7.7: a Complete UMP and a Start..End sequence give the payload.
    #[test]
    fn reassembler_gives_complete_messages() {
        let mut r = SysEx7Reassembler::<32>::new(g(1));
        let payload: Vec<u8> = (0..15).collect();
        let packets: Vec<[u32; 2]> = split(&payload).into_iter().map(words).collect();
        let events = push_all(&mut r, &packets);
        assert_eq!(events, [None, None, Some(payload)]);
        let single: Vec<[u32; 2]> = split(&[0x7D, 0x01]).into_iter().map(words).collect();
        assert_eq!(push_all(&mut r, &single), [Some(std::vec![0x7D, 0x01])]);
    }

    /// 7.7 note: a Start or Continue with fewer than 6 bytes does not end the message.
    #[test]
    fn reassembler_short_start_and_continue_do_not_end() {
        let mut r = SysEx7Reassembler::<16>::new(g(0));
        let start = words(SysEx7::try_new(g(0), SysEx7Status::Start, &[1, 2]).unwrap());
        let cont = words(SysEx7::try_new(g(0), SysEx7Status::Continue, &[3]).unwrap());
        let end = words(SysEx7::try_new(g(0), SysEx7Status::End, &[4]).unwrap());
        assert_eq!(
            push_all(&mut r, &[start, cont, end]),
            [None, None, Some(std::vec![1, 2, 3, 4])]
        );
    }

    /// 7.7.1: System Real Time on the same Group, packets of other Groups and groupless
    /// packets may come between the UMPs of a message.
    #[test]
    fn reassembler_ignores_real_time_other_groups_and_groupless() {
        let mut r = SysEx7Reassembler::<16>::new(g(2));
        let start = words(SysEx7::try_new(g(2), SysEx7Status::Start, &[1; 6]).unwrap());
        let end = words(SysEx7::try_new(g(2), SysEx7Status::End, &[2]).unwrap());
        assert_eq!(r.push(&start), None);
        assert_eq!(r.push(&[0x12F8_0000]), None); // Timing Clock, Group 2
        assert_eq!(r.push(&[0x12FE_0000]), None); // Active Sensing, Group 2
        assert_eq!(r.push(&[0x2390_3C40]), None); // Note On, Group 3
        assert_eq!(r.push(&[0x0020_1234]), None); // JR Timestamp (groupless)
        assert_eq!(r.push(&[0xF000_0000, 0, 0, 0]), None); // UMP Stream (groupless)
        let other = words(SysEx7::try_new(g(3), SysEx7Status::Complete, &[9]).unwrap());
        assert_eq!(r.push(&other), None);
        assert_eq!(
            r.push(&end),
            Some(SysEx7Event::Complete(&[1, 1, 1, 1, 1, 1, 2]))
        );
    }

    /// 7.7.1: any other UMP on the same Group after a Start ends the message (REQ-SX7-003).
    #[test]
    fn reassembler_other_ump_on_same_group_terminates() {
        let mut r = SysEx7Reassembler::<16>::new(g(0));
        let start = words(SysEx7::try_new(g(0), SysEx7Status::Start, &[1, 2, 3]).unwrap());
        let cont = words(SysEx7::try_new(g(0), SysEx7Status::Continue, &[4]).unwrap());
        let end = words(SysEx7::try_new(g(0), SysEx7Status::End, &[5]).unwrap());
        // MIDI 1.0 Note On on Group 0.
        assert_eq!(r.push(&start), None);
        assert_eq!(
            r.push(&[0x2090_3C40]),
            Some(SysEx7Event::Terminated { received: 3 })
        );
        assert_eq!(r.push(&end), None);
        // System Common (Song Select) on Group 0 is not Real Time.
        assert_eq!(r.push(&start), None);
        assert_eq!(
            r.push(&[0x10F3_0100]),
            Some(SysEx7Event::Terminated { received: 3 })
        );
        // A reserved SysEx7 status on Group 0.
        assert_eq!(r.push(&start), None);
        assert_eq!(
            r.push(&[0x3040_0000, 0]),
            Some(SysEx7Event::Terminated { received: 3 })
        );
        // A new Start ends the old message and starts a new one.
        assert_eq!(r.push(&start), None);
        assert_eq!(r.push(&cont), None);
        assert_eq!(
            r.push(&start),
            Some(SysEx7Event::Terminated { received: 4 })
        );
        assert_eq!(r.push(&end), Some(SysEx7Event::Complete(&[1, 2, 3, 5])));
        // A Complete ends the old message; pushed again, it is read.
        let complete = words(SysEx7::try_new(g(0), SysEx7Status::Complete, &[7]).unwrap());
        assert_eq!(r.push(&start), None);
        assert_eq!(
            r.push(&complete),
            Some(SysEx7Event::Terminated { received: 3 })
        );
        assert_eq!(r.push(&complete), Some(SysEx7Event::Complete(&[7])));
    }

    #[test]
    fn reassembler_ignores_continue_and_end_outside_a_message() {
        let mut r = SysEx7Reassembler::<4>::new(g(0));
        let cont = words(SysEx7::try_new(g(0), SysEx7Status::Continue, &[4]).unwrap());
        let end = words(SysEx7::try_new(g(0), SysEx7Status::End, &[5]).unwrap());
        assert_eq!(r.push(&cont), None);
        assert_eq!(r.push(&end), None);
        assert_eq!(r.push(&[]), None);
        assert_eq!(r.push(&[0x2090_3C40]), None);
    }

    #[test]
    fn reassembler_reports_overflow_and_reset_clears() {
        let mut r = SysEx7Reassembler::<8>::new(g(0));
        let payload: Vec<u8> = (0..13).collect();
        let packets: Vec<[u32; 2]> = split(&payload)
            .into_iter()
            .map(|p| words(p.with_group(g(0))))
            .collect();
        assert_eq!(r.push(&packets[0]), None);
        assert_eq!(r.push(&packets[1]), None);
        assert_eq!(
            r.push(&packets[2]),
            Some(SysEx7Event::Overflow { length: 13 })
        );
        assert_eq!(r.push(&packets[0]), None);
        r.reset();
        assert_eq!(r.push(&packets[2]), None);
    }

    /// DEC-UMP-003: every payload splits into packets that reassemble to the same payload, with
    /// 6 bytes in each packet but the last (REQ-SX7-001, REQ-SX7-002).
    #[test]
    fn prop_packets_reassemble_to_payload() {
        proptest!(|(payload in proptest::collection::vec(0u8..=0x7F, 0..64), group in 0u8..16)| {
            let packets: Vec<SysEx7> = SysEx7Packets::try_new(g(group), &payload).unwrap().collect();
            let (last, init) = packets.split_last().unwrap();
            prop_assert!(init.iter().all(|p| p.number_of_bytes() == 6));
            prop_assert!(last.number_of_bytes() as usize <= 6);
            let mut r = SysEx7Reassembler::<64>::new(g(group));
            for p in init {
                prop_assert_eq!(r.push(&words(*p)), None);
            }
            prop_assert_eq!(r.push(&words(*last)), Some(SysEx7Event::Complete(&payload[..])));
        });
    }

    /// The reader never panics, clears reserved bits, and writes back what it decoded.
    #[test]
    fn prop_sysex7_parse_never_panics_and_round_trips() {
        proptest!(|(w0 in any::<u32>(), w1 in any::<u32>())| {
            if let Some(p) = SysEx7::parse(&[w0, w1]) {
                let out = words(p);
                prop_assert_eq!(out[0] & 0xFFFF_0000, w0 & 0xFFFF_0000);
                prop_assert_eq!(SysEx7::parse(&out), Some(p));
                prop_assert_eq!(p.data().len(), p.number_of_bytes() as usize);
                prop_assert!(p.data().iter().all(|&b| b <= 0x7F));
            }
        });
    }

    /// The reassembler never panics on any input.
    #[test]
    fn prop_reassembler_never_panics() {
        proptest!(|(input in proptest::collection::vec(proptest::collection::vec(any::<u32>(), 0..5), 0..20))| {
            let mut r = SysEx7Reassembler::<8>::new(g(0));
            for w in &input {
                let _ = r.push(w);
            }
        });
    }
}
