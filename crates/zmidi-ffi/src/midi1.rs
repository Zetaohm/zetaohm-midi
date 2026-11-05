//! The C ABI of the MIDI 1.0 parts of zmidi-io (DEC-019, DEC-022): the typed byte reader and writer
//! (feature `messages`, REQ-FFI-MIDI1-002, -003), the receiver mode state (`channel-mode`,
//! REQ-FFI-MIDI1-004, -009), the controller state (`controllers`, REQ-FFI-MIDI1-005, -010) and the
//! parameter receiver (`parameters`, REQ-FFI-MIDI1-006, -011), the sync receiver (`sync`,
//! REQ-FFI-MIDI1-012) and the Active Sensing monitor and sender (`active-sensing`, REQ-FFI-MIDI1-013).
//! The program gives the storage of each state (REQ-FFI-MIDI1-007).
//!
//! A typed MIDI 1.0 message is the flat byte struct [`ZmidiMidi1Message`], with no UMP Group
//! (REQ-FFI-MIDI1-001). The group feature `midi1` (header macro `ZMIDI_MIDI1`) selects these parts
//! with the MIDI 1.0-only io, ble and usb parts, so that a C program links no MIDI 2.0 code
//! (REQ-FFI-ABI-004).

#[cfg(any(
    feature = "messages",
    feature = "channel-mode",
    feature = "controllers",
    feature = "parameters",
    feature = "sync",
    feature = "active-sensing"
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
#[cfg(any(feature = "messages", feature = "sync"))]
use zmidi_ump::SystemMessage;

#[cfg(any(
    feature = "messages",
    feature = "channel-mode",
    feature = "controllers",
    feature = "parameters",
    feature = "sync",
    feature = "active-sensing"
))]
use crate::status::ZmidiStatus;

/// The kind of a [`ZmidiMidi1Message`].
#[cfg(any(
    feature = "messages",
    feature = "channel-mode",
    feature = "controllers",
    feature = "parameters",
    feature = "sync"
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
    feature = "parameters",
    feature = "sync"
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
    feature = "parameters",
    feature = "sync"
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
    #[cfg(any(
        feature = "messages",
        feature = "channel-mode",
        feature = "controllers",
        feature = "parameters"
    ))]
    fn to_channel_voice(self) -> Option<Midi1ChannelVoiceMessage> {
        if self.kind != ZmidiMidi1Kind::ChannelVoice as u32 || !(0x80..=0xEF).contains(&self.status)
        {
            return None;
        }
        Midi1ChannelVoiceMessage::parse(&[self.word(0x2)?])
    }

    /// The typed System message (status F1H to FFH, not F7H), or `None` for another kind or an
    /// invalid byte.
    #[cfg(any(feature = "messages", feature = "sync"))]
    fn to_system(self) -> Option<SystemMessage> {
        if self.kind != ZmidiMidi1Kind::System as u32 || matches!(self.status, 0..=0xF0 | 0xF7) {
            return None;
        }
        SystemMessage::parse(&[self.word(0x1)?])
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
        k if k == ZmidiMidi1Kind::System as u32 => m.to_system().map(Midi1Message::System),
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

// ------------------------------------------------------------------------------------------ sync

#[cfg(feature = "sync")]
use zmidi_io::{SyncCommand, SyncEvent, SyncReceiver, SyncRun};

/// Storage words of a [`ZmidiSyncReceiver`].
#[cfg(feature = "sync")]
pub const ZMIDI_SYNC_RECEIVER_WORDS: usize = 1;

/// Storage of one sync receiver. The program gives it; [`zmidi_sync_init`] sets it up.
#[cfg(feature = "sync")]
#[repr(C)]
pub struct ZmidiSyncReceiver {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_SYNC_RECEIVER_WORDS],
}

#[cfg(feature = "sync")]
const _: () = assert!(size_of::<SyncReceiver>() <= size_of::<ZmidiSyncReceiver>());
#[cfg(feature = "sync")]
const _: () = assert!(align_of::<SyncReceiver>() <= align_of::<ZmidiSyncReceiver>());

/// The kind of a [`ZmidiSyncEvent`].
#[cfg(feature = "sync")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiSyncEventKind {
    /// The message or command gives no event.
    None = 0,
    /// Start: the position is 0 and the receiver waits for the next Timing Clock.
    Start = 1,
    /// Continue: the receiver waits at the held position for the next Timing Clock.
    Continue = 2,
    /// Stop: the receiver stopped and keeps its position.
    Stop = 3,
    /// A Timing Clock while the receiver plays: `position` is the position of this clock.
    Clock = 4,
    /// An accepted Song Position Pointer: `position` is the new position.
    Locate = 5,
    /// An accepted Song Select: `song` is the song number.
    Song = 6,
}

/// A start, stop or continue command of the program (a front panel).
#[cfg(feature = "sync")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiSyncCommand {
    /// Start: position 0, then play at the next Timing Clock.
    Start = 1,
    /// Continue: play from the held position at the next Timing Clock.
    Continue = 2,
    /// Stop: stop at once and keep the position.
    Stop = 3,
}

/// The event of one message or command.
#[cfg(feature = "sync")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiSyncEvent {
    /// A [`ZmidiSyncEventKind`] value.
    pub kind: u32,
    /// The position in MIDI clocks of a Clock or Locate event.
    pub position: u32,
    /// The song number of a Song event.
    pub song: u8,
}

/// The information of a sync receiver.
#[cfg(feature = "sync")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiSyncInfo {
    /// The position in MIDI clocks: the position of the next Timing Clock that plays.
    pub position: u32,
    /// The run state: 0 stopped, 1 waits for the first Timing Clock, 2 plays.
    pub run: u8,
    /// The song of the last accepted Song Select.
    pub song: u8,
    /// MIDI sync mode on.
    pub sync_mode: bool,
}

#[cfg(feature = "sync")]
fn sync_event(event: Option<SyncEvent>) -> ZmidiSyncEvent {
    use ZmidiSyncEventKind as K;
    let (kind, position, song) = match event {
        None => (K::None, 0, 0),
        Some(SyncEvent::Start) => (K::Start, 0, 0),
        Some(SyncEvent::Continue) => (K::Continue, 0, 0),
        Some(SyncEvent::Stop) => (K::Stop, 0, 0),
        Some(SyncEvent::Clock { position }) => (K::Clock, position, 0),
        Some(SyncEvent::Locate { position }) => (K::Locate, position, 0),
        Some(SyncEvent::Song { number }) => (K::Song, 0, number),
    };
    ZmidiSyncEvent {
        kind: kind as u32,
        position,
        song,
    }
}

/// Runs `f` on the sync receiver at `state`.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_sync_init`].
#[cfg(feature = "sync")]
unsafe fn with_sync(
    state: *mut ZmidiSyncReceiver,
    f: impl FnOnce(&mut SyncReceiver),
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match unsafe { state.cast::<SyncReceiver>().as_mut() } {
        Some(s) => {
            f(s);
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::InvalidArgument,
    }
}

/// Sets up `state`: stopped at position 0, song 0, MIDI sync mode on.
///
/// # Safety
/// `state` is null or points to writable storage of a [`ZmidiSyncReceiver`].
#[cfg(feature = "sync")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_sync_init(state: *mut ZmidiSyncReceiver) -> ZmidiStatus {
    if state.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe { state.cast::<SyncReceiver>().write(SyncReceiver::new()) };
    ZmidiStatus::Ok
}

/// Takes one System `message` and writes its event into `event` (the kind none when the message
/// gives no event). A message that is not a valid System message gives
/// [`ZmidiStatus::InvalidArgument`] and changes nothing.
///
/// # Safety
/// `state` was set up by [`zmidi_sync_init`]; `message` is readable; `event` is writable.
#[cfg(feature = "sync")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_sync_receive(
    state: *mut ZmidiSyncReceiver,
    message: *const ZmidiMidi1Message,
    event: *mut ZmidiSyncEvent,
) -> ZmidiStatus {
    if state.is_null() || event.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(m) = (unsafe { message.as_ref() }).and_then(|m| m.to_system()) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { *event = sync_event((*state.cast::<SyncReceiver>()).receive(&m)) };
    ZmidiStatus::Ok
}

/// Takes one `command` of the program (a [`ZmidiSyncCommand`] value) with the transitions of the
/// MIDI command, and writes its event into `event`. An unknown command gives
/// [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `state` was set up by [`zmidi_sync_init`]; `event` is writable.
#[cfg(feature = "sync")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_sync_command(
    state: *mut ZmidiSyncReceiver,
    command: u32,
    event: *mut ZmidiSyncEvent,
) -> ZmidiStatus {
    if state.is_null() || event.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    let command = match command {
        1 => SyncCommand::Start,
        2 => SyncCommand::Continue,
        3 => SyncCommand::Stop,
        _ => return ZmidiStatus::InvalidArgument,
    };
    // SAFETY: caller contract above.
    unsafe { *event = sync_event((*state.cast::<SyncReceiver>()).command(command)) };
    ZmidiStatus::Ok
}

/// The System Reset action of the program: position 0 and stopped; the song and the MIDI sync mode
/// stay.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_sync_init`].
#[cfg(feature = "sync")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_sync_system_reset(state: *mut ZmidiSyncReceiver) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { with_sync(state, SyncReceiver::system_reset) }
}

/// Writes the information of `state` into `info`.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_sync_init`]; `info` is null or writable.
#[cfg(feature = "sync")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_sync_info(
    state: *const ZmidiSyncReceiver,
    info: *mut ZmidiSyncInfo,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(s), Some(info)) = (unsafe { state.cast::<SyncReceiver>().as_ref() }, unsafe {
        info.as_mut()
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    *info = ZmidiSyncInfo {
        position: s.position(),
        run: match s.run() {
            SyncRun::Stopped => 0,
            SyncRun::Waiting => 1,
            SyncRun::Playing => 2,
        },
        song: s.song(),
        sync_mode: s.sync_mode(),
    };
    ZmidiStatus::Ok
}

/// Sets MIDI sync mode on or off; out of it the receiver ignores Song Position Pointer and Song
/// Select.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_sync_init`].
#[cfg(feature = "sync")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_sync_set_mode(
    state: *mut ZmidiSyncReceiver,
    on: bool,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { with_sync(state, |s| s.set_sync_mode(on)) }
}

/// Gives `size_of` the Rust sync receiver, for a binding that checks its copy of the layout
/// (DEC-019 rule 6).
#[cfg(feature = "sync")]
#[no_mangle]
pub extern "C" fn zmidi_sync_receiver_size() -> usize {
    size_of::<SyncReceiver>()
}

// -------------------------------------------------------------------------------- active-sensing

#[cfg(feature = "active-sensing")]
use zmidi_io::{ActiveSensingMonitor, ActiveSensingSender};

/// Storage words of a [`ZmidiActiveSensingMonitor`].
#[cfg(feature = "active-sensing")]
pub const ZMIDI_ACTIVE_SENSING_MONITOR_WORDS: usize = 2;
/// Storage words of a [`ZmidiActiveSensingSender`].
#[cfg(feature = "active-sensing")]
pub const ZMIDI_ACTIVE_SENSING_SENDER_WORDS: usize = 2;
/// The default timeout of the monitor: 330 ms, in microseconds.
#[cfg(feature = "active-sensing")]
pub const ZMIDI_ACTIVE_SENSING_TIMEOUT_US: u32 = 330_000;
/// The default interval of the sender: 270 ms, in microseconds.
#[cfg(feature = "active-sensing")]
pub const ZMIDI_ACTIVE_SENSING_INTERVAL_US: u32 = 270_000;

#[cfg(feature = "active-sensing")]
const _: () = assert!(ZMIDI_ACTIVE_SENSING_TIMEOUT_US == zmidi_io::ACTIVE_SENSING_TIMEOUT_US);
#[cfg(feature = "active-sensing")]
const _: () = assert!(ZMIDI_ACTIVE_SENSING_INTERVAL_US == zmidi_io::ACTIVE_SENSING_INTERVAL_US);

/// Storage of one Active Sensing monitor. The program gives it;
/// [`zmidi_active_sensing_monitor_init`] sets it up.
#[cfg(feature = "active-sensing")]
#[repr(C)]
pub struct ZmidiActiveSensingMonitor {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_ACTIVE_SENSING_MONITOR_WORDS],
}

/// Storage of one Active Sensing sender. The program gives it;
/// [`zmidi_active_sensing_sender_init`] sets it up.
#[cfg(feature = "active-sensing")]
#[repr(C)]
pub struct ZmidiActiveSensingSender {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_ACTIVE_SENSING_SENDER_WORDS],
}

#[cfg(feature = "active-sensing")]
const _: () = assert!(size_of::<ActiveSensingMonitor>() <= size_of::<ZmidiActiveSensingMonitor>());
#[cfg(feature = "active-sensing")]
const _: () =
    assert!(align_of::<ActiveSensingMonitor>() <= align_of::<ZmidiActiveSensingMonitor>());
#[cfg(feature = "active-sensing")]
const _: () = assert!(size_of::<ActiveSensingSender>() <= size_of::<ZmidiActiveSensingSender>());
#[cfg(feature = "active-sensing")]
const _: () = assert!(align_of::<ActiveSensingSender>() <= align_of::<ZmidiActiveSensingSender>());

/// Sets up `monitor` with a timeout of `timeout_us` microseconds, not armed.
///
/// # Safety
/// `monitor` is null or points to writable storage of a [`ZmidiActiveSensingMonitor`].
#[cfg(feature = "active-sensing")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_active_sensing_monitor_init(
    monitor: *mut ZmidiActiveSensingMonitor,
    timeout_us: u32,
) -> ZmidiStatus {
    if monitor.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe {
        monitor
            .cast::<ActiveSensingMonitor>()
            .write(ActiveSensingMonitor::new(timeout_us))
    };
    ZmidiStatus::Ok
}

/// Takes one received byte at the time `now_us`: FEH arms the monitor; while it is armed, each
/// byte restarts the timer.
///
/// # Safety
/// `monitor` is null or was set up by [`zmidi_active_sensing_monitor_init`].
#[cfg(feature = "active-sensing")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_active_sensing_monitor_receive_byte(
    monitor: *mut ZmidiActiveSensingMonitor,
    byte: u8,
    now_us: u64,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match unsafe { monitor.cast::<ActiveSensingMonitor>().as_mut() } {
        Some(m) => {
            m.receive_byte(byte, now_us);
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::InvalidArgument,
    }
}

/// Takes one received message of a transport with no bytes at the time `now_us`;
/// `active_sensing` is true for an Active Sensing message.
///
/// # Safety
/// `monitor` is null or was set up by [`zmidi_active_sensing_monitor_init`].
#[cfg(feature = "active-sensing")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_active_sensing_monitor_receive(
    monitor: *mut ZmidiActiveSensingMonitor,
    active_sensing: bool,
    now_us: u64,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match unsafe { monitor.cast::<ActiveSensingMonitor>().as_mut() } {
        Some(m) => {
            m.receive(active_sensing, now_us);
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::InvalidArgument,
    }
}

/// Checks the timer at the time `now_us`; writes true into `timeout` one time when more than the
/// timeout passed since the last received byte, and the monitor then waits for FEH again. Writes
/// whether the monitor is armed after the check into `armed`.
///
/// # Safety
/// `monitor` was set up by [`zmidi_active_sensing_monitor_init`]; `timeout` and `armed` are
/// writable.
#[cfg(feature = "active-sensing")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_active_sensing_monitor_poll(
    monitor: *mut ZmidiActiveSensingMonitor,
    now_us: u64,
    timeout: *mut bool,
    armed: *mut bool,
) -> ZmidiStatus {
    if monitor.is_null() || timeout.is_null() || armed.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe {
        let m = &mut *monitor.cast::<ActiveSensingMonitor>();
        *timeout = m.poll(now_us);
        *armed = m.is_armed();
    }
    ZmidiStatus::Ok
}

/// Writes whether `monitor` is armed (it received FEH and gave no timeout after it) into `armed`,
/// with no check of the timer.
///
/// # Safety
/// `monitor` was set up by [`zmidi_active_sensing_monitor_init`]; `armed` is writable.
#[cfg(feature = "active-sensing")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_active_sensing_monitor_is_armed(
    monitor: *const ZmidiActiveSensingMonitor,
    armed: *mut bool,
) -> ZmidiStatus {
    if monitor.is_null() || armed.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe { *armed = (*monitor.cast::<ActiveSensingMonitor>()).is_armed() };
    ZmidiStatus::Ok
}

/// Gives `size_of` the Rust monitor, for a binding that checks its copy of the layout (DEC-019 rule
/// 6).
#[cfg(feature = "active-sensing")]
#[no_mangle]
pub extern "C" fn zmidi_active_sensing_monitor_size() -> usize {
    size_of::<ActiveSensingMonitor>()
}

/// Sets up `sender` with an interval of `interval_us` microseconds that starts at time 0.
///
/// # Safety
/// `sender` is null or points to writable storage of a [`ZmidiActiveSensingSender`].
#[cfg(feature = "active-sensing")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_active_sensing_sender_init(
    sender: *mut ZmidiActiveSensingSender,
    interval_us: u32,
) -> ZmidiStatus {
    if sender.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe {
        sender
            .cast::<ActiveSensingSender>()
            .write(ActiveSensingSender::new(interval_us))
    };
    ZmidiStatus::Ok
}

/// Tells the sender that the program sent a byte at the time `now_us`; the interval starts again.
///
/// # Safety
/// `sender` is null or was set up by [`zmidi_active_sensing_sender_init`].
#[cfg(feature = "active-sensing")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_active_sensing_sender_sent(
    sender: *mut ZmidiActiveSensingSender,
    now_us: u64,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match unsafe { sender.cast::<ActiveSensingSender>().as_mut() } {
        Some(s) => {
            s.sent(now_us);
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::InvalidArgument,
    }
}

/// Checks the interval at the time `now_us`; writes true into `send` when no byte was sent for the
/// interval: the program then sends one FEH byte, and the interval starts again.
///
/// # Safety
/// `sender` was set up by [`zmidi_active_sensing_sender_init`]; `send` is writable.
#[cfg(feature = "active-sensing")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_active_sensing_sender_poll(
    sender: *mut ZmidiActiveSensingSender,
    now_us: u64,
    send: *mut bool,
) -> ZmidiStatus {
    if sender.is_null() || send.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe { *send = (*sender.cast::<ActiveSensingSender>()).poll(now_us) };
    ZmidiStatus::Ok
}

/// Gives `size_of` the Rust sender, for a binding that checks its copy of the layout (DEC-019 rule
/// 6).
#[cfg(feature = "active-sensing")]
#[no_mangle]
pub extern "C" fn zmidi_active_sensing_sender_size() -> usize {
    size_of::<ActiveSensingSender>()
}

// ------------------------------------------------------------------------------------------- mpe

#[cfg(feature = "mpe")]
use zmidi_io::{
    MpeClass, MpeEvent, MpeLayout, MpeMessage, MpeNoteAllocator, MpeReceiver, MpeRole,
    MpeSensitivity, MpeZone,
};

/// Storage words of a [`ZmidiMpeReceiver`].
#[cfg(feature = "mpe")]
pub const ZMIDI_MPE_RECEIVER_WORDS: usize = 10;
/// Storage words of a [`ZmidiMpeNoteAllocator`].
#[cfg(feature = "mpe")]
pub const ZMIDI_MPE_NOTE_ALLOCATOR_WORDS: usize = 13;

/// Storage of one MPE receiver (all 16 channels of one input). The program gives it;
/// [`zmidi_mpe_receiver_init`] sets it up.
#[cfg(feature = "mpe")]
#[repr(C)]
pub struct ZmidiMpeReceiver {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_MPE_RECEIVER_WORDS],
}

/// Storage of one MPE note allocator of a sender. The program gives it;
/// [`zmidi_mpe_note_allocator_init`] sets it up.
#[cfg(feature = "mpe")]
#[repr(C)]
pub struct ZmidiMpeNoteAllocator {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_MPE_NOTE_ALLOCATOR_WORDS],
}

#[cfg(feature = "mpe")]
const _: () = assert!(size_of::<MpeReceiver>() <= size_of::<ZmidiMpeReceiver>());
#[cfg(feature = "mpe")]
const _: () = assert!(align_of::<MpeReceiver>() <= align_of::<ZmidiMpeReceiver>());
#[cfg(feature = "mpe")]
const _: () = assert!(size_of::<MpeNoteAllocator>() <= size_of::<ZmidiMpeNoteAllocator>());
#[cfg(feature = "mpe")]
const _: () = assert!(align_of::<MpeNoteAllocator>() <= align_of::<ZmidiMpeNoteAllocator>());

/// A Zone: the `zone` argument and field values.
#[cfg(feature = "mpe")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiMpeZone {
    /// Manager Channel 1, Member Channels from Channel 2 upwards.
    Lower = 0,
    /// Manager Channel 16, Member Channels from Channel 15 downwards.
    Upper = 1,
}

/// The role of a channel.
#[cfg(feature = "mpe")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiMpeRole {
    /// The channel is in no Zone.
    NoZone = 0,
    /// The Manager Channel of a Zone.
    Manager = 1,
    /// A Member Channel of a Zone.
    Member = 2,
}

/// The kind of a [`ZmidiMpeEvent`].
#[cfg(feature = "mpe")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiMpeEventKind {
    /// The channel is in no Zone: a conventional MIDI 1.0 message.
    NoZone = 0,
    /// A message on a channel of `zone`; `manager` is true on the Manager Channel.
    Zone = 1,
    /// An MCM changed the layout; `reset` holds the channels to reset (bit n: channel n).
    Layout = 2,
    /// RPN 0 changed the Manager (`manager` true) or Member sensitivity of `zone`.
    Sensitivity = 3,
    /// A Mode message changed the MIDI Mode of `zone`.
    Mode = 4,
    /// A message that MPE prohibits or that has no function.
    Ignored = 5,
}

/// The class of a message kind in Appendix E.
#[cfg(feature = "mpe")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiMpeClass {
    /// M: Mandatory.
    Mandatory = 0,
    /// O: Optional.
    Optional = 1,
    /// P: Prohibited.
    Prohibited = 2,
}

/// A message kind of Appendix E, the `message` argument of [`zmidi_mpe_class`].
#[cfg(feature = "mpe")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiMpeMessage {
    /// RPN 00H 06H, the MPE Configuration Message.
    Mcm = 0,
    /// RPN 0, Pitch Bend Sensitivity.
    PitchBendSensitivity = 1,
    /// Pitch Bend.
    PitchBend = 2,
    /// Channel Pressure.
    ChannelPressure = 3,
    /// Control Change 74.
    Brightness = 4,
    /// Polyphonic Key Pressure.
    PolyPressure = 5,
    /// Control Change 120.
    AllSoundOff = 6,
    /// Control Change 121.
    ResetAllControllers = 7,
    /// Control Change 122.
    LocalControl = 8,
    /// Control Change 123.
    AllNotesOff = 9,
    /// Control Change 124.
    OmniOff = 10,
    /// Control Change 125.
    OmniOn = 11,
    /// Control Change 126.
    MonoModeOn = 12,
    /// Control Change 127.
    PolyModeOn = 13,
    /// Each other Control Change.
    OtherControlChange = 14,
    /// Each other RPN.
    OtherRpn = 15,
    /// Each NRPN.
    Nrpn = 16,
    /// Program Change.
    ProgramChange = 17,
    /// Bank Select.
    BankSelect = 18,
    /// Note On and Note Off.
    Note = 19,
    /// System messages.
    System = 20,
}

/// A Zone layout: the Member Channel count of each Zone (0 for an inactive Zone, at most 15; two
/// active Zones have at most 14 in all).
#[cfg(feature = "mpe")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMpeLayout {
    /// Member Channels of the Lower Zone.
    pub lower: u8,
    /// Member Channels of the Upper Zone.
    pub upper: u8,
}

/// A Pitch Bend Sensitivity: semitones and cents, 0 to 127 each.
#[cfg(feature = "mpe")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMpeSensitivity {
    /// Semitones.
    pub semitones: u8,
    /// Cents.
    pub cents: u8,
}

/// The event of one received message.
#[cfg(feature = "mpe")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMpeEvent {
    /// A [`ZmidiMpeEventKind`] value.
    pub kind: u32,
    /// A [`ZmidiMpeZone`] value for the kinds zone, sensitivity and mode; 0 otherwise.
    pub zone: u32,
    /// The Manager Channel (kind zone) or the Manager sensitivity (kind sensitivity).
    pub manager: bool,
    /// The channels to reset for the kind layout (bit n: the channel with packet value n).
    pub reset: u16,
}

/// The state of an MPE receiver.
#[cfg(feature = "mpe")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMpeInfo {
    /// The layout.
    pub layout: ZmidiMpeLayout,
    /// The MIDI Mode of each Zone (index: [`ZmidiMpeZone`]): 3 or 4.
    pub mode: [u8; 2],
    /// The Manager Channel sensitivity of each Zone.
    pub manager: [ZmidiMpeSensitivity; 2],
    /// The Member Channel sensitivity of each Zone.
    pub member: [ZmidiMpeSensitivity; 2],
}

/// The values of one channel of an MPE receiver.
#[cfg(feature = "mpe")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMpeChannel {
    /// The last Pitch Bend, 0 to 16383.
    pub pitch_bend: u16,
    /// The last Channel Pressure.
    pub pressure: u8,
    /// The last Control Change 74.
    pub brightness: u8,
    /// The channel is in a Zone; `pitch_offset` is then valid.
    pub in_zone: bool,
    /// The pitch offset in hundredths of a cent: the Manager plus the Member Pitch Bend.
    pub pitch_offset: i32,
}

#[cfg(feature = "mpe")]
const fn rust_zone(zone: u32) -> Option<MpeZone> {
    match zone {
        0 => Some(MpeZone::Lower),
        1 => Some(MpeZone::Upper),
        _ => None,
    }
}

#[cfg(feature = "mpe")]
const fn rust_layout(lower: u8, upper: u8) -> Option<MpeLayout> {
    match MpeLayout::new(lower, upper) {
        Ok(l) => Some(l),
        Err(_) => None,
    }
}

#[cfg(feature = "mpe")]
const fn c_layout(layout: MpeLayout) -> ZmidiMpeLayout {
    ZmidiMpeLayout {
        lower: layout.members(MpeZone::Lower),
        upper: layout.members(MpeZone::Upper),
    }
}

#[cfg(feature = "mpe")]
const fn rust_sensitivity(semitones: u8, cents: u8) -> Option<MpeSensitivity> {
    if semitones > 0x7F || cents > 0x7F {
        return None;
    }
    Some(MpeSensitivity { semitones, cents })
}

#[cfg(feature = "mpe")]
const fn c_sensitivity(s: MpeSensitivity) -> ZmidiMpeSensitivity {
    ZmidiMpeSensitivity {
        semitones: s.semitones,
        cents: s.cents,
    }
}

#[cfg(feature = "mpe")]
const fn mpe_event(event: MpeEvent) -> ZmidiMpeEvent {
    use ZmidiMpeEventKind as K;
    let (kind, zone, manager, reset) = match event {
        MpeEvent::NoZone => (K::NoZone, 0, false, 0),
        MpeEvent::Zone { zone, manager } => (K::Zone, zone as u32, manager, 0),
        MpeEvent::Layout { reset } => (K::Layout, 0, false, reset),
        MpeEvent::Sensitivity { zone, manager } => (K::Sensitivity, zone as u32, manager, 0),
        MpeEvent::Mode { zone } => (K::Mode, zone as u32, false, 0),
        MpeEvent::Ignored => (K::Ignored, 0, false, 0),
    };
    ZmidiMpeEvent {
        kind: kind as u32,
        zone,
        manager,
        reset,
    }
}

/// Writes the layout of `lower` and `upper` Member Channels into `layout`. A count above 15, or
/// two active Zones with more than 14 Member Channels, gives [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `layout` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_layout_new(
    lower: u8,
    upper: u8,
    layout: *mut ZmidiMpeLayout,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(out), Ok(l)) = (unsafe { layout.as_mut() }, MpeLayout::new(lower, upper)) else {
        return ZmidiStatus::InvalidArgument;
    };
    *out = c_layout(l);
    ZmidiStatus::Ok
}

/// Writes the layout of `lower` and `upper` Member Channels after one MCM with `members` for `zone` (the overlap rule of AMB-M1-100-03;
/// a value above 15 gives the same layout) into `out`.
///
/// # Safety
/// `out` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_layout_with_mcm(
    lower: u8,
    upper: u8,
    zone: u32,
    members: u8,
    out: *mut ZmidiMpeLayout,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(out), Some(l), Some(z)) = (
        unsafe { out.as_mut() },
        rust_layout(lower, upper),
        rust_zone(zone),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    *out = c_layout(l.with_mcm(z, members));
    ZmidiStatus::Ok
}

/// Writes the role of `channel` (0 to 15) in the layout of `lower` and `upper` Member Channels into `role` (a [`ZmidiMpeRole`] value) and its Zone into
/// `zone` (0 for no Zone).
///
/// # Safety
/// `role` and `zone` are writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_layout_role(
    lower: u8,
    upper: u8,
    channel: u8,
    role: *mut u32,
    zone: *mut u32,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(role), Some(zone), Some(l), Ok(c)) = (
        unsafe { role.as_mut() },
        unsafe { zone.as_mut() },
        rust_layout(lower, upper),
        Channel::new(channel),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    (*role, *zone) = match l.role(c) {
        MpeRole::NoZone => (ZmidiMpeRole::NoZone as u32, 0),
        MpeRole::Manager(z) => (ZmidiMpeRole::Manager as u32, z as u32),
        MpeRole::Member(z) => (ZmidiMpeRole::Member as u32, z as u32),
    };
    ZmidiStatus::Ok
}

/// Writes whether `zone` is active into `active` and its lowest Member Channel (0 to 15; 0 for an
/// inactive Zone) into `channel`.
///
/// # Safety
/// `active` and `channel` are writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_layout_lowest_member(
    lower: u8,
    upper: u8,
    zone: u32,
    active: *mut bool,
    channel: *mut u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(active), Some(channel), Some(l), Some(z)) = (
        unsafe { active.as_mut() },
        unsafe { channel.as_mut() },
        rust_layout(lower, upper),
        rust_zone(zone),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    let lowest = l.lowest_member(z);
    *active = lowest.is_some();
    *channel = lowest.map_or(0, Channel::index);
    ZmidiStatus::Ok
}

/// Writes the C.3 Pitch Bend value (0 to 16383) of a sender for `offset` (hundredths of a cent)
/// at a sensitivity of `semitones` and `cents` into `value`.
///
/// # Safety
/// `value` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_pitch_bend_value(
    offset: i32,
    semitones: u8,
    cents: u8,
    value: *mut u16,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(out), Some(s)) = (
        unsafe { value.as_mut() },
        rust_sensitivity(semitones, cents),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    *out = zmidi_io::pitch_bend_value(offset, s);
    ZmidiStatus::Ok
}

/// Writes the C.5 pitch offset (hundredths of a cent) of a received Pitch Bend `value` (0 to
/// 16383) at a sensitivity of `semitones` and `cents` into `offset`.
///
/// # Safety
/// `offset` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_pitch_offset(
    value: u16,
    semitones: u8,
    cents: u8,
    offset: *mut i32,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(out), Some(s), true) = (
        unsafe { offset.as_mut() },
        rust_sensitivity(semitones, cents),
        value <= 0x3FFF,
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    *out = zmidi_io::pitch_offset(value, s);
    ZmidiStatus::Ok
}

/// Writes the Appendix E class (a [`ZmidiMpeClass`] value) of `message` (a [`ZmidiMpeMessage`]
/// value) on the Manager Channel or a Member Channel, to send or to receive, with the MIDI Mode
/// (`mono` for Mode 4) into `class`.
///
/// # Safety
/// `class` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_class(
    message: u32,
    manager: bool,
    transmit: bool,
    mono: bool,
    class: *mut u32,
) -> ZmidiStatus {
    use MpeMessage as K;
    const KINDS: [MpeMessage; 21] = [
        K::Mcm,
        K::PitchBendSensitivity,
        K::PitchBend,
        K::ChannelPressure,
        K::Brightness,
        K::PolyPressure,
        K::AllSoundOff,
        K::ResetAllControllers,
        K::LocalControl,
        K::AllNotesOff,
        K::OmniOff,
        K::OmniOn,
        K::MonoModeOn,
        K::PolyModeOn,
        K::OtherControlChange,
        K::OtherRpn,
        K::Nrpn,
        K::ProgramChange,
        K::BankSelect,
        K::Note,
        K::System,
    ];
    // SAFETY: caller contract above.
    let (Some(out), Some(&kind)) = (unsafe { class.as_mut() }, KINDS.get(message as usize)) else {
        return ZmidiStatus::InvalidArgument;
    };
    *out = match zmidi_io::mpe_class(kind, manager, transmit, mono) {
        MpeClass::Mandatory => ZmidiMpeClass::Mandatory,
        MpeClass::Optional => ZmidiMpeClass::Optional,
        MpeClass::Prohibited => ZmidiMpeClass::Prohibited,
    } as u32;
    ZmidiStatus::Ok
}

/// Sets up `state`: no Zone, Mode 3, the controls of each channel at 8192, 0 and 64.
///
/// # Safety
/// `state` is null or points to writable storage of a [`ZmidiMpeReceiver`].
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_receiver_init(state: *mut ZmidiMpeReceiver) -> ZmidiStatus {
    if state.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe { state.cast::<MpeReceiver>().write(MpeReceiver::new()) };
    ZmidiStatus::Ok
}

/// Takes one Channel Voice `message` and the `parameter` event that [`zmidi_parameters_receive`]
/// of its channel gave for it (change none for no event), and writes the event into `event`. A
/// message that is not a valid Channel Voice message, or an invalid parameter event, gives
/// [`ZmidiStatus::InvalidArgument`] and changes nothing.
///
/// # Safety
/// `state` was set up by [`zmidi_mpe_receiver_init`]; `message` and `parameter` are readable;
/// `event` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_receive(
    state: *mut ZmidiMpeReceiver,
    message: *const ZmidiMidi1Message,
    parameter: *const ZmidiParameterEvent,
    event: *mut ZmidiMpeEvent,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(s), Some(p), Some(out), Some(m)) = (
        unsafe { state.cast::<MpeReceiver>().as_mut() },
        unsafe { parameter.as_ref() },
        unsafe { event.as_mut() },
        unsafe { channel_voice_at(message) },
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    let parameter = if p.change == ZmidiParameterChange::None as u32 {
        None
    } else {
        let (Some(parameter), Some(change)) =
            (rust_parameter(p.parameter), rust_change(p.change, p.value))
        else {
            return ZmidiStatus::InvalidArgument;
        };
        Some(ParameterEvent { parameter, change })
    };
    *out = mpe_event(s.receive(&m, parameter));
    ZmidiStatus::Ok
}

/// Sets the layout of `lower` and `upper` Member Channels of the program (a power-up default or a
/// front panel setting) and writes the channels to reset into `reset`. An invalid layout gives [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `state` was set up by [`zmidi_mpe_receiver_init`]; `reset` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_set_layout(
    state: *mut ZmidiMpeReceiver,
    lower: u8,
    upper: u8,
    reset: *mut u16,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(s), Some(out), Some(l)) = (
        unsafe { state.cast::<MpeReceiver>().as_mut() },
        unsafe { reset.as_mut() },
        rust_layout(lower, upper),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    *out = s.set_layout(l);
    ZmidiStatus::Ok
}

/// Writes the layout, the MIDI Mode and the sensitivities of each Zone into `info`.
///
/// # Safety
/// `state` was set up by [`zmidi_mpe_receiver_init`]; `info` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_info(
    state: *const ZmidiMpeReceiver,
    info: *mut ZmidiMpeInfo,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(s), Some(out)) = (unsafe { state.cast::<MpeReceiver>().as_ref() }, unsafe {
        info.as_mut()
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let zones = [MpeZone::Lower, MpeZone::Upper];
    *out = ZmidiMpeInfo {
        layout: c_layout(s.layout()),
        mode: zones.map(|z| s.mode(z)),
        manager: zones.map(|z| c_sensitivity(s.sensitivity(z, true))),
        member: zones.map(|z| c_sensitivity(s.sensitivity(z, false))),
    };
    ZmidiStatus::Ok
}

/// Writes the values of `channel` (0 to 15) into `out`.
///
/// # Safety
/// `state` was set up by [`zmidi_mpe_receiver_init`]; `out` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_channel(
    state: *const ZmidiMpeReceiver,
    channel: u8,
    out: *mut ZmidiMpeChannel,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(s), Some(out), Ok(c)) = (
        unsafe { state.cast::<MpeReceiver>().as_ref() },
        unsafe { out.as_mut() },
        Channel::new(channel),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    let offset = s.pitch_offset(c);
    *out = ZmidiMpeChannel {
        pitch_bend: s.pitch_bend(c),
        pressure: s.pressure(c),
        brightness: s.brightness(c),
        in_zone: offset.is_some(),
        pitch_offset: offset.unwrap_or(0),
    };
    ZmidiStatus::Ok
}

/// Gives `size_of` the Rust receiver, for a binding that checks its copy of the layout (DEC-019
/// rule 6).
#[cfg(feature = "mpe")]
#[no_mangle]
pub extern "C" fn zmidi_mpe_receiver_size() -> usize {
    size_of::<MpeReceiver>()
}

/// Writes the MCM of `zone` with `members` (0 to 15) into `out` (`capacity` entries, at least 3),
/// and 3 into `count`.
///
/// # Safety
/// `out` points to `capacity` writable entries; `count` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_mcm_messages(
    zone: u32,
    members: u8,
    out: *mut ZmidiMidi1Message,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    let Some(Ok(messages)) = rust_zone(zone).map(|z| zmidi_io::mcm_messages(z, members)) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { put_messages(&messages, out, capacity, count) }
}

/// Writes RPN 0 with `semitones` (0 to 96) and no Data Entry LSB on `channel` (0 to 15) into `out`
/// (`capacity` entries, at least 3), and 3 into `count`.
///
/// # Safety
/// `out` points to `capacity` writable entries; `count` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_sensitivity_messages(
    channel: u8,
    semitones: u8,
    out: *mut ZmidiMidi1Message,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    let Ok(messages) =
        Channel::new(channel).and_then(|c| zmidi_io::sensitivity_messages(c, semitones))
    else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { put_messages(&messages, out, capacity, count) }
}

/// Writes RPN 0 with `manager` semitones on the Manager Channel and with `member` semitones on
/// each Member Channel of `zone` into `out` (`capacity` entries, 3 for each channel), and the
/// count into `count`. An inactive Zone or a value above 96 gives
/// [`ZmidiStatus::InvalidArgument`]; a small `capacity` gives [`ZmidiStatus::BufferTooSmall`].
///
/// # Safety
/// `out` points to `capacity` writable entries; `count` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_zone_sensitivity_messages(
    lower: u8,
    upper: u8,
    zone: u32,
    manager: u8,
    member: u8,
    out: *mut ZmidiMidi1Message,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    let (Some(l), Some(z)) = (rust_layout(lower, upper), rust_zone(zone)) else {
        return ZmidiStatus::InvalidArgument;
    };
    let blank = Midi1ChannelVoiceMessage::ProgramChange(
        match zmidi_ump::Midi1ProgramChange::try_new(
            zmidi_ump::Group::from_bits(0),
            Channel::from_bits(0),
            0,
        ) {
            Ok(m) => m,
            Err(_) => return ZmidiStatus::InvalidArgument,
        },
    );
    let mut messages = [blank; 48];
    match zmidi_io::zone_sensitivity_messages(l, z, manager, member, &mut messages) {
        // SAFETY: caller contract above.
        Ok(n) => unsafe { put_messages(&messages[..n], out, capacity, count) },
        Err(_) => ZmidiStatus::InvalidArgument,
    }
}

/// Writes the Mode message of `zone` on its lowest Member Channel into `out`: Mono Mode On with M =
/// the Member Channel count (`mono`), or Poly Mode On. An inactive Zone gives
/// [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `out` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_mode_message(
    lower: u8,
    upper: u8,
    zone: u32,
    mono: bool,
    out: *mut ZmidiMidi1Message,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(out), Some(l), Some(z)) = (
        unsafe { out.as_mut() },
        rust_layout(lower, upper),
        rust_zone(zone),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Ok(m) = zmidi_io::mode_message(l, z, mono) else {
        return ZmidiStatus::InvalidArgument;
    };
    *out = ZmidiMidi1Message::channel_voice(m);
    ZmidiStatus::Ok
}

/// Writes the start of a note on `channel` (0 to 15) into `out` (`capacity` entries, at least 4):
/// Pitch Bend, Control Change 74, Channel Pressure and Note On; 4 into `count`. Velocity 0 or a
/// value out of range gives [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `out` points to `capacity` writable entries; `count` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_mpe_note_messages(
    channel: u8,
    note: u8,
    velocity: u8,
    pitch_bend: u16,
    brightness: u8,
    pressure: u8,
    out: *mut ZmidiMidi1Message,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    let Ok(messages) = Channel::new(channel)
        .and_then(|c| zmidi_io::note_messages(c, note, velocity, pitch_bend, brightness, pressure))
    else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { put_messages(&messages, out, capacity, count) }
}

/// Runs `f` on the note allocator at `state`.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mpe_note_allocator_init`].
#[cfg(feature = "mpe")]
unsafe fn with_allocator(
    state: *mut ZmidiMpeNoteAllocator,
    f: impl FnOnce(&mut MpeNoteAllocator),
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match unsafe { state.cast::<MpeNoteAllocator>().as_mut() } {
        Some(s) => {
            f(s);
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::InvalidArgument,
    }
}

/// Sets up `state`: no active note, the same-note option off.
///
/// # Safety
/// `state` is null or points to writable storage of a [`ZmidiMpeNoteAllocator`].
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_note_allocator_init(
    state: *mut ZmidiMpeNoteAllocator,
) -> ZmidiStatus {
    if state.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe {
        state
            .cast::<MpeNoteAllocator>()
            .write(MpeNoteAllocator::new())
    };
    ZmidiStatus::Ok
}

/// Gives the Member Channel of `zone` for a new `note`: writes whether the Zone is active into
/// `found` and the channel (0 to 15) into `channel`.
///
/// # Safety
/// `state` was set up by [`zmidi_mpe_note_allocator_init`]; `found` and `channel` are writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_note_on(
    state: *mut ZmidiMpeNoteAllocator,
    lower: u8,
    upper: u8,
    zone: u32,
    note: u8,
    found: *mut bool,
    channel: *mut u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(s), Some(found), Some(channel), Some(l), Some(z), true) = (
        unsafe { state.cast::<MpeNoteAllocator>().as_mut() },
        unsafe { found.as_mut() },
        unsafe { channel.as_mut() },
        rust_layout(lower, upper),
        rust_zone(zone),
        note <= 0x7F,
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    let c = s.note_on(l, z, note);
    *found = c.is_some();
    *channel = c.map_or(0, Channel::index);
    ZmidiStatus::Ok
}

/// One note on `channel` (0 to 15) ended.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mpe_note_allocator_init`].
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_note_off(
    state: *mut ZmidiMpeNoteAllocator,
    channel: u8,
) -> ZmidiStatus {
    let Ok(c) = Channel::new(channel) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { with_allocator(state, |s| s.note_off(c)) }
}

/// Writes the count of active notes on `channel` (0 to 15) into `count`.
///
/// # Safety
/// `state` was set up by [`zmidi_mpe_note_allocator_init`]; `count` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_active_notes(
    state: *const ZmidiMpeNoteAllocator,
    channel: u8,
    count: *mut u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(s), Some(count), Ok(c)) = (
        unsafe { state.cast::<MpeNoteAllocator>().as_ref() },
        unsafe { count.as_mut() },
        Channel::new(channel),
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    *count = s.active_notes(c);
    ZmidiStatus::Ok
}

/// Writes the same-note option into `on`.
///
/// # Safety
/// `state` was set up by [`zmidi_mpe_note_allocator_init`]; `on` is writable.
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_same_note(
    state: *const ZmidiMpeNoteAllocator,
    on: *mut bool,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(s), Some(on)) = (
        unsafe { state.cast::<MpeNoteAllocator>().as_ref() },
        unsafe { on.as_mut() },
    ) else {
        return ZmidiStatus::InvalidArgument;
    };
    *on = s.same_note();
    ZmidiStatus::Ok
}

/// Sets the same-note option.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mpe_note_allocator_init`].
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_set_same_note(
    state: *mut ZmidiMpeNoteAllocator,
    on: bool,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { with_allocator(state, |s| s.set_same_note(on)) }
}

/// No active note and no age; the same-note option stays.
///
/// # Safety
/// `state` is null or was set up by [`zmidi_mpe_note_allocator_init`].
#[cfg(feature = "mpe")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mpe_note_reset(state: *mut ZmidiMpeNoteAllocator) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { with_allocator(state, MpeNoteAllocator::reset) }
}

/// Gives `size_of` the Rust note allocator, for a binding that checks its copy of the layout
/// (DEC-019 rule 6).
#[cfg(feature = "mpe")]
#[no_mangle]
pub extern "C" fn zmidi_mpe_note_allocator_size() -> usize {
    size_of::<MpeNoteAllocator>()
}
