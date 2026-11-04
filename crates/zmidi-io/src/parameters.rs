//! MIDI 1.0 parameter receiver of one channel (M1 v4.2.1 Registered and Non-Registered Parameter
//! Numbers pp. 17, 18, Changing Tuning Programs p. 50; RP-015 Response to Reset All
//! Controllers; RP-018 Response to Data Increment/Decrement Controllers; DEC-022).
//!
//! - [`ParameterState::receive`] takes a typed MIDI 1.0 Channel Voice message (DIN bytes,
//!   USB-MIDI 1.0, BLE-MIDI or a Message Type 0x2 UMP) and gives at most one
//!   [`ParameterEvent`]: the selected parameter and the change of a Data Entry, Data Increment
//!   or Data Decrement. The application keeps one receiver for each channel;
//!   [`Midi1ChannelVoiceMessage::channel`] selects it.
//! - The receiver holds no parameter value. The application holds the value of each parameter
//!   that it uses and gets the new value from [`ParameterEvent::apply`] (RP-018 steps).
//! - The 1.0 to 2.0 translator of `zmidi-ump` keeps its own state: it holds a Data Entry MSB
//!   until a trigger (M2-104 D.3.3). This receiver gives each Data Entry at once (DEC-022
//!   addendum of 2026-11-04).
//! - With a `ModeState`, give only its `Voice` actions to this receiver and call
//!   [`ParameterState::reset_all_controllers`] at its `ResetAllControllers` action. Without
//!   one, `receive` applies controller 121 itself.
//!
//! No alloc (DEC-002), no I/O (DEC-003). Never panics.

use zmidi_ump::controller::Rpn;
use zmidi_ump::{Channel, Group, Midi1ChannelVoiceMessage, Midi1ControlChange, UmpError};

/// The mark of a received number byte: a stored 0 is "not received", a stored `HELD | value` holds
/// `value`. A new receiver is all zeros, so a `static` receiver goes in `.bss`.
const HELD: u8 = 0x80;
/// The null number byte: 7FH 7FH selects no parameter (RP-015, RP-018, M2-104 D.3.3).
const NULL: u8 = 0x7F;

/// Controller numbers (Table III, p. 17).
const DATA_ENTRY_MSB: u8 = 6;
const DATA_ENTRY_LSB: u8 = 38;
const DATA_INCREMENT: u8 = 96;
const DATA_DECREMENT: u8 = 97;
const NRPN_LSB: u8 = 98;
const NRPN_MSB: u8 = 99;
const RPN_LSB: u8 = 100;
const RPN_MSB: u8 = 101;
const RESET_ALL_CONTROLLERS: u8 = 121;

/// Flag bits: the kind selected last is NRPN; RPN reception off; NRPN reception on.
const NRPN_LAST: u8 = 0b001;
const RPN_OFF: u8 = 0b010;
const NRPN_ON: u8 = 0b100;

/// A parameter: a Registered Parameter Number (controllers 101 and 100) or a Non-Registered
/// Parameter Number (controllers 99 and 98).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Parameter {
    /// A Registered Parameter Number (Table IIIa).
    Registered(Rpn),
    /// A Non-Registered Parameter Number; the manufacturer gives its meaning (p. 17).
    NonRegistered {
        /// Controller 99, 7 bits.
        msb: u8,
        /// Controller 98, 7 bits.
        lsb: u8,
    },
}

/// One change of the selected parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParameterChange {
    /// Data Entry MSB (controller 6), 7 bits.
    DataEntryMsb(u8),
    /// Data Entry LSB (controller 38), 7 bits.
    DataEntryLsb(u8),
    /// Data Increment (controller 96); the value byte is ignored (RP-018).
    Increment,
    /// Data Decrement (controller 97); the value byte is ignored (RP-018).
    Decrement,
}

/// What the receiver gives for a Data Entry, Data Increment or Data Decrement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParameterEvent {
    /// The selected parameter.
    pub parameter: Parameter,
    /// The change.
    pub change: ParameterChange,
}

impl ParameterEvent {
    /// The new 14-bit value of the parameter (MSB in bits 7 to 13) from its current value
    /// (REQ-PARAM-005):
    ///
    /// - Data Entry MSB sets the MSB and sets the LSB to 0 (p. 12).
    /// - Data Entry LSB sets the LSB.
    /// - Data Increment and Data Decrement (RP-018): RPN 0 (Pitch Bend Sensitivity) steps the LSB
    ///   in cents with a carry at 100 (99 + 1 gives the next semitone and 0 cents); RPN 2, 3 and
    ///   4 step the MSB; RPN 1 and every other RPN step the 14-bit value. For an NRPN the
    ///   manufacturer defines the step; this function steps the 14-bit value as a default.
    /// - A step stops at the limits (0 and 3FFFH, MSB 0 and 127, 127 semitones 99 cents) and does
    ///   not wrap (AMB-RP-018-01).
    pub const fn apply(self, value: u16) -> u16 {
        let value = value & 0x3FFF;
        match self.change {
            ParameterChange::DataEntryMsb(msb) => ((msb & 0x7F) as u16) << 7,
            ParameterChange::DataEntryLsb(lsb) => value & 0x3F80 | (lsb & 0x7F) as u16,
            ParameterChange::Increment => step(self.parameter, value, true),
            ParameterChange::Decrement => step(self.parameter, value, false),
        }
    }
}

/// One RP-018 step of `value` (14 bits).
const fn step(parameter: Parameter, value: u16, up: bool) -> u16 {
    let rpn = match parameter {
        Parameter::Registered(rpn) => rpn.parts(),
        Parameter::NonRegistered { .. } => return step14(value, up),
    };
    let (msb, lsb) = (value >> 7, value & 0x7F);
    match rpn {
        // RPN 0: cents, with a carry at 100.
        (0, 0) => match (up, lsb, msb) {
            (true, 0..=98, _) => value + 1,
            (true, _, 0..=126) => (msb + 1) << 7,
            (false, 1.., _) => value - 1,
            (false, 0, 1..) => (msb - 1) << 7 | 99,
            _ => value,
        },
        // RPN 2, 3 and 4: the MSB.
        (0, 2..=4) => match (up, msb) {
            (true, 0..=126) => value + 0x80,
            (false, 1..) => value - 0x80,
            _ => value,
        },
        _ => step14(value, up),
    }
}

/// One step of the 14-bit value.
const fn step14(value: u16, up: bool) -> u16 {
    match (up, value) {
        (true, 0..=0x3FFE) => value + 1,
        (false, 1..) => value - 1,
        _ => value,
    }
}

/// Parameter receiver of one channel: the RPN number and the NRPN number, each byte with a
/// "not received" mark, the kind selected last, and the reception of each kind. 5 bytes, all
/// zeros when new, no alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParameterState {
    /// `HELD |` controllers 101 and 100, or 0.
    rpn: [u8; 2],
    /// `HELD |` controllers 99 and 98, or 0.
    nrpn: [u8; 2],
    /// `NRPN_LAST`, `RPN_OFF`, `NRPN_ON`.
    flags: u8,
}

impl ParameterState {
    /// The initial state: no number received, RPN reception on and NRPN reception off (p. 17,
    /// rules 2 and 5).
    pub const fn new() -> Self {
        Self {
            rpn: [0; 2],
            nrpn: [0; 2],
            flags: 0,
        }
    }

    /// Takes one typed MIDI 1.0 Channel Voice message of the channel of this receiver, and gives
    /// at most one event (REQ-PARAM-001, REQ-PARAM-002, REQ-PARAM-004). Controllers 98 to 101
    /// change the number also while reception of their kind is off, so that a Data Entry for an
    /// NRPN never goes to the last RPN. Controller 121 with any value is Reset All Controllers.
    /// Never panics.
    pub fn receive(&mut self, message: &Midi1ChannelVoiceMessage) -> Option<ParameterEvent> {
        let Midi1ChannelVoiceMessage::ControlChange(m) = message else {
            return None;
        };
        let value = m.data();
        let change = match m.index() {
            DATA_ENTRY_MSB => ParameterChange::DataEntryMsb(value),
            DATA_ENTRY_LSB => ParameterChange::DataEntryLsb(value),
            DATA_INCREMENT => ParameterChange::Increment,
            DATA_DECREMENT => ParameterChange::Decrement,
            RPN_MSB | RPN_LSB => {
                self.rpn[(m.index() == RPN_LSB) as usize] = HELD | value;
                self.flags &= !NRPN_LAST;
                return None;
            }
            NRPN_MSB | NRPN_LSB => {
                self.nrpn[(m.index() == NRPN_LSB) as usize] = HELD | value;
                self.flags |= NRPN_LAST;
                return None;
            }
            RESET_ALL_CONTROLLERS => {
                self.reset_all_controllers();
                return None;
            }
            _ => return None,
        };
        Some(ParameterEvent {
            parameter: self.selection()?,
            change,
        })
    }

    /// The Reset All Controllers action from the application (a `ModeState` action or General
    /// MIDI System On): the RPN number and the NRPN number become 7FH 7FH, so that no parameter
    /// is selected (RP-015; REQ-PARAM-004). The values of the parameters stay.
    pub fn reset_all_controllers(&mut self) {
        self.rpn = [HELD | NULL; 2];
        self.nrpn = [HELD | NULL; 2];
    }

    /// The parameter that a Data Entry acts on now, or `None`: a byte of the number of the kind
    /// selected last is not received, the number is the null number 7FH 7FH, or reception of
    /// that kind is off (REQ-PARAM-001).
    pub const fn selection(&self) -> Option<Parameter> {
        let nrpn = self.flags & NRPN_LAST != 0;
        let ([msb, lsb], on) = if nrpn {
            (self.nrpn, self.nrpn_reception())
        } else {
            (self.rpn, self.rpn_reception())
        };
        let (msb, lsb) = (msb & !HELD, lsb & !HELD);
        if !on || self.number_missing(nrpn) || (msb == NULL && lsb == NULL) {
            None
        } else if nrpn {
            Some(Parameter::NonRegistered { msb, lsb })
        } else {
            Some(Parameter::Registered(Rpn::from_parts(msb, lsb)))
        }
    }

    /// Tells if a byte of the number of a kind is not received.
    const fn number_missing(&self, nrpn: bool) -> bool {
        let [msb, lsb] = if nrpn { self.nrpn } else { self.rpn };
        msb & lsb & HELD == 0
    }

    /// Tells if RPN reception is on (REQ-PARAM-003).
    pub const fn rpn_reception(&self) -> bool {
        self.flags & RPN_OFF == 0
    }

    /// Turns RPN reception on or off. When it goes on, the receiver waits again for both bytes
    /// of an RPN number (p. 17, rule 3; REQ-PARAM-003).
    pub fn set_rpn_reception(&mut self, on: bool) {
        if on && !self.rpn_reception() {
            self.rpn = [0; 2];
        }
        self.flags = if on {
            self.flags & !RPN_OFF
        } else {
            self.flags | RPN_OFF
        };
    }

    /// Tells if NRPN reception is on (REQ-PARAM-003).
    pub const fn nrpn_reception(&self) -> bool {
        self.flags & NRPN_ON != 0
    }

    /// Turns NRPN reception on or off. When it goes on, the receiver waits again for both bytes
    /// of an NRPN number (p. 17, rules 2 and 3; REQ-PARAM-003).
    pub fn set_nrpn_reception(&mut self, on: bool) {
        if on && !self.nrpn_reception() {
            self.nrpn = [0; 2];
        }
        self.flags = if on {
            self.flags | NRPN_ON
        } else {
            self.flags & !NRPN_ON
        };
    }
}

impl Default for ParameterState {
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

/// Writes the number of `parameter` as the Control Change of its MSB and then of its LSB (101
/// and 100, or 99 and 98), with Group 0 (p. 17; REQ-PARAM-006). `Registered(Rpn::Null)` writes
/// the null number 7FH 7FH, which ends a selection (RP-018 example). Gives `OutOfRange` for a
/// number byte above 127.
pub const fn select_messages(
    channel: Channel,
    parameter: Parameter,
) -> Result<[Midi1ChannelVoiceMessage; 2], UmpError> {
    let (controllers, (msb, lsb)) = match parameter {
        Parameter::Registered(Rpn::Other { msb, lsb }) => ((RPN_MSB, RPN_LSB), (msb, lsb)),
        Parameter::Registered(rpn) => ((RPN_MSB, RPN_LSB), rpn.parts()),
        Parameter::NonRegistered { msb, lsb } => ((NRPN_MSB, NRPN_LSB), (msb, lsb)),
    };
    match (
        control_change(channel, controllers.0, msb),
        control_change(channel, controllers.1, lsb),
    ) {
        (Ok(msb), Ok(lsb)) => Ok([msb, lsb]),
        _ => Err(UmpError::OutOfRange),
    }
}

/// Writes one change as a Control Change with Group 0: Data Entry MSB (6), Data Entry LSB (38),
/// Data Increment (96) or Data Decrement (97), the last two with value 0 (RP-018 example;
/// REQ-PARAM-006). Gives `OutOfRange` for a data byte above 127.
pub const fn change_message(
    channel: Channel,
    change: ParameterChange,
) -> Result<Midi1ChannelVoiceMessage, UmpError> {
    match change {
        ParameterChange::DataEntryMsb(msb) => control_change(channel, DATA_ENTRY_MSB, msb),
        ParameterChange::DataEntryLsb(lsb) => control_change(channel, DATA_ENTRY_LSB, lsb),
        ParameterChange::Increment => control_change(channel, DATA_INCREMENT, 0),
        ParameterChange::Decrement => control_change(channel, DATA_DECREMENT, 0),
    }
}

/// Writes a parameter value as the number of `parameter` and then Data Entry MSB (6) and Data
/// Entry LSB (38), with Group 0 (p. 17: "it is recommended that the LSB and MSB be sent each
/// time a new parameter number is selected"; REQ-PARAM-006). Gives `OutOfRange` for a number
/// byte above 127 or a value above 3FFFH.
pub const fn value_messages(
    channel: Channel,
    parameter: Parameter,
    value: u16,
) -> Result<[Midi1ChannelVoiceMessage; 4], UmpError> {
    if value > 0x3FFF {
        return Err(UmpError::OutOfRange);
    }
    match (
        select_messages(channel, parameter),
        control_change(channel, DATA_ENTRY_MSB, (value >> 7) as u8),
        control_change(channel, DATA_ENTRY_LSB, (value & 0x7F) as u8),
    ) {
        (Ok([msb, lsb]), Ok(data_msb), Ok(data_lsb)) => Ok([msb, lsb, data_msb, data_lsb]),
        _ => Err(UmpError::OutOfRange),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;

    const PBS: Parameter = Parameter::Registered(Rpn::PitchBendSensitivity);

    fn ch() -> Channel {
        Channel::from_bits(0)
    }

    fn cc(controller: u8, value: u8) -> Midi1ChannelVoiceMessage {
        control_change(ch(), controller, value).unwrap()
    }

    fn event(parameter: Parameter, change: ParameterChange) -> Option<ParameterEvent> {
        Some(ParameterEvent { parameter, change })
    }

    fn feed(s: &mut ParameterState, pairs: &[(u8, u8)]) -> Vec<Option<ParameterEvent>> {
        pairs.iter().map(|&(c, v)| s.receive(&cc(c, v))).collect()
    }

    #[test]
    fn new_receiver_is_all_zeros() {
        // A static receiver goes in .bss: no flash image.
        let s = ParameterState::new();
        assert_eq!((s.rpn, s.nrpn, s.flags), ([0; 2], [0; 2], 0));
        assert_eq!(s, ParameterState::default());
    }

    #[test]
    fn selection_needs_both_bytes() {
        // p. 17, rule 3: wait for both the LSB and the MSB; either order (AMB-M1-62).
        let mut s = ParameterState::new();
        assert_eq!(s.receive(&cc(6, 2)), None);
        assert_eq!(s.receive(&cc(101, 0)), None);
        assert_eq!(s.receive(&cc(6, 2)), None);
        assert_eq!(s.receive(&cc(100, 0)), None);
        assert_eq!(
            s.receive(&cc(6, 2)),
            event(PBS, ParameterChange::DataEntryMsb(2))
        );
        let mut s = ParameterState::new();
        feed(&mut s, &[(100, 1), (101, 0)]);
        assert_eq!(s.selection(), Some(Parameter::Registered(Rpn::FineTuning)));
    }

    #[test]
    fn one_byte_changes_a_complete_number() {
        // p. 17, rule 4: after a complete number, an LSB alone or an MSB alone selects again.
        let mut s = ParameterState::new();
        feed(&mut s, &[(101, 0), (100, 0)]);
        s.receive(&cc(100, 2));
        assert_eq!(
            s.selection(),
            Some(Parameter::Registered(Rpn::CoarseTuning))
        );
        s.receive(&cc(101, 5));
        assert_eq!(
            s.selection(),
            Some(Parameter::Registered(Rpn::Other { msb: 5, lsb: 2 }))
        );
        // Each kind keeps its own number: back to RPN with the MSB alone.
        s.set_nrpn_reception(true);
        feed(&mut s, &[(99, 1), (98, 2)]);
        assert_eq!(
            s.selection(),
            Some(Parameter::NonRegistered { msb: 1, lsb: 2 })
        );
        s.receive(&cc(101, 0));
        assert_eq!(
            s.selection(),
            Some(Parameter::Registered(Rpn::CoarseTuning))
        );
    }

    #[test]
    fn null_number_selects_nothing() {
        // RP-015, RP-018 example, M2-104 D.3.3: 7FH 7FH is the null number.
        let mut s = ParameterState::new();
        s.set_nrpn_reception(true);
        let events = feed(
            &mut s,
            &[(101, 0), (100, 0), (101, 0x7F), (100, 0x7F), (6, 1)],
        );
        assert_eq!(events, [None, None, None, None, None]);
        assert_eq!(s.selection(), None);
        let events = feed(&mut s, &[(99, 0x7F), (98, 0x7F), (96, 0)]);
        assert_eq!(events, [None, None, None]);
        // 7FH 00H is a number, not the null number.
        s.receive(&cc(98, 0));
        assert_eq!(
            s.selection(),
            Some(Parameter::NonRegistered { msb: 0x7F, lsb: 0 })
        );
    }

    #[test]
    fn data_entry_and_steps_give_events() {
        // p. 17: Data Entry, Data Increment or Data Decrement acts on the number selected last.
        // RP-018: the value byte of 96 and 97 is ignored.
        let mut s = ParameterState::new();
        feed(&mut s, &[(101, 0), (100, 0)]);
        let events = feed(&mut s, &[(6, 3), (38, 50), (96, 0), (96, 0x7F), (97, 0x33)]);
        assert_eq!(
            events,
            [
                event(PBS, ParameterChange::DataEntryMsb(3)),
                event(PBS, ParameterChange::DataEntryLsb(50)),
                event(PBS, ParameterChange::Increment),
                event(PBS, ParameterChange::Increment),
                event(PBS, ParameterChange::Decrement),
            ]
        );
        // An NRPN step goes to the application with the number (RP-018).
        s.set_nrpn_reception(true);
        feed(&mut s, &[(99, 0x10), (98, 0x20)]);
        let nrpn = Parameter::NonRegistered {
            msb: 0x10,
            lsb: 0x20,
        };
        assert_eq!(
            s.receive(&cc(96, 1)),
            event(nrpn, ParameterChange::Increment)
        );
        // With NRPN reception off, an NRPN Data Entry gives nothing, and does not go to the RPN.
        s.set_nrpn_reception(false);
        feed(&mut s, &[(101, 0), (100, 0), (99, 1), (98, 1)]);
        assert_eq!(s.receive(&cc(6, 9)), None);
        // Other messages give nothing.
        assert_eq!(s.receive(&cc(7, 9)), None);
    }

    #[test]
    fn new_number_gives_no_event() {
        // p. 17, rule 6: the parameter keeps its value until a Data Entry, Increment or
        // Decrement arrives.
        let mut s = ParameterState::new();
        s.set_nrpn_reception(true);
        let events = feed(&mut s, &[(101, 0), (100, 1), (99, 2), (98, 3), (100, 2)]);
        assert_eq!(events, [None, None, None, None, None]);
    }

    #[test]
    fn tuning_program_and_bank_select_vectors() {
        // p. 50: Bn 64 03 65 00 06 tt; Bn 64 03 65 00 60 7F; Bn 64 03 65 00 61 7F (LSB first),
        // and the same with 04 for Tuning Bank Select. tt = 0 to 127 (AMB-M1-47).
        for (lsb, rpn) in [(3, Rpn::TuningProgramSelect), (4, Rpn::TuningBankSelect)] {
            let p = Parameter::Registered(rpn);
            let mut s = ParameterState::new();
            let events = feed(&mut s, &[(0x64, lsb), (0x65, 0), (0x06, 0x05)]);
            assert_eq!(events[2], event(p, ParameterChange::DataEntryMsb(5)));
            let e = feed(&mut s, &[(0x64, lsb), (0x65, 0), (0x60, 0x7F)])[2].unwrap();
            assert_eq!(
                e,
                ParameterEvent {
                    parameter: p,
                    change: ParameterChange::Increment
                }
            );
            // RP-018: RPN 3 and 4 step the MSB.
            assert_eq!(e.apply(5 << 7), 6 << 7);
            let e = feed(&mut s, &[(0x64, lsb), (0x65, 0), (0x61, 0x7F)])[2].unwrap();
            assert_eq!(e.apply(5 << 7), 4 << 7);
            // The writer gives the same parameter and value.
            let mut r = ParameterState::new();
            let m = value_messages(ch(), p, 5 << 7).unwrap();
            let events: Vec<_> = m.iter().map(|m| r.receive(m)).collect();
            assert_eq!(events[2], event(p, ParameterChange::DataEntryMsb(5)));
        }
    }

    #[test]
    fn reception_defaults_and_switches() {
        // p. 17: NRPN reception off at power-up (rule 2), RPN reception may be on (rule 5); after
        // reception goes on, wait for both bytes (rule 3).
        let mut s = ParameterState::new();
        assert!(s.rpn_reception());
        assert!(!s.nrpn_reception());
        feed(&mut s, &[(99, 1), (98, 1)]);
        assert_eq!(s.receive(&cc(6, 1)), None);
        s.set_nrpn_reception(true);
        assert!(s.nrpn_reception());
        assert_eq!(s.receive(&cc(6, 1)), None);
        feed(&mut s, &[(99, 1), (98, 1)]);
        assert!(s.receive(&cc(6, 1)).is_some());
        // RPN off, then on again: both bytes again.
        feed(&mut s, &[(101, 0), (100, 0)]);
        s.set_rpn_reception(false);
        assert!(!s.rpn_reception());
        assert_eq!(s.receive(&cc(6, 1)), None);
        s.set_rpn_reception(true);
        assert_eq!(s.receive(&cc(6, 1)), None);
        s.receive(&cc(100, 0));
        assert_eq!(s.receive(&cc(6, 1)), None);
        s.receive(&cc(101, 0));
        assert_eq!(
            s.receive(&cc(6, 1)),
            event(PBS, ParameterChange::DataEntryMsb(1))
        );
        // Turning on what is on changes nothing.
        s.set_rpn_reception(true);
        assert!(s.receive(&cc(6, 1)).is_some());
    }

    #[test]
    fn reset_all_controllers_nulls_the_selection() {
        // RP-015: the RPN and NRPN numbers become 127 (null); parameter values stay (the
        // application holds them). Any value byte.
        for v in [0u8, 0x7F] {
            let mut s = ParameterState::new();
            s.set_nrpn_reception(true);
            feed(&mut s, &[(99, 1), (98, 1), (101, 0), (100, 0)]);
            assert_eq!(s.receive(&cc(121, v)), None);
            assert_eq!(s.selection(), None);
            assert_eq!(s.receive(&cc(6, 1)), None);
            // The numbers are 7FH 7FH: one byte gives a complete number.
            s.receive(&cc(101, 0));
            assert_eq!(
                s.selection(),
                Some(Parameter::Registered(Rpn::Other { msb: 0, lsb: 0x7F }))
            );
        }
        let mut a = ParameterState::new();
        let mut b = ParameterState::new();
        feed(&mut a, &[(101, 0), (100, 0), (121, 0)]);
        feed(&mut b, &[(101, 0), (100, 0)]);
        b.reset_all_controllers();
        assert_eq!(a, b);
    }

    fn apply(parameter: Parameter, change: ParameterChange, value: u16) -> u16 {
        ParameterEvent { parameter, change }.apply(value)
    }

    #[test]
    fn rp_018_steps() {
        use ParameterChange::{Decrement as Dn, Increment as Up};
        let st = |p, c, (m, l): (u16, u16)| {
            let v = apply(p, c, m << 7 | l);
            (v >> 7, v & 0x7F)
        };
        // RPN 0: the LSB (cents), with a carry at 100 (RP-018 receiving devices).
        assert_eq!(st(PBS, Up, (2, 0)), (2, 1));
        assert_eq!(st(PBS, Up, (2, 99)), (3, 0));
        assert_eq!(st(PBS, Dn, (3, 0)), (2, 99));
        assert_eq!(st(PBS, Dn, (0, 0)), (0, 0));
        assert_eq!(st(PBS, Up, (127, 99)), (127, 99));
        assert_eq!(st(PBS, Up, (2, 120)), (3, 0));
        assert_eq!(st(PBS, Dn, (2, 120)), (2, 119));
        // RPN 1 and other RPNs: the 14-bit value (LSB 127 + 1 carries).
        let fine = Parameter::Registered(Rpn::FineTuning);
        assert_eq!(st(fine, Up, (0x40, 0x7F)), (0x41, 0));
        assert_eq!(st(fine, Dn, (0x41, 0)), (0x40, 0x7F));
        assert_eq!(st(fine, Up, (0x7F, 0x7F)), (0x7F, 0x7F));
        let mdr = Parameter::Registered(Rpn::Other { msb: 0, lsb: 5 });
        assert_eq!(st(mdr, Up, (0, 0x7F)), (1, 0));
        // RPN 2, 3, 4: the MSB; the LSB stays.
        for rpn in [
            Rpn::CoarseTuning,
            Rpn::TuningProgramSelect,
            Rpn::TuningBankSelect,
        ] {
            let p = Parameter::Registered(rpn);
            assert_eq!(st(p, Up, (0x40, 3)), (0x41, 3));
            assert_eq!(st(p, Dn, (0x40, 3)), (0x3F, 3));
            assert_eq!(st(p, Up, (127, 0)), (127, 0));
            assert_eq!(st(p, Dn, (0, 0)), (0, 0));
        }
        // NRPN: the 14-bit default.
        let n = Parameter::NonRegistered { msb: 1, lsb: 1 };
        assert_eq!(st(n, Up, (0, 0x7F)), (1, 0));
        assert_eq!(st(n, Dn, (0, 0)), (0, 0));
        // Data Entry: MSB sets the LSB to 0; LSB keeps the MSB.
        assert_eq!(st(PBS, ParameterChange::DataEntryMsb(12), (2, 50)), (12, 0));
        assert_eq!(
            st(PBS, ParameterChange::DataEntryLsb(25), (12, 0)),
            (12, 25)
        );
    }

    #[test]
    fn rp_018_example_two_increments_give_two_cents() {
        // RP-018 example: B0 65 00, B0 64 00, B0 60 00, B0 60 00, B0 65 7F, B0 64 7F.
        let mut s = ParameterState::new();
        let mut pbs: u16 = 2 << 7;
        for e in feed(
            &mut s,
            &[
                (0x65, 0),
                (0x64, 0),
                (0x60, 0),
                (0x60, 0),
                (0x65, 0x7F),
                (0x64, 0x7F),
            ],
        )
        .into_iter()
        .flatten()
        {
            pbs = e.apply(pbs);
        }
        assert_eq!(pbs, 2 << 7 | 2);
        assert_eq!(s.selection(), None);
    }

    #[test]
    fn prop_steps_stay_in_range() {
        // AMB-RP-018-01: a step never leaves 0 to 3FFFH, and an increment then a decrement
        // gives the start value, except at a limit.
        proptest!(|(msb in 0u8..128, lsb in 0u8..128, rpn in any::<bool>(), value in 0u16..0x4000)| {
            let p = if rpn {
                Parameter::Registered(Rpn::from_parts(msb, lsb))
            } else {
                Parameter::NonRegistered { msb, lsb }
            };
            let up = apply(p, ParameterChange::Increment, value);
            let down = apply(p, ParameterChange::Decrement, value);
            prop_assert!(up <= 0x3FFF && down <= 0x3FFF);
            prop_assert!(up >= value && down <= value);
            if up != value && !(rpn && (msb, lsb) == (0, 0) && value & 0x7F > 99) {
                prop_assert_eq!(apply(p, ParameterChange::Decrement, up), value);
            }
        });
    }

    #[test]
    fn writer_writes_number_then_data_entry() {
        // p. 17: number MSB, number LSB, then Data Entry MSB and LSB.
        let m = value_messages(Channel::from_bits(3), PBS, 2 << 7 | 50).unwrap();
        assert_eq!(
            m.map(Midi1ChannelVoiceMessage::into_bits),
            [0x20B3_6500, 0x20B3_6400, 0x20B3_0602, 0x20B3_2632]
        );
        let n = Parameter::NonRegistered { msb: 1, lsb: 2 };
        let m = select_messages(ch(), n).unwrap();
        assert_eq!(
            m.map(Midi1ChannelVoiceMessage::into_bits),
            [0x20B0_6301, 0x20B0_6202]
        );
        assert_eq!(value_messages(ch(), PBS, 0x4000), Err(UmpError::OutOfRange));
        let bad = Parameter::NonRegistered { msb: 0x80, lsb: 0 };
        assert_eq!(select_messages(ch(), bad), Err(UmpError::OutOfRange));
        let bad = Parameter::Registered(Rpn::Other { msb: 0, lsb: 0x80 });
        assert_eq!(select_messages(ch(), bad), Err(UmpError::OutOfRange));
        assert_eq!(
            change_message(ch(), ParameterChange::DataEntryLsb(0x80)),
            Err(UmpError::OutOfRange)
        );
    }

    #[test]
    fn rp_018_example_bytes() {
        // RP-018 example: $B0 $65 $0, $B0 $64 $0, $B0 $60 $0, $B0 $60 $0, $B0 $65 $7F, $B0 $64 $7F.
        let [a, b] = select_messages(ch(), PBS).unwrap();
        let up = change_message(ch(), ParameterChange::Increment).unwrap();
        let [y, z] = select_messages(ch(), Parameter::Registered(Rpn::Null)).unwrap();
        assert_eq!(
            [a, b, up, up, y, z].map(Midi1ChannelVoiceMessage::into_bits),
            [
                0x20B0_6500,
                0x20B0_6400,
                0x20B0_6000,
                0x20B0_6000,
                0x20B0_657F,
                0x20B0_647F
            ]
        );
        let down = change_message(ch(), ParameterChange::Decrement).unwrap();
        assert_eq!(down.into_bits(), 0x20B0_6100);
    }

    #[test]
    fn prop_written_values_round_trip() {
        // The writer and the receiver agree: the events give the written value.
        proptest!(|(msb in 0u8..128, lsb in 0u8..128, rpn in any::<bool>(), value in 0u16..0x4000, start in 0u16..0x4000)| {
            prop_assume!((msb, lsb) != (NULL, NULL));
            let p = if rpn {
                Parameter::Registered(Rpn::from_parts(msb, lsb))
            } else {
                Parameter::NonRegistered { msb, lsb }
            };
            let mut s = ParameterState::new();
            s.set_nrpn_reception(true);
            let mut v = start;
            for m in value_messages(ch(), p, value).unwrap() {
                if let Some(e) = s.receive(&m) {
                    prop_assert_eq!(e.parameter, p);
                    v = e.apply(v);
                }
            }
            prop_assert_eq!(v, value);
        });
    }
}
