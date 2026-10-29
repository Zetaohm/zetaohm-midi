//! Sender-side order rules for multi-packet messages (M2-104-UM 7.5.2, 7.5.10, 7.5.12, 7.7.1,
//! 7.8, 7.9.1).
//!
//! - Flex Data (7.5.2, within a Group): between the Start and End UMPs of a message, send only
//!   its Continue UMPs, System Real Time and JR Clock. Lyrics (7.5.10) and Ruby (7.5.12) UMPs
//!   are sent together. These are "should" rules: a caller that needs another order skips
//!   [`SendOrder::check`].
//! - System Exclusive (7-bit) (7.7.1): between Start and End, no other message on the same
//!   Group except System Real Time. Groupless messages and other Groups may interleave.
//! - System Exclusive 8 (7.8): a sender with one stream uses Stream ID 0; with N streams, IDs 0
//!   to N-1. A merger learns the receiver's stream count with Function Block Discovery before
//!   it sends simultaneous streams. The count is for the UMP Stream (7.1.8).
//! - Mixed Data Set (7.9.1): a set that stops early ends with a final Chunk, preferably header
//!   only (16 bytes); see [`end_mds_early`].
//!
//! No alloc (DEC-002), no I/O (DEC-003).

use zmidi_ump::{
    FunctionBlockDiscovery, Group, MdsHeader, MdsIds, MessageType, UmpError, UmpWords,
};

/// Number of Valid Bytes of a header-only Mixed Data Set Chunk (7.9.1, DEC-UMP-006).
const MDS_HEADER_ONLY: u16 = 16;

/// Error of the order guard (thiserror 2, no_std).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SendOrderError {
    /// A Flex Data message is open on the Group and the packet is not permitted (7.5.2).
    #[error("a Flex Data message is open")]
    FlexDataOpen,
    /// A System Exclusive (7-bit) message is open on the Group of the packet (7.7.1).
    #[error("a System Exclusive message is open on this Group")]
    SysEx7Open,
    /// The SysEx8 Stream ID is N or higher (7.8).
    #[error("SysEx8 Stream ID out of range")]
    StreamIdOutOfRange,
    /// A second simultaneous SysEx8 stream before the receiver's stream count is known (7.8).
    #[error("receiver SysEx8 stream count unknown; send Function Block Discovery first")]
    ReceiverStreamsUnknown,
    /// More simultaneous SysEx8 streams than the receiver supports (7.1.8, 7.8).
    #[error("too many simultaneous SysEx8 streams for the receiver")]
    TooManyStreams,
    /// The sender's stream count N is 0 or more than 256.
    #[error("SysEx8 stream count must be 1 to 256")]
    StreamCount,
}

/// Format of a multi-packet UMP: the 2 or 4 bits after the Group (Figures 69, 50, 55).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Complete,
    Start,
    Continue,
    End,
}

impl Part {
    fn from_bits(bits: u32) -> Self {
        match bits {
            0 => Self::Complete,
            1 => Self::Start,
            2 => Self::Continue,
            _ => Self::End,
        }
    }
}

/// The sender order guard. Fixed size; no alloc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOrder {
    /// Status Bank and Status of the open Flex Data message of each Group.
    flex: [Option<(u8, u8)>; 16],
    /// Bit g set: a SysEx7 message is open on Group g.
    sysex7: u16,
    /// Open SysEx8 Stream IDs of the UMP Stream, one bit each.
    streams: [u128; 2],
    /// Number of SysEx8 streams this sender supports, 1 to 256.
    own_streams: u16,
    /// The receiver's Max Number of SysEx 8 Streams, when known.
    receiver_streams: Option<u8>,
}

impl SendOrder {
    /// Creates a guard for a sender with `streams` SysEx8 streams (1 to 256). Returns
    /// [`SendOrderError::StreamCount`] for other values. The receiver's count is unknown.
    pub fn new(streams: u16) -> Result<Self, SendOrderError> {
        if streams == 0 || streams > 256 {
            return Err(SendOrderError::StreamCount);
        }
        Ok(Self {
            flex: [None; 16],
            sysex7: 0,
            streams: [0; 2],
            own_streams: streams,
            receiver_streams: None,
        })
    }

    /// Records the receiver's Max Number of SysEx 8 Streams from its Function Block Info
    /// Notification (7.1.8): 0 none, 1 one at a time, 2 to 255 simultaneous.
    pub fn set_receiver_streams(&mut self, streams: u8) {
        self.receiver_streams = Some(streams);
    }

    /// The Function Block Discovery packet (all blocks, Filter Info) that a merger sends to
    /// learn the receiver's SysEx8 stream count (7.8, 7.1.7).
    pub fn discovery_request() -> [u32; 4] {
        match FunctionBlockDiscovery::try_new(FunctionBlockDiscovery::ALL, true, false) {
            Ok(m) => packet(&m),
            // 0xFF is always a valid Function Block Number (7.1.7).
            Err(_) => unreachable!(),
        }
    }

    /// Checks one outgoing packet. On `Ok` the state takes the packet into account; on an
    /// error the state is unchanged. Never panics.
    pub fn check(&mut self, words: &[u32]) -> Result<(), SendOrderError> {
        let Some(&w0) = words.first() else {
            return Ok(());
        };
        let message_type = MessageType::from_bits((w0 >> 28) as u8);
        let group = ((w0 >> 24) & 0xF) as usize;
        let groupless = matches!(message_type, MessageType::Utility | MessageType::UmpStream);
        // System Real Time: Message Type 0x1, status 0xF8 to 0xFF (7.6).
        let real_time = message_type == MessageType::System && (w0 >> 16) & 0xFF >= 0xF8;
        // JR Clock: Message Type 0x0, Status 0x1 (7.2.2.1).
        let jr_clock = message_type == MessageType::Utility && (w0 >> 20) & 0xF == 0x1;

        // 7.5.2: Flex Data rules, within a Group.
        let flex_part = (message_type == MessageType::FlexData).then(|| {
            // Format: bits 23..22; Status Bank: 15..8; Status: 7..0 (Figure 69).
            let id = ((w0 >> 8) as u8, w0 as u8);
            (Part::from_bits((w0 >> 22) & 0b11), id)
        });
        if groupless {
            if !jr_clock && self.flex.iter().any(Option::is_some) {
                return Err(SendOrderError::FlexDataOpen);
            }
        } else if let Some(open) = self.flex[group] {
            let same_message = matches!(
                flex_part,
                Some((Part::Continue | Part::End, id)) if id == open
            );
            if !real_time && !same_message {
                return Err(SendOrderError::FlexDataOpen);
            }
        }

        // 7.7.1: SysEx7 rules, within a Group.
        let sysex7_part =
            (message_type == MessageType::Data64).then(|| Part::from_bits((w0 >> 20) & 0xF));
        let sysex7_open = !groupless && self.sysex7 & (1 << group) != 0;
        if sysex7_open && !real_time && !matches!(sysex7_part, Some(Part::Continue | Part::End)) {
            return Err(SendOrderError::SysEx7Open);
        }

        // 7.8: SysEx8 Stream IDs. Status 0x0 to 0x3 (Table 19); 0x8 and 0x9 are MDS.
        let sysex8 = (message_type == MessageType::Data128 && (w0 >> 20) & 0xF <= 0x3).then(|| {
            (
                Part::from_bits((w0 >> 20) & 0xF),
                ((w0 >> 8) & 0xFF) as usize,
            )
        });
        if let Some((part, id)) = sysex8 {
            if id >= usize::from(self.own_streams) {
                return Err(SendOrderError::StreamIdOutOfRange);
            }
            let open = self.streams[id / 128] & (1 << (id % 128)) != 0;
            if !open && matches!(part, Part::Complete | Part::Start) {
                let count = self.streams.iter().map(|s| s.count_ones()).sum::<u32>() + 1;
                match self.receiver_streams {
                    Some(max) if count > u32::from(max) => {
                        return Err(SendOrderError::TooManyStreams)
                    }
                    None if count > 1 => return Err(SendOrderError::ReceiverStreamsUnknown),
                    _ => {}
                }
            }
        }

        // The packet is permitted: update the state.
        if let Some((part, id)) = flex_part {
            match part {
                Part::Start => self.flex[group] = Some(id),
                Part::End => self.flex[group] = None,
                Part::Complete | Part::Continue => {}
            }
        }
        match sysex7_part {
            Some(Part::Start) => self.sysex7 |= 1 << group,
            Some(Part::End) => self.sysex7 &= !(1 << group),
            _ => {}
        }
        if let Some((part, id)) = sysex8 {
            let bit = 1u128 << (id % 128);
            match part {
                Part::Start => self.streams[id / 128] |= bit,
                Part::End => self.streams[id / 128] &= !bit,
                Part::Complete | Part::Continue => {}
            }
        }
        Ok(())
    }
}

/// The header-only final Chunk (Number of Valid Bytes 16) that ends a Mixed Data Set early and
/// closes its MDS ID (7.9.1). For valid data, Number of Chunks equals `number_of_this_chunk`;
/// for data of unknown validity, Number of This Chunk is 0 and Number of Chunks stays
/// `number_of_chunks` (REQ-MDS-002). Returns `OutOfRange` for an MDS ID above 15.
pub fn end_mds_early(
    group: Group,
    mds_id: u8,
    ids: MdsIds,
    number_of_chunks: u16,
    number_of_this_chunk: u16,
    valid: bool,
) -> Result<[u32; 4], UmpError> {
    let (chunks, this) = if valid {
        (number_of_this_chunk, number_of_this_chunk)
    } else {
        (number_of_chunks, 0)
    };
    MdsHeader::try_new(group, mds_id, MDS_HEADER_ONLY, chunks, this, ids).map(|h| packet(&h))
}

/// The words of a 128-bit packet.
fn packet<T: UmpWords>(message: &T) -> [u32; 4] {
    let mut out = [0u32; 4];
    let _ = message.to_words(&mut out);
    out
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;
    use zmidi_ump::ManufacturerId;

    /// Flex Data on Group 1, Address 1, Status Bank 0x01 (Metadata Text), Status 0x01.
    const FLEX_START: u32 = 0xD150_0101;
    const FLEX_CONTINUE: u32 = 0xD190_0101;
    const FLEX_END: u32 = 0xD1D0_0101;
    /// Flex Data Lyrics (Status Bank 0x02, Status 0x01) Start on Group 1.
    const LYRICS_START: u32 = 0xD150_0201;
    const NOTE_ON_G1: u32 = 0x2190_3C64;
    const NOTE_ON_G2: u32 = 0x2290_3C64;
    const CLOCK_G1: u32 = 0x11F8_0000;
    const JR_CLOCK: u32 = 0x0010_0000;
    const JR_TIMESTAMP: u32 = 0x0020_0000;

    fn sx7(group: u32, status: u32) -> [u32; 2] {
        [0x3000_0000 | (group << 24) | (status << 20), 0]
    }

    fn sx8(status: u32, stream: u32) -> [u32; 4] {
        [
            0x5000_0000 | (status << 20) | (1 << 16) | (stream << 8),
            0,
            0,
            0,
        ]
    }

    #[test]
    fn flex_data_permits_continue_real_time_and_jr_clock_only() {
        let mut s = SendOrder::new(1).unwrap();
        s.check(&[FLEX_START, 0, 0, 0]).unwrap();
        s.check(&[CLOCK_G1]).unwrap();
        s.check(&[JR_CLOCK]).unwrap();
        s.check(&[FLEX_CONTINUE, 0, 0, 0]).unwrap();
        assert_eq!(s.check(&[NOTE_ON_G1]), Err(SendOrderError::FlexDataOpen));
        assert_eq!(s.check(&[JR_TIMESTAMP]), Err(SendOrderError::FlexDataOpen));
        assert_eq!(
            s.check(&[LYRICS_START, 0, 0, 0]),
            Err(SendOrderError::FlexDataOpen)
        );
        // 7.5.2 rules are within a Group: another Group may interleave.
        s.check(&[NOTE_ON_G2]).unwrap();
        s.check(&[FLEX_END, 0, 0, 0]).unwrap();
        s.check(&[NOTE_ON_G1]).unwrap();
    }

    #[test]
    fn lyrics_umps_are_kept_together() {
        let mut s = SendOrder::new(1).unwrap();
        s.check(&[LYRICS_START, 0, 0, 0]).unwrap();
        // A Continue of another text message is not part of this Lyrics message.
        assert_eq!(
            s.check(&[FLEX_CONTINUE, 0, 0, 0]),
            Err(SendOrderError::FlexDataOpen)
        );
        s.check(&[0xD1D0_0201, 0, 0, 0]).unwrap();
        s.check(&[FLEX_CONTINUE, 0, 0, 0]).unwrap();
        // The same for Ruby (Status 0x03).
        s.check(&[0xD150_0203, 0, 0, 0]).unwrap();
        assert_eq!(
            s.check(&[FLEX_CONTINUE, 0, 0, 0]),
            Err(SendOrderError::FlexDataOpen)
        );
        s.check(&[0xD1D0_0203, 0, 0, 0]).unwrap();
        s.check(&[FLEX_CONTINUE, 0, 0, 0]).unwrap();
    }

    #[test]
    fn sysex7_blocks_same_group_only() {
        let mut s = SendOrder::new(1).unwrap();
        s.check(&sx7(1, 1)).unwrap();
        s.check(&[CLOCK_G1]).unwrap();
        s.check(&[NOTE_ON_G2]).unwrap();
        s.check(&[JR_TIMESTAMP]).unwrap();
        s.check(&sx7(2, 0)).unwrap();
        assert_eq!(s.check(&[NOTE_ON_G1]), Err(SendOrderError::SysEx7Open));
        assert_eq!(s.check(&sx7(1, 1)), Err(SendOrderError::SysEx7Open));
        assert_eq!(s.check(&sx7(1, 0)), Err(SendOrderError::SysEx7Open));
        s.check(&sx7(1, 2)).unwrap();
        s.check(&sx7(1, 3)).unwrap();
        s.check(&[NOTE_ON_G1]).unwrap();
    }

    #[test]
    fn sysex8_stream_ids_stay_below_sender_count() {
        let mut s = SendOrder::new(1).unwrap();
        assert_eq!(s.check(&sx8(1, 1)), Err(SendOrderError::StreamIdOutOfRange));
        s.check(&sx8(1, 0)).unwrap();
        let mut s = SendOrder::new(4).unwrap();
        s.check(&sx8(0, 3)).unwrap();
        assert_eq!(s.check(&sx8(0, 4)), Err(SendOrderError::StreamIdOutOfRange));
        assert_eq!(SendOrder::new(0), Err(SendOrderError::StreamCount));
        assert_eq!(SendOrder::new(257), Err(SendOrderError::StreamCount));
    }

    #[test]
    fn sysex8_simultaneous_streams_need_receiver_count() {
        let mut s = SendOrder::new(4).unwrap();
        s.check(&sx8(1, 0)).unwrap();
        assert_eq!(
            s.check(&sx8(1, 1)),
            Err(SendOrderError::ReceiverStreamsUnknown)
        );
        s.set_receiver_streams(2);
        s.check(&sx8(1, 1)).unwrap();
        assert_eq!(s.check(&sx8(0, 2)), Err(SendOrderError::TooManyStreams));
        s.check(&sx8(3, 0)).unwrap();
        s.check(&sx8(0, 2)).unwrap();
        s.set_receiver_streams(0);
        let mut t = SendOrder::new(1).unwrap();
        t.set_receiver_streams(0);
        assert_eq!(t.check(&sx8(0, 0)), Err(SendOrderError::TooManyStreams));
        // An End closes only its own stream: with a receiver count of 1, stream 1 after the End
        // of stream 0 is allowed, and stream 2 while stream 1 is open is not.
        let mut u = SendOrder::new(4).unwrap();
        u.set_receiver_streams(1);
        u.check(&sx8(1, 0)).unwrap();
        u.check(&sx8(3, 0)).unwrap();
        u.check(&sx8(1, 1)).unwrap();
        assert_eq!(u.check(&sx8(1, 2)), Err(SendOrderError::TooManyStreams));
        // Stream IDs of 128 and more (7.8: up to 255).
        let mut v = SendOrder::new(256).unwrap();
        v.set_receiver_streams(2);
        v.check(&sx8(1, 130)).unwrap();
        v.check(&sx8(1, 2)).unwrap();
        assert_eq!(v.check(&sx8(1, 200)), Err(SendOrderError::TooManyStreams));
        v.check(&sx8(2, 130)).unwrap();
        v.check(&sx8(3, 130)).unwrap();
        v.check(&sx8(1, 200)).unwrap();
    }

    #[test]
    fn only_system_real_time_passes_an_open_sysex7_and_utility_is_not_sysex8() {
        // 7.7.1: only System Real Time (Message Type 0x1, F8H to FFH) may come between the UMPs
        // of a SysEx7 message on its Group.
        let mut s = SendOrder::new(1).unwrap();
        s.check(&sx7(1, 1)).unwrap();
        assert_eq!(s.check(&[0x11F2_0102]), Err(SendOrderError::SysEx7Open));
        assert_eq!(s.check(&[0x21F8_0000]), Err(SendOrderError::SysEx7Open));
        s.check(&[CLOCK_G1]).unwrap();
        // A Utility message is not a SysEx8 packet, whatever its bits 15..8.
        s.check(&[0x0020_0500]).unwrap();
        // Mixed Data Set packets (Status 0x8, 0x9) are not SysEx8 packets either (7.8, 7.9).
        s.check(&[0x5080_0500, 0, 0, 0]).unwrap();
        s.check(&[0x5090_0500, 0, 0, 0]).unwrap();
    }

    #[test]
    fn discovery_request_asks_all_blocks_for_info() {
        // Figure 20: Status 0x010, Function Block Number 0xFF, Filter bit i.
        assert_eq!(SendOrder::discovery_request(), [0xF010_FF01, 0, 0, 0]);
    }

    #[test]
    fn end_mds_early_gives_header_only_final_chunk() {
        let ids = MdsIds {
            manufacturer: ManufacturerId::new(0x007E).unwrap(),
            device_id: 0x1234,
            sub_id1: 5,
            sub_id2: 6,
        };
        let g = Group::from_bits(0);
        // Valid data: Number of Chunks = Number of This Chunk = 3, Valid Bytes 16.
        assert_eq!(
            end_mds_early(g, 4, ids, 10, 3, true),
            Ok([0x5084_0010, 0x0003_0003, 0x007E_1234, 0x0005_0006])
        );
        // Unknown validity: Number of This Chunk 0.
        assert_eq!(
            end_mds_early(g, 4, ids, 10, 3, false),
            Ok([0x5084_0010, 0x000A_0000, 0x007E_1234, 0x0005_0006])
        );
        assert_eq!(
            end_mds_early(g, 16, ids, 10, 3, true),
            Err(UmpError::OutOfRange)
        );
    }

    #[test]
    fn prop_check_never_panics_and_error_keeps_state() {
        proptest!(|(packets in proptest::collection::vec(
            proptest::collection::vec(any::<u32>(), 0..5), 0..32))| {
            let mut s = SendOrder::new(2).unwrap();
            for words in &packets {
                let before = s.clone();
                if s.check(words).is_err() {
                    prop_assert_eq!(&s, &before);
                }
            }
        });
    }
}
