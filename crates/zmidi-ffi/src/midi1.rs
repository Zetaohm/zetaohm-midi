//! The C ABI of the MIDI 1.0 parts of zmidi-io (DEC-019, DEC-022): the typed byte reader and writer
//! (feature `messages`, REQ-FFI-MIDI1-002, -003), the receiver mode state (`channel-mode`,
//! REQ-FFI-MIDI1-004, -009), the controller state (`controllers`, REQ-FFI-MIDI1-005, -010) and the
//! parameter receiver (`parameters`, REQ-FFI-MIDI1-006, -011). The program gives the storage of
//! each state (REQ-FFI-MIDI1-007).
//!
//! A typed MIDI 1.0 message is the flat byte struct [`ZmidiMidi1Message`], with no UMP Group
//! (REQ-FFI-MIDI1-001). The group feature `midi1` (header macro `ZMIDI_MIDI1`) selects these parts
//! with the MIDI 1.0-only io, ble and usb parts, so that a C program links no MIDI 2.0 code
//! (REQ-FFI-ABI-004).

#[cfg(any(
    feature = "messages",
    feature = "channel-mode",
    feature = "controllers",
    feature = "parameters"
))]
use core::mem::{align_of, size_of};

#[cfg(feature = "parameters")]
use zmidi_ump::controller::Rpn;
#[cfg(any(
    feature = "channel-mode",
    feature = "controllers",
    feature = "parameters"
))]
use zmidi_ump::Channel;
#[cfg(any(
    feature = "messages",
    feature = "channel-mode",
    feature = "controllers",
    feature = "parameters"
))]
use zmidi_ump::Midi1ChannelVoiceMessage;
#[cfg(feature = "messages")]
use zmidi_ump::SystemMessage;

#[cfg(any(
    feature = "messages",
    feature = "channel-mode",
    feature = "controllers",
    feature = "parameters"
))]
use crate::status::ZmidiStatus;

/// The kind of a [`ZmidiMidi1Message`].
#[cfg(any(
    feature = "messages",
    feature = "channel-mode",
    feature = "controllers",
    feature = "parameters"
))]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiMidi1Kind {
    /// No message.
    None = 0,
    /// A Channel Voice or Channel Mode message (status 80H to EFH).
    ChannelVoice = 1,
    /// A System Common or System Real Time message (status F1H to FFH, not F7H).
    System = 2,
    /// A complete System Exclusive message, F0H to F7H, in the caller buffer.
    SysEx = 3,
    /// A System Exclusive message that does not fit in the caller buffer; its bytes are discarded.
    SysExTooLong = 4,
}

/// One typed MIDI 1.0 message as flat bytes, with no UMP Group (REQ-FFI-MIDI1-001).
#[cfg(any(
    feature = "messages",
    feature = "channel-mode",
    feature = "controllers",
    feature = "parameters"
))]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMidi1Message {
    /// A [`ZmidiMidi1Kind`] value.
    pub kind: u32,
    /// The status byte, 80H to FFH; F0H for a System Exclusive message.
    pub status: u8,
    /// The first data byte, 00H to 7FH; 0 when the message has none.
    pub data1: u8,
    /// The second data byte, 00H to 7FH; 0 when the message has none.
    pub data2: u8,
    /// The length in bytes of a System Exclusive message from F0H to F7H; 0 for the other kinds.
    pub length: u32,
}

#[cfg(any(
    feature = "messages",
    feature = "channel-mode",
    feature = "controllers",
    feature = "parameters"
))]
impl ZmidiMidi1Message {
    /// The message of a Message Type 0x2 or 0x1 word: the status and data bytes are bytes 2 to 4.
    #[cfg(any(feature = "messages", feature = "controllers", feature = "parameters"))]
    const fn of_word(kind: ZmidiMidi1Kind, word: u32) -> Self {
        Self {
            kind: kind as u32,
            status: (word >> 16) as u8,
            data1: (word >> 8) as u8 & 0x7F,
            data2: word as u8 & 0x7F,
            length: 0,
        }
    }

    #[cfg(any(feature = "messages", feature = "controllers", feature = "parameters"))]
    const fn channel_voice(message: Midi1ChannelVoiceMessage) -> Self {
        Self::of_word(ZmidiMidi1Kind::ChannelVoice, message.into_bits())
    }

    /// The word of the status and data bytes, or `None` when a data byte is above 7FH.
    const fn word(&self, message_type: u32) -> Option<u32> {
        if self.data1 > 0x7F || self.data2 > 0x7F {
            return None;
        }
        Some(
            message_type << 28
                | (self.status as u32) << 16
                | (self.data1 as u32) << 8
                | self.data2 as u32,
        )
    }

    /// The typed Channel Voice message, or `None` for another kind or an invalid byte.
    fn to_channel_voice(self) -> Option<Midi1ChannelVoiceMessage> {
        if self.kind != ZmidiMidi1Kind::ChannelVoice as u32 || !(0x80..=0xEF).contains(&self.status)
        {
            return None;
        }
        Midi1ChannelVoiceMessage::parse(&[self.word(0x2)?])
    }
}

/// The typed Channel Voice message at `message`, or `None` for a null pointer or an invalid message.
///
/// # Safety
/// `message` is null or points to a readable [`ZmidiMidi1Message`].
#[cfg(any(
    feature = "channel-mode",
    feature = "controllers",
    feature = "parameters"
))]
unsafe fn channel_voice_at(message: *const ZmidiMidi1Message) -> Option<Midi1ChannelVoiceMessage> {
    // SAFETY: caller contract above.
    unsafe { message.as_ref() }?.to_channel_voice()
}

/// Writes `messages` into the caller array `out` (`capacity` entries) and their count into `count`.
///
/// # Safety
/// `out` points to `capacity` writable entries and `count` is writable (both not null).
#[cfg(any(feature = "controllers", feature = "parameters"))]
unsafe fn put_messages(
    messages: &[Midi1ChannelVoiceMessage],
    out: *mut ZmidiMidi1Message,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if capacity < messages.len() {
        return ZmidiStatus::BufferTooSmall;
    }
    // SAFETY: caller contract above.
    let out = unsafe { core::slice::from_raw_parts_mut(out, capacity) };
    for (slot, m) in out.iter_mut().zip(messages) {
        *slot = ZmidiMidi1Message::channel_voice(*m);
    }
    // SAFETY: caller contract above.
    unsafe { *count = messages.len() };
    ZmidiStatus::Ok
}

// ------------------------------------------------------------------------------------- messages

#[cfg(feature = "messages")]
use zmidi_io::{Midi1Event, Midi1Message, Midi1Reader, Midi1Writer};

/// Storage words of a [`ZmidiMidi1Reader`].
#[cfg(feature = "messages")]
pub const ZMIDI_MIDI1_READER_WORDS: usize = 5;
/// Storage words of a [`ZmidiMidi1Writer`].
#[cfg(feature = "messages")]
pub const ZMIDI_MIDI1_WRITER_WORDS: usize = 1;
/// The most messages that one byte completes: a System Exclusive message that a status byte
/// ends, and the message of that status byte.
#[cfg(feature = "messages")]
pub const ZMIDI_MIDI1_READER_MAX_EVENTS: usize = 2;

/// Storage of one typed MIDI 1.0 byte reader. The program gives it; [`zmidi_midi1_reader_init`]
/// sets it up.
#[cfg(feature = "messages")]
#[repr(C)]
pub struct ZmidiMidi1Reader {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_MIDI1_READER_WORDS],
}

/// Storage of one typed MIDI 1.0 byte writer. The program gives it; [`zmidi_midi1_writer_init`]
/// sets it up.
#[cfg(feature = "messages")]
#[repr(C)]
pub struct ZmidiMidi1Writer {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_MIDI1_WRITER_WORDS],
}

#[cfg(feature = "messages")]
const _: () = assert!(size_of::<Midi1Reader>() <= size_of::<ZmidiMidi1Reader>());
#[cfg(feature = "messages")]
const _: () = assert!(align_of::<Midi1Reader>() <= align_of::<ZmidiMidi1Reader>());
#[cfg(feature = "messages")]
const _: () = assert!(size_of::<Midi1Writer>() <= size_of::<ZmidiMidi1Writer>());
#[cfg(feature = "messages")]
const _: () = assert!(align_of::<Midi1Writer>() <= align_of::<ZmidiMidi1Writer>());

/// Sets up `reader`: no running status and no System Exclusive message in progress.
///
/// # Safety
/// `reader` is null or points to writable storage of a [`ZmidiMidi1Reader`].
#[cfg(feature = "messages")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_midi1_reader_init(reader: *mut ZmidiMidi1Reader) -> ZmidiStatus {
    if reader.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe { reader.cast::<Midi1Reader>().write(Midi1Reader::new()) };
    ZmidiStatus::Ok
}

/// The System Reset action of the program (AMB-M1-13): the reader forgets its running status, its
/// partial message and its System Exclusive message in progress.
///
/// # Safety
/// `reader` is null or was set up by [`zmidi_midi1_reader_init`].
#[cfg(feature = "messages")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_midi1_reader_reset(reader: *mut ZmidiMidi1Reader) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(reader) = (unsafe { reader.cast::<Midi1Reader>().as_mut() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    reader.reset();
    ZmidiStatus::Ok
}

/// Reads `byte` and writes each message that it completes into `out` (`capacity` entries), and
/// their count into `count`. The bytes of a System Exclusive message go into `sysex`
/// (`sysex_capacity` bytes); its message gives the length. A System Exclusive message that does
/// not fit gives the kind [`ZmidiMidi1Kind::SysExTooLong`] with its full length. A `capacity`
/// below [`ZMIDI_MIDI1_READER_MAX_EVENTS`] gives [`ZmidiStatus::BufferTooSmall`] before the byte
/// is read.
///
/// # Safety
/// `reader` was set up by [`zmidi_midi1_reader_init`]; `sysex` points to `sysex_capacity` writable
/// bytes; `out` points to `capacity` writable entries; `count` is writable. Null pointers give
/// [`ZmidiStatus::InvalidArgument`].
#[cfg(feature = "messages")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_midi1_reader_push(
    reader: *mut ZmidiMidi1Reader,
    byte: u8,
    sysex: *mut u8,
    sysex_capacity: usize,
    out: *mut ZmidiMidi1Message,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if reader.is_null() || sysex.is_null() || out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    if capacity < ZMIDI_MIDI1_READER_MAX_EVENTS {
        return ZmidiStatus::BufferTooSmall;
    }
    // SAFETY: caller contract above.
    let (reader, sysex, out) = unsafe {
        (
            &mut *reader.cast::<Midi1Reader>(),
            core::slice::from_raw_parts_mut(sysex, sysex_capacity),
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let mut n = 0;
    reader.feed(byte, sysex, |event| {
        let m = match event {
            Midi1Event::Message(Midi1Message::ChannelVoice(m)) => {
                ZmidiMidi1Message::channel_voice(m)
            }
            Midi1Event::Message(Midi1Message::System(m)) => {
                ZmidiMidi1Message::of_word(ZmidiMidi1Kind::System, m.into_bits())
            }
            Midi1Event::Message(Midi1Message::SysEx(bytes)) => ZmidiMidi1Message {
                kind: ZmidiMidi1Kind::SysEx as u32,
                status: 0xF0,
                data1: 0,
                data2: 0,
                length: bytes.len() as u32,
            },
            Midi1Event::SysExTooLong { length } => ZmidiMidi1Message {
                kind: ZmidiMidi1Kind::SysExTooLong as u32,
                status: 0xF0,
                data1: 0,
                data2: 0,
                length: length as u32,
            },
        };
        // At most ZMIDI_MIDI1_READER_MAX_EVENTS messages, so this always fits.
        if let Some(slot) = out.get_mut(n) {
            *slot = m;
            n += 1;
        }
    });
    // SAFETY: caller contract above.
    unsafe { *count = n };
    ZmidiStatus::Ok
}

/// Gives `size_of` the Rust reader, for a binding that checks its copy of the layout (DEC-019 rule
/// 6).
#[cfg(feature = "messages")]
#[no_mangle]
pub extern "C" fn zmidi_midi1_reader_size() -> usize {
    size_of::<Midi1Reader>()
}

/// Sets up `writer`, with running status when `running_status` is true.
///
/// # Safety
/// `writer` is null or points to writable storage of a [`ZmidiMidi1Writer`].
#[cfg(feature = "messages")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_midi1_writer_init(
    writer: *mut ZmidiMidi1Writer,
    running_status: bool,
) -> ZmidiStatus {
    if writer.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe {
        writer
            .cast::<Midi1Writer>()
            .write(Midi1Writer::new(running_status))
    };
    ZmidiStatus::Ok
}

/// The next Channel Voice message writes its status byte, also with running status.
///
/// # Safety
/// `writer` is null or was set up by [`zmidi_midi1_writer_init`].
#[cfg(feature = "messages")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_midi1_writer_refresh_status(
    writer: *mut ZmidiMidi1Writer,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(writer) = (unsafe { writer.cast::<Midi1Writer>().as_mut() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    writer.refresh_status();
    ZmidiStatus::Ok
}

/// Writes the bytes of `message` into `out` (`capacity` bytes) and their count into `count`. A
/// System Exclusive message takes its `length` bytes from `sysex`, which may be null for the other
/// kinds. An invalid message (a status byte of another kind, a data byte above 7FH, a System
/// Exclusive message with no F0H first, no F7H last or a data byte above 7FH) gives
/// [`ZmidiStatus::InvalidArgument`], and a small `out` gives [`ZmidiStatus::BufferTooSmall`],
/// both with no state change.
///
/// # Safety
/// `writer` was set up by [`zmidi_midi1_writer_init`]; `message` is readable; `sysex` is null or
/// points to `message.length` readable bytes; `out` points to `capacity` writable bytes; `count`
/// is writable.
#[cfg(feature = "messages")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_midi1_writer_write(
    writer: *mut ZmidiMidi1Writer,
    message: *const ZmidiMidi1Message,
    sysex: *const u8,
    out: *mut u8,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if writer.is_null() || message.is_null() || out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (writer, m) = unsafe { (&mut *writer.cast::<Midi1Writer>(), *message) };
    let typed = match m.kind {
        k if k == ZmidiMidi1Kind::ChannelVoice as u32 => {
            m.to_channel_voice().map(Midi1Message::ChannelVoice)
        }
        k if k == ZmidiMidi1Kind::System as u32 => match m.status {
            0xF1..=0xF6 | 0xF8..=0xFF => m
                .word(0x1)
                .and_then(|w| SystemMessage::parse(&[w]))
                .map(Midi1Message::System),
            _ => None,
        },
        k if k == ZmidiMidi1Kind::SysEx as u32 && !sysex.is_null() => {
            // SAFETY: caller contract above.
            Some(Midi1Message::SysEx(unsafe {
                core::slice::from_raw_parts(sysex, m.length as usize)
            }))
        }
        _ => None,
    };
    let Some(typed) = typed else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let out = unsafe { core::slice::from_raw_parts_mut(out, capacity) };
    match writer.write(typed, out) {
        Ok(n) => {
            // SAFETY: caller contract above.
            unsafe { *count = n };
            ZmidiStatus::Ok
        }
        Err(zmidi_io::MessageError::BufferTooSmall) => ZmidiStatus::BufferTooSmall,
        Err(zmidi_io::MessageError::InvalidSysEx) => ZmidiStatus::InvalidArgument,
    }
}

/// Gives `size_of` the Rust writer, for a binding that checks its copy of the layout (DEC-019 rule
/// 6).
#[cfg(feature = "messages")]
#[no_mangle]
pub extern "C" fn zmidi_midi1_writer_size() -> usize {
    size_of::<Midi1Writer>()
}

// ---------------------------------------------------------------------------------- channel-mode

#[cfg(feature = "channel-mode")]
use zmidi_io::{ModeAction, ModeConfig, ModeState};

/// Storage words of a [`ZmidiModeState`].
#[cfg(feature = "channel-mode")]
pub const ZMIDI_MODE_STATE_WORDS: usize = 2;

/// Storage of one receiver mode state (one instrument). The program gives it; [`zmidi_mode_init`]
/// sets it up.
#[cfg(feature = "channel-mode")]
#[repr(C)]
pub struct ZmidiModeState {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_MODE_STATE_WORDS],
}

#[cfg(feature = "channel-mode")]
const _: () = assert!(size_of::<ModeState>() <= size_of::<ZmidiModeState>());
#[cfg(feature = "channel-mode")]
const _: () = assert!(align_of::<ModeState>() <= align_of::<ZmidiModeState>());

/// The configuration of a mode state.
#[cfg(feature = "channel-mode")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiModeConfig {
    /// The Basic Channel, 0 to 15.
    pub basic_channel: u8,
    /// The implemented modes: bit 0 Mode 1 (always implemented), bit 1 Mode 2, bit 2 Mode 3, bit 3
    /// Mode 4.
    pub implemented: u8,
    /// True: a mode that is not implemented gives the alternate mode (p. 23); false: the Mode
    /// message is ignored.
    pub alternate: bool,
    /// The voice count for M = 0 (Mode 4).
    pub voices: u8,
    /// True: the channel below the Basic Channel carries the Global Controllers in Mode 4.
    pub global_controllers: bool,
}

/// The kind of a [`ZmidiModeAction`].
#[cfg(feature = "channel-mode")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiModeActionKind {
    /// The message is not for this instrument.
    Ignore = 0,
    /// A voice message for the instrument; `voice` is the Mode 4 voice.
    Voice = 1,
    /// A Global Controller message (Mode 4).
    Global = 2,
    /// All Notes Off for `channel`.
    AllNotesOff = 3,
    /// All Sound Off for `channel`.
    AllSoundOff = 4,
    /// Reset All Controllers for `channel`.
    ResetAllControllers = 5,
    /// Local Control; `on`.
    LocalControl = 6,
    /// A mode message: all notes off for all voices; `changed` and `alternate`.
    ModeChange = 7,
}

/// The action of one received message.
#[cfg(feature = "channel-mode")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiModeAction {
    /// A [`ZmidiModeActionKind`] value.
    pub kind: u32,
    /// The channel of an All Notes Off, All Sound Off or Reset All Controllers action (Mode 4), or
    /// FFH for all channels of the instrument.
    pub channel: u8,
    /// The Mode 4 voice of a voice message, or FFH for none.
    pub voice: u8,
    /// Local Control on.
    pub on: bool,
    /// The mode message changed the mode.
    pub changed: bool,
    /// The mode in use is an alternate mode.
    pub alternate: bool,
}

/// The mode information of a mode state.
#[cfg(feature = "channel-mode")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiModeInfo {
    /// The mode in use, 1 to 4.
    pub mode: u8,
    /// The mode of the Omni and Mono flags, 1 to 4.
    pub requested_mode: u8,
    /// The mode in use is an alternate mode.
    pub alternate: bool,
    /// The Basic Channel, 0 to 15.
    pub basic_channel: u8,
    /// M of the last Mono On.
    pub m: u8,
    /// Local Control on.
    pub local: bool,
}

#[cfg(feature = "channel-mode")]
fn mode_action(action: ModeAction) -> ZmidiModeAction {
    use ZmidiModeActionKind as K;
    let mut a = ZmidiModeAction {
        kind: K::Ignore as u32,
        channel: 0xFF,
        voice: 0xFF,
        on: false,
        changed: false,
        alternate: false,
    };
    let channel = |c: Option<Channel>| c.map_or(0xFF, Channel::index);
    match action {
        ModeAction::Ignore => {}
        ModeAction::Voice { voice } => {
            a.kind = K::Voice as u32;
            a.voice = voice.unwrap_or(0xFF);
        }
        ModeAction::Global => a.kind = K::Global as u32,
        ModeAction::AllNotesOff { channel: c } => {
            a.kind = K::AllNotesOff as u32;
            a.channel = channel(c);
        }
        ModeAction::AllSoundOff { channel: c } => {
            a.kind = K::AllSoundOff as u32;
            a.channel = channel(c);
        }
        ModeAction::ResetAllControllers { channel: c } => {
            a.kind = K::ResetAllControllers as u32;
            a.channel = channel(c);
        }
        ModeAction::LocalControl { on } => {
            a.kind = K::LocalControl as u32;
            a.on = on;
        }
        ModeAction::ModeChange { changed, alternate } => {
            a.kind = K::ModeChange as u32;
            a.changed = changed;
            a.alternate = alternate;
        }
    }
    a
}

/// Sets up `state` from `config`, in Mode 1 with Local Control on. A Basic Channel above 15 or a
/// mode mask above 0FH gives [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `state` is null or points to writable storage of a [`ZmidiModeState`]; `config` is null or
/// readable.
#[cfg(feature = "channel-mode")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mode_init(
    state: *mut ZmidiModeState,
    config: *const ZmidiModeConfig,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(c) = (unsafe { config.as_ref() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let (Ok(basic_channel), true) = (Channel::new(c.basic_channel), c.implemented <= 0xF) else {
        return ZmidiStatus::InvalidArgument;
    };
    if state.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    let config = ModeConfig {
        basic_channel,
        implemented: [0, 1, 2, 3].map(|bit| c.implemented >> bit & 1 != 0),
        alternate: c.alternate,
        voices: c.voices,
        global_controllers: c.global_controllers,
    };
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe { state.cast::<ModeState>().write(ModeState::new(config)) };
    ZmidiStatus::Ok
}

/// Takes one Channel Voice `message` and writes its action into `action`. A message that is not a
/// valid Channel Voice message gives [`ZmidiStatus::InvalidArgument`] and changes nothing.
///
/// # Safety
/// `state` was set up by [`zmidi_mode_init`]; `message` is readable; `action` is writable.
#[cfg(feature = "channel-mode")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mode_receive(
    state: *mut ZmidiModeState,
    message: *const ZmidiMidi1Message,
    action: *mut ZmidiModeAction,
) -> ZmidiStatus {
    if state.is_null() || action.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(m) = (unsafe { channel_voice_at(message) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { *action = mode_action((*state.cast::<ModeState>()).receive(&m)) };
    ZmidiStatus::Ok
}

/// Runs `f` on the mode state at `state`.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mode_init`].
#[cfg(feature = "channel-mode")]
unsafe fn with_mode(state: *mut ZmidiModeState, f: impl FnOnce(&mut ModeState)) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match unsafe { state.cast::<ModeState>().as_mut() } {
        Some(s) => {
            f(s);
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::InvalidArgument,
    }
}

/// The System Reset action of the program: Omni On, Poly and Local On; the Basic Channel and M stay.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mode_init`].
#[cfg(feature = "channel-mode")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mode_system_reset(state: *mut ZmidiModeState) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { with_mode(state, ModeState::system_reset) }
}

/// Writes the mode information of `state` into `info`.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mode_init`]; `info` is null or writable.
#[cfg(feature = "channel-mode")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mode_info(
    state: *const ZmidiModeState,
    info: *mut ZmidiModeInfo,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(s), Some(info)) = (unsafe { state.cast::<ModeState>().as_ref() }, unsafe {
        info.as_mut()
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    *info = ZmidiModeInfo {
        mode: s.mode().number(),
        requested_mode: s.requested_mode().number(),
        alternate: s.is_alternate(),
        basic_channel: s.basic_channel().index(),
        m: s.m(),
        local: s.local(),
    };
    ZmidiStatus::Ok
}

/// Sets the Basic Channel (0 to 15), for example from a front panel.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mode_init`].
#[cfg(feature = "channel-mode")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mode_set_basic_channel(
    state: *mut ZmidiModeState,
    channel: u8,
) -> ZmidiStatus {
    let Ok(channel) = Channel::new(channel) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { with_mode(state, |s| s.set_basic_channel(channel)) }
}

/// Sets the Omni flag, as Omni On or Omni Off does, from the program.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mode_init`].
#[cfg(feature = "channel-mode")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mode_set_omni(state: *mut ZmidiModeState, on: bool) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { with_mode(state, |s| s.set_omni(on)) }
}

/// Selects Poly, as Poly On does, from the program.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mode_init`].
#[cfg(feature = "channel-mode")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mode_set_poly(state: *mut ZmidiModeState) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { with_mode(state, ModeState::set_poly) }
}

/// Selects Mono with M (0 to 127), as Mono On does, from the program.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mode_init`].
#[cfg(feature = "channel-mode")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mode_set_mono(state: *mut ZmidiModeState, m: u8) -> ZmidiStatus {
    if m > 0x7F {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe { with_mode(state, |s| s.set_mono(m)) }
}

/// Sets Local Control on or off from the program.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mode_init`].
#[cfg(feature = "channel-mode")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mode_set_local(state: *mut ZmidiModeState, on: bool) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { with_mode(state, |s| s.set_local(on)) }
}

/// Gives `size_of` the Rust mode state, for a binding that checks its copy of the layout (DEC-019
/// rule 6).
#[cfg(feature = "channel-mode")]
#[no_mangle]
pub extern "C" fn zmidi_mode_state_size() -> usize {
    size_of::<ModeState>()
}

// ----------------------------------------------------------------------------------- controllers

#[cfg(feature = "controllers")]
use zmidi_io::{ControllerEvent, ControllerState};

/// Storage words of a [`ZmidiControllerState`].
#[cfg(feature = "controllers")]
pub const ZMIDI_CONTROLLER_STATE_WORDS: usize = 5;
/// The reset value of a controller that keeps its value at Reset All Controllers.
#[cfg(feature = "controllers")]
pub const ZMIDI_CONTROLLER_KEEP: u8 = 0xFF;

/// Storage of one controller state (one channel). The program gives it;
/// [`zmidi_controllers_init`] sets it up.
#[cfg(feature = "controllers")]
#[repr(C)]
pub struct ZmidiControllerState {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_CONTROLLER_STATE_WORDS],
}

#[cfg(feature = "controllers")]
const _: () = assert!(size_of::<ControllerState>() <= size_of::<ZmidiControllerState>());
#[cfg(feature = "controllers")]
const _: () = assert!(align_of::<ControllerState>() <= align_of::<ZmidiControllerState>());

/// The kind of a [`ZmidiControllerEvent`].
#[cfg(feature = "controllers")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiControllerEventKind {
    /// The message gives no event.
    None = 0,
    /// A 14-bit controller: `controller` 0 to 31 and the 14-bit `value`.
    Controller14 = 1,
    /// A single-byte controller: `controller` 64 to 119 and `value` 0 to 127.
    Controller7 = 2,
    /// Program Change: `program`, and `bank` when `has_bank`.
    Program = 3,
    /// A Note On after a Portamento Control: `source` is the source note.
    Portamento = 4,
    /// Reset All Controllers (RP-015): see `zmidi_controller_reset_value`.
    ResetAllControllers = 5,
}

/// The event of one received message.
#[cfg(feature = "controllers")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiControllerEvent {
    /// A [`ZmidiControllerEventKind`] value.
    pub kind: u32,
    /// The controller number.
    pub controller: u8,
    /// The program number.
    pub program: u8,
    /// The Portamento source note.
    pub source: u8,
    /// The program event has a bank.
    pub has_bank: bool,
    /// The 14-bit or 7-bit controller value.
    pub value: u16,
    /// The 14-bit bank of a program event.
    pub bank: u16,
}

#[cfg(feature = "controllers")]
fn controller_event(event: Option<ControllerEvent>) -> ZmidiControllerEvent {
    use ZmidiControllerEventKind as K;
    let mut e = ZmidiControllerEvent {
        kind: K::None as u32,
        controller: 0,
        program: 0,
        source: 0,
        has_bank: false,
        value: 0,
        bank: 0,
    };
    match event {
        None => {}
        Some(ControllerEvent::Controller14 { controller, value }) => {
            e.kind = K::Controller14 as u32;
            e.controller = controller;
            e.value = value;
        }
        Some(ControllerEvent::Controller7 { controller, value }) => {
            e.kind = K::Controller7 as u32;
            e.controller = controller;
            e.value = value.into();
        }
        Some(ControllerEvent::Program { program, bank }) => {
            e.kind = K::Program as u32;
            e.program = program;
            e.has_bank = bank.is_some();
            e.bank = bank.unwrap_or(0);
        }
        Some(ControllerEvent::Portamento { source }) => {
            e.kind = K::Portamento as u32;
            e.source = source;
        }
        Some(ControllerEvent::ResetAllControllers) => e.kind = K::ResetAllControllers as u32,
    }
    e
}

/// Sets up `state`: no bank and no Portamento source.
///
/// # Safety
/// `state` is null or points to writable storage of a [`ZmidiControllerState`].
#[cfg(feature = "controllers")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_controllers_init(state: *mut ZmidiControllerState) -> ZmidiStatus {
    if state.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe {
        state
            .cast::<ControllerState>()
            .write(ControllerState::new())
    };
    ZmidiStatus::Ok
}

/// Takes one Channel Voice `message` of the channel of `state` and writes its event into `event`
/// (the kind none when the message gives no event). A message that is not a valid Channel Voice
/// message gives [`ZmidiStatus::InvalidArgument`] and changes nothing.
///
/// # Safety
/// `state` was set up by [`zmidi_controllers_init`]; `message` is readable; `event` is writable.
#[cfg(feature = "controllers")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_controllers_receive(
    state: *mut ZmidiControllerState,
    message: *const ZmidiMidi1Message,
    event: *mut ZmidiControllerEvent,
) -> ZmidiStatus {
    if state.is_null() || event.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(m) = (unsafe { channel_voice_at(message) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { *event = controller_event((*state.cast::<ControllerState>()).receive(&m)) };
    ZmidiStatus::Ok
}

/// The Reset All Controllers action of the program (a mode state action or General MIDI System On).
///
/// # Safety
/// `state` is null or was set up by [`zmidi_controllers_init`].
#[cfg(feature = "controllers")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_controllers_reset(state: *mut ZmidiControllerState) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match unsafe { state.cast::<ControllerState>().as_mut() } {
        Some(s) => {
            s.reset_all_controllers();
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::InvalidArgument,
    }
}

/// Writes whether `state` holds a bank into `has_bank`, and the held 14-bit bank (or 0) into `bank`.
///
/// # Safety
/// `state` was set up by [`zmidi_controllers_init`]; `has_bank` and `bank` are writable.
#[cfg(feature = "controllers")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_controllers_bank(
    state: *const ZmidiControllerState,
    has_bank: *mut bool,
    bank: *mut u16,
) -> ZmidiStatus {
    if state.is_null() || has_bank.is_null() || bank.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe {
        let b = (*state.cast::<ControllerState>()).bank();
        *has_bank = b.is_some();
        *bank = b.unwrap_or(0);
    }
    ZmidiStatus::Ok
}

/// Gives `size_of` the Rust controller state, for a binding that checks its copy of the layout
/// (DEC-019 rule 6).
#[cfg(feature = "controllers")]
#[no_mangle]
pub extern "C" fn zmidi_controller_state_size() -> usize {
    size_of::<ControllerState>()
}

/// Writes the RP-015 reset value of `controller` into `value`, or [`ZMIDI_CONTROLLER_KEEP`] when
/// the controller keeps its value (zmidi-io `reset_value`).
///
/// # Safety
/// `value` is null or writable.
#[cfg(feature = "controllers")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_controller_reset_value(
    controller: u8,
    value: *mut u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(value) = (unsafe { value.as_mut() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    *value = zmidi_io::reset_value(controller).unwrap_or(ZMIDI_CONTROLLER_KEEP);
    ZmidiStatus::Ok
}

/// Writes a 14-bit controller value (`controller` 0 to 31, `value` 0 to 3FFFH) on `channel` (0
/// to 15) as the MSB and then the LSB Control Change into `out` (`capacity` entries, at least 2),
/// and 2 into `count`.
///
/// # Safety
/// `out` points to `capacity` writable entries; `count` is writable.
#[cfg(feature = "controllers")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_controller14_messages(
    channel: u8,
    controller: u8,
    value: u16,
    out: *mut ZmidiMidi1Message,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    let messages = match Channel::new(channel)
        .and_then(|c| zmidi_io::controller14_messages(c, controller, value))
    {
        Ok(m) => m,
        Err(_) => return ZmidiStatus::InvalidArgument,
    };
    // SAFETY: caller contract above.
    unsafe { put_messages(&messages, out, capacity, count) }
}

/// Writes a bank (0 to 3FFFH) and program (0 to 127) selection on `channel` (0 to 15) as Bank
/// Select MSB, Bank Select LSB and Program Change into `out` (`capacity` entries, at least 3), and
/// 3 into `count`.
///
/// # Safety
/// `out` points to `capacity` writable entries; `count` is writable.
#[cfg(feature = "controllers")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_bank_program_messages(
    channel: u8,
    bank: u16,
    program: u8,
    out: *mut ZmidiMidi1Message,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    let messages = match Channel::new(channel)
        .and_then(|c| zmidi_io::bank_program_messages(c, bank, program))
    {
        Ok(m) => m,
        Err(_) => return ZmidiStatus::InvalidArgument,
    };
    // SAFETY: caller contract above.
    unsafe { put_messages(&messages, out, capacity, count) }
}

// ------------------------------------------------------------------------------------ parameters

#[cfg(feature = "parameters")]
use zmidi_io::{Parameter, ParameterChange, ParameterEvent, ParameterState};

/// Storage words of a [`ZmidiParameterState`].
#[cfg(feature = "parameters")]
pub const ZMIDI_PARAMETER_STATE_WORDS: usize = 1;

/// Storage of one parameter receiver (one channel). The program gives it;
/// [`zmidi_parameters_init`] sets it up.
#[cfg(feature = "parameters")]
#[repr(C)]
pub struct ZmidiParameterState {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_PARAMETER_STATE_WORDS],
}

#[cfg(feature = "parameters")]
const _: () = assert!(size_of::<ParameterState>() <= size_of::<ZmidiParameterState>());
#[cfg(feature = "parameters")]
const _: () = assert!(align_of::<ParameterState>() <= align_of::<ZmidiParameterState>());

/// A parameter number.
#[cfg(feature = "parameters")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiParameter {
    /// True: a Registered Parameter Number (controllers 101 and 100); false: a Non-Registered
    /// Parameter Number (controllers 99 and 98).
    pub registered: bool,
    /// The MSB, 0 to 127. 7FH 7FH is the null number.
    pub msb: u8,
    /// The LSB, 0 to 127.
    pub lsb: u8,
}

/// The change of a [`ZmidiParameterEvent`].
#[cfg(feature = "parameters")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiParameterChange {
    /// The message gives no event.
    None = 0,
    /// Data Entry MSB (controller 6): `value`.
    DataEntryMsb = 1,
    /// Data Entry LSB (controller 38): `value`.
    DataEntryLsb = 2,
    /// Data Increment (controller 96).
    Increment = 3,
    /// Data Decrement (controller 97).
    Decrement = 4,
}

/// The event of one received message.
#[cfg(feature = "parameters")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiParameterEvent {
    /// A [`ZmidiParameterChange`] value.
    pub change: u32,
    /// The selected parameter.
    pub parameter: ZmidiParameter,
    /// The data byte of a Data Entry, 0 to 127.
    pub value: u8,
}

#[cfg(feature = "parameters")]
const fn c_parameter(p: Parameter) -> ZmidiParameter {
    match p {
        Parameter::Registered(rpn) => {
            let (msb, lsb) = rpn.parts();
            ZmidiParameter {
                registered: true,
                msb,
                lsb,
            }
        }
        Parameter::NonRegistered { msb, lsb } => ZmidiParameter {
            registered: false,
            msb,
            lsb,
        },
    }
}

#[cfg(feature = "parameters")]
const fn rust_parameter(p: ZmidiParameter) -> Option<Parameter> {
    if p.msb > 0x7F || p.lsb > 0x7F {
        return None;
    }
    Some(if p.registered {
        Parameter::Registered(Rpn::from_parts(p.msb, p.lsb))
    } else {
        Parameter::NonRegistered {
            msb: p.msb,
            lsb: p.lsb,
        }
    })
}

#[cfg(feature = "parameters")]
const fn rust_change(change: u32, value: u8) -> Option<ParameterChange> {
    if value > 0x7F {
        return None;
    }
    match change {
        1 => Some(ParameterChange::DataEntryMsb(value)),
        2 => Some(ParameterChange::DataEntryLsb(value)),
        3 => Some(ParameterChange::Increment),
        4 => Some(ParameterChange::Decrement),
        _ => None,
    }
}

#[cfg(feature = "parameters")]
fn parameter_event(event: Option<ParameterEvent>) -> ZmidiParameterEvent {
    use ZmidiParameterChange as C;
    let Some(e) = event else {
        return ZmidiParameterEvent {
            change: C::None as u32,
            parameter: ZmidiParameter {
                registered: false,
                msb: 0,
                lsb: 0,
            },
            value: 0,
        };
    };
    let (change, value) = match e.change {
        ParameterChange::DataEntryMsb(v) => (C::DataEntryMsb, v),
        ParameterChange::DataEntryLsb(v) => (C::DataEntryLsb, v),
        ParameterChange::Increment => (C::Increment, 0),
        ParameterChange::Decrement => (C::Decrement, 0),
    };
    ZmidiParameterEvent {
        change: change as u32,
        parameter: c_parameter(e.parameter),
        value,
    }
}

/// Runs `f` on the parameter receiver at `state`.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_parameters_init`].
#[cfg(feature = "parameters")]
unsafe fn with_parameters(
    state: *mut ZmidiParameterState,
    f: impl FnOnce(&mut ParameterState),
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match unsafe { state.cast::<ParameterState>().as_mut() } {
        Some(s) => {
            f(s);
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::InvalidArgument,
    }
}

/// Sets up `state`: no number received, RPN reception on and NRPN reception off.
///
/// # Safety
/// `state` is null or points to writable storage of a [`ZmidiParameterState`].
#[cfg(feature = "parameters")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_parameters_init(state: *mut ZmidiParameterState) -> ZmidiStatus {
    if state.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe { state.cast::<ParameterState>().write(ParameterState::new()) };
    ZmidiStatus::Ok
}

/// Takes one Channel Voice `message` of the channel of `state` and writes its event into `event`
/// (the change none when the message gives no event). A message that is not a valid Channel Voice
/// message gives [`ZmidiStatus::InvalidArgument`] and changes nothing.
///
/// # Safety
/// `state` was set up by [`zmidi_parameters_init`]; `message` is readable; `event` is writable.
#[cfg(feature = "parameters")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_parameters_receive(
    state: *mut ZmidiParameterState,
    message: *const ZmidiMidi1Message,
    event: *mut ZmidiParameterEvent,
) -> ZmidiStatus {
    if state.is_null() || event.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(m) = (unsafe { channel_voice_at(message) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { *event = parameter_event((*state.cast::<ParameterState>()).receive(&m)) };
    ZmidiStatus::Ok
}

/// The Reset All Controllers action of the program: the RPN and NRPN numbers become 7FH 7FH.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_parameters_init`].
#[cfg(feature = "parameters")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_parameters_reset(state: *mut ZmidiParameterState) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { with_parameters(state, ParameterState::reset_all_controllers) }
}

/// Writes whether a parameter is selected into `selected`, and the parameter (or zeros) into
/// `parameter`.
///
/// # Safety
/// `state` was set up by [`zmidi_parameters_init`]; `selected` and `parameter` are writable.
#[cfg(feature = "parameters")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_parameters_selection(
    state: *const ZmidiParameterState,
    selected: *mut bool,
    parameter: *mut ZmidiParameter,
) -> ZmidiStatus {
    if state.is_null() || selected.is_null() || parameter.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe {
        let p = (*state.cast::<ParameterState>()).selection();
        *selected = p.is_some();
        *parameter = p.map_or(
            ZmidiParameter {
                registered: false,
                msb: 0,
                lsb: 0,
            },
            c_parameter,
        );
    }
    ZmidiStatus::Ok
}

/// Writes whether RPN reception is on into `rpn` and whether NRPN reception is on into `nrpn`.
///
/// # Safety
/// `state` was set up by [`zmidi_parameters_init`]; `rpn` and `nrpn` are writable.
#[cfg(feature = "parameters")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_parameters_reception(
    state: *const ZmidiParameterState,
    rpn: *mut bool,
    nrpn: *mut bool,
) -> ZmidiStatus {
    if state.is_null() || rpn.is_null() || nrpn.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe {
        let s = &*state.cast::<ParameterState>();
        *rpn = s.rpn_reception();
        *nrpn = s.nrpn_reception();
    }
    ZmidiStatus::Ok
}

/// Turns RPN reception and NRPN reception on or off. A kind that goes on waits again for both bytes
/// of its number.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_parameters_init`].
#[cfg(feature = "parameters")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_parameters_set_reception(
    state: *mut ZmidiParameterState,
    rpn: bool,
    nrpn: bool,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe {
        with_parameters(state, |s| {
            s.set_rpn_reception(rpn);
            s.set_nrpn_reception(nrpn);
        })
    }
}

/// Gives `size_of` the Rust parameter receiver, for a binding that checks its copy of the layout
/// (DEC-019 rule 6).
#[cfg(feature = "parameters")]
#[no_mangle]
pub extern "C" fn zmidi_parameter_state_size() -> usize {
    size_of::<ParameterState>()
}

/// Writes the new 14-bit value of the parameter of `event` from its current `value` (0 to 3FFFH)
/// into `out`, with the RP-018 steps (zmidi-io `ParameterEvent::apply`). An event with no change,
/// an invalid parameter or a value above 3FFFH gives [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `event` is null or readable; `out` is null or writable.
#[cfg(feature = "parameters")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_parameter_apply(
    event: *const ZmidiParameterEvent,
    value: u16,
    out: *mut u16,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(e), Some(out)) = (unsafe { event.as_ref() }, unsafe { out.as_mut() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let (Some(parameter), Some(change), true) = (
        rust_parameter(e.parameter),
        rust_change(e.change, e.value),
        value <= 0x3FFF,
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    *out = ParameterEvent { parameter, change }.apply(value);
    ZmidiStatus::Ok
}

/// Writes the number of `parameter` on `channel` (0 to 15) as its MSB and then its LSB Control
/// Change into `out` (`capacity` entries, at least 2), and 2 into `count`. A registered 7FH 7FH
/// is the null number.
///
/// # Safety
/// `parameter` is readable; `out` points to `capacity` writable entries; `count` is writable.
#[cfg(feature = "parameters")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_parameter_select_messages(
    channel: u8,
    parameter: *const ZmidiParameter,
    out: *mut ZmidiMidi1Message,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (Some(p), Ok(channel)) = (
        unsafe { parameter.as_ref() }.and_then(|p| rust_parameter(*p)),
        Channel::new(channel),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Ok(messages) = zmidi_io::select_messages(channel, p) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { put_messages(&messages, out, capacity, count) }
}

/// Writes one `change` (a [`ZmidiParameterChange`] value, not none; `value` is the Data Entry byte)
/// on `channel` (0 to 15) into `out`: Data Entry MSB (6), Data Entry LSB (38), Data Increment (96)
/// or Data Decrement (97) with value 0.
///
/// # Safety
/// `out` is null or writable.
#[cfg(feature = "parameters")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_parameter_change_message(
    channel: u8,
    change: u32,
    value: u8,
    out: *mut ZmidiMidi1Message,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let (Some(change), Ok(channel)) = (rust_change(change, value), Channel::new(channel)) else {
        return ZmidiStatus::InvalidArgument;
    };
    match zmidi_io::change_message(channel, change) {
        Ok(m) => {
            *out = ZmidiMidi1Message::channel_voice(m);
            ZmidiStatus::Ok
        }
        Err(_) => ZmidiStatus::InvalidArgument,
    }
}

/// Writes a parameter `value` (0 to 3FFFH) of `parameter` on `channel` (0 to 15) as the number
/// and then Data Entry MSB and LSB into `out` (`capacity` entries, at least 4), and 4 into
/// `count`.
///
/// # Safety
/// `parameter` is readable; `out` points to `capacity` writable entries; `count` is writable.
#[cfg(feature = "parameters")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_parameter_value_messages(
    channel: u8,
    parameter: *const ZmidiParameter,
    value: u16,
    out: *mut ZmidiMidi1Message,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (Some(p), Ok(channel)) = (
        unsafe { parameter.as_ref() }.and_then(|p| rust_parameter(*p)),
        Channel::new(channel),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Ok(messages) = zmidi_io::value_messages(channel, p, value) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { put_messages(&messages, out, capacity, count) }
}
