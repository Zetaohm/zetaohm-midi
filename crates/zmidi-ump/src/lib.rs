//! Universal MIDI Packet (UMP) messages, as defined in M2-104-UM v1.1.2.
//!
//! - Each message is a `bitfield-struct` type with fields in specification order (DEC-UMP-001).
//! - The crate is `no_std` and its core API needs no `alloc` (DEC-002).
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod channel_voice1;
pub mod channel_voice2;
pub mod data128;
pub mod flex_data;
pub mod packet;
pub mod scale;
pub mod sysex7;
pub mod system;
#[cfg(feature = "nfc")]
pub mod text;
pub mod translate;
pub mod ump;
pub mod ump_stream;
pub mod utility;

pub use channel_voice1::{
    ChannelModeController, Midi1ChannelPressure, Midi1ChannelVoiceMessage, Midi1ControlChange,
    Midi1NoteOff, Midi1NoteOn, Midi1Opcode, Midi1PitchBend, Midi1PolyPressure, Midi1ProgramChange,
};
pub use channel_voice2::{
    msb_value, AssignableController, AssignablePerNoteController, AttributeType,
    Midi2ChannelPressure, Midi2ChannelVoiceMessage, Midi2ControlChange, Midi2NoteOff, Midi2NoteOn,
    Midi2Opcode, Midi2PitchBend, Midi2PolyPressure, Midi2ProgramChange, NoteAttribute,
    PerNoteManagement, PerNotePitchBend, PerNotePitchBendSensitivity, Pitch7_25, Pitch7_9,
    RegisteredController, RegisteredPerNoteController, RegisteredPerNoteControllerIndex,
    RelativeAssignableController, RelativeRegisteredController,
};
pub use data128::{
    Data128Message, ManufacturerId, MdsChunkPackets, MdsHeader, MdsIds, MdsPayload, SysEx8,
    SysEx8Data, SysEx8Event, SysEx8Packets, SysEx8Reassembler, SysEx8Status,
};
pub use flex_data::{
    ChordAlteration, ChordName, FlexAddress, FlexDataMessage, FlexDestination, FlexFormat,
    FlexStatusBank, FlexText, FlexTextBytes, FlexTextEvent, FlexTextPacket, FlexTextPackets,
    FlexTextReassembler, FlexTextStatus, MetadataTextStatus, PerformanceTextStatus, SetChordName,
    SetKeySignature, SetMetronome, SetTempo, SetTimeSignature, SetupStatus,
};
pub use packet::{Channel, Group, MessageType, UmpError, UmpWords};
pub use sysex7::{SysEx7, SysEx7Data, SysEx7Event, SysEx7Packets, SysEx7Reassembler, SysEx7Status};
pub use system::{
    ActiveSensing, Continue, MidiTimeCode, Reset, SongPositionPointer, SongSelect, Start, Stop,
    SystemMessage, SystemStatus, TimingClock, TuneRequest,
};
pub use translate::{
    midi2_to_midi1, to_midi1, Midi1ToMidi2, Midi1ToMidi2Group, Midi1Words, Translated,
};
pub use ump::{Packet, Ump, UnknownPacket};
pub use ump_stream::{
    DeviceIdentityNotification, EndOfClip, EndpointDiscovery, EndpointInfoNotification,
    FunctionBlockDirection, FunctionBlockDiscovery, FunctionBlockInfo,
    FunctionBlockInfoNotification, FunctionBlockMidi1, StartOfClip,
    StreamConfigurationNotification, StreamConfigurationRequest, StreamFormat, StreamProtocol,
    StreamText, StreamTextBytes, StreamTextEvent, StreamTextKind, StreamTextPacket,
    StreamTextPackets, StreamTextReassembler, UiHint, UmpStreamMessage, UmpStreamStatus,
};
pub use utility::{
    DeltaClockstamp, DeltaClockstampTpq, JrClock, JrTimestamp, Noop, UtilityMessage, UtilityStatus,
};
