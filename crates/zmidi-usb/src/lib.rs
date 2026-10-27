//! USB-MIDI (USB Device Class Definition for MIDI Devices v2.0 and v1.0): UMP over USB packets,
//! the USB-MIDI 1.0 Event Packets, and the MIDI Streaming descriptors (DEC-014).
//!
//! - The crate is `no_std` and needs no `alloc` (DEC-002). It is synchronous and sans-IO
//!   (DEC-004): the caller's USB stack moves the bytes. It has no USB stack dependency; the
//!   adapters for `embassy-usb` and `usb-device` are separate crates.
//! - Cargo feature `alt0` (on by default, DEC-017): the USB-MIDI 1.0 data path of Alternate
//!   Setting 0 in [`MidiLink`]. With it off, Alternate Setting 0 has no Jack and no endpoint and
//!   the link moves no data there (DEC-014 addendum, REQ-USB-GEN-014). A crate that depends on
//!   this one with `default-features = false` names `features = ["alt0"]` to keep it.
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![no_std]

pub mod ac;
pub mod constants;
pub mod device;
pub mod event_packet;
pub mod fixed;
pub mod gtb;
pub mod host;
pub mod link;
pub mod ms1;
pub mod ms2;
pub mod request;
pub mod ump_usb;
pub mod walk;

/// The UMP types of the public API (`Ump`, `Group`, `StreamProtocol`, `UmpError`) come from
/// this crate.
pub use zmidi_ump;

// The adapter-facing set. All other items stay under their module paths. The module-local
// length constants (`ms1::HEADER_LEN`, `gtb::HEADER_LEN`, ...) are not re-exported, because
// their names repeat with different values.
pub use constants::{EndpointDirection, GtbType, MidiProtocol};
pub use device::{
    function_len, gtb_set, write_function, write_function_to, CableCounts, DescriptorSink,
    DeviceError, EmitError, EndpointFields, FunctionConfig, InterfaceFields, Midi1Setting,
    Midi2Setting, SettingEndpoint,
};
pub use gtb::{GroupMask, GroupTerminalBlock, GtbList};
pub use host::{read_configuration, FunctionInfo};
pub use link::{LinkConfig, LinkError, LinkEvent, MidiLink, Received, SettingNumber};
pub use ms2::{BusSpeed, EndpointConfig};
pub use request::{handle_gtb_request, handle_gtb_setup, GtbResponse, RequestError, SetupPacket};
