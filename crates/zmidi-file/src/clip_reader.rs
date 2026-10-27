//! MIDI Clip File reader (M2-116-U v1.0).
//!
//! - The file starts with the File Header "SMF2CLIP"; after it there are only UMP messages,
//!   big endian (sections 3, 4).
//! - The Clip Configuration Header is everything after the File Header up to the Delta
//!   Clockstamp of Start of Clip; that Delta Clockstamp starts the Clip Sequence Data (6, 7).
//! - A Delta Clockstamp counts ticks in the unit of the most recent DCTPQ; every other message
//!   has the time of the most recent Delta Clockstamp, so messages after one Delta Clockstamp
//!   share its time (M2-104-UM 7.2.3, M2-116 5).
//! - No data follows End of Clip (7.3).
//!
//! The reader iterates a borrowed byte slice. No alloc (DEC-002).

use crate::clip_format::{word_from_bytes, FILE_HEADER};
use zmidi_ump::{MessageType, Ump, UmpStreamMessage, UtilityMessage};

/// Error of the clip reader (thiserror 2, no_std).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClipError {
    /// The bytes do not start with the File Header (Table 5).
    #[error("not a MIDI Clip File: bad File Header")]
    BadHeader,
    /// A trailing partial word, or a UMP cut by the end of the data.
    #[error("MIDI Clip File truncated")]
    Truncated,
    /// A byte follows End of Clip.
    #[error("data after End of Clip")]
    DataAfterEnd,
}

/// The part of the clip that holds a message (sections 6 and 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipSection {
    /// The Clip Configuration Header: before Start of Clip.
    Configuration,
    /// The Clip Sequence Data: Start of Clip and all later messages.
    Sequence,
}

/// One message of the clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipEvent {
    /// The section that holds the message.
    pub section: ClipSection,
    /// Absolute ticks since the start of the file, in the unit of the DCTPQ in force.
    pub ticks: u64,
    /// The message.
    pub ump: Ump,
}

/// Iterator over the messages of a MIDI Clip File in a byte slice. No alloc.
#[derive(Debug, Clone)]
pub struct ClipReader<'a> {
    /// The file bytes.
    data: &'a [u8],
    /// Position of the next unread byte.
    pos: usize,
    /// Current section.
    section: ClipSection,
    /// Tick time of the most recent Delta Clockstamp.
    ticks: u64,
    /// Ticks Per Quarter Note of the most recent DCTPQ.
    tpq: Option<u16>,
    /// True after End of Clip.
    ended: bool,
    /// True when the iterator has ended.
    done: bool,
}

impl<'a> ClipReader<'a> {
    /// A reader over the bytes of a file. Gives [`ClipError::BadHeader`] when the bytes do not
    /// start with the File Header.
    pub fn new(data: &'a [u8]) -> Result<Self, ClipError> {
        if !data.starts_with(&FILE_HEADER) {
            return Err(ClipError::BadHeader);
        }
        Ok(Self {
            data,
            pos: FILE_HEADER.len(),
            section: ClipSection::Configuration,
            ticks: 0,
            tpq: None,
            ended: false,
            done: false,
        })
    }

    /// The Ticks Per Quarter Note of the most recent DCTPQ read so far.
    pub fn ticks_per_quarter_note(&self) -> Option<u16> {
        self.tpq
    }

    /// Ends the iterator with `error`.
    fn fail(&mut self, error: ClipError) -> Option<Result<ClipEvent, ClipError>> {
        self.done = true;
        Some(Err(error))
    }

    /// The word at byte position `at`, if four bytes are there.
    fn word(&self, at: usize) -> Option<u32> {
        let b = self.data.get(at..at + 4)?;
        Some(word_from_bytes([b[0], b[1], b[2], b[3]]))
    }
}

impl Iterator for ClipReader<'_> {
    type Item = Result<ClipEvent, ClipError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.ended {
            self.done = true;
            return (self.pos < self.data.len()).then_some(Err(ClipError::DataAfterEnd));
        }
        loop {
            if self.pos == self.data.len() {
                self.done = true;
                return None;
            }
            let Some(w0) = self.word(self.pos) else {
                return self.fail(ClipError::Truncated);
            };
            let count = MessageType::from_bits((w0 >> 28) as u8).word_count();
            let mut words = [0u32; 4];
            for (i, w) in words.iter_mut().enumerate().take(count) {
                match self.word(self.pos + 4 * i) {
                    Some(v) => *w = v,
                    None => return self.fail(ClipError::Truncated),
                }
            }
            self.pos += 4 * count;
            let Ok((ump, _)) = Ump::parse(&words[..count]) else {
                return self.fail(ClipError::Truncated);
            };
            match ump {
                Ump::Utility(UtilityMessage::DeltaClockstamp(dc)) => {
                    self.ticks += u64::from(dc.ticks());
                    continue;
                }
                Ump::Utility(UtilityMessage::DeltaClockstampTpq(tpq)) => {
                    self.tpq = Some(tpq.ticks_per_quarter_note());
                    continue;
                }
                Ump::UmpStream(UmpStreamMessage::StartOfClip(_)) => {
                    self.section = ClipSection::Sequence;
                }
                Ump::UmpStream(UmpStreamMessage::EndOfClip(_)) => self.ended = true,
                _ => {}
            }
            return Some(Ok(ClipEvent {
                section: self.section,
                ticks: self.ticks,
                ump,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::clip_format::word_to_bytes;
    use proptest::prelude::*;
    use std::vec::Vec;

    /// A file: the File Header and `words`, big endian.
    fn file(words: &[u32]) -> Vec<u8> {
        let mut f = FILE_HEADER.to_vec();
        for w in words {
            f.extend_from_slice(&word_to_bytes(*w));
        }
        f
    }

    const DCS0: u32 = 0x0040_0000;
    const DCTPQ_96: u32 = 0x0030_0060;
    const START: [u32; 4] = [0xF020_0000, 0, 0, 0];
    const END: [u32; 4] = [0xF021_0000, 0, 0, 0];
    const NOTE_ON: [u32; 2] = [0x4090_3C00, 0xFFFF_0000];
    /// Set Tempo, Group 0 (Figure 70), 500 000 µs per quarter note.
    const TEMPO: [u32; 4] = [0xD010_0000, 50_000_000, 0, 0];

    fn clip(seq: &[u32]) -> Vec<u8> {
        let mut w = [DCS0, DCTPQ_96, DCS0].to_vec();
        w.extend_from_slice(&START);
        w.extend_from_slice(seq);
        w.push(DCS0);
        w.extend_from_slice(&END);
        file(&w)
    }

    fn events(bytes: &[u8]) -> Vec<Result<ClipEvent, ClipError>> {
        ClipReader::new(bytes).unwrap().collect()
    }

    #[test]
    fn reader_rejects_bad_header() {
        assert_eq!(
            ClipReader::new(b"SMF2CLIQ").err(),
            Some(ClipError::BadHeader)
        );
        assert_eq!(ClipReader::new(b"SMF2").err(), Some(ClipError::BadHeader));
    }

    #[test]
    fn reader_gives_sections_and_ticks() {
        let mut seq = [0x0040_0060u32].to_vec(); // 96 ticks
        seq.extend_from_slice(&NOTE_ON);
        seq.extend_from_slice(&NOTE_ON); // shares the Delta Clockstamp
        let bytes = clip(&seq);
        let mut r = ClipReader::new(&bytes).unwrap();
        let start = r.next().unwrap().unwrap();
        assert_eq!(start.section, ClipSection::Sequence);
        assert_eq!(start.ticks, 0);
        assert_eq!(r.ticks_per_quarter_note(), Some(96));
        let a = r.next().unwrap().unwrap();
        let b = r.next().unwrap().unwrap();
        assert_eq!((a.ticks, b.ticks), (96, 96));
        let end = r.next().unwrap().unwrap();
        assert!(matches!(
            end.ump,
            Ump::UmpStream(UmpStreamMessage::EndOfClip(_))
        ));
        assert_eq!(end.ticks, 96);
        assert!(r.next().is_none());
    }

    #[test]
    fn reader_treats_data_before_start_of_clip_as_configuration() {
        // Set Tempo sharing the DCTPQ Delta Clockstamp (CFG-005 first form), then a
        // configuration message with its own Delta Clockstamp 0.
        let mut w = [DCS0, DCTPQ_96].to_vec();
        w.extend_from_slice(&TEMPO);
        w.push(DCS0);
        w.extend_from_slice(&NOTE_ON);
        w.push(0x0040_0010); // 16 ticks before Start of Clip: they count
        w.extend_from_slice(&START);
        w.push(DCS0);
        w.extend_from_slice(&END);
        let ev: Vec<_> = events(&file(&w)).into_iter().map(Result::unwrap).collect();
        assert_eq!(ev.len(), 4);
        assert_eq!(ev[0].section, ClipSection::Configuration);
        assert!(matches!(ev[0].ump, Ump::FlexData(_)));
        assert_eq!(ev[1].section, ClipSection::Configuration);
        assert_eq!((ev[2].section, ev[2].ticks), (ClipSection::Sequence, 16));
    }

    #[test]
    fn reader_accepts_tempo_with_own_delta_clockstamp() {
        // CFG-005 second form: Set Tempo with its own Delta Clockstamp 0.
        let mut w = [DCS0, DCTPQ_96, DCS0].to_vec();
        w.extend_from_slice(&TEMPO);
        w.push(DCS0);
        w.extend_from_slice(&START);
        w.push(DCS0);
        w.extend_from_slice(&END);
        let ev: Vec<_> = events(&file(&w)).into_iter().map(Result::unwrap).collect();
        assert_eq!(
            (ev[0].section, ev[0].ticks),
            (ClipSection::Configuration, 0)
        );
    }

    #[test]
    fn reader_accepts_any_message_in_sequence() {
        // FILE-007: several Set Tempo messages and a NOOP in the Clip Sequence Data.
        let mut seq = [0x0040_0060u32].to_vec();
        seq.extend_from_slice(&TEMPO);
        seq.push(0x0000_0000);
        seq.push(0x0040_0060);
        seq.extend_from_slice(&TEMPO);
        let ev: Vec<_> = events(&clip(&seq))
            .into_iter()
            .map(Result::unwrap)
            .collect();
        let ticks: Vec<_> = ev.iter().map(|e| e.ticks).collect();
        assert_eq!(ticks, [0, 96, 96, 192, 192]);
    }

    #[test]
    fn reader_reports_data_after_end_of_clip() {
        let mut bytes = clip(&[]);
        bytes.extend_from_slice(&word_to_bytes(DCS0));
        let ev = events(&bytes);
        assert_eq!(ev.last(), Some(&Err(ClipError::DataAfterEnd)));
        assert_eq!(ev.len(), 3);
    }

    #[test]
    fn reader_reports_truncated_data() {
        let mut bytes = clip(&[]);
        bytes.truncate(bytes.len() - 2);
        assert_eq!(events(&bytes).last(), Some(&Err(ClipError::Truncated)));
        // A 128-bit UMP cut after 2 words.
        let bytes = file(&[DCS0, DCTPQ_96, DCS0, 0xF020_0000, 0]);
        assert_eq!(events(&bytes), [Err(ClipError::Truncated)]);
    }

    #[test]
    fn prop_reader_never_panics() {
        proptest!(|(tail in proptest::collection::vec(any::<u8>(), 0..128))| {
            let mut bytes = FILE_HEADER.to_vec();
            bytes.extend(tail);
            let reader = ClipReader::new(&bytes).unwrap();
            let mut last = 0;
            for event in reader.take(64).flatten() {
                // Ticks never decrease (DC-003).
                prop_assert!(event.ticks >= last);
                last = event.ticks;
            }
        });
    }
}
