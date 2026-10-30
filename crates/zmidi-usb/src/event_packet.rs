//! USB-MIDI 1.0 Event Packets of a MIDI Streaming endpoint (USB-MIDI v1.0 sections 3.2.1, 4
//! and 5; Alternate Setting 0 of a USB-MIDI 2.0 function).
//!
//! - An [`EventPacket`] is 4 bytes in bus order: byte 0 holds the Cable Number (bits 7 to 4)
//!   and the Code Index Number (bits 3 to 0), then MIDI_0, MIDI_1 and MIDI_2 (Figure 8,
//!   REQ-USB-EVT-001). [`CIN_SIZE`] is Table 4-1 (REQ-USB-EVT-002).
//! - [`PacketReader`] gives the UMPs of the packets of one USB transfer. It keeps one zmidi-io
//!   [`BytesToUmpCore`] for each cable (REQ-USB-EVT-011) and gives cable n on Group n
//!   (REQ-USB-EVT-015, choice U-19 of DEC-USB-001).
//! - [`PacketWriter`] writes the packets of UMPs straight from the words. The packets hold the
//!   MIDI 1.0 bytes of zmidi-io [`UmpToBytes`](zmidi_io::UmpToBytes) with running status off (REQ-USB-EVT-013, REQ-USB-EVT-014) and writes
//!   Group n on cable n (REQ-USB-EVT-016).
//! - The number of cables is the const generic `CABLES`, 1 to 16, default 1 (choice U-21).
//!
//! No alloc (DEC-002). Synchronous and sans-IO with no USB stack (DEC-004, DEC-014). Nothing
//! panics on received bytes or on UMP words; the reader is lenient (choice U-24).

use zmidi_io::{BytesToUmpCore, Pushed};
use zmidi_ump::Midi1ToMidi2Group;
use zmidi_ump::{midi2_to_midi1, Group, MessageType, Packet, StreamProtocol, Ump};

/// The size of one Event Packet in bytes (section 4).
pub const PACKET_LEN: usize = 4;

/// The largest number of packets that one UMP gives in the parsed mode: 4 MIDI 1.0 messages of
/// a translated MIDI 2.0 Channel Voice UMP and 1 packet that ends a System Exclusive message
/// (REQ-USB-EVT-021).
pub const MAX_PACKETS_PER_UMP: usize = 5;

/// The largest number of packets that one UMP gives in the unparsed mode: 12 bytes of 4
/// translated MIDI 1.0 messages and 1 added 0xF7 (REQ-USB-EVT-012, REQ-USB-EVT-024).
pub const MAX_UNPARSED_PACKETS_PER_UMP: usize = 13;

/// The largest number of cables of one endpoint (Cable Number 0x0 to 0xF, section 4).
pub const MAX_CABLES: usize = 16;

/// Error of the Event Packet reader and writer (thiserror 2, no_std).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EventPacketError {
    /// The free part of the caller buffer cannot hold all packets of the UMP; nothing was
    /// written and the writer state did not change (REQ-USB-EVT-021). When `out` is the free
    /// rest of one USB packet, send that packet and give the same UMP again. (The Link module
    /// does not get this error: it splits the packets of a UMP over USB packets,
    /// REQ-USB-EVT-030.)
    #[error("output buffer too small")]
    Overflow,
    /// 1 to 3 bytes after the last whole Event Packet of a transfer; they are ignored
    /// (REQ-USB-EVT-020, choice U-04).
    #[error("{len} bytes after the last whole Event Packet")]
    Tail {
        /// The number of ignored bytes, 1 to 3.
        len: usize,
    },
    /// A Cable Number above 0xF (REQ-USB-EVT-001).
    #[error("Cable Number out of range")]
    InvalidCable,
    /// MIDI 2.0 Protocol output needs one translator for each cable: `X` of the reader is
    /// below `CABLES` (OPT-011).
    #[error("MIDI 2.0 Protocol output needs X >= CABLES translators")]
    NoTranslators,
}

/// Code Index Number: the classification of the MIDI_x bytes of a packet (Table 4-1). 4 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cin {
    /// 0x0: miscellaneous function codes, reserved. No size.
    Misc = 0x0,
    /// 0x1: cable events, reserved. No size.
    CableEvent = 0x1,
    /// 0x2: two-byte System Common message (0xF1, 0xF3).
    SystemCommon2 = 0x2,
    /// 0x3: three-byte System Common message (0xF2).
    SystemCommon3 = 0x3,
    /// 0x4: System Exclusive starts or continues.
    SysExStart = 0x4,
    /// 0x5: single-byte System Common message (0xF6), or System Exclusive ends with 1 byte.
    SingleByteCommonOrSysExEnd1 = 0x5,
    /// 0x6: System Exclusive ends with 2 bytes.
    SysExEnd2 = 0x6,
    /// 0x7: System Exclusive ends with 3 bytes.
    SysExEnd3 = 0x7,
    /// 0x8: Note Off.
    NoteOff = 0x8,
    /// 0x9: Note On.
    NoteOn = 0x9,
    /// 0xA: Polyphonic Key Pressure.
    PolyKeyPress = 0xA,
    /// 0xB: Control Change and Channel Mode.
    ControlChange = 0xB,
    /// 0xC: Program Change.
    ProgramChange = 0xC,
    /// 0xD: Channel Pressure.
    ChannelPressure = 0xD,
    /// 0xE: Pitch Bend Change.
    PitchBend = 0xE,
    /// 0xF: single byte.
    SingleByte = 0xF,
}

/// The number of valid MIDI_x bytes for each Code Index Number, by value 0x0 to 0xF
/// (Table 4-1, REQ-USB-EVT-002). CIN 0x0 and 0x1 have no size (REQ-USB-EVT-009).
pub const CIN_SIZE: [Option<u8>; 16] = [
    None,
    None,
    Some(2),
    Some(3),
    Some(3),
    Some(1),
    Some(2),
    Some(3),
    Some(3),
    Some(3),
    Some(3),
    Some(3),
    Some(2),
    Some(2),
    Some(3),
    Some(1),
];

impl Cin {
    /// The CIN of the low 4 bits of `value`. Every 4-bit value is valid.
    pub const fn from_bits(value: u8) -> Self {
        match value & 0xF {
            0x0 => Cin::Misc,
            0x1 => Cin::CableEvent,
            0x2 => Cin::SystemCommon2,
            0x3 => Cin::SystemCommon3,
            0x4 => Cin::SysExStart,
            0x5 => Cin::SingleByteCommonOrSysExEnd1,
            0x6 => Cin::SysExEnd2,
            0x7 => Cin::SysExEnd3,
            0x8 => Cin::NoteOff,
            0x9 => Cin::NoteOn,
            0xA => Cin::PolyKeyPress,
            0xB => Cin::ControlChange,
            0xC => Cin::ProgramChange,
            0xD => Cin::ChannelPressure,
            0xE => Cin::PitchBend,
            _ => Cin::SingleByte,
        }
    }

    /// The 4-bit value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The number of valid MIDI_x bytes, or `None` for CIN 0x0 and 0x1 (REQ-USB-EVT-002).
    pub const fn size(self) -> Option<u8> {
        CIN_SIZE[self as usize]
    }
}

/// One USB-MIDI Event Packet as 4 bytes in bus order (Figure 8, REQ-USB-EVT-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct EventPacket([u8; 4]);

impl EventPacket {
    /// A packet with `cable` (0 to 15), `cin` and the bytes MIDI_0 to MIDI_2. Gives
    /// [`EventPacketError::InvalidCable`] for a cable above 15.
    pub const fn new(cable: u8, cin: Cin, midi: [u8; 3]) -> Result<Self, EventPacketError> {
        if cable > 0xF {
            return Err(EventPacketError::InvalidCable);
        }
        Ok(EventPacket([
            cable << 4 | cin.into_bits(),
            midi[0],
            midi[1],
            midi[2],
        ]))
    }

    /// The packet of 4 bytes in bus order.
    pub const fn from_bytes(bytes: [u8; 4]) -> Self {
        EventPacket(bytes)
    }

    /// The 4 bytes in bus order.
    pub const fn to_bytes(self) -> [u8; 4] {
        self.0
    }

    /// The Cable Number, bits 7 to 4 of byte 0. 4 bits.
    pub const fn cable(self) -> u8 {
        self.0[0] >> 4
    }

    /// The Code Index Number, bits 3 to 0 of byte 0. 4 bits.
    pub const fn cin(self) -> Cin {
        Cin::from_bits(self.0[0])
    }

    /// MIDI_0, MIDI_1 and MIDI_2.
    pub const fn midi(self) -> [u8; 3] {
        [self.0[1], self.0[2], self.0[3]]
    }

    /// The valid MIDI_x bytes by the CIN size; empty for CIN 0x0 and 0x1 (REQ-USB-EVT-008,
    /// REQ-USB-EVT-009).
    pub fn valid(&self) -> &[u8] {
        let n = match self.cin().size() {
            Some(n) => n as usize,
            None => 0,
        };
        self.0.get(1..1 + n).unwrap_or(&[])
    }
}

/// Reader of the Event Packets of one MIDI endpoint, with `CABLES` cables (1 to 16).
///
/// It keeps one zmidi-io [`BytesToUmpCore`] for each cable, each with its own System Exclusive
/// state and running status (REQ-USB-EVT-011), and `X` MIDI 1.0 to 2.0 translators
/// ([`Midi1ToMidi2Group`], 128 bytes each): MIDI 2.0 Protocol output needs `X >= CABLES`
/// (cable n uses translator n); MIDI 1.0 Protocol output needs none, so the default `X` is 0
/// (OPT-011). No alloc.
///
/// [`Self::reset`] clears the cables in place, one cable at a time, with no copy of the whole
/// reader on the stack.
#[derive(Debug, Clone)]
pub struct PacketReader<const CABLES: usize = 1, const X: usize = 0> {
    /// The Protocol of the given UMPs (REQ-USB-EVT-023).
    protocol: StreamProtocol,
    /// One byte stream reader for each cable; cable n gives Group n (REQ-USB-EVT-015).
    cables: [BytesToUmpCore; CABLES],
    /// The translators of MIDI 2.0 Protocol output; translator n is for cable n.
    translators: [Midi1ToMidi2Group; X],
    /// True for each cable inside a System Exclusive message: after 0xF0 and before the next
    /// status byte that is not System Real Time (the same rule as zmidi-io `BytesToUmp`).
    in_sysex: [bool; CABLES],
}

impl<const CABLES: usize, const X: usize> Default for PacketReader<CABLES, X> {
    /// A reader that gives MIDI 1.0 Protocol UMPs (REQ-USB-EVT-023, choice U-20).
    fn default() -> Self {
        Self::build(StreamProtocol::Midi1)
    }
}

impl<const CABLES: usize, const X: usize> PacketReader<CABLES, X> {
    /// A reader that gives UMPs in `protocol`: [`StreamProtocol::Midi1`] gives Message Type
    /// 0x2, [`StreamProtocol::Midi2`] gives Message Type 0x4 through the zmidi-io translator
    /// (REQ-USB-EVT-023). `CABLES` must be 1 to 16 (checked at compile time).
    /// [`EventPacketError::NoTranslators`] for [`StreamProtocol::Midi2`] when `X < CABLES`.
    pub fn new(protocol: StreamProtocol) -> Result<Self, EventPacketError> {
        if matches!(protocol, StreamProtocol::Midi2) && X < CABLES {
            return Err(EventPacketError::NoTranslators);
        }
        Ok(Self::build(protocol))
    }

    fn build(protocol: StreamProtocol) -> Self {
        const {
            assert!(
                CABLES >= 1 && CABLES <= MAX_CABLES,
                "CABLES must be 1 to 16"
            )
        };
        PacketReader {
            protocol,
            cables: core::array::from_fn(|i| BytesToUmpCore::new(Group::from_bits(i as u8))),
            translators: [Midi1ToMidi2Group::new(); X],
            in_sysex: [false; CABLES],
        }
    }

    /// The Protocol of the given UMPs.
    pub fn protocol(&self) -> StreamProtocol {
        self.protocol
    }

    /// Clears the state of every cable (for example after a change of Alternate Setting).
    ///
    /// The cables are cleared in place, one at a time: the stack holds at most one cable
    /// reader, never a copy of the whole reader.
    pub fn reset(&mut self) {
        for (i, cable) in self.cables.iter_mut().enumerate() {
            *cable = BytesToUmpCore::new(Group::from_bits(i as u8));
        }
        for t in self.translators.iter_mut() {
            t.reset();
        }
        self.in_sysex = [false; CABLES];
    }

    /// True when `cable` is inside a received System Exclusive message: its bytes are held
    /// until the message ends. False for a cable that is not below `CABLES`.
    pub fn in_sysex(&self, cable: usize) -> bool {
        self.in_sysex.get(cable).copied().unwrap_or(false)
    }

    /// True when a cable is inside a received System Exclusive message ([`Self::in_sysex`]).
    pub fn any_in_sysex(&self) -> bool {
        self.in_sysex.iter().any(|&b| b)
    }

    /// Keeps the System Exclusive state of `cable` for one received byte, and gives the byte
    /// stream reader of the cable with its translator (MIDI 2.0 Protocol output only).
    /// `None` for a cable that is not below `CABLES`.
    fn parts(
        &mut self,
        cable: usize,
        byte: u8,
    ) -> Option<(&mut BytesToUmpCore, Option<&mut Midi1ToMidi2Group>)> {
        if let Some(flag) = self.in_sysex.get_mut(cable) {
            match byte {
                0xF0 => *flag = true,
                0x80..=0xF7 => *flag = false,
                _ => {}
            }
        }
        let translator = match self.protocol {
            StreamProtocol::Midi2 => self.translators.get_mut(cable),
            StreamProtocol::Midi1 => None,
        };
        self.cables.get_mut(cable).map(|c| (c, translator))
    }

    /// Gives one byte to the byte stream reader of `cable` and gives the UMPs it completes. The
    /// value form of zmidi-io: the DIN reader uses the same copy in one build (OPT-021).
    fn push(&mut self, cable: usize, byte: u8) -> Option<Pushed> {
        self.parts(cable, byte).map(|(c, t)| c.push_with(byte, t))
    }

    /// Reads `bytes`, the data of one USB transfer, as Event Packets in order
    /// (REQ-USB-EVT-020). The iterator gives each UMP as `Ok`, and then one
    /// [`EventPacketError::Tail`] when 1 to 3 bytes remain after the last whole packet. No part
    /// of a packet is kept for the next call.
    ///
    /// If the iterator is dropped before its end, the drop reads the rest of `bytes` into
    /// the cable state and discards the UMPs of that rest. Thus the state of each cable always
    /// agrees with the byte stream. [`Self::read_with`] is the callback form.
    pub fn read<'a>(&'a mut self, bytes: &'a [u8]) -> PacketRead<'a, CABLES, X> {
        let (packets, rest) = bytes.as_chunks::<PACKET_LEN>();
        let tail = rest.len();
        PacketRead {
            reader: self,
            packets: packets.iter(),
            tail,
            current: Current::EMPTY,
            pending: None,
        }
    }

    /// Gives the valid bytes of `packet` to the byte stream reader of its cable and gives the
    /// UMPs in order through `sink`. Ignores CIN 0x0 and 0x1 (REQ-USB-EVT-009) and a cable that
    /// is not below `CABLES` (REQ-USB-EVT-018).
    pub fn read_packet(&mut self, packet: EventPacket, mut sink: impl FnMut(Packet)) {
        let current = Current::of(packet, CABLES);
        for &b in current.bytes() {
            // The callback form of zmidi-io: no `Pushed` value is built (OPT-050).
            if let Some((c, t)) = self.parts(current.cable, b) {
                c.feed_with(b, t, |w| {
                    if let Ok(u) = Packet::from_words(w) {
                        sink(u);
                    }
                });
            }
        }
    }

    /// Reads all of `bytes` and gives the same items as [`Self::read`] to `sink`, in order.
    /// The callback form: a build that uses only this form links no iterator (OPT-021).
    pub fn read_with(
        &mut self,
        bytes: &[u8],
        mut sink: impl FnMut(Result<Packet, EventPacketError>),
    ) {
        let (packets, rest) = bytes.as_chunks::<PACKET_LEN>();
        for &p in packets {
            self.read_packet(EventPacket::from_bytes(p), |u| sink(Ok(u)));
        }
        if !rest.is_empty() {
            sink(Err(EventPacketError::Tail { len: rest.len() }));
        }
    }
}

/// The valid bytes of the packet in progress of a [`PacketRead`].
#[derive(Debug, Clone, Copy)]
struct Current {
    cable: usize,
    bytes: [u8; 3],
    len: usize,
    next: usize,
}

impl Current {
    const EMPTY: Self = Current {
        cable: 0,
        bytes: [0; 3],
        len: 0,
        next: 0,
    };

    /// The bytes that the reader gives for `packet`: the CIN size only, no compare with the
    /// status (REQ-USB-EVT-010); none for CIN 0x0 and 0x1 (REQ-USB-EVT-009) or for a cable
    /// at or above `cables` (REQ-USB-EVT-018).
    fn of(packet: EventPacket, cables: usize) -> Self {
        let cable = packet.cable() as usize;
        let len = match packet.cin().size() {
            Some(n) if cable < cables => n as usize,
            _ => 0,
        };
        Current {
            cable,
            bytes: packet.midi(),
            len,
            next: 0,
        }
    }

    fn bytes(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or(&[])
    }
}

/// Iterator over the UMPs of one transfer; see [`PacketReader::read`]. A drop before the end
/// reads the rest of the transfer and discards its UMPs.
#[derive(Debug)]
#[must_use = "the UMPs of the transfer are discarded if the iterator is not used"]
pub struct PacketRead<'a, const CABLES: usize, const X: usize = 0> {
    reader: &'a mut PacketReader<CABLES, X>,
    packets: core::slice::Iter<'a, [u8; PACKET_LEN]>,
    /// Number of bytes after the last whole packet; set to 0 when reported.
    tail: usize,
    current: Current,
    pending: Option<Pushed>,
}

impl<const CABLES: usize, const X: usize> Iterator for PacketRead<'_, CABLES, X> {
    type Item = Result<Packet, EventPacketError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ump) = self.pending.as_mut().and_then(Iterator::next) {
                return Some(Ok(ump));
            }
            if let Some(&b) = self.current.bytes().get(self.current.next) {
                self.current.next += 1;
                self.pending = self.reader.push(self.current.cable, b);
                continue;
            }
            if let Some(&p) = self.packets.next() {
                self.current = Current::of(EventPacket::from_bytes(p), CABLES);
                continue;
            }
            if self.tail > 0 {
                let len = self.tail;
                self.tail = 0;
                return Some(Err(EventPacketError::Tail { len }));
            }
            return None;
        }
    }
}

impl<const CABLES: usize, const X: usize> Drop for PacketRead<'_, CABLES, X> {
    /// Reads the rest of the transfer into the cable state and discards its UMPs, so the state
    /// agrees with the byte stream (a drop before the end never loses a byte of a message that
    /// continues in the next transfer). Bounded by the transfer length; never panics.
    fn drop(&mut self) {
        for _ in self.by_ref() {}
    }
}

/// Options of the [`PacketWriter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WriterOptions {
    /// Unparsed mode: each MIDI 1.0 byte in its own CIN 0xF packet. Default off: the parsed
    /// mode (REQ-USB-EVT-012, choice U-22).
    pub unparsed: bool,
}

/// The System Exclusive state of one cable of the writer (REQ-USB-EVT-011, REQ-USB-EVT-024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CableTx {
    /// True inside a System Exclusive message.
    in_sysex: bool,
    /// System Exclusive bytes that do not yet fill a CIN 0x4 packet.
    held: [u8; 2],
    /// Number of valid bytes in `held`, 0 to 2.
    held_len: usize,
}

/// Writer of the Event Packets of one MIDI endpoint, with `CABLES` cables (1 to 16).
#[derive(Debug, Clone)]
pub struct PacketWriter<const CABLES: usize = 1> {
    options: WriterOptions,
    cables: [CableTx; CABLES],
}

impl<const CABLES: usize> Default for PacketWriter<CABLES> {
    /// A writer in the parsed mode.
    fn default() -> Self {
        Self::new(WriterOptions::default())
    }
}

impl<const CABLES: usize> PacketWriter<CABLES> {
    /// A writer with `options`. `CABLES` must be 1 to 16 (checked at compile time).
    pub fn new(options: WriterOptions) -> Self {
        const {
            assert!(
                CABLES >= 1 && CABLES <= MAX_CABLES,
                "CABLES must be 1 to 16"
            )
        };
        PacketWriter {
            options,
            cables: [CableTx::default(); CABLES],
        }
    }

    /// The options of the writer.
    pub fn options(&self) -> WriterOptions {
        self.options
    }

    /// Clears the held System Exclusive bytes of every cable; they are not written. In place,
    /// with no copy of the writer on the stack.
    pub fn reset(&mut self) {
        for cable in self.cables.iter_mut() {
            *cable = CableTx::default();
        }
    }

    /// The number of held System Exclusive bytes of `cable` (0 to 2), or 0 for a cable that
    /// is not below `CABLES` (REQ-USB-EVT-024).
    pub fn held(&self, cable: usize) -> usize {
        self.cables.get(cable).map_or(0, |c| c.held_len)
    }

    /// Writes the Event Packets of the UMP `ump` into `out`; see [`PacketWriter::write`].
    pub fn write_ump(&mut self, ump: &Ump, out: &mut [u8]) -> Result<usize, EventPacketError> {
        let mut words = [0u32; 4];
        match ump.write(&mut words) {
            Ok(n) => self.write(words.get(..n).unwrap_or(&[]), out),
            Err(_) => Ok(0),
        }
    }

    /// Writes the Event Packets of the UMP at the start of `words` into the start of `out`
    /// and gives the number of bytes (a multiple of 4).
    ///
    /// - The bytes are those of [`UmpToBytes`](zmidi_io::UmpToBytes) with running status off; a UMP with no byte
    ///   gives no packet (REQ-USB-EVT-013, REQ-USB-EVT-014). Group n goes on cable n; a Group
    ///   at or above `CABLES` gives no packet (REQ-USB-EVT-016, REQ-USB-EVT-018).
    /// - Parsed CINs (REQ-USB-EVT-003 to REQ-USB-EVT-006), System Real Time as CIN 0xF
    ///   (REQ-USB-EVT-005, REQ-USB-EVT-007), held System Exclusive bytes and the added 0xF7
    ///   (REQ-USB-EVT-024), zero padding (REQ-USB-EVT-008), never CIN 0x0 or 0x1
    ///   (REQ-USB-EVT-009).
    /// - All packets of the UMP or none: [`EventPacketError::Overflow`] when `out` is too small,
    ///   with nothing written and no state change (REQ-USB-EVT-021).
    ///
    /// Never panics.
    pub fn write(&mut self, words: &[u32], out: &mut [u8]) -> Result<usize, EventPacketError> {
        // REQ-USB-EVT-021: the most bytes of one UMP. When `out` has room for them, the packets
        // go straight into it; otherwise into a buffer first, so that an overflow writes nothing.
        let max = if self.options.unparsed {
            MAX_UNPARSED_PACKETS_PER_UMP
        } else {
            MAX_PACKETS_PER_UMP
        };
        if out.len() >= max * PACKET_LEN {
            self.write_to(words, out)
        } else {
            self.write_small(words, out)
        }
    }

    /// [`PacketWriter::write`] into a small `out`: the packets go to a buffer first, and the
    /// state of the cables comes back on an overflow. Not inlined: the fast path clears no
    /// buffer (OPT-033).
    #[inline(never)]
    fn write_small(&mut self, words: &[u32], out: &mut [u8]) -> Result<usize, EventPacketError> {
        let mut buf = [0u8; MAX_UNPARSED_PACKETS_PER_UMP * PACKET_LEN];
        let saved = self.cables;
        let n = self.write_to(words, &mut buf)?;
        match out.get_mut(..n) {
            Some(dst) => {
                dst.copy_from_slice(&buf[..n]);
                Ok(n)
            }
            None => {
                self.cables = saved;
                Err(EventPacketError::Overflow)
            }
        }
    }

    /// [`PacketWriter::write`] into an `out` with room for the most bytes of one UMP.
    fn write_to(&mut self, words: &[u32], out: &mut [u8]) -> Result<usize, EventPacketError> {
        let Some(&w0) = words.first() else {
            return Ok(0);
        };
        // The UmpToBytes rule: a UMP shorter than its Table 4 size gives no byte.
        if words.len() < MessageType::from_bits((w0 >> 28) as u8).word_count() {
            return Ok(0);
        }
        // Group field, bits 27 to 24 of word 0, for every Message Type with MIDI 1.0 bytes.
        let cable = ((w0 >> 24) & 0xF) as usize;
        let Some(&state) = self.cables.get(cable) else {
            return Ok(0);
        };
        // Each UMP goes straight to its packets, with no MIDI 1.0 byte stream between them
        // (OPT-050 option 1). The bytes are the bytes of UmpToBytes with running
        // status off; the test `prop_writer_matches_byte_stream_writer` compares the two.
        let mut tx = Tx {
            state,
            packets: Packets::new(cable as u8, out),
            unparsed: self.options.unparsed,
        };
        let status = (w0 >> 16) as u8;
        match w0 >> 28 {
            // System Common and System Real Time with a MIDI 1.0 form (M2-104 7.6; no 0xF4,
            // 0xF5, 0xF9 or 0xFD).
            0x1 if matches!(status, 0xF1..=0xF3 | 0xF6 | 0xF8 | 0xFA..=0xFC | 0xFE | 0xFF) => {
                tx.message(w0)
            }
            0x2 if (0x80..=0xEF).contains(&status) => tx.message(w0),
            // MIDI 2.0 Channel Voice: the MIDI 1.0 messages of M2-104 D.2 (zmidi-ump).
            0x4 => {
                if let Some(m) = midi2_to_midi1(words) {
                    for &w in m.as_slice() {
                        tx.message(w);
                    }
                }
            }
            0x3 => tx.sysex7(w0, words.get(1).copied().unwrap_or(0)),
            // Utility, Data 128, Flex Data, UMP Stream and unknown packets (D.2.9).
            _ => {}
        }
        let len = tx.packets.len * PACKET_LEN;
        let state = tx.state;
        if let Some(slot) = self.cables.get_mut(cable) {
            *slot = state;
        }
        Ok(len)
    }
}

/// The packets of one UMP, before the length check. No alloc.
struct Packets<'a> {
    cable: u8,
    /// The packets in bus order.
    buf: &'a mut [u8],
    /// The number of packets in `buf`.
    len: usize,
}

impl<'a> Packets<'a> {
    fn new(cable: u8, buf: &'a mut [u8]) -> Self {
        Packets {
            cable: cable & 0xF,
            buf,
            len: 0,
        }
    }

    /// Adds a packet with the 4-bit `cin` and the bytes MIDI_0 to MIDI_2; the caller gives
    /// 0x00 in each unused byte (REQ-USB-EVT-008).
    #[inline(never)]
    fn put(&mut self, cin: u8, midi: [u8; 3]) {
        let at = self.len * PACKET_LEN;
        if let Some(slot) = self.buf.get_mut(at..at + PACKET_LEN) {
            slot.copy_from_slice(&[self.cable << 4 | cin, midi[0], midi[1], midi[2]]);
            self.len += 1;
        }
    }

    #[cfg(test)]
    fn as_slice(&self) -> &[u8] {
        self.buf.get(..self.len * PACKET_LEN).unwrap_or(&[])
    }
}

/// Number of data bytes after a status byte that is not System Exclusive (M1 Tables I and V).
fn data_len(status: u8) -> usize {
    match status {
        0x80..=0xBF | 0xE0..=0xEF | 0xF2 => 2,
        0xC0..=0xDF | 0xF1 | 0xF3 => 1,
        _ => 0,
    }
}

/// The packets of one UMP for one cable, and the System Exclusive state of that cable after
/// them.
///
/// Each message other than System Exclusive starts with its status byte (no running status,
/// REQ-USB-EVT-013). The data bytes of a SysEx7 Continue or End UMP with no open System
/// Exclusive message on the cable are dropped (REQ-USB-EVT-029): as CIN 0x4 packets they would
/// end up as running status data at the reader.
struct Tx<'a> {
    state: CableTx,
    packets: Packets<'a>,
    unparsed: bool,
}

impl Tx<'_> {
    /// One packet with `cin` and the first `n` of `midi` (the others are 0x00); in the unparsed
    /// mode, one CIN 0xF packet for each of the `n` bytes (REQ-USB-EVT-012).
    fn put(&mut self, cin: u8, midi: [u8; 3], n: usize) {
        if self.unparsed {
            for &b in midi.iter().take(n) {
                self.packets.put(0xF, [b, 0, 0]);
            }
        } else {
            self.packets.put(cin, midi);
        }
    }

    /// Ends an open System Exclusive message with 0xF7 and the held bytes: CIN 0x5, 0x6 or
    /// 0x7 (REQ-USB-EVT-006, REQ-USB-EVT-024, choice U-23). The unparsed mode holds no byte.
    fn end_sysex(&mut self) {
        if !self.state.in_sysex {
            return;
        }
        let [h0, h1] = self.state.held;
        let (midi, held) = match self.state.held_len {
            0 => ([0xF7, 0, 0], 0),
            1 => ([h0, 0xF7, 0], 1),
            _ => ([h0, h1, 0xF7], 2),
        };
        self.put(0x5 + held as u8, midi, held + 1);
        self.state = CableTx::default();
    }

    /// One MIDI 1.0 message of a Message Type 0x1 or 0x2 word. The caller gives only a status
    /// with a MIDI 1.0 form: 0x80 to 0xEF, 0xF1 to 0xF3, 0xF6, 0xF8, 0xFA to 0xFC, 0xFE, 0xFF.
    fn message(&mut self, w: u32) {
        let status = (w >> 16) as u8;
        // REQ-USB-EVT-003 to REQ-USB-EVT-005.
        let cin = match status {
            0x80..=0xEF => status >> 4,
            0xF1 | 0xF3 => 0x2,
            0xF2 => 0x3,
            0xF6 => 0x5,
            _ => 0xF,
        };
        // System Real Time goes out at once; the held bytes stay (REQ-USB-EVT-007).
        if status < 0xF8 {
            self.end_sysex();
        }
        let n = data_len(status);
        let d1 = if n >= 1 { (w >> 8) as u8 & 0x7F } else { 0 };
        let d2 = if n >= 2 { w as u8 & 0x7F } else { 0 };
        self.put(cin, [status, d1, d2], 1 + n);
    }

    /// One SysEx7 UMP (M2-104 7.7): 0xF0 first for Complete and Start, the data bytes, 0xF7
    /// last for Complete and End (REQ-USB-EVT-006, REQ-USB-EVT-024).
    fn sysex7(&mut self, w0: u32, w1: u32) {
        let form = (w0 >> 20) & 0xF;
        let n = ((w0 >> 16) & 0xF) as usize;
        if form > 3 || n > 6 {
            return;
        }
        if form <= 1 {
            self.end_sysex();
            self.state.in_sysex = true;
            if self.unparsed {
                self.packets.put(0xF, [0xF0, 0, 0]);
            } else {
                self.state.held = [0xF0, 0];
                self.state.held_len = 1;
            }
        }
        // Data bytes 1 and 2 are bits 15 to 0 of word 0, bytes 3 to 6 are word 1.
        let data = (u64::from(w0) << 32) | u64::from(w1);
        for i in 0..n {
            let b = (data >> (40 - 8 * i)) as u8 & 0x7F;
            if !self.state.in_sysex {
                // REQ-USB-EVT-029.
            } else if self.unparsed {
                self.packets.put(0xF, [b, 0, 0]);
            } else if self.state.held_len >= 2 {
                let [h0, h1] = self.state.held;
                self.packets.put(0x4, [h0, h1, b]);
                self.state.held_len = 0;
            } else if let Some(slot) = self.state.held.get_mut(self.state.held_len) {
                *slot = b;
                self.state.held_len += 1;
            }
        }
        if form == 0 || form == 3 {
            self.end_sysex();
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    // REQ-USB-EVT-018: a cable at or above the cable count gives no byte; the writer gives
    // back its options.
    #[test]
    fn cable_count_and_writer_options() {
        let mut r = PacketReader::<1>::new(StreamProtocol::Midi1).unwrap();
        let mut got = std::vec::Vec::new();
        r.read_packet(EventPacket::from_bytes([0x19, 0x90, 0x3C, 0x40]), |p| {
            got.push(p)
        });
        assert!(got.is_empty(), "cable 1 of 1");
        r.read_packet(EventPacket::from_bytes([0x09, 0x90, 0x3C, 0x40]), |p| {
            got.push(p)
        });
        assert_eq!(got.len(), 1);
        let o = WriterOptions { unparsed: true };
        assert_eq!(PacketWriter::<1>::new(o).options(), o);
    }
    use proptest::prelude::*;
    use std::vec::Vec;
    use zmidi_io::{BytesToUmp, UmpToBytes};

    // The writer before OPT-050 option 1: the bytes of UmpToBytes with running
    // status off, cut into packets one byte at a time. The oracle of
    // `prop_writer_matches_byte_stream_writer` (REQ-USB-EVT-013, REQ-USB-EVT-014).
    impl Packets<'_> {
        /// Adds a packet with `cin` and 1 to 3 `bytes`; the unused bytes are 0x00
        /// (REQ-USB-EVT-008).
        fn push(&mut self, cin: Cin, bytes: &[u8]) {
            let mut midi = [0u8; 3];
            for (to, from) in midi.iter_mut().zip(bytes) {
                *to = *from;
            }
            self.put(cin.into_bits(), midi);
        }

        /// Adds one CIN 0xF packet for each byte (REQ-USB-EVT-012).
        fn push_single(&mut self, bytes: &[u8]) {
            for &b in bytes {
                self.push(Cin::SingleByte, &[b]);
            }
        }
    }

    /// The CIN of a whole MIDI 1.0 message that is not System Exclusive or System Real Time
    /// (REQ-USB-EVT-003, REQ-USB-EVT-004), or `None` for an undefined status.
    fn message_cin(status: u8) -> Option<Cin> {
        match status {
            0x80..=0xEF => Some(Cin::from_bits(status >> 4)),
            0xF1 | 0xF3 => Some(Cin::SystemCommon2),
            0xF2 => Some(Cin::SystemCommon3),
            0xF6 => Some(Cin::SingleByteCommonOrSysExEnd1),
            _ => None,
        }
    }

    /// The CIN of the last System Exclusive packet with `n` bytes, 0xF7 included
    /// (REQ-USB-EVT-006).
    fn sysex_end_cin(n: usize) -> Cin {
        match n {
            1 => Cin::SingleByteCommonOrSysExEnd1,
            2 => Cin::SysExEnd2,
            _ => Cin::SysExEnd3,
        }
    }

    /// Ends an open System Exclusive message with 0xF7 and the held bytes (REQ-USB-EVT-024,
    /// choice U-23).
    fn end_sysex(state: &mut CableTx, packets: &mut Packets<'_>, unparsed: bool) {
        if !state.in_sysex {
            return;
        }
        if unparsed {
            packets.push_single(&[0xF7]);
        } else {
            let mut group = [0u8; 3];
            let held = state.held_len.min(2);
            group[..held].copy_from_slice(&state.held[..held]);
            group[held] = 0xF7;
            packets.push(sysex_end_cin(held + 1), &group[..held + 1]);
        }
        *state = CableTx::default();
    }

    struct ByteWriter<const CABLES: usize> {
        unparsed: bool,
        bytes: UmpToBytes,
        cables: [CableTx; CABLES],
    }

    impl<const CABLES: usize> ByteWriter<CABLES> {
        fn new(unparsed: bool) -> Self {
            ByteWriter {
                unparsed,
                bytes: UmpToBytes::new(false),
                cables: [CableTx::default(); CABLES],
            }
        }

        fn write(&mut self, words: &[u32]) -> Vec<u8> {
            let Some(&w0) = words.first() else {
                return Vec::new();
            };
            let cable = ((w0 >> 24) & 0xF) as usize;
            let Some(state) = self.cables.get_mut(cable) else {
                return Vec::new();
            };
            let mut buf = [0u8; 64];
            let mut packets = Packets::new(cable as u8, &mut buf);
            let mut cut = Packetizer::new(state, &mut packets, self.unparsed);
            self.bytes.write_each(words, |b| cut.byte(b));
            packets.as_slice().to_vec()
        }
    }

    /// Cuts the MIDI 1.0 bytes of one UMP into Event Packets for one cable, one byte at a time.
    ///
    /// The bytes have no running status (REQ-USB-EVT-013), so each message other than System
    /// Exclusive starts with its status byte. Data bytes with no message in progress (a SysEx7
    /// Continue or End UMP with no open System Exclusive message on the cable) are dropped: as CIN
    /// 0x4 packets they would end up as running status data at the reader.
    struct Packetizer<'a, 'b> {
        state: &'a mut CableTx,
        packets: &'a mut Packets<'b>,
        unparsed: bool,
        /// The message in progress, other than System Exclusive.
        msg: [u8; 3],
        msg_len: usize,
        /// Length of the message in progress; 0 when there is none.
        msg_need: usize,
    }

    impl<'a, 'b> Packetizer<'a, 'b> {
        fn new(state: &'a mut CableTx, packets: &'a mut Packets<'b>, unparsed: bool) -> Self {
            Packetizer {
                state,
                packets,
                unparsed,
                msg: [0; 3],
                msg_len: 0,
                msg_need: 0,
            }
        }

        fn byte(&mut self, b: u8) {
            let unparsed = self.unparsed;
            match b {
                // System Real Time: its own packet at once; the held bytes stay (REQ-USB-EVT-005,
                // REQ-USB-EVT-007).
                0xF8..=0xFF => self.packets.push(Cin::SingleByte, &[b]),
                0xF0 => {
                    end_sysex(self.state, self.packets, unparsed);
                    self.msg_need = 0;
                    self.state.in_sysex = true;
                    if unparsed {
                        self.packets.push_single(&[b]);
                    } else {
                        self.state.held = [b, 0];
                        self.state.held_len = 1;
                    }
                }
                0xF7 => {
                    // A lone 0xF7 with no open System Exclusive message is dropped.
                    end_sysex(self.state, self.packets, unparsed);
                    self.msg_need = 0;
                }
                0x80..=0xF6 => {
                    end_sysex(self.state, self.packets, unparsed);
                    self.msg[0] = b;
                    self.msg_len = 1;
                    self.msg_need = 1 + data_len(b);
                    if self.msg_need == 1 {
                        // Tune Request; 0xF4 and 0xF5 are never given by UmpToBytes.
                        if let Some(cin) = message_cin(b) {
                            if unparsed {
                                self.packets.push_single(&[b]);
                            } else {
                                self.packets.push(cin, &[b]);
                            }
                        }
                        self.msg_need = 0;
                    }
                }
                _ => {
                    if self.state.in_sysex {
                        if unparsed {
                            self.packets.push_single(&[b]);
                        } else if self.state.held_len >= 2 {
                            self.packets.push(
                                Cin::SysExStart,
                                &[self.state.held[0], self.state.held[1], b],
                            );
                            self.state.held_len = 0;
                        } else {
                            self.state.held[self.state.held_len] = b;
                            self.state.held_len += 1;
                        }
                    } else if self.msg_need > 0 && self.msg_len < self.msg_need {
                        self.msg[self.msg_len] = b;
                        self.msg_len += 1;
                        if self.msg_len == self.msg_need {
                            let bytes = &self.msg[..self.msg_len];
                            match message_cin(self.msg[0]) {
                                Some(_) if unparsed => self.packets.push_single(bytes),
                                Some(cin) => self.packets.push(cin, bytes),
                                None => {}
                            }
                            self.msg_need = 0;
                        }
                    }
                }
            }
        }
    }

    /// The words of a UMP.
    fn words(p: Packet) -> Vec<u32> {
        p.words().to_vec()
    }

    /// Word lists of the UMPs (errors as an empty list) that `reader` gives for `bytes`.
    fn read<const C: usize, const X: usize>(
        reader: &mut PacketReader<C, X>,
        bytes: &[u8],
    ) -> Vec<Vec<u32>> {
        reader
            .read(bytes)
            .map(|r| r.map(words).unwrap_or_default())
            .collect()
    }

    fn read1(bytes: &[u8]) -> Vec<Vec<u32>> {
        read(&mut PacketReader::<16>::default(), bytes)
    }

    /// The bytes that `writer` gives for each UMP, in order.
    fn write<const C: usize>(writer: &mut PacketWriter<C>, umps: &[&[u32]]) -> Vec<u8> {
        let mut out = Vec::new();
        for u in umps {
            let mut buf = [0u8; 64];
            let n = writer.write(u, &mut buf).unwrap();
            assert_eq!(n % 4, 0);
            out.extend_from_slice(&buf[..n]);
        }
        out
    }

    fn write16(umps: &[&[u32]]) -> Vec<u8> {
        write(&mut PacketWriter::<16>::default(), umps)
    }

    #[test]
    fn packet_holds_cable_high_and_cin_low_in_bus_order() {
        // REQ-USB-EVT-001: Figure 8; Table 4-2 "19 9n kk vv".
        let p = EventPacket::new(1, Cin::NoteOn, [0x90, 0x3C, 0x64]).unwrap();
        assert_eq!(p.to_bytes(), [0x19, 0x90, 0x3C, 0x64]);
        assert_eq!(p.cable(), 1);
        assert_eq!(p.cin(), Cin::NoteOn);
        assert_eq!(p.midi(), [0x90, 0x3C, 0x64]);
        assert_eq!(EventPacket::from_bytes([0xAB, 1, 2, 3]).cable(), 0xA);
        assert_eq!(
            EventPacket::new(16, Cin::NoteOn, [0; 3]),
            Err(EventPacketError::InvalidCable)
        );
    }

    #[test]
    fn cin_size_table_matches_table_4_1() {
        // REQ-USB-EVT-002.
        let expected = [0, 0, 2, 3, 3, 1, 2, 3, 3, 3, 3, 3, 2, 2, 3, 1];
        for (v, &n) in expected.iter().enumerate() {
            let cin = Cin::from_bits(v as u8);
            assert_eq!(cin.into_bits(), v as u8);
            assert_eq!(cin.size().unwrap_or(0), n);
        }
        assert_eq!(Cin::Misc.size(), None);
        assert_eq!(Cin::CableEvent.size(), None);
        assert_eq!(
            EventPacket::from_bytes([0x0C, 0xC5, 0x01, 0x7F]).valid(),
            &[0xC5, 0x01]
        );
        assert!(EventPacket::from_bytes([0x01, 1, 2, 3]).valid().is_empty());
    }

    #[test]
    fn writer_channel_messages_use_status_high_nibble_as_cin() {
        // REQ-USB-EVT-003, REQ-USB-EVT-013.
        let cases: [(u32, [u8; 4]); 8] = [
            (0x2083_3C40, [0x08, 0x83, 0x3C, 0x40]),
            (0x2093_3C40, [0x09, 0x93, 0x3C, 0x40]),
            (0x20A3_3C40, [0x0A, 0xA3, 0x3C, 0x40]),
            (0x20B3_0740, [0x0B, 0xB3, 0x07, 0x40]),
            (0x20B3_7B00, [0x0B, 0xB3, 0x7B, 0x00]), // Channel Mode: All Notes Off
            (0x20C3_0500, [0x0C, 0xC3, 0x05, 0x00]),
            (0x20D3_4000, [0x0D, 0xD3, 0x40, 0x00]),
            (0x20E3_0040, [0x0E, 0xE3, 0x00, 0x40]),
        ];
        for (ump, packet) in cases {
            assert_eq!(write16(&[&[ump]]), packet);
        }
    }

    #[test]
    fn writer_system_common_uses_cin_2_3_and_5() {
        // REQ-USB-EVT-004.
        assert_eq!(write16(&[&[0x10F1_2300]]), [0x02, 0xF1, 0x23, 0x00]);
        assert_eq!(write16(&[&[0x10F3_0500]]), [0x02, 0xF3, 0x05, 0x00]);
        assert_eq!(write16(&[&[0x10F2_0102]]), [0x03, 0xF2, 0x01, 0x02]);
        assert_eq!(write16(&[&[0x10F6_0000]]), [0x05, 0xF6, 0x00, 0x00]);
    }

    #[test]
    fn writer_real_time_uses_cin_f_with_zero_padding() {
        // REQ-USB-EVT-005, REQ-USB-EVT-008 (writer clause).
        for status in [0xF8u8, 0xFA, 0xFB, 0xFC, 0xFE, 0xFF] {
            let ump = 0x1500_0000 | u32::from(status) << 16;
            assert_eq!(write16(&[&[ump]]), [0x5F, status, 0x00, 0x00]);
        }
    }

    #[test]
    fn writer_cuts_sysex_into_three_byte_groups_across_umps() {
        // REQ-USB-EVT-006, REQ-USB-EVT-024: F0 1..13 F7 in Start, Continue, End UMPs.
        let out = write16(&[
            &[0x3016_0102, 0x0304_0506],
            &[0x3026_0708, 0x090A_0B0C],
            &[0x3031_0D00, 0],
        ]);
        assert_eq!(
            out,
            [0x04, 0xF0, 1, 2, 0x04, 3, 4, 5, 0x04, 6, 7, 8, 0x04, 9, 10, 11, 0x07, 12, 13, 0xF7]
        );
    }

    #[test]
    fn writer_holds_one_or_two_sysex_bytes_per_cable() {
        // REQ-USB-EVT-024, REQ-USB-EVT-011: the Start UMP F0 01 leaves 2 held bytes.
        let mut w = PacketWriter::<2>::default();
        assert!(write(&mut w, &[&[0x3111_0100, 0]]).is_empty());
        assert_eq!(w.held(1), 2);
        assert_eq!(w.held(0), 0);
        // A Continue UMP with 2 bytes: one CIN 0x4 packet, one byte held.
        assert_eq!(
            write(&mut w, &[&[0x3122_0203, 0]]),
            [0x14, 0xF0, 0x01, 0x02]
        );
        assert_eq!(w.held(1), 1);
        // The End UMP: 03 04 F7.
        assert_eq!(
            write(&mut w, &[&[0x3131_0400, 0]]),
            [0x17, 0x03, 0x04, 0xF7]
        );
        assert_eq!(w.held(1), 0);
    }

    #[test]
    fn writer_real_time_inside_sysex_goes_out_at_once() {
        // REQ-USB-EVT-007: the clock is its own packet; the held bytes F0 01 stay.
        let mut w = PacketWriter::<4>::default();
        assert!(write(&mut w, &[&[0x3211_0100, 0]]).is_empty());
        assert_eq!(write(&mut w, &[&[0x12F8_0000]]), [0x2F, 0xF8, 0, 0]);
        assert_eq!(w.held(2), 2);
        assert_eq!(
            write(&mut w, &[&[0x3231_0200, 0]]),
            [0x24, 0xF0, 0x01, 0x02, 0x25, 0xF7, 0, 0]
        );
    }

    #[test]
    fn writer_ends_interrupted_sysex_with_f7() {
        // REQ-USB-EVT-024, choice U-23: 0, 1 and 2 held bytes give CIN 0x7 ... 0x5.
        let note = 0x2090_3C64u32;
        // 1 held byte (F0 01 02 03: 1 packet and 03 held) -> CIN 0x6 (03 F7).
        let mut w = PacketWriter::<1>::default();
        assert_eq!(
            write(&mut w, &[&[0x3013_0102, 0x0300_0000], &[note]]),
            [0x04, 0xF0, 1, 2, 0x06, 0x03, 0xF7, 0x00, 0x09, 0x90, 0x3C, 0x64]
        );
        // 2 held bytes (F0 01) -> CIN 0x7 (F0 01 F7).
        let mut w = PacketWriter::<1>::default();
        assert_eq!(
            write(&mut w, &[&[0x3011_0100, 0], &[note]]),
            [0x07, 0xF0, 0x01, 0xF7, 0x09, 0x90, 0x3C, 0x64]
        );
        // 0 held bytes (F0 01 02) -> CIN 0x5 (F7 00 00).
        let mut w = PacketWriter::<1>::default();
        assert_eq!(
            write(&mut w, &[&[0x3012_0102, 0], &[0x10F6_0000]]),
            [0x04, 0xF0, 1, 2, 0x05, 0xF7, 0, 0, 0x05, 0xF6, 0, 0]
        );
        // A new Start ends the old message too.
        let mut w = PacketWriter::<1>::default();
        assert_eq!(
            write(&mut w, &[&[0x3011_0100, 0], &[0x3001_0200, 0]]),
            [0x07, 0xF0, 0x01, 0xF7, 0x07, 0xF0, 0x02, 0xF7]
        );
    }

    #[test]
    fn writer_drops_sysex_data_with_no_open_message() {
        // REQ-USB-EVT-014: a Continue or End UMP with no Start on the cable gives no packet.
        assert!(write16(&[&[0x3022_0102, 0]]).is_empty());
        assert!(write16(&[&[0x3032_0102, 0]]).is_empty());
    }

    // REQ-USB-EVT-029: a Continue or End UMP with no open message gives no packet, and the
    // End UMP with 0 data bytes (UmpToBytes gives a lone 0xF7) gives no packet either.
    #[test]
    fn writer_drops_lone_f7_and_data_with_no_open_message() {
        assert!(write16(&[&[0x3030_0000, 0]]).is_empty());
        assert!(write16(&[&[0x3530_0000, 0]]).is_empty());
        assert!(write16(&[&[0x3022_0102, 0], &[0x3032_0304, 0]]).is_empty());
        // After a real SysEx, the lone End still gives nothing.
        let mut w = PacketWriter::<1>::default();
        assert_eq!(
            write(&mut w, &[&[0x3001_7E00, 0]]),
            [0x07, 0xF0, 0x7E, 0xF7]
        );
        assert!(write(&mut w, &[&[0x3030_0000, 0]]).is_empty());
        assert_eq!(w.held(0), 0);
    }

    #[test]
    fn prop_writer_never_writes_cin_0_or_1() {
        // REQ-USB-EVT-009 (writer clause), REQ-USB-EVT-008: every packet of any UMP has CIN
        // 0x2 to 0xF and zero padding.
        proptest!(|(w0 in any::<u32>(), w1 in any::<u32>(), w2 in any::<u32>(), w3 in any::<u32>(), unparsed in any::<bool>())| {
            let mut w = PacketWriter::<16>::new(WriterOptions { unparsed });
            let mut buf = [0u8; 64];
            for _ in 0..2 {
                let n = w.write(&[w0, w1, w2, w3], &mut buf).unwrap();
                for &p in buf[..n].as_chunks::<4>().0 {
                    let p = EventPacket::from_bytes(p);
                    let size = p.cin().size();
                    prop_assert!(size.is_some());
                    let size = size.unwrap() as usize;
                    prop_assert!(p.midi()[size..].iter().all(|&b| b == 0));
                }
            }
        });
    }

    /// A UMP with a Message Type that has MIDI 1.0 bytes most of the time, on Group 0 to 2,
    /// with 1 to 4 words (some shorter than the Message Type needs).
    fn ump_stream() -> impl Strategy<Value = Vec<Vec<u32>>> {
        let ump = (
            prop_oneof![
                Just(1u32),
                Just(2),
                Just(3),
                Just(3),
                Just(4),
                Just(0),
                Just(5),
                any::<u32>()
            ],
            0u32..3,
            any::<u32>(),
            proptest::array::uniform3(any::<u32>()),
            1usize..=4,
            prop_oneof![4 => 0u32..4, 1 => 0u32..16],
            prop_oneof![4 => 0u32..7, 1 => 0u32..16],
        )
            .prop_map(|(mt, group, low, rest, n, form, count)| {
                let mut low = low & 0x00FF_FFFF;
                match mt {
                    // A System status 0xF0 to 0xFF.
                    1 => low |= 0x00F0_0000,
                    // A status byte 0x80 to 0xFF.
                    2 => low |= 0x0080_0000,
                    // Mostly a valid SysEx7 status and byte count (M2-104 7.7).
                    3 => low = (form << 20) | (count << 16) | (low & 0xFFFF),
                    _ => {}
                }
                let w0 = ((mt & 0xF) << 28) | (group << 24) | low;
                let mut w = std::vec![w0, rest[0], rest[1], rest[2]];
                w.truncate(n);
                w
            });
        proptest::collection::vec(ump, 0..24)
    }

    #[test]
    fn prop_writer_matches_byte_stream_writer() {
        // REQ-USB-EVT-013, REQ-USB-EVT-014: the packets are the packets of the UmpToBytes
        // bytes (running status off), for each UMP of a stream, with the held bytes of each
        // cable in between; both modes (REQ-USB-EVT-012).
        proptest!(|(umps in ump_stream(), unparsed in any::<bool>())| {
            let mut w = PacketWriter::<2>::new(WriterOptions { unparsed });
            let mut oracle = ByteWriter::<2>::new(unparsed);
            for u in &umps {
                let mut buf = [0u8; 64];
                let n = w.write(u, &mut buf).unwrap();
                prop_assert_eq!(&buf[..n], &oracle.write(u)[..], "UMP {:08X?}", u);
                for c in 0..2 {
                    prop_assert_eq!(w.held(c), oracle.cables[c].held_len);
                    prop_assert_eq!(w.cables[c], oracle.cables[c]);
                }
            }
        });
    }

    #[test]
    fn prop_writer_small_out_matches_large_out() {
        // REQ-USB-EVT-021, OPT-033: an `out` with room for the most bytes of one UMP takes the
        // packets straight; a smaller `out` goes through a buffer. Both give the same bytes, the
        // same cable state, and on an overflow no bytes and no state change.
        proptest!(|(umps in ump_stream(), unparsed in any::<bool>(), room in 0usize..20)| {
            let mut big = PacketWriter::<2>::new(WriterOptions { unparsed });
            let mut small = PacketWriter::<2>::new(WriterOptions { unparsed });
            for u in &umps {
                let mut a = [0u8; 64];
                let n = big.write(u, &mut a).unwrap();
                let mut b = [0xAAu8; 20];
                let before = small.cables;
                match small.write(u, &mut b[..room]) {
                    Ok(m) => prop_assert_eq!(&b[..m], &a[..n]),
                    Err(EventPacketError::Overflow) => {
                        prop_assert!(n > room);
                        prop_assert!(b.iter().all(|&x| x == 0xAA));
                        prop_assert_eq!(small.cables, before);
                        // Write it again with room, so both writers stay in step.
                        let mut c = [0u8; 64];
                        let m = small.write(u, &mut c).unwrap();
                        prop_assert_eq!(&c[..m], &a[..n]);
                    }
                    Err(e) => prop_assert!(false, "{:?}", e),
                }
                prop_assert_eq!(small.cables, big.cables);
            }
        });
    }

    #[test]
    fn writer_unparsed_mode_writes_one_byte_per_packet() {
        // REQ-USB-EVT-012, choice U-22: parsed by default; CIN 0xF per byte as an option.
        assert!(!PacketWriter::<1>::default().options().unparsed);
        let mut w = PacketWriter::<1>::new(WriterOptions { unparsed: true });
        assert_eq!(
            write(&mut w, &[&[0x2090_3C64]]),
            [0x0F, 0x90, 0, 0, 0x0F, 0x3C, 0, 0, 0x0F, 0x64, 0, 0]
        );
        assert_eq!(
            write(&mut w, &[&[0x3001_7E00, 0]]),
            [0x0F, 0xF0, 0, 0, 0x0F, 0x7E, 0, 0, 0x0F, 0xF7, 0, 0]
        );
        // The reader reads the unparsed stream back.
        let bytes = write(&mut w, &[&[0x2090_3C64]]);
        assert_eq!(read1(&bytes), [[0x2090_3C64u32].to_vec()]);
    }

    #[test]
    fn writer_repeats_status_byte_in_each_packet() {
        // REQ-USB-EVT-013: no running status.
        assert_eq!(
            write16(&[&[0x2090_3C64], &[0x2090_4064]]),
            [0x09, 0x90, 0x3C, 0x64, 0x09, 0x90, 0x40, 0x64]
        );
    }

    #[test]
    fn writer_writes_nothing_for_umps_without_bytes() {
        // REQ-USB-EVT-014: Utility, SysEx8, UMP Stream, Flex Data, undefined status, short or
        // empty words.
        for ump in [
            &[0x0020_0010u32][..],
            &[0x5001_0000, 0, 0, 0],
            &[0xF000_0101, 0x1F, 0, 0],
            &[0xD010_0100, 0, 0, 0],
            &[0x10F4_0000],
            &[0x10F9_0000],
            &[0x4090_3C00],
            &[],
        ] {
            let mut w = PacketWriter::<16>::default();
            let mut buf = [0u8; 64];
            assert_eq!(w.write(ump, &mut buf), Ok(0));
        }
    }

    #[test]
    fn reader_gives_cable_n_on_group_n() {
        // REQ-USB-EVT-015, choice U-19.
        let mut r = PacketReader::<16>::default();
        for cable in 0..16u8 {
            let bytes = [cable << 4 | 0x9, 0x90, 0x3C, 0x64];
            assert_eq!(
                read(&mut r, &bytes),
                [[0x2090_3C64u32 | u32::from(cable) << 24].to_vec()]
            );
        }
    }

    #[test]
    fn writer_writes_group_n_on_cable_n() {
        // REQ-USB-EVT-016, choice U-19.
        for group in 0..16u32 {
            let out = write16(&[&[0x2090_3C64 | group << 24]]);
            assert_eq!(out, [(group as u8) << 4 | 0x9, 0x90, 0x3C, 0x64]);
        }
    }

    #[test]
    fn cable_or_group_beyond_count_is_ignored() {
        // REQ-USB-EVT-018.
        let mut r = PacketReader::<2>::default();
        assert!(read(&mut r, &[0x29, 0x90, 0x3C, 0x64]).is_empty());
        assert_eq!(read(&mut r, &[0x19, 0x90, 0x3C, 0x64]).len(), 1);
        let mut w = PacketWriter::<2>::default();
        let mut buf = [0u8; 8];
        assert_eq!(w.write(&[0x2290_3C64], &mut buf), Ok(0));
        assert_eq!(w.write(&[0x2190_3C64], &mut buf), Ok(4));
    }

    #[test]
    fn writer_keeps_caller_order_across_cables() {
        // REQ-USB-EVT-019: one stream, packets in the order of the UMPs.
        let mut w = PacketWriter::<3>::default();
        let mut buf = [0u8; 64];
        let mut len = 0;
        for ump in [0x2290_3C64u32, 0x2090_3C64, 0x12F8_0000, 0x2280_3C00] {
            len += w.write(&[ump], &mut buf[len..]).unwrap();
        }
        assert_eq!(
            &buf[..len],
            &[
                0x29, 0x90, 0x3C, 0x64, 0x09, 0x90, 0x3C, 0x64, 0x2F, 0xF8, 0, 0, 0x28, 0x80, 0x3C,
                0x00
            ]
        );
    }

    #[test]
    fn reader_ignores_cin_0_and_1() {
        // REQ-USB-EVT-009: no byte of a CIN 0x0 or 0x1 packet reaches the codec, so the data
        // bytes after it have no running status. An all-zero packet is padding.
        let bytes = [
            0x01, 0x90, 0x3C, 0x64, 0x00, 0x90, 0x3C, 0x64, 0x00, 0x00, 0x00, 0x00, 0x0F, 0x3C,
            0x00, 0x00, 0x0F, 0x64, 0x00, 0x00,
        ];
        assert!(read1(&bytes).is_empty());
    }

    #[test]
    fn reader_ignores_unused_bytes() {
        // REQ-USB-EVT-008 (reader clause): the CIN size only.
        assert_eq!(
            read1(&[0x0C, 0xC5, 0x01, 0x7F, 0x05, 0xF6, 0x12, 0x34]),
            [[0x20C5_0100u32].to_vec(), [0x10F6_0000].to_vec()]
        );
    }

    #[test]
    fn reader_takes_length_from_cin_only() {
        // REQ-USB-EVT-010: single bytes with CIN 0xF, a mix of CIN 0xF and parsed packets,
        // running status data with a CIN that does not agree with the status, and a System
        // Real Time byte with CIN 0x5.
        assert_eq!(
            read1(&[
                0x0F, 0x90, 0, 0, 0x0F, 0x3C, 0, 0, 0x0F, 0x64, 0, 0, // CIN 0xF bytes
                0x02, 0x3E, 0x40, 0x00, // running status data, CIN 0x2
                0x09, 0x80, 0x3C, 0x00, // parsed
                0x0F, 0x3E, 0x00, 0x00, 0x0F, 0x00, 0x00, 0x00, // running status, CIN 0xF
                0x05, 0xF8, 0x00, 0x00, // real time with CIN 0x5
            ]),
            [
                [0x2090_3C64u32].to_vec(),
                [0x2090_3E40].to_vec(),
                [0x2080_3C00].to_vec(),
                [0x2080_3E00].to_vec(),
                [0x10F8_0000].to_vec(),
            ]
        );
    }

    #[test]
    fn reader_keeps_state_for_each_cable() {
        // REQ-USB-EVT-011: a SysEx on cable 0 and a note with running status on cable 1.
        let mut r = PacketReader::<2>::default();
        let out = read(
            &mut r,
            &[
                0x04, 0xF0, 0x01, 0x02, // cable 0: SysEx starts
                0x19, 0x90, 0x3C, 0x64, // cable 1: Note On
                0x06, 0x03, 0xF7, 0x00, // cable 0: SysEx ends
                0x12, 0x40, 0x64, 0x00, // cable 1: running status
            ],
        );
        assert_eq!(
            out,
            [
                [0x2190_3C64u32].to_vec(),
                [0x3003_0102, 0x0300_0000].to_vec(),
                [0x2190_4064].to_vec(),
            ]
        );
    }

    #[test]
    fn reader_reports_tail_and_keeps_no_part_of_packet() {
        // REQ-USB-EVT-020, choice U-04.
        let mut r = PacketReader::<1>::default();
        let got: Vec<_> = r
            .read(&[0x09, 0x90, 0x3C, 0x64, 0x09, 0x90])
            .map(|x| x.map(words))
            .collect();
        assert_eq!(
            got,
            [
                Ok([0x2090_3C64u32].to_vec()),
                Err(EventPacketError::Tail { len: 2 })
            ]
        );
        // The next slice starts with a whole packet.
        assert_eq!(
            read(&mut r, &[0x09, 0x80, 0x3C, 0x00]),
            [[0x2080_3C00u32].to_vec()]
        );
        assert_eq!(
            r.read(&[1, 2, 3]).collect::<Vec<_>>(),
            [Err(EventPacketError::Tail { len: 3 })]
        );
        assert_eq!(r.read(&[]).count(), 0);
    }

    #[test]
    fn writer_overflow_writes_nothing_and_keeps_state() {
        // REQ-USB-EVT-021.
        let mut w = PacketWriter::<1>::default();
        let mut buf = [0xAAu8; 8];
        assert_eq!(w.write(&[0x3011_0100, 0], &mut buf), Ok(0));
        // F0 01 held; a Note On needs 2 packets (end of SysEx and the note).
        assert_eq!(
            w.write(&[0x2090_3C64], &mut buf[..7]),
            Err(EventPacketError::Overflow)
        );
        assert_eq!(buf, [0xAA; 8]);
        assert_eq!(w.held(0), 2);
        assert_eq!(w.write(&[0x2090_3C64], &mut buf), Ok(8));
        assert_eq!(buf, [0x07, 0xF0, 0x01, 0xF7, 0x09, 0x90, 0x3C, 0x64]);
    }

    #[test]
    fn writer_largest_ump_gives_five_packets() {
        // REQ-USB-EVT-021: an open SysEx and a MIDI 2.0 Registered Controller (4 Control
        // Change messages, M2-104 D.2) give 20 bytes.
        let mut w = PacketWriter::<1>::default();
        let mut buf = [0u8; 20];
        assert_eq!(w.write(&[0x3011_0100, 0], &mut buf), Ok(0));
        assert_eq!(
            w.write(&[0x4020_0102, 0x1234_5678], &mut buf),
            Ok(MAX_PACKETS_PER_UMP * PACKET_LEN)
        );
        assert_eq!(&buf[..4], &[0x07, 0xF0, 0x01, 0xF7]);
        assert!(buf[4..]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] == 0x0B && p[1] == 0xB0));
    }

    #[test]
    fn prop_writer_parsed_gives_at_most_twenty_bytes() {
        // REQ-USB-EVT-021; unparsed: at most 13 packets.
        proptest!(|(umps in proptest::collection::vec(proptest::array::uniform4(any::<u32>()), 0..16), unparsed in any::<bool>())| {
            let mut w = PacketWriter::<16>::new(WriterOptions { unparsed });
            // 20 bytes parsed, 52 bytes (13 packets) unparsed.
            let max = if unparsed { 52 } else { 20 };
            for u in umps {
                let mut buf = [0u8; 64];
                let n = w.write(&u, &mut buf).unwrap();
                prop_assert!(n <= max);
            }
        });
    }

    #[test]
    fn reader_midi2_protocol_option_gives_message_type_4() {
        // REQ-USB-EVT-023, choice U-20.
        assert_eq!(
            PacketReader::<1>::default().protocol(),
            StreamProtocol::Midi1
        );
        // OPT-011: MIDI 2.0 Protocol output needs a translator for each cable.
        assert_eq!(
            PacketReader::<2>::new(StreamProtocol::Midi2).err(),
            Some(EventPacketError::NoTranslators)
        );
        assert_eq!(
            PacketReader::<2, 1>::new(StreamProtocol::Midi2).err(),
            Some(EventPacketError::NoTranslators)
        );
        assert!(PacketReader::<2>::new(StreamProtocol::Midi1).is_ok());
        let mut r = PacketReader::<2, 2>::new(StreamProtocol::Midi2).unwrap();
        let out = read(&mut r, &[0x19, 0x90, 0x3C, 0x64]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0][0] >> 16, 0x4190);
        // System messages keep Message Type 0x1.
        assert_eq!(
            read(&mut r, &[0x1F, 0xF8, 0, 0]),
            [[0x11F8_0000u32].to_vec()]
        );
    }

    #[test]
    fn table_4_2_channel_vectors_both_directions() {
        // REQ-USB-EVT-025.
        for (packet, ump) in [
            ([0x19u8, 0x90, 0x3C, 0x64], 0x2190_3C64u32),
            ([0xAB, 0xB0, 0x07, 0x7F], 0x2AB0_077F),
        ] {
            assert_eq!(read1(&packet), [[ump].to_vec()]);
            assert_eq!(write16(&[&[ump]]), packet);
        }
    }

    #[test]
    fn table_4_2_timing_clock_vector_both_directions() {
        // REQ-USB-EVT-026, REQ-USB-EVT-008: the "xx" bytes are ignored on read, 0 on write.
        assert_eq!(write16(&[&[0x13F8_0000]]), [0x3F, 0xF8, 0x00, 0x00]);
        assert_eq!(
            read1(&[0x3F, 0xF8, 0x00, 0x00]),
            [[0x13F8_0000u32].to_vec()]
        );
        assert_eq!(
            read1(&[0x3F, 0xF8, 0x12, 0x34]),
            [[0x13F8_0000u32].to_vec()]
        );
    }

    #[test]
    fn table_4_2_sysex_vectors_both_directions() {
        // REQ-USB-EVT-027.
        let cases: [(&[u8], [u32; 2]); 3] = [
            (
                &[0x24, 0xF0, 0x00, 0x01, 0x25, 0xF7, 0x00, 0x00],
                [0x3202_0001, 0x0000_0000],
            ),
            (
                &[0x24, 0xF0, 0x00, 0x01, 0x26, 0x02, 0xF7, 0x00],
                [0x3203_0001, 0x0200_0000],
            ),
            (
                &[0x24, 0xF0, 0x00, 0x01, 0x27, 0x02, 0x03, 0xF7],
                [0x3204_0001, 0x0203_0000],
            ),
        ];
        for (packets, ump) in cases {
            assert_eq!(read1(packets), [ump.to_vec()]);
            assert_eq!(write16(&[&ump]), packets);
        }
    }

    #[test]
    fn table_4_2_special_sysex_vectors_both_directions() {
        // REQ-USB-EVT-028, REQ-USB-EVT-006.
        for (packet, ump) in [
            ([0x26u8, 0xF0, 0xF7, 0x00], [0x3200_0000u32, 0]),
            ([0x27, 0xF0, 0x7E, 0xF7], [0x3201_7E00, 0]),
        ] {
            assert_eq!(read1(&packet), [ump.to_vec()]);
            assert_eq!(write16(&[&ump]), packet);
        }
    }

    #[test]
    fn write_ump_and_read_packet_helpers_agree() {
        // REQ-USB-EVT-001, REQ-USB-EVT-025: the Ump forms of write and read.
        let (ump, _) = Ump::parse(&[0x2190_3C64]).unwrap();
        let mut w = PacketWriter::<2>::default();
        let mut buf = [0u8; 4];
        assert_eq!(w.write_ump(&ump, &mut buf), Ok(4));
        let mut r = PacketReader::<2>::default();
        let mut got = Vec::new();
        r.read_packet(EventPacket::from_bytes(buf), |u| got.push(u));
        assert_eq!(got.iter().map(Packet::ump).collect::<Vec<_>>(), [ump]);
    }

    #[test]
    fn prop_bytes_pass_unchanged_both_directions() {
        // REQ-USB-EVT-022: MIDI 1.0 Channel Voice and System UMPs go out with the bytes of
        // UmpToBytes and come back as the same UMPs.
        let msg = prop_oneof![
            (0u32..16, 0x80u32..0xF0, 0u32..0x80, 0u32..0x80).prop_map(|(g, s, d0, d1)| {
                let d1 = if (0xC0..0xE0).contains(&s) { 0 } else { d1 };
                0x2000_0000 | g << 24 | s << 16 | d0 << 8 | d1
            }),
            (
                0u32..16,
                prop::sample::select(&[0xF8u32, 0xFA, 0xFB, 0xFC, 0xFE, 0xFF, 0xF6][..])
            )
                .prop_map(|(g, s)| 0x1000_0000 | g << 24 | s << 16),
            (0u32..16, 0u32..0x80).prop_map(|(g, d)| 0x10F3_0000 | g << 24 | d << 8),
        ];
        proptest!(|(umps in proptest::collection::vec(msg, 0..32))| {
            let mut w = PacketWriter::<16>::default();
            let mut enc = UmpToBytes::new(false);
            let mut stream = Vec::new();
            for &u in &umps {
                let mut buf = [0u8; 20];
                let n = w.write(&[u], &mut buf).unwrap();
                let mut bytes = [0u8; 16];
                let m = enc.write(&[u], &mut bytes).unwrap();
                let carried: Vec<u8> = buf[..n].as_chunks::<4>().0.iter().flat_map(|&p| {
                    EventPacket::from_bytes(p).valid().to_vec()
                }).collect();
                prop_assert_eq!(&carried[..], &bytes[..m]);
                stream.extend_from_slice(&buf[..n]);
            }
            let expected: Vec<Vec<u32>> = umps.iter().map(|&u| [u].to_vec()).collect();
            prop_assert_eq!(read1(&stream), expected);
        });
    }

    #[test]
    fn prop_sysex_round_trips_through_packets() {
        // REQ-USB-EVT-006, REQ-USB-EVT-022, REQ-USB-EVT-024: the SysEx7 UMPs of a byte stream
        // give packets that the reader turns back into the same UMPs, with real time between.
        proptest!(|(data in proptest::collection::vec(0u8..0x80, 0..40), clock_at in 0usize..48, cable in 0u8..4)| {
            let group = Group::from_bits(cable);
            let mut src = BytesToUmp::midi1(group);
            let mut stream = [0xF0].to_vec();
            stream.extend_from_slice(&data);
            stream.push(0xF7);
            let at = clock_at.min(stream.len());
            stream.insert(at, 0xF8);
            let umps: Vec<Vec<u32>> = stream.iter().flat_map(|&b| src.push(b).collect::<Vec<_>>()).map(words).collect();
            let mut w = PacketWriter::<4>::default();
            let refs: Vec<&[u32]> = umps.iter().map(|u| &u[..]).collect();
            let packets = write(&mut w, &refs);
            prop_assert_eq!(w.held(cable as usize), 0);
            let mut r = PacketReader::<4>::default();
            // The clock can come out at another place between the SysEx7 UMPs, because the
            // writer holds bytes; the SysEx7 UMPs and the clock are the same.
            let split = |v: Vec<Vec<u32>>| -> (Vec<Vec<u32>>, usize) {
                let clocks = v.iter().filter(|u| u[0] >> 28 == 0x1).count();
                (v.into_iter().filter(|u| u[0] >> 28 == 0x3).collect(), clocks)
            };
            prop_assert_eq!(split(read(&mut r, &packets)), split(umps));
        });
    }

    #[test]
    fn prop_reader_never_panics() {
        // REQ-USB-EVT-020: any bytes; each item is a UMP or the tail error at the end.
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..96), midi2 in any::<bool>())| {
            let protocol = if midi2 { StreamProtocol::Midi2 } else { StreamProtocol::Midi1 };
            let mut r = PacketReader::<3, 3>::new(protocol).unwrap();
            let items: Vec<_> = r.read(&bytes).collect();
            let errors = items.iter().filter(|i| i.is_err()).count();
            prop_assert_eq!(errors, usize::from(bytes.len() % 4 != 0));
            if errors == 1 {
                prop_assert_eq!(items.last().copied(), Some(Err(EventPacketError::Tail { len: bytes.len() % 4 })));
            }
        });
    }

    #[test]
    fn prop_read_with_gives_the_same_items_as_read() {
        // REQ-USB-EVT-020, OPT-021: the callback form gives the same items and keeps the same
        // state as the iterator, over several transfers. Packets on cables 0 to 3 (cable 3 is
        // not below CABLES), every CIN, any data bytes.
        let packet = (0u8..4, 0u8..16, any::<[u8; 3]>())
            .prop_map(|(c, cin, d)| [c << 4 | cin, d[0], d[1], d[2]]);
        let transfer =
            (proptest::collection::vec(packet, 0..12), 0usize..4).prop_map(|(p, tail)| {
                let mut v: Vec<u8> = p.concat();
                v.extend(std::iter::repeat_n(0x09, tail));
                v
            });
        proptest!(|(transfers in proptest::collection::vec(transfer, 1..4), midi2 in any::<bool>())| {
            let protocol = if midi2 { StreamProtocol::Midi2 } else { StreamProtocol::Midi1 };
            let mut a = PacketReader::<3, 3>::new(protocol).unwrap();
            let mut b = PacketReader::<3, 3>::new(protocol).unwrap();
            for t in &transfers {
                let x: Vec<_> = a.read(t).collect();
                let mut y = Vec::new();
                b.read_with(t, |i| y.push(i));
                prop_assert_eq!(x, y);
                for c in 0..3 {
                    prop_assert_eq!(a.in_sysex(c), b.in_sysex(c));
                }
            }
        });
    }

    #[test]
    fn reset_clears_state() {
        // REQ-USB-EVT-011, REQ-USB-EVT-024: state is per cable and can be cleared.
        let mut w = PacketWriter::<1>::default();
        let _ = write(&mut w, &[&[0x3011_0100, 0]]);
        w.reset();
        assert_eq!(w.held(0), 0);
        let mut r = PacketReader::<1>::default();
        assert!(read(&mut r, &[0x09, 0x90, 0x3C, 0x64]).len() == 1);
        r.reset();
        assert!(read(&mut r, &[0x02, 0x3E, 0x40, 0x00]).is_empty());
        // A reset in place clears an open SysEx of each cable and keeps the Protocol.
        let mut r = PacketReader::<2, 2>::new(StreamProtocol::Midi2).unwrap();
        assert!(read(&mut r, &[0x14, 0xF0, 0x01, 0x02]).is_empty());
        assert!(r.in_sysex(1));
        r.reset();
        assert!(!r.any_in_sysex());
        assert_eq!(r.protocol(), StreamProtocol::Midi2);
        assert!(read(&mut r, &[0x16, 0x03, 0xF7, 0x00]).is_empty());
        let out = read(&mut r, &[0x19, 0x90, 0x3C, 0x64]);
        assert_eq!(out[0][0] >> 16, 0x4190);
    }

    #[test]
    fn reader_tracks_open_sysex_per_cable() {
        // REQ-USB-EVT-011: SysEx state per cable; real time does not end it, a status does.
        let mut r = PacketReader::<2>::default();
        assert!(!r.in_sysex(0));
        assert!(read(&mut r, &[0x04, 0xF0, 0x01, 0x02]).is_empty());
        assert!(r.in_sysex(0) && !r.in_sysex(1) && r.any_in_sysex());
        assert_eq!(read(&mut r, &[0x0F, 0xF8, 0, 0]).len(), 1);
        assert!(r.in_sysex(0));
        assert_eq!(read(&mut r, &[0x06, 0x03, 0xF7, 0x00]).len(), 1);
        assert!(!r.in_sysex(0));
        assert!(read(&mut r, &[0x04, 0xF0, 0x01, 0x02]).is_empty());
        assert_eq!(read(&mut r, &[0x09, 0x90, 0x3C, 0x64]).len(), 2);
        assert!(!r.any_in_sysex());
        assert!(!r.in_sysex(99));
    }

    #[test]
    fn dropped_reader_keeps_the_sysex_bytes_of_the_rest() {
        // REQ-USB-EVT-011, REQ-USB-EVT-020: a drop after the first UMP reads the rest of the
        // transfer, so the byte 08 is not lost from the SysEx (review M1, case 1).
        let mut r = PacketReader::<1>::default();
        let first: Vec<_> = r
            .read(&[
                0x04, 0xF0, 0x01, 0x02, 0x04, 0x03, 0x04, 0x05, 0x04, 0x06, 0x07, 0x08,
            ])
            .take(1)
            .map(|x| x.map(words))
            .collect();
        assert_eq!(first, [Ok([0x3016_0102u32, 0x0304_0506].to_vec())]);
        assert_eq!(
            read(&mut r, &[0x07, 0x09, 0x0A, 0xF7]),
            [[0x3034_0708u32, 0x090A_0000].to_vec()]
        );
        // Case 2: F6 gives two UMPs from one byte; a drop after the first loses no state, and
        // the next transfer starts clean.
        let mut r = PacketReader::<1>::default();
        let n = r
            .read(&[0x06, 0xF0, 0x01, 0x00, 0x05, 0xF6, 0x00, 0x00])
            .take(1)
            .count();
        assert_eq!(n, 1);
        assert!(!r.in_sysex(0));
        assert_eq!(
            read(&mut r, &[0x09, 0x90, 0x3C, 0x64]),
            [[0x2090_3C64u32].to_vec()]
        );
    }

    #[test]
    fn packet_read_with_gives_the_same_items_as_read() {
        // REQ-USB-EVT-020: the callback form.
        let bytes = [
            0x09, 0x90, 0x3C, 0x64, 0x04, 0xF0, 0x01, 0x02, 0x06, 0x03, 0xF7, 0, 1,
        ];
        let mut a = PacketReader::<1>::default();
        let mut b = PacketReader::<1>::default();
        let x: Vec<_> = a.read(&bytes).collect();
        let mut y = Vec::new();
        b.read_with(&bytes, |i| y.push(i));
        assert_eq!(x, y);
        assert_eq!(y.len(), 3);
    }
}
