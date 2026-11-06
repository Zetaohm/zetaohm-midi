//! MIDI 1.0 Universal System Exclusive messages (DEC-022): the header, the Manufacturer ID and
//! Device ID rules, and the parts Device Inquiry (`inquiry`), General MIDI (`gm`), Device Control
//! (`device-control`) and MIDI Tuning (`tuning`).
//!
//! The input is one complete System Exclusive message as bytes, with or without F0H and F7H, so
//! the crate works for DIN bytes, USB-MIDI 1.0, BLE-MIDI and UMP System Exclusive (7-bit) data.
//! No alloc (DEC-002), no I/O (DEC-003). Never panics.

#![no_std]

pub mod message;
pub mod universal;

pub use message::Message;
pub use universal::{
    addressed, has_sub_id2, write_manufacturer, NonRealTimeId, RealTimeId, Sysex, SysexError,
    SysexId, Universal, ALL_DEVICES,
};

#[cfg(feature = "inquiry")]
pub mod inquiry;
#[cfg(feature = "inquiry")]
pub use inquiry::{respond, Identity, InquiryMessage};

#[cfg(feature = "gm")]
pub mod gm;
#[cfg(feature = "gm")]
pub use gm::{GeneralMidi, GmCommand};

#[cfg(feature = "device-control")]
pub mod device_control;
#[cfg(feature = "device-control")]
pub use device_control::{
    coarse_tuning_semitones, fine_tuning_offset, DeviceControl, DeviceControlValue,
};

#[cfg(feature = "tuning")]
pub mod tuning;
#[cfg(feature = "tuning")]
pub use tuning::{
    checksum, write_dump_request, write_key_dump, write_note_change, write_scale_dump,
    write_scale_octave, FrequencyWord, FrequencyWords, NoteChanges, ScaleOffsets, TuningMessage,
};

#[cfg(test)]
mod tests {
    #[test]
    fn inquiry_is_in_the_default_features() {
        // REQ-INQ-004: the Identity messages are on by default.
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("inquiry = []"), "{manifest}");
        assert!(
            manifest.contains("full = [\"inquiry\", \"gm\", \"device-control\", \"tuning\"]"),
            "{manifest}"
        );
        assert!(manifest.contains("default = [\"full\"]"), "{manifest}");
    }

    #[test]
    fn gm_is_in_the_default_features() {
        // REQ-GM-002: the General MIDI System messages are on by default.
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("gm = []"), "{manifest}");
        assert!(
            manifest.contains("full = [\"inquiry\", \"gm\", \"device-control\", \"tuning\"]"),
            "{manifest}"
        );
    }

    #[test]
    fn device_control_is_in_the_default_features() {
        // REQ-DC-004: the Device Control messages are on by default.
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("device-control = []"), "{manifest}");
        assert!(
            manifest.contains("full = [\"inquiry\", \"gm\", \"device-control\", \"tuning\"]"),
            "{manifest}"
        );
    }

    #[test]
    fn tuning_is_in_the_default_features() {
        // REQ-TUN-008: the MIDI Tuning messages are on by default.
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("tuning = []"), "{manifest}");
        assert!(
            manifest.contains("full = [\"inquiry\", \"gm\", \"device-control\", \"tuning\"]"),
            "{manifest}"
        );
    }
}
