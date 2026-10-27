//! UMPs over the USB packets of a USB MIDI 2.0 Alternate Setting (USB-MIDI v2.0 sections 3.2
//! to 3.3 and 4).
//!
//! - Each 32-bit UMP word is sent with the least significant byte first, word 0 first
//!   (3.2.2, REQ-USB-UMP-003, REQ-USB-UMP-004): [`word_to_bytes`], [`word_from_bytes`].
//! - [`UmpPacketWriter`] fills one USB packet of at most wMaxPacketSize bytes for each
//!   [`UmpPacketWriter::packet`] call. It puts only whole UMPs into a packet
//!   (REQ-USB-UMP-009) and splits a UMP that is longer than wMaxPacketSize at word
//!   boundaries into consecutive packets (REQ-USB-UMP-010, choice U-08 of DEC-USB-001).
//! - [`UmpPacketReader`] reads the packets of one endpoint as one sequence of UMPs. It frames
//!   each UMP by the size of its Message Type (zmidi-ump [`MessageType::word_count`], M2-104
//!   Table 4 = USB-MIDI v2.0 Table 3-1) and keeps at most 3 words of a UMP that a packet cuts
//!   (REQ-USB-UMP-005, REQ-USB-UMP-006). A tail of 1 to 3 bytes is ignored and reported
//!   (REQ-USB-UMP-007, choice U-04).
//! - [`ReceivedUmp`] gives the words of one UMP with its [`UmpTarget`]: the Group Terminal of
//!   its Group (REQ-USB-UMP-014), "no Group Terminal" (REQ-USB-UMP-016, choice U-05) or the
//!   whole stream for Message Types 0x0 and 0xF (REQ-USB-UMP-015, AMB-USB-MIDI-2-01).
//!
//! The module reads only the Message Type and the Group field of a UMP and changes no bit
//! (section 2, 3.2.1.1, REQ-USB-UMP-002). Reserved Message Types are moved like the others.
//! The data of each endpoint is one UMP stream with up to 16 Groups (3.3.2, REQ-USB-UMP-012):
//! one reader and one writer for each endpoint, with no shared state.
//!
//! No alloc (DEC-002). Synchronous and sans-IO with no USB stack (DEC-004, DEC-014). Nothing
//! panics on received bytes; the reader is lenient (choice U-24).

use zmidi_ump::{Group, MessageType, Ump, UmpError};

use crate::gtb::{GroupMask, GroupTerminal, TerminalDirection};
use crate::ms2::EndpointDirection;

/// The size of one UMP word in bytes (3.2.2).
pub const WORD_LEN: usize = 4;

/// The largest UMP in words (128 bits, Table 3-1).
pub const MAX_UMP_WORDS: usize = 4;

/// The largest number of words that the reader holds for one endpoint (REQ-USB-UMP-006).
pub const MAX_HELD_WORDS: usize = MAX_UMP_WORDS - 1;

/// Error of the UMP packet reader and writer (thiserror 2, no_std).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UmpUsbError {
    /// The caller buffer is shorter than the packet size of the writer; nothing was written.
    /// A caller defect: give a buffer of [`UmpPacketWriter::packet_size`] bytes.
    #[error("output buffer too small")]
    Overflow,
    /// The UMP does not fit in the rest of this packet. Nothing was written: send this packet
    /// and give the same UMP to the next packet (REQ-USB-UMP-009, REQ-USB-UMP-010).
    #[error("USB packet full")]
    PacketFull,
    /// The words that the caller gives hold fewer words than the Message Type of word 0 needs;
    /// nothing was written. [`UmpPacketWriter::write_packet`] takes such a UMP from the
    /// iterator and reports it with this error, so the output does not stop.
    #[error("UMP needs {needed} words, {got} given")]
    ShortUmp {
        /// The size of the Message Type in words (Table 3-1).
        needed: usize,
        /// The number of words given.
        got: usize,
    },
    /// A wMaxPacketSize below 4 bytes: no UMP word fits in a packet.
    #[error("packet size below 4 bytes")]
    InvalidPacketSize,
    /// 1 to 3 bytes after the last whole word of a USB packet; they are ignored and a held
    /// part of a UMP is kept (REQ-USB-UMP-007, choice U-04).
    #[error("{len} bytes after the last whole UMP word")]
    Tail {
        /// The number of ignored bytes, 1 to 3.
        len: usize,
    },
}

/// The 4 USB packet bytes of a UMP word: least significant byte first (3.2.2,
/// REQ-USB-UMP-003).
pub const fn word_to_bytes(word: u32) -> [u8; WORD_LEN] {
    word.to_le_bytes()
}

/// The UMP word of 4 USB packet bytes: least significant byte first (3.2.2,
/// REQ-USB-UMP-003).
pub const fn word_from_bytes(bytes: [u8; WORD_LEN]) -> u32 {
    u32::from_le_bytes(bytes)
}

/// The size in words of the UMP that starts with `word0` (Message Type, bits 31..28; Table
/// 3-1 = M2-104 Table 4, REQ-USB-UMP-005).
pub const fn ump_words(word0: u32) -> usize {
    MessageType::from_bits((word0 >> 28) as u8).word_count()
}

/// Where a received UMP goes (REQ-USB-UMP-014, REQ-USB-UMP-015, REQ-USB-UMP-016).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UmpTarget {
    /// The Group Terminal of the Group field of the UMP; it is in the Group Terminal Blocks of
    /// the endpoint (REQ-USB-UMP-014).
    Terminal(GroupTerminal),
    /// The Group of the UMP has no Group Terminal on this endpoint. The UMP is not dropped
    /// (REQ-USB-UMP-016, choice U-05).
    NoGroupTerminal(Group),
    /// Message Type 0x0 (Utility) or 0xF (UMP Stream): a message of the whole UMP stream of
    /// the endpoint, with no Group Terminal (REQ-USB-UMP-015, AMB-USB-MIDI-2-01).
    Stream,
}

/// One UMP that the reader gives, with its target and the Group Terminal direction of its
/// endpoint (REQ-USB-UMP-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReceivedUmp {
    /// Words 0 to 3; words after the UMP size are zero.
    words: [u32; MAX_UMP_WORDS],
    /// The size in words, 1 to 4 (Table 3-1).
    len: u8,
    /// The target of the UMP.
    target: UmpTarget,
    /// IN Group Terminal for a MIDI OUT endpoint, OUT for a MIDI IN endpoint.
    direction: TerminalDirection,
}

impl ReceivedUmp {
    /// The UMP words, unchanged, word 0 first (REQ-USB-UMP-002, REQ-USB-UMP-003).
    pub fn words(&self) -> &[u32] {
        self.words.get(..self.len as usize).unwrap_or(&self.words)
    }

    /// The Message Type of the UMP (bits 31..28 of word 0).
    pub fn message_type(&self) -> MessageType {
        MessageType::from_bits((self.words[0] >> 28) as u8)
    }

    /// The target of the UMP (REQ-USB-UMP-014 to REQ-USB-UMP-016).
    pub fn target(&self) -> UmpTarget {
        self.target
    }

    /// The Group Terminal of the UMP when it is in the Group Terminal Blocks of the endpoint
    /// (REQ-USB-UMP-014); `None` for [`UmpTarget::NoGroupTerminal`] and [`UmpTarget::Stream`].
    pub fn group_terminal(&self) -> Option<GroupTerminal> {
        match self.target {
            UmpTarget::Terminal(t) => Some(t),
            _ => None,
        }
    }

    /// The Group Terminal Number of the UMP: its Group field value (REQ-USB-UMP-014); `None`
    /// for Message Types 0x0 and 0xF, which have no Group (REQ-USB-UMP-015).
    pub fn group_terminal_number(&self) -> Option<u8> {
        match self.target {
            UmpTarget::Terminal(t) => Some(t.number),
            UmpTarget::NoGroupTerminal(g) => Some(g.index()),
            UmpTarget::Stream => None,
        }
    }

    /// The Group Terminal direction of the endpoint: IN for a MIDI OUT endpoint, OUT for a
    /// MIDI IN endpoint (REQ-USB-UMP-014).
    pub fn direction(&self) -> TerminalDirection {
        self.direction
    }

    /// The UMP decoded by zmidi-ump; an unknown Status or reserved Message Type gives
    /// `Ump::Unknown` with the raw words.
    pub fn ump(&self) -> Result<Ump, UmpError> {
        Ump::parse(self.words()).map(|(u, _)| u)
    }

    /// The received UMP of `words` (exactly its size) on an endpoint with `mask` and
    /// `direction`.
    fn new(words: &[u32], mask: GroupMask, direction: TerminalDirection) -> Self {
        let mut w = [0u32; MAX_UMP_WORDS];
        for (to, from) in w.iter_mut().zip(words) {
            *to = *from;
        }
        let target = match MessageType::from_bits((w[0] >> 28) as u8) {
            MessageType::Utility | MessageType::UmpStream => UmpTarget::Stream,
            _ => {
                let group = Group::from_bits((w[0] >> 24) as u8);
                if mask.contains_group(group) {
                    UmpTarget::Terminal(GroupTerminal::from_group(group, direction))
                } else {
                    UmpTarget::NoGroupTerminal(group)
                }
            }
        };
        ReceivedUmp {
            words: w,
            len: words.len().min(MAX_UMP_WORDS) as u8,
            target,
            direction,
        }
    }
}

/// Reader of the UMPs of one MIDI Streaming Data Endpoint (USB-MIDI v2.0 3.2.3, 3.3.2).
///
/// It keeps at most 3 words of a UMP that a USB packet cuts (12 bytes, no alloc). Use one
/// reader for each endpoint: the state of one reader does not change another
/// (REQ-USB-UMP-012).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UmpPacketReader {
    /// The Group Terminal direction of the endpoint (REQ-USB-UMP-014).
    direction: TerminalDirection,
    /// The Group Terminals of the endpoint (REQ-USB-UMP-016).
    mask: GroupMask,
    /// The words of the UMP in progress; `held[0]` is word 0.
    held: [u32; MAX_UMP_WORDS],
    /// The number of words in `held`, 0 to 3 between calls.
    held_len: usize,
}

impl Default for UmpPacketReader {
    /// A reader of a MIDI OUT endpoint (device side) with all 16 Group Terminals.
    fn default() -> Self {
        Self::new(EndpointDirection::MidiOut, GroupMask::ALL)
    }
}

impl UmpPacketReader {
    /// A reader of an endpoint of direction `endpoint` whose Group Terminal Blocks give
    /// `mask` (for example [`GroupMask::from_blocks`]).
    pub const fn new(endpoint: EndpointDirection, mask: GroupMask) -> Self {
        UmpPacketReader {
            direction: TerminalDirection::of_endpoint(endpoint),
            mask,
            held: [0; MAX_UMP_WORDS],
            held_len: 0,
        }
    }

    /// The Group Terminal direction of the endpoint.
    pub fn direction(&self) -> TerminalDirection {
        self.direction
    }

    /// The Group Terminals of the endpoint.
    pub fn mask(&self) -> GroupMask {
        self.mask
    }

    /// Changes the Group Terminals of the endpoint (for example after a change of Alternate
    /// Setting). A held part of a UMP is kept.
    pub fn set_mask(&mut self, mask: GroupMask) {
        self.mask = mask;
    }

    /// The number of held words of a UMP that a packet cut, 0 to 3 (REQ-USB-UMP-006).
    pub fn held(&self) -> usize {
        self.held_len
    }

    /// Discards a held part of a UMP: the next packet starts at a UMP boundary. Call it on a
    /// change of Alternate Setting, a USB reset or a change of configuration
    /// (REQ-USB-UMP-008, done by the Link module).
    pub fn reset(&mut self) {
        self.held = [0; MAX_UMP_WORDS];
        self.held_len = 0;
    }

    /// Gives one received word to the reader. Gives the UMP when this word completes it, and
    /// never a part of a UMP (REQ-USB-UMP-005, REQ-USB-UMP-006).
    pub fn push_word(&mut self, word: u32) -> Option<ReceivedUmp> {
        if let Some(slot) = self.held.get_mut(self.held_len) {
            *slot = word;
            self.held_len += 1;
        } else {
            // Not reachable: a full UMP is given below. Start again at this word.
            self.held[0] = word;
            self.held_len = 1;
        }
        let need = ump_words(self.held[0]);
        if self.held_len < need {
            return None;
        }
        let ump = ReceivedUmp::new(
            self.held.get(..need).unwrap_or(&self.held),
            self.mask,
            self.direction,
        );
        self.reset();
        Some(ump)
    }

    /// Reads `bytes`, the data of one USB packet (or transfer) of this endpoint, word by word,
    /// least significant byte first (REQ-USB-UMP-003).
    ///
    /// The iterator gives each complete UMP as `Ok`, in order, and then one
    /// [`UmpUsbError::Tail`] when 1 to 3 bytes remain after the last whole word
    /// (REQ-USB-UMP-007). The words of a UMP that the packet cuts stay in the reader for the
    /// next call (REQ-USB-UMP-006).
    ///
    /// If the iterator is dropped before its end, the drop reads the rest of the words into
    /// the reader and discards the UMPs that they complete. Thus the held part always agrees
    /// with the byte stream. [`Self::read_with`] is the callback form.
    pub fn read<'a>(&'a mut self, bytes: &'a [u8]) -> UmpRead<'a> {
        let (words, rest) = bytes.as_chunks::<WORD_LEN>();
        UmpRead {
            reader: self,
            words: words.iter(),
            tail: rest.len(),
        }
    }

    /// Reads all of `bytes` as [`Self::read`] does and gives each item to `sink`, in order.
    pub fn read_with(
        &mut self,
        bytes: &[u8],
        mut sink: impl FnMut(Result<ReceivedUmp, UmpUsbError>),
    ) {
        self.read(bytes).for_each(&mut sink);
    }
}

/// Iterator over the UMPs of one USB packet; see [`UmpPacketReader::read`]. A drop before the
/// end reads the rest of the packet and discards its UMPs.
#[derive(Debug)]
#[must_use = "the UMPs of the packet are discarded if the iterator is not used"]
pub struct UmpRead<'a> {
    reader: &'a mut UmpPacketReader,
    words: core::slice::Iter<'a, [u8; WORD_LEN]>,
    /// Number of bytes after the last whole word; set to 0 when reported.
    tail: usize,
}

impl Iterator for UmpRead<'_> {
    type Item = Result<ReceivedUmp, UmpUsbError>;

    fn next(&mut self) -> Option<Self::Item> {
        for &b in self.words.by_ref() {
            if let Some(ump) = self.reader.push_word(word_from_bytes(b)) {
                return Some(Ok(ump));
            }
        }
        if self.tail > 0 {
            let len = self.tail;
            self.tail = 0;
            return Some(Err(UmpUsbError::Tail { len }));
        }
        None
    }
}

impl Drop for UmpRead<'_> {
    /// Reads the rest of the packet into the reader and discards the UMPs that it completes,
    /// so the held words agree with the byte stream (a cut UMP is never joined to wrong
    /// words). Bounded by the packet length; never panics.
    fn drop(&mut self) {
        for &b in self.words.by_ref() {
            let _ = self.reader.push_word(word_from_bytes(b));
        }
    }
}

/// The rest of a UMP that the writer split (U-08): at most 4 words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rest {
    /// The words of the split UMP.
    words: [u32; MAX_UMP_WORDS],
    /// The next word of `words` to write.
    pos: usize,
    /// The number of words in `words`; `pos == len` when nothing is held.
    len: usize,
}

impl Rest {
    const EMPTY: Rest = Rest {
        words: [0; MAX_UMP_WORDS],
        pos: 0,
        len: 0,
    };

    const fn pending(&self) -> bool {
        self.pos < self.len
    }
}

/// Writer of the UMPs of one MIDI Streaming Data Endpoint into USB packets of at most
/// wMaxPacketSize bytes (USB-MIDI v2.0 3.2.3).
///
/// Each [`Self::packet`] call starts one USB packet in a caller buffer. The writer keeps only
/// the rest of a UMP that it split (U-08), at most 3 words. It never reorders UMPs
/// (REQ-USB-UMP-012) and never reads the Group (REQ-USB-UMP-016).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UmpPacketWriter {
    /// The packet size in bytes: wMaxPacketSize rounded down to a multiple of 4.
    size: usize,
    /// The rest of the UMP that the writer split; committed by [`UmpPacket::finish`].
    rest: Rest,
}

impl UmpPacketWriter {
    /// A writer for packets of at most `max_packet_size` bytes (wMaxPacketSize of the
    /// endpoint). A value that is not a multiple of 4 is rounded down to whole words;
    /// [`UmpUsbError::InvalidPacketSize`] for a value below 4.
    pub const fn new(max_packet_size: u16) -> Result<Self, UmpUsbError> {
        let size = max_packet_size as usize & !(WORD_LEN - 1);
        if size == 0 {
            return Err(UmpUsbError::InvalidPacketSize);
        }
        Ok(UmpPacketWriter {
            size,
            rest: Rest::EMPTY,
        })
    }

    /// The packet size in bytes (a multiple of 4).
    pub fn packet_size(&self) -> usize {
        self.size
    }

    /// True when the writer holds the rest of a split UMP: the next packet starts with it
    /// (REQ-USB-UMP-010). Send a packet even when no new UMP is ready.
    pub fn pending(&self) -> bool {
        self.rest.pending()
    }

    /// Discards the rest of a split UMP (REQ-USB-UMP-008, done by the Link module).
    pub fn reset(&mut self) {
        self.rest = Rest::EMPTY;
    }

    /// Starts one USB packet in `out` and writes the rest of a split UMP first
    /// (REQ-USB-UMP-010). Add UMPs with [`UmpPacket::push`], then call
    /// [`UmpPacket::finish`] and send the first bytes of `out` that it gives as one USB
    /// packet.
    ///
    /// The writer state changes only in [`UmpPacket::finish`]: a packet that is dropped
    /// without `finish` (not sent) changes nothing, and the next packet starts again with the
    /// same rest.
    ///
    /// [`UmpUsbError::Overflow`], with nothing written and no state change, when `out` is
    /// shorter than [`Self::packet_size`].
    pub fn packet<'a>(&'a mut self, out: &'a mut [u8]) -> Result<UmpPacket<'a>, UmpUsbError> {
        let out = out.get_mut(..self.size).ok_or(UmpUsbError::Overflow)?;
        let rest = self.rest;
        let mut packet = UmpPacket {
            writer: self,
            rest,
            out,
            len: 0,
        };
        packet.write_rest();
        Ok(packet)
    }

    /// Writes one USB packet into `out` from `umps` and gives its length in bytes: the rest of
    /// a split UMP, then each UMP of `umps` while it fits. A UMP that does not fit stays in
    /// `umps` for the next call (REQ-USB-UMP-009). Each item of `umps` is the words of one UMP.
    ///
    /// - [`UmpUsbError::Overflow`], with nothing written and no state change, when `out` is
    ///   shorter than [`Self::packet_size`].
    /// - [`UmpUsbError::ShortUmp`] when the first item of the packet has fewer words than its
    ///   Message Type needs: the item is taken from `umps`, nothing is written and the writer
    ///   state does not change. When such an item comes after other UMPs, the packet ends
    ///   before it and the next call reports it. Thus a short UMP never stops the output and
    ///   is never dropped silently.
    pub fn write_packet<'w, I>(
        &mut self,
        umps: &mut core::iter::Peekable<I>,
        out: &mut [u8],
    ) -> Result<usize, UmpUsbError>
    where
        I: Iterator<Item = &'w [u32]>,
    {
        let mut packet = self.packet(out)?;
        while let Some(words) = umps.peek() {
            match packet.push(words) {
                Err(UmpUsbError::PacketFull) => break,
                Err(e @ UmpUsbError::ShortUmp { .. }) => {
                    if packet.is_empty() {
                        umps.next();
                        return Err(e);
                    }
                    break;
                }
                _ => {
                    umps.next();
                }
            }
        }
        Ok(packet.finish())
    }
}

/// One USB packet in progress; see [`UmpPacketWriter::packet`]. Call [`Self::finish`] when
/// the packet is sent; a drop without `finish` changes no writer state.
#[derive(Debug)]
#[must_use = "call finish() to send the packet; a dropped packet changes no writer state"]
pub struct UmpPacket<'a> {
    writer: &'a mut UmpPacketWriter,
    /// The rest of a split UMP after this packet; [`Self::finish`] commits it.
    rest: Rest,
    /// The caller buffer, exactly the packet size.
    out: &'a mut [u8],
    /// Bytes written, a multiple of 4.
    len: usize,
}

impl UmpPacket<'_> {
    /// The number of bytes in the packet (a multiple of 4).
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when the packet has no byte.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of free bytes in the packet.
    pub fn free(&self) -> usize {
        self.out.len().saturating_sub(self.len)
    }

    /// Writes the UMP at the start of `words` (its size from the Message Type of word 0) with
    /// no change, least significant byte first, and gives the number of words used
    /// (REQ-USB-UMP-002, REQ-USB-UMP-003). An empty `words` gives `Ok(0)`.
    ///
    /// - [`UmpUsbError::PacketFull`] when the UMP does not fit in the rest of the packet, or
    ///   when the packet holds the rest of a split UMP: nothing is written; give the UMP to
    ///   the next packet (REQ-USB-UMP-009).
    /// - A UMP longer than the packet size, in an empty packet, is split at word boundaries:
    ///   this packet gets its first words and the next packets get the rest, in word order,
    ///   with no other UMP between them (REQ-USB-UMP-010, choice U-08).
    /// - [`UmpUsbError::ShortUmp`] when `words` is shorter than the UMP.
    pub fn push(&mut self, words: &[u32]) -> Result<usize, UmpUsbError> {
        let Some(&w0) = words.first() else {
            return Ok(0);
        };
        let need = ump_words(w0);
        let Some(ump) = words.get(..need) else {
            return Err(UmpUsbError::ShortUmp {
                needed: need,
                got: words.len(),
            });
        };
        if self.rest.pending() {
            return Err(UmpUsbError::PacketFull);
        }
        let bytes = need * WORD_LEN;
        if bytes <= self.free() {
            self.put(ump);
            return Ok(need);
        }
        if bytes > self.out.len() && self.is_empty() {
            // U-08: split at word boundaries; the rest starts the next packets.
            let mut rest = Rest::EMPTY;
            for (to, from) in rest.words.iter_mut().zip(ump) {
                *to = *from;
            }
            rest.len = need;
            self.rest = rest;
            self.write_rest();
            return Ok(need);
        }
        Err(UmpUsbError::PacketFull)
    }

    /// Writes the UMP `ump` as [`Self::push`] does.
    pub fn push_ump(&mut self, ump: &Ump) -> Result<usize, UmpUsbError> {
        let mut words = [0u32; MAX_UMP_WORDS];
        let n = ump.write(&mut words).unwrap_or(0);
        self.push(words.get(..n).unwrap_or(&[]))
    }

    /// Ends the packet, commits the rest of a split UMP to the writer, and gives the length
    /// of the packet in bytes. Send these bytes as one USB packet.
    pub fn finish(self) -> usize {
        self.writer.rest = self.rest;
        self.len
    }

    /// Writes whole words of `words`; the caller checked the free space.
    fn put(&mut self, words: &[u32]) {
        let free = self.out.get_mut(self.len..).unwrap_or(&mut []);
        let (chunks, _) = free.as_chunks_mut::<WORD_LEN>();
        for (to, from) in chunks.iter_mut().zip(words) {
            *to = word_to_bytes(*from);
            self.len += WORD_LEN;
        }
    }

    /// Writes as many words of the held rest of a split UMP as fit.
    fn write_rest(&mut self) {
        let r = self.rest;
        let rest = r.words.get(r.pos..r.len).unwrap_or(&[]);
        let n = rest.len().min(self.free() / WORD_LEN);
        self.put(rest.get(..n).unwrap_or(&[]));
        self.rest.pos += n;
        if !self.rest.pending() {
            self.rest = Rest::EMPTY;
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;
    use std::vec;
    use std::vec::Vec;

    /// The word lists of the UMPs that `reader` gives for `bytes` (errors as an empty list).
    fn read(reader: &mut UmpPacketReader, bytes: &[u8]) -> Vec<Vec<u32>> {
        reader
            .read(bytes)
            .map(|r| r.map(|u| u.words().to_vec()).unwrap_or_default())
            .collect()
    }

    /// All packets that `writer` gives for `umps`, each as its bytes.
    fn write_all(writer: &mut UmpPacketWriter, umps: &[&[u32]]) -> Vec<Vec<u8>> {
        let mut it = umps.iter().copied().peekable();
        let mut out = Vec::new();
        let mut buf = vec![0u8; writer.packet_size()];
        while it.peek().is_some() || writer.pending() {
            let n = writer.write_packet(&mut it, &mut buf).unwrap();
            assert!(n > 0, "no progress");
            out.push(buf[..n].to_vec());
        }
        out
    }

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    // REQ-USB-UMP-004, REQ-USB-UMP-003: the 3.2.2 Figure 6 vector.
    #[test]
    fn figure6_note_on_is_little_endian_words_in_order() {
        let ump = [0x4190_3C00, 0xFEDC_0000];
        let expected = [0x00, 0x3C, 0x90, 0x41, 0x00, 0x00, 0xDC, 0xFE];
        let mut w = UmpPacketWriter::new(64).unwrap();
        let mut buf = [0u8; 64];
        let mut p = w.packet(&mut buf).unwrap();
        assert_eq!(p.push(&ump), Ok(2));
        let n = p.finish();
        assert_eq!(&buf[..n], &expected);

        let mut r = UmpPacketReader::default();
        let got: Vec<_> = r.read(&expected).collect();
        assert_eq!(got.len(), 1);
        let u = got[0].unwrap();
        assert_eq!(u.words(), &ump);
        assert_eq!(u.message_type(), MessageType::Midi2ChannelVoice);
    }

    // REQ-USB-UMP-003
    #[test]
    fn word_bytes_are_least_significant_first() {
        assert_eq!(word_to_bytes(0x1122_3344), [0x44, 0x33, 0x22, 0x11]);
        assert_eq!(word_from_bytes([0x44, 0x33, 0x22, 0x11]), 0x1122_3344);
    }

    // REQ-USB-UMP-003, REQ-USB-UMP-002
    #[test]
    fn prop_word_round_trip() {
        proptest!(|(w in any::<u32>())| {
            prop_assert_eq!(word_from_bytes(word_to_bytes(w)), w);
        });
    }

    // REQ-USB-UMP-002: every Message Type, reserved ones included, every bit kept.
    #[test]
    fn prop_any_ump_stream_round_trip_unchanged() {
        proptest!(|(raw in proptest::collection::vec(any::<u32>(), 0..40), mps in 1u16..20)| {
            // Build whole UMPs from random words: word 0 gives the size.
            let mut umps: Vec<Vec<u32>> = Vec::new();
            let mut i = 0;
            while i < raw.len() {
                let n = ump_words(raw[i]);
                if i + n > raw.len() { break; }
                umps.push(raw[i..i + n].to_vec());
                i += n;
            }
            let refs: Vec<&[u32]> = umps.iter().map(|u| u.as_slice()).collect();
            let mut w = UmpPacketWriter::new(mps * 4).unwrap();
            let packets = write_all(&mut w, &refs);
            let mut r = UmpPacketReader::new(EndpointDirection::MidiOut, GroupMask::EMPTY);
            let mut got = Vec::new();
            for p in &packets {
                prop_assert!(p.len() <= (mps * 4) as usize);
                got.extend(read(&mut r, p));
            }
            prop_assert_eq!(got, umps);
            prop_assert_eq!(r.held(), 0);
        });
    }

    // REQ-USB-UMP-002: a reserved Message Type (0xB, 96 bits) is moved, not refused.
    #[test]
    fn reserved_message_type_is_moved() {
        let ump = [0xB123_4567, 0x89AB_CDEF, 0x0102_0304];
        let mut r = UmpPacketReader::default();
        let got = read(&mut r, &le(&ump));
        assert_eq!(got, vec![ump.to_vec()]);
    }

    // REQ-USB-UMP-005: several UMPs of all sizes in one packet.
    #[test]
    fn many_umps_in_one_packet() {
        let umps: [&[u32]; 4] = [
            &[0x2090_3C40],
            &[0x4190_3C00, 0xFEDC_0000],
            &[0xB000_0001, 2, 3],
            &[0xF000_0000, 1, 2, 3],
        ];
        let bytes: Vec<u8> = umps.iter().flat_map(|u| le(u)).collect();
        let mut r = UmpPacketReader::default();
        let got = read(&mut r, &bytes);
        let want: Vec<Vec<u32>> = umps.iter().map(|u| u.to_vec()).collect();
        assert_eq!(got, want);
    }

    // REQ-USB-UMP-005, REQ-USB-UMP-006: a 128-bit UMP over 4 packets of 4 bytes; no part given.
    #[test]
    fn ump_cut_by_packets_is_held_and_given_whole() {
        let ump = [0x5012_3456, 0x789A_BCDE, 0xF012_3456, 0x789A_BCDE];
        let bytes = le(&ump);
        let mut r = UmpPacketReader::default();
        for (i, c) in bytes.chunks(4).enumerate() {
            let got = read(&mut r, c);
            if i < 3 {
                assert!(got.is_empty());
                assert_eq!(r.held(), i + 1);
            } else {
                assert_eq!(got, vec![ump.to_vec()]);
                assert_eq!(r.held(), 0);
            }
        }
    }

    // REQ-USB-UMP-006: a held part is given with the next packet that also holds other UMPs.
    #[test]
    fn held_part_completes_in_next_packet() {
        let a = [0x4190_3C00, 0xFEDC_0000];
        let b = [0x2190_3C40];
        let mut r = UmpPacketReader::default();
        assert!(read(&mut r, &le(&a[..1])).is_empty());
        let mut p2 = le(&a[1..]);
        p2.extend(le(&b));
        assert_eq!(read(&mut r, &p2), vec![a.to_vec(), b.to_vec()]);
    }

    // REQ-USB-UMP-007: tail bytes ignored, reported once, held part kept.
    #[test]
    fn tail_bytes_reported_and_held_part_kept() {
        let ump = [0x4190_3C00, 0xFEDC_0000];
        let mut r = UmpPacketReader::default();
        let mut p1 = le(&ump[..1]);
        p1.extend([0xAA, 0xBB, 0xCC]);
        let got: Vec<_> = r.read(&p1).collect();
        assert_eq!(got, vec![Err(UmpUsbError::Tail { len: 3 })]);
        assert_eq!(r.held(), 1);
        let got: Vec<_> = r.read(&le(&ump[1..])).collect();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].unwrap().words(), &ump);
        // A packet of 1 byte only.
        let got: Vec<_> = r.read(&[0x00]).collect();
        assert_eq!(got, vec![Err(UmpUsbError::Tail { len: 1 })]);
    }

    // REQ-USB-UMP-007, REQ-USB-UMP-005: UMPs before the tail are given first.
    #[test]
    fn tail_error_comes_after_whole_umps() {
        let mut bytes = le(&[0x2090_3C40]);
        bytes.extend([1, 2]);
        let mut r = UmpPacketReader::default();
        let got: Vec<_> = r.read(&bytes).collect();
        assert_eq!(got.len(), 2);
        assert!(got[0].is_ok());
        assert_eq!(got[1], Err(UmpUsbError::Tail { len: 2 }));
    }

    // REQ-USB-UMP-005, U-24: the callback form gives the same items.
    #[test]
    fn read_with_gives_the_same_items_as_read() {
        let mut bytes = le(&[0x2090_3C40, 0x4190_3C00, 0xFEDC_0000, 0x5000_0000]);
        bytes.push(7);
        let mut a = UmpPacketReader::default();
        let mut b = UmpPacketReader::default();
        let x: Vec<_> = a.read(&bytes).collect();
        let mut y = Vec::new();
        b.read_with(&bytes, |i| y.push(i));
        assert_eq!(x, y);
        assert_eq!(a, b);
    }

    // REQ-USB-UMP-008 (method for Link): reset discards a held part.
    #[test]
    fn reset_discards_held_part() {
        let mut r = UmpPacketReader::default();
        assert!(read(&mut r, &le(&[0x4190_3C00])).is_empty());
        r.reset();
        assert_eq!(r.held(), 0);
        assert_eq!(read(&mut r, &le(&[0x2090_3C40])), vec![vec![0x2090_3C40]]);

        let mut w = UmpPacketWriter::new(4).unwrap();
        let mut buf = [0u8; 4];
        let mut p = w.packet(&mut buf).unwrap();
        p.push(&[0x4190_3C00, 1]).unwrap();
        p.finish();
        assert!(w.pending());
        w.reset();
        assert!(!w.pending());
        assert_eq!(w.packet(&mut buf).unwrap().len(), 0);
    }

    // REQ-USB-UMP-009: only whole UMPs; a UMP that does not fit starts the next packet.
    #[test]
    fn ump_that_does_not_fit_starts_next_packet() {
        let a: &[u32] = &[0x2090_3C40];
        let b: &[u32] = &[0x4190_3C00, 0xFEDC_0000];
        let mut w = UmpPacketWriter::new(8).unwrap();
        let mut buf = [0u8; 8];
        let mut p = w.packet(&mut buf).unwrap();
        assert_eq!(p.push(a), Ok(1));
        assert_eq!(p.free(), 4);
        assert_eq!(p.push(b), Err(UmpUsbError::PacketFull));
        assert_eq!(p.finish(), 4);
        assert_eq!(&buf[..4], &le(a)[..]);
        let mut p = w.packet(&mut buf).unwrap();
        assert_eq!(p.push(b), Ok(2));
        assert_eq!(p.finish(), 8);
        assert_eq!(&buf[..], &le(b)[..]);
    }

    // REQ-USB-UMP-009: write_packet keeps the UMP that does not fit in the iterator.
    #[test]
    fn write_packet_keeps_ump_that_does_not_fit() {
        let umps: [&[u32]; 3] = [&[0x2090_3C40], &[0x4190_3C00, 2], &[0x2090_3C41]];
        let mut w = UmpPacketWriter::new(8).unwrap();
        let packets = write_all(&mut w, &umps);
        assert_eq!(
            packets,
            vec![
                le(&[0x2090_3C40]),
                le(&[0x4190_3C00, 2]),
                le(&[0x2090_3C41])
            ]
        );
    }

    // REQ-USB-UMP-010: a 128-bit UMP over packets of 8 bytes, in word order, no UMP between.
    #[test]
    fn long_ump_is_split_at_word_boundaries() {
        let long: &[u32] = &[0xF000_0001, 2, 3, 4];
        let next: &[u32] = &[0x2090_3C40];
        let mut w = UmpPacketWriter::new(8).unwrap();
        let mut buf = [0u8; 8];
        let mut p = w.packet(&mut buf).unwrap();
        assert_eq!(p.push(long), Ok(4));
        assert_eq!(p.push(next), Err(UmpUsbError::PacketFull));
        assert_eq!(p.finish(), 8);
        assert_eq!(&buf[..], &le(&long[..2])[..]);
        assert!(w.pending());
        let mut p = w.packet(&mut buf).unwrap();
        assert_eq!(p.len(), 8);
        assert_eq!(p.push(next), Err(UmpUsbError::PacketFull));
        assert_eq!(p.finish(), 8);
        assert_eq!(&buf[..], &le(&long[2..])[..]);
        assert!(!w.pending());

        // With packets of 4 bytes and a UMP that ends mid-packet.
        let mut w = UmpPacketWriter::new(12).unwrap();
        let packets = write_all(&mut w, &[long, next]);
        assert_eq!(
            packets,
            vec![le(&long[..3]), {
                let mut v = le(&long[3..]);
                v.extend(le(next));
                v
            }]
        );
    }

    // REQ-USB-UMP-010: a split UMP is read back whole.
    #[test]
    fn split_ump_reads_back_whole() {
        let long: &[u32] = &[0x5000_0001, 2, 3, 4];
        let mut w = UmpPacketWriter::new(4).unwrap();
        let packets = write_all(&mut w, &[long]);
        assert_eq!(packets.len(), 4);
        let mut r = UmpPacketReader::default();
        let got: Vec<_> = packets.iter().flat_map(|p| read(&mut r, p)).collect();
        assert_eq!(got, vec![long.to_vec()]);
    }

    // REQ-USB-UMP-012: all Groups in one byte sequence, in caller order.
    #[test]
    fn writer_keeps_caller_order_across_groups() {
        let umps: Vec<Vec<u32>> = (0..16u32)
            .rev()
            .map(|g| vec![0x2090_3C40 | (g << 24)])
            .collect();
        let refs: Vec<&[u32]> = umps.iter().map(|u| u.as_slice()).collect();
        let mut w = UmpPacketWriter::new(64).unwrap();
        let packets = write_all(&mut w, &refs);
        assert_eq!(packets.len(), 1);
        let want: Vec<u8> = umps.iter().flat_map(|u| le(u)).collect();
        assert_eq!(packets[0], want);
    }

    // REQ-USB-UMP-012: the held part of one endpoint reader does not change another.
    #[test]
    fn endpoint_readers_are_independent() {
        let mut a = UmpPacketReader::default();
        let mut b = UmpPacketReader::default();
        assert!(read(&mut a, &le(&[0x4190_3C00])).is_empty());
        assert_eq!(read(&mut b, &le(&[0x2090_3C40])), vec![vec![0x2090_3C40]]);
        assert_eq!(b.held(), 0);
        assert_eq!(a.held(), 1);
        assert_eq!(read(&mut a, &le(&[5])), vec![vec![0x4190_3C00, 5]]);
    }

    // REQ-USB-UMP-014: Group Terminal Number = Group; IN for MIDI OUT, OUT for MIDI IN.
    #[test]
    fn received_ump_gives_group_terminal_and_direction() {
        let bytes = le(&[0x2590_3C40]);
        let mut r = UmpPacketReader::new(EndpointDirection::MidiOut, GroupMask::ALL);
        let u = r.read(&bytes).next().unwrap().unwrap();
        assert_eq!(
            u.target(),
            UmpTarget::Terminal(GroupTerminal {
                number: 5,
                direction: TerminalDirection::In
            })
        );
        assert_eq!(u.group_terminal_number(), Some(5));
        assert_eq!(u.direction(), TerminalDirection::In);

        let mut r = UmpPacketReader::new(EndpointDirection::MidiIn, GroupMask::ALL);
        let u = r.read(&bytes).next().unwrap().unwrap();
        assert_eq!(
            u.group_terminal().unwrap().direction,
            TerminalDirection::Out
        );
        assert_eq!(u.direction(), TerminalDirection::Out);
        assert!(u.ump().is_ok());
    }

    // REQ-USB-UMP-015: MT 0x0 and 0xF go to the whole stream, never dropped.
    #[test]
    fn utility_and_stream_messages_have_no_group_terminal() {
        // Bits 27..24 = 0x7, not in the mask: still given, as Stream.
        let words = [0x0710_0000, 0xF7FF_0000, 0, 0, 0];
        let mut r = UmpPacketReader::new(EndpointDirection::MidiOut, GroupMask::from_bits(1));
        let got: Vec<_> = r.read(&le(&words)).map(|x| x.unwrap()).collect();
        assert_eq!(got.len(), 2);
        for u in &got {
            assert_eq!(u.target(), UmpTarget::Stream);
            assert_eq!(u.group_terminal(), None);
            assert_eq!(u.group_terminal_number(), None);
        }
        assert_eq!(got[1].words(), &words[1..]);
    }

    // REQ-USB-UMP-016: a Group with no Group Terminal is given with the mark, not dropped;
    // the writer writes it unchanged.
    #[test]
    fn group_with_no_terminal_is_marked_not_dropped() {
        let ump: &[u32] = &[0x4390_3C00, 0xFEDC_0000];
        let mut r = UmpPacketReader::new(EndpointDirection::MidiOut, GroupMask::from_bits(1));
        let u = r.read(&le(ump)).next().unwrap().unwrap();
        assert_eq!(u.target(), UmpTarget::NoGroupTerminal(Group::from_bits(3)));
        assert_eq!(u.group_terminal(), None);
        assert_eq!(u.group_terminal_number(), Some(3));
        assert_eq!(u.words(), ump);

        let mut w = UmpPacketWriter::new(64).unwrap();
        assert_eq!(write_all(&mut w, &[ump]), vec![le(ump)]);
    }

    // REQ-USB-UMP-009: writer input checks; nothing written on error.
    #[test]
    fn writer_errors_write_nothing() {
        assert_eq!(UmpPacketWriter::new(3), Err(UmpUsbError::InvalidPacketSize));
        assert_eq!(UmpPacketWriter::new(10).unwrap().packet_size(), 8);
        let mut w = UmpPacketWriter::new(8).unwrap();
        let mut small = [0u8; 4];
        assert!(matches!(w.packet(&mut small), Err(UmpUsbError::Overflow)));
        let mut buf = [0u8; 8];
        let mut p = w.packet(&mut buf).unwrap();
        assert_eq!(
            p.push(&[0x4190_3C00]),
            Err(UmpUsbError::ShortUmp { needed: 2, got: 1 })
        );
        assert_eq!(p.push(&[]), Ok(0));
        assert!(p.is_empty());
        let note = Ump::parse(&[0x2090_3C40]).unwrap().0;
        assert_eq!(p.push_ump(&note), Ok(1));
        assert_eq!(p.finish(), 4);
    }

    // REQ-USB-UMP-009: write_packet reports a short UMP and takes it; nothing is dropped
    // silently and the output does not stop.
    #[test]
    fn write_packet_reports_short_ump() {
        let note: &[u32] = &[0x2090_3C40];
        let short: &[u32] = &[0x4190_3C00];
        let umps = [note, short, note];
        let mut it = umps.iter().copied().peekable();
        let mut w = UmpPacketWriter::new(64).unwrap();
        let mut buf = [0xAAu8; 64];
        // The packet ends before the short UMP.
        assert_eq!(w.write_packet(&mut it, &mut buf), Ok(4));
        // The next call reports it, writes nothing and takes it.
        let mut buf = [0xAAu8; 64];
        assert_eq!(
            w.write_packet(&mut it, &mut buf),
            Err(UmpUsbError::ShortUmp { needed: 2, got: 1 })
        );
        assert_eq!(buf, [0xAA; 64]);
        assert_eq!(w.write_packet(&mut it, &mut buf), Ok(4));
        assert!(it.peek().is_none());
    }

    // REQ-USB-UMP-010: the rest of a split UMP is committed only by finish(); a packet that is
    // not sent changes nothing.
    #[test]
    fn unsent_packet_does_not_commit_the_rest() {
        let long: &[u32] = &[0x5000_0001, 2, 3, 4];
        let mut w = UmpPacketWriter::new(8).unwrap();
        let mut buf = [0u8; 8];
        // A split in a packet that is dropped: nothing held.
        let mut p = w.packet(&mut buf).unwrap();
        assert_eq!(p.push(long), Ok(4));
        drop(p);
        assert!(!w.pending());
        // The same split, sent.
        let mut p = w.packet(&mut buf).unwrap();
        assert_eq!(p.push(long), Ok(4));
        assert_eq!(p.finish(), 8);
        assert!(w.pending());
        // A packet with the rest that is not sent: the rest stays held.
        let p = w.packet(&mut buf).unwrap();
        assert_eq!(p.len(), 8);
        drop(p);
        assert!(w.pending());
        let p = w.packet(&mut buf).unwrap();
        assert_eq!(p.finish(), 8);
        assert_eq!(&buf[..], &le(&long[2..])[..]);
        assert!(!w.pending());
    }

    // REQ-USB-UMP-006: a drop of the iterator after the first UMP reads the rest of the
    // packet, so a cut UMP is completed with the right words (review M1, case 3).
    #[test]
    fn dropped_reader_keeps_framing() {
        let a = 0x2090_3C40;
        let b = [0x4090_3C00, 0x8000_0000];
        let c = 0x2080_3C00;
        let d = 0x2090_3C40;
        let mut r = UmpPacketReader::default();
        let first: Vec<_> = r
            .read(&le(&[a, b[0]]))
            .take(1)
            .map(|x| x.unwrap().words().to_vec())
            .collect();
        assert_eq!(first, vec![vec![a]]);
        assert_eq!(r.held(), 1);
        assert_eq!(
            read(&mut r, &le(&[b[1], c, d])),
            vec![b.to_vec(), vec![c], vec![d]]
        );
    }

    // REQ-USB-UMP-005, REQ-USB-UMP-006, U-24: never panics; the reader holds at most 3 words.
    #[test]
    fn prop_reader_never_panics() {
        proptest!(|(packets in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..70), 0..6))| {
            let mut r = UmpPacketReader::new(EndpointDirection::MidiIn, GroupMask::from_bits(0x00F0));
            for p in &packets {
                for u in r.read(p).flatten() {
                    prop_assert_eq!(u.words().len(), ump_words(u.words()[0]));
                }
                prop_assert!(r.held() <= MAX_HELD_WORDS);
            }
        });
    }

    // REQ-USB-UMP-005: any cut of a byte stream into packets gives the same UMPs.
    #[test]
    fn prop_packet_cuts_do_not_change_umps() {
        proptest!(|(raw in proptest::collection::vec(any::<u32>(), 0..30), cut in 1usize..9)| {
            let bytes = le(&raw);
            let mut whole = UmpPacketReader::default();
            let a = read(&mut whole, &bytes);
            let mut parts = UmpPacketReader::default();
            let b: Vec<_> = bytes.chunks(cut * 4).flat_map(|c| read(&mut parts, c)).collect();
            prop_assert_eq!(a, b);
            prop_assert_eq!(whole, parts);
        });
    }
}
