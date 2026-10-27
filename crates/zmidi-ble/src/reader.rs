//! BLE-MIDI v1.0 packet reader: packets of the MIDI Data I/O characteristic to UMPs with their
//! 13-bit timestamps (sections 7 to 9, DEC-BLE-001).
//!
//! - The first byte is the header byte; a packet whose first byte has bit 7 = 0 is dropped
//!   (AMB-BLE-MIDI-09). Bit 6 is ignored (AMB-BLE-MIDI-04).
//! - After a message, a byte with bit 7 set is a timestamp byte; after a timestamp byte, a byte
//!   with bit 7 set is a status byte (7, rule 2: every status byte has a timestamp byte).
//! - Running status (7): data bytes with no status use the status of the most recent full
//!   Channel Voice or Mode message of the packet, with or without a timestamp byte. System
//!   Common, System Real-Time and System Exclusive messages do not cancel it; the end of the
//!   packet does (AMB-BLE-MIDI-01, -02). The reader writes the status byte again, so `zmidi-io`
//!   gets whole messages only.
//! - Timestamps (9): when timestampLow is smaller than the preceding one of the packet,
//!   timestampHigh goes up by one.
//! - System Exclusive (8): it can continue in the next packets; a continuation packet has SysEx
//!   data right after the header byte. A UMP that ends there gets the most recent timestamp of
//!   its SysEx message (AMB-BLE-MIDI-07). A status that is not System Real-Time or F7H ends it.
//! - Each message goes through the zmidi-io `BytesToUmp` of the caller's Group and Protocol
//!   (DEC-016). Errors go to the sink and the rest of the packet is read (AMB-BLE-MIDI-09).
//! - [`PacketReader::read_words`] gives the words of each UMP; [`PacketReader::read`] is the
//!   typed form on top of it and parses them to a [`Ump`] (DEC-BLE-002). A firmware that calls
//!   only `read_words` does not link `Ump::parse` (OPT-044).

use zmidi_io::BytesToUmp;
use zmidi_ump::{Group, StreamProtocol, Ump};

use crate::timestamp::{header_high, is_header, Timestamp};

/// System Exclusive start (F0H) and end (F7H).
const SOX: u8 = 0xF0;
const EOX: u8 = 0xF7;

/// Error of one part of a packet. The reader drops that part and reads the rest of the packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    /// The first byte of the packet has bit 7 = 0: the packet is dropped whole.
    #[error("packet with no header byte")]
    NoHeader,
    /// A data byte with no running status in the packet and no open SysEx: the byte is dropped.
    #[error("data byte with no running status")]
    NoRunningStatus,
    /// A message that the end of the packet or a byte with bit 7 set cuts: the message is
    /// dropped.
    #[error("incomplete message")]
    Incomplete,
    /// A timestamp byte at the end of the packet with no message after it.
    #[error("timestamp byte with no message")]
    LoneTimestamp,
}

/// One UMP of a packet with the 13-bit timestamp of its message (sender clock).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Received {
    /// The timestamp of the message.
    pub timestamp: Timestamp,
    /// The UMP.
    pub ump: Ump,
}

/// One UMP of a packet as words, with the 13-bit timestamp of its message (sender clock); the
/// word form of [`Received`] (DEC-BLE-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceivedWords<'w> {
    /// The timestamp of the message.
    pub timestamp: Timestamp,
    /// The words of the UMP: 1 or 2 words.
    pub words: &'w [u32],
}

/// Reads BLE-MIDI packets. Fixed size; no alloc. The state between packets is an open SysEx
/// message and the most recent timestamp.
#[derive(Debug, Clone)]
pub struct PacketReader {
    group: Group,
    protocol: StreamProtocol,
    bytes: BytesToUmp,
    /// True while a SysEx message is open.
    sysex: bool,
    /// The most recent timestamp.
    time: Timestamp,
}

impl Default for PacketReader {
    /// Group 0, MIDI 1.0 Protocol.
    fn default() -> Self {
        Self::new(Group::default(), StreamProtocol::Midi1)
    }
}

/// The message that the reader collects: at most a status byte and two data bytes.
#[derive(Debug, Clone, Copy, Default)]
struct Message {
    bytes: [u8; 3],
    have: usize,
    need: usize,
}

/// Number of data bytes after a status byte that is not System Exclusive (M1 Tables I, V, VI).
fn data_len(status: u8) -> usize {
    match status {
        0x80..=0xBF | 0xE0..=0xEF | 0xF2 => 2,
        0xC0..=0xDF | 0xF1 | 0xF3 => 1,
        _ => 0,
    }
}

impl PacketReader {
    /// A reader that gives UMPs on `group` with `protocol`: [`StreamProtocol::Midi1`] gives
    /// Message Type 0x2, [`StreamProtocol::Midi2`] Message Type 0x4 (zmidi-io `BytesToUmp`).
    pub fn new(group: Group, protocol: StreamProtocol) -> Self {
        Self {
            group,
            protocol,
            bytes: BytesToUmp::new(group, protocol),
            sysex: false,
            time: Timestamp::default(),
        }
    }

    /// Forgets an open SysEx message, for example after a new connection.
    pub fn reset(&mut self) {
        *self = Self::new(self.group, self.protocol);
    }

    /// True while a SysEx message is open (it continues in the next packet).
    pub fn in_sysex(&self) -> bool {
        self.sysex
    }

    /// Reads one packet and gives each UMP, or an error, to `sink` in order. An empty packet
    /// and a packet with the header byte only give nothing. Never panics. The typed form of
    /// [`Self::read_words`] (DEC-BLE-002).
    pub fn read(&mut self, packet: &[u8], mut sink: impl FnMut(Result<Received, ReadError>)) {
        self.read_words(packet, |r| match r {
            // The reader gives only whole UMPs, so `parse` always succeeds.
            Ok(w) => {
                if let Ok((ump, _)) = Ump::parse(w.words) {
                    sink(Ok(Received {
                        timestamp: w.timestamp,
                        ump,
                    }));
                }
            }
            Err(e) => sink(Err(e)),
        });
    }

    /// Reads one packet and gives the words of each UMP, or an error, to `sink` in order, with
    /// no parse to a [`Ump`] (DEC-BLE-002, OPT-044). Same rules, items and errors as
    /// [`Self::read`]. Never panics.
    pub fn read_words(
        &mut self,
        packet: &[u8],
        mut sink: impl FnMut(Result<ReceivedWords<'_>, ReadError>),
    ) {
        let Some((&first, rest)) = packet.split_first() else {
            return;
        };
        if !is_header(first) {
            sink(Err(ReadError::NoHeader));
            return;
        }
        let mut high = header_high(first);
        let mut prev_low: Option<u8> = None;
        // Running status of this packet (7, rule 4: the end of a packet cancels it).
        let mut running = 0u8;
        let mut after_timestamp = false;
        let mut msg = Message::default();
        for &b in rest {
            if msg.need > 0 {
                if b < 0x80 {
                    msg.bytes[msg.have] = b;
                    msg.have += 1;
                    if msg.have == msg.need {
                        self.feed(&msg.bytes[..msg.need], &mut sink);
                        msg = Message::default();
                    }
                    continue;
                }
                // A byte with bit 7 set cuts the message; it is a timestamp byte.
                sink(Err(ReadError::Incomplete));
                msg = Message::default();
            }
            if after_timestamp {
                after_timestamp = false;
                if b >= 0x80 {
                    self.status(b, &mut running, &mut msg, &mut sink);
                } else {
                    self.data(b, running, &mut msg, &mut sink);
                }
                continue;
            }
            if b >= 0x80 {
                let low = b & 0x7F;
                if prev_low.is_some_and(|p| low < p) {
                    high = (high + 1) & 0x3F;
                }
                prev_low = Some(low);
                self.time = Timestamp::from_parts(high, low);
                after_timestamp = true;
            } else {
                self.data(b, running, &mut msg, &mut sink);
            }
        }
        if msg.need > 0 {
            sink(Err(ReadError::Incomplete));
        }
        if after_timestamp {
            sink(Err(ReadError::LoneTimestamp));
        }
    }

    /// A status byte after a timestamp byte.
    fn status(
        &mut self,
        b: u8,
        running: &mut u8,
        msg: &mut Message,
        sink: &mut impl FnMut(Result<ReceivedWords<'_>, ReadError>),
    ) {
        match b {
            // System Real-Time: also inside a SysEx message (8, item 4).
            0xF8..=0xFF => self.feed(&[b], sink),
            SOX => {
                self.sysex = true;
                self.feed(&[b], sink);
            }
            EOX => {
                self.sysex = false;
                self.feed(&[b], sink);
            }
            _ => {
                // Any other status ends an open SysEx (M1 EOX); zmidi-io ends it on this byte.
                self.sysex = false;
                if b < 0xF0 {
                    *running = b;
                }
                let need = 1 + data_len(b);
                if need == 1 {
                    self.feed(&[b], sink);
                } else {
                    *msg = Message {
                        bytes: [b, 0, 0],
                        have: 1,
                        need,
                    };
                }
            }
        }
    }

    /// A data byte that is not part of a message in progress.
    fn data(
        &mut self,
        b: u8,
        running: u8,
        msg: &mut Message,
        sink: &mut impl FnMut(Result<ReceivedWords<'_>, ReadError>),
    ) {
        if self.sysex {
            self.feed(&[b], sink);
        } else if running != 0 {
            let need = 1 + data_len(running);
            *msg = Message {
                bytes: [running, b, 0],
                have: 2,
                need,
            };
            if need == 2 {
                self.feed(&msg.bytes[..2], sink);
                *msg = Message::default();
            }
        } else {
            sink(Err(ReadError::NoRunningStatus));
        }
    }

    /// Gives `bytes` to zmidi-io and each UMP that they complete to the sink.
    fn feed(&mut self, bytes: &[u8], sink: &mut impl FnMut(Result<ReceivedWords<'_>, ReadError>)) {
        let timestamp = self.time;
        for &b in bytes {
            // The callback path of zmidi-io (OPT-036).
            self.bytes
                .feed(b, |words| sink(Ok(ReceivedWords { timestamp, words })));
        }
    }
}
