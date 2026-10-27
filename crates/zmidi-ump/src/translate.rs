//! Default Translation between MIDI 1.0 Protocol and MIDI 2.0 Protocol UMP messages
//! (M2-104-UM Appendix D).
//!
//! - Scope: MIDI 1.0 Channel Voice (Message Type 0x2) to and from MIDI 2.0 Channel Voice
//!   (Message Type 0x4). System messages (Message Type 0x1) are the same in both protocols and
//!   pass unchanged (D.2.6, D.3.7). Every other packet, System Exclusive (7-bit) included, also
//!   passes unchanged: it is a UMP in both protocols. Translation to and from a MIDI 1.0 byte
//!   stream, with its 0xF0 and 0xF7 bytes (D.2.7, D.3.7), is in the `zmidi-io` crate.
//! - Data values scale to the full range of the destination field (D.1). The scaling follows
//!   M2-115-U where it and Appendix D differ (REQ-XLATE-002): Min-Center-Max for most values,
//!   and Zero-Extension for Registered Controllers with Index (RPN LSB) 0 to 31 (M2-115 3.1,
//!   4.1).
//! - MIDI 2.0 to MIDI 1.0 ([`to_midi1`]) is stateless. One message gives up to 4 MIDI 1.0
//!   messages (D.2.3). Relative controllers and the per-note messages have no MIDI 1.0
//!   equivalent and give none (D.2.8).
//! - MIDI 1.0 to MIDI 2.0 ([`Midi1ToMidi2`]) holds, for each Group and Channel, the latest
//!   RPN/NRPN controllers (CC 6, 98, 99, 100, 101) and Bank Select (CC 0, 32) (D.3.3, D.3.4).
//!   [`Midi1ToMidi2Group`] holds them for the 16 Channels of one Group only (1/16 of the size).

use crate::channel_voice1::Midi1ChannelVoiceMessage;
#[cfg(test)]
use crate::channel_voice1::{
    Midi1ChannelPressure, Midi1ControlChange, Midi1NoteOff, Midi1NoteOn, Midi1PitchBend,
    Midi1PolyPressure, Midi1ProgramChange,
};
use crate::channel_voice2::Midi2ChannelVoiceMessage;
#[cfg(test)]
use crate::channel_voice2::{
    AssignableController, Midi2ChannelPressure, Midi2ControlChange, Midi2NoteOff, Midi2NoteOn,
    Midi2PitchBend, Midi2PolyPressure, Midi2ProgramChange, NoteAttribute, RegisteredController,
};
use crate::packet::{Channel, Group};
use crate::scale::{downscale, upscale, zero_extend_downscale, zero_extend_upscale};
use crate::ump::Ump;
use crate::utility::{Noop, UtilityMessage};

/// Capacity of [`Translated`]: a Registered or Assignable Controller gives 4 Control Changes
/// (D.2.3).
const CAPACITY: usize = 4;

/// Filler for the unused slots of [`Translated`]; never visible to a caller.
const FILL: Ump = Ump::Utility(UtilityMessage::Noop(Noop::new()));

/// Up to 4 UMPs: the result of one MIDI 2.0 to MIDI 1.0 translation. Needs no alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Translated {
    items: [Ump; CAPACITY],
    len: u8,
}

impl Translated {
    const EMPTY: Self = Self {
        items: [FILL; CAPACITY],
        len: 0,
    };

    fn one(ump: Ump) -> Self {
        let mut t = Self::EMPTY;
        t.push(ump);
        t
    }

    fn push(&mut self, ump: Ump) {
        if let Some(slot) = self.items.get_mut(self.len as usize) {
            *slot = ump;
            self.len += 1;
        }
    }

    /// The number of UMPs.
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// True when the message has no MIDI 1.0 equivalent (D.2.8).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The UMPs in output order.
    pub fn as_slice(&self) -> &[Ump] {
        &self.items[..self.len as usize]
    }
}

impl IntoIterator for Translated {
    type Item = Ump;
    type IntoIter = core::iter::Take<core::array::IntoIter<Ump, CAPACITY>>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter().take(self.len as usize)
    }
}

/// Min-Center-Max upscale between valid widths; the widths are constants, so it never fails.
fn up(value: u32, src: u8, dst: u8) -> u32 {
    upscale(value, src, dst).unwrap_or(0)
}

/// Min-Center-Max downscale between valid widths; the widths are constants, so it never fails.
#[cfg(test)]
fn down(value: u32, src: u8, dst: u8) -> u32 {
    downscale(value, src, dst).unwrap_or(0)
}

/// True when a Registered Controller uses Zero-Extension Scaling: Index (RPN LSB) 0 to 31
/// (M2-115 4.1). Other Registered Controllers and all Assignable Controllers use
/// Min-Center-Max (M2-115 3.1).
const fn zero_extension(index: u8) -> bool {
    index < 32
}

#[cfg(test)]
fn cc(group: Group, channel: Channel, index: u8, data: u32) -> Option<Ump> {
    Midi1ControlChange::try_new(group, channel, index, data as u8)
        .ok()
        .map(|m| Ump::Midi1ChannelVoice(Midi1ChannelVoiceMessage::ControlChange(m)))
}

#[cfg(test)]
fn midi1(m: Midi1ChannelVoiceMessage) -> Ump {
    Ump::Midi1ChannelVoice(m)
}

fn midi2(m: Midi2ChannelVoiceMessage) -> Ump {
    Ump::Midi2ChannelVoice(m)
}

/// The MIDI 1.0 Channel Voice words (Message Type 0x2) that one MIDI 2.0 Channel Voice packet
/// translates to: 0 to 4 one-word packets (D.2.3). No alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Midi1Words {
    words: [u32; CAPACITY],
    len: u8,
}

impl Midi1Words {
    /// The words, one MIDI 1.0 Channel Voice packet each, in send order.
    pub fn as_slice(&self) -> &[u32] {
        &self.words[..self.len as usize]
    }
}

/// Translates the words of one MIDI 2.0 Channel Voice packet (Message Type 0x4) to MIDI 1.0
/// Channel Voice words (D.2), with the rules of [`to_midi1`] and no typed value (DEC-018).
///
/// Gives `None` when `words` is not a MIDI 2.0 Channel Voice packet that the crate decodes
/// (another Message Type, opcode 0x7, or fewer than 2 words): the caller passes such a packet on
/// unchanged, as [`to_midi1`] does (D.2.6). A message with no MIDI 1.0 equivalent gives an empty
/// list (D.2.8). Reserved bits are not used (2.1.3). Never panics.
pub fn midi2_to_midi1(words: &[u32]) -> Option<Midi1Words> {
    let (&w0, &w1) = (words.first()?, words.get(1)?);
    let opcode = (w0 >> 20) & 0xF;
    if w0 >> 28 != 0x4 || opcode == 0x7 {
        return None;
    }
    // Group and Channel stay; Message Type 0x2 (D.2.1).
    let head = 0x2000_0000 | (w0 & 0x0F0F_0000);
    let (b2, b3) = ((w0 >> 8) & 0x7F, w0 & 0x7F);
    let word = |status: u32, d0: u32, d1: u32| head | status << 20 | (d0 & 0x7F) << 8 | (d1 & 0x7F);
    let down = |value: u32, src: u8, dst: u8| downscale(value, src, dst).unwrap_or(0);
    // Each case builds the whole array at once, so the compiler writes the words with plain
    // stores and does not first clear the value with a memclr call (OPT-037).
    let one = |w: u32| ([w, 0, 0, 0], 1);
    let parameter = |select: [u32; 2], value: u32| {
        let w = [
            word(0xB, select[0], b2),
            word(0xB, select[1], b3),
            word(0xB, 6, value >> 7),
            word(0xB, 38, value),
        ];
        (w, 4)
    };
    let (words, len) = match opcode {
        0x8 => one(word(0x8, b2, down(w1 >> 16, 16, 7))),
        // A velocity that downscales to 0 becomes 1 (7.4.2).
        0x9 => one(word(0x9, b2, down(w1 >> 16, 16, 7).max(1))),
        0xA => one(word(0xA, b2, down(w1, 32, 7))),
        0xB => one(word(0xB, b2, down(w1, 32, 7))),
        0x2 => {
            let value = if zero_extension(b3 as u8) {
                zero_extend_downscale(w1, 32, 14).unwrap_or(0)
            } else {
                down(w1, 32, 14)
            };
            parameter([101, 100], value)
        }
        0x3 => parameter([99, 98], down(w1, 32, 14)),
        0xC => {
            let program = word(0xC, w1 >> 24, 0);
            // Option Flag Bank Valid (bit 0 of word 0): CC 0, CC 32, then the Program Change (D.2.4).
            if w0 & 1 != 0 {
                ([word(0xB, 0, w1 >> 8), word(0xB, 32, w1), program, 0], 3)
            } else {
                one(program)
            }
        }
        0xD => one(word(0xD, down(w1, 32, 7), 0)),
        0xE => {
            let value = down(w1, 32, 14);
            one(word(0xE, value, value >> 7))
        }
        // Per-Note and Relative messages have no MIDI 1.0 equivalent (D.2.8).
        _ => ([0; CAPACITY], 0),
    };
    Some(Midi1Words { words, len })
}

/// Translates one UMP from the MIDI 2.0 Protocol to the MIDI 1.0 Protocol (D.2).
///
/// - Values are downscaled by Min-Center-Max (D.1.4), or by Zero-Extension for a Registered
///   Controller with Index 0 to 31 (M2-115 4.4).
/// - A Note On whose velocity downscales to 0 gets velocity 1 (7.4.2).
/// - A Registered Controller gives CC 101, 100, 6, 38 and an Assignable Controller gives CC 99,
///   98, 6, 38, with the 32-bit data downscaled to 14 bits (D.2.3).
/// - A Program Change with Bank Valid gives CC 0, CC 32 and the Program Change, in that order;
///   without it, the Program Change only (D.2.4).
/// - Relative Registered and Assignable Controllers, Per-Note Controllers, Per-Note Management
///   and Per-Note Pitch Bend give no message (D.2.8).
/// - Every other UMP, MIDI 1.0 Channel Voice and System included, passes unchanged (D.2.6).
///
/// Never panics.
pub fn to_midi1(ump: &Ump) -> Translated {
    let Ump::Midi2ChannelVoice(m) = ump else {
        return Translated::one(*ump);
    };
    let mut out = Translated::EMPTY;
    for &w in midi2_to_midi1(&m.into_words())
        .unwrap_or_default()
        .as_slice()
    {
        if let Some(m1) = Midi1ChannelVoiceMessage::parse(&[w]) {
            out.push(Ump::Midi1ChannelVoice(m1));
        }
    }
    out
}

/// The typed implementation of [`to_midi1`] before DEC-018, kept as the test reference of
/// [`midi2_to_midi1`].
#[cfg(test)]
fn reference_to_midi1(ump: &Ump) -> Translated {
    let Ump::Midi2ChannelVoice(m) = ump else {
        return Translated::one(*ump);
    };
    use Midi2ChannelVoiceMessage as M2;
    let mut out = Translated::EMPTY;
    match m {
        M2::NoteOff(m) => {
            let v = down(m.velocity() as u32, 16, 7) as u8;
            if let Ok(n) = Midi1NoteOff::try_new(m.group(), m.channel(), m.note(), v) {
                out.push(midi1(Midi1ChannelVoiceMessage::NoteOff(n)));
            }
        }
        M2::NoteOn(m) => {
            let v = (down(m.velocity() as u32, 16, 7) as u8).max(1);
            if let Ok(n) = Midi1NoteOn::try_new(m.group(), m.channel(), m.note(), v) {
                out.push(midi1(Midi1ChannelVoiceMessage::NoteOn(n)));
            }
        }
        M2::PolyPressure(m) => {
            let v = down(m.pressure(), 32, 7) as u8;
            if let Ok(n) = Midi1PolyPressure::try_new(m.group(), m.channel(), m.note(), v) {
                out.push(midi1(Midi1ChannelVoiceMessage::PolyPressure(n)));
            }
        }
        M2::ControlChange(m) => {
            if let Some(u) = cc(m.group(), m.channel(), m.index(), down(m.data(), 32, 7)) {
                out.push(u);
            }
        }
        M2::RegisteredController(m) => {
            let value = if zero_extension(m.index()) {
                zero_extend_downscale(m.data(), 32, 14).unwrap_or(0)
            } else {
                down(m.data(), 32, 14)
            };
            push_parameter(
                &mut out,
                m.group(),
                m.channel(),
                [101, 100],
                m.bank(),
                m.index(),
                value,
            );
        }
        M2::AssignableController(m) => {
            let value = down(m.data(), 32, 14);
            push_parameter(
                &mut out,
                m.group(),
                m.channel(),
                [99, 98],
                m.bank(),
                m.index(),
                value,
            );
        }
        M2::ProgramChange(m) => {
            if m.bank_valid() {
                for u in [
                    cc(m.group(), m.channel(), 0, m.bank_msb() as u32),
                    cc(m.group(), m.channel(), 32, m.bank_lsb() as u32),
                ]
                .into_iter()
                .flatten()
                {
                    out.push(u);
                }
            }
            if let Ok(p) = Midi1ProgramChange::try_new(m.group(), m.channel(), m.program()) {
                out.push(midi1(Midi1ChannelVoiceMessage::ProgramChange(p)));
            }
        }
        M2::ChannelPressure(m) => {
            let v = down(m.pressure(), 32, 7) as u8;
            if let Ok(p) = Midi1ChannelPressure::try_new(m.group(), m.channel(), v) {
                out.push(midi1(Midi1ChannelVoiceMessage::ChannelPressure(p)));
            }
        }
        M2::PitchBend(m) => {
            let v = down(m.value(), 32, 14) as u16;
            if let Ok(p) = Midi1PitchBend::try_new(m.group(), m.channel(), v) {
                out.push(midi1(Midi1ChannelVoiceMessage::PitchBend(p)));
            }
        }
        M2::RegisteredPerNoteController(_)
        | M2::AssignablePerNoteController(_)
        | M2::RelativeRegisteredController(_)
        | M2::RelativeAssignableController(_)
        | M2::PerNotePitchBend(_)
        | M2::PerNoteManagement(_) => {}
    }
    out
}

/// CC `select[0]` (bank), CC `select[1]` (index), CC 6 (data MSB), CC 38 (data LSB) (D.2.3).
#[cfg(test)]
fn push_parameter(
    out: &mut Translated,
    group: Group,
    channel: Channel,
    select: [u8; 2],
    bank: u8,
    index: u8,
    value: u32,
) {
    for u in [
        cc(group, channel, select[0], bank as u32),
        cc(group, channel, select[1], index as u32),
        cc(group, channel, 6, value >> 7),
        cc(group, channel, 38, value & 0x7F),
    ]
    .into_iter()
    .flatten()
    {
        out.push(u);
    }
}

/// Which parameter the latest CC 98 to 101 selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Parameter {
    None,
    /// CC 101 or 100: a Registered Parameter (RPN).
    Registered,
    /// CC 99 or 98: a Non-Registered Parameter (NRPN), an Assignable Controller in MIDI 2.0.
    Assignable,
}

/// One held 7-bit Control Change value, or no value: 1 byte (OPT-011). The byte is the value
/// plus 1, and 0 is no value, so an empty translator is all zero bits and a `static` of it
/// needs no flash image (OPT-013).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Held(u8);

impl Held {
    /// No value.
    const NONE: Held = Held(0);

    /// A held value; `value` is a 7-bit Control Change field.
    const fn some(value: u8) -> Held {
        Held((value & 0x7F) + 1)
    }

    const fn get(self) -> Option<u8> {
        match self.0 {
            0 => None,
            v => Some(v - 1),
        }
    }
}

/// Held values of one Group and Channel (D.3.3, D.3.4), in 8 bytes (OPT-011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChannelState {
    /// CC 101, CC 100.
    rpn: [Held; 2],
    /// CC 99, CC 98.
    nrpn: [Held; 2],
    /// CC 6.
    data_msb: Held,
    /// CC 0, CC 32.
    bank: [Held; 2],
    /// Bits 0-1: the selected [`Parameter`] (0 None, 1 Registered, 2 Assignable). Bit 2: CC 6
    /// was received and its value is not sent yet.
    flags: u8,
}

impl ChannelState {
    const EMPTY: ChannelState = ChannelState {
        rpn: [Held::NONE; 2],
        nrpn: [Held::NONE; 2],
        data_msb: Held::NONE,
        bank: [Held::NONE; 2],
        flags: 0,
    };
    const PENDING: u8 = 0b100;

    const fn parameter(&self) -> Parameter {
        match self.flags & 0b11 {
            1 => Parameter::Registered,
            2 => Parameter::Assignable,
            _ => Parameter::None,
        }
    }

    fn set_parameter(&mut self, p: Parameter) {
        let bits = match p {
            Parameter::None => 0,
            Parameter::Registered => 1,
            Parameter::Assignable => 2,
        };
        self.flags = (self.flags & !0b11) | bits;
    }

    const fn pending(&self) -> bool {
        self.flags & Self::PENDING != 0
    }

    fn set_pending(&mut self, pending: bool) {
        if pending {
            self.flags |= Self::PENDING;
        } else {
            self.flags &= !Self::PENDING;
        }
    }
}

/// Stateful translator from the MIDI 1.0 Protocol to the MIDI 2.0 Protocol (D.3).
///
/// It holds the latest CC 6, 98, 99, 100, 101 and Bank Select values for each of the 16 Groups
/// and 16 Channels, in fixed storage: one [`Midi1ToMidi2Group`] for each Group.
#[derive(Debug, Clone)]
pub struct Midi1ToMidi2 {
    groups: [Midi1ToMidi2Group; 16],
}

impl Default for Midi1ToMidi2 {
    fn default() -> Self {
        Self::new()
    }
}

impl Midi1ToMidi2 {
    /// A translator with no held values.
    pub const fn new() -> Self {
        Self {
            groups: [Midi1ToMidi2Group::new(); 16],
        }
    }

    /// Clears all held values.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Gives the RPN or NRPN of `group` and `channel` whose CC 6 is held and not sent (no CC 38
    /// came after it), as a Registered or Assignable Controller with data LSB 0. It is for the
    /// optional timeout of the D.3.3 note. Gives `None` when nothing is pending.
    pub fn flush(&mut self, group: Group, channel: Channel) -> Option<Ump> {
        self.groups[group.index() as usize & 0xF].flush(group, channel)
    }

    /// Translates one UMP and gives zero or one UMP (D.3), with the held values of the Group of
    /// the UMP. The rules are those of [`Midi1ToMidi2Group::translate`].
    ///
    /// Never panics.
    pub fn translate(&mut self, ump: &Ump) -> Option<Ump> {
        let Ump::Midi1ChannelVoice(m) = ump else {
            return Some(*ump);
        };
        let word = m.into_bits();
        self.groups[(word >> 24) as usize & 0xF]
            .translate_words(word)
            .map(|w| midi2(Midi2ChannelVoiceMessage::from_translated(w)))
    }
}

/// Stateful translator from the MIDI 1.0 Protocol to the MIDI 2.0 Protocol (D.3) for the
/// messages of one Group, for a source that uses one Group only (OPT-001).
///
/// It holds the latest CC 6, 98, 99, 100, 101 and Bank Select values for each of the 16
/// Channels, in fixed storage. The held values do not depend on the Group: messages of two
/// Groups with the same Channel use the same values. Give it the messages of one Group, or use
/// [`Midi1ToMidi2`]. Each output UMP has the Group of its input UMP.
#[derive(Debug, Clone, Copy)]
pub struct Midi1ToMidi2Group {
    state: [ChannelState; 16],
}

impl Default for Midi1ToMidi2Group {
    fn default() -> Self {
        Self::new()
    }
}

impl Midi1ToMidi2Group {
    /// A translator with no held values.
    pub const fn new() -> Self {
        Self {
            state: [ChannelState::EMPTY; 16],
        }
    }

    /// Clears all held values.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Gives the RPN or NRPN of `channel` whose CC 6 is held and not sent (no CC 38 came after
    /// it), as a Registered or Assignable Controller on `group` with data LSB 0. It is for the
    /// optional timeout of the D.3.3 note. Gives `None` when nothing is pending.
    pub fn flush(&mut self, group: Group, channel: Channel) -> Option<Ump> {
        let head = 0x4000_0000 | (group.index() as u32) << 24 | (channel.index() as u32) << 16;
        let state = &mut self.state[channel.index() as usize & 0xF];
        take_pending_words(head, state).map(|w| midi2(Midi2ChannelVoiceMessage::from_translated(w)))
    }

    /// The typed implementation of [`Self::flush`] before DEC-018, kept as the test reference.
    #[cfg(test)]
    fn flush_typed(&mut self, group: Group, channel: Channel) -> Option<Ump> {
        let state = self.state_mut(channel);
        if !state.pending() {
            return None;
        }
        state.set_pending(false);
        let msb = state.data_msb.get()?;
        controller(group, channel, state, msb, 0)
    }

    /// Translates one UMP and gives zero or one UMP (D.3).
    ///
    /// - Note On with velocity 0 gives Note Off with velocity 0x8000 (D.3.1). Other velocities
    ///   and all other values are upscaled by Min-Center-Max; Attribute Type and data are 0.
    /// - A 14-bit Control Change pair stays two Control Changes, and CC 96 and 97 stay Control
    ///   Changes (D.3.3).
    /// - CC 6, 38, 98, 99, 100 and 101 are held. A valid RPN or NRPN is sent as one Registered
    ///   or Assignable Controller on a CC 38, on a following CC 6, or on a CC 98 to 101 that
    ///   starts a new one. The Null Function (0x7F, 0x7F) is not sent (D.3.3).
    /// - CC 0 and 32 are held until the next Program Change, which carries them with Bank
    ///   Valid 1. A Program Change with no Bank Select since the last Program Change has Bank
    ///   Valid 0 and zero bank fields (D.3.4). A CC 0 sets the held LSB to 0, as the MIDI 1.0
    ///   Detailed Specification says for every MSB; a CC 32 alone gives MSB 0.
    /// - Every other UMP, MIDI 2.0 and System included, passes unchanged (D.3.7).
    ///
    /// Never panics.
    pub fn translate(&mut self, ump: &Ump) -> Option<Ump> {
        let Ump::Midi1ChannelVoice(m) = ump else {
            return Some(*ump);
        };
        // One implementation on words (DEC-018).
        self.translate_words(m.into_bits())
            .map(|w| midi2(Midi2ChannelVoiceMessage::from_translated(w)))
    }

    /// The typed implementation of [`Self::translate`] before DEC-018, kept as the test
    /// reference of [`Self::translate_words`].
    #[cfg(test)]
    fn translate_typed(&mut self, ump: &Ump) -> Option<Ump> {
        let Ump::Midi1ChannelVoice(m) = ump else {
            return Some(*ump);
        };
        use Midi1ChannelVoiceMessage as M1;
        use Midi2ChannelVoiceMessage as M2;
        match m {
            M1::NoteOff(m) => {
                let v = up(m.velocity() as u32, 7, 16) as u16;
                Midi2NoteOff::try_new(m.group(), m.channel(), m.note(), v, NoteAttribute::None)
                    .ok()
                    .map(|n| midi2(M2::NoteOff(n)))
            }
            M1::NoteOn(m) if m.velocity() == 0 => Midi2NoteOff::try_new(
                m.group(),
                m.channel(),
                m.note(),
                0x8000,
                NoteAttribute::None,
            )
            .ok()
            .map(|n| midi2(M2::NoteOff(n))),
            M1::NoteOn(m) => {
                let v = up(m.velocity() as u32, 7, 16) as u16;
                Midi2NoteOn::try_new(m.group(), m.channel(), m.note(), v, NoteAttribute::None)
                    .ok()
                    .map(|n| midi2(M2::NoteOn(n)))
            }
            M1::PolyPressure(m) => {
                let v = up(m.pressure() as u32, 7, 32);
                Midi2PolyPressure::try_new(m.group(), m.channel(), m.note(), v)
                    .ok()
                    .map(|n| midi2(M2::PolyPressure(n)))
            }
            M1::ControlChange(m) => self.control_change(m),
            M1::ProgramChange(m) => {
                let state = self.state_mut(m.channel());
                // The Bank Select values belong to this Program Change only. A later Program
                // Change without a new Bank Select gets Bank Valid 0: the receiver keeps its
                // bank (7.4.9), and the round trip gives the original messages (D.1.2).
                let [msb, lsb] = core::mem::replace(&mut state.bank, [Held::NONE; 2]);
                let bank = match [msb.get(), lsb.get()] {
                    [None, None] => None,
                    [msb, lsb] => Some((msb.unwrap_or(0), lsb.unwrap_or(0))),
                };
                Midi2ProgramChange::try_new(m.group(), m.channel(), m.program(), bank)
                    .ok()
                    .map(|p| midi2(M2::ProgramChange(p)))
            }
            M1::ChannelPressure(m) => Some(midi2(M2::ChannelPressure(
                Midi2ChannelPressure::new()
                    .with_group(m.group())
                    .with_channel(m.channel())
                    .with_pressure(up(m.pressure() as u32, 7, 32)),
            ))),
            M1::PitchBend(m) => Some(midi2(M2::PitchBend(
                Midi2PitchBend::new()
                    .with_group(m.group())
                    .with_channel(m.channel())
                    .with_value(up(m.value() as u32, 14, 32)),
            ))),
        }
    }

    #[cfg(test)]
    fn state_mut(&mut self, channel: Channel) -> &mut ChannelState {
        &mut self.state[channel.index() as usize & 0xF]
    }

    #[cfg(test)]
    /// D.3.3: Control Change, RPN and NRPN.
    fn control_change(&mut self, m: &Midi1ControlChange) -> Option<Ump> {
        let (group, channel, value) = (m.group(), m.channel(), m.data());
        let state = self.state_mut(channel);
        match m.index() {
            index @ (98..=101) => {
                // A new parameter ends the last one: send its held CC 6.
                let out = take_pending(group, channel, state);
                match index {
                    101 => state.rpn[0] = Held::some(value),
                    100 => state.rpn[1] = Held::some(value),
                    99 => state.nrpn[0] = Held::some(value),
                    _ => state.nrpn[1] = Held::some(value),
                }
                state.set_parameter(if index >= 100 {
                    Parameter::Registered
                } else {
                    Parameter::Assignable
                });
                state.data_msb = Held::NONE;
                out
            }
            6 => {
                let out = take_pending(group, channel, state);
                state.data_msb = Held::some(value);
                state.set_pending(true);
                out
            }
            38 => {
                let msb = state.data_msb.get()?;
                state.set_pending(false);
                controller(group, channel, state, msb, value)
            }
            0 => {
                // MIDI 1.0 Detailed Specification 4.2.1, Control Change: "When an MSB is
                // received, the receiver should set its concept of the LSB to zero."
                state.bank = [Held::some(value), Held::some(0)];
                None
            }
            32 => {
                state.bank[1] = Held::some(value);
                None
            }
            index => Midi2ControlChange::try_new(group, channel, index, up(value as u32, 7, 32))
                .ok()
                .map(|c| midi2(Midi2ChannelVoiceMessage::ControlChange(c))),
        }
    }

    /// [`Self::translate`] on words (DEC-018): takes the word of one MIDI 1.0 Channel Voice
    /// packet (Message Type 0x2) and gives the two words of the MIDI 2.0 Channel Voice packet
    /// (Message Type 0x4), with the same rules and the same held values, and no typed value.
    ///
    /// Gives `None` when the word gives no message (a held value or the Null Function, D.3.3),
    /// and when `word` is not a MIDI 1.0 Channel Voice word that the crate decodes. The caller
    /// passes the packets of other Message Types on unchanged (D.3.7). Reserved bits are not
    /// used (2.1.3). Never panics.
    pub fn translate_words(&mut self, word: u32) -> Option<[u32; 2]> {
        if word >> 28 != 0x2 {
            return None;
        }
        // Group and Channel stay; Message Type 0x4 (D.3).
        let head = 0x4000_0000 | (word & 0x0F0F_0000);
        let (d0, d1) = ((word >> 8) & 0x7F, word & 0x7F);
        let message =
            |opcode: u32, b2: u32, b3: u32, w1: u32| Some([head | opcode << 20 | b2 << 8 | b3, w1]);
        match (word >> 20) & 0xF {
            0x8 => message(0x8, d0, 0, up(d1, 7, 16) << 16),
            // Note On with velocity 0 is Note Off with velocity 0x8000 (D.3.1).
            0x9 if d1 == 0 => message(0x8, d0, 0, 0x8000 << 16),
            0x9 => message(0x9, d0, 0, up(d1, 7, 16) << 16),
            0xA => message(0xA, d0, 0, up(d1, 7, 32)),
            0xB => self.control_change_words(head, d0 as u8, d1 as u8),
            0xC => {
                let state = &mut self.state[(word >> 16) as usize & 0xF];
                // D.3.4: the held Bank Select values belong to this Program Change only.
                let [msb, lsb] = core::mem::replace(&mut state.bank, [Held::NONE; 2]);
                match [msb.get(), lsb.get()] {
                    [None, None] => message(0xC, 0, 0, d0 << 24),
                    [msb, lsb] => {
                        let bank = (msb.unwrap_or(0) as u32) << 8 | lsb.unwrap_or(0) as u32;
                        // Option Flag Bank Valid is bit 0 of word 0.
                        message(0xC, 0, 1, d0 << 24 | bank)
                    }
                }
            }
            0xD => message(0xD, 0, 0, up(d0, 7, 32)),
            // MIDI 1.0 Pitch Bend: LSB in the first data byte, MSB in the second.
            0xE => message(0xE, 0, 0, up(d1 << 7 | d0, 14, 32)),
            _ => None,
        }
    }

    /// D.3.3 on words: Control Change, RPN and NRPN; `head` holds Message Type 0x4, Group and
    /// Channel.
    fn control_change_words(&mut self, head: u32, index: u8, value: u8) -> Option<[u32; 2]> {
        let state = &mut self.state[(head >> 16) as usize & 0xF];
        match index {
            98..=101 => {
                // A new parameter ends the last one: send its held CC 6.
                let out = take_pending_words(head, state);
                match index {
                    101 => state.rpn[0] = Held::some(value),
                    100 => state.rpn[1] = Held::some(value),
                    99 => state.nrpn[0] = Held::some(value),
                    _ => state.nrpn[1] = Held::some(value),
                }
                state.set_parameter(if index >= 100 {
                    Parameter::Registered
                } else {
                    Parameter::Assignable
                });
                state.data_msb = Held::NONE;
                out
            }
            6 => {
                let out = take_pending_words(head, state);
                state.data_msb = Held::some(value);
                state.set_pending(true);
                out
            }
            38 => {
                let msb = state.data_msb.get()?;
                state.set_pending(false);
                controller_words(head, state, msb, value)
            }
            0 => {
                // MIDI 1.0 Detailed Specification 4.2.1: an MSB sets the held LSB to zero.
                state.bank = [Held::some(value), Held::some(0)];
                None
            }
            32 => {
                state.bank[1] = Held::some(value);
                None
            }
            index => Some([
                head | 0xB << 20 | (index as u32) << 8,
                up(value as u32, 7, 32),
            ]),
        }
    }
}

/// [`take_pending`] on words; `head` holds Message Type 0x4, Group and Channel.
fn take_pending_words(head: u32, state: &mut ChannelState) -> Option<[u32; 2]> {
    if !state.pending() {
        return None;
    }
    state.set_pending(false);
    controller_words(head, state, state.data_msb.get()?, 0)
}

/// [`controller`] on words: the Registered (opcode 0x2) or Assignable (opcode 0x3) Controller
/// words, or `None` when no valid RPN or NRPN is selected or it is the Null Function (D.3.3).
fn controller_words(head: u32, state: &ChannelState, msb: u8, lsb: u8) -> Option<[u32; 2]> {
    let value = ((msb as u32) << 7) | lsb as u32;
    let (select, opcode) = match state.parameter() {
        Parameter::None => return None,
        Parameter::Registered => (state.rpn, 0x2),
        Parameter::Assignable => (state.nrpn, 0x3),
    };
    let [Some(bank), Some(index)] = select.map(Held::get) else {
        return None;
    };
    if bank == 0x7F && index == 0x7F {
        return None;
    }
    let data = if opcode == 0x2 && zero_extension(index) {
        zero_extend_upscale(value, 14, 32).unwrap_or(0)
    } else {
        up(value, 14, 32)
    };
    Some([
        head | opcode << 20 | (bank as u32) << 8 | index as u32,
        data,
    ])
}

#[cfg(test)]
/// The held CC 6 of `state` as a controller with data LSB 0, if one is pending.
fn take_pending(group: Group, channel: Channel, state: &mut ChannelState) -> Option<Ump> {
    if !state.pending() {
        return None;
    }
    state.set_pending(false);
    controller(group, channel, state, state.data_msb.get()?, 0)
}

#[cfg(test)]
/// The Registered or Assignable Controller for the selected parameter and the 14-bit data
/// `msb`, `lsb`, or `None` when no valid RPN or NRPN is selected or it is the Null Function.
fn controller(
    group: Group,
    channel: Channel,
    state: &ChannelState,
    msb: u8,
    lsb: u8,
) -> Option<Ump> {
    let value = ((msb as u32) << 7) | lsb as u32;
    let (select, registered) = match state.parameter() {
        Parameter::None => return None,
        Parameter::Registered => (state.rpn, true),
        Parameter::Assignable => (state.nrpn, false),
    };
    let [Some(bank), Some(index)] = select.map(Held::get) else {
        return None;
    };
    if bank == 0x7F && index == 0x7F {
        return None;
    }
    let m = if registered {
        let data = if zero_extension(index) {
            zero_extend_upscale(value, 14, 32).unwrap_or(0)
        } else {
            up(value, 14, 32)
        };
        Midi2ChannelVoiceMessage::RegisteredController(
            RegisteredController::try_new(group, channel, bank, index, data).ok()?,
        )
    } else {
        Midi2ChannelVoiceMessage::AssignableController(
            AssignableController::try_new(group, channel, bank, index, up(value, 14, 32)).ok()?,
        )
    };
    Some(midi2(m))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec::Vec;

    use super::*;
    #[cfg(feature = "channel-voice2")]
    use crate::channel_voice2::{
        PerNoteManagement, PerNotePitchBend, RelativeAssignableController,
        RelativeRegisteredController,
    };
    use crate::sysex7::{SysEx7, SysEx7Status};
    use crate::system::{SystemMessage, TimingClock};
    use proptest::prelude::*;

    const G: u8 = 3;
    const C: u8 = 9;

    fn g() -> Group {
        Group::new(G).unwrap()
    }

    fn c() -> Channel {
        Channel::new(C).unwrap()
    }

    fn m1(m: Midi1ChannelVoiceMessage) -> Ump {
        Ump::Midi1ChannelVoice(m)
    }

    fn m2(m: Midi2ChannelVoiceMessage) -> Ump {
        Ump::Midi2ChannelVoice(m)
    }

    fn cc1(index: u8, data: u8) -> Ump {
        m1(Midi1ChannelVoiceMessage::ControlChange(
            Midi1ControlChange::try_new(g(), c(), index, data).unwrap(),
        ))
    }

    fn note_on1(note: u8, velocity: u8) -> Ump {
        m1(Midi1ChannelVoiceMessage::NoteOn(
            Midi1NoteOn::try_new(g(), c(), note, velocity).unwrap(),
        ))
    }

    fn note_on2(note: u8, velocity: u16) -> Ump {
        m2(Midi2ChannelVoiceMessage::NoteOn(
            Midi2NoteOn::try_new(g(), c(), note, velocity, NoteAttribute::None).unwrap(),
        ))
    }

    fn program1(program: u8) -> Ump {
        m1(Midi1ChannelVoiceMessage::ProgramChange(
            Midi1ProgramChange::try_new(g(), c(), program).unwrap(),
        ))
    }

    fn rc2(bank: u8, index: u8, data: u32) -> Ump {
        m2(Midi2ChannelVoiceMessage::RegisteredController(
            RegisteredController::try_new(g(), c(), bank, index, data).unwrap(),
        ))
    }

    fn ac2(bank: u8, index: u8, data: u32) -> Ump {
        m2(Midi2ChannelVoiceMessage::AssignableController(
            AssignableController::try_new(g(), c(), bank, index, data).unwrap(),
        ))
    }

    fn feed(t: &mut Midi1ToMidi2, input: &[Ump]) -> Vec<Ump> {
        input.iter().filter_map(|u| t.translate(u)).collect()
    }

    /// 7.4.2, D.2.1: a MIDI 2.0 Note On whose velocity downscales to 0 gives velocity 1.
    #[test]
    fn to_midi1_note_on_velocity_zero_gives_one() {
        for v in [0x0000, 0x0100, 0x01FF] {
            assert_eq!(to_midi1(&note_on2(60, v)).as_slice(), &[note_on1(60, 1)]);
        }
        assert_eq!(
            to_midi1(&note_on2(60, 0x0200)).as_slice(),
            &[note_on1(60, 1)]
        );
        assert_eq!(
            to_midi1(&note_on2(60, 0x0400)).as_slice(),
            &[note_on1(60, 2)]
        );
    }

    /// D.1.4: downscaling cuts off the low-order bits: 0x1400, 0x8000, 0xAEBA, 0xFFFF give
    /// 0x0A, 0x40, 0x57, 0x7F.
    #[test]
    fn to_midi1_velocity_matches_d_1_4() {
        for (v2, v1) in [
            (0x1400, 0x0A),
            (0x8000, 0x40),
            (0xAEBA, 0x57),
            (0xFFFF, 0x7F),
        ] {
            assert_eq!(to_midi1(&note_on2(1, v2)).as_slice(), &[note_on1(1, v1)]);
        }
    }

    /// D.2.1: Note Off, Note On, Poly Pressure and Control Change keep their kind.
    #[test]
    fn to_midi1_note_poly_pressure_and_control_change() {
        let off = m2(Midi2ChannelVoiceMessage::NoteOff(
            Midi2NoteOff::try_new(g(), c(), 61, 0x8000, NoteAttribute::None).unwrap(),
        ));
        let off1 = m1(Midi1ChannelVoiceMessage::NoteOff(
            Midi1NoteOff::try_new(g(), c(), 61, 0x40).unwrap(),
        ));
        assert_eq!(to_midi1(&off).as_slice(), &[off1]);
        let poly = m2(Midi2ChannelVoiceMessage::PolyPressure(
            Midi2PolyPressure::try_new(g(), c(), 62, 0xFFFF_FFFF).unwrap(),
        ));
        let poly1 = m1(Midi1ChannelVoiceMessage::PolyPressure(
            Midi1PolyPressure::try_new(g(), c(), 62, 0x7F).unwrap(),
        ));
        assert_eq!(to_midi1(&poly).as_slice(), &[poly1]);
        let cc = m2(Midi2ChannelVoiceMessage::ControlChange(
            Midi2ControlChange::try_new(g(), c(), 7, 0x8000_0000).unwrap(),
        ));
        assert_eq!(to_midi1(&cc).as_slice(), &[cc1(7, 0x40)]);
    }

    /// D.2.2: Channel Pressure.
    #[test]
    fn to_midi1_channel_pressure() {
        let p = m2(Midi2ChannelVoiceMessage::ChannelPressure(
            Midi2ChannelPressure::new()
                .with_group(g())
                .with_channel(c())
                .with_pressure(0x1400_0000),
        ));
        let p1 = m1(Midi1ChannelVoiceMessage::ChannelPressure(
            Midi1ChannelPressure::try_new(g(), c(), 0x0A).unwrap(),
        ));
        assert_eq!(to_midi1(&p).as_slice(), &[p1]);
    }

    /// D.2.3: Registered Controller gives CC 101, 100, 6, 38; Assignable Controller gives CC 99,
    /// 98, 6, 38. Index 0 to 31 uses Zero-Extension (M2-115 4.1), others Min-Center-Max.
    #[test]
    fn to_midi1_registered_and_assignable_controllers() {
        // RPN 0/0 Pitch Bend Sensitivity: 2 semitones is 0x0100 in 14 bits, 0x04000000 in 32.
        assert_eq!(
            to_midi1(&rc2(0, 0, 0x0400_0000)).as_slice(),
            &[cc1(101, 0), cc1(100, 0), cc1(6, 2), cc1(38, 0)]
        );
        // Zero-Extension rounds to nearest: one 14-bit step is 0x40000, so 0x0401_FFFF stays
        // 0x0100 and 0x0402_0000 gives 0x0101.
        assert_eq!(
            to_midi1(&rc2(0, 0, 0x0402_0000)).as_slice(),
            &[cc1(101, 0), cc1(100, 0), cc1(6, 2), cc1(38, 1)]
        );
        assert_eq!(
            to_midi1(&rc2(0, 0, 0x0401_FFFF)).as_slice(),
            &[cc1(101, 0), cc1(100, 0), cc1(6, 2), cc1(38, 0)]
        );
        // Index 32 and above: Min-Center-Max cuts off the low bits.
        assert_eq!(
            to_midi1(&rc2(1, 32, 0x0400_3FFF)).as_slice(),
            &[cc1(101, 1), cc1(100, 32), cc1(6, 2), cc1(38, 0)]
        );
        assert_eq!(
            to_midi1(&ac2(5, 6, 0xFFFF_FFFF)).as_slice(),
            &[cc1(99, 5), cc1(98, 6), cc1(6, 0x7F), cc1(38, 0x7F)]
        );
    }

    /// D.2.4: Program Change with Bank Valid gives CC 0, CC 32 and Program Change.
    #[test]
    fn to_midi1_program_change_and_bank() {
        let with_bank = m2(Midi2ChannelVoiceMessage::ProgramChange(
            Midi2ProgramChange::try_new(g(), c(), 5, Some((1, 2))).unwrap(),
        ));
        assert_eq!(
            to_midi1(&with_bank).as_slice(),
            &[cc1(0, 1), cc1(32, 2), program1(5)]
        );
        let without = m2(Midi2ChannelVoiceMessage::ProgramChange(
            Midi2ProgramChange::try_new(g(), c(), 5, None).unwrap(),
        ));
        assert_eq!(to_midi1(&without).as_slice(), &[program1(5)]);
    }

    /// D.2.5: Pitch Bend downscaled to 14 bits; the center 0x80000000 gives 0x2000.
    #[test]
    fn to_midi1_pitch_bend() {
        for (v2, v1) in [(0x8000_0000u32, 0x2000u16), (0, 0), (0xFFFF_FFFF, 0x3FFF)] {
            let p = m2(Midi2ChannelVoiceMessage::PitchBend(
                Midi2PitchBend::new()
                    .with_group(g())
                    .with_channel(c())
                    .with_value(v2),
            ));
            let p1 = m1(Midi1ChannelVoiceMessage::PitchBend(
                Midi1PitchBend::try_new(g(), c(), v1).unwrap(),
            ));
            assert_eq!(to_midi1(&p).as_slice(), &[p1]);
        }
    }

    /// D.2.6, D.3.7: System messages and other packets pass unchanged in both directions.
    #[test]
    fn system_and_other_packets_pass_unchanged() {
        let clock = Ump::System(SystemMessage::TimingClock(TimingClock::new(g())));
        let sysex =
            Ump::SysEx7(SysEx7::try_new(g(), SysEx7Status::Complete, &[0x43, 0x12]).unwrap());
        let (unknown, _) = Ump::parse(&[0x6000_0000]).unwrap();
        let mut t = Midi1ToMidi2::new();
        for u in [clock, sysex, unknown, FILL] {
            assert_eq!(to_midi1(&u).as_slice(), &[u]);
            assert_eq!(t.translate(&u), Some(u));
        }
        // A MIDI 1.0 message passes to_midi1 unchanged; a MIDI 2.0 message passes translate.
        assert_eq!(to_midi1(&cc1(7, 1)).as_slice(), &[cc1(7, 1)]);
        assert_eq!(t.translate(&note_on2(1, 2)), Some(note_on2(1, 2)));
    }

    /// D.2.8: relative controllers and per-note messages have no MIDI 1.0 equivalent.
    #[cfg(feature = "channel-voice2")]
    #[test]
    fn to_midi1_untranslatable_messages_give_nothing() {
        let messages = [
            m2(Midi2ChannelVoiceMessage::RelativeRegisteredController(
                RelativeRegisteredController::try_new(g(), c(), 0, 0, 1).unwrap(),
            )),
            m2(Midi2ChannelVoiceMessage::RelativeAssignableController(
                RelativeAssignableController::try_new(g(), c(), 0, 0, -1).unwrap(),
            )),
            m2(Midi2ChannelVoiceMessage::PerNotePitchBend(
                PerNotePitchBend::try_new(g(), c(), 60, 0).unwrap(),
            )),
            m2(Midi2ChannelVoiceMessage::PerNoteManagement(
                PerNoteManagement::try_new(g(), c(), 60, true, false).unwrap(),
            )),
        ];
        for u in messages {
            let out = to_midi1(&u);
            assert!(out.is_empty());
            assert_eq!(out.len(), 0);
            assert_eq!(out.into_iter().count(), 0);
        }
        let (pnc, _) = Ump::parse(&[0x4009_3C07, 0]).unwrap();
        assert!(to_midi1(&pnc).is_empty());
        let (anc, _) = Ump::parse(&[0x4019_3C07, 0]).unwrap();
        assert!(to_midi1(&anc).is_empty());
    }

    /// D.3.1: MIDI 1.0 Note On with velocity 0 gives MIDI 2.0 Note Off with velocity 0x8000.
    #[test]
    fn translate_note_on_velocity_zero_gives_note_off_8000() {
        let mut t = Midi1ToMidi2::new();
        let off = m2(Midi2ChannelVoiceMessage::NoteOff(
            Midi2NoteOff::try_new(g(), c(), 60, 0x8000, NoteAttribute::None).unwrap(),
        ));
        assert_eq!(t.translate(&note_on1(60, 0)), Some(off));
    }

    /// D.1.3, D.3.1: velocity upscaled by Min-Center-Max (0x0A, 0x40, 0x57, 0x7F give 0x1400,
    /// 0x8000, 0xAEBA, 0xFFFF; 0x01 gives 0x0200), Attribute Type 0 and data 0.
    #[test]
    fn translate_velocity_matches_d_1_3_and_d_3_1() {
        let mut t = Midi1ToMidi2::new();
        for (v1, v2) in [
            (0x0A, 0x1400),
            (0x40, 0x8000),
            (0x57, 0xAEBA),
            (0x7F, 0xFFFF),
            (0x01, 0x0200),
        ] {
            let out = t.translate(&note_on1(64, v1)).unwrap();
            assert_eq!(out, note_on2(64, v2));
            let mut words = [0u32; 2];
            out.write(&mut words).unwrap();
            assert_eq!(words[0] & 0xFF, 0, "Attribute Type");
            assert_eq!(words[1] & 0xFFFF, 0, "Attribute Data");
        }
        let off1 = m1(Midi1ChannelVoiceMessage::NoteOff(
            Midi1NoteOff::try_new(g(), c(), 5, 0x7F).unwrap(),
        ));
        let off2 = m2(Midi2ChannelVoiceMessage::NoteOff(
            Midi2NoteOff::try_new(g(), c(), 5, 0xFFFF, NoteAttribute::None).unwrap(),
        ));
        assert_eq!(t.translate(&off1), Some(off2));
    }

    /// D.3.2: Poly Pressure upscaled to 32 bits.
    #[test]
    fn translate_poly_pressure() {
        let mut t = Midi1ToMidi2::new();
        let p1 = m1(Midi1ChannelVoiceMessage::PolyPressure(
            Midi1PolyPressure::try_new(g(), c(), 60, 0x40).unwrap(),
        ));
        let p2 = m2(Midi2ChannelVoiceMessage::PolyPressure(
            Midi2PolyPressure::try_new(g(), c(), 60, 0x8000_0000).unwrap(),
        ));
        assert_eq!(t.translate(&p1), Some(p2));
    }

    fn cc2(index: u8, data: u32) -> Ump {
        m2(Midi2ChannelVoiceMessage::ControlChange(
            Midi2ControlChange::try_new(g(), c(), index, data).unwrap(),
        ))
    }

    /// D.3.3: a 14-bit Control Change pair stays two Control Changes.
    #[test]
    fn translate_14_bit_cc_pair_stays_two() {
        let mut t = Midi1ToMidi2::new();
        assert_eq!(
            feed(&mut t, &[cc1(1, 0x40), cc1(33, 0x7F)]),
            [cc2(1, 0x8000_0000), cc2(33, 0xFFFF_FFFF)]
        );
    }

    /// D.3.3: Data Increment (CC 96) and Decrement (CC 97) stay Control Changes.
    #[test]
    fn translate_increment_decrement_stay_cc() {
        let mut t = Midi1ToMidi2::new();
        let out = feed(&mut t, &[cc1(101, 0), cc1(100, 0), cc1(96, 1), cc1(97, 0)]);
        assert_eq!(out, [cc2(96, 0x0200_0000), cc2(97, 0)]);
    }

    /// D.3.3: CC 6, 98 to 101 are held, and a complete RPN or NRPN gives one controller.
    #[test]
    fn translate_rpn_and_nrpn_give_one_controller() {
        let mut t = Midi1ToMidi2::new();
        // RPN 0/0 (Pitch Bend Sensitivity, Index < 32): Zero-Extension, 2 semitones.
        let out = feed(&mut t, &[cc1(101, 0), cc1(100, 0), cc1(6, 2), cc1(38, 0)]);
        assert_eq!(out, [rc2(0, 0, 0x0400_0000)]);
        // RPN with Index 32 and above: Min-Center-Max.
        let out = feed(
            &mut t,
            &[cc1(101, 1), cc1(100, 40), cc1(6, 0x7F), cc1(38, 0x7F)],
        );
        assert_eq!(out, [rc2(1, 40, 0xFFFF_FFFF)]);
        // NRPN: Assignable Controller, Min-Center-Max.
        let out = feed(&mut t, &[cc1(99, 3), cc1(98, 4), cc1(6, 0x40), cc1(38, 0)]);
        assert_eq!(out, [ac2(3, 4, 0x8000_0000)]);
        // CC 6 or 38 without a selected parameter is not translated.
        let mut t = Midi1ToMidi2::new();
        assert!(feed(&mut t, &[cc1(6, 1), cc1(38, 1), cc1(6, 2)]).is_empty());
        assert_eq!(t.flush(g(), c()), None);
        // An incomplete selection (CC 101 only) is not a valid RPN.
        assert!(feed(&mut t, &[cc1(101, 0), cc1(6, 1), cc1(38, 0)]).is_empty());
    }

    /// D.3.3: the RPN is sent on CC 38, on a following CC 6, or on a CC 98 to 101 that starts a
    /// new one; flush sends a held CC 6 for the timeout of the D.3.3 note.
    #[test]
    fn translate_rpn_send_triggers_and_flush() {
        let mut t = Midi1ToMidi2::new();
        // A following CC 6 sends the held one with data LSB 0.
        let out = feed(
            &mut t,
            &[cc1(101, 0), cc1(100, 1), cc1(6, 0x40), cc1(6, 0x41)],
        );
        assert_eq!(
            out,
            [rc2(0, 1, zero_extend_upscale(0x40 << 7, 14, 32).unwrap())]
        );
        // A new parameter sends the held CC 6.
        let out = feed(&mut t, &[cc1(99, 0)]);
        assert_eq!(
            out,
            [rc2(0, 1, zero_extend_upscale(0x41 << 7, 14, 32).unwrap())]
        );
        // CC 38 sends at once; a later CC 38 sends again with the same MSB.
        let out = feed(&mut t, &[cc1(98, 2), cc1(6, 1), cc1(38, 2), cc1(38, 3)]);
        assert_eq!(
            out,
            [ac2(0, 2, up(0x82, 14, 32)), ac2(0, 2, up(0x83, 14, 32))]
        );
        // flush sends a held CC 6 once.
        assert!(feed(&mut t, &[cc1(6, 5)]).is_empty());
        assert_eq!(t.flush(g(), c()), Some(ac2(0, 2, up(5 << 7, 14, 32))));
        assert_eq!(t.flush(g(), c()), None);
        // Other Groups and Channels hold their own values.
        assert_eq!(t.flush(Group::new(0).unwrap(), c()), None);
        t.reset();
        assert!(feed(&mut t, &[cc1(6, 5), cc1(38, 0)]).is_empty());
    }

    /// D.3.3: the RPN/NRPN Null Function (0x7F, 0x7F) is not translated.
    #[test]
    fn translate_null_function_is_not_sent() {
        let mut t = Midi1ToMidi2::new();
        assert!(feed(
            &mut t,
            &[cc1(101, 0x7F), cc1(100, 0x7F), cc1(6, 1), cc1(38, 1)]
        )
        .is_empty());
        assert!(feed(
            &mut t,
            &[cc1(99, 0x7F), cc1(98, 0x7F), cc1(6, 1), cc1(6, 2)]
        )
        .is_empty());
        assert_eq!(t.flush(g(), c()), None);
    }

    /// D.3.3, D.3.4: CC 0 and 32 are not translated alone; a Program Change carries them.
    #[test]
    fn translate_program_change_carries_bank_select() {
        let mut t = Midi1ToMidi2::new();
        let without = m2(Midi2ChannelVoiceMessage::ProgramChange(
            Midi2ProgramChange::try_new(g(), c(), 9, None).unwrap(),
        ));
        assert_eq!(feed(&mut t, &[program1(9)]), [without]);
        let with = m2(Midi2ChannelVoiceMessage::ProgramChange(
            Midi2ProgramChange::try_new(g(), c(), 9, Some((1, 2))).unwrap(),
        ));
        assert_eq!(feed(&mut t, &[cc1(0, 1), cc1(32, 2), program1(9)]), [with]);
        // A later Program Change alone keeps the bank: Bank Valid 0 (7.4.9), and the round
        // trip gives the original messages (D.1.2).
        assert_eq!(feed(&mut t, &[program1(9)]), [without]);
        let two = feed(&mut t, &[cc1(0, 1), cc1(32, 2), program1(9), program1(10)]);
        let back: Vec<Ump> = two.iter().flat_map(to_midi1).collect();
        assert_eq!(back, [cc1(0, 1), cc1(32, 2), program1(9), program1(10)]);
        let mut t = Midi1ToMidi2::new();
        let msb_only = m2(Midi2ChannelVoiceMessage::ProgramChange(
            Midi2ProgramChange::try_new(g(), c(), 9, Some((1, 0))).unwrap(),
        ));
        assert_eq!(feed(&mut t, &[cc1(0, 1), program1(9)]), [msb_only]);
        // A new MSB sets the held LSB to zero (MIDI 1.0 4.2.1, Control Change).
        assert_eq!(
            feed(&mut t, &[cc1(0, 1), cc1(32, 2), cc1(0, 1), program1(9)]),
            [msb_only]
        );
    }

    /// D.3.5: Channel Pressure upscaled to 32 bits.
    #[test]
    fn translate_channel_pressure() {
        let mut t = Midi1ToMidi2::new();
        let p1 = m1(Midi1ChannelVoiceMessage::ChannelPressure(
            Midi1ChannelPressure::try_new(g(), c(), 0x7F).unwrap(),
        ));
        let p2 = m2(Midi2ChannelVoiceMessage::ChannelPressure(
            Midi2ChannelPressure::new()
                .with_group(g())
                .with_channel(c())
                .with_pressure(0xFFFF_FFFF),
        ));
        assert_eq!(t.translate(&p1), Some(p2));
    }

    /// D.3.6, D.1.2: Pitch Bend 14 bits to 32 bits; center 0x2000 gives 0x80000000.
    #[test]
    fn translate_pitch_bend_and_centers() {
        let mut t = Midi1ToMidi2::new();
        for (v1, v2) in [(0x2000u16, 0x8000_0000u32), (0, 0), (0x3FFF, 0xFFFF_FFFF)] {
            let p1 = m1(Midi1ChannelVoiceMessage::PitchBend(
                Midi1PitchBend::try_new(g(), c(), v1).unwrap(),
            ));
            let p2 = m2(Midi2ChannelVoiceMessage::PitchBend(
                Midi2PitchBend::new()
                    .with_group(g())
                    .with_channel(c())
                    .with_value(v2),
            ));
            assert_eq!(t.translate(&p1), Some(p2));
        }
        // D.1.2 centers: 0x40 gives 0x8000 (16 bits) and 0x80000000 (32 bits).
        assert_eq!(t.translate(&note_on1(1, 0x40)), Some(note_on2(1, 0x8000)));
        assert_eq!(t.translate(&cc1(7, 0x40)), Some(cc2(7, 0x8000_0000)));
    }

    /// The MIDI 1.0 Channel Voice messages that translate one to one.
    fn midi1_message() -> impl Strategy<Value = Ump> {
        let gc = (0u8..16, 0u8..16);
        prop_oneof![
            (gc.clone(), 0u8..128, 0u8..128).prop_map(|((gr, ch), n, v)| m1(
                Midi1ChannelVoiceMessage::NoteOff(
                    Midi1NoteOff::try_new(Group::new(gr).unwrap(), Channel::new(ch).unwrap(), n, v)
                        .unwrap()
                )
            )),
            (gc.clone(), 0u8..128, 1u8..128).prop_map(|((gr, ch), n, v)| m1(
                Midi1ChannelVoiceMessage::NoteOn(
                    Midi1NoteOn::try_new(Group::new(gr).unwrap(), Channel::new(ch).unwrap(), n, v)
                        .unwrap()
                )
            )),
            (gc.clone(), 0u8..128, 0u8..128).prop_map(|((gr, ch), n, v)| m1(
                Midi1ChannelVoiceMessage::PolyPressure(
                    Midi1PolyPressure::try_new(
                        Group::new(gr).unwrap(),
                        Channel::new(ch).unwrap(),
                        n,
                        v
                    )
                    .unwrap()
                )
            )),
            (gc.clone(), 0u8..128, 0u8..128)
                .prop_filter("held controllers", |(_, i, _)| !matches!(
                    i,
                    0 | 6 | 32 | 38 | 98..=101
                ))
                .prop_map(
                    |((gr, ch), i, v)| m1(Midi1ChannelVoiceMessage::ControlChange(
                        Midi1ControlChange::try_new(
                            Group::new(gr).unwrap(),
                            Channel::new(ch).unwrap(),
                            i,
                            v
                        )
                        .unwrap()
                    ))
                ),
            (gc.clone(), 0u8..128).prop_map(|((gr, ch), p)| m1(
                Midi1ChannelVoiceMessage::ProgramChange(
                    Midi1ProgramChange::try_new(
                        Group::new(gr).unwrap(),
                        Channel::new(ch).unwrap(),
                        p
                    )
                    .unwrap()
                )
            )),
            (gc.clone(), 0u8..128).prop_map(|((gr, ch), p)| m1(
                Midi1ChannelVoiceMessage::ChannelPressure(
                    Midi1ChannelPressure::try_new(
                        Group::new(gr).unwrap(),
                        Channel::new(ch).unwrap(),
                        p
                    )
                    .unwrap()
                )
            )),
            (gc, 0u16..0x4000).prop_map(|((gr, ch), v)| m1(Midi1ChannelVoiceMessage::PitchBend(
                Midi1PitchBend::try_new(Group::new(gr).unwrap(), Channel::new(ch).unwrap(), v)
                    .unwrap()
            ))),
        ]
    }

    /// D.1.1, D.1.2: MIDI 1.0 to MIDI 2.0 and back gives the original MIDI 1.0 data. (Note On
    /// with velocity 0 is excluded: D.3.1 makes it a Note Off.)
    #[test]
    fn prop_midi1_to_midi2_round_trip() {
        proptest!(|(u in midi1_message())| {
            let mut t = Midi1ToMidi2::new();
            let two = t.translate(&u).unwrap();
            prop_assert!(matches!(two, Ump::Midi2ChannelVoice(_)));
            let back = to_midi1(&two);
            prop_assert_eq!(back.as_slice(), &[u]);
        });
    }

    /// D.1.2: an RPN or NRPN sequence and a Bank Select with Program Change survive the round
    /// trip through MIDI 2.0.
    #[test]
    fn prop_rpn_nrpn_and_bank_round_trip() {
        proptest!(|(rpn in any::<bool>(), bank in 0u8..128, index in 0u8..128, msb in 0u8..128, lsb in 0u8..128, program in 0u8..128)| {
            prop_assume!(!(bank == 0x7F && index == 0x7F));
            let select = if rpn { [101, 100] } else { [99, 98] };
            let sequence = [cc1(select[0], bank), cc1(select[1], index), cc1(6, msb), cc1(38, lsb)];
            let mut t = Midi1ToMidi2::new();
            let two = feed(&mut t, &sequence);
            prop_assert_eq!(two.len(), 1);
            let back = to_midi1(&two[0]);
            prop_assert_eq!(back.as_slice(), &sequence[..]);
            let sequence = [cc1(0, bank), cc1(32, index), program1(program)];
            let two = feed(&mut t, &sequence);
            prop_assert_eq!(two.len(), 1);
            let back = to_midi1(&two[0]);
            prop_assert_eq!(back.as_slice(), &sequence[..]);
        });
    }

    /// No packet makes either direction panic, and to_midi1 gives at most 4 UMPs.
    #[test]
    fn prop_translation_never_panics() {
        proptest!(|(packets in proptest::collection::vec(any::<[u32; 4]>(), 0..32))| {
            let mut t = Midi1ToMidi2::new();
            for words in &packets {
                if let Ok((u, _)) = Ump::parse(words) {
                    let out = to_midi1(&u);
                    prop_assert!(out.len() <= 4);
                    let _ = t.translate(&u);
                    let _ = t.flush(Group::from_bits(words[0] as u8), Channel::from_bits(words[1] as u8));
                }
            }
        });
    }

    /// Any MIDI 1.0 Channel Voice message of one Group, the held controllers included.
    fn midi1_any_one_group() -> impl Strategy<Value = Ump> {
        (0u8..7, 0u8..16, 0u8..128, 0u8..128, 0u16..0x4000).prop_map(|(kind, ch, a, b, bend)| {
            let (gr, ch) = (g(), Channel::new(ch).unwrap());
            m1(match kind {
                0 => {
                    Midi1ChannelVoiceMessage::NoteOff(Midi1NoteOff::try_new(gr, ch, a, b).unwrap())
                }
                1 => Midi1ChannelVoiceMessage::NoteOn(Midi1NoteOn::try_new(gr, ch, a, b).unwrap()),
                2 => Midi1ChannelVoiceMessage::PolyPressure(
                    Midi1PolyPressure::try_new(gr, ch, a, b).unwrap(),
                ),
                // Half of the Control Changes are the held controllers of D.3.3 and D.3.4.
                3 => {
                    let held = [0, 6, 32, 38, 98, 99, 100, 101];
                    let index = if a < 64 { held[a as usize % 8] } else { a };
                    Midi1ChannelVoiceMessage::ControlChange(
                        Midi1ControlChange::try_new(gr, ch, index, b).unwrap(),
                    )
                }
                4 => Midi1ChannelVoiceMessage::ProgramChange(
                    Midi1ProgramChange::try_new(gr, ch, a).unwrap(),
                ),
                5 => Midi1ChannelVoiceMessage::ChannelPressure(
                    Midi1ChannelPressure::try_new(gr, ch, a).unwrap(),
                ),
                _ => Midi1ChannelVoiceMessage::PitchBend(
                    Midi1PitchBend::try_new(gr, ch, bend).unwrap(),
                ),
            })
        })
    }

    /// OPT-001: for the messages of one Group, Midi1ToMidi2Group gives the same UMPs and the same
    /// flush results as Midi1ToMidi2 (D.3.3, D.3.4).
    #[test]
    fn prop_group_translator_matches_midi1_to_midi2() {
        proptest!(|(input in proptest::collection::vec(midi1_any_one_group(), 0..64), ch in 0u8..16)| {
            let mut all = Midi1ToMidi2::new();
            let mut one = Midi1ToMidi2Group::new();
            for u in &input {
                prop_assert_eq!(one.translate(u), all.translate(u));
            }
            let ch = Channel::new(ch).unwrap();
            prop_assert_eq!(one.flush(g(), ch), all.flush(g(), ch));
            one.reset();
            prop_assert_eq!(one.flush(g(), ch), None);
        });
    }

    /// DEC-018: translate_words, and translate and flush on top of it, give the results of the
    /// typed reference, with the same held values, for MIDI 1.0 words with any Group, any
    /// reserved bits and any opcode (D.3, 2.1.3).
    #[test]
    fn prop_translate_words_equals_typed_translate() {
        let word = prop_oneof![
            (midi1_any_one_group(), any::<u32>())
                .prop_map(|(u, noise)| u.to_packet().words()[0] ^ (noise & 0x0F00_8080)),
            any::<u32>().prop_map(|w| 0x2000_0000 | (w & 0x0FFF_FFFF)),
        ];
        proptest!(|(input in proptest::collection::vec(word, 0..64), ch in 0u8..16)| {
            let mut typed = Midi1ToMidi2Group::new();
            let (mut words, mut wrapped) = (typed, typed);
            for &w in &input {
                let parsed = Midi1ChannelVoiceMessage::parse(&[w]).map(m1);
                let expected = parsed.and_then(|u| typed.translate_typed(&u));
                let got = words.translate_words(w);
                prop_assert_eq!(
                    got,
                    expected.map(|u| <[u32; 2]>::try_from(u.to_packet().words()).unwrap())
                );
                if let Some(u) = parsed {
                    // The typed API on top of the words gives the typed reference (DEC-018).
                    prop_assert_eq!(wrapped.translate(&u), expected);
                }
            }
            let ch = Channel::new(ch).unwrap();
            let expected = typed.flush_typed(g(), ch);
            prop_assert_eq!(words.flush(g(), ch), expected);
            prop_assert_eq!(wrapped.flush(g(), ch), expected);
        });
    }

    /// Midi1ToMidi2Group: each output UMP has the Group of its input, and other packets pass
    /// unchanged (D.3.7).
    #[test]
    fn empty_translator_is_all_zero_bits() {
        // OPT-013: every field of an empty channel state is 0, and the state is 8 bytes of u8
        // with no padding, so `Midi1ToMidi2::new()` in a `static` goes to `.bss`, not `.data`.
        let e = ChannelState::EMPTY;
        let bytes = [
            e.rpn[0].0,
            e.rpn[1].0,
            e.nrpn[0].0,
            e.nrpn[1].0,
            e.data_msb.0,
            e.bank[0].0,
            e.bank[1].0,
            e.flags,
        ];
        assert_eq!(bytes, [0; 8]);
        assert_eq!(core::mem::size_of::<ChannelState>(), 8);
        assert_eq!(core::mem::size_of::<Midi1ToMidi2Group>(), 16 * 8);
        // Every 7-bit value is kept, and 0 is a value, not "no value".
        for v in 0..=0x7F {
            assert_eq!(Held::some(v).get(), Some(v));
        }
        assert_eq!(Held::NONE.get(), None);
    }

    #[test]
    fn group_translator_keeps_the_input_group() {
        let mut t = Midi1ToMidi2Group::default();
        let out = feed_group(&mut t, &[cc1(101, 0), cc1(100, 0), cc1(6, 2), cc1(38, 0)]);
        assert_eq!(out, [rc2(0, 0, 0x0400_0000)]);
        assert!(feed_group(&mut t, &[cc1(6, 5)]).is_empty());
        assert_eq!(
            t.flush(g(), c()),
            Some(rc2(0, 0, zero_extend_upscale(5 << 7, 14, 32).unwrap()))
        );
        let clock = Ump::System(SystemMessage::TimingClock(TimingClock::new(g())));
        assert_eq!(t.translate(&clock), Some(clock));
        assert_eq!(t.translate(&note_on2(1, 2)), Some(note_on2(1, 2)));
    }

    fn feed_group(t: &mut Midi1ToMidi2Group, input: &[Ump]) -> Vec<Ump> {
        input.iter().filter_map(|u| t.translate(u)).collect()
    }

    #[cfg(all(feature = "channel-voice1", feature = "channel-voice2"))]
    #[test]
    fn prop_midi2_to_midi1_equals_typed_translation() {
        // DEC-018: the word path gives the same MIDI 1.0 messages as the typed reference, for
        // every MIDI 2.0 Channel Voice packet, and None exactly where the typed path passes the
        // packet on unchanged.
        proptest!(ProptestConfig::with_cases(20_000), |(w0 in any::<u32>(), w1 in any::<u32>())| {
            let words = [0x4000_0000 | (w0 & 0x0FFF_FFFF), w1];
            let ump = Ump::parse(&words).unwrap().0;
            let typed = reference_to_midi1(&ump);
            prop_assert_eq!(to_midi1(&ump), typed);
            match (ump, midi2_to_midi1(&words)) {
                (Ump::Midi2ChannelVoice(_), Some(m)) => {
                    let got: std::vec::Vec<Ump> = m
                        .as_slice()
                        .iter()
                        .map(|w| Ump::parse(&[*w]).unwrap().0)
                        .collect();
                    prop_assert_eq!(got.as_slice(), typed.as_slice());
                }
                (Ump::Midi2ChannelVoice(_), None) => prop_assert!(false, "None for {:08x?}", words),
                (_, r) => prop_assert_eq!(r, None),
            }
        });
    }

    #[test]
    fn prop_to_midi1_passes_other_message_types_unchanged() {
        // D.2.6: every UMP that is not MIDI 2.0 Channel Voice passes unchanged, in both paths.
        proptest!(|(w in proptest::array::uniform4(any::<u32>()))| {
            prop_assume!(w[0] >> 28 != 0x4);
            let ump = Ump::parse(&w).unwrap().0;
            prop_assert_eq!(to_midi1(&ump), reference_to_midi1(&ump));
            prop_assert_eq!(midi2_to_midi1(&w), None);
        });
    }
}
