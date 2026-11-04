//! Typed MIDI 1.0 messages on bytes, with no UMP Group in the API (M1 v4.2.1; DEC-022).
//!
//! - [`Midi1Reader`] reads bytes with the rules of [`BytesToUmpCore`] (REQ-BS-004 to
//!   REQ-BS-008, REQ-BS-012 to REQ-BS-015) and gives typed messages. Inside, a message is the
//!   UMP word of Group 1 (DEC-022, Q5); the API shows no Group.
//! - System Exclusive (M1 p. 34): the caller gives the buffer on each call, as the translator of
//!   [`BytesToUmpCore::push_with`], so a C program can give its own storage. The reader gives
//!   each complete message from F0H to F7H; a message that another status byte ends (M1 EOX)
//!   gets F7H. A System Real Time byte inside the message is given at once (M1 p. 30).
//! - System Reset (FFH) does not reset the reader (REQ-BS-005); a receiver that implements
//!   System Reset calls [`Midi1Reader::reset`] as part of its reset (M1 p. 33, item 6,
//!   AMB-M1-13).
//! - [`Midi1Writer`] writes typed messages with the rules of [`UmpToBytes`].
//!
//! No alloc (DEC-002), no I/O (DEC-003). Never panics.

use zmidi_ump::{Group, Midi1ChannelVoiceMessage, SystemMessage};

use crate::byte_stream::{BytesToUmpCore, UmpToBytes};

/// System Exclusive start (F0H) and EOX (F7H).
const SOX: u8 = 0xF0;
const EOX: u8 = 0xF7;
/// SysEx7 Status values of the reader core (M2-104 7.7, Table 13).
const SYSEX_COMPLETE: u32 = 0;
const SYSEX_START: u32 = 1;
const SYSEX_END: u32 = 3;

/// A typed MIDI 1.0 message with no Group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Midi1Message<'a> {
    /// A Channel Voice or Channel Mode message.
    ChannelVoice(Midi1ChannelVoiceMessage),
    /// A System Common or System Real Time message.
    System(SystemMessage),
    /// A complete System Exclusive message: F0H, the data bytes, F7H.
    SysEx(&'a [u8]),
}

/// What [`Midi1Reader::feed`] gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Midi1Event<'a> {
    /// A complete message.
    Message(Midi1Message<'a>),
    /// A System Exclusive message longer than the buffer of the caller; its bytes are discarded.
    SysExTooLong {
        /// Full length of the message, F0H and F7H included.
        length: usize,
    },
}

/// Error of the typed writer (thiserror 2, no_std).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MessageError {
    /// The output slice cannot hold the bytes of the message; the state did not change.
    #[error("output buffer too small")]
    BufferTooSmall,
    /// The System Exclusive message does not start with F0H, does not end with F7H, or has a
    /// data byte above 7FH; nothing was written.
    #[error("invalid System Exclusive message")]
    InvalidSysEx,
}

/// Typed MIDI 1.0 byte reader (REQ-MSG-001 to REQ-MSG-003). Fixed size; the System Exclusive
/// storage belongs to the caller.
#[derive(Debug, Clone)]
pub struct Midi1Reader {
    /// The byte rules (module ByteStream).
    core: BytesToUmpCore,
    /// Bytes of the System Exclusive message in progress, F0H included; can be above the buffer
    /// length after an overflow.
    sysex_len: usize,
}

impl Default for Midi1Reader {
    fn default() -> Self {
        Self::new()
    }
}

impl Midi1Reader {
    /// A reader with no running status and no message in progress.
    pub const fn new() -> Self {
        Self {
            core: BytesToUmpCore::new(Group::from_bits(0)),
            sysex_len: 0,
        }
    }

    /// Clears the running status, a message in progress and a System Exclusive message in
    /// progress: the System Reset action of the application (M1 p. 33, item 6; AMB-M1-13). The
    /// reader does not do this when it reads FFH (REQ-BS-005).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Takes one received byte and gives each event that it completes to `f`, in order, at most
    /// two (a System Exclusive message that a status byte ends, then that message). `sysex` is
    /// the System Exclusive buffer: give the same buffer on each call while a message is in
    /// progress. A message longer than `sysex` gives [`Midi1Event::SysExTooLong`]. Never panics.
    pub fn feed(&mut self, byte: u8, sysex: &mut [u8], mut f: impl FnMut(Midi1Event<'_>)) {
        let len = &mut self.sysex_len;
        self.core.feed(byte, |w| {
            let (w0, w1) = (
                w.first().copied().unwrap_or(0),
                w.get(1).copied().unwrap_or(0),
            );
            match w0 >> 28 {
                0x2 => {
                    if let Some(m) = Midi1ChannelVoiceMessage::parse(w) {
                        f(Midi1Event::Message(Midi1Message::ChannelVoice(m)));
                    }
                }
                0x1 => {
                    if let Some(m) = SystemMessage::parse(w) {
                        f(Midi1Event::Message(Midi1Message::System(m)));
                    }
                }
                0x3 => {
                    let status = (w0 >> 20) & 0xF;
                    let count = ((w0 >> 16) & 0xF).min(6) as usize;
                    let first = status == SYSEX_COMPLETE || status == SYSEX_START;
                    if first {
                        *len = 0;
                    }
                    let mut put = |b: u8| {
                        if let Some(slot) = sysex.get_mut(*len) {
                            *slot = b;
                        }
                        *len = len.saturating_add(1);
                    };
                    if first {
                        put(SOX);
                    }
                    for i in 0..count {
                        let b = if i < 2 {
                            w0 >> (8 - 8 * i)
                        } else {
                            w1 >> (24 - 8 * (i - 2))
                        };
                        put(b as u8);
                    }
                    if status == SYSEX_COMPLETE || status == SYSEX_END {
                        put(EOX);
                        let length = core::mem::take(len);
                        match sysex.get(..length) {
                            Some(bytes) => f(Midi1Event::Message(Midi1Message::SysEx(bytes))),
                            None => f(Midi1Event::SysExTooLong { length }),
                        }
                    }
                }
                _ => {}
            }
        });
    }
}

/// Typed MIDI 1.0 byte writer (REQ-MSG-004, REQ-MSG-005). Fixed size; no alloc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Midi1Writer {
    /// The byte rules and the running status (module ByteStream).
    inner: UmpToBytes,
}

impl Midi1Writer {
    /// A writer, with or without running status (M1 Running Status).
    pub const fn new(running_status: bool) -> Self {
        Self {
            inner: UmpToBytes::new(running_status),
        }
    }

    /// The next Channel Voice or Channel Mode message writes its status byte (Appendix A-2).
    pub fn refresh_status(&mut self) {
        self.inner.refresh_status();
    }

    /// Gives the bytes of `message` to `f`, in order, and keeps the running status. Gives
    /// [`MessageError::InvalidSysEx`] before any byte for a System Exclusive message that does
    /// not start with F0H, does not end with F7H or has a data byte above 7FH. After a System
    /// Exclusive message the next Channel Voice or Channel Mode status is written (M1 Running
    /// Status). Never panics.
    pub fn write_each(
        &mut self,
        message: Midi1Message<'_>,
        mut f: impl FnMut(u8),
    ) -> Result<(), MessageError> {
        match message {
            Midi1Message::ChannelVoice(m) => self.inner.write_each(&[m.into_bits()], f),
            Midi1Message::System(m) => self.inner.write_each(&[m.into_bits()], f),
            Midi1Message::SysEx(bytes) => {
                let [SOX, data @ .., EOX] = bytes else {
                    return Err(MessageError::InvalidSysEx);
                };
                if data.iter().any(|&b| b > 0x7F) {
                    return Err(MessageError::InvalidSysEx);
                }
                bytes.iter().for_each(|&b| f(b));
                self.inner.refresh_status();
            }
        }
        Ok(())
    }

    /// Writes the bytes of `message` into `out` and gives the count. Gives
    /// [`MessageError::BufferTooSmall`] with no state change when `out` is too small; the content
    /// of `out` is then not defined. Never panics.
    pub fn write(
        &mut self,
        message: Midi1Message<'_>,
        out: &mut [u8],
    ) -> Result<usize, MessageError> {
        let before = self.inner.clone();
        let (mut at, mut over) = (0usize, false);
        self.write_each(message, |b| match out.get_mut(at) {
            Some(slot) => {
                *slot = b;
                at += 1;
            }
            None => over = true,
        })?;
        if over {
            self.inner = before;
            return Err(MessageError::BufferTooSmall);
        }
        Ok(at)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;
    use zmidi_ump::{
        Channel, Midi1ControlChange, Midi1NoteOn, Midi1PitchBend, SongPositionPointer, TimingClock,
        TuneRequest,
    };

    /// An owned event, for comparisons.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Owned {
        Cv(Midi1ChannelVoiceMessage),
        Sys(SystemMessage),
        SysEx(Vec<u8>),
        TooLong(usize),
    }

    fn owned(e: Midi1Event<'_>) -> Owned {
        match e {
            Midi1Event::Message(Midi1Message::ChannelVoice(m)) => Owned::Cv(m),
            Midi1Event::Message(Midi1Message::System(m)) => Owned::Sys(m),
            Midi1Event::Message(Midi1Message::SysEx(b)) => Owned::SysEx(b.to_vec()),
            Midi1Event::SysExTooLong { length } => Owned::TooLong(length),
        }
    }

    fn read_with(reader: &mut Midi1Reader, bytes: &[u8], cap: usize) -> Vec<Owned> {
        let mut buf = std::vec![0u8; cap];
        let mut out = Vec::new();
        for &b in bytes {
            reader.feed(b, &mut buf, |e| out.push(owned(e)));
        }
        out
    }

    fn read(bytes: &[u8]) -> Vec<Owned> {
        read_with(&mut Midi1Reader::new(), bytes, 64)
    }

    fn g0() -> Group {
        Group::from_bits(0)
    }

    fn ch(n: u8) -> Channel {
        Channel::from_bits(n)
    }

    fn cv(m: impl Into<Midi1ChannelVoiceMessage>) -> Owned {
        Owned::Cv(m.into())
    }

    fn note_on(c: u8, note: u8, vel: u8) -> Owned {
        Owned::Cv(Midi1ChannelVoiceMessage::NoteOn(
            Midi1NoteOn::try_new(g0(), ch(c), note, vel).unwrap(),
        ))
    }

    fn clock() -> Owned {
        Owned::Sys(SystemMessage::TimingClock(TimingClock::new(g0())))
    }

    #[test]
    fn reader_gives_typed_channel_voice_and_system_messages() {
        // M1 Running Status: 90 3C 40 then 3E 40 is a second Note On; F8 inside a message.
        let got = read(&[
            0x90, 0x3C, 0xF8, 0x40, 0x3E, 0x40, 0xB2, 0x07, 0x64, 0xF2, 0x10, 0x01,
        ]);
        let cc = Midi1ControlChange::try_new(g0(), ch(2), 7, 100).unwrap();
        let spp = SongPositionPointer::try_new(g0(), 0x90).unwrap();
        assert_eq!(
            got,
            [
                clock(),
                note_on(0, 0x3C, 0x40),
                note_on(0, 0x3E, 0x40),
                cv(Midi1ChannelVoiceMessage::ControlChange(cc)),
                Owned::Sys(SystemMessage::SongPositionPointer(spp)),
            ]
        );
        // Data with no status is ignored; F9H and FDH give nothing (REQ-BS-012, REQ-BS-014).
        assert_eq!(read(&[0x40, 0x41, 0xF9, 0xFD]), []);
    }

    #[test]
    fn prop_reader_gives_the_messages_of_bytes_to_ump() {
        // REQ-MSG-001, REQ-MSG-002: the typed events are the UMPs of BytesToUmpCore, typed,
        // with each SysEx7 run joined from F0H to F7H.
        proptest!(|(bytes in proptest::collection::vec(any::<u8>(), 0..64))| {
            let mut core = BytesToUmpCore::new(g0());
            let mut want = Vec::new();
            let mut sysex = Vec::new();
            for &b in &bytes {
                core.feed(b, |w| match w[0] >> 28 {
                    0x2 => want.push(Owned::Cv(Midi1ChannelVoiceMessage::parse(w).unwrap())),
                    0x1 => want.push(Owned::Sys(SystemMessage::parse(w).unwrap())),
                    0x3 => {
                        let status = (w[0] >> 20) & 0xF;
                        if status == 0 || status == 1 {
                            sysex = std::vec![SOX];
                        }
                        let n = ((w[0] >> 16) & 0xF) as usize;
                        let all = [(w[0] >> 8) as u8, w[0] as u8, (w[1] >> 24) as u8,
                            (w[1] >> 16) as u8, (w[1] >> 8) as u8, w[1] as u8];
                        sysex.extend_from_slice(&all[..n]);
                        if status == 0 || status == 3 {
                            sysex.push(EOX);
                            want.push(Owned::SysEx(core::mem::take(&mut sysex)));
                        }
                    }
                    _ => unreachable!(),
                });
            }
            prop_assert_eq!(read_with(&mut Midi1Reader::new(), &bytes, 128), want);
        });
    }

    #[test]
    fn reader_gives_sysex_from_f0_to_f7() {
        // M1 p. 34: F0H, data, F7H; an empty message too.
        let msg = [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0x10, 0x20, 0x30, 0x40, 0xF7];
        assert_eq!(read(&msg), [Owned::SysEx(msg.to_vec())]);
        assert_eq!(read(&[0xF0, 0xF7]), [Owned::SysEx(std::vec![0xF0, 0xF7])]);
        // A lone F7H is ignored.
        assert_eq!(read(&[0xF7]), []);
    }

    #[test]
    fn reader_adds_f7_when_a_status_ends_sysex() {
        // M1 EOX (REQ-BS-006): a status byte ends the message; F6H gives two events.
        assert_eq!(
            read(&[0xF0, 0x01, 0x02, 0x90, 0x3C, 0x40]),
            [
                Owned::SysEx(std::vec![0xF0, 0x01, 0x02, 0xF7]),
                note_on(0, 0x3C, 0x40)
            ]
        );
        assert_eq!(
            read(&[0xF0, 0x01, 0xF6]),
            [
                Owned::SysEx(std::vec![0xF0, 0x01, 0xF7]),
                Owned::Sys(SystemMessage::TuneRequest(TuneRequest::new(g0()))),
            ]
        );
    }

    #[test]
    fn reader_gives_real_time_inside_sysex() {
        // M1 p. 30: Real Time bytes come anywhere and stay out of the message.
        let got = read(&[
            0xF0, 0x01, 0xF8, 0x02, 0x03, 0x04, 0x05, 0x06, 0xF8, 0x07, 0xF7,
        ]);
        assert_eq!(
            got,
            [
                clock(),
                clock(),
                Owned::SysEx(std::vec![0xF0, 1, 2, 3, 4, 5, 6, 7, 0xF7]),
            ]
        );
    }

    #[test]
    fn reader_reports_sysex_too_long() {
        // The buffer holds 4 bytes: F0 01 02 F7 fits, F0 01 02 03 F7 does not.
        let mut r = Midi1Reader::new();
        assert_eq!(
            read_with(&mut r, &[0xF0, 1, 2, 0xF7], 4),
            [Owned::SysEx(std::vec![0xF0, 1, 2, 0xF7])]
        );
        assert_eq!(
            read_with(&mut r, &[0xF0, 1, 2, 3, 0xF7], 4),
            [Owned::TooLong(5)]
        );
        // A long message: full length; the next message reads normally.
        let mut long = std::vec![0xF0];
        long.extend(core::iter::repeat_n(0x11, 300));
        long.push(0xF7);
        assert_eq!(read_with(&mut r, &long, 4), [Owned::TooLong(302)]);
        assert_eq!(
            read_with(&mut r, &[0xF0, 0xF7], 4),
            [Owned::SysEx(std::vec![0xF0, 0xF7])]
        );
        // No buffer: every SysEx is too long; other messages still read.
        assert_eq!(
            read_with(&mut r, &[0xF0, 0xF7, 0x91, 1, 2], 0),
            [Owned::TooLong(2), note_on(1, 1, 2)]
        );
    }

    #[test]
    fn reset_clears_running_status_and_sysex() {
        let mut r = Midi1Reader::new();
        let mut buf = [0u8; 16];
        let mut got = Vec::new();
        // Running status 90H, then reset: 3C 40 is data with no status.
        for b in [0x90, 0x3C, 0x40] {
            r.feed(b, &mut buf, |e| got.push(owned(e)));
        }
        r.reset();
        for b in [0x3C, 0x40] {
            r.feed(b, &mut buf, |e| got.push(owned(e)));
        }
        assert_eq!(got, [note_on(0, 0x3C, 0x40)]);
        // A SysEx in progress is dropped: its data is ignored, F7H alone gives nothing.
        let mut got = Vec::new();
        for b in [0xF0, 0x01] {
            r.feed(b, &mut buf, |e| got.push(owned(e)));
        }
        r.reset();
        for b in [0x02, 0xF7] {
            r.feed(b, &mut buf, |e| got.push(owned(e)));
        }
        assert_eq!(got, []);
    }

    #[test]
    fn reader_does_not_reset_at_ffh() {
        // REQ-BS-005: FFH is a Real Time message; the running status stays.
        let got = read(&[0x90, 0x3C, 0x40, 0xFF, 0x3E, 0x40]);
        let reset = Owned::Sys(SystemMessage::Reset(zmidi_ump::Reset::new(g0())));
        assert_eq!(got, [note_on(0, 0x3C, 0x40), reset, note_on(0, 0x3E, 0x40)]);
    }

    fn bytes_of(w: &mut Midi1Writer, m: Midi1Message<'_>) -> Vec<u8> {
        let mut out = Vec::new();
        w.write_each(m, |b| out.push(b)).unwrap();
        out
    }

    fn on(c: u8, note: u8, vel: u8) -> Midi1Message<'static> {
        Midi1Message::ChannelVoice(Midi1ChannelVoiceMessage::NoteOn(
            Midi1NoteOn::try_new(g0(), ch(c), note, vel).unwrap(),
        ))
    }

    #[test]
    fn writer_writes_typed_messages_with_running_status() {
        // M1 Running Status: the second Note On omits 90H; Real Time does not change it.
        let mut w = Midi1Writer::new(true);
        let clock = Midi1Message::System(SystemMessage::TimingClock(TimingClock::new(g0())));
        assert_eq!(bytes_of(&mut w, on(0, 0x3C, 0x40)), [0x90, 0x3C, 0x40]);
        assert_eq!(bytes_of(&mut w, clock), [0xF8]);
        assert_eq!(bytes_of(&mut w, on(0, 0x3E, 0x40)), [0x3E, 0x40]);
        w.refresh_status();
        assert_eq!(bytes_of(&mut w, on(0, 0x3E, 0)), [0x90, 0x3E, 0x00]);
        let pb = Midi1PitchBend::try_new(g0(), ch(1), 0x2000).unwrap();
        let pb = Midi1Message::ChannelVoice(Midi1ChannelVoiceMessage::PitchBend(pb));
        assert_eq!(bytes_of(&mut w, pb), [0xE1, 0x00, 0x40]);
        // Without running status every status is written.
        let mut w = Midi1Writer::new(false);
        assert_eq!(bytes_of(&mut w, on(0, 1, 2)), [0x90, 1, 2]);
        assert_eq!(bytes_of(&mut w, on(0, 1, 2)), [0x90, 1, 2]);
    }

    #[test]
    fn writer_buffer_too_small_keeps_state() {
        let mut w = Midi1Writer::new(true);
        let mut out = [0u8; 2];
        assert_eq!(
            w.write(on(0, 1, 2), &mut out),
            Err(MessageError::BufferTooSmall)
        );
        // The failed write did not set the running status: 90H is still written.
        let mut out = [0u8; 3];
        assert_eq!(w.write(on(0, 1, 2), &mut out), Ok(3));
        assert_eq!(out, [0x90, 1, 2]);
        let sysex = Midi1Message::SysEx(&[0xF0, 1, 2, 0xF7]);
        assert_eq!(w.write(sysex, &mut out), Err(MessageError::BufferTooSmall));
        // The SysEx did not clear the running status.
        assert_eq!(w.write(on(0, 3, 4), &mut out), Ok(2));
        assert_eq!(out[..2], [3, 4]);
    }

    #[test]
    fn writer_writes_sysex_and_clears_running_status() {
        // M1 p. 5: System Exclusive clears the running status.
        let mut w = Midi1Writer::new(true);
        let msg = [0xF0, 0x7E, 0x7F, 0x09, 0x01, 0xF7];
        assert_eq!(bytes_of(&mut w, on(0, 1, 2)), [0x90, 1, 2]);
        assert_eq!(bytes_of(&mut w, Midi1Message::SysEx(&msg)), msg);
        assert_eq!(bytes_of(&mut w, on(0, 1, 2)), [0x90, 1, 2]);
        assert_eq!(
            bytes_of(&mut w, Midi1Message::SysEx(&[0xF0, 0xF7])),
            [0xF0, 0xF7]
        );
    }

    #[test]
    fn writer_refuses_bad_sysex() {
        let mut w = Midi1Writer::new(true);
        for bad in [
            &[][..],
            &[0xF0],
            &[0xF7],
            &[0xF0, 0x01],
            &[0x01, 0xF7],
            &[0xF0, 0x80, 0xF7],
            &[0xF0, 0xF7, 0xF7],
        ] {
            let mut n = 0;
            assert_eq!(
                w.write_each(Midi1Message::SysEx(bad), |_| n += 1),
                Err(MessageError::InvalidSysEx),
                "{bad:x?}"
            );
            assert_eq!(n, 0, "no byte before the error");
        }
        // The failed SysEx did not clear the running status.
        assert_eq!(bytes_of(&mut w, on(0, 1, 2)), [0x90, 1, 2]);
        assert_eq!(bytes_of(&mut w, on(0, 1, 2)), [1, 2]);
    }

    fn any_message() -> impl Strategy<Value = Midi1Message<'static>> {
        prop_oneof![
            (0x80u32..=0xEF, 0u32..128, 0u32..128).prop_map(|(s, a, b)| {
                let w = 0x2000_0000 | s << 16 | a << 8 | b;
                Midi1Message::ChannelVoice(Midi1ChannelVoiceMessage::parse(&[w]).unwrap())
            }),
            prop_oneof![
                Just(0xF1u32),
                Just(0xF2),
                Just(0xF3),
                Just(0xF6),
                Just(0xF8),
                Just(0xFA),
                Just(0xFB),
                Just(0xFC),
                Just(0xFE),
                Just(0xFF)
            ]
            .prop_flat_map(|s| (Just(s), 0u32..128, 0u32..128))
            .prop_map(|(s, a, b)| {
                let w = 0x1000_0000 | s << 16 | a << 8 | b;
                Midi1Message::System(SystemMessage::parse(&[w]).unwrap())
            }),
        ]
    }

    #[test]
    fn prop_typed_round_trip() {
        // REQ-MSG-004: the bytes of the writer read back as the same messages, with and
        // without running status, with SysEx messages between them.
        proptest!(|(msgs in proptest::collection::vec(any_message(), 0..24), rs in any::<bool>(),
                    sysex in proptest::collection::vec(0u8..128, 0..8))| {
            let mut frame = std::vec![SOX];
            frame.extend_from_slice(&sysex);
            frame.push(EOX);
            let mut w = Midi1Writer::new(rs);
            let mut bytes = Vec::new();
            let mut want = Vec::new();
            for (i, &m) in msgs.iter().enumerate() {
                if i % 5 == 4 {
                    let s = Midi1Message::SysEx(&frame);
                    w.write_each(s, |b| bytes.push(b)).unwrap();
                    want.push(Owned::SysEx(frame.clone()));
                }
                w.write_each(m, |b| bytes.push(b)).unwrap();
                want.push(owned(Midi1Event::Message(m)));
            }
            prop_assert_eq!(read_with(&mut Midi1Reader::new(), &bytes, 16), want);
        });
    }
}
