//! BLE-MIDI v1.0 packets to UMP and back (zmidi-ble `PacketReader`, `PacketWriter`).

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use zmidi_ble::{PacketReader, PacketWriter, TimedUmp};

use crate::invalid;
use crate::io::{group, protocol};

/// Reads BLE-MIDI packets (sections 7 and 8) and gives UMPs with the 13-bit timestamp of their
/// message.
#[pyclass(module = "zetaohm_midi._native")]
pub struct BleReader(PacketReader);

#[pymethods]
impl BleReader {
    /// A reader that gives UMPs on `group` with `protocol` (1: MIDI 1.0, 2: MIDI 2.0).
    #[new]
    #[pyo3(signature = (group=0, protocol=1))]
    fn new(group: u8, protocol: u8) -> PyResult<Self> {
        Ok(Self(PacketReader::new(
            self::group(group)?,
            self::protocol(protocol)?,
        )))
    }

    /// Reads one packet. Gives `(umps, dropped)`: each UMP as `(timestamp_ms, words)`, and the
    /// count of the parts of the packet that the reader dropped.
    fn read(&mut self, packet: &[u8]) -> (Vec<(u16, Vec<u32>)>, usize) {
        let (mut umps, mut dropped) = (Vec::new(), 0);
        self.0.read(packet, |r| match r {
            Ok(rx) => {
                let mut w = [0u32; 4];
                let n = rx.ump.write(&mut w).unwrap_or(0);
                umps.push((rx.timestamp.millis(), w[..n].to_vec()));
            }
            Err(_) => dropped += 1,
        });
        (umps, dropped)
    }

    /// Drops an open SysEx message and the last timestamp, for example after a new connection.
    fn reset(&mut self) {
        self.0.reset();
    }
}

/// Writes BLE-MIDI packets from timed UMPs.
#[pyclass(module = "zetaohm_midi._native")]
pub struct BleWriter(PacketWriter);

#[pymethods]
impl BleWriter {
    /// A writer for the ATT_MTU `att_mtu` (23 or more); the packet length is `att_mtu - 3`.
    #[new]
    #[pyo3(signature = (att_mtu=23, running_status=true))]
    fn new(att_mtu: u16, running_status: bool) -> PyResult<Self> {
        let mut w = PacketWriter::new(att_mtu).map_err(|e| invalid(e.to_string()))?;
        w.set_running_status(running_status);
        Ok(Self(w))
    }

    /// Changes the ATT_MTU (23 or more).
    fn set_att_mtu(&mut self, att_mtu: u16) -> PyResult<()> {
        self.0
            .set_att_mtu(att_mtu)
            .map_err(|e| invalid(e.to_string()))
    }

    /// The packet length, `att_mtu - 3`.
    #[getter]
    fn packet_len(&self) -> usize {
        self.0.packet_len()
    }

    /// True when the writer holds bytes for the next packet.
    #[getter]
    fn has_held(&self) -> bool {
        self.0.has_held()
    }

    /// Drops the held bytes and the state.
    fn reset(&mut self) {
        self.0.reset();
    }

    /// Writes one packet from the held bytes and then from `umps` (each `(millis, words)`). Gives
    /// `(packet, taken)`: the packet (`b""` when there is nothing to send) and the count of UMPs
    /// taken from `umps`.
    fn write_packet<'py>(
        &mut self,
        py: Python<'py>,
        umps: Vec<(u64, Vec<u32>)>,
    ) -> PyResult<(Bound<'py, PyBytes>, usize)> {
        let mut out = vec![0u8; self.0.packet_len()];
        let mut iter = umps
            .iter()
            .map(|(millis, words)| TimedUmp {
                millis: *millis,
                words,
            })
            .peekable();
        let n = self
            .0
            .write_packet(&mut iter, &mut out)
            .map_err(|e| invalid(e.to_string()))?;
        let taken = umps.len() - iter.count();
        Ok((PyBytes::new(py, &out[..n]), taken))
    }
}
