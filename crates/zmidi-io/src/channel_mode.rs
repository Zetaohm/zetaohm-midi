//! MIDI 1.0 receiver mode state for one virtual instrument (M1 v4.2.1 Channel Modes pp. 6 to 8,
//! Channel Mode Messages pp. 20 to 26, Global Controllers p. 12; DEC-022).
//!
//! - [`ModeState::receive`] takes a typed MIDI 1.0 Channel Voice message (DIN bytes, USB-MIDI 1.0,
//!   BLE-MIDI or a Message Type 0x2 UMP) and gives one [`ModeAction`].
//! - The mode comes from two flags, Omni and Poly/Mono (p. 23: "A receiver should set individual
//!   flags"), with M of the last Mono On. A mode that the instrument does not implement gives the
//!   alternate mode of the table on p. 23 (recommended), or the Mode message is ignored, as the
//!   [`ModeConfig`] says.
//! - Where the specification leaves a choice to the receiver, the state reports a fact: the mode
//!   changed (a repeated Mode message, AMB-M1-27), or the mode is an alternate mode (the
//!   continuous controllers of p. 23, AMB-M1-10).
//! - Mode 4: All Sound Off, Reset All Controllers and All Notes Off act on the channel where they
//!   arrive (p. 25, AMB-M1-03, AMB-M1-04).
//! - Multi Mode (p. 7): one state for each virtual instrument.
//!
//! No alloc (DEC-002), no I/O (DEC-003). Never panics.

use zmidi_ump::{Channel, ChannelModeMessage, Midi1ChannelVoiceMessage};

/// The receive mode (p. 7, p. 20).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReceiveMode {
    /// Mode 1: Omni On, Poly.
    Mode1,
    /// Mode 2: Omni On, Mono.
    Mode2,
    /// Mode 3: Omni Off, Poly.
    Mode3,
    /// Mode 4: Omni Off, Mono.
    Mode4,
}

impl ReceiveMode {
    /// The mode of the Omni and Mono flags (p. 20).
    pub const fn from_flags(omni: bool, mono: bool) -> Self {
        match (omni, mono) {
            (true, false) => Self::Mode1,
            (true, true) => Self::Mode2,
            (false, false) => Self::Mode3,
            (false, true) => Self::Mode4,
        }
    }

    /// The mode number, 1 to 4.
    pub const fn number(self) -> u8 {
        match self {
            Self::Mode1 => 1,
            Self::Mode2 => 2,
            Self::Mode3 => 3,
            Self::Mode4 => 4,
        }
    }
}

/// The fixed properties of one virtual instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModeConfig {
    /// The Basic Channel N; only the caller changes it (p. 20).
    pub basic_channel: Channel,
    /// The implemented modes, index 0 for Mode 1. Mode 1 is always implemented (AMB-M1-09).
    pub implemented: [bool; 4],
    /// For a requested mode that is not implemented: `true` uses the alternate mode of p. 23
    /// (recommended), `false` ignores the Mode message (p. 7).
    pub alternate: bool,
    /// The voice count V: Mono On with M = 0 uses channels N to N+V-1 (p. 22, AMB-M1-07).
    pub voices: u8,
    /// Mode 4 Global Controllers: a Control Change on the channel below N acts on all voices
    /// (p. 12).
    pub global_controllers: bool,
}

impl Default for ModeConfig {
    /// Basic Channel 1, every mode implemented, the alternate mode, 16 voices, no Global
    /// Controllers (p. 8).
    fn default() -> Self {
        Self {
            basic_channel: Channel::from_bits(0),
            implemented: [true; 4],
            alternate: true,
            voices: 16,
            global_controllers: false,
        }
    }
}

/// What the instrument does with one message ([`ModeState::receive`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModeAction {
    /// The message is not for the instrument: another channel, a Mode message off the Basic
    /// Channel, All Notes Off or Reset All Controllers while Omni is on, or a mode that is not
    /// implemented with the ignore policy.
    Ignore,
    /// A Voice message for the instrument. `voice` is the Mode 4 voice (0 for the voice of the
    /// Basic Channel); `None` in Modes 1 to 3.
    Voice {
        /// Mode 4 voice, channel minus N.
        voice: Option<u8>,
    },
    /// A Mode 4 Global Controller: a Control Change for all voices (p. 12).
    Global,
    /// All Notes Off (123). `channel`: `None` for every note of the instrument, or in Mode 4 the
    /// one channel whose notes go off.
    AllNotesOff {
        /// Mode 4: the channel of the message.
        channel: Option<Channel>,
    },
    /// All Sound Off (120), with the same `channel` rule as [`ModeAction::AllNotesOff`].
    AllSoundOff {
        /// Mode 4: the channel of the message.
        channel: Option<Channel>,
    },
    /// Reset All Controllers (121), with the same `channel` rule as [`ModeAction::AllNotesOff`].
    ResetAllControllers {
        /// Mode 4: the channel of the message.
        channel: Option<Channel>,
    },
    /// Local Control (122): the new Local state.
    LocalControl {
        /// `true` for Local On.
        on: bool,
    },
    /// Omni Off, Omni On, Mono On or Poly On (124 to 127): also All Notes Off for every note of
    /// the instrument (p. 20, Table IV).
    ModeChange {
        /// The mode, or its Mode 4 channel range, changed (Appendix p. A-4). `false` for a
        /// repeated message, which the receiver can skip (p. 23).
        changed: bool,
        /// The mode is an alternate mode for a requested mode that is not implemented (p. 23).
        alternate: bool,
    },
}

/// The receive mode state of one virtual instrument (REQ-MODE-001 to REQ-MODE-010). Fixed size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModeState {
    config: ModeConfig,
    omni: bool,
    mono: bool,
    /// M of the last Mono On, 0 to 127; kept while Omni is on (AMB-M1-08).
    m: u8,
    local: bool,
}

impl Default for ModeState {
    /// [`ModeState::new`] with the default [`ModeConfig`].
    fn default() -> Self {
        Self::new(ModeConfig {
            basic_channel: Channel::from_bits(0),
            implemented: [true; 4],
            alternate: true,
            voices: 16,
            global_controllers: false,
        })
    }
}

impl ModeState {
    /// A state in Mode 1 (Omni On, Poly) with Local On (pp. 8 and 26).
    pub const fn new(config: ModeConfig) -> Self {
        Self {
            config,
            omni: true,
            mono: false,
            m: 0,
            local: true,
        }
    }

    /// The System Reset action: Omni On, Poly, Local On; the Basic Channel stays (p. 33, items 1
    /// and 2; AMB-M1-12).
    pub fn system_reset(&mut self) {
        self.omni = true;
        self.mono = false;
        self.local = true;
    }

    /// The mode in use: the mode of the flags, or its alternate mode (p. 23).
    pub fn mode(&self) -> ReceiveMode {
        self.mode_for(self.omni, self.mono, self.m)
    }

    /// The mode of the flags (p. 20).
    pub fn requested_mode(&self) -> ReceiveMode {
        ReceiveMode::from_flags(self.omni, self.mono)
    }

    /// `true` while the mode in use is an alternate mode for a requested mode that is not
    /// implemented (p. 23). The application selects the messages it ignores then (AMB-M1-10).
    pub fn is_alternate(&self) -> bool {
        self.mode() != self.requested_mode()
    }

    /// The Basic Channel N.
    pub fn basic_channel(&self) -> Channel {
        self.config.basic_channel
    }

    /// Sets the Basic Channel (panel control or System Exclusive, p. 20).
    pub fn set_basic_channel(&mut self, channel: Channel) {
        self.config.basic_channel = channel;
    }

    /// M of the last Mono On (or of [`Self::set_mono`]).
    pub fn m(&self) -> u8 {
        self.m
    }

    /// `true` for Local On.
    pub fn local(&self) -> bool {
        self.local
    }

    /// Sets Omni (panel control, p. 21).
    pub fn set_omni(&mut self, on: bool) {
        self.omni = on;
    }

    /// Sets Poly (panel control).
    pub fn set_poly(&mut self) {
        self.mono = false;
    }

    /// Sets Mono with M (panel control).
    pub fn set_mono(&mut self, m: u8) {
        self.mono = true;
        self.m = m;
    }

    /// Sets Local (panel control).
    pub fn set_local(&mut self, on: bool) {
        self.local = on;
    }

    /// Takes one message and gives its action (REQ-MODE-004 to REQ-MODE-010). Never panics.
    pub fn receive(&mut self, message: &Midi1ChannelVoiceMessage) -> ModeAction {
        let channel = ((message.into_bits() >> 16) & 0xF) as u8;
        let mode = self.mode();
        if let Midi1ChannelVoiceMessage::ControlChange(cc) = message {
            if let Some(m) = cc.channel_mode_message() {
                return self.mode_message(m, channel, mode);
            }
        }
        match mode {
            ReceiveMode::Mode1 | ReceiveMode::Mode2 => ModeAction::Voice { voice: None },
            ReceiveMode::Mode3 if channel == self.config.basic_channel.into_bits() => {
                ModeAction::Voice { voice: None }
            }
            ReceiveMode::Mode3 => ModeAction::Ignore,
            ReceiveMode::Mode4 => match self.voice(channel) {
                Some(voice) => ModeAction::Voice { voice: Some(voice) },
                None if self.global(channel, message) => ModeAction::Global,
                None => ModeAction::Ignore,
            },
        }
    }

    /// The action of a Channel Mode message on `channel` (0 to 15) in `mode`.
    fn mode_message(
        &mut self,
        m: ChannelModeMessage,
        channel: u8,
        mode: ReceiveMode,
    ) -> ModeAction {
        let basic = channel == self.config.basic_channel.into_bits();
        match m {
            ChannelModeMessage::AllSoundOff
            | ChannelModeMessage::ResetAllControllers
            | ChannelModeMessage::AllNotesOff => {
                // Mode 4: each channel of the instrument, for that channel (AMB-M1-03, -04).
                let scope = if mode == ReceiveMode::Mode4 {
                    if !basic && self.voice(channel).is_none() {
                        return ModeAction::Ignore;
                    }
                    Some(Channel::from_bits(channel))
                } else if basic {
                    None
                } else {
                    return ModeAction::Ignore;
                };
                let omni = matches!(mode, ReceiveMode::Mode1 | ReceiveMode::Mode2);
                match m {
                    ChannelModeMessage::AllSoundOff => ModeAction::AllSoundOff { channel: scope },
                    // p. 25: ignore if Omni is on.
                    _ if omni => ModeAction::Ignore,
                    ChannelModeMessage::ResetAllControllers => {
                        ModeAction::ResetAllControllers { channel: scope }
                    }
                    _ => ModeAction::AllNotesOff { channel: scope },
                }
            }
            _ if !basic => ModeAction::Ignore,
            ChannelModeMessage::LocalControl(on) => {
                self.local = on;
                ModeAction::LocalControl { on }
            }
            ChannelModeMessage::OmniOff => self.change(false, self.mono, self.m),
            ChannelModeMessage::OmniOn => self.change(true, self.mono, self.m),
            ChannelModeMessage::MonoOn(m) => self.change(self.omni, true, m),
            ChannelModeMessage::PolyOn => self.change(self.omni, false, self.m),
        }
    }

    /// Sets the flags of a Mode message 124 to 127, or ignores it (p. 7, p. 23).
    fn change(&mut self, omni: bool, mono: bool, m: u8) -> ModeAction {
        if !self.config.alternate && !self.implemented(ReceiveMode::from_flags(omni, mono)) {
            return ModeAction::Ignore;
        }
        let before = self.range();
        (self.omni, self.mono, self.m) = (omni, mono, m);
        ModeAction::ModeChange {
            changed: self.range() != before,
            alternate: self.is_alternate(),
        }
    }

    /// The mode in use with its Mode 4 channel count (0 in Modes 1 to 3).
    fn range(&self) -> (ReceiveMode, u8) {
        let mode = self.mode();
        let count = if mode == ReceiveMode::Mode4 {
            self.count()
        } else {
            0
        };
        (mode, count)
    }

    /// The mode in use for these flags: the alternate mode of p. 23 when the instrument does not
    /// implement it (AMB-M1-09).
    fn mode_for(&self, omni: bool, mono: bool, m: u8) -> ReceiveMode {
        let requested = ReceiveMode::from_flags(omni, mono);
        if self.implemented(requested) {
            return requested;
        }
        let table = if requested == ReceiveMode::Mode4 && m == 1 {
            ReceiveMode::Mode2
        } else {
            ReceiveMode::Mode1
        };
        if self.implemented(table) {
            table
        } else {
            ReceiveMode::Mode1
        }
    }

    fn implemented(&self, mode: ReceiveMode) -> bool {
        match mode {
            ReceiveMode::Mode1 => true,
            ReceiveMode::Mode2 => self.config.implemented[1],
            ReceiveMode::Mode3 => self.config.implemented[2],
            ReceiveMode::Mode4 => self.config.implemented[3],
        }
    }

    /// The Mode 4 channel count C: M, or V for M = 0; at most 16 (AMB-M1-07).
    fn count(&self) -> u8 {
        let c = if self.m == 0 {
            self.config.voices
        } else {
            self.m
        };
        c.min(16)
    }

    /// The Mode 4 voice of `channel` (0 to 15): channel minus N, when it is below C.
    fn voice(&self, channel: u8) -> Option<u8> {
        let voice = channel.checked_sub(self.config.basic_channel.into_bits())?;
        (voice < self.count()).then_some(voice)
    }

    /// A Mode 4 Global Controller: a Control Change on the channel below N (16 for N = 1),
    /// outside the voice channels (p. 12, AMB-M1-11).
    fn global(&self, channel: u8, message: &Midi1ChannelVoiceMessage) -> bool {
        let below = (self.config.basic_channel.into_bits() + 15) % 16;
        self.config.global_controllers
            && channel == below
            && matches!(message, Midi1ChannelVoiceMessage::ControlChange(_))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;
    use zmidi_ump::{Group, Midi1ChannelPressure, Midi1ControlChange, Midi1NoteOn, Midi1PitchBend};

    fn ch(n: u8) -> Channel {
        Channel::from_bits(n)
    }

    fn cc(c: u8, index: u8, value: u8) -> Midi1ChannelVoiceMessage {
        Midi1ChannelVoiceMessage::ControlChange(
            Midi1ControlChange::try_new(Group::from_bits(0), ch(c), index, value).unwrap(),
        )
    }

    fn note(c: u8) -> Midi1ChannelVoiceMessage {
        Midi1ChannelVoiceMessage::NoteOn(
            Midi1NoteOn::try_new(Group::from_bits(0), ch(c), 60, 100).unwrap(),
        )
    }

    fn config(n: u8) -> ModeConfig {
        ModeConfig {
            basic_channel: ch(n),
            ..ModeConfig::default()
        }
    }

    const OMNI_OFF: u8 = 124;
    const OMNI_ON: u8 = 125;
    const MONO: u8 = 126;
    const POLY: u8 = 127;

    fn change(changed: bool, alternate: bool) -> ModeAction {
        ModeAction::ModeChange { changed, alternate }
    }

    #[test]
    fn mode_follows_the_two_flags_in_any_order() {
        // p. 20 and p. 23: two flags; Mono clears Poly; any order gives the same mode.
        let mut s = ModeState::default();
        assert_eq!(s.mode(), ReceiveMode::Mode1);
        assert_eq!(s.receive(&cc(0, OMNI_OFF, 0)), change(true, false));
        assert_eq!(s.mode(), ReceiveMode::Mode3);
        assert_eq!(s.receive(&cc(0, MONO, 4)), change(true, false));
        assert_eq!((s.mode(), s.m()), (ReceiveMode::Mode4, 4));
        let mut t = ModeState::default();
        t.receive(&cc(0, MONO, 4));
        assert_eq!(t.mode(), ReceiveMode::Mode2);
        t.receive(&cc(0, OMNI_OFF, 0));
        assert_eq!((t.mode(), t.m()), (ReceiveMode::Mode4, 4));
        // A repeat is accepted and changes nothing.
        assert_eq!(t.receive(&cc(0, OMNI_OFF, 0)), change(false, false));
        assert_eq!(t.receive(&cc(0, POLY, 0)), change(true, false));
        assert_eq!(t.mode(), ReceiveMode::Mode3);
        assert_eq!(t.receive(&cc(0, OMNI_ON, 0)), change(true, false));
        assert_eq!(t.mode(), ReceiveMode::Mode1);
        for (omni, mono, n) in [
            (true, false, 1),
            (true, true, 2),
            (false, false, 3),
            (false, true, 4),
        ] {
            assert_eq!(ReceiveMode::from_flags(omni, mono).number(), n);
        }
    }

    #[test]
    fn mode_state_starts_in_mode_1_with_local_on() {
        // p. 8 and p. 26: Basic Channel 1, Mode 1, Local On.
        let s = ModeState::default();
        assert_eq!(
            (s.mode(), s.local(), s.basic_channel()),
            (ReceiveMode::Mode1, true, ch(0))
        );
        assert!(!s.is_alternate());
        assert_eq!(ModeState::new(config(5)).basic_channel(), ch(5));
    }

    #[test]
    fn system_reset_keeps_the_basic_channel() {
        // p. 33, items 1 and 2; AMB-M1-12.
        let mut s = ModeState::new(config(3));
        s.receive(&cc(3, OMNI_OFF, 0));
        s.receive(&cc(3, MONO, 2));
        s.receive(&cc(3, 122, 0));
        assert_eq!((s.mode(), s.local()), (ReceiveMode::Mode4, false));
        s.system_reset();
        assert_eq!(
            (s.mode(), s.local(), s.basic_channel()),
            (ReceiveMode::Mode1, true, ch(3))
        );
    }

    #[test]
    fn panel_setters_change_the_mode() {
        // p. 20 and p. 21: the Basic Channel and the mode from the panel; no message changes N.
        let mut s = ModeState::default();
        s.set_basic_channel(ch(9));
        s.set_omni(false);
        assert_eq!(s.mode(), ReceiveMode::Mode3);
        s.set_mono(3);
        assert_eq!((s.mode(), s.m()), (ReceiveMode::Mode4, 3));
        s.set_poly();
        assert_eq!(s.mode(), ReceiveMode::Mode3);
        s.set_local(false);
        assert!(!s.local());
        for m in 0..=127 {
            s.receive(&cc(9, m, 127));
            assert_eq!(s.basic_channel(), ch(9));
        }
    }

    #[test]
    fn voice_messages_follow_the_mode() {
        // p. 7: Modes 1 and 2 all channels; Mode 3 channel N only.
        let mut s = ModeState::new(config(2));
        for c in 0..16 {
            assert_eq!(s.receive(&note(c)), ModeAction::Voice { voice: None });
        }
        s.receive(&cc(2, MONO, 1));
        assert_eq!(s.mode(), ReceiveMode::Mode2);
        assert_eq!(s.receive(&note(15)), ModeAction::Voice { voice: None });
        s.receive(&cc(2, POLY, 0));
        s.receive(&cc(2, OMNI_OFF, 0));
        assert_eq!(s.mode(), ReceiveMode::Mode3);
        for c in 0..16 {
            let want = if c == 2 {
                ModeAction::Voice { voice: None }
            } else {
                ModeAction::Ignore
            };
            assert_eq!(s.receive(&note(c)), want);
        }
    }

    #[test]
    fn mode_4_channel_range_and_voice() {
        // p. 22: N to N+M-1, no channel above 16; M = 0 uses V; M above 16 acts as 16.
        let mut s = ModeState::new(ModeConfig {
            basic_channel: ch(2),
            voices: 6,
            ..ModeConfig::default()
        });
        s.receive(&cc(2, OMNI_OFF, 0));
        s.receive(&cc(2, MONO, 4));
        let got: Vec<_> = (0..16).map(|c| s.receive(&note(c))).collect();
        for (c, a) in got.iter().enumerate() {
            let want = match c {
                2..=5 => ModeAction::Voice {
                    voice: Some(c as u8 - 2),
                },
                _ => ModeAction::Ignore,
            };
            assert_eq!(*a, want, "channel {c}");
        }
        s.receive(&cc(2, MONO, 0));
        assert_eq!(s.receive(&note(7)), ModeAction::Voice { voice: Some(5) });
        assert_eq!(s.receive(&note(8)), ModeAction::Ignore);
        s.receive(&cc(2, MONO, 100));
        assert_eq!(s.receive(&note(15)), ModeAction::Voice { voice: Some(13) });
        assert_eq!(s.receive(&note(1)), ModeAction::Ignore);
    }

    #[test]
    fn prop_voice_acceptance_matches_the_mode_table() {
        // p. 7 table: which channels each mode receives, for any N, M and V.
        proptest!(|(n in 0u8..16, m in 0u8..=127, v in 0u8..=16, omni in any::<bool>(),
                    mono in any::<bool>(), c in 0u8..16)| {
            let mut s = ModeState::new(ModeConfig { basic_channel: ch(n), voices: v, ..ModeConfig::default() });
            s.set_omni(omni);
            if mono { s.set_mono(m) } else { s.set_poly() }
            let count = if m == 0 { v } else { m.min(16) };
            let want = match (omni, mono) {
                (true, _) => ModeAction::Voice { voice: None },
                (false, false) if c == n => ModeAction::Voice { voice: None },
                (false, true) if c >= n && c - n < count => ModeAction::Voice { voice: Some(c - n) },
                _ => ModeAction::Ignore,
            };
            prop_assert_eq!(s.receive(&note(c)), want);
        });
    }

    #[test]
    fn mode_messages_only_on_the_basic_channel() {
        // p. 7 and p. 20: Mode messages only on N, in every mode.
        let mut s = ModeState::new(config(4));
        for index in 120..=127 {
            assert_eq!(s.receive(&cc(5, index, 0)), ModeAction::Ignore, "{index}");
        }
        assert_eq!(s.mode(), ReceiveMode::Mode1);
        assert_eq!(
            s.receive(&cc(4, 122, 0)),
            ModeAction::LocalControl { on: false }
        );
        s.receive(&cc(4, OMNI_OFF, 0));
        s.receive(&cc(4, MONO, 3));
        // Mode 4: 122 and 124 to 127 stay Basic Channel only.
        assert_eq!(s.receive(&cc(5, 122, 127)), ModeAction::Ignore);
        assert_eq!(s.receive(&cc(5, POLY, 0)), ModeAction::Ignore);
        assert_eq!(s.mode(), ReceiveMode::Mode4);
    }

    #[test]
    fn mode_4_note_off_messages_act_on_their_channel() {
        // p. 25 (AMB-M1-03, AMB-M1-04): in Mode 4, 120, 121 and 123 on each channel N to N+C-1.
        let mut s = ModeState::new(config(4));
        s.receive(&cc(4, OMNI_OFF, 0));
        s.receive(&cc(4, MONO, 3));
        for c in [4, 5, 6] {
            let scope = Some(ch(c));
            assert_eq!(
                s.receive(&cc(c, 120, 0)),
                ModeAction::AllSoundOff { channel: scope }
            );
            assert_eq!(
                s.receive(&cc(c, 121, 0)),
                ModeAction::ResetAllControllers { channel: scope }
            );
            assert_eq!(
                s.receive(&cc(c, 123, 0)),
                ModeAction::AllNotesOff { channel: scope }
            );
        }
        for c in [3, 7] {
            for index in [120, 121, 123] {
                assert_eq!(s.receive(&cc(c, index, 0)), ModeAction::Ignore);
            }
        }
    }

    #[test]
    fn omni_rule_for_all_notes_off_and_reset() {
        // p. 25: ignore 123 and 121 while Omni is on (Modes 1 and 2); Mode 3 for the instrument.
        let mut s = ModeState::new(config(0));
        assert_eq!(s.receive(&cc(0, 123, 0)), ModeAction::Ignore);
        assert_eq!(s.receive(&cc(0, 121, 0)), ModeAction::Ignore);
        s.receive(&cc(0, MONO, 1));
        assert_eq!(s.receive(&cc(0, 123, 0)), ModeAction::Ignore);
        s.receive(&cc(0, POLY, 0));
        s.receive(&cc(0, OMNI_OFF, 0));
        assert_eq!(
            s.receive(&cc(0, 123, 0)),
            ModeAction::AllNotesOff { channel: None }
        );
        assert_eq!(
            s.receive(&cc(0, 121, 0)),
            ModeAction::ResetAllControllers { channel: None }
        );
        assert_eq!(s.receive(&cc(1, 123, 0)), ModeAction::Ignore);
        // RP-015: Reset All Controllers keeps the mode and Local Control (AMB-RP-015-02).
        s.receive(&cc(0, 122, 0));
        s.receive(&cc(0, 121, 0));
        assert_eq!((s.mode(), s.local()), (ReceiveMode::Mode3, false));
    }

    #[test]
    fn all_sound_off_in_every_mode() {
        // p. 25: All Sound Off has no Omni rule (AMB-M1-04).
        let mut s = ModeState::new(config(0));
        assert_eq!(
            s.receive(&cc(0, 120, 0)),
            ModeAction::AllSoundOff { channel: None }
        );
        s.receive(&cc(0, MONO, 1));
        assert_eq!(
            s.receive(&cc(0, 120, 0)),
            ModeAction::AllSoundOff { channel: None }
        );
        s.receive(&cc(0, OMNI_OFF, 0));
        assert_eq!(s.mode(), ReceiveMode::Mode4);
        assert_eq!(
            s.receive(&cc(0, 120, 0)),
            ModeAction::AllSoundOff {
                channel: Some(ch(0))
            }
        );
    }

    #[test]
    fn mode_messages_turn_notes_off_with_facts() {
        // p. 20, Table IV, A-4 (AMB-M1-27): each accepted 124 to 127 is a mode change with All
        // Notes Off, also while Omni is on and for a repeat; `changed` tells a repeat.
        let mut s = ModeState::default();
        assert_eq!(s.receive(&cc(0, OMNI_ON, 0)), change(false, false));
        assert_eq!(s.receive(&cc(0, POLY, 0)), change(false, false));
        assert_eq!(s.receive(&cc(0, MONO, 3)), change(true, false));
        // In Mode 2, a new M changes nothing that the state uses.
        assert_eq!(s.receive(&cc(0, MONO, 5)), change(false, false));
        s.receive(&cc(0, OMNI_OFF, 0));
        // In Mode 4, a new M changes the channel range.
        assert_eq!(s.receive(&cc(0, MONO, 2)), change(true, false));
        assert_eq!(s.receive(&cc(0, MONO, 2)), change(false, false));
    }

    #[test]
    fn mono_on_stores_m_while_omni_is_on() {
        // p. 23 (AMB-M1-08): M is kept from Mono On in Mode 2 for a later Omni Off.
        let mut s = ModeState::default();
        s.receive(&cc(0, MONO, 4));
        assert_eq!((s.mode(), s.m()), (ReceiveMode::Mode2, 4));
        s.receive(&cc(0, OMNI_OFF, 0));
        assert_eq!(s.receive(&note(3)), ModeAction::Voice { voice: Some(3) });
        assert_eq!(s.receive(&note(4)), ModeAction::Ignore);
    }

    #[test]
    fn alternate_mode_follows_the_page_24_vector() {
        // p. 24 figure: Mode 4 not implemented; power-up, Omni Off, Mono M >= 2, Omni Off, Poly
        // give Modes 1, 3, 1, 1, 3.
        let mut s = ModeState::new(ModeConfig {
            implemented: [true, true, true, false],
            ..ModeConfig::default()
        });
        let mut modes = std::vec![s.mode().number()];
        assert_eq!(s.receive(&cc(0, OMNI_OFF, 0)), change(true, false));
        modes.push(s.mode().number());
        assert_eq!(s.receive(&cc(0, MONO, 2)), change(true, true));
        modes.push(s.mode().number());
        assert_eq!(s.receive(&cc(0, OMNI_OFF, 0)), change(false, true));
        modes.push(s.mode().number());
        assert_eq!(s.receive(&cc(0, POLY, 0)), change(true, false));
        modes.push(s.mode().number());
        assert_eq!(modes, [1, 3, 1, 1, 3]);
    }

    #[test]
    fn alternate_mode_table_for_mode_4() {
        // p. 23 table (AMB-M1-09): no Mode 4: M = 1 uses Mode 2, another M Mode 1; without
        // Mode 2, Mode 1.
        let mut s = ModeState::new(ModeConfig {
            implemented: [true, true, true, false],
            ..ModeConfig::default()
        });
        s.set_omni(false);
        s.set_mono(1);
        assert_eq!(
            (s.requested_mode(), s.mode()),
            (ReceiveMode::Mode4, ReceiveMode::Mode2)
        );
        assert!(s.is_alternate());
        // In the alternate Mode 2 the instrument receives every channel; Omni is on in effect.
        assert_eq!(s.receive(&note(9)), ModeAction::Voice { voice: None });
        assert_eq!(s.receive(&cc(0, 123, 0)), ModeAction::Ignore);
        s.set_mono(0);
        assert_eq!(s.mode(), ReceiveMode::Mode1);
        let mut s = ModeState::new(ModeConfig {
            implemented: [true, false, true, false],
            ..ModeConfig::default()
        });
        s.set_omni(false);
        s.set_mono(1);
        assert_eq!(s.mode(), ReceiveMode::Mode1);
        // Mode 1 is always implemented.
        let mut s = ModeState::new(ModeConfig {
            implemented: [false; 4],
            ..ModeConfig::default()
        });
        s.set_omni(false);
        assert_eq!(s.mode(), ReceiveMode::Mode1);
    }

    #[test]
    fn ignore_policy_keeps_the_mode() {
        // p. 7: a receiver can ignore a Mode message for a mode that it does not implement.
        let mut s = ModeState::new(ModeConfig {
            implemented: [true, false, true, false],
            alternate: false,
            ..ModeConfig::default()
        });
        assert_eq!(s.receive(&cc(0, OMNI_OFF, 0)), change(true, false));
        assert_eq!(s.receive(&cc(0, MONO, 2)), ModeAction::Ignore);
        assert_eq!(
            (s.mode(), s.requested_mode()),
            (ReceiveMode::Mode3, ReceiveMode::Mode3)
        );
        assert_eq!(s.receive(&cc(0, OMNI_ON, 0)), change(true, false));
        assert_eq!(s.mode(), ReceiveMode::Mode1);
    }

    #[test]
    fn global_controllers_in_mode_4() {
        // p. 12 (AMB-M1-11): channel N-1 (16 for N = 1), Control Change only, outside N..N+C-1.
        let pb = |c| {
            Midi1ChannelVoiceMessage::PitchBend(
                Midi1PitchBend::try_new(Group::from_bits(0), ch(c), 0x2000).unwrap(),
            )
        };
        let at = |c| {
            Midi1ChannelVoiceMessage::ChannelPressure(
                Midi1ChannelPressure::try_new(Group::from_bits(0), ch(c), 9).unwrap(),
            )
        };
        let mut s = ModeState::new(ModeConfig {
            basic_channel: ch(3),
            global_controllers: true,
            ..ModeConfig::default()
        });
        s.receive(&cc(3, OMNI_OFF, 0));
        s.receive(&cc(3, MONO, 4));
        assert_eq!(s.receive(&cc(2, 7, 100)), ModeAction::Global);
        assert_eq!(s.receive(&pb(2)), ModeAction::Ignore);
        assert_eq!(s.receive(&at(2)), ModeAction::Ignore);
        assert_eq!(s.receive(&cc(1, 7, 100)), ModeAction::Ignore);
        // N = 1: the Global Channel is 16, unless channel 16 is a voice channel.
        let mut s = ModeState::new(ModeConfig {
            global_controllers: true,
            ..ModeConfig::default()
        });
        s.receive(&cc(0, OMNI_OFF, 0));
        s.receive(&cc(0, MONO, 4));
        assert_eq!(s.receive(&cc(15, 1, 64)), ModeAction::Global);
        s.receive(&cc(0, MONO, 16));
        assert_eq!(
            s.receive(&cc(15, 1, 64)),
            ModeAction::Voice { voice: Some(15) }
        );
        // Off by default.
        let mut s = ModeState::default();
        s.receive(&cc(0, OMNI_OFF, 0));
        s.receive(&cc(0, MONO, 4));
        assert_eq!(s.receive(&cc(15, 1, 64)), ModeAction::Ignore);
    }

    #[test]
    fn local_control_sets_the_local_flag() {
        // p. 26 and the switch rule of p. 12 (AMB-M1-24).
        let mut s = ModeState::default();
        for (value, on) in [(0, false), (63, false), (64, true), (127, true), (1, false)] {
            assert_eq!(
                s.receive(&cc(0, 122, value)),
                ModeAction::LocalControl { on }
            );
            assert_eq!(s.local(), on);
        }
    }
}
