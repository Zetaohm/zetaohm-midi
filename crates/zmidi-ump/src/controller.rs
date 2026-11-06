//! The meanings of MIDI 1.0 controller numbers and Registered Parameter Numbers (M1 v4.2.1 Control
//! Change pp. 11 to 18, Table III p. T-3, Table IIIa p. T-4). No state.
//!
//! - [`Controller`] names each controller number of Table III; a number with no function is
//!   [`Controller::Undefined`] (REQ-CTL-001). [`ControllerClass`] gives the range of a number:
//!   MSB of a 14-bit pair (0 to 31), its LSB (32 to 63), single-byte (64 to 95), Data Increment,
//!   Data Decrement and the parameter numbers (96 to 101), undefined (102 to 119) and Channel Mode
//!   (120 to 127) (REQ-CTL-002).
//! - A switch value is off from 0 to 63 and on from 64 to 127 (p. 12, REQ-CTL-003); a value with a
//!   center reads as its offset from 64 (REQ-CTL-004).
//! - [`Rpn`] names the Registered Parameter Numbers of Table IIIa, RPN 00H 05H of GM2 (RP-024 3.4.4)
//!   and the null value 7FH 7FH (RP-015, RP-018, M2-104 D.3.3; REQ-RPN-001); the functions give
//!   their values in units (REQ-RPN-002 to REQ-RPN-005).
//! - GM2 (RP-024 3.3.15 to 3.3.20) gives default names to Sound Controllers 6 to 9 and to Effects 1
//!   and 3 Depth; the variants keep the M1 names (REQ-CTL-005).
//!
//! The module is behind the feature `channel-voice1` (DEC-022): the meanings stay with their message
//! family. These meanings hold for MIDI 1.0 bytes and for Message Type 0x2 UMPs.

/// A controller number of a MIDI 1.0 Control Change (M1 Table III), 0 to 127.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Controller {
    /// 0: Bank Select MSB; its LSB is 32 (p. 13, a 14-bit bank).
    BankSelect,
    /// 1: Modulation wheel or lever.
    ModulationWheel,
    /// 2: Breath Controller.
    BreathController,
    /// 4: Foot controller.
    FootController,
    /// 5: Portamento time.
    PortamentoTime,
    /// 6: Data entry MSB.
    DataEntry,
    /// 7: Channel Volume (formerly Main Volume).
    ChannelVolume,
    /// 8: Balance, center 64 (p. 13).
    Balance,
    /// 10: Pan, center 64 (p. 13).
    Pan,
    /// 11: Expression Controller.
    Expression,
    /// 12: Effect Control 1.
    EffectControl1,
    /// 13: Effect Control 2.
    EffectControl2,
    /// General Purpose Controller 1 to 8: numbers 16 to 19 are 1 to 4, numbers 80 to 83 are 5 to 8.
    GeneralPurpose(u8),
    /// 32 to 63: the LSB of controller 0 to 31; the field is that MSB controller number.
    Lsb(u8),
    /// 64: Damper pedal (sustain), a switch.
    DamperPedal,
    /// 65: Portamento On/Off, a switch.
    PortamentoOnOff,
    /// 66: Sostenuto, a switch.
    Sostenuto,
    /// 67: Soft pedal, a switch.
    SoftPedal,
    /// 68: Legato Footswitch: 00H to 3FH Normal, 40H to 7FH Legato ([`switch_on`]).
    LegatoFootswitch,
    /// 69: Hold 2.
    Hold2,
    /// Sound Controller 1 to 10 (numbers 70 to 79). Defaults of 1 to 5: Sound Variation,
    /// Timbre/Harmonic Intensity, Release Time, Attack Time, Brightness. M1 gives 6 to 10 no
    /// default; GM2 (RP-024 3.3.15 to 3.3.18) names 6 to 9 Decay Time (75), Vibrato Rate (76),
    /// Vibrato Depth (77) and Vibrato Delay (78).
    SoundController(u8),
    /// 84: Portamento Control; its value is a source note number.
    PortamentoControl,
    /// Effects 1 to 5 Depth (numbers 91 to 95; formerly External Effects, Tremolo, Chorus,
    /// Celeste (Detune) and Phaser Depth). GM2 (RP-024 3.3.19, 3.3.20) names Effects 1 Depth (91)
    /// Reverb Send Level and Effects 3 Depth (93) Chorus Send Level.
    EffectsDepth(u8),
    /// 96: Data increment.
    DataIncrement,
    /// 97: Data decrement.
    DataDecrement,
    /// 98: Non-Registered Parameter Number LSB.
    NrpnLsb,
    /// 99: Non-Registered Parameter Number MSB.
    NrpnMsb,
    /// 100: Registered Parameter Number LSB.
    RpnLsb,
    /// 101: Registered Parameter Number MSB.
    RpnMsb,
    /// 120 to 127: reserved for the Channel Mode messages; the field is the number.
    ChannelMode(u8),
    /// A number with no function in Table III (3, 9, 14, 15, 20 to 31, 85 to 90, 102 to 119).
    Undefined(u8),
}

impl Controller {
    /// The controller of `number`, or `None` above 127.
    pub const fn from_number(number: u8) -> Option<Self> {
        Some(match number {
            0 => Self::BankSelect,
            1 => Self::ModulationWheel,
            2 => Self::BreathController,
            4 => Self::FootController,
            5 => Self::PortamentoTime,
            6 => Self::DataEntry,
            7 => Self::ChannelVolume,
            8 => Self::Balance,
            10 => Self::Pan,
            11 => Self::Expression,
            12 => Self::EffectControl1,
            13 => Self::EffectControl2,
            16..=19 => Self::GeneralPurpose(number - 15),
            32..=63 => Self::Lsb(number - 32),
            64 => Self::DamperPedal,
            65 => Self::PortamentoOnOff,
            66 => Self::Sostenuto,
            67 => Self::SoftPedal,
            68 => Self::LegatoFootswitch,
            69 => Self::Hold2,
            70..=79 => Self::SoundController(number - 69),
            80..=83 => Self::GeneralPurpose(number - 75),
            84 => Self::PortamentoControl,
            91..=95 => Self::EffectsDepth(number - 90),
            96 => Self::DataIncrement,
            97 => Self::DataDecrement,
            98 => Self::NrpnLsb,
            99 => Self::NrpnMsb,
            100 => Self::RpnLsb,
            101 => Self::RpnMsb,
            120..=127 => Self::ChannelMode(number),
            128.. => return None,
            _ => Self::Undefined(number),
        })
    }

    /// The controller number, 0 to 127. A field out of its range (for example `GeneralPurpose(9)`)
    /// gives the number of the nearest valid field.
    pub const fn number(self) -> u8 {
        match self {
            Self::BankSelect => 0,
            Self::ModulationWheel => 1,
            Self::BreathController => 2,
            Self::FootController => 4,
            Self::PortamentoTime => 5,
            Self::DataEntry => 6,
            Self::ChannelVolume => 7,
            Self::Balance => 8,
            Self::Pan => 10,
            Self::Expression => 11,
            Self::EffectControl1 => 12,
            Self::EffectControl2 => 13,
            Self::GeneralPurpose(n) => match n {
                0..=4 => 15 + if n == 0 { 1 } else { n },
                5.. => 75 + if n > 8 { 8 } else { n },
            },
            Self::Lsb(msb) => 32 + if msb > 31 { 31 } else { msb },
            Self::DamperPedal => 64,
            Self::PortamentoOnOff => 65,
            Self::Sostenuto => 66,
            Self::SoftPedal => 67,
            Self::LegatoFootswitch => 68,
            Self::Hold2 => 69,
            Self::SoundController(n) => 69 + clamp(n, 1, 10),
            Self::PortamentoControl => 84,
            Self::EffectsDepth(n) => 90 + clamp(n, 1, 5),
            Self::DataIncrement => 96,
            Self::DataDecrement => 97,
            Self::NrpnLsb => 98,
            Self::NrpnMsb => 99,
            Self::RpnLsb => 100,
            Self::RpnMsb => 101,
            Self::ChannelMode(n) => clamp(n, 120, 127),
            Self::Undefined(n) => n & 0x7F,
        }
    }
}

const fn clamp(n: u8, min: u8, max: u8) -> u8 {
    if n < min {
        min
    } else if n > max {
        max
    } else {
        n
    }
}

/// The range of a controller number (M1 Control Change pp. 11, 12; Table III).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControllerClass {
    /// 0 to 31: the MSB of a 14-bit pair; its LSB is the number plus 32 ([`lsb_of`]).
    Msb,
    /// 32 to 63: the LSB of controller `msb` (the number minus 32).
    Lsb {
        /// The MSB controller number, 0 to 31.
        msb: u8,
    },
    /// 64 to 95: a single-byte controller (switches among them).
    SingleByte,
    /// 96 to 101: Data Increment, Data Decrement and the parameter numbers.
    Parameter,
    /// 102 to 119: undefined.
    Undefined,
    /// 120 to 127: a Channel Mode message.
    ChannelMode,
}

impl ControllerClass {
    /// The class of controller `number`, or `None` above 127.
    pub const fn of(number: u8) -> Option<Self> {
        Some(match number {
            0..=31 => Self::Msb,
            32..=63 => Self::Lsb { msb: number - 32 },
            64..=95 => Self::SingleByte,
            96..=101 => Self::Parameter,
            102..=119 => Self::Undefined,
            120..=127 => Self::ChannelMode,
            128.. => return None,
        })
    }
}

/// The LSB controller of MSB controller `msb` (number plus 32, Table III), or `None` when `msb` is
/// above 31.
pub const fn lsb_of(msb: u8) -> Option<u8> {
    if msb <= 31 {
        Some(msb + 32)
    } else {
        None
    }
}

/// The switch reading of a controller value: on from 64 (40H), off below (p. 12).
pub const fn switch_on(value: u8) -> bool {
    value & 0x7F >= 0x40
}

/// The signed offset of a controller value from the center 64 (40H): -64 to +63, for Balance, Pan
/// and the relative Sound Controllers (pp. 13 to 16).
pub const fn centered(value: u8) -> i8 {
    (value & 0x7F) as i8 - 64
}

/// A Registered Parameter Number (M1 Table IIIa), selected by controllers 101 (MSB) and 100 (LSB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rpn {
    /// 00H 00H: Pitch Bend Sensitivity ([`pitch_bend_sensitivity`]).
    PitchBendSensitivity,
    /// 00H 01H: Fine Tuning, named Channel Fine Tuning by RP-022 ([`fine_tuning`]).
    FineTuning,
    /// 00H 02H: Coarse Tuning, named Channel Coarse Tuning by RP-022 ([`coarse_tuning`]).
    CoarseTuning,
    /// 00H 03H: Tuning Program Select.
    TuningProgramSelect,
    /// 00H 04H: Tuning Bank Select.
    TuningBankSelect,
    /// 00H 05H: Modulation Depth Range of GM2, also Vibrato Depth Range (RP-024 3.4.4;
    /// [`modulation_depth_range`]).
    ModulationDepthRange,
    /// 7FH 7FH: the null value; no parameter is selected (RP-015, RP-018, M2-104 D.3.3).
    Null,
    /// Another number pair, 7 bits each.
    Other {
        /// Controller 101.
        msb: u8,
        /// Controller 100.
        lsb: u8,
    },
}

impl Rpn {
    /// The parameter of the controller 101 and 100 values (7 bits each; bit 7 is ignored).
    pub const fn from_parts(msb: u8, lsb: u8) -> Self {
        match (msb & 0x7F, lsb & 0x7F) {
            (0, 0) => Self::PitchBendSensitivity,
            (0, 1) => Self::FineTuning,
            (0, 2) => Self::CoarseTuning,
            (0, 3) => Self::TuningProgramSelect,
            (0, 4) => Self::TuningBankSelect,
            (0, 5) => Self::ModulationDepthRange,
            (0x7F, 0x7F) => Self::Null,
            (msb, lsb) => Self::Other { msb, lsb },
        }
    }

    /// The controller 101 and 100 values (MSB, LSB).
    pub const fn parts(self) -> (u8, u8) {
        match self {
            Self::PitchBendSensitivity => (0, 0),
            Self::FineTuning => (0, 1),
            Self::CoarseTuning => (0, 2),
            Self::TuningProgramSelect => (0, 3),
            Self::TuningBankSelect => (0, 4),
            Self::ModulationDepthRange => (0, 5),
            Self::Null => (0x7F, 0x7F),
            Self::Other { msb, lsb } => (msb & 0x7F, lsb & 0x7F),
        }
    }
}

/// Pitch Bend Sensitivity (RPN 00H 00H, p. 17): semitones from the Data Entry MSB and cents from the
/// Data Entry LSB.
pub const fn pitch_bend_sensitivity(msb: u8, lsb: u8) -> (u8, u8) {
    (msb & 0x7F, lsb & 0x7F)
}

/// Fine Tuning (RPN 00H 01H, p. 18): the signed offset of the 14-bit value from 8192, -8192 to
/// +8191, where 8192 steps are 100 cents from A440.
pub const fn fine_tuning(value: u16) -> i16 {
    (value & 0x3FFF) as i16 - 8192
}

/// Coarse Tuning (RPN 00H 02H, p. 18): the Data Entry MSB minus 64, in semitones from A440; the
/// Data Entry LSB is ignored.
pub const fn coarse_tuning(msb: u8) -> i8 {
    (msb & 0x7F) as i8 - 64
}

/// Modulation Depth Range (RPN 00H 05H, RP-024 3.4.4) in hundredths of a cent, from the 14-bit
/// Data Entry value: one MSB step is 100 cents and one LSB step is 100/128 cent, rounded to the
/// nearest hundredth (REQ-RPN-005). 01H 00H gives 10000, 00H 08H gives 625, the GM2 default
/// 00H 40H gives 5000.
pub const fn modulation_depth_range(value: u16) -> u32 {
    ((value & 0x3FFF) as u32 * 625 + 4) / 8
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn controller_numbers_follow_table_iii() {
        // REQ-CTL-001: Table III (p. T-3).
        let named = [
            (0, Controller::BankSelect),
            (1, Controller::ModulationWheel),
            (2, Controller::BreathController),
            (4, Controller::FootController),
            (5, Controller::PortamentoTime),
            (6, Controller::DataEntry),
            (7, Controller::ChannelVolume),
            (8, Controller::Balance),
            (10, Controller::Pan),
            (11, Controller::Expression),
            (12, Controller::EffectControl1),
            (13, Controller::EffectControl2),
            (16, Controller::GeneralPurpose(1)),
            (19, Controller::GeneralPurpose(4)),
            (32, Controller::Lsb(0)),
            (63, Controller::Lsb(31)),
            (64, Controller::DamperPedal),
            (68, Controller::LegatoFootswitch),
            (69, Controller::Hold2),
            (70, Controller::SoundController(1)),
            (74, Controller::SoundController(5)),
            (79, Controller::SoundController(10)),
            (80, Controller::GeneralPurpose(5)),
            (83, Controller::GeneralPurpose(8)),
            (84, Controller::PortamentoControl),
            (91, Controller::EffectsDepth(1)),
            (95, Controller::EffectsDepth(5)),
            (96, Controller::DataIncrement),
            (97, Controller::DataDecrement),
            (98, Controller::NrpnLsb),
            (99, Controller::NrpnMsb),
            (100, Controller::RpnLsb),
            (101, Controller::RpnMsb),
            (120, Controller::ChannelMode(120)),
            (127, Controller::ChannelMode(127)),
        ];
        for (n, c) in named {
            assert_eq!(Controller::from_number(n), Some(c), "{n}");
            assert_eq!(c.number(), n);
        }
        for n in [3, 9, 14, 15, 20, 31, 85, 90, 102, 119] {
            assert_eq!(
                Controller::from_number(n),
                Some(Controller::Undefined(n)),
                "{n}"
            );
        }
        assert_eq!(Controller::from_number(128), None);
    }

    #[test]
    fn prop_controller_number_round_trip() {
        // REQ-CTL-001: each number 0 to 127 gives a controller that gives the number back.
        proptest!(|(n in 0u8..=127)| {
            let c = Controller::from_number(n).unwrap();
            prop_assert_eq!(c.number(), n);
        });
    }

    #[test]
    fn controller_classes_follow_the_number_ranges() {
        // REQ-CTL-002: pp. 11, 12.
        assert_eq!(ControllerClass::of(0), Some(ControllerClass::Msb));
        assert_eq!(ControllerClass::of(31), Some(ControllerClass::Msb));
        assert_eq!(
            ControllerClass::of(32),
            Some(ControllerClass::Lsb { msb: 0 })
        );
        assert_eq!(
            ControllerClass::of(63),
            Some(ControllerClass::Lsb { msb: 31 })
        );
        assert_eq!(ControllerClass::of(64), Some(ControllerClass::SingleByte));
        assert_eq!(ControllerClass::of(95), Some(ControllerClass::SingleByte));
        assert_eq!(ControllerClass::of(96), Some(ControllerClass::Parameter));
        assert_eq!(ControllerClass::of(101), Some(ControllerClass::Parameter));
        assert_eq!(ControllerClass::of(102), Some(ControllerClass::Undefined));
        assert_eq!(ControllerClass::of(119), Some(ControllerClass::Undefined));
        assert_eq!(ControllerClass::of(120), Some(ControllerClass::ChannelMode));
        assert_eq!(ControllerClass::of(127), Some(ControllerClass::ChannelMode));
        assert_eq!(ControllerClass::of(128), None);
    }

    #[test]
    fn prop_lsb_pair_of_each_msb() {
        // REQ-CTL-002: controller n + 32 is the LSB of n, and its class names n.
        proptest!(|(n in 0u8..=255)| {
            match lsb_of(n) {
                Some(lsb) => {
                    prop_assert!(n <= 31);
                    prop_assert_eq!(lsb, n + 32);
                    prop_assert_eq!(ControllerClass::of(lsb), Some(ControllerClass::Lsb { msb: n }));
                }
                None => prop_assert!(n > 31),
            }
        });
    }

    #[test]
    fn switch_reading_splits_at_40h() {
        // REQ-CTL-003: p. 12; Legato Footswitch 00H-3FH Normal, 40H-7FH Legato (Table III).
        assert!(!switch_on(0x00));
        assert!(!switch_on(0x3F));
        assert!(switch_on(0x40));
        assert!(switch_on(0x7F));
    }

    #[test]
    fn centered_offset_is_value_minus_64() {
        // REQ-CTL-004: pp. 13 to 16.
        assert_eq!(centered(0), -64);
        assert_eq!(centered(64), 0);
        assert_eq!(centered(127), 63);
    }

    #[test]
    fn registered_parameter_numbers_follow_table_iiia() {
        // REQ-RPN-001: Table IIIa (p. T-4), RPN 00H 05H of RP-024 3.4.4 and the null value 7FH 7FH.
        let named = [
            ((0, 0), Rpn::PitchBendSensitivity),
            ((0, 1), Rpn::FineTuning),
            ((0, 2), Rpn::CoarseTuning),
            ((0, 3), Rpn::TuningProgramSelect),
            ((0, 4), Rpn::TuningBankSelect),
            ((0, 5), Rpn::ModulationDepthRange),
            ((0x7F, 0x7F), Rpn::Null),
            ((0, 6), Rpn::Other { msb: 0, lsb: 6 }),
        ];
        for ((msb, lsb), r) in named {
            assert_eq!(Rpn::from_parts(msb, lsb), r);
            assert_eq!(r.parts(), (msb, lsb));
        }
    }

    #[test]
    fn prop_rpn_round_trip() {
        // REQ-RPN-001: every 7-bit pair gives itself back.
        proptest!(|(msb in 0u8..=127, lsb in 0u8..=127)| {
            prop_assert_eq!(Rpn::from_parts(msb, lsb).parts(), (msb, lsb));
        });
    }

    #[test]
    fn pitch_bend_sensitivity_is_semitones_and_cents() {
        // REQ-RPN-002: p. 17, MSB 01H LSB 00H is 1 semitone and 0 cents.
        assert_eq!(pitch_bend_sensitivity(0x01, 0x00), (1, 0));
        assert_eq!(pitch_bend_sensitivity(0x02, 0x32), (2, 50));
    }

    #[test]
    fn fine_tuning_is_the_offset_from_8192() {
        // REQ-RPN-003: p. 18: 00H 00H is -100 cents, 40H 00H is A440, 7FH 7FH is +8191 steps.
        assert_eq!(fine_tuning(0x0000), -8192);
        assert_eq!(fine_tuning(0x40 << 7), 0);
        assert_eq!(fine_tuning(0x3FFF), 8191);
    }

    #[test]
    fn coarse_tuning_is_msb_minus_64() {
        // REQ-RPN-004: p. 18.
        assert_eq!(coarse_tuning(0x00), -64);
        assert_eq!(coarse_tuning(0x40), 0);
        assert_eq!(coarse_tuning(0x7F), 63);
    }

    #[test]
    fn modulation_depth_range_follows_gm2() {
        // REQ-RPN-005: RP-024 3.4.4 (p. 13): 01H 00H is one semitone, 00H 08H is 6.25 cents, the
        // default 00H 40H is 50 cents; one LSB step is 100/128 cent (78.125, rounded to 78).
        let v = |msb: u16, lsb: u16| (msb << 7) | lsb;
        assert_eq!(modulation_depth_range(v(1, 0)), 10_000);
        assert_eq!(modulation_depth_range(v(0, 8)), 625);
        assert_eq!(modulation_depth_range(v(0, 0x40)), 5_000);
        assert_eq!(modulation_depth_range(v(0, 1)), 78);
        assert_eq!(modulation_depth_range(0), 0);
        assert_eq!(modulation_depth_range(0x3FFF), 1_279_922);
        // Bits above the 14-bit value are ignored.
        assert_eq!(modulation_depth_range(0xC000 | v(1, 0)), 10_000);
    }

    #[test]
    fn prop_modulation_depth_range_rounds_to_nearest() {
        // REQ-RPN-005: the exact value is value × 10000 / 128 hundredths of a cent.
        proptest!(|(value in 0u16..0x4000)| {
            let exact = value as f64 * 10_000.0 / 128.0;
            let got = modulation_depth_range(value) as f64;
            prop_assert!((got - exact).abs() <= 0.5, "{value}: {got} for {exact}");
        });
    }

    #[test]
    fn gm2_default_names_map_to_table_iii() {
        // REQ-CTL-005: RP-024 3.3.15 to 3.3.20 name controllers that Table III numbers as Sound
        // Controllers 6 to 9 and Effects 1 and 3 Depth; the variants stay.
        let gm2 = [
            (75, Controller::SoundController(6)), // Decay Time
            (76, Controller::SoundController(7)), // Vibrato Rate
            (77, Controller::SoundController(8)), // Vibrato Depth
            (78, Controller::SoundController(9)), // Vibrato Delay
            (91, Controller::EffectsDepth(1)),    // Reverb Send Level
            (93, Controller::EffectsDepth(3)),    // Chorus Send Level
        ];
        for (n, c) in gm2 {
            assert_eq!(Controller::from_number(n), Some(c));
            assert_eq!(c.number(), n);
        }
    }
}
