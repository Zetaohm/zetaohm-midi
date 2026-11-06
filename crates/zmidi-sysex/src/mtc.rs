//! MIDI Time Code (RP-004/RP-008 v4.2.1; DEC-022). Feature `mtc`.
//!
//! - [`MtcTime`] and [`TimeCodeType`] (0: 24, 1: 25, 2: 30 drop-frame, 3: 30 frames per second;
//!   REQ-MTC-001) with the frame arithmetic of SMPTE 12M: at 30 drop-frame the frame numbers 00 and
//!   01 are skipped at the start of each minute except 00, 10, 20, 30, 40 and 50 (AMB-RP-004-16;
//!   REQ-MTC-002). A reader gives the counts as received, masked to their widths, and
//!   [`MtcTime::is_valid`] checks them (AMB-RP-004-01).
//! - [`QuarterFrame`] is the data byte of the Quarter Frame message F1H: `0nnn dddd`, piece `nnn`
//!   and value `dddd` (REQ-MTC-003). The input and output are this byte, so the part needs no
//!   `zmidi-ump` message family and works on every transport.
//! - [`MtcGenerator`] gives the pieces 0 to 7 forward and 7 to 0 in reverse; each sequence
//!   encodes the frame that starts at its piece 0, and the time moves 2 frames for each sequence
//!   (p. 3; REQ-MTC-004).
//! - [`MtcReader`] assembles a full sequence, gives the lock, the direction and the current time:
//!   the raw time plus 2 frames forward (the assembled time is 2 frames late, p. 3) and minus 1
//!   frame in reverse. In reverse the last piece 0 comes at the start of the encoded frame, as play
//!   enters the frame before it (AMB-RP-004-03; REQ-MTC-005, REQ-MTC-006).
//! - [`FullMessage`] and [`UserBits`]: Universal Real Time Sub-ID#1 01H, Sub-ID#2 01H and 02H
//!   (pp. 5, 6; REQ-MTC-007, REQ-MTC-008).
//!
//! No alloc (DEC-002). Never panics.

use crate::universal::real_time::midi_time_code::{FULL_MESSAGE, USER_BITS};
use crate::universal::{write_universal, RealTimeId, SysexError, Universal};

/// Sub-ID#1 01H: MIDI Time Code.
const MIDI_TIME_CODE: u8 = RealTimeId::MidiTimeCode.into_bits();
/// The frames of 10 minutes at 30 drop-frame: 10 × 1800 − 9 × 2.
const DROP_10_MINUTES: u32 = 17_982;
/// The frames of a minute that drops 2 numbers at 30 drop-frame.
const DROP_MINUTE: u32 = 1_798;

/// The time code type: the 2-bit value of the hours byte (p. 2; REQ-MTC-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TimeCodeType {
    /// 0: 24 frames per second.
    Fps24 = 0,
    /// 1: 25 frames per second.
    Fps25 = 1,
    /// 2: 30 frames per second, drop-frame.
    Fps30Drop = 2,
    /// 3: 30 frames per second, non-drop.
    Fps30 = 3,
}

impl TimeCodeType {
    /// The 2-bit value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The type of the 2 low bits of `value`.
    pub const fn from_bits(value: u8) -> Self {
        match value & 0x03 {
            0 => Self::Fps24,
            1 => Self::Fps25,
            2 => Self::Fps30Drop,
            _ => Self::Fps30,
        }
    }

    /// The frames of a counting second: 24, 25 or 30.
    pub const fn frames_per_second(self) -> u8 {
        match self {
            Self::Fps24 => 24,
            Self::Fps25 => 25,
            Self::Fps30Drop | Self::Fps30 => 30,
        }
    }

    /// The frames of 24 hours.
    const fn day(self) -> u32 {
        match self {
            Self::Fps30Drop => 24 * 6 * DROP_10_MINUTES,
            t => 86_400 * t.frames_per_second() as u32,
        }
    }
}

/// An MTC time (REQ-MTC-001). A reader gives the counts as received, masked to their widths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MtcTime {
    /// Hours, 0 to 23 (5 bits).
    pub hours: u8,
    /// Minutes, 0 to 59 (6 bits).
    pub minutes: u8,
    /// Seconds, 0 to 59 (6 bits).
    pub seconds: u8,
    /// Frames, below the frame rate of the type (5 bits).
    pub frames: u8,
    /// The time code type.
    pub rate: TimeCodeType,
}

impl MtcTime {
    /// The time, or [`SysexError::OutOfRange`] for a count above its range or a frame number that
    /// the drop-frame type skips (REQ-MTC-001).
    pub const fn new(
        hours: u8,
        minutes: u8,
        seconds: u8,
        frames: u8,
        rate: TimeCodeType,
    ) -> Result<Self, SysexError> {
        let t = Self {
            hours,
            minutes,
            seconds,
            frames,
            rate,
        };
        if t.is_valid() {
            Ok(t)
        } else {
            Err(SysexError::OutOfRange)
        }
    }

    /// True when each count is in its range and the frame number is not skipped (AMB-RP-004-01).
    pub const fn is_valid(&self) -> bool {
        self.hours <= 23
            && self.minutes <= 59
            && self.seconds <= 59
            && self.frames < self.rate.frames_per_second()
            && !self.skipped()
    }

    /// A frame number 00 or 01 at the start of a minute that the drop-frame type skips.
    const fn skipped(&self) -> bool {
        matches!(self.rate, TimeCodeType::Fps30Drop)
            && self.seconds == 0
            && self.frames < 2
            && !self.minutes.is_multiple_of(10)
    }

    /// The next higher valid time for a skipped drop-frame number (frame 02), or the time itself
    /// (RP-013 Drop Frame Notes; REQ-MTC-002).
    pub const fn next_valid(self) -> Self {
        if self.skipped() {
            Self { frames: 2, ..self }
        } else {
            self
        }
    }

    /// The frames from 00:00:00:00 to the time, by the rule of its type (REQ-MTC-002).
    pub const fn to_frame_count(&self) -> u32 {
        let fps = self.rate.frames_per_second() as u32;
        let minutes = self.hours as u32 * 60 + self.minutes as u32;
        let n = (minutes * 60 + self.seconds as u32) * fps + self.frames as u32;
        let n = if matches!(self.rate, TimeCodeType::Fps30Drop) {
            n.saturating_sub(2 * (minutes - minutes / 10))
        } else {
            n
        };
        n % self.rate.day()
    }

    /// The time of `count` frames from 00:00:00:00, with the 24-hour wrap (REQ-MTC-002).
    pub const fn from_frame_count(count: u32, rate: TimeCodeType) -> Self {
        let mut n = count % rate.day();
        if matches!(rate, TimeCodeType::Fps30Drop) {
            let tens = n / DROP_10_MINUTES;
            let rest = n % DROP_10_MINUTES;
            n += 18 * tens;
            if rest > 1 {
                n += 2 * ((rest - 2) / DROP_MINUTE);
            }
        }
        let fps = rate.frames_per_second() as u32;
        let s = n / fps;
        Self {
            hours: (s / 3600) as u8,
            minutes: (s / 60 % 60) as u8,
            seconds: (s % 60) as u8,
            frames: (n % fps) as u8,
            rate,
        }
    }

    /// The time `delta` frames later (or earlier), with the 24-hour wrap (REQ-MTC-002).
    pub const fn add_frames(self, delta: i32) -> Self {
        // A day has fewer than 2^22 frames: 32-bit arithmetic, with no 64-bit division code.
        let day = self.rate.day();
        let step = delta.rem_euclid(day as i32) as u32;
        Self::from_frame_count((self.to_frame_count() + step) % day, self.rate)
    }

    /// A time of raw counts, masked to their widths.
    pub(crate) const fn raw(hours: u8, minutes: u8, seconds: u8, frames: u8) -> Self {
        Self {
            hours: hours & 0x1F,
            minutes: minutes & 0x3F,
            seconds: seconds & 0x3F,
            frames: frames & 0x1F,
            rate: TimeCodeType::from_bits(hours >> 5),
        }
    }

    /// The hours byte `0 yy zzzzz`.
    pub(crate) const fn hours_byte(&self) -> u8 {
        self.rate.into_bits() << 5 | (self.hours & 0x1F)
    }
}

/// The data byte of a Quarter Frame message (p. 1; REQ-MTC-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuarterFrame {
    /// The piece, 0 (frames LS nibble) to 7 (hours MS nibble and type).
    pub piece: u8,
    /// The 4-bit value.
    pub value: u8,
}

impl QuarterFrame {
    /// The piece and value of a data byte; bit 7 is ignored.
    pub const fn from_byte(byte: u8) -> Self {
        Self {
            piece: byte >> 4 & 0x07,
            value: byte & 0x0F,
        }
    }

    /// The data byte `0nnn dddd`.
    pub const fn to_byte(self) -> u8 {
        (self.piece & 0x07) << 4 | (self.value & 0x0F)
    }

    /// The 8 data bytes of `time`, piece 0 first; the reserved bits are 0 (p. 2).
    pub const fn pieces(time: &MtcTime) -> [u8; 8] {
        let (f, s, m, h) = (time.frames, time.seconds, time.minutes, time.hours);
        [
            f & 0x0F,
            0x10 | (f >> 4 & 0x01),
            0x20 | (s & 0x0F),
            0x30 | (s >> 4 & 0x03),
            0x40 | (m & 0x0F),
            0x50 | (m >> 4 & 0x03),
            0x60 | (h & 0x0F),
            0x70 | time.rate.into_bits() << 1 | (h >> 4 & 0x01),
        ]
    }
}

/// The direction of a Quarter Frame sequence (p. 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Pieces 0 to 7.
    Forward,
    /// Pieces 7 to 0.
    Reverse,
}

impl Direction {
    /// The first piece of a sequence.
    const fn first(self) -> u8 {
        match self {
            Self::Forward => 0,
            Self::Reverse => 7,
        }
    }

    /// The piece after `last`.
    const fn after(self, last: u8) -> u8 {
        match self {
            Self::Forward => (last + 1) & 7,
            Self::Reverse => (last + 7) & 7,
        }
    }
}

/// The Quarter Frame generator (REQ-MTC-004). The caller gives each data byte at its quarter
/// frame, piece 0 at the start of the frame that the sequence encodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MtcGenerator {
    time: MtcTime,
    direction: Direction,
    last: Option<u8>,
    running: bool,
}

impl MtcGenerator {
    /// A running generator: the first sequence encodes `time` and starts at piece 0 forward or
    /// piece 7 in reverse.
    pub const fn new(time: MtcTime, direction: Direction) -> Self {
        Self {
            time,
            direction,
            last: None,
            running: true,
        }
    }

    /// The data byte of the next piece, or `None` while stopped. A forward sequence that starts
    /// after a piece 7 encodes the time plus 2 frames; a reverse sequence that starts after a
    /// piece 0 encodes the time minus 2 frames (p. 3).
    pub fn next_byte(&mut self) -> Option<u8> {
        if !self.running {
            return None;
        }
        let piece = match self.last {
            None => self.direction.first(),
            Some(l) => self.direction.after(l),
        };
        match (self.direction, self.last) {
            (Direction::Forward, Some(7)) => self.time = self.time.add_frames(2),
            (Direction::Reverse, Some(0)) => self.time = self.time.add_frames(-2),
            _ => {}
        }
        self.last = Some(piece);
        Some(QuarterFrame::pieces(&self.time)[piece as usize])
    }

    /// Runs or stops the generator. Stop it while the transport is stopped or shuttles, and send
    /// a [`FullMessage`] at the destination (p. 3).
    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    /// Changes the direction: the next piece is the next one in the new order from the last
    /// piece, and the time changes when the next sequence starts (AMB-RP-004-13).
    pub fn set_direction(&mut self, direction: Direction) {
        self.direction = direction;
    }

    /// A new time, for example after a [`FullMessage`]: the next piece is the first piece of the
    /// direction.
    pub fn locate(&mut self, time: MtcTime) {
        self.time = time;
        self.last = None;
    }

    /// The time that the current or next sequence encodes.
    pub const fn time(&self) -> MtcTime {
        self.time
    }
}

/// The end of a full sequence (REQ-MTC-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MtcSequence {
    /// The assembled time, counts as received.
    pub raw: MtcTime,
    /// The current time: `raw` plus 2 frames forward, minus 1 frame in reverse.
    pub current: MtcTime,
    /// The direction of the sequence.
    pub direction: Direction,
    /// True when `raw` is not the previous raw time plus 2 frames (minus 2 in reverse).
    pub discontinuity: bool,
}

/// What one data byte gives (REQ-MTC-005, REQ-MTC-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MtcEvent {
    /// The piece, 0 to 7.
    pub piece: u8,
    /// True for pieces 0 and 4, which mark frame boundaries (p. 3).
    pub boundary: bool,
    /// The end of a full sequence.
    pub sequence: Option<MtcSequence>,
}

/// The Quarter Frame reader (REQ-MTC-005 to REQ-MTC-007). 26 bytes, no alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MtcReader {
    nibbles: [u8; 8],
    count: u8,
    first: u8,
    last: Option<u8>,
    direction: Option<Direction>,
    locked: bool,
    running: bool,
    previous: Option<MtcTime>,
    time: Option<MtcTime>,
}

impl Default for MtcReader {
    fn default() -> Self {
        Self::new()
    }
}

impl MtcReader {
    /// A reader that is not locked and is stopped.
    pub const fn new() -> Self {
        Self {
            nibbles: [0; 8],
            count: 0,
            first: 0,
            last: None,
            direction: None,
            locked: false,
            running: false,
            previous: None,
            time: None,
        }
    }

    /// Takes one Quarter Frame data byte (REQ-MTC-005, REQ-MTC-006). A piece that is not the next
    /// one in the direction drops the partial sequence and the lock: the next piece in the other
    /// order changes the direction, any other piece (also a repeated one) only breaks the order
    /// (AMB-RP-004-13, AMB-RP-004-14). A sequence starts at the first piece of the direction.
    pub fn receive(&mut self, byte: u8) -> MtcEvent {
        let q = QuarterFrame::from_byte(byte);
        let p = q.piece;
        self.running = true;
        let next = match (self.last, self.direction) {
            (Some(l), Some(d)) if d.after(l) == p => true,
            (Some(l), d) => {
                let found = if Direction::Forward.after(l) == p {
                    Some(Direction::Forward)
                } else if Direction::Reverse.after(l) == p {
                    Some(Direction::Reverse)
                } else {
                    None
                };
                match (d, found) {
                    // The second piece gives the direction: go on when the sequence started
                    // at its first piece.
                    (None, Some(f)) => {
                        self.direction = Some(f);
                        self.count > 0 && self.first == f.first()
                    }
                    (_, Some(f)) => {
                        self.direction = Some(f);
                        self.locked = false;
                        false
                    }
                    (_, None) => {
                        self.locked = false;
                        false
                    }
                }
            }
            (None, _) => false,
        };
        self.last = Some(p);
        if !next {
            self.count = 0;
        }
        let mut event = MtcEvent {
            piece: p,
            boundary: p == 0 || p == 4,
            sequence: None,
        };
        if self.count == 0 {
            let starts = match self.direction {
                Some(d) => p == d.first(),
                None => p == 0 || p == 7,
            };
            if !starts {
                return event;
            }
            self.first = p;
        }
        self.nibbles[p as usize] = q.value;
        self.count += 1;
        if self.count == 8 {
            self.count = 0;
            event.sequence = Some(self.complete());
        }
        event
    }

    fn complete(&mut self) -> MtcSequence {
        let n = &self.nibbles;
        let raw = MtcTime::raw(
            n[6] | n[7] << 4,
            n[4] | n[5] << 4,
            n[2] | n[3] << 4,
            n[0] | n[1] << 4,
        );
        let direction = self.direction.unwrap_or(Direction::Forward);
        let (offset, step) = match direction {
            Direction::Forward => (2, 2),
            Direction::Reverse => (-1, -2),
        };
        let current = raw.add_frames(offset);
        let discontinuity = match self.previous {
            Some(prev) => prev.add_frames(step) != raw,
            None => false,
        };
        self.previous = Some(raw);
        self.time = Some(current);
        self.locked = true;
        MtcSequence {
            raw,
            current,
            direction,
            discontinuity,
        }
    }

    /// Takes a Full Message: the time and the state stopped; the next Quarter Frame data byte sets
    /// the state running with that time (AMB-RP-004-06; REQ-MTC-007).
    pub fn full_message(&mut self, message: &FullMessage) {
        self.time = Some(message.time);
        self.running = false;
        self.locked = false;
        self.count = 0;
        self.last = None;
        self.previous = None;
    }

    /// True after a full sequence, until a broken order or a direction change.
    pub const fn is_locked(&self) -> bool {
        self.locked
    }

    /// True after a Quarter Frame data byte, false after a Full Message.
    pub const fn is_running(&self) -> bool {
        self.running
    }

    /// The direction, once two pieces gave it.
    pub const fn direction(&self) -> Option<Direction> {
        self.direction
    }

    /// The current time of the last full sequence, or the time of a Full Message after it.
    pub const fn time(&self) -> Option<MtcTime> {
        self.time
    }
}

/// The MTC Full Message: `F0 7F <device ID> 01 01 hr mn sc fr F7` (p. 5; REQ-MTC-007).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FullMessage {
    /// The Device ID; 7FH for the whole system.
    pub device_id: u8,
    /// The time with its type.
    pub time: MtcTime,
}

impl FullMessage {
    /// Reads the message from a Universal header (Real Time, Sub-ID#1 01H, Sub-ID#2 01H). Gives
    /// `None` for another message and [`SysexError::Length`] for a wrong length; the counts are
    /// masked to their widths (AMB-RP-004-01).
    pub fn from_universal(u: &Universal<'_>) -> Result<Option<Self>, SysexError> {
        if !u.real_time || u.sub_id1 != MIDI_TIME_CODE || u.sub_id2 != Some(FULL_MESSAGE) {
            return Ok(None);
        }
        let &[hr, mn, sc, fr] = u.data else {
            return Err(SysexError::Length);
        };
        Ok(Some(Self {
            device_id: u.device_id,
            time: MtcTime::raw(hr, mn, sc, fr),
        }))
    }

    /// Writes the whole message into `out` and gives the length (REQ-SX-007). Gives
    /// [`SysexError::OutOfRange`] for a time that is not valid or a Device ID above 7FH.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, SysexError> {
        let t = self.time;
        if !t.is_valid() {
            return Err(SysexError::OutOfRange);
        }
        write_universal(
            out,
            true,
            self.device_id,
            MIDI_TIME_CODE,
            Some(FULL_MESSAGE),
            |w| {
                w.bytes(&[t.hours_byte(), t.minutes, t.seconds, t.frames]);
                Ok(())
            },
        )
    }
}

/// The MTC User Bits message: `F0 7F <device ID> 01 02 u1 ... u8 u9 F7` (p. 6; REQ-MTC-008).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserBits {
    /// The Device ID; 7FH for the whole system.
    pub device_id: u8,
    /// Binary Groups 1 to 8 (`u1` to `u8`), 4 bits each.
    pub groups: [u8; 8],
    /// The Binary Group Flag Bits of `u9`: bit 1 is j, bit 0 is i.
    pub flags: u8,
}

impl UserBits {
    /// Reads the message from a Universal header (Real Time, Sub-ID#1 01H, Sub-ID#2 02H). Gives
    /// `None` for another message and [`SysexError::Length`] for a wrong length; the reserved bits
    /// are ignored.
    pub fn from_universal(u: &Universal<'_>) -> Result<Option<Self>, SysexError> {
        if !u.real_time || u.sub_id1 != MIDI_TIME_CODE || u.sub_id2 != Some(USER_BITS) {
            return Ok(None);
        }
        let &[g1, g2, g3, g4, g5, g6, g7, g8, u9] = u.data else {
            return Err(SysexError::Length);
        };
        Ok(Some(Self {
            device_id: u.device_id,
            groups: [g1, g2, g3, g4, g5, g6, g7, g8].map(|g| g & 0x0F),
            flags: u9 & 0x03,
        }))
    }

    /// Writes the whole message into `out` and gives the length (REQ-SX-007). Gives
    /// [`SysexError::OutOfRange`] for a group above 0FH, flags above 03H or a Device ID above 7FH.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, SysexError> {
        if self.groups.iter().any(|&g| g > 0x0F) || self.flags > 0x03 {
            return Err(SysexError::OutOfRange);
        }
        write_universal(
            out,
            true,
            self.device_id,
            MIDI_TIME_CODE,
            Some(USER_BITS),
            |w| {
                w.bytes(&self.groups);
                w.byte(self.flags);
                Ok(())
            },
        )
    }

    /// The groups as four bytes `hhhhgggg ffffeeee ddddcccc bbbbaaaa` (p. 6).
    pub const fn to_bytes(&self) -> [u8; 4] {
        let g = &self.groups;
        [
            (g[7] & 0x0F) << 4 | (g[6] & 0x0F),
            (g[5] & 0x0F) << 4 | (g[4] & 0x0F),
            (g[3] & 0x0F) << 4 | (g[2] & 0x0F),
            (g[1] & 0x0F) << 4 | (g[0] & 0x0F),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universal::Sysex;
    use proptest::prelude::*;
    use TimeCodeType::*;

    fn t(h: u8, m: u8, s: u8, f: u8, rate: TimeCodeType) -> MtcTime {
        MtcTime::new(h, m, s, f, rate).unwrap()
    }

    #[test]
    fn time_code_types_and_ranges() {
        // REQ-MTC-001: p. 2 "0 = 24 Frames/Second ... 3 = 30 Frames/Second (Non-Drop)".
        for (v, r, fps) in [
            (0, Fps24, 24),
            (1, Fps25, 25),
            (2, Fps30Drop, 30),
            (3, Fps30, 30),
        ] {
            assert_eq!(
                (
                    TimeCodeType::from_bits(v),
                    r.into_bits(),
                    r.frames_per_second()
                ),
                (r, v, fps)
            );
        }
        assert_eq!(TimeCodeType::from_bits(0x07), Fps30);
        assert!(MtcTime::new(23, 59, 59, 29, Fps30).is_ok());
        for (h, m, s, f, r) in [
            (24, 0, 0, 0, Fps30),
            (0, 60, 0, 0, Fps30),
            (0, 0, 60, 0, Fps30),
            (0, 0, 0, 24, Fps24),
            (0, 0, 0, 25, Fps25),
        ] {
            assert_eq!(MtcTime::new(h, m, s, f, r), Err(SysexError::OutOfRange));
        }
        // Drop-frame: 00:01:00:00 and 00:01:00:01 are skipped, 00:10:00:00 is not.
        assert_eq!(
            MtcTime::new(0, 1, 0, 0, Fps30Drop),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            MtcTime::new(0, 1, 0, 1, Fps30Drop),
            Err(SysexError::OutOfRange)
        );
        assert!(MtcTime::new(0, 1, 0, 2, Fps30Drop).is_ok());
        assert!(MtcTime::new(0, 10, 0, 0, Fps30Drop).is_ok());
        assert!(MtcTime::new(0, 1, 0, 0, Fps30).is_ok());
    }

    #[test]
    fn drop_frame_arithmetic() {
        // REQ-MTC-002: RP-013 p. 10: 00:22:00:00 is not a drop-frame number, the next is
        // 00:22:00:02; 00:22:00:02 drop-frame is 00:21:58:22 non-drop.
        let skipped = MtcTime {
            hours: 0,
            minutes: 22,
            seconds: 0,
            frames: 0,
            rate: Fps30Drop,
        };
        assert_eq!(skipped.next_valid(), t(0, 22, 0, 2, Fps30Drop));
        let df = t(0, 22, 0, 2, Fps30Drop);
        assert_eq!(
            MtcTime::from_frame_count(df.to_frame_count(), Fps30),
            t(0, 21, 58, 22, Fps30)
        );
        // The minute boundary: 00:00:59:29 + 1 = 00:01:00:02; 00:09:59:29 + 1 = 00:10:00:00.
        assert_eq!(
            t(0, 0, 59, 29, Fps30Drop).add_frames(1),
            t(0, 1, 0, 2, Fps30Drop)
        );
        assert_eq!(
            t(0, 1, 0, 2, Fps30Drop).add_frames(-1),
            t(0, 0, 59, 29, Fps30Drop)
        );
        assert_eq!(
            t(0, 9, 59, 29, Fps30Drop).add_frames(1),
            t(0, 10, 0, 0, Fps30Drop)
        );
        // 24-hour wrap, both ways, every type.
        for r in [Fps24, Fps25, Fps30Drop, Fps30] {
            let last = MtcTime::from_frame_count(r.day() - 1, r);
            assert_eq!(last.add_frames(1), t(0, 0, 0, 0, r));
            assert_eq!(t(0, 0, 0, 0, r).add_frames(-1), last);
            assert!(last.is_valid());
        }
        assert_eq!(
            t(23, 59, 59, 29, Fps30Drop).to_frame_count(),
            Fps30Drop.day() - 1
        );
    }

    #[test]
    fn prop_frame_count_round_trip() {
        // REQ-MTC-002: each frame count of a day is one valid time, and back.
        proptest!(|(r in 0u8..4, n in 0u32..2_592_000, d in any::<i32>())| {
            let rate = TimeCodeType::from_bits(r);
            let n = n % rate.day();
            let time = MtcTime::from_frame_count(n, rate);
            prop_assert!(time.is_valid(), "{time:?}");
            prop_assert_eq!(time.to_frame_count(), n);
            prop_assert_eq!(time.add_frames(1).to_frame_count(), (n + 1) % rate.day());
            let want = (n as i64 + d as i64).rem_euclid(rate.day() as i64) as u32;
            prop_assert_eq!(time.add_frames(d).to_frame_count(), want);
        });
    }

    #[test]
    fn quarter_frame_pieces_follow_page_2() {
        // REQ-MTC-003: the p. 4 example 01:37:52:16 at 30 frames per second.
        let time = t(1, 37, 52, 16, Fps30);
        let bytes = QuarterFrame::pieces(&time);
        assert_eq!(bytes, [0x00, 0x11, 0x24, 0x33, 0x45, 0x52, 0x61, 0x76]);
        for (n, b) in bytes.into_iter().enumerate() {
            let q = QuarterFrame::from_byte(b);
            assert_eq!(q.piece as usize, n);
            assert_eq!(q.to_byte(), b);
        }
        // Bit 7 is ignored; the reserved bits are written as 0.
        assert_eq!(
            QuarterFrame::from_byte(0xF5),
            QuarterFrame { piece: 7, value: 5 }
        );
        let max = t(23, 59, 59, 29, Fps30Drop);
        assert_eq!(
            QuarterFrame::pieces(&max),
            [0x0D, 0x11, 0x2B, 0x33, 0x4B, 0x53, 0x67, 0x75]
        );
    }

    fn run(g: &mut MtcGenerator) -> [u8; 8] {
        core::array::from_fn(|_| g.next_byte().unwrap())
    }

    #[test]
    fn mtc_generator_order_and_time() {
        // REQ-MTC-004: pieces 0 to 7, 2 frames for each sequence (p. 3).
        let start = t(1, 37, 52, 16, Fps30);
        let mut g = MtcGenerator::new(start, Direction::Forward);
        assert_eq!(run(&mut g), QuarterFrame::pieces(&start));
        assert_eq!(g.time(), start);
        let second = run(&mut g);
        assert_eq!(second, QuarterFrame::pieces(&t(1, 37, 52, 18, Fps30)));
        // Stopped: nothing; locate restarts at piece 0.
        g.set_running(false);
        assert_eq!(g.next_byte(), None);
        g.set_running(true);
        g.locate(t(0, 0, 0, 0, Fps25));
        assert_eq!(g.next_byte(), Some(0x00));
        assert_eq!(g.next_byte(), Some(0x10));
    }

    #[test]
    fn mtc_generator_reverse_and_direction_change() {
        // REQ-MTC-004: pieces 7 to 0 in reverse, 2 frames back for each sequence; a direction
        // change gives the next piece in the new order (AMB-RP-004-13).
        let start = t(0, 0, 10, 10, Fps30);
        let mut g = MtcGenerator::new(start, Direction::Reverse);
        let mut want = QuarterFrame::pieces(&start);
        want.reverse();
        assert_eq!(run(&mut g), want);
        assert_eq!(
            g.next_byte(),
            Some(QuarterFrame::pieces(&t(0, 0, 10, 8, Fps30))[7])
        );
        assert_eq!(g.time(), t(0, 0, 10, 8, Fps30));
        // Forward again after piece 7: piece 0 of the sequence 2 frames later.
        g.set_direction(Direction::Forward);
        assert_eq!(
            g.next_byte(),
            Some(QuarterFrame::pieces(&t(0, 0, 10, 10, Fps30))[0])
        );
        // A change in the middle of a sequence keeps its time.
        g.next_byte();
        g.next_byte();
        g.set_direction(Direction::Reverse);
        assert_eq!(
            g.next_byte(),
            Some(QuarterFrame::pieces(&t(0, 0, 10, 10, Fps30))[1])
        );
    }

    fn feed(r: &mut MtcReader, bytes: &[u8]) -> Option<MtcSequence> {
        let mut last = None;
        for &b in bytes {
            if let Some(s) = r.receive(b).sequence {
                last = Some(s);
            }
        }
        last
    }

    #[test]
    fn mtc_reader_assembles_and_locks() {
        // REQ-MTC-005: a full sequence gives the time and the lock (p. 3); boundaries at 0 and 4.
        let time = t(1, 37, 52, 16, Fps30);
        let bytes = QuarterFrame::pieces(&time);
        let mut r = MtcReader::new();
        let events: [MtcEvent; 8] = core::array::from_fn(|n| r.receive(bytes[n]));
        assert_eq!((events[0].piece, events[0].boundary), (0, true));
        assert!(events[4].boundary && !events[3].boundary);
        assert!(events[..7].iter().all(|e| e.sequence.is_none()));
        assert_eq!(events[7].sequence.unwrap().raw, time);
        assert!(r.is_locked() && r.is_running());
        assert_eq!(r.direction(), Some(Direction::Forward));
        // A reader that comes on line in the middle waits for piece 0.
        let mut r = MtcReader::new();
        assert_eq!(feed(&mut r, &bytes[3..]), None);
        assert!(!r.is_locked());
        assert_eq!(feed(&mut r, &bytes).unwrap().raw, time);
        // Reverse: 7 to 0.
        let mut r = MtcReader::new();
        let mut rev = bytes;
        rev.reverse();
        let s = feed(&mut r, &rev).unwrap();
        assert_eq!((s.raw, s.direction), (time, Direction::Reverse));
        assert_eq!(r.direction(), Some(Direction::Reverse));
    }

    #[test]
    fn mtc_reader_drops_a_broken_sequence() {
        // REQ-MTC-005: a gap, a repeated piece and a direction change drop the partial sequence
        // and the lock (AMB-RP-004-13, AMB-RP-004-14).
        let a = QuarterFrame::pieces(&t(0, 0, 1, 0, Fps25));
        let b = QuarterFrame::pieces(&t(0, 0, 1, 2, Fps25));
        let mut r = MtcReader::new();
        feed(&mut r, &a);
        assert!(r.is_locked());
        // A gap: piece 2 after piece 0.
        assert_eq!(
            feed(&mut r, &[b[0], b[2], b[3], b[4], b[5], b[6], b[7]]),
            None
        );
        assert!(!r.is_locked());
        // A repeated piece.
        assert_eq!(
            feed(
                &mut r,
                &[b[0], b[1], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]
            ),
            None
        );
        // A direction change after piece 3: the reverse sequence waits for piece 7.
        feed(&mut r, &b);
        assert!(r.is_locked());
        assert_eq!(
            feed(&mut r, &[b[0], b[1], b[2], b[3], b[2], b[1], b[0]]),
            None
        );
        assert!(!r.is_locked());
        assert_eq!(r.direction(), Some(Direction::Reverse));
    }

    #[test]
    fn mtc_reader_current_time_offsets() {
        // REQ-MTC-006: +2 frames forward (p. 3), -1 frame in reverse (AMB-RP-004-03).
        let time = t(0, 0, 0, 10, Fps24);
        let mut r = MtcReader::new();
        let s = feed(&mut r, &QuarterFrame::pieces(&time)).unwrap();
        assert_eq!((s.raw, s.current), (time, t(0, 0, 0, 12, Fps24)));
        assert_eq!(r.time(), Some(t(0, 0, 0, 12, Fps24)));
        let mut r = MtcReader::new();
        let mut rev = QuarterFrame::pieces(&time);
        rev.reverse();
        let s = feed(&mut r, &rev).unwrap();
        assert_eq!((s.raw, s.current), (time, t(0, 0, 0, 9, Fps24)));
        // An odd frame number and a type that is not valid for the count are accepted as raw.
        let mut r = MtcReader::new();
        let odd = MtcTime {
            hours: 0,
            minutes: 0,
            seconds: 0,
            frames: 27,
            rate: Fps25,
        };
        let s = feed(&mut r, &QuarterFrame::pieces(&odd)).unwrap();
        assert_eq!(s.raw, odd);
        assert!(!s.raw.is_valid());
    }

    #[test]
    fn mtc_reader_reports_discontinuities() {
        // REQ-MTC-006: a step other than 2 frames is a discontinuity; 0 and 1 keep the lock
        // (AMB-RP-004-15).
        let mut r = MtcReader::new();
        let base = t(0, 0, 5, 0, Fps30);
        assert!(
            !feed(&mut r, &QuarterFrame::pieces(&base))
                .unwrap()
                .discontinuity
        );
        for (step, disc) in [(2, false), (1, true), (0, true), (2, false), (10, true)] {
            let next = r.time().unwrap().add_frames(step - 2);
            let s = feed(&mut r, &QuarterFrame::pieces(&next)).unwrap();
            assert_eq!(s.discontinuity, disc, "step {step}");
            assert!(r.is_locked());
        }
    }

    #[test]
    fn full_message_bytes_and_reader_state() {
        // REQ-MTC-007: p. 5 "F0 7F <device ID> 01 <sub-ID 2> hr mn sc fr F7"; the next Quarter
        // Frame sets running (AMB-RP-004-06).
        let m = FullMessage {
            device_id: 0x7F,
            time: t(1, 37, 52, 16, Fps30),
        };
        let mut out = [0u8; 10];
        assert_eq!(m.write(&mut out), Ok(10));
        assert_eq!(out, [0xF0, 0x7F, 0x7F, 0x01, 0x01, 0x61, 37, 52, 16, 0xF7]);
        let s = Sysex::parse(&out).unwrap();
        let u = Universal::parse(&s).unwrap().unwrap();
        assert_eq!(FullMessage::from_universal(&u), Ok(Some(m)));
        assert_eq!(UserBits::from_universal(&u), Ok(None));
        let s = Sysex::parse(&out[..9]).unwrap();
        let short = Universal {
            data: &s.data[3..5],
            ..Universal::parse(&s).unwrap().unwrap()
        };
        assert_eq!(FullMessage::from_universal(&short), Err(SysexError::Length));
        let bad = FullMessage {
            time: MtcTime {
                frames: 30,
                ..m.time
            },
            ..m
        };
        assert_eq!(bad.write(&mut out), Err(SysexError::OutOfRange));
        // The reader: stopped with the time, running at the next Quarter Frame.
        let mut r = MtcReader::new();
        feed(&mut r, &QuarterFrame::pieces(&t(0, 0, 0, 0, Fps30)));
        r.full_message(&m);
        assert!(!r.is_running() && !r.is_locked());
        assert_eq!(r.time(), Some(m.time));
        let pieces = QuarterFrame::pieces(&m.time);
        r.receive(pieces[0]);
        assert!(r.is_running());
        assert_eq!(r.time(), Some(m.time));
        let s = feed(&mut r, &pieces[1..]).unwrap();
        assert_eq!((s.raw, s.discontinuity), (m.time, false));
    }

    #[test]
    fn user_bits_bytes() {
        // REQ-MTC-008: p. 6 "u1 0000aaaa ... u8 0000hhhh u9 000000ji", and the 8-bit order
        // "hhhhgggg ffffeeee ddddcccc bbbbaaaa".
        let ub = UserBits {
            device_id: 0x7F,
            groups: [1, 2, 3, 4, 5, 6, 7, 8],
            flags: 0x02,
        };
        let mut out = [0u8; 15];
        assert_eq!(ub.write(&mut out), Ok(15));
        assert_eq!(
            out,
            [0xF0, 0x7F, 0x7F, 0x01, 0x02, 1, 2, 3, 4, 5, 6, 7, 8, 0x02, 0xF7]
        );
        let s = Sysex::parse(&out).unwrap();
        let u = Universal::parse(&s).unwrap().unwrap();
        assert_eq!(UserBits::from_universal(&u), Ok(Some(ub)));
        assert_eq!(ub.to_bytes(), [0x87, 0x65, 0x43, 0x21]);
        let bad = UserBits { flags: 4, ..ub };
        assert_eq!(bad.write(&mut [0u8; 15]), Err(SysexError::OutOfRange));
        let bad = UserBits {
            groups: [0x10, 0, 0, 0, 0, 0, 0, 0],
            ..ub
        };
        assert_eq!(bad.write(&mut [0u8; 15]), Err(SysexError::OutOfRange));
        // The reserved bits of a received message are ignored.
        let s = Sysex::parse(&[0x7F, 0x7F, 0x01, 0x02, 0x71, 2, 3, 4, 5, 6, 7, 8, 0x7E]).unwrap();
        let u = Universal::parse(&s).unwrap().unwrap();
        let got = UserBits::from_universal(&u).unwrap().unwrap();
        assert_eq!((got.groups[0], got.flags), (1, 2));
    }
}
