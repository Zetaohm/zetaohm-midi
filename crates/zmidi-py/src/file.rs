//! MIDI Clip Files (zmidi-file `ClipReader`, `ClipWriter`; M2-116).

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use zmidi_file::{
    ClipConfig, ClipReader, ClipSection, ClipWriter as RsClipWriter, ProfileDestination, ProfileOn,
};
use zmidi_ump::{SetTempo, SetTimeSignature};

use crate::io::group;
use crate::{invalid, malformed, rejected};

/// Reads all messages of a MIDI Clip File. Gives a list of `(section, ticks, words)`: section 0
/// is the Clip Configuration Header, 1 the Clip Sequence Data. Raises `MalformedError` for a bad
/// file.
#[pyfunction]
pub fn read_clip(data: &[u8]) -> PyResult<Vec<(u8, u64, Vec<u32>)>> {
    let reader = ClipReader::new(data).map_err(|e| malformed(e.to_string()))?;
    let mut out = Vec::new();
    for ev in reader {
        let ev = ev.map_err(|e| malformed(e.to_string()))?;
        let mut w = [0u32; 4];
        let n = ev.ump.write(&mut w).unwrap_or(0);
        out.push((
            u8::from(ev.section == ClipSection::Sequence),
            ev.ticks,
            w[..n].to_vec(),
        ));
    }
    Ok(out)
}

/// One step of a clip.
enum Step {
    Config(Vec<u32>),
    Start(u64, Option<u32>),
    Event(u64, Vec<u32>),
}

/// Writes a MIDI Clip File. Each call is checked at once against the rules of zmidi-file; `finish`
/// gives the bytes.
#[pyclass(module = "zetaohm_midi._native")]
pub struct ClipWriter {
    tpq: u16,
    group: u8,
    tempo: Option<u32>,
    time_signature: Option<(u8, u8, u8)>,
    profiles: Vec<ProfileOn>,
    steps: Vec<Step>,
}

/// Bytes for the File Header, the configuration and `steps`: 8 + 16 bytes per message and a
/// reserve.
fn capacity(w: &ClipWriter) -> usize {
    8 + 64 * (w.profiles.len() + 4) + 32 * (w.steps.len() + 4)
}

impl ClipWriter {
    /// Runs all steps (and `finish` at `end`) on a zmidi-file writer; gives the bytes.
    fn replay(&self, end: Option<u64>) -> PyResult<Vec<u8>> {
        let g = group(self.group)?;
        let sig = match self.time_signature {
            Some((n, d, t)) => {
                Some(SetTimeSignature::try_new(g, n, d, t).map_err(|e| invalid(e.to_string()))?)
            }
            None => None,
        };
        let cfg = ClipConfig {
            ticks_per_quarter_note: self.tpq,
            profiles: &self.profiles,
            tempo: self.tempo.map(|t| SetTempo::new(g, t)),
            time_signature: sig,
            group: g,
        };
        let mut buf = vec![0u8; capacity(self)];
        let map = |e: zmidi_file::ClipWriteError| match e {
            zmidi_file::ClipWriteError::InvalidConfig
            | zmidi_file::ClipWriteError::InvalidMessage => invalid(e.to_string()),
            _ => rejected(e.to_string()),
        };
        let mut w = RsClipWriter::new(&mut buf, &cfg).map_err(map)?;
        for s in &self.steps {
            match s {
                Step::Config(words) => w.config_event(words).map_err(map)?,
                Step::Start(tick, tempo) => w
                    .start(*tick, tempo.map(|t| SetTempo::new(g, t)), None)
                    .map_err(map)?,
                Step::Event(tick, words) => w.event(*tick, words).map_err(map)?,
            }
        }
        let n = match end {
            Some(tick) => w.finish(tick).map_err(map)?,
            None => w.len(),
        };
        buf.truncate(n);
        Ok(buf)
    }

    /// Adds `step` when the steps with it are valid.
    fn push(&mut self, step: Step) -> PyResult<()> {
        self.steps.push(step);
        if let Err(e) = self.replay(None) {
            self.steps.pop();
            return Err(e);
        }
        Ok(())
    }
}

#[pymethods]
impl ClipWriter {
    /// A clip with Ticks Per Quarter Note `ticks_per_quarter_note`, Flex Data on `group`, an
    /// optional Set Tempo (units of 10 ns per quarter note), an optional Set Time Signature
    /// `(numerator, denominator_power_of_2, thirty_second_notes)` and Set Profile On messages
    /// `(profile_id: bytes of 5, destination: "channel" | "group" | "function_block", channel,
    /// channels)`.
    #[new]
    #[pyo3(signature = (ticks_per_quarter_note=96, group=0, tempo_10ns=None, time_signature=None, profiles=Vec::new()))]
    fn new(
        ticks_per_quarter_note: u16,
        group: u8,
        tempo_10ns: Option<u32>,
        time_signature: Option<(u8, u8, u8)>,
        profiles: Vec<(Vec<u8>, String, u8, u16)>,
    ) -> PyResult<Self> {
        let mut ps = Vec::new();
        for (id, dest, channel, channels) in profiles {
            let profile_id: [u8; 5] = id
                .try_into()
                .map_err(|_| invalid("a Profile ID has 5 bytes"))?;
            let destination = match dest.as_str() {
                "channel" => ProfileDestination::Channel(channel),
                "group" => ProfileDestination::Group,
                "function_block" => ProfileDestination::FunctionBlock,
                _ => {
                    return Err(invalid(
                        "destination must be channel, group or function_block",
                    ))
                }
            };
            ps.push(ProfileOn {
                profile_id,
                destination,
                channels,
            });
        }
        let w = Self {
            tpq: ticks_per_quarter_note,
            group,
            tempo: tempo_10ns,
            time_signature,
            profiles: ps,
            steps: Vec::new(),
        };
        w.replay(None)?;
        Ok(w)
    }

    /// Writes one more message of the Clip Configuration Header.
    fn config_event(&mut self, words: Vec<u32>) -> PyResult<()> {
        self.push(Step::Config(words))
    }

    /// Writes Start of Clip at `tick`, with an optional Set Tempo, and starts the Clip Sequence
    /// Data.
    #[pyo3(signature = (tick=0, tempo_10ns=None))]
    fn start(&mut self, tick: u64, tempo_10ns: Option<u32>) -> PyResult<()> {
        self.push(Step::Start(tick, tempo_10ns))
    }

    /// Writes one message of the Clip Sequence Data at absolute `tick`.
    fn event(&mut self, tick: u64, words: Vec<u32>) -> PyResult<()> {
        self.push(Step::Event(tick, words))
    }

    /// Writes End of Clip at `tick` and gives the bytes of the clip.
    fn finish<'py>(&self, py: Python<'py>, tick: u64) -> PyResult<Bound<'py, PyBytes>> {
        Ok(PyBytes::new(py, &self.replay(Some(tick))?))
    }
}
