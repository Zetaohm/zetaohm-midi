//! MIDI 1.0 byte stream codec and translation to and from UMP (M1 v4.2.1; M2-104-UM Appendix D).
//!
//! - Byte format (M1 Data Format, Tables I, V, VI): a status byte is 80H to FFH, a data byte
//!   00H to 7FH. 8nH, 9nH, AnH, BnH, EnH and F2H have 2 data bytes; CnH, DnH, F1H and F3H have
//!   1; F6H, F7H and F8H to FFH have none. F0H starts System Exclusive.
//! - System Real Time bytes may come at any position, even inside another message or a System
//!   Exclusive message, and do not change the running status (M1 Message Types, page 30).
//! - Running status (M1 page 5, Appendix A-1, A-2): a data byte as the first byte of a message
//!   uses the last Channel Voice or Mode status. System Exclusive and System Common clear it.
//!   F4H and F5H are ignored and clear it; F9H and FDH are ignored and keep it.
//! - System Exclusive ends at F7H or at any other status that is not System Real Time (M1 EOX).
//!   To UMP, the data goes into SysEx7 packets without F0H and F7H (D.3.7); from UMP, the writer
//!   adds F0H and F7H (D.2.7).
//! - Utility, System Exclusive 8 and Mixed Data Set have no MIDI 1.0 byte form (D.2.9).
//! - MIDI 2.0 Channel Voice goes through the zmidi-ump translators (6.2, D.2, D.3).
//! - [`Midi1Scheduler`] writes each message at the render time from
//!   [`crate::JrReceiver`] (4.1.1).
//!
//! The byte stream has no Group: the reader gives its UMPs on one Group, and the writer ignores
//! the Group (the caller routes by Group, see [`crate::route`]). No alloc (DEC-002), no I/O
//! (DEC-003).

use crate::Disposition;
#[cfg(feature = "midi2-protocol")]
use zmidi_ump::StreamProtocol;
use zmidi_ump::{midi2_to_midi1, Group, MessageType, Midi1ToMidi2Group, Packet};

/// Error of the byte stream codec (thiserror 2, no_std).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ByteStreamError {
    /// The output slice cannot hold the bytes of the UMP; nothing was written.
    #[error("output buffer too small")]
    BufferTooSmall,
    /// The scheduler queue has no free slot.
    #[error("scheduler queue full")]
    QueueFull,
}

/// System Exclusive start (F0H) and EOX (F7H).
const SOX: u8 = 0xF0;
/// SysEx7 Status values (M2-104 7.7, Table 13).
const SYSEX_COMPLETE: u8 = 0;
const SYSEX_START: u8 = 1;
const SYSEX_CONTINUE: u8 = 2;
const SYSEX_END: u8 = 3;
const EOX: u8 = 0xF7;

/// Number of data bytes after a status byte that is not System Exclusive (Tables I and V).
fn data_len(status: u8) -> usize {
    match status {
        0x80..=0xBF | 0xE0..=0xEF | 0xF2 => 2,
        0xC0..=0xDF | 0xF1 | 0xF3 => 1,
        _ => 0,
    }
}

/// The undefined status bytes (Tables V and VI).
fn undefined(status: u8) -> bool {
    matches!(status, 0xF4 | 0xF5 | 0xF9 | 0xFD)
}

/// UMPs given by one [`BytesToUmp::push`]: at most two [`Packet`] values, in order (DEC-018).
/// [`Packet::ump`] gives the typed message. It keeps only the words (16 bytes, OPT-021, OPT-030)
/// and makes each [`Packet`] when the iterator gives it. No alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pushed {
    /// The words of the UMPs: the first in words 0 and 1, the second in word 2. One byte
    /// completes at most a SysEx7 End or Complete (2 words) and then a Tune Request (1 word) for
    /// 0xF6, or one UMP of 1 or 2 words ([`BytesToUmpCore::feed`]), so the second UMP is
    /// always 1 word.
    words: [u32; 3],
    /// The word count of each UMP.
    lens: [u8; 2],
    /// Number of UMPs.
    len: u8,
    /// Next UMP to give.
    next: u8,
}

impl Pushed {
    const EMPTY: Self = Self {
        words: [0; 3],
        lens: [0; 2],
        len: 0,
        next: 0,
    };

    /// The words of UMP `i` (0 or 1) with `n` words.
    fn slot(i: usize, n: usize) -> core::ops::Range<usize> {
        let at = 2 * i;
        at..at + n
    }

    fn add(&mut self, w: &[u32]) {
        let i = usize::from(self.len);
        let k = w.len().min(2);
        if let (Some(slot), Some(n)) = (self.words.get_mut(Self::slot(i, k)), self.lens.get_mut(i))
        {
            slot.copy_from_slice(&w[..k]);
            *n = k as u8;
            self.len += 1;
        }
    }
}

impl Iterator for Pushed {
    type Item = Packet;

    fn next(&mut self) -> Option<Packet> {
        let i = usize::from(self.next);
        if i >= usize::from(self.len) {
            return None;
        }
        self.next += 1;
        let words = self.words.get(Self::slot(i, usize::from(self.lens[i])))?;
        Packet::from_words(words).ok()
    }
}

/// MIDI 1.0 byte stream reader that gives UMPs on one Group. Fixed size; no alloc.
///
/// It is a [`BytesToUmpCore`] and, for MIDI 2.0 Protocol output, its own translator.
#[derive(Debug, Clone)]
pub struct BytesToUmp {
    core: BytesToUmpCore,
    /// Translator for MIDI 2.0 Protocol output (one Group: 128 bytes, OPT-001, OPT-011); `None` gives
    /// MIDI 1.0 Protocol UMPs. Only with the feature `midi2-protocol` (DEC-017).
    #[cfg(feature = "midi2-protocol")]
    midi2: Option<Midi1ToMidi2Group>,
}

impl BytesToUmp {
    /// Creates a reader for `group`. [`StreamProtocol::Midi1`] gives Message Type 0x2;
    /// [`StreamProtocol::Midi2`] gives Message Type 0x4 through [`Midi1ToMidi2Group`] (D.3).
    /// Needs the feature `midi2-protocol`; without it, use [`Self::midi1`].
    #[cfg(feature = "midi2-protocol")]
    pub fn new(group: Group, protocol: StreamProtocol) -> Self {
        Self {
            core: BytesToUmpCore::new(group),
            midi2: matches!(protocol, StreamProtocol::Midi2).then(Midi1ToMidi2Group::new),
        }
    }

    /// Creates a reader for `group` that gives MIDI 1.0 Protocol UMPs (Message Type 0x2). With
    /// the feature `midi2-protocol` off, the reader holds no translator and links none (DEC-017).
    pub const fn midi1(group: Group) -> Self {
        Self {
            core: BytesToUmpCore::new(group),
            #[cfg(feature = "midi2-protocol")]
            midi2: None,
        }
    }

    /// Takes one received byte and gives the UMPs it completes. Never panics.
    pub fn push(&mut self, byte: u8) -> Pushed {
        #[cfg(feature = "midi2-protocol")]
        return self.core.push_with(byte, self.midi2.as_mut());
        #[cfg(not(feature = "midi2-protocol"))]
        return self.core.push_with(byte, None);
    }

    /// Takes one received byte and gives the words of each UMP that it completes to `f`, in
    /// order, at most two: the same UMPs as [`Self::push`]. The fast path: it moves no
    /// [`Pushed`] value for each byte (OPT-036). Never panics.
    pub fn feed(&mut self, byte: u8, f: impl FnMut(&[u32])) {
        #[cfg(feature = "midi2-protocol")]
        self.core.feed_with(byte, self.midi2.as_mut(), f);
        #[cfg(not(feature = "midi2-protocol"))]
        self.core.feed(byte, f);
    }
}

/// The MIDI 1.0 byte stream reader of [`BytesToUmp`] with no translator: 24 bytes on a 32-bit
/// target (OPT-011). [`Self::push_with`] takes the translator of the caller for MIDI 2.0
/// Protocol output, so that a reader of several streams (for example the cables of a USB MIDI
/// endpoint) keeps translators only when it gives MIDI 2.0 Protocol UMPs.
#[derive(Debug, Clone)]
pub struct BytesToUmpCore {
    /// The Group of the UMPs.
    group: Group,
    /// Running status: the last Channel Voice or Mode status, or 0 for none.
    running: u8,
    /// Status of the message in progress, or 0 for none.
    status: u8,
    /// Data bytes of the message in progress.
    data: [u8; 2],
    /// Number of valid bytes in `data`.
    count: usize,
    /// True inside a System Exclusive message.
    in_sysex: bool,
    /// System Exclusive data not yet given.
    chunk: [u8; 6],
    /// Number of valid bytes in `chunk`.
    chunk_len: usize,
    /// True when a SysEx7 Start was given for the message in progress.
    started: bool,
}

impl BytesToUmpCore {
    /// Creates a reader for `group`.
    pub const fn new(group: Group) -> Self {
        Self {
            group,
            running: 0,
            status: 0,
            data: [0; 2],
            count: 0,
            in_sysex: false,
            chunk: [0; 6],
            chunk_len: 0,
            started: false,
        }
    }

    /// Takes one received byte and gives the words of each UMP that it completes to `f`, in
    /// order, at most two: MIDI 1.0 Protocol (Message Type 0x2 for Channel Voice). The core of
    /// the reader: it moves no result value, so a build that uses it links no copy code
    /// (OPT-021, REQ-BS-017). Never panics.
    pub fn feed(&mut self, byte: u8, mut f: impl FnMut(&[u32])) {
        let group = self.group;
        let one = |f: &mut dyn FnMut(&[u32]), mt: u32, status: u8, d0: u8, d1: u8| {
            f(&[word(mt, group, status, d0, d1)]);
        };
        match byte {
            // System Real Time: its own message; nothing else changes. F9H and FDH are ignored.
            0xF8..=0xFF => {
                if !undefined(byte) {
                    one(&mut f, 0x1, byte, 0, 0);
                }
            }
            0x80..=0xF7 => {
                // Any status that is not Real Time ends a System Exclusive message (M1 EOX).
                if self.in_sysex {
                    let status = if self.started {
                        SYSEX_END
                    } else {
                        SYSEX_COMPLETE
                    };
                    self.sysex(status, &mut f);
                    self.in_sysex = false;
                }
                // A new status discards an incomplete message (M1 Status Bytes).
                self.status = 0;
                self.count = 0;
                match byte {
                    0x80..=0xEF => {
                        self.running = byte;
                        self.status = byte;
                    }
                    SOX => {
                        self.running = 0;
                        self.in_sysex = true;
                        self.chunk_len = 0;
                        self.started = false;
                    }
                    0xF6 => {
                        self.running = 0;
                        one(&mut f, 0x1, byte, 0, 0);
                    }
                    // F1H to F3H wait for data; F4H, F5H and a lone F7H are ignored. All clear
                    // the running status (Appendix A-2).
                    _ => {
                        self.running = 0;
                        if data_len(byte) > 0 {
                            self.status = byte;
                        }
                    }
                }
            }
            _ => {
                if self.in_sysex {
                    if self.chunk_len == self.chunk.len() {
                        let status = if self.started {
                            SYSEX_CONTINUE
                        } else {
                            SYSEX_START
                        };
                        self.sysex(status, &mut f);
                        self.started = true;
                    }
                    if let Some(slot) = self.chunk.get_mut(self.chunk_len) {
                        *slot = byte;
                        self.chunk_len += 1;
                    }
                    return;
                }
                if self.status == 0 {
                    // Running status; data with no status is ignored (M1 Data Bytes).
                    if self.running == 0 {
                        return;
                    }
                    self.status = self.running;
                    self.count = 0;
                }
                if let Some(slot) = self.data.get_mut(self.count) {
                    *slot = byte;
                }
                self.count += 1;
                if self.count == data_len(self.status) {
                    let (status, d0, d1) = (self.status, self.data[0], self.data[1]);
                    let d1 = if self.count == 2 { d1 } else { 0 };
                    self.status = 0;
                    self.count = 0;
                    let mt = if status < 0xF0 { 0x2 } else { 0x1 };
                    one(&mut f, mt, status, d0, d1);
                }
            }
        }
    }

    /// [`Self::feed`] with an optional translator: with `midi2`, each Channel Voice message goes
    /// through it and gives Message Type 0x4 (D.3).
    pub fn feed_with(
        &mut self,
        byte: u8,
        midi2: Option<&mut Midi1ToMidi2Group>,
        mut f: impl FnMut(&[u32]),
    ) {
        match midi2 {
            None => self.feed(byte, f),
            Some(t) => self.feed(byte, |w| {
                if w[0] >> 28 == 0x2 {
                    if let Some(words) = t.translate_words(w[0]) {
                        f(&words);
                    }
                } else {
                    f(w);
                }
            }),
        }
    }

    /// Takes one received byte and gives the UMPs it completes; see [`Self::feed_with`].
    #[inline(never)]
    pub fn push_with(&mut self, byte: u8, midi2: Option<&mut Midi1ToMidi2Group>) -> Pushed {
        let mut out = Pushed::EMPTY;
        self.feed_with(byte, midi2, |w| out.add(w));
        out
    }

    /// Gives the SysEx7 UMP of the pending data to `f` and empties the chunk (D.3.7, Figure 41).
    /// `status` is 0 Complete, 1 Start, 2 Continue or 3 End (Table 13).
    fn sysex(&mut self, status: u8, f: &mut dyn FnMut(&[u32])) {
        let n = self.chunk_len.min(self.chunk.len());
        let chunk = &self.chunk;
        let byte = |i: usize| if i < n { u32::from(chunk[i]) } else { 0 };
        let w0 = 0x3 << 28
            | u32::from(self.group.into_bits()) << 24
            | u32::from(status) << 20
            | (n as u32) << 16
            | byte(0) << 8
            | byte(1);
        let w1 = byte(2) << 24 | byte(3) << 16 | byte(4) << 8 | byte(5);
        self.chunk_len = 0;
        f(&[w0, w1]);
    }
}

/// Word 0 of a 32-bit UMP with a status byte and two data bytes.
fn word(message_type: u32, group: Group, status: u8, d0: u8, d1: u8) -> u32 {
    message_type << 28
        | u32::from(group.into_bits()) << 24
        | u32::from(status) << 16
        | u32::from(d0) << 8
        | u32::from(d1)
}

/// UMP to MIDI 1.0 byte stream writer. Fixed size; no alloc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UmpToBytes {
    /// True when the writer omits repeated Channel Voice and Mode status bytes.
    running_status: bool,
    /// The last Channel Voice or Mode status written, or 0 when the next one must be written.
    last_status: u8,
}

impl UmpToBytes {
    /// Creates a writer, with or without running status (M1 Running Status).
    pub const fn new(running_status: bool) -> Self {
        Self {
            running_status,
            last_status: 0,
        }
    }

    /// The next Channel Voice or Mode message writes its status byte (Appendix A-2: refresh the
    /// status every few seconds).
    pub fn refresh_status(&mut self) {
        self.last_status = 0;
    }

    /// Gives the MIDI 1.0 bytes of one UMP to `f`, in order, and keeps the running status. A UMP
    /// with no MIDI 1.0 byte form (D.2.8, D.2.9), or `words` shorter than the Table 4 size, gives
    /// none. The core of the writer: no buffer and no result value, so a build that uses it links
    /// no copy code (OPT-021, REQ-BS-017). Reserved bits are not used (M2-104 2.1.3). Never
    /// panics.
    pub fn write_each(&mut self, words: &[u32], mut f: impl FnMut(u8)) {
        let Some(&w0) = words.first() else {
            return;
        };
        if words.len() < MessageType::from_bits((w0 >> 28) as u8).word_count() {
            return;
        }
        let status = (w0 >> 16) as u8;
        let mut last = self.last_status;
        let running = self.running_status;
        // The bytes of one MIDI 1.0 Channel Voice word (Message Type 0x2).
        let channel = |w: u32, last: &mut u8, f: &mut dyn FnMut(u8)| {
            let status = (w >> 16) as u8;
            if !(running && *last == status) {
                f(status);
            }
            *last = status;
            f((w >> 8) as u8 & 0x7F);
            if data_len(status) == 2 {
                f(w as u8 & 0x7F);
            }
        };
        match w0 >> 28 {
            0x1 if matches!(status, 0xF1..=0xF3 | 0xF6 | 0xF8 | 0xFA..=0xFC | 0xFE | 0xFF) => {
                f(status);
                if status < 0xF8 {
                    // System Common ends running status (M1 Running Status).
                    last = 0;
                    for i in 0..data_len(status) {
                        f((w0 >> (8 - 8 * i)) as u8 & 0x7F);
                    }
                }
            }
            0x2 if (0x80..=0xEF).contains(&status) => channel(w0, &mut last, &mut f),
            0x4 => {
                if let Some(m) = midi2_to_midi1(words) {
                    for &w in m.as_slice() {
                        channel(w, &mut last, &mut f);
                    }
                }
            }
            0x3 if (w0 >> 20) & 0xF <= 3 && (w0 >> 16) & 0xF <= 6 => {
                let sysex = ((w0 >> 20) & 0xF) as u8;
                let n = ((w0 >> 16) & 0xF) as usize;
                if sysex == SYSEX_COMPLETE || sysex == SYSEX_START {
                    f(SOX);
                    last = 0;
                }
                let w1 = words[1];
                for i in 0..n {
                    let b = if i < 2 {
                        w0 >> (8 - 8 * i)
                    } else {
                        w1 >> (24 - 8 * (i - 2))
                    };
                    f(b as u8 & 0x7F);
                }
                if sysex == SYSEX_COMPLETE || sysex == SYSEX_END {
                    f(EOX);
                }
            }
            // Utility, Data 128, Flex Data, UMP Stream and unknown packets (D.2.9).
            _ => {}
        }
        self.last_status = last;
    }

    /// Writes the MIDI 1.0 bytes of one UMP into `out` and gives the count. Gives
    /// [`ByteStreamError::BufferTooSmall`] with no state change when `out` is too small; the
    /// content of `out` is then not defined (accepted by the user on 2026-10-21, OPT-021). A UMP
    /// with no MIDI 1.0 byte form writes nothing (D.2.8, D.2.9). One pass of
    /// [`Self::write_each`] straight into `out`, with no staging buffer. Never panics.
    // One copy of the writer for every caller of the value form (OPT-021).
    #[inline(never)]
    pub fn write(&mut self, words: &[u32], out: &mut [u8]) -> Result<usize, ByteStreamError> {
        // One pass straight into `out`; on overflow the state goes back.
        let before = self.last_status;
        let (mut at, mut over) = (0usize, false);
        // `&mut dyn`: one copy of the writer for this form and other `dyn` callers (OPT-021).
        let f: &mut dyn FnMut(u8) = &mut |b| match out.get_mut(at) {
            Some(slot) => {
                *slot = b;
                at += 1;
            }
            None => over = true,
        };
        self.write_each(words, f);
        if over {
            self.last_status = before;
            return Err(ByteStreamError::BufferTooSmall);
        }
        Ok(at)
    }
}

/// Bytes of one UMP: at most 4 translated messages of 3 bytes (D.2.3), or 8 SysEx bytes.
#[cfg(test)]
#[derive(Debug, Default)]
struct Bytes {
    buf: [u8; 12],
    len: usize,
}

#[cfg(test)]
impl Bytes {
    fn push(&mut self, byte: u8) {
        if let Some(slot) = self.buf.get_mut(self.len) {
            *slot = byte;
            self.len += 1;
        }
    }

    fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

/// Writes UMPs as MIDI 1.0 bytes at their JR render times (4.1.1). A fixed queue of `Q` UMPs
/// in receive order; the render times from [`crate::JrReceiver`] do not decrease.
#[derive(Debug, Clone)]
pub struct Midi1Scheduler<const Q: usize> {
    /// Render time (local µs) and words of each queued UMP, from `head`; `len` entries are in
    /// use. The words are those of the parsed UMP, and the Message Type of the first word gives
    /// their number (M2-104 Table 4): 24 bytes for each entry, not 40 (OPT-002).
    queue: [(u64, [u32; 4]); Q],
    /// Index of the oldest entry.
    head: usize,
    /// Number of queued entries.
    len: usize,
}

impl<const Q: usize> Default for Midi1Scheduler<Q> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const Q: usize> Midi1Scheduler<Q> {
    /// Creates an empty scheduler.
    pub fn new() -> Self {
        Self {
            queue: [(0, [0; 4]); Q],
            head: 0,
            len: 0,
        }
    }

    /// Queues one received UMP with the [`Disposition`] that [`crate::JrReceiver::receive`]
    /// gave for it. `RenderNow` is due at once; `RenderAt(t)` at local time `t`. `Consumed` and
    /// `UnexpectedJrTimestamp` belong to JR messages, which have no MIDI 1.0 bytes, and queue
    /// nothing. Gives [`ByteStreamError::QueueFull`] when no slot is free.
    pub fn push(&mut self, disposition: Disposition, words: &[u32]) -> Result<(), ByteStreamError> {
        let at = match disposition {
            Disposition::RenderNow => 0,
            Disposition::RenderAt(t) => t,
            Disposition::Consumed | Disposition::UnexpectedJrTimestamp => return Ok(()),
        };
        let Ok(packet) = Packet::from_words(words) else {
            return Ok(());
        };
        if self.len == Q {
            return Err(ByteStreamError::QueueFull);
        }
        let mut w = [0u32; 4];
        w[..packet.words().len()].copy_from_slice(packet.words());
        self.queue[(self.head + self.len) % Q] = (at, w);
        self.len += 1;
        Ok(())
    }

    /// Writes, in order, the bytes of every queued UMP that is due at local time `now_us`, and
    /// gives the count. Stops before a UMP whose bytes do not fit; it stays queued.
    pub fn poll(&mut self, now_us: u64, writer: &mut UmpToBytes, out: &mut [u8]) -> usize {
        let mut total = 0;
        while self.len > 0 {
            let (at, words) = self.queue[self.head];
            if at > now_us {
                break;
            }
            let n = MessageType::from_bits((words[0] >> 28) as u8).word_count();
            match writer.write(&words[..n], &mut out[total..]) {
                Ok(count) => total += count,
                Err(_) => break,
            }
            self.head = (self.head + 1) % Q;
            self.len -= 1;
        }
        total
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;
    use zmidi_ump::Ump;

    fn g0() -> Group {
        Group::from_bits(0)
    }

    /// The words of a UMP.
    fn words(p: Packet) -> Vec<u32> {
        p.words().to_vec()
    }

    /// Word lists of the UMPs that `bytes` give with a MIDI 1.0 Protocol reader.
    fn read(bytes: &[u8]) -> Vec<Vec<u32>> {
        let mut r = BytesToUmp::midi1(g0());
        bytes
            .iter()
            .flat_map(|&b| r.push(b).collect::<Vec<_>>())
            .map(words)
            .collect()
    }

    fn write_all(w: &mut UmpToBytes, packets: &[&[u32]]) -> Vec<u8> {
        let mut out = Vec::new();
        for p in packets {
            let mut buf = [0u8; 16];
            let n = w.write(p, &mut buf).unwrap();
            out.extend_from_slice(&buf[..n]);
        }
        out
    }

    #[test]
    fn reader_keeps_sysex_and_channel_voice_order() {
        // REQ-CI-IO-101: a MIDI-CI System Exclusive message and Channel Voice messages come out
        // in the order received.
        let out = read(&[
            0x90, 0x3C, 0x27, // Note On
            0xF0, 0x7E, 0x7F, 0x0D, 0x70, 0x02, 0xF7, // MIDI-CI header start
            0x80, 0x3C, 0x00, // Note Off
        ]);
        assert_eq!(
            out,
            [
                [0x2090_3C27u32].to_vec(),
                [0x3005_7E7F, 0x0D70_0200].to_vec(),
                [0x2080_3C00].to_vec()
            ]
        );
    }

    #[test]
    fn one_push_gives_sysex_end_and_tune_request() {
        // REQ-BS-006: 0xF6 ends an open SysEx message (M1 EOX) and is a message itself, so one
        // byte gives two UMPs: the SysEx7 (2 words) and the Tune Request (1 word), in that order
        // (the 3 words of `Pushed`, OPT-030). The MIDI 2.0 Protocol reader gives the same.
        let readers = [
            BytesToUmp::midi1(g0()),
            #[cfg(feature = "midi2-protocol")]
            BytesToUmp::new(g0(), StreamProtocol::Midi2),
        ];
        for reader in readers {
            let mut r = reader.clone();
            for b in [0xF0, 0x01, 0x02] {
                assert_eq!(r.push(b).count(), 0);
            }
            let out: Vec<Vec<u32>> = r.push(0xF6).map(words).collect();
            assert_eq!(out, [std::vec![0x3002_0102, 0], std::vec![0x10F6_0000]]);
            // A full chunk, then 0xF6: SysEx7 End, then the Tune Request.
            let mut r = reader;
            for b in [0xF0, 1, 2, 3, 4, 5, 6, 7] {
                r.push(b).for_each(drop);
            }
            let out: Vec<Vec<u32>> = r.push(0xF6).map(words).collect();
            assert_eq!(out, [std::vec![0x3031_0700, 0], std::vec![0x10F6_0000]]);
        }
    }

    #[test]
    fn reader_decodes_running_status_examples() {
        // Appendix A-1: both streams give the same three Note On messages.
        let full = read(&[0x90, 0x3C, 0x27, 0x90, 0x40, 0x2B, 0x90, 0x43, 0x25]);
        let running = read(&[0x90, 0x3C, 0x27, 0x40, 0x2B, 0x43, 0x25]);
        let expected = [
            [0x2090_3C27u32].to_vec(),
            [0x2090_402B].to_vec(),
            [0x2090_4325].to_vec(),
        ];
        assert_eq!(full, expected);
        assert_eq!(running, expected);
        // Second example: Note Off by Note On with velocity 0.
        let running = read(&[0x90, 0x3C, 0x27, 0x3C, 0x00, 0x3E, 0x29]);
        assert_eq!(
            running,
            [
                [0x2090_3C27u32].to_vec(),
                [0x2090_3C00].to_vec(),
                [0x2090_3E29].to_vec()
            ]
        );
    }

    #[test]
    fn reader_keeps_running_status_of_any_channel_and_mode_messages() {
        // Appendix A-1: Omni Off on Channel 1, then running-status Control Change.
        assert_eq!(
            read(&[0xB0, 0x7C, 0x00, 0x01, 0x37]),
            [[0x20B0_7C00u32].to_vec(), [0x20B0_0137].to_vec()]
        );
        // 1-byte messages with running status.
        assert_eq!(
            read(&[0xC5, 0x01, 0x02]),
            [[0x20C5_0100u32].to_vec(), [0x20C5_0200].to_vec()]
        );
    }

    #[test]
    fn reader_passes_real_time_anywhere() {
        // Inside a message: the clock comes first; the message and running status continue.
        assert_eq!(
            read(&[0x90, 0xF8, 0x3C, 0xFA, 0x27, 0x40, 0xFE, 0x2B]),
            [
                [0x10F8_0000u32].to_vec(),
                [0x10FA_0000].to_vec(),
                [0x2090_3C27].to_vec(),
                [0x10FE_0000].to_vec(),
                [0x2090_402B].to_vec(),
            ]
        );
        // Inside System Exclusive: it does not end the message.
        assert_eq!(
            read(&[0xF0, 0x7E, 0xF8, 0x01, 0xF7]),
            [[0x10F8_0000u32].to_vec(), [0x3002_7E01, 0].to_vec()]
        );
    }

    #[test]
    fn reader_packs_sysex_in_six_byte_umps() {
        // D.3.7: no F0H or F7H in the UMPs; 13 bytes give Start, Continue, End.
        let mut bytes = [0xF0].to_vec();
        bytes.extend(1..=13u8);
        bytes.push(0xF7);
        assert_eq!(
            read(&bytes),
            [
                [0x3016_0102u32, 0x0304_0506].to_vec(),
                [0x3026_0708, 0x090A_0B0C].to_vec(),
                [0x3031_0D00, 0].to_vec(),
            ]
        );
        // Exactly 6 bytes: one Complete UMP. Empty: Complete with no data.
        assert_eq!(
            read(&[0xF0, 1, 2, 3, 4, 5, 6, 0xF7]),
            [[0x3006_0102u32, 0x0304_0506].to_vec()]
        );
        assert_eq!(read(&[0xF0, 0xF7]), [[0x3000_0000u32, 0].to_vec()]);
    }

    #[test]
    fn reader_ends_sysex_at_any_status() {
        // F6H ends the System Exclusive message and is a message itself: two UMPs.
        let mut r = BytesToUmp::midi1(g0());
        for b in [0xF0, 0x7D, 0x01] {
            assert_eq!(r.push(b).count(), 0);
        }
        let out: Vec<_> = r.push(0xF6).map(words).collect();
        assert_eq!(out, [[0x3002_7D01u32, 0].to_vec(), [0x10F6_0000].to_vec()]);
        // A Note On status ends it too, and starts the note.
        assert_eq!(
            read(&[0xF0, 0x7D, 0x90, 0x3C, 0x27]),
            [[0x3001_7D00u32, 0].to_vec(), [0x2090_3C27].to_vec()]
        );
    }

    #[test]
    fn reader_discards_incomplete_message_at_new_status() {
        assert_eq!(
            read(&[0x90, 0x3C, 0xB0, 0x07, 0x64]),
            [[0x20B0_0764u32].to_vec()]
        );
    }

    #[test]
    fn reader_handles_undefined_status() {
        // F4H clears running status: the data after it is ignored.
        assert_eq!(
            read(&[0x90, 0x3C, 0x27, 0xF4, 0x40, 0x2B]),
            [[0x2090_3C27u32].to_vec()]
        );
        // F9H keeps running status.
        assert_eq!(
            read(&[0x90, 0x3C, 0x27, 0xF9, 0x40, 0x2B]),
            [[0x2090_3C27u32].to_vec(), [0x2090_402B].to_vec()]
        );
        // F5H clears it as F4H does; FDH keeps it as F9H does, and gives no message.
        assert_eq!(
            read(&[0x90, 0x3C, 0x27, 0xF5, 0x40, 0x2B]),
            [[0x2090_3C27u32].to_vec()]
        );
        assert_eq!(
            read(&[0x90, 0x3C, 0x27, 0xFD, 0x40, 0x2B]),
            [[0x2090_3C27u32].to_vec(), [0x2090_402B].to_vec()]
        );
    }

    #[test]
    fn reader_and_writer_use_the_group_and_whole_umps() {
        // The Group of the reader is in each UMP: a Note On and a SysEx7 on Group 5.
        let mut r = BytesToUmp::midi1(Group::new(5).unwrap());
        let out: Vec<_> = [0x90, 0x3C, 0x27, 0xF0, 0x01, 0xF7]
            .iter()
            .flat_map(|&b| r.push(b).collect::<Vec<_>>())
            .map(words)
            .collect();
        assert_eq!(out, [[0x2590_3C27u32].to_vec(), [0x3501_0100, 0].to_vec()]);
        // The writer needs the whole UMP: one word of a 64-bit UMP gives no byte.
        let mut w = UmpToBytes::new(false);
        let mut n = 0;
        w.write_each(&[0x4090_3C00], |_| n += 1);
        assert_eq!(n, 0);
    }

    #[test]
    fn reader_data_counts_and_last_status() {
        // REQ-BS-004: 2 data bytes for AnH and EnH, 1 for DnH and F1H.
        assert_eq!(read(&[0xA0, 0x05, 0x06]), [[0x20A0_0506u32].to_vec()]);
        assert_eq!(read(&[0xE0, 0x05, 0x06]), [[0x20E0_0506u32].to_vec()]);
        assert_eq!(
            read(&[0xD0, 0x05, 0x06]),
            [[0x20D0_0500u32].to_vec(), [0x20D0_0600].to_vec()]
        );
        assert_eq!(read(&[0xF1, 0x05]), [[0x10F1_0500u32].to_vec()]);
        // REQ-BS-008: running status is the last status received, also on another channel.
        assert_eq!(
            read(&[0x90, 0x3C, 0x27, 0x91, 0x40, 0x2B, 0x43, 0x25]),
            [
                [0x2090_3C27u32].to_vec(),
                [0x2091_402B].to_vec(),
                [0x2091_4325].to_vec()
            ]
        );
        // REQ-BS-015: a System Exclusive status clears running status.
        assert_eq!(
            read(&[0x90, 0x3C, 0x27, 0xF0, 0x01, 0xF7, 0x40, 0x2B]),
            [[0x2090_3C27u32].to_vec(), [0x3001_0100, 0].to_vec()]
        );
    }

    #[test]
    fn reader_gives_only_complete_messages() {
        let mut r = BytesToUmp::midi1(g0());
        assert_eq!(r.push(0xF2).count(), 0);
        assert_eq!(r.push(0x01).count(), 0);
        assert_eq!(
            r.push(0x02).map(words).collect::<Vec<_>>(),
            [[0x10F2_0102u32].to_vec()]
        );
    }

    #[test]
    fn reader_ignores_data_without_status() {
        // At start, after EOX, and after System Common (which clears running status).
        assert!(read(&[0x3C, 0x27]).is_empty());
        assert_eq!(
            read(&[0xF0, 0xF7, 0x3C, 0x27]),
            [[0x3000_0000u32, 0].to_vec()]
        );
        assert_eq!(
            read(&[0x90, 0x3C, 0x27, 0xF3, 0x05, 0x3C, 0x27]),
            [[0x2090_3C27u32].to_vec(), [0x10F3_0500].to_vec()]
        );
    }

    #[cfg(feature = "midi2-protocol")]
    #[test]
    fn reader_translates_to_midi2_protocol() {
        // 6.2, D.3.1: Note On with velocity 0 becomes a MIDI 2.0 Note Off.
        let mut r = BytesToUmp::new(g0(), StreamProtocol::Midi2);
        let out: Vec<_> = [0x90, 0x3C, 0x00].iter().flat_map(|&b| r.push(b)).collect();
        // D.3.3: Velocity 0x8000, Attribute Type 0 and Attribute 0.
        assert_eq!(out.len(), 1);
        assert_eq!(words(out[0]), [0x4080_3C00, 0x8000_0000]);
    }

    #[test]
    fn writer_frames_sysex_with_f0_and_f7() {
        let mut w = UmpToBytes::new(false);
        let bytes = write_all(
            &mut w,
            &[
                &[0x3016_0102, 0x0304_0506],
                &[0x3026_0708, 0x090A_0B0C],
                &[0x3031_0D00, 0],
            ],
        );
        let mut expected = [0xF0].to_vec();
        expected.extend(1..=13u8);
        expected.push(0xF7);
        assert_eq!(bytes, expected);
        assert_eq!(
            write_all(&mut w, &[&[0x3002_7E01, 0]]),
            [0xF0, 0x7E, 0x01, 0xF7]
        );
    }

    #[test]
    fn writer_skips_messages_without_byte_form() {
        let mut w = UmpToBytes::new(false);
        // JR Timestamp (Utility), SysEx8, UMP Stream: nothing (D.2.9).
        assert!(write_all(&mut w, &[&[0x0020_0010]]).is_empty());
        assert!(write_all(&mut w, &[&[0x5001_0000, 0, 0, 0]]).is_empty());
        assert!(write_all(&mut w, &[&[0xF000_0101, 0x1F, 0, 0]]).is_empty());
        // Mixed Data Set Header and Payload (Message Type 0x5, Status 0x8 and 0x9).
        assert!(write_all(&mut w, &[&[0x5080_0000, 0, 0, 0]]).is_empty());
        assert!(write_all(&mut w, &[&[0x5090_0000, 0, 0, 0]]).is_empty());
        // REQ-BS-004 on the writer side: the data bytes of AnH, DnH, EnH and F1H.
        assert_eq!(write_all(&mut w, &[&[0x20A0_0506]]), [0xA0, 0x05, 0x06]);
        assert_eq!(write_all(&mut w, &[&[0x20D0_0500]]), [0xD0, 0x05]);
        assert_eq!(write_all(&mut w, &[&[0x20E0_0506]]), [0xE0, 0x05, 0x06]);
        assert_eq!(write_all(&mut w, &[&[0x10F1_0500]]), [0xF1, 0x05]);
        assert_eq!(write_all(&mut w, &[&[0x20B0_0764]]), [0xB0, 0x07, 0x64]);
        assert_eq!(write_all(&mut w, &[&[0x20C0_0500]]), [0xC0, 0x05]);
        assert_eq!(write_all(&mut w, &[&[0x10F6_0000]]), [0xF6]);
    }

    /// REQ-BS-016: with the feature off, the reader is the core reader alone.
    #[cfg(not(feature = "midi2-protocol"))]
    #[test]
    fn reader_has_no_translator_state_without_the_feature() {
        assert_eq!(
            core::mem::size_of::<BytesToUmp>(),
            core::mem::size_of::<BytesToUmpCore>()
        );
    }

    #[test]
    fn writer_never_writes_undefined_status() {
        let mut w = UmpToBytes::new(false);
        for status in [0xF4u32, 0xF5, 0xF9, 0xFD] {
            assert!(write_all(&mut w, &[&[0x1000_0000 | status << 16]]).is_empty());
        }
        assert_eq!(write_all(&mut w, &[&[0x10F2_0102]]), [0xF2, 0x01, 0x02]);
    }

    #[test]
    fn writer_uses_running_status_when_enabled() {
        let mut w = UmpToBytes::new(true);
        let packets: [&[u32]; 6] = [
            &[0x2090_3C27],
            &[0x10F8_0000], // real time keeps running status
            &[0x2090_402B],
            &[0x2080_3C40], // other status: written
            &[0x10F3_0500], // System Common: next status written again
            &[0x2080_3E00],
        ];
        assert_eq!(
            write_all(&mut w, &packets),
            [0x90, 0x3C, 0x27, 0xF8, 0x40, 0x2B, 0x80, 0x3C, 0x40, 0xF3, 0x05, 0x80, 0x3E, 0x00]
        );
        // Without running status every status is written.
        let mut w = UmpToBytes::new(false);
        assert_eq!(
            write_all(&mut w, &[&[0x2090_3C27], &[0x2090_402B]]),
            [0x90, 0x3C, 0x27, 0x90, 0x40, 0x2B]
        );
    }

    #[test]
    fn writer_refresh_writes_status_again() {
        let mut w = UmpToBytes::new(true);
        write_all(&mut w, &[&[0x2090_3C27]]);
        w.refresh_status();
        assert_eq!(write_all(&mut w, &[&[0x2090_402B]]), [0x90, 0x40, 0x2B]);
    }

    #[test]
    fn writer_translates_midi2_channel_voice() {
        // 6.2, D.2.1: MIDI 2.0 Note On, velocity 0xFFFF, gives 90H 3CH 7FH.
        let mut w = UmpToBytes::new(false);
        assert_eq!(
            write_all(&mut w, &[&[0x4090_3C00, 0xFFFF_0000]]),
            [0x90, 0x3C, 0x7F]
        );
    }

    #[test]
    fn writer_rejects_small_buffer_without_change() {
        let mut w = UmpToBytes::new(true);
        let mut small = [0u8; 2];
        assert_eq!(
            w.write(&[0x2090_3C27], &mut small),
            Err(ByteStreamError::BufferTooSmall)
        );
        // The status was not recorded: the next write still has it.
        assert_eq!(write_all(&mut w, &[&[0x2090_3C27]]), [0x90, 0x3C, 0x27]);
    }

    #[test]
    fn scheduler_writes_at_render_time_in_order() {
        let mut s = Midi1Scheduler::<2>::new();
        let mut w = UmpToBytes::new(false);
        let mut out = [0u8; 16];
        s.push(Disposition::Consumed, &[0x0010_0000]).unwrap();
        s.push(Disposition::RenderAt(1_000), &[0x2090_3C27])
            .unwrap();
        s.push(Disposition::RenderAt(2_000), &[0x2080_3C40])
            .unwrap();
        assert_eq!(
            s.push(Disposition::RenderNow, &[0x10F8_0000]),
            Err(ByteStreamError::QueueFull)
        );
        assert_eq!(s.poll(999, &mut w, &mut out), 0);
        assert_eq!(s.poll(1_000, &mut w, &mut out), 3);
        assert_eq!(out[..3], [0x90, 0x3C, 0x27]);
        s.push(Disposition::RenderNow, &[0x10F8_0000]).unwrap();
        // Receive order: the clock waits behind the Note Off.
        assert_eq!(s.poll(1_500, &mut w, &mut out), 0);
        assert_eq!(s.poll(2_000, &mut w, &mut out), 4);
        assert_eq!(out[..4], [0x80, 0x3C, 0x40, 0xF8]);
    }

    #[test]
    fn prop_scheduler_writes_what_the_writer_writes() {
        // OPT-002: the scheduler keeps words, not Ump values. For any packets, the bytes are
        // those of the writer for each parsed UMP in order, and the word count of the written
        // UMP is the one of its Message Type (M2-104 Table 4).
        proptest!(|(packets in proptest::collection::vec(any::<[u32; 4]>(), 0..8))| {
            let mut s = Midi1Scheduler::<8>::new();
            let mut direct = UmpToBytes::new(false);
            let mut want = Vec::new();
            for p in &packets {
                s.push(Disposition::RenderNow, p).unwrap();
                if let Ok((ump, _)) = Ump::parse(p) {
                    let mut w = [0u32; 4];
                    let n = ump.write(&mut w).unwrap();
                    prop_assert_eq!(n, MessageType::from_bits((w[0] >> 28) as u8).word_count());
                    let mut buf = [0u8; 64];
                    if let Ok(k) = direct.write(&w[..n], &mut buf) {
                        want.extend_from_slice(&buf[..k]);
                    }
                }
            }
            let mut via = UmpToBytes::new(false);
            let mut out = [0u8; 1024];
            let n = s.poll(0, &mut via, &mut out);
            prop_assert_eq!(&out[..n], &want[..]);
        });
    }

    #[test]
    fn scheduler_keeps_message_that_does_not_fit() {
        let mut s = Midi1Scheduler::<4>::new();
        let mut w = UmpToBytes::new(false);
        s.push(Disposition::RenderNow, &[0x2090_3C27]).unwrap();
        s.push(Disposition::RenderNow, &[0x2090_402B]).unwrap();
        let mut out = [0u8; 4];
        assert_eq!(s.poll(0, &mut w, &mut out), 3);
        assert_eq!(s.poll(0, &mut w, &mut out), 3);
        assert_eq!(out[..3], [0x90, 0x40, 0x2B]);
        let mut empty = Midi1Scheduler::<0>::new();
        assert_eq!(
            empty.push(Disposition::RenderNow, &[0x10F8_0000]),
            Err(ByteStreamError::QueueFull)
        );
    }

    #[cfg(feature = "midi2-protocol")]
    #[test]
    fn prop_reader_never_panics() {
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..64), midi2 in any::<bool>())| {
            let protocol = if midi2 { StreamProtocol::Midi2 } else { StreamProtocol::Midi1 };
            let mut r = BytesToUmp::new(g0(), protocol);
            for b in bytes {
                prop_assert!(r.push(b).count() <= 2);
            }
        });
    }

    #[test]
    fn prop_channel_messages_round_trip_with_running_status() {
        proptest!(|(msgs in proptest::collection::vec((0x80u8..0xF0, 0u8..0x80, 0u8..0x80), 0..32))| {
            let mut w = UmpToBytes::new(true);
            let mut bytes = Vec::new();
            let mut expected = Vec::new();
            for (status, d0, d1) in msgs {
                let d1 = if data_len(status) == 2 { d1 } else { 0 };
                let packet = word(0x2, g0(), status, d0, d1);
                expected.push([packet].to_vec());
                bytes.extend(write_all(&mut w, &[&[packet]]));
            }
            prop_assert_eq!(read(&bytes), expected);
        });
    }

    /// The typed reader and writer before DEC-018, kept as the test reference of the word paths.
    #[allow(dead_code)]
    mod reference {
        use super::super::{data_len, undefined, ByteStreamError, Bytes, EOX, SOX};
        use zmidi_ump::{to_midi1, Group, Midi1ToMidi2Group, StreamProtocol, SysEx7Status, Ump};

        /// UMPs given by one [`BytesToUmp::push`]: at most two, in order. No alloc.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Pushed {
            /// The UMPs.
            items: [Option<Ump>; 2],
            /// Next item to give.
            next: usize,
        }

        impl Pushed {
            const EMPTY: Self = Self {
                items: [None; 2],
                next: 0,
            };

            fn add(&mut self, ump: Option<Ump>) {
                if let Some(slot) = self.items.iter_mut().find(|s| s.is_none()) {
                    *slot = ump;
                }
            }
        }

        impl Iterator for Pushed {
            type Item = Ump;

            fn next(&mut self) -> Option<Ump> {
                while let Some(slot) = self.items.get(self.next) {
                    self.next += 1;
                    if slot.is_some() {
                        return *slot;
                    }
                }
                None
            }
        }

        /// MIDI 1.0 byte stream reader that gives UMPs on one Group. Fixed size; no alloc.
        ///
        /// It is a [`BytesToUmpCore`] and, for MIDI 2.0 Protocol output, its own translator.
        #[derive(Debug, Clone)]
        pub struct BytesToUmp {
            core: BytesToUmpCore,
            /// Translator for MIDI 2.0 Protocol output (one Group: 128 bytes, OPT-001, OPT-011); `None` gives
            /// MIDI 1.0 Protocol UMPs.
            midi2: Option<Midi1ToMidi2Group>,
        }

        impl BytesToUmp {
            /// Creates a reader for `group`. [`StreamProtocol::Midi1`] gives Message Type 0x2;
            /// [`StreamProtocol::Midi2`] gives Message Type 0x4 through [`Midi1ToMidi2Group`] (D.3).
            pub fn new(group: Group, protocol: StreamProtocol) -> Self {
                Self {
                    core: BytesToUmpCore::new(group),
                    midi2: matches!(protocol, StreamProtocol::Midi2).then(Midi1ToMidi2Group::new),
                }
            }

            /// Takes one received byte and gives the UMPs it completes. Never panics.
            pub fn push(&mut self, byte: u8) -> Pushed {
                self.core.push_with(byte, self.midi2.as_mut())
            }
        }

        /// The MIDI 1.0 byte stream reader of [`BytesToUmp`] with no translator: 24 bytes on a 32-bit
        /// target (OPT-011). [`Self::push_with`] takes the translator of the caller for MIDI 2.0
        /// Protocol output, so that a reader of several streams (for example the cables of a USB MIDI
        /// endpoint) keeps translators only when it gives MIDI 2.0 Protocol UMPs.
        #[derive(Debug, Clone)]
        pub struct BytesToUmpCore {
            /// The Group of the UMPs.
            group: Group,
            /// Running status: the last Channel Voice or Mode status, or 0 for none.
            running: u8,
            /// Status of the message in progress, or 0 for none.
            status: u8,
            /// Data bytes of the message in progress.
            data: [u8; 2],
            /// Number of valid bytes in `data`.
            count: usize,
            /// True inside a System Exclusive message.
            in_sysex: bool,
            /// System Exclusive data not yet given.
            chunk: [u8; 6],
            /// Number of valid bytes in `chunk`.
            chunk_len: usize,
            /// True when a SysEx7 Start was given for the message in progress.
            started: bool,
        }

        impl BytesToUmpCore {
            /// Creates a reader for `group`.
            pub const fn new(group: Group) -> Self {
                Self {
                    group,
                    running: 0,
                    status: 0,
                    data: [0; 2],
                    count: 0,
                    in_sysex: false,
                    chunk: [0; 6],
                    chunk_len: 0,
                    started: false,
                }
            }

            /// Takes one received byte and gives the UMPs it completes. With `midi2`, each Channel
            /// Voice message goes through that translator and gives Message Type 0x4 (D.3); with
            /// `None`, Message Type 0x2. Give the same translator for each byte of one stream. Never
            /// panics.
            pub fn push_with(&mut self, byte: u8, midi2: Option<&mut Midi1ToMidi2Group>) -> Pushed {
                let mut out = Pushed::EMPTY;
                match byte {
                    // System Real Time: its own message; nothing else changes. F9H and FDH are ignored.
                    0xF8..=0xFF => {
                        if !undefined(byte) {
                            out.add(self.system(byte, 0, 0));
                        }
                    }
                    0x80..=0xF7 => {
                        // Any status that is not Real Time ends a System Exclusive message (M1 EOX).
                        if self.in_sysex {
                            let status = if self.started {
                                SysEx7Status::End
                            } else {
                                SysEx7Status::Complete
                            };
                            out.add(self.sysex(status));
                            self.in_sysex = false;
                        }
                        // A new status discards an incomplete message (M1 Status Bytes).
                        self.status = 0;
                        self.count = 0;
                        match byte {
                            0x80..=0xEF => {
                                self.running = byte;
                                self.status = byte;
                            }
                            SOX => {
                                self.running = 0;
                                self.in_sysex = true;
                                self.chunk_len = 0;
                                self.started = false;
                            }
                            0xF6 => {
                                self.running = 0;
                                out.add(self.system(byte, 0, 0));
                            }
                            // F1H to F3H wait for data; F4H, F5H and a lone F7H are ignored. All clear
                            // the running status (Appendix A-2).
                            _ => {
                                self.running = 0;
                                if data_len(byte) > 0 {
                                    self.status = byte;
                                }
                            }
                        }
                    }
                    _ => {
                        if self.in_sysex {
                            if self.chunk_len == self.chunk.len() {
                                let status = if self.started {
                                    SysEx7Status::Continue
                                } else {
                                    SysEx7Status::Start
                                };
                                out.add(self.sysex(status));
                                self.started = true;
                            }
                            self.chunk[self.chunk_len] = byte;
                            self.chunk_len += 1;
                            return out;
                        }
                        if self.status == 0 {
                            // Running status; data with no status is ignored (M1 Data Bytes).
                            if self.running == 0 {
                                return out;
                            }
                            self.status = self.running;
                            self.count = 0;
                        }
                        self.data[self.count] = byte;
                        self.count += 1;
                        if self.count == data_len(self.status) {
                            let (status, d0, d1) = (self.status, self.data[0], self.data[1]);
                            let d1 = if self.count == 2 { d1 } else { 0 };
                            self.status = 0;
                            self.count = 0;
                            out.add(if status < 0xF0 {
                                self.channel(status, d0, d1, midi2)
                            } else {
                                self.system(status, d0, d1)
                            });
                        }
                    }
                }
                out
            }

            /// A System UMP (Message Type 0x1, Figure 34).
            fn system(&self, status: u8, d0: u8, d1: u8) -> Option<Ump> {
                self.ump(&[word(0x1, self.group, status, d0, d1)])
            }

            /// A Channel Voice UMP: Message Type 0x2, or 0x4 through the translator (D.3).
            fn channel(
                &mut self,
                status: u8,
                d0: u8,
                d1: u8,
                midi2: Option<&mut Midi1ToMidi2Group>,
            ) -> Option<Ump> {
                let ump = self.ump(&[word(0x2, self.group, status, d0, d1)])?;
                match midi2 {
                    Some(translator) => translator.translate(&ump),
                    None => Some(ump),
                }
            }

            /// The SysEx7 UMP of the pending data, and the chunk emptied (D.3.7).
            fn sysex(&mut self, status: SysEx7Status) -> Option<Ump> {
                let data = &self.chunk[..self.chunk_len];
                let ump = zmidi_ump::SysEx7::try_new(self.group, status, data)
                    .ok()
                    .map(Ump::SysEx7);
                self.chunk_len = 0;
                ump
            }

            fn ump(&self, words: &[u32]) -> Option<Ump> {
                Ump::parse(words).ok().map(|(ump, _)| ump)
            }
        }

        /// Word 0 of a 32-bit UMP with a status byte and two data bytes.
        fn word(message_type: u32, group: Group, status: u8, d0: u8, d1: u8) -> u32 {
            message_type << 28
                | u32::from(group.into_bits()) << 24
                | u32::from(status) << 16
                | u32::from(d0) << 8
                | u32::from(d1)
        }

        /// UMP to MIDI 1.0 byte stream writer. Fixed size; no alloc.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct UmpToBytes {
            /// True when the writer omits repeated Channel Voice and Mode status bytes.
            running_status: bool,
            /// The last Channel Voice or Mode status written, or 0 when the next one must be written.
            last_status: u8,
        }

        impl UmpToBytes {
            /// Creates a writer, with or without running status (M1 Running Status).
            pub fn new(running_status: bool) -> Self {
                Self {
                    running_status,
                    last_status: 0,
                }
            }

            /// The next Channel Voice or Mode message writes its status byte (Appendix A-2: refresh the
            /// status every few seconds).
            pub fn refresh_status(&mut self) {
                self.last_status = 0;
            }

            /// Writes the MIDI 1.0 bytes of one UMP into `out` and gives the count. Gives
            /// [`ByteStreamError::BufferTooSmall`], with nothing written and no state change, when
            /// `out` is too small. A UMP with no MIDI 1.0 byte form writes nothing (D.2.8, D.2.9).
            /// Never panics.
            pub fn write(
                &mut self,
                words: &[u32],
                out: &mut [u8],
            ) -> Result<usize, ByteStreamError> {
                let Ok((ump, _)) = Ump::parse(words) else {
                    return Ok(0);
                };
                let mut bytes = Bytes::default();
                let mut last = self.last_status;
                match ump {
                    Ump::System(_) => {
                        let status = (words[0] >> 16) as u8;
                        if !undefined(status) {
                            bytes.push(status);
                            if status < 0xF8 {
                                // System Common ends running status (M1 Running Status).
                                last = 0;
                                for i in 0..data_len(status) {
                                    bytes.push((words[0] >> (8 - 8 * i)) as u8 & 0x7F);
                                }
                            }
                        }
                    }
                    Ump::Midi1ChannelVoice(_) => self.channel(words[0], &mut last, &mut bytes),
                    Ump::Midi2ChannelVoice(_) => {
                        for m in to_midi1(&ump).as_slice() {
                            let mut w = [0u32; 1];
                            if m.write(&mut w).is_ok() {
                                self.channel(w[0], &mut last, &mut bytes);
                            }
                        }
                    }
                    Ump::SysEx7(packet) => {
                        let status = packet.status();
                        if matches!(status, SysEx7Status::Start | SysEx7Status::Complete) {
                            bytes.push(SOX);
                            last = 0;
                        }
                        let data = packet.data();
                        for &b in data.iter() {
                            bytes.push(b);
                        }
                        if matches!(status, SysEx7Status::End | SysEx7Status::Complete) {
                            bytes.push(EOX);
                        }
                    }
                    // Utility, Data 128, Flex Data, UMP Stream and unknown packets (D.2.9).
                    _ => {}
                }
                let src = bytes.as_slice();
                let dst = out
                    .get_mut(..src.len())
                    .ok_or(ByteStreamError::BufferTooSmall)?;
                dst.copy_from_slice(src);
                self.last_status = last;
                Ok(src.len())
            }

            /// The bytes of one MIDI 1.0 Channel Voice word (Message Type 0x2).
            fn channel(&self, w0: u32, last: &mut u8, bytes: &mut Bytes) {
                let status = (w0 >> 16) as u8;
                if !(self.running_status && *last == status) {
                    bytes.push(status);
                }
                *last = status;
                bytes.push((w0 >> 8) as u8 & 0x7F);
                if data_len(status) == 2 {
                    bytes.push(w0 as u8 & 0x7F);
                }
            }
        }
    }

    fn ump_words(u: &Ump) -> Vec<u32> {
        let mut w = [0u32; 4];
        let n = u.write(&mut w).unwrap();
        w[..n].to_vec()
    }

    #[test]
    fn prop_reader_words_equal_typed_reference() {
        // DEC-018: for random byte streams, both protocols, the word reader gives the same UMPs
        // as the typed reader it replaces.
        proptest!(ProptestConfig::with_cases(5_000), |(
            bytes in proptest::collection::vec(any::<u8>(), 0..96),
            midi2 in any::<bool>(),
        )| {
            // Without the feature midi2-protocol, only MIDI 1.0 Protocol output exists.
            let midi2 = midi2 && cfg!(feature = "midi2-protocol");
            let protocol = if midi2 {
                zmidi_ump::StreamProtocol::Midi2
            } else {
                zmidi_ump::StreamProtocol::Midi1
            };
            let mut a = reference::BytesToUmp::new(g0(), protocol);
            #[cfg(feature = "midi2-protocol")]
            let mut b = BytesToUmp::new(g0(), protocol);
            #[cfg(not(feature = "midi2-protocol"))]
            let mut b = BytesToUmp::midi1(g0());
            for &x in &bytes {
                let ua: Vec<Vec<u32>> = a.push(x).map(|u| ump_words(&u)).collect();
                let ub: Vec<Vec<u32>> = b.push(x).map(words).collect();
                prop_assert_eq!(ua, ub, "byte {:02x}", x);
            }
        });
    }

    #[test]
    fn prop_writer_words_equal_typed_reference() {
        // DEC-018: for random packets of every Message Type, with or without running status and
        // after any previous status, the word writer gives the same bytes and the same state.
        proptest!(ProptestConfig::with_cases(20_000), |(
            mt in prop_oneof![Just(1u32), Just(2), Just(3), Just(4), 0u32..16],
            w0 in any::<u32>(),
            w1 in any::<u32>(),
            running in any::<bool>(),
            prev in 0u32..16,
        )| {
            let words = [mt << 28 | (w0 & 0x0FFF_FFFF), w1, 0, 0];
            let mut a = reference::UmpToBytes::new(running);
            let mut b = UmpToBytes::new(running);
            let first = [0x2090_3C40 | prev << 16];
            let mut t = [0u8; 16];
            a.write(&first, &mut t).unwrap();
            b.write(&first, &mut t).unwrap();
            let (mut oa, mut ob) = ([0u8; 16], [0u8; 16]);
            let ra = a.write(&words, &mut oa);
            let rb = b.write(&words, &mut ob);
            prop_assert_eq!(&ra, &rb, "words {:08x?}", words);
            if let Ok(n) = ra {
                prop_assert_eq!(&oa[..n], &ob[..n], "words {:08x?}", words);
            }
            let mut t2 = [0u8; 16];
            prop_assert_eq!(a.write(&first, &mut t2), b.write(&first, &mut t), "state after {:08x?}", words);
        });
    }

    #[test]
    fn prop_feed_gives_the_packets_of_push() {
        // REQ-BS-017: the callback reader gives the words of the packets of push, in order.
        proptest!(ProptestConfig::with_cases(5_000), |(bytes in proptest::collection::vec(any::<u8>(), 0..96))| {
            let (mut a, mut b) = (BytesToUmpCore::new(g0()), BytesToUmpCore::new(g0()));
            for &x in &bytes {
                let pushed: Vec<Vec<u32>> = a.push_with(x, None).map(words).collect();
                let mut fed: Vec<Vec<u32>> = Vec::new();
                b.feed(x, |w| fed.push(w.to_vec()));
                prop_assert_eq!(pushed, fed);
            }
        });
    }

    #[test]
    fn prop_write_each_gives_the_bytes_of_write() {
        // REQ-BS-017: the callback writer gives the bytes of write and keeps the same running
        // status; write changes nothing when `out` is too small.
        proptest!(ProptestConfig::with_cases(20_000), |(
            mt in prop_oneof![Just(1u32), Just(2), Just(3), Just(4), 0u32..16],
            w0 in any::<u32>(),
            w1 in any::<u32>(),
            running in any::<bool>(),
            room in 0usize..16,
        )| {
            let words = [mt << 28 | (w0 & 0x0FFF_FFFF), w1, 0, 0];
            let (mut a, mut b) = (UmpToBytes::new(running), UmpToBytes::new(running));
            let mut out = [0u8; 16];
            let n = a.write(&words, &mut out).unwrap();
            let mut each = Vec::new();
            b.write_each(&words, |x| each.push(x));
            prop_assert_eq!(&out[..n], each.as_slice());
            prop_assert_eq!(&a, &b);
            let before = a.clone();
            let mut small = std::vec![0u8; room];
            match a.write(&words, &mut small) {
                Ok(k) => prop_assert!(k <= room),
                Err(e) => {
                    prop_assert_eq!(e, ByteStreamError::BufferTooSmall);
                    prop_assert_eq!(&a, &before);
                }
            }
        });
    }

    #[test]
    fn prop_reader_feed_gives_the_packets_of_push() {
        // REQ-BS-017, OPT-036: BytesToUmp::feed gives the packets of BytesToUmp::push, with the
        // translator of the reader when it has one.
        proptest!(ProptestConfig::with_cases(5_000), |(
            bytes in proptest::collection::vec(any::<u8>(), 0..96),
            midi2 in any::<bool>(),
        )| {
            #[cfg(feature = "midi2-protocol")]
            let new = || BytesToUmp::new(g0(), if midi2 { StreamProtocol::Midi2 } else { StreamProtocol::Midi1 });
            #[cfg(not(feature = "midi2-protocol"))]
            let new = || { let _ = midi2; BytesToUmp::midi1(g0()) };
            let (mut a, mut b) = (new(), new());
            for &x in &bytes {
                let pushed: Vec<Vec<u32>> = a.push(x).map(words).collect();
                let mut fed: Vec<Vec<u32>> = Vec::new();
                b.feed(x, |w| fed.push(w.to_vec()));
                prop_assert_eq!(pushed, fed);
            }
        });
    }

    #[cfg(feature = "midi2-protocol")]
    #[test]
    fn prop_feed_with_gives_the_packets_of_push_with() {
        // REQ-BS-017: with and without the translator, the callback reader gives the packets of
        // the value form.
        proptest!(ProptestConfig::with_cases(5_000), |(
            bytes in proptest::collection::vec(any::<u8>(), 0..96),
            midi2 in any::<bool>(),
        )| {
            let (mut a, mut b) = (BytesToUmpCore::new(g0()), BytesToUmpCore::new(g0()));
            let (mut ta, mut tb) = (Midi1ToMidi2Group::new(), Midi1ToMidi2Group::new());
            for &x in &bytes {
                let pushed: Vec<Vec<u32>> = a
                    .push_with(x, midi2.then_some(&mut ta))
                    .map(words)
                    .collect();
                let mut fed: Vec<Vec<u32>> = Vec::new();
                b.feed_with(x, midi2.then_some(&mut tb), |w| fed.push(w.to_vec()));
                prop_assert_eq!(pushed, fed);
            }
        });
    }
}
