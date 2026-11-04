//! MIDI 1.0 controller state of one channel (M1 v4.2.1 Control Change pp. 11 to 17, Table II
//! notes p. T-2, Table III p. T-3; RP-015 Response to Reset All Controllers; DEC-022).
//!
//! - [`ControllerState::receive`] takes a typed MIDI 1.0 Channel Voice message (DIN bytes,
//!   USB-MIDI 1.0, BLE-MIDI or a Message Type 0x2 UMP) and gives at most one
//!   [`ControllerEvent`]. The application keeps one state for each channel;
//!   [`Midi1ChannelVoiceMessage::channel`] selects it.
//! - 14-bit controllers (p. 12): controllers 0 to 31 are MSBs and 32 to 63 their LSBs. An MSB
//!   gives its value with LSB 0 ("the receiver should set its concept of the LSB to zero"); an
//!   LSB gives the held MSB with the new LSB.
//! - Bank Select (pp. 13, 14, AMB-M1-18): the bank stays for every later Program Change. Bank
//!   Select alone selects no program.
//! - Portamento Control (pp. 16, 17): the source note goes with the next Note On only.
//! - Reset All Controllers (p. 25, RP-015): [`reset_value`] gives the list, and
//!   [`ControllerEvent::ResetAllControllers`] the actions of the device.
//! - The Channel Mode rules (Omni, Basic Channel, Mode 4) are in the module `channel_mode`. With
//!   a `ModeState`, give only its `Voice` actions to this state and call
//!   [`ControllerState::reset_all_controllers`] at its `ResetAllControllers` action. Without
//!   one, `receive` applies controller 121 itself.
//! - The parameter controllers (6, 38, 96 to 101) belong to the parameter receiver (feature
//!   `parameters`).
//!
//! No alloc (DEC-002), no I/O (DEC-003). Never panics.

use zmidi_ump::{
    Channel, Group, Midi1ChannelVoiceMessage, Midi1ControlChange, Midi1ProgramChange, UmpError,
};

/// The mark of a held byte: a stored 0 holds nothing, a stored `HELD | value` holds `value`. A new
/// state is all zeros, so a `static` state goes in `.bss` and needs no flash image.
const HELD: u8 = 0x80;

/// Bank Select MSB and LSB (p. 13).
const BANK_MSB: u8 = 0;
const BANK_LSB: u8 = 32;
/// Data Entry MSB and LSB (Table III).
const DATA_ENTRY_MSB: u8 = 6;
const DATA_ENTRY_LSB: u8 = 38;
/// Portamento Control (p. 16).
const PORTAMENTO_CONTROL: u8 = 84;
/// Reset All Controllers (p. 25).
const RESET_ALL_CONTROLLERS: u8 = 121;

/// The held MSB of a new state for each MSB controller 0 to 31: the value of [`reset_value`], 0
/// for a kept controller. The state stores an MSB as `msb ^ INITIAL_MSB[n]`, so a new state and
/// Reset All Controllers both store 0.
const INITIAL_MSB: [u8; 32] = {
    let mut table = [0; 32];
    let mut i = 0;
    while i < 32 {
        if let Some(value) = reset_value(i as u8) {
            table[i] = value;
        }
        i += 1;
    }
    table
};

/// Bit n is set when MSB controller n keeps its value at Reset All Controllers.
const KEEP_MSB: u32 = {
    let mut mask = 0;
    let mut i = 0;
    while i < 32 {
        if reset_value(i as u8).is_none() {
            mask |= 1 << i;
        }
        i += 1;
    }
    mask
};

/// The RP-015 value of controller `controller` at Reset All Controllers, or `None` when the
/// controller keeps its value (REQ-CTRL-007):
///
/// - 0: Modulation (1, 33), the LSB of Expression (43), Sustain, Portamento On/Off, Sostenuto
///   and Soft (64 to 67), and each other controller ("Any other controllers ... should be set to
///   0").
/// - 127: Expression (11).
/// - Keep: Bank Select (0, 32), Volume (7, 39), Pan (10, 42), Balance (8, 40; RP-015 does not
///   list it, and 0 is full left: a documented choice, AMB-RP-015-04), Sound Controllers (70 to 79),
///   Effects Depth (91 to 95), Data Entry (6, 38) and the parameter controllers (96 to 101; the
///   parameter receiver sets the RPN and NRPN numbers to null), Portamento Control (84; the
///   state clears its source note), the Channel Mode messages (120 to 127), and every number
///   above 127.
pub const fn reset_value(controller: u8) -> Option<u8> {
    match controller {
        11 => Some(127),
        0 | 6..=8 | 10 | 32 | 38..=40 | 42 | 70..=79 | PORTAMENTO_CONTROL | 91..=101 => None,
        0..=119 => Some(0),
        _ => None,
    }
}

/// The one event that a received message gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControllerEvent {
    /// A 14-bit controller (p. 12): `controller` 0 to 31, Bank Select and Data Entry excluded;
    /// `value` 0 to 3FFFH. An MSB gives LSB 0; an LSB (controller + 32) gives the held MSB with
    /// the new LSB.
    Controller14 {
        /// The MSB controller number, 0 to 31.
        controller: u8,
        /// The 14-bit value, MSB in bits 7 to 13.
        value: u16,
    },
    /// A single-byte controller, 64 to 119 (Portamento Control and 96 to 101 excluded). The value
    /// of a switch is on from 64 (`zmidi_ump::switch_on`).
    Controller7 {
        /// The controller number, 64 to 119.
        controller: u8,
        /// The value, 0 to 127.
        value: u8,
    },
    /// Program Change (p. 14): the program and the held bank of the channel, or `None` before
    /// the first Bank Select (the device keeps its own bank).
    Program {
        /// The program number, 0 to 127.
        program: u8,
        /// The 14-bit bank, Bank Select MSB in bits 7 to 13.
        bank: Option<u16>,
    },
    /// A Note On (velocity above 0) after a Portamento Control: the pitch glides from `source`
    /// to the note of the message (p. 16). The Note On itself is the message.
    Portamento {
        /// The source note number, 0 to 127.
        source: u8,
    },
    /// Reset All Controllers (p. 25, RP-015). The device sets Pitch Bend to the center
    /// (`zmidi_ump::PITCH_BEND_CENTER`), Channel Pressure to 0, the Poly Pressure of every note
    /// to 0, and each controller that it uses to [`reset_value`]. It keeps the program, the
    /// bank and the values of the registered and non-registered parameters.
    ResetAllControllers,
}

/// Controller state of one channel: the held MSB of controllers 0 to 31, the bank and the
/// Portamento Control source. 34 bytes, all zeros when new, no alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControllerState {
    /// The last MSB of controllers 0 to 31, as `msb ^ INITIAL_MSB[n]`; index 0 is the bank MSB.
    msb: [u8; 32],
    /// `HELD | LSB` of the bank, or 0 before the first Bank Select.
    bank_lsb: u8,
    /// `HELD | note` of the last Portamento Control, or 0.
    portamento: u8,
}

impl ControllerState {
    /// The initial state: the held MSBs at the values of [`reset_value`] (0 for a kept
    /// controller), no bank, no Portamento Control source.
    pub const fn new() -> Self {
        Self {
            msb: [0; 32],
            bank_lsb: 0,
            portamento: 0,
        }
    }

    /// Takes one typed MIDI 1.0 Channel Voice message of the channel of this state, and gives
    /// at most one event (REQ-CTRL-001 to REQ-CTRL-006). Controller 121 with any value is Reset
    /// All Controllers. Never panics.
    pub fn receive(&mut self, message: &Midi1ChannelVoiceMessage) -> Option<ControllerEvent> {
        match message {
            Midi1ChannelVoiceMessage::ControlChange(m) => self.control_change(m.index(), m.data()),
            Midi1ChannelVoiceMessage::ProgramChange(m) => Some(ControllerEvent::Program {
                program: m.program(),
                bank: self.bank(),
            }),
            Midi1ChannelVoiceMessage::NoteOn(m) if m.velocity() > 0 => {
                let held = core::mem::take(&mut self.portamento);
                (held != 0).then_some(ControllerEvent::Portamento {
                    source: held & 0x7F,
                })
            }
            _ => None,
        }
    }

    /// The Reset All Controllers action from the application (a `ModeState` action or General
    /// MIDI System On, RP-015): the held MSBs get the values of [`reset_value`], the Portamento
    /// Control source is cleared, and the bank stays (REQ-CTRL-006).
    pub fn reset_all_controllers(&mut self) {
        for (n, held) in self.msb.iter_mut().enumerate() {
            if KEEP_MSB >> n & 1 == 0 {
                *held = 0;
            }
        }
        self.portamento = 0;
    }

    /// The held bank (Bank Select MSB in bits 7 to 13), or `None` before the first Bank Select.
    pub const fn bank(&self) -> Option<u16> {
        if self.bank_lsb == 0 {
            None
        } else {
            let msb = self.msb[BANK_MSB as usize] ^ INITIAL_MSB[BANK_MSB as usize];
            Some((msb as u16) << 7 | (self.bank_lsb & 0x7F) as u16)
        }
    }

    fn control_change(&mut self, controller: u8, value: u8) -> Option<ControllerEvent> {
        match controller {
            BANK_MSB => {
                // p. 12: an MSB sets the LSB to zero (AMB-M1-18).
                self.msb[BANK_MSB as usize] = value ^ INITIAL_MSB[BANK_MSB as usize];
                self.bank_lsb = HELD;
                None
            }
            BANK_LSB => {
                self.bank_lsb = HELD | value;
                None
            }
            DATA_ENTRY_MSB | DATA_ENTRY_LSB | 96..=101 => None,
            1..=5 | 7..=31 => {
                let n = controller as usize & 31;
                self.msb[n] = value ^ INITIAL_MSB[n];
                Some(ControllerEvent::Controller14 {
                    controller,
                    value: (value as u16) << 7,
                })
            }
            33..=37 | 39..=63 => {
                let n = controller as usize & 31;
                Some(ControllerEvent::Controller14 {
                    controller: n as u8,
                    value: ((self.msb[n] ^ INITIAL_MSB[n]) as u16) << 7 | value as u16,
                })
            }
            PORTAMENTO_CONTROL => {
                self.portamento = HELD | value;
                None
            }
            64..=83 | 85..=95 | 102..=119 => {
                Some(ControllerEvent::Controller7 { controller, value })
            }
            RESET_ALL_CONTROLLERS => {
                self.reset_all_controllers();
                Some(ControllerEvent::ResetAllControllers)
            }
            _ => None,
        }
    }
}

impl Default for ControllerState {
    fn default() -> Self {
        Self::new()
    }
}

/// One Control Change with Group 0.
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

/// Writes a 14-bit controller value as the Control Change of the MSB controller (0 to 31) and
/// then the Control Change of the LSB controller (controller + 32), with Group 0 (p. 12, Table II
/// notes: "first the MSB, then the LSB"; REQ-CTRL-008). A 7-bit value is one Control Change of
/// the MSB controller. Gives `OutOfRange` for a controller above 31 or a value above 3FFFH.
pub const fn controller14_messages(
    channel: Channel,
    controller: u8,
    value: u16,
) -> Result<[Midi1ChannelVoiceMessage; 2], UmpError> {
    if controller > 31 || value > 0x3FFF {
        return Err(UmpError::OutOfRange);
    }
    match (
        control_change(channel, controller, (value >> 7) as u8),
        control_change(channel, controller + 32, (value & 0x7F) as u8),
    ) {
        (Ok(msb), Ok(lsb)) => Ok([msb, lsb]),
        _ => Err(UmpError::OutOfRange),
    }
}

/// Writes a bank and program selection as Bank Select MSB (0), Bank Select LSB (32) and Program
/// Change, in that order, with Group 0 (p. 13: "the Program Change must be sent immediately after
/// the Bank Select pair"; REQ-CTRL-009). Gives `OutOfRange` for a bank above 3FFFH or a program
/// above 127.
pub const fn bank_program_messages(
    channel: Channel,
    bank: u16,
    program: u8,
) -> Result<[Midi1ChannelVoiceMessage; 3], UmpError> {
    if bank > 0x3FFF {
        return Err(UmpError::OutOfRange);
    }
    match (
        control_change(channel, BANK_MSB, (bank >> 7) as u8),
        control_change(channel, BANK_LSB, (bank & 0x7F) as u8),
        Midi1ProgramChange::try_new(Group::from_bits(0), channel, program),
    ) {
        (Ok(msb), Ok(lsb), Ok(pc)) => Ok([msb, lsb, Midi1ChannelVoiceMessage::ProgramChange(pc)]),
        _ => Err(UmpError::OutOfRange),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;
    use zmidi_ump::{Midi1NoteOff, Midi1NoteOn};

    fn ch() -> Channel {
        Channel::from_bits(0)
    }

    fn cc(controller: u8, value: u8) -> Midi1ChannelVoiceMessage {
        control_change(ch(), controller, value).unwrap()
    }

    fn pc(program: u8) -> Midi1ChannelVoiceMessage {
        Midi1ChannelVoiceMessage::ProgramChange(
            Midi1ProgramChange::try_new(Group::from_bits(0), ch(), program).unwrap(),
        )
    }

    fn note_on(note: u8, velocity: u8) -> Midi1ChannelVoiceMessage {
        Midi1ChannelVoiceMessage::NoteOn(
            Midi1NoteOn::try_new(Group::from_bits(0), ch(), note, velocity).unwrap(),
        )
    }

    fn note_off(note: u8) -> Midi1ChannelVoiceMessage {
        Midi1ChannelVoiceMessage::NoteOff(
            Midi1NoteOff::try_new(Group::from_bits(0), ch(), note, 0x40).unwrap(),
        )
    }

    fn c14(controller: u8, value: u16) -> Option<ControllerEvent> {
        Some(ControllerEvent::Controller14 { controller, value })
    }

    #[test]
    fn new_state_is_all_zeros() {
        // A static state goes in .bss: no flash image.
        let s = ControllerState::new();
        assert_eq!((s.msb, s.bank_lsb, s.portamento), ([0; 32], 0, 0));
        assert_eq!(s, ControllerState::default());
    }

    #[test]
    fn msb_and_lsb_give_14_bit_values() {
        // p. 12: 32 to 63 are the LSBs of 0 to 31; an MSB sets the LSB to zero; an LSB alone is a
        // fine adjustment of the held MSB.
        let mut s = ControllerState::new();
        assert_eq!(s.receive(&cc(1, 0x12)), c14(1, 0x12 << 7));
        assert_eq!(s.receive(&cc(33, 0x34)), c14(1, 0x12 << 7 | 0x34));
        assert_eq!(s.receive(&cc(33, 0x35)), c14(1, 0x12 << 7 | 0x35));
        assert_eq!(s.receive(&cc(1, 0x13)), c14(1, 0x13 << 7));
        // An undefined MSB controller (3) pairs with 35 too.
        assert_eq!(s.receive(&cc(3, 0x7F)), c14(3, 0x7F << 7));
        assert_eq!(s.receive(&cc(35, 0x7F)), c14(3, 0x3FFF));
        // Before any MSB, an LSB uses the initial MSB: 127 for Expression (RP-015), else 0.
        let mut s = ControllerState::new();
        assert_eq!(s.receive(&cc(43, 5)), c14(11, 127 << 7 | 5));
        assert_eq!(s.receive(&cc(39, 5)), c14(7, 5));
    }

    #[test]
    fn prop_lsb_uses_the_held_msb() {
        // p. 12 and Table II note: the LSB combines with the last MSB of the same controller.
        proptest!(|(c in 1u8..32, msb in 0u8..128, lsbs in proptest::collection::vec(0u8..128, 1..4))| {
            prop_assume!(c != DATA_ENTRY_MSB);
            let mut s = ControllerState::new();
            prop_assert_eq!(s.receive(&cc(c, msb)), c14(c, (msb as u16) << 7));
            for lsb in lsbs {
                prop_assert_eq!(s.receive(&cc(c + 32, lsb)), c14(c, (msb as u16) << 7 | lsb as u16));
            }
        });
    }

    #[test]
    fn single_byte_controllers_give_their_value() {
        // p. 12: 64 and above have single-byte values only.
        let mut s = ControllerState::new();
        for c in (64u8..=119).filter(|c| *c != PORTAMENTO_CONTROL && !(96..=101).contains(c)) {
            assert_eq!(
                s.receive(&cc(c, 0x41)),
                Some(ControllerEvent::Controller7 {
                    controller: c,
                    value: 0x41
                }),
                "controller {c}"
            );
        }
    }

    #[test]
    fn parameter_and_mode_controllers_give_nothing() {
        // The parameter receiver owns 6, 38 and 96 to 101; the mode state owns 120 to 127.
        let mut s = ControllerState::new();
        for c in [
            6u8, 38, 96, 97, 98, 99, 100, 101, 120, 122, 123, 124, 125, 126, 127,
        ] {
            assert_eq!(s.receive(&cc(c, 0x7F)), None, "controller {c}");
        }
        // Data Entry MSB is not held as a controller MSB.
        assert_eq!(s.receive(&cc(38, 1)), None);
        // Other messages give nothing.
        assert_eq!(s.receive(&note_off(60)), None);
        assert_eq!(s.receive(&note_on(60, 0)), None);
        assert_eq!(s, ControllerState::new());
    }

    #[test]
    fn bank_select_is_held_for_program_change() {
        // pp. 13, 14: the bank is held for the next Program Change, and later Program Changes
        // select programs in that bank. AMB-M1-18: no bank before the first Bank Select.
        let mut s = ControllerState::new();
        assert_eq!(s.bank(), None);
        assert_eq!(
            s.receive(&pc(4)),
            Some(ControllerEvent::Program {
                program: 4,
                bank: None
            })
        );
        s.receive(&cc(0, 1));
        s.receive(&cc(32, 2));
        let program = |p| {
            Some(ControllerEvent::Program {
                program: p,
                bank: Some(1 << 7 | 2),
            })
        };
        assert_eq!(s.receive(&pc(5)), program(5));
        assert_eq!(s.receive(&pc(6)), program(6));
        // An MSB alone sets the LSB to zero; an LSB alone changes only the LSB.
        s.receive(&cc(0, 3));
        assert_eq!(s.bank(), Some(3 << 7));
        s.receive(&cc(32, 9));
        assert_eq!(s.bank(), Some(3 << 7 | 9));
        // RP-015: Reset All Controllers keeps the bank.
        assert_eq!(
            s.receive(&cc(121, 0)),
            Some(ControllerEvent::ResetAllControllers)
        );
        assert_eq!(s.bank(), Some(3 << 7 | 9));
        // An LSB alone before any MSB: MSB 0.
        let mut s = ControllerState::new();
        s.receive(&cc(32, 7));
        assert_eq!(s.bank(), Some(7));
    }

    #[test]
    fn bank_select_alone_selects_no_program() {
        // p. 14: "Bank Select alone must not change the program."
        let mut s = ControllerState::new();
        assert_eq!(s.receive(&cc(0, 1)), None);
        assert_eq!(s.receive(&cc(32, 2)), None);
    }

    #[test]
    fn portamento_control_acts_on_the_next_note_on_only() {
        // p. 17, Example 1: 90 3C 40, B0 54 3C, 90 40 40 (re-tune from 60 to 64), 80 3C 40,
        // 80 40 40.
        let mut s = ControllerState::new();
        assert_eq!(s.receive(&note_on(0x3C, 0x40)), None);
        assert_eq!(s.receive(&cc(0x54, 0x3C)), None);
        assert_eq!(
            s.receive(&note_on(0x40, 0x40)),
            Some(ControllerEvent::Portamento { source: 0x3C })
        );
        assert_eq!(s.receive(&note_off(0x3C)), None);
        assert_eq!(s.receive(&note_off(0x40)), None);
        // Example 2: B0 54 3C, 90 40 40 (glide from 60), 80 40 40; a later Note On has no source.
        assert_eq!(s.receive(&cc(0x54, 0x3C)), None);
        assert_eq!(
            s.receive(&note_on(0x40, 0x40)),
            Some(ControllerEvent::Portamento { source: 0x3C })
        );
        assert_eq!(s.receive(&note_off(0x40)), None);
        assert_eq!(s.receive(&note_on(0x40, 0x40)), None);
        // A Note On with velocity 0 is a Note Off and keeps the source.
        s.receive(&cc(0x54, 0x30));
        assert_eq!(s.receive(&note_on(0x40, 0)), None);
        assert_eq!(
            s.receive(&note_on(0x41, 1)),
            Some(ControllerEvent::Portamento { source: 0x30 })
        );
        // Reset All Controllers clears the source (p. 25).
        s.receive(&cc(0x54, 0x30));
        s.reset_all_controllers();
        assert_eq!(s.receive(&note_on(0x41, 1)), None);
    }

    #[test]
    fn reset_all_controllers_follows_rp_015() {
        // RP-015 DETAILS: Modulation 0, Expression 127, keep Volume, Pan and Bank Select; other
        // controllers 0. The held MSBs follow, so a later LSB uses them.
        let mut s = ControllerState::new();
        for c in 0u8..32 {
            s.receive(&cc(c, 0x55));
        }
        assert_eq!(
            s.receive(&cc(121, 0)),
            Some(ControllerEvent::ResetAllControllers)
        );
        let lsb = |s: &mut ControllerState, c: u8| match s.receive(&cc(c + 32, 0)) {
            Some(ControllerEvent::Controller14 { value, .. }) => value >> 7,
            other => panic!("{other:?}"),
        };
        assert_eq!(lsb(&mut s, 1), 0);
        assert_eq!(lsb(&mut s, 11), 127);
        assert_eq!(lsb(&mut s, 7), 0x55);
        assert_eq!(lsb(&mut s, 10), 0x55);
        // AMB-RP-015-04: Balance stays, as Pan does.
        assert_eq!(lsb(&mut s, 8), 0x55);
        assert_eq!(lsb(&mut s, 2), 0);
        assert_eq!(s.bank(), Some(0x55 << 7));
        // The application path gives the same state.
        let mut t = ControllerState::new();
        for c in 0u8..32 {
            t.receive(&cc(c, 0x55));
        }
        t.reset_all_controllers();
        let mut u = ControllerState::new();
        for c in 0u8..32 {
            u.receive(&cc(c, 0x55));
        }
        u.receive(&cc(121, 0));
        assert_eq!(t, u);
    }

    #[test]
    fn reset_accepts_any_value_byte() {
        // RP-015: "The value byte is always 0, although many implementations allow any value".
        for v in 0u8..128 {
            let mut s = ControllerState::new();
            s.receive(&cc(1, 9));
            assert_eq!(
                s.receive(&cc(121, v)),
                Some(ControllerEvent::ResetAllControllers)
            );
            assert_eq!(s.receive(&cc(33, 0)), c14(1, 0));
        }
    }

    #[test]
    fn reset_values_follow_rp_015() {
        // RP-015 DETAILS, item by item.
        assert_eq!(reset_value(11), Some(127));
        for c in [1u8, 33, 43, 64, 65, 66, 67] {
            assert_eq!(reset_value(c), Some(0), "controller {c}");
        }
        let keep: Vec<u8> = [0u8, 32, 7, 39, 10, 42, 8, 40, 6, 38, 84]
            .into_iter()
            .chain(70..=79)
            .chain(91..=101)
            .chain(120..=255)
            .collect();
        for c in &keep {
            assert_eq!(reset_value(*c), None, "controller {c}");
        }
        // "Any other controllers ... should be set to 0".
        for c in (0u8..=119).filter(|c| !keep.contains(c) && *c != 11) {
            assert_eq!(reset_value(c), Some(0), "controller {c}");
        }
    }

    #[test]
    fn controller14_messages_write_msb_then_lsb() {
        // p. 12, Table II note: "first the MSB, then the LSB".
        let [msb, lsb] = controller14_messages(Channel::from_bits(5), 1, 0x1234).unwrap();
        assert_eq!(msb.into_bits(), 0x20B5_0124);
        assert_eq!(lsb.into_bits(), 0x20B5_2134);
        assert_eq!(
            controller14_messages(ch(), 32, 0),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            controller14_messages(ch(), 0, 0x4000),
            Err(UmpError::OutOfRange)
        );
    }

    #[test]
    fn prop_controller14_messages_round_trip() {
        // The writer and the state agree: the LSB message gives the full value.
        proptest!(|(c in 1u8..32, value in 0u16..0x4000)| {
            prop_assume!(c != DATA_ENTRY_MSB);
            let mut s = ControllerState::new();
            let [msb, lsb] = controller14_messages(ch(), c, value).unwrap();
            prop_assert_eq!(s.receive(&msb), c14(c, value & 0x3F80));
            prop_assert_eq!(s.receive(&lsb), c14(c, value));
        });
    }

    #[test]
    fn bank_program_messages_in_order() {
        // p. 13: Bank Select MSB, Bank Select LSB, then Program Change at once.
        let m = bank_program_messages(Channel::from_bits(2), 0x0102, 0x11).unwrap();
        assert_eq!(
            m.map(Midi1ChannelVoiceMessage::into_bits),
            [0x20B2_0002, 0x20B2_2002, 0x20C2_1100]
        );
        let mut s = ControllerState::new();
        let events: Vec<_> = m.iter().map(|m| s.receive(m)).collect();
        assert_eq!(
            events,
            [
                None,
                None,
                Some(ControllerEvent::Program {
                    program: 0x11,
                    bank: Some(0x0102)
                })
            ]
        );
        assert_eq!(
            bank_program_messages(ch(), 0x4000, 0),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            bank_program_messages(ch(), 0, 0x80),
            Err(UmpError::OutOfRange)
        );
    }
}
