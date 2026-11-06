//! MIDI 2.0 I/O: UMP Endpoint behaviour and, later, the transports (DEC-003, DEC-004).
//!
//! - The crate is `no_std` and its core API needs no `alloc` (DEC-002).
//! - Protocol logic is sans-IO and synchronous; async code belongs to transport adapters only
//!   (DEC-003).
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![no_std]

#[cfg(feature = "active-sensing")]
pub mod active_sensing;
pub mod byte_stream;
#[cfg(feature = "channel-mode")]
pub mod channel_mode;
#[cfg(feature = "controllers")]
pub mod controllers;
pub mod endpoint;
#[cfg(feature = "gm")]
pub mod gm;
pub mod jitter_reduction;
#[cfg(feature = "messages")]
pub mod messages;
#[cfg(feature = "mpe")]
pub mod mpe;
#[cfg(feature = "parameters")]
pub mod parameters;
pub mod router;
pub mod send_order;
#[cfg(feature = "sync")]
pub mod sync;

#[cfg(feature = "active-sensing")]
pub use active_sensing::{
    ActiveSensingMonitor, ActiveSensingSender, ACTIVE_SENSING_INTERVAL_US,
    ACTIVE_SENSING_TIMEOUT_US,
};
pub use byte_stream::{
    ByteStreamError, BytesToUmp, BytesToUmpCore, Midi1Scheduler, Pushed, UmpToBytes,
};
#[cfg(feature = "channel-mode")]
pub use channel_mode::{ModeAction, ModeConfig, ModeState, ReceiveMode};
#[cfg(feature = "controllers")]
pub use controllers::{
    bank_program_messages, controller14_messages, reset_value, ControllerEvent, ControllerState,
};
pub use endpoint::{
    Endpoint, EndpointDescription, EndpointError, FunctionBlock, Replies, StreamConfiguration,
};
#[cfg(feature = "gm")]
pub use gm::{
    gm2_controller_default, gm_reset_controller_state, rhythm_ignores_controller,
    rhythm_ignores_message, rhythm_ignores_parameter, ChannelKind, GmConfig, GmLevel, GmReceiver,
    GmReset, GM2_CHANNEL_COARSE_TUNING, GM2_CHANNEL_FINE_TUNING, GM2_CHANNEL_PRESSURE,
    GM2_MASTER_COARSE_TUNING, GM2_MASTER_FINE_TUNING, GM2_MASTER_VOLUME, GM2_MELODY_BANK,
    GM2_MODULATION_DEPTH_RANGE, GM2_PITCH_BEND, GM2_PITCH_BEND_SENSITIVITY, GM2_PROGRAM_DEFAULT,
    GM2_RHYTHM_BANK, GM2_SCALE_TUNING_OFFSET,
};
pub use jitter_reduction::{Disposition, JrError, JrReceiver, JrSender};
#[cfg(feature = "messages")]
pub use messages::{MessageError, Midi1Event, Midi1Message, Midi1Reader, Midi1Writer};
#[cfg(feature = "mpe")]
pub use mpe::{
    mcm_messages, mode_message, mpe_class, note_messages, pitch_bend_value, pitch_offset,
    sensitivity_messages, zone_sensitivity_messages, MpeClass, MpeEvent, MpeLayout, MpeMessage,
    MpeNoteAllocator, MpeReceiver, MpeRole, MpeSensitivity, MpeZone, PITCH_UNITS_PER_SEMITONE,
};
#[cfg(feature = "parameters")]
pub use parameters::{
    change_message, select_messages, value_messages, Parameter, ParameterChange, ParameterEvent,
    ParameterState,
};
pub use router::{blocks_for_group, ci_address, route, BlocksForGroup, CiAddress, Destination};
pub use send_order::{end_mds_early, SendOrder, SendOrderError};
#[cfg(feature = "sync")]
pub use sync::{SyncCommand, SyncEvent, SyncReceiver, SyncRun};

#[cfg(test)]
mod tests {
    #[test]
    fn channel_mode_is_in_the_default_features() {
        // REQ-MODE-011: the mode state is on by default (`full` turns on `channel-mode`).
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("channel-mode = []"), "{manifest}");
        assert!(
            manifest.contains("full = [\"midi2-protocol\", \"messages\", \"channel-mode\", \"controllers\", \"parameters\", \"sync\", \"active-sensing\", \"mpe\", \"gm\"]"),
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
            manifest.contains("full = [\"midi2-protocol\", \"messages\", \"channel-mode\", \"controllers\", \"parameters\", \"sync\", \"active-sensing\", \"mpe\", \"gm\"]"),
            "{manifest}"
        );
    }

    #[test]
    fn controllers_is_in_the_default_features() {
        // REQ-CTRL-010: the controller state is on by default (`full` turns on `controllers`).
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("controllers = []"), "{manifest}");
        assert!(
            manifest.contains("full = [\"midi2-protocol\", \"messages\", \"channel-mode\", \"controllers\", \"parameters\", \"sync\", \"active-sensing\", \"mpe\", \"gm\"]"),
            "{manifest}"
        );
    }

    #[test]
    fn parameters_is_in_the_default_features() {
        // REQ-PARAM-007: the parameter receiver is on by default (`full` turns on `parameters`).
        let manifest = include_str!("../Cargo.toml");
        assert!(
            manifest.contains("parameters = [\"zmidi-ump/channel-voice1\"]"),
            "{manifest}"
        );
        assert!(
            manifest.contains("full = [\"midi2-protocol\", \"messages\", \"channel-mode\", \"controllers\", \"parameters\", \"sync\", \"active-sensing\", \"mpe\", \"gm\"]"),
            "{manifest}"
        );
    }

    #[test]
    fn sync_is_in_the_default_features() {
        // REQ-SYNC-006: the sync receiver is on by default (`full` turns on `sync`).
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("sync = []"), "{manifest}");
        assert!(manifest.contains("full = [\"midi2-protocol\", \"messages\", \"channel-mode\", \"controllers\", \"parameters\", \"sync\", \"active-sensing\", \"mpe\", \"gm\"]"), "{manifest}");
    }

    #[test]
    fn active_sensing_is_in_the_default_features() {
        // REQ-AS-003: the Active Sensing monitor and sender are on by default (`full` turns on
        // `active-sensing`).
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("active-sensing = []"), "{manifest}");
        assert!(manifest.contains("full = [\"midi2-protocol\", \"messages\", \"channel-mode\", \"controllers\", \"parameters\", \"sync\", \"active-sensing\", \"mpe\", \"gm\"]"), "{manifest}");
    }

    #[test]
    fn mpe_is_in_the_default_features() {
        // REQ-MPE-011: MPE is on by default (`full` turns on `mpe`, which needs `parameters`).
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("mpe = [\"parameters\"]"), "{manifest}");
        assert!(manifest.contains("full = [\"midi2-protocol\", \"messages\", \"channel-mode\", \"controllers\", \"parameters\", \"sync\", \"active-sensing\", \"mpe\", \"gm\"]"), "{manifest}");
    }

    #[test]
    fn gm_is_in_the_default_features() {
        // REQ-GMR-010: the GM receiver is on by default and needs channel-mode, controllers and
        // parameters.
        let manifest = include_str!("../Cargo.toml");
        assert!(
            manifest.contains("gm = [\"channel-mode\", \"controllers\", \"parameters\"]"),
            "{manifest}"
        );
        assert!(manifest.contains("\"mpe\", \"gm\"]"), "{manifest}");
    }
}
