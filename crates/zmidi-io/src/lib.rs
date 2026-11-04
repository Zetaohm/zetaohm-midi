//! MIDI 2.0 I/O: UMP Endpoint behaviour and, later, the transports (DEC-003, DEC-004).
//!
//! - The crate is `no_std` and its core API needs no `alloc` (DEC-002).
//! - Protocol logic is sans-IO and synchronous; async code belongs to transport adapters only
//!   (DEC-003).
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![no_std]

pub mod byte_stream;
#[cfg(feature = "channel-mode")]
pub mod channel_mode;
pub mod endpoint;
pub mod jitter_reduction;
#[cfg(feature = "messages")]
pub mod messages;
pub mod router;
pub mod send_order;

pub use byte_stream::{
    ByteStreamError, BytesToUmp, BytesToUmpCore, Midi1Scheduler, Pushed, UmpToBytes,
};
#[cfg(feature = "channel-mode")]
pub use channel_mode::{ModeAction, ModeConfig, ModeState, ReceiveMode};
pub use endpoint::{
    Endpoint, EndpointDescription, EndpointError, FunctionBlock, Replies, StreamConfiguration,
};
pub use jitter_reduction::{Disposition, JrError, JrReceiver, JrSender};
#[cfg(feature = "messages")]
pub use messages::{MessageError, Midi1Event, Midi1Message, Midi1Reader, Midi1Writer};
pub use router::{blocks_for_group, ci_address, route, BlocksForGroup, CiAddress, Destination};
pub use send_order::{end_mds_early, SendOrder, SendOrderError};

#[cfg(test)]
mod tests {
    #[test]
    fn channel_mode_is_in_the_default_features() {
        // REQ-MODE-011: the mode state is on by default (`full` turns on `channel-mode`).
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("channel-mode = []"), "{manifest}");
        assert!(
            manifest.contains("full = [\"midi2-protocol\", \"messages\", \"channel-mode\"]"),
            "{manifest}"
        );
    }

    #[test]
    fn messages_is_in_the_default_features() {
        // REQ-MSG-006: the typed byte reader and writer are on by default (`full` turns on
        // `messages`).
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("default = [\"full\"]"), "{manifest}");
        assert!(
            manifest.contains("full = [\"midi2-protocol\", \"messages\", \"channel-mode\"]"),
            "{manifest}"
        );
    }
}
