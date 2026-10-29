//! MIDI-CI Management messages (M2-101-UM v1.2.1 section 5, Sub-ID#2 0x70-0x7F).
//!
//! - Discovery 0x70 (Table 6), Reply to Discovery 0x71 (Table 8), Inquiry: Endpoint
//!   Information 0x72 (Table 9), Reply to Endpoint Information 0x73 (Table 11), ACK 0x7D
//!   (Table 13), Invalidate MUID 0x7E (Table 12), NAK 0x7F (Table 15).
//! - Readers take the parsed [`Header`] and the body after the Destination MUID, read only
//!   the fields of the received Version and ignore later bytes (5.3, 5.4).
//! - ACK and NAK are never larger than the receiver's Receivable Maximum SysEx Message Size,
//!   or 128 bytes where it is not known; the Message Text is shortened (DEC-011 D-11).

use crate::message::{
    escape_each, need, start, u14, u28, Category, DeviceId, Emit, Framing, Header, Muid,
    ParseError, SubId2, TextChars, WriteError,
};

/// A System Exclusive manufacturer ID (REQ-CI-DISC-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManufacturerId {
    /// One-byte ID (0x01-0x7D).
    OneByte(u8),
    /// Three-byte ID, first byte 0x00.
    ThreeByte([u8; 3]),
}

impl ManufacturerId {
    /// Reads three bytes: first byte 0x00 gives a three-byte ID, else a one-byte ID.
    pub const fn from_bytes(b: [u8; 3]) -> Self {
        if b[0] == 0 {
            ManufacturerId::ThreeByte(b)
        } else {
            ManufacturerId::OneByte(b[0])
        }
    }

    /// Three bytes; a one-byte ID is written as ID 0x00 0x00.
    pub const fn to_bytes(self) -> [u8; 3] {
        match self {
            ManufacturerId::OneByte(id) => [id, 0, 0],
            ManufacturerId::ThreeByte(b) => b,
        }
    }
}

/// Device identity of Discovery and Reply to Discovery (Tables 6, 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceIdentity {
    /// Device Manufacturer, 3 bytes.
    pub manufacturer: ManufacturerId,
    /// Device Family, 14 bits.
    pub family: u16,
    /// Device Family Model Number, 14 bits.
    pub model: u16,
    /// Software Revision Level, 4 bytes of 7 bits.
    pub revision: [u8; 4],
}

impl DeviceIdentity {
    fn read(b: &[u8]) -> Self {
        DeviceIdentity {
            manufacturer: ManufacturerId::from_bytes([b[0], b[1], b[2]]),
            family: u14(&b[3..5]),
            model: u14(&b[5..7]),
            revision: [b[7], b[8], b[9], b[10]],
        }
    }

    fn write(&self, w: &mut Emit<'_>) {
        w.bytes(&self.manufacturer.to_bytes());
        w.u14(self.family);
        w.u14(self.model);
        w.bytes(&self.revision);
    }
}

/// Capability Inquiry Category Supported bitmap (Table 7, REQ-CI-DISC-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Categories {
    /// D1, deprecated.
    pub protocol_negotiation: bool,
    /// D2.
    pub profile_configuration: bool,
    /// D3.
    pub property_exchange: bool,
    /// D4.
    pub process_inquiry: bool,
}

impl Categories {
    /// Reads the bitmap; reserved bits D0, D5, D6 are ignored.
    pub const fn from_byte(b: u8) -> Self {
        Categories {
            protocol_negotiation: b & 0x02 != 0,
            profile_configuration: b & 0x04 != 0,
            property_exchange: b & 0x08 != 0,
            process_inquiry: b & 0x10 != 0,
        }
    }

    /// The bitmap; reserved bits are 0.
    pub const fn to_byte(self) -> u8 {
        (self.protocol_negotiation as u8) << 1
            | (self.profile_configuration as u8) << 2
            | (self.property_exchange as u8) << 3
            | (self.process_inquiry as u8) << 4
    }
}

/// Smallest Receivable Maximum SysEx Message Size for `categories` (5.5.3, Appendix F item 1;
/// REQ-CI-LEN-003): 512 with Profile Configuration or Property Exchange, else 128.
const fn min_sysex_size(categories: Categories) -> u32 {
    if categories.profile_configuration || categories.property_exchange {
        512
    } else {
        128
    }
}

const IDENTITY_LEN: usize = 11;
/// Identity, categories and maximum SysEx size.
const DISCOVERY_V1_LEN: usize = IDENTITY_LEN + 1 + 4;

/// Discovery (Table 6, REQ-CI-DISC-007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Discovery {
    /// Device identity.
    pub identity: DeviceIdentity,
    /// Categories supported.
    pub categories: Categories,
    /// Receivable Maximum SysEx Message Size, 28 bits.
    pub max_sysex_size: u32,
    /// Initiator's Output Path ID; `None` in a version 1 message.
    pub output_path_id: Option<u8>,
}

impl Discovery {
    /// Reads the body of a Discovery message.
    pub fn read(header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        need(body, DISCOVERY_V1_LEN)?;
        let output_path_id = if header.version.minor() >= 2 {
            need(body, DISCOVERY_V1_LEN + 1)?;
            Some(body[DISCOVERY_V1_LEN])
        } else {
            None
        };
        Ok(Discovery {
            identity: DeviceIdentity::read(body),
            categories: Categories::from_byte(body[IDENTITY_LEN]),
            max_sysex_size: u28(&body[IDENTITY_LEN + 1..]),
            output_path_id,
        })
    }

    /// Writes Discovery: Device ID 0x7F, Destination Broadcast; Output Path ID 0 when `None`. A
    /// Receivable Maximum SysEx Message Size below 128, or below 512 with Profile Configuration or
    /// Property Exchange, is an InvalidValue error (REQ-CI-LEN-003).
    pub fn write(
        &self,
        source: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        if self.max_sysex_size < min_sysex_size(self.categories) {
            return Err(WriteError::InvalidValue);
        }
        let mut w = start(
            DeviceId::FunctionBlock,
            SubId2::Discovery,
            source,
            Muid::BROADCAST,
            framing,
            out,
        );
        self.identity.write(&mut w);
        w.u7(self.categories.to_byte());
        w.u28(self.max_sysex_size);
        w.u7(self.output_path_id.unwrap_or(0));
        w.finish()
    }
}

/// Reply to Discovery (Table 8, REQ-CI-DISC-012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscoveryReply {
    /// Device identity.
    pub identity: DeviceIdentity,
    /// Categories supported.
    pub categories: Categories,
    /// Receivable Maximum SysEx Message Size, 28 bits.
    pub max_sysex_size: u32,
    /// Initiator's Output Path ID; `None` in a version 1 message.
    pub output_path_id: Option<u8>,
    /// Function Block number; `None` for 0x7F (no Function Block) or version 1.
    pub function_block: Option<u8>,
}

impl DiscoveryReply {
    /// Reads the body of a Reply to Discovery message.
    pub fn read(header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        need(body, DISCOVERY_V1_LEN)?;
        let (output_path_id, function_block) = if header.version.minor() >= 2 {
            need(body, DISCOVERY_V1_LEN + 2)?;
            let fb = body[DISCOVERY_V1_LEN + 1];
            (Some(body[DISCOVERY_V1_LEN]), (fb != 0x7F).then_some(fb))
        } else {
            (None, None)
        };
        Ok(DiscoveryReply {
            identity: DeviceIdentity::read(body),
            categories: Categories::from_byte(body[IDENTITY_LEN]),
            max_sysex_size: u28(&body[IDENTITY_LEN + 1..]),
            output_path_id,
            function_block,
        })
    }

    /// Writes Reply to Discovery (Device ID 0x7F), with the size rule of [`Discovery::write`].
    pub fn write(
        &self,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        if self.max_sysex_size < min_sysex_size(self.categories) {
            return Err(WriteError::InvalidValue);
        }
        let mut w = start(
            DeviceId::FunctionBlock,
            SubId2::DiscoveryReply,
            source,
            destination,
            framing,
            out,
        );
        self.identity.write(&mut w);
        w.u7(self.categories.to_byte());
        w.u28(self.max_sysex_size);
        w.u7(self.output_path_id.unwrap_or(0));
        match self.function_block {
            Some(fb) if fb < 0x7F => w.u7(fb),
            Some(_) => return Err(WriteError::InvalidValue),
            None => w.u7(0x7F),
        }
        w.finish()
    }
}

/// Status of the Endpoint Information messages (Table 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointInfoStatus {
    /// 0x00 Product Instance ID.
    ProductInstanceId,
    /// 0x01-0x7F reserved.
    Reserved(u8),
}

impl EndpointInfoStatus {
    fn from_byte(b: u8) -> Self {
        if b == 0 {
            EndpointInfoStatus::ProductInstanceId
        } else {
            EndpointInfoStatus::Reserved(b)
        }
    }

    fn to_byte(self) -> u8 {
        match self {
            EndpointInfoStatus::ProductInstanceId => 0,
            EndpointInfoStatus::Reserved(b) => b,
        }
    }
}

/// Inquiry: Endpoint Information (Table 9, REQ-CI-EPI-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointInfoInquiry {
    /// Requested information.
    pub status: EndpointInfoStatus,
}

impl EndpointInfoInquiry {
    /// Reads the body.
    pub fn read(_header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        need(body, 1)?;
        Ok(EndpointInfoInquiry {
            status: EndpointInfoStatus::from_byte(body[0]),
        })
    }

    /// Writes the inquiry (Device ID 0x7F); a reserved status is an error.
    pub fn write(
        &self,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        if self.status != EndpointInfoStatus::ProductInstanceId {
            return Err(WriteError::InvalidValue);
        }
        let mut w = start(
            DeviceId::FunctionBlock,
            SubId2::EndpointInfo,
            source,
            destination,
            framing,
            out,
        );
        w.u7(self.status.to_byte());
        w.finish()
    }
}

/// Checks a Product Instance ID: at most 42 bytes of ASCII 32-126 (REQ-CI-EPI-008).
pub fn product_instance_id_check(id: &[u8]) -> bool {
    id.len() <= 42 && id.iter().all(|b| (32..=126).contains(b))
}

/// Reply to Endpoint Information (Table 11, REQ-CI-EPI-005).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointInfoReply<'a> {
    /// Status of the inquiry.
    pub status: EndpointInfoStatus,
    /// Information Data.
    pub data: &'a [u8],
}

impl<'a> EndpointInfoReply<'a> {
    /// Reads the body.
    pub fn read(_header: &Header, body: &'a [u8]) -> Result<Self, ParseError> {
        need(body, 3)?;
        let len = u14(&body[1..3]) as usize;
        need(body, 3 + len)?;
        let reply = EndpointInfoReply {
            status: EndpointInfoStatus::from_byte(body[0]),
            data: &body[3..3 + len],
        };
        if reply.status == EndpointInfoStatus::ProductInstanceId
            && !product_instance_id_check(reply.data)
        {
            return Err(ParseError::InvalidValue);
        }
        Ok(reply)
    }

    /// Writes the reply (Device ID 0x7F).
    pub fn write(
        &self,
        source: Muid,
        destination: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        if self.status == EndpointInfoStatus::ProductInstanceId
            && !product_instance_id_check(self.data)
        {
            return Err(WriteError::InvalidValue);
        }
        let len = u16::try_from(self.data.len()).map_err(|_| WriteError::InvalidValue)?;
        let mut w = start(
            DeviceId::FunctionBlock,
            SubId2::EndpointInfoReply,
            source,
            destination,
            framing,
            out,
        );
        w.u7(self.status.to_byte());
        w.u14(len);
        w.bytes(self.data);
        w.finish()
    }
}

/// Invalidate MUID (Table 12, REQ-CI-INV-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidateMuid {
    /// The MUID to invalidate.
    pub target: Muid,
}

impl InvalidateMuid {
    /// Reads the body.
    pub fn read(_header: &Header, body: &[u8]) -> Result<Self, ParseError> {
        need(body, 4)?;
        Ok(InvalidateMuid {
            target: Muid::from_bytes([body[0], body[1], body[2], body[3]])?,
        })
    }

    /// Writes the message: Device ID 0x7F, Destination Broadcast.
    pub fn write(
        &self,
        source: Muid,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let mut w = start(
            DeviceId::FunctionBlock,
            SubId2::InvalidateMuid,
            source,
            Muid::BROADCAST,
            framing,
            out,
        );
        w.u28(self.target.value());
        w.finish()
    }
}

/// Range of a status code (Tables 14 and 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusRange {
    /// ACK 0x00-0x0F.
    Success,
    /// ACK 0x10-0x1F; NAK 0x20-0x3F (further action recommended).
    Notification,
    /// NAK 0x00-0x1F: do not retry.
    FailNoRetry,
    /// NAK 0x40-0x5F: retry recommended.
    FailRetry,
    /// NAK 0x60-0x7F.
    Reserved,
    /// No range defined.
    Other,
}

/// ACK Status Code with its Status Data (Table 14, REQ-CI-ACK-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckStatus {
    /// 0x00.
    Ack,
    /// 0x10, wait time in 100 ms units.
    TimeoutWait(u8),
    /// 0x11 Flow Control: send next Chunks.
    FlowControlSendNext,
    /// Any other code with its data.
    Unknown { code: u8, data: u8 },
}

impl AckStatus {
    fn from_bytes(code: u8, data: u8) -> Self {
        match code {
            0x00 => AckStatus::Ack,
            0x10 => AckStatus::TimeoutWait(data),
            0x11 => AckStatus::FlowControlSendNext,
            _ => AckStatus::Unknown { code, data },
        }
    }

    fn to_bytes(self) -> (u8, u8) {
        match self {
            AckStatus::Ack => (0x00, 0),
            AckStatus::TimeoutWait(t) => (0x10, t),
            AckStatus::FlowControlSendNext => (0x11, 0),
            AckStatus::Unknown { code, data } => (code, data),
        }
    }

    /// The range of the code (Table 14).
    pub fn range(self) -> StatusRange {
        match self.to_bytes().0 {
            0x00..=0x0F => StatusRange::Success,
            0x10..=0x1F => StatusRange::Notification,
            _ => StatusRange::Other,
        }
    }
}

/// NAK Status Code with its Status Data (Table 16, REQ-CI-NAK-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NakStatus {
    /// 0x00.
    Nak,
    /// 0x01 MIDI-CI message not supported.
    NotSupported,
    /// 0x02 MIDI-CI version not supported.
    VersionNotSupported,
    /// 0x03 Channel/Group/Function Block not in use.
    NotInUse,
    /// 0x04 Profile not supported.
    ProfileNotSupported,
    /// 0x12 Flow Control: resend the most recent Chunk.
    FlowControlResend,
    /// 0x20 terminate transaction; Status Data 0x01 when the Initiator terminates.
    Terminate { by_initiator: bool },
    /// 0x21 Property Exchange Chunks out of sequence.
    ChunksOutOfSequence,
    /// 0x40 error, please retry.
    Error,
    /// 0x41 message malformed.
    Malformed,
    /// 0x42 timeout.
    Timeout,
    /// 0x43 busy; retry time in 100 ms units.
    Busy(u8),
    /// Any other code with its data.
    Unknown { code: u8, data: u8 },
}

impl NakStatus {
    fn from_bytes(code: u8, data: u8) -> Self {
        use NakStatus::*;
        match code {
            0x00 => Nak,
            0x01 => NotSupported,
            0x02 => VersionNotSupported,
            0x03 => NotInUse,
            0x04 => ProfileNotSupported,
            0x12 => FlowControlResend,
            0x20 if data <= 1 => Terminate {
                by_initiator: data == 1,
            },
            0x21 => ChunksOutOfSequence,
            0x40 => Error,
            0x41 => Malformed,
            0x42 => Timeout,
            0x43 => Busy(data),
            _ => Unknown { code, data },
        }
    }

    fn to_bytes(self) -> (u8, u8) {
        use NakStatus::*;
        match self {
            Nak => (0x00, 0),
            NotSupported => (0x01, 0),
            VersionNotSupported => (0x02, 0),
            NotInUse => (0x03, 0),
            ProfileNotSupported => (0x04, 0),
            FlowControlResend => (0x12, 0),
            Terminate { by_initiator } => (0x20, by_initiator as u8),
            ChunksOutOfSequence => (0x21, 0),
            Error => (0x40, 0),
            Malformed => (0x41, 0),
            Timeout => (0x42, 0),
            Busy(t) => (0x43, t),
            Unknown { code, data } => (code, data),
        }
    }

    /// The range of the code (Table 16).
    pub fn range(self) -> StatusRange {
        match self.to_bytes().0 {
            0x00..=0x1F => StatusRange::FailNoRetry,
            0x20..=0x3F => StatusRange::Notification,
            0x40..=0x5F => StatusRange::FailRetry,
            _ => StatusRange::Reserved,
        }
    }
}

/// The 5 detail bytes of an ACK or NAK (5.10.3, 5.11.3; REQ-CI-ACK-004, REQ-CI-NAK-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Details {
    /// Profile Configuration: the Profile ID bytes.
    Profile([u8; 5]),
    /// Property Exchange: Request ID and Chunk number.
    PropertyExchange { request_id: u8, chunk: u16 },
    /// Every other Category: reserved.
    Reserved,
}

impl Details {
    /// Reads the details for the Category of the Original Transaction Sub-ID#2.
    pub fn read(category: Category, b: [u8; 5]) -> Self {
        match category {
            Category::ProfileConfiguration => Details::Profile(b),
            Category::PropertyExchange => Details::PropertyExchange {
                request_id: b[0],
                chunk: u14(&b[1..3]),
            },
            _ => Details::Reserved,
        }
    }

    /// The 5 bytes; reserved bytes are 0x00.
    pub fn to_bytes(self) -> [u8; 5] {
        match self {
            Details::Profile(b) => b,
            Details::PropertyExchange { request_id, chunk } => [
                request_id,
                (chunk & 0x7F) as u8,
                ((chunk >> 7) & 0x7F) as u8,
                0,
                0,
            ],
            Details::Reserved => [0; 5],
        }
    }
}

/// Size used when the receiver reported no Receivable Maximum SysEx Message Size (D-11).
const DEFAULT_MAX_SYSEX: u32 = 128;
/// Body length of ACK/NAK before the Message Text.
const ACK_FIXED: usize = 1 + 1 + 1 + 5 + 2;

#[allow(clippy::too_many_arguments)]
fn write_ack_nak(
    sub_id2: SubId2,
    device_id: DeviceId,
    source: Muid,
    destination: Muid,
    original: u8,
    status: (u8, u8),
    details: Details,
    text: &str,
    max_sysex_size: Option<u32>,
    framing: Framing,
    out: &mut [u8],
) -> Result<usize, WriteError> {
    let max = max_sysex_size.unwrap_or(DEFAULT_MAX_SYSEX) as usize;
    let framing_len = if framing == Framing::SysEx { 2 } else { 0 };
    let limit = max
        .checked_sub(Header::LEN + ACK_FIXED + framing_len)
        .ok_or(WriteError::InvalidValue)?;
    let mut w = start(device_id, sub_id2, source, destination, framing, out);
    w.u7(original);
    w.u7(status.0);
    w.u7(status.1);
    w.bytes(&details.to_bytes());
    let len_at = w.len();
    w.u14(0);
    let room = w.remaining().saturating_sub(framing_len / 2);
    let (n, _) = escape_each(text, limit.min(room).min(0x3FFF), |b| w.bytes(b));
    w.patch_u14(len_at, n as u16);
    w.finish()
}

/// Original Sub-ID#2, Status Code, Status Data, details, Message Text.
type AckNakFields<'a> = (u8, u8, u8, [u8; 5], &'a [u8]);

fn read_ack_nak(body: &[u8]) -> Result<AckNakFields<'_>, ParseError> {
    need(body, ACK_FIXED)?;
    let len = u14(&body[8..10]) as usize;
    need(body, ACK_FIXED + len)?;
    Ok((
        body[0],
        body[1],
        body[2],
        [body[3], body[4], body[5], body[6], body[7]],
        &body[10..10 + len],
    ))
}

/// MIDI-CI ACK (Table 13, REQ-CI-ACK-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ack<'a> {
    /// Original Transaction Sub-ID#2 Classification.
    pub original_sub_id2: u8,
    /// Status Code and Status Data.
    pub status: AckStatus,
    /// The 5 detail bytes.
    pub details: Details,
    /// Raw Message Text.
    pub text: &'a [u8],
}

impl<'a> Ack<'a> {
    /// Reads the body.
    pub fn read(_header: &Header, body: &'a [u8]) -> Result<Self, ParseError> {
        let (orig, code, data, details, text) = read_ack_nak(body)?;
        Ok(Ack {
            original_sub_id2: orig,
            status: AckStatus::from_bytes(code, data),
            details: Details::read(Category::from_sub_id2(orig), details),
            text,
        })
    }

    /// The decoded Message Text.
    pub fn text(&self) -> TextChars<'a> {
        TextChars::new(self.text)
    }

    /// Writes an ACK with `text` as Message Text; `self.text` is not used. The message fits
    /// `max_sysex_size` (128 when `None`) and the text is shortened to fit (REQ-CI-ACK-005).
    #[allow(clippy::too_many_arguments)]
    pub fn write(
        &self,
        text: &str,
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        max_sysex_size: Option<u32>,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        write_ack_nak(
            SubId2::Ack,
            device_id,
            source,
            destination,
            self.original_sub_id2,
            self.status.to_bytes(),
            self.details,
            text,
            max_sysex_size,
            framing,
            out,
        )
    }
}

/// MIDI-CI NAK (Table 15, REQ-CI-NAK-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Nak<'a> {
    /// Original Transaction Sub-ID#2 Classification (0 in a version 1 NAK).
    pub original_sub_id2: u8,
    /// Status Code and Status Data.
    pub status: NakStatus,
    /// The 5 detail bytes.
    pub details: Details,
    /// Raw Message Text.
    pub text: &'a [u8],
}

impl<'a> Nak<'a> {
    /// Reads the body; a version 1 NAK has no data.
    pub fn read(header: &Header, body: &'a [u8]) -> Result<Self, ParseError> {
        if header.version.minor() < 2 {
            return Ok(Nak {
                original_sub_id2: 0,
                status: NakStatus::Nak,
                details: Details::Reserved,
                text: &[],
            });
        }
        let (orig, code, data, details, text) = read_ack_nak(body)?;
        Ok(Nak {
            original_sub_id2: orig,
            status: NakStatus::from_bytes(code, data),
            details: Details::read(Category::from_sub_id2(orig), details),
            text,
        })
    }

    /// The decoded Message Text.
    pub fn text(&self) -> TextChars<'a> {
        TextChars::new(self.text)
    }

    /// Writes a NAK with `text` as Message Text, with the size rule of [`Ack::write`].
    /// [`Self::write`] with no Message Text, the NAK that a Device sends by itself: the same bytes,
    /// with no text escape code in the build (OPT-049, step 4).
    pub(crate) fn write_without_text(
        &self,
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        max_sysex_size: Option<u32>,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        let max = max_sysex_size.unwrap_or(DEFAULT_MAX_SYSEX) as usize;
        let framing_len = if framing == Framing::SysEx { 2 } else { 0 };
        if max < Header::LEN + ACK_FIXED + framing_len {
            return Err(WriteError::InvalidValue);
        }
        let mut w = start(device_id, SubId2::Nak, source, destination, framing, out);
        let (code, data) = self.status.to_bytes();
        let d = self.details.to_bytes();
        // Message Length 0 (Table 15).
        w.bytes(&[
            self.original_sub_id2,
            code,
            data,
            d[0],
            d[1],
            d[2],
            d[3],
            d[4],
            0,
            0,
        ]);
        w.finish()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write(
        &self,
        text: &str,
        device_id: DeviceId,
        source: Muid,
        destination: Muid,
        max_sysex_size: Option<u32>,
        framing: Framing,
        out: &mut [u8],
    ) -> Result<usize, WriteError> {
        write_ack_nak(
            SubId2::Nak,
            device_id,
            source,
            destination,
            self.original_sub_id2,
            self.status.to_bytes(),
            self.details,
            text,
            max_sysex_size,
            framing,
            out,
        )
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::message::Version;
    use proptest::prelude::*;
    use std::string::String;

    const SRC: Muid = match Muid::new(0x0123_4567) {
        Ok(m) => m,
        Err(_) => panic!(),
    };
    const DST: Muid = match Muid::new(0x0765_4321) {
        Ok(m) => m,
        Err(_) => panic!(),
    };

    fn identity() -> DeviceIdentity {
        DeviceIdentity {
            manufacturer: ManufacturerId::OneByte(0x41),
            family: 0x1234,
            model: 0x0201,
            revision: [1, 2, 3, 4],
        }
    }

    fn parse(buf: &[u8]) -> (Header, &[u8]) {
        Header::parse(buf).unwrap()
    }

    #[test]
    fn discovery_bytes() {
        // Table 6; REQ-CI-DISC-007, REQ-CI-DISC-008, REQ-CI-DISC-009.
        let d = Discovery {
            identity: identity(),
            categories: Categories {
                profile_configuration: true,
                property_exchange: true,
                ..Default::default()
            },
            max_sysex_size: 512,
            output_path_id: Some(3),
        };
        let mut buf = [0u8; 64];
        let n = d.write(SRC, Framing::Bare, &mut buf).unwrap();
        assert_eq!(
            &buf[..n],
            &[
                0x7E, 0x7F, 0x0D, 0x70, 0x02, 0x67, 0x0A, 0x0D, 0x09, 0x7F, 0x7F, 0x7F,
                0x7F, // header
                0x41, 0x00, 0x00, 0x34, 0x24, 0x01, 0x04, 1, 2, 3, 4,    // identity
                0x0C, // categories D2 | D3
                0x00, 0x04, 0x00, 0x00, // 512
                0x03, // output path
            ]
        );
        let (h, body) = parse(&buf[..n]);
        assert_eq!(Discovery::read(&h, body), Ok(d));
        // REQ-CI-LEN-003: 512 or more with D2 or D3, 128 or more without them.
        for (categories, min) in [
            (d.categories, 512),
            (Categories::from_byte(0x04), 512),
            (Categories::from_byte(0x08), 512),
            (Categories::from_byte(0x10), 128),
            (Categories::default(), 128),
        ] {
            let low = Discovery {
                categories,
                max_sysex_size: min - 1,
                ..d
            };
            assert_eq!(
                low.write(SRC, Framing::Bare, &mut buf),
                Err(WriteError::InvalidValue),
                "{categories:?}"
            );
            let ok = Discovery {
                max_sysex_size: min,
                ..low
            };
            let n = ok.write(SRC, Framing::Bare, &mut buf).unwrap();
            assert_eq!(buf[n - 5..n - 1], le28(min));
        }
        // Version 1: no Output Path ID (REQ-CI-MSG-014).
        let small = Discovery {
            categories: Categories::default(),
            max_sysex_size: 128,
            ..d
        };
        let n = small.write(SRC, Framing::Bare, &mut buf).unwrap();
        buf[4] = 0x01;
        let (h, body) = parse(&buf[..n - 1]);
        assert_eq!(Discovery::read(&h, body).unwrap().output_path_id, None);
        // Version 2 needs the Output Path ID.
        buf[4] = 0x02;
        let (h, body) = parse(&buf[..n - 1]);
        assert_eq!(Discovery::read(&h, body), Err(ParseError::TooShort));
    }

    fn le28(v: u32) -> [u8; 4] {
        [
            (v & 0x7F) as u8,
            (v >> 7 & 0x7F) as u8,
            (v >> 14 & 0x7F) as u8,
            (v >> 21 & 0x7F) as u8,
        ]
    }

    #[test]
    fn categories_reserved_bits() {
        // Table 7; REQ-CI-DISC-009: one bit for each Category, reserved bits D0, D5, D6 ignored
        // and written as 0.
        assert_eq!(Categories::from_byte(0x7F).to_byte(), 0x1E);
        let one = |b: u8| Categories::from_byte(b);
        assert_eq!(
            one(0x02),
            Categories {
                protocol_negotiation: true,
                ..Default::default()
            }
        );
        assert_eq!(
            one(0x04),
            Categories {
                profile_configuration: true,
                ..Default::default()
            }
        );
        assert_eq!(
            one(0x08),
            Categories {
                property_exchange: true,
                ..Default::default()
            }
        );
        assert_eq!(
            one(0x10),
            Categories {
                process_inquiry: true,
                ..Default::default()
            }
        );
        assert_eq!(one(0x61), Categories::default());
        // The writer puts each Category in its own bit of the Discovery message.
        let mut buf = [0u8; 64];
        for bit in [0x02u8, 0x04, 0x08, 0x10] {
            let d = Discovery {
                identity: identity(),
                categories: one(bit),
                max_sysex_size: 512,
                output_path_id: Some(0),
            };
            let n = d.write(SRC, Framing::Bare, &mut buf).unwrap();
            assert_eq!((buf[24], n), (bit, 30));
        }
    }

    #[test]
    fn discovery_reply_round_trip() {
        // Table 8; REQ-CI-DISC-012.
        let r = DiscoveryReply {
            identity: DeviceIdentity {
                manufacturer: ManufacturerId::ThreeByte([0, 0x21, 0x09]),
                ..identity()
            },
            categories: Categories::default(),
            max_sysex_size: 128,
            output_path_id: Some(0),
            function_block: None,
        };
        let mut buf = [0u8; 64];
        let n = r.write(SRC, DST, Framing::SysEx, &mut buf).unwrap();
        // F0, header with Sub-ID#2 0x71, identity with a three-byte manufacturer, categories,
        // size 128, Output Path ID 0, no Function Block 0x7F, F7.
        assert_eq!(
            &buf[..n],
            &[
                0xF0, 0x7E, 0x7F, 0x0D, 0x71, 0x02, 0x67, 0x0A, 0x0D, 0x09, 0x21, 0x06, 0x15, 0x3B,
                0x00, 0x21, 0x09, 0x34, 0x24, 0x01, 0x04, 1, 2, 3, 4, 0x00, 0x00, 0x01, 0x00, 0x00,
                0x00, 0x7F, 0xF7
            ]
        );
        let (h, body) = parse(&buf[..n]);
        assert_eq!(DiscoveryReply::read(&h, body), Ok(r));
        let r2 = DiscoveryReply {
            function_block: Some(5),
            ..r
        };
        let n = r2.write(SRC, DST, Framing::Bare, &mut buf).unwrap();
        let (h, body) = parse(&buf[..n]);
        assert_eq!(DiscoveryReply::read(&h, body), Ok(r2));
        // Version 2 needs both fields; a version 1 reply has neither.
        assert_eq!(
            DiscoveryReply::read(&h, &body[..body.len() - 1]),
            Err(ParseError::TooShort)
        );
        buf[4] = 0x01;
        let (h, body) = parse(&buf[..n - 2]);
        assert_eq!(
            DiscoveryReply::read(&h, body),
            Ok(DiscoveryReply {
                output_path_id: None,
                function_block: None,
                ..r2
            })
        );
        // REQ-CI-LEN-003: 512 or more with Profile Configuration or Property Exchange.
        let pe = DiscoveryReply {
            categories: Categories::from_byte(0x08),
            ..r
        };
        assert_eq!(
            pe.write(SRC, DST, Framing::Bare, &mut buf),
            Err(WriteError::InvalidValue)
        );
        let pe = DiscoveryReply {
            max_sysex_size: 512,
            ..pe
        };
        assert!(pe.write(SRC, DST, Framing::Bare, &mut buf).is_ok());
        let low = DiscoveryReply {
            max_sysex_size: 127,
            ..r
        };
        assert_eq!(
            low.write(SRC, DST, Framing::Bare, &mut buf),
            Err(WriteError::InvalidValue)
        );
    }

    #[test]
    fn endpoint_info() {
        // Tables 9-11; REQ-CI-EPI-002, REQ-CI-EPI-005, REQ-CI-EPI-008.
        let mut buf = [0u8; 80];
        let q = EndpointInfoInquiry {
            status: EndpointInfoStatus::ProductInstanceId,
        };
        let n = q.write(SRC, DST, Framing::Bare, &mut buf).unwrap();
        // Table 9: Device ID 0x7F, Sub-ID#2 0x72, Status 0x00 (REQ-CI-EPI-002).
        assert_eq!(
            &buf[..n],
            &[0x7E, 0x7F, 0x0D, 0x72, 0x02, 0x67, 0x0A, 0x0D, 0x09, 0x21, 0x06, 0x15, 0x3B, 0x00]
        );
        let (h, body) = parse(&buf[..n]);
        assert_eq!(EndpointInfoInquiry::read(&h, body), Ok(q));
        assert_eq!(
            EndpointInfoInquiry::read(&h, &[0x05]),
            Ok(EndpointInfoInquiry {
                status: EndpointInfoStatus::Reserved(5)
            })
        );
        let bad = EndpointInfoInquiry {
            status: EndpointInfoStatus::Reserved(1),
        };
        assert_eq!(
            bad.write(SRC, DST, Framing::Bare, &mut buf),
            Err(WriteError::InvalidValue)
        );

        let r = EndpointInfoReply {
            status: EndpointInfoStatus::ProductInstanceId,
            data: b"SN-0001",
        };
        let n = r.write(SRC, DST, Framing::Bare, &mut buf).unwrap();
        assert_eq!(&buf[13..16], &[0x00, 0x07, 0x00]);
        let (h, body) = parse(&buf[..n]);
        assert_eq!(EndpointInfoReply::read(&h, body), Ok(r));
        assert_eq!(&buf[3..4], &[0x73]);
        assert_eq!(
            EndpointInfoReply::read(&h, &body[..body.len() - 1]),
            Err(ParseError::TooShort)
        );
        // REQ-CI-EPI-005: bytes after the Information Data are ignored.
        let mut long = body.to_vec();
        long.extend([0x41, 0x42]);
        assert_eq!(EndpointInfoReply::read(&h, &long), Ok(r));
        assert!(!product_instance_id_check(&[b'a'; 43]));
        assert!(!product_instance_id_check(b"a\x1F"));
        assert!(!product_instance_id_check(b"a\x7F"));
        assert!(product_instance_id_check(&[b'~'; 42]));
        assert!(product_instance_id_check(b" "));
        // REQ-CI-EPI-008: the writer gives an error for an ID that is too long or has a byte
        // outside 32-126, and the reader rejects one.
        for bad in [&b"a\x7F"[..], b"a\x1F", &[b'a'; 43]] {
            let w = EndpointInfoReply {
                status: EndpointInfoStatus::ProductInstanceId,
                data: bad,
            };
            assert_eq!(
                w.write(SRC, DST, Framing::Bare, &mut buf),
                Err(WriteError::InvalidValue)
            );
            // The same bytes with a reserved Status are written, then read as Status 0x00.
            let n = EndpointInfoReply {
                status: EndpointInfoStatus::Reserved(1),
                data: bad,
            }
            .write(SRC, DST, Framing::Bare, &mut buf)
            .unwrap();
            buf[13] = 0x00;
            let (h, body) = parse(&buf[..n]);
            assert_eq!(
                EndpointInfoReply::read(&h, body),
                Err(ParseError::InvalidValue)
            );
        }
        let max = EndpointInfoReply {
            status: EndpointInfoStatus::ProductInstanceId,
            data: &[b' '; 42],
        };
        let n = max.write(SRC, DST, Framing::Bare, &mut buf).unwrap();
        let (h, body) = parse(&buf[..n]);
        assert_eq!(EndpointInfoReply::read(&h, body), Ok(max));
    }

    #[test]
    fn invalidate_muid() {
        // Table 12; REQ-CI-INV-008.
        let mut buf = [0u8; 32];
        let m = InvalidateMuid { target: DST };
        let n = m.write(SRC, Framing::Bare, &mut buf).unwrap();
        assert_eq!(&buf[9..13], &[0x7F; 4]);
        let (h, body) = parse(&buf[..n]);
        assert_eq!(InvalidateMuid::read(&h, body), Ok(m));
    }

    #[test]
    fn ack_round_trip_and_details() {
        // Tables 13, 14; REQ-CI-ACK-001..004, REQ-CI-ACK-006.
        let mut buf = [0u8; 128];
        let a = Ack {
            original_sub_id2: 0x34,
            status: AckStatus::FlowControlSendNext,
            details: Details::PropertyExchange {
                request_id: 5,
                chunk: 300,
            },
            text: &[],
        };
        let n = a
            .write(
                "ok \u{266A}",
                DeviceId::FunctionBlock,
                SRC,
                DST,
                None,
                Framing::Bare,
                &mut buf,
            )
            .unwrap();
        assert_eq!(&buf[..4], &[0x7E, 0x7F, 0x0D, 0x7D]);
        assert_eq!(&buf[13..23], &[0x34, 0x11, 0x00, 5, 0x2C, 2, 0, 0, 9, 0]);
        let (h, body) = parse(&buf[..n]);
        let back = Ack::read(&h, body).unwrap();
        assert_eq!(back.details, a.details);
        assert_eq!(back.text().collect::<String>(), "ok \u{266A}");
        // REQ-CI-ACK-001: shorter than the Message Length needs: rejected; bytes after the
        // Message Text: ignored.
        assert_eq!(
            Ack::read(&h, &body[..body.len() - 1]),
            Err(ParseError::TooShort)
        );
        assert_eq!(Ack::read(&h, &body[..9]), Err(ParseError::TooShort));
        let mut long = body.to_vec();
        long.extend([0x41, 0x42]);
        let more = Ack::read(&h, &long).unwrap();
        assert_eq!(more, back);
        assert_eq!(more.text().collect::<String>(), "ok \u{266A}");
        let p = Details::read(Category::ProfileConfiguration, [0x7E, 1, 2, 3, 4]);
        assert_eq!(p, Details::Profile([0x7E, 1, 2, 3, 4]));
        assert_eq!(
            Details::read(Category::Management, [1; 5]).to_bytes(),
            [0; 5]
        );
        assert_eq!(AckStatus::from_bytes(0x05, 0).range(), StatusRange::Success);
        assert_eq!(AckStatus::from_bytes(0x10, 7), AckStatus::TimeoutWait(7));
    }

    #[test]
    fn ack_status_codes() {
        // Table 14; REQ-CI-ACK-003.
        assert_eq!(AckStatus::from_bytes(0x00, 0), AckStatus::Ack);
        assert_eq!(
            AckStatus::from_bytes(0x10, 0x7F),
            AckStatus::TimeoutWait(0x7F)
        );
        assert_eq!(
            AckStatus::from_bytes(0x11, 0),
            AckStatus::FlowControlSendNext
        );
        assert_eq!(AckStatus::Ack.to_bytes(), (0x00, 0x00));
        assert_eq!(AckStatus::TimeoutWait(3).to_bytes(), (0x10, 3));
        assert_eq!(AckStatus::FlowControlSendNext.to_bytes(), (0x11, 0x00));
        for code in 0u8..=0x7F {
            let s = AckStatus::from_bytes(code, 0x05);
            if !matches!(code, 0x00 | 0x10 | 0x11) {
                assert_eq!(s, AckStatus::Unknown { code, data: 0x05 });
                assert_eq!(s.to_bytes(), (code, 0x05));
            }
            let range = match code {
                0x00..=0x0F => StatusRange::Success,
                0x10..=0x1F => StatusRange::Notification,
                _ => StatusRange::Other,
            };
            assert_eq!(s.range(), range, "{code:#04x}");
        }
        // The written bytes of ACK 0x00 and Timeout Wait.
        let mut buf = [0u8; 64];
        for (status, bytes) in [
            (AckStatus::Ack, [0x00, 0x00]),
            (AckStatus::TimeoutWait(0x0A), [0x10, 0x0A]),
        ] {
            let a = Ack {
                original_sub_id2: 0x36,
                status,
                details: Details::Reserved,
                text: &[],
            };
            let n = a
                .write(
                    "",
                    DeviceId::FunctionBlock,
                    SRC,
                    DST,
                    None,
                    Framing::Bare,
                    &mut buf,
                )
                .unwrap();
            assert_eq!(
                &buf[13..n],
                &[0x36, bytes[0], bytes[1], 0, 0, 0, 0, 0, 0, 0]
            );
            let (h, body) = parse(&buf[..n]);
            assert_eq!(Ack::read(&h, body).unwrap().status, status);
        }
    }

    #[test]
    fn ack_nak_size_limit() {
        // REQ-CI-ACK-005, REQ-CI-NAK-005; DEC-011 D-11.
        let long = "x".repeat(300);
        let mut buf = [0u8; 600];
        let a = Ack {
            original_sub_id2: 0x22,
            status: AckStatus::Ack,
            details: Details::Reserved,
            text: &[],
        };
        let n = a
            .write(
                &long,
                DeviceId::FunctionBlock,
                SRC,
                DST,
                None,
                Framing::SysEx,
                &mut buf,
            )
            .unwrap();
        assert_eq!(n, 128);
        let k = Nak {
            original_sub_id2: 0x22,
            status: NakStatus::Nak,
            details: Details::Reserved,
            text: &[],
        };
        let n = k
            .write(
                &long,
                DeviceId::FunctionBlock,
                SRC,
                DST,
                Some(200),
                Framing::Bare,
                &mut buf,
            )
            .unwrap();
        assert_eq!(n, 200);
        let (h, body) = parse(&buf[..n]);
        assert_eq!(Nak::read(&h, body).unwrap().text.len(), 200 - 23);
        // An ACK with a reported size; a text that fits is not shortened.
        let n = a
            .write(
                &long,
                DeviceId::FunctionBlock,
                SRC,
                DST,
                Some(200),
                Framing::SysEx,
                &mut buf,
            )
            .unwrap();
        assert_eq!(n, 200);
        let (h, body) = parse(&buf[..n]);
        assert_eq!(Ack::read(&h, body).unwrap().text.len(), 200 - 25);
        let n = a
            .write(
                "short",
                DeviceId::FunctionBlock,
                SRC,
                DST,
                Some(200),
                Framing::Bare,
                &mut buf,
            )
            .unwrap();
        assert_eq!(n, 23 + 5);
        // A non-ASCII character is not split.
        let n = k
            .write(
                &"é".repeat(40),
                DeviceId::FunctionBlock,
                SRC,
                DST,
                None,
                Framing::Bare,
                &mut buf,
            )
            .unwrap();
        // 128 - 23 = 105 bytes of text: 17 escapes of 6 bytes, not 105 bytes of a split escape.
        assert_eq!(n, 23 + 17 * 6);
        let (h, body) = parse(&buf[..n]);
        assert_eq!(
            Nak::read(&h, body).unwrap().text().collect::<String>(),
            "é".repeat(17)
        );
    }

    #[test]
    fn nak_codes_and_version1() {
        // Table 16; REQ-CI-NAK-001, REQ-CI-NAK-003.
        assert_eq!(
            NakStatus::from_bytes(0x20, 1),
            NakStatus::Terminate { by_initiator: true }
        );
        assert_eq!(NakStatus::from_bytes(0x43, 9), NakStatus::Busy(9));
        assert_eq!(
            NakStatus::from_bytes(0x60, 0).range(),
            StatusRange::Reserved
        );
        assert_eq!(
            NakStatus::FlowControlResend.range(),
            StatusRange::FailNoRetry
        );
        assert_eq!(
            NakStatus::ChunksOutOfSequence.range(),
            StatusRange::Notification
        );
        assert_eq!(NakStatus::Timeout.range(), StatusRange::FailRetry);
        for code in 0u8..=0x7F {
            let s = NakStatus::from_bytes(code, 0);
            assert_eq!(s.to_bytes(), (code, 0));
        }
        // Each named code of Table 16 with its Status Data.
        {
            use NakStatus::*;
            for (code, data, named) in [
                (0x00, 0, Nak),
                (0x01, 0, NotSupported),
                (0x02, 0, VersionNotSupported),
                (0x03, 0, NotInUse),
                (0x04, 0, ProfileNotSupported),
                (0x12, 0, FlowControlResend),
                (
                    0x20,
                    0,
                    Terminate {
                        by_initiator: false,
                    },
                ),
                (0x20, 1, Terminate { by_initiator: true }),
                (0x21, 0, ChunksOutOfSequence),
                (0x40, 0, Error),
                (0x41, 0, Malformed),
                (0x42, 0, Timeout),
                (0x43, 0x7F, Busy(0x7F)),
            ] {
                assert_eq!(NakStatus::from_bytes(code, data), named, "{code:#04x}");
                assert_eq!(named.to_bytes(), (code, data));
            }
            // Other values are unknown codes with their range, reserved values included.
            assert_eq!(
                NakStatus::from_bytes(0x20, 2),
                Unknown {
                    code: 0x20,
                    data: 2
                }
            );
            for (code, range) in [
                (0x05, StatusRange::FailNoRetry),
                (0x30, StatusRange::Notification),
                (0x50, StatusRange::FailRetry),
                (0x7F, StatusRange::Reserved),
            ] {
                let s = NakStatus::from_bytes(code, 9);
                assert_eq!((s, s.range()), (Unknown { code, data: 9 }, range));
            }
        }
        // REQ-CI-MSG-012, -014: the version selects the fields; version 1 has no NAK data.
        let v1 = [0x7E, 0x05, 0x0D, 0x7F, 0x01, 1, 0, 0, 0, 2, 0, 0, 0];
        let (h, body) = parse(&v1);
        assert_eq!(h.version.minor(), 1);
        assert_eq!(Nak::read(&h, body).unwrap().status, NakStatus::Nak);
        let mut v2 = v1;
        v2[4] = 0x02;
        let (h, body) = parse(&v2);
        assert_eq!(Nak::read(&h, body), Err(ParseError::TooShort));
        // Version 0x00 is version 1.
        let mut v0 = v1;
        v0[4] = 0x00;
        let (h, body) = parse(&v0);
        assert_eq!(Nak::read(&h, body).unwrap().status, NakStatus::Nak);
    }

    #[test]
    fn higher_version_ignores_appended_fields() {
        // REQ-CI-MSG-013, REQ-CI-MSG-016: a version 3 message is read with the fields this
        // crate knows; appended bytes are ignored.
        let d = Discovery {
            identity: identity(),
            categories: Categories::default(),
            max_sysex_size: 256,
            output_path_id: Some(1),
        };
        let mut buf = [0u8; 64];
        let n = d.write(SRC, Framing::Bare, &mut buf).unwrap();
        buf[4] = 0x03;
        buf[n] = 0x55;
        buf[n + 1] = 0x66;
        let (h, body) = parse(&buf[..n + 2]);
        assert_eq!(Discovery::read(&h, body), Ok(d));
    }

    #[test]
    fn prop_nak_without_text_is_write_with_empty_text() {
        // OPT-049 step 4: the NAK of the Device gives the bytes of Nak::write with
        // "" for every field value, Receivable Maximum SysEx Message Size and buffer size, and
        // the same error when the buffer is large enough.
        proptest!(|(orig in any::<u8>(), code in any::<u8>(), data in any::<u8>(), det in any::<[u8; 5]>(),
                    dev in 0u8..0x80, max in proptest::option::of(0u32..64), sysex in any::<bool>(), len in 0usize..40)| {
            let k = Nak {
                original_sub_id2: orig,
                status: NakStatus::from_bytes(code, data),
                details: Details::Profile(det),
                text: &[],
            };
            let Ok(device_id) = DeviceId::from_byte(dev) else { return Ok(()) };
            let framing = if sysex { Framing::SysEx } else { Framing::Bare };
            let (mut a, mut b) = ([0u8; 40], [0u8; 40]);
            let x = k.write("", device_id, SRC, DST, max, framing, &mut a[..len]);
            let y = k.write_without_text(device_id, SRC, DST, max, framing, &mut b[..len]);
            // A value out of range in a buffer that is also too small can give either error.
            prop_assert_eq!(x.is_ok(), y.is_ok());
            if len >= 29 {
                prop_assert_eq!(x, y);
            }
            if let Ok(n) = x {
                prop_assert_eq!(&a[..n], &b[..n]);
            }
        });
    }

    #[test]
    fn nak_round_trip_with_details() {
        // Table 15; REQ-CI-NAK-001, REQ-CI-NAK-002, REQ-CI-NAK-004.
        let mut buf = [0u8; 128];
        let k = Nak {
            original_sub_id2: 0x22,
            status: NakStatus::ProfileNotSupported,
            details: Details::Profile([0x7E, 0x21, 0, 1, 0x7F]),
            text: &[],
        };
        let n = k
            .write(
                "no",
                DeviceId::Channel(4),
                SRC,
                DST,
                Some(512),
                Framing::Bare,
                &mut buf,
            )
            .unwrap();
        assert_eq!(&buf[13..21], &[0x22, 0x04, 0x00, 0x7E, 0x21, 0, 1, 0x7F]);
        let (h, body) = parse(&buf[..n]);
        let back = Nak::read(&h, body).unwrap();
        assert_eq!(
            (back.original_sub_id2, back.status, back.details),
            (0x22, k.status, k.details)
        );
        assert_eq!(back.text().collect::<String>(), "no");
    }

    #[test]
    fn prop_readers_never_panic() {
        proptest!(|(bytes in proptest::collection::vec(0u8..0x80, 0..40), v in 0u8..4)| {
            let h = Header {
                device_id: DeviceId::FunctionBlock,
                sub_id2: 0x70,
                version: Version::from_byte(v).unwrap(),
                source: SRC,
                destination: DST,
            };
            let _ = Discovery::read(&h, &bytes);
            let _ = DiscoveryReply::read(&h, &bytes);
            let _ = EndpointInfoInquiry::read(&h, &bytes);
            let _ = EndpointInfoReply::read(&h, &bytes);
            let _ = InvalidateMuid::read(&h, &bytes);
            let _ = Ack::read(&h, &bytes);
            let _ = Nak::read(&h, &bytes);
        });
    }
}
