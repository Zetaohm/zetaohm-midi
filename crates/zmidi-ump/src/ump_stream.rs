//! UMP Stream Messages: Message Type 0xF (M2-104-UM 7.1, Figures 10 to 25).
//!
//! - Format: one 128-bit packet. `mt=F` (bits 127..124), Format (123..122), Status (121..112),
//!   then data (Figure 10). The messages are addressed to the UMP Endpoint: they have no Group
//!   and no Channel (7.1, REQ-UMP-009).
//! - Format (Figure 10): 0x0 Complete, 0x1 Start, 0x2 Continue, 0x3 End. The fixed-size
//!   messages are Complete only (`f=0` in Figures 11 to 25). The text messages use all four.
//! - Text messages: Endpoint Name (Status 0x003, UTF-8, at most 98 bytes, 14 bytes in each UMP),
//!   Product Instance Id (Status 0x004, ASCII 32 to 126, at most 42 bytes, 14 bytes in each UMP)
//!   and Function Block Name (Status 0x012, UTF-8, at most 91 bytes, 13 bytes in each UMP after
//!   the Function Block Number). When a text ends inside a UMP, the remaining bytes are 0x00; this
//!   happens only in an End or Complete UMP (7.1.4, 7.1.5, 7.1.9).
//! - UMP text is UTF-8 in Normalization Form C without a Byte Order Mark (1.8). The caller gives
//!   NFC text; the writers do not normalize it (DEC-UMP-005).
//! - Reserved bits are zero on write and cleared on read (2.1.3). Message Type, Format and Status
//!   are read-only. The structs have no public `from_bits` (DEC-UMP-002).

use core::ops::Deref;

use bitfield_struct::bitfield;

use crate::packet::{join_u128, put, split_u128, take, Group, MessageType, UmpError, UmpWords};

/// Format of a UMP Stream packet, bits 123..122 (Figure 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum StreamFormat {
    /// 0x0: Complete message in one UMP.
    Complete = 0x0,
    /// 0x1: Start of a message that spans two or more UMPs.
    Start = 0x1,
    /// 0x2: Continue a message that spans three or more UMPs.
    Continue = 0x2,
    /// 0x3: End of a message that spans two or more UMPs.
    End = 0x3,
}

impl StreamFormat {
    /// The 2-bit Format value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Format for the low 2 bits of `value`. Every 2-bit value is defined.
    pub const fn from_bits(value: u8) -> Self {
        match value & 0x3 {
            0x0 => Self::Complete,
            0x1 => Self::Start,
            0x2 => Self::Continue,
            _ => Self::End,
        }
    }
}

/// Status of a UMP Stream message, bits 121..112 (7.1.1 to 7.1.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum UmpStreamStatus {
    /// 0x000: Endpoint Discovery (7.1.1).
    EndpointDiscovery = 0x000,
    /// 0x001: Endpoint Info Notification (7.1.2).
    EndpointInfoNotification = 0x001,
    /// 0x002: Device Identity Notification (7.1.3).
    DeviceIdentityNotification = 0x002,
    /// 0x003: Endpoint Name Notification (7.1.4).
    EndpointNameNotification = 0x003,
    /// 0x004: Product Instance Id Notification (7.1.5).
    ProductInstanceIdNotification = 0x004,
    /// 0x005: Stream Configuration Request (7.1.6.2).
    StreamConfigurationRequest = 0x005,
    /// 0x006: Stream Configuration Notification (7.1.6.3).
    StreamConfigurationNotification = 0x006,
    /// 0x010: Function Block Discovery (7.1.7).
    FunctionBlockDiscovery = 0x010,
    /// 0x011: Function Block Info Notification (7.1.8).
    FunctionBlockInfoNotification = 0x011,
    /// 0x012: Function Block Name Notification (7.1.9).
    FunctionBlockNameNotification = 0x012,
    /// 0x020: Start of Clip (7.1.10).
    StartOfClip = 0x020,
    /// 0x021: End of Clip (7.1.11).
    EndOfClip = 0x021,
}

impl UmpStreamStatus {
    /// The 10-bit Status value.
    pub const fn into_bits(self) -> u16 {
        self as u16
    }

    /// The Status for a 10-bit `value`, or `None` for a value that 7.1 does not define.
    pub const fn from_bits(value: u16) -> Option<Self> {
        match value {
            0x000 => Some(Self::EndpointDiscovery),
            0x001 => Some(Self::EndpointInfoNotification),
            0x002 => Some(Self::DeviceIdentityNotification),
            0x003 => Some(Self::EndpointNameNotification),
            0x004 => Some(Self::ProductInstanceIdNotification),
            0x005 => Some(Self::StreamConfigurationRequest),
            0x006 => Some(Self::StreamConfigurationNotification),
            0x010 => Some(Self::FunctionBlockDiscovery),
            0x011 => Some(Self::FunctionBlockInfoNotification),
            0x012 => Some(Self::FunctionBlockNameNotification),
            0x020 => Some(Self::StartOfClip),
            0x021 => Some(Self::EndOfClip),
            _ => None,
        }
    }
}

/// Message Type, Format and Status bits of a packet.
const fn header(format: StreamFormat, status: UmpStreamStatus) -> u128 {
    ((MessageType::UmpStream.into_bits() as u128) << 124)
        | ((format.into_bits() as u128) << 122)
        | ((status.into_bits() as u128) << 112)
}

/// Mask of the Message Type, Format and Status bits.
const HEADER: u128 = 0xFFFF << 112;

/// UMP Version 1.1 in the version bytes of Figures 11 and 13 (Table 5).
const VERSION_1_1: u128 = (0x01 << 104) | (0x01 << 96);

/// Reads a Complete packet with `status` from `words`, and clears every bit outside `valid`.
fn decode(words: &[u32], status: UmpStreamStatus, valid: u128) -> Result<u128, UmpError> {
    let bits = join_u128(take::<4>(words)?);
    if bits & HEADER != header(StreamFormat::Complete, status) {
        return Err(UmpError::UnexpectedMessage);
    }
    Ok(bits & valid)
}

/// Endpoint Discovery, Status 0x000 (7.1.1, Figures 11 and 12). The Filter bits request the
/// replies: `e` Endpoint Info, `d` Device Identity, `n` Endpoint Name, `i` Product Instance Id,
/// `s` Stream Configuration.
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct EndpointDiscovery {
    /// Message Type, 4 bits: 0xF.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Format, 2 bits: Complete.
    #[bits(2, access = RO)]
    pub format: StreamFormat,
    /// Status, 10 bits: 0x000.
    #[bits(10, access = RO)]
    pub status: u16,
    /// UMP Version Major, 8 bits (Table 5).
    pub ump_version_major: u8,
    /// UMP Version Minor, 8 bits (Table 5).
    pub ump_version_minor: u8,
    /// Reserved, 27 bits.
    #[bits(27)]
    __: u32,
    /// Filter bit `s`: request a Stream Configuration Notification.
    pub stream_configuration: bool,
    /// Filter bit `i`: request a Product Instance Id Notification.
    pub product_instance_id: bool,
    /// Filter bit `n`: request an Endpoint Name Notification.
    pub endpoint_name: bool,
    /// Filter bit `d`: request a Device Identity Notification.
    pub device_identity: bool,
    /// Filter bit `e`: request an Endpoint Info Notification.
    pub endpoint_info: bool,
    /// Reserved, 64 bits.
    __: u64,
}

impl EndpointDiscovery {
    const VALID: u128 = (0xFFFF_FFFF << 96) | (0x1F << 64);

    /// An Endpoint Discovery with UMP Version 1.1 (Table 5) and no Filter bit set. Set the
    /// Filter bits with the setters.
    pub const fn new() -> Self {
        Self(header(StreamFormat::Complete, UmpStreamStatus::EndpointDiscovery) | VERSION_1_1)
    }
}

impl Default for EndpointDiscovery {
    fn default() -> Self {
        Self::new()
    }
}

/// Endpoint Info Notification, Status 0x001 (7.1.2, Figure 13).
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct EndpointInfoNotification {
    /// Message Type, 4 bits: 0xF.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Format, 2 bits: Complete.
    #[bits(2, access = RO)]
    pub format: StreamFormat,
    /// Status, 10 bits: 0x001.
    #[bits(10, access = RO)]
    pub status: u16,
    /// UMP Version Major, 8 bits: 0x01 for this revision.
    pub ump_version_major: u8,
    /// UMP Version Minor, 8 bits: 0x01 for this revision.
    pub ump_version_minor: u8,
    /// `S`: Static Function Blocks.
    pub static_function_blocks: bool,
    /// Number of Function Blocks, 7 bits: 0x00 to 0x20; 0x21 to 0x7F are reserved.
    #[bits(7, access = RO)]
    pub number_of_function_blocks: u8,
    /// Reserved, 14 bits.
    #[bits(14)]
    __: u16,
    /// `M2`: MIDI 2.0 Protocol capability.
    pub midi2: bool,
    /// `M1`: MIDI 1.0 Protocol capability.
    pub midi1: bool,
    /// Reserved, 6 bits.
    #[bits(6)]
    __: u8,
    /// `RXJR`: can receive JR Timestamps.
    pub rxjr: bool,
    /// `TXJR`: can send JR Timestamps.
    pub txjr: bool,
    /// Reserved, 64 bits.
    __: u64,
}

impl EndpointInfoNotification {
    const VALID: u128 = (0xFFFF_FFFF << 96) | (0xFF << 88) | (0b11 << 72) | (0b11 << 64);

    /// An Endpoint Info Notification with UMP Version 1.1 (7.1.2). Gives `OutOfRange` for a
    /// Number of Function Blocks above 0x20 (0x21 to 0x7F are reserved).
    pub const fn try_new(
        static_function_blocks: bool,
        number_of_function_blocks: u8,
        midi2: bool,
        midi1: bool,
        rxjr: bool,
        txjr: bool,
    ) -> Result<Self, UmpError> {
        if number_of_function_blocks > 0x20 {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(
                StreamFormat::Complete,
                UmpStreamStatus::EndpointInfoNotification,
            ) | VERSION_1_1
                | ((static_function_blocks as u128) << 95)
                | ((number_of_function_blocks as u128) << 88)
                | ((midi2 as u128) << 73)
                | ((midi1 as u128) << 72)
                | ((rxjr as u128) << 65)
                | ((txjr as u128) << 64),
        ))
    }
}

/// Device Identity Notification, Status 0x002 (7.1.3, Figure 14). Each identity byte is 7 bits
/// with a reserved high bit, as in the MIDI 1.0 Device Inquiry reply.
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct DeviceIdentityNotification {
    /// Message Type, 4 bits: 0xF.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Format, 2 bits: Complete.
    #[bits(2, access = RO)]
    pub format: StreamFormat,
    /// Status, 10 bits: 0x002.
    #[bits(10, access = RO)]
    pub status: u16,
    /// Reserved, 24 bits (word 0 bits 15..0 and word 1 bits 31..24).
    #[bits(24)]
    __: u32,
    /// Device Manufacturer System Exclusive ID, bytes 1 to 3: 24 bits, each byte 7 bits.
    #[bits(24, access = RO)]
    manufacturer: u32,
    /// Device Family LSB and MSB, then Model Number LSB and MSB: 32 bits, each byte 7 bits.
    #[bits(32, access = RO)]
    family_and_model: u32,
    /// Software Revision Level bytes 1 to 4: 32 bits, each byte 7 bits.
    #[bits(32, access = RO)]
    software_revision: u32,
}

/// Gives `OutOfRange` when a byte of `bytes` is above 0x7F, else the bytes as a big-endian value.
const fn seven_bit_bytes<const N: usize>(bytes: [u8; N]) -> Result<u32, UmpError> {
    let mut value = 0u32;
    let mut i = 0;
    while i < N {
        if bytes[i] > 0x7F {
            return Err(UmpError::OutOfRange);
        }
        value = (value << 8) | bytes[i] as u32;
        i += 1;
    }
    Ok(value)
}

/// A 14-bit value as its LSB and MSB 7-bit bytes, in that order.
const fn lsb_msb(value: u16) -> u32 {
    (((value & 0x7F) as u32) << 8) | ((value >> 7) & 0x7F) as u32
}

impl DeviceIdentityNotification {
    const VALID: u128 = (0xFFFF << 112) | (0x7F7F7F << 64) | (0x7F7F_7F7F << 32) | 0x7F7F_7F7F;

    /// A Device Identity Notification (7.1.3). A 1-byte System Exclusive ID goes in the first
    /// manufacturer byte, with two zero bytes after it. Device Family and Model Number are
    /// 14-bit values, written LSB first. Gives `OutOfRange` for a manufacturer or software
    /// revision byte above 0x7F, or a family or model above 0x3FFF.
    pub const fn try_new(
        manufacturer: [u8; 3],
        family: u16,
        model: u16,
        software_revision: [u8; 4],
    ) -> Result<Self, UmpError> {
        let manufacturer = match seven_bit_bytes(manufacturer) {
            Ok(v) => v,
            Err(e) => return Err(e),
        };
        let revision = match seven_bit_bytes(software_revision) {
            Ok(v) => v,
            Err(e) => return Err(e),
        };
        if family > 0x3FFF || model > 0x3FFF {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(
                StreamFormat::Complete,
                UmpStreamStatus::DeviceIdentityNotification,
            ) | ((manufacturer as u128) << 64)
                | ((lsb_msb(family) as u128) << 48)
                | ((lsb_msb(model) as u128) << 32)
                | revision as u128,
        ))
    }

    /// The 3-byte Device Manufacturer System Exclusive ID.
    pub const fn device_manufacturer(&self) -> [u8; 3] {
        let m = self.manufacturer();
        [(m >> 16) as u8, (m >> 8) as u8, m as u8]
    }

    /// The 14-bit Device Family.
    pub const fn device_family(&self) -> u16 {
        let v = self.family_and_model() >> 16;
        ((v & 0x7F) as u16) << 7 | ((v >> 8) & 0x7F) as u16
    }

    /// The 14-bit Device Family Model Number.
    pub const fn device_family_model_number(&self) -> u16 {
        let v = self.family_and_model() & 0xFFFF;
        ((v & 0x7F) as u16) << 7 | ((v >> 8) & 0x7F) as u16
    }

    /// The 4-byte Software Revision Level.
    pub const fn software_revision_level(&self) -> [u8; 4] {
        self.software_revision().to_be_bytes()
    }
}

/// Protocol of a Stream Configuration message (7.1.6.2, 7.1.6.3). Other values are reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum StreamProtocol {
    /// 0x01: MIDI 1.0 Protocol.
    Midi1 = 0x01,
    /// 0x02: MIDI 2.0 Protocol.
    Midi2 = 0x02,
}

impl StreamProtocol {
    /// The Protocol byte.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Protocol for `value`, or `None` for a reserved value.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::Midi1),
            0x02 => Some(Self::Midi2),
            _ => None,
        }
    }
}

/// Stream Configuration Request, Status 0x005 (7.1.6.2, Figure 18).
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct StreamConfigurationRequest {
    /// Message Type, 4 bits: 0xF.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Format, 2 bits: Complete.
    #[bits(2, access = RO)]
    pub format: StreamFormat,
    /// Status, 10 bits.
    #[bits(10, access = RO)]
    pub status: u16,
    /// Protocol, 8 bits. Read it with `protocol()`.
    #[bits(8, access = RO)]
    protocol_bits: u8,
    /// Reserved, 6 bits.
    #[bits(6)]
    __: u8,
    /// `RXJR`: JR Timestamps on messages to the receiver of the request, or to the
    /// sender of the notification.
    pub rxjr: bool,
    /// `TXJR`: JR Timestamps on messages from the receiver of the request, or from the
    /// sender of the notification.
    pub txjr: bool,
    /// Reserved, 96 bits.
    #[bits(96)]
    __: u128,
}

impl StreamConfigurationRequest {
    const VALID: u128 = (0xFFFF << 112) | (0xFF << 104) | (0b11 << 96);

    /// A message with `protocol` and the JR Timestamp bits.
    pub const fn new(protocol: StreamProtocol, rxjr: bool, txjr: bool) -> Self {
        Self(
            header(
                StreamFormat::Complete,
                UmpStreamStatus::StreamConfigurationRequest,
            ) | ((protocol.into_bits() as u128) << 104)
                | ((rxjr as u128) << 97)
                | ((txjr as u128) << 96),
        )
    }

    /// The Protocol. `parse` and `from_words` reject a reserved value.
    pub const fn protocol(&self) -> StreamProtocol {
        match StreamProtocol::from_bits(self.protocol_bits()) {
            Some(p) => p,
            None => StreamProtocol::Midi1,
        }
    }

    fn checked(bits: u128) -> Result<Self, UmpError> {
        let m = Self(bits);
        match StreamProtocol::from_bits(m.protocol_bits()) {
            Some(_) => Ok(m),
            None => Err(UmpError::UnexpectedMessage),
        }
    }
}

/// Stream Configuration Notification, Status 0x006 (7.1.6.3, Figure 19).
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct StreamConfigurationNotification {
    /// Message Type, 4 bits: 0xF.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Format, 2 bits: Complete.
    #[bits(2, access = RO)]
    pub format: StreamFormat,
    /// Status, 10 bits.
    #[bits(10, access = RO)]
    pub status: u16,
    /// Protocol, 8 bits. Read it with `protocol()`.
    #[bits(8, access = RO)]
    protocol_bits: u8,
    /// Reserved, 6 bits.
    #[bits(6)]
    __: u8,
    /// `RXJR`: JR Timestamps on messages to the receiver of the request, or to the
    /// sender of the notification.
    pub rxjr: bool,
    /// `TXJR`: JR Timestamps on messages from the receiver of the request, or from the
    /// sender of the notification.
    pub txjr: bool,
    /// Reserved, 96 bits.
    #[bits(96)]
    __: u128,
}

impl StreamConfigurationNotification {
    const VALID: u128 = (0xFFFF << 112) | (0xFF << 104) | (0b11 << 96);

    /// A message with `protocol` and the JR Timestamp bits.
    pub const fn new(protocol: StreamProtocol, rxjr: bool, txjr: bool) -> Self {
        Self(
            header(
                StreamFormat::Complete,
                UmpStreamStatus::StreamConfigurationNotification,
            ) | ((protocol.into_bits() as u128) << 104)
                | ((rxjr as u128) << 97)
                | ((txjr as u128) << 96),
        )
    }

    /// The Protocol. `parse` and `from_words` reject a reserved value.
    pub const fn protocol(&self) -> StreamProtocol {
        match StreamProtocol::from_bits(self.protocol_bits()) {
            Some(p) => p,
            None => StreamProtocol::Midi1,
        }
    }

    fn checked(bits: u128) -> Result<Self, UmpError> {
        let m = Self(bits);
        match StreamProtocol::from_bits(m.protocol_bits()) {
            Some(_) => Ok(m),
            None => Err(UmpError::UnexpectedMessage),
        }
    }
}

/// Function Block Discovery, Status 0x010 (7.1.7, Figures 20 and 21).
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct FunctionBlockDiscovery {
    /// Message Type, 4 bits: 0xF.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Format, 2 bits: Complete.
    #[bits(2, access = RO)]
    pub format: StreamFormat,
    /// Status, 10 bits: 0x010.
    #[bits(10, access = RO)]
    pub status: u16,
    /// Function Block Number, 8 bits: 0x00 to 0x1F, or 0xFF for all Function Blocks.
    #[bits(8, access = RO)]
    pub function_block_number: u8,
    /// Reserved, 6 bits.
    #[bits(6)]
    __: u8,
    /// Filter bit `n`: request a Function Block Name Notification.
    pub name: bool,
    /// Filter bit `i`: request a Function Block Info Notification.
    pub info: bool,
    /// Reserved, 96 bits.
    #[bits(96)]
    __: u128,
}

impl FunctionBlockDiscovery {
    const VALID: u128 = (0xFFFF << 112) | (0xFF << 104) | (0b11 << 96);

    /// The Function Block Number that requests all Function Blocks.
    pub const ALL: u8 = 0xFF;

    /// A Function Block Discovery for `function_block_number` (0 to 31, or 0xFF for all) with
    /// the Filter bits `info` and `name`. Gives `OutOfRange` for 0x20 to 0xFE (7.1.7).
    pub const fn try_new(
        function_block_number: u8,
        info: bool,
        name: bool,
    ) -> Result<Self, UmpError> {
        if function_block_number > 0x1F && function_block_number != Self::ALL {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self(
            header(
                StreamFormat::Complete,
                UmpStreamStatus::FunctionBlockDiscovery,
            ) | ((function_block_number as u128) << 104)
                | ((name as u128) << 97)
                | ((info as u128) << 96),
        ))
    }
}

/// Direction of a Function Block (7.1.8). 0b00 is reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FunctionBlockDirection {
    /// 0b01: the Function Block receives MIDI Messages only.
    Input = 0b01,
    /// 0b10: the Function Block transmits MIDI Messages only.
    Output = 0b10,
    /// 0b11: every Input Group member has a matching Output Group.
    Bidirectional = 0b11,
}

impl FunctionBlockDirection {
    /// The 2-bit Direction value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The Direction for a 2-bit `value`, or `None` for the reserved 0b00.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0b01 => Some(Self::Input),
            0b10 => Some(Self::Output),
            0b11 => Some(Self::Bidirectional),
            _ => None,
        }
    }
}

/// MIDI 1.0 field of a Function Block (7.1.8). 0x03 is reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FunctionBlockMidi1 {
    /// 0x00: not MIDI 1.0.
    NotMidi1 = 0x00,
    /// 0x01: a MIDI 1.0 Port; do not restrict bandwidth.
    Unrestricted = 0x01,
    /// 0x02: a MIDI 1.0 Port; restrict bandwidth to 31.25 kbps.
    Restricted = 0x02,
}

impl FunctionBlockMidi1 {
    /// The 2-bit MIDI 1.0 value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The MIDI 1.0 value for a 2-bit `value`, or `None` for the reserved 0x03.
    pub const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::NotMidi1),
            0x01 => Some(Self::Unrestricted),
            0x02 => Some(Self::Restricted),
            _ => None,
        }
    }
}

/// User Interface Hint of a Function Block (7.1.8). Every 2-bit value is defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum UiHint {
    /// 0b00: unknown or undeclared.
    Unknown = 0b00,
    /// 0b01: primarily a Receiver or destination of MIDI messages.
    Receiver = 0b01,
    /// 0b10: primarily a Sender or source of MIDI messages.
    Sender = 0b10,
    /// 0b11: both a Sender and a Receiver.
    SenderReceiver = 0b11,
}

impl UiHint {
    /// The 2-bit User Interface Hint value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The User Interface Hint for the low 2 bits of `value`.
    pub const fn from_bits(value: u8) -> Self {
        match value & 0b11 {
            0b00 => Self::Unknown,
            0b01 => Self::Receiver,
            0b10 => Self::Sender,
            _ => Self::SenderReceiver,
        }
    }
}

/// The field values of a Function Block Info Notification (7.1.8, Figure 22).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionBlockInfo {
    /// `a`: the Function Block is active.
    pub active: bool,
    /// Function Block Number, 0 to 31.
    pub number: u8,
    /// Direction.
    pub direction: FunctionBlockDirection,
    /// MIDI 1.0 Port declaration.
    pub midi1: FunctionBlockMidi1,
    /// User Interface Hint.
    pub ui_hint: UiHint,
    /// The member Group with the lowest number.
    pub first_group: Group,
    /// Number of Groups Spanned, 1 to 16.
    pub groups_spanned: u8,
    /// MIDI-CI Message Version/Format; 0x00 is none or unknown (see M2-101-UM).
    pub zmidi_ci_version: u8,
    /// Max Number of SysEx 8 Streams: 0 none, 1 one at a time, 2 to 255 simultaneous streams.
    pub max_sysex8_streams: u8,
}

/// Function Block Info Notification, Status 0x011 (7.1.8, Figure 22). Read the fields with
/// [`FunctionBlockInfoNotification::info`].
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct FunctionBlockInfoNotification {
    /// Message Type, 4 bits: 0xF.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Format, 2 bits: Complete.
    #[bits(2, access = RO)]
    pub format: StreamFormat,
    /// Status, 10 bits: 0x011.
    #[bits(10, access = RO)]
    pub status: u16,
    /// `a`, then Function Block Number (7 bits), reserved (2 bits), User Interface Hint (2
    /// bits), MIDI 1.0 (2 bits) and Direction (2 bits): 16 bits.
    #[bits(16, access = RO)]
    block_bits: u16,
    /// First Group, Number of Groups Spanned, MIDI-CI Message Version/Format and Max Number of
    /// SysEx 8 Streams: 8 bits each.
    #[bits(32, access = RO)]
    group_bits: u32,
    /// Reserved, 64 bits.
    __: u64,
}

impl FunctionBlockInfoNotification {
    const VALID: u128 = (0xFFFF << 112) | (0xFF3F << 96) | (0xFFFF_FFFF << 64);

    /// A Function Block Info Notification (7.1.8). Gives `OutOfRange` for a Function Block
    /// Number above 31, a Number of Groups Spanned outside 1 to 16, a MIDI 1.0 Port with a
    /// Number of Groups Spanned other than 1, or Direction Output with User Interface Hint
    /// Receiver.
    pub const fn try_new(info: FunctionBlockInfo) -> Result<Self, UmpError> {
        let midi1_port = !matches!(info.midi1, FunctionBlockMidi1::NotMidi1);
        if info.number > 31
            || info.groups_spanned == 0
            || info.groups_spanned > 16
            || (midi1_port && info.groups_spanned != 1)
            || (matches!(info.direction, FunctionBlockDirection::Output)
                && matches!(info.ui_hint, UiHint::Receiver))
        {
            return Err(UmpError::OutOfRange);
        }
        let block = ((info.active as u128) << 15)
            | ((info.number as u128) << 8)
            | ((info.ui_hint.into_bits() as u128) << 4)
            | ((info.midi1.into_bits() as u128) << 2)
            | info.direction.into_bits() as u128;
        let groups = ((info.first_group.into_bits() as u128) << 24)
            | ((info.groups_spanned as u128) << 16)
            | ((info.zmidi_ci_version as u128) << 8)
            | info.max_sysex8_streams as u128;
        Ok(Self(
            header(
                StreamFormat::Complete,
                UmpStreamStatus::FunctionBlockInfoNotification,
            ) | (block << 96)
                | (groups << 64),
        ))
    }

    /// The field values. Gives `None` when the received Direction (0b00) or MIDI 1.0 field
    /// (0x03) is reserved, or the First Group is above 0xF.
    pub const fn info(&self) -> Option<FunctionBlockInfo> {
        let b = self.block_bits();
        let g = self.group_bits();
        let direction = match FunctionBlockDirection::from_bits((b & 0b11) as u8) {
            Some(d) => d,
            None => return None,
        };
        let midi1 = match FunctionBlockMidi1::from_bits(((b >> 2) & 0b11) as u8) {
            Some(m) => m,
            None => return None,
        };
        let first_group = (g >> 24) as u8;
        if first_group > 0xF {
            return None;
        }
        Some(FunctionBlockInfo {
            active: b >> 15 != 0,
            number: ((b >> 8) & 0x7F) as u8,
            direction,
            midi1,
            ui_hint: UiHint::from_bits((b >> 4) as u8),
            first_group: Group::from_bits(first_group),
            groups_spanned: (g >> 16) as u8,
            zmidi_ci_version: (g >> 8) as u8,
            max_sysex8_streams: g as u8,
        })
    }
}

/// Start of Clip, Status 0x020 (7.1.10, Figure 24): the first event of the Clip Sequence
/// Data of a MIDI Clip File.
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct StartOfClip {
    /// Message Type, 4 bits: 0xF.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Format, 2 bits: Complete.
    #[bits(2, access = RO)]
    pub format: StreamFormat,
    /// Status, 10 bits.
    #[bits(10, access = RO)]
    pub status: u16,
    /// Reserved, 112 bits.
    #[bits(112)]
    __: u128,
}

impl StartOfClip {
    const VALID: u128 = 0xFFFF << 112;

    /// The message; all bits after the Status are zero.
    pub const fn new() -> Self {
        Self(header(StreamFormat::Complete, UmpStreamStatus::StartOfClip))
    }
}

impl Default for StartOfClip {
    fn default() -> Self {
        Self::new()
    }
}

/// End of Clip, Status 0x021 (7.1.11, Figure 25): the last event of the Clip Sequence Data
/// of a MIDI Clip File.
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct EndOfClip {
    /// Message Type, 4 bits: 0xF.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Format, 2 bits: Complete.
    #[bits(2, access = RO)]
    pub format: StreamFormat,
    /// Status, 10 bits.
    #[bits(10, access = RO)]
    pub status: u16,
    /// Reserved, 112 bits.
    #[bits(112)]
    __: u128,
}

impl EndOfClip {
    const VALID: u128 = 0xFFFF << 112;

    /// The message; all bits after the Status are zero.
    pub const fn new() -> Self {
        Self(header(StreamFormat::Complete, UmpStreamStatus::EndOfClip))
    }
}

impl Default for EndOfClip {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! stream_words {
    ($($ty:ident => $status:ident, $check:expr);* $(;)?) => {$(
        impl UmpWords for $ty {
            const WORDS: usize = 4;

            fn from_words(words: &[u32]) -> Result<Self, UmpError> {
                let check: fn(u128) -> Result<Self, UmpError> = $check;
                decode(words, UmpStreamStatus::$status, Self::VALID).and_then(check)
            }

            fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
                put(&split_u128(self.0), out)
            }
        }
    )*};
}

stream_words! {
    EndpointDiscovery => EndpointDiscovery, |b| Ok(EndpointDiscovery(b));
    EndpointInfoNotification => EndpointInfoNotification, |b| Ok(EndpointInfoNotification(b));
    DeviceIdentityNotification => DeviceIdentityNotification, |b| Ok(DeviceIdentityNotification(b));
    StreamConfigurationRequest => StreamConfigurationRequest, StreamConfigurationRequest::checked;
    StreamConfigurationNotification => StreamConfigurationNotification, StreamConfigurationNotification::checked;
    FunctionBlockDiscovery => FunctionBlockDiscovery, |b| Ok(FunctionBlockDiscovery(b));
    FunctionBlockInfoNotification => FunctionBlockInfoNotification, |b| Ok(FunctionBlockInfoNotification(b));
    StartOfClip => StartOfClip, |b| Ok(StartOfClip(b));
    EndOfClip => EndOfClip, |b| Ok(EndOfClip(b));
}

/// The three text messages of a UMP Stream (7.1.4, 7.1.5, 7.1.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamTextKind {
    /// Endpoint Name Notification, Status 0x003: UTF-8, at most 98 bytes.
    EndpointName,
    /// Product Instance Id Notification, Status 0x004: ASCII 32 to 126, at most 42 bytes.
    ProductInstanceId,
    /// Function Block Name Notification, Status 0x012: UTF-8, at most 91 bytes.
    FunctionBlockName,
}

impl StreamTextKind {
    const fn status(self) -> UmpStreamStatus {
        match self {
            Self::EndpointName => UmpStreamStatus::EndpointNameNotification,
            Self::ProductInstanceId => UmpStreamStatus::ProductInstanceIdNotification,
            Self::FunctionBlockName => UmpStreamStatus::FunctionBlockNameNotification,
        }
    }

    const fn from_status(status: UmpStreamStatus) -> Option<Self> {
        match status {
            UmpStreamStatus::EndpointNameNotification => Some(Self::EndpointName),
            UmpStreamStatus::ProductInstanceIdNotification => Some(Self::ProductInstanceId),
            UmpStreamStatus::FunctionBlockNameNotification => Some(Self::FunctionBlockName),
            _ => None,
        }
    }

    /// Largest text in bytes.
    const fn max_len(self) -> usize {
        match self {
            Self::EndpointName => 98,
            Self::ProductInstanceId => 42,
            Self::FunctionBlockName => 91,
        }
    }

    /// Text bytes in one UMP: 14, or 13 after the Function Block Number.
    const fn chunk(self) -> usize {
        match self {
            Self::FunctionBlockName => 13,
            _ => 14,
        }
    }
}

/// Largest text of any kind (Endpoint Name, 7.1.4).
const MAX_TEXT: usize = 98;

/// One text UMP of an Endpoint Name, Product Instance Id or Function Block Name
/// (Figures 15, 16, 23). [`StreamTextPackets`] writes it and [`UmpStreamMessage::parse`] reads
/// it.
#[bitfield(u128, order = Msb, conversion = false, hash = true, new = false, default = false)]
#[derive(PartialEq, Eq)]
pub struct StreamTextPacket {
    /// Message Type, 4 bits: 0xF.
    #[bits(4, access = RO)]
    pub message_type: MessageType,
    /// Format, 2 bits. Read it with `format()`.
    #[bits(2, access = RO)]
    format_bits: StreamFormat,
    /// Status, 10 bits: 0x003, 0x004 or 0x012.
    #[bits(10, access = RO)]
    pub status: u16,
    /// 14 data bytes: text bytes, or the Function Block Number and 13 text bytes.
    #[bits(112, access = RO)]
    data: u128,
}

impl StreamTextPacket {
    /// A packet of `kind` with `format`, the Function Block Number `block` (Function Block Name
    /// only) and up to one chunk of `text`. The caller checks the lengths.
    fn from_parts(kind: StreamTextKind, format: StreamFormat, block: u8, text: &[u8]) -> Self {
        let mut bits = header(format, kind.status());
        let mut byte = 0;
        if matches!(kind, StreamTextKind::FunctionBlockName) {
            bits |= (block as u128) << 104;
            byte = 1;
        }
        for &b in text.iter().take(kind.chunk()) {
            bits |= (b as u128) << (104 - 8 * byte);
            byte += 1;
        }
        Self(bits)
    }

    /// Reads a text packet, or `None` for another message or a Function Block Number above 31.
    /// Clears the bytes after the first 0x00 byte, which end the text (7.1.4).
    fn parse(words: &[u32]) -> Option<Self> {
        let bits = join_u128(take::<4>(words).ok()?);
        let packet = Self(bits);
        if !matches!(packet.message_type(), MessageType::UmpStream) {
            return None;
        }
        let kind = StreamTextKind::from_status(UmpStreamStatus::from_bits(packet.status())?)?;
        let mut clean = bits & HEADER;
        let mut first = 0;
        if matches!(kind, StreamTextKind::FunctionBlockName) {
            let block = packet.byte(0);
            if block > 31 {
                return None;
            }
            clean |= (block as u128) << 104;
            first = 1;
        }
        let mut i = first;
        while i < 14 && packet.byte(i) != 0 {
            clean |= (packet.byte(i) as u128) << (104 - 8 * i);
            i += 1;
        }
        Some(Self(clean))
    }

    /// Data byte `i` (0 to 13).
    const fn byte(&self, i: usize) -> u8 {
        (self.data() >> (104 - 8 * i)) as u8
    }

    /// The text message of the packet.
    pub const fn kind(&self) -> StreamTextKind {
        match UmpStreamStatus::from_bits(self.status()) {
            Some(UmpStreamStatus::ProductInstanceIdNotification) => {
                StreamTextKind::ProductInstanceId
            }
            Some(UmpStreamStatus::FunctionBlockNameNotification) => {
                StreamTextKind::FunctionBlockName
            }
            _ => StreamTextKind::EndpointName,
        }
    }

    /// The Format of the packet.
    pub const fn format(&self) -> StreamFormat {
        self.format_bits()
    }

    /// The Function Block Number of a Function Block Name packet, else `None`.
    pub const fn function_block_number(&self) -> Option<u8> {
        match self.kind() {
            StreamTextKind::FunctionBlockName => Some(self.byte(0)),
            _ => None,
        }
    }

    /// The text bytes of the packet, up to the first 0x00 byte.
    pub fn text(&self) -> StreamTextBytes {
        let first = 14 - self.kind().chunk();
        let mut bytes = [0u8; 14];
        let mut len = 0;
        for i in first..14 {
            let b = self.byte(i);
            if b == 0 {
                break;
            }
            bytes[len] = b;
            len += 1;
        }
        StreamTextBytes {
            bytes,
            len: len as u8,
        }
    }
}

impl UmpWords for StreamTextPacket {
    const WORDS: usize = 4;

    fn from_words(words: &[u32]) -> Result<Self, UmpError> {
        take::<4>(words)?;
        Self::parse(words).ok_or(UmpError::UnexpectedMessage)
    }

    fn to_words(&self, out: &mut [u32]) -> Result<usize, UmpError> {
        put(&split_u128(self.0), out)
    }
}

/// The 0 to 14 text bytes of one [`StreamTextPacket`]. Derefs to a byte slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamTextBytes {
    bytes: [u8; 14],
    len: u8,
}

impl Deref for StreamTextBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

/// Iterator over the text UMPs of one Endpoint Name, Product Instance Id or Function Block Name,
/// in send order (7.1.4, 7.1.5, 7.1.9).
///
/// A text that fits in one UMP gives one Complete packet; a longer text gives Start, zero or
/// more Continue and End packets, with full packets before the last. An empty text gives one
/// Complete packet of zero bytes.
#[derive(Debug, Clone)]
pub struct StreamTextPackets<'a> {
    kind: StreamTextKind,
    block: u8,
    rest: &'a [u8],
    started: bool,
    done: bool,
}

impl<'a> StreamTextPackets<'a> {
    fn checked(kind: StreamTextKind, block: u8, text: &'a str) -> Result<Self, UmpError> {
        // A zero byte would end the text early (7.1.4); U+FEFF reads as a Byte Order Mark (1.8).
        if text.len() > kind.max_len() || text.contains(['\0', '\u{FEFF}']) {
            return Err(UmpError::OutOfRange);
        }
        Ok(Self {
            kind,
            block,
            rest: text.as_bytes(),
            started: false,
            done: false,
        })
    }

    /// The packets of an Endpoint Name (7.1.4). Gives `OutOfRange` for a name longer than
    /// 98 bytes, a Byte Order Mark (U+FEFF) or a zero byte. The caller gives NFC text
    /// (1.8, DEC-UMP-005).
    pub fn endpoint_name(name: &'a str) -> Result<Self, UmpError> {
        Self::checked(StreamTextKind::EndpointName, 0, name)
    }

    /// The packets of a Product Instance Id (7.1.5). Gives `OutOfRange` for a text longer than
    /// 42 bytes or a character outside ASCII 32 to 126.
    pub fn product_instance_id(id: &'a str) -> Result<Self, UmpError> {
        if !id.bytes().all(|b| (32..=126).contains(&b)) {
            return Err(UmpError::OutOfRange);
        }
        Self::checked(StreamTextKind::ProductInstanceId, 0, id)
    }

    /// The packets of the name of Function Block `number` (7.1.9). Gives `OutOfRange` for a
    /// number above 31, a name longer than 91 bytes, a Byte Order Mark (U+FEFF) or a zero
    /// byte. The caller gives NFC text (1.8, DEC-UMP-005).
    pub fn function_block_name(number: u8, name: &'a str) -> Result<Self, UmpError> {
        if number > 31 {
            return Err(UmpError::OutOfRange);
        }
        Self::checked(StreamTextKind::FunctionBlockName, number, name)
    }
}

impl Iterator for StreamTextPackets<'_> {
    type Item = StreamTextPacket;

    fn next(&mut self) -> Option<StreamTextPacket> {
        if self.done {
            return None;
        }
        let chunk = self.kind.chunk();
        let last = self.rest.len() <= chunk;
        let format = match (self.started, last) {
            (false, true) => StreamFormat::Complete,
            (false, false) => StreamFormat::Start,
            (true, false) => StreamFormat::Continue,
            (true, true) => StreamFormat::End,
        };
        let (head, rest) = self.rest.split_at(self.rest.len().min(chunk));
        self.rest = rest;
        self.started = true;
        self.done = last;
        Some(StreamTextPacket::from_parts(
            self.kind, format, self.block, head,
        ))
    }
}

/// A complete text from [`StreamTextReassembler`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamText<'a> {
    kind: StreamTextKind,
    block: u8,
    bytes: &'a [u8],
}

impl<'a> StreamText<'a> {
    /// The text message the text came from.
    pub const fn kind(&self) -> StreamTextKind {
        self.kind
    }

    /// The Function Block Number of a Function Block Name, else `None`.
    pub const fn function_block_number(&self) -> Option<u8> {
        match self.kind {
            StreamTextKind::FunctionBlockName => Some(self.block),
            _ => None,
        }
    }

    /// The text bytes, without the 0x00 bytes that end the text.
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The text, or `None` when the bytes are not valid UTF-8.
    pub fn as_str(&self) -> Option<&'a str> {
        core::str::from_utf8(self.bytes).ok()
    }
}

/// Result of [`StreamTextReassembler::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamTextEvent<'a> {
    /// A complete text.
    Complete(StreamText<'a>),
    /// A text of `kind` ended before its End packet, or grew longer than 98 bytes; its bytes are
    /// discarded.
    Incomplete {
        /// The text message of the discarded text.
        kind: StreamTextKind,
    },
}

/// Reassembles the text messages of a UMP Stream into a fixed buffer of 98 bytes, the largest
/// text (7.1.4). Needs no alloc.
#[derive(Debug, Clone)]
pub struct StreamTextReassembler {
    buffer: [u8; MAX_TEXT],
    len: usize,
    current: Option<(StreamTextKind, u8)>,
}

impl Default for StreamTextReassembler {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamTextReassembler {
    /// An empty reassembler.
    pub const fn new() -> Self {
        Self {
            buffer: [0; MAX_TEXT],
            len: 0,
            current: None,
        }
    }

    /// Clears the reassembler, so it holds no partial text.
    pub fn reset(&mut self) {
        self.len = 0;
        self.current = None;
    }

    /// Takes the words of one packet and gives an event when a text ends.
    ///
    /// - A packet that is not a text UMP of Message Type 0xF gives no event.
    /// - A Start begins a text and a Continue extends it. An End or Complete gives the text,
    ///   which ends at the first 0x00 byte.
    /// - A Start or Complete before the End of the current text ends it with
    ///   [`StreamTextEvent::Incomplete`]. A Start also begins a new text; a Complete is not
    ///   decoded, and the caller pushes it again.
    /// - A Continue or End of another kind or Function Block Number, or a text longer than
    ///   98 bytes, ends the current text with `Incomplete`.
    /// - A Continue or End with no Start gives no event.
    ///
    /// Never panics.
    pub fn push(&mut self, words: &[u32]) -> Option<StreamTextEvent<'_>> {
        let packet = StreamTextPacket::parse(words)?;
        let id = (packet.kind(), packet.function_block_number().unwrap_or(0));
        let text = packet.text();
        match (packet.format(), self.current) {
            (StreamFormat::Complete, None) => {
                self.len = 0;
                self.append(&text);
                Some(self.complete(id))
            }
            (StreamFormat::Complete, Some(old)) => {
                self.current = None;
                Some(StreamTextEvent::Incomplete { kind: old.0 })
            }
            (StreamFormat::Start, old) => {
                self.len = 0;
                self.current = Some(id);
                self.append(&text);
                old.map(|o| StreamTextEvent::Incomplete { kind: o.0 })
            }
            (StreamFormat::Continue | StreamFormat::End, None) => None,
            (format, Some(old)) => {
                if old != id || self.len + text.len() > MAX_TEXT {
                    self.current = None;
                    return Some(StreamTextEvent::Incomplete { kind: old.0 });
                }
                self.append(&text);
                if matches!(format, StreamFormat::End) {
                    self.current = None;
                    Some(self.complete(id))
                } else {
                    None
                }
            }
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if let Some(slot) = self.buffer.get_mut(self.len) {
                *slot = b;
                self.len += 1;
            }
        }
    }

    fn complete(&self, id: (StreamTextKind, u8)) -> StreamTextEvent<'_> {
        StreamTextEvent::Complete(StreamText {
            kind: id.0,
            block: id.1,
            bytes: &self.buffer[..self.len],
        })
    }
}

/// One UMP Stream message (7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UmpStreamMessage {
    /// Endpoint Discovery (0x000).
    EndpointDiscovery(EndpointDiscovery),
    /// Endpoint Info Notification (0x001).
    EndpointInfoNotification(EndpointInfoNotification),
    /// Device Identity Notification (0x002).
    DeviceIdentityNotification(DeviceIdentityNotification),
    /// Stream Configuration Request (0x005).
    StreamConfigurationRequest(StreamConfigurationRequest),
    /// Stream Configuration Notification (0x006).
    StreamConfigurationNotification(StreamConfigurationNotification),
    /// Function Block Discovery (0x010).
    FunctionBlockDiscovery(FunctionBlockDiscovery),
    /// Function Block Info Notification (0x011).
    FunctionBlockInfoNotification(FunctionBlockInfoNotification),
    /// Start of Clip (0x020).
    StartOfClip(StartOfClip),
    /// End of Clip (0x021).
    EndOfClip(EndOfClip),
    /// One text UMP of an Endpoint Name (0x003), Product Instance Id (0x004) or Function Block
    /// Name (0x012).
    Text(StreamTextPacket),
}

impl UmpStreamMessage {
    /// Decodes one UMP Stream packet (four words) by its Status. Gives `None` for short input,
    /// a Message Type other than 0xF, an undefined Status, a fixed-size message with a Format
    /// other than Complete, a reserved Protocol, or a Function Block Name with a Function Block
    /// Number above 31. Reserved bits are cleared. Never panics.
    pub fn parse(words: &[u32]) -> Option<Self> {
        use UmpStreamStatus as S;
        let w0 = *words.first()?;
        if w0 >> 28 != MessageType::UmpStream.into_bits() as u32 {
            return None;
        }
        Some(match S::from_bits(((w0 >> 16) & 0x3FF) as u16)? {
            S::EndpointDiscovery => {
                Self::EndpointDiscovery(EndpointDiscovery::from_words(words).ok()?)
            }
            S::EndpointInfoNotification => {
                Self::EndpointInfoNotification(EndpointInfoNotification::from_words(words).ok()?)
            }
            S::DeviceIdentityNotification => Self::DeviceIdentityNotification(
                DeviceIdentityNotification::from_words(words).ok()?,
            ),
            S::StreamConfigurationRequest => Self::StreamConfigurationRequest(
                StreamConfigurationRequest::from_words(words).ok()?,
            ),
            S::StreamConfigurationNotification => Self::StreamConfigurationNotification(
                StreamConfigurationNotification::from_words(words).ok()?,
            ),
            S::FunctionBlockDiscovery => {
                Self::FunctionBlockDiscovery(FunctionBlockDiscovery::from_words(words).ok()?)
            }
            S::FunctionBlockInfoNotification => Self::FunctionBlockInfoNotification(
                FunctionBlockInfoNotification::from_words(words).ok()?,
            ),
            S::StartOfClip => Self::StartOfClip(StartOfClip::from_words(words).ok()?),
            S::EndOfClip => Self::EndOfClip(EndOfClip::from_words(words).ok()?),
            S::EndpointNameNotification
            | S::ProductInstanceIdNotification
            | S::FunctionBlockNameNotification => Self::Text(StreamTextPacket::parse(words)?),
        })
    }

    /// Words 0 to 3 of the packet.
    pub(crate) const fn into_words(self) -> [u32; 4] {
        split_u128(match self {
            Self::EndpointDiscovery(m) => m.0,
            Self::EndpointInfoNotification(m) => m.0,
            Self::DeviceIdentityNotification(m) => m.0,
            Self::StreamConfigurationRequest(m) => m.0,
            Self::StreamConfigurationNotification(m) => m.0,
            Self::FunctionBlockDiscovery(m) => m.0,
            Self::FunctionBlockInfoNotification(m) => m.0,
            Self::StartOfClip(m) => m.0,
            Self::EndOfClip(m) => m.0,
            Self::Text(m) => m.0,
        })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::string::String;
    use std::vec::Vec;

    use super::*;
    use proptest::prelude::*;

    fn words<T: UmpWords>(m: &T) -> [u32; 4] {
        let mut out = [0u32; 4];
        m.to_words(&mut out).unwrap();
        out
    }

    /// Figures 11 and 12: Endpoint Discovery with version 1.1 and the Filter bits.
    #[test]
    fn endpoint_discovery_matches_figures_11_12() {
        let m = EndpointDiscovery::new();
        assert_eq!(words(&m), [0xF000_0101, 0, 0, 0]);
        assert_eq!(m.message_type(), MessageType::UmpStream);
        assert_eq!(m.format(), StreamFormat::Complete);
        let all = m
            .with_endpoint_info(true)
            .with_device_identity(true)
            .with_endpoint_name(true)
            .with_product_instance_id(true)
            .with_stream_configuration(true);
        assert_eq!(words(&all), [0xF000_0101, 0x0000_001F, 0, 0]);
        assert_eq!(
            words(&m.with_endpoint_info(true)),
            [0xF000_0101, 0x01, 0, 0]
        );
        assert_eq!(
            words(&m.with_stream_configuration(true)),
            [0xF000_0101, 0x10, 0, 0]
        );
    }

    /// Figure 13: Endpoint Info Notification; Number of Function Blocks 0 to 0x20.
    #[test]
    fn endpoint_info_matches_figure_13() {
        let m = EndpointInfoNotification::try_new(true, 0x20, true, true, true, true).unwrap();
        assert_eq!(words(&m), [0xF001_0101, 0xA000_0303, 0, 0]);
        assert_eq!(m.number_of_function_blocks(), 0x20);
        assert!(m.static_function_blocks() && m.midi2() && m.midi1() && m.rxjr() && m.txjr());
        let m = EndpointInfoNotification::try_new(false, 3, true, false, false, true).unwrap();
        assert_eq!(words(&m), [0xF001_0101, 0x0300_0201, 0, 0]);
        assert_eq!(
            EndpointInfoNotification::try_new(false, 0x21, true, false, false, false),
            Err(UmpError::OutOfRange)
        );
    }

    /// Figure 14: Device Identity Notification, 7-bit bytes with LSB before MSB.
    #[test]
    fn device_identity_matches_figure_14() {
        let m =
            DeviceIdentityNotification::try_new([0x00, 0x21, 0x09], 0x0123, 0x0204, [1, 2, 3, 4])
                .unwrap();
        // Family 0x0123: LSB 0x23, MSB 0x02. Model 0x0204: LSB 0x04, MSB 0x04.
        assert_eq!(
            words(&m),
            [0xF002_0000, 0x0000_2109, 0x2302_0404, 0x0102_0304]
        );
        assert_eq!(m.device_manufacturer(), [0x00, 0x21, 0x09]);
        assert_eq!(m.device_family(), 0x0123);
        assert_eq!(m.device_family_model_number(), 0x0204);
        assert_eq!(m.software_revision_level(), [1, 2, 3, 4]);
        // A 1-byte System Exclusive ID is in the first byte, then two zero bytes (7.1.3).
        let one = DeviceIdentityNotification::try_new([0x43, 0, 0], 0, 0, [0; 4]).unwrap();
        assert_eq!(words(&one)[1], 0x0043_0000);
        assert_eq!(
            DeviceIdentityNotification::try_new([0x80, 0, 0], 0, 0, [0; 4]),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            DeviceIdentityNotification::try_new([0; 3], 0x4000, 0, [0; 4]),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            DeviceIdentityNotification::try_new([0; 3], 0, 0, [0, 0, 0, 0x80]),
            Err(UmpError::OutOfRange)
        );
    }

    /// Figures 18 and 19: Stream Configuration Request and Notification; reserved Protocol
    /// values are not decoded.
    #[test]
    fn stream_configuration_matches_figures_18_19() {
        let r = StreamConfigurationRequest::new(StreamProtocol::Midi2, true, false);
        assert_eq!(words(&r), [0xF005_0202, 0, 0, 0]);
        assert_eq!(r.protocol(), StreamProtocol::Midi2);
        let n = StreamConfigurationNotification::new(StreamProtocol::Midi1, false, true);
        assert_eq!(words(&n), [0xF006_0101, 0, 0, 0]);
        assert_eq!(n.protocol(), StreamProtocol::Midi1);
        assert_eq!(StreamProtocol::from_bits(0x03), None);
        assert_eq!(UmpStreamMessage::parse(&[0xF006_0303, 0, 0, 0]), None);
        assert_eq!(
            StreamConfigurationRequest::from_words(&[0xF005_0000, 0, 0, 0]),
            Err(UmpError::UnexpectedMessage)
        );
    }

    /// Figures 20 and 21: Function Block Discovery; 0x00 to 0x1F or 0xFF.
    #[test]
    fn function_block_discovery_matches_figures_20_21() {
        let m = FunctionBlockDiscovery::try_new(FunctionBlockDiscovery::ALL, true, true).unwrap();
        assert_eq!(words(&m), [0xF010_FF03, 0, 0, 0]);
        let m = FunctionBlockDiscovery::try_new(0x1F, true, false).unwrap();
        assert_eq!(words(&m), [0xF010_1F01, 0, 0, 0]);
        assert!(m.info() && !m.name());
        assert_eq!(
            FunctionBlockDiscovery::try_new(0x20, true, false),
            Err(UmpError::OutOfRange)
        );
        assert_eq!(
            FunctionBlockDiscovery::try_new(0xFE, true, false),
            Err(UmpError::OutOfRange)
        );
    }

    fn block() -> FunctionBlockInfo {
        FunctionBlockInfo {
            active: true,
            number: 5,
            direction: FunctionBlockDirection::Bidirectional,
            midi1: FunctionBlockMidi1::NotMidi1,
            ui_hint: UiHint::SenderReceiver,
            first_group: Group::new(2).unwrap(),
            groups_spanned: 4,
            zmidi_ci_version: 0x02,
            max_sysex8_streams: 1,
        }
    }

    /// Figure 22: Function Block Info Notification field positions.
    #[test]
    fn function_block_info_matches_figure_22() {
        let m = FunctionBlockInfoNotification::try_new(block()).unwrap();
        // a=1, number 5 -> 0x85; reserved 00, ui 11, m1.0 00, dir 11 -> 0x33.
        assert_eq!(words(&m), [0xF011_8533, 0x0204_0201, 0, 0]);
        assert_eq!(m.info(), Some(block()));
        let port = FunctionBlockInfo {
            midi1: FunctionBlockMidi1::Restricted,
            groups_spanned: 1,
            direction: FunctionBlockDirection::Input,
            ui_hint: UiHint::Receiver,
            ..block()
        };
        let m = FunctionBlockInfoNotification::try_new(port).unwrap();
        assert_eq!(words(&m)[0], 0xF011_8519);
        assert_eq!(m.info(), Some(port));
    }

    /// 7.1.8: a MIDI 1.0 Port spans one Group (REQ-STREAM-011); Output with a Receiver hint is
    /// rejected (REQ-STREAM-012); the number is 0 to 31 and the span 1 to 16.
    #[test]
    fn function_block_info_rejects_invalid_blocks() {
        let bad = [
            FunctionBlockInfo {
                midi1: FunctionBlockMidi1::Unrestricted,
                ..block()
            },
            FunctionBlockInfo {
                midi1: FunctionBlockMidi1::Restricted,
                ..block()
            },
            FunctionBlockInfo {
                direction: FunctionBlockDirection::Output,
                ui_hint: UiHint::Receiver,
                ..block()
            },
            FunctionBlockInfo {
                number: 32,
                ..block()
            },
            FunctionBlockInfo {
                groups_spanned: 0,
                ..block()
            },
            FunctionBlockInfo {
                groups_spanned: 17,
                ..block()
            },
        ];
        for info in bad {
            assert_eq!(
                FunctionBlockInfoNotification::try_new(info),
                Err(UmpError::OutOfRange)
            );
        }
        // Reserved Direction (0b00) or MIDI 1.0 value (0x03) on receive: no field values.
        let m = UmpStreamMessage::parse(&[0xF011_8530, 0x0204_0201, 0, 0]);
        let Some(UmpStreamMessage::FunctionBlockInfoNotification(m)) = m else {
            panic!()
        };
        assert_eq!(m.info(), None);
        let m = UmpStreamMessage::parse(&[0xF011_853F, 0x0204_0201, 0, 0]);
        let Some(UmpStreamMessage::FunctionBlockInfoNotification(m)) = m else {
            panic!()
        };
        assert_eq!(m.info(), None);
    }

    /// Figures 24 and 25: Start of Clip and End of Clip.
    #[test]
    fn clip_markers_match_figures_24_25() {
        assert_eq!(words(&StartOfClip::new()), [0xF020_0000, 0, 0, 0]);
        assert_eq!(words(&EndOfClip::new()), [0xF021_0000, 0, 0, 0]);
    }

    /// Table of 7.1: the defined Status values; others are not decoded (REQ-UMP-004).
    #[test]
    fn ump_stream_status_values() {
        for v in [
            0x000u16, 0x001, 0x002, 0x003, 0x004, 0x005, 0x006, 0x010, 0x011, 0x012, 0x020, 0x021,
        ] {
            assert_eq!(UmpStreamStatus::from_bits(v).unwrap().into_bits(), v);
        }
        for v in [0x007u16, 0x00F, 0x013, 0x022, 0x3FF] {
            assert_eq!(UmpStreamStatus::from_bits(v), None);
        }
        for f in 0..4u8 {
            assert_eq!(StreamFormat::from_bits(f).into_bits(), f);
        }
    }

    /// Figures 11 to 25: a fixed-size message is Complete only; another Format is not decoded.
    #[test]
    fn fixed_messages_need_complete_format() {
        assert!(UmpStreamMessage::parse(&[0xF000_0101, 0, 0, 0]).is_some());
        for format in 1..4u32 {
            assert_eq!(
                UmpStreamMessage::parse(&[0xF000_0101 | (format << 26), 0, 0, 0]),
                None
            );
        }
    }

    /// 2.1.3: the reader clears reserved bits.
    #[test]
    fn ump_stream_reader_clears_reserved_bits() {
        let m =
            UmpStreamMessage::parse(&[0xF000_0101, 0xFFFF_FFE1, 0xFFFF_FFFF, 0xFFFF_FFFF]).unwrap();
        assert_eq!(m.into_words(), [0xF000_0101, 0x01, 0, 0]);
        let m =
            UmpStreamMessage::parse(&[0xF020_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF]).unwrap();
        assert_eq!(m.into_words(), [0xF020_0000, 0, 0, 0]);
        let m =
            UmpStreamMessage::parse(&[0xF002_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF]).unwrap();
        assert_eq!(
            m.into_words(),
            [0xF002_0000, 0x007F_7F7F, 0x7F7F_7F7F, 0x7F7F_7F7F]
        );
        let m =
            UmpStreamMessage::parse(&[0xF011_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF]).unwrap();
        assert_eq!(m.into_words(), [0xF011_FF3F, 0xFFFF_FFFF, 0, 0]);
    }

    fn text_words(packets: StreamTextPackets<'_>) -> Vec<[u32; 4]> {
        packets.map(|p| words(&p)).collect()
    }

    /// Figure 15: an Endpoint Name of up to 14 bytes is one Complete UMP; the rest is 0x00.
    #[test]
    fn endpoint_name_matches_figure_15() {
        let w = text_words(StreamTextPackets::endpoint_name("Zetaohm").unwrap());
        assert_eq!(w, [[0xF003_5A65, 0x7461_6F68, 0x6D00_0000, 0]]);
        let w = text_words(StreamTextPackets::endpoint_name("ABCDEFGHIJKLMN").unwrap());
        assert_eq!(w, [[0xF003_4142, 0x4344_4546, 0x4748_494A, 0x4B4C_4D4E]]);
        let w = text_words(StreamTextPackets::endpoint_name("").unwrap());
        assert_eq!(w, [[0xF003_0000, 0, 0, 0]]);
    }

    /// 7.1.4: a longer name is Start, Continue, End with full packets before the last;
    /// 98 bytes is the most (Start, five Continue, End).
    #[test]
    fn endpoint_name_splits_into_start_continue_end() {
        let name: String = "x".repeat(98);
        let p: Vec<_> = StreamTextPackets::endpoint_name(&name).unwrap().collect();
        assert_eq!(p.len(), 7);
        assert_eq!(p[0].format(), StreamFormat::Start);
        assert!(p[1..6].iter().all(|p| p.format() == StreamFormat::Continue));
        assert_eq!(p[6].format(), StreamFormat::End);
        assert!(p.iter().all(|p| p.text().len() == 14));
        let p: Vec<_> = StreamTextPackets::endpoint_name("0123456789abcdef")
            .unwrap()
            .collect();
        assert_eq!(p.len(), 2);
        assert_eq!(
            (p[0].format(), p[0].text().len()),
            (StreamFormat::Start, 14)
        );
        assert_eq!(
            (p[1].format(), &*p[1].text()),
            (StreamFormat::End, &b"ef"[..])
        );
        assert_eq!(
            StreamTextPackets::endpoint_name(&"x".repeat(99)).err(),
            Some(UmpError::OutOfRange)
        );
    }

    /// 1.8, REQ-TEXT-002: no Byte Order Mark; a zero byte would end the text early.
    #[test]
    fn stream_text_rejects_bom_and_zero() {
        assert!(StreamTextPackets::endpoint_name("\u{FEFF}Name").is_err());
        assert!(StreamTextPackets::endpoint_name("Na\0me").is_err());
        assert!(StreamTextPackets::function_block_name(0, "\u{FEFF}").is_err());
        // UTF-8 text is written as is: "é" is two bytes.
        let w = text_words(StreamTextPackets::endpoint_name("é").unwrap());
        assert_eq!(w[0][0], 0xF003_C3A9);
    }

    /// Figure 16: Product Instance Id, ASCII 32 to 126, at most 42 bytes (Start, one Continue,
    /// End).
    #[test]
    fn product_instance_id_matches_figure_16() {
        let w = text_words(StreamTextPackets::product_instance_id("SN-0042").unwrap());
        assert_eq!(w, [[0xF004_534E, 0x2D30_3034, 0x3200_0000, 0]]);
        let id = "A".repeat(42);
        let p: Vec<_> = StreamTextPackets::product_instance_id(&id)
            .unwrap()
            .collect();
        assert_eq!(p.len(), 3);
        assert_eq!(
            StreamTextPackets::product_instance_id(&"A".repeat(43)).err(),
            Some(UmpError::OutOfRange)
        );
        for bad in ["tab\t", "é", "del\u{7F}"] {
            assert!(
                StreamTextPackets::product_instance_id(bad).is_err(),
                "{bad:?}"
            );
        }
    }

    /// Figure 23: Function Block Name with the number in the first data byte, then 13 bytes;
    /// at most 91 bytes.
    #[test]
    fn function_block_name_matches_figure_23() {
        let w = text_words(StreamTextPackets::function_block_name(3, "Synth").unwrap());
        assert_eq!(w, [[0xF012_0353, 0x796E_7468, 0, 0]]);
        let name = "y".repeat(91);
        let p: Vec<_> = StreamTextPackets::function_block_name(31, &name)
            .unwrap()
            .collect();
        assert_eq!(p.len(), 7);
        assert!(p
            .iter()
            .all(|p| p.function_block_number() == Some(31) && p.text().len() == 13));
        assert!(StreamTextPackets::function_block_name(32, "x").is_err());
        assert!(StreamTextPackets::function_block_name(0, &"y".repeat(92)).is_err());
        // A Function Block Name with a number above 31 is not decoded (7.1.9).
        assert_eq!(UmpStreamMessage::parse(&[0xF012_2053, 0, 0, 0]), None);
    }

    /// 7.1.4: the text ends at the first 0x00; the reader clears the bytes after it.
    #[test]
    fn stream_text_reader_ends_at_zero() {
        let Some(UmpStreamMessage::Text(p)) =
            UmpStreamMessage::parse(&[0xF003_4100, 0x4242_4242, 0, 0])
        else {
            panic!()
        };
        assert_eq!(&*p.text(), b"A");
        assert_eq!(
            UmpStreamMessage::Text(p).into_words(),
            [0xF003_4100, 0, 0, 0]
        );
    }

    fn push_all(r: &mut StreamTextReassembler, packets: &[[u32; 4]]) -> Vec<String> {
        let mut out = Vec::new();
        for w in packets {
            if let Some(StreamTextEvent::Complete(t)) = r.push(w) {
                out.push(String::from(t.as_str().unwrap()));
            }
        }
        out
    }

    /// 7.1.4, 7.1.9: the reassembler gives the text with its kind and Function Block Number.
    #[test]
    fn stream_text_reassembler_gives_complete_text() {
        let mut r = StreamTextReassembler::new();
        let name = "Zetaohm MIDI 2.0 Sequencer with a long name";
        let packets = text_words(StreamTextPackets::endpoint_name(name).unwrap());
        assert_eq!(push_all(&mut r, &packets), [name]);
        let packets = text_words(StreamTextPackets::function_block_name(7, "Drums").unwrap());
        match r.push(&packets[0]) {
            Some(StreamTextEvent::Complete(t)) => {
                assert_eq!(t.kind(), StreamTextKind::FunctionBlockName);
                assert_eq!(t.function_block_number(), Some(7));
                assert_eq!(t.as_bytes(), b"Drums");
            }
            other => panic!("{other:?}"),
        }
        // Other UMP Stream messages and other Message Types give no event.
        assert_eq!(r.push(&[0xF000_0101, 0, 0, 0]), None);
        assert_eq!(r.push(&[0x2090_3C40]), None);
        assert_eq!(r.push(&[]), None);
    }

    /// The reassembler ends a text that a new Start or Complete, or a packet of another kind,
    /// interrupts; a Continue or End with no Start gives no event.
    #[test]
    fn stream_text_reassembler_reports_incomplete_text() {
        let mut r = StreamTextReassembler::new();
        let long = text_words(StreamTextPackets::endpoint_name(&"n".repeat(30)).unwrap());
        let other = text_words(StreamTextPackets::product_instance_id("ID").unwrap());
        let block = text_words(StreamTextPackets::function_block_name(1, &"f".repeat(20)).unwrap());
        assert_eq!(r.push(&long[1]), None);
        assert_eq!(r.push(&long[0]), None);
        assert_eq!(
            r.push(&other[0]),
            Some(StreamTextEvent::Incomplete {
                kind: StreamTextKind::EndpointName
            })
        );
        assert!(matches!(
            r.push(&other[0]),
            Some(StreamTextEvent::Complete(_))
        ));
        assert_eq!(r.push(&long[0]), None);
        assert_eq!(
            r.push(&block[1]),
            Some(StreamTextEvent::Incomplete {
                kind: StreamTextKind::EndpointName
            })
        );
        assert_eq!(r.push(&block[0]), None);
        assert_eq!(
            r.push(&long[0]),
            Some(StreamTextEvent::Incomplete {
                kind: StreamTextKind::FunctionBlockName
            })
        );
        r.reset();
        assert_eq!(r.push(&long[2]), None);
        // A text longer than 98 bytes ends with Incomplete.
        let max = text_words(StreamTextPackets::endpoint_name(&"m".repeat(98)).unwrap());
        for w in &max[..6] {
            assert_eq!(r.push(w), None);
        }
        assert_eq!(r.push(&max[5]), None);
        assert_eq!(
            r.push(&max[6]),
            Some(StreamTextEvent::Incomplete {
                kind: StreamTextKind::EndpointName
            })
        );
    }

    /// DEC-UMP-003: every valid text splits into packets that reassemble to the same text.
    #[test]
    fn prop_stream_text_round_trip() {
        proptest!(|(name in "[^\u{0}\u{FEFF}]{0,24}", id in "[ -~]{0,42}", block in 0u8..32)| {
            let mut r = StreamTextReassembler::new();
            if name.len() <= 98 {
                let packets = text_words(StreamTextPackets::endpoint_name(&name).unwrap());
                prop_assert_eq!(push_all(&mut r, &packets), [name.clone()]);
            }
            let packets = text_words(StreamTextPackets::product_instance_id(&id).unwrap());
            prop_assert_eq!(push_all(&mut r, &packets), [id.clone()]);
            if name.len() <= 91 {
                let packets = text_words(StreamTextPackets::function_block_name(block, &name).unwrap());
                prop_assert_eq!(push_all(&mut r, &packets), [name]);
            }
        });
    }

    /// The reader never panics, clears reserved bits, and gives back what it decoded.
    #[test]
    fn prop_ump_stream_parse_never_panics_and_round_trips() {
        proptest!(|(w in any::<[u32; 4]>(), status in prop::sample::select(&[0x000u32, 0x001, 0x002, 0x003, 0x004, 0x005, 0x006, 0x010, 0x011, 0x012, 0x020, 0x021][..]))| {
            let w0 = 0xF000_0000 | (w[0] & 0x0C00_FFFF) | (status << 16);
            let words = [w0, w[1], w[2], w[3]];
            if let Some(m) = UmpStreamMessage::parse(&words) {
                let out = m.into_words();
                prop_assert_eq!(out[0] >> 16, w0 >> 16);
                prop_assert_eq!(UmpStreamMessage::parse(&out), Some(m));
            }
            let _ = UmpStreamMessage::parse(&w);
            let mut r = StreamTextReassembler::new();
            let _ = r.push(&words);
            let _ = r.push(&w);
        });
    }
}
