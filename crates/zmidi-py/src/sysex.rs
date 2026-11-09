//! The Python API of zmidi-sysex (DEC-022): the Universal System Exclusive reader and writers,
//! Device Inquiry, the General MIDI System messages, Device Control, MIDI Tuning, MIDI Time Code,
//! the MIDI Machine Control transport tier and Notation Information. A reader takes one whole message, with or without
//! F0H and F7H, and gives None for a message of another part; a bad message of its part raises
//! MalformedError. A writer gives the bytes of the whole message; a value out of range raises
//! InvalidValueError. Kinds are strings.

use std::borrow::Cow;

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use zmidi_sysex::{
    self as sx, DeviceControlValue, FinalByte, FrequencyWord, GmCommand, InfoField, InfoFields,
    InquiryMessage as RsInquiry, MmcCommand as RsCommand, MmcMessage as RsMmc, ScaleOffsets,
    ShortTimeCode, StandardTimeCode, Sysex, SysexError, SysexId, TimeCodeStatus, TimeCodeType,
    TuningMessage as RsTuning, Universal,
};
use zmidi_ump::ManufacturerId;

use crate::{invalid, malformed};

/// The Python error of a zmidi-sysex error: a bad message, or a value out of range.
fn error(e: SysexError) -> PyErr {
    match e {
        SysexError::OutOfRange | SysexError::BufferTooSmall { .. } => {
            invalid(format!("value out of range: {e}"))
        }
        _ => malformed(format!("not a valid message: {e}")),
    }
}

/// The bytes of a writer: a buffer that holds every message of the parts.
fn write(f: impl FnOnce(&mut [u8]) -> Result<usize, SysexError>) -> PyResult<Bytes> {
    let mut out = [0u8; 600];
    let n = f(&mut out).map_err(error)?;
    Ok(Cow::Owned(out[..n].to_vec()))
}

/// Bytes for Python (`bytes`, not a list).
type Bytes = Cow<'static, [u8]>;

/// The Universal header of one whole message, or None for another class.
fn universal(data: &[u8]) -> PyResult<Option<Universal<'_>>> {
    let s = Sysex::parse(data).map_err(error)?;
    Universal::parse(&s).map_err(error)
}

/// The message of a part reader: None for another message.
fn read<'a, T>(
    data: &'a [u8],
    f: impl FnOnce(&Universal<'a>) -> Result<Option<T>, SysexError>,
) -> PyResult<Option<T>> {
    match universal(data)? {
        Some(u) => f(&u).map_err(error),
        None => Ok(None),
    }
}

fn manufacturer(bytes: &[u8]) -> PyResult<ManufacturerId> {
    ManufacturerId::from_midi1(bytes)
        .map_err(|_| invalid("manufacturer must be 1 byte 01H to 7FH or 3 bytes 00H b2 b3"))
}

fn manufacturer_bytes(id: ManufacturerId) -> Vec<u8> {
    let b = id.to_midi1();
    if id.is_three_byte() {
        b.to_vec()
    } else {
        b[..1].to_vec()
    }
}

/// One System Exclusive message (M1 Table VII and VIIa).
#[pyclass(frozen, module = "zetaohm_midi._native")]
pub struct SysexInfo {
    /// "manufacturer", "non_commercial", "non_real_time" or "real_time".
    #[pyo3(get)]
    kind: &'static str,
    /// The Manufacturer ID bytes (1 or 3) of a manufacturer message, else empty.
    manufacturer: Vec<u8>,
    /// The Device ID of a Universal message.
    #[pyo3(get)]
    device_id: Option<u8>,
    /// Sub-ID#1 of a Universal message.
    #[pyo3(get)]
    sub_id1: Option<u8>,
    /// Sub-ID#2, when the family has one.
    #[pyo3(get)]
    sub_id2: Option<u8>,
    data: Vec<u8>,
}

#[pymethods]
impl SysexInfo {
    /// The manufacturer bytes.
    #[getter]
    fn manufacturer<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.manufacturer)
    }

    /// The data after the header (after the ID of a manufacturer or Non-Commercial message),
    /// without F7H.
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }
}

/// Reads one whole System Exclusive message, with or without F0H and F7H.
#[pyfunction]
pub fn parse_sysex(data: &[u8]) -> PyResult<SysexInfo> {
    let s = Sysex::parse(data).map_err(error)?;
    let mut r = SysexInfo {
        kind: "non_commercial",
        manufacturer: Vec::new(),
        device_id: None,
        sub_id1: None,
        sub_id2: None,
        data: s.data.to_vec(),
    };
    match s.id {
        SysexId::Manufacturer(id) => {
            r.kind = "manufacturer";
            r.manufacturer = manufacturer_bytes(id);
        }
        SysexId::NonCommercial => {}
        SysexId::NonRealTime | SysexId::RealTime => {
            let u = Universal::parse(&s)
                .map_err(error)?
                .ok_or_else(|| malformed("not a Universal message"))?;
            r.kind = if u.real_time {
                "real_time"
            } else {
                "non_real_time"
            };
            (r.device_id, r.sub_id1, r.sub_id2) = (Some(u.device_id), Some(u.sub_id1), u.sub_id2);
            r.data = u.data.to_vec();
        }
    }
    Ok(r)
}

/// True when a received Device ID addresses the device: 7FH or `own`.
#[pyfunction]
pub fn sysex_addressed(device_id: u8, own: u8) -> bool {
    sx::addressed(device_id, own)
}

/// The bytes of a manufacturer message: F0H, the ID (1 or 3 bytes), `data` and F7H.
#[pyfunction]
pub fn manufacturer_sysex(manufacturer: &[u8], data: &[u8]) -> PyResult<Bytes> {
    let id = self::manufacturer(manufacturer)?;
    write(|o| sx::write_manufacturer(id, data, o))
}

/// The bytes of a Universal message; `sub_id2` must be given exactly for a family that has one.
#[pyfunction]
#[pyo3(signature = (real_time, device_id, sub_id1, sub_id2, data))]
pub fn universal_sysex(
    real_time: bool,
    device_id: u8,
    sub_id1: u8,
    sub_id2: Option<u8>,
    data: &[u8],
) -> PyResult<Bytes> {
    let u = Universal {
        real_time,
        device_id,
        sub_id1,
        sub_id2,
        data,
    };
    write(|o| u.write(o))
}

/// The identity of a device in an Identity Reply (M1 General Information).
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct Identity(sx::Identity);

#[pymethods]
impl Identity {
    /// `manufacturer`: 1 byte, or 3 bytes 00H b2 b3; `family` and `member` 14 bits; `revision` 4
    /// bytes of 7 bits.
    #[new]
    fn new(manufacturer: &[u8], family: u16, member: u16, revision: [u8; 4]) -> PyResult<Self> {
        sx::Identity::new(self::manufacturer(manufacturer)?, family, member, revision)
            .map(Self)
            .map_err(error)
    }

    /// The Manufacturer ID bytes.
    #[getter]
    fn manufacturer<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &manufacturer_bytes(self.0.manufacturer))
    }

    /// The device family code.
    #[getter]
    fn family(&self) -> u16 {
        self.0.family
    }

    /// The device family member code.
    #[getter]
    fn member(&self) -> u16 {
        self.0.member
    }

    /// The software revision level.
    #[getter]
    fn revision<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.0.revision)
    }
}

/// An Identity Request or an Identity Reply.
#[pyclass(frozen, module = "zetaohm_midi._native")]
pub struct InquiryMessage {
    /// True for a reply.
    #[pyo3(get)]
    is_reply: bool,
    /// The Device ID.
    #[pyo3(get)]
    device_id: u8,
    /// The identity of a reply.
    #[pyo3(get)]
    identity: Option<Identity>,
}

/// Reads an Identity Request or Reply; None for another message.
#[pyfunction]
pub fn read_inquiry(data: &[u8]) -> PyResult<Option<InquiryMessage>> {
    Ok(read(data, RsInquiry::from_universal)?.map(|m| match m {
        RsInquiry::Request { device_id } => InquiryMessage {
            is_reply: false,
            device_id,
            identity: None,
        },
        RsInquiry::Reply {
            device_id,
            identity,
        } => InquiryMessage {
            is_reply: true,
            device_id,
            identity: Some(Identity(identity)),
        },
    }))
}

/// The bytes of an Identity Request to `device_id` (7FH: all devices).
#[pyfunction]
pub fn identity_request(device_id: u8) -> PyResult<Bytes> {
    write(|o| RsInquiry::Request { device_id }.write(o))
}

/// The bytes of an Identity Reply from `device_id`.
#[pyfunction]
pub fn identity_reply(device_id: u8, identity: &Identity) -> PyResult<Bytes> {
    write(|o| {
        RsInquiry::Reply {
            device_id,
            identity: identity.0,
        }
        .write(o)
    })
}

/// The Identity Reply to an Identity Request for `own_device_id` or for all devices; None for
/// another message.
#[pyfunction]
pub fn respond_identity(
    data: &[u8],
    own_device_id: u8,
    identity: &Identity,
) -> PyResult<Option<Bytes>> {
    let Some(m) = read(data, RsInquiry::from_universal)? else {
        return Ok(None);
    };
    let mut out = [0u8; 32];
    Ok(sx::respond(&m, own_device_id, &identity.0, &mut out)
        .map_err(error)?
        .map(|n| Cow::Owned(out[..n].to_vec())))
}

/// Reads a General MIDI System message: (device_id, command) with command 1 (GM1 System On), 2
/// (GM System Off) or 3 (GM2 System On); None for another message.
#[pyfunction]
pub fn read_gm_system(data: &[u8]) -> PyResult<Option<(u8, u8)>> {
    Ok(read(data, sx::GeneralMidi::from_universal)?.map(|m| (m.device_id, m.command.into_bits())))
}

/// The bytes of a General MIDI System message (command 1, 2 or 3).
#[pyfunction]
pub fn gm_system(device_id: u8, command: u8) -> PyResult<Bytes> {
    let command = GmCommand::from_bits(command).ok_or_else(|| invalid("command must be 1 to 3"))?;
    write(|o| sx::GeneralMidi { device_id, command }.write(o))
}

/// One Device Control message (M1 Device Control, CA-025).
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceControl(sx::DeviceControl);

#[pymethods]
impl DeviceControl {
    /// `kind`: "master_volume", "master_balance", "master_fine_tuning" (14-bit values) or
    /// "master_coarse_tuning" (the 7-bit MSB).
    #[new]
    fn new(kind: &str, device_id: u8, value: u16) -> PyResult<Self> {
        let value = match kind {
            "master_volume" => DeviceControlValue::MasterVolume(value),
            "master_balance" => DeviceControlValue::MasterBalance(value),
            "master_fine_tuning" => DeviceControlValue::MasterFineTuning(value),
            "master_coarse_tuning" if value <= 0x7F => {
                DeviceControlValue::MasterCoarseTuning(value as u8)
            }
            _ => return Err(invalid("not a Device Control kind or value")),
        };
        let m = sx::DeviceControl { device_id, value };
        m.write(&mut [0u8; 16]).map_err(error)?;
        Ok(Self(m))
    }

    /// The kind.
    #[getter]
    fn kind(&self) -> &'static str {
        match self.0.value {
            DeviceControlValue::MasterVolume(_) => "master_volume",
            DeviceControlValue::MasterBalance(_) => "master_balance",
            DeviceControlValue::MasterFineTuning(_) => "master_fine_tuning",
            DeviceControlValue::MasterCoarseTuning(_) => "master_coarse_tuning",
        }
    }

    /// The Device ID.
    #[getter]
    fn device_id(&self) -> u8 {
        self.0.device_id
    }

    /// The value.
    #[getter]
    fn value(&self) -> u16 {
        match self.0.value {
            DeviceControlValue::MasterVolume(v)
            | DeviceControlValue::MasterBalance(v)
            | DeviceControlValue::MasterFineTuning(v) => v,
            DeviceControlValue::MasterCoarseTuning(v) => u16::from(v),
        }
    }

    /// The bytes of the message.
    fn to_bytes(&self) -> PyResult<Bytes> {
        write(|o| self.0.write(o))
    }
}

/// Reads a Device Control message; None for another message.
#[pyfunction]
pub fn read_device_control(data: &[u8]) -> PyResult<Option<DeviceControl>> {
    Ok(read(data, sx::DeviceControl::from_universal)?.map(DeviceControl))
}

/// The signed offset from 8192 of a 14-bit Master Fine Tuning value (CA-025).
#[pyfunction]
pub fn fine_tuning_offset(value: u16) -> PyResult<i16> {
    if value > 0x3FFF {
        return Err(invalid("value must be 0 to 16383"));
    }
    Ok(sx::fine_tuning_offset(value))
}

/// The semitones from A440 of a Master Coarse Tuning MSB, -64 to +63 (CA-025).
#[pyfunction]
pub fn coarse_tuning_semitones(msb: u8) -> PyResult<i8> {
    if msb > 0x7F {
        return Err(invalid("msb must be 0 to 127"));
    }
    Ok(sx::coarse_tuning_semitones(msb))
}

/// One Bar Marker message (M1 Notation Information): the Device ID and the signed bar number,
/// -8192 (BAR_NOT_RUNNING) to 8191 (BAR_UNKNOWN); -8191 to 0 are count-in bars.
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct BarMarker(sx::BarMarker);

#[pymethods]
impl BarMarker {
    #[new]
    fn new(device_id: u8, bar: i16) -> PyResult<Self> {
        let bar = sx::Bar::from_value(bar).ok_or_else(|| invalid("bar must be -8192 to 8191"))?;
        let m = sx::BarMarker { device_id, bar };
        m.write(&mut [0u8; 8]).map_err(error)?;
        Ok(Self(m))
    }

    /// The Device ID.
    #[getter]
    fn device_id(&self) -> u8 {
        self.0.device_id
    }

    /// The signed bar number.
    #[getter]
    fn bar(&self) -> i16 {
        self.0.bar.value()
    }

    /// The bytes of the message.
    fn to_bytes(&self) -> PyResult<Bytes> {
        write(|o| self.0.write(o))
    }
}

/// Reads a Bar Marker; None for another message.
#[pyfunction]
pub fn read_bar_marker(data: &[u8]) -> PyResult<Option<BarMarker>> {
    Ok(read(data, sx::BarMarker::from_universal)?.map(BarMarker))
}

/// One Time Signature message, Immediate or Delayed (M1 Notation Information).
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct TimeSignature {
    device_id: u8,
    delayed: bool,
    fields: [u8; 4],
    additional: Vec<u8>,
}

impl TimeSignature {
    fn message(&self) -> sx::TimeSignature<'_> {
        let [numerator, denominator_power, clocks_per_click, thirty_seconds_per_quarter] =
            self.fields;
        sx::TimeSignature {
            device_id: self.device_id,
            delayed: self.delayed,
            numerator,
            denominator_power,
            clocks_per_click,
            thirty_seconds_per_quarter,
            additional: &self.additional,
        }
    }

    fn of(m: &sx::TimeSignature<'_>) -> Self {
        Self {
            device_id: m.device_id,
            delayed: m.delayed,
            fields: [
                m.numerator,
                m.denominator_power,
                m.clocks_per_click,
                m.thirty_seconds_per_quarter,
            ],
            additional: m.additional.to_vec(),
        }
    }
}

#[pymethods]
impl TimeSignature {
    /// `denominator_power` is a negative power of 2 (2 = quarter note); `additional` holds the
    /// numerator and denominator-power pairs of a compound time signature, at most 61 pairs.
    #[new]
    #[pyo3(signature = (device_id, numerator, denominator_power, clocks_per_click, thirty_seconds_per_quarter, additional=Vec::new(), delayed=false))]
    fn new(
        device_id: u8,
        numerator: u8,
        denominator_power: u8,
        clocks_per_click: u8,
        thirty_seconds_per_quarter: u8,
        additional: Vec<u8>,
        delayed: bool,
    ) -> PyResult<Self> {
        let t = Self {
            device_id,
            delayed,
            fields: [
                numerator,
                denominator_power,
                clocks_per_click,
                thirty_seconds_per_quarter,
            ],
            additional,
        };
        t.message().write(&mut [0u8; 140]).map_err(error)?;
        Ok(t)
    }

    /// The Device ID.
    #[getter]
    fn device_id(&self) -> u8 {
        self.device_id
    }

    /// True for the Delayed form (42H), which takes effect at the next Bar Marker.
    #[getter]
    fn delayed(&self) -> bool {
        self.delayed
    }

    /// `nn`: the numerator.
    #[getter]
    fn numerator(&self) -> u8 {
        self.fields[0]
    }

    /// `dd`: the denominator as a negative power of 2.
    #[getter]
    fn denominator_power(&self) -> u8 {
        self.fields[1]
    }

    /// `cc`: MIDI clocks in a metronome click.
    #[getter]
    fn clocks_per_click(&self) -> u8 {
        self.fields[2]
    }

    /// `bb`: notated 32nd notes in a MIDI quarter note.
    #[getter]
    fn thirty_seconds_per_quarter(&self) -> u8 {
        self.fields[3]
    }

    /// The additional pairs as bytes.
    #[getter]
    fn additional(&self) -> Bytes {
        Cow::Owned(self.additional.clone())
    }

    /// The additional pairs, `(numerator, denominator_power)`.
    #[getter]
    fn pairs(&self) -> Vec<(u8, u8)> {
        self.message().pairs().collect()
    }

    /// The bytes of the message.
    fn to_bytes(&self) -> PyResult<Bytes> {
        write(|o| self.message().write(o))
    }
}

/// Reads a Time Signature; None for another message.
#[pyfunction]
pub fn read_time_signature(data: &[u8]) -> PyResult<Option<TimeSignature>> {
    Ok(read(data, sx::TimeSignature::from_universal)?.map(|m| TimeSignature::of(&m)))
}

/// One MIDI Tuning message (M1 MIDI Tuning, CA-020, CA-021).
#[pyclass(frozen, module = "zetaohm_midi._native")]
pub struct TuningMessage {
    /// "dump_request", "key_dump", "note_change", "scale_dump" or "scale_octave".
    #[pyo3(get)]
    kind: &'static str,
    /// True for the Real Time form.
    #[pyo3(get)]
    real_time: bool,
    /// The Device ID.
    #[pyo3(get)]
    device_id: u8,
    /// The bank, or None.
    #[pyo3(get)]
    bank: Option<u8>,
    /// The tuning program.
    #[pyo3(get)]
    program: u8,
    /// The 16-byte name of a dump.
    name: Vec<u8>,
    /// The checksum result of a 01H Bulk Tuning Dump (True for the other kinds).
    #[pyo3(get)]
    checksum_ok: bool,
    /// The channel mask of scale_octave.
    #[pyo3(get)]
    channels: u16,
    /// True for the 2-byte offsets.
    #[pyo3(get)]
    two_byte: bool,
    /// The 12 offsets of scale_dump and scale_octave.
    #[pyo3(get)]
    offsets: Vec<i16>,
    /// The notes (key, semitone, fraction) of key_dump and note_change.
    #[pyo3(get)]
    notes: Vec<(u8, u8, u16)>,
}

#[pymethods]
impl TuningMessage {
    /// The name bytes.
    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.name)
    }
}

fn offsets(o: ScaleOffsets) -> (bool, Vec<i16>) {
    match o {
        ScaleOffsets::OneByte(x) => (false, x.iter().map(|&v| i16::from(v)).collect()),
        ScaleOffsets::TwoByte(x) => (true, x.to_vec()),
    }
}

fn rust_offsets(offsets: &[i16], two_byte: bool) -> PyResult<ScaleOffsets> {
    let o: [i16; 12] = offsets
        .try_into()
        .map_err(|_| invalid("offsets must be 12 values"))?;
    if two_byte {
        return Ok(ScaleOffsets::TwoByte(o));
    }
    let mut b = [0i8; 12];
    for (x, v) in b.iter_mut().zip(o) {
        *x = i8::try_from(v).map_err(|_| invalid("a 1-byte offset must be -64 to 63"))?;
    }
    Ok(ScaleOffsets::OneByte(b))
}

fn word(semitone: u8, fraction: u16) -> PyResult<FrequencyWord> {
    if semitone > 0x7F || fraction > 0x3FFF {
        return Err(invalid("semitone must be 0 to 127 and fraction 0 to 16383"));
    }
    Ok(FrequencyWord { semitone, fraction })
}

fn name(name: &[u8]) -> PyResult<[u8; 16]> {
    name.try_into()
        .map_err(|_| invalid("name must be 16 bytes"))
}

/// Reads a MIDI Tuning message; None for another message.
#[pyfunction]
pub fn read_tuning(data: &[u8]) -> PyResult<Option<TuningMessage>> {
    let Some(m) = read(data, RsTuning::from_universal)? else {
        return Ok(None);
    };
    let mut t = TuningMessage {
        kind: "dump_request",
        real_time: false,
        device_id: 0,
        bank: None,
        program: 0,
        name: Vec::new(),
        checksum_ok: true,
        channels: 0,
        two_byte: false,
        offsets: Vec::new(),
        notes: Vec::new(),
    };
    match m {
        RsTuning::DumpRequest {
            device_id,
            bank,
            program,
        } => (t.device_id, t.bank, t.program) = (device_id, bank, program),
        RsTuning::KeyDump {
            device_id,
            bank,
            program,
            name,
            words,
            checksum_ok,
        } => {
            t.kind = "key_dump";
            (t.device_id, t.bank, t.program) = (device_id, bank, program);
            (t.name, t.checksum_ok) = (name.to_vec(), checksum_ok);
            t.notes = words
                .enumerate()
                .map(|(k, w)| (k as u8, w.semitone, w.fraction))
                .collect();
        }
        RsTuning::NoteChange {
            real_time,
            device_id,
            bank,
            program,
            changes,
        } => {
            t.kind = "note_change";
            (t.real_time, t.device_id, t.bank, t.program) = (real_time, device_id, bank, program);
            t.notes = changes.map(|(k, w)| (k, w.semitone, w.fraction)).collect();
        }
        RsTuning::ScaleDump {
            device_id,
            bank,
            program,
            name,
            offsets: o,
        } => {
            t.kind = "scale_dump";
            (t.device_id, t.bank, t.program, t.name) =
                (device_id, Some(bank), program, name.to_vec());
            (t.two_byte, t.offsets) = offsets(o);
        }
        RsTuning::ScaleOctave {
            real_time,
            device_id,
            channels,
            offsets: o,
        } => {
            t.kind = "scale_octave";
            (t.real_time, t.device_id, t.channels) = (real_time, device_id, channels);
            (t.two_byte, t.offsets) = offsets(o);
        }
    }
    Ok(Some(t))
}

/// The bytes of a Bulk Tuning Dump Request (00H, or 03H with a bank).
#[pyfunction]
#[pyo3(signature = (device_id, program, bank=None))]
pub fn tuning_dump_request(device_id: u8, program: u8, bank: Option<u8>) -> PyResult<Bytes> {
    write(|o| sx::write_dump_request(device_id, bank, program, o))
}

/// The bytes of a Bulk Tuning Dump (01H), or a Key-Based Tuning Dump (04H) with a bank: the
/// 16-byte `name` and 128 `words` of (semitone, fraction).
#[pyfunction]
#[pyo3(signature = (device_id, program, name, words, bank=None))]
pub fn tuning_key_dump(
    device_id: u8,
    program: u8,
    name: &[u8],
    words: Vec<(u8, u16)>,
    bank: Option<u8>,
) -> PyResult<Bytes> {
    if words.len() != 128 {
        return Err(invalid("words must be 128 (semitone, fraction) pairs"));
    }
    let mut w = [FrequencyWord::NO_CHANGE; 128];
    for (x, &(s, f)) in w.iter_mut().zip(&words) {
        *x = word(s, f)?;
    }
    let name = self::name(name)?;
    write(|o| sx::write_key_dump(device_id, bank, program, &name, &w, o))
}

/// The bytes of a Single Note Tuning Change: Real Time 02H, or 07H with a bank (the Non-Real
/// Time form needs a bank), from 1 to 127 `notes` of (key, semitone, fraction).
#[pyfunction]
#[pyo3(signature = (real_time, device_id, program, notes, bank=None))]
pub fn tuning_note_change(
    real_time: bool,
    device_id: u8,
    program: u8,
    notes: Vec<(u8, u8, u16)>,
    bank: Option<u8>,
) -> PyResult<Bytes> {
    let changes = notes
        .iter()
        .map(|&(k, s, f)| Ok((k, word(s, f)?)))
        .collect::<PyResult<Vec<_>>>()?;
    write(|o| sx::write_note_change(real_time, device_id, bank, program, &changes, o))
}

/// The bytes of a Scale/Octave Tuning Dump (05H, or 06H for `two_byte`).
#[pyfunction]
pub fn tuning_scale_dump(
    device_id: u8,
    bank: u8,
    program: u8,
    name: &[u8],
    offsets: Vec<i16>,
    two_byte: bool,
) -> PyResult<Bytes> {
    let (name, o) = (self::name(name)?, rust_offsets(&offsets, two_byte)?);
    write(|out| sx::write_scale_dump(device_id, bank, program, &name, &o, out))
}

/// The bytes of a Scale/Octave Tuning message (08H, or 09H for `two_byte`) for the channel mask
/// `channels`.
#[pyfunction]
pub fn tuning_scale_octave(
    real_time: bool,
    device_id: u8,
    channels: u16,
    offsets: Vec<i16>,
    two_byte: bool,
) -> PyResult<Bytes> {
    let o = rust_offsets(&offsets, two_byte)?;
    write(|out| sx::write_scale_octave(real_time, device_id, channels, &o, out))
}

/// The checksum of the bytes of a tuning dump: the XOR of the bytes, masked to 7 bits.
#[pyfunction]
pub fn tuning_checksum(data: &[u8]) -> u8 {
    sx::checksum(data)
}

/// The (semitone, fraction) of the three bytes of a frequency word.
#[pyfunction]
pub fn frequency_word(data: [u8; 3]) -> PyResult<(u8, u16)> {
    let w = FrequencyWord::from_bytes(data).map_err(error)?;
    Ok((w.semitone, w.fraction))
}

/// The three bytes of a frequency word.
#[pyfunction]
pub fn frequency_word_bytes(semitone: u8, fraction: u16) -> PyResult<Bytes> {
    Ok(Cow::Owned(word(semitone, fraction)?.to_bytes().to_vec()))
}

/// An MTC time (RP-004): hours, minutes, seconds, frames and the time code type `rate` (0: 24,
/// 1: 25, 2: 30 drop-frame, 3: 30 frames per second). A reader gives the counts as received.
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct MtcTime(pub(crate) sx::MtcTime);

#[pymethods]
impl MtcTime {
    /// A valid time; a count out of range or a skipped drop-frame number raises
    /// InvalidValueError.
    #[new]
    fn new(hours: u8, minutes: u8, seconds: u8, frames: u8, rate: u8) -> PyResult<Self> {
        if rate > 3 {
            return Err(invalid("rate must be 0 to 3"));
        }
        sx::MtcTime::new(
            hours,
            minutes,
            seconds,
            frames,
            TimeCodeType::from_bits(rate),
        )
        .map(Self)
        .map_err(error)
    }

    /// The time of `count` frames from 00:00:00:00, with the 24-hour wrap.
    #[staticmethod]
    fn from_frame_count(count: u32, rate: u8) -> PyResult<Self> {
        if rate > 3 {
            return Err(invalid("rate must be 0 to 3"));
        }
        Ok(Self(sx::MtcTime::from_frame_count(
            count,
            TimeCodeType::from_bits(rate),
        )))
    }

    #[getter]
    fn hours(&self) -> u8 {
        self.0.hours
    }

    #[getter]
    fn minutes(&self) -> u8 {
        self.0.minutes
    }

    #[getter]
    fn seconds(&self) -> u8 {
        self.0.seconds
    }

    #[getter]
    fn frames(&self) -> u8 {
        self.0.frames
    }

    #[getter]
    fn rate(&self) -> u8 {
        self.0.rate.into_bits()
    }

    /// The frames of a second of the type: 24, 25 or 30.
    #[getter]
    fn frames_per_second(&self) -> u8 {
        self.0.rate.frames_per_second()
    }

    /// True when each count is in range and the frame number is not skipped.
    fn is_valid(&self) -> bool {
        self.0.is_valid()
    }

    /// The time, or frame 02 for a frame number that the drop-frame type skips.
    fn next_valid(&self) -> Self {
        Self(self.0.next_valid())
    }

    /// The frames from 00:00:00:00 by the rule of the type.
    fn to_frame_count(&self) -> u32 {
        self.0.to_frame_count()
    }

    /// The time `delta` frames later (or earlier), with the 24-hour wrap.
    fn add_frames(&self, delta: i32) -> Self {
        Self(self.0.add_frames(delta))
    }

    /// The 8 Quarter Frame data bytes, piece 0 first.
    fn quarter_frames<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &sx::QuarterFrame::pieces(&self.0))
    }

    fn __repr__(&self) -> String {
        let t = self.0;
        format!(
            "MtcTime({:02}:{:02}:{:02}:{:02}, rate={})",
            t.hours,
            t.minutes,
            t.seconds,
            t.frames,
            t.rate.into_bits()
        )
    }
}

fn direction(reverse: bool) -> sx::Direction {
    if reverse {
        sx::Direction::Reverse
    } else {
        sx::Direction::Forward
    }
}

/// The Quarter Frame generator (RP-004 p. 3): one data byte at each call, with no clock.
#[pyclass(module = "zetaohm_midi._native")]
pub struct MtcGenerator(sx::MtcGenerator);

#[pymethods]
impl MtcGenerator {
    /// A running generator: the first sequence encodes `time` from piece 0, or from piece 7 when
    /// `reverse`.
    #[new]
    #[pyo3(signature = (time, reverse=false))]
    fn new(time: &MtcTime, reverse: bool) -> PyResult<Self> {
        if !time.0.is_valid() {
            return Err(invalid("time is not valid"));
        }
        Ok(Self(sx::MtcGenerator::new(time.0, direction(reverse))))
    }

    /// The data byte of the next piece, or None while stopped.
    fn next_byte(&mut self) -> Option<u8> {
        self.0.next_byte()
    }

    /// Runs or stops the generator.
    fn set_running(&mut self, running: bool) {
        self.0.set_running(running);
    }

    /// Sets the direction; the next piece is the next one in the new order.
    fn set_direction(&mut self, reverse: bool) {
        self.0.set_direction(direction(reverse));
    }

    /// A new time; the next piece is the first piece of the direction.
    fn locate(&mut self, time: &MtcTime) -> PyResult<()> {
        if !time.0.is_valid() {
            return Err(invalid("time is not valid"));
        }
        self.0.locate(time.0);
        Ok(())
    }

    /// The time that the current or next sequence encodes at its piece 0.
    #[getter]
    fn time(&self) -> MtcTime {
        MtcTime(self.0.time())
    }
}

/// What one Quarter Frame data byte gives (RP-004 p. 3).
#[pyclass(frozen, module = "zetaohm_midi._native")]
pub struct MtcEvent {
    /// The piece, 0 to 7.
    #[pyo3(get)]
    piece: u8,
    /// True for pieces 0 and 4, which mark frame boundaries.
    #[pyo3(get)]
    boundary: bool,
    /// The assembled time at the end of a full sequence, else None.
    #[pyo3(get)]
    raw: Option<MtcTime>,
    /// The current time at the end of a full sequence: raw plus 2 frames forward, minus 1 frame
    /// in reverse.
    #[pyo3(get)]
    current: Option<MtcTime>,
    /// True for a reverse sequence.
    #[pyo3(get)]
    reverse: bool,
    /// True when raw is not the previous raw time plus 2 frames (minus 2 in reverse).
    #[pyo3(get)]
    discontinuity: bool,
}

/// The Quarter Frame reader (RP-004 p. 3): lock, direction and the current time.
#[pyclass(module = "zetaohm_midi._native")]
pub struct MtcReader(sx::MtcReader);

#[pymethods]
impl MtcReader {
    /// A reader that is not locked and is stopped.
    #[new]
    fn new() -> Self {
        Self(sx::MtcReader::new())
    }

    /// Takes one Quarter Frame data byte, 0 to 127.
    fn receive(&mut self, byte: u8) -> PyResult<MtcEvent> {
        if byte > 0x7F {
            return Err(invalid("byte must be 0 to 127"));
        }
        let e = self.0.receive(byte);
        let s = e.sequence;
        Ok(MtcEvent {
            piece: e.piece,
            boundary: e.boundary,
            raw: s.map(|s| MtcTime(s.raw)),
            current: s.map(|s| MtcTime(s.current)),
            reverse: s.is_some_and(|s| s.direction == sx::Direction::Reverse),
            discontinuity: s.is_some_and(|s| s.discontinuity),
        })
    }

    /// Takes the time of a Full Message: the reader has this time and is stopped until the next
    /// data byte.
    fn full_message(&mut self, time: &MtcTime) {
        self.0.full_message(&sx::FullMessage {
            device_id: 0x7F,
            time: time.0,
        });
    }

    /// True after a full sequence, until a broken order or a direction change.
    #[getter]
    fn is_locked(&self) -> bool {
        self.0.is_locked()
    }

    /// True after a data byte, False after a Full Message.
    #[getter]
    fn is_running(&self) -> bool {
        self.0.is_running()
    }

    /// "forward", "reverse", or None before two pieces gave it.
    #[getter]
    fn direction(&self) -> Option<&'static str> {
        self.0.direction().map(|d| match d {
            sx::Direction::Forward => "forward",
            sx::Direction::Reverse => "reverse",
        })
    }

    /// The current time of the last full sequence, or the time of a Full Message after it.
    #[getter]
    fn time(&self) -> Option<MtcTime> {
        self.0.time().map(MtcTime)
    }
}

/// Reads an MTC Full Message: (device_id, time); None for another message.
#[pyfunction]
pub fn read_mtc_full(data: &[u8]) -> PyResult<Option<(u8, MtcTime)>> {
    Ok(read(data, sx::FullMessage::from_universal)?.map(|m| (m.device_id, MtcTime(m.time))))
}

/// The bytes of an MTC Full Message.
#[pyfunction]
pub fn mtc_full(device_id: u8, time: &MtcTime) -> PyResult<Bytes> {
    write(|o| {
        sx::FullMessage {
            device_id,
            time: time.0,
        }
        .write(o)
    })
}

/// Reads an MTC User Bits message: (device_id, the 8 groups, the flags); None for another
/// message.
#[pyfunction]
pub fn read_mtc_user_bits(data: &[u8]) -> PyResult<Option<(u8, Bytes, u8)>> {
    Ok(read(data, sx::UserBits::from_universal)?
        .map(|m| (m.device_id, Cow::Owned(m.groups.to_vec()), m.flags)))
}

/// The bytes of an MTC User Bits message: 8 groups of 4 bits and the 2 flag bits.
#[pyfunction]
pub fn mtc_user_bits(device_id: u8, groups: [u8; 8], flags: u8) -> PyResult<Bytes> {
    write(|o| {
        sx::UserBits {
            device_id,
            groups,
            flags,
        }
        .write(o)
    })
}

/// An MMC Standard Time Code (RP-013 pp. 8, 9). The final byte is the subframes (status_form
/// False) or the status flags estimated, invalid, video_field1 and no_time_code.
#[pyclass(frozen, from_py_object, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct TimeCode(StandardTimeCode);

#[pymethods]
impl TimeCode {
    /// A time code of a time; the writer checks the counts.
    #[new]
    #[pyo3(signature = (time, color_frame=false, blank=false, negative=false, status_form=false, subframes=0, estimated=false, invalid=false, video_field1=false, no_time_code=false))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        time: &MtcTime,
        color_frame: bool,
        blank: bool,
        negative: bool,
        status_form: bool,
        subframes: u8,
        estimated: bool,
        invalid: bool,
        video_field1: bool,
        no_time_code: bool,
    ) -> PyResult<Self> {
        let final_byte = if status_form {
            FinalByte::Status(TimeCodeStatus {
                estimated,
                invalid,
                video_field1,
                no_time_code,
            })
        } else if subframes <= 99 {
            FinalByte::Subframes(subframes)
        } else {
            return Err(crate::invalid("subframes must be 0 to 99"));
        };
        Ok(Self(StandardTimeCode {
            time: time.0,
            color_frame,
            blank,
            negative,
            final_byte,
        }))
    }

    /// Reads the 5 bytes; the counts are as received.
    #[staticmethod]
    fn from_bytes(data: [u8; 5]) -> Self {
        Self(StandardTimeCode::from_bytes(data))
    }

    /// The 5 bytes; a time that is not valid raises InvalidValueError.
    fn to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(py, &self.0.to_bytes().map_err(error)?))
    }

    /// The time code with the 2 bytes of a short time code applied to it.
    fn with_short(&self, data: [u8; 2]) -> Self {
        Self(self.0.with_short(ShortTimeCode::from_bytes(data)))
    }

    /// The 2 bytes of the short form.
    fn short_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(py, &self.0.short().to_bytes().map_err(error)?))
    }

    /// The MOVE rule to the subframes form.
    fn to_subframes_form(&self) -> Self {
        Self(self.0.to_subframes_form())
    }

    /// The MOVE rule to the status form, with the video field 1 flag of the caller.
    fn to_status_form(&self, video_field1: bool) -> Self {
        Self(self.0.to_status_form(video_field1))
    }

    #[getter]
    fn time(&self) -> MtcTime {
        MtcTime(self.0.time)
    }

    #[getter]
    fn color_frame(&self) -> bool {
        self.0.color_frame
    }

    #[getter]
    fn blank(&self) -> bool {
        self.0.blank
    }

    #[getter]
    fn negative(&self) -> bool {
        self.0.negative
    }

    #[getter]
    fn status_form(&self) -> bool {
        matches!(self.0.final_byte, FinalByte::Status(_))
    }

    #[getter]
    fn subframes(&self) -> u8 {
        match self.0.final_byte {
            FinalByte::Subframes(s) => s,
            FinalByte::Status(_) => 0,
        }
    }

    #[getter]
    fn estimated(&self) -> bool {
        self.status().estimated
    }

    #[getter]
    fn invalid(&self) -> bool {
        self.status().invalid
    }

    #[getter]
    fn video_field1(&self) -> bool {
        self.status().video_field1
    }

    #[getter]
    fn no_time_code(&self) -> bool {
        self.status().no_time_code
    }
}

impl TimeCode {
    fn status(&self) -> TimeCodeStatus {
        match self.0.final_byte {
            FinalByte::Status(s) => s,
            FinalByte::Subframes(_) => TimeCodeStatus::default(),
        }
    }
}

/// One MMC command (RP-013 pp. 20 to 32).
#[pyclass(frozen, from_py_object, module = "zetaohm_midi._native")]
#[derive(Clone)]
pub struct MmcCommand {
    /// "stop", "play", "deferred_play", "fast_forward", "rewind", "record_strobe",
    /// "record_exit", "pause", "reset", "write", "locate_field", "locate_target", "move" or
    /// "other".
    #[pyo3(get)]
    kind: String,
    /// The register of locate_field, 08H to 0FH.
    #[pyo3(get)]
    register: u8,
    /// The destination field of move.
    #[pyo3(get)]
    destination: u8,
    /// The source field of move.
    #[pyo3(get)]
    source: u8,
    /// The target of locate_target.
    #[pyo3(get)]
    target: Option<TimeCode>,
    /// The name bytes of other, with its 00H extension prefixes.
    name: Vec<u8>,
    /// The Information Field bytes of write, or the data of other.
    data: Vec<u8>,
}

const COMMANDS: [&str; 9] = [
    "stop",
    "play",
    "deferred_play",
    "fast_forward",
    "rewind",
    "record_strobe",
    "record_exit",
    "pause",
    "reset",
];

#[pymethods]
impl MmcCommand {
    /// The name bytes.
    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.name)
    }

    /// The data bytes.
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }

    /// A command of `kind` with the arguments of its kind.
    #[new]
    #[pyo3(signature = (kind, register=0, destination=0, source=0, target=None, name=Vec::new(), data=Vec::new()))]
    fn new(
        kind: &str,
        register: u8,
        destination: u8,
        source: u8,
        target: Option<TimeCode>,
        name: Vec<u8>,
        data: Vec<u8>,
    ) -> PyResult<Self> {
        let c = Self {
            kind: kind.to_owned(),
            register,
            destination,
            source,
            target,
            name,
            data,
        };
        let r = c.rust()?;
        sx::write_commands(&mut [0u8; 64], 0x7F, &[r]).map_err(error)?;
        Ok(c)
    }
}

impl MmcCommand {
    fn of(c: &RsCommand<'_>) -> Self {
        let mut r = Self {
            kind: String::new(),
            register: 0,
            destination: 0,
            source: 0,
            target: None,
            name: Vec::new(),
            data: Vec::new(),
        };
        let kind = match c {
            RsCommand::Stop => "stop",
            RsCommand::Play => "play",
            RsCommand::DeferredPlay => "deferred_play",
            RsCommand::FastForward => "fast_forward",
            RsCommand::Rewind => "rewind",
            RsCommand::RecordStrobe => "record_strobe",
            RsCommand::RecordExit => "record_exit",
            RsCommand::Pause => "pause",
            RsCommand::Reset => "reset",
            RsCommand::Write(f) => {
                r.data = f.as_bytes().to_vec();
                "write"
            }
            RsCommand::LocateField { register } => {
                r.register = *register;
                "locate_field"
            }
            RsCommand::LocateTarget(t) => {
                r.target = Some(TimeCode(*t));
                "locate_target"
            }
            RsCommand::Move {
                destination,
                source,
            } => {
                (r.destination, r.source) = (*destination, *source);
                "move"
            }
            RsCommand::Other { name, data } => {
                (r.name, r.data) = (name.to_vec(), data.to_vec());
                "other"
            }
        };
        r.kind = kind.to_owned();
        r
    }

    fn rust(&self) -> PyResult<RsCommand<'_>> {
        Ok(match self.kind.as_str() {
            "stop" => RsCommand::Stop,
            "play" => RsCommand::Play,
            "deferred_play" => RsCommand::DeferredPlay,
            "fast_forward" => RsCommand::FastForward,
            "rewind" => RsCommand::Rewind,
            "record_strobe" => RsCommand::RecordStrobe,
            "record_exit" => RsCommand::RecordExit,
            "pause" => RsCommand::Pause,
            "reset" => RsCommand::Reset,
            "write" => RsCommand::Write(InfoFields::new(&self.data)),
            "locate_field" => RsCommand::LocateField {
                register: self.register,
            },
            "locate_target" => RsCommand::LocateTarget(
                self.target
                    .as_ref()
                    .ok_or_else(|| invalid("locate_target needs a target"))?
                    .0,
            ),
            "move" => RsCommand::Move {
                destination: self.destination,
                source: self.source,
            },
            "other" => RsCommand::Other {
                name: &self.name,
                data: &self.data,
            },
            _ => {
                return Err(invalid(format!(
                    "kind must be one of {}, write, locate_field, locate_target, move, other",
                    COMMANDS.join(", ")
                )))
            }
        })
    }
}

/// One MMC Information Field (RP-013 pp. 8, 9, 44, 47).
#[pyclass(frozen, from_py_object, module = "zetaohm_midi._native")]
#[derive(Clone)]
pub struct MmcField {
    /// "time_code" (a field 01H to 1FH), "short" (its short form) or "other".
    #[pyo3(get)]
    kind: &'static str,
    /// The field name 01H to 1FH of time_code and short.
    #[pyo3(get)]
    field: u8,
    /// The time code of time_code; of short, only the frames, the sign and the final byte (use
    /// TimeCode.with_short with the last full value and data).
    #[pyo3(get)]
    code: Option<TimeCode>,
    /// The name bytes of other, with its 00H extension prefixes.
    name: Vec<u8>,
    /// The data bytes (after the count byte when the name has one).
    data: Vec<u8>,
}

#[pymethods]
impl MmcField {
    /// The name bytes.
    #[getter]
    fn name<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.name)
    }

    /// The data bytes.
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }

    /// A field of `kind`: time_code with `field` and `code`, short with `field` and `code`, or
    /// other with `name` and `data`.
    #[new]
    #[pyo3(signature = (kind, field=0, code=None, name=Vec::new(), data=Vec::new()))]
    fn new(
        kind: &str,
        field: u8,
        code: Option<TimeCode>,
        name: Vec<u8>,
        data: Vec<u8>,
    ) -> PyResult<Self> {
        let kind = match kind {
            "time_code" => "time_code",
            "short" => "short",
            "other" => "other",
            _ => return Err(invalid("kind must be time_code, short or other")),
        };
        let f = Self {
            kind,
            field,
            code,
            name,
            data,
        };
        sx::write_info_fields(&mut [0u8; 64], &[f.rust()?]).map_err(error)?;
        Ok(f)
    }
}

impl MmcField {
    /// The field of `f`, whose bytes in the string are `used`.
    fn of(f: &InfoField<'_>, used: &[u8]) -> Self {
        let (name, data) = used.split_at(used.len().min(1));
        match *f {
            InfoField::TimeCode { field, code } => Self {
                kind: "time_code",
                field,
                code: Some(TimeCode(code)),
                name: name.to_vec(),
                data: data.to_vec(),
            },
            InfoField::Short { field, code } => {
                let t = StandardTimeCode {
                    time: sx::MtcTime {
                        frames: code.frames,
                        ..sx::MtcTime::from_frame_count(0, TimeCodeType::Fps24)
                    },
                    color_frame: false,
                    blank: false,
                    negative: code.negative,
                    final_byte: code.final_byte,
                };
                Self {
                    kind: "short",
                    field,
                    code: Some(TimeCode(t)),
                    name: name.to_vec(),
                    data: data.to_vec(),
                }
            }
            InfoField::Other { name, data } => Self {
                kind: "other",
                field: 0,
                code: None,
                name: name.to_vec(),
                data: data.to_vec(),
            },
        }
    }

    fn rust(&self) -> PyResult<InfoField<'_>> {
        let code = || {
            self.code
                .as_ref()
                .ok_or_else(|| invalid("the field needs a code"))
        };
        Ok(match self.kind {
            "time_code" => InfoField::TimeCode {
                field: self.field,
                code: code()?.0,
            },
            "short" => InfoField::Short {
                field: self.field,
                code: code()?.0.short(),
            },
            _ => InfoField::Other {
                name: &self.name,
                data: &self.data,
            },
        })
    }
}

/// An MMC command or response message (RP-013 p. 1).
#[pyclass(frozen, module = "zetaohm_midi._native")]
pub struct MmcMessage {
    /// True for a response (Sub-ID#1 07H).
    #[pyo3(get)]
    response: bool,
    /// The destination of commands, or the source of responses.
    #[pyo3(get)]
    device_id: u8,
    /// The commands of a command message.
    #[pyo3(get)]
    commands: Vec<MmcCommand>,
    /// The Information Fields of a response.
    #[pyo3(get)]
    fields: Vec<MmcField>,
}

fn fields(mut f: InfoFields<'_>) -> PyResult<Vec<MmcField>> {
    let mut v = Vec::new();
    loop {
        let before = f.as_bytes();
        match f.next() {
            None => return Ok(v),
            Some(Err(e)) => return Err(error(e)),
            Some(Ok(x)) => v.push(MmcField::of(
                &x,
                &before[..before.len() - f.as_bytes().len()],
            )),
        }
    }
}

/// Reads an MMC message; None for another message. A string cut short raises MalformedError.
#[pyfunction]
pub fn read_mmc(data: &[u8]) -> PyResult<Option<MmcMessage>> {
    Ok(match read(data, RsMmc::from_universal)? {
        None => None,
        Some(RsMmc::Commands {
            device_id,
            commands,
        }) => Some(MmcMessage {
            response: false,
            device_id,
            commands: commands
                .map(|c| c.map(|c| MmcCommand::of(&c)).map_err(error))
                .collect::<PyResult<_>>()?,
            fields: Vec::new(),
        }),
        Some(RsMmc::Responses { device_id, fields }) => Some(MmcMessage {
            response: true,
            device_id,
            commands: Vec::new(),
            fields: self::fields(fields)?,
        }),
    })
}

/// Reads the Information Fields of the data of a WRITE command.
#[pyfunction]
pub fn read_mmc_fields(data: &[u8]) -> PyResult<Vec<MmcField>> {
    fields(InfoFields::new(data))
}

/// The bytes of an MMC command message to `device_id` (7FH: all devices).
#[pyfunction]
pub fn mmc_commands(device_id: u8, commands: Vec<MmcCommand>) -> PyResult<Bytes> {
    let c = commands
        .iter()
        .map(MmcCommand::rust)
        .collect::<PyResult<Vec<_>>>()?;
    write(|o| sx::write_commands(o, device_id, &c))
}

/// The bytes of an MMC response message from `device_id` (00H to 7EH).
#[pyfunction]
pub fn mmc_responses(device_id: u8, fields: Vec<MmcField>) -> PyResult<Bytes> {
    let f = fields
        .iter()
        .map(MmcField::rust)
        .collect::<PyResult<Vec<_>>>()?;
    write(|o| sx::write_responses(o, device_id, &f))
}

/// The name and data bytes of Information Fields, for the data of a WRITE command.
#[pyfunction]
pub fn mmc_fields(fields: Vec<MmcField>) -> PyResult<Bytes> {
    let f = fields
        .iter()
        .map(MmcField::rust)
        .collect::<PyResult<Vec<_>>>()?;
    write(|o| sx::write_info_fields(o, &f))
}

/// The short name (the name plus 20H) of a time code field 01H to 1FH, or None.
#[pyfunction]
pub fn mmc_short_name(field: u8) -> Option<u8> {
    sx::short_name(field)
}

/// The time code field of a short name 21H to 3FH, or None.
#[pyfunction]
pub fn mmc_full_name(short: u8) -> Option<u8> {
    sx::full_name(short)
}
