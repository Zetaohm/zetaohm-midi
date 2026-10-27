//! MIDI Clip File (M2-116-U v1.0) and Standard MIDI File 1.0 (RP-001, DEC-021): readers and writers.
//!
//! - The crate is `no_std` and its core API needs no `alloc` (DEC-002): the reader iterates a
//!   borrowed byte slice, and the writer fills a caller buffer.
//! - The Standard MIDI File reader and writer give and take MIDI 1.0 channel messages also as UMP
//!   Message Type 0x2 words (DEC-021). The conversion between SMF and the MIDI Clip File is not
//!   in scope yet.
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![no_std]

pub mod clip_format;
pub mod clip_reader;
pub mod clip_writer;
#[cfg(test)]
mod smf_examples;
pub mod smf_format;
pub mod smf_reader;
pub mod smf_writer;

pub use clip_format::{word_from_bytes, word_to_bytes, EXTENSION, FILE_HEADER, MAX_DELTA_TICKS};
pub use clip_reader::{ClipError, ClipEvent, ClipReader, ClipSection};
pub use clip_writer::{ClipConfig, ClipWriteError, ClipWriter, ProfileDestination, ProfileOn};
pub use smf_format::{
    is_known_meta_kind, read_vlq, write_vlq, ChannelMessage, Division, FileFormat, KeySignature,
    ManufacturerId, MetaEvent, SmfError, SmpteFormat, SmpteOffset, TextKind, TimeSignature,
    DEFAULT_TEMPO, MAX_VLQ, MTHD, MTRK,
};
pub use smf_reader::{Event, SmfReader, SysExKind, TrackEvent, TrackReader, Tracks};
pub use smf_writer::{SmfWriteError, SmfWriter};
