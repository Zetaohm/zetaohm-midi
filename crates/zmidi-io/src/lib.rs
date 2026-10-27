//! MIDI 2.0 I/O: UMP Endpoint behaviour and, later, the transports (DEC-003, DEC-004).
//!
//! - The crate is `no_std` and its core API needs no `alloc` (DEC-002).
//! - Protocol logic is sans-IO and synchronous; async code belongs to transport adapters only
//!   (DEC-003).
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![no_std]

pub mod byte_stream;
pub mod endpoint;
pub mod jitter_reduction;
pub mod router;
pub mod send_order;

pub use byte_stream::{
    ByteStreamError, BytesToUmp, BytesToUmpCore, Midi1Scheduler, Pushed, UmpToBytes,
};
pub use endpoint::{
    Endpoint, EndpointDescription, EndpointError, FunctionBlock, Replies, StreamConfiguration,
};
pub use jitter_reduction::{Disposition, JrError, JrReceiver, JrSender};
pub use router::{blocks_for_group, ci_address, route, BlocksForGroup, CiAddress, Destination};
pub use send_order::{end_mds_early, SendOrder, SendOrderError};
