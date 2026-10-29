//! MIDI Clip File writer (M2-116-U v1.0).
//!
//! - File Header "SMF2CLIP", then the Clip Configuration Header, then the Clip Sequence Data;
//!   only UMP messages after the File Header, big endian (sections 3, 4, Figure 2).
//! - Clip Configuration Header (6): Set Profile On messages first, with no Delta Clockstamp;
//!   then Delta Clockstamp 0 and exactly one DCTPQ; then at most one Set Tempo and one Set Time
//!   Signature, which may share that Delta Clockstamp; every other message has its own Delta
//!   Clockstamp.
//! - Set Profile On uses the Broadcast MUID for Source and Destination (6.2, M2-101-UM 7.8).
//! - Clip Sequence Data (7): Start of Clip first and End of Clip last, each with a preceding
//!   Delta Clockstamp; Set Tempo and Set Time Signature should follow Start of Clip at its
//!   time; no Property Exchange messages (6.4, 7.2).
//! - Delta Clockstamps: messages at one time may share one; a gap longer than 1,048,575 ticks
//!   gets a Delta Clockstamp and a NOOP (M2-104-UM 7.2.3.2, M2-116 5).
//! - Set Tempo at 1/24 of a quarter note (M2-104-UM 7.5.3); Set Time Signature at a bar
//!   boundary (7.5.4); the UMPs of one Lyrics or Ruby message at one time (7.5.10, 7.5.12).
//!
//! The writer fills a caller buffer. A call that fails changes nothing. No alloc (DEC-002).

use crate::clip_format::{word_to_bytes, FILE_HEADER, MAX_DELTA_TICKS};
use zmidi_ump::{
    DeltaClockstamp, DeltaClockstampTpq, EndOfClip, FlexDataMessage, Group, SetTempo,
    SetTimeSignature, StartOfClip, SysEx7, SysEx7Packets, SysEx7Status, Ump, UmpStreamMessage,
    UmpWords, UtilityMessage,
};

/// Error of the clip writer (thiserror 2, no_std).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClipWriteError {
    /// The buffer cannot hold the bytes.
    #[error("output buffer too small")]
    BufferTooSmall,
    /// A TPQ of 0, a value above 0x7F in a Profile message, or a second Set Tempo or Set Time
    /// Signature in the Clip Configuration Header.
    #[error("invalid clip configuration")]
    InvalidConfig,
    /// The words are not one whole UMP.
    #[error("not one whole UMP")]
    InvalidMessage,
    /// A tick earlier than the last one (events are stored in presentation order).
    #[error("ticks must not decrease")]
    OutOfOrder,
    /// A Property Exchange message (6.4, 7.2).
    #[error("Property Exchange messages are not allowed in a MIDI Clip File")]
    PropertyExchange,
    /// A Delta Clockstamp, DCTPQ, Start of Clip or End of Clip: the writer writes those.
    #[error("the writer writes this message itself")]
    ReservedMessage,
    /// Set Tempo off the 1/24 quarter note grid (M2-104-UM 7.5.3).
    #[error("Set Tempo must be at 1/24 of a quarter note")]
    TempoTiming,
    /// Set Time Signature not at a bar boundary (M2-104-UM 7.5.4).
    #[error("Set Time Signature must be at a bar boundary")]
    BarTiming,
    /// A different tick while a multi-UMP Lyrics or Ruby message is open (7.5.10, 7.5.12).
    #[error("all UMPs of one Lyrics or Ruby message must have one time")]
    FlexTiming,
    /// The call is not allowed in the current phase.
    #[error("call not allowed in this phase")]
    WrongPhase,
}

/// The destination of a Set Profile On message (M2-101-UM Table 24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileDestination {
    /// One MIDI Channel, 0 to 15.
    Channel(u8),
    /// The whole Group (0x7E).
    Group,
    /// The whole Function Block (0x7F).
    FunctionBlock,
}

/// One Set Profile On message for the Clip Configuration Header (6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProfileOn {
    /// Profile ID, 5 bytes of 7-bit data.
    pub profile_id: [u8; 5],
    /// Destination.
    pub destination: ProfileDestination,
    /// Number of Channels Requested (14 bits); written as 0 for a Group or Function Block.
    pub channels: u16,
}

/// The Clip Configuration Header of a new clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipConfig<'a> {
    /// Ticks Per Quarter Note of the DCTPQ, 1 to 65535.
    pub ticks_per_quarter_note: u16,
    /// Set Profile On messages, written first.
    pub profiles: &'a [ProfileOn],
    /// Set Tempo, written after the DCTPQ.
    pub tempo: Option<SetTempo>,
    /// Set Time Signature, written after the Set Tempo.
    pub time_signature: Option<SetTimeSignature>,
    /// Group of the Set Profile On messages.
    pub group: Group,
}

/// Writer phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Configuration,
    Sequence,
    Ended,
}

/// The time signature in force: tick of the change, Numerator, Denominator (power of 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Signature {
    tick: u64,
    numerator: u8,
    denominator: u8,
}

/// MIDI Clip File writer into a caller buffer. No alloc.
#[derive(Debug)]
pub struct ClipWriter<'a> {
    /// The output buffer.
    buf: &'a mut [u8],
    /// Bytes written.
    len: usize,
    /// Tick time of the last Delta Clockstamp.
    last_tick: u64,
    /// Current phase.
    phase: Phase,
    /// Ticks Per Quarter Note.
    tpq: u16,
    /// The time signature in force, if any.
    signature: Option<Signature>,
    /// Tick of an open multi-UMP Lyrics or Ruby message.
    flex_open: Option<u64>,
}

impl<'a> ClipWriter<'a> {
    /// Writes the File Header and the Clip Configuration Header of `config` into `buf`.
    pub fn new(buf: &'a mut [u8], config: &ClipConfig<'_>) -> Result<Self, ClipWriteError> {
        if config.ticks_per_quarter_note == 0 {
            return Err(ClipWriteError::InvalidConfig);
        }
        let mut w = Self {
            buf,
            len: 0,
            last_tick: 0,
            phase: Phase::Configuration,
            tpq: config.ticks_per_quarter_note,
            signature: None,
            flex_open: None,
        };
        w.bytes(&FILE_HEADER)?;
        for profile in config.profiles {
            w.profile_on(config.group, profile)?;
        }
        w.put(&DeltaClockstamp::try_new(0).map_err(|_| ClipWriteError::InvalidConfig)?)?;
        let tpq = DeltaClockstampTpq::try_new(config.ticks_per_quarter_note)
            .map_err(|_| ClipWriteError::InvalidConfig)?;
        w.put(&tpq)?;
        if let Some(tempo) = &config.tempo {
            w.put(tempo)?;
        }
        if let Some(sig) = &config.time_signature {
            w.put(sig)?;
            w.signature = Some(signature(0, sig));
        }
        Ok(w)
    }

    /// Bytes written so far.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True before anything is written. Never true after [`ClipWriter::new`].
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Writes one more UMP in the Clip Configuration Header, with its own Delta Clockstamp 0.
    pub fn config_event(&mut self, words: &[u32]) -> Result<(), ClipWriteError> {
        if self.phase != Phase::Configuration {
            return Err(ClipWriteError::WrongPhase);
        }
        let ump = check_message(words)?;
        if matches!(
            ump,
            Ump::FlexData(FlexDataMessage::SetTempo(_) | FlexDataMessage::SetTimeSignature(_))
        ) {
            return Err(ClipWriteError::InvalidConfig);
        }
        self.atomic(|w| {
            w.put(&DeltaClockstamp::try_new(0).map_err(|_| ClipWriteError::InvalidConfig)?)?;
            w.ump(&ump)
        })
    }

    /// Writes the Delta Clockstamp and Start of Clip at `tick`, then `tempo` and
    /// `time_signature` at the same time, and begins the Clip Sequence Data.
    pub fn start(
        &mut self,
        tick: u64,
        tempo: Option<SetTempo>,
        time_signature: Option<SetTimeSignature>,
    ) -> Result<(), ClipWriteError> {
        if self.phase != Phase::Configuration {
            return Err(ClipWriteError::WrongPhase);
        }
        if tempo.is_some() {
            self.check_tempo(tick)?;
        }
        if time_signature.is_some() {
            self.check_bar(tick)?;
        }
        self.atomic(|w| {
            w.delta(tick, true)?;
            w.put(&StartOfClip::new())?;
            if let Some(tempo) = &tempo {
                w.put(tempo)?;
            }
            if let Some(sig) = &time_signature {
                w.put(sig)?;
                w.signature = Some(signature(tick, sig));
            }
            w.phase = Phase::Sequence;
            Ok(())
        })
    }

    /// Writes one UMP of the Clip Sequence Data at absolute `tick`.
    pub fn event(&mut self, tick: u64, words: &[u32]) -> Result<(), ClipWriteError> {
        if self.phase != Phase::Sequence {
            return Err(ClipWriteError::WrongPhase);
        }
        if tick < self.last_tick {
            return Err(ClipWriteError::OutOfOrder);
        }
        let ump = check_message(words)?;
        if self.flex_open.is_some_and(|t| t != tick) {
            return Err(ClipWriteError::FlexTiming);
        }
        let mut new_signature = None;
        let mut flex_open = self.flex_open;
        if let Ump::FlexData(m) = &ump {
            match m {
                FlexDataMessage::SetTempo(_) => self.check_tempo(tick)?,
                FlexDataMessage::SetTimeSignature(sig) => {
                    self.check_bar(tick)?;
                    new_signature = Some(signature(tick, sig));
                }
                _ => {}
            }
            // Lyrics (Status Bank 0x02, Status 0x01) and Ruby (0x02, 0x03): Format bits 23..22
            // are 1 Start and 3 End (7.5.1).
            let w0 = words[0];
            if (w0 >> 8) & 0xFF == 0x02 && matches!(w0 & 0xFF, 0x01 | 0x03) {
                match (w0 >> 22) & 0b11 {
                    1 => flex_open = Some(tick),
                    3 => flex_open = None,
                    _ => {}
                }
            }
        }
        self.atomic(|w| {
            w.delta(tick, false)?;
            w.ump(&ump)?;
            if new_signature.is_some() {
                w.signature = new_signature;
            }
            w.flex_open = flex_open;
            Ok(())
        })
    }

    /// Writes the Delta Clockstamp and End of Clip at `tick` and gives the total length. No
    /// call after it writes data.
    pub fn finish(&mut self, tick: u64) -> Result<usize, ClipWriteError> {
        if self.phase != Phase::Sequence {
            return Err(ClipWriteError::WrongPhase);
        }
        if tick < self.last_tick {
            return Err(ClipWriteError::OutOfOrder);
        }
        if self.flex_open.is_some_and(|t| t != tick) {
            return Err(ClipWriteError::FlexTiming);
        }
        self.atomic(|w| {
            w.delta(tick, true)?;
            w.put(&EndOfClip::new())?;
            w.phase = Phase::Ended;
            Ok(())
        })?;
        Ok(self.len)
    }

    /// Runs `f`; on an error, restores the length and the state.
    fn atomic(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<(), ClipWriteError>,
    ) -> Result<(), ClipWriteError> {
        let saved = (
            self.len,
            self.last_tick,
            self.phase,
            self.signature,
            self.flex_open,
        );
        let result = f(self);
        if result.is_err() {
            (
                self.len,
                self.last_tick,
                self.phase,
                self.signature,
                self.flex_open,
            ) = saved;
        }
        result
    }

    /// Set Tempo only at 1/24 of a quarter note (M2-104-UM 7.5.3).
    fn check_tempo(&self, tick: u64) -> Result<(), ClipWriteError> {
        if (u128::from(tick) * 24).is_multiple_of(u128::from(self.tpq)) {
            Ok(())
        } else {
            Err(ClipWriteError::TempoTiming)
        }
    }

    /// Set Time Signature only at a bar boundary of the time signature in force (7.5.4). A bar
    /// is Numerator × TPQ × 4 / 2^Denominator ticks. With no time signature in force, or a
    /// non-standard Denominator (0), there is no bar grid to check.
    fn check_bar(&self, tick: u64) -> Result<(), ClipWriteError> {
        let Some(sig) = self
            .signature
            .filter(|s| s.denominator != 0 && s.denominator < 32)
        else {
            return Ok(());
        };
        // (tick - change) / bar is whole: (tick - change) × 2^den is a multiple of num × tpq × 4.
        let elapsed = u128::from(tick - sig.tick) << sig.denominator;
        let bar = u128::from(sig.numerator) * u128::from(self.tpq) * 4;
        if elapsed.is_multiple_of(bar) {
            Ok(())
        } else {
            Err(ClipWriteError::BarTiming)
        }
    }

    /// Delta Clockstamps up to `tick`: NOOP-separated full steps for long gaps (DC-004). With
    /// `always`, a Delta Clockstamp is written even for no gap (Start and End of Clip).
    fn delta(&mut self, tick: u64, always: bool) -> Result<(), ClipWriteError> {
        let mut gap = tick - self.last_tick;
        while gap > u64::from(MAX_DELTA_TICKS) {
            self.put_word(0x0040_0000 | MAX_DELTA_TICKS)?;
            self.put_word(0x0000_0000)?; // NOOP
            gap -= u64::from(MAX_DELTA_TICKS);
        }
        if gap > 0 || always {
            self.put_word(0x0040_0000 | gap as u32)?;
        }
        self.last_tick = tick;
        Ok(())
    }

    /// One Set Profile On message as SysEx7 UMPs (M2-101-UM Table 24).
    fn profile_on(&mut self, group: Group, p: &ProfileOn) -> Result<(), ClipWriteError> {
        let (dest, channels) = match p.destination {
            ProfileDestination::Channel(c) if c < 16 => (c, p.channels),
            ProfileDestination::Channel(_) => return Err(ClipWriteError::InvalidConfig),
            ProfileDestination::Group => (0x7E, 0),
            ProfileDestination::FunctionBlock => (0x7F, 0),
        };
        if channels > 0x3FFF {
            return Err(ClipWriteError::InvalidConfig);
        }
        // 7E, Destination, 0D, 22, Version 02, Source and Destination Broadcast MUIDs
        // (7F 7F 7F 7F each), Profile ID (5 bytes), Number of Channels Requested (LSB first).
        let mut payload = [0x7Fu8; 20];
        payload[..5].copy_from_slice(&[0x7E, dest, 0x0D, 0x22, 0x02]);
        payload[13..18].copy_from_slice(&p.profile_id);
        payload[18] = (channels & 0x7F) as u8;
        payload[19] = (channels >> 7) as u8;
        let packets =
            SysEx7Packets::try_new(group, &payload).map_err(|_| ClipWriteError::InvalidConfig)?;
        for packet in packets {
            self.put(&packet)?;
        }
        Ok(())
    }

    fn ump(&mut self, ump: &Ump) -> Result<(), ClipWriteError> {
        let mut words = [0u32; 4];
        let n = ump
            .write(&mut words)
            .map_err(|_| ClipWriteError::InvalidMessage)?;
        words[..n].iter().try_for_each(|&w| self.put_word(w))
    }

    fn put<T: UmpWords>(&mut self, message: &T) -> Result<(), ClipWriteError> {
        let mut words = [0u32; 4];
        let n = message
            .to_words(&mut words)
            .map_err(|_| ClipWriteError::InvalidMessage)?;
        words[..n].iter().try_for_each(|&w| self.put_word(w))
    }

    fn put_word(&mut self, word: u32) -> Result<(), ClipWriteError> {
        self.bytes(&word_to_bytes(word))
    }

    fn bytes(&mut self, bytes: &[u8]) -> Result<(), ClipWriteError> {
        let end = self.len + bytes.len();
        self.buf
            .get_mut(self.len..end)
            .ok_or(ClipWriteError::BufferTooSmall)?
            .copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

/// The signature record of a Set Time Signature at `tick`.
fn signature(tick: u64, sig: &SetTimeSignature) -> Signature {
    Signature {
        tick,
        numerator: sig.numerator(),
        denominator: sig.denominator(),
    }
}

/// Parses one whole UMP and rejects what a clip writer caller may not give.
fn check_message(words: &[u32]) -> Result<Ump, ClipWriteError> {
    let (ump, n) = Ump::parse(words).map_err(|_| ClipWriteError::InvalidMessage)?;
    if n != words.len() {
        return Err(ClipWriteError::InvalidMessage);
    }
    match ump {
        Ump::Utility(
            UtilityMessage::DeltaClockstamp(_) | UtilityMessage::DeltaClockstampTpq(_),
        )
        | Ump::UmpStream(UmpStreamMessage::StartOfClip(_) | UmpStreamMessage::EndOfClip(_)) => {
            Err(ClipWriteError::ReservedMessage)
        }
        Ump::SysEx7(p) if is_property_exchange(&p) => Err(ClipWriteError::PropertyExchange),
        _ => Ok(ump),
    }
}

/// A SysEx7 Start or Complete packet of a MIDI-CI Property Exchange message: Universal
/// System Exclusive 0x7E, any Device ID, Sub-ID#1 0x0D, Sub-ID#2 0x30 to 0x3F (M2-101-UM).
fn is_property_exchange(p: &SysEx7) -> bool {
    let data = p.data();
    matches!(p.status(), SysEx7Status::Start | SysEx7Status::Complete)
        && data.len() >= 4
        && data[0] == 0x7E
        && data[2] == 0x0D
        && (0x30..=0x3F).contains(&data[3])
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::clip_format::word_from_bytes;
    use crate::clip_reader::{ClipReader, ClipSection};
    use proptest::prelude::*;
    use std::vec::Vec;

    fn g0() -> Group {
        Group::from_bits(0)
    }

    fn config(tpq: u16) -> ClipConfig<'static> {
        ClipConfig {
            ticks_per_quarter_note: tpq,
            profiles: &[],
            tempo: None,
            time_signature: None,
            group: g0(),
        }
    }

    /// The words after the File Header.
    fn words(bytes: &[u8]) -> Vec<u32> {
        assert_eq!(bytes[..8], FILE_HEADER);
        bytes[8..]
            .chunks(4)
            .map(|c| word_from_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    const NOTE_ON: [u32; 2] = [0x4090_3C00, 0xFFFF_0000];
    const DCS0: u32 = 0x0040_0000;
    const START: [u32; 4] = [0xF020_0000, 0, 0, 0];
    const END: [u32; 4] = [0xF021_0000, 0, 0, 0];

    fn tempo() -> SetTempo {
        SetTempo::new(g0(), 50_000_000)
    }

    fn four_four() -> SetTimeSignature {
        SetTimeSignature::try_new(g0(), 4, 2, 8).unwrap()
    }

    #[test]
    fn writer_writes_minimal_clip() {
        let mut buf = [0u8; 128];
        let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
        w.start(0, None, None).unwrap();
        let n = w.finish(0).unwrap();
        // FILE-002/003, CFG-002, MARK-001/002: header, DC 0 + DCTPQ, DC + Start, DC + End.
        let mut expected = [DCS0, 0x0030_0060, DCS0].to_vec();
        expected.extend_from_slice(&START);
        expected.push(DCS0);
        expected.extend_from_slice(&END);
        assert_eq!(words(&buf[..n]), expected);
    }

    #[test]
    fn writer_puts_profiles_first_and_tempo_after_dctpq() {
        let profiles = [ProfileOn {
            profile_id: [0x7E, 0x00, 0x01, 0x02, 0x03],
            destination: ProfileDestination::Group,
            channels: 5,
        }];
        let cfg = ClipConfig {
            profiles: &profiles,
            tempo: Some(tempo()),
            time_signature: Some(four_four()),
            ..config(96)
        };
        let mut buf = [0u8; 256];
        let w = ClipWriter::new(&mut buf, &cfg).unwrap();
        let n = w.len();
        let ws = words(&buf[..n]);
        // CFG-001, CFG-008: Set Profile On first, no Delta Clockstamp, Broadcast MUIDs, Number
        // of Channels 0 for a Group: 7E 7E 0D 22 02 | 7F×4 | 7F×4 | id×5 | 00 00 (20 bytes).
        assert_eq!(ws[0..2], [0x3016_7E7E, 0x0D22_027F]);
        assert_eq!(ws[2..4], [0x3026_7F7F, 0x7F7F_7F7F]);
        assert_eq!(ws[4..6], [0x3026_7F7E, 0x0001_0203]);
        assert_eq!(ws[6..8], [0x3032_0000, 0]);
        // CFG-002, CFG-005, CFG-006: DC 0, DCTPQ, Set Tempo and Set Time Signature share it.
        assert_eq!(ws[8..10], [DCS0, 0x0030_0060]);
        assert_eq!(ws[10..14], [0xD010_0000, 50_000_000, 0, 0]);
        assert_eq!(ws[14..18], [0xD010_0001, 0x0402_0800, 0, 0]);
        assert_eq!(ws.len(), 18);
    }

    #[test]
    fn writer_profile_destinations() {
        let id = [0x7E, 0x00, 0x01, 0x02, 0x03];
        let mut buf = [0u8; 256];
        // CFG-008: a Channel destination with its Number of Channels (0x81: LSB first, 01 01), and
        // a Function Block destination with 0.
        let profiles = [
            ProfileOn {
                profile_id: id,
                destination: ProfileDestination::Channel(3),
                channels: 0x85,
            },
            ProfileOn {
                profile_id: id,
                destination: ProfileDestination::FunctionBlock,
                channels: 9,
            },
        ];
        let cfg = ClipConfig {
            profiles: &profiles,
            ..config(96)
        };
        let w = ClipWriter::new(&mut buf, &cfg).unwrap();
        let n = w.len();
        let ws = words(&buf[..n]);
        assert_eq!(ws[0..2], [0x3016_7E03, 0x0D22_027F]);
        // 0x85 channels, LSB first: 05 01.
        assert_eq!(ws[6..8], [0x3032_0501, 0]);
        assert_eq!(ws[8..10], [0x3016_7E7F, 0x0D22_027F]);
        assert_eq!(ws[14..16], [0x3032_0000, 0]);
        let bad = [ProfileOn {
            profile_id: id,
            destination: ProfileDestination::Channel(16),
            channels: 1,
        }];
        let cfg = ClipConfig {
            profiles: &bad,
            ..config(96)
        };
        assert_eq!(
            ClipWriter::new(&mut buf, &cfg).err(),
            Some(ClipWriteError::InvalidConfig)
        );
    }

    #[test]
    fn writer_gives_config_events_own_delta_clockstamp() {
        let mut buf = [0u8; 128];
        let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
        w.config_event(&NOTE_ON).unwrap();
        let mut t = [0u32; 4];
        tempo().to_words(&mut t).unwrap();
        assert_eq!(w.config_event(&t), Err(ClipWriteError::InvalidConfig));
        four_four().to_words(&mut t).unwrap();
        assert_eq!(w.config_event(&t), Err(ClipWriteError::InvalidConfig));
        // FILE-006: no Property Exchange message in the Clip Configuration Header either.
        assert_eq!(
            w.config_event(&[0x3006_7E7F, 0x0D34_0201]),
            Err(ClipWriteError::PropertyExchange)
        );
        // CFG-002: exactly one DCTPQ; the caller can not add a second one.
        assert_eq!(
            w.config_event(&[0x0030_0060]),
            Err(ClipWriteError::ReservedMessage)
        );
        let n = w.len();
        assert_eq!(words(&buf[..n])[2..], [DCS0, NOTE_ON[0], NOTE_ON[1]]);
    }

    #[test]
    fn writer_shares_delta_clockstamp_at_same_tick() {
        let mut buf = [0u8; 256];
        let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
        w.start(0, None, None).unwrap();
        w.event(0, &NOTE_ON).unwrap(); // shares the Start of Clip Delta Clockstamp
        w.event(96, &NOTE_ON).unwrap();
        w.event(96, &NOTE_ON).unwrap();
        let n = w.finish(96).unwrap();
        let ws = words(&buf[..n]);
        let seq = &ws[7..];
        assert_eq!(
            seq,
            [
                NOTE_ON[0],
                NOTE_ON[1],
                0x0040_0060,
                NOTE_ON[0],
                NOTE_ON[1],
                NOTE_ON[0],
                NOTE_ON[1],
                DCS0,
                END[0],
                0,
                0,
                0
            ]
        );
    }

    #[test]
    fn writer_splits_long_gaps_with_noop() {
        let mut buf = [0u8; 256];
        let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
        w.start(0, None, None).unwrap();
        w.event(2 * 0xF_FFFF + 5, &NOTE_ON).unwrap();
        let n = w.len();
        let ws = words(&buf[..n]);
        assert_eq!(
            ws[7..],
            [
                0x004F_FFFF,
                0,
                0x004F_FFFF,
                0,
                0x0040_0005,
                NOTE_ON[0],
                NOTE_ON[1]
            ]
        );
        // DC-004 at the limits: 0xFFFFF ticks is one Delta Clockstamp, 0x100000 needs a NOOP.
        for (gap, tail) in [
            (0xF_FFFF, &[0x004F_FFFF][..]),
            (0x10_0000, &[0x004F_FFFF, 0, 0x0040_0001][..]),
        ] {
            let mut buf = [0u8; 256];
            let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
            w.start(0, None, None).unwrap();
            w.event(gap, &NOTE_ON).unwrap();
            let n = w.len();
            let mut expected = tail.to_vec();
            expected.extend_from_slice(&NOTE_ON);
            assert_eq!(words(&buf[..n])[7..], expected, "gap {gap:#X}");
        }
    }

    #[test]
    fn writer_rejects_out_of_order_and_phase_errors() {
        let mut buf = [0u8; 256];
        let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
        assert_eq!(w.event(0, &NOTE_ON), Err(ClipWriteError::WrongPhase));
        assert_eq!(w.finish(0), Err(ClipWriteError::WrongPhase));
        w.start(10, None, None).unwrap();
        assert_eq!(w.event(9, &NOTE_ON), Err(ClipWriteError::OutOfOrder));
        assert_eq!(w.config_event(&NOTE_ON), Err(ClipWriteError::WrongPhase));
        let n = w.finish(10).unwrap();
        // FILE-005: nothing after End of Clip.
        assert_eq!(w.event(10, &NOTE_ON), Err(ClipWriteError::WrongPhase));
        assert_eq!(w.finish(10), Err(ClipWriteError::WrongPhase));
        assert_eq!(w.len(), n);
        assert_eq!(
            ClipWriter::new(&mut [0u8; 64], &config(0)).err(),
            Some(ClipWriteError::InvalidConfig)
        );
        // DC-003: End of Clip before the last event is out of order, and changes nothing.
        let mut buf = [0u8; 256];
        let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
        w.start(0, None, None).unwrap();
        w.event(20, &NOTE_ON).unwrap();
        let n = w.len();
        assert_eq!(w.finish(19), Err(ClipWriteError::OutOfOrder));
        assert_eq!(w.len(), n);
        assert!(w.finish(20).is_ok());
    }

    #[test]
    fn writer_rejects_property_exchange_and_reserved_messages() {
        let mut buf = [0u8; 256];
        let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
        w.start(0, None, None).unwrap();
        // Get Property Data inquiry (Sub-ID#2 0x34), SysEx7 Start.
        assert_eq!(
            w.event(0, &[0x3016_7E7F, 0x0D34_0201, 0]),
            Err(ClipWriteError::InvalidMessage)
        );
        assert_eq!(
            w.event(0, &[0x3016_7E7F, 0x0D34_0201]),
            Err(ClipWriteError::PropertyExchange)
        );
        // A Complete packet at both ends of the range 0x30 to 0x3F.
        for sub2 in [0x30, 0x3F] {
            assert_eq!(
                w.event(0, &[0x3006_7E7F, 0x0D00_0201 | sub2 << 16]),
                Err(ClipWriteError::PropertyExchange)
            );
        }
        // Other MIDI-CI (Set Profile On, 0x22) is accepted.
        w.event(0, &[0x3016_7E7F, 0x0D22_0201]).unwrap();
        w.event(0, &[0x3033_7F7F, 0x7F00_0000]).unwrap();
        for reserved in [&[DCS0][..], &[0x0030_0060], &START, &END] {
            assert_eq!(w.event(0, reserved), Err(ClipWriteError::ReservedMessage));
        }
    }

    #[test]
    fn writer_checks_tempo_grid() {
        let mut buf = [0u8; 256];
        let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
        w.start(0, Some(tempo()), None).unwrap();
        let mut t = [0u32; 4];
        tempo().to_words(&mut t).unwrap();
        // 96 / 24 = 4 ticks per MIDI Clock.
        assert_eq!(w.event(6, &t), Err(ClipWriteError::TempoTiming));
        w.event(8, &t).unwrap();
        // start() checks it too.
        let mut buf = [0u8; 256];
        let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
        assert_eq!(
            w.start(3, Some(tempo()), None),
            Err(ClipWriteError::TempoTiming)
        );
    }

    #[test]
    fn writer_checks_bar_boundaries() {
        let cfg = ClipConfig {
            time_signature: Some(four_four()),
            ..config(96)
        };
        let mut buf = [0u8; 256];
        let mut w = ClipWriter::new(&mut buf, &cfg).unwrap();
        w.start(0, None, None).unwrap();
        let mut sig = [0u32; 4];
        SetTimeSignature::try_new(g0(), 3, 3, 8)
            .unwrap()
            .to_words(&mut sig)
            .unwrap();
        // 4/4 at 96 TPQ: a bar is 384 ticks.
        assert_eq!(w.event(192, &sig), Err(ClipWriteError::BarTiming));
        w.event(384, &sig).unwrap();
        // 3/8 from tick 384: a bar is 144 ticks.
        assert_eq!(w.event(500, &sig), Err(ClipWriteError::BarTiming));
        w.event(528, &sig).unwrap();
        // start() checks a time signature at the Start of Clip too.
        let three_eight = SetTimeSignature::try_new(g0(), 3, 3, 8).unwrap();
        let mut buf = [0u8; 256];
        let mut w = ClipWriter::new(&mut buf, &cfg).unwrap();
        assert_eq!(
            w.start(100, None, Some(three_eight)),
            Err(ClipWriteError::BarTiming)
        );
        w.start(384, None, Some(three_eight)).unwrap();
    }

    #[test]
    fn writer_keeps_lyrics_umps_at_one_time() {
        let mut buf = [0u8; 256];
        let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
        w.start(0, None, None).unwrap();
        // Lyrics (Bank 0x02, Status 0x01) Start, Continue, End on Group 0, Address 1.
        let start = [0xD050_0201, 0x4142_4344, 0, 0];
        let cont = [0xD090_0201, 0x4546_4748, 0, 0];
        let end = [0xD0D0_0201, 0x4900_0000, 0, 0];
        w.event(10, &start).unwrap();
        assert_eq!(w.event(11, &cont), Err(ClipWriteError::FlexTiming));
        assert_eq!(w.finish(11), Err(ClipWriteError::FlexTiming));
        w.event(10, &cont).unwrap();
        w.event(10, &end).unwrap();
        w.event(11, &NOTE_ON).unwrap();
        // Ruby (Status 0x03) has the same rule.
        let ruby = |form: u32| [0xD000_0203 | form << 22, 0x4142_4344, 0, 0];
        w.event(20, &ruby(1)).unwrap();
        assert_eq!(w.event(21, &ruby(3)), Err(ClipWriteError::FlexTiming));
        w.event(20, &ruby(3)).unwrap();
        w.event(21, &NOTE_ON).unwrap();
    }

    #[test]
    fn writer_failure_changes_nothing() {
        // Header 8 + DC/DCTPQ 8 + DC/Start of Clip 20 = 36 bytes; 8 bytes free.
        let mut buf = [0u8; 44];
        let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
        w.start(0, None, None).unwrap();
        assert_eq!(w.len(), 36);
        // The Delta Clockstamp fits, the 64-bit note does not: nothing is kept.
        assert_eq!(w.event(5, &NOTE_ON), Err(ClipWriteError::BufferTooSmall));
        assert_eq!(w.len(), 36);
        // The tick was not taken either: a 32-bit message at tick 5 still gets its DC.
        w.event(5, &[0x2090_3C40]).unwrap();
        assert_eq!(words(&buf[..44])[7..], [0x0040_0005, 0x2090_3C40]);
        assert_eq!(
            ClipWriter::new(&mut [0u8; 8], &config(96)).err(),
            Some(ClipWriteError::BufferTooSmall)
        );
    }

    #[test]
    fn writer_output_reads_back() {
        let cfg = ClipConfig {
            tempo: Some(tempo()),
            ..config(480)
        };
        let mut buf = [0u8; 512];
        let mut w = ClipWriter::new(&mut buf, &cfg).unwrap();
        w.start(0, Some(tempo()), Some(four_four())).unwrap();
        w.event(480, &NOTE_ON).unwrap();
        let n = w.finish(960).unwrap();
        // SEQ-001, SEQ-002: Start of Clip, then Set Tempo and Set Time Signature with no Delta
        // Clockstamp of their own.
        let mut t = [0u32; 4];
        let mut seq = std::vec![DCS0];
        seq.extend_from_slice(&START);
        tempo().to_words(&mut t).unwrap();
        seq.extend_from_slice(&t);
        four_four().to_words(&mut t).unwrap();
        seq.extend_from_slice(&t);
        seq.push(0x0040_01E0);
        seq.extend_from_slice(&NOTE_ON);
        seq.push(0x0040_01E0);
        seq.extend_from_slice(&END);
        assert_eq!(words(&buf[..n])[6..], seq);
        let events: Vec<_> = ClipReader::new(&buf[..n])
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let summary: Vec<_> = events.iter().map(|e| (e.section, e.ticks)).collect();
        use ClipSection::*;
        assert_eq!(
            summary,
            [
                (Configuration, 0),
                (Sequence, 0),
                (Sequence, 0),
                (Sequence, 0),
                (Sequence, 480),
                (Sequence, 960)
            ]
        );
    }

    #[test]
    fn prop_written_events_read_back_in_order() {
        proptest!(|(deltas in proptest::collection::vec(0u64..3_000_000, 0..16))| {
            let mut buf = std::vec![0u8; 4096];
            let mut w = ClipWriter::new(&mut buf, &config(96)).unwrap();
            w.start(0, None, None).unwrap();
            let mut tick = 0;
            let mut expected = Vec::new();
            for d in deltas {
                tick += d;
                w.event(tick, &NOTE_ON).unwrap();
                expected.push(tick);
            }
            let n = w.finish(tick).unwrap();
            let got: Vec<_> = ClipReader::new(&buf[..n])
                .unwrap()
                .map(Result::unwrap)
                .filter(|e| matches!(e.ump, Ump::Midi2ChannelVoice(_)))
                .map(|e| e.ticks)
                .collect();
            prop_assert_eq!(got, expected);
        });
    }
}
