//! Complete MIDI 2.0 implementation: one crate that re-exports the zetaohm-midi crates.
//!
//! `no_std` and no alloc, as every crate of the workspace (DEC-002). Each module is a Cargo feature,
//! on by default (`full`); `ump` is always present.

#![no_std]

#[cfg(feature = "ble")]
pub use zmidi_ble as ble;
#[cfg(feature = "ci")]
pub use zmidi_ci as ci;
#[cfg(feature = "ci-pe")]
pub use zmidi_ci_pe as ci_pe;
#[cfg(feature = "file")]
pub use zmidi_file as file;
#[cfg(feature = "io")]
pub use zmidi_io as io;
#[cfg(feature = "net")]
pub use zmidi_net as net;
pub use zmidi_ump as ump;
#[cfg(feature = "usb")]
pub use zmidi_usb as usb;
