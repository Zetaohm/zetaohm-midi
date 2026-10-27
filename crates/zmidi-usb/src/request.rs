//! The request for the Group Terminal Block descriptors of a USB MIDI 2.0 Alternate Setting
//! (USB-MIDI v2.0 section 6 and Appendix B.10; DEC-014).
//!
//! - [`SetupPacket`]: the 8 bytes of a USB setup packet. Each 2-byte field is little-endian
//!   (USB 2.0 section 8.1, AMB-USB-MIDI-2-25).
//! - [`handle_gtb_request`]: the device-side handler. It identifies the request by
//!   bmRequestType 0x81, bRequest GET_DESCRIPTOR 0x06 and the wValue high byte CS_GR_TRM_BLOCK
//!   0x26 (REQ-USB-GTB-017). The wValue low byte is the Alternate Setting number, not a
//!   descriptor index. The 16-bit wIndex is compared with the MIDI Streaming interface number
//!   (AMB-USB-MIDI-2-38). The result is a [`GtbResponse`]: [`GtbResponse::Answered`] with the
//!   first min(wLength, wTotalLength) bytes of the set (REQ-USB-GTB-018),
//!   [`GtbResponse::Stall`] for Alternate Setting 0, a setting with no set or another interface
//!   (REQ-USB-GTB-019), and [`GtbResponse::NotHandled`] for every other setup packet.
//! - [`write_gtb_request`] and [`GtbRequest`]: the host side (REQ-USB-GTB-020). The host first
//!   writes a request with wLength 5, reads wTotalLength from the header in the answer, then
//!   writes a request with wLength equal to wTotalLength. It reads the answers with the Group
//!   Terminal Block set reader of [`crate::gtb`] and accepts an answer shorter than wLength.
//!
//! The module does not do a control transfer and does not send a stall: the USB stack of the
//! caller does the transfer stages and sends the stall (DEC-004, DEC-014). No alloc (DEC-002).
//! Nothing panics on received bytes. A setup packet that is not this request is a value
//! ([`GtbResponse::NotHandled`]), not an error.

use crate::constants::{CS_GR_TRM_BLOCK, GET_DESCRIPTOR};
use crate::device::{gtb_set_len, DeviceError, FunctionConfig};
use crate::gtb::{
    header_bytes, read_set, EndpointBlocks, GtbError, GtbReader, GtbSetInfo, HEADER_LEN,
};

/// The length of a USB setup packet (USB 2.0 section 9.3).
pub const SETUP_LEN: usize = 8;

/// bmRequestType of the request: device to host, standard, interface (section 6 request
/// table "0b10000001", AMB-USB-MIDI-2-10).
pub const GTB_REQUEST_TYPE: u8 = 0x81;

/// wLength of the first host request: the Group Terminal Block Header only (section 6,
/// Table B-23).
pub const HEADER_REQUEST_LENGTH: u16 = HEADER_LEN as u16;

/// Error of the request handler and of the request writer. Each variant names the broken rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    /// The caller buffer is too small; nothing is written (REQ-USB-GTB-018).
    #[error("buffer too small")]
    Overflow,
    /// Fewer than 8 bytes for a setup packet.
    #[error("setup packet shorter than 8 bytes")]
    Truncated,
    /// The answer to the first request has no readable Group Terminal Block Header, so it
    /// gives no wTotalLength (REQ-USB-GTB-020).
    #[error("no wTotalLength in the answer: {0}")]
    NoTotalLength(GtbError),
    /// The configuration of the device breaks a Group Terminal Block rule; nothing is written.
    #[error("{0}")]
    Device(DeviceError),
}

impl From<DeviceError> for RequestError {
    fn from(e: DeviceError) -> Self {
        match e {
            DeviceError::Overflow => RequestError::Overflow,
            e => RequestError::Device(e),
        }
    }
}

/// The 8 bytes of a USB setup packet (USB 2.0 section 9.3). Each 2-byte field is little-endian
/// on the bus (AMB-USB-MIDI-2-25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SetupPacket {
    /// bmRequestType: direction, type and recipient. 8 bits.
    pub request_type: u8,
    /// bRequest. 8 bits.
    pub request: u8,
    /// wValue. 16 bits.
    pub value: u16,
    /// wIndex. 16 bits.
    pub index: u16,
    /// wLength: the largest number of bytes of the data stage. 16 bits.
    pub length: u16,
}

impl SetupPacket {
    /// The setup packet from the parsed fields that a USB stack gives (review api M10).
    /// [`Self::request_type_from`] gives bmRequestType from its three parts.
    pub const fn from_parts(
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Self {
        SetupPacket {
            request_type,
            request,
            value,
            index,
            length,
        }
    }

    /// bmRequestType from its parts (USB 2.0 Table 9-2): D7 is 1 for device to host, D6..5
    /// the type (0 standard, 1 class, 2 vendor) and D4..0 the recipient (0 device,
    /// 1 interface, 2 endpoint, 3 other). The Group Terminal Block request is
    /// `request_type_from(true, 0, 1)` = 0x81.
    pub const fn request_type_from(device_to_host: bool, kind: u8, recipient: u8) -> u8 {
        ((device_to_host as u8) << 7) | ((kind & 0x03) << 5) | (recipient & 0x1F)
    }

    /// The setup packet of 8 bytes, 2-byte fields least significant byte first
    /// (REQ-USB-GTB-017).
    pub const fn from_bytes(b: &[u8; SETUP_LEN]) -> Self {
        SetupPacket {
            request_type: b[0],
            request: b[1],
            value: u16::from_le_bytes([b[2], b[3]]),
            index: u16::from_le_bytes([b[4], b[5]]),
            length: u16::from_le_bytes([b[6], b[7]]),
        }
    }

    /// The setup packet in the first 8 bytes of `bytes`; [`RequestError::Truncated`] when
    /// `bytes` is shorter.
    pub fn parse(bytes: &[u8]) -> Result<Self, RequestError> {
        let b: &[u8; SETUP_LEN] = bytes
            .get(..SETUP_LEN)
            .and_then(|s| s.try_into().ok())
            .ok_or(RequestError::Truncated)?;
        Ok(Self::from_bytes(b))
    }

    /// The 8 bytes, 2-byte fields least significant byte first (REQ-USB-GTB-020).
    pub const fn to_bytes(&self) -> [u8; SETUP_LEN] {
        let v = self.value.to_le_bytes();
        let i = self.index.to_le_bytes();
        let l = self.length.to_le_bytes();
        [
            self.request_type,
            self.request,
            v[0],
            v[1],
            i[0],
            i[1],
            l[0],
            l[1],
        ]
    }

    /// Writes the 8 bytes into `out` and gives 8; [`RequestError::Overflow`] and nothing
    /// written when `out` is shorter.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, RequestError> {
        let dst = out.get_mut(..SETUP_LEN).ok_or(RequestError::Overflow)?;
        dst.copy_from_slice(&self.to_bytes());
        Ok(SETUP_LEN)
    }

    /// The Group Terminal Block descriptors request (section 6): bmRequestType 0x81,
    /// bRequest 0x06, wValue 0x26 in the high byte and `alt` in the low byte, wIndex the
    /// interface number (high byte 0, AMB-USB-MIDI-2-38) and wLength `length`
    /// (REQ-USB-GTB-020).
    pub const fn gtb_request(alt: u8, interface: u8, length: u16) -> Self {
        SetupPacket {
            request_type: GTB_REQUEST_TYPE,
            request: GET_DESCRIPTOR,
            value: u16::from_le_bytes([alt, CS_GR_TRM_BLOCK]),
            index: interface as u16,
            length,
        }
    }

    /// The Alternate Setting number (the wValue low byte) when this is the Group Terminal
    /// Block descriptors request: bmRequestType 0x81, bRequest 0x06 and wValue high byte 0x26
    /// (REQ-USB-GTB-017). `None` for every other setup packet.
    pub const fn gtb_alternate_setting(&self) -> Option<u8> {
        let [alt, dtype] = self.value.to_le_bytes();
        if self.request_type == GTB_REQUEST_TYPE
            && self.request == GET_DESCRIPTOR
            && dtype == CS_GR_TRM_BLOCK
        {
            Some(alt)
        } else {
            None
        }
    }
}

/// The result of the device-side handler (REQ-USB-GTB-017 to REQ-USB-GTB-019).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GtbResponse {
    /// The handler wrote this number of bytes into the caller buffer: the data stage of the
    /// answer (REQ-USB-GTB-018).
    Answered(usize),
    /// The caller stalls the request (USB 2.0 9.2.7 Request Error). Nothing is written
    /// (REQ-USB-GTB-019).
    Stall,
    /// Not the Group Terminal Block descriptors request. Nothing is written; the caller
    /// handles the request in another way (REQ-USB-GTB-017).
    NotHandled,
}

/// The device-side handler of the Group Terminal Block descriptors request (section 6,
/// REQ-USB-GTB-017 to REQ-USB-GTB-019).
///
/// - Not bmRequestType 0x81, bRequest 0x06 and wValue high byte 0x26:
///   [`GtbResponse::NotHandled`].
/// - wIndex is not the MIDI Streaming interface of `cfg` (16-bit compare), the wValue low byte
///   is Alternate Setting 0, or `cfg` has no USB MIDI 2.0 setting with that number:
///   [`GtbResponse::Stall`].
/// - Else the handler writes the first min(wLength, wTotalLength) bytes of the set of that
///   setting (the header, then the block descriptors) into `out` and gives
///   [`GtbResponse::Answered`] with that number. wLength 5 gives only the header.
///
/// [`RequestError::Overflow`] when `out` is shorter than that number, and
/// [`RequestError::Device`] when the set of the setting breaks a Group Terminal Block rule;
/// in each case nothing is written. The handler writes nothing for `Stall` and `NotHandled`.
///
/// A USB stack that gives the parsed fields of the request calls [`handle_gtb_setup`] with
/// [`SetupPacket::from_parts`]; the result is the same.
pub fn handle_gtb_request<const J: usize, const S: usize, const E: usize, const B: usize>(
    setup: &[u8; SETUP_LEN],
    cfg: &FunctionConfig<'_, J, S, E, B>,
    out: &mut [u8],
) -> Result<GtbResponse, RequestError> {
    handle_gtb_setup(&SetupPacket::from_bytes(setup), cfg, out)
}

/// [`handle_gtb_request`] for a setup packet that the USB stack gives as parsed fields
/// ([`SetupPacket::from_parts`], review api M10). The same rules and the same results
/// (REQ-USB-GTB-017 to REQ-USB-GTB-019).
pub fn handle_gtb_setup<const J: usize, const S: usize, const E: usize, const B: usize>(
    setup: &SetupPacket,
    cfg: &FunctionConfig<'_, J, S, E, B>,
    out: &mut [u8],
) -> Result<GtbResponse, RequestError> {
    let Some(alt) = setup.gtb_alternate_setting() else {
        return Ok(GtbResponse::NotHandled);
    };
    if setup.index != u16::from(cfg.ms_interface) {
        return Ok(GtbResponse::Stall);
    }
    // Alternate Setting 0 and a number with no USB MIDI 2.0 setting have no set.
    let (Ok(total), Some(setting)) = (gtb_set_len(cfg, alt), cfg.midi2_setting(alt)) else {
        return Ok(GtbResponse::Stall);
    };
    // The same checks as `device::gtb_set` (GtbList::write), before any byte is written.
    setting
        .blocks
        .validate_endpoints(&setting.endpoints)
        .map_err(|error| RequestError::Device(DeviceError::Gtb { alt, error }))?;
    let total = u16::try_from(total).map_err(|_| {
        RequestError::Device(DeviceError::Gtb {
            alt,
            error: GtbError::TooManyBlocks,
        })
    })?;
    let n = usize::from(setup.length.min(total));
    let dst = out.get_mut(..n).ok_or(RequestError::Overflow)?;
    let mut w = PrefixWriter { dst, at: 0 };
    w.put(&header_bytes(total));
    for b in setting.blocks.iter() {
        w.put(&b.to_bytes());
    }
    Ok(GtbResponse::Answered(w.at))
}

/// Copies bytes into `dst` up to its end and drops the rest: the first bytes of the set.
struct PrefixWriter<'a> {
    dst: &'a mut [u8],
    at: usize,
}

impl PrefixWriter<'_> {
    fn put(&mut self, bytes: &[u8]) {
        let rest = self.dst.get_mut(self.at..).unwrap_or(&mut []);
        let k = rest.len().min(bytes.len());
        if let (Some(d), Some(s)) = (rest.get_mut(..k), bytes.get(..k)) {
            d.copy_from_slice(s);
            self.at += k;
        }
    }
}

/// Writes the 8 setup packet bytes of the Group Terminal Block descriptors request for
/// Alternate Setting `alt`, interface `interface` and wLength `length` into `out`
/// (REQ-USB-GTB-020): 0x81, 0x06, `alt`, 0x26, the interface number as 2 bytes and wLength as
/// 2 bytes, least significant byte first. [`RequestError::Overflow`] and nothing written when
/// `out` is shorter than 8.
pub fn write_gtb_request(
    alt: u8,
    interface: u8,
    length: u16,
    out: &mut [u8],
) -> Result<usize, RequestError> {
    SetupPacket::gtb_request(alt, interface, length).write(out)
}

/// The host-side procedure of section 6 for one Alternate Setting of one MIDI Streaming
/// interface (REQ-USB-GTB-020). The caller's USB stack sends the control transfers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GtbRequest {
    /// bAlternateSetting of the USB MIDI 2.0 setting (1 or more).
    pub alternate_setting: u8,
    /// bInterfaceNumber of the MIDI Streaming interface.
    pub interface: u8,
}

impl GtbRequest {
    /// The procedure for Alternate Setting `alternate_setting` of interface `interface`.
    pub const fn new(alternate_setting: u8, interface: u8) -> Self {
        GtbRequest {
            alternate_setting,
            interface,
        }
    }

    /// Step 1: writes the request with wLength 5 (the header only, Table B-23).
    pub fn write_header_request(&self, out: &mut [u8]) -> Result<usize, RequestError> {
        write_gtb_request(
            self.alternate_setting,
            self.interface,
            HEADER_REQUEST_LENGTH,
            out,
        )
    }

    /// Step 2: reads wTotalLength from the header in `header_answer` (the answer to step 1)
    /// with [`GtbReader`], and writes the request with wLength equal to wTotalLength
    /// (Table B-24). [`RequestError::NoTotalLength`] when the answer has no readable header.
    pub fn write_set_request(
        &self,
        header_answer: &[u8],
        out: &mut [u8],
    ) -> Result<usize, RequestError> {
        let total = answer_total_length(header_answer)?;
        write_gtb_request(self.alternate_setting, self.interface, total, out)
    }
}

/// wTotalLength of the Group Terminal Block Header at the start of `answer`, read with
/// [`GtbReader`] (REQ-USB-GTB-020). The answer may be only the 5-byte header.
/// [`RequestError::NoTotalLength`] when the header is missing or too short, and also for a
/// wTotalLength below 5 ([`GtbError::HeaderTooShort`]): a second request must ask for at
/// least the header.
pub fn answer_total_length(answer: &[u8]) -> Result<u16, RequestError> {
    let mut reader = GtbReader::new(answer);
    let first = reader.next();
    match (reader.total_length(), first) {
        (Some(t), _) => Ok(t),
        (None, Some(Err(e))) => Err(RequestError::NoTotalLength(e)),
        (None, _) => Err(RequestError::NoTotalLength(GtbError::NoHeader)),
    }
}

/// Reads an answer to the request with the Group Terminal Block set reader [`read_set`] and
/// checks it against the endpoints of the same Alternate Setting (REQ-USB-GTB-016,
/// REQ-USB-GTB-020). An answer shorter than wLength or than wTotalLength is accepted: the
/// complete blocks in it are given (choice U-24). A short answer (for example the header-only
/// answer to the first request) gives no [`GtbError::NoBlock`] and no endpoint problems,
/// because the blocks that the endpoints name can be in the missing part
/// ([`GtbSetInfo::is_short`]).
pub fn read_answer<const B: usize, E: EndpointBlocks>(
    answer: &[u8],
    endpoints: &[E],
) -> GtbSetInfo<B> {
    read_set(answer, endpoints)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::constants::{EndpointDirection, GtbType, MidiProtocol};
    use crate::device::{gtb_set, Midi2Setting};
    use crate::gtb::{GroupTerminalBlock, GtbList};
    use crate::ms2::{BusSpeed, EndpointConfig};
    use proptest::prelude::*;

    /// Table B-23: the first request (Alternate Setting 1, interface 1, wLength 5).
    const B23: [u8; 8] = [0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x05, 0x00];
    /// Table B-24: the second request (Alternate Setting 1, interface 1, wLength 18).
    const B24: [u8; 8] = [0x81, 0x06, 0x01, 0x26, 0x01, 0x00, 0x12, 0x00];
    /// Tables B-21 and B-22: the Group Terminal Block set of the example (18 bytes).
    const B21_B22: [u8; 18] = [
        0x05, 0x26, 0x01, 0x12, 0x00, // B-21 header, wTotalLength 0x0012
        0x0D, 0x26, 0x02, 0x01, 0x00, 0x00, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, // B-22
    ];

    /// The USB-MIDI v2.0 Appendix B example configuration: AudioControl interface 0, MIDI
    /// Streaming interface 1, Alternate Setting 1 with bulk OUT endpoint 1 (B-17) and interrupt
    /// IN endpoint 1, 64 bytes, bInterval 1 (B-19), and the block of Table B-22.
    fn v2_example() -> FunctionConfig<'static> {
        let mut cfg = FunctionConfig::<'static, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        let mut s = Midi2Setting::empty();
        s.endpoints
            .push(EndpointConfig::bulk(
                EndpointDirection::MidiOut,
                1,
                BusSpeed::Full,
            ))
            .unwrap();
        s.endpoints
            .push(EndpointConfig::interrupt(
                EndpointDirection::MidiIn,
                1,
                64,
                1,
            ))
            .unwrap();
        let block = GroupTerminalBlock {
            string_index: 4,
            protocol: MidiProtocol::UseMidiCi,
            max_input_bandwidth: 0x0001,
            max_output_bandwidth: 0x0000,
            ..GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 1)
        };
        s.blocks = GtbList::from_slice(&[block]).unwrap();
        assert_eq!(cfg.add_midi2_setting(s), Ok(1));
        cfg
    }

    /// A configuration with two blocks in Alternate Setting 1 (set of 31 bytes).
    fn two_block_config() -> FunctionConfig<'static, 4, 1, 2, 2> {
        let mut cfg = FunctionConfig::<'static, 4, 1, 2, 2>::midi1_only(0, 1, BusSpeed::Full);
        let mut s = Midi2Setting::new(BusSpeed::Full).unwrap();
        let b1 = GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 2);
        let b2 = GroupTerminalBlock::new(2, GtbType::Bidirectional, 2, 1);
        s.blocks = GtbList::from_slice(&[b1, b2]).unwrap();
        for ep in s.endpoints.as_mut_slice() {
            *ep = ep.with_blocks(&[1, 2]).unwrap();
        }
        assert_eq!(cfg.add_midi2_setting(s), Ok(1));
        cfg
    }

    #[test]
    fn table_b23_request_answers_the_header() {
        // REQ-USB-VEC2-012, REQ-USB-GTB-018: wLength 5 gives only the header B-21.
        let cfg = v2_example();
        let mut out = [0xAAu8; 32];
        assert_eq!(
            handle_gtb_request(&B23, &cfg, &mut out),
            Ok(GtbResponse::Answered(5))
        );
        assert_eq!(out[..5], [0x05, 0x26, 0x01, 0x12, 0x00]);
        assert!(out[5..].iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn table_b24_request_answers_the_full_set() {
        // REQ-USB-VEC2-013, REQ-USB-GTB-018: wLength 18 gives the 18 bytes of B-21 and B-22.
        let cfg = v2_example();
        let mut out = [0xAAu8; 32];
        assert_eq!(
            handle_gtb_request(&B24, &cfg, &mut out),
            Ok(GtbResponse::Answered(18))
        );
        assert_eq!(out[..18], B21_B22);
        assert!(out[18..].iter().all(|&b| b == 0xAA));
        // The same bytes as the set writer of the Device module.
        let mut set = [0u8; 18];
        assert_eq!(gtb_set(&cfg, 1, &mut set), Ok(18));
        assert_eq!(set, B21_B22);
    }

    #[test]
    fn host_writes_tables_b23_and_b24() {
        // REQ-USB-VEC2-012, REQ-USB-VEC2-013, REQ-USB-GTB-020: the two-step procedure of
        // Appendix B.10 for Alternate Setting 1 of interface 1.
        let req = GtbRequest::new(1, 1);
        let mut out = [0u8; 8];
        assert_eq!(req.write_header_request(&mut out), Ok(8));
        assert_eq!(out, B23);
        assert_eq!(write_gtb_request(1, 1, 5, &mut out), Ok(8));
        assert_eq!(out, B23);

        // The device answers the first request; the host reads wTotalLength from it.
        let cfg = v2_example();
        let mut answer = [0u8; 64];
        let Ok(GtbResponse::Answered(n)) = handle_gtb_request(&out, &cfg, &mut answer) else {
            panic!("no answer");
        };
        assert_eq!(answer_total_length(&answer[..n]), Ok(0x12));
        assert_eq!(req.write_set_request(&answer[..n], &mut out), Ok(8));
        assert_eq!(out, B24);
        assert_eq!(write_gtb_request(1, 1, 18, &mut out), Ok(8));
        assert_eq!(out, B24);

        // The device answers the second request; the host reads the set.
        let Ok(GtbResponse::Answered(n)) = handle_gtb_request(&out, &cfg, &mut answer) else {
            panic!("no answer");
        };
        let s = cfg.midi2_setting(1).unwrap();
        let info: GtbSetInfo<4> = read_answer(&answer[..n], &s.endpoints);
        assert_eq!(info.total_length, Some(18));
        assert_eq!(info.problem_count(), 0);
        assert_eq!(info.blocks.len(), 1);
        assert_eq!(info.blocks[0].string_index, 4);
    }

    #[test]
    fn host_accepts_an_answer_shorter_than_wlength() {
        // REQ-USB-GTB-020: the reader accepts a short answer and gives the complete blocks.
        let short = &B21_B22[..5];
        assert_eq!(answer_total_length(short), Ok(18));
        let s = v2_example();
        let s = s.midi2_setting(1).unwrap();
        let info: GtbSetInfo<4> = read_answer(&B21_B22, &s.endpoints);
        assert_eq!(info.blocks.len(), 1);
        // A device that answers 31 bytes (two blocks) cut at one block: one block is kept.
        let cfg = two_block_config();
        let mut set = [0u8; 31];
        assert_eq!(gtb_set(&cfg, 1, &mut set), Ok(31));
        let mut reader = GtbReader::new(&set[..18]);
        assert!(reader.next().unwrap().is_ok());
        assert!(reader.next().is_none());
        assert!(reader.is_short());
    }

    #[test]
    fn host_reads_the_header_only_answer_with_no_problems() {
        // REQ-USB-GTB-020 (review correctness L-2): the answer of Table B-23 read with
        // read_answer gives wTotalLength 18, no block and no problem, because it is short.
        let cfg = v2_example();
        let s = cfg.midi2_setting(1).unwrap();
        let info: GtbSetInfo<4> = read_answer(&B21_B22[..5], &s.endpoints);
        assert_eq!(info.total_length, Some(18));
        assert!(info.is_short());
        assert!(info.blocks.is_empty());
        assert_eq!(info.problems().count(), 0);
        // The full answer is not short.
        let info: GtbSetInfo<4> = read_answer(&B21_B22, &s.endpoints);
        assert!(!info.is_short() && info.is_clean());
    }

    #[test]
    fn host_refuses_a_total_length_below_the_header() {
        // REQ-USB-GTB-020 (review robustness L2): a wTotalLength below 5 gives no second
        // request, and nothing is written.
        let req = GtbRequest::new(1, 1);
        for total in 0u8..5 {
            let answer = [0x05, 0x26, 0x01, total, 0x00];
            assert_eq!(
                answer_total_length(&answer),
                Err(RequestError::NoTotalLength(GtbError::HeaderTooShort))
            );
            let mut out = [0xAAu8; 8];
            assert_eq!(
                req.write_set_request(&answer, &mut out),
                Err(RequestError::NoTotalLength(GtbError::HeaderTooShort))
            );
            assert!(out.iter().all(|&b| b == 0xAA));
        }
        assert_eq!(answer_total_length(&[0x05, 0x26, 0x01, 5, 0]), Ok(5));
    }

    #[test]
    fn handler_takes_parsed_fields_of_the_stack() {
        // REQ-USB-GTB-017, REQ-USB-GTB-018 (review api M10): the setup packet built from the
        // parsed fields of a USB stack gives the same answer as the 8 bytes.
        let cfg = v2_example();
        let request_type = SetupPacket::request_type_from(true, 0, 1);
        assert_eq!(request_type, GTB_REQUEST_TYPE);
        let setup = SetupPacket::from_parts(request_type, GET_DESCRIPTOR, 0x2601, 1, 18);
        assert_eq!(setup.to_bytes(), B24);
        let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
        assert_eq!(
            handle_gtb_setup(&setup, &cfg, &mut a),
            Ok(GtbResponse::Answered(18))
        );
        assert_eq!(
            handle_gtb_request(&B24, &cfg, &mut b),
            Ok(GtbResponse::Answered(18))
        );
        assert_eq!(a, b);
        // A class request to the interface is not handled.
        let other = SetupPacket::from_parts(
            SetupPacket::request_type_from(true, 1, 1),
            GET_DESCRIPTOR,
            0x2601,
            1,
            18,
        );
        assert_eq!(
            handle_gtb_setup(&other, &cfg, &mut a),
            Ok(GtbResponse::NotHandled)
        );
    }

    #[test]
    fn host_set_request_needs_a_header() {
        // REQ-USB-GTB-020: no wTotalLength without a readable header; nothing is written.
        let req = GtbRequest::new(1, 1);
        let mut out = [0xAAu8; 8];
        assert_eq!(
            req.write_set_request(&[], &mut out),
            Err(RequestError::NoTotalLength(GtbError::NoHeader))
        );
        assert_eq!(
            req.write_set_request(&[0x05, 0x24, 0x01, 0x12, 0x00], &mut out),
            Err(RequestError::NoTotalLength(GtbError::NoHeader))
        );
        assert_eq!(
            req.write_set_request(&[0x04, 0x26, 0x01, 0x12], &mut out),
            Err(RequestError::NoTotalLength(GtbError::HeaderTooShort))
        );
        assert!(out.iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn request_writer_refuses_a_short_buffer() {
        // REQ-USB-GTB-020: overflow, nothing written.
        let mut out = [0xAAu8; 7];
        assert_eq!(
            write_gtb_request(1, 1, 5, &mut out),
            Err(RequestError::Overflow)
        );
        assert!(out.iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn setup_fields_are_little_endian() {
        // REQ-USB-GTB-017, REQ-USB-GTB-020, AMB-USB-MIDI-2-25: least significant byte first.
        let p = SetupPacket::from_bytes(&[0x81, 0x06, 0x03, 0x26, 0x02, 0x01, 0x34, 0x12]);
        assert_eq!(p.request_type, 0x81);
        assert_eq!(p.request, 0x06);
        assert_eq!(p.value, 0x2603);
        assert_eq!(p.index, 0x0102);
        assert_eq!(p.length, 0x1234);
        assert_eq!(p.gtb_alternate_setting(), Some(3));
        assert_eq!(
            SetupPacket::parse(&B24),
            Ok(SetupPacket::gtb_request(1, 1, 18))
        );
        assert_eq!(SetupPacket::parse(&B24[..7]), Err(RequestError::Truncated));
    }

    #[test]
    fn handler_answers_min_of_wlength_and_wtotallength() {
        // REQ-USB-GTB-018: min(wLength, wTotalLength); a longer wLength gives the full set.
        let cfg = two_block_config();
        let mut full = [0u8; 31];
        assert_eq!(gtb_set(&cfg, 1, &mut full), Ok(31));
        for length in [0u16, 1, 5, 6, 17, 18, 30, 31, 32, 255, 0xFFFF] {
            let mut out = [0xAAu8; 64];
            let setup = SetupPacket::gtb_request(1, 1, length).to_bytes();
            let n = usize::from(length).min(31);
            assert_eq!(
                handle_gtb_request(&setup, &cfg, &mut out),
                Ok(GtbResponse::Answered(n))
            );
            assert_eq!(out[..n], full[..n]);
            assert!(out[n..].iter().all(|&b| b == 0xAA));
        }
    }

    #[test]
    fn handler_overflow_writes_nothing() {
        // REQ-USB-GTB-018: a caller buffer smaller than the answer gives an overflow error.
        let cfg = v2_example();
        let mut out = [0xAAu8; 17];
        assert_eq!(
            handle_gtb_request(&B24, &cfg, &mut out),
            Err(RequestError::Overflow)
        );
        assert!(out.iter().all(|&b| b == 0xAA));
        // The same buffer holds the 5-byte answer.
        assert_eq!(
            handle_gtb_request(&B23, &cfg, &mut out),
            Ok(GtbResponse::Answered(5))
        );
    }

    #[test]
    fn handler_stalls_alt0_unknown_setting_and_other_interface() {
        // REQ-USB-GTB-019: Stall and nothing written.
        let cfg = v2_example();
        let cases = [
            SetupPacket::gtb_request(0, 1, 18), // Alternate Setting 0 (USB-MIDI 1.0)
            SetupPacket::gtb_request(2, 1, 18), // no Alternate Setting 2
            SetupPacket::gtb_request(255, 1, 18),
            SetupPacket::gtb_request(1, 0, 18), // the AudioControl interface
            SetupPacket::gtb_request(1, 2, 18), // another interface
            SetupPacket {
                index: 0x0101, // 16-bit compare: high byte not 0 (AMB-USB-MIDI-2-38)
                ..SetupPacket::gtb_request(1, 1, 18)
            },
        ];
        for setup in cases {
            let mut out = [0xAAu8; 32];
            assert_eq!(
                handle_gtb_request(&setup.to_bytes(), &cfg, &mut out),
                Ok(GtbResponse::Stall),
                "{setup:?}"
            );
            assert!(out.iter().all(|&b| b == 0xAA));
        }
        // A plain USB-MIDI 1.0 function has no set at all.
        let cfg1 = FunctionConfig::<'static, 4, 1, 2, 1>::midi1_only(0, 1, BusSpeed::Full);
        assert_eq!(
            handle_gtb_request(&B24, &cfg1, &mut [0u8; 32]),
            Ok(GtbResponse::Stall)
        );
    }

    #[test]
    fn handler_ignores_other_requests() {
        // REQ-USB-GTB-017: only bmRequestType 0x81, bRequest 0x06, wValue high byte 0x26.
        let cfg = v2_example();
        let base = SetupPacket::gtb_request(1, 1, 18);
        let cases = [
            SetupPacket {
                request_type: 0x80, // to the device
                ..base
            },
            SetupPacket {
                request_type: 0xA1, // class request
                ..base
            },
            SetupPacket {
                request_type: 0x01, // host to device
                ..base
            },
            SetupPacket {
                request: 0x0A, // GET_INTERFACE
                ..base
            },
            SetupPacket {
                value: 0x2201, // HID report descriptor type
                ..base
            },
            SetupPacket {
                value: 0x0126, // byte order swapped: 0x26 in the low byte
                ..base
            },
        ];
        for setup in cases {
            let mut out = [0xAAu8; 32];
            assert_eq!(
                handle_gtb_request(&setup.to_bytes(), &cfg, &mut out),
                Ok(GtbResponse::NotHandled),
                "{setup:?}"
            );
            assert!(out.iter().all(|&b| b == 0xAA));
        }
    }

    #[test]
    fn handler_reads_alternate_setting_not_descriptor_index() {
        // REQ-USB-GTB-017: the wValue low byte selects the Alternate Setting, so settings 1
        // and 2 give their own sets.
        let mut cfg = FunctionConfig::<'static, 4, 2, 2, 2>::new(0, 3, BusSpeed::Full).unwrap();
        let mut s2 = Midi2Setting::new(BusSpeed::Full).unwrap();
        s2.blocks = GtbList::from_slice(&[
            GroupTerminalBlock::new(1, GtbType::Bidirectional, 0, 2),
            GroupTerminalBlock::new(2, GtbType::Bidirectional, 2, 1),
        ])
        .unwrap();
        for ep in s2.endpoints.as_mut_slice() {
            *ep = ep.with_blocks(&[1, 2]).unwrap();
        }
        assert_eq!(cfg.add_midi2_setting(s2), Ok(2));
        let mut out = [0u8; 64];
        let r1 = handle_gtb_request(
            &SetupPacket::gtb_request(1, 3, 0xFF).to_bytes(),
            &cfg,
            &mut out,
        );
        assert_eq!(r1, Ok(GtbResponse::Answered(18)));
        let r2 = handle_gtb_request(
            &SetupPacket::gtb_request(2, 3, 0xFF).to_bytes(),
            &cfg,
            &mut out,
        );
        assert_eq!(r2, Ok(GtbResponse::Answered(31)));
        assert_eq!(out[..5], [0x05, 0x26, 0x01, 0x1F, 0x00]);
    }

    #[test]
    fn handler_refuses_a_broken_set() {
        // REQ-USB-GTB-018 with the writer rules of the Device module: an endpoint that names
        // a block with no descriptor is an error, and nothing is written.
        let mut cfg = v2_example();
        for ep in cfg.midi2.as_mut_slice()[0].endpoints.as_mut_slice() {
            *ep = ep.with_blocks(&[7]).unwrap();
        }
        let mut out = [0xAAu8; 32];
        assert!(matches!(
            handle_gtb_request(&B24, &cfg, &mut out),
            Err(RequestError::Device(DeviceError::Gtb { alt: 1, .. }))
        ));
        assert!(out.iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn prop_setup_packet_round_trip() {
        // REQ-USB-GTB-017, REQ-USB-GTB-020: 8 bytes in, the same 8 bytes out.
        proptest!(|(b in any::<[u8; 8]>())| {
            let p = SetupPacket::from_bytes(&b);
            prop_assert_eq!(p.to_bytes(), b);
            let mut out = [0u8; 8];
            prop_assert_eq!(p.write(&mut out), Ok(8));
            prop_assert_eq!(out, b);
        });
    }

    #[test]
    fn prop_host_request_is_recognized_by_the_handler() {
        // REQ-USB-GTB-017, REQ-USB-GTB-020: the writer and the reader agree.
        proptest!(|(alt in any::<u8>(), interface in any::<u8>(), length in any::<u16>())| {
            let mut out = [0u8; 8];
            write_gtb_request(alt, interface, length, &mut out).unwrap();
            let p = SetupPacket::from_bytes(&out);
            prop_assert_eq!(p.gtb_alternate_setting(), Some(alt));
            prop_assert_eq!(p.index, u16::from(interface));
            prop_assert_eq!(p.length, length);
        });
    }

    #[test]
    fn prop_handler_never_panics_and_answers_a_prefix() {
        // REQ-USB-GTB-017 to REQ-USB-GTB-019: any 8 bytes and any buffer size; an answer is
        // the first bytes of the set, and every other result writes nothing.
        let cfg = two_block_config();
        let mut full = [0u8; 31];
        gtb_set(&cfg, 1, &mut full).unwrap();
        proptest!(|(b in any::<[u8; 8]>(), size in 0usize..40)| {
            let mut out = [0xAAu8; 40];
            match handle_gtb_request(&b, &cfg, &mut out[..size]) {
                Ok(GtbResponse::Answered(n)) => {
                    prop_assert_eq!(&out[..n], &full[..n]);
                    prop_assert!(out[n..].iter().all(|&x| x == 0xAA));
                }
                _ => prop_assert!(out.iter().all(|&x| x == 0xAA)),
            }
        });
    }

    #[test]
    fn prop_answer_total_length_never_panics() {
        // REQ-USB-GTB-020: the host reader is lenient on any answer bytes.
        proptest!(|(b in proptest::collection::vec(any::<u8>(), 0..40))| {
            let _ = answer_total_length(&b);
            let _: GtbSetInfo<4> = read_answer::<4, EndpointConfig>(&b, &[]);
        });
    }
}
