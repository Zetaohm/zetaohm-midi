//! MIDI Clip File (M2-116-U v1.0) and Standard MIDI File 1.0 (RP-001, DEC-021): readers and writers.
//!
//! - The MIDI Clip File modules need the Cargo feature `clip` (on by default through `full`). Without
//!   it the crate has only the Standard MIDI File modules and links no MIDI 2.0 code (DEC-022).
//! - The crate is `no_std` and its core API needs no `alloc` (DEC-002): the reader iterates a
//!   borrowed byte slice, and the writer fills a caller buffer.
//! - The Standard MIDI File reader and writer give and take MIDI 1.0 channel messages also as UMP
//!   Message Type 0x2 words (DEC-021). The conversion between SMF and the MIDI Clip File is not
//!   in scope yet.
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![no_std]

#[cfg(feature = "clip")]
pub mod clip_format;
#[cfg(feature = "clip")]
pub mod clip_reader;
#[cfg(feature = "clip")]
pub mod clip_writer;
#[cfg(test)]
mod smf_examples;
pub mod smf_format;
pub mod smf_reader;
pub mod smf_writer;

#[cfg(feature = "clip")]
pub use clip_format::{word_from_bytes, word_to_bytes, EXTENSION, FILE_HEADER, MAX_DELTA_TICKS};
#[cfg(feature = "clip")]
pub use clip_reader::{ClipError, ClipEvent, ClipReader, ClipSection};
#[cfg(feature = "clip")]
pub use clip_writer::{ClipConfig, ClipWriteError, ClipWriter, ProfileDestination, ProfileOn};
pub use smf_format::{
    is_known_meta_kind, read_vlq, write_vlq, ChannelMessage, Division, FileFormat, KeySignature,
    ManufacturerId, MetaEvent, SmfError, SmpteFormat, SmpteOffset, TextKind, TimeSignature,
    DEFAULT_TEMPO, MAX_VLQ, MTHD, MTRK,
};
pub use smf_reader::{Event, SmfReader, SysExKind, TrackEvent, TrackReader, Tracks};
pub use smf_writer::{SmfWriteError, SmfWriter};

#[cfg(test)]
mod feature_tests {
    #[test]
    fn clip_is_in_the_default_features() {
        // REQ-CLIP-FEAT-001: the Clip File modules are on by default (`full` turns on `clip`).
        let manifest = include_str!("../Cargo.toml");
        assert!(manifest.contains("default = [\"full\"]"), "{manifest}");
        assert!(
            manifest.contains("full = [\"clip\", \"zmidi-ump/full\"]"),
            "{manifest}"
        );
    }
}
