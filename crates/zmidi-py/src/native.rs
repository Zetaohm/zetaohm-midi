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
    use crate::net::{NetClient, NetEvent, NetHost};
    #[pymodule_export]
    use crate::pe::{PeEngine, PeEvent, PeResource, PeSubscription};
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
