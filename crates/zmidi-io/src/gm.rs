//! The General MIDI receiver of one input (RP-024 General MIDI 2 v1.2a, RP-003 General MIDI
//! Level 1; DEC-022). Feature `gm`.
//!
//! - [`GmReceiver`] holds the kind of the 16 channels (Melody or Rhythm, [`ChannelKind`]), their
//!   GM mode (Mode 3 or Mode 4), the GM level ([`GmLevel`]) and the [`GmConfig`], in 10 bytes
//!   (REQ-GMR-001). It starts with channel 10 (packet value 9) a Rhythm Channel and all channels
//!   in Mode 3 (RP-024 2.4, 2.5). [`GmReceiver::reset`] is the GM reset of a GM1 or GM2 System On
//!   (RP-024 4.9; REQ-GMR-002); the application then mutes all notes, calls
//!   [`gm_reset_controller_state`] for each channel (REQ-GMR-005) and resets each parameter
//!   receiver.
//! - [`GmReceiver::program`] sets the kind from a program selection with its held bank: MSB 78H
//!   Rhythm on a channel that can be one (10 and 11 by default), 79H Melody, another bank keeps
//!   the kind (RP-024 2.4, 3.3.1; AMB-RP-024-06; REQ-GMR-006).
//! - [`GmReceiver::receive`] gives the [`ModeAction`] of a message by the GM2 rules: Omni Off and
//!   Omni On give All Notes Off and change no mode (AMB-RP-024-04; REQ-GMR-007); Mono On acts only
//!   with M = 1, and on a Rhythm Channel sets Mode 4 only when the configuration permits it
//!   (AMB-RP-024-05; REQ-GMR-008). The MIDI 1.0 [`crate::ModeState`] does not change (rule R0).
//! - The GM2 default values (RP-024 sections 3 and 4; also the GM1 values of RP-003;
//!   REQ-GMR-003, REQ-GMR-004) and the Rhythm Channel table of the messages that GM2 recommends a
//!   Rhythm Channel does not respond to (REQ-GMR-009). The application decides the response.
//!
//! The kind and the reset come from the General MIDI System messages, which the crate
//! `zmidi-sysex` (feature `gm`) reads; the application gives them here. No alloc (DEC-002), no I/O
//! (DEC-003). Never panics.

use zmidi_ump::controller::Rpn;
use zmidi_ump::{Channel, ChannelModeMessage, Midi1ChannelVoiceMessage};

use crate::channel_mode::ModeAction;
use crate::controllers::ControllerState;
use crate::parameters::Parameter;

/// Channel 10, packet value 9: the Rhythm Channel at the start and after a GM reset.
const CHANNEL_10: u16 = 1 << 9;
/// Channels 10 and 11: the channels that can be Rhythm Channels by default (RP-024 2.4).
const CHANNELS_10_11: u16 = 1 << 9 | 1 << 10;
/// Bank Select MSB 78H: the Rhythm banks (RP-024 2.4).
const RHYTHM_MSB: u16 = 0x78;
/// Bank Select MSB 79H: the Melody banks (RP-024 2.4).
const MELODY_MSB: u16 = 0x79;

/// The General MIDI level of the receiver (REQ-GMR-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GmLevel {
    /// After a GM System Off: the device is in its own mode.
    Off,
    /// After a GM1 System On (RP-003).
    Gm1,
    /// At the start and after a GM2 System On (RP-024).
    Gm2,
}

/// The kind of a channel (RP-024 2.4; REQ-GMR-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelKind {
    /// A channel that plays the GM2 Sound Set (banks 79H xxH).
    Melody,
    /// A channel that plays the GM2 Percussion Sound Set (banks 78H xxH).
    Rhythm,
}

/// The configuration of a [`GmReceiver`] (REQ-GMR-006, REQ-GMR-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GmConfig {
    /// Bit n is set when the channel with packet value n can be a Rhythm Channel. RP-024 2.4
    /// gives channels 10 and 11; any channel is optional.
    pub rhythm_capable: u16,
    /// `true` when a Mono On with M = 1 sets Mode 4 on a Rhythm Channel too (RP-024 3.5.6: "may or
    /// may not change modes"); `false` keeps the drums polyphonic.
    pub rhythm_mono: bool,
}

impl GmConfig {
    /// Channels 10 and 11 can be Rhythm Channels; Mode 4 is not set on a Rhythm Channel.
    pub const DEFAULT: Self = Self::new(CHANNELS_10_11, false);

    /// The configuration of `rhythm_capable` and `rhythm_mono`.
    pub const fn new(rhythm_capable: u16, rhythm_mono: bool) -> Self {
        Self {
            rhythm_capable,
            rhythm_mono,
        }
    }
}

impl Default for GmConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The event of a GM reset (REQ-GMR-002). The application mutes all notes "without producing a
/// click" (RP-024 4.9.1), calls [`gm_reset_controller_state`] for each channel and resets each
/// parameter receiver; for [`GmLevel::Gm1`] it can leave out the GM2 features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GmReset {
    /// The new level.
    pub level: GmLevel,
}

/// The General MIDI state of the 16 channels of one input (REQ-GMR-001). 10 bytes, no alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GmReceiver {
    /// Bit n: the channel with packet value n is a Rhythm Channel.
    rhythm: u16,
    /// Bit n: the channel with packet value n is in Mode 4 (Omni Off, Mono).
    mono: u16,
    level: GmLevel,
    config: GmConfig,
}

impl GmReceiver {
    /// The start state: channel 10 a Rhythm Channel, the other channels Melody Channels, each
    /// channel in Mode 3 (Omni Off, Poly), level GM2 (RP-024 2.4, 2.5; REQ-GMR-002).
    pub const fn new(config: GmConfig) -> Self {
        Self {
            rhythm: CHANNEL_10,
            mono: 0,
            level: GmLevel::Gm2,
            config,
        }
    }

    /// The GM reset of a GM1 System On or a GM2 System On (RP-024 4.9.1, 4.9.2): the start channel
    /// state, the same configuration and the new level (REQ-GMR-002). [`GmLevel::Off`] is
    /// [`GmReceiver::system_off`].
    pub fn reset(&mut self, level: GmLevel) -> GmReset {
        if level == GmLevel::Off {
            self.system_off();
        } else {
            *self = Self {
                level,
                ..Self::new(self.config)
            };
        }
        GmReset { level }
    }

    /// A GM System Off (RP-024 4.9.3): the level Off; no other state changes (REQ-GMR-002).
    pub fn system_off(&mut self) {
        self.level = GmLevel::Off;
    }

    /// A program selection of `channel` with the bank that its controller state holds
    /// ([`crate::ControllerEvent::Program`]): bank MSB 78H makes the channel a Rhythm Channel when
    /// the configuration permits it, 79H a Melody Channel; another bank MSB or no bank keeps the
    /// kind (RP-024 2.4, 3.3.1; AMB-RP-024-06; REQ-GMR-006). A Bank Select alone is not a program
    /// selection.
    pub fn program(&mut self, channel: Channel, bank: Option<u16>) {
        let bit = 1u16 << channel.into_bits();
        match bank.map(|b| b >> 7 & 0x7F) {
            Some(RHYTHM_MSB) if self.config.rhythm_capable & bit != 0 => self.rhythm |= bit,
            Some(MELODY_MSB) => self.rhythm &= !bit,
            _ => {}
        }
    }

    /// The action of one message of the input by the GM2 rules (RP-024 2.5, 3.5; REQ-GMR-007,
    /// REQ-GMR-008). The Channel Mode messages act on their channel only: All Sound Off, Reset
    /// All Controllers and All Notes Off for the channel; Local Control; Omni Off and Omni On give
    /// All Notes Off with no mode change; Poly On sets Mode 3; Mono On with M = 1 sets Mode 4 on a
    /// Melody Channel, and on a Rhythm Channel only with [`GmConfig::rhythm_mono`] (else All Notes
    /// Off); Mono On with another M is ignored. Each other message is a voice message of its
    /// channel (Mode 4: voice 0).
    pub fn receive(&mut self, message: &Midi1ChannelVoiceMessage) -> ModeAction {
        let channel = message.channel();
        let bit = 1u16 << channel.into_bits();
        let mode = match message {
            Midi1ChannelVoiceMessage::ControlChange(cc) => cc.channel_mode_message(),
            _ => None,
        };
        let one = Some(channel);
        match mode {
            None => ModeAction::Voice {
                voice: (self.mono & bit != 0).then_some(0),
            },
            Some(ChannelModeMessage::AllSoundOff) => ModeAction::AllSoundOff { channel: one },
            Some(ChannelModeMessage::ResetAllControllers) => {
                ModeAction::ResetAllControllers { channel: one }
            }
            Some(ChannelModeMessage::AllNotesOff)
            | Some(ChannelModeMessage::OmniOff)
            | Some(ChannelModeMessage::OmniOn) => ModeAction::AllNotesOff { channel: one },
            Some(ChannelModeMessage::LocalControl(on)) => ModeAction::LocalControl { on },
            Some(ChannelModeMessage::MonoOn(1)) => {
                if self.rhythm & bit != 0 && !self.config.rhythm_mono {
                    ModeAction::AllNotesOff { channel: one }
                } else {
                    self.set_mono(bit, true)
                }
            }
            Some(ChannelModeMessage::MonoOn(_)) => ModeAction::Ignore,
            Some(ChannelModeMessage::PolyOn) => self.set_mono(bit, false),
        }
    }

    fn set_mono(&mut self, bit: u16, mono: bool) -> ModeAction {
        let before = self.mono;
        if mono {
            self.mono |= bit;
        } else {
            self.mono &= !bit;
        }
        ModeAction::ModeChange {
            changed: self.mono != before,
            alternate: false,
        }
    }

    /// The kind of `channel` (REQ-GMR-001).
    pub const fn kind(&self, channel: Channel) -> ChannelKind {
        if self.rhythm >> channel.into_bits() & 1 != 0 {
            ChannelKind::Rhythm
        } else {
            ChannelKind::Melody
        }
    }

    /// True when `channel` is in Mode 4 (Omni Off, Mono), false in Mode 3 (REQ-GMR-001).
    pub const fn is_mono(&self, channel: Channel) -> bool {
        self.mono >> channel.into_bits() & 1 != 0
    }

    /// The Rhythm Channels: bit n is the channel with packet value n (REQ-GMR-001). The
    /// application takes them out of the channel set of a Scale/Octave Tuning message (RP-024
    /// 4.7, REQ-GMR-009).
    pub const fn rhythm_channels(&self) -> u16 {
        self.rhythm
    }

    /// The GM level (REQ-GMR-002).
    pub const fn level(&self) -> GmLevel {
        self.level
    }

    /// The configuration.
    pub const fn config(&self) -> GmConfig {
        self.config
    }
}

impl Default for GmReceiver {
    fn default() -> Self {
        Self::new(GmConfig::DEFAULT)
    }
}

/// The GM2 default value of controller `controller` (RP-024 3.3; REQ-GMR-003), or `None` when GM2
/// gives no default: Modulation (1), Portamento Time (5) and Data Entry (6) 0, Channel Volume (7)
/// 100, Pan (10) 64, Expression (11) 127, Hold1, Portamento On/Off, Sostenuto and Soft (64 to 67)
/// 0, the Sound Controllers 71 to 78 64, Reverb Send Level (91) 40 and Chorus Send Level (93) 0.
/// RP-003 gives the same Volume, and Reset All Controllers values for the others (AMB-RP-003-01).
pub const fn gm2_controller_default(controller: u8) -> Option<u8> {
    match controller {
        1 | 5 | 6 | 64..=67 | 93 => Some(0),
        7 => Some(100),
        10 | 71..=78 => Some(64),
        11 => Some(127),
        91 => Some(40),
        _ => None,
    }
}

/// The GM2 default program: 0, "1" in one-based documentation (RP-024 3.2; REQ-GMR-003).
pub const GM2_PROGRAM_DEFAULT: u8 = 0;
/// The GM2 Melody bank 79H 00H, as the 14-bit bank of the controller state (RP-024 3.3.1;
/// REQ-GMR-003).
pub const GM2_MELODY_BANK: u16 = MELODY_MSB << 7;
/// The GM2 Rhythm bank 78H 00H, the default bank of channel 10 (RP-024 3.3.1; REQ-GMR-003).
pub const GM2_RHYTHM_BANK: u16 = RHYTHM_MSB << 7;
/// The default Pitch Bend Sensitivity (RPN 00H 00H): 02H 00H, 2 semitones 0 cents, as the 14-bit
/// Data Entry value (RP-024 3.4.1; REQ-GMR-003).
pub const GM2_PITCH_BEND_SENSITIVITY: u16 = 2 << 7;
/// The default Channel Fine Tuning (RPN 00H 01H): 40H 00H, 2000H (RP-024 3.4.2; REQ-GMR-003).
pub const GM2_CHANNEL_FINE_TUNING: u16 = 0x2000;
/// The default Channel Coarse Tuning (RPN 00H 02H): 40H 00H, 2000H (RP-024 3.4.3; REQ-GMR-003).
pub const GM2_CHANNEL_COARSE_TUNING: u16 = 0x2000;
/// The default Modulation Depth Range (RPN 00H 05H): 00H 40H, 50 cents (RP-024 3.4.4;
/// REQ-GMR-003).
pub const GM2_MODULATION_DEPTH_RANGE: u16 = 0x0040;
/// The default Pitch Bend: 2000H, the center (RP-024 3.6; REQ-GMR-003).
pub const GM2_PITCH_BEND: u16 = 0x2000;
/// The default Channel Pressure: 0 (RP-024 3.7; REQ-GMR-003).
pub const GM2_CHANNEL_PRESSURE: u8 = 0;
/// The default Master Volume: 3FFFH (RP-024 4.1; AMB-RP-024-10; REQ-GMR-004).
pub const GM2_MASTER_VOLUME: u16 = 0x3FFF;
/// The default Master Fine Tuning: 2000H, A440 (RP-024 4.2; REQ-GMR-004).
pub const GM2_MASTER_FINE_TUNING: u16 = 0x2000;
/// The default Master Coarse Tuning MSB: 40H, A440 (RP-024 4.3; REQ-GMR-004).
pub const GM2_MASTER_COARSE_TUNING: u8 = 0x40;
/// The default of each of the 12 Scale/Octave Tuning offsets: 40H, 0 cents (RP-024 4.7;
/// AMB-RP-024-13; REQ-GMR-004).
pub const GM2_SCALE_TUNING_OFFSET: u8 = 0x40;

/// The GM2 default of each controller 0 to 31 ([`gm2_controller_default`]).
const GM2_MSB: [Option<u8>; 32] = {
    let mut t = [None; 32];
    let mut n = 0;
    while n < 32 {
        t[n] = gm2_controller_default(n as u8);
        n += 1;
    }
    t
};

/// Sets `state`, the controller state of `channel`, for a GM reset: the GM2 defaults of
/// controllers 0 to 31, the Rhythm bank on channel 10 and the Melody bank on the others, and no
/// Portamento Control source (RP-024 3.3, 4.9.1; REQ-GMR-005). A controller with no GM2 default
/// gets its start value. The state is set in place.
pub fn gm_reset_controller_state(state: &mut ControllerState, channel: Channel) {
    let bank = if channel.into_bits() == 9 {
        GM2_RHYTHM_BANK
    } else {
        GM2_MELODY_BANK
    };
    state.hold(&GM2_MSB, bank);
}

/// True when GM2 recommends that a Rhythm Channel does not respond to controller `controller`:
/// Modulation (1), 64 to 67 and 71 to 78 (RP-024 3.3.2 to 3.3.18; REQ-GMR-009). The 14-bit event of
/// the controller state names controller 1 for its LSB 33 too.
pub const fn rhythm_ignores_controller(controller: u8) -> bool {
    matches!(controller, 1 | 64..=67 | 71..=78)
}

/// True when GM2 recommends that a Rhythm Channel does not respond to `parameter`: RPN 00H 00H,
/// 00H 01H, 00H 02H and 00H 05H (RP-024 3.4; REQ-GMR-009).
pub const fn rhythm_ignores_parameter(parameter: Parameter) -> bool {
    matches!(
        parameter,
        Parameter::Registered(
            Rpn::PitchBendSensitivity
                | Rpn::FineTuning
                | Rpn::CoarseTuning
                | Rpn::ModulationDepthRange
        )
    )
}

/// True when GM2 recommends that a Rhythm Channel does not respond to `message`: Pitch Bend,
/// Channel Pressure and the controllers of [`rhythm_ignores_controller`] (RP-024 3.3, 3.6, 3.7;
/// REQ-GMR-009). For Data Entry, use [`rhythm_ignores_parameter`] with the selected parameter.
pub const fn rhythm_ignores_message(message: &Midi1ChannelVoiceMessage) -> bool {
    match message {
        Midi1ChannelVoiceMessage::PitchBend(_) | Midi1ChannelVoiceMessage::ChannelPressure(_) => {
            true
        }
        Midi1ChannelVoiceMessage::ControlChange(cc) => rhythm_ignores_controller(cc.index()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controllers::ControllerEvent;

    fn ch(n: u8) -> Channel {
        Channel::from_bits(n)
    }

    /// The typed message of MIDI 1.0 bytes.
    fn msg(bytes: [u8; 3]) -> Midi1ChannelVoiceMessage {
        let [a, b, c] = bytes.map(u32::from);
        Midi1ChannelVoiceMessage::parse(&[0x2000_0000 | a << 16 | b << 8 | c]).unwrap()
    }

    fn cc(channel: u8, controller: u8, value: u8) -> Midi1ChannelVoiceMessage {
        msg([0xB0 | channel, controller, value])
    }

    #[test]
    fn gm_receiver_holds_kind_and_mode_of_16_channels() {
        // REQ-GMR-001: 16 channels in a fixed size, the Rhythm mask.
        let mut r = GmReceiver::default();
        assert!(core::mem::size_of::<GmReceiver>() <= 10);
        assert_eq!(r.rhythm_channels(), 1 << 9);
        for n in 0..16 {
            let k = if n == 9 {
                ChannelKind::Rhythm
            } else {
                ChannelKind::Melody
            };
            assert_eq!((r.kind(ch(n)), r.is_mono(ch(n))), (k, false), "channel {n}");
        }
        r.receive(&cc(3, 126, 1));
        r.program(ch(10), Some(GM2_RHYTHM_BANK));
        assert!(r.is_mono(ch(3)));
        assert_eq!(r.rhythm_channels(), 1 << 9 | 1 << 10);
    }

    #[test]
    fn gm_receiver_starts_and_resets_in_gm2_state() {
        // REQ-GMR-002: RP-024 2.4, 2.5, 4.9.1 and 4.9.2.
        let config = GmConfig::new(0xFFFF, true);
        let mut r = GmReceiver::new(config);
        assert_eq!(r.level(), GmLevel::Gm2);
        r.program(ch(0), Some(GM2_RHYTHM_BANK));
        r.receive(&cc(4, 126, 1));
        assert_eq!(
            r.reset(GmLevel::Gm1),
            GmReset {
                level: GmLevel::Gm1
            }
        );
        assert_eq!(
            r,
            GmReceiver {
                level: GmLevel::Gm1,
                ..GmReceiver::new(config)
            }
        );
        assert_eq!(r.config(), config);
        assert_eq!(r.reset(GmLevel::Gm2).level, GmLevel::Gm2);
        assert_eq!(r, GmReceiver::new(config));
    }

    #[test]
    fn gm_system_off_keeps_the_channel_state() {
        // REQ-GMR-002: RP-024 4.9.3; the level only.
        let mut r = GmReceiver::default();
        r.receive(&cc(4, 126, 1));
        let before = r;
        r.system_off();
        assert_eq!(r.level(), GmLevel::Off);
        assert_eq!(
            r,
            GmReceiver {
                level: GmLevel::Off,
                ..before
            }
        );
        assert_eq!(r.reset(GmLevel::Off).level, GmLevel::Off);
        assert!(r.is_mono(ch(4)));
    }

    #[test]
    fn gm2_channel_defaults_follow_rp_024() {
        // REQ-GMR-003: each "Default Value" of RP-024 sections 3.2 to 3.7.
        let expected = [
            (1, 0),
            (5, 0),
            (6, 0),
            (7, 100),
            (10, 64),
            (11, 127),
            (64, 0),
            (65, 0),
            (66, 0),
            (67, 0),
            (91, 40),
            (93, 0),
        ];
        for (c, v) in expected {
            assert_eq!(gm2_controller_default(c), Some(v), "controller {c}");
        }
        for c in 71..=78 {
            assert_eq!(gm2_controller_default(c), Some(64), "controller {c}");
        }
        let listed = |c: u8| expected.iter().any(|e| e.0 == c) || (71..=78).contains(&c);
        for c in (0..=127u8).filter(|c| !listed(*c)) {
            assert_eq!(gm2_controller_default(c), None, "controller {c}");
        }
        assert_eq!(GM2_PROGRAM_DEFAULT, 0);
        assert_eq!((GM2_MELODY_BANK >> 7, GM2_MELODY_BANK & 0x7F), (0x79, 0));
        assert_eq!((GM2_RHYTHM_BANK >> 7, GM2_RHYTHM_BANK & 0x7F), (0x78, 0));
        assert_eq!(
            (
                GM2_PITCH_BEND_SENSITIVITY >> 7,
                GM2_PITCH_BEND_SENSITIVITY & 0x7F
            ),
            (2, 0)
        );
        assert_eq!(GM2_CHANNEL_FINE_TUNING, 0x40 << 7);
        assert_eq!(GM2_CHANNEL_COARSE_TUNING, 0x40 << 7);
        assert_eq!(
            zmidi_ump::controller::modulation_depth_range(GM2_MODULATION_DEPTH_RANGE),
            5_000
        );
        assert_eq!((GM2_PITCH_BEND, GM2_CHANNEL_PRESSURE), (0x2000, 0));
    }

    #[test]
    fn gm2_device_defaults_follow_rp_024() {
        // REQ-GMR-004: RP-024 4.1 "7FH/7FH", 4.2 and 4.3 "40H/00H", 4.7 "40H" (AMB-RP-024-10).
        assert_eq!(GM2_MASTER_VOLUME, 0x7F << 7 | 0x7F);
        assert_eq!(GM2_MASTER_FINE_TUNING, 0x40 << 7);
        assert_eq!(GM2_MASTER_COARSE_TUNING, 0x40);
        assert_eq!(GM2_SCALE_TUNING_OFFSET, 0x40);
    }

    #[test]
    fn gm_controller_state_holds_gm2_defaults() {
        // REQ-GMR-005: an LSB gives the held MSB; the bank of channel 10 is 78H 00H.
        for (channel, bank) in [
            (9, GM2_RHYTHM_BANK),
            (0, GM2_MELODY_BANK),
            (10, GM2_MELODY_BANK),
        ] {
            let mut s = ControllerState::new();
            s.receive(&msg([0xB0 | channel, 84, 60])); // a Portamento Control source
            gm_reset_controller_state(&mut s, ch(channel));
            assert_eq!(s.bank(), Some(bank));
            for (c, v) in [
                (1u8, 0u16),
                (5, 0),
                (7, 100),
                (10, 64),
                (11, 127),
                (2, 0),
                (8, 0),
            ] {
                let e = s.receive(&cc(channel, c + 32, 3));
                assert_eq!(
                    e,
                    Some(ControllerEvent::Controller14 {
                        controller: c,
                        value: v << 7 | 3
                    }),
                    "controller {c}"
                );
            }
            // No Portamento Control source.
            let on = msg([0x90 | channel, 60, 64]);
            assert_eq!(s.receive(&on), None);
            let pc = msg([0xC0 | channel, 0, 0]);
            assert_eq!(
                s.receive(&pc),
                Some(ControllerEvent::Program {
                    program: 0,
                    bank: Some(bank)
                })
            );
        }
    }

    #[test]
    fn program_selection_sets_the_channel_kind() {
        // REQ-GMR-006: RP-024 3.3.1 on channels 10 and 11 (AMB-RP-024-06).
        let mut r = GmReceiver::default();
        r.program(ch(10), Some(GM2_RHYTHM_BANK | 5));
        assert_eq!(r.kind(ch(10)), ChannelKind::Rhythm);
        r.program(ch(10), Some(0x00 << 7)); // a GM1 file: bank 00H keeps the kind
        assert_eq!(r.kind(ch(10)), ChannelKind::Rhythm);
        r.program(ch(10), None);
        assert_eq!(r.kind(ch(10)), ChannelKind::Rhythm);
        r.program(ch(10), Some(GM2_MELODY_BANK | 1));
        assert_eq!(r.kind(ch(10)), ChannelKind::Melody);
        r.program(ch(9), Some(GM2_MELODY_BANK));
        assert_eq!(r.kind(ch(9)), ChannelKind::Melody);
        r.program(ch(9), Some(GM2_RHYTHM_BANK));
        assert_eq!(r.kind(ch(9)), ChannelKind::Rhythm);
        // Channel 1 can not be a Rhythm Channel by default.
        r.program(ch(0), Some(GM2_RHYTHM_BANK));
        assert_eq!(r.kind(ch(0)), ChannelKind::Melody);
    }

    #[test]
    fn rhythm_channels_follow_the_configuration() {
        // REQ-GMR-006: RP-024 2.4 "Any Channel can be used as a Rhythm Channel" (optional).
        let mut r = GmReceiver::new(GmConfig::new(0xFFFF, false));
        for n in 0..16 {
            r.program(ch(n), Some(GM2_RHYTHM_BANK));
        }
        assert_eq!(r.rhythm_channels(), 0xFFFF);
        assert_eq!(GmConfig::default(), GmConfig::new(1 << 9 | 1 << 10, false));
    }

    #[test]
    fn gm_channel_mode_rules_follow_rp_024() {
        // REQ-GMR-007: RP-024 3.5.1 to 3.5.5, 3.5.7; each message acts on its channel only.
        let mut r = GmReceiver::default();
        let one = Some(ch(2));
        assert_eq!(
            r.receive(&cc(2, 120, 0)),
            ModeAction::AllSoundOff { channel: one }
        );
        assert_eq!(
            r.receive(&cc(2, 121, 0)),
            ModeAction::ResetAllControllers { channel: one }
        );
        assert_eq!(
            r.receive(&cc(2, 122, 0)),
            ModeAction::LocalControl { on: false }
        );
        assert_eq!(
            r.receive(&cc(2, 123, 0)),
            ModeAction::AllNotesOff { channel: one }
        );
        // Omni Off and Omni On: All Notes Off, no mode change (AMB-RP-024-04).
        r.receive(&cc(2, 126, 1));
        for c in [124, 125] {
            assert_eq!(
                r.receive(&cc(2, c, 0)),
                ModeAction::AllNotesOff { channel: one }
            );
            assert!(r.is_mono(ch(2)));
        }
        // Poly On: Mode 3.
        let change = |changed| ModeAction::ModeChange {
            changed,
            alternate: false,
        };
        assert_eq!(r.receive(&cc(2, 127, 0)), change(true));
        assert!(!r.is_mono(ch(2)));
        assert_eq!(r.receive(&cc(2, 127, 0)), change(false));
        // Voice messages: Mode 3 no voice number, Mode 4 voice 0.
        let note = msg([0x92, 60, 64]);
        assert_eq!(r.receive(&note), ModeAction::Voice { voice: None });
        r.receive(&cc(2, 126, 1));
        assert_eq!(r.receive(&note), ModeAction::Voice { voice: Some(0) });
        assert!(!r.is_mono(ch(3)));
    }

    #[test]
    fn gm_mono_on_acts_only_with_m_1() {
        // REQ-GMR-008: RP-024 2.5, 3.5.6 (AMB-RP-024-05).
        let mut r = GmReceiver::default();
        let change = ModeAction::ModeChange {
            changed: true,
            alternate: false,
        };
        for m in [0, 2, 16, 127] {
            assert_eq!(r.receive(&cc(0, 126, m)), ModeAction::Ignore, "M = {m}");
            assert_eq!(r.receive(&cc(9, 126, m)), ModeAction::Ignore, "M = {m}");
        }
        assert!(!r.is_mono(ch(0)));
        assert_eq!(r.receive(&cc(0, 126, 1)), change);
        assert!(r.is_mono(ch(0)));
        // A Rhythm Channel: All Notes Off, no Mode 4 by default.
        assert_eq!(
            r.receive(&cc(9, 126, 1)),
            ModeAction::AllNotesOff {
                channel: Some(ch(9))
            }
        );
        assert!(!r.is_mono(ch(9)));
        let mut r = GmReceiver::new(GmConfig::new(GmConfig::DEFAULT.rhythm_capable, true));
        assert_eq!(r.receive(&cc(9, 126, 1)), change);
        assert!(r.is_mono(ch(9)));
    }

    #[test]
    fn rhythm_channel_table_follows_rp_024() {
        // REQ-GMR-009: each "Rhythm Channels shall not respond" sentence of sections 3 and 4.7.
        for c in 0..=127u8 {
            let listed = c == 1 || (64..=67).contains(&c) || (71..=78).contains(&c);
            assert_eq!(rhythm_ignores_controller(c), listed, "controller {c}");
            assert_eq!(
                rhythm_ignores_message(&cc(9, c, 0)),
                listed,
                "controller {c}"
            );
        }
        for (rpn, listed) in [
            (Rpn::PitchBendSensitivity, true),
            (Rpn::FineTuning, true),
            (Rpn::CoarseTuning, true),
            (Rpn::ModulationDepthRange, true),
            (Rpn::TuningProgramSelect, false),
            (Rpn::TuningBankSelect, false),
            (Rpn::Null, false),
        ] {
            assert_eq!(
                rhythm_ignores_parameter(Parameter::Registered(rpn)),
                listed,
                "{rpn:?}"
            );
        }
        assert!(!rhythm_ignores_parameter(Parameter::NonRegistered {
            msb: 0,
            lsb: 0
        }));
        let pb = msg([0xE0, 0, 0x40]);
        let cp = msg([0xD0, 0, 0]);
        let note = msg([0x90, 60, 64]);
        assert!(rhythm_ignores_message(&pb) && rhythm_ignores_message(&cp));
        assert!(!rhythm_ignores_message(&note));
    }
}
