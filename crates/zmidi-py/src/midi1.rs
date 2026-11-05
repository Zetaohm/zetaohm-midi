//! MIDI 1.0 typed messages on bytes and the MIDI 1.0 receiver states (zmidi-io `Midi1Reader`,
//! `Midi1Writer`, `ModeState`, `ControllerState`, `ParameterState`, `SyncReceiver`, the Active
//! Sensing monitor and sender, and MPE; M1 v4.2.1, RP-015, RP-018, M1-100; DEC-022). A message is a `Midi1Message` with the bytes as on the wire, and no UMP Group.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use zmidi_io::{
    ActiveSensingMonitor as RsActiveSensingMonitor, ActiveSensingSender as RsActiveSensingSender,
    SyncCommand, SyncEvent as RsSyncEvent, SyncReceiver as RsSyncReceiver, SyncRun,
};
use zmidi_io::{
    ControllerEvent as RsControllerEvent, ControllerState as RsControllerState,
    Midi1Event as RsEvent, Midi1Message as RsMessage, Midi1Reader as RsReader,
    Midi1Writer as RsWriter, ModeAction as RsModeAction, ModeConfig, ModeState as RsModeState,
    Parameter as RsParameter, ParameterChange as RsChange, ParameterEvent as RsParameterEvent,
    ParameterState as RsParameterState,
};
use zmidi_io::{
    MpeClass, MpeEvent as RsMpeEvent, MpeLayout as RsMpeLayout, MpeMessage,
    MpeNoteAllocator as RsMpeNoteAllocator, MpeReceiver as RsMpeReceiver, MpeRole, MpeSensitivity,
    MpeZone,
};
use zmidi_ump::controller::Rpn;
use zmidi_ump::{Channel, Midi1ChannelVoiceMessage, SystemMessage};

use crate::invalid;

/// The Python text of an optional number: the number or `None`.
fn opt(v: Option<impl core::fmt::Display>) -> String {
    v.map_or_else(|| "None".into(), |v| v.to_string())
}

/// The Python text of a bool.
fn py_bool(b: bool) -> &'static str {
    if b {
        "True"
    } else {
        "False"
    }
}

/// Gives the Channel for 0 to 15.
fn channel(c: u8) -> PyResult<Channel> {
    Channel::new(c).map_err(|_| invalid("channel must be 0 to 15"))
}

/// One MIDI 1.0 message with the bytes as on the wire, and no UMP Group.
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct Midi1Message {
    /// "channel_voice", "system", "sysex" or "sysex_too_long".
    #[pyo3(get)]
    kind: &'static str,
    /// The status byte; F0H for a System Exclusive message.
    #[pyo3(get)]
    status: u8,
    /// The first data byte, or 0.
    #[pyo3(get)]
    data1: u8,
    /// The second data byte, or 0.
    #[pyo3(get)]
    data2: u8,
    /// The bytes of a System Exclusive message, F0H to F7H.
    sysex: Vec<u8>,
    /// The length of a System Exclusive message (also when it was too long for the reader).
    #[pyo3(get)]
    length: usize,
}

impl Midi1Message {
    /// The message of a Message Type 0x2 or 0x1 word: the status and data bytes are bytes 2 to 4.
    fn of_word(kind: &'static str, w: u32) -> Self {
        Self {
            kind,
            status: (w >> 16) as u8,
            data1: (w >> 8) as u8 & 0x7F,
            data2: w as u8 & 0x7F,
            sysex: Vec::new(),
            length: 0,
        }
    }

    fn channel_voice(m: Midi1ChannelVoiceMessage) -> Self {
        Self::of_word("channel_voice", m.into_bits())
    }

    fn of_event(e: RsEvent<'_>) -> Self {
        match e {
            RsEvent::Message(RsMessage::ChannelVoice(m)) => Self::channel_voice(m),
            RsEvent::Message(RsMessage::System(m)) => Self::of_word("system", m.into_bits()),
            RsEvent::Message(RsMessage::SysEx(s)) => Self {
                kind: "sysex",
                status: 0xF0,
                data1: 0,
                data2: 0,
                sysex: s.to_vec(),
                length: s.len(),
            },
            RsEvent::SysExTooLong { length } => Self {
                kind: "sysex_too_long",
                status: 0xF0,
                data1: 0,
                data2: 0,
                sysex: Vec::new(),
                length,
            },
        }
    }

    fn word(&self, message_type: u32) -> u32 {
        message_type << 28
            | u32::from(self.status) << 16
            | u32::from(self.data1) << 8
            | u32::from(self.data2)
    }

    /// The typed Channel Voice message, or an InvalidValueError.
    fn to_channel_voice(&self) -> PyResult<Midi1ChannelVoiceMessage> {
        (self.kind == "channel_voice")
            .then(|| Midi1ChannelVoiceMessage::parse(&[self.word(0x2)]))
            .flatten()
            .ok_or_else(|| invalid("the message is not a Channel Voice message"))
    }

    /// The typed message for the writer, or an InvalidValueError.
    fn to_typed(&self) -> PyResult<RsMessage<'_>> {
        match self.kind {
            "channel_voice" => self.to_channel_voice().map(RsMessage::ChannelVoice),
            "system" => SystemMessage::parse(&[self.word(0x1)])
                .map(RsMessage::System)
                .ok_or_else(|| invalid("not a System message")),
            "sysex" => Ok(RsMessage::SysEx(&self.sysex)),
            _ => Err(invalid("a message that is too long has no bytes to write")),
        }
    }
}

#[pymethods]
impl Midi1Message {
    /// A Channel Voice (status 80H to EFH) or System (F1H to FFH, not F7H) message. `data1` and
    /// `data2` are 0 to 127; a byte that the message does not have is ignored.
    #[new]
    #[pyo3(signature = (status, data1=0, data2=0))]
    fn new(status: u8, data1: u8, data2: u8) -> PyResult<Self> {
        if data1 > 0x7F || data2 > 0x7F {
            return Err(invalid("data bytes must be 0 to 127"));
        }
        let m = Self::of_word(
            "channel_voice",
            u32::from(status) << 16 | u32::from(data1) << 8 | u32::from(data2),
        );
        match status {
            0x80..=0xEF => Ok(Self::channel_voice(m.to_channel_voice()?)),
            0xF1..=0xF6 | 0xF8..=0xFF => SystemMessage::parse(&[m.word(0x1)])
                .map(|s| Self::of_word("system", s.into_bits()))
                .ok_or_else(|| invalid("not a defined System message")),
            _ => Err(invalid(
                "status must be 80H to EFH or F1H to FFH (not F7H); use Midi1Message.sysex",
            )),
        }
    }

    /// A System Exclusive message: `data` starts with F0H, ends with F7H, and has data bytes 0
    /// to 127 between them.
    #[staticmethod]
    fn sysex(data: &[u8]) -> PyResult<Self> {
        match data {
            [0xF0, inner @ .., 0xF7] if inner.iter().all(|b| *b < 0x80) => Ok(Self {
                kind: "sysex",
                status: 0xF0,
                data1: 0,
                data2: 0,
                sysex: data.to_vec(),
                length: data.len(),
            }),
            _ => Err(invalid(
                "a System Exclusive message is F0H, data bytes 0 to 127, F7H",
            )),
        }
    }

    /// The bytes of a System Exclusive message, F0H to F7H; `b""` for the other kinds.
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.sysex)
    }

    fn __repr__(&self) -> String {
        format!(
            "Midi1Message(kind={:?}, status=0x{:02X}, data1={}, data2={}, length={})",
            self.kind, self.status, self.data1, self.data2, self.length
        )
    }
}

/// Reads a MIDI 1.0 byte stream and gives typed messages; a System Exclusive message comes whole.
#[pyclass(module = "zetaohm_midi._native")]
pub struct Midi1Reader {
    inner: RsReader,
    sysex: Vec<u8>,
}

#[pymethods]
impl Midi1Reader {
    /// A reader that holds System Exclusive messages of up to `sysex_capacity` bytes (F0H to
    /// F7H); a longer one gives the kind "sysex_too_long" with its length.
    #[new]
    #[pyo3(signature = (sysex_capacity=256))]
    fn new(sysex_capacity: usize) -> Self {
        Self {
            inner: RsReader::new(),
            sysex: vec![0; sysex_capacity],
        }
    }

    /// Reads one byte and gives the messages that it completes (at most two).
    fn push(&mut self, byte: u8) -> Vec<Midi1Message> {
        let mut out = Vec::new();
        self.inner.feed(byte, &mut self.sysex, |e| {
            out.push(Midi1Message::of_event(e))
        });
        out
    }

    /// Reads `data` and gives all the messages that it completes, in order.
    fn feed(&mut self, data: &[u8]) -> Vec<Midi1Message> {
        let mut out = Vec::new();
        for b in data {
            self.inner
                .feed(*b, &mut self.sysex, |e| out.push(Midi1Message::of_event(e)));
        }
        out
    }

    /// The System Reset action of the program (AMB-M1-13): no running status, no partial message.
    fn reset(&mut self) {
        self.inner.reset();
    }
}

/// Writes typed MIDI 1.0 messages as bytes.
#[pyclass(module = "zetaohm_midi._native")]
pub struct Midi1Writer(RsWriter);

#[pymethods]
impl Midi1Writer {
    /// A writer, with running status when `running_status` is true.
    #[new]
    #[pyo3(signature = (running_status=false))]
    fn new(running_status: bool) -> Self {
        Self(RsWriter::new(running_status))
    }

    /// The bytes of `message`.
    fn write<'py>(
        &mut self,
        py: Python<'py>,
        message: &Midi1Message,
    ) -> PyResult<Bound<'py, PyBytes>> {
        let mut out = Vec::new();
        self.0
            .write_each(message.to_typed()?, |b| out.push(b))
            .map_err(|_| invalid("a System Exclusive message is F0H, data bytes 0 to 127, F7H"))?;
        Ok(PyBytes::new(py, &out))
    }

    /// The next Channel Voice message writes its status byte, also with running status.
    fn refresh_status(&mut self) {
        self.0.refresh_status();
    }
}

/// The action of one received message for a mode state.
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct ModeAction {
    /// "ignore", "voice", "global", "all_notes_off", "all_sound_off", "reset_all_controllers",
    /// "local_control" or "mode_change".
    #[pyo3(get)]
    kind: &'static str,
    /// The channel of an All Notes Off, All Sound Off or Reset All Controllers action in Mode 4,
    /// or None for all channels of the instrument.
    #[pyo3(get)]
    channel: Option<u8>,
    /// The Mode 4 voice of a voice message, or None.
    #[pyo3(get)]
    voice: Option<u8>,
    /// Local Control on.
    #[pyo3(get)]
    on: bool,
    /// The mode message changed the mode.
    #[pyo3(get)]
    changed: bool,
    /// The mode in use is an alternate mode.
    #[pyo3(get)]
    alternate: bool,
}

impl ModeAction {
    fn of(a: RsModeAction) -> Self {
        let mut m = Self {
            kind: "ignore",
            channel: None,
            voice: None,
            on: false,
            changed: false,
            alternate: false,
        };
        let ch = |c: Option<Channel>| c.map(Channel::index);
        match a {
            RsModeAction::Ignore => {}
            RsModeAction::Voice { voice } => {
                m.kind = "voice";
                m.voice = voice;
            }
            RsModeAction::Global => m.kind = "global",
            RsModeAction::AllNotesOff { channel } => {
                m.kind = "all_notes_off";
                m.channel = ch(channel);
            }
            RsModeAction::AllSoundOff { channel } => {
                m.kind = "all_sound_off";
                m.channel = ch(channel);
            }
            RsModeAction::ResetAllControllers { channel } => {
                m.kind = "reset_all_controllers";
                m.channel = ch(channel);
            }
            RsModeAction::LocalControl { on } => {
                m.kind = "local_control";
                m.on = on;
            }
            RsModeAction::ModeChange { changed, alternate } => {
                m.kind = "mode_change";
                m.changed = changed;
                m.alternate = alternate;
            }
        }
        m
    }
}

#[pymethods]
impl ModeAction {
    fn __repr__(&self) -> String {
        format!(
            "ModeAction(kind={:?}, channel={}, voice={}, on={}, changed={}, alternate={})",
            self.kind,
            opt(self.channel),
            opt(self.voice),
            py_bool(self.on),
            py_bool(self.changed),
            py_bool(self.alternate)
        )
    }
}

/// The MIDI 1.0 receiver mode state of one instrument (Modes 1 to 4, M1 pp. 20 to 26).
#[pyclass(module = "zetaohm_midi._native")]
pub struct ModeState(RsModeState);

#[pymethods]
impl ModeState {
    /// A mode state in Mode 1 with Local Control on. `implemented` lists the mode numbers that the
    /// instrument implements (Mode 1 always); `alternate` selects the alternate mode of p. 23 for
    /// the others (else the Mode message is ignored); `voices` is the count for M = 0.
    #[new]
    #[pyo3(signature = (basic_channel=0, implemented=vec![1, 2, 3, 4], alternate=true, voices=16, global_controllers=false))]
    fn new(
        basic_channel: u8,
        implemented: Vec<u8>,
        alternate: bool,
        voices: u8,
        global_controllers: bool,
    ) -> PyResult<Self> {
        let mut modes = [false; 4];
        for n in implemented {
            *modes
                .get_mut(usize::from(n).wrapping_sub(1))
                .ok_or_else(|| invalid("mode numbers are 1 to 4"))? = true;
        }
        Ok(Self(RsModeState::new(ModeConfig {
            basic_channel: channel(basic_channel)?,
            implemented: modes,
            alternate,
            voices,
            global_controllers,
        })))
    }

    /// Takes one Channel Voice message and gives its action.
    fn receive(&mut self, message: &Midi1Message) -> PyResult<ModeAction> {
        Ok(ModeAction::of(self.0.receive(&message.to_channel_voice()?)))
    }

    /// The System Reset action: Omni On, Poly and Local On; the Basic Channel and M stay.
    fn system_reset(&mut self) {
        self.0.system_reset();
    }

    /// The mode in use, 1 to 4.
    #[getter]
    fn mode(&self) -> u8 {
        self.0.mode().number()
    }

    /// The mode of the Omni and Mono flags, 1 to 4.
    #[getter]
    fn requested_mode(&self) -> u8 {
        self.0.requested_mode().number()
    }

    /// The mode in use is an alternate mode.
    #[getter]
    fn is_alternate(&self) -> bool {
        self.0.is_alternate()
    }

    /// The Basic Channel, 0 to 15.
    #[getter]
    fn basic_channel(&self) -> u8 {
        self.0.basic_channel().index()
    }

    #[setter]
    fn set_basic_channel(&mut self, c: u8) -> PyResult<()> {
        self.0.set_basic_channel(channel(c)?);
        Ok(())
    }

    /// M of the last Mono On.
    #[getter]
    fn m(&self) -> u8 {
        self.0.m()
    }

    /// Local Control on.
    #[getter]
    fn local(&self) -> bool {
        self.0.local()
    }

    #[setter]
    fn set_local(&mut self, on: bool) {
        self.0.set_local(on);
    }

    /// Sets the Omni flag, as Omni On or Omni Off does.
    fn set_omni(&mut self, on: bool) {
        self.0.set_omni(on);
    }

    /// Selects Poly, as Poly On does.
    fn set_poly(&mut self) {
        self.0.set_poly();
    }

    /// Selects Mono with M (0 to 127), as Mono On does.
    fn set_mono(&mut self, m: u8) -> PyResult<()> {
        if m > 0x7F {
            return Err(invalid("M must be 0 to 127"));
        }
        self.0.set_mono(m);
        Ok(())
    }
}

/// The event of one received message for a controller state.
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct ControllerEvent {
    /// "controller14", "controller7", "program", "portamento" or "reset_all_controllers".
    #[pyo3(get)]
    kind: &'static str,
    /// The controller number (0 to 31 for "controller14", 64 to 119 for "controller7").
    #[pyo3(get)]
    controller: Option<u8>,
    /// The 14-bit or 7-bit controller value.
    #[pyo3(get)]
    value: Option<u16>,
    /// The program number.
    #[pyo3(get)]
    program: Option<u8>,
    /// The 14-bit bank of a program event, or None before the first Bank Select.
    #[pyo3(get)]
    bank: Option<u16>,
    /// The Portamento Control source note.
    #[pyo3(get)]
    source: Option<u8>,
}

impl ControllerEvent {
    fn of(e: RsControllerEvent) -> Self {
        let mut c = Self {
            kind: "reset_all_controllers",
            controller: None,
            value: None,
            program: None,
            bank: None,
            source: None,
        };
        match e {
            RsControllerEvent::Controller14 { controller, value } => {
                c.kind = "controller14";
                c.controller = Some(controller);
                c.value = Some(value);
            }
            RsControllerEvent::Controller7 { controller, value } => {
                c.kind = "controller7";
                c.controller = Some(controller);
                c.value = Some(value.into());
            }
            RsControllerEvent::Program { program, bank } => {
                c.kind = "program";
                c.program = Some(program);
                c.bank = bank;
            }
            RsControllerEvent::Portamento { source } => {
                c.kind = "portamento";
                c.source = Some(source);
            }
            RsControllerEvent::ResetAllControllers => {}
        }
        c
    }
}

#[pymethods]
impl ControllerEvent {
    fn __repr__(&self) -> String {
        format!(
            "ControllerEvent(kind={:?}, controller={}, value={}, program={}, bank={}, source={})",
            self.kind,
            opt(self.controller),
            opt(self.value),
            opt(self.program),
            opt(self.bank),
            opt(self.source)
        )
    }
}

/// The MIDI 1.0 controller state of one channel: 14-bit pairs, bank and program, Portamento
/// Control, Reset All Controllers (M1 pp. 11 to 17, RP-015).
#[pyclass(module = "zetaohm_midi._native")]
pub struct ControllerState(RsControllerState);

#[pymethods]
impl ControllerState {
    #[new]
    fn new() -> Self {
        Self(RsControllerState::new())
    }

    /// Takes one Channel Voice message of the channel of this state and gives its event, or None.
    fn receive(&mut self, message: &Midi1Message) -> PyResult<Option<ControllerEvent>> {
        Ok(self
            .0
            .receive(&message.to_channel_voice()?)
            .map(ControllerEvent::of))
    }

    /// The Reset All Controllers action of the program (a ModeState action or General MIDI System
    /// On).
    fn reset_all_controllers(&mut self) {
        self.0.reset_all_controllers();
    }

    /// The held 14-bit bank, or None before the first Bank Select.
    #[getter]
    fn bank(&self) -> Option<u16> {
        self.0.bank()
    }
}

/// The RP-015 value of `controller` at Reset All Controllers, or None when the controller keeps
/// its value.
#[pyfunction]
pub fn reset_value(controller: u8) -> Option<u8> {
    zmidi_io::reset_value(controller)
}

/// The two messages of a 14-bit controller value (`controller` 0 to 31, `value` 0 to 3FFFH):
/// MSB, then LSB.
#[pyfunction]
pub fn controller14_messages(
    channel: u8,
    controller: u8,
    value: u16,
) -> PyResult<Vec<Midi1Message>> {
    zmidi_io::controller14_messages(self::channel(channel)?, controller, value)
        .map(|m| m.map(Midi1Message::channel_voice).to_vec())
        .map_err(|_| invalid("controller must be 0 to 31 and value 0 to 16383"))
}

/// The three messages of a bank (0 to 3FFFH) and program (0 to 127) selection: Bank Select MSB,
/// Bank Select LSB, Program Change.
#[pyfunction]
pub fn bank_program_messages(channel: u8, bank: u16, program: u8) -> PyResult<Vec<Midi1Message>> {
    zmidi_io::bank_program_messages(self::channel(channel)?, bank, program)
        .map(|m| m.map(Midi1Message::channel_voice).to_vec())
        .map_err(|_| invalid("bank must be 0 to 16383 and program 0 to 127"))
}

/// A parameter number: a Registered Parameter Number (controllers 101 and 100) or a
/// Non-Registered Parameter Number (controllers 99 and 98).
#[pyclass(frozen, eq, hash, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Parameter {
    /// The MSB, 0 to 127.
    #[pyo3(get)]
    msb: u8,
    /// The LSB, 0 to 127.
    #[pyo3(get)]
    lsb: u8,
    /// True for a Registered Parameter Number.
    #[pyo3(get)]
    registered: bool,
}

impl Parameter {
    fn of(p: RsParameter) -> Self {
        match p {
            RsParameter::Registered(rpn) => {
                let (msb, lsb) = rpn.parts();
                Self {
                    msb,
                    lsb,
                    registered: true,
                }
            }
            RsParameter::NonRegistered { msb, lsb } => Self {
                msb,
                lsb,
                registered: false,
            },
        }
    }

    fn rust(&self) -> RsParameter {
        if self.registered {
            RsParameter::Registered(Rpn::from_parts(self.msb, self.lsb))
        } else {
            RsParameter::NonRegistered {
                msb: self.msb,
                lsb: self.lsb,
            }
        }
    }
}

#[pymethods]
impl Parameter {
    /// A parameter number; `Parameter(0x7F, 0x7F)` is the null number, which ends a selection.
    #[new]
    #[pyo3(signature = (msb, lsb, registered=true))]
    fn new(msb: u8, lsb: u8, registered: bool) -> PyResult<Self> {
        if msb > 0x7F || lsb > 0x7F {
            return Err(invalid("parameter number bytes must be 0 to 127"));
        }
        Ok(Self {
            msb,
            lsb,
            registered,
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "Parameter(msb={}, lsb={}, registered={})",
            self.msb,
            self.lsb,
            py_bool(self.registered)
        )
    }
}

/// The event of a Data Entry, Data Increment or Data Decrement for the selected parameter.
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct ParameterEvent {
    /// The selected parameter.
    #[pyo3(get)]
    parameter: Parameter,
    /// "data_entry_msb", "data_entry_lsb", "increment" or "decrement".
    #[pyo3(get)]
    change: &'static str,
    /// The Data Entry byte, or None for a step.
    #[pyo3(get)]
    value: Option<u8>,
}

/// The zmidi-io change of `change` ("data_entry_msb", "data_entry_lsb", "increment",
/// "decrement") and the Data Entry byte `value`.
fn rust_change(change: &str, value: u8) -> PyResult<RsChange> {
    if value > 0x7F {
        return Err(invalid("value must be 0 to 127"));
    }
    match change {
        "data_entry_msb" => Ok(RsChange::DataEntryMsb(value)),
        "data_entry_lsb" => Ok(RsChange::DataEntryLsb(value)),
        "increment" => Ok(RsChange::Increment),
        "decrement" => Ok(RsChange::Decrement),
        _ => Err(invalid(
            "change must be data_entry_msb, data_entry_lsb, increment or decrement",
        )),
    }
}

impl ParameterEvent {
    fn of(e: RsParameterEvent) -> Self {
        let (change, value) = match e.change {
            RsChange::DataEntryMsb(v) => ("data_entry_msb", Some(v)),
            RsChange::DataEntryLsb(v) => ("data_entry_lsb", Some(v)),
            RsChange::Increment => ("increment", None),
            RsChange::Decrement => ("decrement", None),
        };
        Self {
            parameter: Parameter::of(e.parameter),
            change,
            value,
        }
    }
}

#[pymethods]
impl ParameterEvent {
    /// The new 14-bit value of the parameter from its current value (0 to 16383), with the RP-018
    /// steps: RPN 0 steps the cents with a carry at 100, RPN 2 to 4 step the MSB, the others the
    /// 14-bit value; a step stops at the limits.
    fn apply(&self, current: u16) -> PyResult<u16> {
        if current > 0x3FFF {
            return Err(invalid("value must be 0 to 16383"));
        }
        let change = rust_change(self.change, self.value.unwrap_or(0))?;
        Ok(RsParameterEvent {
            parameter: self.parameter.rust(),
            change,
        }
        .apply(current))
    }

    fn __repr__(&self) -> String {
        format!(
            "ParameterEvent(parameter={}, change={:?}, value={})",
            self.parameter.__repr__(),
            self.change,
            opt(self.value)
        )
    }
}

/// The MIDI 1.0 RPN and NRPN receiver of one channel (M1 p. 17, RP-015, RP-018). It holds no
/// parameter value: the program holds them and uses `ParameterEvent.apply`.
#[pyclass(module = "zetaohm_midi._native")]
pub struct ParameterState(RsParameterState);

#[pymethods]
impl ParameterState {
    /// A receiver with RPN reception on and NRPN reception off.
    #[new]
    fn new() -> Self {
        Self(RsParameterState::new())
    }

    /// Takes one Channel Voice message of the channel of this receiver and gives its event, or
    /// None.
    fn receive(&mut self, message: &Midi1Message) -> PyResult<Option<ParameterEvent>> {
        Ok(self
            .0
            .receive(&message.to_channel_voice()?)
            .map(ParameterEvent::of))
    }

    /// The Reset All Controllers action of the program: no parameter is selected.
    fn reset_all_controllers(&mut self) {
        self.0.reset_all_controllers();
    }

    /// The parameter that a Data Entry acts on now, or None.
    #[getter]
    fn selection(&self) -> Option<Parameter> {
        self.0.selection().map(Parameter::of)
    }

    /// RPN reception on.
    #[getter]
    fn rpn_reception(&self) -> bool {
        self.0.rpn_reception()
    }

    #[setter]
    fn set_rpn_reception(&mut self, on: bool) {
        self.0.set_rpn_reception(on);
    }

    /// NRPN reception on.
    #[getter]
    fn nrpn_reception(&self) -> bool {
        self.0.nrpn_reception()
    }

    #[setter]
    fn set_nrpn_reception(&mut self, on: bool) {
        self.0.set_nrpn_reception(on);
    }
}

/// The two messages of a parameter number: MSB, then LSB.
#[pyfunction]
pub fn select_messages(channel: u8, parameter: &Parameter) -> PyResult<Vec<Midi1Message>> {
    zmidi_io::select_messages(self::channel(channel)?, parameter.rust())
        .map(|m| m.map(Midi1Message::channel_voice).to_vec())
        .map_err(|_| invalid("parameter number bytes must be 0 to 127"))
}

/// The message of one change: "data_entry_msb" or "data_entry_lsb" with `value`, "increment" or
/// "decrement" (value 0).
#[pyfunction]
#[pyo3(signature = (channel, change, value=0))]
pub fn change_message(channel: u8, change: &str, value: u8) -> PyResult<Midi1Message> {
    zmidi_io::change_message(self::channel(channel)?, rust_change(change, value)?)
        .map(Midi1Message::channel_voice)
        .map_err(|_| invalid("value must be 0 to 127"))
}

/// The four messages of a parameter value (0 to 3FFFH): the number, then Data Entry MSB and LSB.
#[pyfunction]
pub fn value_messages(
    channel: u8,
    parameter: &Parameter,
    value: u16,
) -> PyResult<Vec<Midi1Message>> {
    zmidi_io::value_messages(self::channel(channel)?, parameter.rust(), value)
        .map(|m| m.map(Midi1Message::channel_voice).to_vec())
        .map_err(|_| invalid("value must be 0 to 16383"))
}

impl Midi1Message {
    /// The typed System message, or an InvalidValueError.
    fn to_system(&self) -> PyResult<SystemMessage> {
        (self.kind == "system")
            .then(|| SystemMessage::parse(&[self.word(0x1)]))
            .flatten()
            .ok_or_else(|| invalid("the message is not a System message"))
    }
}

/// The event of one message or command for a sync receiver.
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct SyncEvent {
    /// "start", "continue", "stop", "clock", "locate" or "song".
    #[pyo3(get)]
    kind: &'static str,
    /// The position in MIDI clocks of a "clock" event (the position of this clock) or a "locate"
    /// event (the new position).
    #[pyo3(get)]
    position: Option<u32>,
    /// The song number of a "song" event.
    #[pyo3(get)]
    song: Option<u8>,
}

impl SyncEvent {
    fn of(e: RsSyncEvent) -> Self {
        let (kind, position, song) = match e {
            RsSyncEvent::Start => ("start", None, None),
            RsSyncEvent::Continue => ("continue", None, None),
            RsSyncEvent::Stop => ("stop", None, None),
            RsSyncEvent::Clock { position } => ("clock", Some(position), None),
            RsSyncEvent::Locate { position } => ("locate", Some(position), None),
            RsSyncEvent::Song { number } => ("song", None, Some(number)),
        };
        Self {
            kind,
            position,
            song,
        }
    }
}

#[pymethods]
impl SyncEvent {
    fn __repr__(&self) -> String {
        format!(
            "SyncEvent(kind={:?}, position={}, song={})",
            self.kind,
            opt(self.position),
            opt(self.song)
        )
    }
}

/// The MIDI 1.0 sync receiver of a sequencer or drum machine: Start, Stop, Continue, Timing Clock,
/// Song Position Pointer and Song Select (M1 pp. 27 to 32). The position is a count of MIDI clocks.
#[pyclass(module = "zetaohm_midi._native")]
pub struct SyncReceiver(RsSyncReceiver);

#[pymethods]
impl SyncReceiver {
    /// A receiver that is stopped at position 0, with song 0, in MIDI sync mode.
    #[new]
    fn new() -> Self {
        Self(RsSyncReceiver::new())
    }

    /// Takes one System message and gives its event, or None.
    fn receive(&mut self, message: &Midi1Message) -> PyResult<Option<SyncEvent>> {
        Ok(self.0.receive(&message.to_system()?).map(SyncEvent::of))
    }

    /// Takes one command of the program ("start", "continue" or "stop") with the transitions of
    /// the MIDI command, and gives its event, or None.
    fn command(&mut self, command: &str) -> PyResult<Option<SyncEvent>> {
        let command = match command {
            "start" => SyncCommand::Start,
            "continue" => SyncCommand::Continue,
            "stop" => SyncCommand::Stop,
            _ => return Err(invalid("command must be start, continue or stop")),
        };
        Ok(self.0.command(command).map(SyncEvent::of))
    }

    /// The System Reset action of the program: position 0 and stopped; the song and the MIDI sync
    /// mode stay.
    fn system_reset(&mut self) {
        self.0.system_reset();
    }

    /// The position in MIDI clocks: the position of the next Timing Clock that plays.
    #[getter]
    fn position(&self) -> u32 {
        self.0.position()
    }

    /// The run state: "stopped", "waiting" (for the first Timing Clock) or "playing".
    #[getter]
    fn run(&self) -> &'static str {
        match self.0.run() {
            SyncRun::Stopped => "stopped",
            SyncRun::Waiting => "waiting",
            SyncRun::Playing => "playing",
        }
    }

    /// The song of the last accepted Song Select.
    #[getter]
    fn song(&self) -> u8 {
        self.0.song()
    }

    /// MIDI sync mode on (the default); out of it the receiver ignores Song Position Pointer and
    /// Song Select.
    #[getter]
    fn sync_mode(&self) -> bool {
        self.0.sync_mode()
    }

    #[setter]
    fn set_sync_mode(&mut self, on: bool) {
        self.0.set_sync_mode(on);
    }
}

/// The MIDI 1.0 Active Sensing monitor of a receiver (M1 pp. 32, 33). The program gives the time of
/// each call in microseconds. The defaults of the constructors are the zmidi-io constants (a test
/// checks them).
#[pyclass(module = "zetaohm_midi._native")]
pub struct ActiveSensingMonitor(RsActiveSensingMonitor);

#[pymethods]
impl ActiveSensingMonitor {
    /// A monitor with a timeout of `timeout_us` microseconds (330 ms by default), not armed.
    #[new]
    #[pyo3(signature = (timeout_us=330_000))]
    fn new(timeout_us: u32) -> Self {
        Self(RsActiveSensingMonitor::new(timeout_us))
    }

    /// Takes one received byte at the time `now_us`: FEH arms the monitor; while it is armed, each
    /// byte restarts the timer.
    fn receive_byte(&mut self, byte: u8, now_us: u64) {
        self.0.receive_byte(byte, now_us);
    }

    /// Takes one received message of a transport with no bytes at the time `now_us`;
    /// `active_sensing` is true for an Active Sensing message.
    fn receive(&mut self, active_sensing: bool, now_us: u64) {
        self.0.receive(active_sensing, now_us);
    }

    /// Checks the timer at the time `now_us`: True one time when more than the timeout passed since
    /// the last received byte; the monitor then waits for FEH again.
    fn poll(&mut self, now_us: u64) -> bool {
        self.0.poll(now_us)
    }

    /// The monitor received FEH and gave no timeout after it.
    #[getter]
    fn is_armed(&self) -> bool {
        self.0.is_armed()
    }

    /// The timeout, in microseconds.
    #[getter]
    fn timeout_us(&self) -> u32 {
        self.0.timeout_us()
    }
}

/// The MIDI 1.0 Active Sensing sender of a transmitter (M1 p. 32). The program gives the time of
/// each call in microseconds.
#[pyclass(module = "zetaohm_midi._native")]
pub struct ActiveSensingSender(RsActiveSensingSender);

#[pymethods]
impl ActiveSensingSender {
    /// A sender with an interval of `interval_us` microseconds (270 ms by default) that starts at
    /// time 0.
    #[new]
    #[pyo3(signature = (interval_us=270_000))]
    fn new(interval_us: u32) -> Self {
        Self(RsActiveSensingSender::new(interval_us))
    }

    /// The program sent a byte at the time `now_us`; the interval starts again.
    fn sent(&mut self, now_us: u64) {
        self.0.sent(now_us);
    }

    /// Checks the interval at the time `now_us`: True when no byte was sent for the interval; the
    /// program then sends one FEH byte, and the interval starts again.
    fn poll(&mut self, now_us: u64) -> bool {
        self.0.poll(now_us)
    }

    /// The interval, in microseconds.
    #[getter]
    fn interval_us(&self) -> u32 {
        self.0.interval_us()
    }
}

/// The zmidi-io Zone of "lower" or "upper".
fn mpe_zone(zone: &str) -> PyResult<MpeZone> {
    match zone {
        "lower" => Ok(MpeZone::Lower),
        "upper" => Ok(MpeZone::Upper),
        _ => Err(invalid("zone must be lower or upper")),
    }
}

/// The name of a Zone.
fn zone_name(zone: MpeZone) -> &'static str {
    match zone {
        MpeZone::Lower => "lower",
        MpeZone::Upper => "upper",
    }
}

/// A sensitivity of semitones and cents, 0 to 127 each.
fn mpe_sensitivity(semitones: u8, cents: u8) -> PyResult<MpeSensitivity> {
    if semitones > 0x7F || cents > 0x7F {
        return Err(invalid("semitones and cents must be 0 to 127"));
    }
    Ok(MpeSensitivity { semitones, cents })
}

/// An MPE Zone layout (M1-100 2.2.1): the Member Channel count of the Lower Zone (Manager Channel 1)
/// and of the Upper Zone (Manager Channel 16); 0 is an inactive Zone.
#[pyclass(frozen, eq, hash, module = "zetaohm_midi._native")]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MpeLayout(RsMpeLayout);

#[pymethods]
impl MpeLayout {
    /// A layout with `lower` and `upper` Member Channels (0 to 15 each; two active Zones have at
    /// most 14 in all).
    #[new]
    #[pyo3(signature = (lower=0, upper=0))]
    fn new(lower: u8, upper: u8) -> PyResult<Self> {
        RsMpeLayout::new(lower, upper)
            .map(Self)
            .map_err(|_| invalid("a Zone has 0 to 15 Member Channels, two Zones 14 in all"))
    }

    /// The Lower Zone with 15 Member Channels (2.2.2).
    #[classattr]
    #[allow(non_snake_case)]
    fn DEFAULT() -> Self {
        Self(RsMpeLayout::DEFAULT)
    }

    /// No Zone: MPE Mode off.
    #[classattr]
    #[allow(non_snake_case)]
    fn NONE() -> Self {
        Self(RsMpeLayout::NONE)
    }

    /// The Member Channels of the Lower Zone.
    #[getter]
    fn lower(&self) -> u8 {
        self.0.members(MpeZone::Lower)
    }

    /// The Member Channels of the Upper Zone.
    #[getter]
    fn upper(&self) -> u8 {
        self.0.members(MpeZone::Upper)
    }

    /// MPE Mode is on: at least one Zone is active.
    #[getter]
    fn is_active(&self) -> bool {
        self.0.is_active()
    }

    /// The layout after one MPE Configuration Message with `members` for `zone` (the newest MCM
    /// takes the channels; a value above 15 changes nothing).
    fn with_mcm(&self, zone: &str, members: u8) -> PyResult<Self> {
        Ok(Self(self.0.with_mcm(mpe_zone(zone)?, members)))
    }

    /// The role of `channel` (0 to 15): ("no_zone", None), ("manager", zone) or ("member", zone).
    fn role(&self, channel: u8) -> PyResult<(&'static str, Option<&'static str>)> {
        Ok(match self.0.role(self::channel(channel)?) {
            MpeRole::NoZone => ("no_zone", None),
            MpeRole::Manager(z) => ("manager", Some(zone_name(z))),
            MpeRole::Member(z) => ("member", Some(zone_name(z))),
        })
    }

    /// The lowest Member Channel of `zone`, which takes the Mode messages, or None.
    fn lowest_member(&self, zone: &str) -> PyResult<Option<u8>> {
        Ok(self.0.lowest_member(mpe_zone(zone)?).map(Channel::index))
    }

    fn __repr__(&self) -> String {
        format!("MpeLayout(lower={}, upper={})", self.lower(), self.upper())
    }
}

/// The event of one message for an MPE receiver.
#[pyclass(frozen, eq, module = "zetaohm_midi._native")]
#[derive(Clone, PartialEq, Eq)]
pub struct MpeEvent {
    /// "no_zone" (a conventional MIDI 1.0 message), "zone", "layout", "sensitivity", "mode" or
    /// "ignored".
    #[pyo3(get)]
    kind: &'static str,
    /// The Zone of a "zone", "sensitivity" or "mode" event.
    #[pyo3(get)]
    zone: Option<&'static str>,
    /// The Manager Channel ("zone") or the Manager sensitivity ("sensitivity").
    #[pyo3(get)]
    manager: bool,
    /// The channels to reset after a "layout" event: bit n is channel n.
    #[pyo3(get)]
    reset: u16,
}

impl MpeEvent {
    fn of(e: RsMpeEvent) -> Self {
        let (kind, zone, manager, reset) = match e {
            RsMpeEvent::NoZone => ("no_zone", None, false, 0),
            RsMpeEvent::Zone { zone, manager } => ("zone", Some(zone_name(zone)), manager, 0),
            RsMpeEvent::Layout { reset } => ("layout", None, false, reset),
            RsMpeEvent::Sensitivity { zone, manager } => {
                ("sensitivity", Some(zone_name(zone)), manager, 0)
            }
            RsMpeEvent::Mode { zone } => ("mode", Some(zone_name(zone)), false, 0),
            RsMpeEvent::Ignored => ("ignored", None, false, 0),
        };
        Self {
            kind,
            zone,
            manager,
            reset,
        }
    }
}

#[pymethods]
impl MpeEvent {
    fn __repr__(&self) -> String {
        format!(
            "MpeEvent(kind={:?}, zone={}, manager={}, reset={:#06x})",
            self.kind,
            self.zone
                .map_or_else(|| "None".into(), |z| format!("{z:?}")),
            py_bool(self.manager),
            self.reset
        )
    }
}

/// The MPE receiver of one MIDI 1.0 input (M1-100): the layout, the MIDI Mode and the Pitch Bend
/// Sensitivities of each Zone, and the last Pitch Bend, Channel Pressure and Control Change 74 of
/// each channel. Give each message with the event of the `ParameterState` of its channel.
#[pyclass(module = "zetaohm_midi._native")]
pub struct MpeReceiver(RsMpeReceiver);

#[pymethods]
impl MpeReceiver {
    /// A receiver with no Zone (MPE Mode off).
    #[new]
    fn new() -> Self {
        Self(RsMpeReceiver::new())
    }

    /// Takes one Channel Voice message and the event that the ParameterState of its channel gave
    /// for it (None for no event), and gives the MPE event.
    #[pyo3(signature = (message, parameter=None))]
    fn receive(
        &mut self,
        message: &Midi1Message,
        parameter: Option<&ParameterEvent>,
    ) -> PyResult<MpeEvent> {
        let parameter = match parameter {
            Some(p) => Some(RsParameterEvent {
                parameter: p.parameter.rust(),
                change: rust_change(p.change, p.value.unwrap_or(0))?,
            }),
            None => None,
        };
        Ok(MpeEvent::of(
            self.0.receive(&message.to_channel_voice()?, parameter),
        ))
    }

    /// Sets the layout of the program (power-up or front panel); gives the channels to reset (bit
    /// n is channel n).
    fn set_layout(&mut self, layout: &MpeLayout) -> u16 {
        self.0.set_layout(layout.0)
    }

    /// The layout.
    #[getter]
    fn layout(&self) -> MpeLayout {
        MpeLayout(self.0.layout())
    }

    /// The MIDI Mode of `zone`: 3 or 4.
    fn mode(&self, zone: &str) -> PyResult<u8> {
        Ok(self.0.mode(mpe_zone(zone)?))
    }

    /// The Manager Channel (`manager`) or Member Channel Pitch Bend Sensitivity of `zone`:
    /// (semitones, cents).
    fn sensitivity(&self, zone: &str, manager: bool) -> PyResult<(u8, u8)> {
        let s = self.0.sensitivity(mpe_zone(zone)?, manager);
        Ok((s.semitones, s.cents))
    }

    /// The last Pitch Bend of `channel`, 0 to 16383.
    fn pitch_bend(&self, channel: u8) -> PyResult<u16> {
        Ok(self.0.pitch_bend(self::channel(channel)?))
    }

    /// The last Channel Pressure of `channel`.
    fn pressure(&self, channel: u8) -> PyResult<u8> {
        Ok(self.0.pressure(self::channel(channel)?))
    }

    /// The last Control Change 74 of `channel`.
    fn brightness(&self, channel: u8) -> PyResult<u8> {
        Ok(self.0.brightness(self::channel(channel)?))
    }

    /// The pitch offset of `channel` in hundredths of a cent (the Manager plus the Member Pitch
    /// Bend), or None for a channel in no Zone.
    fn pitch_offset(&self, channel: u8) -> PyResult<Option<i32>> {
        Ok(self.0.pitch_offset(self::channel(channel)?))
    }
}

/// The C.3 Pitch Bend value (0 to 16383) of a sender for `offset` in hundredths of a cent at a
/// sensitivity of `semitones` and `cents`.
#[pyfunction]
#[pyo3(signature = (offset, semitones, cents=0))]
pub fn pitch_bend_value(offset: i32, semitones: u8, cents: u8) -> PyResult<u16> {
    Ok(zmidi_io::pitch_bend_value(
        offset,
        mpe_sensitivity(semitones, cents)?,
    ))
}

/// The C.5 pitch offset in hundredths of a cent of a received Pitch Bend `value` (0 to 16383) at a
/// sensitivity of `semitones` and `cents`.
#[pyfunction]
#[pyo3(signature = (value, semitones, cents=0))]
pub fn pitch_offset(value: u16, semitones: u8, cents: u8) -> PyResult<i32> {
    if value > 0x3FFF {
        return Err(invalid("value must be 0 to 16383"));
    }
    Ok(zmidi_io::pitch_offset(
        value,
        mpe_sensitivity(semitones, cents)?,
    ))
}

/// The Appendix E class ("mandatory", "optional" or "prohibited") of `message` on the Manager
/// Channel (`manager`) or a Member Channel, to send (`transmit`) or to receive, with Mode 4
/// (`mono`) for Program Change and Bank Select. `message` is "mcm", "pitch_bend_sensitivity",
/// "pitch_bend", "channel_pressure", "brightness", "poly_pressure", "all_sound_off",
/// "reset_all_controllers", "local_control", "all_notes_off", "omni_off", "omni_on",
/// "mono_mode_on", "poly_mode_on", "other_control_change", "other_rpn", "nrpn", "program_change",
/// "bank_select", "note" or "system".
#[pyfunction]
#[pyo3(signature = (message, manager, transmit, mono=false))]
pub fn mpe_class(
    message: &str,
    manager: bool,
    transmit: bool,
    mono: bool,
) -> PyResult<&'static str> {
    use MpeMessage as K;
    let kind = match message {
        "mcm" => K::Mcm,
        "pitch_bend_sensitivity" => K::PitchBendSensitivity,
        "pitch_bend" => K::PitchBend,
        "channel_pressure" => K::ChannelPressure,
        "brightness" => K::Brightness,
        "poly_pressure" => K::PolyPressure,
        "all_sound_off" => K::AllSoundOff,
        "reset_all_controllers" => K::ResetAllControllers,
        "local_control" => K::LocalControl,
        "all_notes_off" => K::AllNotesOff,
        "omni_off" => K::OmniOff,
        "omni_on" => K::OmniOn,
        "mono_mode_on" => K::MonoModeOn,
        "poly_mode_on" => K::PolyModeOn,
        "other_control_change" => K::OtherControlChange,
        "other_rpn" => K::OtherRpn,
        "nrpn" => K::Nrpn,
        "program_change" => K::ProgramChange,
        "bank_select" => K::BankSelect,
        "note" => K::Note,
        "system" => K::System,
        _ => return Err(invalid("unknown MPE message kind")),
    };
    Ok(match zmidi_io::mpe_class(kind, manager, transmit, mono) {
        MpeClass::Mandatory => "mandatory",
        MpeClass::Optional => "optional",
        MpeClass::Prohibited => "prohibited",
    })
}

/// The three messages of the MPE Configuration Message of `zone` with `members` (0 to 15).
#[pyfunction]
pub fn mcm_messages(zone: &str, members: u8) -> PyResult<Vec<Midi1Message>> {
    zmidi_io::mcm_messages(mpe_zone(zone)?, members)
        .map(|m| m.map(Midi1Message::channel_voice).to_vec())
        .map_err(|_| invalid("members must be 0 to 15"))
}

/// The three messages of RPN 0 with `semitones` (0 to 96) and no Data Entry LSB on `channel`.
#[pyfunction]
pub fn sensitivity_messages(channel: u8, semitones: u8) -> PyResult<Vec<Midi1Message>> {
    zmidi_io::sensitivity_messages(self::channel(channel)?, semitones)
        .map(|m| m.map(Midi1Message::channel_voice).to_vec())
        .map_err(|_| invalid("semitones must be 0 to 96"))
}

/// RPN 0 with `manager` semitones on the Manager Channel and with `member` semitones on each Member
/// Channel of `zone` (0 to 96 each; the Zone must be active).
#[pyfunction]
pub fn zone_sensitivity_messages(
    layout: &MpeLayout,
    zone: &str,
    manager: u8,
    member: u8,
) -> PyResult<Vec<Midi1Message>> {
    let blank = Midi1ChannelVoiceMessage::parse(&[0x20C0_0000]).expect("Program Change 0");
    let mut out = [blank; 48];
    let n =
        zmidi_io::zone_sensitivity_messages(layout.0, mpe_zone(zone)?, manager, member, &mut out)
            .map_err(|_| invalid("the Zone must be active and the semitones 0 to 96"))?;
    Ok(out[..n]
        .iter()
        .map(|m| Midi1Message::channel_voice(*m))
        .collect())
}

/// The Mode message of `zone` on its lowest Member Channel: Mono Mode On with M = the Member Channel
/// count (`mono`), or Poly Mode On.
#[pyfunction]
pub fn mode_message(layout: &MpeLayout, zone: &str, mono: bool) -> PyResult<Midi1Message> {
    zmidi_io::mode_message(layout.0, mpe_zone(zone)?, mono)
        .map(Midi1Message::channel_voice)
        .map_err(|_| invalid("the Zone must be active"))
}

/// The four messages that start a note on `channel`: Pitch Bend, Control Change 74, Channel
/// Pressure and Note On (velocity 1 to 127).
#[pyfunction]
#[pyo3(signature = (channel, note, velocity, pitch_bend=8192, brightness=64, pressure=0))]
pub fn note_messages(
    channel: u8,
    note: u8,
    velocity: u8,
    pitch_bend: u16,
    brightness: u8,
    pressure: u8,
) -> PyResult<Vec<Midi1Message>> {
    zmidi_io::note_messages(
        self::channel(channel)?,
        note,
        velocity,
        pitch_bend,
        brightness,
        pressure,
    )
    .map(|m| m.map(Midi1Message::channel_voice).to_vec())
    .map_err(|_| invalid("a value is out of range, or the velocity is 0"))
}

/// The Member Channel allocator of an MPE sender (M1-100 2.2.4.1, A.3): the fewest active notes,
/// then the oldest last Note Off; with `same_note`, first a free channel that last played the same
/// Note Number.
#[pyclass(module = "zetaohm_midi._native")]
pub struct MpeNoteAllocator(RsMpeNoteAllocator);

#[pymethods]
impl MpeNoteAllocator {
    /// An allocator with no active note.
    #[new]
    #[pyo3(signature = (same_note=false))]
    fn new(same_note: bool) -> Self {
        let mut a = RsMpeNoteAllocator::new();
        a.set_same_note(same_note);
        Self(a)
    }

    /// The Member Channel of `zone` for a new `note`, or None for an inactive Zone.
    fn note_on(&mut self, layout: &MpeLayout, zone: &str, note: u8) -> PyResult<Option<u8>> {
        if note > 0x7F {
            return Err(invalid("note must be 0 to 127"));
        }
        Ok(self
            .0
            .note_on(layout.0, mpe_zone(zone)?, note)
            .map(Channel::index))
    }

    /// One note on `channel` ended.
    fn note_off(&mut self, channel: u8) -> PyResult<()> {
        self.0.note_off(self::channel(channel)?);
        Ok(())
    }

    /// The count of active notes on `channel`.
    fn active_notes(&self, channel: u8) -> PyResult<u8> {
        Ok(self.0.active_notes(self::channel(channel)?))
    }

    /// The same-note option.
    #[getter]
    fn same_note(&self) -> bool {
        self.0.same_note()
    }

    #[setter]
    fn set_same_note(&mut self, on: bool) {
        self.0.set_same_note(on);
    }

    /// No active note and no age; the same-note option stays.
    fn reset(&mut self) {
        self.0.reset();
    }
}
