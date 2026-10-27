//! Sans-IO Standard MIDI File 1.0 reader (RP-001, revised February 1996).
//!
//! - `SmfReader::new` checks the header chunk `MThd` (at least 6 data bytes; a longer header is
//!   skipped by its length, p. 5) and reads format, ntrks and division (p. 4).
//! - `SmfReader::tracks` gives the `MTrk` chunks in file order and skips alien chunks (p. 3). It
//!   gives every `MTrk` chunk it finds, also when their number differs from ntrks
//!   (AMB-RP-001-03).
//! - A `TrackReader` gives the events of one track: delta-time, absolute ticks and the event, with
//!   running status (p. 6). Sysex events and meta-events clear running status (p. 7).
//! - Sysex events in packets (p. 6 and 7): an F0 event with no final F7 opens a message, F7 events
//!   continue it, and the F7 event that ends with F7 closes it. A channel message or End of Track
//!   in an open message is an error; a meta-event is allowed (AMB-RP-001-06). An F7 event with no
//!   open message is an escape.
//! - End of Track ends the track (p. 8 and 9). A chunk with no End of Track or with data after it
//!   gives an error once.
//!
//! No buffer, no alloc (DEC-002), no I/O. After an error an iterator ends. Never panics on any
//! input bytes.

use crate::smf_format::{
    read_vlq, ChannelMessage, Division, FileFormat, MetaEvent, SmfError, MTHD, MTRK,
};

/// The 8-byte chunk header at `pos`: the type and the data range. `Truncated` when the header or
/// the data is cut by the end of `bytes`.
fn chunk_at(bytes: &[u8], pos: usize) -> Result<([u8; 4], usize, usize), SmfError> {
    let head = bytes.get(pos..pos + 8).ok_or(SmfError::Truncated)?;
    let kind = [head[0], head[1], head[2], head[3]];
    let len = u32::from_be_bytes([head[4], head[5], head[6], head[7]]) as usize;
    let start = pos + 8;
    let end = start.checked_add(len).ok_or(SmfError::Truncated)?;
    if end > bytes.len() {
        return Err(SmfError::Truncated);
    }
    Ok((kind, start, end))
}

/// A Standard MIDI File in a borrowed byte slice: the header values and the track chunks.
#[derive(Debug, Clone, Copy)]
pub struct SmfReader<'a> {
    bytes: &'a [u8],
    format: FileFormat,
    track_count: u16,
    division: Division,
    first_chunk: usize,
}

impl<'a> SmfReader<'a> {
    /// Checks the header chunk and reads its values. `NoHeader` when the data does not start with
    /// `MThd`, `BadHeaderLength` for less than 6 data bytes, `Truncated` when the chunk is cut,
    /// `BadDivision` for a bad SMPTE format.
    pub fn new(bytes: &'a [u8]) -> Result<Self, SmfError> {
        if bytes.get(..4) != Some(&MTHD[..]) {
            return Err(SmfError::NoHeader);
        }
        let len = bytes
            .get(4..8)
            .map(|l| u32::from_be_bytes([l[0], l[1], l[2], l[3]]))
            .ok_or(SmfError::Truncated)?;
        if len < 6 {
            return Err(SmfError::BadHeaderLength);
        }
        let (_, start, end) = chunk_at(bytes, 0)?;
        let word = |i: usize| u16::from_be_bytes([bytes[start + i], bytes[start + i + 1]]);
        Ok(Self {
            bytes,
            format: FileFormat::from_word(word(0)),
            track_count: word(2),
            division: Division::from_word(word(4))?,
            first_chunk: end,
        })
    }

    /// The `<format>` of the header chunk.
    pub fn format(&self) -> FileFormat {
        self.format
    }

    /// The `<ntrks>` of the header chunk, as written in the file.
    pub fn track_count(&self) -> u16 {
        self.track_count
    }

    /// The `<division>` of the header chunk.
    pub fn division(&self) -> Division {
        self.division
    }

    /// The track chunks in file order.
    pub fn tracks(&self) -> Tracks<'a> {
        Tracks {
            bytes: self.bytes,
            pos: self.first_chunk,
            done: false,
        }
    }
}

/// An iterator over the `MTrk` chunks after the header chunk. It skips chunks of other types. A
/// chunk cut by the end of the data gives `Truncated` once, then the iterator ends.
#[derive(Debug, Clone)]
pub struct Tracks<'a> {
    bytes: &'a [u8],
    pos: usize,
    done: bool,
}

impl<'a> Iterator for Tracks<'a> {
    type Item = Result<TrackReader<'a>, SmfError>;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done && self.pos < self.bytes.len() {
            match chunk_at(self.bytes, self.pos) {
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
                Ok((kind, start, end)) => {
                    self.pos = end;
                    if kind == MTRK {
                        return Some(Ok(TrackReader::new(&self.bytes[start..end])));
                    }
                }
            }
        }
        None
    }
}

/// The kind of a sysex event (Track Chunks, p. 6 and 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SysExKind {
    /// An F0 event that ends with F7: one complete message.
    Complete,
    /// An F0 event that does not end with F7: the first packet of a message in packets.
    Start,
    /// An F7 event in an open message that does not end with F7.
    Continue,
    /// An F7 event in an open message that ends with F7: the last packet.
    End,
    /// An F7 event with no open message: any bytes to transmit.
    Escape,
}

/// One event of a track (Track Chunks, p. 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event<'a> {
    /// A MIDI channel message.
    Channel(ChannelMessage),
    /// A sysex event: the bytes after the length (the final F7 included, the F0 or F7 code not).
    SysEx {
        /// The kind of the event.
        kind: SysExKind,
        /// The bytes after the length.
        data: &'a [u8],
    },
    /// A meta-event of any type: the type byte and the data bytes.
    Meta {
        /// The type byte.
        kind: u8,
        /// The data bytes.
        data: &'a [u8],
    },
}

impl<'a> Event<'a> {
    /// The typed value of a meta-event (`MetaEvent::decode`). None for another event, an unknown
    /// type, short data or a value out of range.
    pub fn meta(&self) -> Option<MetaEvent<'a>> {
        match self {
            Self::Meta { kind, data } => MetaEvent::decode(*kind, data),
            _ => None,
        }
    }

    /// The UMP MIDI 1.0 Channel Voice word of a channel message with `group` (M2-104-UM 7.3).
    /// None for another event.
    pub fn to_ump(&self, group: u8) -> Option<u32> {
        match self {
            Self::Channel(m) => Some(m.to_ump(group)),
            _ => None,
        }
    }
}

/// An event with its time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TrackEvent<'a> {
    /// The delta-time in ticks before the event.
    pub delta: u32,
    /// The absolute ticks since the start of the track.
    pub ticks: u64,
    /// The event.
    pub event: Event<'a>,
}

/// An iterator over the events of one track chunk.
#[derive(Debug, Clone)]
pub struct TrackReader<'a> {
    data: &'a [u8],
    pos: usize,
    running: Option<u8>,
    ticks: u64,
    sysex_open: bool,
    end_of_track: bool,
    done: bool,
}

impl<'a> TrackReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            running: None,
            ticks: 0,
            sysex_open: false,
            end_of_track: false,
            done: false,
        }
    }

    /// The absolute ticks of the last event read.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// A variable-length quantity length and the bytes it counts, at the position.
    fn counted_bytes(&mut self) -> Result<&'a [u8], SmfError> {
        let (len, n) = read_vlq(&self.data[self.pos..])?;
        let start = self.pos + n;
        let bytes = self
            .data
            .get(start..start + len as usize)
            .ok_or(SmfError::Truncated)?;
        self.pos = start + len as usize;
        Ok(bytes)
    }

    fn read_event(&mut self) -> Result<TrackEvent<'a>, SmfError> {
        let (delta, n) = read_vlq(&self.data[self.pos..])?;
        self.pos += n;
        let first = *self.data.get(self.pos).ok_or(SmfError::Truncated)?;
        let event = match first {
            0x00..=0xEF => {
                let status = if first < 0x80 {
                    self.running.ok_or(SmfError::NoRunningStatus)?
                } else {
                    self.pos += 1;
                    first
                };
                if self.sysex_open {
                    return Err(SmfError::SysExInterrupted);
                }
                let len = ChannelMessage::data_len_of(status);
                let d = self
                    .data
                    .get(self.pos..self.pos + len)
                    .ok_or(SmfError::Truncated)?;
                let m = ChannelMessage::new(status, d[0], *d.get(1).unwrap_or(&0))?;
                self.pos += len;
                self.running = Some(status);
                Event::Channel(m)
            }
            0xF0 | 0xF7 => {
                self.pos += 1;
                self.running = None;
                let data = self.counted_bytes()?;
                let ends = data.last() == Some(&0xF7);
                let kind = match (first, self.sysex_open) {
                    (0xF0, true) => return Err(SmfError::SysExInterrupted),
                    (0xF0, false) if ends => SysExKind::Complete,
                    (0xF0, false) => SysExKind::Start,
                    (_, true) if ends => SysExKind::End,
                    (_, true) => SysExKind::Continue,
                    (_, false) => SysExKind::Escape,
                };
                self.sysex_open = matches!(kind, SysExKind::Start | SysExKind::Continue);
                Event::SysEx { kind, data }
            }
            0xFF => {
                self.pos += 1;
                self.running = None;
                let kind = *self.data.get(self.pos).ok_or(SmfError::Truncated)?;
                self.pos += 1;
                let data = self.counted_bytes()?;
                if kind == 0x2F {
                    if self.sysex_open {
                        return Err(SmfError::SysExInterrupted);
                    }
                    self.end_of_track = true;
                }
                Event::Meta { kind, data }
            }
            status => return Err(SmfError::UnexpectedStatus(status)),
        };
        self.ticks += u64::from(delta);
        Ok(TrackEvent {
            delta,
            ticks: self.ticks,
            event,
        })
    }
}

impl<'a> Iterator for TrackReader<'a> {
    type Item = Result<TrackEvent<'a>, SmfError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.end_of_track || self.pos >= self.data.len() {
            self.done = true;
            return match (self.end_of_track, self.pos < self.data.len()) {
                (true, true) => Some(Err(SmfError::DataAfterEndOfTrack)),
                (true, false) => None,
                (false, _) => Some(Err(SmfError::NoEndOfTrack)),
            };
        }
        let r = self.read_event();
        if r.is_err() {
            self.done = true;
        }
        Some(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smf_examples::{FORMAT_0, FORMAT_1};
    use crate::smf_format::{SmpteFormat, TimeSignature};
    use proptest::prelude::*;

    /// A file with one track chunk that holds `track`.
    fn file(track: &[u8]) -> [u8; 64] {
        let mut f = [0u8; 64];
        f[..14].copy_from_slice(&FORMAT_0[..14]);
        f[14..18].copy_from_slice(b"MTrk");
        f[18..22].copy_from_slice(&(track.len() as u32).to_be_bytes());
        f[22..22 + track.len()].copy_from_slice(track);
        f
    }

    /// The events of the first track of `bytes`, and the error that ends them.
    fn events(bytes: &[u8]) -> ([Option<Event<'_>>; 8], Option<SmfError>) {
        let mut out = [None; 8];
        let mut err = None;
        let track = SmfReader::new(bytes)
            .unwrap()
            .tracks()
            .next()
            .unwrap()
            .unwrap();
        for (i, r) in track.enumerate() {
            match r {
                Ok(e) => out[i] = Some(e.event),
                Err(e) => err = Some(e),
            }
        }
        (out, err)
    }

    fn track_bytes(track: &[u8]) -> ([u8; 64], usize) {
        (file(track), 22 + track.len())
    }

    fn ch(s: u8, a: u8, b: u8) -> Option<Event<'static>> {
        Some(Event::Channel(ChannelMessage::new(s, a, b).unwrap()))
    }

    #[test]
    fn example_format_0_reads() {
        let r = SmfReader::new(FORMAT_0).unwrap();
        assert_eq!(r.format(), FileFormat::Single);
        assert_eq!(r.track_count(), 1);
        assert_eq!(r.division(), Division::TicksPerQuarterNote(96));
        let mut tracks = r.tracks();
        let t = tracks.next().unwrap().unwrap();
        assert!(tracks.next().is_none());
        let expected: [(u32, u64, Option<Event>); 14] = [
            (
                0,
                0,
                Some(Event::Meta {
                    kind: 0x58,
                    data: &[4, 2, 0x18, 8],
                }),
            ),
            (
                0,
                0,
                Some(Event::Meta {
                    kind: 0x51,
                    data: &[0x07, 0xA1, 0x20],
                }),
            ),
            (0, 0, ch(0xC0, 5, 0)),
            (0, 0, ch(0xC1, 46, 0)),
            (0, 0, ch(0xC2, 70, 0)),
            (0, 0, ch(0x92, 48, 96)),
            (0, 0, ch(0x92, 60, 96)),
            (96, 96, ch(0x91, 67, 64)),
            (96, 192, ch(0x90, 76, 32)),
            (192, 384, ch(0x82, 48, 64)),
            (0, 384, ch(0x82, 60, 64)),
            (0, 384, ch(0x81, 67, 64)),
            (0, 384, ch(0x80, 76, 64)),
            (
                0,
                384,
                Some(Event::Meta {
                    kind: 0x2F,
                    data: &[],
                }),
            ),
        ];
        let mut n = 0;
        for (e, (delta, ticks, event)) in t.zip(expected) {
            let e = e.unwrap();
            assert_eq!(
                (e.delta, e.ticks, Some(e.event)),
                (delta, ticks, event),
                "event {n}"
            );
            n += 1;
        }
        assert_eq!(n, 14);
        let mut t = SmfReader::new(FORMAT_0)
            .unwrap()
            .tracks()
            .next()
            .unwrap()
            .unwrap();
        let first = t.next().unwrap().unwrap().event;
        assert_eq!(
            first.meta(),
            Some(MetaEvent::TimeSignature(TimeSignature::DEFAULT))
        );
        assert_eq!(first.to_ump(0), None);
        assert_eq!(
            t.nth(4).unwrap().unwrap().event.to_ump(1),
            Some(0x2192_3060)
        );
        assert_eq!(
            t.nth(7).unwrap().unwrap().event.meta(),
            Some(MetaEvent::EndOfTrack)
        );
        assert!(t.next().is_none());
        assert_eq!(t.ticks(), 384);
    }

    #[test]
    fn example_format_1_reads() {
        let r = SmfReader::new(FORMAT_1).unwrap();
        assert_eq!(r.format(), FileFormat::MultiTrack);
        assert_eq!(r.track_count(), 4);
        let mut counts = [0usize; 4];
        let mut ends = [0u64; 4];
        let mut n = 0;
        for (i, t) in r.tracks().enumerate() {
            for e in t.unwrap() {
                let e = e.unwrap();
                counts[i] += 1;
                ends[i] = e.ticks;
            }
            n += 1;
        }
        assert_eq!(n, 4);
        assert_eq!(counts, [3, 4, 4, 6]);
        assert_eq!(ends, [384; 4]);
        // Running status with Note On velocity 0 (p. 13).
        let t = r.tracks().nth(1).unwrap().unwrap();
        let last_note = t.map(|e| e.unwrap()).nth(2).unwrap();
        assert_eq!(
            (last_note.delta, last_note.event),
            (192, ch(0x90, 76, 0).unwrap())
        );
    }

    #[test]
    fn reader_rejects_missing_mthd() {
        assert_eq!(SmfReader::new(b"").err(), Some(SmfError::NoHeader));
        assert_eq!(
            SmfReader::new(b"MTrk\0\0\0\x06\0\0\0\x01\0\x60").err(),
            Some(SmfError::NoHeader)
        );
        assert_eq!(SmfReader::new(b"MThd\0\0").err(), Some(SmfError::Truncated));
        assert_eq!(
            SmfReader::new(b"MThd\0\0\0\x05\0\0\0\x01\0").err(),
            Some(SmfError::BadHeaderLength)
        );
        assert_eq!(
            SmfReader::new(&FORMAT_0[..13]).err(),
            Some(SmfError::Truncated)
        );
    }

    #[test]
    fn reader_honors_long_header() {
        let mut f = [0u8; 120];
        f[..8].copy_from_slice(b"MThd\0\0\0\x09");
        f[8..14].copy_from_slice(&FORMAT_0[8..14]);
        f[14..17].copy_from_slice(&[0xAA, 0xBB, 0xCC]);
        f[17..17 + FORMAT_0.len() - 14].copy_from_slice(&FORMAT_0[14..]);
        let len = 17 + FORMAT_0.len() - 14;
        let r = SmfReader::new(&f[..len]).unwrap();
        assert_eq!(r.division(), Division::TicksPerQuarterNote(96));
        let events = r.tracks().next().unwrap().unwrap().count();
        assert_eq!(events, 14);
    }

    #[test]
    fn reader_rejects_bad_smpte_format() {
        let mut f = [0u8; 14];
        f.copy_from_slice(&FORMAT_0[..14]);
        f[12..14].copy_from_slice(&0xE650u16.to_be_bytes());
        assert_eq!(SmfReader::new(&f).err(), Some(SmfError::BadDivision));
        f[12..14].copy_from_slice(&0xE250u16.to_be_bytes());
        assert_eq!(
            SmfReader::new(&f).unwrap().division(),
            Division::Smpte {
                format: SmpteFormat::Fps30,
                ticks_per_frame: 80
            }
        );
    }

    #[test]
    fn reader_accepts_unknown_format_and_track_count() {
        let mut f = [0u8; 256];
        f[..FORMAT_1.len()].copy_from_slice(FORMAT_1);
        f[8..10].copy_from_slice(&7u16.to_be_bytes());
        f[10..12].copy_from_slice(&1u16.to_be_bytes());
        let r = SmfReader::new(&f[..FORMAT_1.len()]).unwrap();
        assert_eq!(r.format(), FileFormat::Other(7));
        assert_eq!(r.track_count(), 1);
        assert_eq!(r.tracks().count(), 4);
    }

    #[test]
    fn reader_rejects_truncated_chunk() {
        let r = SmfReader::new(&FORMAT_1[..FORMAT_1.len() - 1]).unwrap();
        let mut t = r.tracks();
        assert!(t.next().unwrap().is_ok());
        assert!(t.next().unwrap().is_ok());
        assert!(t.next().unwrap().is_ok());
        assert_eq!(t.next().unwrap().err(), Some(SmfError::Truncated));
        assert!(t.next().is_none());
        // A chunk header cut by the end of the data.
        let r = SmfReader::new(&FORMAT_0[..18]).unwrap();
        assert_eq!(r.tracks().next().unwrap().err(), Some(SmfError::Truncated));
    }

    #[test]
    fn reader_skips_alien_chunks() {
        let mut f = [0u8; 128];
        f[..14].copy_from_slice(&FORMAT_0[..14]);
        f[14..25].copy_from_slice(b"XFIH\0\0\0\x03abc");
        f[25..25 + FORMAT_0.len() - 14].copy_from_slice(&FORMAT_0[14..]);
        let len = 25 + FORMAT_0.len() - 14;
        f[len..len + 8].copy_from_slice(b"ZZZZ\0\0\0\0");
        let r = SmfReader::new(&f[..len + 8]).unwrap();
        let mut t = r.tracks();
        assert_eq!(t.next().unwrap().unwrap().count(), 14);
        assert!(t.next().is_none());
    }

    #[test]
    fn reader_rejects_data_without_status() {
        let (f, n) = track_bytes(&[0x00, 0x3C, 0x40, 0x00, 0xFF, 0x2F, 0x00]);
        let (e, err) = events(&f[..n]);
        assert_eq!(e[0], None);
        assert_eq!(err, Some(SmfError::NoRunningStatus));
    }

    #[test]
    fn running_status_cleared_by_meta_and_sysex() {
        // A data byte after a meta-event.
        let (f, n) = track_bytes(&[
            0x00, 0x90, 0x3C, 0x40, 0x00, 0xFF, 0x01, 0x00, 0x00, 0x3C, 0x00,
        ]);
        let (e, err) = events(&f[..n]);
        assert_eq!(e[0], ch(0x90, 0x3C, 0x40));
        assert!(e[1].is_some());
        assert_eq!(err, Some(SmfError::NoRunningStatus));
        // A data byte after a sysex event.
        let (f, n) = track_bytes(&[
            0x00, 0x90, 0x3C, 0x40, 0x00, 0xF0, 0x01, 0xF7, 0x00, 0x3C, 0x00,
        ]);
        let (_, err) = events(&f[..n]);
        assert_eq!(err, Some(SmfError::NoRunningStatus));
    }

    #[test]
    fn reader_rejects_system_status_as_event() {
        for s in [0xF1u8, 0xF2, 0xF6, 0xF8, 0xFA, 0xFE] {
            let (f, n) = track_bytes(&[0x00, s, 0x00, 0xFF, 0x2F, 0x00]);
            assert_eq!(events(&f[..n]).1, Some(SmfError::UnexpectedStatus(s)));
        }
    }

    #[test]
    fn reader_rejects_data_byte_above_7f() {
        let (f, n) = track_bytes(&[0x00, 0x90, 0x3C, 0x80, 0x00, 0xFF, 0x2F, 0x00]);
        assert_eq!(events(&f[..n]).1, Some(SmfError::BadDataByte));
        let (f, n) = track_bytes(&[0x00, 0x90, 0x3C]);
        assert_eq!(events(&f[..n]).1, Some(SmfError::Truncated));
    }

    #[test]
    fn sysex_f0_event_example() {
        // F0 43 12 00 07 F7 is stored as F0 05 43 12 00 07 F7 (p. 6).
        let (f, n) = track_bytes(&[
            0x00, 0xF0, 0x05, 0x43, 0x12, 0x00, 0x07, 0xF7, 0x00, 0xFF, 0x2F, 0x00,
        ]);
        let (e, err) = events(&f[..n]);
        assert_eq!(
            e[0],
            Some(Event::SysEx {
                kind: SysExKind::Complete,
                data: &[0x43, 0x12, 0x00, 0x07, 0xF7]
            })
        );
        assert_eq!(err, None);
    }

    #[test]
    fn sysex_f7_escape() {
        // A Song Position Pointer through the escape; no final F7 is needed (p. 7).
        let (f, n) = track_bytes(&[0x00, 0xF7, 0x03, 0xF2, 0x00, 0x00, 0x00, 0xFF, 0x2F, 0x00]);
        let (e, err) = events(&f[..n]);
        assert_eq!(
            e[0],
            Some(Event::SysEx {
                kind: SysExKind::Escape,
                data: &[0xF2, 0, 0]
            })
        );
        assert_eq!(err, None);
    }

    #[test]
    fn sysex_multi_packet_example() {
        // Page 7: F0 03 43 12 00 / 81 48 / F7 06 43 12 00 43 12 00 / 64 / F7 04 43 12 00 F7.
        #[rustfmt::skip]
        let (f, n) = track_bytes(&[
            0x00, 0xF0, 0x03, 0x43, 0x12, 0x00,
            0x81, 0x48, 0xF7, 0x06, 0x43, 0x12, 0x00, 0x43, 0x12, 0x00,
            0x00, 0xFF, 0x01, 0x01, b'x', // a meta-event between packets is allowed
            0x64, 0xF7, 0x04, 0x43, 0x12, 0x00, 0xF7,
            0x00, 0xFF, 0x2F, 0x00,
        ]);
        let r = SmfReader::new(&f[..n]).unwrap();
        let t = r.tracks().next().unwrap().unwrap();
        let mut kinds = [None; 5];
        let mut ticks = [0u64; 5];
        for (i, e) in t.enumerate() {
            let e = e.unwrap();
            ticks[i] = e.ticks;
            if let Event::SysEx { kind, .. } = e.event {
                kinds[i] = Some(kind);
            }
        }
        assert_eq!(
            kinds,
            [
                Some(SysExKind::Start),
                Some(SysExKind::Continue),
                None,
                Some(SysExKind::End),
                None
            ]
        );
        assert_eq!(ticks, [0, 200, 200, 300, 300]);
    }

    #[test]
    fn reader_rejects_channel_message_inside_sysex_packets() {
        let (f, n) = track_bytes(&[0x00, 0xF0, 0x01, 0x43, 0x00, 0x90, 0x3C, 0x40]);
        assert_eq!(events(&f[..n]).1, Some(SmfError::SysExInterrupted));
        let (f, n) = track_bytes(&[0x00, 0xF0, 0x01, 0x43, 0x00, 0xF0, 0x01, 0xF7]);
        assert_eq!(events(&f[..n]).1, Some(SmfError::SysExInterrupted));
    }

    #[test]
    fn end_of_track_with_open_sysex_rejected() {
        let (f, n) = track_bytes(&[0x00, 0xF0, 0x01, 0x43, 0x00, 0xFF, 0x2F, 0x00]);
        assert_eq!(events(&f[..n]).1, Some(SmfError::SysExInterrupted));
        let mut buf = [0u8; 64];
        let mut w = crate::SmfWriter::new(
            &mut buf,
            FileFormat::Single,
            Division::TicksPerQuarterNote(96),
        )
        .unwrap();
        w.begin_track().unwrap();
        w.sysex_start(0, &[0x43]).unwrap();
        assert_eq!(w.end_track(0), Err(crate::SmfWriteError::SysExOpen));
    }

    #[test]
    fn reader_gives_unknown_meta_event() {
        let (f, n) = track_bytes(&[
            0x00, 0xFF, 0x60, 0x02, 0xAB, 0xCD, 0x00, 0x90, 0x3C, 0x40, 0x00, 0xFF, 0x2F, 0x00,
        ]);
        let (e, err) = events(&f[..n]);
        assert_eq!(
            e[0],
            Some(Event::Meta {
                kind: 0x60,
                data: &[0xAB, 0xCD]
            })
        );
        assert_eq!(e[0].unwrap().meta(), None);
        assert_eq!(e[1], ch(0x90, 0x3C, 0x40));
        assert_eq!(err, None);
    }

    #[test]
    fn reader_rejects_track_without_end_of_track() {
        let (f, n) = track_bytes(&[0x00, 0x90, 0x3C, 0x40]);
        let (e, err) = events(&f[..n]);
        assert_eq!(e[0], ch(0x90, 0x3C, 0x40));
        assert_eq!(err, Some(SmfError::NoEndOfTrack));
        let (f, n) = track_bytes(&[]);
        assert_eq!(events(&f[..n]).1, Some(SmfError::NoEndOfTrack));
    }

    #[test]
    fn reader_rejects_event_after_end_of_track() {
        let (f, n) = track_bytes(&[0x00, 0xFF, 0x2F, 0x00, 0x00, 0x90, 0x3C, 0x40]);
        let r = SmfReader::new(&f[..n]).unwrap();
        let mut t = r.tracks().next().unwrap().unwrap();
        assert!(t.next().unwrap().is_ok());
        assert_eq!(t.next(), Some(Err(SmfError::DataAfterEndOfTrack)));
        assert_eq!(t.next(), None);
    }

    #[test]
    fn reader_gives_misplaced_track_name() {
        let (f, n) = track_bytes(&[
            0x00, 0x90, 0x3C, 0x40, 0x60, 0xFF, 0x03, 0x01, b'A', 0x00, 0xFF, 0x2F, 0x00,
        ]);
        let (e, err) = events(&f[..n]);
        assert_eq!(
            e[1],
            Some(Event::Meta {
                kind: 0x03,
                data: b"A"
            })
        );
        assert_eq!(err, None);
    }

    #[test]
    fn prop_smf_reader_never_panics() {
        proptest!(|(tail in proptest::collection::vec(any::<u8>(), 0..200), cut in 0usize..300)| {
            let mut f = [0u8; 300];
            f[..22].copy_from_slice(&file(&[])[..22]);
            f[18..22].copy_from_slice(&(tail.len() as u32).to_be_bytes());
            f[22..22 + tail.len()].copy_from_slice(&tail);
            let len = (22 + tail.len()).min(cut.max(14));
            if let Ok(r) = SmfReader::new(&f[..len]) {
                for t in r.tracks().take(4).flatten() {
                    let mut n = 0;
                    for e in t {
                        n += 1;
                        if e.is_err() {
                            break;
                        }
                        prop_assert!(n <= 200);
                    }
                }
            }
        });
    }
}
