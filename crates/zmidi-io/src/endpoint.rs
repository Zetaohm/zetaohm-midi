//! UMP Endpoint responder and stream state (M2-104-UM 3, 6, 7.1).
//!
//! - Sans-IO: [`Endpoint::receive`] takes the words of one received packet and gives the reply
//!   packets through a lazy [`Replies`] iterator. No buffer, no alloc (DEC-002), no async
//!   (DEC-003).
//! - The endpoint implements UMP 1.1. An Endpoint Discovery with a lower version gets no reply,
//!   because UMP 1.0 does not define it; a higher version is read with the 1.1 fields only (the
//!   zmidi-ump reader clears reserved bits) (7.1.1).
//! - Discovery replies: one message for each Filter bit that is set (7.1.1, 7.1.7).
//! - Stream configuration is held from this endpoint's side (send JR, receive JR). It changes
//!   only through Stream Configuration Request and Notification (7.1.6).
//! - Function Blocks: at most 32, numbered 0 to N-1; the count never changes; static blocks
//!   never change and are always active (6.1.1, 7.1.2, 7.1.8).

use zmidi_ump::{
    DeviceIdentityNotification, EndpointInfoNotification, FunctionBlockDirection,
    FunctionBlockInfo, FunctionBlockInfoNotification, FunctionBlockMidi1, Group,
    StreamConfigurationNotification, StreamConfigurationRequest, StreamProtocol, StreamTextPackets,
    UiHint, UmpError, UmpStreamMessage, UmpWords,
};

/// Largest number of Function Blocks of one endpoint (7.1.2).
const MAX_FUNCTION_BLOCKS: usize = 32;

/// UMP version that this endpoint implements: 1.1 (Table 5).
const UMP_VERSION: (u8, u8) = (1, 1);

/// Message Type 0x2: MIDI 1.0 Channel Voice (Table 4).
const MT_MIDI1_CHANNEL_VOICE: u32 = 0x2;

/// Message Type 0x4: MIDI 2.0 Channel Voice (Table 4).
const MT_MIDI2_CHANNEL_VOICE: u32 = 0x4;

/// One Function Block of the endpoint (6.1, 7.1.8, 7.1.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionBlock<'a> {
    /// The field values of its Function Block Info Notification.
    pub info: FunctionBlockInfo,
    /// The Function Block Name; the caller gives NFC text (DEC-UMP-005).
    pub name: &'a str,
}

impl<'a> FunctionBlock<'a> {
    /// An active Function Block for one MIDI 1.0 port, or one MIDI 1.0 DIN In and Out pair
    /// with `direction` Bidirectional (6.2.1.1, 6.2.1.2). It spans one Group, and its User
    /// Interface Hint follows `direction`. MIDI-CI Message Version/Format is 0x01, as UMP 1.1
    /// asks (7.1.8).
    pub const fn midi1_port(
        number: u8,
        group: Group,
        direction: FunctionBlockDirection,
        midi1: FunctionBlockMidi1,
        name: &'a str,
    ) -> Self {
        let ui_hint = match direction {
            FunctionBlockDirection::Input => UiHint::Receiver,
            FunctionBlockDirection::Output => UiHint::Sender,
            FunctionBlockDirection::Bidirectional => UiHint::SenderReceiver,
        };
        Self {
            info: FunctionBlockInfo {
                active: true,
                number,
                direction,
                midi1,
                ui_hint,
                first_group: group,
                groups_spanned: 1,
                zmidi_ci_version: 0x01,
                max_sysex8_streams: 0,
            },
            name,
        }
    }

    /// True when `group` is one of the Groups of the block.
    fn spans(&self, group: u8) -> bool {
        let first = self.info.first_group.index();
        group >= first && group < first.saturating_add(self.info.groups_spanned)
    }
}

/// What the endpoint declares about itself (7.1.2 to 7.1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointDescription<'a> {
    /// Endpoint Name, at most 98 bytes; the caller gives NFC text (DEC-UMP-005).
    pub name: &'a str,
    /// Product Instance Id, at most 42 ASCII bytes.
    pub product_instance_id: &'a str,
    /// The Device Identity Notification to send.
    pub device_identity: DeviceIdentityNotification,
    /// `M1`: MIDI 1.0 Protocol capability.
    pub midi1: bool,
    /// `M2`: MIDI 2.0 Protocol capability.
    pub midi2: bool,
    /// `RXJR`: can receive JR Timestamps.
    pub rxjr: bool,
    /// `TXJR`: can send JR Timestamps.
    pub txjr: bool,
    /// `S`: the Function Blocks are static.
    pub static_function_blocks: bool,
}

/// A Stream Configuration from this endpoint's side (7.1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamConfiguration {
    /// The Protocol of the stream.
    pub protocol: StreamProtocol,
    /// This endpoint sends JR Timestamps.
    pub send_jr: bool,
    /// This endpoint receives JR Timestamps.
    pub receive_jr: bool,
}

/// Errors of the [`Endpoint`] API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    /// More than 32 Function Blocks (7.1.2).
    #[error("an endpoint has at most 32 Function Blocks")]
    TooManyFunctionBlocks,
    /// The Function Block Numbers are not 0 to N-1 in order (7.1.2).
    #[error("Function Block Numbers must be 0 to N-1 in order")]
    FunctionBlockNumbering,
    /// zmidi-ump rejects the block, its MIDI-CI Message Version/Format is 0 (7.1.8), or a
    /// block of a static endpoint is not active (7.1.8).
    #[error("the Function Block is not valid")]
    InvalidFunctionBlock,
    /// zmidi-ump rejects a name or the Product Instance Id (7.1.4, 7.1.5, 7.1.9).
    #[error("a name or the Product Instance Id is not valid")]
    InvalidText,
    /// A change was requested on static Function Blocks (6.1.1, 7.1.2).
    #[error("the Function Blocks are static")]
    StaticFunctionBlocks,
    /// The description does not declare the requested Protocol or JR use (7.1.6).
    #[error("the endpoint does not declare this Protocol or JR use")]
    ProtocolNotSupported,
    /// An outgoing message of the Protocol that is not in effect (3.2, 3.3.1).
    #[error("the message does not belong to the Protocol in effect")]
    WrongProtocol,
    /// An error of zmidi-ump.
    #[error(transparent)]
    Ump(#[from] UmpError),
}

/// A UMP Endpoint with `N` Function Blocks (M2-104-UM 3, 6, 7.1).
#[derive(Debug, Clone)]
pub struct Endpoint<'a, const N: usize> {
    description: EndpointDescription<'a>,
    blocks: [FunctionBlock<'a>; N],
    config: StreamConfiguration,
    /// The configuration of a request that waits for its Notification.
    pending: Option<StreamConfiguration>,
}

impl<'a, const N: usize> Endpoint<'a, N> {
    /// An endpoint with `description` and the Function Blocks `blocks`, numbered 0 to N-1.
    ///
    /// Gives an error for more than 32 blocks, wrong numbers, a block that zmidi-ump rejects or
    /// that has MIDI-CI Message Version/Format 0, an inactive block of a static endpoint, or a
    /// text that zmidi-ump rejects. The first configuration is the MIDI 2.0 Protocol when the
    /// description declares it, else the MIDI 1.0 Protocol, with no JR Timestamps.
    pub fn try_new(
        description: EndpointDescription<'a>,
        blocks: [FunctionBlock<'a>; N],
    ) -> Result<Self, EndpointError> {
        if N > MAX_FUNCTION_BLOCKS {
            return Err(EndpointError::TooManyFunctionBlocks);
        }
        if !description.midi1 && !description.midi2 {
            return Err(EndpointError::ProtocolNotSupported);
        }
        StreamTextPackets::endpoint_name(description.name)
            .and(StreamTextPackets::product_instance_id(
                description.product_instance_id,
            ))
            .map_err(|_| EndpointError::InvalidText)?;
        for (i, block) in blocks.iter().enumerate() {
            if block.info.number as usize != i {
                return Err(EndpointError::FunctionBlockNumbering);
            }
            check_block(block, description.static_function_blocks)?;
        }
        let protocol = if description.midi2 {
            StreamProtocol::Midi2
        } else {
            StreamProtocol::Midi1
        };
        Ok(Self {
            description,
            blocks,
            config: StreamConfiguration {
                protocol,
                send_jr: false,
                receive_jr: false,
            },
            pending: None,
        })
    }

    /// Takes the words of one received packet, updates the state and gives the replies.
    ///
    /// - Endpoint Discovery (version 1.1 or higher): one reply for each Filter bit that is set,
    ///   in the order e, d, n, i, s (7.1.1).
    /// - Function Block Discovery: the Info (bit i) and Name (bit n) replies for one block, or
    ///   for every block for number 0xFF; none for a number of N or higher (7.1.7 to 7.1.9).
    /// - Stream Configuration Request: applies each part that the description declares and
    ///   replies with the configuration in effect (7.1.6.1, 7.1.6.3).
    /// - Stream Configuration Notification: adopts a configuration that the description
    ///   declares, and ends the wait of a request (7.1.6).
    /// - Any other packet: no reply.
    ///
    /// Never panics.
    pub fn receive(&mut self, words: &[u32]) -> Replies<'_, 'a, N> {
        let mut replies = Replies::new();
        match UmpStreamMessage::parse(words) {
            Some(UmpStreamMessage::EndpointDiscovery(d)) => {
                if (d.ump_version_major(), d.ump_version_minor()) >= UMP_VERSION {
                    replies.steps = (d.endpoint_info() as u8 * ENDPOINT_INFO)
                        | (d.device_identity() as u8 * DEVICE_IDENTITY)
                        | (d.endpoint_name() as u8 * ENDPOINT_NAME)
                        | (d.product_instance_id() as u8 * PRODUCT_INSTANCE_ID)
                        | (d.stream_configuration() as u8 * STREAM_CONFIGURATION);
                }
            }
            Some(UmpStreamMessage::FunctionBlockDiscovery(f)) => {
                let number = f.function_block_number() as usize;
                let (first, end) = if number == 0xFF {
                    (0, N)
                } else if number < N {
                    (number, number + 1)
                } else {
                    (0, 0)
                };
                replies.blocks(first, end, f.info(), f.name());
            }
            Some(UmpStreamMessage::StreamConfigurationRequest(r)) => {
                // The request names the JR use from the side of its receiver, this endpoint.
                if self.supports_protocol(r.protocol()) {
                    self.config.protocol = r.protocol();
                }
                if !r.rxjr() || self.description.rxjr {
                    self.config.receive_jr = r.rxjr();
                }
                if !r.txjr() || self.description.txjr {
                    self.config.send_jr = r.txjr();
                }
                replies.steps = STREAM_CONFIGURATION;
            }
            Some(UmpStreamMessage::StreamConfigurationNotification(n)) => {
                // The notification names the JR use from the side of its sender, the peer.
                let offered = StreamConfiguration {
                    protocol: n.protocol(),
                    send_jr: n.rxjr(),
                    receive_jr: n.txjr(),
                };
                if self.supports(offered) {
                    self.config = offered;
                }
                self.pending = None;
            }
            _ => {}
        }
        replies.endpoint = Some(self);
        replies
    }

    /// The Stream Configuration in effect.
    pub const fn stream_configuration(&self) -> StreamConfiguration {
        self.config
    }

    /// The Stream Configuration Request that asks the peer for `config`, which is the only way
    /// to select a Protocol and JR use (7.1.6.1). The endpoint waits for the answering
    /// Notification. Gives `ProtocolNotSupported` when the description does not declare
    /// `config`.
    pub fn request_configuration(
        &mut self,
        config: StreamConfiguration,
    ) -> Result<StreamConfigurationRequest, EndpointError> {
        if !self.supports(config) {
            return Err(EndpointError::ProtocolNotSupported);
        }
        self.pending = Some(config);
        // RXJR asks for JR Timestamps to the peer, TXJR for JR Timestamps from the peer.
        Ok(StreamConfigurationRequest::new(
            config.protocol,
            config.send_jr,
            config.receive_jr,
        ))
    }

    /// True between [`Endpoint::request_configuration`] and the Notification that answers it.
    pub const fn is_waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// True when a JR Timestamp must prefix every outgoing message (7.1.6.2). False while the
    /// endpoint waits for the answer to a request.
    pub const fn sends_jr_timestamps(&self) -> bool {
        self.pending.is_none() && self.config.send_jr
    }

    /// True when the endpoint expects JR Timestamps on received messages. False while the
    /// endpoint waits for the answer to a request (7.1.6.2).
    pub const fn expects_jr_timestamps(&self) -> bool {
        self.pending.is_none() && self.config.receive_jr
    }

    /// Checks one outgoing message (its first word decides) against the Protocol in effect.
    /// Gives `WrongProtocol` for a MIDI 2.0 Channel Voice message (Message Type 0x4) under the
    /// MIDI 1.0 Protocol (3.2), and for a MIDI 1.0 Channel Voice message (0x2) under the
    /// MIDI 2.0 Protocol (3.3.1), except on a Group of a Function Block that declares a MIDI 1.0
    /// connection (7.1.6.2). Gives a `Truncated` error for empty input.
    pub fn check_outgoing(&self, words: &[u32]) -> Result<(), EndpointError> {
        let Some(&w0) = words.first() else {
            return Err(UmpError::Truncated { needed: 1, got: 0 }.into());
        };
        let group = ((w0 >> 24) & 0xF) as u8;
        let wrong = match (self.config.protocol, w0 >> 28) {
            (StreamProtocol::Midi1, MT_MIDI2_CHANNEL_VOICE) => true,
            (StreamProtocol::Midi2, MT_MIDI1_CHANNEL_VOICE) => !self
                .blocks
                .iter()
                .any(|b| !matches!(b.info.midi1, FunctionBlockMidi1::NotMidi1) && b.spans(group)),
            _ => false,
        };
        if wrong {
            Err(EndpointError::WrongProtocol)
        } else {
            Ok(())
        }
    }

    /// The Function Blocks, in number order.
    pub fn function_blocks(&self) -> &[FunctionBlock<'a>] {
        &self.blocks
    }

    /// Replaces the Function Block with the number of `block`, with the checks of
    /// [`Endpoint::try_new`], and gives its Function Block Info Notification, and its Name
    /// Notification when the name changed (7.1.8, 7.1.9). Gives `StaticFunctionBlocks` on an
    /// endpoint with static blocks (6.1.1), and `FunctionBlockNumbering` for a number of N or
    /// higher: the count never changes (7.1.2).
    pub fn set_function_block(
        &mut self,
        block: FunctionBlock<'a>,
    ) -> Result<Replies<'_, 'a, N>, EndpointError> {
        if self.description.static_function_blocks {
            return Err(EndpointError::StaticFunctionBlocks);
        }
        let number = block.info.number as usize;
        if number >= N {
            return Err(EndpointError::FunctionBlockNumbering);
        }
        check_block(&block, false)?;
        let name_changed = self.blocks[number].name != block.name;
        self.blocks[number] = block;
        let mut replies = Replies::new();
        replies.blocks(number, number + 1, true, name_changed);
        replies.endpoint = Some(self);
        Ok(replies)
    }

    /// Gives the Function Block Info Notification of block `number`, which did not change, also
    /// on an endpoint with static blocks. The application calls it after the MIDI-CI MUID of the
    /// block changed (M2-101-UM 3.3.1, REQ-CI-IO-MUID-001, DEC-009). Gives
    /// `FunctionBlockNumbering` for a number of N or higher.
    pub fn function_block_notification(
        &self,
        number: u8,
    ) -> Result<Replies<'_, 'a, N>, EndpointError> {
        let number = number as usize;
        if number >= N {
            return Err(EndpointError::FunctionBlockNumbering);
        }
        let mut replies = Replies::new();
        replies.blocks(number, number + 1, true, false);
        replies.endpoint = Some(self);
        Ok(replies)
    }

    /// Sets the Protocol and JR capabilities and gives the Endpoint Info Notification (7.1.2).
    /// When the configuration in effect is no longer declared, the endpoint falls back to the
    /// MIDI 2.0 Protocol if declared, else MIDI 1.0, drops the JR use it cannot keep, and also
    /// gives a Stream Configuration Notification (7.1.6.3). Gives `ProtocolNotSupported` when
    /// neither Protocol is declared.
    pub fn set_capabilities(
        &mut self,
        midi1: bool,
        midi2: bool,
        rxjr: bool,
        txjr: bool,
    ) -> Result<Replies<'_, 'a, N>, EndpointError> {
        if !midi1 && !midi2 {
            return Err(EndpointError::ProtocolNotSupported);
        }
        let d = &mut self.description;
        (d.midi1, d.midi2, d.rxjr, d.txjr) = (midi1, midi2, rxjr, txjr);
        let mut replies = Replies::new();
        replies.steps = ENDPOINT_INFO;
        if !self.supports(self.config) {
            if !self.supports_protocol(self.config.protocol) {
                self.config.protocol = if midi2 {
                    StreamProtocol::Midi2
                } else {
                    StreamProtocol::Midi1
                };
            }
            self.config.send_jr &= txjr;
            self.config.receive_jr &= rxjr;
            replies.steps |= STREAM_CONFIGURATION;
        }
        replies.endpoint = Some(self);
        Ok(replies)
    }

    fn supports_protocol(&self, protocol: StreamProtocol) -> bool {
        match protocol {
            StreamProtocol::Midi1 => self.description.midi1,
            StreamProtocol::Midi2 => self.description.midi2,
        }
    }

    fn supports(&self, config: StreamConfiguration) -> bool {
        self.supports_protocol(config.protocol)
            && (!config.send_jr || self.description.txjr)
            && (!config.receive_jr || self.description.rxjr)
    }
}

/// Checks one block: zmidi-ump accepts it, MIDI-CI Message Version/Format is 0x01 or higher
/// (7.1.8), and a block of a static endpoint is active (7.1.8).
fn check_block(block: &FunctionBlock<'_>, static_blocks: bool) -> Result<(), EndpointError> {
    let valid = FunctionBlockInfoNotification::try_new(block.info).is_ok()
        && block.info.zmidi_ci_version >= 0x01
        && (block.info.active || !static_blocks);
    if !valid {
        return Err(EndpointError::InvalidFunctionBlock);
    }
    StreamTextPackets::function_block_name(block.info.number, block.name)
        .map(|_| ())
        .map_err(|_| EndpointError::InvalidText)
}

const ENDPOINT_INFO: u8 = 1 << 0;
const DEVICE_IDENTITY: u8 = 1 << 1;
const ENDPOINT_NAME: u8 = 1 << 2;
const PRODUCT_INSTANCE_ID: u8 = 1 << 3;
const STREAM_CONFIGURATION: u8 = 1 << 4;

/// The four words of `message`.
fn words<T: UmpWords>(message: &T) -> [u32; 4] {
    let mut out = [0; 4];
    // Every message of this module is four words, so the buffer is large enough.
    let _ = message.to_words(&mut out);
    out
}

/// Lazy iterator over the reply packets of one [`Endpoint`] call, each as four words, in send
/// order. Holds no buffer and needs no alloc.
#[derive(Debug, Clone)]
pub struct Replies<'e, 'a, const N: usize> {
    endpoint: Option<&'e Endpoint<'a, N>>,
    /// Endpoint replies still to send, as `ENDPOINT_INFO` and the other bits.
    steps: u8,
    /// Packets of the current text reply.
    text: Option<StreamTextPackets<'a>>,
    /// Next Function Block and the end of the range.
    block: usize,
    block_end: usize,
    block_info: bool,
    block_name: bool,
    /// The Info reply of `block` is sent.
    info_sent: bool,
}

impl<'a, const N: usize> Replies<'_, 'a, N> {
    const fn new() -> Self {
        Self {
            endpoint: None,
            steps: 0,
            text: None,
            block: 0,
            block_end: 0,
            block_info: false,
            block_name: false,
            info_sent: false,
        }
    }

    fn blocks(&mut self, first: usize, end: usize, info: bool, name: bool) {
        (self.block, self.block_end) = (first, end);
        (self.block_info, self.block_name) = (info, name);
    }

    /// Clears `step` in `steps` and tells whether it was set.
    fn take(&mut self, step: u8) -> bool {
        let set = self.steps & step != 0;
        self.steps &= !step;
        set
    }
}

impl<const N: usize> Iterator for Replies<'_, '_, N> {
    type Item = [u32; 4];

    fn next(&mut self) -> Option<[u32; 4]> {
        let endpoint = self.endpoint?;
        let d = &endpoint.description;
        loop {
            if let Some(text) = self.text.as_mut() {
                if let Some(packet) = text.next() {
                    return Some(words(&packet));
                }
                self.text = None;
            }
            if self.take(ENDPOINT_INFO) {
                let info = EndpointInfoNotification::try_new(
                    d.static_function_blocks,
                    N as u8,
                    d.midi2,
                    d.midi1,
                    d.rxjr,
                    d.txjr,
                );
                if let Ok(info) = info {
                    return Some(words(&info));
                }
                continue;
            }
            if self.take(DEVICE_IDENTITY) {
                return Some(words(&d.device_identity));
            }
            if self.take(ENDPOINT_NAME) {
                self.text = StreamTextPackets::endpoint_name(d.name).ok();
                continue;
            }
            if self.take(PRODUCT_INSTANCE_ID) {
                self.text = StreamTextPackets::product_instance_id(d.product_instance_id).ok();
                continue;
            }
            if self.take(STREAM_CONFIGURATION) {
                // The notification names the JR use from the side of its sender, this endpoint.
                let c = endpoint.config;
                let notification =
                    StreamConfigurationNotification::new(c.protocol, c.receive_jr, c.send_jr);
                return Some(words(&notification));
            }
            let block = endpoint
                .blocks
                .get(self.block)
                .filter(|_| self.block < self.block_end)?;
            if self.block_info && !self.info_sent {
                self.info_sent = true;
                if let Ok(info) = FunctionBlockInfoNotification::try_new(block.info) {
                    return Some(words(&info));
                }
            }
            if self.block_name {
                self.text =
                    StreamTextPackets::function_block_name(block.info.number, block.name).ok();
            }
            self.block += 1;
            self.info_sent = false;
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use std::vec::Vec;

    use super::*;
    use proptest::prelude::*;

    fn g(n: u8) -> Group {
        Group::new(n).unwrap()
    }

    fn identity() -> DeviceIdentityNotification {
        DeviceIdentityNotification::try_new([0x00, 0x21, 0x09], 0x0123, 0x0204, [1, 2, 3, 4])
            .unwrap()
    }

    fn description(static_blocks: bool) -> EndpointDescription<'static> {
        EndpointDescription {
            name: "Synth",
            product_instance_id: "SN01",
            device_identity: identity(),
            midi1: true,
            midi2: true,
            rxjr: true,
            txjr: false,
            static_function_blocks: static_blocks,
        }
    }

    fn blocks() -> [FunctionBlock<'static>; 2] {
        [
            FunctionBlock::midi1_port(
                0,
                g(0),
                FunctionBlockDirection::Bidirectional,
                FunctionBlockMidi1::Restricted,
                "DIN",
            ),
            FunctionBlock {
                info: FunctionBlockInfo {
                    active: true,
                    number: 1,
                    direction: FunctionBlockDirection::Bidirectional,
                    midi1: FunctionBlockMidi1::NotMidi1,
                    ui_hint: UiHint::SenderReceiver,
                    first_group: g(1),
                    groups_spanned: 2,
                    zmidi_ci_version: 0x02,
                    max_sysex8_streams: 1,
                },
                name: "Main",
            },
        ]
    }

    fn endpoint(static_blocks: bool) -> Endpoint<'static, 2> {
        Endpoint::try_new(description(static_blocks), blocks()).unwrap()
    }

    fn collect<const N: usize>(replies: Replies<'_, '_, N>) -> Vec<[u32; 4]> {
        replies.collect()
    }

    /// Endpoint Discovery (Figures 11, 12) with Filter bits `filter`.
    fn discovery(major: u8, minor: u8, filter: u32) -> [u32; 4] {
        [
            0xF000_0000 | (major as u32) << 8 | minor as u32,
            filter,
            0,
            0,
        ]
    }

    const INFO: [u32; 4] = [0xF001_0101, 0x8200_0302, 0, 0];
    const IDENTITY: [u32; 4] = [0xF002_0000, 0x0000_2109, 0x2302_0404, 0x0102_0304];
    const NAME: [u32; 4] = [0xF003_5379, 0x6E74_6800, 0, 0];
    const PRODUCT_ID: [u32; 4] = [0xF004_534E, 0x3031_0000, 0, 0];
    const FB0_INFO: [u32; 4] = [0xF011_803B, 0x0001_0100, 0, 0];
    const FB0_NAME: [u32; 4] = [0xF012_0044, 0x494E_0000, 0, 0];
    const FB1_INFO: [u32; 4] = [0xF011_8133, 0x0102_0201, 0, 0];
    const FB1_NAME: [u32; 4] = [0xF012_014D, 0x6169_6E00, 0, 0];

    /// 7.1.1, 7.1.2: one reply for each Filter bit, in the order e, d, n, i, s (Figures 13 to
    /// 17, 19).
    #[test]
    fn endpoint_discovery_replies_per_filter_bit() {
        let mut ep = endpoint(true);
        // S=1, 2 Function Blocks, M2, M1, RXJR (Figure 13).
        assert_eq!(
            collect(ep.receive(&discovery(1, 1, 0x1F))),
            [INFO, IDENTITY, NAME, PRODUCT_ID, [0xF006_0200, 0, 0, 0]]
        );
        assert_eq!(collect(ep.receive(&discovery(1, 1, 0x01))), [INFO]);
        assert_eq!(collect(ep.receive(&discovery(1, 1, 0x04))), [NAME]);
        assert_eq!(
            collect(ep.receive(&discovery(1, 1, 0x10))),
            [[0xF006_0200, 0, 0, 0]]
        );
        assert!(collect(ep.receive(&discovery(1, 1, 0))).is_empty());
        // A long name gives several packets (7.1.4).
        let long = "A name of more than fourteen bytes";
        let mut d = description(true);
        d.name = long;
        let mut ep = Endpoint::try_new(d, blocks()).unwrap();
        let expected: Vec<_> = StreamTextPackets::endpoint_name(long)
            .unwrap()
            .map(|p| words(&p))
            .collect();
        assert_eq!(expected.len(), 3);
        assert_eq!(collect(ep.receive(&discovery(1, 1, 0x04))), expected);
    }

    /// 7.1.1: a version below 1.1 gets no reply; a higher version is read with the 1.1 fields
    /// and its reserved bits are ignored.
    #[test]
    fn endpoint_discovery_version_rules() {
        let mut ep = endpoint(true);
        assert!(collect(ep.receive(&discovery(1, 0, 0x1F))).is_empty());
        assert!(collect(ep.receive(&discovery(0, 9, 0x1F))).is_empty());
        for (major, minor) in [(1, 2), (2, 0), (0xFF, 0xFF)] {
            assert_eq!(
                collect(ep.receive(&[
                    0xF000_0000 | (major as u32) << 8 | minor as u32,
                    0xFFFF_FFE1,
                    0xFFFF_FFFF,
                    0xFFFF_FFFF
                ])),
                [INFO]
            );
        }
    }

    /// 7.1.7 to 7.1.9: Info and Name replies for one block or every block (0xFF).
    #[test]
    fn function_block_discovery_replies() {
        let mut ep = endpoint(true);
        let fbd = |number: u32, filter: u32| [0xF010_0000 | number << 8 | filter, 0, 0, 0];
        assert_eq!(
            collect(ep.receive(&fbd(0xFF, 0x03))),
            [FB0_INFO, FB0_NAME, FB1_INFO, FB1_NAME]
        );
        assert_eq!(collect(ep.receive(&fbd(0xFF, 0x01))), [FB0_INFO, FB1_INFO]);
        assert_eq!(collect(ep.receive(&fbd(0xFF, 0x02))), [FB0_NAME, FB1_NAME]);
        assert_eq!(collect(ep.receive(&fbd(1, 0x03))), [FB1_INFO, FB1_NAME]);
        assert!(collect(ep.receive(&fbd(2, 0x03))).is_empty());
        assert!(collect(ep.receive(&fbd(0, 0))).is_empty());
        // An endpoint with no Function Blocks does not reply.
        let mut none = Endpoint::<0>::try_new(description(true), []).unwrap();
        assert!(collect(none.receive(&fbd(0xFF, 0x03))).is_empty());
        assert_eq!(
            collect(none.receive(&discovery(1, 1, 0x01))),
            [[0xF001_0101, 0x8000_0302, 0, 0]]
        );
        // Other packets give no reply.
        assert!(collect(ep.receive(&[0x2090_3C40])).is_empty());
        assert!(collect(ep.receive(&[])).is_empty());
    }

    /// 6.2.1.1, 6.2.1.2: a MIDI 1.0 port spans one Group.
    #[test]
    fn midi1_port_spans_one_group() {
        let b = FunctionBlock::midi1_port(
            3,
            g(7),
            FunctionBlockDirection::Input,
            FunctionBlockMidi1::Unrestricted,
            "In",
        );
        assert_eq!(b.info.groups_spanned, 1);
        assert_eq!(b.info.first_group, g(7));
        assert_eq!(b.info.ui_hint, UiHint::Receiver);
        assert!(b.info.active);
        assert!(b.spans(7) && !b.spans(8) && !b.spans(6));
        assert!(FunctionBlockInfoNotification::try_new(b.info).is_ok());
    }

    /// 7.1.2, 7.1.8: at most 32 blocks numbered 0 to N-1; MIDI-CI version 0x01 or higher;
    /// static blocks are active; texts that zmidi-ump accepts.
    #[test]
    fn endpoint_try_new_rejects_invalid_description() {
        let fb = |n: u8| {
            FunctionBlock::midi1_port(
                n,
                g(0),
                FunctionBlockDirection::Input,
                FunctionBlockMidi1::Unrestricted,
                "x",
            )
        };
        let many: [FunctionBlock; 33] = core::array::from_fn(|i| fb(i as u8 % 32));
        assert_eq!(
            Endpoint::try_new(description(true), many).map(|_| ()),
            Err(EndpointError::TooManyFunctionBlocks)
        );
        let max: [FunctionBlock; 32] = core::array::from_fn(|i| fb(i as u8));
        assert!(Endpoint::try_new(description(true), max).is_ok());
        assert_eq!(
            Endpoint::try_new(description(true), [fb(1)]).map(|_| ()),
            Err(EndpointError::FunctionBlockNumbering)
        );
        // Numbered 0 to N-1 in order: no duplicate, no other order.
        for bad in [[fb(0), fb(0)], [fb(1), fb(0)]] {
            assert_eq!(
                Endpoint::try_new(description(true), bad).map(|_| ()),
                Err(EndpointError::FunctionBlockNumbering)
            );
        }
        let mut b = blocks();
        b[1].info.zmidi_ci_version = 0;
        assert_eq!(
            Endpoint::try_new(description(true), b).map(|_| ()),
            Err(EndpointError::InvalidFunctionBlock)
        );
        let mut b = blocks();
        b[1].info.active = false;
        assert_eq!(
            Endpoint::try_new(description(true), b).map(|_| ()),
            Err(EndpointError::InvalidFunctionBlock)
        );
        assert!(Endpoint::try_new(description(false), b).is_ok());
        let mut b = blocks();
        b[0].info.groups_spanned = 2;
        assert_eq!(
            Endpoint::try_new(description(true), b).map(|_| ()),
            Err(EndpointError::InvalidFunctionBlock)
        );
        let mut b = blocks();
        b[0].name = "\u{FEFF}";
        assert_eq!(
            Endpoint::try_new(description(true), b).map(|_| ()),
            Err(EndpointError::InvalidText)
        );
        let mut d = description(true);
        d.product_instance_id = "S\u{E9}rie";
        assert_eq!(
            Endpoint::try_new(d, blocks()).map(|_| ()),
            Err(EndpointError::InvalidText)
        );
        let mut d = description(true);
        (d.midi1, d.midi2) = (false, false);
        assert_eq!(
            Endpoint::try_new(d, blocks()).map(|_| ()),
            Err(EndpointError::ProtocolNotSupported)
        );
        let mut d = description(true);
        d.midi2 = false;
        assert_eq!(
            Endpoint::try_new(d, blocks())
                .unwrap()
                .stream_configuration()
                .protocol,
            StreamProtocol::Midi1
        );
    }

    /// 7.1.6.1, 7.1.6.3: a Request is applied where the description allows it, and the reply
    /// gives the configuration in effect.
    #[test]
    fn stream_configuration_request_is_applied_and_confirmed() {
        let mut ep = endpoint(true);
        // MIDI 1.0 with JR to this endpoint (RXJR): declared, so applied.
        let request = words(&StreamConfigurationRequest::new(
            StreamProtocol::Midi1,
            true,
            false,
        ));
        assert_eq!(request, [0xF005_0102, 0, 0, 0]);
        assert_eq!(collect(ep.receive(&request)), [[0xF006_0102, 0, 0, 0]]);
        assert_eq!(
            ep.stream_configuration(),
            StreamConfiguration {
                protocol: StreamProtocol::Midi1,
                send_jr: false,
                receive_jr: true
            }
        );
        assert!(ep.expects_jr_timestamps() && !ep.sends_jr_timestamps());
        // JR from this endpoint (TXJR) is not declared: the reply keeps the current value.
        let request = words(&StreamConfigurationRequest::new(
            StreamProtocol::Midi2,
            false,
            true,
        ));
        assert_eq!(collect(ep.receive(&request)), [[0xF006_0200, 0, 0, 0]]);
        // A Protocol that the description does not declare keeps the current Protocol.
        let mut d = description(true);
        (d.midi2, d.txjr) = (false, true);
        let mut ep1 = Endpoint::try_new(d, blocks()).unwrap();
        assert_eq!(collect(ep1.receive(&request)), [[0xF006_0101, 0, 0, 0]]);
        assert!(ep1.sends_jr_timestamps());
    }

    /// 7.1.6.1, 7.1.6.2: request_configuration is the way to select a configuration; while the
    /// endpoint waits, it neither sends nor expects JR Timestamps.
    #[test]
    fn stream_configuration_request_waits_for_notification() {
        let mut d = description(true);
        d.txjr = true;
        let mut ep = Endpoint::try_new(d, blocks()).unwrap();
        let wanted = StreamConfiguration {
            protocol: StreamProtocol::Midi2,
            send_jr: true,
            receive_jr: true,
        };
        let request = ep.request_configuration(wanted).unwrap();
        assert_eq!(words(&request), [0xF005_0203, 0, 0, 0]);
        assert!(ep.is_waiting());
        assert!(!ep.sends_jr_timestamps() && !ep.expects_jr_timestamps());
        // Unrelated packets do not end the wait.
        assert!(collect(ep.receive(&discovery(1, 1, 0x01))).len() == 1);
        assert!(ep.is_waiting());
        // The peer confirms: JR to the peer (RXJR) and from the peer (TXJR).
        assert!(collect(ep.receive(&[0xF006_0203, 0, 0, 0])).is_empty());
        assert!(!ep.is_waiting());
        assert_eq!(ep.stream_configuration(), wanted);
        assert!(ep.sends_jr_timestamps() && ep.expects_jr_timestamps());
        // The peer answers with less: JR from the peer only.
        ep.request_configuration(wanted).unwrap();
        ep.receive(&[0xF006_0201, 0, 0, 0]).count();
        assert_eq!(
            ep.stream_configuration(),
            StreamConfiguration {
                protocol: StreamProtocol::Midi2,
                send_jr: false,
                receive_jr: true
            }
        );
        let mut ep = endpoint(true);
        assert_eq!(
            ep.request_configuration(wanted),
            Err(EndpointError::ProtocolNotSupported)
        );
        assert!(!ep.is_waiting());
        // REQ-SCFG-004, -007, -009, -010: with JR on in both directions, a request for MIDI 1.0
        // with no JR changes nothing before the Notification, and no JR Timestamp is sent or
        // expected while the endpoint waits.
        let mut d = description(true);
        d.txjr = true;
        let mut ep = Endpoint::try_new(d, blocks()).unwrap();
        ep.receive(&[0xF005_0203, 0, 0, 0]).count();
        assert_eq!(ep.stream_configuration(), wanted);
        assert!(ep.sends_jr_timestamps() && ep.expects_jr_timestamps());
        let midi1 = StreamConfiguration {
            protocol: StreamProtocol::Midi1,
            send_jr: false,
            receive_jr: false,
        };
        ep.request_configuration(midi1).unwrap();
        assert!(ep.is_waiting());
        assert_eq!(ep.stream_configuration(), wanted);
        assert!(!ep.sends_jr_timestamps());
        assert!(!ep.expects_jr_timestamps());
        ep.receive(&[0xF006_0100, 0, 0, 0]).count();
        assert!(!ep.is_waiting());
        assert_eq!(ep.stream_configuration(), midi1);
    }

    #[test]
    fn stream_configuration_needs_a_declared_jr_use() {
        // REQ-SCFG-002, -003: an endpoint that does not declare RXJR keeps receive_jr off and
        // replies with its current configuration.
        let mut d = description(true);
        d.rxjr = false;
        let mut ep = Endpoint::try_new(d, blocks()).unwrap();
        assert_eq!(
            collect(ep.receive(&[0xF005_0202, 0, 0, 0])),
            [[0xF006_0200, 0, 0, 0]]
        );
        assert!(!ep.expects_jr_timestamps());
        // REQ-SCFG-010: a Notification that offers a JR use the endpoint does not declare (it
        // has no TXJR) is not adopted.
        let mut ep = endpoint(true);
        ep.request_configuration(StreamConfiguration {
            protocol: StreamProtocol::Midi2,
            send_jr: false,
            receive_jr: false,
        })
        .unwrap();
        ep.receive(&[0xF006_0202, 0, 0, 0]).count();
        assert!(!ep.is_waiting());
        assert!(!ep.sends_jr_timestamps());
        // A Notification that the peer sends when its configuration changes (7.1.6.3) is followed
        // when the endpoint supports it.
        ep.receive(&[0xF006_0100, 0, 0, 0]).count();
        assert_eq!(ep.stream_configuration().protocol, StreamProtocol::Midi1);
    }

    /// REQ-JR-013: after an unexpected JR Timestamp, the application asks the sender to stop
    /// them with a Stream Configuration Request with no JR.
    #[test]
    fn unexpected_jr_timestamp_then_request_without_jr() {
        let mut r = crate::JrReceiver::<4>::new(0);
        r.receive(&[0x0010_03E8], false, 32_000);
        assert_eq!(
            r.receive(&[0x0020_044C], false, 32_000),
            crate::Disposition::UnexpectedJrTimestamp
        );
        let mut ep = endpoint(true);
        let request = ep
            .request_configuration(StreamConfiguration {
                protocol: StreamProtocol::Midi2,
                send_jr: false,
                receive_jr: false,
            })
            .unwrap();
        assert_eq!(words(&request), [0xF005_0200, 0, 0, 0]);
    }

    /// 3.2, 3.3.1, 7.1.6.2: the Protocol in effect limits the Channel Voice Message Type.
    #[test]
    fn check_outgoing_enforces_protocol() {
        let mut ep = endpoint(true);
        let cv1 = |group: u32| [0x2090_3C40 | group << 24];
        let cv2 = [0x4090_3C00, 0xFFFF_0000];
        assert_eq!(ep.check_outgoing(&cv2), Ok(()));
        // Group 1 belongs to a block without a MIDI 1.0 connection.
        assert_eq!(
            ep.check_outgoing(&cv1(1)),
            Err(EndpointError::WrongProtocol)
        );
        assert_eq!(
            ep.check_outgoing(&cv1(5)),
            Err(EndpointError::WrongProtocol)
        );
        // Group 0 is the MIDI 1.0 port.
        assert_eq!(ep.check_outgoing(&cv1(0)), Ok(()));
        ep.receive(&words(&StreamConfigurationRequest::new(
            StreamProtocol::Midi1,
            false,
            false,
        )))
        .count();
        assert_eq!(ep.check_outgoing(&cv2), Err(EndpointError::WrongProtocol));
        assert_eq!(ep.check_outgoing(&cv1(1)), Ok(()));
        // Other Message Types pass under both Protocols.
        assert_eq!(ep.check_outgoing(&[0x10F8_0000]), Ok(()));
        assert_eq!(ep.check_outgoing(&[0xF000_0101, 0, 0, 0]), Ok(()));
        assert_eq!(
            ep.check_outgoing(&[]),
            Err(EndpointError::Ump(UmpError::Truncated {
                needed: 1,
                got: 0
            }))
        );
    }

    /// 6.1.1, 7.1.8, 7.1.9: static blocks never change; a change of a non-static block gives
    /// its Info Notification, and its Name Notification when the name changed.
    #[test]
    fn function_block_notification_on_static_endpoint() {
        // REQ-CI-IO-MUID-001: the Info Notification of an unchanged block, also when static.
        let ep = endpoint(true);
        let replies = collect(ep.function_block_notification(1).unwrap());
        assert_eq!(replies, [FB1_INFO]);
        assert_eq!(
            ep.function_block_notification(2).map(|r| r.count()),
            Err(EndpointError::FunctionBlockNumbering)
        );
    }

    #[test]
    fn set_function_block_follows_static_rule() {
        let mut ep = endpoint(true);
        assert_eq!(
            ep.set_function_block(blocks()[1]).map(|r| r.count()),
            Err(EndpointError::StaticFunctionBlocks)
        );
        let mut ep = endpoint(false);
        let mut b = blocks()[1];
        b.info.active = false;
        assert_eq!(
            collect(ep.set_function_block(b).unwrap()),
            [[0xF011_0133, 0x0102_0201, 0, 0]]
        );
        assert!(!ep.function_blocks()[1].info.active);
        b.name = "Main 2";
        let replies = collect(ep.set_function_block(b).unwrap());
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[1], [0xF012_014D, 0x6169_6E20, 0x3200_0000, 0]);
        b.info.number = 2;
        assert_eq!(
            ep.set_function_block(b).map(|r| r.count()),
            Err(EndpointError::FunctionBlockNumbering)
        );
        b.info.number = 1;
        b.info.zmidi_ci_version = 0;
        assert_eq!(
            ep.set_function_block(b).map(|r| r.count()),
            Err(EndpointError::InvalidFunctionBlock)
        );
        assert_eq!(ep.function_blocks().len(), 2);
    }

    /// 7.1.2, 7.1.6.3: a capability change gives the Endpoint Info Notification, and a Stream
    /// Configuration Notification when the configuration falls back.
    #[test]
    fn set_capabilities_notifies() {
        let mut ep = endpoint(true);
        assert_eq!(
            collect(ep.set_capabilities(true, true, false, false).unwrap()),
            [[0xF001_0101, 0x8200_0300, 0, 0]]
        );
        ep.receive(&words(&StreamConfigurationRequest::new(
            StreamProtocol::Midi2,
            false,
            false,
        )))
        .count();
        assert_eq!(
            collect(ep.set_capabilities(true, false, false, false).unwrap()),
            [[0xF001_0101, 0x8200_0100, 0, 0], [0xF006_0100, 0, 0, 0]]
        );
        assert_eq!(ep.stream_configuration().protocol, StreamProtocol::Midi1);
        // A capability that is removed turns off the JR use in effect (REQ-SCFG-010 exception).
        let mut d = description(true);
        d.txjr = true;
        let mut jr = Endpoint::try_new(d, blocks()).unwrap();
        jr.receive(&[0xF005_0203, 0, 0, 0]).count();
        assert!(jr.sends_jr_timestamps() && jr.expects_jr_timestamps());
        assert_eq!(
            collect(jr.set_capabilities(true, true, false, false).unwrap()),
            [[0xF001_0101, 0x8200_0300, 0, 0], [0xF006_0200, 0, 0, 0]]
        );
        assert!(!jr.sends_jr_timestamps() && !jr.expects_jr_timestamps());
        assert_eq!(
            ep.set_capabilities(false, false, true, true)
                .map(|r| r.count()),
            Err(EndpointError::ProtocolNotSupported)
        );
    }

    /// The endpoint never panics on received words, and every reply is a valid UMP Stream
    /// message.
    #[test]
    fn prop_endpoint_receive_never_panics() {
        proptest!(|(w in any::<[u32; 4]>(), status in 0u32..0x20, len in 0usize..5)| {
            let mut ep = endpoint(false);
            let words = [0xF000_0000 | (w[0] & 0x0C00_FFFF) | status << 16, w[1], w[2], w[3]];
            for input in [&words[..], &w[..len.min(4)]] {
                let replies: Vec<_> = ep.receive(input).collect();
                prop_assert!(replies.len() <= 64);
                for r in replies {
                    prop_assert!(UmpStreamMessage::parse(&r).is_some(), "{:08X?}", r);
                }
            }
            let _ = ep.check_outgoing(&w[..len.min(4)]);
        });
    }
}
