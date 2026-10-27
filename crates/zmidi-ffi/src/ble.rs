//! The C ABI of zmidi-ble (DEC-019): BLE-MIDI v1.0 packets to UMP ([`PacketReader`],
//! REQ-FFI-BLE-001, -002) and UMP to packets ([`PacketWriter`], REQ-FFI-BLE-003, -004), for C BLE
//! stacks. The program gives the storage of each object.

use core::mem::{align_of, size_of};

use zmidi_ble::zmidi_ump::{Group, StreamProtocol};
use zmidi_ble::{PacketReader, PacketWriter, TimedUmp, WriteError};

use crate::status::ZmidiStatus;

/// Storage words of a [`ZmidiBleReader`] on a 64-bit target.
#[cfg(target_pointer_width = "64")]
pub const ZMIDI_BLE_READER_WORDS: usize = 25;
/// Storage words of a [`ZmidiBleReader`] on a 32-bit or 16-bit target.
#[cfg(not(target_pointer_width = "64"))]
pub const ZMIDI_BLE_READER_WORDS: usize = 23;
/// Storage words of a [`ZmidiBleWriter`] on a 64-bit target.
#[cfg(target_pointer_width = "64")]
pub const ZMIDI_BLE_WRITER_WORDS: usize = 11;
/// Storage words of a [`ZmidiBleWriter`] on a 32-bit or 16-bit target.
#[cfg(not(target_pointer_width = "64"))]
pub const ZMIDI_BLE_WRITER_WORDS: usize = 9;

/// Storage of one packet reader. The program gives it; [`zmidi_ble_reader_init`] sets it up.
#[repr(C)]
pub struct ZmidiBleReader {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_BLE_READER_WORDS],
}

/// Storage of one packet writer. The program gives it; [`zmidi_ble_writer_init`] sets it up.
#[repr(C)]
pub struct ZmidiBleWriter {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_BLE_WRITER_WORDS],
}

const _: () = assert!(size_of::<PacketReader>() <= size_of::<ZmidiBleReader>());
const _: () = assert!(align_of::<PacketReader>() <= align_of::<ZmidiBleReader>());
const _: () = assert!(size_of::<PacketWriter>() <= size_of::<ZmidiBleWriter>());
const _: () = assert!(align_of::<PacketWriter>() <= align_of::<ZmidiBleWriter>());

/// One UMP of a read packet with the 13-bit timestamp of its message (sender clock).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiBleUmp {
    /// Timestamp in milliseconds, 0 to 8191.
    pub timestamp_ms: u16,
    /// Words of the UMP, 1 to 4.
    pub word_count: u8,
    /// The words; words after `word_count` are 0.
    pub words: [u32; 4],
}

/// One UMP to write, with the time of its message in milliseconds (the clock of the program).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiBleTimedUmp {
    /// Time in milliseconds.
    pub millis: u64,
    /// Words of the UMP, 1 to 4.
    pub word_count: u8,
    /// The words.
    pub words: [u32; 4],
}

fn group_protocol(group: u8, protocol: u32) -> Option<(Group, StreamProtocol)> {
    let g = Group::new(group).ok()?;
    let p = match protocol {
        1 => StreamProtocol::Midi1,
        2 => StreamProtocol::Midi2,
        _ => return None,
    };
    Some((g, p))
}

/// Sets up `reader` for Group `group` (0 to 15) and protocol `protocol` (1 or 2).
///
/// # Safety
/// `reader` is null or points to writable storage of a [`ZmidiBleReader`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ble_reader_init(
    reader: *mut ZmidiBleReader,
    group: u8,
    protocol: u32,
) -> ZmidiStatus {
    let Some((g, p)) = group_protocol(group, protocol) else {
        return ZmidiStatus::InvalidArgument;
    };
    if reader.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe { reader.cast::<PacketReader>().write(PacketReader::new(g, p)) };
    ZmidiStatus::Ok
}

/// Reads one packet (`len` bytes) and writes each UMP of it into `out` (`capacity` records), the
/// record count into `count` and the count of dropped parts into `dropped`. A `capacity` below
/// `len` gives [`ZmidiStatus::BufferTooSmall`] before the packet is read.
///
/// # Safety
/// `reader` was set up by [`zmidi_ble_reader_init`]; `packet` points to `len` bytes; `out` points to
/// `capacity` writable records; `count` and `dropped` are writable. Null pointers give
/// [`ZmidiStatus::InvalidArgument`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ble_read(
    reader: *mut ZmidiBleReader,
    packet: *const u8,
    len: usize,
    out: *mut ZmidiBleUmp,
    capacity: usize,
    count: *mut usize,
    dropped: *mut usize,
) -> ZmidiStatus {
    if reader.is_null() || packet.is_null() || out.is_null() || count.is_null() || dropped.is_null()
    {
        return ZmidiStatus::InvalidArgument;
    }
    // Each UMP takes at least one byte of the packet, so `len` records always hold all of them.
    if capacity < len {
        return ZmidiStatus::BufferTooSmall;
    }
    // SAFETY: caller contract above.
    let (reader, packet, out) = unsafe {
        (
            &mut *reader.cast::<PacketReader>(),
            core::slice::from_raw_parts(packet, len),
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let (mut n, mut bad) = (0usize, 0usize);
    reader.read(packet, |r| match r {
        Ok(rx) => {
            let mut words = [0u32; 4];
            let wc = rx.ump.write(&mut words).unwrap_or(0);
            if let Some(slot) = out.get_mut(n) {
                *slot = ZmidiBleUmp {
                    timestamp_ms: rx.timestamp.millis(),
                    word_count: wc as u8,
                    words,
                };
                n += 1;
            }
        }
        Err(_) => bad += 1,
    });
    // SAFETY: caller contract above.
    unsafe {
        count.write(n);
        dropped.write(bad);
    }
    ZmidiStatus::Ok
}

/// Drops an open SysEx message and the last timestamp of `reader`.
///
/// # Safety
/// `reader` is null or was set up by [`zmidi_ble_reader_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ble_reader_reset(reader: *mut ZmidiBleReader) -> ZmidiStatus {
    if reader.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe { (*reader.cast::<PacketReader>()).reset() };
    ZmidiStatus::Ok
}

/// Gives the size in bytes of the Rust `PacketReader` (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_ble_reader_size() -> usize {
    size_of::<PacketReader>()
}

/// Sets up `writer` for the ATT_MTU `att_mtu` (23 or more).
///
/// # Safety
/// `writer` is null or points to writable storage of a [`ZmidiBleWriter`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ble_writer_init(
    writer: *mut ZmidiBleWriter,
    att_mtu: u16,
) -> ZmidiStatus {
    if writer.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    let Ok(w) = PacketWriter::new(att_mtu) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe { writer.cast::<PacketWriter>().write(w) };
    ZmidiStatus::Ok
}

/// Gives the writer of `writer`, or `None` for a null pointer.
///
/// # Safety
/// `writer` is null or was set up by [`zmidi_ble_writer_init`].
unsafe fn writer_mut<'a>(writer: *mut ZmidiBleWriter) -> Option<&'a mut PacketWriter> {
    // SAFETY: caller contract above.
    unsafe { writer.cast::<PacketWriter>().as_mut() }
}

/// Changes the ATT_MTU of `writer` (23 or more).
///
/// # Safety
/// `writer` is null or was set up by [`zmidi_ble_writer_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ble_writer_set_att_mtu(
    writer: *mut ZmidiBleWriter,
    att_mtu: u16,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match unsafe { writer_mut(writer) }.map(|w| w.set_att_mtu(att_mtu)) {
        Some(Ok(())) => ZmidiStatus::Ok,
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// Turns running status of `writer` on or off.
///
/// # Safety
/// `writer` is null or was set up by [`zmidi_ble_writer_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ble_writer_set_running_status(
    writer: *mut ZmidiBleWriter,
    on: bool,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(w) = (unsafe { writer_mut(writer) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    w.set_running_status(on);
    ZmidiStatus::Ok
}

/// Gives the packet length of `writer` (ATT_MTU - 3), or 0 for a null pointer.
///
/// # Safety
/// `writer` is null or was set up by [`zmidi_ble_writer_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ble_writer_packet_len(writer: *mut ZmidiBleWriter) -> usize {
    // SAFETY: caller contract above.
    unsafe { writer_mut(writer) }.map_or(0, |w| w.packet_len())
}

/// Gives true when `writer` holds bytes for the next packet; false for a null pointer.
///
/// # Safety
/// `writer` is null or was set up by [`zmidi_ble_writer_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ble_writer_has_held(writer: *mut ZmidiBleWriter) -> bool {
    // SAFETY: caller contract above.
    unsafe { writer_mut(writer) }.is_some_and(|w| w.has_held())
}

/// Drops the held bytes and the state of `writer`.
///
/// # Safety
/// `writer` is null or was set up by [`zmidi_ble_writer_init`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ble_writer_reset(writer: *mut ZmidiBleWriter) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(w) = (unsafe { writer_mut(writer) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    w.reset();
    ZmidiStatus::Ok
}

/// Gives the size in bytes of the Rust `PacketWriter` (DEC-019 rule 6).
#[no_mangle]
pub extern "C" fn zmidi_ble_writer_size() -> usize {
    size_of::<PacketWriter>()
}

/// Writes one packet into `out` (`capacity` bytes) from the held bytes and then from `umps`
/// (`ump_count` timed UMPs), and gives its length in `len` (0: nothing to send) and the count of
/// UMPs taken from `umps` in `taken`.
///
/// # Safety
/// `writer` was set up by [`zmidi_ble_writer_init`]; `umps` points to `ump_count` timed UMPs (or is
/// null when `ump_count` is 0); `out` points to `capacity` writable bytes; `len` and `taken` are
/// writable. Null pointers give [`ZmidiStatus::InvalidArgument`].
#[no_mangle]
pub unsafe extern "C" fn zmidi_ble_write_packet(
    writer: *mut ZmidiBleWriter,
    umps: *const ZmidiBleTimedUmp,
    ump_count: usize,
    out: *mut u8,
    capacity: usize,
    len: *mut usize,
    taken: *mut usize,
) -> ZmidiStatus {
    if (umps.is_null() && ump_count > 0) || out.is_null() || len.is_null() || taken.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(w) = (unsafe { writer_mut(writer) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let (umps, out) = unsafe {
        (
            if ump_count == 0 {
                &[][..]
            } else {
                core::slice::from_raw_parts(umps, ump_count)
            },
            core::slice::from_raw_parts_mut(out, capacity),
        )
    };
    let mut iter = umps
        .iter()
        .map(|u| TimedUmp {
            millis: u.millis,
            words: &u.words[..usize::from(u.word_count).min(4)],
        })
        .peekable();
    match w.write_packet(&mut iter, out) {
        Ok(n) => {
            let left = iter.count();
            // SAFETY: caller contract above.
            unsafe {
                len.write(n);
                taken.write(ump_count - left);
            }
            ZmidiStatus::Ok
        }
        Err(WriteError::BufferTooSmall) => ZmidiStatus::BufferTooSmall,
        Err(_) => ZmidiStatus::InvalidArgument,
    }
}
