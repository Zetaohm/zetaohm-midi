//! The C ABI of the zmidi-io converters (DEC-019): a MIDI 1.0 byte stream to UMP
//! ([`BytesToUmp`], REQ-FFI-IO-001, -002) and UMP to a MIDI 1.0 byte stream ([`UmpToBytes`],
//! REQ-FFI-IO-003, -004). The program gives the storage of each converter (REQ-FFI-IO-005).

use core::mem::{align_of, size_of};

use zmidi_io::{ByteStreamError, BytesToUmp, UmpToBytes};
use zmidi_ump::{Group, StreamProtocol};

use crate::status::ZmidiStatus;

/// Storage words of a [`ZmidiBytesToUmp`] on a 64-bit target.
#[cfg(target_pointer_width = "64")]
pub const ZMIDI_BYTES_TO_UMP_WORDS: usize = 24;
/// Storage words of a [`ZmidiBytesToUmp`] on a 32-bit or 16-bit target.
#[cfg(not(target_pointer_width = "64"))]
pub const ZMIDI_BYTES_TO_UMP_WORDS: usize = 22;
/// Storage words of a [`ZmidiUmpToBytes`].
pub const ZMIDI_UMP_TO_BYTES_WORDS: usize = 1;
/// The most words that one byte completes: two UMPs of at most two words each (Message Types
/// 0x1 to 0x4).
pub const ZMIDI_BYTES_TO_UMP_MAX_WORDS: usize = 4;

/// Storage of one byte stream reader. The program gives it; [`zmidi_bytes_to_ump_init`] sets it up.
#[repr(C)]
pub struct ZmidiBytesToUmp {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_BYTES_TO_UMP_WORDS],
}

/// Storage of one byte stream writer. The program gives it; [`zmidi_ump_to_bytes_init`] sets it up.
#[repr(C)]
pub struct ZmidiUmpToBytes {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_UMP_TO_BYTES_WORDS],
}

const _: () = assert!(size_of::<BytesToUmp>() <= size_of::<ZmidiBytesToUmp>());
const _: () = assert!(align_of::<BytesToUmp>() <= align_of::<ZmidiBytesToUmp>());
const _: () = assert!(size_of::<UmpToBytes>() <= size_of::<ZmidiUmpToBytes>());
const _: () = assert!(align_of::<UmpToBytes>() <= align_of::<ZmidiUmpToBytes>());

/// Protocol of the UMPs that a reader gives (M2-104 Stream Configuration values).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiProtocol {
    /// MIDI 1.0 Protocol: Message Type 0x2.
    Midi1 = 1,
    /// MIDI 2.0 Protocol: Message Type 0x4 (M2-104 Appendix D.3).
    Midi2 = 2,
}

/// Sets up `reader` for Group `group` (0 to 15) and protocol `protocol` (1 or 2, see
/// [`ZmidiProtocol`]).
///
/// # Safety
/// `reader` is null or points to writable storage of a [`ZmidiBytesToUmp`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_bytes_to_ump_init(
    reader: *mut ZmidiBytesToUmp,
    group: u8,
    protocol: u32,
) -> ZmidiStatus {
    let Ok(group) = Group::new(group) else {
        return ZmidiStatus::InvalidArgument;
    };
    let protocol = match protocol {
        1 => StreamProtocol::Midi1,
        2 => StreamProtocol::Midi2,
        _ => return ZmidiStatus::InvalidArgument,
    };
    if reader.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe {
        reader
            .cast::<BytesToUmp>()
            .write(BytesToUmp::new(group, protocol))
    };
    ZmidiStatus::Ok
}

/// Reads `byte` and writes the words of each UMP that it completes into `out` (`capacity`
/// words), and their count into `count`. A `capacity` below [`ZMIDI_BYTES_TO_UMP_MAX_WORDS`] gives
/// [`ZmidiStatus::BufferTooSmall`] before the byte is read.
///
/// # Safety
/// `reader` was set up by [`zmidi_bytes_to_ump_init`]; `out` points to `capacity` writable words;
/// `count` is writable. Null pointers give [`ZmidiStatus::InvalidArgument`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_bytes_to_ump_push(
    reader: *mut ZmidiBytesToUmp,
    byte: u8,
    out: *mut u32,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if reader.is_null() || out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    if capacity < ZMIDI_BYTES_TO_UMP_MAX_WORDS {
        return ZmidiStatus::BufferTooSmall;
    }
    // SAFETY: caller contract above.
    let (reader, out) = unsafe {
        (
            &mut *reader.cast::<BytesToUmp>(),
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let mut n = 0;
    reader.feed(byte, |words| {
        // At most ZMIDI_BYTES_TO_UMP_MAX_WORDS words in all, so this always fits.
        if let Some(dst) = out.get_mut(n..n + words.len()) {
            dst.copy_from_slice(words);
            n += words.len();
        }
    });
    // SAFETY: caller contract above.
    unsafe { count.write(n) };
    ZmidiStatus::Ok
}

/// Reads the bytes of `bytes` (`len` bytes) in order and writes the words of each UMP that they
/// complete into `out` (`capacity` words) (OPT-035: one call for many bytes). It stops before a
/// byte when `out` has less than [`ZMIDI_BYTES_TO_UMP_MAX_WORDS`] free words, so no UMP is cut:
/// `read` gives the number of bytes read (call again with the rest) and `count` the number of
/// words. A `capacity` below [`ZMIDI_BYTES_TO_UMP_MAX_WORDS`] gives [`ZmidiStatus::BufferTooSmall`]
/// before a byte is read. The result is the same as one [`zmidi_bytes_to_ump_push`] for each
/// byte.
///
/// # Safety
/// `reader` was set up by [`zmidi_bytes_to_ump_init`]; `bytes` points to `len` bytes (or is null
/// when `len` is 0); `out` points to `capacity` writable words; `read` and `count` are writable.
/// Null pointers give [`ZmidiStatus::InvalidArgument`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_bytes_to_ump_feed(
    reader: *mut ZmidiBytesToUmp,
    bytes: *const u8,
    len: usize,
    out: *mut u32,
    capacity: usize,
    read: *mut usize,
    count: *mut usize,
) -> ZmidiStatus {
    if reader.is_null()
        || (bytes.is_null() && len > 0)
        || out.is_null()
        || read.is_null()
        || count.is_null()
    {
        return ZmidiStatus::InvalidArgument;
    }
    if capacity < ZMIDI_BYTES_TO_UMP_MAX_WORDS {
        return ZmidiStatus::BufferTooSmall;
    }
    // SAFETY: caller contract above.
    let (reader, out) = unsafe {
        (
            &mut *reader.cast::<BytesToUmp>(),
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let input: &[u8] = if len == 0 {
        &[]
    } else {
        // SAFETY: caller contract above.
        unsafe { core::slice::from_raw_parts(bytes, len) }
    };
    let mut n = 0;
    let mut taken = 0;
    for &byte in input {
        if capacity - n < ZMIDI_BYTES_TO_UMP_MAX_WORDS {
            break;
        }
        reader.feed(byte, |words| {
            // At most ZMIDI_BYTES_TO_UMP_MAX_WORDS words for each byte, so this always fits.
            if let Some(dst) = out.get_mut(n..n + words.len()) {
                dst.copy_from_slice(words);
                n += words.len();
            }
        });
        taken += 1;
    }
    // SAFETY: caller contract above.
    unsafe {
        read.write(taken);
        count.write(n);
    }
    ZmidiStatus::Ok
}

/// Gives the size in bytes of the Rust `BytesToUmp` (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_bytes_to_ump_size() -> usize {
    size_of::<BytesToUmp>()
}

/// Sets up `writer`, with MIDI 1.0 running status when `running_status` is true.
///
/// # Safety
/// `writer` is null or points to writable storage of a [`ZmidiUmpToBytes`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_to_bytes_init(
    writer: *mut ZmidiUmpToBytes,
    running_status: bool,
) -> ZmidiStatus {
    if writer.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe {
        writer
            .cast::<UmpToBytes>()
            .write(UmpToBytes::new(running_status))
    };
    ZmidiStatus::Ok
}

/// Writes the MIDI 1.0 bytes of one UMP (`words`, `word_count` words) into `out` (`capacity`
/// bytes), and their count into `count`. A buffer that is too small gives
/// [`ZmidiStatus::BufferTooSmall`] with no state change.
///
/// # Safety
/// `writer` was set up by [`zmidi_ump_to_bytes_init`]; `words` points to `word_count` words; `out`
/// points to `capacity` writable bytes; `count` is writable. Null pointers give
/// [`ZmidiStatus::InvalidArgument`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_to_bytes_write(
    writer: *mut ZmidiUmpToBytes,
    words: *const u32,
    word_count: usize,
    out: *mut u8,
    capacity: usize,
    count: *mut usize,
) -> ZmidiStatus {
    if writer.is_null() || words.is_null() || out.is_null() || count.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (writer, words, out) = unsafe {
        (
            &mut *writer.cast::<UmpToBytes>(),
            core::slice::from_raw_parts(words, word_count),
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    match writer.write(words, out) {
        Ok(n) => {
            // SAFETY: caller contract above.
            unsafe { count.write(n) };
            ZmidiStatus::Ok
        }
        Err(ByteStreamError::BufferTooSmall) => ZmidiStatus::BufferTooSmall,
        Err(_) => ZmidiStatus::Rejected,
    }
}

/// The next Channel Voice or Mode message writes its status byte.
///
/// # Safety
/// `writer` is null or was set up by [`zmidi_ump_to_bytes_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ump_to_bytes_refresh_status(
    writer: *mut ZmidiUmpToBytes,
) -> ZmidiStatus {
    if writer.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe { (*writer.cast::<UmpToBytes>()).refresh_status() };
    ZmidiStatus::Ok
}

/// Gives the size in bytes of the Rust `UmpToBytes` (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_ump_to_bytes_size() -> usize {
    size_of::<UmpToBytes>()
}
