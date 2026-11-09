//! The extension module `zetaohm_midi._native` (pyo3 guidance: declarative module, every entry
//! exported) and the exceptions that Python sees (each failure is a `[pyo3]` exception; the
//! crate root maps the Rust errors to them in one place).

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

create_exception!(
    _native,
    MidiError,
    PyException,
    "Base class of the zetaohm-midi errors."
);
create_exception!(
    _native,
    InvalidValueError,
    MidiError,
    "A value is outside the range that the specification allows."
);
create_exception!(
    _native,
    MalformedError,
    MidiError,
    "The data is not a valid message or file."
);
create_exception!(
    _native,
    RejectedError,
    MidiError,
    "The library refused the request in its current state."
);

#[pymodule]
pub mod _native {
    #[pymodule_export]
    use super::{InvalidValueError, MalformedError, MidiError, RejectedError};
    #[pymodule_export]
    use crate::ble::{BleReader, BleWriter};
    #[pymodule_export]
    use crate::ci::{CiDevice, CiEvent};
    #[pymodule_export]
    use crate::file::{read_clip, ClipWriter};
    #[pymodule_export]
    use crate::io::{BytesToUmp, UmpToBytes};
    #[pymodule_export]
    use crate::midi1::{
        bank_program_messages, change_message, controller14_messages, reset_value, select_messages,
        value_messages, ActiveSensingMonitor, ActiveSensingSender, ControllerEvent,
        ControllerState, Midi1Message, Midi1Reader, Midi1Writer, ModeAction, ModeState, Parameter,
        ParameterEvent, ParameterState, SyncEvent, SyncReceiver,
    };
    /// Pitch units for one semitone: an MPE pitch offset is in hundredths of a cent.
    #[pymodule_export]
    use crate::midi1::{
        gm2_controller_default, gm_reset_controller_state, modulation_depth_range,
        rhythm_ignores_controller, rhythm_ignores_message, rhythm_ignores_parameter, GmReceiver,
    };
    #[pymodule_export]
    use crate::midi1::{
        mcm_messages, mode_message, mpe_class, note_messages, pitch_bend_value, pitch_offset,
        sensitivity_messages, zone_sensitivity_messages, MpeEvent, MpeLayout, MpeNoteAllocator,
        MpeReceiver,
    };
    #[pymodule_export]
    const PITCH_UNITS_PER_SEMITONE: i32 = 10_000;
    #[pymodule_export]
    use crate::net::{NetClient, NetEvent, NetHost};
    #[pymodule_export]
    use crate::pe::{PeEngine, PeEvent, PeResource, PeSubscription};
    #[pymodule_export]
    const BAR_NOT_RUNNING: i16 = -8192;
    #[pymodule_export]
    const BAR_UNKNOWN: i16 = 8191;
    #[pymodule_export]
    use crate::sysex::{
        coarse_tuning_semitones, fine_tuning_offset, frequency_word, frequency_word_bytes,
        gm_system, identity_reply, identity_request, manufacturer_sysex, mmc_commands, mmc_fields,
        mmc_full_name, mmc_responses, mmc_short_name, mtc_full, mtc_user_bits, parse_sysex,
        read_bar_marker, read_device_control, read_gm_system, read_inquiry, read_mmc,
        read_mmc_fields, read_mtc_full, read_mtc_user_bits, read_time_signature, read_tuning,
        respond_identity, sysex_addressed, tuning_checksum, tuning_dump_request, tuning_key_dump,
        tuning_note_change, tuning_scale_dump, tuning_scale_octave, universal_sysex, BarMarker,
        DeviceControl, Identity, InquiryMessage, MmcCommand, MmcField, MmcMessage, MtcEvent,
        MtcGenerator, MtcReader, MtcTime, SysexInfo, TimeCode, TimeSignature, TuningMessage,
    };
    #[pymodule_export]
    use crate::ump::{
        downscale, midi2_channel_pressure, midi2_control_change, midi2_note_off, midi2_note_on,
        midi2_pitch_bend, midi2_program_change, midi2_to_midi1, parse, upscale, word_count, Ump,
    };
    #[pymodule_export]
    use crate::usb::{
        gtb_blocks, gtb_request, gtb_total_length, UsbAlt0, UsbBlock, UsbEndpoint, UsbFunction,
        UsbLink, UsbSetting,
    };
}
