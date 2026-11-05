//! MIDI Polyphonic Expression (M1-100-UM MPE v1.1, 14 April 2022; DEC-022): the Zone layout, the
//! receiver of one MIDI 1.0 input, the Appendix E class function, the pitch helper of Appendix C,
//! the writers of a sender and the note allocator of a sender.
//!
//! - [`MpeReceiver::receive`] takes a typed MIDI 1.0 Channel Voice message (DIN bytes, USB-MIDI
//!   1.0, BLE-MIDI or a Message Type 0x2 UMP) and the [`ParameterEvent`] that the
//!   [`ParameterState`](crate::ParameterState) of the same channel gave for it, and gives one
//!   [`MpeEvent`]. The MPE Configuration Message (RPN 00H 06H, the MCM) and RPN 0 come from that
//!   event, so MPE has no RPN parser of its own. The receiver acts at the Data Entry MSB of an MCM
//!   (2.2.1: the LSB has no function; AMB-M1-100-19).
//! - The receiver keeps the last Pitch Bend, Channel Pressure and Control Change 74 of each of
//!   the 16 channels, also while no note plays (2.2.6 to 2.2.8). The sound engine combines the
//!   Manager and Member values (device); [`MpeReceiver::pitch_offset`] gives the sum of Appendix
//!   C.
//! - Pitch offsets are integers in hundredths of a cent ([`PITCH_UNITS_PER_SEMITONE`]): no float,
//!   no `libm`.
//! - All Sound Off (120) on the Manager Channel is Prohibited in Appendix E but the receiver gives
//!   it with its role, and the application decides (AMB-M1-100-12, user decision of 2026-11-05).
//! - The alternate translation of MPE to MIDI 2.0 per-note messages (M2-104 C.3, D.4) is not
//!   here; the default translation needs no MPE code.
//!
//! No alloc (DEC-002), no I/O (DEC-003). Never panics.

use crate::parameters::{
    change_message, select_messages, Parameter, ParameterChange, ParameterEvent,
};
use zmidi_ump::controller::Rpn;
use zmidi_ump::{
    Channel, Group, Midi1ChannelPressure, Midi1ChannelVoiceMessage, Midi1ControlChange,
    Midi1NoteOn, Midi1PitchBend, UmpError,
};

/// The center of a 14-bit Pitch Bend value.
const PITCH_BEND_CENTER: u16 = 0x2000;
/// The value of Control Change 74 after a reset (2.2.3 "reasonable default values"; the start
/// value of the Relative scheme of A.4.3).
const BRIGHTNESS_DEFAULT: u8 = 64;
/// The largest Member Channel count of one Zone (2.2.1).
const MAX_MEMBERS: u8 = 15;
/// The largest Member Channel count of two active Zones: Channels 1 and 16 are the Managers.
const MAX_MEMBERS_OF_TWO: u8 = 14;
/// The largest Pitch Bend Sensitivity that the writers write (2.1, 2.2.5; AMB-M1-100-07).
const MAX_SENSITIVITY: u8 = 96;
/// The mark of a held Note Number in the note allocator: 0 is "none".
const HELD: u8 = 0x80;

/// Controller numbers (M1 Table III; M1-100 2.2.8, Appendix E).
const BANK_SELECT_MSB: u8 = 0;
const BANK_SELECT_LSB: u8 = 32;
const BRIGHTNESS: u8 = 74;
const RESET_ALL_CONTROLLERS: u8 = 121;
const MONO_MODE_ON: u8 = 126;
const POLY_MODE_ON: u8 = 127;

/// The RPN of the MCM (2.2.1) and of the Pitch Bend Sensitivity (2.2.5, C.2).
const MCM: Rpn = Rpn::Other { msb: 0, lsb: 6 };

/// One of the two Zones (2.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MpeZone {
    /// Manager Channel 1, Member Channels from Channel 2 upwards.
    Lower,
    /// Manager Channel 16, Member Channels from Channel 15 downwards.
    Upper,
}

impl MpeZone {
    /// The Manager Channel: Channel 1 or Channel 16.
    const fn manager(self) -> Channel {
        match self {
            Self::Lower => Channel::from_bits(0),
            Self::Upper => Channel::from_bits(15),
        }
    }

    /// The index of the Zone in the tables of the receiver.
    const fn index(self) -> usize {
        self as usize
    }
}

/// The role of a channel in a layout (1.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MpeRole {
    /// The channel is in no Zone: a conventional MIDI 1.0 channel (2.2.1).
    NoZone,
    /// The Manager Channel of the Zone.
    Manager(MpeZone),
    /// A Member Channel of the Zone.
    Member(MpeZone),
}

/// The Zone layout: the Member Channel count of each Zone. 2 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MpeLayout {
    /// Member Channels of the Lower Zone, 0 to 15.
    lower: u8,
    /// Member Channels of the Upper Zone, 0 to 15.
    upper: u8,
}

impl MpeLayout {
    /// No Zone: MPE Mode is off.
    pub const NONE: Self = Self { lower: 0, upper: 0 };

    /// The Lower Zone with 15 Member Channels, Channels 2 to 16 (2.2.2: the power-up default of
    /// a device in MPE Mode).
    pub const DEFAULT: Self = Self {
        lower: MAX_MEMBERS,
        upper: 0,
    };

    /// A layout with `lower` and `upper` Member Channels (0 means the Zone is inactive). Gives
    /// `OutOfRange` for a count above 15, and for two active Zones with more than 14 Member
    /// Channels in all, so that no channel is in two Zones (2.2.1).
    pub const fn new(lower: u8, upper: u8) -> Result<Self, UmpError> {
        if lower > MAX_MEMBERS
            || upper > MAX_MEMBERS
            || (lower > 0 && upper > 0 && lower + upper > MAX_MEMBERS_OF_TWO)
        {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self { lower, upper })
    }

    /// The Member Channel count of `zone`; 0 when the Zone is inactive.
    pub const fn members(self, zone: MpeZone) -> u8 {
        match zone {
            MpeZone::Lower => self.lower,
            MpeZone::Upper => self.upper,
        }
    }

    /// The layout after one MCM with `members` for `zone` (2.2.1, Appendix B). 0 deactivates only
    /// that Zone (AMB-M1-100-02). With two active Zones the Zone of the MCM keeps its value and
    /// the other Zone keeps the smaller of its value and 14 minus the new value; at 0 or less it
    /// is inactive (AMB-M1-100-03). A value above 15 gives the same layout (AMB-M1-100-01).
    pub const fn with_mcm(self, zone: MpeZone, members: u8) -> Self {
        if members > MAX_MEMBERS {
            return self;
        }
        let other = match zone {
            MpeZone::Lower => self.upper,
            MpeZone::Upper => self.lower,
        };
        let other = if members == 0 || other == 0 {
            other
        } else if members >= MAX_MEMBERS_OF_TWO {
            0
        } else if other > MAX_MEMBERS_OF_TWO - members {
            MAX_MEMBERS_OF_TWO - members
        } else {
            other
        };
        match zone {
            MpeZone::Lower => Self {
                lower: members,
                upper: other,
            },
            MpeZone::Upper => Self {
                lower: other,
                upper: members,
            },
        }
    }

    /// The role of `channel` (1.5.1, 2.2.1). The Manager Channel of an inactive Zone can be a
    /// Member Channel of the other Zone (Examples B.1, B.4).
    pub const fn role(self, channel: Channel) -> MpeRole {
        let i = channel.index();
        if self.lower > 0 {
            if i == 0 {
                return MpeRole::Manager(MpeZone::Lower);
            }
            if i <= self.lower {
                return MpeRole::Member(MpeZone::Lower);
            }
        }
        if self.upper > 0 {
            if i == 15 {
                return MpeRole::Manager(MpeZone::Upper);
            }
            if i >= 15 - self.upper {
                return MpeRole::Member(MpeZone::Upper);
            }
        }
        MpeRole::NoZone
    }

    /// MPE Mode is on: at least one Zone is active (2.2.1).
    pub const fn is_active(self) -> bool {
        self.lower > 0 || self.upper > 0
    }

    /// The lowest Member Channel of `zone`, which takes the Mode messages (2.2.4.3), or `None` when
    /// the Zone is inactive.
    pub const fn lowest_member(self, zone: MpeZone) -> Option<Channel> {
        match zone {
            MpeZone::Lower if self.lower > 0 => Some(Channel::from_bits(1)),
            MpeZone::Upper if self.upper > 0 => Some(Channel::from_bits(15 - self.upper)),
            _ => None,
        }
    }
}

/// A Pitch Bend Sensitivity: semitones (Data Entry MSB of RPN 0) and cents (Data Entry LSB) (M1
/// p. 18; 2.2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MpeSensitivity {
    /// Semitones, 7 bits.
    pub semitones: u8,
    /// Cents, 7 bits.
    pub cents: u8,
}

impl MpeSensitivity {
    /// The Manager Channel default: 2 semitones (2.1, 2.2.5).
    pub const MANAGER_DEFAULT: Self = Self {
        semitones: 2,
        cents: 0,
    };

    /// The Member Channel default: 48 semitones (2.1, 2.2.5).
    pub const MEMBER_DEFAULT: Self = Self {
        semitones: 48,
        cents: 0,
    };

    /// The sensitivity in pitch units: at most 2 575 500 (127 semitones and 127 cents), below
    /// 2^22.
    const fn units(self) -> u32 {
        ((self.semitones & 0x7F) as u32 * 100 + (self.cents & 0x7F) as u32) * 100
    }
}

/// Pitch units for one semitone: a pitch offset is in hundredths of a cent.
pub const PITCH_UNITS_PER_SEMITONE: i32 = 10_000;

/// round(n × 8192 / d), half up, for n < d < 2^22, in two 32-bit steps of 6 and 7 bits, so that
/// the firmware links no 64-bit division (about 900 bytes on Cortex-M).
const fn scale_8192(n: u32, d: u32) -> u32 {
    let a = n << 6;
    let b = (a % d) << 7;
    ((a / d) << 7) + b / d + (2 * (b % d) >= d) as u32
}

/// The 14-bit Pitch Bend value of a sender for a pitch offset (hundredths of a cent) at
/// `sensitivity` (Appendix C.3): round(offset × 8192 / sensitivity) + 8192, rounded half away
/// from zero and limited to 0 to 16383 (AMB-M1-100-09). A sensitivity of 0 gives the center
/// 8192. Example C.4: +7 semitones at 48 gives 9387, +2 semitones at 2 gives 16383.
pub const fn pitch_bend_value(offset: i32, sensitivity: MpeSensitivity) -> u16 {
    let range = sensitivity.units();
    if range == 0 {
        return PITCH_BEND_CENTER;
    }
    let n = offset.unsigned_abs();
    // At or beyond the full swing the value is at a limit.
    let k = if n >= range {
        0x2000
    } else {
        scale_8192(n, range)
    };
    if offset >= 0 {
        let value = PITCH_BEND_CENTER as u32 + k;
        if value > 0x3FFF {
            0x3FFF
        } else {
            value as u16
        }
    } else {
        (PITCH_BEND_CENTER as u32 - k) as u16
    }
}

/// The pitch offset (hundredths of a cent) of a received 14-bit Pitch Bend value at
/// `sensitivity` (Appendix C.5): sensitivity × (value − 8192) / 8191, rounded half away from
/// zero, in 32-bit arithmetic (sensitivity = 8191 q + r, so the product is q × v + r × v / 8191).
/// Example C.6: 16383 at 2 gives 2 semitones, 9387 at 48 gives 7.003 semitones.
pub const fn pitch_offset(value: u16, sensitivity: MpeSensitivity) -> i32 {
    let v = (value & 0x3FFF) as i32 - PITCH_BEND_CENTER as i32;
    let m = v.unsigned_abs();
    let d = sensitivity.units();
    let x = (d % 8191) * m;
    let k = (d / 8191) * m + x / 8191 + (2 * (x % 8191) >= 8191) as u32;
    if v < 0 {
        -(k as i32)
    } else {
        k as i32
    }
}

/// What [`MpeReceiver::receive`] gives for one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MpeEvent {
    /// The channel is in no Zone: a conventional MIDI 1.0 message.
    NoZone,
    /// A message on a channel of `zone`: for the whole Zone on the Manager Channel, for the notes
    /// of that channel on a Member Channel.
    Zone {
        /// The Zone of the channel.
        zone: MpeZone,
        /// The channel is the Manager Channel.
        manager: bool,
    },
    /// An MCM changed the layout. The application stops the Sounding Notes and resets the
    /// controls of each channel in `reset` (bit n is the channel with packet value n; 2.2.3,
    /// AMB-M1-100-05).
    Layout {
        /// The channels to reset.
        reset: u16,
    },
    /// RPN 0 changed the Manager or Member Pitch Bend Sensitivity of `zone` (2.2.5).
    Sensitivity {
        /// The Zone.
        zone: MpeZone,
        /// The Manager value changed; otherwise the Member value of the Zone.
        manager: bool,
    },
    /// Mono Mode On or Poly Mode On on the lowest Member Channel changed the MIDI Mode of `zone`
    /// (2.2.4.3).
    Mode {
        /// The Zone.
        zone: MpeZone,
    },
    /// A message that MPE prohibits or that has no function here: an MCM on Channels 2 to 15 or
    /// above 15, the LSB or a step of an MCM, Polyphonic Key Pressure on a Member Channel, a
    /// Program Change or Bank Select on a Member Channel in Mode 3 (2.2.1, 2.2.7, 2.3.3,
    /// Appendix E).
    Ignored,
}

/// The MPE receiver of one MIDI 1.0 input: the layout, the MIDI Mode and the two Pitch Bend
/// Sensitivities of each Zone, and the last Pitch Bend, Channel Pressure and Control Change 74 of
/// each of the 16 channels. 76 bytes, all zeros when new (Pitch Bend is stored XOR 8192 and
/// Control Change 74 XOR 64), no alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MpeReceiver {
    layout: MpeLayout,
    /// Bit 0: the Lower Zone is in Mode 4; bit 1: the Upper Zone.
    mono: u8,
    /// Semitones and cents for each Zone: [Manager, Member].
    sensitivity: [[[u8; 2]; 2]; 2],
    /// Pitch Bend XOR 8192.
    pitch_bend: [u16; 16],
    pressure: [u8; 16],
    /// Control Change 74 XOR 64.
    brightness: [u8; 16],
}

impl MpeReceiver {
    /// A receiver with no Zone (MPE Mode off), Mode 3, and the controls of each channel at 8192,
    /// 0 and 64. All zeros, so a `static` receiver goes in `.bss`.
    pub const fn new() -> Self {
        Self {
            layout: MpeLayout::NONE,
            mono: 0,
            sensitivity: [[[0; 2]; 2]; 2],
            pitch_bend: [0; 16],
            pressure: [0; 16],
            brightness: [0; 16],
        }
    }

    /// Takes one message and the parameter event that the `ParameterState` of its channel gave
    /// for it (`None` for other messages), and gives one event (REQ-MPE-002 to REQ-MPE-006,
    /// REQ-MPE-013, REQ-MPE-015). The controls of each channel are kept in every role. Never
    /// panics.
    pub fn receive(
        &mut self,
        message: &Midi1ChannelVoiceMessage,
        parameter: Option<ParameterEvent>,
    ) -> MpeEvent {
        let channel = message.channel();
        let i = channel.index() as usize;
        match message {
            Midi1ChannelVoiceMessage::PitchBend(m) => {
                self.pitch_bend[i] = m.value() ^ PITCH_BEND_CENTER
            }
            Midi1ChannelVoiceMessage::ChannelPressure(m) => self.pressure[i] = m.pressure(),
            Midi1ChannelVoiceMessage::ControlChange(m) => match m.index() {
                BRIGHTNESS => self.brightness[i] = m.data() ^ BRIGHTNESS_DEFAULT,
                // RP-015: Pitch Bend to the center, Channel Pressure to 0; 70 to 79 stay.
                RESET_ALL_CONTROLLERS => {
                    self.pitch_bend[i] = 0;
                    self.pressure[i] = 0;
                }
                _ => {}
            },
            _ => {}
        }
        let role = self.layout.role(channel);
        if let (Midi1ChannelVoiceMessage::ControlChange(_), Some(event)) = (message, parameter) {
            if let Parameter::Registered(rpn) = event.parameter {
                match (rpn, role) {
                    (MCM, _) => return self.mcm(i, event.change),
                    (Rpn::PitchBendSensitivity, MpeRole::Manager(zone)) => {
                        return self.rpn_0(zone, true, event)
                    }
                    (Rpn::PitchBendSensitivity, MpeRole::Member(zone)) => {
                        return self.rpn_0(zone, false, event)
                    }
                    _ => {}
                }
            }
        }
        let (zone, manager) = match role {
            MpeRole::NoZone => return MpeEvent::NoZone,
            MpeRole::Manager(zone) => (zone, true),
            MpeRole::Member(zone) => (zone, false),
        };
        let mode_3_member = !manager && self.mode(zone) == 3;
        match message {
            Midi1ChannelVoiceMessage::PolyPressure(_) if !manager => MpeEvent::Ignored,
            Midi1ChannelVoiceMessage::ProgramChange(_) if mode_3_member => MpeEvent::Ignored,
            Midi1ChannelVoiceMessage::ControlChange(m) => match m.index() {
                BANK_SELECT_MSB | BANK_SELECT_LSB if mode_3_member => MpeEvent::Ignored,
                MONO_MODE_ON | POLY_MODE_ON if matches!(self.layout.lowest_member(zone), Some(c) if c.index() == i as u8) =>
                {
                    let bit = 1 << zone.index();
                    if m.index() == MONO_MODE_ON {
                        self.mono |= bit;
                    } else {
                        self.mono &= !bit;
                    }
                    MpeEvent::Mode { zone }
                }
                _ => MpeEvent::Zone { zone, manager },
            },
            _ => MpeEvent::Zone { zone, manager },
        }
    }

    /// An MCM change on the channel with packet value `i` (2.2.1).
    fn mcm(&mut self, i: usize, change: ParameterChange) -> MpeEvent {
        let zone = match i {
            0 => MpeZone::Lower,
            15 => MpeZone::Upper,
            _ => return MpeEvent::Ignored,
        };
        let ParameterChange::DataEntryMsb(members) = change else {
            return MpeEvent::Ignored;
        };
        if members > MAX_MEMBERS {
            return MpeEvent::Ignored;
        }
        let after = self.layout.with_mcm(zone, members);
        let reset = self.change_layout(after, Some(zone));
        if members > 0 {
            self.activate(zone);
        }
        MpeEvent::Layout { reset }
    }

    /// RPN 0 on a channel of `zone`: Data Entry MSB, LSB, Increment or Decrement (2.2.5,
    /// RP-018).
    fn rpn_0(&mut self, zone: MpeZone, manager: bool, event: ParameterEvent) -> MpeEvent {
        let slot = &mut self.sensitivity[zone.index()][!manager as usize];
        let value = event.apply((slot[0] as u16) << 7 | slot[1] as u16);
        *slot = [(value >> 7) as u8, (value & 0x7F) as u8];
        MpeEvent::Sensitivity { zone, manager }
    }

    /// Sets the layout to `after` and resets the controls of the channels whose role changes or
    /// that are in `zone` before or after (each channel in a Zone for `None`); gives the mask.
    fn change_layout(&mut self, after: MpeLayout, zone: Option<MpeZone>) -> u16 {
        let before = self.layout;
        let mut reset = 0u16;
        let mut i = 0;
        while i < 16 {
            let channel = Channel::from_bits(i);
            let (b, a) = (before.role(channel), after.role(channel));
            let hit = match zone {
                Some(zone) => b != a || in_zone(b, zone) || in_zone(a, zone),
                None => !matches!((b, a), (MpeRole::NoZone, MpeRole::NoZone)),
            };
            if hit {
                reset |= 1 << i;
                self.pitch_bend[i as usize] = 0;
                self.pressure[i as usize] = 0;
                self.brightness[i as usize] = 0;
            }
            i += 1;
        }
        self.layout = after;
        reset
    }

    /// The values of a Zone that an MCM or the application activates: Pitch Bend Sensitivity 2 and
    /// 48 semitones, Mode 3 (2.2.4, 2.2.5; AMB-M1-100-06).
    fn activate(&mut self, zone: MpeZone) {
        let (m, n) = (
            MpeSensitivity::MANAGER_DEFAULT,
            MpeSensitivity::MEMBER_DEFAULT,
        );
        self.sensitivity[zone.index()] = [[m.semitones, m.cents], [n.semitones, n.cents]];
        self.mono &= !(1 << zone.index());
    }

    /// Sets the layout of the application: a power-up default or a front panel setting
    /// (REQ-MPE-012). Each active Zone gets Pitch Bend Sensitivity 2 and 48 semitones and Mode 3;
    /// gives the mask of every channel that is in a Zone before or after (2.2.3). The application
    /// stops the Sounding Notes on those channels.
    pub fn set_layout(&mut self, layout: MpeLayout) -> u16 {
        let reset = self.change_layout(layout, None);
        if layout.lower > 0 {
            self.activate(MpeZone::Lower);
        }
        if layout.upper > 0 {
            self.activate(MpeZone::Upper);
        }
        reset
    }

    /// The layout.
    pub const fn layout(&self) -> MpeLayout {
        self.layout
    }

    /// The MIDI Mode of `zone`: 3 (Poly, the default) or 4 (Mono) (2.2.4).
    pub const fn mode(&self, zone: MpeZone) -> u8 {
        if self.mono & (1 << zone.index()) != 0 {
            4
        } else {
            3
        }
    }

    /// The Manager Channel or Member Channel Pitch Bend Sensitivity of `zone` (2.2.5). A value
    /// above 96 stays as received (AMB-M1-100-07).
    pub const fn sensitivity(&self, zone: MpeZone, manager: bool) -> MpeSensitivity {
        let [semitones, cents] = self.sensitivity[zone.index()][!manager as usize];
        MpeSensitivity { semitones, cents }
    }

    /// The last Pitch Bend of `channel`, 0 to 16383 (8192 at the start and after a reset).
    pub const fn pitch_bend(&self, channel: Channel) -> u16 {
        self.pitch_bend[channel.index() as usize] ^ PITCH_BEND_CENTER
    }

    /// The last Channel Pressure of `channel` (0 at the start and after a reset).
    pub const fn pressure(&self, channel: Channel) -> u8 {
        self.pressure[channel.index() as usize]
    }

    /// The last Control Change 74 of `channel` (64 at the start and after a reset).
    pub const fn brightness(&self, channel: Channel) -> u8 {
        self.brightness[channel.index() as usize] ^ BRIGHTNESS_DEFAULT
    }

    /// The pitch offset (hundredths of a cent) of `channel` (Appendix C.5): on a Member Channel
    /// the Manager Channel offset plus the Member Channel offset, on the Manager Channel its own
    /// offset, and `None` in no Zone.
    pub const fn pitch_offset(&self, channel: Channel) -> Option<i32> {
        let (zone, member) = match self.layout.role(channel) {
            MpeRole::NoZone => return None,
            MpeRole::Manager(zone) => (zone, None),
            MpeRole::Member(zone) => (zone, Some(channel)),
        };
        let manager = pitch_offset(
            self.pitch_bend(zone.manager()),
            self.sensitivity(zone, true),
        );
        match member {
            None => Some(manager),
            Some(c) => {
                Some(manager + pitch_offset(self.pitch_bend(c), self.sensitivity(zone, false)))
            }
        }
    }
}

impl Default for MpeReceiver {
    fn default() -> Self {
        Self::new()
    }
}

/// The role is in `zone`.
const fn in_zone(role: MpeRole, zone: MpeZone) -> bool {
    match role {
        MpeRole::Manager(z) | MpeRole::Member(z) => z as u8 == zone as u8,
        MpeRole::NoZone => false,
    }
}

/// A message kind of Appendix E (Table 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MpeMessage {
    /// RPN 00H 06H, the MPE Configuration Message.
    Mcm,
    /// RPN 0, Pitch Bend Sensitivity.
    PitchBendSensitivity,
    /// Pitch Bend.
    PitchBend,
    /// Channel Pressure.
    ChannelPressure,
    /// Control Change 74 (Brightness).
    Brightness,
    /// Polyphonic Key Pressure.
    PolyPressure,
    /// Control Change 120.
    AllSoundOff,
    /// Control Change 121.
    ResetAllControllers,
    /// Control Change 122.
    LocalControl,
    /// Control Change 123.
    AllNotesOff,
    /// Control Change 124.
    OmniOff,
    /// Control Change 125.
    OmniOn,
    /// Control Change 126.
    MonoModeOn,
    /// Control Change 127.
    PolyModeOn,
    /// Each other Control Change, for example Modulation, Volume or Damper Pedal.
    OtherControlChange,
    /// Each other RPN.
    OtherRpn,
    /// Each NRPN.
    Nrpn,
    /// Program Change.
    ProgramChange,
    /// Bank Select, Control Change 0 and 32.
    BankSelect,
    /// Note On and Note Off.
    Note,
    /// System Common, System Real Time and System Exclusive.
    System,
}

/// The class of a message kind in Appendix E.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MpeClass {
    /// M: Mandatory.
    Mandatory,
    /// O: Optional.
    Optional,
    /// P: Prohibited.
    Prohibited,
}

/// The class of `message` in Appendix E (Table 5): on the Manager Channel (`manager`) or a Member
/// Channel, to send (`transmit`) or to receive, with the MIDI Mode of the Zone for Program Change
/// and Bank Select on a Member Channel (`mono` for Mode 4). All Sound Off on the Manager Channel is
/// Prohibited, as the table gives (AMB-M1-100-12).
pub const fn mpe_class(message: MpeMessage, manager: bool, transmit: bool, mono: bool) -> MpeClass {
    use MpeClass::{Mandatory as M, Optional as O, Prohibited as P};
    // The columns of Table 5: Manager Tx, Manager Rx, Member Tx, Member Rx.
    let row = match message {
        MpeMessage::Mcm => [M, M, P, P],
        MpeMessage::PitchBendSensitivity
        | MpeMessage::PitchBend
        | MpeMessage::ChannelPressure
        | MpeMessage::Brightness => [O, M, O, M],
        MpeMessage::PolyPressure => [O, O, P, P],
        MpeMessage::AllSoundOff | MpeMessage::MonoModeOn | MpeMessage::PolyModeOn => [P, P, O, O],
        MpeMessage::OmniOn => [P, P, P, P],
        MpeMessage::ProgramChange | MpeMessage::BankSelect if !mono => [O, O, P, P],
        MpeMessage::Note => [O, M, M, M],
        _ => [O, O, O, O],
    };
    row[(!manager as usize) * 2 + !transmit as usize]
}

/// A Control Change with Group 0.
const fn control_change(
    channel: Channel,
    controller: u8,
    value: u8,
) -> Result<Midi1ChannelVoiceMessage, UmpError> {
    match Midi1ControlChange::try_new(Group::from_bits(0), channel, controller, value) {
        Ok(m) => Ok(Midi1ChannelVoiceMessage::ControlChange(m)),
        Err(e) => Err(e),
    }
}

/// RPN `rpn` with Data Entry MSB `value` and no LSB: controllers 101, 100 and 6.
const fn rpn_messages(
    channel: Channel,
    rpn: Rpn,
    value: u8,
) -> Result<[Midi1ChannelVoiceMessage; 3], UmpError> {
    match (
        select_messages(channel, Parameter::Registered(rpn)),
        change_message(channel, ParameterChange::DataEntryMsb(value)),
    ) {
        (Ok([msb, lsb]), Ok(data)) => Ok([msb, lsb, data]),
        _ => Err(UmpError::OutOfRange),
    }
}

/// Writes the MCM of `zone` with Group 0: controllers 101 = 0, 100 = 6 and 6 = `members` on
/// Channel 1 (Lower Zone) or Channel 16 (Upper Zone) (2.2.1, Appendix B). Gives `OutOfRange` for
/// `members` above 15.
pub const fn mcm_messages(
    zone: MpeZone,
    members: u8,
) -> Result<[Midi1ChannelVoiceMessage; 3], UmpError> {
    if members > MAX_MEMBERS {
        return Err(UmpError::OutOfRange);
    }
    rpn_messages(zone.manager(), MCM, members)
}

/// Writes the Pitch Bend Sensitivity of `channel` with Group 0 in whole semitones and with no
/// Data Entry LSB, as 2.2.5 recommends: controllers 101 = 0, 100 = 0 and 6 = `semitones` (C.2).
/// Gives `OutOfRange` for a value above 96.
pub const fn sensitivity_messages(
    channel: Channel,
    semitones: u8,
) -> Result<[Midi1ChannelVoiceMessage; 3], UmpError> {
    if semitones > MAX_SENSITIVITY {
        return Err(UmpError::OutOfRange);
    }
    rpn_messages(channel, Rpn::PitchBendSensitivity, semitones)
}

/// Writes RPN 0 with `manager` semitones on the Manager Channel and with `member` semitones on
/// each Member Channel of `zone`, from the lowest channel up, into `out`, and gives the count of
/// messages (3 for each channel; 2.2.5: "sending RPN 0 to every Member Channel individually").
/// Gives `OutOfRange` for an inactive Zone or a value above 96, and `BufferTooSmall` when `out` is
/// too small; then `out` is unchanged.
pub fn zone_sensitivity_messages(
    layout: MpeLayout,
    zone: MpeZone,
    manager: u8,
    member: u8,
    out: &mut [Midi1ChannelVoiceMessage],
) -> Result<usize, UmpError> {
    let (Some(lowest), Ok(first), true) = (
        layout.lowest_member(zone),
        sensitivity_messages(zone.manager(), manager),
        member <= MAX_SENSITIVITY,
    ) else {
        return Err(UmpError::OutOfRange);
    };
    let members = layout.members(zone) as usize;
    let needed = 3 * (1 + members);
    let Some(out) = out.get_mut(..needed) else {
        return Err(UmpError::BufferTooSmall {
            needed,
            got: out.len(),
        });
    };
    let (chunks, _) = out.as_chunks_mut::<3>();
    chunks[0] = first;
    for (k, chunk) in chunks[1..].iter_mut().enumerate() {
        *chunk = sensitivity_messages(Channel::from_bits(lowest.index() + k as u8), member)?;
    }
    Ok(needed)
}

/// Writes the Mode message of `zone` with Group 0 on its lowest Member Channel (2.2.4.3): Mono
/// Mode On (126) with M = the Member Channel count (AMB-M1-100-14), or Poly Mode On (127) with 0.
/// Gives `OutOfRange` for an inactive Zone.
pub const fn mode_message(
    layout: MpeLayout,
    zone: MpeZone,
    mono: bool,
) -> Result<Midi1ChannelVoiceMessage, UmpError> {
    let Some(channel) = layout.lowest_member(zone) else {
        return Err(UmpError::OutOfRange);
    };
    if mono {
        control_change(channel, MONO_MODE_ON, layout.members(zone))
    } else {
        control_change(channel, POLY_MODE_ON, 0)
    }
}

/// Writes the start of a note with Group 0 on `channel` (2.4, Table 4): the initial Pitch Bend,
/// Control Change 74 and Channel Pressure, then Note On. Gives `OutOfRange` for a value out of
/// range and for velocity 0, because a Note On with velocity 0 is a Note Off. Example (Table 4
/// with the corrected value of AMB-M1-100-08): a quarter tone up at 48 semitones is
/// `pitch_bend_value(5000, MEMBER_DEFAULT)` = 8277, bytes E2 55 40.
pub const fn note_messages(
    channel: Channel,
    note: u8,
    velocity: u8,
    pitch_bend: u16,
    brightness: u8,
    pressure: u8,
) -> Result<[Midi1ChannelVoiceMessage; 4], UmpError> {
    let group = Group::from_bits(0);
    if velocity == 0 {
        return Err(UmpError::OutOfRange);
    }
    match (
        Midi1PitchBend::try_new(group, channel, pitch_bend),
        control_change(channel, BRIGHTNESS, brightness),
        Midi1ChannelPressure::try_new(group, channel, pressure),
        Midi1NoteOn::try_new(group, channel, note, velocity),
    ) {
        (Ok(bend), Ok(cc74), Ok(press), Ok(on)) => Ok([
            Midi1ChannelVoiceMessage::PitchBend(bend),
            cc74,
            Midi1ChannelVoiceMessage::ChannelPressure(press),
            Midi1ChannelVoiceMessage::NoteOn(on),
        ]),
        _ => Err(UmpError::OutOfRange),
    }
}

/// The Member Channel allocator of an MPE sender (2.2.4.1, Appendix A.3): for each channel the
/// count of active notes, the last Note Number and the age of the last Note Off. 104 bytes, all
/// zeros when new, no alloc. The application gives the layout at each call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MpeNoteAllocator {
    active: [u8; 16],
    /// `HELD | note` of the last note on each channel; 0 for none.
    last_note: [u8; 16],
    /// The Note Off count at the last Note Off of each channel; 0 for none.
    last_off: [u32; 16],
    /// The count of Note Offs.
    clock: u32,
    same_note: bool,
}

impl MpeNoteAllocator {
    /// An allocator with no active note and the same-note option off.
    pub const fn new() -> Self {
        Self {
            active: [0; 16],
            last_note: [0; 16],
            last_off: [0; 16],
            clock: 0,
            same_note: false,
        }
    }

    /// Gives the Member Channel of `zone` for a new note and counts the note as active there
    /// (1.5.1, 2.2.4.1, A.3): with the same-note option, first an unoccupied channel that last
    /// played `note` (the one with the newest Note Off); else the channel with the fewest active
    /// notes, then the oldest last Note Off, then the lowest channel. When every channel is
    /// occupied, the new note shares one (2.2.4.1). Gives `None` for an inactive Zone.
    pub fn note_on(&mut self, layout: MpeLayout, zone: MpeZone, note: u8) -> Option<Channel> {
        let first = layout.lowest_member(zone)?.index() as usize;
        let channels = first..first + layout.members(zone) as usize;
        let held = HELD | (note & 0x7F);
        let mut best = None;
        if self.same_note {
            for c in channels.clone() {
                if self.active[c] == 0
                    && self.last_note[c] == held
                    && best.is_none_or(|b: usize| self.last_off[c] > self.last_off[b])
                {
                    best = Some(c);
                }
            }
        }
        let c = match best {
            Some(c) => c,
            None => channels.min_by_key(|&c| (self.active[c], self.last_off[c]))?,
        };
        self.active[c] = self.active[c].saturating_add(1);
        self.last_note[c] = held;
        Some(Channel::from_bits(c as u8))
    }

    /// One note on `channel` ended (Note Off, or Note On with velocity 0). A channel with no
    /// active note changes nothing.
    pub fn note_off(&mut self, channel: Channel) {
        let c = channel.index() as usize;
        if self.active[c] > 0 {
            self.active[c] -= 1;
            self.clock = self.clock.wrapping_add(1);
            self.last_off[c] = self.clock;
        }
    }

    /// The count of active notes on `channel`; above 0 the channel is occupied (1.5.1).
    pub const fn active_notes(&self, channel: Channel) -> u8 {
        self.active[channel.index() as usize]
    }

    /// The same-note option of A.3 (off by default).
    pub const fn same_note(&self) -> bool {
        self.same_note
    }

    /// Sets the same-note option.
    pub fn set_same_note(&mut self, on: bool) {
        self.same_note = on;
    }

    /// No active note and no age, for example after a layout change; the same-note option stays.
    pub fn reset(&mut self) {
        *self = Self {
            same_note: self.same_note,
            ..Self::new()
        };
    }
}

impl Default for MpeNoteAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::parameters::ParameterState;
    use proptest::prelude::*;
    use std::vec;
    use std::vec::Vec;

    const LOWER: MpeZone = MpeZone::Lower;
    const UPPER: MpeZone = MpeZone::Upper;

    fn ch(index: u8) -> Channel {
        Channel::from_bits(index)
    }

    fn layout(lower: u8, upper: u8) -> MpeLayout {
        MpeLayout::new(lower, upper).unwrap()
    }

    /// The typed message of MIDI 1.0 bytes.
    fn msg(bytes: &[u8]) -> Midi1ChannelVoiceMessage {
        let byte = |i: usize| u32::from(bytes.get(i).copied().unwrap_or(0));
        Midi1ChannelVoiceMessage::parse(&[0x2000_0000 | byte(0) << 16 | byte(1) << 8 | byte(2)])
            .unwrap()
    }

    /// The MIDI 1.0 bytes of typed messages.
    fn bytes(messages: &[Midi1ChannelVoiceMessage]) -> Vec<u8> {
        let mut out = Vec::new();
        for m in messages {
            let w = m.into_bits();
            let status = (w >> 16) as u8;
            out.extend([status, (w >> 8) as u8 & 0x7F]);
            if !matches!(status & 0xF0, 0xC0 | 0xD0) {
                out.push(w as u8 & 0x7F);
            }
        }
        out
    }

    /// One MIDI 1.0 input as an application keeps it: a parameter receiver for each channel and
    /// the MPE receiver.
    struct Input {
        parameters: [ParameterState; 16],
        mpe: MpeReceiver,
    }

    impl Input {
        fn new() -> Self {
            Self {
                parameters: [ParameterState::new(); 16],
                mpe: MpeReceiver::new(),
            }
        }

        fn one(&mut self, message: &Midi1ChannelVoiceMessage) -> MpeEvent {
            let parameter = self.parameters[message.channel().index() as usize].receive(message);
            self.mpe.receive(message, parameter)
        }

        /// Feeds 3-byte messages; gives the event of each.
        fn feed(&mut self, stream: &[[u8; 3]]) -> Vec<MpeEvent> {
            stream.iter().map(|b| self.one(&msg(b))).collect()
        }

        /// Feeds typed messages; gives the last event.
        fn send(&mut self, messages: &[Midi1ChannelVoiceMessage]) -> MpeEvent {
            messages.iter().map(|m| self.one(m)).last().unwrap()
        }
    }

    fn mcm(channel: u8, members: u8) -> [[u8; 3]; 3] {
        let s = 0xB0 | channel;
        [[s, 0x65, 0], [s, 0x64, 6], [s, 0x06, members]]
    }

    fn rpn_0(channel: u8, semitones: u8) -> [[u8; 3]; 3] {
        let s = 0xB0 | channel;
        [[s, 0x65, 0], [s, 0x64, 0], [s, 0x06, semitones]]
    }

    fn sens(semitones: u8, cents: u8) -> MpeSensitivity {
        MpeSensitivity { semitones, cents }
    }

    #[test]
    fn layout_roles_follow_2_2_1() {
        // 2.2.1: Lower Zone Manager 1, Members from 2 upwards; Upper Zone Manager 16, Members
        // from 15 downwards; Examples B.1, B.3, B.4.
        let l = MpeLayout::DEFAULT;
        assert_eq!(l.role(ch(0)), MpeRole::Manager(LOWER));
        assert!((1..16).all(|i| l.role(ch(i)) == MpeRole::Member(LOWER)));
        let two = layout(7, 7);
        assert_eq!(two.role(ch(0)), MpeRole::Manager(LOWER));
        assert!((1..=7).all(|i| two.role(ch(i)) == MpeRole::Member(LOWER)));
        assert!((8..=14).all(|i| two.role(ch(i)) == MpeRole::Member(UPPER)));
        assert_eq!(two.role(ch(15)), MpeRole::Manager(UPPER));
        let upper = layout(0, 15);
        assert!((0..=14).all(|i| upper.role(ch(i)) == MpeRole::Member(UPPER)));
        assert_eq!(upper.role(ch(15)), MpeRole::Manager(UPPER));
        let small = layout(3, 0);
        assert!((4..16).all(|i| small.role(ch(i)) == MpeRole::NoZone));
        assert!(!MpeLayout::NONE.is_active() && small.is_active() && upper.is_active());
        assert_eq!(MpeLayout::NONE.role(ch(0)), MpeRole::NoZone);
        assert_eq!(
            (two.lowest_member(LOWER), two.lowest_member(UPPER)),
            (Some(ch(1)), Some(ch(8)))
        );
        assert_eq!(small.lowest_member(UPPER), None);
    }

    #[test]
    fn layout_new_rejects_overlap() {
        // 2.2.1: no channel in two Zones; at most 15 Member Channels for one Zone.
        for (lower, upper) in [(16, 0), (0, 16), (7, 8), (15, 1), (1, 14)] {
            assert_eq!(MpeLayout::new(lower, upper), Err(UmpError::OutOfRange));
        }
        for (lower, upper) in [(0, 0), (15, 0), (0, 15), (7, 7), (1, 13)] {
            assert_eq!(layout(lower, upper).members(UPPER), upper);
        }
    }

    #[test]
    fn default_layout_is_lower_15() {
        // 2.2.2: the Lower Zone with Member Channels 2 to 16.
        assert_eq!(MpeLayout::DEFAULT, layout(15, 0));
        assert_eq!(
            (
                MpeLayout::DEFAULT.members(LOWER),
                MpeLayout::DEFAULT.members(UPPER)
            ),
            (15, 0)
        );
    }

    #[test]
    fn mcm_examples_b1_to_b4() {
        // B.1: the Lower Zone with 15 Member Channels.
        let mut i = Input::new();
        let events = i.feed(&mcm(0, 0x0F));
        assert_eq!(events[..2], [MpeEvent::NoZone, MpeEvent::NoZone]);
        assert_eq!(events[2], MpeEvent::Layout { reset: 0xFFFF });
        assert_eq!(i.mpe.layout(), layout(15, 0));
        assert_eq!(
            i.mpe.sensitivity(LOWER, true),
            MpeSensitivity::MANAGER_DEFAULT
        );
        assert_eq!(
            i.mpe.sensitivity(LOWER, false),
            MpeSensitivity::MEMBER_DEFAULT
        );
        assert_eq!(i.mpe.mode(LOWER), 3);
        // B.2: Lower 2 to 8, Upper off (it was not active: nothing to reset).
        let mut i = Input::new();
        i.feed(&mcm(0, 7));
        assert_eq!(i.feed(&mcm(15, 0))[2], MpeEvent::Layout { reset: 0 });
        assert_eq!(i.mpe.layout(), layout(7, 0));
        // B.3: Lower 2 to 8 and Upper 9 to 15, in either order.
        let mut i = Input::new();
        i.feed(&mcm(0, 7));
        assert_eq!(i.feed(&mcm(15, 7))[2], MpeEvent::Layout { reset: 0xFF00 });
        assert_eq!(i.mpe.layout(), layout(7, 7));
        let mut j = Input::new();
        j.feed(&mcm(15, 7));
        j.feed(&mcm(0, 7));
        assert_eq!(j.mpe.layout(), layout(7, 7));
        // B.4: the Upper Zone with 15 Member Channels, Channel 1 among them.
        let mut i = Input::new();
        i.feed(&mcm(15, 0x0F));
        assert_eq!(i.mpe.layout(), layout(0, 15));
        assert_eq!(i.mpe.layout().role(ch(0)), MpeRole::Member(UPPER));
    }

    #[test]
    fn mcm_example_b5_corrected() {
        // B.5 with the data byte 0x0B (AMB-M1-100-04): Upper 5 to 15, Lower keeps 2 to 4.
        let mut i = Input::new();
        i.feed(&mcm(0, 7));
        let reset = 0xFFF0; // Channels 5 to 16: the Upper Zone after the MCM.
        assert_eq!(i.feed(&mcm(15, 0x0B))[2], MpeEvent::Layout { reset });
        assert_eq!(i.mpe.layout(), layout(3, 11));
        assert_eq!(i.mpe.layout().role(ch(3)), MpeRole::Member(LOWER));
        assert_eq!(i.mpe.layout().role(ch(4)), MpeRole::Member(UPPER));
    }

    #[test]
    fn mcm_example_b6_deactivates() {
        // B.6: the Upper Zone with 14 Member Channels leaves the Lower Zone none.
        let mut i = Input::new();
        i.feed(&mcm(0, 7));
        assert_eq!(
            i.feed(&mcm(15, 0x0E))[2],
            MpeEvent::Layout { reset: 0xFFFF }
        );
        assert_eq!(i.mpe.layout(), layout(0, 14));
        assert_eq!(i.mpe.layout().role(ch(0)), MpeRole::NoZone);
        assert!((1..=14).all(|c| i.mpe.layout().role(ch(c)) == MpeRole::Member(UPPER)));
    }

    #[test]
    fn mcm_off_channel_and_bad_values_are_ignored() {
        // 2.2.1: an MCM on Channels 2 to 15 is invalid (Appendix E: P); AMB-M1-100-01: a value
        // above 15 changes nothing; the Data Entry LSB has no function.
        let mut i = Input::new();
        assert_eq!(i.feed(&mcm(2, 5))[2], MpeEvent::Ignored);
        assert_eq!(i.feed(&mcm(0, 0x10))[2], MpeEvent::Ignored);
        assert_eq!(i.mpe.layout(), MpeLayout::NONE);
        i.feed(&mcm(0, 7));
        assert_eq!(
            i.feed(&[[0xB0, 0x26, 0x05], [0xB0, 0x60, 0]]),
            [MpeEvent::Ignored; 2]
        );
        assert_eq!(i.mpe.layout(), layout(7, 0));
    }

    #[test]
    fn zone_change_gives_reset_mask_and_defaults() {
        // 2.2.3: reset the controls of each channel that enters or leaves a Zone; 2.2.5 and
        // AMB-M1-100-06: only the Zone of the MCM gets 2 and 48 semitones.
        let mut i = Input::new();
        i.feed(&mcm(15, 7));
        i.feed(&rpn_0(8, 12));
        i.feed(&[[0xE5, 0, 0x50], [0xB5, 0x4A, 0x10]]);
        i.one(&msg(&[0xD5, 0x22]));
        i.feed(&[[0xB1, 0x7E, 0x07]]);
        assert_eq!(
            (
                i.mpe.pitch_bend(ch(5)),
                i.mpe.pressure(ch(5)),
                i.mpe.brightness(ch(5))
            ),
            (0x2800, 0x22, 0x10)
        );
        assert_eq!(i.feed(&mcm(0, 7))[2], MpeEvent::Layout { reset: 0x00FF });
        assert_eq!(
            (
                i.mpe.pitch_bend(ch(5)),
                i.mpe.pressure(ch(5)),
                i.mpe.brightness(ch(5))
            ),
            (8192, 0, 64)
        );
        assert_eq!(i.mpe.sensitivity(UPPER, false), sens(12, 0));
        assert_eq!(
            i.mpe.sensitivity(LOWER, false),
            MpeSensitivity::MEMBER_DEFAULT
        );
        // A new MCM for a Zone in Mode 4 gives Mode 3 again (2.2.4).
        i.feed(&[[0xB1, 0x7E, 0x07]]);
        assert_eq!(i.mpe.mode(LOWER), 4);
        i.feed(&mcm(0, 7));
        assert_eq!(i.mpe.mode(LOWER), 3);
    }

    #[test]
    fn set_layout_activates_defaults() {
        // REQ-MPE-012: a power-up or panel layout.
        let mut r = MpeReceiver::new();
        assert_eq!(r.set_layout(layout(3, 0)), 0b1111);
        assert_eq!(r.sensitivity(LOWER, true), MpeSensitivity::MANAGER_DEFAULT);
        assert_eq!(r.sensitivity(LOWER, false), MpeSensitivity::MEMBER_DEFAULT);
        assert_eq!(r.set_layout(MpeLayout::DEFAULT), 0xFFFF);
        assert_eq!((r.layout(), r.mode(LOWER)), (MpeLayout::DEFAULT, 3));
        assert_eq!(r.set_layout(MpeLayout::NONE), 0xFFFF);
        assert!(!r.layout().is_active());
    }

    #[test]
    fn rpn_0_sets_zone_sensitivity() {
        // 2.2.5: RPN 0 on the Manager Channel, and on any Member Channel for all of them; the
        // LSB gives cents; RP-018 steps; a value above 96 stays (AMB-M1-100-07).
        let mut i = Input::new();
        i.mpe.set_layout(layout(7, 0));
        let manager = MpeEvent::Sensitivity {
            zone: LOWER,
            manager: true,
        };
        let member = MpeEvent::Sensitivity {
            zone: LOWER,
            manager: false,
        };
        assert_eq!(i.feed(&rpn_0(0, 12))[2], manager);
        assert_eq!(i.feed(&[[0xB0, 0x26, 50]]), [manager]);
        assert_eq!(i.mpe.sensitivity(LOWER, true), sens(12, 50));
        assert_eq!(i.feed(&rpn_0(3, 24))[2], member);
        assert_eq!(i.mpe.sensitivity(LOWER, false), sens(24, 0));
        assert_eq!(i.feed(&[[0xB3, 0x60, 0]]), [member]);
        assert_eq!(i.mpe.sensitivity(LOWER, false), sens(24, 1));
        i.feed(&[[0xB3, 0x06, 120]]);
        assert_eq!(i.mpe.sensitivity(LOWER, false), sens(120, 0));
        // A channel in no Zone: a conventional RPN 0.
        assert_eq!(i.feed(&rpn_0(10, 5))[2], MpeEvent::NoZone);
        assert_eq!(i.mpe.sensitivity(LOWER, false), sens(120, 0));
    }

    #[test]
    fn controls_are_tracked_without_notes() {
        // 2.2.6 to 2.2.8: the receiver keeps the last values with no note, in every role.
        let mut i = Input::new();
        assert!((0..16).all(|c| (
            i.mpe.pitch_bend(ch(c)),
            i.mpe.pressure(ch(c)),
            i.mpe.brightness(ch(c))
        ) == (8192, 0, 64)));
        assert_eq!(
            i.feed(&[[0xE5, 0, 0x50], [0xB5, 0x4A, 0x10]]),
            [MpeEvent::NoZone; 2]
        );
        i.mpe.set_layout(MpeLayout::DEFAULT);
        let e = i.feed(&[[0xE5, 0x01, 0x50], [0xB5, 0x4A, 0x11], [0xE0, 0x7F, 0x7F]]);
        assert_eq!(
            e[0],
            MpeEvent::Zone {
                zone: LOWER,
                manager: false
            }
        );
        assert_eq!(
            e[2],
            MpeEvent::Zone {
                zone: LOWER,
                manager: true
            }
        );
        assert_eq!(
            i.one(&msg(&[0xD5, 0x30])),
            MpeEvent::Zone {
                zone: LOWER,
                manager: false
            }
        );
        assert_eq!(
            (
                i.mpe.pitch_bend(ch(5)),
                i.mpe.pressure(ch(5)),
                i.mpe.brightness(ch(5))
            ),
            (0x2801, 0x30, 0x11)
        );
        assert_eq!(i.mpe.pitch_bend(ch(0)), 0x3FFF);
        // Notes on both roles (Appendix E: Rx M).
        assert_eq!(
            i.feed(&[[0x90, 60, 64]]),
            [MpeEvent::Zone {
                zone: LOWER,
                manager: true
            }]
        );
        assert_eq!(
            i.feed(&[[0x85, 60, 64]]),
            [MpeEvent::Zone {
                zone: LOWER,
                manager: false
            }]
        );
    }

    #[test]
    fn reset_all_controllers_centers_pitch_bend() {
        // RP-015: Pitch Bend to the center, Channel Pressure to 0; Control Change 74 stays.
        let mut i = Input::new();
        i.feed(&[[0xE5, 0, 0x50], [0xB5, 0x4A, 0x10]]);
        i.one(&msg(&[0xD5, 0x22]));
        i.feed(&[[0xB5, 0x79, 0]]);
        assert_eq!(
            (
                i.mpe.pitch_bend(ch(5)),
                i.mpe.pressure(ch(5)),
                i.mpe.brightness(ch(5))
            ),
            (8192, 0, 0x10)
        );
    }

    #[test]
    fn mode_message_on_lowest_member_channel() {
        // 2.2.4.3: the Mode message goes to the lowest Member Channel; any M (AMB-M1-100-14).
        let mut i = Input::new();
        i.mpe.set_layout(layout(7, 7));
        assert_eq!(
            i.feed(&[[0xB1, 0x7E, 0x03]]),
            [MpeEvent::Mode { zone: LOWER }]
        );
        assert_eq!(
            i.feed(&[[0xB8, 0x7E, 0x07]]),
            [MpeEvent::Mode { zone: UPPER }]
        );
        assert_eq!((i.mpe.mode(LOWER), i.mpe.mode(UPPER)), (4, 4));
        assert_eq!(i.feed(&[[0xB1, 0x7F, 0]]), [MpeEvent::Mode { zone: LOWER }]);
        assert_eq!(i.mpe.mode(LOWER), 3);
        // Not the lowest Member Channel, or the Manager Channel: given with the role.
        let e = i.feed(&[[0xB2, 0x7E, 0x07], [0xB0, 0x7E, 0x07]]);
        assert_eq!(
            e[0],
            MpeEvent::Zone {
                zone: LOWER,
                manager: false
            }
        );
        assert_eq!(
            e[1],
            MpeEvent::Zone {
                zone: LOWER,
                manager: true
            }
        );
        assert_eq!(i.mpe.mode(LOWER), 3);
    }

    #[test]
    fn program_change_follows_2_3_3() {
        // 2.3.3: in Mode 3 a Program Change on a Member Channel is ignored; Bank Select follows it
        // (AMB-M1-100-11); in Mode 4 both are given.
        let mut i = Input::new();
        i.mpe.set_layout(layout(7, 0));
        assert_eq!(i.one(&msg(&[0xC2, 5])), MpeEvent::Ignored);
        assert_eq!(
            i.feed(&[[0xB2, 0, 1], [0xB2, 0x20, 1]]),
            [MpeEvent::Ignored; 2]
        );
        assert_eq!(
            i.one(&msg(&[0xC0, 5])),
            MpeEvent::Zone {
                zone: LOWER,
                manager: true
            }
        );
        i.feed(&[[0xB1, 0x7E, 7]]);
        let member = MpeEvent::Zone {
            zone: LOWER,
            manager: false,
        };
        assert_eq!(i.one(&msg(&[0xC2, 5])), member);
        assert_eq!(i.feed(&[[0xB2, 0, 1]]), [member]);
    }

    #[test]
    fn poly_pressure_on_member_is_ignored() {
        // 2.2.7 and Appendix E: Polyphonic Key Pressure is P on Member Channels, O on the Manager.
        let mut i = Input::new();
        i.mpe.set_layout(MpeLayout::DEFAULT);
        assert_eq!(i.feed(&[[0xA2, 60, 64]]), [MpeEvent::Ignored]);
        assert_eq!(
            i.feed(&[[0xA0, 60, 64]]),
            [MpeEvent::Zone {
                zone: LOWER,
                manager: true
            }]
        );
    }

    #[test]
    fn all_sound_off_on_manager_is_given() {
        // AMB-M1-100-12 (user decision of 2026-11-05): the class is P, the receiver gives it.
        let mut i = Input::new();
        i.mpe.set_layout(MpeLayout::DEFAULT);
        assert_eq!(
            i.feed(&[[0xB0, 0x78, 0]]),
            [MpeEvent::Zone {
                zone: LOWER,
                manager: true
            }]
        );
        assert_eq!(
            mpe_class(MpeMessage::AllSoundOff, true, false, false),
            MpeClass::Prohibited
        );
    }

    #[test]
    fn class_follows_appendix_e() {
        // Table 5 (p. 27): Manager Tx, Manager Rx, Member Tx, Member Rx.
        use MpeClass::{Mandatory as M, Optional as O, Prohibited as P};
        let row = |m: MpeMessage, mono: bool| {
            [(true, true), (true, false), (false, true), (false, false)]
                .map(|(manager, tx)| mpe_class(m, manager, tx, mono))
        };
        let table = [
            (MpeMessage::Mcm, [M, M, P, P]),
            (MpeMessage::PitchBendSensitivity, [O, M, O, M]),
            (MpeMessage::PitchBend, [O, M, O, M]),
            (MpeMessage::ChannelPressure, [O, M, O, M]),
            (MpeMessage::Brightness, [O, M, O, M]),
            (MpeMessage::PolyPressure, [O, O, P, P]),
            (MpeMessage::AllSoundOff, [P, P, O, O]),
            (MpeMessage::ResetAllControllers, [O, O, O, O]),
            (MpeMessage::LocalControl, [O, O, O, O]),
            (MpeMessage::AllNotesOff, [O, O, O, O]),
            (MpeMessage::OmniOff, [O, O, O, O]),
            (MpeMessage::OmniOn, [P, P, P, P]),
            (MpeMessage::MonoModeOn, [P, P, O, O]),
            (MpeMessage::PolyModeOn, [P, P, O, O]),
            (MpeMessage::OtherControlChange, [O, O, O, O]),
            (MpeMessage::OtherRpn, [O, O, O, O]),
            (MpeMessage::Nrpn, [O, O, O, O]),
            (MpeMessage::ProgramChange, [O, O, P, P]),
            (MpeMessage::BankSelect, [O, O, P, P]),
            (MpeMessage::Note, [O, M, M, M]),
            (MpeMessage::System, [O, O, O, O]),
        ];
        for (m, classes) in table {
            assert_eq!(row(m, false), classes, "{m:?}");
        }
        // Program Change and Bank Select on Member Channels: Optional in Mode 4.
        assert_eq!(row(MpeMessage::ProgramChange, true), [O; 4]);
        assert_eq!(row(MpeMessage::BankSelect, true), [O; 4]);
        assert_eq!(row(MpeMessage::Mcm, true), [M, M, P, P]);
    }

    #[test]
    fn pitch_helper_c4_and_c6() {
        let (manager, member) = (
            MpeSensitivity::MANAGER_DEFAULT,
            MpeSensitivity::MEMBER_DEFAULT,
        );
        // C.4: +7 semitones at 48 gives 9387; +2 semitones at 2 gives 16383.
        assert_eq!(pitch_bend_value(7 * PITCH_UNITS_PER_SEMITONE, member), 9387);
        assert_eq!(
            pitch_bend_value(2 * PITCH_UNITS_PER_SEMITONE, manager),
            16383
        );
        // C.6: 2 semitones, 7 semitones within 0.01, 9 semitones within 0.01 (AMB-M1-100-09).
        assert_eq!(pitch_offset(16383, manager), 20_000);
        assert!((pitch_offset(9387, member) - 70_000).abs() <= 100);
        assert!((pitch_offset(16383, manager) + pitch_offset(9387, member) - 90_000).abs() <= 100);
        // AMB-M1-100-08: a quarter tone at 48 is 8277 (55H 40H); 0x2B 0x41 is a quarter tone at
        // 24 and one semitone at 48.
        assert_eq!(pitch_bend_value(5_000, member), 0x40 << 7 | 0x55);
        assert_eq!(pitch_bend_value(5_000, sens(24, 0)), 0x41 << 7 | 0x2B);
        assert_eq!(pitch_bend_value(10_000, member), 0x41 << 7 | 0x2B);
        // Down, the limits, and a sensitivity of 0.
        assert_eq!(pitch_bend_value(-70_000, member), 8192 - 1195);
        assert_eq!(pitch_bend_value(-480_000, member), 0);
        assert_eq!(pitch_bend_value(-500_000, member), 0);
        assert_eq!(pitch_bend_value(12_345, sens(0, 0)), 8192);
        assert_eq!(
            (pitch_offset(8192, member), pitch_offset(0, sens(0, 0))),
            (0, 0)
        );
        // Cents of the sensitivity: 0.5 semitone (5000) at 1 semitone 0 cents and at 0 semitones
        // 50 cents.
        assert_eq!(pitch_bend_value(5_000, sens(1, 0)), 8192 + 4096);
        assert_eq!(pitch_bend_value(5_000, sens(0, 50)), 16383);
    }

    #[test]
    fn prop_pitch_bend_value_stays_in_range() {
        // C.3 limits the value to 0 to 16383 (AMB-M1-100-09); C.5 then C.3 gives the value back
        // within 2 steps (8192 / 8191 and two roundings).
        proptest!(|(offset in any::<i32>(), semitones in 0u8..=127, cents in 0u8..=127, value in 0u16..=0x3FFF)| {
            let s = sens(semitones, cents);
            prop_assert!(pitch_bend_value(offset, s) <= 0x3FFF);
            if semitones > 0 {
                let back = pitch_bend_value(pitch_offset(value, s), s);
                prop_assert!((i32::from(back) - i32::from(value)).abs() <= 2, "{} {}", value, back);
            }
        });
    }

    /// `n / d` rounded half away from zero, in 64 bits: the reference of the C.3 and C.5 formulas.
    fn div_round(n: i64, d: i64) -> i64 {
        if n >= 0 {
            (n + d / 2) / d
        } else {
            -((d / 2 - n) / d)
        }
    }

    #[test]
    fn prop_pitch_helper_equals_the_formulas() {
        // The 32-bit helper gives the C.3 and C.5 formulas exactly.
        proptest!(|(offset in any::<i32>(), semitones in 0u8..=127, cents in 0u8..=127, value in 0u16..=0x3FFF)| {
            let s = sens(semitones, cents);
            let range = (i64::from(semitones) * 100 + i64::from(cents)) * 100;
            if range > 0 {
                let c3 = (div_round(i64::from(offset) * 8192, range) + 8192).clamp(0, 0x3FFF);
                prop_assert_eq!(i64::from(pitch_bend_value(offset, s)), c3);
            }
            let c5 = div_round(range * (i64::from(value) - 8192), 8191);
            prop_assert_eq!(i64::from(pitch_offset(value, s)), c5);
        });
    }

    #[test]
    fn zone_pitch_offset_is_the_sum() {
        // C.1, C.5, C.6: the pitch of a note is the Manager plus the Member Pitch Bend.
        let mut i = Input::new();
        i.mpe.set_layout(layout(3, 0));
        i.feed(&[[0xE0, 0x7F, 0x7F], [0xE1, 0x2B, 0x49]]); // 16383 and 9387.
        assert_eq!(i.mpe.pitch_offset(ch(0)), Some(20_000));
        let total = i.mpe.pitch_offset(ch(1)).unwrap();
        assert!((total - 90_000).abs() <= 100, "{total}");
        assert_eq!(i.mpe.pitch_offset(ch(2)), Some(20_000));
        assert_eq!(i.mpe.pitch_offset(ch(10)), None);
    }

    #[test]
    fn writers_give_the_appendix_b_bytes() {
        // Appendix B and C.2.
        assert_eq!(
            bytes(&mcm_messages(LOWER, 15).unwrap()),
            [0xB0, 0x65, 0, 0xB0, 0x64, 6, 0xB0, 0x06, 0x0F]
        );
        assert_eq!(
            bytes(&mcm_messages(UPPER, 0).unwrap()),
            [0xBF, 0x65, 0, 0xBF, 0x64, 6, 0xBF, 0x06, 0]
        );
        assert_eq!(
            bytes(&sensitivity_messages(ch(2), 48).unwrap()),
            [0xB2, 0x65, 0, 0xB2, 0x64, 0, 0xB2, 0x06, 48]
        );
        // A receiver takes what the writer gives (Example B.3).
        let mut i = Input::new();
        i.send(&mcm_messages(LOWER, 7).unwrap());
        let e = i.send(&mcm_messages(UPPER, 7).unwrap());
        assert_eq!(
            (e, i.mpe.layout()),
            (MpeEvent::Layout { reset: 0xFF00 }, layout(7, 7))
        );
    }

    #[test]
    fn zone_sensitivity_writes_every_member() {
        // 2.2.5: RPN 0 on the Manager Channel and on every Member Channel.
        let l = layout(7, 7);
        let blank = msg(&[0x80, 0, 0]);
        let mut out = [blank; 24];
        assert_eq!(zone_sensitivity_messages(l, UPPER, 2, 48, &mut out), Ok(24));
        let mut expected = vec![0xBF, 0x65, 0, 0xBF, 0x64, 0, 0xBF, 0x06, 2];
        for c in 8..=14u8 {
            expected.extend([0xB0 | c, 0x65, 0, 0xB0 | c, 0x64, 0, 0xB0 | c, 0x06, 48]);
        }
        assert_eq!(bytes(&out), expected);
        let mut i = Input::new();
        i.mpe.set_layout(l);
        i.feed(&rpn_0(15, 9));
        i.send(&out);
        assert_eq!(i.mpe.sensitivity(UPPER, true), sens(2, 0));
        assert_eq!(i.mpe.sensitivity(UPPER, false), sens(48, 0));
        let mut short = [blank; 23];
        assert_eq!(
            zone_sensitivity_messages(l, UPPER, 2, 48, &mut short),
            Err(UmpError::BufferTooSmall {
                needed: 24,
                got: 23
            })
        );
        assert_eq!(short, [blank; 23]);
        assert_eq!(
            zone_sensitivity_messages(layout(7, 0), UPPER, 2, 48, &mut out),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            zone_sensitivity_messages(l, LOWER, 2, 97, &mut out),
            Err(UmpError::OutOfRange)
        );
    }

    #[test]
    fn mode_writer_gives_m_members() {
        // 2.2.4.3 and AMB-M1-100-14: on the lowest Member Channel, M = the Member Channel count.
        let l = layout(7, 7);
        assert_eq!(
            bytes(&[mode_message(l, LOWER, true).unwrap()]),
            [0xB1, 0x7E, 7]
        );
        assert_eq!(
            bytes(&[mode_message(l, UPPER, true).unwrap()]),
            [0xB8, 0x7E, 7]
        );
        assert_eq!(
            bytes(&[mode_message(l, LOWER, false).unwrap()]),
            [0xB1, 0x7F, 0]
        );
        assert_eq!(
            mode_message(layout(7, 0), UPPER, true),
            Err(UmpError::OutOfRange)
        );
        let mut i = Input::new();
        i.mpe.set_layout(l);
        assert_eq!(
            i.send(&[mode_message(l, UPPER, true).unwrap()]),
            MpeEvent::Mode { zone: UPPER }
        );
        assert_eq!(i.mpe.mode(UPPER), 4);
    }

    #[test]
    fn note_writer_gives_corrected_table_4() {
        // Table 4 (p. 16) with the corrected Pitch Bend of AMB-M1-100-08: a quarter tone up at 48
        // semitones on Channel 3, CC 74 = 64, Channel Pressure 0, Note On 3CH 38H.
        let bend = pitch_bend_value(5_000, MpeSensitivity::MEMBER_DEFAULT);
        let m = note_messages(ch(2), 0x3C, 0x38, bend, 0x40, 0).unwrap();
        assert_eq!(
            bytes(&m),
            [0xE2, 0x55, 0x40, 0xB2, 0x4A, 0x40, 0xD2, 0x00, 0x92, 0x3C, 0x38]
        );
    }

    #[test]
    fn writers_reject_bad_values() {
        assert_eq!(mcm_messages(LOWER, 16), Err(UmpError::OutOfRange));
        assert_eq!(sensitivity_messages(ch(0), 97), Err(UmpError::OutOfRange));
        for (note, velocity, bend, cc74, pressure) in [
            (60, 0, 8192, 64, 0),
            (128, 64, 8192, 64, 0),
            (60, 128, 8192, 64, 0),
            (60, 64, 0x4000, 64, 0),
            (60, 64, 8192, 128, 0),
            (60, 64, 8192, 64, 128),
        ] {
            assert_eq!(
                note_messages(ch(1), note, velocity, bend, cc74, pressure),
                Err(UmpError::OutOfRange)
            );
        }
    }

    #[test]
    fn allocator_prefers_free_then_oldest() {
        // 2.2.4.1 and A.3: an unoccupied channel first, then the oldest last Note Off.
        let l = layout(3, 0);
        let mut a = MpeNoteAllocator::new();
        let first: Vec<_> = (0..3)
            .map(|k| a.note_on(l, LOWER, 60 + k).unwrap())
            .collect();
        assert_eq!(first, [ch(1), ch(2), ch(3)]);
        a.note_off(ch(2));
        a.note_off(ch(1));
        a.note_off(ch(3));
        let next: Vec<_> = (0..3)
            .map(|k| a.note_on(l, LOWER, 70 + k).unwrap())
            .collect();
        assert_eq!(next, [ch(2), ch(1), ch(3)]);
        assert_eq!(a.note_on(l, UPPER, 60), None);
        // A Note Off on a channel with no active note changes nothing.
        let mut b = MpeNoteAllocator::new();
        b.note_off(ch(1));
        assert_eq!(b, MpeNoteAllocator::new());
    }

    #[test]
    fn allocator_shares_when_full() {
        // 2.2.4.1: with every Member Channel occupied, a new note shares the channel with the
        // fewest active notes.
        let l = layout(3, 0);
        let mut a = MpeNoteAllocator::new();
        for k in 0..3 {
            a.note_on(l, LOWER, 60 + k);
        }
        assert_eq!(a.note_on(l, LOWER, 63), Some(ch(1)));
        assert_eq!((a.active_notes(ch(1)), a.active_notes(ch(2))), (2, 1));
        a.note_off(ch(2));
        assert_eq!(a.note_on(l, LOWER, 64), Some(ch(2)));
        a.reset();
        assert!((0..16).all(|c| a.active_notes(ch(c)) == 0));
    }

    #[test]
    fn allocator_same_note_option() {
        // A.3: re-use the channel that last played the same Note Number (option, off by default).
        let l = layout(3, 0);
        let play = |a: &mut MpeNoteAllocator| {
            a.note_on(l, LOWER, 60);
            a.note_on(l, LOWER, 62);
            a.note_off(ch(1));
            a.note_off(ch(2));
            a.note_on(l, LOWER, 60)
        };
        let mut off = MpeNoteAllocator::default();
        assert!(!off.same_note());
        assert_eq!(play(&mut off), Some(ch(3)));
        let mut on = MpeNoteAllocator::new();
        on.set_same_note(true);
        assert_eq!(play(&mut on), Some(ch(1)));
        on.reset();
        assert!(on.same_note());
    }

    #[test]
    fn new_states_are_all_zeros() {
        // A static receiver or allocator goes in .bss: no flash image.
        let r = MpeReceiver::default();
        assert_eq!(r, MpeReceiver::new());
        assert_eq!(
            (r.layout, r.mono, r.sensitivity),
            (MpeLayout::NONE, 0, [[[0; 2]; 2]; 2])
        );
        assert_eq!(
            (r.pitch_bend, r.pressure, r.brightness),
            ([0; 16], [0; 16], [0; 16])
        );
        let a = MpeNoteAllocator::new();
        assert_eq!(
            (a.active, a.last_note, a.last_off, a.clock, a.same_note),
            ([0; 16], [0; 16], [0; 16], 0, false)
        );
        assert_eq!(
            (
                core::mem::size_of::<MpeLayout>(),
                core::mem::size_of::<MpeReceiver>(),
                core::mem::size_of::<MpeNoteAllocator>()
            ),
            (2, 76, 104)
        );
    }

    #[test]
    fn prop_mpe_receiver_never_panics() {
        // Any message stream keeps a valid layout (no channel in two Zones).
        proptest!(|(stream in proptest::collection::vec((0x80u8..=0xEF, 0u8..=0x7F, 0u8..=0x7F), 0..64))| {
            let mut i = Input::new();
            for (s, d1, d2) in stream {
                i.one(&msg(&[s, d1, d2]));
                let l = i.mpe.layout();
                prop_assert_eq!(MpeLayout::new(l.members(LOWER), l.members(UPPER)), Ok(l));
            }
        });
    }
}
