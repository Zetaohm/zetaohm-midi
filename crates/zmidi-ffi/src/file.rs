//! The C ABI of zmidi-file (DEC-019): the MIDI Clip File reader ([`ClipReader`], REQ-FFI-FILE-001)
//! and writer ([`ClipWriter`], REQ-FFI-FILE-002, -003). The program gives the storage of each
//! object and the byte buffer of the clip; the object keeps a pointer to that buffer, so the
//! buffer stays valid while the program uses the object.

use core::mem::{align_of, size_of};

use zmidi_file::{
    ClipConfig, ClipError, ClipReader, ClipSection, ClipWriteError, ClipWriter, ProfileDestination,
    ProfileOn,
};
use zmidi_ump::{Group, SetTempo, SetTimeSignature};

use crate::status::ZmidiStatus;

/// Storage words of a [`ZmidiClipReader`] on a 64-bit target.
#[cfg(target_pointer_width = "64")]
pub const ZMIDI_CLIP_READER_WORDS: usize = 7;
/// Storage words of a [`ZmidiClipReader`] on a 32-bit or 16-bit target.
#[cfg(not(target_pointer_width = "64"))]
pub const ZMIDI_CLIP_READER_WORDS: usize = 6;
/// Storage words of a [`ZmidiClipWriter`] on a 64-bit target.
#[cfg(target_pointer_width = "64")]
pub const ZMIDI_CLIP_WRITER_WORDS: usize = 12;
/// Storage words of a [`ZmidiClipWriter`] on a 32-bit or 16-bit target.
#[cfg(not(target_pointer_width = "64"))]
pub const ZMIDI_CLIP_WRITER_WORDS: usize = 10;

/// Storage of one clip reader. The program gives it; [`zmidi_clip_reader_init`] sets it up.
#[repr(C)]
pub struct ZmidiClipReader {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_CLIP_READER_WORDS],
}

/// Storage of one clip writer. The program gives it; [`zmidi_clip_writer_init`] sets it up.
#[repr(C)]
pub struct ZmidiClipWriter {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_CLIP_WRITER_WORDS],
}

type Reader = ClipReader<'static>;
type Writer = ClipWriter<'static>;

const _: () = assert!(size_of::<Reader>() <= size_of::<ZmidiClipReader>());
const _: () = assert!(align_of::<Reader>() <= align_of::<ZmidiClipReader>());
const _: () = assert!(size_of::<Writer>() <= size_of::<ZmidiClipWriter>());
const _: () = assert!(align_of::<Writer>() <= align_of::<ZmidiClipWriter>());

/// One message of a clip.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiClipEvent {
    /// 0: Clip Configuration Header; 1: Clip Sequence Data.
    pub section: u8,
    /// Absolute ticks since the start of the file, in the unit of the DCTPQ in force.
    pub ticks: u64,
    /// Words of the UMP, 1 to 4.
    pub word_count: u8,
    /// The words; words after `word_count` are 0.
    pub words: [u32; 4],
}

/// One Set Profile On message of the Clip Configuration Header.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiClipProfileOn {
    /// Profile ID, 5 bytes of 7-bit data.
    pub profile_id: [u8; 5],
    /// 0: one Channel (`channel`); 1: the whole Group; 2: the whole Function Block.
    pub destination: u8,
    /// Channel, 0 to 15, when `destination` is 0.
    pub channel: u8,
    /// Number of Channels Requested (14 bits).
    pub channels: u16,
}

/// The Clip Configuration Header of a new clip.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ZmidiClipConfig {
    /// Ticks Per Quarter Note of the DCTPQ, 1 to 65535.
    pub ticks_per_quarter_note: u16,
    /// Group of the Flex Data and Profile messages, 0 to 15.
    pub group: u8,
    /// True to write a Set Tempo.
    pub has_tempo: bool,
    /// Set Tempo: units of 10 ns per quarter note.
    pub tempo_10ns: u32,
    /// True to write a Set Time Signature.
    pub has_time_signature: bool,
    /// Set Time Signature: Numerator.
    pub numerator: u8,
    /// Set Time Signature: Denominator as a power of 2.
    pub denominator: u8,
    /// Set Time Signature: Number of 1/32 Notes.
    pub thirty_second_notes: u8,
    /// Set Profile On messages (`profile_count` of them), or null.
    pub profiles: *const ZmidiClipProfileOn,
    /// Count of `profiles`.
    pub profile_count: usize,
}

/// The error of a file function, in detail.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiClipError {
    /// No error.
    None = 0,
    /// The bytes do not start with the File Header.
    BadHeader = 1,
    /// A trailing partial word, or a UMP cut by the end of the data.
    Truncated = 2,
    /// A byte follows End of Clip.
    DataAfterEnd = 3,
    /// The buffer cannot hold the bytes.
    BufferTooSmall = 4,
    /// A bad clip configuration.
    InvalidConfig = 5,
    /// The words are not one whole UMP.
    InvalidMessage = 6,
    /// A tick earlier than the last one.
    OutOfOrder = 7,
    /// A Property Exchange message.
    PropertyExchange = 8,
    /// A message that the writer writes itself.
    ReservedMessage = 9,
    /// Set Tempo off the 1/24 quarter note grid.
    TempoTiming = 10,
    /// Set Time Signature not at a bar boundary.
    BarTiming = 11,
    /// A different tick inside one multi-UMP Lyrics or Ruby message.
    FlexTiming = 12,
    /// The call is not allowed in the current phase.
    WrongPhase = 13,
}

fn read_error(e: ClipError) -> ZmidiClipError {
    match e {
        ClipError::BadHeader => ZmidiClipError::BadHeader,
        ClipError::Truncated => ZmidiClipError::Truncated,
        ClipError::DataAfterEnd => ZmidiClipError::DataAfterEnd,
    }
}

fn write_error(e: ClipWriteError) -> (ZmidiStatus, ZmidiClipError) {
    use ClipWriteError as E;
    use ZmidiClipError as C;
    match e {
        E::BufferTooSmall => (ZmidiStatus::BufferTooSmall, C::BufferTooSmall),
        E::InvalidConfig => (ZmidiStatus::InvalidArgument, C::InvalidConfig),
        E::InvalidMessage => (ZmidiStatus::InvalidArgument, C::InvalidMessage),
        E::OutOfOrder => (ZmidiStatus::Rejected, C::OutOfOrder),
        E::PropertyExchange => (ZmidiStatus::Rejected, C::PropertyExchange),
        E::ReservedMessage => (ZmidiStatus::Rejected, C::ReservedMessage),
        E::TempoTiming => (ZmidiStatus::Rejected, C::TempoTiming),
        E::BarTiming => (ZmidiStatus::Rejected, C::BarTiming),
        E::FlexTiming => (ZmidiStatus::Rejected, C::FlexTiming),
        E::WrongPhase => (ZmidiStatus::Rejected, C::WrongPhase),
    }
}

/// Writes `value` to `error` when `error` is not null.
///
/// # Safety
/// `error` is null or writable.
unsafe fn set_error(error: *mut ZmidiClipError, value: ZmidiClipError) {
    if !error.is_null() {
        // SAFETY: caller contract above.
        unsafe { error.write(value) };
    }
}

/// Gives the status of `r` and writes its detail to `error`.
///
/// # Safety
/// `error` is null or writable.
unsafe fn finish_write(r: Result<(), ClipWriteError>, error: *mut ZmidiClipError) -> ZmidiStatus {
    let (status, detail) = match r {
        Ok(()) => (ZmidiStatus::Ok, ZmidiClipError::None),
        Err(e) => write_error(e),
    };
    // SAFETY: caller contract above.
    unsafe { set_error(error, detail) };
    status
}

/// Sets up `reader` over the clip `data` (`len` bytes). `data` stays valid while `reader` is used.
/// A bad File Header gives [`ZmidiStatus::Rejected`] and [`ZmidiClipError::BadHeader`].
///
/// # Safety
/// `reader` points to writable storage of a [`ZmidiClipReader`]; `data` points to `len` bytes that
/// stay valid while the reader is used; `error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_clip_reader_init(
    reader: *mut ZmidiClipReader,
    data: *const u8,
    len: usize,
    error: *mut ZmidiClipError,
) -> ZmidiStatus {
    if reader.is_null() || data.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above: the bytes outlive the reader.
    let data: &'static [u8] = unsafe { core::slice::from_raw_parts(data, len) };
    match ClipReader::new(data) {
        Ok(r) => {
            // SAFETY: the storage is large and aligned enough (asserts above) and writable.
            unsafe {
                reader.cast::<Reader>().write(r);
                set_error(error, ZmidiClipError::None);
            }
            ZmidiStatus::Ok
        }
        Err(e) => {
            // SAFETY: caller contract above.
            unsafe { set_error(error, read_error(e)) };
            ZmidiStatus::Rejected
        }
    }
}

/// Reads the next message of the clip into `event` and sets `has_event`; `has_event` false at the
/// end of the clip. A read error gives [`ZmidiStatus::Rejected`] and its detail in `error`.
///
/// # Safety
/// `reader` was set up by [`zmidi_clip_reader_init`]; `event` and `has_event` are writable; `error`
/// is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_clip_reader_next(
    reader: *mut ZmidiClipReader,
    event: *mut ZmidiClipEvent,
    has_event: *mut bool,
    error: *mut ZmidiClipError,
) -> ZmidiStatus {
    if reader.is_null() || event.is_null() || has_event.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let reader = unsafe { &mut *reader.cast::<Reader>() };
    let (status, detail, found) = match reader.next() {
        None => (ZmidiStatus::Ok, ZmidiClipError::None, None),
        Some(Ok(ev)) => (ZmidiStatus::Ok, ZmidiClipError::None, Some(ev)),
        Some(Err(e)) => (ZmidiStatus::Rejected, read_error(e), None),
    };
    // SAFETY: caller contract above.
    unsafe {
        if let Some(ev) = found {
            let mut words = [0u32; 4];
            let n = ev.ump.write(&mut words).unwrap_or(0);
            event.write(ZmidiClipEvent {
                section: u8::from(ev.section == ClipSection::Sequence),
                ticks: ev.ticks,
                word_count: n as u8,
                words,
            });
        }
        has_event.write(found.is_some());
        set_error(error, detail);
    }
    status
}

/// Writes the Ticks Per Quarter Note of the most recent DCTPQ to `tpq` and gives true; false
/// before the first DCTPQ or for a null pointer.
///
/// # Safety
/// `reader` is null or was set up by [`zmidi_clip_reader_init`]; `tpq` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_clip_reader_ticks_per_quarter_note(
    reader: *mut ZmidiClipReader,
    tpq: *mut u16,
) -> bool {
    if reader.is_null() || tpq.is_null() {
        return false;
    }
    // SAFETY: caller contract above.
    match unsafe { (*reader.cast::<Reader>()).ticks_per_quarter_note() } {
        Some(v) => {
            // SAFETY: caller contract above.
            unsafe { tpq.write(v) };
            true
        }
        None => false,
    }
}

/// Gives the size in bytes of the Rust `ClipReader` (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_clip_reader_size() -> usize {
    size_of::<Reader>()
}

fn profile(p: &ZmidiClipProfileOn) -> Option<ProfileOn> {
    let destination = match p.destination {
        0 => ProfileDestination::Channel(p.channel),
        1 => ProfileDestination::Group,
        2 => ProfileDestination::FunctionBlock,
        _ => return None,
    };
    Some(ProfileOn {
        profile_id: p.profile_id,
        destination,
        channels: p.channels,
    })
}

/// The most Set Profile On messages of one configuration.
pub const ZMIDI_CLIP_MAX_PROFILES: usize = 16;

/// Sets up `writer` into `buf` (`capacity` bytes) and writes the Clip Configuration Header of
/// `config`. `buf` stays valid while `writer` is used.
///
/// # Safety
/// `writer` points to writable storage of a [`ZmidiClipWriter`]; `buf` points to `capacity`
/// writable bytes that stay valid while the writer is used; `config` is valid, with
/// `profile_count` profiles at `profiles`; `error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_clip_writer_init(
    writer: *mut ZmidiClipWriter,
    buf: *mut u8,
    capacity: usize,
    config: *const ZmidiClipConfig,
    error: *mut ZmidiClipError,
) -> ZmidiStatus {
    if writer.is_null() || buf.is_null() || config.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let c = unsafe { &*config };
    let Ok(group) = Group::new(c.group) else {
        return ZmidiStatus::InvalidArgument;
    };
    if c.profile_count > ZMIDI_CLIP_MAX_PROFILES || (c.profiles.is_null() && c.profile_count > 0) {
        return ZmidiStatus::InvalidArgument;
    }
    let mut profiles = [ProfileOn {
        profile_id: [0; 5],
        destination: ProfileDestination::Group,
        channels: 0,
    }; ZMIDI_CLIP_MAX_PROFILES];
    for (i, slot) in profiles.iter_mut().enumerate().take(c.profile_count) {
        // SAFETY: caller contract above.
        let Some(p) = profile(unsafe { &*c.profiles.add(i) }) else {
            return ZmidiStatus::InvalidArgument;
        };
        *slot = p;
    }
    let time_signature = if c.has_time_signature {
        match SetTimeSignature::try_new(group, c.numerator, c.denominator, c.thirty_second_notes) {
            Ok(s) => Some(s),
            Err(_) => return ZmidiStatus::InvalidArgument,
        }
    } else {
        None
    };
    let cfg = ClipConfig {
        ticks_per_quarter_note: c.ticks_per_quarter_note,
        profiles: &profiles[..c.profile_count],
        tempo: c.has_tempo.then(|| SetTempo::new(group, c.tempo_10ns)),
        time_signature,
        group,
    };
    // SAFETY: caller contract above: the buffer outlives the writer.
    let buf: &'static mut [u8] = unsafe { core::slice::from_raw_parts_mut(buf, capacity) };
    match ClipWriter::new(buf, &cfg) {
        Ok(w) => {
            // SAFETY: the storage is large and aligned enough (asserts above) and writable.
            unsafe {
                writer.cast::<Writer>().write(w);
                set_error(error, ZmidiClipError::None);
            }
            ZmidiStatus::Ok
        }
        // SAFETY: caller contract above.
        Err(e) => unsafe { finish_write(Err(e), error) },
    }
}

/// Gives the writer of `writer`, or `None` for a null pointer.
///
/// # Safety
/// `writer` is null or was set up by [`zmidi_clip_writer_init`].
unsafe fn writer_mut<'a>(writer: *mut ZmidiClipWriter) -> Option<&'a mut Writer> {
    // SAFETY: caller contract above.
    unsafe { writer.cast::<Writer>().as_mut() }
}

/// Gives the words of a UMP from the program, or `None` for a null pointer.
///
/// # Safety
/// `words` is null or points to `word_count` words.
unsafe fn words_in<'a>(words: *const u32, word_count: usize) -> Option<&'a [u32]> {
    // SAFETY: caller contract above.
    (!words.is_null()).then(|| unsafe { core::slice::from_raw_parts(words, word_count) })
}

/// Writes one UMP (`words`, `word_count` words) of the Clip Configuration Header.
///
/// # Safety
/// `writer` was set up by [`zmidi_clip_writer_init`]; `words` points to `word_count` words;
/// `error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_clip_writer_config_event(
    writer: *mut ZmidiClipWriter,
    words: *const u32,
    word_count: usize,
    error: *mut ZmidiClipError,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(w), Some(words)) = (unsafe { writer_mut(writer) }, unsafe {
        words_in(words, word_count)
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { finish_write(w.config_event(words), error) }
}

/// Writes Start of Clip at `tick` with an optional Set Tempo (`has_tempo`, `tempo_10ns`), and
/// begins the Clip Sequence Data.
///
/// # Safety
/// `writer` was set up by [`zmidi_clip_writer_init`]; `error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_clip_writer_start(
    writer: *mut ZmidiClipWriter,
    tick: u64,
    has_tempo: bool,
    tempo_group: u8,
    tempo_10ns: u32,
    error: *mut ZmidiClipError,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(w) = (unsafe { writer_mut(writer) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Ok(group) = Group::new(tempo_group) else {
        return ZmidiStatus::InvalidArgument;
    };
    let tempo = has_tempo.then(|| SetTempo::new(group, tempo_10ns));
    // SAFETY: caller contract above.
    unsafe { finish_write(w.start(tick, tempo, None), error) }
}

/// Writes one UMP (`words`, `word_count` words) of the Clip Sequence Data at absolute `tick`.
///
/// # Safety
/// `writer` was set up by [`zmidi_clip_writer_init`]; `words` points to `word_count` words;
/// `error` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_clip_writer_event(
    writer: *mut ZmidiClipWriter,
    tick: u64,
    words: *const u32,
    word_count: usize,
    error: *mut ZmidiClipError,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let (Some(w), Some(words)) = (unsafe { writer_mut(writer) }, unsafe {
        words_in(words, word_count)
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { finish_write(w.event(tick, words), error) }
}

/// Writes End of Clip at `tick` and gives the length of the clip in `len`.
///
/// # Safety
/// `writer` was set up by [`zmidi_clip_writer_init`]; `len` is writable; `error` is null or
/// writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_clip_writer_finish(
    writer: *mut ZmidiClipWriter,
    tick: u64,
    len: *mut usize,
    error: *mut ZmidiClipError,
) -> ZmidiStatus {
    if len.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(w) = (unsafe { writer_mut(writer) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let r = w.finish(tick).map(|n| {
        // SAFETY: caller contract above.
        unsafe { len.write(n) }
    });
    // SAFETY: caller contract above.
    unsafe { finish_write(r, error) }
}

/// Gives the count of bytes written, or 0 for a null pointer.
///
/// # Safety
/// `writer` is null or was set up by [`zmidi_clip_writer_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_clip_writer_len(writer: *mut ZmidiClipWriter) -> usize {
    // SAFETY: caller contract above.
    unsafe { writer_mut(writer) }.map_or(0, |w| w.len())
}

/// Gives the size in bytes of the Rust `ClipWriter` (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_clip_writer_size() -> usize {
    size_of::<Writer>()
}
