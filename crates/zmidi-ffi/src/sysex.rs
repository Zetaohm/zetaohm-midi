//! The C ABI of zmidi-sysex (DEC-019, DEC-022): the Universal System Exclusive reader and the raw
//! writers (feature `sysex`, REQ-FFI-SX-001), Device Inquiry (`inquiry`, REQ-FFI-SX-003), the
//! General MIDI System messages (`gm`, REQ-FFI-SX-004), Device Control (`device-control`,
//! REQ-FFI-SX-005), MIDI Tuning (`tuning`, REQ-FFI-SX-006), MIDI Time Code (`mtc`,
//! REQ-FFI-SX-007) and the MIDI Machine Control transport tier (`mmc-transport`, REQ-FFI-SX-008).
//!
//! - Every reader of a part takes one whole message, with or without F0H and F7H. It gives
//!   [`ZmidiStatus::NoPending`] for a message of another part and [`ZmidiStatus::InvalidArgument`]
//!   for a message of its part that is not valid (REQ-FFI-SX-002).
//! - Every writer takes `out`, `capacity` and `length`: it writes the whole message and gives its
//!   length; a small buffer gives [`ZmidiStatus::BufferTooSmall`] with the needed length in
//!   `*length`, and `out` may then be null when `capacity` is 0. The content of `out` after an
//!   error is not defined.
//! - A message part (a data string, an MMC string) is an offset and a length into the bytes that
//!   the program gave to the call, not a pointer. A data pointer may be null when its length is 0.
//! - The program gives the storage of the MTC generator and reader (REQ-FFI-SX-009).

#[cfg(feature = "mtc")]
use core::mem::{align_of, size_of};

#[cfg(feature = "device-control")]
use zmidi_sysex::{DeviceControl, DeviceControlValue};
#[cfg(feature = "mtc")]
use zmidi_sysex::{
    Direction, FullMessage, MtcEvent, MtcGenerator, MtcReader, MtcTime, QuarterFrame, TimeCodeType,
    UserBits,
};
#[cfg(feature = "mmc-transport")]
use zmidi_sysex::{
    FinalByte, InfoField, InfoFields, MmcCommand, MmcMessage, ShortTimeCode, StandardTimeCode,
    TimeCodeStatus, MAX_STRING,
};
#[cfg(feature = "tuning")]
use zmidi_sysex::{FrequencyWord, ScaleOffsets, TuningMessage};
#[cfg(feature = "gm")]
use zmidi_sysex::{GeneralMidi, GmCommand};
#[cfg(feature = "inquiry")]
use zmidi_sysex::{Identity, InquiryMessage};
use zmidi_sysex::{Sysex, SysexError, SysexId, Universal};
use zmidi_ump::ManufacturerId;

use crate::status::ZmidiStatus;

/// The bytes of the program: `None` for a null pointer with a length above 0.
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes.
unsafe fn input<'a>(bytes: *const u8, length: usize) -> Option<&'a [u8]> {
    if bytes.is_null() {
        return if length == 0 { Some(&[]) } else { None };
    }
    // SAFETY: caller contract above.
    Some(unsafe { core::slice::from_raw_parts(bytes, length) })
}

/// The output buffer of the program: `None` for a null pointer with a capacity above 0.
///
/// # Safety
/// `out` is null or points to `capacity` writable bytes.
unsafe fn output<'a>(out: *mut u8, capacity: usize) -> Option<&'a mut [u8]> {
    if out.is_null() {
        return if capacity == 0 { Some(&mut []) } else { None };
    }
    // SAFETY: caller contract above.
    Some(unsafe { core::slice::from_raw_parts_mut(out, capacity) })
}

/// Gives the result of a writer: the length, or the needed length of a small buffer
/// (REQ-FFI-SX-002).
///
/// # Safety
/// `length` is writable.
unsafe fn written(result: Result<usize, SysexError>, length: *mut usize) -> ZmidiStatus {
    match result {
        Ok(n) => {
            // SAFETY: caller contract above.
            unsafe { *length = n };
            ZmidiStatus::Ok
        }
        Err(SysexError::BufferTooSmall { needed }) => {
            // SAFETY: caller contract above.
            unsafe { *length = needed };
            ZmidiStatus::BufferTooSmall
        }
        Err(_) => ZmidiStatus::InvalidArgument,
    }
}

/// Runs a writer into the buffer of the program (REQ-FFI-SX-002).
///
/// # Safety
/// `out` is null or points to `capacity` writable bytes; `length` is null or writable.
unsafe fn write_into(
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
    write: impl FnOnce(&mut [u8]) -> Result<usize, SysexError>,
) -> ZmidiStatus {
    if length.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(out) = (unsafe { output(out, capacity) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { written(write(out), length) }
}

/// The Universal header of one whole message of the program, for a part reader
/// (REQ-FFI-SX-002).
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes.
#[cfg(any(
    feature = "inquiry",
    feature = "gm",
    feature = "device-control",
    feature = "tuning",
    feature = "mtc"
))]
unsafe fn universal<'a>(bytes: *const u8, length: usize) -> Result<Universal<'a>, ZmidiStatus> {
    // SAFETY: caller contract above.
    let b = unsafe { input(bytes, length) }.ok_or(ZmidiStatus::InvalidArgument)?;
    let s = Sysex::parse(b).map_err(|_| ZmidiStatus::InvalidArgument)?;
    match Universal::parse(&s) {
        Ok(Some(u)) => Ok(u),
        Ok(None) => Err(ZmidiStatus::NoPending),
        Err(_) => Err(ZmidiStatus::InvalidArgument),
    }
}

/// The message of a part reader, or the status of REQ-FFI-SX-002.
#[cfg(any(
    feature = "inquiry",
    feature = "gm",
    feature = "device-control",
    feature = "tuning",
    feature = "mtc"
))]
fn part<T>(result: Result<Option<T>, SysexError>) -> Result<T, ZmidiStatus> {
    match result {
        Ok(Some(m)) => Ok(m),
        Ok(None) => Err(ZmidiStatus::NoPending),
        Err(_) => Err(ZmidiStatus::InvalidArgument),
    }
}

/// The offset of `part` in `base`, which holds it.
fn offset(base: &[u8], part: &[u8]) -> u32 {
    let d = (part.as_ptr() as usize).wrapping_sub(base.as_ptr() as usize);
    d.min(base.len()) as u32
}

/// The ID of the three bytes of the C structs: `[id, 0, 0]` or `[00H, b2, b3]`.
fn manufacturer_id(bytes: [u8; 3]) -> Option<ManufacturerId> {
    let b: &[u8] = if bytes[0] == 0 { &bytes } else { &bytes[..1] };
    ManufacturerId::from_midi1(b).ok()
}

/// The class of a System Exclusive message (M1 Table VII; REQ-FFI-SX-001).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiSysexClass {
    /// A manufacturer ID.
    Manufacturer = 0,
    /// 7DH, Non-Commercial.
    NonCommercial = 1,
    /// 7EH, Universal Non-Real Time.
    NonRealTime = 2,
    /// 7FH, Universal Real Time.
    RealTime = 3,
}

/// One System Exclusive message as flat values (REQ-FFI-SX-001).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiSysex {
    /// A [`ZmidiSysexClass`] value.
    pub kind: u32,
    /// The Manufacturer ID of the class Manufacturer: `[id, 0, 0]` or `[00H, b2, b3]`.
    pub manufacturer: [u8; 3],
    /// The Device ID of a Universal message.
    pub device_id: u8,
    /// Sub-ID#1 of a Universal message.
    pub sub_id1: u8,
    /// Sub-ID#2 when `has_sub_id2`.
    pub sub_id2: u8,
    /// True when the family of Sub-ID#1 has a Sub-ID#2.
    pub has_sub_id2: bool,
    /// The offset in the input of the data after the header (after the ID for a manufacturer or
    /// Non-Commercial message), without F7H.
    pub data_offset: u32,
    /// The length of the data.
    pub data_length: u32,
}

/// Reads one whole System Exclusive message, with or without F0H and F7H (REQ-FFI-SX-001). A
/// message that is not valid gives [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `out` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_sysex_parse(
    bytes: *const u8,
    length: usize,
    out: *mut ZmidiSysex,
) -> ZmidiStatus {
    if out.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(b) = (unsafe { input(bytes, length) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Ok(s) = Sysex::parse(b) else {
        return ZmidiStatus::InvalidArgument;
    };
    let mut r = ZmidiSysex {
        kind: ZmidiSysexClass::NonCommercial as u32,
        manufacturer: [0; 3],
        device_id: 0,
        sub_id1: 0,
        sub_id2: 0,
        has_sub_id2: false,
        data_offset: offset(b, s.data),
        data_length: s.data.len() as u32,
    };
    match s.id {
        SysexId::Manufacturer(id) => {
            r.kind = ZmidiSysexClass::Manufacturer as u32;
            r.manufacturer = id.to_midi1();
        }
        SysexId::NonCommercial => {}
        SysexId::NonRealTime | SysexId::RealTime => {
            let Ok(Some(u)) = Universal::parse(&s) else {
                return ZmidiStatus::InvalidArgument;
            };
            r.kind = if u.real_time {
                ZmidiSysexClass::RealTime
            } else {
                ZmidiSysexClass::NonRealTime
            } as u32;
            r.device_id = u.device_id;
            r.sub_id1 = u.sub_id1;
            r.sub_id2 = u.sub_id2.unwrap_or(0);
            r.has_sub_id2 = u.sub_id2.is_some();
            r.data_offset = offset(b, u.data);
            r.data_length = u.data.len() as u32;
        }
    }
    // SAFETY: caller contract above.
    unsafe { *out = r };
    ZmidiStatus::Ok
}

/// Gives in `*result` whether a received Device ID addresses the device: 7FH or `own`
/// (REQ-FFI-SX-001).
///
/// # Safety
/// `result` is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_sysex_addressed(
    device_id: u8,
    own: u8,
    result: *mut bool,
) -> ZmidiStatus {
    if result.is_null() || device_id > 0x7F || own > 0x7F {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe { *result = zmidi_sysex::addressed(device_id, own) };
    ZmidiStatus::Ok
}

/// Writes F0H, the Manufacturer ID (`[id, 0, 0]` or `[00H, b2, b3]`), the data and F7H
/// (REQ-FFI-SX-001). The reserved ID 00H 00H 00H, an ID that is not valid and the Universal IDs
/// 7EH and 7FH give [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `manufacturer` is null or points to 3 readable bytes; `data` is null or points to
/// `data_length` readable bytes; `out` is null or points to `capacity` writable bytes; `length`
/// is null or writable.
#[no_mangle]
pub unsafe extern "C" fn zmidi_sysex_write_manufacturer(
    manufacturer: *const u8,
    data: *const u8,
    data_length: usize,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    if manufacturer.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (m, data) = unsafe { (*manufacturer.cast::<[u8; 3]>(), input(data, data_length)) };
    let (Some(id), Some(data)) = (manufacturer_id(m), data) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            zmidi_sysex::write_manufacturer(id, data, o)
        })
    }
}

/// Writes a Universal message: F0H, 7EH or 7FH, the Device ID, Sub-ID#1, Sub-ID#2 when
/// `has_sub_id2`, the data and F7H (REQ-FFI-SX-001). A Sub-ID#2 that does not match the family
/// of Sub-ID#1 gives [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `data` is null or points to `data_length` readable bytes; `out` is null or points to
/// `capacity` writable bytes; `length` is null or writable.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_sysex_write_universal(
    real_time: bool,
    device_id: u8,
    sub_id1: u8,
    has_sub_id2: bool,
    sub_id2: u8,
    data: *const u8,
    data_length: usize,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(data) = (unsafe { input(data, data_length) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let u = Universal {
        real_time,
        device_id,
        sub_id1,
        sub_id2: has_sub_id2.then_some(sub_id2),
        data,
    };
    // SAFETY: caller contract above.
    unsafe { write_into(out, capacity, length, |o| u.write(o)) }
}

/// The identity of a device (M1 General Information; REQ-FFI-SX-003).
#[cfg(feature = "inquiry")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiIdentity {
    /// The Manufacturer ID: `[id, 0, 0]` or `[00H, b2, b3]`.
    pub manufacturer: [u8; 3],
    /// The device family code, 14 bits.
    pub family: u16,
    /// The device family member code, 14 bits.
    pub member: u16,
    /// The software revision level, four 7-bit bytes.
    pub revision: [u8; 4],
}

#[cfg(feature = "inquiry")]
impl ZmidiIdentity {
    fn of(i: Identity) -> Self {
        Self {
            manufacturer: i.manufacturer.to_midi1(),
            family: i.family,
            member: i.member,
            revision: i.revision,
        }
    }

    fn identity(&self) -> Option<Identity> {
        Identity::new(
            manufacturer_id(self.manufacturer)?,
            self.family,
            self.member,
            self.revision,
        )
        .ok()
    }
}

/// Reads an Identity Request or an Identity Reply (REQ-FFI-SX-003). `*is_reply` tells which;
/// `*identity` is the identity of a reply, and zeroes for a request.
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `is_reply`, `device_id` and `identity`
/// are null or writable.
#[cfg(feature = "inquiry")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_inquiry_read(
    bytes: *const u8,
    length: usize,
    is_reply: *mut bool,
    device_id: *mut u8,
    identity: *mut ZmidiIdentity,
) -> ZmidiStatus {
    if is_reply.is_null() || device_id.is_null() || identity.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let m = match unsafe { universal(bytes, length) }
        .and_then(|u| part(InquiryMessage::from_universal(&u)))
    {
        Ok(m) => m,
        Err(s) => return s,
    };
    let (reply, id, ident) = match m {
        InquiryMessage::Request { device_id } => (
            false,
            device_id,
            ZmidiIdentity {
                manufacturer: [0; 3],
                family: 0,
                member: 0,
                revision: [0; 4],
            },
        ),
        InquiryMessage::Reply {
            device_id,
            identity,
        } => (true, device_id, ZmidiIdentity::of(identity)),
    };
    // SAFETY: caller contract above.
    unsafe {
        *is_reply = reply;
        *device_id = id;
        *identity = ident;
    }
    ZmidiStatus::Ok
}

/// Writes an Identity Request to `device_id` (7FH: all devices; REQ-FFI-SX-003).
///
/// # Safety
/// `out` is null or points to `capacity` writable bytes; `length` is null or writable.
#[cfg(feature = "inquiry")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_inquiry_request_write(
    device_id: u8,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            InquiryMessage::Request { device_id }.write(o)
        })
    }
}

/// Writes an Identity Reply from `device_id` with `identity` (REQ-FFI-SX-003).
///
/// # Safety
/// `identity` is null or readable; `out` is null or points to `capacity` writable bytes;
/// `length` is null or writable.
#[cfg(feature = "inquiry")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_inquiry_reply_write(
    device_id: u8,
    identity: *const ZmidiIdentity,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    if identity.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(identity) = (unsafe { *identity }).identity() else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            InquiryMessage::Reply {
                device_id,
                identity,
            }
            .write(o)
        })
    }
}

/// Answers an Identity Request for `own_device_id` or for all devices with the Identity Reply
/// (REQ-FFI-SX-003). Another message, or a request for another device, gives
/// [`ZmidiStatus::NoPending`].
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `identity` is null or readable; `out`
/// is null or points to `capacity` writable bytes; `out_length` is null or writable.
#[cfg(feature = "inquiry")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_inquiry_respond(
    bytes: *const u8,
    length: usize,
    own_device_id: u8,
    identity: *const ZmidiIdentity,
    out: *mut u8,
    capacity: usize,
    out_length: *mut usize,
) -> ZmidiStatus {
    if identity.is_null() || out_length.is_null() || own_device_id > 0x7F {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (Some(identity), Some(o)) = (unsafe { (*identity).identity() }, unsafe {
        output(out, capacity)
    }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    let m = match unsafe { universal(bytes, length) }
        .and_then(|u| part(InquiryMessage::from_universal(&u)))
    {
        Ok(m) => m,
        Err(s) => return s,
    };
    match zmidi_sysex::respond(&m, own_device_id, &identity, o) {
        Ok(None) => ZmidiStatus::NoPending,
        // SAFETY: caller contract above.
        r => unsafe { written(r.map(|n| n.unwrap_or(0)), out_length) },
    }
}

/// Reads a General MIDI System message (REQ-FFI-SX-004): `*command` is 1 (GM1 System On), 2 (GM
/// System Off) or 3 (GM2 System On).
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `device_id` and `command` are null or
/// writable.
#[cfg(feature = "gm")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_gm_system_read(
    bytes: *const u8,
    length: usize,
    device_id: *mut u8,
    command: *mut u8,
) -> ZmidiStatus {
    if device_id.is_null() || command.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    match unsafe { universal(bytes, length) }.and_then(|u| part(GeneralMidi::from_universal(&u))) {
        Ok(m) => {
            // SAFETY: caller contract above.
            unsafe {
                *device_id = m.device_id;
                *command = m.command.into_bits();
            }
            ZmidiStatus::Ok
        }
        Err(s) => s,
    }
}

/// Writes a General MIDI System message: `command` 1, 2 or 3 as [`zmidi_gm_system_read`] gives
/// it (REQ-FFI-SX-004).
///
/// # Safety
/// `out` is null or points to `capacity` writable bytes; `length` is null or writable.
#[cfg(feature = "gm")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_gm_system_write(
    device_id: u8,
    command: u8,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    let Some(command) = GmCommand::from_bits(command) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            GeneralMidi { device_id, command }.write(o)
        })
    }
}

/// The kind of a Device Control message (REQ-FFI-SX-005).
#[cfg(feature = "device-control")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiDeviceControlKind {
    /// 04H 01H Master Volume, 14 bits.
    MasterVolume = 0,
    /// 04H 02H Master Balance, 14 bits.
    MasterBalance = 1,
    /// 04H 03H Master Fine Tuning, 14 bits.
    MasterFineTuning = 2,
    /// 04H 04H Master Coarse Tuning, the 7-bit MSB.
    MasterCoarseTuning = 3,
}

/// One Device Control message (REQ-FFI-SX-005).
#[cfg(feature = "device-control")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiDeviceControl {
    /// A [`ZmidiDeviceControlKind`] value.
    pub kind: u32,
    /// The Device ID; 7FH addresses all devices.
    pub device_id: u8,
    /// The value: 14 bits, or 7 bits for Master Coarse Tuning.
    pub value: u16,
}

/// Reads a Device Control message (REQ-FFI-SX-005).
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `out` is null or writable.
#[cfg(feature = "device-control")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_device_control_read(
    bytes: *const u8,
    length: usize,
    out: *mut ZmidiDeviceControl,
) -> ZmidiStatus {
    if out.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let m = match unsafe { universal(bytes, length) }
        .and_then(|u| part(DeviceControl::from_universal(&u)))
    {
        Ok(m) => m,
        Err(s) => return s,
    };
    let (kind, value) = match m.value {
        DeviceControlValue::MasterVolume(v) => (ZmidiDeviceControlKind::MasterVolume, v),
        DeviceControlValue::MasterBalance(v) => (ZmidiDeviceControlKind::MasterBalance, v),
        DeviceControlValue::MasterFineTuning(v) => (ZmidiDeviceControlKind::MasterFineTuning, v),
        DeviceControlValue::MasterCoarseTuning(v) => {
            (ZmidiDeviceControlKind::MasterCoarseTuning, u16::from(v))
        }
    };
    // SAFETY: caller contract above.
    unsafe {
        *out = ZmidiDeviceControl {
            kind: kind as u32,
            device_id: m.device_id,
            value,
        }
    };
    ZmidiStatus::Ok
}

/// Writes a Device Control message (REQ-FFI-SX-005). A value above its width gives
/// [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `control` is null or readable; `out` is null or points to `capacity` writable bytes; `length`
/// is null or writable.
#[cfg(feature = "device-control")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_device_control_write(
    control: *const ZmidiDeviceControl,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    if control.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let c = unsafe { *control };
    let value = match c.kind {
        k if k == ZmidiDeviceControlKind::MasterVolume as u32 => {
            DeviceControlValue::MasterVolume(c.value)
        }
        k if k == ZmidiDeviceControlKind::MasterBalance as u32 => {
            DeviceControlValue::MasterBalance(c.value)
        }
        k if k == ZmidiDeviceControlKind::MasterFineTuning as u32 => {
            DeviceControlValue::MasterFineTuning(c.value)
        }
        k if k == ZmidiDeviceControlKind::MasterCoarseTuning as u32 && c.value <= 0x7F => {
            DeviceControlValue::MasterCoarseTuning(c.value as u8)
        }
        _ => return ZmidiStatus::InvalidArgument,
    };
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            DeviceControl {
                device_id: c.device_id,
                value,
            }
            .write(o)
        })
    }
}

/// Gives the signed offset from 8192 of a 14-bit Master Fine Tuning value (CA-025;
/// REQ-FFI-SX-005).
///
/// # Safety
/// `offset` is null or writable.
#[cfg(feature = "device-control")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_fine_tuning_offset(value: u16, offset: *mut i16) -> ZmidiStatus {
    if offset.is_null() || value > 0x3FFF {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe { *offset = zmidi_sysex::fine_tuning_offset(value) };
    ZmidiStatus::Ok
}

/// Gives the semitones from A440 of a Master Coarse Tuning MSB, -64 to +63 (CA-025;
/// REQ-FFI-SX-005).
///
/// # Safety
/// `semitones` is null or writable.
#[cfg(feature = "device-control")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_coarse_tuning_semitones(msb: u8, semitones: *mut i8) -> ZmidiStatus {
    if semitones.is_null() || msb > 0x7F {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    unsafe { *semitones = zmidi_sysex::coarse_tuning_semitones(msb) };
    ZmidiStatus::Ok
}

/// A MIDI Tuning frequency word: the semitone and the 14-bit fraction (REQ-FFI-SX-006).
#[cfg(feature = "tuning")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiFrequencyWord {
    /// The semitone, 0 to 127.
    pub semitone: u8,
    /// The fraction in steps of 100/16384 cent, 0 to 3FFFH.
    pub fraction: u16,
}

#[cfg(feature = "tuning")]
impl ZmidiFrequencyWord {
    const fn of(w: FrequencyWord) -> Self {
        Self {
            semitone: w.semitone,
            fraction: w.fraction,
        }
    }

    /// The Rust word, or `None` for a value above its width.
    const fn word(self) -> Option<FrequencyWord> {
        if self.semitone > 0x7F || self.fraction > 0x3FFF {
            None
        } else {
            Some(FrequencyWord {
                semitone: self.semitone,
                fraction: self.fraction,
            })
        }
    }
}

/// One note of a tuning: the key and its frequency word (REQ-FFI-SX-006).
#[cfg(feature = "tuning")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiTuningNote {
    /// The key, 0 to 127.
    pub key: u8,
    /// The frequency word.
    pub word: ZmidiFrequencyWord,
}

/// The 12 scale/octave offsets from C to B (REQ-FFI-SX-006).
#[cfg(feature = "tuning")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiTuningOffsets {
    /// True for the 2-byte form (-8192 to +8191, steps of 100/8192 cent); false for the 1-byte
    /// form (cents, -64 to +63).
    pub two_byte: bool,
    /// The offsets.
    pub offsets: [i16; 12],
}

#[cfg(feature = "tuning")]
impl ZmidiTuningOffsets {
    fn of(o: ScaleOffsets) -> Self {
        match o {
            ScaleOffsets::OneByte(x) => Self {
                two_byte: false,
                offsets: x.map(i16::from),
            },
            ScaleOffsets::TwoByte(offsets) => Self {
                two_byte: true,
                offsets,
            },
        }
    }

    /// The Rust offsets; `None` for a 1-byte offset outside an `i8` (the writer checks the
    /// ranges).
    fn offsets(&self) -> Option<ScaleOffsets> {
        if self.two_byte {
            return Some(ScaleOffsets::TwoByte(self.offsets));
        }
        let mut o = [0i8; 12];
        for (x, &v) in o.iter_mut().zip(&self.offsets) {
            *x = i8::try_from(v).ok()?;
        }
        Some(ScaleOffsets::OneByte(o))
    }
}

/// The kind of a MIDI Tuning message (REQ-FFI-SX-006).
#[cfg(feature = "tuning")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiTuningKind {
    /// 00H or 03H Bulk Tuning Dump Request.
    DumpRequest = 0,
    /// 01H Bulk Tuning Dump or 04H Key-Based Tuning Dump.
    KeyDump = 1,
    /// 02H or 07H Single Note Tuning Change.
    NoteChange = 2,
    /// 05H or 06H Scale/Octave Tuning Dump.
    ScaleDump = 3,
    /// 08H or 09H Scale/Octave Tuning.
    ScaleOctave = 4,
}

/// One MIDI Tuning message as flat values; the fields that the kind does not have are 0
/// (REQ-FFI-SX-006).
#[cfg(feature = "tuning")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiTuning {
    /// A [`ZmidiTuningKind`] value.
    pub kind: u32,
    /// True for the Real Time form (NoteChange, ScaleOctave).
    pub real_time: bool,
    /// The Device ID.
    pub device_id: u8,
    /// True when the message has a bank.
    pub has_bank: bool,
    /// The bank, 0 to 127.
    pub bank: u8,
    /// The tuning program, 0 to 127.
    pub program: u8,
    /// The 16-byte tuning name (KeyDump, ScaleDump).
    pub name: [u8; 16],
    /// The result of the checksum check of a 01H Bulk Tuning Dump (true for the other kinds).
    pub checksum_ok: bool,
    /// The channels of ScaleOctave: bit n is the channel with packet value n.
    pub channels: u16,
    /// The offsets of ScaleDump and ScaleOctave.
    pub offsets: ZmidiTuningOffsets,
    /// The count of notes: 128 for KeyDump, the sets of NoteChange.
    pub count: u32,
}

/// Reads a MIDI Tuning message (REQ-FFI-SX-006). The notes go into `notes`: 128 for a key-based
/// dump (key = index), the sets of a note change. When `capacity` is smaller than the count,
/// `*tuning` holds the message with the count and the status is
/// [`ZmidiStatus::BufferTooSmall`]; `notes` may be null when `capacity` is 0.
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `tuning` is null or writable; `notes`
/// is null or points to `capacity` writable notes.
#[cfg(feature = "tuning")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_tuning_read(
    bytes: *const u8,
    length: usize,
    tuning: *mut ZmidiTuning,
    notes: *mut ZmidiTuningNote,
    capacity: usize,
) -> ZmidiStatus {
    if tuning.is_null() || (notes.is_null() && capacity != 0) {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let m = match unsafe { universal(bytes, length) }
        .and_then(|u| part(TuningMessage::from_universal(&u)))
    {
        Ok(m) => m,
        Err(s) => return s,
    };
    let mut t = ZmidiTuning {
        kind: 0,
        real_time: false,
        device_id: 0,
        has_bank: false,
        bank: 0,
        program: 0,
        name: [0; 16],
        checksum_ok: true,
        channels: 0,
        offsets: ZmidiTuningOffsets {
            two_byte: false,
            offsets: [0; 12],
        },
        count: 0,
    };
    let notes: &mut [ZmidiTuningNote] = if capacity == 0 {
        &mut []
    } else {
        // SAFETY: caller contract above.
        unsafe { core::slice::from_raw_parts_mut(notes, capacity) }
    };
    let mut fits = true;
    match m {
        TuningMessage::DumpRequest {
            device_id,
            bank,
            program,
        } => {
            t.kind = ZmidiTuningKind::DumpRequest as u32;
            (t.device_id, t.has_bank, t.bank, t.program) =
                (device_id, bank.is_some(), bank.unwrap_or(0), program);
        }
        TuningMessage::KeyDump {
            device_id,
            bank,
            program,
            name,
            words,
            checksum_ok,
        } => {
            t.kind = ZmidiTuningKind::KeyDump as u32;
            (t.device_id, t.has_bank, t.bank, t.program) =
                (device_id, bank.is_some(), bank.unwrap_or(0), program);
            (t.name, t.checksum_ok, t.count) = (*name, checksum_ok, words.len() as u32);
            fits = notes.len() >= words.len();
            if fits {
                for (k, (n, w)) in notes.iter_mut().zip(words).enumerate() {
                    *n = ZmidiTuningNote {
                        key: k as u8,
                        word: ZmidiFrequencyWord::of(w),
                    };
                }
            }
        }
        TuningMessage::NoteChange {
            real_time,
            device_id,
            bank,
            program,
            changes,
        } => {
            t.kind = ZmidiTuningKind::NoteChange as u32;
            (t.real_time, t.device_id, t.has_bank, t.bank, t.program) = (
                real_time,
                device_id,
                bank.is_some(),
                bank.unwrap_or(0),
                program,
            );
            t.count = changes.len() as u32;
            fits = notes.len() >= changes.len();
            if fits {
                for (n, (key, w)) in notes.iter_mut().zip(changes) {
                    *n = ZmidiTuningNote {
                        key,
                        word: ZmidiFrequencyWord::of(w),
                    };
                }
            }
        }
        TuningMessage::ScaleDump {
            device_id,
            bank,
            program,
            name,
            offsets,
        } => {
            t.kind = ZmidiTuningKind::ScaleDump as u32;
            (t.device_id, t.has_bank, t.bank, t.program, t.name) =
                (device_id, true, bank, program, *name);
            t.offsets = ZmidiTuningOffsets::of(offsets);
        }
        TuningMessage::ScaleOctave {
            real_time,
            device_id,
            channels,
            offsets,
        } => {
            t.kind = ZmidiTuningKind::ScaleOctave as u32;
            (t.real_time, t.device_id, t.channels) = (real_time, device_id, channels);
            t.offsets = ZmidiTuningOffsets::of(offsets);
        }
    }
    // SAFETY: caller contract above.
    unsafe { *tuning = t };
    if fits {
        ZmidiStatus::Ok
    } else {
        ZmidiStatus::BufferTooSmall
    }
}

/// Reads a frequency word from its three bytes (M1 MIDI Tuning; REQ-FFI-SX-006).
///
/// # Safety
/// `bytes` is null or points to 3 readable bytes; `word` is null or writable.
#[cfg(feature = "tuning")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_frequency_word_from_bytes(
    bytes: *const u8,
    word: *mut ZmidiFrequencyWord,
) -> ZmidiStatus {
    if bytes.is_null() || word.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    match FrequencyWord::from_bytes(unsafe { *bytes.cast::<[u8; 3]>() }) {
        Ok(w) => {
            // SAFETY: caller contract above.
            unsafe { *word = ZmidiFrequencyWord::of(w) };
            ZmidiStatus::Ok
        }
        Err(_) => ZmidiStatus::InvalidArgument,
    }
}

/// Writes the three bytes of a frequency word (REQ-FFI-SX-006).
///
/// # Safety
/// `word` is null or readable; `bytes` is null or points to 3 writable bytes.
#[cfg(feature = "tuning")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_frequency_word_to_bytes(
    word: *const ZmidiFrequencyWord,
    bytes: *mut u8,
) -> ZmidiStatus {
    if word.is_null() || bytes.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(w) = (unsafe { *word }).word() else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe { bytes.cast::<[u8; 3]>().write_unaligned(w.to_bytes()) };
    ZmidiStatus::Ok
}

/// Gives the checksum of `bytes`: the XOR of the bytes, masked to 7 bits (M1 MIDI Tuning;
/// REQ-FFI-SX-006).
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `checksum` is null or writable.
#[cfg(feature = "tuning")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_tuning_checksum(
    bytes: *const u8,
    length: usize,
    checksum: *mut u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match (unsafe { input(bytes, length) }, checksum.is_null()) {
        (Some(b), false) => {
            // SAFETY: caller contract above.
            unsafe { *checksum = zmidi_sysex::checksum(b) };
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// Writes a Bulk Tuning Dump Request: 00H, or 03H with `bank` when `has_bank`
/// (REQ-FFI-SX-006).
///
/// # Safety
/// `out` is null or points to `capacity` writable bytes; `length` is null or writable.
#[cfg(feature = "tuning")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_tuning_dump_request_write(
    device_id: u8,
    has_bank: bool,
    bank: u8,
    program: u8,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            zmidi_sysex::write_dump_request(device_id, has_bank.then_some(bank), program, o)
        })
    }
}

/// Writes a Bulk Tuning Dump (01H), or a Key-Based Tuning Dump (04H) with `bank` when
/// `has_bank`, from the 16-byte `name` and 128 `words` (REQ-FFI-SX-006).
///
/// # Safety
/// `name` is null or points to 16 readable bytes; `words` is null or points to 128 readable
/// words; `out` is null or points to `capacity` writable bytes; `length` is null or writable.
#[cfg(feature = "tuning")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_tuning_key_dump_write(
    device_id: u8,
    has_bank: bool,
    bank: u8,
    program: u8,
    name: *const u8,
    words: *const ZmidiFrequencyWord,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    if name.is_null() || words.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (name, words) = unsafe {
        (
            &*name.cast::<[u8; 16]>(),
            core::slice::from_raw_parts(words, 128),
        )
    };
    let mut w = [FrequencyWord::NO_CHANGE; 128];
    for (x, c) in w.iter_mut().zip(words) {
        let Some(v) = c.word() else {
            return ZmidiStatus::InvalidArgument;
        };
        *x = v;
    }
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            zmidi_sysex::write_key_dump(device_id, has_bank.then_some(bank), program, name, &w, o)
        })
    }
}

/// Writes a Single Note Tuning Change: Real Time 02H, or 07H with `bank` when `has_bank` (the
/// Non-Real Time form needs a bank), from 1 to 127 `notes` (REQ-FFI-SX-006).
///
/// # Safety
/// `notes` is null or points to `count` readable notes; `out` is null or points to `capacity`
/// writable bytes; `length` is null or writable.
#[cfg(feature = "tuning")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_tuning_note_change_write(
    real_time: bool,
    device_id: u8,
    has_bank: bool,
    bank: u8,
    program: u8,
    notes: *const ZmidiTuningNote,
    count: usize,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    if notes.is_null() || count > 127 {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let notes = unsafe { core::slice::from_raw_parts(notes, count) };
    let mut changes = [(0u8, FrequencyWord::NO_CHANGE); 127];
    for (x, n) in changes.iter_mut().zip(notes) {
        let Some(w) = n.word.word() else {
            return ZmidiStatus::InvalidArgument;
        };
        *x = (n.key, w);
    }
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            zmidi_sysex::write_note_change(
                real_time,
                device_id,
                has_bank.then_some(bank),
                program,
                &changes[..count],
                o,
            )
        })
    }
}

/// Writes a Scale/Octave Tuning Dump (05H or 06H by the form of `offsets`) with the 16-byte
/// `name` (REQ-FFI-SX-006).
///
/// # Safety
/// `name` is null or points to 16 readable bytes; `offsets` is null or readable; `out` is null
/// or points to `capacity` writable bytes; `length` is null or writable.
#[cfg(feature = "tuning")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_tuning_scale_dump_write(
    device_id: u8,
    bank: u8,
    program: u8,
    name: *const u8,
    offsets: *const ZmidiTuningOffsets,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    if name.is_null() || offsets.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (name, offsets) = unsafe { (&*name.cast::<[u8; 16]>(), (*offsets).offsets()) };
    let Some(offsets) = offsets else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            zmidi_sysex::write_scale_dump(device_id, bank, program, name, &offsets, o)
        })
    }
}

/// Writes a Scale/Octave Tuning message (08H or 09H by the form of `offsets`), Real Time or
/// Non-Real Time, for the `channels` mask (REQ-FFI-SX-006).
///
/// # Safety
/// `offsets` is null or readable; `out` is null or points to `capacity` writable bytes; `length`
/// is null or writable.
#[cfg(feature = "tuning")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_tuning_scale_octave_write(
    real_time: bool,
    device_id: u8,
    channels: u16,
    offsets: *const ZmidiTuningOffsets,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    if offsets.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(offsets) = (unsafe { *offsets }).offsets() else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            zmidi_sysex::write_scale_octave(real_time, device_id, channels, &offsets, o)
        })
    }
}

/// An MTC time (RP-004; REQ-FFI-SX-007). A reader gives the counts as received; a writer checks
/// them.
#[cfg(feature = "mtc")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMtcTime {
    /// Hours, 0 to 23.
    pub hours: u8,
    /// Minutes, 0 to 59.
    pub minutes: u8,
    /// Seconds, 0 to 59.
    pub seconds: u8,
    /// Frames, below the frame rate of the type.
    pub frames: u8,
    /// The time code type: 0 (24), 1 (25), 2 (30 drop-frame) or 3 (30 frames per second).
    pub rate: u8,
}

#[cfg(feature = "mtc")]
impl ZmidiMtcTime {
    const fn of(t: MtcTime) -> Self {
        Self {
            hours: t.hours,
            minutes: t.minutes,
            seconds: t.seconds,
            frames: t.frames,
            rate: t.rate.into_bits(),
        }
    }

    /// The Rust time, or `None` for a type above 3.
    const fn time(self) -> Option<MtcTime> {
        if self.rate > 3 {
            return None;
        }
        Some(MtcTime {
            hours: self.hours,
            minutes: self.minutes,
            seconds: self.seconds,
            frames: self.frames,
            rate: TimeCodeType::from_bits(self.rate),
        })
    }
}

/// The time of a pointer of the program, or `None` for null or a type above 3.
///
/// # Safety
/// `time` is null or readable.
#[cfg(feature = "mtc")]
unsafe fn mtc_time(time: *const ZmidiMtcTime) -> Option<MtcTime> {
    if time.is_null() {
        return None;
    }
    // SAFETY: caller contract above.
    unsafe { *time }.time()
}

/// Runs `f` on the time of the program and writes its result (REQ-FFI-SX-007).
///
/// # Safety
/// `time` is null or readable; `out` is null or writable.
#[cfg(feature = "mtc")]
unsafe fn map_time<T>(
    time: *const ZmidiMtcTime,
    out: *mut T,
    f: impl FnOnce(MtcTime) -> T,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match (unsafe { mtc_time(time) }, out.is_null()) {
        (Some(t), false) => {
            // SAFETY: caller contract above.
            unsafe { *out = f(t) };
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// Gives in `*valid` whether each count of `time` is in its range and the frame number is not
/// one that the drop-frame type skips (REQ-FFI-SX-007).
///
/// # Safety
/// `time` is null or readable; `valid` is null or writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_time_is_valid(
    time: *const ZmidiMtcTime,
    valid: *mut bool,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { map_time(time, valid, |t| t.is_valid()) }
}

/// Gives `time`, or frame 02 for a frame number that the drop-frame type skips (RP-013 Drop
/// Frame Notes; REQ-FFI-SX-007).
///
/// # Safety
/// `time` is null or readable; `out` is null or writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_next_valid(
    time: *const ZmidiMtcTime,
    out: *mut ZmidiMtcTime,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { map_time(time, out, |t| ZmidiMtcTime::of(t.next_valid())) }
}

/// Gives the frames from 00:00:00:00 to `time` by the rule of its type (REQ-FFI-SX-007).
///
/// # Safety
/// `time` is null or readable; `count` is null or writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_to_frame_count(
    time: *const ZmidiMtcTime,
    count: *mut u32,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { map_time(time, count, |t| t.to_frame_count()) }
}

/// Gives the time of `count` frames from 00:00:00:00 for the type `rate`, with the 24-hour wrap
/// (REQ-FFI-SX-007).
///
/// # Safety
/// `out` is null or writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_from_frame_count(
    count: u32,
    rate: u8,
    out: *mut ZmidiMtcTime,
) -> ZmidiStatus {
    if out.is_null() || rate > 3 {
        return ZmidiStatus::InvalidArgument;
    }
    let t = MtcTime::from_frame_count(count, TimeCodeType::from_bits(rate));
    // SAFETY: caller contract above.
    unsafe { *out = ZmidiMtcTime::of(t) };
    ZmidiStatus::Ok
}

/// Gives the time `delta` frames after (or before) `time`, with the 24-hour wrap
/// (REQ-FFI-SX-007).
///
/// # Safety
/// `time` is null or readable; `out` is null or writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_add_frames(
    time: *const ZmidiMtcTime,
    delta: i32,
    out: *mut ZmidiMtcTime,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { map_time(time, out, |t| ZmidiMtcTime::of(t.add_frames(delta))) }
}

/// Gives the 8 Quarter Frame data bytes of `time`, piece 0 first (RP-004 p. 2;
/// REQ-FFI-SX-007).
///
/// # Safety
/// `time` is null or readable; `bytes` is null or points to 8 writable bytes.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_quarter_frames(
    time: *const ZmidiMtcTime,
    bytes: *mut u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    unsafe { map_time(time, bytes.cast::<[u8; 8]>(), |t| QuarterFrame::pieces(&t)) }
}

/// Storage words of a [`ZmidiMtcGenerator`].
#[cfg(feature = "mtc")]
pub const ZMIDI_MTC_GENERATOR_WORDS: usize = 2;
/// Storage words of a [`ZmidiMtcReader`].
#[cfg(feature = "mtc")]
pub const ZMIDI_MTC_READER_WORDS: usize = 4;

/// Storage of one Quarter Frame generator. The program gives it; [`zmidi_mtc_generator_init`]
/// sets it up (REQ-FFI-SX-009).
#[cfg(feature = "mtc")]
#[repr(C)]
pub struct ZmidiMtcGenerator {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_MTC_GENERATOR_WORDS],
}

/// Storage of one Quarter Frame reader. The program gives it; [`zmidi_mtc_reader_init`] sets it
/// up (REQ-FFI-SX-009).
#[cfg(feature = "mtc")]
#[repr(C)]
pub struct ZmidiMtcReader {
    /// Opaque content.
    pub _opaque: [u64; ZMIDI_MTC_READER_WORDS],
}

#[cfg(feature = "mtc")]
const _: () = assert!(size_of::<MtcGenerator>() <= size_of::<ZmidiMtcGenerator>());
#[cfg(feature = "mtc")]
const _: () = assert!(align_of::<MtcGenerator>() <= align_of::<ZmidiMtcGenerator>());
#[cfg(feature = "mtc")]
const _: () = assert!(size_of::<MtcReader>() <= size_of::<ZmidiMtcReader>());
#[cfg(feature = "mtc")]
const _: () = assert!(align_of::<MtcReader>() <= align_of::<ZmidiMtcReader>());

#[cfg(feature = "mtc")]
const fn direction(reverse: bool) -> Direction {
    if reverse {
        Direction::Reverse
    } else {
        Direction::Forward
    }
}

/// Sets up `generator`, running: the first sequence encodes `time` and starts at piece 0, or at
/// piece 7 when `reverse` (RP-004 p. 3; REQ-FFI-SX-007). A time that is not valid gives
/// [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `generator` is null or points to writable storage of a [`ZmidiMtcGenerator`]; `time` is null
/// or readable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_generator_init(
    generator: *mut ZmidiMtcGenerator,
    time: *const ZmidiMtcTime,
    reverse: bool,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match (generator.is_null(), unsafe { mtc_time(time) }) {
        (false, Some(t)) if t.is_valid() => {
            // SAFETY: the storage is large and aligned enough (asserts above) and writable.
            unsafe {
                generator
                    .cast::<MtcGenerator>()
                    .write(MtcGenerator::new(t, direction(reverse)))
            };
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// The generator of the program.
///
/// # Safety
/// `generator` is null or was set up by [`zmidi_mtc_generator_init`].
#[cfg(feature = "mtc")]
unsafe fn generator<'a>(generator: *mut ZmidiMtcGenerator) -> Option<&'a mut MtcGenerator> {
    // SAFETY: caller contract above.
    unsafe { generator.cast::<MtcGenerator>().as_mut() }
}

/// Gives the data byte of the next piece in `*byte`, or [`ZmidiStatus::NoPending`] while the
/// generator is stopped (REQ-FFI-SX-007).
///
/// # Safety
/// `generator` is null or was set up by [`zmidi_mtc_generator_init`]; `byte` is null or
/// writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_generator_next(
    generator: *mut ZmidiMtcGenerator,
    byte: *mut u8,
) -> ZmidiStatus {
    if byte.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(g) = (unsafe { self::generator(generator) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    match g.next_byte() {
        Some(b) => {
            // SAFETY: caller contract above.
            unsafe { *byte = b };
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::NoPending,
    }
}

/// Runs or stops the generator (RP-004 p. 3; REQ-FFI-SX-007).
///
/// # Safety
/// `generator` is null or was set up by [`zmidi_mtc_generator_init`].
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_generator_set_running(
    generator: *mut ZmidiMtcGenerator,
    running: bool,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match unsafe { self::generator(generator) } {
        Some(g) => {
            g.set_running(running);
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::InvalidArgument,
    }
}

/// Sets the direction: the next piece is the next one in the new order (AMB-RP-004-13;
/// REQ-FFI-SX-007).
///
/// # Safety
/// `generator` is null or was set up by [`zmidi_mtc_generator_init`].
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_generator_set_direction(
    generator: *mut ZmidiMtcGenerator,
    reverse: bool,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match unsafe { self::generator(generator) } {
        Some(g) => {
            g.set_direction(direction(reverse));
            ZmidiStatus::Ok
        }
        None => ZmidiStatus::InvalidArgument,
    }
}

/// Moves the generator to `time`; the next piece is the first piece of the direction
/// (REQ-FFI-SX-007). A time that is not valid gives [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `generator` is null or was set up by [`zmidi_mtc_generator_init`]; `time` is null or
/// readable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_generator_locate(
    generator: *mut ZmidiMtcGenerator,
    time: *const ZmidiMtcTime,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match (unsafe { self::generator(generator) }, unsafe {
        mtc_time(time)
    }) {
        (Some(g), Some(t)) if t.is_valid() => {
            g.locate(t);
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// Gives the time that the current or next sequence encodes at its piece 0 (REQ-FFI-SX-007).
///
/// # Safety
/// `generator` is null or was set up by [`zmidi_mtc_generator_init`]; `time` is null or
/// writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_generator_time(
    generator: *mut ZmidiMtcGenerator,
    time: *mut ZmidiMtcTime,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match (unsafe { self::generator(generator) }, time.is_null()) {
        (Some(g), false) => {
            // SAFETY: caller contract above.
            unsafe { *time = ZmidiMtcTime::of(g.time()) };
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// Gives `size_of` the Rust generator, for a binding that checks its copy of the layout (DEC-019
/// rule 6).
#[cfg(feature = "mtc")]
#[no_mangle]
pub extern "C" fn zmidi_mtc_generator_size() -> usize {
    size_of::<MtcGenerator>()
}

/// What one Quarter Frame data byte gives (REQ-FFI-SX-007).
#[cfg(feature = "mtc")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMtcEvent {
    /// The piece, 0 to 7.
    pub piece: u8,
    /// True for pieces 0 and 4, which mark frame boundaries.
    pub boundary: bool,
    /// True at the end of a full sequence: the fields below are set.
    pub has_sequence: bool,
    /// The assembled time, counts as received.
    pub raw: ZmidiMtcTime,
    /// The current time: `raw` plus 2 frames forward, minus 1 frame in reverse
    /// (AMB-RP-004-03).
    pub current: ZmidiMtcTime,
    /// True for a reverse sequence.
    pub reverse: bool,
    /// True when `raw` is not the previous raw time plus 2 frames (minus 2 in reverse).
    pub discontinuity: bool,
}

/// The state of a reader (REQ-FFI-SX-007).
#[cfg(feature = "mtc")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMtcReaderInfo {
    /// True after a full sequence, until a broken order or a direction change.
    pub locked: bool,
    /// True after a Quarter Frame data byte, false after a Full Message.
    pub running: bool,
    /// True when two pieces gave the direction.
    pub has_direction: bool,
    /// True for reverse, when `has_direction`.
    pub reverse: bool,
    /// True when `time` is set.
    pub has_time: bool,
    /// The current time of the last full sequence, or the time of a Full Message after it.
    pub time: ZmidiMtcTime,
}

#[cfg(feature = "mtc")]
const NO_TIME: ZmidiMtcTime = ZmidiMtcTime {
    hours: 0,
    minutes: 0,
    seconds: 0,
    frames: 0,
    rate: 0,
};

#[cfg(feature = "mtc")]
impl ZmidiMtcEvent {
    fn of(e: MtcEvent) -> Self {
        let mut c = Self {
            piece: e.piece,
            boundary: e.boundary,
            has_sequence: false,
            raw: NO_TIME,
            current: NO_TIME,
            reverse: false,
            discontinuity: false,
        };
        if let Some(s) = e.sequence {
            c.has_sequence = true;
            c.raw = ZmidiMtcTime::of(s.raw);
            c.current = ZmidiMtcTime::of(s.current);
            c.reverse = s.direction == Direction::Reverse;
            c.discontinuity = s.discontinuity;
        }
        c
    }
}

/// Sets up `reader`: not locked, stopped, no time (REQ-FFI-SX-007).
///
/// # Safety
/// `reader` is null or points to writable storage of a [`ZmidiMtcReader`].
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_reader_init(reader: *mut ZmidiMtcReader) -> ZmidiStatus {
    if reader.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: the storage is large and aligned enough (asserts above) and writable (caller).
    unsafe { reader.cast::<MtcReader>().write(MtcReader::new()) };
    ZmidiStatus::Ok
}

/// The reader of the program.
///
/// # Safety
/// `reader` is null or was set up by [`zmidi_mtc_reader_init`].
#[cfg(feature = "mtc")]
unsafe fn reader<'a>(reader: *mut ZmidiMtcReader) -> Option<&'a mut MtcReader> {
    // SAFETY: caller contract above.
    unsafe { reader.cast::<MtcReader>().as_mut() }
}

/// Takes one Quarter Frame data byte, 00H to 7FH (RP-004 p. 3; REQ-FFI-SX-007).
///
/// # Safety
/// `reader` is null or was set up by [`zmidi_mtc_reader_init`]; `event` is null or writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_reader_receive(
    reader: *mut ZmidiMtcReader,
    byte: u8,
    event: *mut ZmidiMtcEvent,
) -> ZmidiStatus {
    if event.is_null() || byte > 0x7F {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(r) = (unsafe { self::reader(reader) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let e = ZmidiMtcEvent::of(r.receive(byte));
    // SAFETY: caller contract above.
    unsafe { *event = e };
    ZmidiStatus::Ok
}

/// Takes the time of a Full Message: the reader has this time and is stopped until the next
/// Quarter Frame data byte (AMB-RP-004-06; REQ-FFI-SX-007).
///
/// # Safety
/// `reader` is null or was set up by [`zmidi_mtc_reader_init`]; `time` is null or readable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_reader_full_message(
    reader: *mut ZmidiMtcReader,
    time: *const ZmidiMtcTime,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match (unsafe { self::reader(reader) }, unsafe { mtc_time(time) }) {
        (Some(r), Some(time)) => {
            r.full_message(&FullMessage {
                device_id: 0x7F,
                time,
            });
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// Gives the state of the reader (REQ-FFI-SX-007).
///
/// # Safety
/// `reader` is null or was set up by [`zmidi_mtc_reader_init`]; `info` is null or writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_reader_info(
    reader: *mut ZmidiMtcReader,
    info: *mut ZmidiMtcReaderInfo,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match (unsafe { self::reader(reader) }, info.is_null()) {
        (Some(r), false) => {
            let i = ZmidiMtcReaderInfo {
                locked: r.is_locked(),
                running: r.is_running(),
                has_direction: r.direction().is_some(),
                reverse: r.direction() == Some(Direction::Reverse),
                has_time: r.time().is_some(),
                time: r.time().map_or(NO_TIME, ZmidiMtcTime::of),
            };
            // SAFETY: caller contract above.
            unsafe { *info = i };
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// Gives `size_of` the Rust reader, for a binding that checks its copy of the layout (DEC-019
/// rule 6).
#[cfg(feature = "mtc")]
#[no_mangle]
pub extern "C" fn zmidi_mtc_reader_size() -> usize {
    size_of::<MtcReader>()
}

/// Reads an MTC Full Message (RP-004 p. 5; REQ-FFI-SX-007). The counts are as received.
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `device_id` and `time` are null or
/// writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_full_read(
    bytes: *const u8,
    length: usize,
    device_id: *mut u8,
    time: *mut ZmidiMtcTime,
) -> ZmidiStatus {
    if device_id.is_null() || time.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    match unsafe { universal(bytes, length) }.and_then(|u| part(FullMessage::from_universal(&u))) {
        Ok(m) => {
            // SAFETY: caller contract above.
            unsafe {
                *device_id = m.device_id;
                *time = ZmidiMtcTime::of(m.time);
            }
            ZmidiStatus::Ok
        }
        Err(s) => s,
    }
}

/// Writes an MTC Full Message (RP-004 p. 5; REQ-FFI-SX-007). A time that is not valid gives
/// [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `time` is null or readable; `out` is null or points to `capacity` writable bytes; `length`
/// is null or writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_full_write(
    device_id: u8,
    time: *const ZmidiMtcTime,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some(time) = (unsafe { mtc_time(time) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            FullMessage { device_id, time }.write(o)
        })
    }
}

/// The MTC User Bits (RP-004 p. 6; REQ-FFI-SX-007).
#[cfg(feature = "mtc")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMtcUserBits {
    /// Binary Groups 1 to 8, 4 bits each.
    pub groups: [u8; 8],
    /// The Binary Group Flag Bits: bit 1 is j, bit 0 is i.
    pub flags: u8,
}

/// Reads an MTC User Bits message (RP-004 p. 6; REQ-FFI-SX-007).
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `device_id` and `bits` are null or
/// writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_user_bits_read(
    bytes: *const u8,
    length: usize,
    device_id: *mut u8,
    bits: *mut ZmidiMtcUserBits,
) -> ZmidiStatus {
    if device_id.is_null() || bits.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    match unsafe { universal(bytes, length) }.and_then(|u| part(UserBits::from_universal(&u))) {
        Ok(m) => {
            // SAFETY: caller contract above.
            unsafe {
                *device_id = m.device_id;
                *bits = ZmidiMtcUserBits {
                    groups: m.groups,
                    flags: m.flags,
                };
            }
            ZmidiStatus::Ok
        }
        Err(s) => s,
    }
}

/// Writes an MTC User Bits message (RP-004 p. 6; REQ-FFI-SX-007). A group above 0FH or flags
/// above 03H give [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `bits` is null or readable; `out` is null or points to `capacity` writable bytes; `length` is
/// null or writable.
#[cfg(feature = "mtc")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mtc_user_bits_write(
    device_id: u8,
    bits: *const ZmidiMtcUserBits,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    if bits.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let b = unsafe { *bits };
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| {
            UserBits {
                device_id,
                groups: b.groups,
                flags: b.flags,
            }
            .write(o)
        })
    }
}

/// An MMC Standard Time Code (RP-013 pp. 8, 9; REQ-FFI-SX-008).
#[cfg(feature = "mmc-transport")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiTimeCode {
    /// The time with its type; a reader gives the counts as received.
    pub time: ZmidiMtcTime,
    /// c: the color frame flag.
    pub color_frame: bool,
    /// k: the blank flag.
    pub blank: bool,
    /// g: the sign.
    pub negative: bool,
    /// i: true for the status form {st}, false for the subframes form {ff}.
    pub status_form: bool,
    /// The subframes of the {ff} form, 0 to 99.
    pub subframes: u8,
    /// e: the estimated code flag of the {st} form.
    pub estimated: bool,
    /// v: the invalid code flag of the {st} form.
    pub invalid: bool,
    /// d: video field 1, of the {st} form.
    pub video_field1: bool,
    /// n: no time code, of the {st} form.
    pub no_time_code: bool,
}

#[cfg(feature = "mmc-transport")]
impl ZmidiTimeCode {
    const fn final_of(f: FinalByte) -> (bool, u8, TimeCodeStatus) {
        match f {
            FinalByte::Subframes(s) => (false, s, TimeCodeStatus::from_byte(0)),
            FinalByte::Status(st) => (true, 0, st),
        }
    }

    const fn of(t: StandardTimeCode) -> Self {
        let (status_form, subframes, st) = Self::final_of(t.final_byte);
        Self {
            time: ZmidiMtcTime::of(t.time),
            color_frame: t.color_frame,
            blank: t.blank,
            negative: t.negative,
            status_form,
            subframes,
            estimated: st.estimated,
            invalid: st.invalid,
            video_field1: st.video_field1,
            no_time_code: st.no_time_code,
        }
    }

    const fn final_byte(&self) -> FinalByte {
        if self.status_form {
            FinalByte::Status(TimeCodeStatus {
                estimated: self.estimated,
                invalid: self.invalid,
                video_field1: self.video_field1,
                no_time_code: self.no_time_code,
            })
        } else {
            FinalByte::Subframes(self.subframes)
        }
    }

    /// The Rust time code, or `None` for a type above 3.
    const fn code(&self) -> Option<StandardTimeCode> {
        let Some(time) = self.time.time() else {
            return None;
        };
        Some(StandardTimeCode {
            time,
            color_frame: self.color_frame,
            blank: self.blank,
            negative: self.negative,
            final_byte: self.final_byte(),
        })
    }

    const fn short(&self) -> ShortTimeCode {
        ShortTimeCode {
            frames: self.time.frames,
            negative: self.negative,
            final_byte: self.final_byte(),
        }
    }
}

/// The time code of a pointer of the program, or `None` for null or a type above 3.
///
/// # Safety
/// `code` is null or readable.
#[cfg(feature = "mmc-transport")]
unsafe fn time_code(code: *const ZmidiTimeCode) -> Option<StandardTimeCode> {
    if code.is_null() {
        return None;
    }
    // SAFETY: caller contract above.
    unsafe { (*code).code() }
}

/// Reads a Standard Time Code from its 5 bytes; the counts are masked to their widths
/// (REQ-FFI-SX-008).
///
/// # Safety
/// `bytes` is null or points to 5 readable bytes; `code` is null or writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_time_code_from_bytes(
    bytes: *const u8,
    code: *mut ZmidiTimeCode,
) -> ZmidiStatus {
    if bytes.is_null() || code.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let t = StandardTimeCode::from_bytes(unsafe { bytes.cast::<[u8; 5]>().read_unaligned() });
    // SAFETY: caller contract above.
    unsafe { *code = ZmidiTimeCode::of(t) };
    ZmidiStatus::Ok
}

/// Writes the 5 bytes of a Standard Time Code (REQ-FFI-SX-008). Subframes above 99 or a time
/// that is not valid give [`ZmidiStatus::InvalidArgument`]; a blank time code is written as
/// zeroes.
///
/// # Safety
/// `code` is null or readable; `bytes` is null or points to 5 writable bytes.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_time_code_to_bytes(
    code: *const ZmidiTimeCode,
    bytes: *mut u8,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match (
        unsafe { time_code(code) }.map(|c| c.to_bytes()),
        bytes.is_null(),
    ) {
        (Some(Ok(b)), false) => {
            // SAFETY: caller contract above.
            unsafe { bytes.cast::<[u8; 5]>().write_unaligned(b) };
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// Applies the 2 bytes of a short time code to the last full value `last`: hours, minutes,
/// seconds and their flags stay (RP-013 p. 9; REQ-FFI-SX-008).
///
/// # Safety
/// `last` is null or readable; `bytes` is null or points to 2 readable bytes; `code` is null or
/// writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_time_code_with_short(
    last: *const ZmidiTimeCode,
    bytes: *const u8,
    code: *mut ZmidiTimeCode,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match (unsafe { time_code(last) }, bytes.is_null(), code.is_null()) {
        (Some(t), false, false) => {
            // SAFETY: caller contract above.
            let s = ShortTimeCode::from_bytes(unsafe { bytes.cast::<[u8; 2]>().read_unaligned() });
            // SAFETY: caller contract above.
            unsafe { *code = ZmidiTimeCode::of(t.with_short(s)) };
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// Writes the 2 bytes of the short form of a time code: the frames, the sign and the final byte
/// (RP-013 p. 9; REQ-FFI-SX-008). Frames above 29 or subframes above 99 give
/// [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `code` is null or readable; `bytes` is null or points to 2 writable bytes.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_time_code_short_bytes(
    code: *const ZmidiTimeCode,
    bytes: *mut u8,
) -> ZmidiStatus {
    if code.is_null() || bytes.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    match unsafe { (*code).short() }.to_bytes() {
        Ok(b) => {
            // SAFETY: caller contract above.
            unsafe { bytes.cast::<[u8; 2]>().write_unaligned(b) };
            ZmidiStatus::Ok
        }
        Err(_) => ZmidiStatus::InvalidArgument,
    }
}

/// The MOVE rule from {st} to {ff}: subframes 0; an {ff} value stays as it is (RP-013 p. 32;
/// REQ-FFI-SX-008).
///
/// # Safety
/// `code` is null or readable; `out` is null or writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_time_code_to_subframes_form(
    code: *const ZmidiTimeCode,
    out: *mut ZmidiTimeCode,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match (unsafe { time_code(code) }, out.is_null()) {
        (Some(t), false) => {
            // SAFETY: caller contract above.
            unsafe { *out = ZmidiTimeCode::of(t.to_subframes_form()) };
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// The MOVE rule from {ff} to {st}: e 0, v 0, d from `video_field1`, n 1; an {st} value stays
/// as it is (RP-013 p. 32; REQ-FFI-SX-008).
///
/// # Safety
/// `code` is null or readable; `out` is null or writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_time_code_to_status_form(
    code: *const ZmidiTimeCode,
    video_field1: bool,
    out: *mut ZmidiTimeCode,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    match (unsafe { time_code(code) }, out.is_null()) {
        (Some(t), false) => {
            // SAFETY: caller contract above.
            unsafe { *out = ZmidiTimeCode::of(t.to_status_form(video_field1)) };
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// Gives the short name (the name plus 20H) of a time code field 01H to 1FH (RP-013 p. 9;
/// REQ-FFI-SX-008).
///
/// # Safety
/// `short` is null or writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mmc_short_name(field: u8, short: *mut u8) -> ZmidiStatus {
    match (zmidi_sysex::short_name(field), short.is_null()) {
        (Some(s), false) => {
            // SAFETY: caller contract above.
            unsafe { *short = s };
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// Gives the time code field 01H to 1FH of a short name 21H to 3FH (RP-013 p. 9;
/// REQ-FFI-SX-008).
///
/// # Safety
/// `field` is null or writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mmc_full_name(short: u8, field: *mut u8) -> ZmidiStatus {
    match (zmidi_sysex::full_name(short), field.is_null()) {
        (Some(f), false) => {
            // SAFETY: caller contract above.
            unsafe { *field = f };
            ZmidiStatus::Ok
        }
        _ => ZmidiStatus::InvalidArgument,
    }
}

/// The header of an MMC message (RP-013 p. 1; REQ-FFI-SX-008).
#[cfg(feature = "mmc-transport")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMmcMessage {
    /// True for a response (Sub-ID#1 07H), false for commands (06H).
    pub response: bool,
    /// The destination of commands, or the source of responses (7FH is not valid there,
    /// AMB-RP-013-04).
    pub device_id: u8,
    /// The offset in the input of the command or response string.
    pub string_offset: u32,
    /// The length of the string.
    pub string_length: u32,
}

/// Reads the header of an MMC message (REQ-FFI-SX-008). Another message gives
/// [`ZmidiStatus::NoPending`]; an empty string gives [`ZmidiStatus::InvalidArgument`]. Read the
/// string with [`zmidi_mmc_next_command`] or [`zmidi_mmc_next_field`].
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `out` is null or writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mmc_read(
    bytes: *const u8,
    length: usize,
    out: *mut ZmidiMmcMessage,
) -> ZmidiStatus {
    if out.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some(b) = (unsafe { input(bytes, length) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let Ok(s) = Sysex::parse(b) else {
        return ZmidiStatus::InvalidArgument;
    };
    let u = match Universal::parse(&s) {
        Ok(Some(u)) => u,
        Ok(None) => return ZmidiStatus::NoPending,
        Err(_) => return ZmidiStatus::InvalidArgument,
    };
    let (response, device_id, string) = match MmcMessage::from_universal(&u) {
        Ok(Some(MmcMessage::Commands {
            device_id,
            commands,
        })) => (false, device_id, commands.as_bytes()),
        Ok(Some(MmcMessage::Responses { device_id, fields })) => {
            (true, device_id, fields.as_bytes())
        }
        Ok(None) => return ZmidiStatus::NoPending,
        Err(_) => return ZmidiStatus::InvalidArgument,
    };
    // SAFETY: caller contract above.
    unsafe {
        *out = ZmidiMmcMessage {
            response,
            device_id,
            string_offset: offset(b, string),
            string_length: string.len() as u32,
        }
    };
    ZmidiStatus::Ok
}

/// The kind of an MMC command (RP-013 pp. 20 to 32; REQ-FFI-SX-008).
#[cfg(feature = "mmc-transport")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiMmcCommandKind {
    /// 01H STOP.
    Stop = 0,
    /// 02H PLAY.
    Play = 1,
    /// 03H DEFERRED PLAY.
    DeferredPlay = 2,
    /// 04H FAST FORWARD.
    FastForward = 3,
    /// 05H REWIND.
    Rewind = 4,
    /// 06H RECORD STROBE.
    RecordStrobe = 5,
    /// 07H RECORD EXIT.
    RecordExit = 6,
    /// 09H PAUSE.
    Pause = 7,
    /// 0DH MMC RESET.
    Reset = 8,
    /// 40H WRITE: `data_offset` and `data_length` give the Information Field string.
    Write = 9,
    /// 44H LOCATE [I/F] with `register`, 08H to 0FH.
    LocateField = 10,
    /// 44H LOCATE [TARGET] with `target` in the {ff} form.
    LocateTarget = 11,
    /// 4CH MOVE from `source` to `destination`, 01H to 1FH.
    Move = 12,
    /// Any other command: the name and the data as offsets.
    Other = 13,
}

/// One MMC command (REQ-FFI-SX-008). The offsets are into the bytes given to the call; the
/// fields that the kind does not use are 0.
#[cfg(feature = "mmc-transport")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMmcCommand {
    /// A [`ZmidiMmcCommandKind`] value.
    pub kind: u32,
    /// The register of LocateField.
    pub register: u8,
    /// The destination of Move.
    pub destination: u8,
    /// The source of Move.
    pub source: u8,
    /// The target of LocateTarget.
    pub target: ZmidiTimeCode,
    /// The offset of the name of Other (with its 00H extension prefixes); of the 1-byte name for
    /// a reader of the other kinds.
    pub name_offset: u32,
    /// The length of the name, 1 to 3.
    pub name_length: u32,
    /// The offset of the data of Write and Other.
    pub data_offset: u32,
    /// The length of the data.
    pub data_length: u32,
}

/// The kind of an MMC Information Field (REQ-FFI-SX-008).
#[cfg(feature = "mmc-transport")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiMmcFieldKind {
    /// A time code field 01H to 1FH with its 5 data bytes in `code`.
    TimeCode = 0,
    /// The short form 21H to 3FH of the field `field`: `code` has the frames, the sign and the
    /// final byte; apply its data bytes to the last full value with
    /// [`zmidi_time_code_with_short`].
    Short = 1,
    /// Any other field: the name and the data as offsets.
    Other = 2,
}

/// One MMC Information Field (REQ-FFI-SX-008). The offsets are into the bytes given to the call.
#[cfg(feature = "mmc-transport")]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZmidiMmcField {
    /// A [`ZmidiMmcFieldKind`] value.
    pub kind: u32,
    /// The field name of TimeCode and Short (the full name 01H to 1FH).
    pub field: u8,
    /// The time code of TimeCode and Short.
    pub code: ZmidiTimeCode,
    /// The offset of the name (with its 00H extension prefixes).
    pub name_offset: u32,
    /// The length of the name, 1 to 3.
    pub name_length: u32,
    /// The offset of the data bytes, after the count byte when the name has one.
    pub data_offset: u32,
    /// The length of the data.
    pub data_length: u32,
}

#[cfg(feature = "mmc-transport")]
const NO_CODE: ZmidiTimeCode = ZmidiTimeCode {
    time: NO_TIME,
    color_frame: false,
    blank: false,
    negative: false,
    status_form: false,
    subframes: 0,
    estimated: false,
    invalid: false,
    video_field1: false,
    no_time_code: false,
};

/// The string of the program from `*offset`, or the status: [`ZmidiStatus::NoPending`] at the
/// end, [`ZmidiStatus::InvalidArgument`] for an offset past the end.
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `offset` is null or readable.
#[cfg(feature = "mmc-transport")]
unsafe fn rest<'a>(
    bytes: *const u8,
    length: usize,
    offset: *const usize,
) -> Result<(&'a [u8], usize), ZmidiStatus> {
    if offset.is_null() {
        return Err(ZmidiStatus::InvalidArgument);
    }
    // SAFETY: caller contract above.
    let (b, at) = unsafe { (input(bytes, length), *offset) };
    let b = b.ok_or(ZmidiStatus::InvalidArgument)?;
    match at {
        n if n == b.len() => Err(ZmidiStatus::NoPending),
        n if n > b.len() => Err(ZmidiStatus::InvalidArgument),
        n => Ok((b, n)),
    }
}

/// Reads the command at `*offset` of a command string (the bytes at `string_offset` of a
/// [`ZmidiMmcMessage`]) and moves `*offset` past it (RP-013 p. 5; REQ-FFI-SX-008). The end
/// gives [`ZmidiStatus::NoPending`]; a string cut short gives [`ZmidiStatus::InvalidArgument`]
/// and moves `*offset` to the end.
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `offset` is null or writable;
/// `command` is null or writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mmc_next_command(
    bytes: *const u8,
    length: usize,
    offset: *mut usize,
    command: *mut ZmidiMmcCommand,
) -> ZmidiStatus {
    if command.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (b, at) = match unsafe { rest(bytes, length, offset) } {
        Ok(r) => r,
        Err(s) => return s,
    };
    // A Universal header over the rest of the string gives its command reader.
    let u = Universal {
        real_time: true,
        device_id: 0,
        sub_id1: RealTimeMmc::COMMAND,
        sub_id2: None,
        data: &b[at..],
    };
    let Ok(Some(MmcMessage::Commands { mut commands, .. })) = MmcMessage::from_universal(&u) else {
        return ZmidiStatus::InvalidArgument;
    };
    let item = commands.next();
    let next = b.len() - commands.as_bytes().len();
    // SAFETY: caller contract above.
    unsafe { *offset = next };
    let Some(Ok(c)) = item else {
        // SAFETY: caller contract above.
        unsafe { *offset = b.len() };
        return ZmidiStatus::InvalidArgument;
    };
    let mut r = ZmidiMmcCommand {
        kind: 0,
        register: 0,
        destination: 0,
        source: 0,
        target: NO_CODE,
        name_offset: at as u32,
        name_length: 1,
        data_offset: 0,
        data_length: 0,
    };
    use ZmidiMmcCommandKind as K;
    let kind = match c {
        MmcCommand::Stop => K::Stop,
        MmcCommand::Play => K::Play,
        MmcCommand::DeferredPlay => K::DeferredPlay,
        MmcCommand::FastForward => K::FastForward,
        MmcCommand::Rewind => K::Rewind,
        MmcCommand::RecordStrobe => K::RecordStrobe,
        MmcCommand::RecordExit => K::RecordExit,
        MmcCommand::Pause => K::Pause,
        MmcCommand::Reset => K::Reset,
        MmcCommand::Write(fields) => {
            let d = fields.as_bytes();
            (r.data_offset, r.data_length) = (self::offset(b, d), d.len() as u32);
            K::Write
        }
        MmcCommand::LocateField { register } => {
            r.register = register;
            K::LocateField
        }
        MmcCommand::LocateTarget(t) => {
            r.target = ZmidiTimeCode::of(t);
            K::LocateTarget
        }
        MmcCommand::Move {
            destination,
            source,
        } => {
            (r.destination, r.source) = (destination, source);
            K::Move
        }
        MmcCommand::Other { name, data } => {
            (r.name_offset, r.name_length) = (self::offset(b, name), name.len() as u32);
            (r.data_offset, r.data_length) = (self::offset(b, data), data.len() as u32);
            K::Other
        }
    };
    r.kind = kind as u32;
    // SAFETY: caller contract above.
    unsafe { *command = r };
    ZmidiStatus::Ok
}

/// The Sub-ID#1 of the MMC command message, for the command reader over a string.
#[cfg(feature = "mmc-transport")]
struct RealTimeMmc;

#[cfg(feature = "mmc-transport")]
impl RealTimeMmc {
    const COMMAND: u8 = zmidi_sysex::RealTimeId::MmcCommand.into_bits();
    const RESPONSE: u8 = zmidi_sysex::RealTimeId::MmcResponse.into_bits();
}

/// Reads the Information Field at `*offset` of a response string or of the data of WRITE, and
/// moves `*offset` past it, with the end and error rules of [`zmidi_mmc_next_command`]
/// (REQ-FFI-SX-008).
///
/// # Safety
/// `bytes` is null or points to `length` readable bytes; `offset` is null or writable; `field`
/// is null or writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mmc_next_field(
    bytes: *const u8,
    length: usize,
    offset: *mut usize,
    field: *mut ZmidiMmcField,
) -> ZmidiStatus {
    if field.is_null() {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let (b, at) = match unsafe { rest(bytes, length, offset) } {
        Ok(r) => r,
        Err(s) => return s,
    };
    let mut fields = InfoFields::new(&b[at..]);
    let item = fields.next();
    let next = b.len() - fields.as_bytes().len();
    // SAFETY: caller contract above.
    unsafe { *offset = next };
    let Some(Ok(f)) = item else {
        // SAFETY: caller contract above.
        unsafe { *offset = b.len() };
        return ZmidiStatus::InvalidArgument;
    };
    // The name is 1 byte for a typed field; the data is up to the next field.
    let mut r = ZmidiMmcField {
        kind: 0,
        field: 0,
        code: NO_CODE,
        name_offset: at as u32,
        name_length: 1,
        data_offset: at as u32 + 1,
        data_length: (next - at - 1) as u32,
    };
    let kind = match f {
        InfoField::TimeCode { field, code } => {
            (r.field, r.code) = (field, ZmidiTimeCode::of(code));
            ZmidiMmcFieldKind::TimeCode
        }
        InfoField::Short { field, code } => {
            r.field = field;
            let (status_form, subframes, st) = ZmidiTimeCode::final_of(code.final_byte);
            r.code = ZmidiTimeCode {
                time: ZmidiMtcTime {
                    frames: code.frames,
                    ..NO_TIME
                },
                negative: code.negative,
                status_form,
                subframes,
                estimated: st.estimated,
                invalid: st.invalid,
                video_field1: st.video_field1,
                no_time_code: st.no_time_code,
                ..NO_CODE
            };
            ZmidiMmcFieldKind::Short
        }
        InfoField::Other { name, data } => {
            (r.name_offset, r.name_length) = (self::offset(b, name), name.len() as u32);
            (r.data_offset, r.data_length) = (self::offset(b, data), data.len() as u32);
            ZmidiMmcFieldKind::Other
        }
    };
    r.kind = kind as u32;
    // SAFETY: caller contract above.
    unsafe { *field = r };
    ZmidiStatus::Ok
}

/// A part of the data bytes of a writer, or `None` when the offsets are outside them.
#[cfg(feature = "mmc-transport")]
fn slice(data: &[u8], offset: u32, length: u32) -> Option<&[u8]> {
    let start = offset as usize;
    data.get(start..start.checked_add(length as usize)?)
}

#[cfg(feature = "mmc-transport")]
impl ZmidiMmcCommand {
    fn command<'a>(&self, data: &'a [u8]) -> Option<MmcCommand<'a>> {
        use ZmidiMmcCommandKind as K;
        Some(match self.kind {
            k if k == K::Stop as u32 => MmcCommand::Stop,
            k if k == K::Play as u32 => MmcCommand::Play,
            k if k == K::DeferredPlay as u32 => MmcCommand::DeferredPlay,
            k if k == K::FastForward as u32 => MmcCommand::FastForward,
            k if k == K::Rewind as u32 => MmcCommand::Rewind,
            k if k == K::RecordStrobe as u32 => MmcCommand::RecordStrobe,
            k if k == K::RecordExit as u32 => MmcCommand::RecordExit,
            k if k == K::Pause as u32 => MmcCommand::Pause,
            k if k == K::Reset as u32 => MmcCommand::Reset,
            k if k == K::Write as u32 => MmcCommand::Write(InfoFields::new(slice(
                data,
                self.data_offset,
                self.data_length,
            )?)),
            k if k == K::LocateField as u32 => MmcCommand::LocateField {
                register: self.register,
            },
            k if k == K::LocateTarget as u32 => MmcCommand::LocateTarget(self.target.code()?),
            k if k == K::Move as u32 => MmcCommand::Move {
                destination: self.destination,
                source: self.source,
            },
            k if k == K::Other as u32 => MmcCommand::Other {
                name: slice(data, self.name_offset, self.name_length)?,
                data: slice(data, self.data_offset, self.data_length)?,
            },
            _ => return None,
        })
    }
}

#[cfg(feature = "mmc-transport")]
impl ZmidiMmcField {
    fn field<'a>(&self, data: &'a [u8]) -> Option<InfoField<'a>> {
        Some(match self.kind {
            k if k == ZmidiMmcFieldKind::TimeCode as u32 => InfoField::TimeCode {
                field: self.field,
                code: self.code.code()?,
            },
            k if k == ZmidiMmcFieldKind::Short as u32 => InfoField::Short {
                field: self.field,
                code: self.code.short(),
            },
            k if k == ZmidiMmcFieldKind::Other as u32 => InfoField::Other {
                name: slice(data, self.name_offset, self.name_length)?,
                data: slice(data, self.data_offset, self.data_length)?,
            },
            _ => return None,
        })
    }
}

/// Appends the bytes of each item to `string` (at most [`MAX_STRING`] bytes), with a small stack
/// buffer for one item.
#[cfg(feature = "mmc-transport")]
fn build<T>(
    items: &[T],
    string: &mut [u8; MAX_STRING],
    mut one: impl for<'t> FnMut(&T, &'t mut [u8]) -> Result<&'t [u8], SysexError>,
) -> Result<usize, SysexError> {
    let mut n = 0;
    let mut tmp = [0u8; MAX_STRING + 8];
    for item in items {
        let b = one(item, &mut tmp)?;
        let end = n + b.len();
        if end > MAX_STRING {
            return Err(SysexError::Length);
        }
        string[n..end].copy_from_slice(b);
        n = end;
    }
    if n == 0 {
        return Err(SysexError::Length);
    }
    Ok(n)
}

/// The commands and the data of the program.
///
/// # Safety
/// `items` is null or points to `count` readable items; `data` is null or points to
/// `data_length` readable bytes.
#[cfg(feature = "mmc-transport")]
unsafe fn items<'a, T>(
    items: *const T,
    count: usize,
    data: *const u8,
    data_length: usize,
) -> Option<(&'a [T], &'a [u8])> {
    if items.is_null() || count > MAX_STRING {
        return None;
    }
    // SAFETY: caller contract above.
    unsafe {
        Some((
            core::slice::from_raw_parts(items, count),
            input(data, data_length)?,
        ))
    }
}

/// Writes an MMC command message to `device_id` from `count` commands (RP-013 p. 1;
/// REQ-FFI-SX-008). The offsets of Write and Other point into `data`. A string above 48 bytes,
/// an empty string, a register outside 08H to 0FH, a LOCATE target in the {st} form or a MOVE
/// name outside 01H to 1FH give [`ZmidiStatus::InvalidArgument`].
///
/// # Safety
/// `commands` is null or points to `count` readable commands; `data` is null or points to
/// `data_length` readable bytes; `out` is null or points to `capacity` writable bytes; `length`
/// is null or writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_mmc_write_commands(
    device_id: u8,
    commands: *const ZmidiMmcCommand,
    count: usize,
    data: *const u8,
    data_length: usize,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some((commands, data)) = (unsafe { items(commands, count, data, data_length) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let mut string = [0u8; MAX_STRING];
    let n = build(commands, &mut string, |c, tmp| {
        let c = c.command(data).ok_or(SysexError::OutOfRange)?;
        // One command in a message of its own: the string is after the 4 header bytes.
        let len = zmidi_sysex::write_commands(tmp, 0x7F, &[c])?;
        Ok(&tmp[4..len - 1])
    });
    let n = match n {
        Ok(n) => n,
        Err(_) => return ZmidiStatus::InvalidArgument,
    };
    let u = Universal {
        real_time: true,
        device_id,
        sub_id1: RealTimeMmc::COMMAND,
        sub_id2: None,
        data: &string[..n],
    };
    // SAFETY: caller contract above.
    unsafe { write_into(out, capacity, length, |o| u.write(o)) }
}

/// Writes an MMC response message from `device_id` (00H to 7EH, AMB-RP-013-04) with `count`
/// Information Fields (RP-013 p. 1; REQ-FFI-SX-008). The offsets of Other point into `data`.
/// SELECTED TIME CODE needs the {st} form and no sign; GP0 needs the {ff} form.
///
/// # Safety
/// `fields` is null or points to `count` readable fields; `data` is null or points to
/// `data_length` readable bytes; `out` is null or points to `capacity` writable bytes; `length`
/// is null or writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn zmidi_mmc_write_responses(
    device_id: u8,
    fields: *const ZmidiMmcField,
    count: usize,
    data: *const u8,
    data_length: usize,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    if device_id > 0x7E {
        return ZmidiStatus::InvalidArgument;
    }
    // SAFETY: caller contract above.
    let Some((fields, data)) = (unsafe { items(fields, count, data, data_length) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let mut string = [0u8; MAX_STRING];
    let n = build(fields, &mut string, |f, tmp| {
        let f = f.field(data).ok_or(SysexError::OutOfRange)?;
        let len = zmidi_sysex::write_info_fields(tmp, &[f])?;
        Ok(&tmp[..len])
    });
    let n = match n {
        Ok(n) => n,
        Err(_) => return ZmidiStatus::InvalidArgument,
    };
    let u = Universal {
        real_time: true,
        device_id,
        sub_id1: RealTimeMmc::RESPONSE,
        sub_id2: None,
        data: &string[..n],
    };
    // SAFETY: caller contract above.
    unsafe { write_into(out, capacity, length, |o| u.write(o)) }
}

/// Writes the name and data pairs of `count` Information Fields, for the data of a WRITE
/// command, with the rules of [`zmidi_mmc_write_responses`] (REQ-FFI-SX-008).
///
/// # Safety
/// `fields` is null or points to `count` readable fields; `data` is null or points to
/// `data_length` readable bytes; `out` is null or points to `capacity` writable bytes; `length`
/// is null or writable.
#[cfg(feature = "mmc-transport")]
#[no_mangle]
pub unsafe extern "C" fn zmidi_mmc_write_fields(
    fields: *const ZmidiMmcField,
    count: usize,
    data: *const u8,
    data_length: usize,
    out: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> ZmidiStatus {
    // SAFETY: caller contract above.
    let Some((fields, data)) = (unsafe { items(fields, count, data, data_length) }) else {
        return ZmidiStatus::InvalidArgument;
    };
    let mut string = [0u8; MAX_STRING];
    let n = build(fields, &mut string, |f, tmp| {
        let f = f.field(data).ok_or(SysexError::OutOfRange)?;
        let len = zmidi_sysex::write_info_fields(tmp, &[f])?;
        Ok(&tmp[..len])
    });
    let n = match n {
        Ok(n) => n,
        Err(_) => return ZmidiStatus::InvalidArgument,
    };
    // SAFETY: caller contract above.
    unsafe {
        write_into(out, capacity, length, |o| match o.get_mut(..n) {
            Some(o) => {
                o.copy_from_slice(&string[..n]);
                Ok(n)
            }
            None => Err(SysexError::BufferTooSmall { needed: n }),
        })
    }
}
