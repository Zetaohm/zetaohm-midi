//! UMP functions and the parsed UMP class (zmidi-ump): word count (M2-104 Table 4), parse, MIDI 2.0
//! Channel Voice writers (7.4), MIDI 2.0 to 1.0 translation (Appendix D.2) and scaling (M2-115).

use pyo3::prelude::*;
use zmidi_ump::scale::{downscale as rs_downscale, upscale as rs_upscale};
use zmidi_ump::{
    midi2_to_midi1 as rs_midi2_to_midi1, Channel, Group, MessageType, Midi2ChannelPressure,
    Midi2ControlChange, Midi2NoteOff, Midi2NoteOn, Midi2PitchBend, Midi2ProgramChange,
    NoteAttribute, Ump as RsUmp, UmpError, UmpWords,
};

use crate::{invalid, malformed};

/// One UMP as plain values; `words` holds the words of the UMP.
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ump {
    /// Message Type, 0x0 to 0xF.
    #[pyo3(get)]
    message_type: u8,
    /// Group, when the Message Type has one.
    #[pyo3(get)]
    group: Option<u8>,
    /// Opcode or status nibble (Message Types 0x0, 0x2 to 0x5) or status byte (0x1).
    #[pyo3(get)]
    status: Option<u8>,
    /// Channel, when the Message Type has one.
    #[pyo3(get)]
    channel: Option<u8>,
    /// True when this build decodes the Message Type.
    #[pyo3(get)]
    decoded: bool,
    /// The words of the UMP.
    #[pyo3(get)]
    words: Vec<u32>,
}

#[pymethods]
impl Ump {
    fn __repr__(&self) -> String {
        let w: Vec<String> = self.words.iter().map(|w| format!("0x{w:08X}")).collect();
        format!(
            "Ump(message_type=0x{:X}, words=[{}])",
            self.message_type,
            w.join(", ")
        )
    }

    fn __len__(&self) -> usize {
        self.words.len()
    }
}

/// Gives the word count of the UMP whose first word is `word0` (M2-104 Table 4).
#[pyfunction]
pub fn word_count(word0: u32) -> usize {
    MessageType::from_bits((word0 >> 28) as u8).word_count()
}

/// Reads the UMP at the start of `words`. Raises `MalformedError` when `words` is shorter than
/// the UMP.
#[pyfunction]
pub fn parse(words: Vec<u32>) -> PyResult<Ump> {
    let (ump, len) = RsUmp::parse(&words).map_err(|e| malformed(e.to_string()))?;
    let w0 = words[0];
    let mt = (w0 >> 28) as u8;
    let status = if mt == 0x1 {
        (w0 >> 16) as u8
    } else {
        ((w0 >> 20) & 0xF) as u8
    };
    Ok(Ump {
        message_type: mt,
        group: matches!(mt, 0x1..=0x5 | 0xD).then_some(((w0 >> 24) & 0xF) as u8),
        status: matches!(mt, 0x0..=0x5).then_some(status),
        channel: matches!(mt, 0x2 | 0x4).then_some(((w0 >> 16) & 0xF) as u8),
        decoded: !matches!(ump, RsUmp::Unknown(_)),
        words: words[..len].to_vec(),
    })
}

fn gc(group: u8, channel: u8) -> PyResult<(Group, Channel)> {
    let g = Group::new(group).map_err(|_| invalid("group must be 0 to 15"))?;
    let c = Channel::new(channel).map_err(|_| invalid("channel must be 0 to 15"))?;
    Ok((g, c))
}

fn words2(m: Result<impl UmpWords, UmpError>) -> PyResult<Vec<u32>> {
    let m = m.map_err(|e| invalid(e.to_string()))?;
    let mut w = [0u32; 2];
    m.to_words(&mut w).map_err(|e| invalid(e.to_string()))?;
    Ok(w.to_vec())
}

/// The two words of a MIDI 2.0 Note On (M2-104 7.4.2) with no attribute.
#[pyfunction]
pub fn midi2_note_on(group: u8, channel: u8, note: u8, velocity: u16) -> PyResult<Vec<u32>> {
    let (g, c) = gc(group, channel)?;
    words2(Midi2NoteOn::try_new(
        g,
        c,
        note,
        velocity,
        NoteAttribute::None,
    ))
}

/// The two words of a MIDI 2.0 Note Off (M2-104 7.4.1) with no attribute.
#[pyfunction]
pub fn midi2_note_off(group: u8, channel: u8, note: u8, velocity: u16) -> PyResult<Vec<u32>> {
    let (g, c) = gc(group, channel)?;
    words2(Midi2NoteOff::try_new(
        g,
        c,
        note,
        velocity,
        NoteAttribute::None,
    ))
}

/// The two words of a MIDI 2.0 Control Change (M2-104 7.4.6).
#[pyfunction]
pub fn midi2_control_change(group: u8, channel: u8, index: u8, data: u32) -> PyResult<Vec<u32>> {
    let (g, c) = gc(group, channel)?;
    words2(Midi2ControlChange::try_new(g, c, index, data))
}

/// The two words of a MIDI 2.0 Program Change (M2-104 7.4.9) with an optional Bank (MSB, LSB).
#[pyfunction]
#[pyo3(signature = (group, channel, program, bank=None))]
pub fn midi2_program_change(
    group: u8,
    channel: u8,
    program: u8,
    bank: Option<(u8, u8)>,
) -> PyResult<Vec<u32>> {
    let (g, c) = gc(group, channel)?;
    words2(Midi2ProgramChange::try_new(g, c, program, bank))
}

/// The two words of a MIDI 2.0 Channel Pressure (M2-104 7.4.10).
#[pyfunction]
pub fn midi2_channel_pressure(group: u8, channel: u8, value: u32) -> PyResult<Vec<u32>> {
    let (g, c) = gc(group, channel)?;
    words2(Ok::<_, UmpError>(
        Midi2ChannelPressure::new()
            .with_group(g)
            .with_channel(c)
            .with_pressure(value),
    ))
}

/// The two words of a MIDI 2.0 Pitch Bend (M2-104 7.4.11); the center is 0x80000000.
#[pyfunction]
pub fn midi2_pitch_bend(group: u8, channel: u8, value: u32) -> PyResult<Vec<u32>> {
    let (g, c) = gc(group, channel)?;
    words2(Ok::<_, UmpError>(
        Midi2PitchBend::new()
            .with_group(g)
            .with_channel(c)
            .with_value(value),
    ))
}

/// The MIDI 1.0 Channel Voice words of a MIDI 2.0 Channel Voice UMP (M2-104 Appendix D.2), or
/// `None` when zmidi-ump does not translate it. A message with no MIDI 1.0 form gives `[]`.
#[pyfunction]
pub fn midi2_to_midi1(words: Vec<u32>) -> Option<Vec<u32>> {
    rs_midi2_to_midi1(&words).map(|t| t.as_slice().to_vec())
}

/// Scales `value` from `src_bits` up to `dst_bits` (M2-115 Min-Center-Max).
#[pyfunction]
pub fn upscale(value: u32, src_bits: u8, dst_bits: u8) -> PyResult<u32> {
    rs_upscale(value, src_bits, dst_bits).map_err(|e| invalid(e.to_string()))
}

/// Scales `value` from `src_bits` down to `dst_bits` (M2-115).
#[pyfunction]
pub fn downscale(value: u32, src_bits: u8, dst_bits: u8) -> PyResult<u32> {
    rs_downscale(value, src_bits, dst_bits).map_err(|e| invalid(e.to_string()))
}
