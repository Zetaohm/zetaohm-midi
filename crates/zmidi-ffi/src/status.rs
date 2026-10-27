//! The status that every function of the C ABI returns (DEC-019, REQ-FFI-ABI-001), and the ABI
//! version (REQ-FFI-ABI-003).

/// The result of a function of the C ABI. New values go at the end only; a value never changes.
#[repr(C)]
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZmidiStatus {
    /// The function did its work.
    Ok = 0,
    /// A pointer is null, or a value is out of its range. No state changed.
    InvalidArgument = 1,
    /// The output buffer of the program is too small. No state changed.
    BufferTooSmall = 2,
    /// A reply call has no pending event to answer, or an event call has no event of its kind.
    NoPending = 3,
    /// The library refused the request (for example a request to an unknown peer).
    Rejected = 4,
    /// This build has no support for the request.
    NotSupported = 5,
}

/// Major version of the C ABI. It changes when a type, a storage size or a function changes.
pub const ZMIDI_ABI_MAJOR: u32 = 0;
/// Minor version of the C ABI. It changes when a function or a status value is added.
pub const ZMIDI_ABI_MINOR: u32 = 7;

/// Gives the ABI version, `major << 16 | minor`. A binding compares it at load time with the
/// version of its copy of the header.
#[no_mangle]
pub extern "C" fn zmidi_abi_version() -> u32 {
    ZMIDI_ABI_MAJOR << 16 | ZMIDI_ABI_MINOR
}
