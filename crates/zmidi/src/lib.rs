//! Complete MIDI 2.0 implementation: one crate that re-exports the zetaohm-midi crates.
//!
//! `no_std` and no alloc, as every crate of the workspace (DEC-002).

#![no_std]

pub use zmidi_ble as ble;
pub use zmidi_ci as ci;
pub use zmidi_ci_pe as ci_pe;
pub use zmidi_file as file;
pub use zmidi_io as io;
pub use zmidi_net as net;
pub use zmidi_ump as ump;
pub use zmidi_usb as usb;
