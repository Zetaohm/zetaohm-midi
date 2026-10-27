//! USB-MIDI 1.0 Event Packets and USB MIDI 2.0 UMP words (zmidi-usb `MidiLink`, one cable, one
//! translator), and the USB descriptor writer with the Group Terminal Block requests (zmidi-usb
//! `FunctionConfig` with the capacities of the C ABI, DEC-019 third addendum).

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use zmidi_usb::gtb::GtbReader;
use zmidi_usb::link::LinkMode;
use zmidi_usb::ms2::{GtbIds, TransferType};
use zmidi_usb::request::{answer_total_length, write_gtb_request, SETUP_LEN};
use zmidi_usb::{
    function_len, gtb_set, handle_gtb_request, write_function, BusSpeed, CableCounts, DeviceError,
    EndpointConfig, EndpointDirection, FunctionConfig, GroupMask, GroupTerminalBlock, GtbList,
    GtbResponse, GtbType, LinkConfig, LinkError, LinkEvent, Midi1Setting, Midi2Setting, MidiLink,
    MidiProtocol, RequestError, SettingNumber,
};

use crate::io::protocol;
use crate::{invalid, rejected};

/// The run-time state of one USB MIDI function for one endpoint pair.
#[pyclass(module = "zetaohm_midi._native")]
pub struct UsbLink(MidiLink<1, 1>);

#[pymethods]
impl UsbLink {
    /// A link at Alternate Setting 0. `host_side`: true on a USB host. `group_mask`: the Group
    /// Terminals of the received endpoint (bit n is Group n). `max_packet`: wMaxPacketSize of the
    /// sent endpoint (4 or more). `protocol`: the UMPs of the Event Packet reader (1 or 2).
    #[new]
    #[pyo3(signature = (host_side=false, group_mask=0xFFFF, max_packet=64, protocol=1))]
    fn new(host_side: bool, group_mask: u16, max_packet: u16, protocol: u8) -> PyResult<Self> {
        let config = LinkConfig {
            receive: if host_side {
                EndpointDirection::MidiIn
            } else {
                EndpointDirection::MidiOut
            },
            mask: GroupMask::from_bits(group_mask),
            max_packet,
            protocol: self::protocol(protocol)?,
            ..LinkConfig::default()
        };
        MidiLink::new(config)
            .map(Self)
            .map_err(|e| invalid(e.to_string()))
    }

    /// The host selected Alternate Setting `setting` (0: Event Packets; other: UMP) with the Group
    /// Terminal mask and wMaxPacketSize of that setting.
    fn set_alternate_setting(
        &mut self,
        setting: u8,
        group_mask: u16,
        max_packet: u16,
    ) -> PyResult<()> {
        self.0
            .report_setting(
                SettingNumber(setting),
                GroupMask::from_bits(group_mask),
                max_packet,
            )
            .map_err(|e| invalid(e.to_string()))
    }

    /// A USB reset: the link goes to Alternate Setting 0.
    fn usb_reset(&mut self) {
        self.0.report(LinkEvent::UsbReset);
    }

    /// A configuration change: the link goes to Alternate Setting 0.
    fn configuration_change(&mut self) {
        self.0.report(LinkEvent::ConfigurationChange);
    }

    /// The selected Alternate Setting.
    #[getter]
    fn setting(&self) -> u8 {
        self.0.setting().0
    }

    /// True at a USB MIDI 2.0 setting (UMP words).
    #[getter]
    fn is_ump(&self) -> bool {
        self.0.mode() == LinkMode::Ump
    }

    /// The size of the USB packets that the writer writes.
    #[getter]
    fn packet_size(&self) -> usize {
        self.0.packet_size()
    }

    /// True when the writer has bytes for the next packet.
    #[getter]
    fn pending(&self) -> bool {
        self.0.pending()
    }

    /// Reads the data of one received USB transfer. Gives `(umps, errors)`: the UMPs (each a list
    /// of words) and the count of errors that zmidi-usb reported.
    fn read(&mut self, data: &[u8]) -> (Vec<Vec<u32>>, usize) {
        let (mut umps, mut errors) = (Vec::new(), 0);
        self.0.read_with(data, |r| match r {
            Ok(rx) => {
                let (w, n) = rx.words();
                umps.push(w[..n].to_vec());
            }
            Err(LinkError::Tail { .. }) => {}
            Err(_) => errors += 1,
        });
        (umps, errors)
    }

    /// Writes one USB packet from `umps`. Gives `(packet, taken)`: the packet (`b""` when there is
    /// nothing to send) and the count of UMPs taken from `umps`.
    fn write_packet<'py>(
        &mut self,
        py: Python<'py>,
        umps: Vec<Vec<u32>>,
    ) -> PyResult<(Bound<'py, PyBytes>, usize)> {
        let mut out = vec![0u8; self.0.packet_size()];
        let mut iter = umps.iter().map(Vec::as_slice).peekable();
        let n = self
            .0
            .write_packet(&mut iter, &mut out)
            .map_err(|e| invalid(e.to_string()))?;
        let taken = umps.len() - iter.count();
        Ok((PyBytes::new(py, &out[..n]), taken))
    }
}

// --- USB descriptors and Group Terminal Block requests (zmidi-usb FunctionConfig) ---

/// The most USB MIDI 2.0 Alternate Settings of a `UsbFunction`: the capacities of the C ABI
/// (DEC-019 third addendum), so the three packages accept the same descriptions.
const MAX_SETTINGS: usize = 2;
/// The most endpoints of one USB MIDI 2.0 setting.
const MAX_ENDPOINTS: usize = 4;
/// The most Group Terminal Blocks of one setting, and block IDs of one endpoint.
const MAX_BLOCKS: usize = 16;
/// The largest Group Terminal Block set: the header and 16 blocks.
const GTB_SET_MAX: usize = 5 + 13 * MAX_BLOCKS;

/// The function configuration: the capacities above, no full Jack list.
type Function = FunctionConfig<'static, 0, MAX_SETTINGS, MAX_ENDPOINTS, MAX_BLOCKS>;

/// The block kinds by bGrpTrmBlkType (USB-MIDI v2.0 Table 5-6).
const BLOCK_KINDS: [&str; 3] = ["bidirectional", "input_only", "output_only"];

/// The exception of a zmidi-usb rule error: `InvalidValueError` for a setting that does not
/// exist, `RejectedError` for a broken rule.
fn device_error(e: DeviceError) -> PyErr {
    match e {
        DeviceError::NoSuchSetting(_) => invalid(e.to_string()),
        _ => rejected(format!("UsbFunction: {e}")),
    }
}

/// The exception of a request error.
fn request_error(e: RequestError) -> PyErr {
    match e {
        RequestError::Device(e) => device_error(e),
        RequestError::Truncated => invalid(e.to_string()),
        _ => rejected(e.to_string()),
    }
}

/// One Group Terminal Block of a USB MIDI 2.0 setting (USB-MIDI v2.0 5.4.2.1).
#[pyclass(frozen, module = "zetaohm_midi._native")]
#[derive(Debug, Clone)]
pub struct UsbBlock {
    /// bGrpTrmBlkID, 1 to 255.
    #[pyo3(get)]
    id: u8,
    /// bGrpTrmBlkType: "bidirectional", "input_only" or "output_only".
    #[pyo3(get)]
    kind: String,
    /// nGroupTrm: the first Group, 0 to 15 (0 is Group 1).
    #[pyo3(get)]
    first_group: u8,
    /// nNumGroupTrm: the number of Groups, 1 to 16.
    #[pyo3(get)]
    group_count: u8,
    /// iBlockItem: the string index (0: no string).
    #[pyo3(get)]
    string: u8,
    /// bMIDIProtocol (0x11: MIDI 2.0).
    #[pyo3(get)]
    protocol: u8,
    /// wMaxInputBandwidth in 4,000 bytes per second (0: not known).
    #[pyo3(get)]
    max_input_bandwidth: u16,
    /// wMaxOutputBandwidth in 4,000 bytes per second (0: not known).
    #[pyo3(get)]
    max_output_bandwidth: u16,
}

#[pymethods]
impl UsbBlock {
    /// A block. Raises `InvalidValueError` for a value out of range or another kind.
    #[new]
    #[pyo3(signature = (id, first_group, kind="bidirectional", group_count=1, string=0,
        protocol=0x11, max_input_bandwidth=0, max_output_bandwidth=0))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: u8,
        first_group: u8,
        kind: &str,
        group_count: u8,
        string: u8,
        protocol: u8,
        max_input_bandwidth: u16,
        max_output_bandwidth: u16,
    ) -> PyResult<Self> {
        if !BLOCK_KINDS.contains(&kind) {
            return Err(invalid(format!(
                "a block kind is \"bidirectional\", \"input_only\" or \"output_only\", not {kind:?}"
            )));
        }
        if first_group > 15 || group_count > 16 {
            return Err(invalid(
                "first_group is 0 to 15 and group_count is 0 to 16".to_string(),
            ));
        }
        Ok(Self {
            id,
            kind: kind.to_string(),
            first_group,
            group_count,
            string,
            protocol,
            max_input_bandwidth,
            max_output_bandwidth,
        })
    }
}

impl UsbBlock {
    fn block(&self) -> GroupTerminalBlock {
        let kind = BLOCK_KINDS
            .iter()
            .position(|k| *k == self.kind)
            .unwrap_or(0);
        GroupTerminalBlock {
            id: self.id,
            kind: GtbType::from_bits(kind as u8),
            first_group: self.first_group,
            num_groups: self.group_count,
            string_index: self.string,
            protocol: MidiProtocol::from_bits(self.protocol),
            max_input_bandwidth: self.max_input_bandwidth,
            max_output_bandwidth: self.max_output_bandwidth,
        }
    }
}

/// One endpoint of a USB MIDI 2.0 setting.
#[pyclass(frozen, module = "zetaohm_midi._native")]
#[derive(Debug, Clone)]
pub struct UsbEndpoint {
    /// "out": a MIDI OUT endpoint (the host sends); "in": a MIDI IN endpoint.
    #[pyo3(get)]
    direction: String,
    /// The endpoint number, 1 to 15.
    #[pyo3(get)]
    number: u8,
    /// "bulk" or "interrupt".
    #[pyo3(get)]
    transfer: String,
    /// bInterval (1 or more for an interrupt endpoint).
    #[pyo3(get)]
    interval: u8,
    /// wMaxPacketSize, or `None` for 64 at full speed and 512 at high speed.
    #[pyo3(get)]
    max_packet: Option<u16>,
    /// The IDs of the Group Terminal Blocks of the endpoint, 1 to 16.
    #[pyo3(get)]
    blocks: Vec<u8>,
}

#[pymethods]
impl UsbEndpoint {
    /// An endpoint. Raises `InvalidValueError` for a value out of range, another direction or
    /// transfer, or a block list that is empty or longer than 16.
    #[new]
    #[pyo3(signature = (direction, number, transfer="bulk", interval=0, max_packet=None, blocks=vec![1]))]
    fn new(
        direction: &str,
        number: u8,
        transfer: &str,
        interval: u8,
        max_packet: Option<u16>,
        blocks: Vec<u8>,
    ) -> PyResult<Self> {
        if direction != "out" && direction != "in" {
            return Err(invalid(format!(
                "an endpoint direction is \"out\" or \"in\", not {direction:?}"
            )));
        }
        if transfer != "bulk" && transfer != "interrupt" {
            return Err(invalid(format!(
                "an endpoint transfer is \"bulk\" or \"interrupt\", not {transfer:?}"
            )));
        }
        if number > 15 {
            return Err(invalid(format!(
                "the endpoint number is 0 to 15, not {number}"
            )));
        }
        if blocks.is_empty() || blocks.len() > MAX_BLOCKS {
            return Err(invalid(format!(
                "an endpoint names 1 to {MAX_BLOCKS} blocks, not {}",
                blocks.len()
            )));
        }
        Ok(Self {
            direction: direction.to_string(),
            number,
            transfer: transfer.to_string(),
            interval,
            max_packet,
            blocks,
        })
    }
}

/// One USB MIDI 2.0 Alternate Setting (1, then 2).
#[pyclass(frozen, module = "zetaohm_midi._native")]
#[derive(Debug, Clone)]
pub struct UsbSetting {
    /// The endpoints, 1 to 4.
    #[pyo3(get)]
    endpoints: Vec<UsbEndpoint>,
    /// The Group Terminal Blocks, 1 to 16, or `None` for one bidirectional block of Group 1.
    #[pyo3(get)]
    blocks: Option<Vec<UsbBlock>>,
    /// iInterface.
    #[pyo3(get)]
    string: u8,
}

#[pymethods]
impl UsbSetting {
    /// A setting. Raises `InvalidValueError` for more than 4 endpoints or 16 blocks.
    #[new]
    #[pyo3(signature = (endpoints, blocks=None, string=0))]
    fn new(
        endpoints: Vec<PyRef<'_, UsbEndpoint>>,
        blocks: Option<Vec<PyRef<'_, UsbBlock>>>,
        string: u8,
    ) -> PyResult<Self> {
        let endpoints: Vec<UsbEndpoint> = endpoints.iter().map(|e| (**e).clone()).collect();
        let blocks: Option<Vec<UsbBlock>> =
            blocks.map(|b| b.iter().map(|x| (**x).clone()).collect());
        if endpoints.len() > MAX_ENDPOINTS {
            return Err(invalid(format!(
                "a setting has at most {MAX_ENDPOINTS} endpoints, not {}",
                endpoints.len()
            )));
        }
        if blocks.as_ref().is_some_and(|b| b.len() > MAX_BLOCKS) {
            return Err(invalid(format!(
                "a setting has at most {MAX_BLOCKS} blocks"
            )));
        }
        Ok(Self {
            endpoints,
            blocks,
            string,
        })
    }
}

/// Alternate Setting 0 (USB-MIDI 1.0) as cable counts.
#[pyclass(frozen, module = "zetaohm_midi._native")]
#[derive(Debug, Clone)]
pub struct UsbAlt0 {
    /// iInterface.
    #[pyo3(get)]
    string: u8,
    /// The cables of the MIDI OUT endpoint.
    #[pyo3(get)]
    out_cables: u8,
    /// The cables of the MIDI IN endpoint.
    #[pyo3(get)]
    in_cables: u8,
    /// The number of the MIDI OUT endpoint.
    #[pyo3(get)]
    out_endpoint: u8,
    /// The number of the MIDI IN endpoint.
    #[pyo3(get)]
    in_endpoint: u8,
    /// wMaxPacketSize, or 0 for 64 at full speed and 512 at high speed.
    #[pyo3(get)]
    max_packet: u16,
}

#[pymethods]
impl UsbAlt0 {
    /// Alternate Setting 0. Raises `InvalidValueError` for an endpoint number above 15.
    #[new]
    #[pyo3(signature = (string=0, out_cables=1, in_cables=1, out_endpoint=1, in_endpoint=1, max_packet=0))]
    fn new(
        string: u8,
        out_cables: u8,
        in_cables: u8,
        out_endpoint: u8,
        in_endpoint: u8,
        max_packet: u16,
    ) -> PyResult<Self> {
        if out_endpoint > 15 || in_endpoint > 15 {
            return Err(invalid("an endpoint number is 0 to 15".to_string()));
        }
        Ok(Self {
            string,
            out_cables,
            in_cables,
            out_endpoint,
            in_endpoint,
            max_packet,
        })
    }
}

fn speed(high_speed: bool) -> BusSpeed {
    if high_speed {
        BusSpeed::High
    } else {
        BusSpeed::Full
    }
}

/// A USB MIDI function: the descriptors of the AudioControl and MIDI Streaming interfaces
/// (USB-MIDI v2.0 Appendix B), the endpoints for the USB stack, the Group Terminal masks for
/// `UsbLink`, and the Group Terminal Block requests (USB-MIDI v2.0 section 6).
#[pyclass(frozen, module = "zetaohm_midi._native")]
pub struct UsbFunction(Function);

impl UsbFunction {
    /// Checks the rules of zmidi-usb (the writer checks them all).
    fn checked(cfg: Function) -> PyResult<Self> {
        function_len(&cfg).map_err(device_error)?;
        Ok(Self(cfg))
    }
}

#[pymethods]
impl UsbFunction {
    /// A function with the AudioControl interface `ac_interface`, the MIDI Streaming interface
    /// `ms_interface`, Alternate Setting 0 `alt0` (default: one cable in each direction on
    /// endpoints 1) and 0 to 2 USB MIDI 2.0 `settings`. Raises `RejectedError` for a description
    /// that breaks a rule of zmidi-usb (the text names the rule), `InvalidValueError` for more
    /// than 2 settings.
    #[new]
    #[pyo3(signature = (ac_interface, ms_interface, high_speed=false, ac_string=0, alt0=None, settings=vec![]))]
    fn new(
        ac_interface: u8,
        ms_interface: u8,
        high_speed: bool,
        ac_string: u8,
        alt0: Option<PyRef<'_, UsbAlt0>>,
        settings: Vec<PyRef<'_, UsbSetting>>,
    ) -> PyResult<Self> {
        let alt0 = alt0.map(|a| (*a).clone());
        if settings.len() > MAX_SETTINGS {
            return Err(invalid(format!(
                "a function has at most {MAX_SETTINGS} USB MIDI 2.0 settings, not {}",
                settings.len()
            )));
        }
        let speed = speed(high_speed);
        let a = alt0.unwrap_or(UsbAlt0 {
            string: 0,
            out_cables: 1,
            in_cables: 1,
            out_endpoint: 1,
            in_endpoint: 1,
            max_packet: 0,
        });
        let mut cfg = Function::midi1_only(ac_interface, ms_interface, speed);
        cfg.ac_string = ac_string;
        let mut cables = CableCounts::new(a.out_cables, a.in_cables, speed);
        cables.out_endpoint = a.out_endpoint;
        cables.in_endpoint = a.in_endpoint;
        if a.max_packet != 0 {
            cables.max_packet = a.max_packet;
        }
        let mut midi1 = Midi1Setting::cables(cables);
        midi1.string_index = a.string;
        cfg.midi1 = Some(midi1);
        for s in settings.iter() {
            let mut setting = Midi2Setting::empty();
            setting.string_index = s.string;
            for e in &s.endpoints {
                let blocks = GtbIds::from_slice(&e.blocks)
                    .ok_or_else(|| invalid("an endpoint names 1 to 16 blocks".to_string()))?;
                let config = EndpointConfig {
                    direction: if e.direction == "out" {
                        EndpointDirection::MidiOut
                    } else {
                        EndpointDirection::MidiIn
                    },
                    number: e.number,
                    transfer: if e.transfer == "bulk" {
                        TransferType::Bulk
                    } else {
                        TransferType::Interrupt
                    },
                    max_packet: e.max_packet.unwrap_or(if high_speed { 512 } else { 64 }),
                    interval: e.interval,
                    blocks,
                };
                setting.endpoints.push(config).map_err(|_| {
                    invalid(format!("a setting has at most {MAX_ENDPOINTS} endpoints"))
                })?;
            }
            if let Some(blocks) = &s.blocks {
                let mut list = GtbList::new();
                for b in blocks {
                    list.push(b.block()).map_err(|_| {
                        invalid(format!("a setting has at most {MAX_BLOCKS} blocks"))
                    })?;
                }
                setting.blocks = list;
            }
            cfg.add_midi2_setting(setting).map_err(device_error)?;
        }
        Self::checked(cfg)
    }

    /// The default function (USB-MIDI v2.0 Appendix B): one cable in each direction, and one USB
    /// MIDI 2.0 setting with one bidirectional block of Group 1.
    #[staticmethod]
    #[pyo3(signature = (ac_interface, ms_interface, high_speed=false))]
    fn default_function(ac_interface: u8, ms_interface: u8, high_speed: bool) -> PyResult<Self> {
        Function::new(ac_interface, ms_interface, speed(high_speed))
            .map_err(device_error)
            .and_then(Self::checked)
    }

    /// The descriptors of the function (USB-MIDI v2.0 Appendix B order), for the Configuration
    /// descriptor.
    fn descriptors<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let n = function_len(&self.0).map_err(device_error)?;
        let mut out = vec![0; n];
        let n = write_function(&self.0, &mut out).map_err(device_error)?;
        Ok(PyBytes::new(py, &out[..n]))
    }

    /// The endpoints of every setting in descriptor order, for the USB stack: tuples of
    /// (alternate setting, bEndpointAddress, bmAttributes, bInterval, wMaxPacketSize).
    fn endpoints(&self) -> Vec<(u8, u8, u8, u8, u16)> {
        self.0
            .endpoints()
            .map(|e| {
                (
                    e.alternate_setting,
                    e.address(),
                    e.transfer.attributes(),
                    e.interval,
                    e.max_packet,
                )
            })
            .collect()
    }

    /// The Group Terminals of the endpoints of `direction` ("out" or "in") in Alternate Setting
    /// `setting` (bit n is Group n), for `UsbLink.set_alternate_setting`: 0xFFFF for Alternate
    /// Setting 0, 0 for a setting that does not exist.
    fn group_mask(&self, setting: u8, direction: &str) -> PyResult<u16> {
        let direction = match direction {
            "out" => EndpointDirection::MidiOut,
            "in" => EndpointDirection::MidiIn,
            _ => {
                return Err(invalid(format!(
                    "a direction is \"out\" or \"in\", not {direction:?}"
                )))
            }
        };
        if setting == 0 {
            return Ok(GroupMask::ALL.bits());
        }
        Ok(self.0.midi2_setting(setting).map_or(0, |s| {
            s.endpoints
                .iter()
                .filter(|e| e.direction == direction)
                .fold(0, |bits, e| {
                    bits | GroupMask::from_blocks(&s.blocks, e).bits()
                })
        }))
    }

    /// The Group Terminal Block descriptor set of USB MIDI 2.0 Alternate Setting `setting`.
    /// Raises `InvalidValueError` for Alternate Setting 0 or a setting that does not exist.
    fn gtb_set<'py>(&self, py: Python<'py>, setting: u8) -> PyResult<Bound<'py, PyBytes>> {
        let mut out = [0; GTB_SET_MAX];
        let n = gtb_set(&self.0, setting, &mut out).map_err(device_error)?;
        Ok(PyBytes::new(py, &out[..n]))
    }

    /// The device side of the Group Terminal Block request: what to do with the 8 bytes of a
    /// SETUP packet. Gives ("answered", the bytes to send), ("stall", b"") or ("not_handled",
    /// b"").
    fn answer_setup<'py>(
        &self,
        py: Python<'py>,
        setup: &[u8],
    ) -> PyResult<(&'static str, Bound<'py, PyBytes>)> {
        let setup: &[u8; SETUP_LEN] = setup.try_into().map_err(|_| {
            invalid(format!(
                "a SETUP packet has {SETUP_LEN} bytes, not {}",
                setup.len()
            ))
        })?;
        let mut out = [0; GTB_SET_MAX];
        Ok(
            match handle_gtb_request(setup, &self.0, &mut out).map_err(request_error)? {
                GtbResponse::Answered(n) => ("answered", PyBytes::new(py, &out[..n])),
                GtbResponse::Stall => ("stall", PyBytes::new(py, &[])),
                GtbResponse::NotHandled => ("not_handled", PyBytes::new(py, &[])),
            },
        )
    }
}

/// The host side: the 8 bytes of the SETUP packet of the Group Terminal Block request for
/// Alternate Setting `setting` of the interface `interface` with wLength `length`. Send wLength 5
/// first (the header), then the wTotalLength that `gtb_total_length` reads from the answer.
#[pyfunction]
pub fn gtb_request<'py>(
    py: Python<'py>,
    setting: u8,
    interface: u8,
    length: u16,
) -> PyResult<Bound<'py, PyBytes>> {
    let mut out = [0; SETUP_LEN];
    write_gtb_request(setting, interface, length, &mut out).map_err(request_error)?;
    Ok(PyBytes::new(py, &out))
}

/// The host side: wTotalLength of the Group Terminal Block Header at the start of `answer`.
/// Raises `RejectedError` when the answer has no header.
#[pyfunction]
pub fn gtb_total_length(answer: &[u8]) -> PyResult<u16> {
    answer_total_length(answer).map_err(request_error)
}

/// The host side: the Group Terminal Blocks of `answer`, read with the lenient reader of
/// zmidi-usb, as tuples of the raw field values (bGrpTrmBlkID, bGrpTrmBlkType, nGroupTrm,
/// nNumGroupTrm, iBlockItem, bMIDIProtocol, wMaxInputBandwidth, wMaxOutputBandwidth).
#[pyfunction]
#[allow(clippy::type_complexity)]
pub fn gtb_blocks(answer: &[u8]) -> Vec<(u8, u8, u8, u8, u8, u8, u16, u16)> {
    GtbReader::new(answer)
        .filter_map(Result::ok)
        .map(|b| {
            (
                b.id,
                b.kind.into_bits(),
                b.first_group,
                b.num_groups,
                b.string_index,
                b.protocol.into_bits(),
                b.max_input_bandwidth,
                b.max_output_bandwidth,
            )
        })
        .collect()
}
