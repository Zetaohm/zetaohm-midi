//! The run-time state of one USB MIDI function: the selected Alternate Setting and the codecs
//! of one endpoint pair (USB-MIDI v2.0 sections 3.1.1 and 3.2, USB-MIDI v1.0 section 4).
//!
//! - The caller gives the selected Alternate Setting of the MIDI Streaming interface; the link
//!   does not read the bus (REQ-USB-GEN-006, DEC-004). [`SettingNumber`] 0 is USB MIDI 1.0:
//!   the link sends and receives only USB-MIDI 1.0 Event Packets, with only MIDI 1.0 messages
//!   in them (REQ-USB-GEN-006). Each other setting is USB MIDI 2.0: the link carries MIDI data
//!   only as Universal MIDI Packets and never uses Event Packets (REQ-USB-UMP-001).
//! - [`MidiLink`] holds one [`PacketReader`] and one [`PacketWriter`] (Event Packets, module
//!   `event_packet`) and one [`UmpPacketReader`] and one [`UmpPacketWriter`] (UMP, module
//!   `ump_usb`) for one endpoint pair, and the [`GroupMask`] of the received endpoint.
//! - The caller reports a change of the selected Alternate Setting, a USB reset and a change
//!   of the device configuration ([`LinkEvent`]). On each report the link discards each held
//!   part of a UMP and each held part of an Event Packet stream, so each endpoint stream
//!   starts at a UMP boundary (REQ-USB-UMP-008, AMB-USB-MIDI-2-35). A held part cannot be
//!   completed after such an event, because the next USB packet starts a new stream (USB 2.0
//!   9.1.1.5, 9.4.10).
//! - At Alternate Setting 0 the Event Packets of one UMP (up to 20 or 52 bytes,
//!   REQ-USB-EVT-021) can be longer than the rest of a USB packet (8, 16 or 32 bytes at full
//!   speed, REQ-USB-MS2-016). The link then writes the packets that fit, holds the others and
//!   writes them first in the next USB packets, with no other UMP between them
//!   (REQ-USB-EVT-030). Each Event Packet stays whole (AMB-USB-MIDI-1-24).
//!
//! No alloc (DEC-002). Synchronous and sans-IO with no USB stack and no clock (DEC-004,
//! DEC-014). Nothing panics on received bytes; the readers are lenient (choice U-24 of
//! DEC-USB-001).

use core::iter::Peekable;

use zmidi_ump::{Packet, StreamProtocol, Ump, UmpError};

#[cfg(feature = "alt0")]
use crate::event_packet::{self, PacketReader, PacketWriter, PACKET_LEN};
use crate::event_packet::{EventPacketError, WriterOptions};
use crate::gtb::GroupMask;
use crate::ms2::EndpointDirection;
use crate::ump_usb::{self, ReceivedUmp, UmpPacketReader, UmpPacketWriter, UmpUsbError};

/// The largest number of Event Packet bytes of one UMP: 13 packets in the unparsed mode
/// (REQ-USB-EVT-021).
#[cfg(feature = "alt0")]
const MAX_EVENT_BYTES: usize = event_packet::MAX_UNPARSED_PACKETS_PER_UMP * PACKET_LEN;

/// Error of the link (thiserror 2, no_std). Each variant names the rule that a call found
/// broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    /// The caller buffer is shorter than [`MidiLink::packet_size`]; nothing was written and the
    /// writer state did not change. A caller defect: give a buffer of one whole USB packet.
    #[error("output buffer too small")]
    Overflow,
    /// The UMP does not fit in the rest of the USB packet: send the packet and give the same
    /// UMP to the next one (REQ-USB-UMP-009). [`MidiLink::write_packet`] does not return it
    /// (it ends the packet and keeps the UMP in the iterator); it is the link form of
    /// [`UmpUsbError::PacketFull`].
    #[error("USB packet full")]
    PacketFull,
    /// The first UMP of the packet has fewer words than its Message Type needs. The link took
    /// it from the iterator and wrote nothing for it, so the output does not stop.
    #[error("UMP needs {needed} words, {got} given")]
    ShortUmp {
        /// The size of the Message Type in words (USB-MIDI v2.0 Table 3-1).
        needed: usize,
        /// The number of words given.
        got: usize,
    },
    /// A wMaxPacketSize below 4 bytes: no UMP word and no Event Packet fits in a packet.
    #[error("packet size below 4 bytes")]
    InvalidPacketSize,
    /// 1 to 3 bytes after the last whole UMP word or Event Packet of a received transfer;
    /// they are ignored (REQ-USB-UMP-007, REQ-USB-EVT-020, choice U-04).
    #[error("{len} bytes after the last whole word or packet")]
    Tail {
        /// The number of ignored bytes, 1 to 3.
        len: usize,
    },
    /// Another error of the Event Packet codec (Alternate Setting 0).
    #[error("Event Packet: {0}")]
    EventPacket(EventPacketError),
}

impl From<EventPacketError> for LinkError {
    fn from(e: EventPacketError) -> Self {
        match e {
            EventPacketError::Overflow => LinkError::Overflow,
            EventPacketError::Tail { len } => LinkError::Tail { len },
            other => LinkError::EventPacket(other),
        }
    }
}

impl From<UmpUsbError> for LinkError {
    fn from(e: UmpUsbError) -> Self {
        match e {
            UmpUsbError::Overflow => LinkError::Overflow,
            UmpUsbError::PacketFull => LinkError::PacketFull,
            UmpUsbError::ShortUmp { needed, got } => LinkError::ShortUmp { needed, got },
            UmpUsbError::InvalidPacketSize => LinkError::InvalidPacketSize,
            UmpUsbError::Tail { len } => LinkError::Tail { len },
        }
    }
}

/// The selected Alternate Setting of the MIDI Streaming interface, as the caller reports it
/// (REQ-USB-GEN-006). 8 bits (bAlternateSetting).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SettingNumber(pub u8);

impl SettingNumber {
    /// Alternate Setting 0: USB MIDI 1.0 (USB-MIDI v2.0 3.1.1).
    pub const MIDI1: SettingNumber = SettingNumber(0);

    /// The transport of this setting: Event Packets for setting 0, UMP for each other setting
    /// (REQ-USB-GEN-006, REQ-USB-UMP-001).
    pub const fn mode(self) -> LinkMode {
        if self.0 == 0 {
            LinkMode::EventPackets
        } else {
            LinkMode::Ump
        }
    }
}

/// The transport of the MIDI data on the endpoints of the selected Alternate Setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkMode {
    /// USB-MIDI 1.0 Event Packets with MIDI 1.0 messages (Alternate Setting 0,
    /// REQ-USB-GEN-006).
    EventPackets,
    /// Universal MIDI Packets, least significant byte first (USB MIDI 2.0 Alternate Settings,
    /// REQ-USB-UMP-001).
    Ump,
}

/// An event that the USB stack sees and the caller reports to the link (REQ-USB-UMP-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkEvent {
    /// The host selected this Alternate Setting of the MIDI Streaming interface
    /// (SET_INTERFACE, USB 2.0 9.4.10). Also when it is the same setting: SET_INTERFACE
    /// resets the endpoint state.
    AlternateSetting(SettingNumber),
    /// A USB reset. The device is not configured after it; the link goes to Alternate
    /// Setting 0 (USB 2.0 9.1.1.3).
    UsbReset,
    /// A change of the device configuration (SET_CONFIGURATION). Each interface then uses
    /// Alternate Setting 0 (USB 2.0 9.1.1.5, 9.4.7); the link goes to Alternate Setting 0.
    ConfigurationChange,
}

/// The configuration of a [`MidiLink`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkConfig {
    /// The direction of the received endpoint: [`EndpointDirection::MidiOut`] on a device,
    /// [`EndpointDirection::MidiIn`] on a host. It gives the Group Terminal direction of the
    /// received UMPs (REQ-USB-UMP-014).
    pub receive: EndpointDirection,
    /// The Group Terminals of the received endpoint (REQ-USB-UMP-016), for example
    /// [`GroupMask::from_blocks`].
    pub mask: GroupMask,
    /// wMaxPacketSize of the sent endpoint, in bytes. The writer rounds it down to a multiple
    /// of 4 and writes at most this number of bytes in one USB packet.
    pub max_packet: u16,
    /// The Protocol of the UMPs that the Event Packet reader gives (REQ-USB-EVT-023, choice
    /// U-20): [`StreamProtocol::Midi1`] by default.
    pub protocol: StreamProtocol,
    /// The options of the Event Packet writer (choice U-22): parsed mode by default.
    pub writer: WriterOptions,
}

impl Default for LinkConfig {
    /// A device: MIDI OUT received endpoint, all 16 Group Terminals, 64-byte packets (full
    /// speed bulk), MIDI 1.0 Protocol UMPs from Event Packets, parsed mode.
    fn default() -> Self {
        LinkConfig {
            receive: EndpointDirection::MidiOut,
            mask: GroupMask::ALL,
            max_packet: 64,
            protocol: StreamProtocol::Midi1,
            writer: WriterOptions::default(),
        }
    }
}

/// The run-time state of one USB MIDI function for one endpoint pair, with `CABLES` Event
/// Packet cables (1 to 16, choice U-21).
///
/// The link starts at Alternate Setting 0. It gives each received transfer to the reader of
/// the selected kind ([`Self::read`]) and each outgoing UMP to the writer of the selected
/// kind ([`Self::write_packet`]). Fixed storage; no alloc.
///
/// `X` is the translator count of the Event Packet reader ([`PacketReader`]): MIDI 2.0 Protocol
/// output from Event Packets ([`LinkConfig::protocol`]) needs `X >= CABLES`, else
/// [`Self::new`] gives [`EventPacketError::NoTranslators`]; MIDI 1.0 Protocol output needs none,
/// so the default is 0 (OPT-011).
///
/// Memory (measured on `thumbv7em-none-eabihf`, 2026-10-18, after OPT-011 option 2): see the
/// table of docs/optimization-backlog.md; each translator adds 128 bytes.
/// [`Self::report`] clears the state in place, so a [`LinkEvent`] puts at most one
/// `BytesToUmp` on the stack, not a copy of the link. [`Self::new`] returns the link by value:
/// for a large `CABLES`, put the link in a `static` cell with care for the stack of the
/// initializer. A `static` of `Option<MidiLink> = None` is in `.data` (its `None` is not all
/// zero bits), so its initial image also uses flash; a `core::mem::MaybeUninit` static (for
/// example a `static_cell::StaticCell`) is in `.bss` (OPT-013).
#[derive(Debug, Clone)]
pub struct MidiLink<const CABLES: usize = 1, const X: usize = 0> {
    /// The selected Alternate Setting, from the caller (REQ-USB-GEN-006).
    setting: SettingNumber,
    /// The Event Packet state of Alternate Setting 0; no state without the feature `alt0`.
    alt0: Alt0<CABLES, X>,
    /// UMP reader of the received endpoint (USB MIDI 2.0 settings).
    ump_reader: UmpPacketReader,
    /// UMP writer of the sent endpoint (USB MIDI 2.0 settings).
    ump_writer: UmpPacketWriter,
    /// wMaxPacketSize of the sent endpoint of Alternate Setting 0 (`LinkConfig::max_packet`),
    /// the packet size again after a USB reset or a configuration change.
    alt0_max_packet: u16,
}

/// The Event Packet reader and writer of Alternate Setting 0 (feature `alt0`, on by default).
#[cfg(feature = "alt0")]
#[derive(Debug, Clone)]
struct Alt0<const CABLES: usize, const X: usize> {
    /// Event Packet reader of the received endpoint.
    reader: PacketReader<CABLES, X>,
    /// Event Packet writer of the sent endpoint.
    writer: PacketWriter<CABLES>,
    /// The held Event Packets of a UMP that did not fit in a USB packet (REQ-USB-EVT-030).
    rest: EventRest,
}

/// Without the feature `alt0`, Alternate Setting 0 has no data path (DEC-014 addendum of
/// 2026-10-23, REQ-USB-GEN-014): the link reads no UMP and writes no packet there.
#[cfg(not(feature = "alt0"))]
#[derive(Debug, Clone)]
struct Alt0<const CABLES: usize, const X: usize>;

/// The Event Packets of one UMP that the link holds for the next USB packets
/// (REQ-USB-EVT-030). At most 48 bytes are held; the buffer has room for 52.
#[cfg(feature = "alt0")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EventRest {
    /// The Event Packet bytes, in bus order.
    bytes: [u8; MAX_EVENT_BYTES],
    /// The next byte of `bytes` to write (a multiple of 4).
    pos: usize,
    /// The number of bytes in `bytes`; `pos == len` when nothing is held.
    len: usize,
}

#[cfg(feature = "alt0")]
impl EventRest {
    const EMPTY: EventRest = EventRest {
        bytes: [0; MAX_EVENT_BYTES],
        pos: 0,
        len: 0,
    };

    const fn pending(&self) -> bool {
        self.pos < self.len
    }

    /// Copies whole held packets into `out` (whole packets only) and gives the byte count.
    fn write(&mut self, out: &mut [u8]) -> usize {
        let held = self.bytes.get(self.pos..self.len).unwrap_or(&[]);
        let n = held.len().min(out.len() / PACKET_LEN * PACKET_LEN);
        if let (Some(dst), Some(src)) = (out.get_mut(..n), held.get(..n)) {
            dst.copy_from_slice(src);
        }
        self.pos += n;
        if !self.pending() {
            *self = EventRest::EMPTY;
        }
        n
    }
}

impl<const CABLES: usize, const X: usize> MidiLink<CABLES, X> {
    /// A link with `config`, at Alternate Setting 0. [`LinkError::InvalidPacketSize`] when
    /// `config.max_packet` is below 4. `CABLES` must be 1 to 16 (checked at compile time).
    pub fn new(config: LinkConfig) -> Result<Self, LinkError> {
        let ump_writer = UmpPacketWriter::new(config.max_packet)?;
        Ok(MidiLink {
            setting: SettingNumber::MIDI1,
            alt0: Alt0::new(&config)?,
            ump_reader: UmpPacketReader::new(config.receive, config.mask),
            ump_writer,
            alt0_max_packet: config.max_packet,
        })
    }

    /// The selected Alternate Setting.
    pub fn setting(&self) -> SettingNumber {
        self.setting
    }

    /// The transport of the selected Alternate Setting.
    pub fn mode(&self) -> LinkMode {
        self.setting.mode()
    }

    /// The Group Terminals of the received endpoint.
    pub fn mask(&self) -> GroupMask {
        self.ump_reader.mask()
    }

    /// Changes the Group Terminals of the received endpoint, for example after a change of
    /// Alternate Setting (REQ-USB-UMP-016).
    pub fn set_mask(&mut self, mask: GroupMask) {
        self.ump_reader.set_mask(mask);
    }

    /// The packet size of the writer in bytes (wMaxPacketSize rounded down to a multiple of 4).
    pub fn packet_size(&self) -> usize {
        self.ump_writer.packet_size()
    }

    /// True when the link holds a part of the output that must go out first: send a packet
    /// even when no new UMP is ready. At a USB MIDI 2.0 setting, the rest of a split UMP
    /// (REQ-USB-UMP-010); at Alternate Setting 0, the held Event Packets of a UMP
    /// (REQ-USB-EVT-030). False after each [`LinkEvent`].
    pub fn pending(&self) -> bool {
        match self.mode() {
            LinkMode::Ump => self.ump_writer.pending(),
            LinkMode::EventPackets => self.alt0.pending(),
        }
    }

    /// True when the link holds a part of a UMP or of an Event Packet stream
    /// (REQ-USB-UMP-008):
    ///
    /// - a received part of a UMP (UMP reader);
    /// - a received System Exclusive message that is not ended, on a cable of the Event
    ///   Packet reader ([`PacketReader::in_sysex`]);
    /// - the rest of a split UMP, or held Event Packets of a UMP ([`Self::pending`]);
    /// - held System Exclusive bytes of the Event Packet writer.
    ///
    /// It does not see a received MIDI 1.0 message other than System Exclusive that a
    /// transfer cuts (for example a status byte in a CIN 0xF packet, with its data bytes in
    /// the next transfer): the zmidi-io reader does not show that state.
    pub fn holds_part(&self) -> bool {
        self.ump_reader.held() > 0 || self.ump_writer.pending() || self.alt0.holds_part()
    }

    /// Takes an event that the caller reports (REQ-USB-UMP-008). The link discards each held
    /// part of a UMP and of an Event Packet stream in each reader and writer, so the next
    /// packet of each endpoint starts at a UMP boundary. The selected setting becomes the
    /// reported setting, or Alternate Setting 0 after a USB reset or a configuration change.
    /// Also the held Event Packets of REQ-USB-EVT-030 are discarded.
    ///
    /// The Group mask and the packet size do not change; [`Self::report_setting`] changes
    /// them with the setting. Each state is cleared in place (no copy of the link on the
    /// stack).
    // Not inlined: it has many call sites (OPT-045).
    #[inline(never)]
    pub fn report(&mut self, event: LinkEvent) {
        self.setting = match event {
            LinkEvent::AlternateSetting(s) => s,
            LinkEvent::UsbReset | LinkEvent::ConfigurationChange => SettingNumber::MIDI1,
        };
        self.alt0.reset();
        self.ump_reader.reset();
        self.ump_writer.reset();
        if matches!(event, LinkEvent::UsbReset | LinkEvent::ConfigurationChange) {
            // Alternate Setting 0 again: the packet size of its endpoint (REQ-USB-MS2-009).
            if let Ok(w) = UmpPacketWriter::new(self.alt0_max_packet) {
                self.ump_writer = w;
            }
        }
    }

    /// Takes the selection of Alternate Setting `setting` as [`Self::report`] does with
    /// [`LinkEvent::AlternateSetting`], and sets the values of that setting: `mask`, the Group
    /// Terminals of the received endpoint (REQ-USB-UMP-016), and `max_packet`, wMaxPacketSize
    /// of the sent endpoint. Each Alternate Setting has its own endpoint descriptors, so these
    /// values can change with the setting.
    ///
    /// [`LinkError::InvalidPacketSize`] when `max_packet` is below 4; then nothing changes.
    pub fn report_setting(
        &mut self,
        setting: SettingNumber,
        mask: GroupMask,
        max_packet: u16,
    ) -> Result<(), LinkError> {
        let writer = UmpPacketWriter::new(max_packet)?;
        self.report(LinkEvent::AlternateSetting(setting));
        self.ump_writer = writer;
        self.ump_reader.set_mask(mask);
        Ok(())
    }

    /// Reads `bytes`, the data of one received USB transfer (or packet), with the reader of
    /// the selected kind: Event Packets at Alternate Setting 0 (REQ-USB-GEN-006), UMP words at
    /// each other setting (REQ-USB-UMP-001).
    ///
    /// The iterator gives each UMP as `Ok`, in order, and then one [`LinkError::Tail`] when 1
    /// to 3 bytes remain. If the iterator is dropped before its end, the drop reads the rest
    /// of `bytes` into the reader state and discards the UMPs of that rest, so the state
    /// stays correct for the next transfer. [`Self::read_with`] is the callback form.
    pub fn read<'a>(&'a mut self, bytes: &'a [u8]) -> LinkRead<'a, CABLES, X> {
        let inner = match self.setting.mode() {
            LinkMode::EventPackets => self.alt0.read(bytes),
            LinkMode::Ump => ReadInner::Ump(self.ump_reader.read(bytes)),
        };
        LinkRead { inner }
    }

    /// Reads all of `bytes` as [`Self::read`] does and gives each item to `sink`, in order.
    /// It uses the callback form of each reader, so a build that uses only this form links no
    /// iterator (OPT-021).
    pub fn read_with(&mut self, bytes: &[u8], mut sink: impl FnMut(Result<Received, LinkError>)) {
        match self.setting.mode() {
            LinkMode::EventPackets => self.alt0.read_with(bytes, sink),
            LinkMode::Ump => self.ump_reader.read_with(bytes, |x| {
                sink(x.map(Received::Ump).map_err(LinkError::from))
            }),
        }
    }

    /// Writes one USB packet into `out` from `umps` and gives its length in bytes (at most
    /// [`Self::packet_size`]; 0 when there is nothing to send). Each item of `umps` is the
    /// words of one UMP. A UMP that does not fit stays in `umps` for the next call.
    ///
    /// - Alternate Setting 0: the held Event Packets first, then the Event Packets of each
    ///   UMP, with only MIDI 1.0 messages (REQ-USB-GEN-006). When the packets of a UMP do not
    ///   fit in the rest of the USB packet, the link writes the whole packets that fit, takes
    ///   the UMP, holds its other packets and writes them first in the next USB packets, with
    ///   no other UMP between them; [`Self::pending`] is then true (REQ-USB-EVT-030). A UMP
    ///   with no MIDI 1.0 byte gives no packet and is taken (REQ-USB-EVT-014).
    /// - A USB MIDI 2.0 setting: the UMP words, the rest of a split UMP first
    ///   (REQ-USB-UMP-001, REQ-USB-UMP-009, REQ-USB-UMP-010).
    ///
    /// Errors, each with nothing written and no state change:
    ///
    /// - [`LinkError::Overflow`] when `out` is shorter than [`Self::packet_size`].
    /// - [`LinkError::ShortUmp`] when the first item of the packet has fewer words than its
    ///   Message Type needs; the item is taken from `umps`. When such an item comes after
    ///   other UMPs, the packet ends before it and the next call reports it.
    pub fn write_packet<'w, I>(
        &mut self,
        umps: &mut Peekable<I>,
        out: &mut [u8],
    ) -> Result<usize, LinkError>
    where
        I: Iterator<Item = &'w [u32]>,
    {
        match self.setting.mode() {
            LinkMode::Ump => Ok(self.ump_writer.write_packet(umps, out)?),
            LinkMode::EventPackets => {
                let size = self.packet_size();
                self.alt0.write_packet(umps, out, size)
            }
        }
    }

    /// Writes one USB packet with the one UMP `ump`; see [`Self::write_packet`]. Gives the
    /// length and true when the UMP was taken, or false when it stays for the next packet
    /// (the rest of a split UMP was written first).
    pub fn write_ump(&mut self, ump: &Ump, out: &mut [u8]) -> Result<(usize, bool), LinkError> {
        let mut words = [0u32; ump_usb::MAX_UMP_WORDS];
        let n = ump.write(&mut words).unwrap_or(0);
        let slice = words.get(..n).unwrap_or(&[]);
        let mut it = core::iter::once(slice).peekable();
        let len = self.write_packet(&mut it, out)?;
        Ok((len, it.peek().is_none()))
    }
}

#[cfg(feature = "alt0")]
impl<const CABLES: usize, const X: usize> Alt0<CABLES, X> {
    fn new(config: &LinkConfig) -> Result<Self, LinkError> {
        Ok(Alt0 {
            reader: PacketReader::new(config.protocol)?,
            writer: PacketWriter::new(config.writer),
            rest: EventRest::EMPTY,
        })
    }

    fn pending(&self) -> bool {
        self.rest.pending()
    }

    fn holds_part(&self) -> bool {
        self.reader.any_in_sysex()
            || self.rest.pending()
            || (0..CABLES).any(|c| self.writer.held(c) > 0)
    }

    fn reset(&mut self) {
        self.reader.reset();
        self.writer.reset();
        self.rest = EventRest::EMPTY;
    }

    fn read<'a>(&'a mut self, bytes: &'a [u8]) -> ReadInner<'a, CABLES, X> {
        ReadInner::Event(self.reader.read(bytes))
    }

    fn read_with(&mut self, bytes: &[u8], mut sink: impl FnMut(Result<Received, LinkError>)) {
        self.reader.read_with(bytes, |x| {
            sink(x.map(Received::EventPacket).map_err(LinkError::from))
        })
    }

    /// [`Self::write_packet`] at Alternate Setting 0 (REQ-USB-GEN-006, REQ-USB-EVT-030).
    fn write_packet<'w, I>(
        &mut self,
        umps: &mut Peekable<I>,
        out: &mut [u8],
        packet_size: usize,
    ) -> Result<usize, LinkError>
    where
        I: Iterator<Item = &'w [u32]>,
    {
        // The packet size is a multiple of 4 = PACKET_LEN.
        let out = out.get_mut(..packet_size).ok_or(LinkError::Overflow)?;
        let mut len = self.rest.write(out);
        if self.rest.pending() {
            // REQ-USB-EVT-030: no other UMP between the packets of one UMP.
            return Ok(len);
        }
        while let Some(&words) = umps.peek() {
            if let Some(e) = short_ump(words) {
                if len == 0 {
                    umps.next();
                    return Err(e);
                }
                break;
            }
            let free = out.get_mut(len..).unwrap_or(&mut []);
            if free.is_empty() {
                break;
            }
            // All packets of one UMP fit in the scratch (REQ-USB-EVT-021), so the writer
            // never gives Overflow here.
            let mut scratch = [0u8; MAX_EVENT_BYTES];
            let n = match self.writer.write(words, &mut scratch) {
                Ok(n) => n,
                Err(_) if len > 0 => break,
                Err(e) => return Err(e.into()),
            };
            umps.next();
            let packets = scratch.get(..n).unwrap_or(&[]);
            if n <= free.len() {
                if let Some(dst) = free.get_mut(..n) {
                    dst.copy_from_slice(packets);
                }
                len += n;
                continue;
            }
            // REQ-USB-EVT-030: the whole packets that fit go now (free is a multiple of 4),
            // the others first in the next USB packets.
            let now = free.len();
            if let (Some(dst), Some(src)) = (free.get_mut(..now), packets.get(..now)) {
                dst.copy_from_slice(src);
            }
            len += now;
            let mut rest = EventRest::EMPTY;
            let held = packets.get(now..).unwrap_or(&[]);
            if let Some(dst) = rest.bytes.get_mut(..held.len()) {
                dst.copy_from_slice(held);
            }
            rest.len = held.len();
            self.rest = rest;
            break;
        }
        Ok(len)
    }
}

#[cfg(not(feature = "alt0"))]
impl<const CABLES: usize, const X: usize> Alt0<CABLES, X> {
    fn new(_config: &LinkConfig) -> Result<Self, LinkError> {
        Ok(Alt0)
    }

    fn pending(&self) -> bool {
        false
    }

    fn holds_part(&self) -> bool {
        false
    }

    fn reset(&mut self) {}

    fn read<'a>(&'a mut self, _bytes: &'a [u8]) -> ReadInner<'a, CABLES, X> {
        ReadInner::None
    }

    fn read_with(&mut self, _bytes: &[u8], _sink: impl FnMut(Result<Received, LinkError>)) {}

    /// Takes every UMP of `umps` and writes nothing: Alternate Setting 0 has no data path.
    fn write_packet<'w, I>(
        &mut self,
        umps: &mut Peekable<I>,
        _out: &mut [u8],
        _packet_size: usize,
    ) -> Result<usize, LinkError>
    where
        I: Iterator<Item = &'w [u32]>,
    {
        umps.for_each(drop);
        Ok(0)
    }
}

/// [`LinkError::ShortUmp`] when `words` is not empty and has fewer words than the Message Type
/// of word 0 needs (USB-MIDI v2.0 Table 3-1).
#[cfg(feature = "alt0")]
fn short_ump(words: &[u32]) -> Option<LinkError> {
    let &w0 = words.first()?;
    let needed = ump_usb::ump_words(w0);
    (words.len() < needed).then_some(LinkError::ShortUmp {
        needed,
        got: words.len(),
    })
}

/// One UMP that the link gives for a received transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Received {
    /// A UMP of the Event Packet reader (Alternate Setting 0): Group n is cable n
    /// (REQ-USB-EVT-015).
    EventPacket(Packet),
    /// A UMP of the UMP reader (a USB MIDI 2.0 setting), with its target (REQ-USB-UMP-014).
    Ump(ReceivedUmp),
}

impl Received {
    /// The UMP decoded by zmidi-ump.
    pub fn ump(&self) -> Result<Ump, UmpError> {
        match self {
            Received::EventPacket(p) => Ok(p.ump()),
            Received::Ump(r) => r.ump(),
        }
    }

    /// The UMP words, word 0 first, and their number (1 to 4).
    pub fn words(&self) -> ([u32; ump_usb::MAX_UMP_WORDS], usize) {
        let mut w = [0u32; ump_usb::MAX_UMP_WORDS];
        let n = match self {
            Received::EventPacket(p) => {
                for (to, from) in w.iter_mut().zip(p.words()) {
                    *to = *from;
                }
                p.words().len()
            }
            Received::Ump(r) => {
                for (to, from) in w.iter_mut().zip(r.words()) {
                    *to = *from;
                }
                r.words().len()
            }
        };
        (w, n)
    }
}

/// Iterator over the UMPs of one received transfer; see [`MidiLink::read`]. A drop before the
/// end reads the rest of the transfer and discards its UMPs.
#[derive(Debug)]
#[must_use = "the UMPs of the transfer are discarded if the iterator is not used"]
pub struct LinkRead<'a, const CABLES: usize, const X: usize = 0> {
    inner: ReadInner<'a, CABLES, X>,
}

#[derive(Debug)]
enum ReadInner<'a, const CABLES: usize, const X: usize> {
    #[cfg(feature = "alt0")]
    Event(event_packet::PacketRead<'a, CABLES, X>),
    /// Alternate Setting 0 with no data path (no feature `alt0`).
    #[cfg(not(feature = "alt0"))]
    None,
    Ump(ump_usb::UmpRead<'a>),
}

impl<const CABLES: usize, const X: usize> Iterator for LinkRead<'_, CABLES, X> {
    type Item = Result<Received, LinkError>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            #[cfg(feature = "alt0")]
            ReadInner::Event(r) => r
                .next()
                .map(|x| x.map(Received::EventPacket).map_err(LinkError::from)),
            #[cfg(not(feature = "alt0"))]
            ReadInner::None => None,
            ReadInner::Ump(r) => r
                .next()
                .map(|x| x.map(Received::Ump).map_err(LinkError::from)),
        }
    }
}

// The tests use Alternate Setting 0; `no_alt0_tests` covers the build without the feature.
#[cfg(all(test, feature = "alt0"))]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;

    const NOTE_ON: u32 = 0x2090_3C64;
    const NOTE_OFF: u32 = 0x2080_3C00;
    /// MIDI 2.0 Note On, Message Type 0x4 (64 bits).
    const NOTE_ON2: [u32; 2] = [0x4090_3C00, 0xFFFF_0000];

    const EVENTS: [LinkEvent; 3] = [
        LinkEvent::AlternateSetting(SettingNumber(1)),
        LinkEvent::UsbReset,
        LinkEvent::ConfigurationChange,
    ];

    #[test]
    fn midi2_event_output_needs_translators() {
        // OPT-011: X translators; MIDI 2.0 Protocol output from Event Packets needs X >= CABLES.
        let cfg = LinkConfig {
            protocol: StreamProtocol::Midi2,
            ..LinkConfig::default()
        };
        assert_eq!(
            MidiLink::<1>::new(cfg).err(),
            Some(LinkError::EventPacket(EventPacketError::NoTranslators))
        );
        let mut l = MidiLink::<1, 1>::new(cfg).unwrap();
        let got: Vec<_> = l
            .read(&[0x09, 0x90, 0x3C, 0x40])
            .map(|r| r.unwrap().words())
            .collect();
        assert_eq!(got[0].0[0] >> 28, 0x4);
    }

    fn link() -> MidiLink<1> {
        MidiLink::new(LinkConfig::default()).unwrap()
    }

    fn link_at(setting: u8) -> MidiLink<1> {
        let mut l = link();
        l.report(LinkEvent::AlternateSetting(SettingNumber(setting)));
        l
    }

    /// Writes the UMPs into one packet; returns the bytes and the number of UMPs taken.
    fn write(l: &mut MidiLink<1>, umps: &[&[u32]]) -> (Vec<u8>, usize) {
        let mut buf = [0u8; 64];
        let mut it = umps.iter().copied().peekable();
        let n = l.write_packet(&mut it, &mut buf).unwrap();
        let left = it.count();
        (buf[..n].to_vec(), umps.len() - left)
    }

    /// The words of each received UMP; panics on an error item.
    fn read(l: &mut MidiLink<1>, bytes: &[u8]) -> Vec<Vec<u32>> {
        l.read(bytes)
            .map(|x| {
                let (w, n) = x.unwrap().words();
                w[..n].to_vec()
            })
            .collect()
    }

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn starts_at_setting_zero_with_event_packets() {
        // REQ-USB-GEN-006.
        let l = link();
        assert_eq!(l.setting(), SettingNumber::MIDI1);
        assert_eq!(l.mode(), LinkMode::EventPackets);
        assert_eq!(SettingNumber(1).mode(), LinkMode::Ump);
        assert_eq!(SettingNumber(255).mode(), LinkMode::Ump);
        assert_eq!(l.packet_size(), 64);
    }

    #[test]
    fn same_umps_go_out_as_event_packets_on_setting_zero() {
        // REQ-USB-GEN-006: Event Packets with MIDI 1.0 messages only (USB-MIDI v1.0 Figure 8).
        let mut l = link();
        let (bytes, taken) = write(&mut l, &[&[NOTE_ON], &[NOTE_OFF]]);
        assert_eq!(taken, 2);
        assert_eq!(bytes, [0x09, 0x90, 0x3C, 0x64, 0x08, 0x80, 0x3C, 0x00]);
        // A MIDI 2.0 UMP goes out as a MIDI 1.0 message (velocity 0xFFFF -> 0x7F).
        let (bytes, taken) = write(&mut l, &[&NOTE_ON2]);
        assert_eq!(taken, 1);
        assert_eq!(bytes, [0x09, 0x90, 0x3C, 0x7F]);
    }

    #[test]
    fn same_umps_go_out_as_le_words_on_setting_one() {
        // REQ-USB-UMP-001: UMP words, least significant byte first; no Event Packet.
        let mut l = link_at(1);
        let (bytes, taken) = write(&mut l, &[&[NOTE_ON], &[NOTE_OFF], &NOTE_ON2]);
        assert_eq!(taken, 3);
        assert_eq!(
            bytes,
            [
                0x64, 0x3C, 0x90, 0x20, 0x00, 0x3C, 0x80, 0x20, //
                0x00, 0x3C, 0x90, 0x40, 0x00, 0x00, 0xFF, 0xFF,
            ]
        );
        assert_eq!(le(&[NOTE_ON, NOTE_OFF, NOTE_ON2[0], NOTE_ON2[1]]), bytes);
    }

    #[test]
    fn received_transfers_come_back_as_umps_in_both_modes() {
        // REQ-USB-GEN-006, REQ-USB-UMP-001.
        let mut l = link();
        assert_eq!(
            read(&mut l, &[0x09, 0x90, 0x3C, 0x64, 0x08, 0x80, 0x3C, 0x00]),
            [[NOTE_ON].to_vec(), [NOTE_OFF].to_vec()]
        );
        let mut l = link_at(1);
        let got: Vec<_> = l
            .read(&le(&[NOTE_ON, NOTE_ON2[0], NOTE_ON2[1]]))
            .map(Result::unwrap)
            .collect();
        assert_eq!(got.len(), 2);
        assert!(matches!(got[0], Received::Ump(_)));
        assert_eq!(got[0].words(), ([NOTE_ON, 0, 0, 0], 1));
        assert_eq!(got[1].words(), ([NOTE_ON2[0], NOTE_ON2[1], 0, 0], 2));
        assert_eq!(got[0].ump(), Ump::parse(&[NOTE_ON]).map(|(u, _)| u));
    }

    #[test]
    fn same_bytes_read_by_the_kind_of_the_setting() {
        // REQ-USB-GEN-006, REQ-USB-UMP-001: the bytes 09 90 3C 64 are a Note On Event Packet
        // at setting 0 and the UMP word 0x643C9009 (Message Type 0x6, 32 bits) at setting 1.
        let bytes = [0x09, 0x90, 0x3C, 0x64];
        assert_eq!(read(&mut link(), &bytes), [[NOTE_ON].to_vec()]);
        assert_eq!(read(&mut link_at(1), &bytes), [[0x643C_9009].to_vec()]);
    }

    #[test]
    fn read_with_and_tail_in_both_modes() {
        // REQ-USB-UMP-007, REQ-USB-EVT-020 through the link.
        for setting in [0u8, 1] {
            let mut l = link_at(setting);
            let mut got = Vec::new();
            l.read_with(&[0x09, 0x90, 0x3C, 0x64, 0x01], |x| got.push(x.map(|_| ())));
            assert_eq!(got, [Ok(()), Err(LinkError::Tail { len: 1 })]);
        }
    }

    #[test]
    fn held_part_of_received_ump_is_discarded_after_each_event() {
        // REQ-USB-UMP-008: word 0 of a 64-bit UMP is held; after the event the next word
        // starts a new UMP.
        for event in EVENTS {
            let mut l = link_at(1);
            assert_eq!(read(&mut l, &le(&[NOTE_ON2[0]])), Vec::<Vec<u32>>::new());
            assert!(l.holds_part());
            l.report(event);
            assert!(!l.holds_part());
            l.report(LinkEvent::AlternateSetting(SettingNumber(1)));
            assert_eq!(read(&mut l, &le(&[NOTE_ON])), [[NOTE_ON].to_vec()]);
        }
        // With no event, the same word completes the held UMP.
        let mut l = link_at(1);
        assert!(read(&mut l, &le(&[NOTE_ON2[0]])).is_empty());
        assert_eq!(
            read(&mut l, &le(&[NOTE_ON])),
            [[NOTE_ON2[0], NOTE_ON].to_vec()]
        );
    }

    #[test]
    fn rest_of_split_ump_is_discarded_after_each_event() {
        // REQ-USB-UMP-008, REQ-USB-UMP-010: a 4-byte packet size splits a 64-bit UMP.
        for event in EVENTS {
            let config = LinkConfig {
                max_packet: 4,
                ..LinkConfig::default()
            };
            let mut l = MidiLink::<1>::new(config).unwrap();
            l.report(LinkEvent::AlternateSetting(SettingNumber(1)));
            let mut buf = [0u8; 4];
            let mut it = [&NOTE_ON2[..]].into_iter().peekable();
            assert_eq!(l.write_packet(&mut it, &mut buf), Ok(4));
            assert_eq!(buf, NOTE_ON2[0].to_le_bytes());
            assert!(l.pending());
            l.report(event);
            assert!(!l.pending());
            assert!(!l.holds_part());
            l.report(LinkEvent::AlternateSetting(SettingNumber(1)));
            let mut it = [&[NOTE_ON][..]].into_iter().peekable();
            assert_eq!(l.write_packet(&mut it, &mut buf), Ok(4));
            assert_eq!(buf, NOTE_ON.to_le_bytes());
        }
    }

    #[test]
    fn held_sysex_byte_of_writer_is_discarded_after_each_event() {
        // REQ-USB-UMP-008: F0 01 are held by the Event Packet writer (REQ-USB-EVT-024).
        for event in EVENTS {
            let mut l = link();
            assert_eq!(write(&mut l, &[&[0x3011_0100, 0]]), (Vec::new(), 1));
            assert!(l.holds_part());
            l.report(event);
            assert!(!l.holds_part());
            l.report(LinkEvent::AlternateSetting(SettingNumber::MIDI1));
            assert_eq!(write(&mut l, &[&[NOTE_ON]]).0, [0x09, 0x90, 0x3C, 0x64]);
        }
        // With no event, the Note On first ends the System Exclusive message (U-23).
        let mut l = link();
        write(&mut l, &[&[0x3011_0100, 0]]);
        assert_eq!(
            write(&mut l, &[&[NOTE_ON]]).0,
            [0x07, 0xF0, 0x01, 0xF7, 0x09, 0x90, 0x3C, 0x64]
        );
    }

    #[test]
    fn held_sysex_byte_of_reader_is_discarded_after_each_event() {
        // REQ-USB-UMP-008: a SysEx start in an Event Packet; after the event its end gives
        // no SysEx7 UMP with the old bytes.
        for event in EVENTS {
            let mut l = link();
            assert!(read(&mut l, &[0x04, 0xF0, 0x01, 0x02]).is_empty());
            l.report(event);
            l.report(LinkEvent::AlternateSetting(SettingNumber::MIDI1));
            let got = read(&mut l, &[0x06, 0x03, 0xF7, 0x00, 0x09, 0x90, 0x3C, 0x64]);
            assert!(!got.contains(&[0x3003_0102, 0x0300_0000].to_vec()));
            assert_eq!(got.last(), Some(&[NOTE_ON].to_vec()));
        }
        // With no event, the end completes the SysEx message (event_packet vector).
        let mut l = link();
        assert!(read(&mut l, &[0x04, 0xF0, 0x01, 0x02]).is_empty());
        assert_eq!(
            read(&mut l, &[0x06, 0x03, 0xF7, 0x00]),
            [[0x3003_0102, 0x0300_0000].to_vec()]
        );
    }

    #[test]
    fn reset_and_configuration_change_go_to_setting_zero() {
        // REQ-USB-UMP-008, REQ-USB-GEN-006: after a USB reset or SET_CONFIGURATION each
        // interface uses Alternate Setting 0.
        for event in [LinkEvent::UsbReset, LinkEvent::ConfigurationChange] {
            let mut l = link_at(1);
            l.report(event);
            assert_eq!(l.mode(), LinkMode::EventPackets);
        }
        let mut l = link();
        l.report(LinkEvent::AlternateSetting(SettingNumber(2)));
        assert_eq!(l.setting(), SettingNumber(2));
        assert_eq!(l.mode(), LinkMode::Ump);
    }

    #[test]
    fn writer_overflow_writes_nothing_in_both_modes() {
        // Writer result passed to the caller (REQ-USB-EVT-021, UmpUsb packet).
        let mut l = link();
        let mut buf = [0xAAu8; 3];
        let mut it = [&[NOTE_ON][..]].into_iter().peekable();
        assert_eq!(l.write_packet(&mut it, &mut buf), Err(LinkError::Overflow));
        assert_eq!(buf, [0xAA; 3]);
        assert!(it.peek().is_some());
        let mut l = link_at(1);
        assert_eq!(l.write_packet(&mut it, &mut buf), Err(LinkError::Overflow));
        assert_eq!(buf, [0xAA; 3]);
        assert!(it.peek().is_some());
    }

    #[test]
    fn reset_and_configuration_change_restore_the_alternate_setting_0_packet_size() {
        // REQ-USB-MS2-009, REQ-USB-UMP-007: after a USB reset or a configuration change the
        // writer uses wMaxPacketSize of the Alternate Setting 0 endpoint again.
        let mut l: MidiLink = MidiLink::new(LinkConfig::default()).unwrap();
        assert_eq!(l.packet_size(), 64);
        for event in [LinkEvent::UsbReset, LinkEvent::ConfigurationChange] {
            l.report_setting(SettingNumber(1), GroupMask::ALL, 32)
                .unwrap();
            assert_eq!(l.packet_size(), 32);
            l.report(event);
            assert_eq!(l.setting(), SettingNumber::MIDI1);
            assert_eq!(l.packet_size(), 64);
        }
    }

    #[test]
    fn event_packets_stop_at_packet_size() {
        // REQ-USB-GEN-006, REQ-USB-EVT-021: whole UMPs only; the rest stays for the next packet.
        let config = LinkConfig {
            max_packet: 6,
            ..LinkConfig::default()
        };
        let mut l = MidiLink::<1>::new(config).unwrap();
        let (bytes, taken) = write(&mut l, &[&[NOTE_ON], &[NOTE_OFF]]);
        assert_eq!(bytes, [0x09, 0x90, 0x3C, 0x64]);
        assert_eq!(taken, 1);
    }

    #[test]
    fn write_ump_reports_whether_taken() {
        // REQ-USB-UMP-010: the rest of a split UMP goes first.
        let ump = Ump::parse(&[NOTE_ON]).unwrap().0;
        let mut l = link();
        let mut buf = [0u8; 64];
        assert_eq!(l.write_ump(&ump, &mut buf), Ok((4, true)));
        let config = LinkConfig {
            max_packet: 4,
            ..LinkConfig::default()
        };
        let mut l = MidiLink::<1>::new(config).unwrap();
        l.report(LinkEvent::AlternateSetting(SettingNumber(1)));
        let big = Ump::parse(&NOTE_ON2).unwrap().0;
        assert_eq!(l.write_ump(&big, &mut buf), Ok((4, true)));
        assert_eq!(l.write_ump(&ump, &mut buf), Ok((4, false)));
        assert_eq!(&buf[..4], &NOTE_ON2[1].to_le_bytes());
        assert_eq!(l.write_ump(&ump, &mut buf), Ok((4, true)));
    }

    #[test]
    fn invalid_packet_size_is_rejected() {
        let config = LinkConfig {
            max_packet: 3,
            ..LinkConfig::default()
        };
        assert_eq!(
            MidiLink::<1>::new(config).err(),
            Some(LinkError::InvalidPacketSize)
        );
    }

    #[test]
    fn prop_read_with_gives_the_same_items_as_read() {
        // OPT-021: the callback form of the link gives the same items as its iterator, in
        // both modes.
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..64), ump_mode in any::<bool>())| {
            let setting = u8::from(ump_mode);
            let (mut a, mut b) = (link_at(setting), link_at(setting));
            let x: Vec<_> = a.read(&bytes).collect();
            let mut y = Vec::new();
            b.read_with(&bytes, |i| y.push(i));
            prop_assert_eq!(x, y);
        });
    }

    #[test]
    fn prop_read_never_panics() {
        // REQ-USB-GEN-006, REQ-USB-UMP-001: lenient readers in both modes (choice U-24).
        proptest!(|(setting in 0u8..3, bytes in proptest::collection::vec(any::<u8>(), 0..64))| {
            let mut l = MidiLink::<2>::new(LinkConfig::default()).unwrap();
            l.report(LinkEvent::AlternateSetting(SettingNumber(setting)));
            let n = l.read(&bytes).count();
            prop_assert!(n <= bytes.len() + 1);
        });
    }

    #[test]
    fn prop_ump_mode_round_trip() {
        // REQ-USB-UMP-001: 32-bit UMPs written at setting 1 read back unchanged.
        proptest!(|(words in proptest::collection::vec(any::<u32>(), 0..16))| {
            let words: Vec<u32> = words.into_iter().map(|w| (w & 0x0FFF_FFFF) | 0x2000_0000).collect();
            let mut l = link_at(1);
            let umps: Vec<&[u32]> = words.chunks(1).collect();
            let (bytes, taken) = write(&mut l, &umps);
            prop_assert_eq!(taken, words.len());
            let back: Vec<u32> = read(&mut l, &bytes).into_iter().flatten().collect();
            prop_assert_eq!(back, words);
        });
    }

    /// MIDI 2.0 Registered Controller: 4 Control Change messages, 16 bytes (parsed).
    const RPN: [u32; 2] = [0x4020_0102, 0x8000_0000];
    /// SysEx7 Complete with 6 bytes: F0 + 6 + F7 = 3 packets, 12 bytes (parsed).
    const SYSEX6: [u32; 2] = [0x3006_0102, 0x0304_0506];
    /// SysEx7 Start with 1 byte: F0 01 held by the writer.
    const SYSEX_START: [u32; 2] = [0x3011_0100, 0];

    fn link_with(max_packet: u16, unparsed: bool) -> MidiLink<1> {
        MidiLink::new(LinkConfig {
            max_packet,
            writer: WriterOptions { unparsed },
            ..LinkConfig::default()
        })
        .unwrap()
    }

    /// All USB packets that the link gives for `umps`, until nothing is left or held.
    fn drain(l: &mut MidiLink<1>, umps: &[&[u32]]) -> Vec<Vec<u8>> {
        let mut it = umps.iter().copied().peekable();
        let mut buf = [0u8; 64];
        let mut packets = Vec::new();
        for _ in 0..200 {
            if it.peek().is_none() && !l.pending() {
                return packets;
            }
            let n = l.write_packet(&mut it, &mut buf).unwrap();
            packets.push(buf[..n].to_vec());
        }
        panic!("the link makes no progress");
    }

    /// The Event Packets of `umps` from one Event Packet writer with no packet limit.
    fn reference(umps: &[&[u32]], unparsed: bool) -> Vec<u8> {
        let mut w = PacketWriter::<1>::new(WriterOptions { unparsed });
        let mut out = Vec::new();
        for u in umps {
            let mut buf = [0u8; 64];
            let n = w.write(u, &mut buf).unwrap();
            out.extend_from_slice(&buf[..n]);
        }
        out
    }

    // REQ-USB-EVT-030: the four cases of the review (robustness H2) and a split in a packet
    // that already holds a UMP. Each USB packet is at most wMaxPacketSize bytes, the held
    // packets go first and no other UMP comes between them.
    #[test]
    fn event_packets_of_one_ump_go_over_several_usb_packets() {
        let cases: [(u16, bool, &[&[u32]]); 5] = [
            (8, false, &[&RPN, &[NOTE_ON]]),
            (16, false, &[&SYSEX_START, &RPN, &[NOTE_ON]]),
            (32, true, &[&RPN, &[NOTE_ON]]),
            (8, false, &[&SYSEX6, &[NOTE_ON]]),
            (8, false, &[&[NOTE_ON], &SYSEX6, &[NOTE_OFF]]),
        ];
        for (max_packet, unparsed, umps) in cases {
            let mut l = link_with(max_packet, unparsed);
            let packets = drain(&mut l, umps);
            let all: Vec<u8> = packets.concat();
            assert_eq!(all, reference(umps, unparsed), "max_packet {max_packet}");
            for p in &packets {
                assert!(p.len() <= max_packet as usize);
                assert_eq!(p.len() % PACKET_LEN, 0);
            }
            assert!(packets.len() > 1);
            assert!(!l.holds_part());
        }
        // The first case packet by packet: 4 CC packets in two USB packets, then the note.
        let mut l = link_with(8, false);
        let umps: [&[u32]; 2] = [&RPN, &[NOTE_ON]];
        let mut it = umps.into_iter().peekable();
        let mut buf = [0u8; 8];
        assert_eq!(l.write_packet(&mut it, &mut buf), Ok(8));
        assert!(l.pending() && l.holds_part());
        assert_eq!(it.peek(), Some(&&[NOTE_ON][..]));
        assert_eq!(l.write_packet(&mut it, &mut buf), Ok(8));
        assert!(buf.as_chunks::<4>().0.iter().all(|p| p[0] == 0x0B));
        assert!(!l.pending());
        assert_eq!(l.write_packet(&mut it, &mut buf), Ok(4));
        assert_eq!(&buf[..4], &[0x09, 0x90, 0x3C, 0x64]);
        // write_ump: the held packets go first and the new UMP is not taken.
        let mut l = link_with(8, false);
        let rpn = Ump::parse(&RPN).unwrap().0;
        let note = Ump::parse(&[NOTE_ON]).unwrap().0;
        assert_eq!(l.write_ump(&rpn, &mut buf), Ok((8, true)));
        assert_eq!(l.write_ump(&note, &mut buf), Ok((8, false)));
        assert_eq!(l.write_ump(&note, &mut buf), Ok((4, true)));
    }

    // REQ-USB-EVT-030: random UMPs and packet sizes give the same Event Packets as one
    // writer with no packet limit, cut into USB packets of at most wMaxPacketSize bytes.
    #[test]
    fn prop_event_packet_split_keeps_the_stream() {
        proptest!(|(raw in proptest::collection::vec(proptest::array::uniform4(any::<u32>()), 0..12),
                    size in prop::sample::select(&[4u16, 8, 16, 32, 64][..]),
                    unparsed in any::<bool>())| {
            let umps: Vec<&[u32]> = raw.iter().map(|w| &w[..ump_usb::ump_words(w[0])]).collect();
            let mut l = link_with(size, unparsed);
            let packets = drain(&mut l, &umps);
            for p in &packets {
                prop_assert!(p.len() <= size as usize);
            }
            prop_assert_eq!(packets.concat(), reference(&umps, unparsed));
        });
    }

    // REQ-USB-EVT-030, REQ-USB-UMP-008: the held Event Packets are discarded on each event.
    #[test]
    fn held_event_packets_are_discarded_after_each_event() {
        for event in EVENTS {
            let mut l = link_with(8, false);
            let mut it = [&RPN[..]].into_iter().peekable();
            let mut buf = [0u8; 8];
            assert_eq!(l.write_packet(&mut it, &mut buf), Ok(8));
            assert!(l.pending());
            l.report(event);
            assert!(!l.pending());
            assert!(!l.holds_part());
            l.report(LinkEvent::AlternateSetting(SettingNumber::MIDI1));
            assert_eq!(
                drain(&mut l, &[&[NOTE_ON]]),
                [[0x09, 0x90, 0x3C, 0x64].to_vec()]
            );
        }
    }

    #[test]
    fn holds_part_sees_open_received_sysex() {
        // REQ-USB-UMP-008: a received SysEx that is not ended is a held part (review L3).
        let mut l = link();
        assert!(read(&mut l, &[0x04, 0xF0, 0x01, 0x02]).is_empty());
        assert!(l.holds_part());
        assert!(!l.pending());
        l.report(LinkEvent::UsbReset);
        assert!(!l.holds_part());
    }

    #[test]
    fn report_setting_sets_mask_and_packet_size() {
        // REQ-USB-UMP-016, REQ-USB-UMP-008: each Alternate Setting has its own Group
        // Terminals and wMaxPacketSize (review L5, M9).
        let mut l = link();
        let mask = GroupMask::from_bits(0x0001);
        assert_eq!(l.report_setting(SettingNumber(1), mask, 8), Ok(()));
        assert_eq!(l.setting(), SettingNumber(1));
        assert_eq!(l.mask(), mask);
        assert_eq!(l.packet_size(), 8);
        // An invalid size changes nothing.
        assert_eq!(
            l.report_setting(SettingNumber(2), GroupMask::ALL, 3),
            Err(LinkError::InvalidPacketSize)
        );
        assert_eq!(l.setting(), SettingNumber(1));
        assert_eq!(l.mask(), mask);
        assert_eq!(l.packet_size(), 8);
        // Back to setting 0 with its own size.
        assert_eq!(
            l.report_setting(SettingNumber::MIDI1, GroupMask::ALL, 16),
            Ok(())
        );
        assert_eq!(l.mode(), LinkMode::EventPackets);
        assert_eq!(l.packet_size(), 16);
    }

    #[test]
    fn short_ump_is_reported_and_taken_in_both_modes() {
        // REQ-USB-UMP-009, REQ-USB-EVT-021: a UMP with too few words is not dropped silently
        // and does not stop the output (review M7).
        for setting in [0u8, 1] {
            let mut l = link_at(setting);
            let umps: [&[u32]; 3] = [&[NOTE_ON], &NOTE_ON2[..1], &[NOTE_OFF]];
            let mut it = umps.into_iter().peekable();
            let mut buf = [0xAAu8; 64];
            assert_eq!(l.write_packet(&mut it, &mut buf), Ok(4));
            let mut buf = [0xAAu8; 64];
            assert_eq!(
                l.write_packet(&mut it, &mut buf),
                Err(LinkError::ShortUmp { needed: 2, got: 1 })
            );
            assert_eq!(buf, [0xAA; 64]);
            assert_eq!(l.write_packet(&mut it, &mut buf), Ok(4));
            assert!(it.peek().is_none());
        }
        assert_eq!(
            LinkError::from(UmpUsbError::PacketFull),
            LinkError::PacketFull
        );
    }

    #[test]
    fn dropped_read_keeps_the_reader_state() {
        // REQ-USB-UMP-001, REQ-USB-GEN-006: a drop of the iterator before its end reads the
        // rest, so a UMP cut by the transfer gets the right words (review M1).
        let mut l = link_at(1);
        assert_eq!(l.read(&le(&[NOTE_ON, NOTE_ON2[0]])).take(1).count(), 1);
        assert!(l.holds_part());
        assert_eq!(
            read(&mut l, &le(&[NOTE_ON2[1], NOTE_OFF])),
            [NOTE_ON2.to_vec(), [NOTE_OFF].to_vec()]
        );
    }
}

#[cfg(all(test, not(feature = "alt0")))]
mod no_alt0_tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    #[test]
    fn without_alt0_alternate_setting_0_moves_no_data() {
        // REQ-USB-GEN-014: at Alternate Setting 0 the link reads no UMP from an Event Packet,
        // takes each UMP and writes no packet; Alternate Setting 1 still moves UMPs.
        let mut l = MidiLink::<1>::new(LinkConfig::default()).unwrap();
        assert_eq!(l.mode(), LinkMode::EventPackets);
        let note_on_packet = [0x09, 0x90, 0x3C, 0x64];
        assert_eq!(l.read(&note_on_packet).count(), 0);
        let mut got = Vec::new();
        l.read_with(&note_on_packet, |x| got.push(x));
        assert!(got.is_empty());
        let words: &[u32] = &[0x2090_3C64];
        let mut umps = [words, words].into_iter().peekable();
        let mut out = [0u8; 64];
        assert_eq!(l.write_packet(&mut umps, &mut out), Ok(0));
        assert!(umps.peek().is_none());
        assert!(!l.pending() && !l.holds_part());

        l.report(LinkEvent::AlternateSetting(SettingNumber(1)));
        let mut umps = [words].into_iter().peekable();
        assert_eq!(l.write_packet(&mut umps, &mut out), Ok(4));
        assert_eq!(l.read(&0x2090_3C64u32.to_le_bytes()).count(), 1);
    }
}
