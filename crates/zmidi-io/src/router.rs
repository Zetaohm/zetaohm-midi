//! Receive-side addressing of UMPs (M2-104-UM 2.1.2, Appendix H).
//!
//! - Message Type 0x0 (Utility) and 0xF (UMP Stream) have no Group field (2.1.2) and go to the
//!   UMP Stream: the endpoint itself.
//! - Message Type 0x1 (System), 0x3 (Data 64, SysEx7) and 0x5 (Data 128) go to their Group.
//!   MIDI-CI Source/Destination and SysEx8/MDS addressing are for higher layers ("see
//!   individual specifications", Appendix H).
//! - Message Type 0x2 and 0x4 (Channel Voice) go to their Group and Channel.
//! - Message Type 0xD (Flex Data) goes by its Address field (7.5.1): 0 to the Channel, 1 to the
//!   Group; 2 and 3 are reserved.
//! - Reserved Message Types have no known destination. Receivers skip them by their Table 4
//!   size.
//!
//! Routing keeps no state, so UMPs of different Groups route the same in any interleaved order
//! (2.1.2). No alloc (DEC-002), no I/O (DEC-003).

use zmidi_ump::{Channel, FlexAddress, FunctionBlockInfo, Group, MessageType};

/// The destination of one received packet (Appendix H).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Destination {
    /// The UMP Stream: endpoint-level messages with no Group.
    Stream,
    /// One Group: messages that apply to all Channels of the Group.
    Group(Group),
    /// One Channel of one Group.
    Channel(Group, Channel),
    /// No destination: a reserved Message Type, a reserved Flex Data Address, or too few words.
    Unroutable,
}

/// Gives the destination of the packet at the start of `words` (Appendix H). Uses only word 0.
/// Never panics.
pub fn route(words: &[u32]) -> Destination {
    let Some(&w0) = words.first() else {
        return Destination::Unroutable;
    };
    let message_type = MessageType::from_bits((w0 >> 28) as u8);
    if words.len() < message_type.word_count() {
        return Destination::Unroutable;
    }
    // Group: bits 27..24; Channel: bits 19..16 (2.1.2, Figures 34, 47, 69).
    let group = Group::from_bits((w0 >> 24) as u8 & 0xF);
    let channel = Channel::from_bits((w0 >> 16) as u8 & 0xF);
    match message_type {
        MessageType::Utility | MessageType::UmpStream => Destination::Stream,
        MessageType::System | MessageType::Data64 | MessageType::Data128 => {
            Destination::Group(group)
        }
        MessageType::Midi1ChannelVoice | MessageType::Midi2ChannelVoice => {
            Destination::Channel(group, channel)
        }
        // Flex Data Address: bits 21..20 of word 0 (Figure 69).
        MessageType::FlexData => match FlexAddress::from_bits((w0 >> 20) as u8 & 0b11) {
            Some(FlexAddress::Channel) => Destination::Channel(group, channel),
            Some(FlexAddress::Group) => Destination::Group(group),
            None => Destination::Unroutable,
        },
        _ => Destination::Unroutable,
    }
}

/// Gives the active Function Blocks of `blocks` whose Groups hold `group` (Appendix H, 7.1.8).
pub fn blocks_for_group(blocks: &[FunctionBlockInfo], group: Group) -> BlocksForGroup<'_> {
    BlocksForGroup {
        blocks: blocks.iter(),
        group,
    }
}

/// Lazy iterator of [`blocks_for_group`]. No alloc.
#[derive(Debug, Clone)]
pub struct BlocksForGroup<'a> {
    /// The blocks not yet examined.
    blocks: core::slice::Iter<'a, FunctionBlockInfo>,
    /// The Group to match.
    group: Group,
}

impl<'a> Iterator for BlocksForGroup<'a> {
    type Item = &'a FunctionBlockInfo;

    fn next(&mut self) -> Option<Self::Item> {
        let g = u16::from(self.group.into_bits());
        self.blocks.by_ref().find(|b| {
            let first = u16::from(b.first_group.into_bits());
            b.active && first <= g && g < first + u16::from(b.groups_spanned)
        })
    }
}

/// Where a received MIDI-CI message goes on a UMP connection (M2-101-UM 5.2, Appendix H;
/// REQ-CI-IO-ADDR-001 to -003). The Device ID is the raw byte, so zmidi-io needs no zmidi-ci
/// (DEC-009). A reply goes on `group`, the Group of the received message.
#[derive(Debug, Clone)]
pub enum CiAddress<'a> {
    /// Device ID 0x00-0x0F: one Channel of the Group.
    Channel { group: Group, channel: Channel },
    /// Device ID 0x7E: the Group.
    Group { group: Group },
    /// Device ID 0x7F: each Function Block that holds the Group.
    FunctionBlocks {
        group: Group,
        blocks: BlocksForGroup<'a>,
    },
    /// Device ID 0x7F with no Function Block declared: the Group as a Function Block with one
    /// member Group.
    GroupAsBlock { group: Group },
    /// A reserved Device ID (0x10-0x7D) or a byte with bit 7 set.
    Invalid,
}

/// Gives the [`CiAddress`] of a MIDI-CI message with `device_id` received on `group`.
/// `blocks` are the Function Blocks of the endpoint (empty when none is declared).
pub fn ci_address(blocks: &[FunctionBlockInfo], group: Group, device_id: u8) -> CiAddress<'_> {
    match device_id {
        0x00..=0x0F => match Channel::new(device_id) {
            Ok(channel) => CiAddress::Channel { group, channel },
            Err(_) => CiAddress::Invalid,
        },
        0x7E => CiAddress::Group { group },
        0x7F if blocks.is_empty() => CiAddress::GroupAsBlock { group },
        0x7F => CiAddress::FunctionBlocks {
            group,
            blocks: blocks_for_group(blocks, group),
        },
        _ => CiAddress::Invalid,
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;
    use zmidi_ump::{FunctionBlockDirection, FunctionBlockMidi1, UiHint};

    fn g(i: u8) -> Group {
        Group::from_bits(i)
    }

    fn c(i: u8) -> Channel {
        Channel::from_bits(i)
    }

    #[test]
    fn route_follows_appendix_h_per_message_type() {
        // Utility JR Clock, UMP Stream Endpoint Discovery: the UMP Stream.
        assert_eq!(route(&[0x0010_1234]), Destination::Stream);
        assert_eq!(route(&[0xF000_0101, 0x1F, 0, 0]), Destination::Stream);
        // System Timing Clock on Group 3, SysEx7 on Group 5, SysEx8 on Group 9: the Group.
        assert_eq!(route(&[0x13F8_0000]), Destination::Group(g(3)));
        assert_eq!(route(&[0x3500_0000, 0]), Destination::Group(g(5)));
        assert_eq!(route(&[0x5900_0000, 0, 0, 0]), Destination::Group(g(9)));
        // MIDI 1.0 Note On Group 2 Channel 7; MIDI 2.0 Note On Group 15 Channel 0.
        assert_eq!(route(&[0x2297_3C64]), Destination::Channel(g(2), c(7)));
        assert_eq!(route(&[0x4F90_3C00, 0]), Destination::Channel(g(15), c(0)));
    }

    #[test]
    fn route_flex_data_by_address_field() {
        // Address 0 (Channel 4), Address 1 (Group), Address 2 and 3 (reserved): Figure 69.
        assert_eq!(
            route(&[0xD104_0000, 0, 0, 0]),
            Destination::Channel(g(1), c(4))
        );
        assert_eq!(route(&[0xD110_0000, 0, 0, 0]), Destination::Group(g(1)));
        assert_eq!(route(&[0xD120_0000, 0, 0, 0]), Destination::Unroutable);
        assert_eq!(route(&[0xD130_0000, 0, 0, 0]), Destination::Unroutable);
    }

    #[test]
    fn route_reserved_or_short_is_unroutable() {
        assert_eq!(route(&[]), Destination::Unroutable);
        assert_eq!(route(&[0x6000_0000]), Destination::Unroutable);
        assert_eq!(route(&[0xE000_0000, 0, 0, 0]), Destination::Unroutable);
        assert_eq!(route(&[0x4090_3C00]), Destination::Unroutable); // 64-bit, 1 word
    }

    #[test]
    fn route_interleaved_groups_route_as_in_sequence() {
        // 2.1.2: UMPs of different Groups may be interleaved in any order.
        let a = [0x2090_3C64u32, 0x2091_3C64, 0x2092_3C64];
        let b = [0x2390_3C64u32, 0x2391_3C64];
        let interleaved = [a[0], b[0], a[1], b[1], a[2]];
        let expected = [
            Destination::Channel(g(0), c(0)),
            Destination::Channel(g(3), c(0)),
            Destination::Channel(g(0), c(1)),
            Destination::Channel(g(3), c(1)),
            Destination::Channel(g(0), c(2)),
        ];
        for (w, e) in interleaved.iter().zip(expected) {
            assert_eq!(route(&[*w]), e);
        }
    }

    fn block(number: u8, active: bool, first: u8, spanned: u8) -> FunctionBlockInfo {
        FunctionBlockInfo {
            active,
            number,
            direction: FunctionBlockDirection::Bidirectional,
            midi1: FunctionBlockMidi1::NotMidi1,
            ui_hint: UiHint::Unknown,
            first_group: g(first),
            groups_spanned: spanned,
            zmidi_ci_version: 1,
            max_sysex8_streams: 0,
        }
    }

    #[test]
    fn blocks_for_group_gives_active_blocks_that_span_the_group() {
        let blocks = [
            block(0, true, 0, 4),  // Groups 0..3
            block(1, true, 2, 2),  // Groups 2..3
            block(2, false, 2, 1), // inactive
            block(3, true, 15, 1), // Group 15
        ];
        let numbers = |grp| {
            blocks_for_group(&blocks, g(grp))
                .map(|b| b.number)
                .collect::<std::vec::Vec<_>>()
        };
        assert_eq!(numbers(2), [0, 1]);
        assert_eq!(numbers(0), [0]);
        assert_eq!(numbers(4), []);
        assert_eq!(numbers(15), [3]);
    }

    #[test]
    fn prop_route_never_panics_and_is_stateless() {
        proptest!(|(words in proptest::collection::vec(any::<u32>(), 0..5))| {
            let first = route(&words);
            prop_assert_eq!(first, route(&words));
        });
    }

    #[test]
    fn ci_address_by_device_id() {
        // REQ-CI-IO-ADDR-001 to -003 (M2-101-UM 5.2.1, Appendix H).
        let fbs = [
            block(0, true, 0, 2),
            block(1, false, 1, 1),
            block(2, true, 1, 3),
        ];
        assert!(
            matches!(ci_address(&fbs, g(1), 0x03), CiAddress::Channel { group, channel } if group == g(1) && channel.into_bits() == 3)
        );
        assert!(
            matches!(ci_address(&fbs, g(1), 0x7E), CiAddress::Group { group } if group == g(1))
        );
        // The limits of the Channel range: 0x00 is Channel 1, 0x0F is Channel 16; 0x10 and 0x7D
        // are not addresses.
        assert!(
            matches!(ci_address(&fbs, g(1), 0x00), CiAddress::Channel { channel, .. } if channel.into_bits() == 0)
        );
        assert!(
            matches!(ci_address(&fbs, g(1), 0x0F), CiAddress::Channel { channel, .. } if channel.into_bits() == 15)
        );
        assert!(matches!(ci_address(&fbs, g(1), 0x10), CiAddress::Invalid));
        assert!(matches!(ci_address(&fbs, g(1), 0x7D), CiAddress::Invalid));
        match ci_address(&fbs, g(1), 0x7F) {
            CiAddress::FunctionBlocks { group, blocks } => {
                assert_eq!(group, g(1), "the reply goes on the Group of the message");
                // Blocks 0 and 2 have Group 1 as a member; block 1 is not active.
                let n: std::vec::Vec<_> = blocks.map(|b| b.number).collect();
                assert_eq!(n, [0, 2]);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            matches!(ci_address(&[], g(4), 0x7F), CiAddress::GroupAsBlock { group } if group == g(4))
        );
        assert!(matches!(ci_address(&fbs, g(1), 0x10), CiAddress::Invalid));
        assert!(matches!(ci_address(&fbs, g(1), 0x80), CiAddress::Invalid));
    }
}
