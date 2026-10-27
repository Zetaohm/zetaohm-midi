//! The C ABI of zmidi-ump for C programs (DEC-019): the word count (REQ-FFI-UMP-001), a flat parse
//! result (REQ-FFI-UMP-002), writers of MIDI 2.0 Channel Voice Messages (REQ-FFI-UMP-003), the
//! MIDI 2.0 to 1.0 translation (REQ-FFI-UMP-004) and the value scaling (REQ-FFI-UMP-005).

use zmidi_ump::scale::{downscale, upscale};
use zmidi_ump::{
    midi2_to_midi1, Channel, Group, MessageType, Midi2ChannelPressure, Midi2ControlChange,
    Midi2NoteOff, Midi2NoteOn, Midi2PitchBend, Midi2ProgramChange, NoteAttribute, Ump, UmpWords,
};

use crate::status::ZmidiStatus;

/// One UMP as plain values (REQ-FFI-UMP-002).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiUmpInfo {
    /// Message Type, 0x0 to 0xF.
    pub message_type: u8,
    /// Words of the UMP, 1 to 4 (M2-104 Table 4).
    pub word_count: u8,
    /// True when the Message Type has a Group field.
    pub has_group: bool,
    /// Group, 0 to 15, when `has_group`.
    pub group: u8,
    /// True when the Message Type has a status field.
    pub has_status: bool,
    /// Status: the opcode or status nibble (Message Types 0x0, 0x2 to 0x5) or the status byte
    /// (Message Type 0x1), when `has_status`.
    pub status: u8,
    /// True when the Message Type has a Channel field.
    pub has_channel: bool,
    /// Channel, 0 to 15, when `has_channel`.
    pub channel: u8,
    /// True when this build decodes the Message Type (zmidi-ump features, DEC-017).
    pub decoded: bool,
    /// The words of the UMP; words after `word_count` are 0.
    pub words: [u32; 4],
}

/// Gives the word count of the UMP whose first word is `word0` (M2-104 Table 4).
#[no_mangle]
pub extern "C" fn zmidi_ump_word_count(word0: u32) -> usize {
    MessageType::from_bits((word0 >> 28) as u8).word_count()
}

/// Reads the UMP at the start of `words` (`word_count` words) into `info`.
///
/// # Safety
/// `words` points to `word_count` words; `info` is writable. Null pointers give
/// [`ZmidiStatus::InvalidArgument`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_parse(
    words: *const u32,
    word_count: usize,
    info: *mut ZmidiUmpInfo,
) -> ZmidiStatus {
    if words.is_null() || info.is_null() || word_count == 0 {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let words = unsafe { core::slice::from_raw_parts(words, word_count) };
    let Ok((ump, len)) = Ump::parse(words) else {
        return ZmidiStatus::InvalidArgument;
    };
    let w0 = words[0];
    let mt = (w0 >> 28) as u8;
    let mut copy = [0u32; 4];
    copy[..len].copy_from_slice(&words[..len]);
    let has_group = matches!(mt, 0x1..=0x5 | 0xD);
    let has_status = matches!(mt, 0x0..=0x5);
    let has_channel = matches!(mt, 0x2 | 0x4);
    let status = if mt == 0x1 {
        (w0 >> 16) as u8
    } else {
        ((w0 >> 20) & 0xF) as u8
    };
    let out = ZmidiUmpInfo {
        message_type: mt,
        word_count: len as u8,
        has_group,
        group: if has_group {
            ((w0 >> 24) & 0xF) as u8
        } else {
            0
        },
        has_status,
        status: if has_status { status } else { 0 },
        has_channel,
        channel: if has_channel {
            ((w0 >> 16) & 0xF) as u8
        } else {
            0
        },
        decoded: !matches!(ump, Ump::Unknown(_)),
        words: copy,
    };
    // SAFETY: caller contract above.
    unsafe { info.write(out) };
    ZmidiStatus::Ok
}

/// Writes the words of `message` into `out` (two words).
///
/// # Safety
/// `out` is null or points to two writable words.
unsafe fn put2(message: Result<impl UmpWords, zmidi_ump::UmpError>, out: *mut u32) -> ZmidiStatus {
    if out.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    let Ok(m) = message else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let out = unsafe { core::slice::from_raw_parts_mut(out, 2) };
    match m.to_words(out) {
        Ok(_) => ZmidiStatus::Ok,
        Err(_) => ZmidiStatus::BufferTooSmall,
    }
}

fn gc(group: u8, channel: u8) -> Result<(Group, Channel), zmidi_ump::UmpError> {
    Ok((Group::new(group)?, Channel::new(channel)?))
}

/// Writes a MIDI 2.0 Note On (M2-104 7.4.2) with no attribute into `out` (two words).
///
/// # Safety
/// `out` is null or points to two writable words.
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_midi2_note_on(
    group: u8,
    channel: u8,
    note: u8,
    velocity: u16,
    out: *mut u32,
) -> ZmidiStatus {
    let m = gc(group, channel)
        .and_then(|(g, c)| Midi2NoteOn::try_new(g, c, note, velocity, NoteAttribute::None));
    // SAFETY: caller contract above.
    unsafe { put2(m, out) }
}

/// Writes a MIDI 2.0 Note Off (M2-104 7.4.1) with no attribute into `out` (two words).
///
/// # Safety
/// `out` is null or points to two writable words.
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_midi2_note_off(
    group: u8,
    channel: u8,
    note: u8,
    velocity: u16,
    out: *mut u32,
) -> ZmidiStatus {
    let m = gc(group, channel)
        .and_then(|(g, c)| Midi2NoteOff::try_new(g, c, note, velocity, NoteAttribute::None));
    // SAFETY: caller contract above.
    unsafe { put2(m, out) }
}

/// Writes a MIDI 2.0 Control Change (M2-104 7.4.6) into `out` (two words).
///
/// # Safety
/// `out` is null or points to two writable words.
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_midi2_control_change(
    group: u8,
    channel: u8,
    index: u8,
    data: u32,
    out: *mut u32,
) -> ZmidiStatus {
    let m = gc(group, channel).and_then(|(g, c)| Midi2ControlChange::try_new(g, c, index, data));
    // SAFETY: caller contract above.
    unsafe { put2(m, out) }
}

/// Writes a MIDI 2.0 Program Change (M2-104 7.4.9), with the Bank `bank_msb`/`bank_lsb` when
/// `has_bank`, into `out` (two words).
///
/// # Safety
/// `out` is null or points to two writable words.
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_midi2_program_change(
    group: u8,
    channel: u8,
    program: u8,
    has_bank: bool,
    bank_msb: u8,
    bank_lsb: u8,
    out: *mut u32,
) -> ZmidiStatus {
    let bank = has_bank.then_some((bank_msb, bank_lsb));
    let m = gc(group, channel).and_then(|(g, c)| Midi2ProgramChange::try_new(g, c, program, bank));
    // SAFETY: caller contract above.
    unsafe { put2(m, out) }
}

/// Writes a MIDI 2.0 Channel Pressure (M2-104 7.4.10) into `out` (two words).
///
/// # Safety
/// `out` is null or points to two writable words.
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_midi2_channel_pressure(
    group: u8,
    channel: u8,
    value: u32,
    out: *mut u32,
) -> ZmidiStatus {
    let m = gc(group, channel).map(|(g, c)| {
        Midi2ChannelPressure::new()
            .with_group(g)
            .with_channel(c)
            .with_pressure(value)
    });
    // SAFETY: caller contract above.
    unsafe { put2(m, out) }
}

/// Writes a MIDI 2.0 Pitch Bend (M2-104 7.4.11) into `out` (two words); the center is 0x80000000.
///
/// # Safety
/// `out` is null or points to two writable words.
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_midi2_pitch_bend(
    group: u8,
    channel: u8,
    value: u32,
    out: *mut u32,
) -> ZmidiStatus {
    let m = gc(group, channel).map(|(g, c)| {
        Midi2PitchBend::new()
            .with_group(g)
            .with_channel(c)
            .with_value(value)
    });
    // SAFETY: caller contract above.
    unsafe { put2(m, out) }
}

/// Translates one MIDI 2.0 Channel Voice UMP (`words`, `word_count` words) to MIDI 1.0 Channel
/// Voice UMPs (M2-104 Appendix D.2) in `out` (`capacity` words) and gives their word count in
/// `count`. A UMP that zmidi-ump does not translate gives [`ZmidiStatus::NotSupported`].
///
/// # Safety
/// `words` points to `word_count` words; `out` points to `capacity` writable words; `count` is
/// writable. Null pointers give [`ZmidiStatus::InvalidArgument`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_midi2_to_midi1(
    words: *const u32,
    word_count: usize,
    out: *mut u32,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if words.is_null() || out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (words, out) = unsafe {
        (
            core::slice::from_raw_parts(words, word_count),
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let Some(translated) = midi2_to_midi1(words) else {
        return ZmidiStatus::NotSupported;
    };
    let t = translated.as_slice();
    let Some(dst) = out.get_mut(..t.len()) else {
        return ZmidiStatus::BufferTooSmall;
    };
    dst.copy_from_slice(t);
    // SAFETY: caller contract above.
    unsafe { count.write(t.len()) };
    ZmidiStatus::Ok
}

/// Scales `value` from `src_bits` up to `dst_bits` (M2-115 Min-Center-Max) into `out`.
///
/// # Safety
/// `out` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_upscale(
    value: u32,
    src_bits: u8,
    dst_bits: u8,
    out: *mut u32,
) -> ZmidiStatus {
    if out.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    match upscale(value, src_bits, dst_bits) {
        Ok(v) => {
            // SAFETY: caller contract above.
            unsafe { out.write(v) };
            ZmidiStatus::Ok
        }
        Err(_) => ZmidiStatus::InvalidArgument,
    }
}

/// Scales `value` from `src_bits` down to `dst_bits` (M2-115) into `out`.
///
/// # Safety
/// `out` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_downscale(
    value: u32,
    src_bits: u8,
    dst_bits: u8,
    out: *mut u32,
) -> ZmidiStatus {
    if out.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    match downscale(value, src_bits, dst_bits) {
        Ok(v) => {
            // SAFETY: caller contract above.
            unsafe { out.write(v) };
            ZmidiStatus::Ok
        }
        Err(_) => ZmidiStatus::InvalidArgument,
    }
}
