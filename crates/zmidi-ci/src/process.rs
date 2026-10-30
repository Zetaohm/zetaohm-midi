//! MIDI-CI Process Inquiry (M2-101-UM v1.2.1 section 9, Sub-ID#2 0x40-0x44).
//!
//! - Inquiry: Process Inquiry Capabilities 0x40 (no data) and its Reply 0x41 (Supported
//!   Features, D0 MIDI Message Report) (9.2, 9.3).
//! - Inquiry: MIDI Message Report 0x42 (Table 43), Reply 0x43 (Table 44) and End 0x44
//!   (Table 46). The bitmaps: System D0 MTC Quarter Frame, D1 Song Position, D2 Song Select;
//!   Channel Controller D0 Pitchbend, D1 Control Change, D2 RPN, D3 NRPN, D4 Program Change,
//!   D5 Channel Pressure; Note Data D0 Notes, D1 Poly Pressure, D2 Per-Note Pitchbend,
//!   D3 Registered Per-Note Controller, D4 Assignable Per-Note Controller.
//! - [`ReportWriter`] gives the reported messages as UMP packets (M2-104-UM 7.3, 7.4, 7.6) in
//!   the order of 9.7.1 and with the rules of 9.7.2. The values come from the application
//!   through [`ReportSource`] (DEC-009); the writer holds no buffer (DEC-002).

use crate::message::{
    need, start, DeviceId, Framing, Header, Muid, ParseError, SubId2, WriteError,
};

/// Inquiry: Process Inquiry Capabilities, 0x40 (REQ-CI-PI-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PiCapabilitiesInquiry;

impl PiCapabilitiesInquiry {
    /// Reads the message; it has no data.
    pub fn read(_header: &Header, _body: &[u8]) -> Result<Self, ParseError> {
        Ok(PiCapabilitiesInquiry)
    }

    /// Writes the message with Device ID 0x7F.
    pub fn write(
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        start(
            DeviceId::FunctionBlock,
            SubId2::ProcessInquiryCapabilities,
            source,
            destination,
            framing,
            out,
        )
        .finish()
    }
}

/// Reply to Process Inquiry Capabilities, 0x41 (REQ-CI-PI-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PiCapabilities {
    /// D0: MIDI Message Report supported.
    pub midi_message_report: bool,
}

impl PiCapabilities {
    /// Reads the message; reserved bits are ignored.
    pub fn read(_header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        need(body, 1)?;
        Ok(PiCapabilities {
            midi_message_report: body[0] & 1 != 0,
        })
    }

    /// Writes the message with Device ID 0x7F.
    pub fn write(
        &self,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let mut w = start(
            DeviceId::FunctionBlock,
            SubId2::ProcessInquiryCapabilitiesReply,
            source,
            destination,
            framing,
            out,
        );
        w.u7(self.midi_message_report as u8);
        w.finish()
    }
}

/// Bitmap of System Messages (Table 43).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SystemBits {
    /// D0 MTC Quarter Frame.
    pub mtc_quarter_frame: bool,
    /// D1 Song Position.
    pub song_position: bool,
    /// D2 Song Select.
    pub song_select: bool,
}

impl SystemBits {
    /// Reads the bitmap byte; reserved bits are ignored.
    pub const fn from_byte(b: u8) -> Self {
        SystemBits {
            mtc_quarter_frame: b & (1 << 0) != 0,
            song_position: b & (1 << 1) != 0,
            song_select: b & (1 << 2) != 0,
        }
    }

    /// The bitmap byte; reserved bits are 0.
    pub const fn to_byte(self) -> u8 {
        self.mtc_quarter_frame as u8
            | (self.song_position as u8) << 1
            | (self.song_select as u8) << 2
    }
}

/// Bitmap of Channel Controller Messages (Table 43).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelBits {
    /// D0 Pitchbend.
    pub pitch_bend: bool,
    /// D1 Control Change.
    pub control_change: bool,
    /// D2 RPN / Registered Controller.
    pub registered: bool,
    /// D3 NRPN / Assignable Controller.
    pub assignable: bool,
    /// D4 Program Change.
    pub program_change: bool,
    /// D5 Channel Pressure.
    pub channel_pressure: bool,
}

impl ChannelBits {
    /// Reads the bitmap byte; reserved bits are ignored.
    pub const fn from_byte(b: u8) -> Self {
        ChannelBits {
            pitch_bend: b & (1 << 0) != 0,
            control_change: b & (1 << 1) != 0,
            registered: b & (1 << 2) != 0,
            assignable: b & (1 << 3) != 0,
            program_change: b & (1 << 4) != 0,
            channel_pressure: b & (1 << 5) != 0,
        }
    }

    /// The bitmap byte; reserved bits are 0.
    pub const fn to_byte(self) -> u8 {
        self.pitch_bend as u8
            | (self.control_change as u8) << 1
            | (self.registered as u8) << 2
            | (self.assignable as u8) << 3
            | (self.program_change as u8) << 4
            | (self.channel_pressure as u8) << 5
    }
}

/// Bitmap of Note Data Messages (Table 43).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoteBits {
    /// D0 Notes.
    pub notes: bool,
    /// D1 Poly Pressure.
    pub poly_pressure: bool,
    /// D2 Per-Note Pitchbend.
    pub per_note_pitch_bend: bool,
    /// D3 Registered Per-Note Controller.
    pub registered_per_note: bool,
    /// D4 Assignable Per-Note Controller.
    pub assignable_per_note: bool,
}

impl NoteBits {
    /// Reads the bitmap byte; reserved bits are ignored.
    pub const fn from_byte(b: u8) -> Self {
        NoteBits {
            notes: b & (1 << 0) != 0,
            poly_pressure: b & (1 << 1) != 0,
            per_note_pitch_bend: b & (1 << 2) != 0,
            registered_per_note: b & (1 << 3) != 0,
            assignable_per_note: b & (1 << 4) != 0,
        }
    }

    /// The bitmap byte; reserved bits are 0.
    pub const fn to_byte(self) -> u8 {
        self.notes as u8
            | (self.poly_pressure as u8) << 1
            | (self.per_note_pitch_bend as u8) << 2
            | (self.registered_per_note as u8) << 3
            | (self.assignable_per_note as u8) << 4
    }
}

/// Message Data Control (Table 43, REQ-CI-MMR-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataControl {
    /// 0x00: Reply and End only.
    BeginEndOnly,
    /// 0x01: non-default values and active notes only.
    NonDefault,
    /// 0x7F: full report.
    Full,
    /// 0x02-0x7E.
    Reserved(u8),
}

impl DataControl {
    /// Reads the byte.
    pub const fn from_byte(b: u8) -> Self {
        match b {
            0x00 => DataControl::BeginEndOnly,
            0x01 => DataControl::NonDefault,
            0x7F => DataControl::Full,
            v => DataControl::Reserved(v),
        }
    }

    /// The byte.
    pub const fn to_byte(self) -> u8 {
        match self {
            DataControl::BeginEndOnly => 0x00,
            DataControl::NonDefault => 0x01,
            DataControl::Full => 0x7F,
            DataControl::Reserved(v) => v,
        }
    }
}

/// Inquiry: MIDI Message Report, 0x42 (Table 43, REQ-CI-MMR-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageReportInquiry {
    /// Message Data Control.
    pub data_control: DataControl,
    /// Requested System Messages.
    pub system: SystemBits,
    /// Requested Channel Controller Messages.
    pub channel: ChannelBits,
    /// Requested Note Data Messages.
    pub note: NoteBits,
}

impl MessageReportInquiry {
    /// Reads the message.
    pub fn read(_header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        need(body, 5)?;
        Ok(MessageReportInquiry {
            data_control: DataControl::from_byte(body[0]),
            system: SystemBits::from_byte(body[1]),
            channel: ChannelBits::from_byte(body[3]),
            note: NoteBits::from_byte(body[4]),
        })
    }

    /// Writes the message; Device ID 0x7F whatever `device_id` is when System Messages are
    /// requested (REQ-CI-MMR-008).
    pub fn write(
        &self,
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let device_id = if self.system.to_byte() != 0 {
            DeviceId::FunctionBlock
        } else {
            device_id
        };
        let mut w = start(
            device_id,
            SubId2::MidiMessageReport,
            source,
            destination,
            framing,
            out,
        );
        w.u7(self.data_control.to_byte());
        w.u7(self.system.to_byte());
        w.u7(0);
        w.u7(self.channel.to_byte());
        w.u7(self.note.to_byte());
        w.finish()
    }
}

/// Reply to MIDI Message Report, 0x43 (Table 44, REQ-CI-MMR-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MessageReportReply {
    /// Reported System Messages.
    pub system: SystemBits,
    /// Reported Channel Controller Messages.
    pub channel: ChannelBits,
    /// Reported Note Data Messages.
    pub note: NoteBits,
}

impl MessageReportReply {
    /// Reads the message.
    pub fn read(_header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        need(body, 4)?;
        Ok(MessageReportReply {
            system: SystemBits::from_byte(body[0]),
            channel: ChannelBits::from_byte(body[2]),
            note: NoteBits::from_byte(body[3]),
        })
    }

    /// Writes the message with the Device ID of the inquiry.
    pub fn write(
        &self,
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let mut w = start(
            device_id,
            SubId2::MidiMessageReportReply,
            source,
            destination,
            framing,
            out,
        );
        w.u7(self.system.to_byte());
        w.u7(0);
        w.u7(self.channel.to_byte());
        w.u7(self.note.to_byte());
        w.finish()
    }

    /// The bits that are both requested and supported (REQ-CI-MMR-013).
    pub const fn declare(requested: &MessageReportInquiry, supported: &MessageReportReply) -> Self {
        MessageReportReply {
            system: SystemBits::from_byte(requested.system.to_byte() & supported.system.to_byte()),
            channel: ChannelBits::from_byte(
                requested.channel.to_byte() & supported.channel.to_byte(),
            ),
            note: NoteBits::from_byte(requested.note.to_byte() & supported.note.to_byte()),
        }
    }
}

/// End of MIDI Message Report, 0x44 (Table 46, REQ-CI-MMR-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageReportEnd;

impl MessageReportEnd {
    /// Reads the message; it has no data.
    pub fn read(_header: &Header, _body: &[u8]) -> Result<Self, ParseError> {
        Ok(MessageReportEnd)
    }

    /// Writes the message with the Device ID of the inquiry.
    pub fn write(
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        start(
            device_id,
            SubId2::MidiMessageReportEnd,
            source,
            destination,
            framing,
            out,
        )
        .finish()
    }
}

/// Format of the reported messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// MIDI 1.0 Channel Voice Messages (UMP Message Type 0x2).
    Midi1,
    /// MIDI 2.0 Channel Voice Messages (UMP Message Type 0x4).
    Midi2,
}

/// Current Mode of a Channel (Control Change 124-127).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMode {
    /// Poly Mode On (127), with Omni On (125) or Off (124).
    Poly { omni: bool },
    /// Mono Mode On (126) with the number of Channels, with Omni On (125) or Off (124).
    Mono { omni: bool, channels: u8 },
}

/// One reported note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteState {
    /// Note number, 7 bits.
    pub note: u8,
    /// True for an active note (Note On), false for an inactive note (Note Off).
    pub on: bool,
    /// Velocity, 16 bits; MIDI 1.0 uses the high 7 bits.
    pub velocity: u16,
    /// Attribute Type, 8 bits (MIDI 2.0).
    pub attribute_type: u8,
    /// Attribute Data, 16 bits (MIDI 2.0).
    pub attribute_data: u16,
}

/// One indexed value: RPN/NRPN or Registered/Assignable (per-note) Controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Controller {
    /// Bank, 7 bits; the note number for a per-note controller.
    pub bank: u8,
    /// Index, 7 bits (8 bits for a per-note controller).
    pub index: u8,
    /// Value: 32 bits in MIDI 2.0, 14 bits in MIDI 1.0.
    pub value: u32,
}

/// Current values that the application reports. `group` is the absolute Group 0-15 and
/// `channel` 0-15. Every method gives nothing by default. Values have the width of the
/// Protocol: MIDI 1.0 7 bits (14 bits for Pitch Bend and RPN/NRPN), MIDI 2.0 32 bits.
/// For DataControl NonDefault give only non-default values (REQ-CI-MMR-004): the crate does not
/// know the defaults (DEC-009). The report writer itself leaves out the inactive notes.
#[allow(unused_variables)]
pub trait ReportSource {
    /// The eight Quarter Frame data nibbles of the current time, pieces 0-7 (REQ-CI-MMR-020).
    fn mtc_quarter_frames(&self, group: u8) -> Option<[u8; 8]> {
        None
    }
    /// Song Position, 14 bits.
    fn song_position(&self, group: u8) -> Option<u16> {
        None
    }
    /// Song Select, 7 bits.
    fn song_select(&self, group: u8) -> Option<u8> {
        None
    }
    /// Pitch Bend.
    fn pitch_bend(&self, group: u8, channel: u8) -> Option<u32> {
        None
    }
    /// Control Change `index` (0-119, 122).
    fn control_change(&self, group: u8, channel: u8, index: u8) -> Option<u32> {
        None
    }
    /// True when CC 0-31 are MSB with LSB 32-63 (REQ-CI-MMR-021).
    fn cc_paired(&self, group: u8, channel: u8) -> bool {
        false
    }
    /// MIDI 1.0: true when CC 0, 6, 32, 38 or 98-101 is an individual controller and not
    /// Bank Select, RPN or NRPN (REQ-CI-MMR-026).
    fn individual_controller(&self, group: u8, channel: u8, index: u8) -> bool {
        false
    }
    /// The current Mode (REQ-CI-MMR-024, REQ-CI-MMR-025).
    fn mode(&self, group: u8, channel: u8) -> Option<ChannelMode> {
        None
    }
    /// Registered Controller (RPN) at `position` 0, 1, ...; nothing after the last.
    fn registered_controller(&self, group: u8, channel: u8, position: usize) -> Option<Controller> {
        None
    }
    /// Assignable Controller (NRPN) at `position`.
    fn assignable_controller(&self, group: u8, channel: u8, position: usize) -> Option<Controller> {
        None
    }
    /// Program and, when Bank Select is supported, Bank MSB and LSB.
    fn program(&self, group: u8, channel: u8) -> Option<(u8, Option<(u8, u8)>)> {
        None
    }
    /// Channel Pressure.
    fn channel_pressure(&self, group: u8, channel: u8) -> Option<u32> {
        None
    }
    /// Note at `position` (REQ-CI-MMR-011, REQ-CI-MMR-032, REQ-CI-MMR-033).
    fn note(&self, group: u8, channel: u8, position: usize) -> Option<NoteState> {
        None
    }
    /// Poly Pressure at `position`: note and value.
    fn poly_pressure(&self, group: u8, channel: u8, position: usize) -> Option<(u8, u32)> {
        None
    }
    /// Per-Note Pitch Bend at `position` (MIDI 2.0): note and value.
    fn per_note_pitch_bend(&self, group: u8, channel: u8, position: usize) -> Option<(u8, u32)> {
        None
    }
    /// Registered Per-Note Controller at `position` (MIDI 2.0); `bank` is the note.
    fn registered_per_note(&self, group: u8, channel: u8, position: usize) -> Option<Controller> {
        None
    }
    /// Assignable Per-Note Controller at `position` (MIDI 2.0); `bank` is the note.
    fn assignable_per_note(&self, group: u8, channel: u8, position: usize) -> Option<Controller> {
        None
    }
}

/// One UMP packet of a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportPacket {
    words: [u32; 2],
    len: usize,
    /// The Group of the packet.
    pub group: u8,
}

impl ReportPacket {
    /// The words of the packet (one or two).
    pub fn words(&self) -> &[u32] {
        &self.words[..self.len]
    }
}

/// Steps of the report of one Group (9.7.1). Positions count values from the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Mtc(u8),
    SongPosition,
    SongSelect,
    ChannelStart,
    PitchBend,
    /// Position in the Control Change order 0-120 (64 + 56 + CC 122).
    Cc(u8),
    /// Mode: 0 Omni message, 1 Poly/Mono message.
    Mode(u8),
    /// Registered Controller: position and part (MIDI 1.0 uses 4 CCs).
    Rpn(usize, u8),
    RpnNull(u8),
    Nrpn(usize, u8),
    NrpnNull(u8),
    /// Program: 0 Bank MSB, 1 Bank LSB, 2 Program Change.
    Program(u8),
    ChannelPressure,
    Note(usize),
    Poly(usize),
    PerNotePitchBend(usize),
    RegisteredPerNote(usize),
    AssignablePerNote(usize),
    NoteOff(usize),
    ChannelEnd,
    Done,
}

/// MIDI 1.0 velocity of a reported note: the high 7 bits; a Note On never gets 0, which
/// means Note Off (M2-104 Translation rule).
fn midi1_velocity(n: NoteState) -> u8 {
    let v = (n.velocity >> 9) as u8;
    if n.on && v == 0 {
        1
    } else {
        v
    }
}

/// Pull iterator over the reported messages (9.7, REQ-CI-MMR-014 to REQ-CI-MMR-035).
#[derive(Debug)]
pub struct ReportWriter<'s, S: ReportSource + ?Sized> {
    source: &'s S,
    bits: MessageReportReply,
    protocol: Protocol,
    /// Groups to report: current and last (inclusive).
    group: u8,
    last_group: u8,
    /// Channels to report as a mask, and the current Channel.
    channels: u16,
    channel: u8,
    system: bool,
    /// Message Data Control 0x01: report active notes only (REQ-CI-MMR-004).
    active_only: bool,
    step: Step,
}

impl<'s, S: ReportSource + ?Sized> ReportWriter<'s, S> {
    /// A report writer for the declared `bits`. `group` is the Group of the inquiry;
    /// `first_group` and `group_count` the Groups of the Function Block; `channels_in_use`
    /// the Channels of each Group as a mask (bit n = Channel n+1).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        bits: MessageReportReply,
        data_control: DataControl,
        device_id: DeviceId,
        group: u8,
        first_group: u8,
        group_count: u8,
        channels_in_use: u16,
        protocol: Protocol,
        source: &'s S,
    ) -> Self {
        let (g0, g1, channels, system) = match device_id {
            DeviceId::Channel(c) => (group, group, channels_in_use & (1 << (c & 0x0F)), false),
            DeviceId::Group => (group, group, channels_in_use, false),
            DeviceId::FunctionBlock => {
                let last = first_group.saturating_add(group_count.max(1) - 1).min(15);
                (first_group.min(15), last, channels_in_use, true)
            }
        };
        let report = matches!(data_control, DataControl::NonDefault | DataControl::Full);
        let mut w = ReportWriter {
            source,
            bits,
            protocol,
            group: g0 & 0x0F,
            last_group: g1,
            channels,
            channel: 0,
            system,
            active_only: data_control == DataControl::NonDefault,
            step: if report { Step::Mtc(0) } else { Step::Done },
        };
        if report && !w.system {
            w.step = Step::ChannelStart;
        }
        w
    }

    fn packet1(&self, w0: u32) -> ReportPacket {
        ReportPacket {
            words: [w0, 0],
            len: 1,
            group: self.group,
        }
    }

    fn packet2(&self, w0: u32, w1: u32) -> ReportPacket {
        ReportPacket {
            words: [w0, w1],
            len: 2,
            group: self.group,
        }
    }

    fn system_word(&self, status: u8, d1: u8, d2: u8) -> ReportPacket {
        self.packet1(
            0x1000_0000
                | (self.group as u32) << 24
                | (status as u32) << 16
                | ((d1 & 0x7F) as u32) << 8
                | (d2 & 0x7F) as u32,
        )
    }

    fn m1(&self, op: u8, d1: u8, d2: u8) -> ReportPacket {
        self.packet1(
            0x2000_0000
                | (self.group as u32) << 24
                | (op as u32) << 20
                | (self.channel as u32) << 16
                | ((d1 & 0x7F) as u32) << 8
                | (d2 & 0x7F) as u32,
        )
    }

    fn m2(&self, op: u8, b2: u8, b3: u8, w1: u32) -> ReportPacket {
        self.packet2(
            0x4000_0000
                | (self.group as u32) << 24
                | (op as u32) << 20
                | (self.channel as u32) << 16
                | (b2 as u32) << 8
                | b3 as u32,
            w1,
        )
    }

    fn cc(&self, index: u8, value: u32) -> ReportPacket {
        match self.protocol {
            Protocol::Midi1 => self.m1(0xB, index, value as u8),
            Protocol::Midi2 => self.m2(0xB, index & 0x7F, 0, value),
        }
    }

    fn cc_index(&self, p: u8) -> u8 {
        let paired = self.source.cc_paired(self.group, self.channel);
        match p {
            0..=63 if paired => p / 2 + if p % 2 == 1 { 32 } else { 0 },
            0..=119 => p,
            _ => 122,
        }
    }

    fn cc_allowed(&self, index: u8) -> bool {
        match index {
            // MIDI 1.0: Bank Select, RPN and NRPN use these (REQ-CI-MMR-026); MIDI 2.0 sends
            // them in Program Change and Registered/Assignable Controllers, so they are sent
            // only as individual controllers in both Protocols.
            0 | 6 | 32 | 38 | 98..=101 => {
                self.source
                    .individual_controller(self.group, self.channel, index)
            }
            _ => true,
        }
    }

    fn next_channel(&mut self) -> Step {
        let mut c = self.channel as u16 + 1;
        while c < 16 && self.channels & (1 << c) == 0 {
            c += 1;
        }
        if c < 16 {
            self.channel = c as u8;
            return Step::PitchBend;
        }
        if self.group < self.last_group {
            self.group += 1;
            return if self.system {
                Step::Mtc(0)
            } else {
                Step::ChannelStart
            };
        }
        Step::Done
    }

    /// Gives the next packet, or `None` at the end of the report. Never panics.
    pub fn next_packet(&mut self) -> Option<ReportPacket> {
        let src = self.source;
        let (sys, ch, note) = (self.bits.system, self.bits.channel, self.bits.note);
        let midi1 = self.protocol == Protocol::Midi1;
        loop {
            // The Group can change inside this loop (next_channel): read it each time.
            let (g, c) = (self.group, self.channel);
            match self.step {
                Step::Mtc(i) if i < 8 => {
                    self.step = Step::Mtc(i + 1);
                    if sys.mtc_quarter_frame {
                        if let Some(frames) = src.mtc_quarter_frames(self.group) {
                            return Some(self.system_word(
                                0xF1,
                                (i << 4) | (frames[i as usize] & 0x0F),
                                0,
                            ));
                        }
                    }
                    if !sys.mtc_quarter_frame || src.mtc_quarter_frames(self.group).is_none() {
                        self.step = Step::SongPosition;
                    }
                }
                Step::Mtc(_) => self.step = Step::SongPosition,
                Step::SongPosition => {
                    self.step = Step::SongSelect;
                    if let Some(p) = sys
                        .song_position
                        .then(|| src.song_position(self.group))
                        .flatten()
                    {
                        return Some(self.system_word(
                            0xF2,
                            (p & 0x7F) as u8,
                            ((p >> 7) & 0x7F) as u8,
                        ));
                    }
                }
                Step::SongSelect => {
                    self.step = Step::ChannelStart;
                    if let Some(s) = sys
                        .song_select
                        .then(|| src.song_select(self.group))
                        .flatten()
                    {
                        return Some(self.system_word(0xF3, s, 0));
                    }
                }
                Step::ChannelStart => {
                    let first = (0..16).find(|c| self.channels & (1 << c) != 0);
                    match first {
                        Some(f) => {
                            self.channel = f;
                            self.step = Step::PitchBend;
                        }
                        None => {
                            self.channel = 15;
                            self.step = self.next_channel();
                        }
                    }
                }
                Step::PitchBend => {
                    self.step = Step::Cc(0);
                    if let Some(v) = ch
                        .pitch_bend
                        .then(|| src.pitch_bend(self.group, c))
                        .flatten()
                    {
                        return Some(if midi1 {
                            self.m1(0xE, (v & 0x7F) as u8, ((v >> 7) & 0x7F) as u8)
                        } else {
                            self.m2(0xE, 0, 0, v)
                        });
                    }
                }
                Step::Cc(p) if p <= 120 && ch.control_change => {
                    self.step = Step::Cc(p + 1);
                    let i = self.cc_index(p);
                    if self.cc_allowed(i) {
                        if let Some(v) = src.control_change(self.group, c, i) {
                            return Some(self.cc(i, v));
                        }
                    }
                }
                Step::Cc(_) => self.step = Step::Mode(0),
                Step::Mode(phase) => {
                    self.step = if phase == 0 {
                        Step::Mode(1)
                    } else {
                        Step::Rpn(0, 0)
                    };
                    if !ch.control_change {
                        continue;
                    }
                    if let Some(m) = src.mode(self.group, c) {
                        let (ChannelMode::Poly { omni } | ChannelMode::Mono { omni, .. }) = m;
                        return Some(match (phase, m) {
                            (0, _) => self.cc(if omni { 125 } else { 124 }, 0),
                            (_, ChannelMode::Poly { .. }) => self.cc(127, 0),
                            (_, ChannelMode::Mono { channels, .. }) => {
                                self.cc(126, channels as u32)
                            }
                        });
                    }
                }
                Step::Rpn(pos, part) => {
                    let value = ch
                        .registered
                        .then(|| src.registered_controller(g, c, pos))
                        .flatten();
                    match value {
                        None => {
                            self.step = if pos > 0 && midi1 {
                                Step::RpnNull(0)
                            } else {
                                Step::Nrpn(0, 0)
                            }
                        }
                        Some(r) if midi1 => {
                            self.step = if part < 3 {
                                Step::Rpn(pos, part + 1)
                            } else {
                                Step::Rpn(pos + 1, 0)
                            };
                            let (i, v) = [
                                (101, r.bank),
                                (100, r.index),
                                (6, (r.value >> 7) as u8),
                                (38, r.value as u8),
                            ][part as usize];
                            return Some(self.m1(0xB, i, v));
                        }
                        Some(r) => {
                            self.step = Step::Rpn(pos + 1, 0);
                            return Some(self.m2(0x2, r.bank & 0x7F, r.index & 0x7F, r.value));
                        }
                    }
                }
                Step::RpnNull(part) => {
                    self.step = if part == 0 {
                        Step::RpnNull(1)
                    } else {
                        Step::Nrpn(0, 0)
                    };
                    return Some(self.m1(0xB, if part == 0 { 101 } else { 100 }, 127));
                }
                Step::Nrpn(pos, part) => {
                    let value = ch
                        .assignable
                        .then(|| src.assignable_controller(g, c, pos))
                        .flatten();
                    match value {
                        None => {
                            self.step = if pos > 0 && midi1 {
                                Step::NrpnNull(0)
                            } else {
                                Step::Program(0)
                            }
                        }
                        Some(r) if midi1 => {
                            self.step = if part < 3 {
                                Step::Nrpn(pos, part + 1)
                            } else {
                                Step::Nrpn(pos + 1, 0)
                            };
                            let (i, v) = [
                                (99, r.bank),
                                (98, r.index),
                                (6, (r.value >> 7) as u8),
                                (38, r.value as u8),
                            ][part as usize];
                            return Some(self.m1(0xB, i, v));
                        }
                        Some(r) => {
                            self.step = Step::Nrpn(pos + 1, 0);
                            return Some(self.m2(0x3, r.bank & 0x7F, r.index & 0x7F, r.value));
                        }
                    }
                }
                Step::NrpnNull(part) => {
                    self.step = if part == 0 {
                        Step::NrpnNull(1)
                    } else {
                        Step::Program(0)
                    };
                    return Some(self.m1(0xB, if part == 0 { 99 } else { 98 }, 127));
                }
                Step::Program(part) => {
                    let value = ch.program_change.then(|| src.program(g, c)).flatten();
                    let Some((program, bank)) = value else {
                        self.step = Step::ChannelPressure;
                        continue;
                    };
                    if !midi1 {
                        self.step = Step::ChannelPressure;
                        let (b, (msb, lsb)) = (bank.is_some() as u8, bank.unwrap_or((0, 0)));
                        let w1 = ((program & 0x7F) as u32) << 24
                            | ((msb & 0x7F) as u32) << 8
                            | (lsb & 0x7F) as u32;
                        return Some(self.m2(0xC, 0, b, w1));
                    }
                    match (part, bank) {
                        (0, Some((msb, _))) => {
                            self.step = Step::Program(1);
                            return Some(self.m1(0xB, 0, msb));
                        }
                        (1, Some((_, lsb))) => {
                            self.step = Step::Program(2);
                            return Some(self.m1(0xB, 32, lsb));
                        }
                        _ => {
                            self.step = Step::ChannelPressure;
                            return Some(self.m1(0xC, program, 0));
                        }
                    }
                }
                Step::ChannelPressure => {
                    self.step = Step::Note(0);
                    if let Some(v) = ch
                        .channel_pressure
                        .then(|| src.channel_pressure(g, c))
                        .flatten()
                    {
                        return Some(if midi1 {
                            self.m1(0xD, v as u8, 0)
                        } else {
                            self.m2(0xD, 0, 0, v)
                        });
                    }
                }
                Step::Note(pos) => {
                    let Some(n) = note.notes.then(|| src.note(g, c, pos)).flatten() else {
                        self.step = Step::Poly(0);
                        continue;
                    };
                    self.step = Step::Note(pos + 1);
                    // Table 44 0x01: active notes only (REQ-CI-MMR-004).
                    if self.active_only && !n.on {
                        continue;
                    }
                    let op = if n.on { 0x9 } else { 0x8 };
                    return Some(if midi1 {
                        self.m1(op, n.note, midi1_velocity(n))
                    } else {
                        self.m2(
                            op,
                            n.note & 0x7F,
                            n.attribute_type,
                            (n.velocity as u32) << 16 | n.attribute_data as u32,
                        )
                    });
                }
                Step::Poly(pos) => {
                    let Some((k, v)) = note
                        .poly_pressure
                        .then(|| src.poly_pressure(g, c, pos))
                        .flatten()
                    else {
                        self.step = Step::PerNotePitchBend(0);
                        continue;
                    };
                    self.step = Step::Poly(pos + 1);
                    return Some(if midi1 {
                        self.m1(0xA, k, v as u8)
                    } else {
                        self.m2(0xA, k & 0x7F, 0, v)
                    });
                }
                Step::PerNotePitchBend(pos) => {
                    let value = (note.per_note_pitch_bend && !midi1)
                        .then(|| src.per_note_pitch_bend(g, c, pos))
                        .flatten();
                    let Some((k, v)) = value else {
                        self.step = Step::RegisteredPerNote(0);
                        continue;
                    };
                    self.step = Step::PerNotePitchBend(pos + 1);
                    return Some(self.m2(0x6, k & 0x7F, 0, v));
                }
                Step::RegisteredPerNote(pos) => {
                    let value = (note.registered_per_note && !midi1)
                        .then(|| src.registered_per_note(g, c, pos))
                        .flatten();
                    let Some(r) = value else {
                        self.step = Step::AssignablePerNote(0);
                        continue;
                    };
                    self.step = Step::RegisteredPerNote(pos + 1);
                    return Some(self.m2(0x0, r.bank & 0x7F, r.index, r.value));
                }
                Step::AssignablePerNote(pos) => {
                    let value = (note.assignable_per_note && !midi1)
                        .then(|| src.assignable_per_note(g, c, pos))
                        .flatten();
                    let Some(r) = value else {
                        self.step = Step::NoteOff(0);
                        continue;
                    };
                    self.step = Step::AssignablePerNote(pos + 1);
                    return Some(self.m2(0x1, r.bank & 0x7F, r.index, r.value));
                }
                Step::NoteOff(pos) => {
                    let Some(n) = note.notes.then(|| src.note(g, c, pos)).flatten() else {
                        self.step = Step::ChannelEnd;
                        continue;
                    };
                    self.step = Step::NoteOff(pos + 1);
                    if n.on {
                        return Some(if midi1 {
                            self.m1(0x8, n.note, 0)
                        } else {
                            self.m2(0x8, n.note & 0x7F, 0, 0)
                        });
                    }
                }
                Step::ChannelEnd => self.step = self.next_channel(),
                Step::Done => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;
    use zmidi_ump::channel_voice1::Midi1ChannelVoiceMessage;
    use zmidi_ump::channel_voice2::Midi2ChannelVoiceMessage;
    use zmidi_ump::system::SystemMessage;

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

    /// A source with a value of each kind; the values have bit 7 set where the writer must
    /// mask it.
    struct Every;

    impl ReportSource for Every {
        fn mtc_quarter_frames(&self, g: u8) -> Option<[u8; 8]> {
            Some([g, 1, 2, 3, 4, 5, 6, 0x1F])
        }
        fn song_position(&self, g: u8) -> Option<u16> {
            Some(0x1234 + u16::from(g))
        }
        fn song_select(&self, g: u8) -> Option<u8> {
            Some(0x40 + g)
        }
        fn pitch_bend(&self, g: u8, c: u8) -> Option<u32> {
            Some(0x8000_0000 | u32::from(g) << 8 | u32::from(c))
        }
        fn control_change(&self, _: u8, c: u8, i: u8) -> Option<u32> {
            (i == 7).then_some(0x1100_00C0 | u32::from(c))
        }
        fn mode(&self, _: u8, c: u8) -> Option<ChannelMode> {
            Some(if c == 1 {
                ChannelMode::Poly { omni: true }
            } else {
                ChannelMode::Mono {
                    omni: false,
                    channels: 3,
                }
            })
        }
        fn registered_controller(&self, _: u8, _: u8, p: usize) -> Option<Controller> {
            (p < 2).then(|| Controller {
                bank: 0x85 + p as u8,
                index: 0x87 + p as u8,
                value: 0x1F2F + p as u32,
            })
        }
        fn assignable_controller(&self, _: u8, _: u8, p: usize) -> Option<Controller> {
            (p < 2).then(|| Controller {
                bank: 0x95 + p as u8,
                index: 0x97 + p as u8,
                value: 0x2F1F + p as u32,
            })
        }
        fn program(&self, _: u8, _: u8) -> Option<(u8, Option<(u8, u8)>)> {
            Some((0x85, Some((0x81, 0x82))))
        }
        fn channel_pressure(&self, _: u8, c: u8) -> Option<u32> {
            Some(0x2200_00C0 | u32::from(c))
        }
        fn note(&self, _: u8, _: u8, p: usize) -> Option<NoteState> {
            match p {
                0 => Some(NoteState {
                    note: 0x83,
                    on: true,
                    velocity: 0xABCD,
                    attribute_type: 3,
                    attribute_data: 0x1234,
                }),
                1 => Some(NoteState {
                    note: 0x10,
                    on: false,
                    velocity: 0x0100,
                    attribute_type: 0,
                    attribute_data: 0,
                }),
                _ => None,
            }
        }
        fn poly_pressure(&self, _: u8, _: u8, p: usize) -> Option<(u8, u32)> {
            (p == 0).then_some((0x84, 0x3300_0045))
        }
        fn per_note_pitch_bend(&self, _: u8, _: u8, p: usize) -> Option<(u8, u32)> {
            (p < 2).then(|| (0x85 + p as u8, 0x4400_0000 + p as u32))
        }
        fn registered_per_note(&self, _: u8, _: u8, p: usize) -> Option<Controller> {
            (p < 2).then(|| Controller {
                bank: 0x86 + p as u8,
                index: 0x90 + p as u8,
                value: 0x5500_0000 + p as u32,
            })
        }
        fn assignable_per_note(&self, _: u8, _: u8, p: usize) -> Option<Controller> {
            (p < 2).then(|| Controller {
                bank: 0x88 + p as u8,
                index: 0xA0 + p as u8,
                value: 0x6600_0000 + p as u32,
            })
        }
    }

    /// A source with no value.
    struct Nothing;
    impl ReportSource for Nothing {}

    fn every_bit() -> MessageReportReply {
        MessageReportReply {
            system: SystemBits::from_byte(0x07),
            channel: ChannelBits::from_byte(0x3F),
            note: NoteBits::from_byte(0x1F),
        }
    }

    fn report<S: ReportSource>(source: &S, protocol: Protocol) -> Vec<(u8, Vec<u32>)> {
        // The Function Block: Groups 2 and 3, Channels 2 and 3 (bits 1 and 2).
        let mut w = ReportWriter::new(
            every_bit(),
            DataControl::Full,
            DeviceId::FunctionBlock,
            0,
            2,
            2,
            0b110,
            protocol,
            source,
        );
        let mut out = Vec::new();
        while let Some(p) = w.next_packet() {
            out.push((p.group, p.words[..p.len].to_vec()));
            assert!(out.len() < 1000);
        }
        out
    }

    /// The report written by hand (Tables 43 and 47, M2-104 Figures 28-30, 45-67).
    fn expected(protocol: Protocol) -> Vec<(u8, Vec<u32>)> {
        let midi1 = protocol == Protocol::Midi1;
        let mut out = Vec::new();
        for g in 2u8..=3 {
            let gw = u32::from(g) << 24;
            let sys =
                |st: u32, d1: u32, d2: u32| std::vec![0x1000_0000 | gw | st << 16 | d1 << 8 | d2];
            let frames = [g, 1, 2, 3, 4, 5, 6, 0x0F];
            for (i, f) in frames.iter().enumerate() {
                out.push((g, sys(0xF1, (i as u32) << 4 | u32::from(*f), 0)));
            }
            let sp = 0x1234 + u32::from(g);
            out.push((g, sys(0xF2, sp & 0x7F, (sp >> 7) & 0x7F)));
            out.push((g, sys(0xF3, 0x40 + u32::from(g), 0)));
            for c in [1u32, 2] {
                let m1 = |op: u32, d1: u32, d2: u32| {
                    std::vec![
                        0x2000_0000 | gw | op << 20 | c << 16 | (d1 & 0x7F) << 8 | (d2 & 0x7F)
                    ]
                };
                let m2 = |op: u32, b2: u32, b3: u32, w1: u32| {
                    std::vec![0x4000_0000 | gw | op << 20 | c << 16 | b2 << 8 | b3, w1]
                };
                let mut push = |v: Vec<u32>| out.push((g, v));
                let pb = 0x8000_0000 | u32::from(g) << 8 | c;
                let cc7 = 0x1100_00C0 | c;
                if midi1 {
                    push(m1(0xE, pb & 0x7F, (pb >> 7) & 0x7F));
                    push(m1(0xB, 7, cc7 & 0xFF));
                    if c == 1 {
                        push(m1(0xB, 125, 0));
                        push(m1(0xB, 127, 0));
                    } else {
                        push(m1(0xB, 124, 0));
                        push(m1(0xB, 126, 3));
                    }
                    for (cc, base) in [
                        ((101, 100), (0x85, 0x87, 0x1F2F)),
                        ((99, 98), (0x95, 0x97, 0x2F1F)),
                    ] {
                        for p in 0..2u32 {
                            let v = base.2 + p;
                            push(m1(0xB, cc.0, base.0 + p));
                            push(m1(0xB, cc.1, base.1 + p));
                            push(m1(0xB, 6, (v >> 7) & 0xFF));
                            push(m1(0xB, 38, v & 0xFF));
                        }
                        push(m1(0xB, cc.0, 127));
                        push(m1(0xB, cc.1, 127));
                    }
                    push(m1(0xB, 0, 0x81));
                    push(m1(0xB, 32, 0x82));
                    push(m1(0xC, 0x85, 0));
                    push(m1(0xD, 0xC0 | c, 0));
                    push(m1(0x9, 0x83, 0xABCD >> 9));
                    push(m1(0x8, 0x10, 0x0100 >> 9));
                    push(m1(0xA, 0x84, 0x45));
                    push(m1(0x8, 0x83, 0));
                } else {
                    push(m2(0xE, 0, 0, pb));
                    push(m2(0xB, 7, 0, cc7));
                    if c == 1 {
                        push(m2(0xB, 125, 0, 0));
                        push(m2(0xB, 127, 0, 0));
                    } else {
                        push(m2(0xB, 124, 0, 0));
                        push(m2(0xB, 126, 0, 3));
                    }
                    for p in 0..2u32 {
                        push(m2(0x2, (0x85 + p) & 0x7F, (0x87 + p) & 0x7F, 0x1F2F + p));
                    }
                    for p in 0..2u32 {
                        push(m2(0x3, (0x95 + p) & 0x7F, (0x97 + p) & 0x7F, 0x2F1F + p));
                    }
                    push(m2(0xC, 0, 1, 0x05 << 24 | 0x01 << 8 | 0x02));
                    push(m2(0xD, 0, 0, 0x2200_00C0 | c));
                    push(m2(0x9, 0x03, 3, 0xABCD << 16 | 0x1234));
                    push(m2(0x8, 0x10, 0, 0x0100 << 16));
                    push(m2(0xA, 0x04, 0, 0x3300_0045));
                    for p in 0..2u32 {
                        push(m2(0x6, (0x85 + p) & 0x7F, 0, 0x4400_0000 + p));
                    }
                    for p in 0..2u32 {
                        push(m2(0x0, (0x86 + p) & 0x7F, 0x90 + p, 0x5500_0000 + p));
                    }
                    for p in 0..2u32 {
                        push(m2(0x1, (0x88 + p) & 0x7F, 0xA0 + p, 0x6600_0000 + p));
                    }
                    push(m2(0x8, 0x03, 0, 0));
                }
            }
        }
        out
    }

    #[test]
    fn whole_report_of_every_kind() {
        // REQ-CI-MMR-007 to -033: each message kind, both Protocols, two Groups and two
        // Channels, in the order of the writer, with the masks of each field.
        for protocol in [Protocol::Midi1, Protocol::Midi2] {
            let got = report(&Every, protocol);
            let want = expected(protocol);
            assert_eq!(got.len(), want.len(), "{protocol:?}");
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                assert_eq!(g, w, "{protocol:?} packet {i}");
            }
            // A source with no value gives no message.
            assert!(report(&Nothing, protocol).is_empty(), "{protocol:?}");
        }
        // Each bit of the bitmaps (Table 43); 0 gives no field.
        assert_eq!(SystemBits::from_byte(0), SystemBits::default());
        assert_eq!(ChannelBits::from_byte(0), ChannelBits::default());
        assert_eq!(NoteBits::from_byte(0), NoteBits::default());
        for b in 0..3 {
            assert_eq!(SystemBits::from_byte(1 << b).to_byte(), 1 << b);
        }
        for b in 0..6 {
            assert_eq!(ChannelBits::from_byte(1 << b).to_byte(), 1 << b);
        }
        for b in 0..5 {
            assert_eq!(NoteBits::from_byte(1 << b).to_byte(), 1 << b);
        }
        // REQ-CI-MMR-013: only the bits that are both requested and supported.
        let q = MessageReportInquiry {
            data_control: DataControl::Full,
            system: SystemBits::from_byte(0b011),
            channel: ChannelBits::from_byte(0b10_1010),
            note: NoteBits::from_byte(0b1_0110),
        };
        let s = MessageReportReply {
            system: SystemBits::from_byte(0b110),
            channel: ChannelBits::from_byte(0b01_1110),
            note: NoteBits::from_byte(0b0_0111),
        };
        let d = MessageReportReply::declare(&q, &s);
        assert_eq!(
            (d.system.to_byte(), d.channel.to_byte(), d.note.to_byte()),
            (0b010, 0b00_1010, 0b0_0110)
        );
    }

    #[test]
    fn capabilities_and_report_messages() {
        // 9.2-9.8, Tables 43, 44, 46; REQ-CI-PI-001, -002, REQ-CI-MMR-001, -002, -005, -006, -008.
        let mut buf = [0u8; 32];
        let n = PiCapabilitiesInquiry::write(SRC, DST, Framing::Bare, &mut buf).unwrap();
        assert_eq!(
            &buf[..n],
            &[0x7E, 0x7F, 0x0D, 0x40, 0x02, 0x56, 0x68, 0x48, 0x00, 0x21, 0x06, 0x15, 0x03]
        );
        let n = PiCapabilities {
            midi_message_report: true,
        }
        .write(SRC, DST, Framing::Bare, &mut buf)
        .unwrap();
        // Table 41; REQ-CI-PI-002: D0 set, reserved bits D1-D6 written as 0.
        assert_eq!((buf[1], buf[3], buf[13], n), (0x7F, 0x41, 0x01, 14));
        let n0 = PiCapabilities::default()
            .write(SRC, DST, Framing::Bare, &mut [0u8; 32])
            .unwrap();
        assert_eq!(n0, 14);
        assert!(
            PiCapabilities::read(&parse(&buf[..n]).0, &[0x7F])
                .unwrap()
                .midi_message_report
        );
        let (h, body) = parse(&buf[..n]);
        assert!(PiCapabilities::read(&h, body).unwrap().midi_message_report);
        assert!(
            !PiCapabilities::read(&h, &[0x7E])
                .unwrap()
                .midi_message_report
        );

        let q = MessageReportInquiry {
            data_control: DataControl::Full,
            system: SystemBits {
                song_position: true,
                ..Default::default()
            },
            channel: ChannelBits {
                control_change: true,
                program_change: true,
                ..Default::default()
            },
            note: NoteBits {
                notes: true,
                ..Default::default()
            },
        };
        let n = q
            .write(DeviceId::Channel(3), SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        assert_eq!(buf[1], 0x7F, "System Messages need Device ID 0x7F");
        assert_eq!(&buf[13..n], &[0x7F, 0x02, 0x00, 0x12, 0x01]);
        let (h, body) = parse(&buf[..n]);
        assert_eq!(MessageReportInquiry::read(&h, body), Ok(q));
        // REQ-CI-MMR-001: the reader ignores the reserved byte and the reserved bits.
        assert_eq!(
            MessageReportInquiry::read(&h, &[0x7F, 0x7A, 0x55, 0x52, 0x61]),
            Ok(q)
        );
        assert_eq!(DataControl::from_byte(0x05), DataControl::Reserved(5));
        // Table 44; REQ-CI-MMR-002: each value both ways.
        for (b, dc) in [
            (0x00, DataControl::BeginEndOnly),
            (0x01, DataControl::NonDefault),
            (0x02, DataControl::Reserved(0x02)),
            (0x7E, DataControl::Reserved(0x7E)),
            (0x7F, DataControl::Full),
        ] {
            assert_eq!(DataControl::from_byte(b), dc);
            assert_eq!(dc.to_byte(), b);
        }
        let q1 = MessageReportInquiry {
            data_control: DataControl::NonDefault,
            ..q
        };
        let n = q1
            .write(DeviceId::Group, SRC, DST, Framing::Bare, &mut [0u8; 32][..])
            .unwrap();
        assert_eq!(n, 18);
        assert_eq!(ChannelBits::from_byte(0xFF).to_byte(), 0x3F);
        assert_eq!(NoteBits::from_byte(0xFF).to_byte(), 0x1F);
        assert_eq!(SystemBits::from_byte(0xFF).to_byte(), 0x07);

        let supported = MessageReportReply {
            channel: ChannelBits::from_byte(0x3F),
            ..Default::default()
        };
        let r = MessageReportReply::declare(&q, &supported);
        assert_eq!(r.channel.to_byte(), 0x12);
        assert_eq!(r.system.to_byte(), 0);
        let n = r
            .write(DeviceId::Group, SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        let (h, body) = parse(&buf[..n]);
        assert_eq!(MessageReportReply::read(&h, body), Ok(r));
        // Table 45; REQ-CI-MMR-005: System, reserved 0x00, Channel, Note; reserved bits 0.
        let r = MessageReportReply {
            system: SystemBits::from_byte(0x02),
            channel: ChannelBits::from_byte(0x12),
            note: NoteBits::from_byte(0x01),
        };
        let n = r
            .write(DeviceId::Channel(3), SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
        assert_eq!((buf[1], buf[3]), (0x03, 0x43));
        assert_eq!(&buf[13..n], &[0x02, 0x00, 0x12, 0x01]);
        // The reader ignores the reserved byte and the reserved bits.
        let (h, _) = parse(&buf[..n]);
        assert_eq!(
            MessageReportReply::read(&h, &[0x7A, 0x55, 0x52, 0x61]),
            Ok(r)
        );
        assert_eq!(
            MessageReportReply::read(&h, &[0x02, 0x00, 0x12]),
            Err(ParseError::TooShort)
        );
        let n =
            MessageReportEnd::write(DeviceId::Group, SRC, DST, Framing::Bare, &mut buf).unwrap();
        assert_eq!((buf[3], n), (0x44, 13));
    }

    #[derive(Default)]
    struct Synth {
        paired: bool,
    }

    impl ReportSource for Synth {
        fn mtc_quarter_frames(&self, _g: u8) -> Option<[u8; 8]> {
            Some([1, 2, 3, 4, 5, 6, 7, 8])
        }
        fn song_position(&self, _g: u8) -> Option<u16> {
            Some(0x1234)
        }
        fn pitch_bend(&self, _g: u8, _c: u8) -> Option<u32> {
            Some(0x2000)
        }
        fn control_change(&self, _g: u8, _c: u8, index: u8) -> Option<u32> {
            matches!(index, 0 | 1 | 7 | 33 | 64 | 122).then_some(index as u32)
        }
        fn cc_paired(&self, _g: u8, _c: u8) -> bool {
            self.paired
        }
        fn mode(&self, _g: u8, _c: u8) -> Option<ChannelMode> {
            Some(ChannelMode::Mono {
                omni: false,
                channels: 4,
            })
        }
        fn registered_controller(&self, _g: u8, _c: u8, p: usize) -> Option<Controller> {
            (p == 0).then_some(Controller {
                bank: 0,
                index: 0,
                value: 0x100,
            })
        }
        fn program(&self, _g: u8, _c: u8) -> Option<(u8, Option<(u8, u8)>)> {
            Some((5, Some((1, 2))))
        }
        fn note(&self, _g: u8, _c: u8, p: usize) -> Option<NoteState> {
            [
                NoteState {
                    note: 60,
                    on: true,
                    velocity: 0xFE00,
                    attribute_type: 3,
                    attribute_data: 0x1234,
                },
                NoteState {
                    note: 61,
                    on: false,
                    velocity: 0,
                    attribute_type: 0,
                    attribute_data: 0,
                },
            ]
            .get(p)
            .copied()
        }
    }

    fn all_bits() -> MessageReportReply {
        MessageReportReply {
            system: SystemBits::from_byte(0x7F),
            channel: ChannelBits::from_byte(0x7F),
            note: NoteBits::from_byte(0x7F),
        }
    }

    fn collect<S: ReportSource>(w: &mut ReportWriter<'_, S>) -> Vec<ReportPacket> {
        let mut v = Vec::new();
        while let Some(p) = w.next_packet() {
            v.push(p);
            assert!(v.len() < 10_000);
        }
        v
    }

    /// (status byte or opcode, data1, data2) of a MIDI 1.0 or System packet.
    fn m1(p: &ReportPacket) -> (u8, u8, u8, u8) {
        let w = p.words()[0];
        (
            (w >> 16) as u8 & 0xF0 | ((w >> 28) as u8),
            (w >> 16) as u8 & 0x0F,
            (w >> 8) as u8,
            w as u8,
        )
    }

    #[test]
    fn midi1_report_order_and_rules() {
        // 9.7.1, 9.7.2; REQ-CI-MMR-017, -020, -021, -023 to -030, -032, -034.
        let s = Synth { paired: true };
        let mut w = ReportWriter::new(
            all_bits(),
            DataControl::Full,
            DeviceId::FunctionBlock,
            0,
            0,
            1,
            0b1,
            Protocol::Midi1,
            &s,
        );
        let p = collect(&mut w);
        for q in &p {
            match q.words()[0] >> 28 {
                1 => assert!(SystemMessage::parse(q.words()).is_some()),
                2 => assert!(Midi1ChannelVoiceMessage::parse(q.words()).is_some()),
                t => panic!("type {t}"),
            }
        }
        let t: Vec<_> = p.iter().map(m1).collect();
        // Eight MTC Quarter Frames, pieces 0-7.
        for i in 0..8u8 {
            assert_eq!(t[i as usize], (0xF1, 1, (i << 4) | (i + 1), 0));
        }
        assert_eq!(t[8], (0xF1, 2, 0x34, 0x24), "Song Position 0xF2");
        assert_eq!(t[9], (0xE2, 0, 0x00, 0x40));
        // CC paired: 1, 33 (0 and 32 are Bank Select, not individual), then 7, 64, 122.
        let ccs: Vec<_> = t
            .iter()
            .filter(|x| x.0 == 0xB2)
            .map(|x| (x.2, x.3))
            .collect();
        assert_eq!(
            ccs,
            [
                (1, 1),
                (33, 33),
                (7, 7),
                (64, 64),
                (122, 122),
                (124, 0),
                (126, 4),
                (101, 0),
                (100, 0),
                (6, 2),
                (38, 0),
                (101, 127),
                (100, 127),
                (0, 1),
                (32, 2)
            ]
        );
        // REQ-CI-MMR-029: Bank Select MSB, then LSB, then Program Change, one after the other.
        let msb = t.iter().position(|x| *x == (0xB2, 0, 0, 1)).unwrap();
        assert_eq!(t[msb + 1], (0xB2, 0, 32, 2));
        assert_eq!(t[msb + 2], (0xC2, 0, 5, 0));
        let tail: Vec<_> = t.iter().filter(|x| x.0 != 0xB2).skip(10).collect();
        assert_eq!(*tail[0], (0xC2, 0, 5, 0));
        assert_eq!(*tail[1], (0x92, 0, 60, 0x7F));
        assert_eq!(*tail[2], (0x82, 0, 61, 0));
        assert_eq!(
            *tail[3],
            (0x82, 0, 60, 0),
            "matching Note Off for the Note On"
        );
        assert_eq!(tail.len(), 4);
    }

    #[test]
    fn midi2_report_formats() {
        // REQ-CI-MMR-031, -033: Program Change with Bank, Note On attributes. REQ-CI-MMR-014:
        // the Synth gives every type, and only the two declared types are sent. REQ-CI-MMR-004:
        // with 0x01 the inactive note 61 is not sent.
        let s = Synth::default();
        let bits = MessageReportReply {
            channel: ChannelBits {
                program_change: true,
                ..Default::default()
            },
            note: NoteBits {
                notes: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut w = ReportWriter::new(
            bits,
            DataControl::NonDefault,
            DeviceId::Channel(2),
            3,
            0,
            4,
            0xFFFF,
            Protocol::Midi2,
            &s,
        );
        let p = collect(&mut w);
        for q in &p {
            assert!(Midi2ChannelVoiceMessage::parse(q.words()).is_some());
            assert_eq!(q.group, 3);
            assert_eq!((q.words()[0] >> 16) & 0xF, 2);
        }
        let words: Vec<&[u32]> = p.iter().map(|q| q.words()).collect();
        assert_eq!(
            words,
            [
                &[0x43C2_0001, 0x0500_0102][..],
                &[0x4392_3C03, 0xFE00_1234],
                &[0x4382_3C00, 0x0000_0000],
            ]
        );
    }

    /// Values that the application gives for Message Data Control 0x01 (REQ-CI-MMR-004): one
    /// non-default controller and per-note controller, one active and one inactive note.
    struct Changed;

    impl ReportSource for Changed {
        fn control_change(&self, _g: u8, _c: u8, index: u8) -> Option<u32> {
            (index == 7).then_some(0x1234_5678)
        }
        fn note(&self, _g: u8, _c: u8, p: usize) -> Option<NoteState> {
            [
                NoteState {
                    note: 60,
                    on: true,
                    velocity: 0x8000,
                    attribute_type: 0,
                    attribute_data: 0,
                },
                NoteState {
                    note: 61,
                    on: false,
                    velocity: 0,
                    attribute_type: 0,
                    attribute_data: 0,
                },
            ]
            .get(p)
            .copied()
        }
        fn registered_per_note(&self, _g: u8, _c: u8, p: usize) -> Option<Controller> {
            (p == 0).then_some(Controller {
                bank: 60,
                index: 7,
                value: 0xAAAA_0000,
            })
        }
    }

    #[test]
    fn non_default_report_has_active_notes_only() {
        // Table 44; REQ-CI-MMR-004: with 0x01 the writer sends the values that the application
        // gives and leaves out the inactive notes; with 0x7F it sends the inactive notes too
        // (9.5.1).
        let bits = MessageReportReply {
            channel: ChannelBits {
                control_change: true,
                ..Default::default()
            },
            note: NoteBits {
                notes: true,
                registered_per_note: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let report = |dc| {
            let mut w = ReportWriter::new(
                bits,
                dc,
                DeviceId::Channel(0),
                0,
                0,
                1,
                1,
                Protocol::Midi2,
                &Changed,
            );
            collect(&mut w)
                .iter()
                .map(|q| [q.words()[0], q.words()[1]])
                .collect::<Vec<_>>()
        };
        let cc = [0x40B0_0700, 0x1234_5678];
        let on = [0x4090_3C00, 0x8000_0000];
        let per_note = [0x4000_3C07, 0xAAAA_0000];
        let off = [0x4080_3C00, 0];
        assert_eq!(report(DataControl::NonDefault), [cc, on, per_note, off]);
        let inactive = [0x4080_3D00, 0];
        assert_eq!(report(DataControl::Full), [cc, on, inactive, per_note, off]);
    }

    #[test]
    fn channel_and_group_order() {
        // REQ-CI-MMR-009, -018, -019: Channels ascending in each Group, Groups ascending.
        let s = Synth::default();
        let bits = MessageReportReply {
            channel: ChannelBits {
                pitch_bend: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut w = ReportWriter::new(
            bits,
            DataControl::Full,
            DeviceId::FunctionBlock,
            5,
            4,
            2,
            0b1000_0000_0000_0101,
            Protocol::Midi1,
            &s,
        );
        let order: Vec<_> = collect(&mut w)
            .iter()
            .map(|p| (p.group, (p.words()[0] >> 16) as u8 & 0xF))
            .collect();
        assert_eq!(order, [(4, 0), (4, 2), (4, 15), (5, 0), (5, 2), (5, 15)]);
        let mut w = ReportWriter::new(
            bits,
            DataControl::Full,
            DeviceId::Group,
            5,
            4,
            2,
            0b101,
            Protocol::Midi1,
            &s,
        );
        assert_eq!(collect(&mut w).len(), 2);
    }

    struct Plain;

    impl ReportSource for Plain {
        fn control_change(&self, _g: u8, _c: u8, index: u8) -> Option<u32> {
            matches!(index, 0 | 6 | 31 | 32 | 33).then_some(1)
        }
        fn individual_controller(&self, _g: u8, _c: u8, _index: u8) -> bool {
            true
        }
        fn assignable_controller(&self, _g: u8, _c: u8, p: usize) -> Option<Controller> {
            (p < 2).then_some(Controller {
                bank: 1,
                index: p as u8,
                value: 0x3FFF,
            })
        }
        fn program(&self, _g: u8, _c: u8) -> Option<(u8, Option<(u8, u8)>)> {
            Some((9, None))
        }
    }

    #[test]
    fn midi1_unpaired_nrpn_and_program_only() {
        // REQ-CI-MMR-022 (CC 0-63 in index order), REQ-CI-MMR-026 (individual controllers),
        // REQ-CI-MMR-028 (NRPN Null), REQ-CI-MMR-030 (Program Change only), REQ-CI-MMR-035.
        let bits = MessageReportReply {
            channel: ChannelBits::from_byte(0x3F),
            ..Default::default()
        };
        let mut w = ReportWriter::new(
            bits,
            DataControl::Full,
            DeviceId::Channel(0),
            0,
            0,
            1,
            1,
            Protocol::Midi1,
            &Plain,
        );
        let t: Vec<_> = collect(&mut w).iter().map(m1).collect();
        let cc: Vec<_> = t
            .iter()
            .filter(|x| x.0 == 0xB2)
            .map(|x| (x.2, x.3))
            .collect();
        assert_eq!(
            cc,
            [
                (0, 1),
                (6, 1),
                (31, 1),
                (32, 1),
                (33, 1),
                (99, 1),
                (98, 0),
                (6, 0x7F),
                (38, 0x7F),
                (99, 1),
                (98, 1),
                (6, 0x7F),
                (38, 0x7F),
                (99, 127),
                (98, 127)
            ]
        );
        assert_eq!(*t.last().unwrap(), (0xC2, 0, 9, 0));
        assert!(w.next_packet().is_none());
    }

    struct Groups;

    impl ReportSource for Groups {
        fn registered_controller(&self, g: u8, _c: u8, p: usize) -> Option<Controller> {
            (g == 0 && p == 0).then_some(Controller {
                bank: 0,
                index: 0,
                value: 1,
            })
        }
        fn control_change(&self, _g: u8, _c: u8, index: u8) -> Option<u32> {
            (index == 0).then_some(5)
        }
        fn note(&self, _g: u8, _c: u8, p: usize) -> Option<NoteState> {
            (p == 0).then_some(NoteState {
                note: 60,
                on: true,
                velocity: 0x0100,
                attribute_type: 0,
                attribute_data: 0,
            })
        }
    }

    #[test]
    fn review_fixes_group_velocity_and_midi2_cc() {
        // Code review findings 2, 4, 5.
        let bits = MessageReportReply {
            channel: ChannelBits {
                registered: true,
                control_change: true,
                ..Default::default()
            },
            note: NoteBits {
                notes: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut w = ReportWriter::new(
            bits,
            DataControl::Full,
            DeviceId::FunctionBlock,
            0,
            0,
            2,
            1,
            Protocol::Midi1,
            &Groups,
        );
        let p = collect(&mut w);
        let rpn_groups: Vec<u8> = p
            .iter()
            .filter(|q| (q.words()[0] >> 8) & 0xFF == 101)
            .map(|q| q.group)
            .collect();
        assert_eq!(
            rpn_groups,
            [0, 0],
            "RPN and its Null only in group 0, not in group 1"
        );
        let on = p
            .iter()
            .find(|q| (q.words()[0] >> 20) & 0xF == 0x9)
            .unwrap();
        assert_eq!(on.words()[0] & 0x7F, 1, "a Note On never has velocity 0");
        let mut w = ReportWriter::new(
            bits,
            DataControl::Full,
            DeviceId::Channel(0),
            0,
            0,
            1,
            1,
            Protocol::Midi2,
            &Groups,
        );
        let p = collect(&mut w);
        assert!(
            !p.iter().any(|q| (q.words()[0] >> 20) & 0xF == 0xB),
            "MIDI 2.0: CC 0 is not sent as a CC"
        );
    }

    #[test]
    fn begin_end_only_reports_nothing() {
        // REQ-CI-MMR-003.
        let s = Synth::default();
        let mut w = ReportWriter::new(
            all_bits(),
            DataControl::BeginEndOnly,
            DeviceId::FunctionBlock,
            0,
            0,
            1,
            1,
            Protocol::Midi1,
            &s,
        );
        assert!(w.next_packet().is_none());
    }

    #[test]
    fn prop_report_writer_ends() {
        proptest!(|(bits in any::<[u8; 3]>(), mask in any::<u16>(), fg in 0u8..16, n in 0u8..17, midi1 in any::<bool>(), paired in any::<bool>())| {
            let s = Synth { paired };
            let r = MessageReportReply { system: SystemBits::from_byte(bits[0]), channel: ChannelBits::from_byte(bits[1]), note: NoteBits::from_byte(bits[2]) };
            let proto = if midi1 { Protocol::Midi1 } else { Protocol::Midi2 };
            let mut w = ReportWriter::new(r, DataControl::Full, DeviceId::FunctionBlock, 0, fg, n, mask, proto, &s);
            let _ = collect(&mut w);
        });
    }
}
