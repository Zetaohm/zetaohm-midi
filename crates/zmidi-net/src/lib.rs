//! Network MIDI 2.0 (UDP), M2-124-UM v1.0.1: the UDP payload format, the Host and Client
//! Sessions, UMP Data with sequence numbers, Forward Error Correction and retransmit, and the
//! DNS-SD record data (DEC-012).
//!
//! - The crate is `no_std` and needs no `alloc` (DEC-002). It is sans-IO: the caller owns the
//!   UDP socket, the clock and the random numbers.
//! - Authentication (SHA-256) is behind the Cargo feature `auth`, off by default (DEC-013).
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![no_std]

#[cfg(feature = "auth")]
pub mod auth;
pub mod command;
pub mod dns;
pub mod endpoint;
pub mod handshake;
pub mod identity;
pub mod packet;
pub mod session;
pub mod stream;

pub use command::{ByeReason, Capabilities, SessionState};
pub use endpoint::{
    check_peer, Client, ClientConfig, Diagnostic, EndpointError, Event, EventKind, Host,
    HostConfig, Polled, Received,
};
pub use handshake::{Credentials, HandshakeConfig, HostPolicy, Method, NoCredentials, PolicyKind};
pub use identity::{EndpointName, Identity, ProductInstanceId};
pub use packet::MaxPayload;
pub use session::{CloseCause, SessionConfig, SessionEvent, SessionRef};
pub use stream::{Delivery, StreamConfig};
