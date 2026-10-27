//! BLE-MIDI (Specification for MIDI over Bluetooth Low Energy v1.0): the GATT data and the packet
//! codec, with UMP on the application side (DEC-016).
//!
//! - The crate is `no_std` and needs no `alloc` (DEC-002). It is synchronous and sans-IO
//!   (DEC-004): the caller's BLE host moves the bytes of the MIDI Data I/O characteristic. It has
//!   no BLE stack dependency; the `trouble-host` adapter is a separate crate.
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![no_std]

pub mod gatt;
pub mod reader;
pub mod timestamp;
pub mod writer;

/// The UMP types of the public API (`Ump`, `Group`, `StreamProtocol`) come from this crate.
pub use zmidi_ump;

pub use reader::{PacketReader, ReadError, Received, ReceivedWords};
pub use timestamp::Timestamp;
pub use writer::{PacketWriter, TimedUmp, WriteError, MIN_ATT_MTU};
