//! Standard MIDI File 1.0 writer into a caller buffer (RP-001, revised February 1996).
//!
//! - `SmfWriter::new` writes the header chunk (p. 4) with ntrks 0; `finish` patches ntrks with
//!   the number of track chunks written. Format 0 has one track chunk only.
//! - `begin_track` opens a track chunk; `end_track` writes End of Track (p. 9) and patches the
//!   chunk length (big endian, no pad, p. 3).
//! - Each event gets a delta-time in the shortest variable-length form (p. 2 and 6).
//! - Running status (p. 6), on by default: the status of a channel message is omitted when it is
//!   the status of the last channel message of the track, with no sysex event or meta-event
//!   between (p. 7).
//! - Sysex events (p. 6 and 7): a complete F0 event, a message in packets (F0 then F7 events, the
//!   last one ends with F7, no channel message between), and the F7 escape.
//! - Meta-events (p. 8 to 10) are written with their defined length only. Sequence Number and
//!   Sequence/Track Name only at time 0; Sequence Number and SMPTE Offset also before any channel
//!   message or sysex event (AMB-RP-001-05). In format 1, Set Tempo, Time Signature and SMPTE
//!   Offset only in the first track (p. 5 and 9).
//!
//! No alloc (DEC-002). A call that fails leaves the buffer and the state as before the call.
//! Never panics.

use thiserror::Error;

use crate::smf_format::{
    is_known_meta_kind, write_vlq, ChannelMessage, Division, FileFormat, MetaEvent, TextKind, MTHD,
    MTRK,
};

/// Error of the SMF writer (thiserror 2, no_std).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Error)]
pub enum SmfWriteError {
    /// The buffer is too small for the next bytes.
    #[error("buffer too small")]
    BufferTooSmall,
    /// A format other than 0, 1 and 2.
    #[error("format other than 0, 1 and 2")]
    BadFormat,
    /// A value that the file cannot hold or that RP-001 does not allow.
    #[error("bad value")]
    BadValue,
    /// An event or End of Track with no open track chunk.
    #[error("no open track chunk")]
    NoTrackOpen,
    /// `begin_track` or `finish` while a track chunk is open.
    #[error("a track chunk is open")]
    TrackOpen,
    /// A second track chunk in a format 0 file.
    #[error("second track chunk in a format 0 file")]
    SecondTrackInFormat0,
    /// A channel message, an F0 event, an escape or End of Track in an open sysex message.
    #[error("sysex message in packets is open")]
    SysExOpen,
    /// A sysex packet with no open sysex message.
    #[error("no open sysex message")]
    NoSysExOpen,
    /// A Sequence Number, Sequence/Track Name or SMPTE Offset at a place RP-001 does not allow.
    #[error("meta-event at a place that RP-001 does not allow")]
    Misplaced,
    /// Set Tempo, Time Signature or SMPTE Offset outside the first track of a format 1 file.
    #[error("tempo map event outside the first track of a format 1 file")]
    TempoMapNotFirstTrack,
    /// `finish` with no track chunk.
    #[error("no track chunk")]
    NoTracks,
}

/// The variable-length form of a delta-time or a length.
fn vlq(value: usize) -> Result<([u8; 4], usize), SmfWriteError> {
    u32::try_from(value)
        .ok()
        .and_then(write_vlq)
        .ok_or(SmfWriteError::BadValue)
}

/// A Standard MIDI File writer into a caller buffer.
#[derive(Debug)]
pub struct SmfWriter<'a> {
    buf: &'a mut [u8],
    len: usize,
    format: FileFormat,
    tracks: u16,
    track_start: Option<usize>,
    running_status: bool,
    running: Option<u8>,
    sysex_open: bool,
    nonzero_delta: bool,
    transmitted: bool,
}

impl<'a> SmfWriter<'a> {
    /// Writes the header chunk: MThd, length 6, format, ntrks 0 (patched by `finish`) and the
    /// division. `BadFormat` for a format other than 0, 1 and 2, `BadValue` for ticks per
    /// quarter-note above 0x7FFF, `BufferTooSmall` for a buffer under 14 bytes.
    pub fn new(
        buf: &'a mut [u8],
        format: FileFormat,
        division: Division,
    ) -> Result<Self, SmfWriteError> {
        if matches!(format, FileFormat::Other(_)) {
            return Err(SmfWriteError::BadFormat);
        }
        let division = division.to_word().ok_or(SmfWriteError::BadValue)?;
        let mut w = Self {
            buf,
            len: 0,
            format,
            tracks: 0,
            track_start: None,
            running_status: true,
            running: None,
            sysex_open: false,
            nonzero_delta: false,
            transmitted: false,
        };
        w.put(&[
            &MTHD,
            &[0, 0, 0, 6],
            &format.to_word().to_be_bytes(),
            &[0, 0],
            &division.to_be_bytes(),
        ])?;
        Ok(w)
    }

    /// Running status on (the default) or off.
    pub fn set_running_status(&mut self, on: bool) {
        self.running_status = on;
    }

    /// The number of bytes written.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when no byte is written (never true after `new`).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Writes all parts, or nothing when the buffer is too small.
    fn put(&mut self, parts: &[&[u8]]) -> Result<(), SmfWriteError> {
        let total: usize = parts.iter().map(|p| p.len()).sum();
        let out = self
            .buf
            .get_mut(self.len..self.len + total)
            .ok_or(SmfWriteError::BufferTooSmall)?;
        let mut i = 0;
        for p in parts {
            out[i..i + p.len()].copy_from_slice(p);
            i += p.len();
        }
        self.len += total;
        Ok(())
    }

    /// The checks of every event: an open track chunk and a valid delta-time.
    fn event_start(&self, delta: u32) -> Result<([u8; 4], usize), SmfWriteError> {
        if self.track_start.is_none() {
            return Err(SmfWriteError::NoTrackOpen);
        }
        vlq(delta as usize)
    }

    /// The state after an event is written.
    fn after_event(&mut self, delta: u32, running: Option<u8>, transmitted: bool) {
        self.nonzero_delta |= delta != 0;
        self.transmitted |= transmitted;
        self.running = running;
    }

    /// Opens a track chunk. `TrackOpen` while one is open, `SecondTrackInFormat0` for a second
    /// track of a format 0 file, `BadValue` after 65,535 tracks.
    pub fn begin_track(&mut self) -> Result<(), SmfWriteError> {
        if self.track_start.is_some() {
            return Err(SmfWriteError::TrackOpen);
        }
        if self.format == FileFormat::Single && self.tracks >= 1 {
            return Err(SmfWriteError::SecondTrackInFormat0);
        }
        let tracks = self.tracks.checked_add(1).ok_or(SmfWriteError::BadValue)?;
        let start = self.len;
        self.put(&[&MTRK, &[0, 0, 0, 0]])?;
        self.tracks = tracks;
        self.track_start = Some(start);
        self.running = None;
        self.sysex_open = false;
        self.nonzero_delta = false;
        self.transmitted = false;
        Ok(())
    }

    /// Writes a channel message. The status is omitted when running status is on and the status
    /// is the same as that of the last channel message, with no sysex or meta event between.
    pub fn channel(&mut self, delta: u32, message: ChannelMessage) -> Result<(), SmfWriteError> {
        let (d, n) = self.event_start(delta)?;
        if self.sysex_open {
            return Err(SmfWriteError::SysExOpen);
        }
        let status = message.status();
        let omit = self.running_status && self.running == Some(status);
        let s = [status];
        self.put(&[&d[..n], if omit { &[] } else { &s }, message.data()])?;
        self.after_event(delta, Some(status), true);
        Ok(())
    }

    /// Writes a channel message from a UMP MIDI 1.0 Channel Voice word (M2-104-UM 7.3). The group
    /// is not written. `BadValue` for another word.
    pub fn channel_ump(&mut self, delta: u32, word: u32) -> Result<(), SmfWriteError> {
        let message = ChannelMessage::from_ump(word).ok_or(SmfWriteError::BadValue)?;
        self.channel(delta, message)
    }

    /// Writes an F0 or F7 sysex event, with its checks done by the caller.
    fn sysex_event(
        &mut self,
        delta: u32,
        code: u8,
        data: &[u8],
        open_after: bool,
    ) -> Result<(), SmfWriteError> {
        let (d, n) = self.event_start(delta)?;
        let (l, m) = vlq(data.len())?;
        self.put(&[&d[..n], &[code], &l[..m], data])?;
        self.after_event(delta, None, true);
        self.sysex_open = open_after;
        Ok(())
    }

    /// True when every byte is 0x00 to 0x7F, except a final F7 when `final_f7`.
    fn sysex_bytes_ok(data: &[u8], final_f7: bool) -> bool {
        let body = match data.split_last() {
            Some((&0xF7, body)) if final_f7 => body,
            _ => data,
        };
        body.iter().all(|&b| b <= 0x7F)
    }

    /// Writes a complete F0 sysex event. `data` is the message after F0; it ends with F7 and has no
    /// other byte above 0x7F (`BadValue`).
    pub fn sysex(&mut self, delta: u32, data: &[u8]) -> Result<(), SmfWriteError> {
        if self.sysex_open {
            return Err(SmfWriteError::SysExOpen);
        }
        if data.last() != Some(&0xF7) || !Self::sysex_bytes_ok(data, true) {
            return Err(SmfWriteError::BadValue);
        }
        self.sysex_event(delta, 0xF0, data, false)
    }

    /// Writes the first packet (an F0 event) of a message in packets. `data` does not end with F7
    /// and has no byte above 0x7F (`BadValue`). The message stays open.
    pub fn sysex_start(&mut self, delta: u32, data: &[u8]) -> Result<(), SmfWriteError> {
        if self.sysex_open {
            return Err(SmfWriteError::SysExOpen);
        }
        if !Self::sysex_bytes_ok(data, false) {
            return Err(SmfWriteError::BadValue);
        }
        self.sysex_event(delta, 0xF0, data, true)
    }

    /// Writes a packet (an F7 event) of the open message. A packet that ends with F7 closes the
    /// message. `NoSysExOpen` with no open message; `BadValue` for another byte above 0x7F.
    pub fn sysex_continue(&mut self, delta: u32, data: &[u8]) -> Result<(), SmfWriteError> {
        if !self.sysex_open {
            return Err(SmfWriteError::NoSysExOpen);
        }
        if !Self::sysex_bytes_ok(data, true) {
            return Err(SmfWriteError::BadValue);
        }
        self.sysex_event(delta, 0xF7, data, data.last() != Some(&0xF7))
    }

    /// Writes an F7 escape event with any bytes to transmit (p. 6). `SysExOpen` while a message
    /// in packets is open.
    pub fn escape(&mut self, delta: u32, bytes: &[u8]) -> Result<(), SmfWriteError> {
        if self.sysex_open {
            return Err(SmfWriteError::SysExOpen);
        }
        self.sysex_event(delta, 0xF7, bytes, false)
    }

    /// The placement rules of RP-001 for a meta-event at `delta` (REQ-SMF-META-005, -007).
    fn check_place(&self, delta: u32, event: &MetaEvent<'_>) -> Result<(), SmfWriteError> {
        let late = self.nonzero_delta || delta != 0;
        let misplaced = match event {
            MetaEvent::SequenceNumber(_) | MetaEvent::SmpteOffset(_) => late || self.transmitted,
            MetaEvent::Text {
                kind: TextKind::TrackName,
                ..
            } => late,
            _ => false,
        };
        if misplaced {
            return Err(SmfWriteError::Misplaced);
        }
        let tempo_map = matches!(
            event,
            MetaEvent::SetTempo(_) | MetaEvent::TimeSignature(_) | MetaEvent::SmpteOffset(_)
        );
        if tempo_map && self.format == FileFormat::MultiTrack && self.tracks > 1 {
            return Err(SmfWriteError::TempoMapNotFirstTrack);
        }
        Ok(())
    }

    /// Writes a meta-event with its defined length. `BadValue` for a value out of range
    /// (`MetaEvent::is_valid`) and for End of Track (use `end_track`); `Misplaced` and
    /// `TempoMapNotFirstTrack` for the placement rules of RP-001.
    pub fn meta(&mut self, delta: u32, event: &MetaEvent<'_>) -> Result<(), SmfWriteError> {
        let (d, n) = self.event_start(delta)?;
        if !event.is_valid() || *event == MetaEvent::EndOfTrack {
            return Err(SmfWriteError::BadValue);
        }
        self.check_place(delta, event)?;
        let data_len = event.data_len();
        let (l, m) = vlq(data_len)?;
        let head_len = n + 2 + m;
        let out = self
            .buf
            .get_mut(self.len..self.len + head_len + data_len)
            .ok_or(SmfWriteError::BufferTooSmall)?;
        out[..n].copy_from_slice(&d[..n]);
        out[n] = 0xFF;
        out[n + 1] = event.kind();
        out[n + 2..head_len].copy_from_slice(&l[..m]);
        event
            .write_data(&mut out[head_len..])
            .ok_or(SmfWriteError::BufferTooSmall)?;
        self.len += head_len + data_len;
        self.after_event(delta, None, false);
        Ok(())
    }

    /// Writes a meta-event from a type byte and data. `BadValue` for a type of 0x80 or more, for
    /// End of Track, and for a known type that does not decode or does not have its defined
    /// length. A known type then gets the checks of `meta`.
    pub fn meta_raw(&mut self, delta: u32, kind: u8, data: &[u8]) -> Result<(), SmfWriteError> {
        if kind >= 0x80 || kind == 0x2F {
            return Err(SmfWriteError::BadValue);
        }
        if is_known_meta_kind(kind) {
            let event = MetaEvent::decode(kind, data).ok_or(SmfWriteError::BadValue)?;
            if event.data_len() != data.len() {
                return Err(SmfWriteError::BadValue);
            }
            return self.meta(delta, &event);
        }
        let (d, n) = self.event_start(delta)?;
        let (l, m) = vlq(data.len())?;
        self.put(&[&d[..n], &[0xFF, kind], &l[..m], data])?;
        self.after_event(delta, None, false);
        Ok(())
    }

    /// Writes End of Track (FF 2F 00) and patches the length of the track chunk. `SysExOpen`
    /// while a message in packets is open.
    pub fn end_track(&mut self, delta: u32) -> Result<(), SmfWriteError> {
        let (d, n) = self.event_start(delta)?;
        if self.sysex_open {
            return Err(SmfWriteError::SysExOpen);
        }
        let start = self.track_start.ok_or(SmfWriteError::NoTrackOpen)?;
        let chunk_len =
            u32::try_from(self.len + n + 3 - (start + 8)).map_err(|_| SmfWriteError::BadValue)?;
        self.put(&[&d[..n], &[0xFF, 0x2F, 0x00]])?;
        self.buf[start + 4..start + 8].copy_from_slice(&chunk_len.to_be_bytes());
        self.track_start = None;
        Ok(())
    }

    /// Patches ntrks with the number of track chunks and gives the length of the file. `TrackOpen`
    /// while a track chunk is open, `NoTracks` with no track chunk.
    pub fn finish(&mut self) -> Result<usize, SmfWriteError> {
        if self.track_start.is_some() {
            return Err(SmfWriteError::TrackOpen);
        }
        if self.tracks == 0 {
            return Err(SmfWriteError::NoTracks);
        }
        self.buf[10..12].copy_from_slice(&self.tracks.to_be_bytes());
        Ok(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smf_examples::{FORMAT_0, FORMAT_1};
    use crate::smf_format::{SmpteOffset, TimeSignature};
    use crate::smf_reader::{Event, SmfReader};
    use proptest::prelude::*;

    const TPQ96: Division = Division::TicksPerQuarterNote(96);

    fn cm(s: u8, a: u8, b: u8) -> ChannelMessage {
        ChannelMessage::new(s, a, b).unwrap()
    }

    fn tempo_and_signature(w: &mut SmfWriter<'_>) {
        w.meta(0, &MetaEvent::TimeSignature(TimeSignature::DEFAULT))
            .unwrap();
        w.meta(0, &MetaEvent::SetTempo(500_000)).unwrap();
    }

    #[test]
    fn example_format_0_writes_with_running_status() {
        let mut buf = [0u8; 128];
        let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
        w.begin_track().unwrap();
        tempo_and_signature(&mut w);
        for (d, m) in [
            (0, cm(0xC0, 5, 0)),
            (0, cm(0xC1, 46, 0)),
            (0, cm(0xC2, 70, 0)),
            (0, cm(0x92, 48, 96)),
            (0, cm(0x92, 60, 96)),
            (96, cm(0x91, 67, 64)),
            (96, cm(0x90, 76, 32)),
            (192, cm(0x82, 48, 64)),
            (0, cm(0x82, 60, 64)),
            (0, cm(0x81, 67, 64)),
            (0, cm(0x80, 76, 64)),
        ] {
            w.channel(d, m).unwrap();
        }
        w.end_track(0).unwrap();
        let n = w.finish().unwrap();
        assert_eq!(&buf[..n], FORMAT_0);
    }

    #[test]
    fn example_format_1_writes() {
        let mut buf = [0u8; 160];
        let mut w = SmfWriter::new(&mut buf, FileFormat::MultiTrack, TPQ96).unwrap();
        w.begin_track().unwrap();
        tempo_and_signature(&mut w);
        w.end_track(384).unwrap();
        w.begin_track().unwrap();
        w.channel(0, cm(0xC0, 5, 0)).unwrap();
        w.channel(192, cm(0x90, 76, 32)).unwrap();
        w.channel(192, cm(0x90, 76, 0)).unwrap();
        w.end_track(0).unwrap();
        w.begin_track().unwrap();
        w.channel(0, cm(0xC1, 46, 0)).unwrap();
        w.channel(96, cm(0x91, 67, 64)).unwrap();
        w.channel(288, cm(0x91, 67, 0)).unwrap();
        w.end_track(0).unwrap();
        w.begin_track().unwrap();
        w.channel(0, cm(0xC2, 70, 0)).unwrap();
        w.channel(0, cm(0x92, 48, 96)).unwrap();
        w.channel(0, cm(0x92, 60, 96)).unwrap();
        w.channel(384, cm(0x92, 48, 0)).unwrap();
        w.channel(0, cm(0x92, 60, 0)).unwrap();
        w.end_track(0).unwrap();
        let n = w.finish().unwrap();
        assert_eq!(&buf[..n], FORMAT_1);
    }

    #[test]
    fn writer_running_status_off() {
        let mut buf = [0u8; 64];
        let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
        w.set_running_status(false);
        w.begin_track().unwrap();
        w.channel(0, cm(0x90, 60, 100)).unwrap();
        w.channel(0, cm(0x90, 60, 0)).unwrap();
        w.end_track(0).unwrap();
        let n = w.finish().unwrap();
        assert_eq!(
            &buf[22..n],
            &[0, 0x90, 60, 100, 0, 0x90, 60, 0, 0, 0xFF, 0x2F, 0]
        );
        // On: the status of the first message after a meta-event or sysex event is written.
        let mut buf = [0u8; 64];
        let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
        w.begin_track().unwrap();
        w.channel(0, cm(0x90, 60, 100)).unwrap();
        w.meta(
            0,
            &MetaEvent::Text {
                kind: TextKind::Marker,
                text: b"",
            },
        )
        .unwrap();
        w.channel(0, cm(0x90, 60, 0)).unwrap();
        w.sysex(0, &[0x7E, 0xF7]).unwrap();
        w.channel(0, cm(0x90, 61, 0)).unwrap();
        w.end_track(0).unwrap();
        let n = w.finish().unwrap();
        #[rustfmt::skip]
        assert_eq!(&buf[22..n], &[
            0, 0x90, 60, 100, 0, 0xFF, 0x06, 0, 0, 0x90, 60, 0,
            0, 0xF0, 2, 0x7E, 0xF7, 0, 0x90, 61, 0, 0, 0xFF, 0x2F, 0,
        ]);
    }

    #[test]
    fn writer_format_0_one_track() {
        let mut buf = [0u8; 64];
        let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
        assert_eq!(w.finish(), Err(SmfWriteError::NoTracks));
        w.begin_track().unwrap();
        assert_eq!(w.begin_track(), Err(SmfWriteError::TrackOpen));
        assert_eq!(w.finish(), Err(SmfWriteError::TrackOpen));
        w.end_track(0).unwrap();
        assert_eq!(w.begin_track(), Err(SmfWriteError::SecondTrackInFormat0));
        assert_eq!(w.finish(), Ok(26));
        assert_eq!(&buf[8..12], &[0, 0, 0, 1]);
        let mut buf = [0u8; 64];
        assert_eq!(
            SmfWriter::new(&mut buf, FileFormat::Other(3), TPQ96).err(),
            Some(SmfWriteError::BadFormat)
        );
        assert_eq!(
            SmfWriter::new(
                &mut buf,
                FileFormat::MultiSequence,
                Division::TicksPerQuarterNote(0x8000)
            )
            .err(),
            Some(SmfWriteError::BadValue)
        );
        assert_eq!(
            SmfWriter::new(&mut buf[..13], FileFormat::Single, TPQ96).err(),
            Some(SmfWriteError::BufferTooSmall)
        );
        // Format 2: several patterns, each with its tempo.
        let mut w = SmfWriter::new(&mut buf, FileFormat::MultiSequence, TPQ96).unwrap();
        for _ in 0..2 {
            w.begin_track().unwrap();
            w.meta(0, &MetaEvent::SetTempo(400_000)).unwrap();
            w.end_track(0).unwrap();
        }
        assert_eq!(w.finish(), Ok(14 + 2 * 19));
        assert_eq!(SmfReader::new(&buf[..52]).unwrap().tracks().count(), 2);
    }

    #[test]
    fn writer_writes_delta_before_every_event() {
        let mut buf = [0u8; 64];
        let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
        assert_eq!(
            w.channel(0, cm(0x90, 1, 1)),
            Err(SmfWriteError::NoTrackOpen)
        );
        assert_eq!(w.end_track(0), Err(SmfWriteError::NoTrackOpen));
        w.begin_track().unwrap();
        w.channel(0x0FFF_FFFF, cm(0x90, 1, 1)).unwrap();
        assert_eq!(
            w.channel(0x1000_0000, cm(0x90, 1, 1)),
            Err(SmfWriteError::BadValue)
        );
        w.end_track(0x80).unwrap();
        let n = w.finish().unwrap();
        assert_eq!(
            &buf[22..n],
            &[0xFF, 0xFF, 0xFF, 0x7F, 0x90, 1, 1, 0x81, 0x00, 0xFF, 0x2F, 0]
        );
    }

    #[test]
    fn chunk_length_big_endian_without_pad() {
        let mut buf = [0u8; 400];
        let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
        w.begin_track().unwrap();
        w.meta(
            0,
            &MetaEvent::Text {
                kind: TextKind::Text,
                text: &[b'a'; 290],
            },
        )
        .unwrap();
        w.end_track(0).unwrap();
        let n = w.finish().unwrap();
        // 1 + 2 + 2 (length 290 = 82 22) + 290 + 4 = 299 = 0x012B, odd, not padded.
        assert_eq!(&buf[14..22], b"MTrk\0\0\x01\x2B");
        assert_eq!(n, 22 + 299);
        assert_eq!(&buf[22..26], &[0, 0xFF, 0x01, 0x82]);
    }

    #[test]
    fn writer_takes_ump_word() {
        let mut buf = [0u8; 64];
        let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
        w.begin_track().unwrap();
        w.channel_ump(0, 0x2592_3060).unwrap();
        w.channel_ump(0, 0x25C1_2E00).unwrap();
        assert_eq!(w.channel_ump(0, 0x4092_3060), Err(SmfWriteError::BadValue));
        assert_eq!(w.channel_ump(0, 0x2012_3060), Err(SmfWriteError::BadValue));
        w.end_track(0).unwrap();
        let n = w.finish().unwrap();
        assert_eq!(
            &buf[22..n],
            &[0, 0x92, 0x30, 0x60, 0, 0xC1, 0x2E, 0, 0xFF, 0x2F, 0]
        );
    }

    #[test]
    fn writer_rejects_channel_message_inside_sysex_packets() {
        let mut buf = [0u8; 128];
        let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
        w.begin_track().unwrap();
        assert_eq!(w.sysex_continue(0, &[1]), Err(SmfWriteError::NoSysExOpen));
        assert_eq!(w.sysex(0, &[0x43, 0x12]), Err(SmfWriteError::BadValue));
        assert_eq!(w.sysex(0, &[0x80, 0xF7]), Err(SmfWriteError::BadValue));
        assert_eq!(
            w.sysex_start(0, &[0x43, 0xF7]),
            Err(SmfWriteError::BadValue)
        );
        // Page 7: F0 03 43 12 00 / 81 48 / F7 06 43 12 00 43 12 00 / 64 / F7 04 43 12 00 F7.
        w.sysex_start(0, &[0x43, 0x12, 0x00]).unwrap();
        assert_eq!(w.channel(0, cm(0x90, 1, 1)), Err(SmfWriteError::SysExOpen));
        assert_eq!(w.sysex(0, &[0xF7]), Err(SmfWriteError::SysExOpen));
        assert_eq!(w.escape(0, &[0xF8]), Err(SmfWriteError::SysExOpen));
        assert_eq!(w.end_track(0), Err(SmfWriteError::SysExOpen));
        w.sysex_continue(200, &[0x43, 0x12, 0x00, 0x43, 0x12, 0x00])
            .unwrap();
        w.sysex_continue(100, &[0x43, 0x12, 0x00, 0xF7]).unwrap();
        w.escape(0, &[0xF8]).unwrap();
        w.end_track(0).unwrap();
        let n = w.finish().unwrap();
        #[rustfmt::skip]
        assert_eq!(&buf[22..n], &[
            0x00, 0xF0, 0x03, 0x43, 0x12, 0x00,
            0x81, 0x48, 0xF7, 0x06, 0x43, 0x12, 0x00, 0x43, 0x12, 0x00,
            0x64, 0xF7, 0x04, 0x43, 0x12, 0x00, 0xF7,
            0x00, 0xF7, 0x01, 0xF8,
            0x00, 0xFF, 0x2F, 0x00,
        ]);
    }

    #[test]
    fn writer_rejects_meta_type_above_7f() {
        let mut buf = [0u8; 64];
        let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
        w.begin_track().unwrap();
        assert_eq!(w.meta_raw(0, 0x80, &[]), Err(SmfWriteError::BadValue));
        assert_eq!(w.meta_raw(0, 0x2F, &[]), Err(SmfWriteError::BadValue));
        assert_eq!(
            w.meta(0, &MetaEvent::EndOfTrack),
            Err(SmfWriteError::BadValue)
        );
        w.meta_raw(0, 0x60, &[1, 2]).unwrap();
        w.end_track(0).unwrap();
        let n = w.finish().unwrap();
        assert_eq!(&buf[22..n], &[0, 0xFF, 0x60, 2, 1, 2, 0, 0xFF, 0x2F, 0]);
    }

    #[test]
    fn writer_writes_defined_length() {
        let mut buf = [0u8; 64];
        let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
        w.begin_track().unwrap();
        // A known type with more data than defined, or less, is refused.
        assert_eq!(
            w.meta_raw(0, 0x51, &[7, 0xA1, 0x20, 0]),
            Err(SmfWriteError::BadValue)
        );
        assert_eq!(
            w.meta_raw(0, 0x51, &[7, 0xA1]),
            Err(SmfWriteError::BadValue)
        );
        assert_eq!(w.meta_raw(0, 0x20, &[16]), Err(SmfWriteError::BadValue));
        assert_eq!(
            w.meta(0, &MetaEvent::ChannelPrefix(16)),
            Err(SmfWriteError::BadValue)
        );
        w.meta_raw(0, 0x51, &[7, 0xA1, 0x20]).unwrap();
        w.meta(0, &MetaEvent::ChannelPrefix(15)).unwrap();
        w.end_track(0).unwrap();
        let n = w.finish().unwrap();
        #[rustfmt::skip]
        assert_eq!(&buf[22..n], &[0, 0xFF, 0x51, 3, 7, 0xA1, 0x20, 0, 0xFF, 0x20, 1, 15, 0, 0xFF, 0x2F, 0]);
    }

    #[test]
    fn writer_ends_track_with_end_of_track() {
        let mut buf = [0u8; 64];
        let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
        w.begin_track().unwrap();
        w.end_track(5).unwrap();
        assert_eq!(
            w.channel(0, cm(0x90, 1, 1)),
            Err(SmfWriteError::NoTrackOpen)
        );
        let n = w.finish().unwrap();
        assert_eq!(&buf[14..n], b"MTrk\0\0\0\x04\x05\xFF\x2F\x00");
        // A failed call leaves the buffer and the state as before.
        let mut small = [0u8; 25];
        let mut w = SmfWriter::new(&mut small, FileFormat::Single, TPQ96).unwrap();
        w.begin_track().unwrap();
        assert_eq!(w.end_track(0), Err(SmfWriteError::BufferTooSmall));
        assert_eq!(w.len(), 22);
        assert!(!w.is_empty());
        assert_eq!(w.channel(0, cm(0xC0, 1, 0)), Ok(()));
        assert_eq!(
            w.channel(0, cm(0xC0, 2, 0)),
            Err(SmfWriteError::BufferTooSmall)
        );
        assert_eq!(w.len(), 25);
    }

    #[test]
    fn writer_rejects_misplaced_sequence_number_name_and_offset() {
        let offset = MetaEvent::SmpteOffset(SmpteOffset {
            hour: 0x20,
            minute: 0,
            second: 0,
            frame: 0,
            fraction: 0,
        });
        let name = MetaEvent::Text {
            kind: TextKind::TrackName,
            text: b"Bass",
        };
        let mut buf = [0u8; 128];
        let mut w = SmfWriter::new(&mut buf, FileFormat::MultiTrack, TPQ96).unwrap();
        w.begin_track().unwrap();
        w.meta(0, &MetaEvent::SequenceNumber(1)).unwrap();
        w.meta(0, &name).unwrap();
        w.meta(0, &offset).unwrap();
        assert_eq!(
            w.meta(1, &MetaEvent::SequenceNumber(1)),
            Err(SmfWriteError::Misplaced)
        );
        w.end_track(0).unwrap();
        w.begin_track().unwrap();
        w.channel(0, cm(0x90, 1, 1)).unwrap();
        w.meta(0, &name).unwrap();
        assert_eq!(
            w.meta(0, &MetaEvent::SequenceNumber(2)),
            Err(SmfWriteError::Misplaced)
        );
        assert_eq!(w.meta_raw(0, 0x00, &[0, 2]), Err(SmfWriteError::Misplaced));
        w.channel(10, cm(0x90, 1, 0)).unwrap();
        assert_eq!(w.meta(0, &name), Err(SmfWriteError::Misplaced));
        w.end_track(0).unwrap();
        w.begin_track().unwrap();
        w.escape(0, &[0xFA]).unwrap();
        assert_eq!(
            w.meta(0, &MetaEvent::SequenceNumber(3)),
            Err(SmfWriteError::Misplaced)
        );
        w.meta(
            0,
            &MetaEvent::Text {
                kind: TextKind::Lyric,
                text: b"la",
            },
        )
        .unwrap();
        w.end_track(0).unwrap();
        assert!(w.finish().is_ok());
    }

    #[test]
    fn writer_rejects_tempo_map_outside_first_track_in_format_1() {
        let mut buf = [0u8; 128];
        let mut w = SmfWriter::new(&mut buf, FileFormat::MultiTrack, TPQ96).unwrap();
        w.begin_track().unwrap();
        w.meta(0, &MetaEvent::SetTempo(500_000)).unwrap();
        w.meta(96, &MetaEvent::SetTempo(400_000)).unwrap();
        w.end_track(0).unwrap();
        w.begin_track().unwrap();
        assert_eq!(
            w.meta(0, &MetaEvent::SetTempo(1)),
            Err(SmfWriteError::TempoMapNotFirstTrack)
        );
        assert_eq!(
            w.meta(0, &MetaEvent::TimeSignature(TimeSignature::DEFAULT)),
            Err(SmfWriteError::TempoMapNotFirstTrack)
        );
        assert_eq!(
            w.meta_raw(0, 0x54, &[0, 0, 0, 0, 0]),
            Err(SmfWriteError::TempoMapNotFirstTrack)
        );
        w.end_track(0).unwrap();
        assert!(w.finish().is_ok());
    }

    #[derive(Debug, Clone)]
    enum Ev {
        Ch(u8, u8, u8),
        Sx(Vec<u8>),
        Text(Vec<u8>),
        Tempo(u32),
    }

    fn ev() -> impl Strategy<Value = (u32, Ev)> {
        let e = prop_oneof![
            (0x80u8..=0xEF, 0u8..=0x7F, 0u8..=0x7F).prop_map(|(s, a, b)| Ev::Ch(s, a, b)),
            proptest::collection::vec(0u8..=0x7F, 0..20).prop_map(Ev::Sx),
            proptest::collection::vec(any::<u8>(), 0..20).prop_map(Ev::Text),
            (0u32..=0xFF_FFFF).prop_map(Ev::Tempo),
        ];
        (prop_oneof![Just(0u32), 0u32..200, 0u32..=0x0FFF_FFFF], e)
    }

    extern crate std;
    use std::vec::Vec;

    #[test]
    fn prop_write_read_round_trip() {
        proptest!(|(events in proptest::collection::vec(ev(), 0..30), rs in any::<bool>())| {
            let mut buf = [0u8; 2048];
            let mut w = SmfWriter::new(&mut buf, FileFormat::Single, TPQ96).unwrap();
            w.set_running_status(rs);
            w.begin_track().unwrap();
            for (d, e) in &events {
                match e {
                    Ev::Ch(s, a, b) => w.channel(*d, cm(*s, *a, *b)).unwrap(),
                    Ev::Sx(v) => {
                        let mut m = v.clone();
                        m.push(0xF7);
                        w.sysex(*d, &m).unwrap()
                    }
                    Ev::Text(t) => w.meta(*d, &MetaEvent::Text { kind: TextKind::Text, text: t }).unwrap(),
                    Ev::Tempo(t) => w.meta(*d, &MetaEvent::SetTempo(*t)).unwrap(),
                }
            }
            w.end_track(7).unwrap();
            let n = w.finish().unwrap();
            let r = SmfReader::new(&buf[..n]).unwrap();
            let mut t = r.tracks().next().unwrap().unwrap();
            let mut ticks = 0u64;
            for (d, e) in &events {
                let got = t.next().unwrap().unwrap();
                ticks += u64::from(*d);
                prop_assert_eq!((got.delta, got.ticks), (*d, ticks));
                match (e, got.event) {
                    (Ev::Ch(s, a, b), Event::Channel(m)) => prop_assert_eq!(m, cm(*s, *a, *b)),
                    (Ev::Sx(v), Event::SysEx { data, .. }) => {
                        prop_assert_eq!(&data[..data.len() - 1], &v[..]);
                    }
                    (Ev::Text(v), ev) => prop_assert_eq!(
                        ev.meta(),
                        Some(MetaEvent::Text { kind: TextKind::Text, text: v })
                    ),
                    (Ev::Tempo(v), ev) => prop_assert_eq!(ev.meta(), Some(MetaEvent::SetTempo(*v))),
                    (e, g) => prop_assert!(false, "{:?} read as {:?}", e, g),
                }
            }
            let end = t.next().unwrap().unwrap();
            prop_assert_eq!(end.event.meta(), Some(MetaEvent::EndOfTrack));
            prop_assert!(t.next().is_none());
        });
    }
}
