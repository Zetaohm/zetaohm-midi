//! BLE-MIDI v1.0 packet writer: UMPs with their times to packets of the MIDI Data I/O
//! characteristic (sections 7 to 9, DEC-BLE-001).
//!
//! - A packet is at most ATT_MTU - 3 bytes (7); the ATT_MTU is 23 or more (default 23).
//! - The first byte is the header byte (bit 6 = 0); a timestamp byte comes before each status
//!   byte (7, rule 2). The header has the timestampHigh of the first timestamp byte.
//! - Running status (on by default): a Channel Voice or Mode message with the status of the
//!   preceding message of the packet, and no system message between them, has no status byte,
//!   and no timestamp byte when its time is the same (7, AMB-BLE-MIDI-01).
//! - A message that is not System Exclusive is whole in one packet (8). SysEx data fills the
//!   packet and continues in continuation packets (header byte, then data); F0H and F7H have a
//!   timestamp byte (8). System Real-Time messages have their own timestamp byte, also inside a
//!   SysEx message.
//! - Another message while a SysEx message is open first ends it with F7H; the later Continue
//!   and End data of that SysEx is dropped (AMB-BLE-MIDI-08).
//! - Times: the caller's milliseconds, modulo 8192 on the air; a time earlier than the
//!   preceding one gets the preceding one (AMB-BLE-MIDI-06). A packet ends before a message
//!   128 ms or more after its first timestamp, so the receiver sees at most one wrap of
//!   timestampLow (AMB-BLE-MIDI-05).
//! - The UMPs go through the zmidi-io `UmpToBytes` (no running status there); the Group is
//!   ignored, and a UMP with no MIDI 1.0 form gives nothing (DEC-016).

use core::iter::Peekable;

use zmidi_io::UmpToBytes;

use crate::timestamp::Timestamp;

/// System Exclusive start (F0H) and end (F7H).
const SOX: u8 = 0xF0;
const EOX: u8 = 0xF7;

/// The smallest LE ATT_MTU.
pub const MIN_ATT_MTU: u16 = 23;

/// The number of bytes that one UMP can give: 4 translated messages of 3 bytes (M2-104 D.2.3)
/// and one F7H that ends an open SysEx.
const HELD: usize = 13;

/// Error of the packet writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// The ATT_MTU is smaller than 23.
    #[error("ATT_MTU smaller than 23")]
    MtuTooSmall,
    /// The output buffer is shorter than [`PacketWriter::packet_len`]; nothing was written.
    #[error("output buffer shorter than the packet length")]
    BufferTooSmall,
}

/// A UMP and the time of its message in milliseconds (the caller's clock).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedUmp<'a> {
    /// The time in milliseconds.
    pub millis: u64,
    /// The words of the UMP.
    pub words: &'a [u32],
}

/// Writes BLE-MIDI packets. Fixed size; no alloc. Between two packets it holds the bytes of at
/// most one UMP (a SysEx part, or a message that did not fit).
#[derive(Debug, Clone)]
pub struct PacketWriter {
    /// ATT_MTU - 3.
    packet_len: usize,
    running_status: bool,
    bytes: UmpToBytes,
    held: [u8; HELD],
    held_len: usize,
    held_pos: usize,
    held_time: u64,
    /// The time of the preceding UMP.
    last_time: Option<u64>,
    /// True while a SysEx message is open in the output.
    sysex: bool,
}

impl Default for PacketWriter {
    /// ATT_MTU 23 (packets of 20 bytes), running status on.
    fn default() -> Self {
        Self {
            packet_len: usize::from(MIN_ATT_MTU) - 3,
            running_status: true,
            bytes: UmpToBytes::new(false),
            held: [0; HELD],
            held_len: 0,
            held_pos: 0,
            held_time: 0,
            last_time: None,
            sysex: false,
        }
    }
}

/// The packet being written.
#[derive(Debug, Clone, Copy, Default)]
struct Packet {
    len: usize,
    /// The timestampHigh of the header byte.
    header_high: u8,
    /// The time of the first and of the most recent timestamp byte.
    first_time: Option<u64>,
    last_time: Option<u64>,
    /// The status that a running status message can use, or 0.
    running: u8,
}

/// Number of data bytes after a status byte that is not System Exclusive (M1 Tables I, V, VI).
fn data_len(status: u8) -> usize {
    match status {
        0x80..=0xBF | 0xE0..=0xEF | 0xF2 => 2,
        0xC0..=0xDF | 0xF1 | 0xF3 => 1,
        _ => 0,
    }
}

impl PacketWriter {
    /// A writer for the ATT_MTU `att_mtu` (23 or more), with running status on.
    pub fn new(att_mtu: u16) -> Result<Self, WriteError> {
        let mut w = Self::default();
        w.set_att_mtu(att_mtu)?;
        Ok(w)
    }

    /// Sets the ATT_MTU after an MTU exchange (section 6). From the next packet on.
    pub fn set_att_mtu(&mut self, att_mtu: u16) -> Result<(), WriteError> {
        if att_mtu < MIN_ATT_MTU {
            return Err(WriteError::MtuTooSmall);
        }
        self.packet_len = usize::from(att_mtu) - 3;
        Ok(())
    }

    /// Running status on or off (on by default). Off gives full messages only.
    pub fn set_running_status(&mut self, on: bool) {
        self.running_status = on;
    }

    /// The largest packet: ATT_MTU - 3 bytes. The buffer of [`Self::write_packet`] must have
    /// this length or more.
    pub fn packet_len(&self) -> usize {
        self.packet_len
    }

    /// True when the writer holds bytes of a UMP for the next packet.
    pub fn has_held(&self) -> bool {
        self.held_pos < self.held_len
    }

    /// Forgets the held bytes, an open SysEx message and the preceding time, for example after
    /// a new connection.
    pub fn reset(&mut self) {
        let (packet_len, running_status) = (self.packet_len, self.running_status);
        *self = Self {
            packet_len,
            running_status,
            ..Self::default()
        };
    }

    /// Writes one packet into `out` from the held bytes and then from `umps`, and gives its
    /// length; 0 when there is nothing to send. A UMP is taken out of `umps` only when its bytes
    /// are written or held. Never panics.
    pub fn write_packet<'w, I>(
        &mut self,
        umps: &mut Peekable<I>,
        out: &mut [u8],
    ) -> Result<usize, WriteError>
    where
        I: Iterator<Item = TimedUmp<'w>>,
    {
        let out = out
            .get_mut(..self.packet_len)
            .ok_or(WriteError::BufferTooSmall)?;
        let mut p = Packet::default();
        loop {
            if !self.has_held() {
                let Some(next) = umps.peek() else { break };
                let time = self.last_time.map_or(next.millis, |t| next.millis.max(t));
                if p.first_time.is_some_and(|f| time - f >= 128) {
                    break;
                }
                let words = next.words;
                umps.next();
                self.last_time = Some(time);
                self.load(words, time);
                continue;
            }
            if !self.write_unit(out, &mut p) {
                break;
            }
        }
        Ok(p.len)
    }

    /// Takes the MIDI 1.0 bytes of one UMP as the held bytes, with the SysEx rules of section 8.
    fn load(&mut self, words: &[u32], time: u64) {
        let mut buf = [0u8; HELD];
        let n = self.bytes.write(words, &mut buf[1..]).unwrap_or(0);
        let bytes = &buf[1..1 + n];
        let Some(&first) = bytes.first() else {
            return;
        };
        let mut start = 1;
        if first < 0x80 {
            // SysEx7 Continue or End data: dropped when no SysEx message is open.
            if !self.sysex {
                return;
            }
        } else if first < 0xF8 && self.sysex {
            // Another message while a SysEx message is open: end it first (AMB-BLE-MIDI-08).
            buf[0] = EOX;
            start = 0;
        }
        let bytes = &buf[start..1 + n];
        self.held[..bytes.len()].copy_from_slice(bytes);
        self.held_len = bytes.len();
        self.held_pos = 0;
        self.held_time = time;
    }

    /// Writes the next unit of the held bytes: one SysEx data byte, or one message with its
    /// timestamp byte. False when it does not fit (the packet ends).
    fn write_unit(&mut self, out: &mut [u8], p: &mut Packet) -> bool {
        let time = self.held_time;
        let b = self.held[self.held_pos];
        let header = usize::from(p.len == 0);
        let free = out.len() - p.len - header;
        if b < 0x80 {
            // SysEx data: no timestamp byte (8, items 2 and 3).
            if free < 1 {
                return false;
            }
            self.start(out, p, time);
            out[p.len] = b;
            p.len += 1;
            self.held_pos += 1;
            return true;
        }
        let len = match b {
            0x80..=0xEF | 0xF1..=0xF6 => 1 + data_len(b),
            _ => 1,
        }
        .min(self.held_len - self.held_pos);
        let running = self.running_status && b < 0xF0 && p.running == b;
        let timestamp = !running || p.last_time != Some(time);
        let need = usize::from(timestamp) + if running { len - 1 } else { len };
        if free < need {
            return false;
        }
        if timestamp {
            let t = Timestamp::from_millis(time);
            if p.len > 0 && p.first_time.is_none() && t.high() != p.header_high {
                // The first timestamp byte must use the timestampHigh of the header.
                return false;
            }
        }
        self.start(out, p, time);
        if timestamp {
            out[p.len] = Timestamp::from_millis(time).timestamp_byte();
            p.len += 1;
            p.first_time.get_or_insert(time);
            p.last_time = Some(time);
        }
        let from = self.held_pos + usize::from(running);
        let to = self.held_pos + len;
        out[p.len..p.len + (to - from)].copy_from_slice(&self.held[from..to]);
        p.len += to - from;
        self.held_pos = to;
        match b {
            SOX => self.sysex = true,
            EOX => self.sysex = false,
            _ => {}
        }
        // Running status only after a channel message, never across a system message.
        p.running = if b < 0xF0 { b } else { 0 };
        true
    }

    /// Writes the header byte when the packet is empty.
    fn start(&self, out: &mut [u8], p: &mut Packet, time: u64) {
        if p.len == 0 {
            let t = Timestamp::from_millis(time);
            out[0] = t.header_byte();
            p.header_high = t.high();
            p.len = 1;
        }
    }
}
