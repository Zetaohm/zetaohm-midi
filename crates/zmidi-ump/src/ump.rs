//! Top-level UMP message and dispatch (M2-104-UM 2.1).
//!
//! - [`Ump::parse`] reads one packet. The Message Type (bits 31..28 of word 0) gives the packet
//!   size (2.1.4, Table 4) and selects the message family; the family reads the Status at the
//!   position and width its Message Type defines (2.1.2, Figure 3).
//! - A packet with a reserved Message Type, or a Status the crate does not define, is an
//!   [`UnknownPacket`] of the Table 4 size (2.1.4). It is not an error, and the caller
//!   continues with the next packet.
//! - [`UnknownPacket`] keeps the raw words unchanged and has no public constructor, so the crate
//!   writes a reserved Message Type or Status only as the words of a received packet
//!   (2.1.3, 2.1.4).
//! - [`Packet`] holds the raw words of any one UMP with its Table 4 size (2.1.4). Code that only
//!   moves UMPs (readers, writers, transports) uses it and does not decode the message; a caller
//!   gets the typed value with [`Packet::ump`] (DEC-018).

use crate::channel_voice1::Midi1ChannelVoiceMessage;
use crate::channel_voice2::Midi2ChannelVoiceMessage;
use crate::data128::Data128Message;
use crate::flex_data::FlexDataMessage;
use crate::packet::{put, MessageType, UmpError};
use crate::sysex7::SysEx7;
use crate::system::SystemMessage;
use crate::ump_stream::UmpStreamMessage;
use crate::utility::UtilityMessage;

/// One UMP: a message of a family the crate decodes, or an unknown packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ump {
    /// Message Type 0x0: Utility Messages (7.2).
    Utility(UtilityMessage),
    /// Message Type 0x1: System Common and System Real Time Messages (7.6).
    System(SystemMessage),
    /// Message Type 0x2: MIDI 1.0 Channel Voice Messages (7.3).
    Midi1ChannelVoice(Midi1ChannelVoiceMessage),
    /// Message Type 0x4: MIDI 2.0 Channel Voice Messages (7.4).
    Midi2ChannelVoice(Midi2ChannelVoiceMessage),
    /// Message Type 0x3: one System Exclusive (7-bit) packet (7.7).
    SysEx7(SysEx7),
    /// Message Type 0x5: System Exclusive 8 and Mixed Data Set Messages (7.8, 7.9).
    Data128(Data128Message),
    /// Message Type 0xD: Flex Data Messages (7.5).
    FlexData(FlexDataMessage),
    /// Message Type 0xF: UMP Stream Messages (7.1).
    UmpStream(UmpStreamMessage),
    /// A packet the crate does not decode, kept as raw words.
    Unknown(UnknownPacket),
}

impl Ump {
    /// Reads one packet from the start of `words` and gives it with the number of words used.
    ///
    /// Gives `Truncated` when `words` holds fewer words than Table 4 gives for the Message Type
    /// of word 0, or when `words` is empty. A Message Type whose Cargo feature is off gives
    /// `Unknown` ([`MessageType::is_decoded`], DEC-017). Never panics.
    pub fn parse(words: &[u32]) -> Result<(Self, usize), UmpError> {
        let Some(&w0) = words.first() else {
            return Err(UmpError::Truncated { needed: 1, got: 0 });
        };
        let message_type = MessageType::from_bits((w0 >> 28) as u8);
        let len = message_type.word_count();
        let Some(packet) = words.get(..len) else {
            return Err(UmpError::Truncated {
                needed: len,
                got: words.len(),
            });
        };
        // DEC-017: a family whose feature is off is not decoded; its packets are Unknown.
        let message = match message_type {
            #[cfg(feature = "utility")]
            MessageType::Utility => UtilityMessage::parse(packet).map(Self::Utility),
            #[cfg(feature = "system-common")]
            MessageType::System => SystemMessage::parse(packet).map(Self::System),
            #[cfg(feature = "channel-voice1")]
            MessageType::Midi1ChannelVoice => {
                Midi1ChannelVoiceMessage::parse(packet).map(Self::Midi1ChannelVoice)
            }
            #[cfg(feature = "channel-voice2")]
            MessageType::Midi2ChannelVoice => {
                Midi2ChannelVoiceMessage::parse(packet).map(Self::Midi2ChannelVoice)
            }
            #[cfg(feature = "sysex7")]
            MessageType::Data64 => SysEx7::parse(packet).map(Self::SysEx7),
            #[cfg(feature = "data128")]
            MessageType::Data128 => Data128Message::parse(packet).map(Self::Data128),
            #[cfg(feature = "flex-data")]
            MessageType::FlexData => FlexDataMessage::parse(packet).map(Self::FlexData),
            #[cfg(feature = "ump-stream")]
            MessageType::UmpStream => UmpStreamMessage::parse(packet).map(Self::UmpStream),
            _ => None,
        };
        let message = message.unwrap_or_else(|| Self::Unknown(UnknownPacket::from_packet(packet)));
        Ok((message, len))
    }

    /// Writes the packet into the start of `out` and gives the number of words written.
    ///
    /// An unknown packet is written as its unchanged raw words. Gives `BufferTooSmall` when
    /// `out` holds fewer words than the packet. Never panics.
    pub fn write(&self, out: &mut [u32]) -> Result<usize, UmpError> {
        match self {
            Self::Utility(m) => put(&[m.into_bits()], out),
            Self::System(m) => put(&[m.into_bits()], out),
            Self::Midi1ChannelVoice(m) => put(&[m.into_bits()], out),
            Self::Midi2ChannelVoice(m) => put(&m.into_words(), out),
            Self::SysEx7(m) => put(&m.into_words(), out),
            Self::Data128(m) => put(&m.into_words(), out),
            Self::FlexData(m) => put(&m.into_words(), out),
            Self::UmpStream(m) => put(&m.into_words(), out),
            Self::Unknown(p) => put(p.words(), out),
        }
    }

    /// Packet size of this message in 32-bit words (Table 4).
    pub fn word_count(&self) -> usize {
        match self {
            Self::Utility(_) => MessageType::Utility.word_count(),
            Self::System(_) => MessageType::System.word_count(),
            Self::Midi1ChannelVoice(_) => MessageType::Midi1ChannelVoice.word_count(),
            Self::Midi2ChannelVoice(_) => MessageType::Midi2ChannelVoice.word_count(),
            Self::SysEx7(_) => MessageType::Data64.word_count(),
            Self::Data128(_) => MessageType::Data128.word_count(),
            Self::FlexData(_) => MessageType::FlexData.word_count(),
            Self::UmpStream(_) => MessageType::UmpStream.word_count(),
            Self::Unknown(p) => p.message_type().word_count(),
        }
    }
}

/// A packet with a reserved Message Type or a Status the crate does not define (2.1.4).
///
/// It keeps the received words unchanged, reserved bits included. Its length is always the
/// Table 4 size of its Message Type. Only [`Ump::parse`] creates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnknownPacket {
    /// Words 0 to 3; words after the packet size are zero.
    words: [u32; 4],
}

impl UnknownPacket {
    /// `packet` holds exactly the Table 4 size of the Message Type of word 0 (1 to 4 words).
    fn from_packet(packet: &[u32]) -> Self {
        let mut words = [0; 4];
        for (to, from) in words.iter_mut().zip(packet) {
            *to = *from;
        }
        Self { words }
    }

    /// The raw words of the packet. The length is the Table 4 size of the Message Type.
    pub fn words(&self) -> &[u32] {
        let len = self.message_type().word_count();
        &self.words[..len]
    }

    /// The Message Type of the packet (bits 31..28 of word 0).
    pub fn message_type(&self) -> MessageType {
        MessageType::from_bits((self.words[0] >> 28) as u8)
    }
}

/// The raw words of one UMP, any Message Type, reserved values included (2.1.4, DEC-018).
///
/// Its length is always the Table 4 size of the Message Type of word 0, so a caller can step
/// through a word stream with it. The words are kept unchanged, reserved bits included
/// (DEC-UMP-002: code that forwards packets uses raw words). 16 bytes; no alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Packet {
    /// Words 0 to 3; words after the packet size are zero.
    words: [u32; 4],
}

impl Packet {
    /// Takes the packet at the start of `words`: the Table 4 size of the Message Type of word 0.
    /// Words after the packet are not used. Gives `Truncated` when `words` is empty or shorter
    /// than that size. Never panics.
    pub fn from_words(words: &[u32]) -> Result<Self, UmpError> {
        let Some(&w0) = words.first() else {
            return Err(UmpError::Truncated { needed: 1, got: 0 });
        };
        let len = MessageType::from_bits((w0 >> 28) as u8).word_count();
        let Some(packet) = words.get(..len) else {
            return Err(UmpError::Truncated {
                needed: len,
                got: words.len(),
            });
        };
        let mut out = [0; 4];
        for (to, from) in out.iter_mut().zip(packet) {
            *to = *from;
        }
        Ok(Self { words: out })
    }

    /// The words of the packet, word 0 first. The length is the Table 4 size of the Message Type.
    pub fn words(&self) -> &[u32] {
        &self.words[..self.message_type().word_count()]
    }

    /// The Message Type of the packet (bits 31..28 of word 0).
    pub fn message_type(&self) -> MessageType {
        MessageType::from_bits((self.words[0] >> 28) as u8)
    }

    /// The typed message ([`Ump::parse`] of the words). A reserved Message Type or an undefined
    /// Status gives [`Ump::Unknown`]. Never panics.
    pub fn ump(&self) -> Ump {
        match Ump::parse(self.words()) {
            Ok((ump, _)) => ump,
            // The packet always holds the Table 4 size, so `parse` never gives Truncated.
            Err(_) => Ump::Unknown(UnknownPacket::from_packet(self.words())),
        }
    }
}

impl Ump {
    /// The words of the message as a [`Packet`] ([`Ump::write`]). Never panics.
    pub fn to_packet(&self) -> Packet {
        let mut words = [0; 4];
        // Every message fits in 4 words, so the write never gives BufferTooSmall.
        let _ = self.write(&mut words);
        Packet { words }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    #[cfg(feature = "utility")]
    use crate::utility::Noop;
    use crate::utility::{JrClock, JrTimestamp};
    use crate::UmpWords;
    use proptest::prelude::*;

    #[test]
    fn ump_parse_uses_table_4_size_for_every_message_type() {
        // 2.1.4: every Message Type, reserved values included, has the Table 4 size. Each word 0
        // has a Status that its Message Type does not define, so the packet stays Unknown when
        // more message families are added.
        let word0: [u32; 16] = [
            0x00F0_0000, // Utility, Status 0xF
            0x10F4_0000, // System, status 0xF4 (reserved)
            0x2000_0000, // MIDI 1.0 Channel Voice, opcode 0x0
            0x30F0_0000, // Data 64, Status 0xF
            0x4070_0000, // MIDI 2.0 Channel Voice, opcode 0x7
            0x50F0_0000, // Data 128, Status 0xF
            0x6000_0000, // reserved
            0x7000_0000, // reserved
            0x8000_0000, // reserved
            0x9000_0000, // reserved
            0xA000_0000, // reserved
            0xB000_0000, // reserved
            0xC000_0000, // reserved
            0xD000_FF00, // Flex Data, Status Bank 0xFF (reserved)
            0xE000_0000, // reserved
            0xF3FF_0000, // UMP Stream, Status 0x3FF
        ];
        for (mt, w0) in word0.iter().enumerate() {
            let words = [*w0, 0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF];
            let len = MessageType::from_bits(mt as u8).word_count();
            let (ump, used) = Ump::parse(&words).unwrap();
            assert_eq!(used, len, "MT {mt:#x}");
            assert_eq!(ump.word_count(), len);
            let Ump::Unknown(p) = ump else {
                panic!("MT {mt:#x} must be Unknown")
            };
            assert_eq!(p.words(), &words[..len]);
            assert_eq!(p.message_type().into_bits() as usize, mt);
        }
    }

    #[test]
    fn ump_parse_reports_truncated_input() {
        assert_eq!(
            Ump::parse(&[]),
            Err(UmpError::Truncated { needed: 1, got: 0 })
        );
        for mt in 0x0..=0xFu32 {
            let len = MessageType::from_bits(mt as u8).word_count();
            let words = [mt << 28; 4];
            for got in 1..len {
                assert_eq!(
                    Ump::parse(&words[..got]),
                    Err(UmpError::Truncated { needed: len, got }),
                    "MT {mt:#x}"
                );
            }
        }
    }

    #[cfg(feature = "utility")]
    #[test]
    fn ump_parse_reads_utility_messages() {
        assert_eq!(
            Ump::parse(&[0x0010_ABCD]),
            Ok((
                Ump::Utility(UtilityMessage::JrClock(JrClock::new().with_time(0xABCD))),
                1
            ))
        );
        assert_eq!(
            Ump::parse(&[0x0000_0000]),
            Ok((Ump::Utility(UtilityMessage::Noop(Noop::new())), 1))
        );
    }

    #[cfg(feature = "system-common")]
    #[test]
    fn ump_parse_reads_system_messages() {
        // MT 0x1, Table 27: Timing Clock on Group 4 (index 3); a reserved status stays Unknown.
        let (ump, used) = Ump::parse(&[0x13F8_0000]).unwrap();
        assert_eq!(used, 1);
        assert!(matches!(
            ump,
            Ump::System(crate::system::SystemMessage::TimingClock(_))
        ));
        let mut out = [0u32; 1];
        assert_eq!(ump.write(&mut out), Ok(1));
        assert_eq!(out, [0x13F8_0000]);
        assert!(matches!(
            Ump::parse(&[0x10F4_0000]),
            Ok((Ump::Unknown(_), 1))
        ));
    }

    #[cfg(all(feature = "channel-voice1", feature = "channel-voice2"))]
    #[test]
    fn ump_parse_reads_channel_voice_messages() {
        // MT 0x2 (Table 28) is one word; MT 0x4 (Table 30) is two words.
        let (m1, used1) = Ump::parse(&[0x2090_3C64, 0xFFFF_FFFF]).unwrap();
        assert_eq!(used1, 1);
        assert!(matches!(m1, Ump::Midi1ChannelVoice(_)));
        let (m2, used2) = Ump::parse(&[0x4090_3C00, 0xC000_0000, 0x1]).unwrap();
        assert_eq!(used2, 2);
        assert!(matches!(m2, Ump::Midi2ChannelVoice(_)));
        let mut out = [0u32; 2];
        assert_eq!(m2.write(&mut out), Ok(2));
        assert_eq!(out, [0x4090_3C00, 0xC000_0000]);
        assert_eq!(
            m2.write(&mut out[..1]),
            Err(UmpError::BufferTooSmall { needed: 2, got: 1 })
        );
    }

    #[test]
    fn ump_parse_keeps_undefined_status_as_unknown() {
        // Utility Status 0x5..0xF is not defined in Table 26.
        for status in 0x5..=0xFu32 {
            let w = 0x0F00_1234 | (status << 20);
            let (ump, used) = Ump::parse(&[w]).unwrap();
            assert_eq!(used, 1);
            let Ump::Unknown(p) = ump else {
                panic!("Status {status:#x} must be Unknown")
            };
            assert_eq!(p.words(), &[w]);
        }
    }

    #[cfg(feature = "utility")]
    #[test]
    fn ump_parse_reads_status_at_message_type_position() {
        // 2.1.2, Figure 3: Utility Status is bits 23..20. Bits 19..16 are not the Status.
        assert!(matches!(
            Ump::parse(&[0x0021_0005]),
            Ok((Ump::Utility(UtilityMessage::JrTimestamp(m)), 1)) if m.timestamp() == 5
        ));
        assert!(matches!(
            Ump::parse(&[0x0012_0000]),
            Ok((Ump::Utility(UtilityMessage::JrClock(_)), 1))
        ));
    }

    #[cfg(feature = "full")]
    #[test]
    fn ump_parse_reads_status_of_each_message_type() {
        // REQ-UMP-010: SysEx7 Status bits 23..20, Data 128 Status bits 23..20, Flex Data Status
        // Bank bits 15..8 and Status bits 7..0, UMP Stream Status bits 25..16 (10 bits).
        assert!(matches!(
            Ump::parse(&[0x3016_0102, 0x0304_0506]),
            Ok((Ump::SysEx7(p), 2)) if p.status() == crate::sysex7::SysEx7Status::Start
        ));
        assert!(matches!(
            Ump::parse(&[0x5080_0012, 0x0001_0001, 0x0043_0000, 0]),
            Ok((
                Ump::Data128(crate::data128::Data128Message::MdsHeader(_)),
                4
            ))
        ));
        assert!(matches!(
            Ump::parse(&[0x5090_0000, 0, 0, 0]),
            Ok((
                Ump::Data128(crate::data128::Data128Message::MdsPayload(_)),
                4
            ))
        ));
        assert!(matches!(
            Ump::parse(&[0xD010_0001, 0x0402_0800, 0, 0]),
            Ok((Ump::FlexData(FlexDataMessage::SetTimeSignature(_)), 4))
        ));
        assert!(matches!(
            Ump::parse(&[0xD010_0101, 0x4100_0000, 0, 0]),
            Ok((Ump::FlexData(FlexDataMessage::Text(_)), 4))
        ));
        assert!(matches!(
            Ump::parse(&[0xF021_0000, 0, 0, 0]),
            Ok((Ump::UmpStream(UmpStreamMessage::EndOfClip(_)), 4))
        ));
        // Status 0x221 has bit 9 set: not End of Clip.
        assert!(!matches!(
            Ump::parse(&[0xF221_0000, 0, 0, 0]),
            Ok((Ump::UmpStream(UmpStreamMessage::EndOfClip(_)), 4))
        ));
    }

    #[test]
    fn ump_write_keeps_unknown_words_unchanged() {
        // A reserved Message Type (0xB, 3 words) is written only as the received words.
        let words = [0xB123_4567, 0x89AB_CDEF, 0xFFFF_FFFF];
        let (ump, used) = Ump::parse(&words).unwrap();
        assert_eq!(used, 3);
        let mut out = [0u32; 4];
        assert_eq!(ump.write(&mut out), Ok(3));
        assert_eq!(&out[..3], &words);
        assert_eq!(out[3], 0);
    }

    #[test]
    fn ump_write_gives_one_complete_packet() {
        let ump = Ump::Utility(UtilityMessage::JrTimestamp(
            JrTimestamp::new().with_timestamp(0x1234),
        ));
        let mut out = [0u32; 1];
        assert_eq!(ump.write(&mut out), Ok(1));
        assert_eq!(out, [0x0020_1234]);
        assert_eq!(
            ump.write(&mut []),
            Err(UmpError::BufferTooSmall { needed: 1, got: 0 })
        );
        let (unknown, _) = Ump::parse(&[0xF000_0000, 1, 2, 3]).unwrap();
        assert_eq!(
            unknown.write(&mut out),
            Err(UmpError::BufferTooSmall { needed: 4, got: 1 })
        );
    }

    #[test]
    fn ump_parse_walks_a_stream_of_packets() {
        // JR Timestamp + MIDI 2.0 Note On (2 words, not decoded yet) + reserved MT 0x6 + NOOP.
        let stream = [
            0x0020_0100,
            0x4090_3C00,
            0x8000_0000,
            0x6000_0001,
            0x0000_0000,
        ];
        let mut rest = &stream[..];
        let mut sizes = [0usize; 4];
        let mut n = 0;
        while !rest.is_empty() {
            let (ump, used) = Ump::parse(rest).unwrap();
            assert_eq!(ump.word_count(), used);
            sizes[n] = used;
            n += 1;
            rest = &rest[used..];
        }
        assert_eq!(&sizes[..n], &[1, 2, 1, 1]);
    }

    /// Message Type 0x3 with a defined status is a System Exclusive (7-bit) packet (7.7).
    #[cfg(feature = "sysex7")]
    #[test]
    fn ump_parses_and_writes_sysex7() {
        let words = [0x3004_4312, 0x0007_0000];
        let (ump, used) = Ump::parse(&words).unwrap();
        assert_eq!(used, 2);
        let Ump::SysEx7(p) = ump else {
            panic!("MT 0x3 status 0x0 must be SysEx7")
        };
        assert_eq!(&*p.data(), &[0x43, 0x12, 0x00, 0x07]);
        let mut out = [0u32; 2];
        assert_eq!(ump.write(&mut out), Ok(2));
        assert_eq!(out, words);
    }

    /// Message Type 0xF with a defined Status is a UMP Stream message; it has no Group (7.1).
    #[cfg(feature = "ump-stream")]
    #[test]
    fn ump_parses_and_writes_ump_stream() {
        let words = [0xF000_0101, 0x0000_001F, 0, 0];
        let (ump, used) = Ump::parse(&words).unwrap();
        assert_eq!(used, 4);
        let Ump::UmpStream(UmpStreamMessage::EndpointDiscovery(m)) = ump else {
            panic!("MT 0xF Status 0x000 must be Endpoint Discovery")
        };
        assert!(m.endpoint_info() && m.stream_configuration());
        let mut out = [0u32; 4];
        assert_eq!(ump.write(&mut out), Ok(4));
        assert_eq!(out, words);
    }

    /// Message Type 0x5 with Status 0x0 to 0x3, 0x8 or 0x9 is a Data128 message (7.8, 7.9);
    /// another Status is unknown.
    #[cfg(feature = "data128")]
    #[test]
    fn ump_parses_and_writes_data128() {
        let words = [0x5105_0000, 0x4310_4C00, 0, 0];
        let (ump, used) = Ump::parse(&words).unwrap();
        assert_eq!((used, ump.word_count()), (4, 4));
        let Ump::Data128(Data128Message::SysEx8(m)) = ump else {
            panic!("MT 0x5 Status 0x0 must be System Exclusive 8")
        };
        assert_eq!(&*m.data(), &[0x00, 0x43, 0x10, 0x4C]);
        let mut out = [0u32; 4];
        assert_eq!(ump.write(&mut out), Ok(4));
        assert_eq!(out, words);
        assert!(matches!(
            Ump::parse(&[0x5040_0000, 1, 2, 3]),
            Ok((Ump::Unknown(_), 4))
        ));
    }

    /// Message Type 0xD with a defined Status Bank and Status is a Flex Data message (7.5); a
    /// reserved Status Bank is unknown.
    #[cfg(feature = "flex-data")]
    #[test]
    fn ump_parses_and_writes_flex_data() {
        let words = [0xD010_0000, 0x02FA_F080, 0, 0];
        let (ump, used) = Ump::parse(&words).unwrap();
        assert_eq!((used, ump.word_count()), (4, 4));
        let Ump::FlexData(FlexDataMessage::SetTempo(m)) = ump else {
            panic!("MT 0xD Bank 0x00 Status 0x00 must be Set Tempo")
        };
        assert_eq!(m.ten_ns_per_quarter_note(), 50_000_000);
        let mut out = [0u32; 4];
        assert_eq!(ump.write(&mut out), Ok(4));
        assert_eq!(out, words);
        let reserved = [0xD010_0300, 1, 2, 3];
        assert!(matches!(Ump::parse(&reserved), Ok((Ump::Unknown(_), 4))));
    }

    #[test]
    fn ump_round_trip_matches_typed_to_words() {
        let m = JrClock::new().with_time(7);
        let mut typed = [0u32; 1];
        m.to_words(&mut typed).unwrap();
        let mut via_ump = [0u32; 1];
        Ump::Utility(UtilityMessage::JrClock(m))
            .write(&mut via_ump)
            .unwrap();
        assert_eq!(typed, via_ump);
    }

    /// DEC-UMP-003: parse and write never panic on any slice or buffer length (REQ-UMP-006).
    /// A parsed packet uses the Table 4 size, and writing it gives back the received words,
    /// with reserved bits cleared for a decoded message (REQ-UMP-001, 004, 007).
    #[test]
    fn prop_ump_parse_and_write_never_panic() {
        proptest!(|(words in proptest::collection::vec(any::<u32>(), 0..6), out_len in 0usize..6)| {
            if let Ok((ump, used)) = Ump::parse(&words) {
                let mt = MessageType::from_bits((words[0] >> 28) as u8);
                prop_assert_eq!(used, mt.word_count());
                prop_assert_eq!(ump.word_count(), used);
                let mut out = [0u32; 6];
                match ump.write(&mut out[..out_len]) {
                    Ok(n) => {
                        prop_assert_eq!(n, used);
                        if let Ump::Unknown(_) = ump {
                            prop_assert_eq!(&out[..n], &words[..n]);
                        }
                    }
                    Err(e) => prop_assert_eq!(
                        e,
                        UmpError::BufferTooSmall { needed: used, got: out_len }
                    ),
                }
            } else {
                prop_assert!(words.is_empty()
                    || words.len() < MessageType::from_bits((words[0] >> 28) as u8).word_count());
            }
        });
    }

    /// A reserved Message Type is kept as an unknown packet and written unchanged (REQ-UMP-007).
    #[test]
    fn prop_ump_unknown_keeps_reserved_message_type_words() {
        proptest!(|(mt in prop::sample::select(&[0x6u32, 0x7, 0x8, 0x9, 0xA, 0xB, 0xC, 0xE][..]), words in any::<[u32; 4]>())| {
                let mut words = words;
                words[0] = (mt << 28) | (words[0] & 0x0FFF_FFFF);
                let (ump, used) = Ump::parse(&words).unwrap();
                let Ump::Unknown(p) = ump else { panic!("reserved MT must be Unknown") };
                prop_assert_eq!(p.words(), &words[..used]);
                let mut out = [0u32; 4];
                prop_assert_eq!(ump.write(&mut out), Ok(used));
                prop_assert_eq!(&out[..used], &words[..used]);
        });
    }

    #[test]
    fn packet_takes_table_4_size_for_every_message_type() {
        // 2.1.4, DEC-018: the packet length is the Table 4 size; later words are not used.
        for mt in 0..16u32 {
            let words = [
                mt << 28 | 0x0012_3456,
                0xAAAA_AAAA,
                0xBBBB_BBBB,
                0xCCCC_CCCC,
                0xDDDD_DDDD,
            ];
            let len = MessageType::from_bits(mt as u8).word_count();
            let p = Packet::from_words(&words).unwrap();
            assert_eq!(p.words(), &words[..len], "MT {mt:#x}");
            assert_eq!(p.message_type().into_bits() as u32, mt);
            assert_eq!(
                Packet::from_words(&words[..len - 1]),
                Err(UmpError::Truncated {
                    needed: len,
                    got: len - 1
                })
            );
        }
        assert_eq!(
            Packet::from_words(&[]),
            Err(UmpError::Truncated { needed: 1, got: 0 })
        );
    }

    #[test]
    fn prop_packet_ump_equals_parse_and_round_trips() {
        // DEC-018: Packet::ump is Ump::parse of the same words; to_packet gives the words back.
        proptest!(|(w in proptest::array::uniform4(any::<u32>()))| {
            let p = Packet::from_words(&w).unwrap();
            let (ump, used) = Ump::parse(&w).unwrap();
            prop_assert_eq!(p.words().len(), used);
            prop_assert_eq!(p.ump(), ump);
            let mut out = [0u32; 4];
            let n = ump.write(&mut out).unwrap();
            let q = ump.to_packet();
            prop_assert_eq!(q.words(), &out[..n]);
        });
    }

    #[test]
    fn ump_parse_gives_unknown_for_message_types_not_decoded() {
        // DEC-017, 2.1.4: in every feature set, a valid packet of a family that this build does
        // not decode is Unknown with its raw words and Table 4 size; is_decoded says which.
        let packets: [&[u32]; 8] = [
            &[0x0010_0000],                    // Utility: JR Clock
            &[0x10F8_0000],                    // System: Timing Clock
            &[0x2090_3C40],                    // MIDI 1.0 Note On
            &[0x3002_7E7F, 0],                 // SysEx7 Complete, 2 bytes
            &[0x4090_3C00, 0x8000_0000],       // MIDI 2.0 Note On
            &[0x5001_0000, 0, 0, 0],           // SysEx8 Complete, 1 byte (stream 0)
            &[0xD010_0000, 0x0007_A120, 0, 0], // Flex Data: Set Tempo
            &[0xF000_0101, 0x0000_001F, 0, 0], // UMP Stream: Endpoint Discovery
        ];
        for words in packets {
            let mt = MessageType::from_bits((words[0] >> 28) as u8);
            let (ump, used) = Ump::parse(words).unwrap();
            assert_eq!(used, mt.word_count());
            match ump {
                Ump::Unknown(p) => {
                    assert!(!mt.is_decoded(), "{mt:?} is decoded but gave Unknown");
                    assert_eq!(p.words(), words);
                }
                _ => assert!(
                    mt.is_decoded(),
                    "{mt:?} is not decoded but gave a typed value"
                ),
            }
        }
        for reserved in [6u8, 7, 8, 9, 0xA, 0xB, 0xC, 0xE] {
            assert!(!MessageType::from_bits(reserved).is_decoded());
        }
    }
}
