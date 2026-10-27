//! MIDI 1.0 byte stream to UMP and back (zmidi-io `BytesToUmp`, `UmpToBytes`; M2-104 Appendix D).

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use zmidi_io::{BytesToUmp as RsBytesToUmp, UmpToBytes as RsUmpToBytes};
use zmidi_ump::{Group, StreamProtocol};

use crate::invalid;

/// Gives the protocol for 1 (MIDI 1.0) or 2 (MIDI 2.0).
pub(crate) fn protocol(p: u8) -> PyResult<StreamProtocol> {
    match p {
        1 => Ok(StreamProtocol::Midi1),
        2 => Ok(StreamProtocol::Midi2),
        _ => Err(invalid("protocol must be 1 (MIDI 1.0) or 2 (MIDI 2.0)")),
    }
}

/// Gives the Group for 0 to 15.
pub(crate) fn group(g: u8) -> PyResult<Group> {
    Group::new(g).map_err(|_| invalid("group must be 0 to 15"))
}

/// Reads a MIDI 1.0 byte stream and gives UMPs on one Group.
#[pyclass(module = "zetaohm_midi._native")]
pub struct BytesToUmp(RsBytesToUmp);

#[pymethods]
impl BytesToUmp {
    /// A reader for `group` (0 to 15) that gives MIDI 1.0 (`protocol=1`) or MIDI 2.0
    /// (`protocol=2`) Protocol UMPs.
    #[new]
    #[pyo3(signature = (group=0, protocol=1))]
    fn new(group: u8, protocol: u8) -> PyResult<Self> {
        Ok(Self(RsBytesToUmp::new(
            self::group(group)?,
            self::protocol(protocol)?,
        )))
    }

    /// Reads one byte and gives the UMPs (each a list of words) that it completes.
    fn push(&mut self, byte: u8) -> Vec<Vec<u32>> {
        self.0.push(byte).map(|p| p.words().to_vec()).collect()
    }

    /// Reads `data` and gives all the UMPs that it completes, in order.
    fn feed(&mut self, data: &[u8]) -> Vec<Vec<u32>> {
        data.iter()
            .flat_map(|b| self.0.push(*b).map(|p| p.words().to_vec()))
            .collect()
    }
}

/// Writes UMPs as a MIDI 1.0 byte stream.
#[pyclass(module = "zetaohm_midi._native")]
pub struct UmpToBytes(RsUmpToBytes);

#[pymethods]
impl UmpToBytes {
    /// A writer, with MIDI 1.0 running status when `running_status` is true.
    #[new]
    #[pyo3(signature = (running_status=false))]
    fn new(running_status: bool) -> Self {
        Self(RsUmpToBytes::new(running_status))
    }

    /// The MIDI 1.0 bytes of one UMP; a UMP with no MIDI 1.0 byte form gives `b""`.
    fn write<'py>(&mut self, py: Python<'py>, words: Vec<u32>) -> Bound<'py, PyBytes> {
        let mut out = Vec::new();
        self.0.write_each(&words, |b| out.push(b));
        PyBytes::new(py, &out)
    }

    /// The next Channel Voice or Mode message writes its status byte (M1 Appendix A-2).
    fn refresh_status(&mut self) {
        self.0.refresh_status();
    }
}
