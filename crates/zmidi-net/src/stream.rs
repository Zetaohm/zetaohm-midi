//! The UMP Data path of one session, in both directions (M2-124-UM v1.0.1, sections 7.1 and
//! 7.2: UMP Data, Zero Length UMP Data, Forward Error Correction and Retransmit).
//!
//! - [`Receiver`] classifies each received Sequence Number with a [`ReceiveWindow`] (serial
//!   arithmetic, W numbers, N-01, N-02, N-03; REQ-NET-DATA-009, -016), skips the Commands that
//!   it already processed (REQ-NET-DATA-008), gives gap and late marks (REQ-NET-DATA-010) and
//!   asks for the missing numbers with Retransmit Requests (REQ-NET-RTX-009, -010, N-35, N-36).
//! - [`Sender`] gives the Sequence Numbers (REQ-NET-DATA-004), packs the UMP words into
//!   Commands (REQ-NET-DATA-007), keeps one [`SendHistory`] for FEC and Retransmit (N-39, N-40,
//!   REQ-NET-RTX-003), writes the FEC repeats (REQ-NET-FEC-001, -002), the idle schedule
//!   (REQ-NET-DATA-012, -013, REQ-NET-FEC-003, N-12, N-33, N-34) and serves the Retransmit
//!   Requests from a [`RetransmitQueue`] (REQ-NET-RTX-005, -006, -008, -015, N-07, N-08, N-37,
//!   N-41).
//! - [`DataStream`] joins both. It knows no session state: the Session module calls it in
//!   Established Session only, and calls [`DataStream::reset`] at session start and at each
//!   Session Reset.
//! - No I/O and no clock (DEC-012): the caller gives the time as u64 milliseconds. No alloc
//!   (DEC-002): all buffers have a fixed size. Nothing panics on received bytes.

use crate::command::{
    message_words, whole_messages, CommandCode, CommandError, RetransmitError,
    RetransmitErrorReason, RetransmitRequest, SeqNum, UmpData, UmpMessages, MAX_UMP_DATA_WORDS,
};
use crate::packet::{Header, PacketError, PayloadWriter, RawCommand};

/// The maximum first idle interval T1 in milliseconds (7.2.1, REQ-NET-DATA-017).
pub const MAX_IDLE_INTERVAL_MS: u32 = 300;

/// Error of the stream configuration and of the stream writers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StreamError {
    /// T1 is 0 or above 300 ms (REQ-NET-DATA-017).
    #[error("idle interval T1 out of range (1 to 300 ms)")]
    IdleInterval,
    /// The idle growth G is 0 (REQ-NET-DATA-013).
    #[error("idle growth G must be 1 or more")]
    IdleGrowth,
    /// The maximum UMP Data Command size is 0 or above 64 words (REQ-NET-DATA-007).
    #[error("UMP Data Command size out of range (1 to 64 words)")]
    CommandWords,
    /// UMP words that do not end at a message boundary (REQ-NET-DATA-006).
    #[error("invalid UMP data")]
    InvalidUmpData,
    /// The send history has no room for the UMP words that are not written yet: write the
    /// queued data first (REQ-NET-RTX-003).
    #[error("send queue full")]
    Full,
    /// The payload writer refused: a Command does not fit in an empty UDP payload.
    #[error("packet: {0}")]
    Packet(#[from] PacketError),
}

/// The configuration of a [`DataStream`] (DEC-NET-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamConfig {
    /// T1: the time from the last UMP Data Command with UMP words to the first Zero Length
    /// Command, 1 to 300 ms, default 100 ms (REQ-NET-DATA-012, -017, N-33).
    pub idle_interval_ms: u32,
    /// G: each further idle interval is G times the interval before it, default 2
    /// (REQ-NET-DATA-013, N-34).
    pub idle_growth: u32,
    /// K: the number of further Zero Length Commands, default 5; 0 sends none
    /// (REQ-NET-DATA-013, N-34).
    pub idle_further: u8,
    /// R: the number of FEC repeats, default 2; 0 turns FEC off (REQ-NET-FEC-001).
    pub fec_repeats: u8,
    /// The Retransmit option: serve Retransmit Requests and send them, default on
    /// (REQ-NET-RTX-009, N-38).
    pub retransmit: bool,
    /// D: the first delay before a Retransmit Request, default 10 ms (REQ-NET-RTX-009, N-35).
    pub request_delay_ms: u32,
    /// M: the maximum number of Retransmit Requests for a gap, default 3 (REQ-NET-RTX-010).
    pub max_requests: u8,
    /// The maximum number of UMP words in one UMP Data Command, 1 to 64, default 64
    /// (REQ-NET-DATA-007). The endpoint makes it smaller when the maximum UDP payload cannot
    /// hold a 64-word Command (REQ-NET-PKT-004).
    pub max_command_words: usize,
}

impl StreamConfig {
    /// The default values of DEC-NET-001.
    pub const DEFAULT: StreamConfig = StreamConfig {
        idle_interval_ms: 100,
        idle_growth: 2,
        idle_further: 5,
        fec_repeats: 2,
        retransmit: true,
        request_delay_ms: 10,
        max_requests: 3,
        max_command_words: MAX_UMP_DATA_WORDS,
    };

    /// Checks the values: T1 from 1 to 300 ms, G 1 or more (REQ-NET-DATA-017), and the
    /// Command size from 1 to 64 words.
    pub const fn check(&self) -> Result<(), StreamError> {
        if self.max_command_words == 0 || self.max_command_words > MAX_UMP_DATA_WORDS {
            return Err(StreamError::CommandWords);
        }
        if self.idle_interval_ms == 0 || self.idle_interval_ms > MAX_IDLE_INTERVAL_MS {
            return Err(StreamError::IdleInterval);
        }
        if self.idle_growth == 0 {
            return Err(StreamError::IdleGrowth);
        }
        Ok(())
    }
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A range of Sequence Numbers: `count` numbers from `first`, with wrap-around.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeqRange {
    /// The oldest number of the range.
    pub first: SeqNum,
    /// The quantity of numbers.
    pub count: u16,
}

impl SeqRange {
    /// True when `seq` is in the range.
    pub fn contains(&self, seq: SeqNum) -> bool {
        seq.0.wrapping_sub(self.first.0) < self.count
    }
}

/// The class of a received Sequence Number (REQ-NET-DATA-016).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// After the highest number H: `gap` numbers between H and it were not received.
    New {
        /// The quantity of numbers skipped (d - 1).
        gap: u16,
    },
    /// One of the W numbers before H, not received before: processed with a late mark (N-02).
    Late,
    /// Already processed, or too old for the window: skipped (REQ-NET-DATA-008).
    Duplicate,
}

/// The result of [`ReceiveWindow::accept`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepted {
    /// The class of the number.
    pub class: Class,
    /// Numbers that were not received and that left the window with this number: a late
    /// copy of them is now skipped. The range goes from the oldest to the newest of them;
    /// numbers in it that were received are not lost.
    pub lost: Option<SeqRange>,
}

/// The receive state of one session: the highest processed Sequence Number H and a bit map
/// of the W numbers before H (REQ-NET-DATA-009, N-01). W is from 1 to 128 (checked at compile
/// time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiveWindow<const W: usize = 64> {
    highest: SeqNum,
    // Bit i: the number H - 1 - i was processed.
    bits: u128,
}

const fn low_mask(n: u32) -> u128 {
    if n >= 128 {
        u128::MAX
    } else {
        (1u128 << n) - 1
    }
}

impl<const W: usize> Default for ReceiveWindow<W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const W: usize> ReceiveWindow<W> {
    const MASK: u128 = {
        assert!(W >= 1 && W <= 128, "the receive window W must be 1 to 128");
        low_mask(W as u32)
    };

    /// The start state: H = 0xFFFF with H and the W numbers before it processed, so that
    /// 0x0000 is the next number (N-03).
    #[inline(always)]
    pub const fn new() -> Self {
        Self::starting_at(SeqNum(0xFFFF))
    }

    /// A window with H = `highest` and H and the W numbers before it processed.
    pub const fn starting_at(highest: SeqNum) -> Self {
        ReceiveWindow {
            highest,
            bits: Self::MASK,
        }
    }

    /// Back to the start state (session start, Session Reset).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// The highest processed number H.
    pub const fn highest(&self) -> SeqNum {
        self.highest
    }

    /// The class of `seq`, with no change of the state.
    pub fn classify(&self, seq: SeqNum) -> Class {
        let d = seq.0.wrapping_sub(self.highest.0);
        if (1..=0x7FFF).contains(&d) {
            return Class::New { gap: d - 1 };
        }
        match self.back_bit(seq) {
            Some(bit) if self.bits & bit == 0 => Class::Late,
            _ => Class::Duplicate,
        }
    }

    // The bit of `seq` when it is one of the W numbers before H.
    fn back_bit(&self, seq: SeqNum) -> Option<u128> {
        let back = usize::from(self.highest.0.wrapping_sub(seq.0));
        (1..=W).contains(&back).then(|| 1u128 << (back - 1))
    }

    /// Classifies `seq` and marks it as processed (REQ-NET-DATA-016).
    pub fn accept(&mut self, seq: SeqNum) -> Accepted {
        let d = seq.0.wrapping_sub(self.highest.0);
        if !(1..=0x7FFF).contains(&d) {
            let class = match self.back_bit(seq) {
                Some(bit) if self.bits & bit == 0 => {
                    self.bits |= bit;
                    Class::Late
                }
                _ => Class::Duplicate,
            };
            return Accepted { class, lost: None };
        }
        let d = u32::from(d);
        let w = W as u32;
        // After the shift, bit i moves to i + d; the old H goes to d - 1; the skipped numbers
        // take the bits 0 to d - 2. Bits at W or above leave the window.
        let missing = !self.bits & Self::MASK;
        let old_lost = if d >= w {
            missing
        } else {
            missing & !low_mask(w - d)
        };
        let gap_lost = d >= w + 2;
        let oldest = if old_lost != 0 {
            Some(127 - old_lost.leading_zeros() + d)
        } else if gap_lost {
            Some(d - 2)
        } else {
            None
        };
        let newest = if gap_lost {
            w
        } else {
            old_lost.trailing_zeros() + d
        };
        let lost = oldest.map(|o| SeqRange {
            first: SeqNum(seq.0.wrapping_sub(1).wrapping_sub(o as u16)),
            count: (o - newest + 1) as u16,
        });
        let shifted = if d < 128 { self.bits << d } else { 0 };
        let h = if d - 1 < 128 { 1u128 << (d - 1) } else { 0 };
        self.bits = (shifted | h) & Self::MASK;
        self.highest = seq;
        Accepted {
            class: Class::New {
                gap: (d - 1) as u16,
            },
            lost,
        }
    }

    /// True when `seq` is H or one of the W numbers before H that was processed.
    pub fn is_processed(&self, seq: SeqNum) -> bool {
        seq == self.highest || self.back_bit(seq).is_some_and(|bit| self.bits & bit != 0)
    }

    // The positions of the oldest and the newest missing numbers in the window.
    fn missing_positions(&self) -> Option<(u32, u32)> {
        let m = !self.bits & Self::MASK;
        (m != 0).then(|| (127 - m.leading_zeros(), m.trailing_zeros()))
    }

    /// The numbers in the window that were not received: from the oldest to the newest of
    /// them.
    pub fn missing(&self) -> Option<SeqRange> {
        self.missing_positions().map(|(o, n)| SeqRange {
            first: SeqNum(self.highest.0.wrapping_sub(1).wrapping_sub(o as u16)),
            count: (o - n + 1) as u16,
        })
    }

    /// The Retransmit Request for the missing numbers: the oldest missing number and the
    /// quantity of numbers from it up to H, H not included (REQ-NET-RTX-009, N-36).
    pub fn request(&self) -> Option<RetransmitRequest> {
        self.missing_positions().map(|(o, _)| RetransmitRequest {
            seq: SeqNum(self.highest.0.wrapping_sub(1).wrapping_sub(o as u16)),
            count: (o + 1) as u16,
        })
    }

    /// Marks all the numbers of the window as processed (after a data loss).
    pub fn mark_all(&mut self) {
        self.bits = Self::MASK;
    }

    /// Marks the missing numbers of the window that are in `range` as processed. Gives the
    /// range from the oldest to the newest of them; None when none was missing.
    pub fn mark_range(&mut self, range: SeqRange) -> Option<SeqRange> {
        let mut oldest: Option<SeqNum> = None;
        let mut newest = SeqNum(0);
        // From the oldest position (W) to the newest (1).
        for back in (1..=W).rev() {
            let seq = SeqNum(self.highest.0.wrapping_sub(back as u16));
            let bit = 1u128 << (back - 1);
            if self.bits & bit == 0 && range.contains(seq) {
                self.bits |= bit;
                oldest.get_or_insert(seq);
                newest = seq;
            }
        }
        oldest.map(|first| SeqRange {
            first,
            count: newest.0.wrapping_sub(first.0).wrapping_add(1),
        })
    }
}

/// The result of [`Receiver::on_ump_data`]: what to give the caller for one received UMP
/// Data Command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivery<'a> {
    /// The Sequence Number of the Command.
    pub seq: SeqNum,
    /// Its class.
    pub class: Class,
    /// The numbers skipped before it: the first number not received (H + 1, H before the
    /// update) and the quantity (REQ-NET-DATA-010).
    pub gap: Option<SeqRange>,
    /// The numbers that left the window without being received (see [`Accepted::lost`]).
    pub lost: Option<SeqRange>,
    data: UmpData<'a>,
}

impl<'a> Delivery<'a> {
    /// True when the UMP messages go to the caller (new or late).
    pub fn delivers(&self) -> bool {
        !matches!(self.class, Class::Duplicate)
    }

    /// True for a late Command: its messages have a late mark (REQ-NET-DATA-010, N-02).
    pub fn is_late(&self) -> bool {
        matches!(self.class, Class::Late)
    }

    /// The UMP messages for the caller; none for a skipped Command (REQ-NET-DATA-008) and
    /// for a Zero Length Command (REQ-NET-DATA-011). [`UmpMessages::truncated`] shows a
    /// message cut at the end (NAK 0x03, REQ-NET-DATA-006).
    pub fn messages(&self) -> UmpMessages<'a> {
        if self.delivers() {
            self.data.messages()
        } else {
            let empty = RawCommand {
                header: Header::from_bytes([CommandCode::UmpData.into_bits(), 0, 0, 0]),
                payload: &[],
            };
            UmpData::read(&empty).messages()
        }
    }
}

/// An event of the stream for the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEvent {
    /// The numbers of `range` that were not received are lost: no answer to the Retransmit
    /// Requests, a NAK for them (`error` None), or a Retransmit Error (`error` Some). They
    /// are now marked as processed. The crate starts no recovery action (REQ-NET-RTX-010,
    /// -011, -013).
    DataLoss {
        /// From the oldest to the newest number not received; numbers in it that were
        /// received were given to the caller before.
        range: SeqRange,
        /// The received Retransmit Error, with its Error Reason and Sequence Number field.
        error: Option<RetransmitError>,
    },
    /// A Retransmit Error with no open Retransmit Request of this side.
    RetransmitError(RetransmitError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RequestTimer {
    due: u64,
    // Requests written for this gap.
    sent: u8,
}

/// The receive direction: the [`ReceiveWindow`] and the Retransmit Request scheduler.
#[derive(Debug, Clone)]
///
/// It holds no [`StreamConfig`]: the methods that use the configuration take it, so that the
/// sessions of one Host keep one copy (OPT-009).
pub struct Receiver<const W: usize = 64> {
    window: ReceiveWindow<W>,
    timer: Option<RequestTimer>,
    request_due: bool,
    refused: bool,
    // The last written Retransmit Request that has no answer yet.
    asked: Option<RetransmitRequest>,
}

impl<const W: usize> Default for Receiver<W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const W: usize> Receiver<W> {
    /// A receiver in the start state.
    #[inline(always)]
    pub const fn new() -> Self {
        Receiver {
            window: ReceiveWindow::new(),
            timer: None,
            request_due: false,
            refused: false,
            asked: None,
        }
    }

    /// The receive window.
    pub const fn window(&self) -> &ReceiveWindow<W> {
        &self.window
    }

    /// Back to the start state (session start, Session Reset; N-03).
    pub fn reset(&mut self) {
        self.window.reset();
        self.timer = None;
        self.request_due = false;
        self.refused = false;
        self.asked = None;
    }

    /// True while this side may write Retransmit Requests: the Retransmit option is on and
    /// the peer did not answer NAK 0x01 (REQ-NET-RTX-011, REQ-NET-CMD-003).
    pub const fn requests_enabled(&self, cfg: &StreamConfig) -> bool {
        cfg.retransmit && !self.refused
    }

    // The delay before the next step after `sent` requests: D, 2D, 4D...
    fn delay(cfg: &StreamConfig, sent: u8) -> u64 {
        u64::from(cfg.request_delay_ms).saturating_mul(1u64 << sent.min(32))
    }

    /// Processes one received UMP Data Command (REQ-NET-DATA-008, -010, -011, -016).
    pub fn on_ump_data<'a>(
        &mut self,
        cfg: &StreamConfig,
        now: u64,
        data: UmpData<'a>,
    ) -> Delivery<'a> {
        let before = self.window.highest();
        let acc = self.window.accept(data.seq);
        let gap = match acc.class {
            Class::New { gap } if gap > 0 => Some(SeqRange {
                first: before.next(),
                count: gap,
            }),
            _ => None,
        };
        if self.window.missing().is_none() {
            self.timer = None;
            self.request_due = false;
            self.asked = None;
        } else if gap.is_some()
            && self.requests_enabled(cfg)
            && self.timer.is_none_or(|t| t.sent > 0)
        {
            // A new gap starts the schedule again, also after requests of an earlier gap:
            // each missing number gets the full M requests (REQ-NET-RTX-009, -010). Before
            // the first request the schedule stays: that request holds the new gap too.
            self.timer = Some(RequestTimer {
                due: now.saturating_add(Self::delay(cfg, 0)),
                sent: 0,
            });
        }
        Delivery {
            seq: data.seq,
            class: acc.class,
            gap,
            lost: acc.lost,
            data,
        }
    }

    /// The time of the next Retransmit Request step.
    pub fn next_deadline(&self) -> Option<u64> {
        self.timer.map(|t| t.due)
    }

    // A data loss for the missing numbers; all are marked as processed.
    fn loss(&mut self, error: Option<RetransmitError>) -> Option<StreamEvent> {
        self.timer = None;
        self.request_due = false;
        self.asked = None;
        let range = self.window.missing()?;
        self.window.mark_all();
        Some(StreamEvent::DataLoss { range, error })
    }

    /// Runs the Retransmit Request timer (REQ-NET-RTX-009, -010): at the end of each delay,
    /// it asks for a request (written by [`Receiver::write_request`]) and doubles the delay;
    /// after M requests and one more delay, it gives a data loss. It never asks for a Bye.
    pub fn poll(&mut self, cfg: &StreamConfig, now: u64) -> Option<StreamEvent> {
        let t = self.timer?;
        if now < t.due {
            return None;
        }
        if self.window.missing().is_none() {
            self.timer = None;
            self.request_due = false;
            self.asked = None;
            return None;
        }
        if t.sent < cfg.max_requests && self.requests_enabled(cfg) {
            let sent = t.sent + 1;
            self.request_due = true;
            self.timer = Some(RequestTimer {
                due: now.saturating_add(Self::delay(cfg, sent)),
                sent,
            });
            return None;
        }
        self.loss(None)
    }

    /// True when a Retransmit Request waits for [`Receiver::write_request`].
    pub const fn request_due(&self) -> bool {
        self.request_due
    }

    fn request_open(&self) -> bool {
        self.asked.is_some()
    }

    /// A received Retransmit Error (REQ-NET-RTX-013): for an open request, it gives a data
    /// loss with the error for the missing numbers of that request (up to the H of the
    /// request); the numbers that went missing after it are still requested. With no open
    /// request, it gives the error.
    pub fn on_retransmit_error(&mut self, err: RetransmitError) -> StreamEvent {
        let Some(req) = self.asked.take() else {
            return StreamEvent::RetransmitError(err);
        };
        let range = SeqRange {
            first: req.seq,
            count: req.count,
        };
        let lost = self.window.mark_range(range);
        if self.window.missing().is_none() {
            self.timer = None;
            self.request_due = false;
        }
        match lost {
            Some(range) => StreamEvent::DataLoss {
                range,
                error: Some(err),
            },
            None => StreamEvent::RetransmitError(err),
        }
    }

    /// A received NAK 0x01 for a Retransmit Request (REQ-NET-RTX-011): no more requests in
    /// this session, and a data loss for an open request.
    pub fn on_retransmit_nak(&mut self) -> Option<StreamEvent> {
        self.refused = true;
        let open = self.request_open();
        self.timer = None;
        self.request_due = false;
        if open {
            self.loss(None)
        } else {
            None
        }
    }

    /// Writes the Retransmit Request that is due (REQ-NET-RTX-009, N-36). Gives true when it
    /// does not fit in this UDP payload.
    pub fn write_request(
        &mut self,
        cfg: &StreamConfig,
        w: &mut PayloadWriter<'_>,
    ) -> Result<bool, StreamError> {
        if !self.request_due {
            return Ok(false);
        }
        let enabled = self.requests_enabled(cfg);
        let Some(req) = self.window.request().filter(|_| enabled) else {
            self.request_due = false;
            return Ok(false);
        };
        match req.write(w) {
            Ok(()) => {
                self.request_due = false;
                self.asked = Some(req);
                Ok(false)
            }
            Err(e) => more_or_err(e),
        }
    }
}

fn packet_error(e: CommandError) -> PacketError {
    match e {
        CommandError::Packet(p) => p,
        _ => PacketError::InvalidValue,
    }
}

// Overflow: write the rest in a new UDP payload; any other error goes to the caller.
fn more_or_err(e: CommandError) -> Result<bool, StreamError> {
    match packet_error(e) {
        PacketError::Overflow => Ok(true),
        p => Err(p.into()),
    }
}

/// The send history of one session: one ring of C 32-bit words that holds the last UMP Data
/// Commands, each as one header word (Sequence Number and pl) and pl payload words, in send
/// order (REQ-NET-RTX-003, N-39, N-40). The numbers in it follow each other with no gap. C is
/// 65 or more (checked at compile time).
#[derive(Debug, Clone)]
pub struct SendHistory<const C: usize = 256> {
    buf: [u32; C],
    // Index of the header word of the oldest entry.
    head: usize,
    // Words in use.
    used: usize,
    // Entries.
    count: usize,
    // Sequence Number of the oldest entry; the next number when empty.
    oldest: SeqNum,
    // Index of the header word of the newest entry.
    last: usize,
}

impl<const C: usize> Default for SendHistory<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const C: usize> SendHistory<C> {
    const CHECK: () = {
        assert!(
            C > MAX_UMP_DATA_WORDS,
            "the send history C must hold one full Command (65 words)"
        );
        // Each entry takes one word or more, so the numbers in the history span at most C:
        // they stay in the serial half range.
        assert!(
            C <= 32768,
            "the send history C must be 32768 words or less (serial half range)"
        );
    };

    /// An empty history; the next number is 0x0000.
    #[inline(always)]
    pub const fn new() -> Self {
        let () = Self::CHECK;
        SendHistory {
            buf: [0; C],
            head: 0,
            used: 0,
            count: 0,
            oldest: SeqNum(0),
            last: 0,
        }
    }

    /// Empties the history; the next number is 0x0000 again.
    pub fn clear(&mut self) {
        self.head = 0;
        self.used = 0;
        self.count = 0;
        self.oldest = SeqNum(0);
        self.last = 0;
    }

    /// The number of Commands held.
    pub const fn len(&self) -> usize {
        self.count
    }

    /// True when no Command is held.
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The words in use.
    pub const fn words(&self) -> usize {
        self.used
    }

    /// The number of the oldest Command; the next number when the history is empty.
    pub const fn oldest(&self) -> SeqNum {
        self.oldest
    }

    /// The number that the next Command gets.
    pub const fn next_seq(&self) -> SeqNum {
        self.oldest.add(self.count as u16)
    }

    /// True when the history holds the Command `seq`.
    pub fn contains(&self, seq: SeqNum) -> bool {
        usize::from(seq.0.wrapping_sub(self.oldest.0)) < self.count
    }

    fn pl(&self, idx: usize) -> usize {
        (self.buf[idx % C] & 0xFF) as usize
    }

    fn next_index(&self, idx: usize) -> usize {
        (idx + 1 + self.pl(idx)) % C
    }

    // The header index of entry k (0 = oldest).
    fn index_of(&self, k: usize) -> usize {
        (0..k).fold(self.head, |idx, _| self.next_index(idx))
    }

    fn pop_oldest(&mut self) {
        let n = 1 + self.pl(self.head);
        self.head = (self.head + n) % C;
        self.used -= n;
        self.count -= 1;
        self.oldest = self.oldest.next();
    }

    // Adds one Command (at most 64 words), removing the oldest Commands first.
    fn push(&mut self, words: &[u32]) {
        let need = 1 + words.len();
        while C - self.used < need && self.count > 0 {
            self.pop_oldest();
        }
        let seq = self.next_seq();
        let idx = (self.head + self.used) % C;
        self.buf[idx] = (u32::from(seq.0) << 16) | words.len() as u32;
        for (j, w) in words.iter().enumerate() {
            self.buf[(idx + 1 + j) % C] = *w;
        }
        self.used += need;
        self.count += 1;
        self.last = idx;
    }

    // Adds words to the newest Command, removing the oldest Commands first when the ring has
    // no room (the caller checks that the newest Command and the words fit).
    fn append_to_last(&mut self, words: &[u32]) {
        while C - self.used < words.len() && self.count > 1 {
            self.pop_oldest();
        }
        let end = (self.head + self.used) % C;
        for (j, w) in words.iter().enumerate() {
            self.buf[(end + j) % C] = *w;
        }
        self.buf[self.last] += words.len() as u32;
        self.used += words.len();
    }

    // Writes the entry at `idx` as a UMP Data Command with its own Sequence Number.
    fn write_entry(&self, w: &mut PayloadWriter<'_>, idx: usize) -> Result<(), PacketError> {
        let hdr = self.buf[idx % C];
        let [a, b] = ((hdr >> 16) as u16).to_be_bytes();
        let pl = (hdr & 0xFF) as usize;
        w.command(CommandCode::UmpData.into_bits(), a, b, |p| {
            (0..pl).try_for_each(|j| p.word(self.buf[(idx + 1 + j) % C]))
        })
    }
}

/// Why a Retransmit Request was not queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignored {
    /// A request with the same Sequence Number waits already (7.2.3).
    Pending,
    /// The queue is full; the peer repeats the request (N-41).
    Full,
}

/// A FIFO of fixed capacity N in a ring: no entry moves on a removal at the front, and a
/// removal in the middle builds a new ring of the kept entries. A `copy_within` shift links
/// `memmove` (about 1.4 KB) into each Network MIDI firmware (OPT-047). `head` and `len` are
/// `u16`, so the ring is not larger than the old array with a `usize` length (the C ABI storage
/// sizes of zmidi-ffi depend on it).
#[derive(Debug, Clone)]
pub(crate) struct Ring<T, const N: usize> {
    items: [T; N],
    head: u16,
    len: u16,
}

impl<T: Copy, const N: usize> Ring<T, N> {
    const CHECK: () = assert!(
        N <= u16::MAX as usize,
        "a queue capacity must be 65535 or less"
    );

    /// An empty ring; `fill` is the value of the unused slots.
    pub(crate) const fn new(fill: T) -> Self {
        let () = Self::CHECK;
        Ring {
            items: [fill; N],
            head: 0,
            len: 0,
        }
    }

    pub(crate) const fn len(&self) -> usize {
        self.len as usize
    }

    pub(crate) fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    /// The slot of entry `i` (`i <= N`, `head < N`).
    fn slot(&self, i: usize) -> usize {
        let j = usize::from(self.head) + i;
        if j >= N {
            j - N
        } else {
            j
        }
    }

    /// Entry `i`, the oldest first.
    pub(crate) fn get(&self, i: usize) -> Option<T> {
        if i < self.len() {
            self.items.get(self.slot(i)).copied()
        } else {
            None
        }
    }

    pub(crate) fn get_mut(&mut self, i: usize) -> Option<&mut T> {
        if i < self.len() {
            let k = self.slot(i);
            self.items.get_mut(k)
        } else {
            None
        }
    }

    /// Adds `t` at the end; gives it back when the ring is full.
    pub(crate) fn push_back(&mut self, t: T) -> Result<(), T> {
        if self.len() >= N {
            return Err(t);
        }
        let k = self.slot(self.len());
        match self.items.get_mut(k) {
            Some(s) => {
                *s = t;
                self.len += 1;
                Ok(())
            }
            None => Err(t),
        }
    }

    /// Removes the `n` oldest entries.
    pub(crate) fn drop_front(&mut self, n: usize) {
        let n = n.min(self.len());
        // Both are at most N, which CHECK keeps within u16.
        self.head = self.slot(n) as u16;
        self.len -= n as u16;
    }

    pub(crate) fn pop_front(&mut self) -> Option<T> {
        let f = self.get(0)?;
        self.drop_front(1);
        Some(f)
    }

    /// Removes entry `i`; the later entries keep their order.
    pub(crate) fn remove(&mut self, i: usize) {
        if i >= self.len() {
            return;
        }
        // A new ring of the kept entries: copies between two separate values, so no `memmove`
        // (an in-place shift links it, OPT-047).
        let mut kept = Ring {
            items: self.items,
            head: 0,
            len: 0,
        };
        for j in 0..self.len() {
            if j != i {
                if let Some(e) = self.get(j) {
                    let _ = kept.push_back(e);
                }
            }
        }
        *self = kept;
    }

    /// The entries, the oldest first.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
        (0..self.len()).filter_map(move |i| self.items.get(self.slot(i)))
    }

    /// The index of the newest entry for which `f` is true.
    pub(crate) fn rposition(&self, f: impl Fn(&T) -> bool) -> Option<usize> {
        (0..self.len())
            .rev()
            .find(|&i| self.get(i).is_some_and(|t| f(&t)))
    }
}

/// The accepted Retransmit Requests that are not served yet: a FIFO of fixed capacity Q
/// (REQ-NET-RTX-008, N-41).
#[derive(Debug, Clone)]
pub struct RetransmitQueue<const Q: usize = 4> {
    ring: Ring<RetransmitRequest, Q>,
}

impl<const Q: usize> Default for RetransmitQueue<Q> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const Q: usize> RetransmitQueue<Q> {
    /// An empty queue.
    #[inline(always)]
    pub const fn new() -> Self {
        RetransmitQueue {
            ring: Ring::new(RetransmitRequest {
                seq: SeqNum(0),
                count: 0,
            }),
        }
    }

    /// The number of requests held.
    pub const fn len(&self) -> usize {
        self.ring.len()
    }

    /// True when no request waits.
    pub const fn is_empty(&self) -> bool {
        self.ring.len() == 0
    }

    /// Empties the queue.
    pub fn clear(&mut self) {
        self.ring.clear();
    }

    /// Adds a request at the end; a request with the same Sequence Number or a full queue
    /// ignores it.
    pub fn push(&mut self, req: RetransmitRequest) -> Result<(), Ignored> {
        if self.ring.iter().any(|r| r.seq == req.seq) {
            return Err(Ignored::Pending);
        }
        self.ring.push_back(req).map_err(|_| Ignored::Full)
    }

    /// The oldest request.
    pub fn front(&self) -> Option<RetransmitRequest> {
        self.ring.get(0)
    }

    fn set_front(&mut self, req: RetransmitRequest) {
        if let Some(f) = self.ring.get_mut(0) {
            *f = req;
        }
    }

    /// Removes the oldest request.
    pub fn pop(&mut self) -> Option<RetransmitRequest> {
        self.ring.pop_front()
    }
}

/// The answer to a received Retransmit Request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOutcome {
    /// Queued; the Commands are resent by the next write (REQ-NET-RTX-005, -015).
    Queued,
    /// Not queued (REQ-NET-RTX-008).
    Ignored(Ignored),
    /// The first number is not in the send history: the caller writes this Retransmit Error
    /// and nothing is resent (REQ-NET-RTX-006, N-07, N-08).
    Error(RetransmitError),
    /// The Retransmit option is off: the caller answers NAK 0x01 (N-38).
    NotSupported,
}

/// The result of a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Written {
    /// The Commands written into the UDP payload.
    pub commands: usize,
    /// True when more Commands wait: send this UDP payload and write again.
    pub more: bool,
}

impl Written {
    fn add(&mut self, other: Written) {
        self.commands += other.commands;
        self.more |= other.more;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Idle {
    due: u64,
    // Zero Length Commands queued in this idle period.
    step: u16,
    interval: u64,
}

// The free words of the writer, header words included: probe with a Command that is not kept.
fn free_words(w: &mut PayloadWriter<'_>) -> usize {
    let mut n = 0usize;
    let r = w.command(0, 0, 0, |p| {
        while n < 400 && p.word(0).is_ok() {
            n += 1;
        }
        Err(PacketError::InvalidValue)
    });
    match r {
        Err(PacketError::InvalidValue) => n + 1,
        _ => 0,
    }
}

/// The send direction: the Sequence Number counter, the [`SendHistory`], the FEC repeats,
/// the idle schedule and the [`RetransmitQueue`].
#[derive(Debug, Clone)]
///
/// It holds no [`StreamConfig`]: the methods that use the configuration take it (OPT-009).
pub struct Sender<const C: usize = 256, const Q: usize = 4> {
    history: SendHistory<C>,
    queue: RetransmitQueue<Q>,
    // The newest entries of the history that are not written yet, and their words.
    unsent: usize,
    unsent_words: usize,
    idle: Option<Idle>,
}

impl<const C: usize, const Q: usize> Default for Sender<C, Q> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const C: usize, const Q: usize> Sender<C, Q> {
    /// A sender in the start state (next number 0x0000).
    #[inline(always)]
    pub const fn new() -> Self {
        Sender {
            history: SendHistory::new(),
            queue: RetransmitQueue::new(),
            unsent: 0,
            unsent_words: 0,
            idle: None,
        }
    }

    /// The send history.
    pub const fn history(&self) -> &SendHistory<C> {
        &self.history
    }

    /// The Retransmit Requests that wait.
    pub const fn queue(&self) -> &RetransmitQueue<Q> {
        &self.queue
    }

    /// Back to the start state: empty history and queue, next number 0x0000, no idle
    /// schedule (REQ-NET-DATA-004, REQ-NET-RTX-003).
    pub fn reset(&mut self) {
        self.history.clear();
        self.queue.clear();
        self.unsent = 0;
        self.unsent_words = 0;
        self.idle = None;
    }

    /// The number that the next written Command gets.
    pub fn next_seq(&self) -> SeqNum {
        self.history
            .oldest()
            .add((self.history.len() - self.unsent) as u16)
    }

    /// Queues whole UMP messages. They join the last queued Command while it has room, and
    /// are divided at message boundaries into Commands of at most 64 words, each with the
    /// next Sequence Number (REQ-NET-DATA-004, -007). On an error nothing is queued.
    pub fn send_ump(&mut self, cfg: &StreamConfig, words: &[u32]) -> Result<(), StreamError> {
        if words.is_empty() {
            return Ok(());
        }
        if !whole_messages(words) {
            return Err(StreamError::InvalidUmpData);
        }
        let max = cfg.max_command_words;
        // A message longer than one Command cannot be sent (REQ-NET-PKT-004).
        let mut i = 0;
        while i < words.len() {
            let m = message_words(words[i]);
            if m > max {
                return Err(PacketError::TooLarge.into());
            }
            i += m;
        }
        let room = if self.unsent > 0 {
            max.saturating_sub(self.history.pl(self.history.last))
        } else {
            0
        };
        // Words that join the last queued Command.
        let mut split = 0;
        while split < words.len() {
            let m = message_words(words[split]);
            if split + m > room {
                break;
            }
            split += m;
        }
        let (join, rest) = words.split_at(split);
        let need = join.len() + rest.len() + Self::commands(rest, max);
        if need > C - self.unsent_words {
            return Err(StreamError::Full);
        }
        if !join.is_empty() {
            self.history.append_to_last(join);
            self.unsent_words += join.len();
        }
        let mut rest = rest;
        while !rest.is_empty() {
            let n = Self::first_command(rest, max);
            let (cmd, tail) = rest.split_at(n);
            self.history.push(cmd);
            self.unsent += 1;
            self.unsent_words += 1 + cmd.len();
            rest = tail;
        }
        Ok(())
    }

    // The words of the first Command of `words`: whole messages, at most `max` words.
    fn first_command(words: &[u32], max: usize) -> usize {
        let mut n = 0;
        while n < words.len() {
            let m = message_words(words[n]);
            if n + m > max {
                break;
            }
            n += m;
        }
        n
    }

    // The number of Commands for `words`.
    fn commands(words: &[u32], max: usize) -> usize {
        let mut rest = words;
        let mut c = 0;
        while !rest.is_empty() {
            let n = Self::first_command(rest, max).clamp(1, rest.len());
            rest = &rest[n..];
            c += 1;
        }
        c
    }

    /// A received Retransmit Request (REQ-NET-RTX-005, -006, -008, N-07, N-08, N-37, N-41).
    pub fn on_retransmit_request(
        &mut self,
        cfg: &StreamConfig,
        req: RetransmitRequest,
    ) -> RequestOutcome {
        if !cfg.retransmit {
            return RequestOutcome::NotSupported;
        }
        let written = self.history.len() - self.unsent;
        let k = usize::from(req.seq.0.wrapping_sub(self.history.oldest().0));
        if k >= written {
            return RequestOutcome::Error(RetransmitError {
                reason: RetransmitErrorReason::NOT_IN_BUFFER,
                seq: self.history.oldest(),
            });
        }
        match self.queue.push(req) {
            Ok(()) => RequestOutcome::Queued,
            Err(i) => RequestOutcome::Ignored(i),
        }
    }

    /// The time of the next idle Zero Length Command (REQ-NET-DATA-012).
    pub fn next_deadline(&self) -> Option<u64> {
        if self.unsent > 0 {
            return None;
        }
        self.idle.map(|i| i.due)
    }

    /// True when a write has Commands to write at `now`.
    pub fn wants_write(&self, now: u64) -> bool {
        self.unsent > 0 || !self.queue.is_empty() || self.idle.is_some_and(|i| now >= i.due)
    }

    /// Writes the resent Commands first (REQ-NET-RTX-015), then the new Commands with their
    /// FEC repeats (REQ-NET-FEC-001, -002), and a Zero Length Command when the idle schedule
    /// is due (REQ-NET-DATA-012, -013, REQ-NET-FEC-003).
    pub fn write(
        &mut self,
        cfg: &StreamConfig,
        now: u64,
        w: &mut PayloadWriter<'_>,
    ) -> Result<Written, StreamError> {
        let mut out = self.write_retransmits(w)?;
        if !out.more {
            out.add(self.write_new(cfg, now, w)?);
        }
        Ok(out)
    }

    fn write_retransmits(&mut self, w: &mut PayloadWriter<'_>) -> Result<Written, StreamError> {
        let mut out = Written::default();
        while let Some(job) = self.queue.front() {
            let written = self.history.len() - self.unsent;
            let k = usize::from(job.seq.0.wrapping_sub(self.history.oldest().0));
            if k >= written {
                // Removed from the history after the request was accepted.
                let err = RetransmitError {
                    reason: RetransmitErrorReason::NOT_IN_BUFFER,
                    seq: self.history.oldest(),
                };
                match err.write(w) {
                    Ok(()) => out.commands += 1,
                    // Too large for any payload: skip it, the peer repeats the request.
                    Err(CommandError::Packet(PacketError::TooLarge)) => {}
                    Err(e) => {
                        out.more = more_or_err(e)?;
                        return Ok(out);
                    }
                }
                self.queue.pop();
                continue;
            }
            let end = if job.count == 0 {
                written - 1
            } else {
                (k + usize::from(job.count) - 1).min(written - 1)
            };
            let mut idx = self.history.index_of(k);
            for j in k..=end {
                if let Err(e) = self.history.write_entry(w, idx) {
                    if e == PacketError::TooLarge {
                        // The Command does not fit in an empty payload: drop this request
                        // so that it does not block the stream (popped below).
                        break;
                    }
                    out.more = more_or_err(CommandError::Packet(e))?;
                    self.queue.set_front(RetransmitRequest {
                        seq: self.history.oldest().add(j as u16),
                        count: (end - j + 1) as u16,
                    });
                    return Ok(out);
                }
                out.commands += 1;
                idx = self.history.next_index(idx);
            }
            self.queue.pop();
        }
        Ok(out)
    }

    // Queues the Zero Length Command of the idle schedule when it is due.
    fn queue_idle(&mut self, cfg: &StreamConfig, now: u64) {
        let Some(mut idle) = self.idle else { return };
        if self.unsent > 0 || now < idle.due {
            return;
        }
        self.history.push(&[]);
        self.unsent += 1;
        self.unsent_words += 1;
        idle.step += 1;
        let r = u16::from(cfg.fec_repeats);
        let k = u16::from(cfg.idle_further);
        self.idle = if idle.step <= r {
            // The FEC idle payloads, at T1 (REQ-NET-FEC-003).
            idle.due = now.saturating_add(idle.interval);
            Some(idle)
        } else if idle.step < 1 + r + k {
            // The further Commands, each interval G times the one before (REQ-NET-DATA-013).
            idle.interval = idle.interval.saturating_mul(u64::from(cfg.idle_growth));
            idle.due = now.saturating_add(idle.interval);
            Some(idle)
        } else {
            None
        };
    }

    fn write_new(
        &mut self,
        cfg: &StreamConfig,
        now: u64,
        w: &mut PayloadWriter<'_>,
    ) -> Result<Written, StreamError> {
        let mut out = Written::default();
        self.queue_idle(cfg, now);
        if self.unsent == 0 {
            return Ok(out);
        }
        let written = self.history.len() - self.unsent;
        let first = self.history.index_of(written);
        let size0 = 1 + self.history.pl(first);
        let free = free_words(w);
        if free < size0 {
            if w.is_empty() {
                return Err(PacketError::TooLarge.into());
            }
            out.more = true;
            return Ok(out);
        }
        // The FEC repeats: the last R written Commands; the oldest leave first when the
        // payload has no room for all of them (REQ-NET-FEC-001).
        let r = usize::from(cfg.fec_repeats).min(written);
        let mut start = written - r;
        let mut idx = self.history.index_of(start);
        let mut total: usize = {
            let mut i = idx;
            (start..written)
                .map(|_| {
                    let s = 1 + self.history.pl(i);
                    i = self.history.next_index(i);
                    s
                })
                .sum()
        };
        while total > free - size0 {
            total -= 1 + self.history.pl(idx);
            idx = self.history.next_index(idx);
            start += 1;
        }
        for _ in start..written {
            if let Err(e) = self.history.write_entry(w, idx) {
                out.more = more_or_err(CommandError::Packet(e))?;
                return Ok(out);
            }
            out.commands += 1;
            idx = self.history.next_index(idx);
        }
        let mut data = false;
        while self.unsent > 0 {
            let pl = self.history.pl(idx);
            if let Err(e) = self.history.write_entry(w, idx) {
                out.more = more_or_err(CommandError::Packet(e))?;
                break;
            }
            out.commands += 1;
            data |= pl > 0;
            self.unsent -= 1;
            self.unsent_words -= 1 + pl;
            idx = self.history.next_index(idx);
        }
        if data {
            let t1 = u64::from(cfg.idle_interval_ms);
            self.idle = Some(Idle {
                due: now.saturating_add(t1),
                step: 0,
                interval: t1,
            });
        }
        Ok(out)
    }
}

/// The UMP Data path of one session in both directions: one [`Sender`] and one [`Receiver`].
/// W is the receive window (default 64, N-01), C the send history in words (default 256,
/// N-40, OPT-007) and Q the Retransmit Request queue (default 4, N-41).
#[derive(Debug, Clone)]
pub struct DataStream<const W: usize = 64, const C: usize = 256, const Q: usize = 4> {
    sender: Sender<C, Q>,
    receiver: Receiver<W>,
}

impl<const W: usize, const C: usize, const Q: usize> DataStream<W, C, Q> {
    /// A stream in the start state; `config` is checked (REQ-NET-DATA-017). The stream does
    /// not keep it: give the same configuration to each call (OPT-009).
    pub const fn new(config: StreamConfig) -> Result<Self, StreamError> {
        if let Err(e) = config.check() {
            return Err(e);
        }
        Ok(Self::checked())
    }

    /// The send direction.
    pub const fn sender(&self) -> &Sender<C, Q> {
        &self.sender
    }

    /// The receive direction.
    pub const fn receiver(&self) -> &Receiver<W> {
        &self.receiver
    }

    /// Back to the start state in both directions (session start, Session Reset; N-03).
    pub fn reset(&mut self) {
        self.sender.reset();
        self.receiver.reset();
    }

    /// A stream in the start state for a configuration that the caller checked
    /// (`StreamConfig::check`). No `Result`, so that the compiler can build it in place (OPT-003).
    #[inline(always)]
    pub(crate) const fn checked() -> Self {
        DataStream {
            sender: Sender {
                history: SendHistory::new(),
                queue: RetransmitQueue::new(),
                unsent: 0,
                unsent_words: 0,
                idle: None,
            },
            receiver: Receiver {
                window: ReceiveWindow::new(),
                timer: None,
                request_due: false,
                refused: false,
                asked: None,
            },
        }
    }

    /// Queues whole UMP messages to send (see [`Sender::send_ump`]).
    pub fn send_ump(&mut self, cfg: &StreamConfig, words: &[u32]) -> Result<(), StreamError> {
        self.sender.send_ump(cfg, words)
    }

    /// A received UMP Data Command (see [`Receiver::on_ump_data`]).
    pub fn on_ump_data<'a>(
        &mut self,
        cfg: &StreamConfig,
        now: u64,
        data: UmpData<'a>,
    ) -> Delivery<'a> {
        self.receiver.on_ump_data(cfg, now, data)
    }

    /// A received Retransmit Request (see [`Sender::on_retransmit_request`]).
    pub fn on_retransmit_request(
        &mut self,
        cfg: &StreamConfig,
        req: RetransmitRequest,
    ) -> RequestOutcome {
        self.sender.on_retransmit_request(cfg, req)
    }

    /// A received Retransmit Error (see [`Receiver::on_retransmit_error`]).
    pub fn on_retransmit_error(&mut self, err: RetransmitError) -> StreamEvent {
        self.receiver.on_retransmit_error(err)
    }

    /// A received NAK 0x01 for a Retransmit Request (see [`Receiver::on_retransmit_nak`]).
    pub fn on_retransmit_nak(&mut self) -> Option<StreamEvent> {
        self.receiver.on_retransmit_nak()
    }

    /// Runs the timers; call it until it gives None.
    pub fn poll(&mut self, cfg: &StreamConfig, now: u64) -> Option<StreamEvent> {
        self.receiver.poll(cfg, now)
    }

    /// The earliest time at which [`DataStream::poll`] or [`DataStream::write_data`] has
    /// timed work. Queued work is shown by [`DataStream::wants_write`].
    pub fn next_deadline(&self) -> Option<u64> {
        match (self.sender.next_deadline(), self.receiver.next_deadline()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// True when [`DataStream::write_data`] has Commands to write at `now`.
    pub fn wants_write(&self, now: u64) -> bool {
        self.receiver.request_due() || self.sender.wants_write(now)
    }

    /// Writes into one UDP payload: the Retransmit Request that is due, then the resent
    /// Commands, then the new UMP Data Commands with their FEC repeats or the idle Zero
    /// Length Command. When [`Written::more`] is true, send the payload and call it again.
    pub fn write_data(
        &mut self,
        cfg: &StreamConfig,
        now: u64,
        w: &mut PayloadWriter<'_>,
    ) -> Result<Written, StreamError> {
        let mut out = Written::default();
        let before = w.len();
        if self.receiver.write_request(cfg, w)? {
            out.more = true;
            return Ok(out);
        }
        if w.len() != before {
            out.commands += 1;
        }
        out.add(self.sender.write(cfg, now, w)?);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::packet::{command_packets, MaxPayload, PacketItem};
    use proptest::prelude::*;
    use std::vec;
    use std::vec::Vec;

    type Stream = TestStream<64, 512, 4>;

    /// A stream and its configuration, with the methods of the stream before OPT-009 (the
    /// stream takes its configuration with each call; the test gives the same one each time).
    #[derive(Debug, Clone)]
    struct TestStream<const W: usize, const C: usize, const Q: usize> {
        s: DataStream<W, C, Q>,
        cfg: StreamConfig,
    }

    impl<const W: usize, const C: usize, const Q: usize> TestStream<W, C, Q> {
        fn new(cfg: StreamConfig) -> Result<Self, StreamError> {
            Ok(TestStream {
                s: DataStream::new(cfg)?,
                cfg,
            })
        }
        fn sender(&self) -> &Sender<C, Q> {
            self.s.sender()
        }
        fn receiver(&self) -> &Receiver<W> {
            self.s.receiver()
        }
        fn requests_enabled(&self) -> bool {
            self.s.receiver().requests_enabled(&self.cfg)
        }
        fn reset(&mut self) {
            self.s.reset()
        }
        fn send_ump(&mut self, words: &[u32]) -> Result<(), StreamError> {
            self.s.send_ump(&self.cfg, words)
        }
        fn on_ump_data<'a>(&mut self, now: u64, data: UmpData<'a>) -> Delivery<'a> {
            self.s.on_ump_data(&self.cfg, now, data)
        }
        fn on_retransmit_request(&mut self, req: RetransmitRequest) -> RequestOutcome {
            self.s.on_retransmit_request(&self.cfg, req)
        }
        fn on_retransmit_error(&mut self, err: RetransmitError) -> StreamEvent {
            self.s.on_retransmit_error(err)
        }
        fn on_retransmit_nak(&mut self) -> Option<StreamEvent> {
            self.s.on_retransmit_nak()
        }
        fn poll(&mut self, now: u64) -> Option<StreamEvent> {
            self.s.poll(&self.cfg, now)
        }
        fn next_deadline(&self) -> Option<u64> {
            self.s.next_deadline()
        }
        fn wants_write(&self, now: u64) -> bool {
            self.s.wants_write(now)
        }
        fn write_data(
            &mut self,
            now: u64,
            w: &mut PayloadWriter<'_>,
        ) -> Result<Written, StreamError> {
            self.s.write_data(&self.cfg, now, w)
        }
    }

    fn stream() -> Stream {
        TestStream::new(StreamConfig::default()).unwrap()
    }

    fn msg(i: u32) -> u32 {
        0x2090_0000 | (i & 0xFFFF)
    }

    // One UDP payload from `write_data`; empty when nothing was written.
    fn payload(s: &mut Stream, now: u64) -> Vec<u8> {
        payload_max(s, now, MaxPayload::default())
    }

    fn payload_max(s: &mut Stream, now: u64, max: MaxPayload) -> Vec<u8> {
        let mut buf = [0u8; 1400];
        let mut w = PayloadWriter::new(&mut buf, max);
        s.write_data(now, &mut w).unwrap();
        let n = w.finish().unwrap_or(0);
        buf[..n].to_vec()
    }

    // (code, Sequence Number or CSD, payload words) of each Command.
    fn cmds(p: &[u8]) -> Vec<(u8, u16, Vec<u32>)> {
        command_packets(p)
            .unwrap()
            .map(|it| match it {
                PacketItem::Command(c) => (c.header.code, c.header.csd(), c.words().collect()),
                PacketItem::Truncated(_) => panic!(),
            })
            .collect()
    }

    fn seqs(p: &[u8]) -> Vec<u16> {
        cmds(p).into_iter().map(|c| c.1).collect()
    }

    // Feeds the UMP Data Commands of a payload; gives (seq, messages, late) of each delivery.
    fn feed(s: &mut Stream, now: u64, p: &[u8]) -> Vec<(u16, Vec<u32>, bool)> {
        let mut out = Vec::new();
        for it in command_packets(p).unwrap() {
            let PacketItem::Command(c) = it else { panic!() };
            if c.header.code != 0xFF {
                continue;
            }
            let d = s.on_ump_data(now, UmpData::read(&c));
            assert!(d.gap.is_none(), "gap {:?}", d.gap);
            let m: Vec<u32> = d.messages().flat_map(|m| m.words().to_vec()).collect();
            if !m.is_empty() {
                out.push((d.seq.0, m, d.is_late()));
            }
        }
        out
    }

    fn data(seq: u16, words: &[u32]) -> Vec<u8> {
        let mut buf = [0u8; 400];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        UmpData::write(&mut w, SeqNum(seq), words).unwrap();
        let n = w.finish().unwrap();
        buf[..n].to_vec()
    }

    fn recv_seq(s: &mut Stream, now: u64, seq: u16) -> Delivery<'static> {
        let p: &'static [u8] =
            std::boxed::Box::leak(data(seq, &[msg(seq.into())]).into_boxed_slice());
        let PacketItem::Command(c) = command_packets(p).unwrap().next().unwrap() else {
            panic!()
        };
        s.on_ump_data(now, UmpData::read(&c))
    }

    // Brings the sender to next number `n` with one 1-word message for each Command.
    fn advance(s: &mut Stream, n: u16) {
        for i in 0..n {
            s.send_ump(&[msg(i.into())]).unwrap();
            payload(s, 0);
        }
    }

    fn rx_at(h: u16) -> Stream {
        let mut s = stream();
        s.s.receiver.window = ReceiveWindow::starting_at(SeqNum(h));
        s
    }

    // ---- REQ-NET-DATA-004 ----

    #[test]
    fn sequence_numbers_start_at_zero_and_wrap_after_ffff() {
        let mut s = stream();
        s.send_ump(&[msg(0)]).unwrap();
        assert_eq!(seqs(&payload(&mut s, 0)), [0x0000]);
        for i in 1..=0x1_0001u32 {
            s.send_ump(&[msg(i)]).unwrap();
            let p = payload(&mut s, 0);
            let new = *seqs(&p).last().unwrap();
            assert_eq!(u32::from(new), i & 0xFFFF);
            if i == 0x3457 {
                // A.1.4: 0x3456 then 0x3457.
                assert_eq!(seqs(&p), [0x3455, 0x3456, 0x3457]);
            }
            if i == 0x1_0000 {
                assert_eq!(seqs(&p), [0xFFFE, 0xFFFF, 0x0000]);
            }
        }
    }

    #[test]
    fn session_reset_restarts_sequence_numbers_at_zero() {
        let mut s = stream();
        advance(&mut s, 5);
        s.reset();
        s.send_ump(&[msg(9)]).unwrap();
        // No FEC repeat of the old session: the history is empty.
        assert_eq!(seqs(&payload(&mut s, 0)), [0x0000]);
    }

    #[test]
    fn zero_length_commands_take_new_numbers() {
        let mut s = stream();
        advance(&mut s, 1);
        let p = payload(&mut s, 100);
        assert_eq!(cmds(&p).last().unwrap(), &(0xFF, 0x0001, vec![]));
    }

    // ---- REQ-NET-DATA-007 ----

    #[test]
    fn many_words_are_split_at_message_boundaries() {
        let mut s = stream();
        // 31 2-word messages (62 words), then one 4-word message: 66 words.
        let mut words = Vec::new();
        for i in 0..31 {
            words.extend([0x4090_0000 | i, 0x8000_0000]);
        }
        words.extend([0xF000_0000, 1, 2, 3]);
        s.send_ump(&words).unwrap();
        let c = cmds(&payload(&mut s, 0));
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].1, c[0].2.len()), (0, 62));
        assert_eq!((c[1].1, c[1].2.len()), (1, 4));
        assert_eq!([c[0].2.clone(), c[1].2.clone()].concat(), words);
    }

    #[test]
    fn queued_messages_join_the_last_queued_command() {
        let mut s = stream();
        for i in 0..100 {
            s.send_ump(&[msg(i)]).unwrap();
        }
        let c = cmds(&payload(&mut s, 0));
        assert_eq!(
            c.iter().map(|c| (c.1, c.2.len())).collect::<Vec<_>>(),
            [(0, 64), (1, 36)]
        );
    }

    #[test]
    fn partial_messages_are_refused() {
        let mut s = stream();
        assert_eq!(s.send_ump(&[0x4090_0000]), Err(StreamError::InvalidUmpData));
        assert!(!s.wants_write(0));
    }

    #[test]
    fn full_send_queue_is_refused_and_nothing_is_queued() {
        let mut s: TestStream<64, 65, 4> = TestStream::new(StreamConfig::default()).unwrap();
        let words = [msg(1); 64];
        s.send_ump(&words).unwrap();
        assert_eq!(s.send_ump(&[msg(2)]), Err(StreamError::Full));
        assert_eq!(s.sender().history().words(), 65);
    }

    #[test]
    fn receiver_processes_several_commands_of_one_payload_in_order() {
        let mut s = stream();
        let mut p = data(0, &[msg(0)]);
        p.extend_from_slice(&data(1, &[msg(1), msg(2)])[4..]);
        p.extend_from_slice(&data(2, &[])[4..]);
        let got = feed(&mut s, 0, &p);
        assert_eq!(
            got,
            [(0, vec![msg(0)], false), (1, vec![msg(1), msg(2)], false)]
        );
        assert_eq!(s.receiver().window().highest(), SeqNum(2));
    }

    // ---- REQ-NET-DATA-008 ----

    #[test]
    fn processed_commands_are_skipped() {
        let mut s = stream();
        let p = data(0, &[msg(0)]);
        assert_eq!(feed(&mut s, 0, &p).len(), 1);
        // A duplicate UDP packet.
        assert!(feed(&mut s, 0, &p).is_empty());
        let d = recv_seq(&mut s, 0, 0);
        assert_eq!(d.class, Class::Duplicate);
        assert_eq!(d.messages().count(), 0);
        assert!(!s.wants_write(0));
    }

    // ---- REQ-NET-DATA-009 ----

    #[test]
    fn start_state_expects_zero_and_marks_the_window() {
        let w = ReceiveWindow::<64>::new();
        assert_eq!(w.highest(), SeqNum(0xFFFF));
        assert_eq!(w.classify(SeqNum(0)), Class::New { gap: 0 });
        assert_eq!(w.classify(SeqNum(0xFFFF)), Class::Duplicate);
        assert_eq!(w.classify(SeqNum(0xFFBF)), Class::Duplicate);
        assert!(w.missing().is_none());
        let mut s = stream();
        recv_seq(&mut s, 0, 5);
        s.reset();
        assert_eq!(*s.receiver().window(), ReceiveWindow::new());
    }

    // ---- REQ-NET-DATA-016 ----

    #[test]
    fn classification_vectors() {
        let mut w = ReceiveWindow::<64>::starting_at(SeqNum(0xFFFE));
        assert_eq!(w.accept(SeqNum(0x0001)).class, Class::New { gap: 2 });
        assert_eq!(
            w.missing(),
            Some(SeqRange {
                first: SeqNum(0xFFFF),
                count: 2
            })
        );
        let mut w = ReceiveWindow::<64>::starting_at(SeqNum(0x0010));
        assert_eq!(w.classify(SeqNum(0x0010)), Class::Duplicate);
        w.bits &= !(1 << 1); // 0x000E not marked
        assert_eq!(w.accept(SeqNum(0x000E)).class, Class::Late);
        assert_eq!(w.accept(SeqNum(0x000E)).class, Class::Duplicate);
        assert_eq!(w.classify(SeqNum(0x8010)), Class::Duplicate);
        assert_eq!(w.classify(SeqNum(0x800F)), Class::New { gap: 0x7FFE });
    }

    #[test]
    fn numbers_older_than_the_window_are_skipped() {
        let mut w = ReceiveWindow::<64>::starting_at(SeqNum(100));
        w.bits = 0;
        assert_eq!(w.classify(SeqNum(36)), Class::Late);
        assert_eq!(w.classify(SeqNum(35)), Class::Duplicate);
    }

    #[test]
    fn missing_numbers_that_leave_the_window_are_reported_lost() {
        let mut w = ReceiveWindow::<8>::new();
        let a = w.accept(SeqNum(3)); // 0, 1, 2 missing
        assert_eq!(a.lost, None);
        let a = w.accept(SeqNum(10)); // 0 and 1 leave the window
        assert_eq!(
            a.lost,
            Some(SeqRange {
                first: SeqNum(0),
                count: 2
            })
        );
        let a = w.accept(SeqNum(30)); // 2 and 11..=21 leave the window
        assert_eq!(
            a.lost,
            Some(SeqRange {
                first: SeqNum(2),
                count: 20
            })
        );
        assert_eq!(
            w.missing(),
            Some(SeqRange {
                first: SeqNum(22),
                count: 8
            })
        );
    }

    // ---- REQ-NET-DATA-010 ----

    #[test]
    fn gap_event_gives_first_missing_number_and_quantity() {
        let mut s = stream();
        recv_seq(&mut s, 0, 0);
        let d = recv_seq(&mut s, 0, 4);
        assert_eq!(
            d.gap,
            Some(SeqRange {
                first: SeqNum(1),
                count: 3
            })
        );
        assert!(d.delivers() && !d.is_late());
    }

    #[test]
    fn late_command_is_delivered_with_a_late_mark() {
        let mut s = stream();
        recv_seq(&mut s, 0, 1);
        let d = recv_seq(&mut s, 0, 0);
        assert!(d.is_late());
        assert_eq!(d.messages().next().unwrap().words(), [msg(0)]);
    }

    // ---- REQ-NET-DATA-011 ----

    #[test]
    fn zero_length_command_is_classified_and_gives_no_message() {
        let mut s = stream();
        let p = data(1, &[]);
        let PacketItem::Command(c) = command_packets(&p).unwrap().next().unwrap() else {
            panic!()
        };
        let d = s.on_ump_data(0, UmpData::read(&c));
        assert_eq!(d.class, Class::New { gap: 1 });
        assert_eq!(d.messages().count(), 0);
        // A late Zero Length Command closes the gap.
        let p = data(0, &[]);
        let PacketItem::Command(c) = command_packets(&p).unwrap().next().unwrap() else {
            panic!()
        };
        assert_eq!(s.on_ump_data(0, UmpData::read(&c)).class, Class::Late);
        assert!(s.receiver().window().missing().is_none());
        assert_eq!(s.next_deadline(), None);
    }

    // ---- REQ-NET-DATA-012 ----

    #[test]
    fn no_idle_command_before_the_first_data() {
        let mut s = stream();
        assert_eq!(s.next_deadline(), None);
        assert!(payload(&mut s, 10_000).is_empty());
    }

    #[test]
    fn first_zero_length_command_comes_t1_after_the_last_data() {
        let mut s = stream();
        s.send_ump(&[msg(0)]).unwrap();
        payload(&mut s, 1000);
        assert_eq!(s.next_deadline(), Some(1100));
        assert!(!s.wants_write(1099));
        assert!(payload(&mut s, 1099).is_empty());
        assert!(s.wants_write(1100));
        let p = payload(&mut s, 1100);
        assert_eq!(&p[p.len() - 4..], [0xFF, 0x00, 0x00, 0x01]);
    }

    // ---- REQ-NET-DATA-013 ----

    #[test]
    fn further_idle_commands_grow_by_g_and_stop_after_k() {
        let mut s = stream();
        s.send_ump(&[msg(0)]).unwrap();
        payload(&mut s, 0);
        let mut times = Vec::new();
        while let Some(t) = s.next_deadline() {
            assert!(!payload(&mut s, t).is_empty());
            times.push(t);
        }
        // T1 = 100: first idle, R = 2 FEC idle payloads, then K = 5 at 2, 4, 8, 16, 32 x T1.
        assert_eq!(times, [100, 200, 300, 500, 900, 1700, 3300, 6500]);
    }

    #[test]
    fn k_zero_sends_no_further_idle_command() {
        let cfg = StreamConfig {
            idle_further: 0,
            fec_repeats: 0,
            ..StreamConfig::default()
        };
        let mut s: Stream = TestStream::new(cfg).unwrap();
        s.send_ump(&[msg(0)]).unwrap();
        payload(&mut s, 0);
        assert_eq!(s.next_deadline(), Some(100));
        payload(&mut s, 100);
        assert_eq!(s.next_deadline(), None);
    }

    #[test]
    fn new_data_restarts_the_idle_sequence() {
        let mut s = stream();
        s.send_ump(&[msg(0)]).unwrap();
        payload(&mut s, 0);
        payload(&mut s, 100);
        payload(&mut s, 200);
        s.send_ump(&[msg(1)]).unwrap();
        assert_eq!(s.next_deadline(), None);
        payload(&mut s, 250);
        assert_eq!(s.next_deadline(), Some(350));
    }

    // ---- REQ-NET-DATA-017 ----

    #[test]
    fn idle_interval_is_checked() {
        assert_eq!(StreamConfig::default().idle_interval_ms, 100);
        let c = |t| StreamConfig {
            idle_interval_ms: t,
            ..StreamConfig::default()
        };
        assert!(Stream::new(c(300)).is_ok());
        assert!(Stream::new(c(1)).is_ok());
        assert_eq!(Stream::new(c(301)).err(), Some(StreamError::IdleInterval));
        assert_eq!(Stream::new(c(0)).err(), Some(StreamError::IdleInterval));
        let g = StreamConfig {
            idle_growth: 0,
            ..StreamConfig::default()
        };
        assert_eq!(Stream::new(g).err(), Some(StreamError::IdleGrowth));
    }

    // ---- REQ-NET-FEC-001 ----

    #[test]
    fn fec_repeats_are_the_last_r_commands() {
        let cfg = StreamConfig {
            fec_repeats: 3,
            ..StreamConfig::default()
        };
        let mut s: Stream = TestStream::new(cfg).unwrap();
        advance(&mut s, 5);
        s.send_ump(&[msg(5)]).unwrap();
        assert_eq!(seqs(&payload(&mut s, 0)), [2, 3, 4, 5]);
    }

    #[test]
    fn r_zero_turns_fec_off() {
        let cfg = StreamConfig {
            fec_repeats: 0,
            ..StreamConfig::default()
        };
        let mut s: Stream = TestStream::new(cfg).unwrap();
        advance(&mut s, 3);
        s.send_ump(&[msg(3)]).unwrap();
        assert_eq!(seqs(&payload(&mut s, 0)), [3]);
    }

    #[test]
    fn oldest_repeats_leave_first_when_the_payload_is_full() {
        let mut s = stream();
        s.send_ump(&[msg(0); 30]).unwrap();
        payload(&mut s, 0);
        s.send_ump(&[msg(1); 30]).unwrap();
        payload(&mut s, 0);
        s.send_ump(&[msg(2); 30]).unwrap();
        // 4 + 3 x 124 = 376 bytes for all; 300 bytes hold 2 Commands.
        let p = payload_max(&mut s, 0, MaxPayload::new(300).unwrap());
        assert_eq!(seqs(&p), [1, 2]);
    }

    #[test]
    fn fec_repeats_hold_zero_length_commands() {
        let mut s = stream();
        advance(&mut s, 1);
        payload(&mut s, 100); // zero 0x0001
        s.send_ump(&[msg(2)]).unwrap();
        let c = cmds(&payload(&mut s, 150));
        assert_eq!(c.iter().map(|c| c.1).collect::<Vec<_>>(), [0, 1, 2]);
        assert!(c[1].2.is_empty());
    }

    // ---- REQ-NET-FEC-002 ----

    #[test]
    fn fec_payload_order_matches_figure_9() {
        let mut s = stream();
        advance(&mut s, 0x23);
        s.send_ump(&[0x2090_3C64]).unwrap();
        let p = payload(&mut s, 0);
        let mut want = Vec::from(*b"MIDI");
        want.extend([0xFF, 0x01, 0x00, 0x21]);
        want.extend(msg(0x21).to_be_bytes());
        want.extend([0xFF, 0x01, 0x00, 0x22]);
        want.extend(msg(0x22).to_be_bytes());
        want.extend([0xFF, 0x01, 0x00, 0x23]);
        want.extend(0x2090_3C64u32.to_be_bytes());
        assert_eq!(p, want);
    }

    // ---- REQ-NET-FEC-003, REQ-NET-VEC-007 ----

    #[test]
    fn idle_payloads_match_figure_19() {
        let mut s = stream();
        advance(&mut s, 0x21);
        s.send_ump(&[msg(0x21)]).unwrap();
        payload(&mut s, 0);
        s.send_ump(&[msg(0x22)]).unwrap();
        payload(&mut s, 0);
        s.send_ump(&[msg(0x23)]).unwrap();
        let p2 = payload(&mut s, 0);
        assert_eq!(seqs(&p2), [0x21, 0x22, 0x23]);
        let i1 = payload(&mut s, 100);
        let i2 = payload(&mut s, 200);
        let i3 = payload(&mut s, 300);
        assert_eq!(seqs(&i1), [0x22, 0x23, 0x24]);
        assert_eq!(seqs(&i2), [0x23, 0x24, 0x25]);
        assert_eq!(seqs(&i3), [0x24, 0x25, 0x26]);
        assert_eq!(
            &i3[4..],
            [0xFF, 0, 0, 0x24, 0xFF, 0, 0, 0x25, 0xFF, 0, 0, 0x26]
        );
        assert_eq!(&i1[i1.len() - 4..], [0xFF, 0x00, 0x00, 0x24]);
        // After the FEC idle payloads, the next one waits 2 x T1 (REQ-NET-DATA-013).
        assert_eq!(s.next_deadline(), Some(500));
        // Receiver with H = 0x0020.
        let mut r = rx_at(0x20);
        let mut got = Vec::new();
        for p in [&p2, &i1, &i2, &i3] {
            got.extend(feed(&mut r, 0, p));
        }
        assert_eq!(
            got,
            [
                (0x21, vec![msg(0x21)], false),
                (0x22, vec![msg(0x22)], false),
                (0x23, vec![msg(0x23)], false),
            ]
        );
    }

    // ---- REQ-NET-VEC-005 ----

    #[test]
    fn figure_17_sender_and_receiver() {
        let mut s = stream();
        let mut ps = Vec::new();
        for i in 0..5 {
            s.send_ump(&[msg(i)]).unwrap();
            ps.push(payload(&mut s, 0));
        }
        let want: [&[u16]; 5] = [&[0], &[0, 1], &[0, 1, 2], &[1, 2, 3], &[2, 3, 4]];
        for (p, w) in ps.iter().zip(want) {
            assert_eq!(seqs(p), w);
        }
        let mut r = stream();
        let mut got = Vec::new();
        for i in [0, 3, 4] {
            got.extend(feed(&mut r, 0, &ps[i]));
        }
        let want: Vec<_> = (0..5).map(|i| (i as u16, vec![msg(i)], false)).collect();
        assert_eq!(got, want);
        assert_eq!(r.poll(1000), None);
        assert!(!r.wants_write(1000));
    }

    // ---- REQ-NET-VEC-006 ----

    fn figure_18() -> [Vec<u8>; 4] {
        let d = |seq: u16| data(seq, &[msg(seq.into())]);
        let z = |seq: u16| data(seq, &[]);
        let join = |parts: &[Vec<u8>]| {
            let mut p = Vec::from(*b"MIDI");
            for x in parts {
                p.extend_from_slice(&x[4..]);
            }
            p
        };
        [
            join(&[d(0x21), d(0x22), d(0x23)]),
            join(&[d(0x22), d(0x23), z(0x24)]),
            join(&[d(0x23), z(0x24)]),
            join(&[z(0x24)]),
        ]
    }

    #[test]
    fn figure_18_receiver() {
        let want: Vec<_> = (0x21..=0x23)
            .map(|i| (i as u16, vec![msg(i)], false))
            .collect();
        let f = figure_18();
        for set in [&[0, 1, 2, 3][..], &[0, 2], &[0, 3]] {
            let mut r = rx_at(0x20);
            let mut got = Vec::new();
            for &i in set {
                got.extend(feed(&mut r, 0, &f[i]));
            }
            assert_eq!(got, want);
            assert_eq!(r.receiver().window().highest(), SeqNum(0x24));
            assert!(r.receiver().window().missing().is_none());
        }
    }

    // ---- REQ-NET-GEN-003 ----

    #[test]
    fn lost_duplicate_and_reordered_payloads_give_each_command_once() {
        let mut tx = stream();
        let mut ps = Vec::new();
        for i in 0..8 {
            tx.send_ump(&[msg(i)]).unwrap();
            ps.push(payload(&mut tx, 0));
        }
        let mut rx = stream();
        let mut got = Vec::new();
        // Payload 2 lost, 4 twice, 6 before 5.
        for i in [0, 1, 3, 4, 4, 6, 5, 7] {
            got.extend(feed(&mut rx, 0, &ps[i]));
        }
        let mut seen: Vec<u16> = got.iter().map(|g| g.0).collect();
        seen.sort();
        assert_eq!(seen, (0..8).collect::<Vec<u16>>());
    }

    #[test]
    fn retransmit_recovers_a_burst_loss() {
        let mut tx = stream();
        let mut rx = stream();
        let mut got = Vec::new();
        for i in 0..10 {
            tx.send_ump(&[msg(i)]).unwrap();
            let p = payload(&mut tx, 0);
            if !(3..=6).contains(&i) {
                for it in command_packets(&p).unwrap() {
                    let PacketItem::Command(c) = it else { panic!() };
                    let d = rx.on_ump_data(0, UmpData::read(&c));
                    if d.delivers() {
                        got.push(d.seq.0);
                    }
                }
            }
        }
        // 0x0003 and 0x0004 are missing (5 and 6 came as FEC repeats of 7 and 8).
        assert_eq!(rx.poll(10), None);
        let req = payload(&mut rx, 10);
        let c = cmds(&req);
        assert_eq!(c, [(0x80, 3, vec![6 << 16])]);
        let PacketItem::Command(raw) = command_packets(&req).unwrap().next().unwrap() else {
            panic!()
        };
        let rq = RetransmitRequest::read(&raw).unwrap();
        assert_eq!(tx.on_retransmit_request(rq), RequestOutcome::Queued);
        let p = payload(&mut tx, 11);
        for it in command_packets(&p).unwrap() {
            let PacketItem::Command(c) = it else { panic!() };
            let d = rx.on_ump_data(11, UmpData::read(&c));
            if d.delivers() {
                assert!(d.is_late());
                got.push(d.seq.0);
            }
        }
        got.sort();
        assert_eq!(got, (0..10).collect::<Vec<u16>>());
        assert_eq!(rx.next_deadline(), None);
    }

    // ---- REQ-NET-RTX-003 ----

    #[test]
    fn send_history_removes_the_oldest_commands_first() {
        let mut h = SendHistory::<65>::new();
        h.push(&[1; 30]);
        h.push(&[2; 30]);
        assert_eq!((h.len(), h.words()), (2, 62));
        h.push(&[3; 10]);
        assert_eq!(h.oldest(), SeqNum(1));
        assert!(!h.contains(SeqNum(0)) && h.contains(SeqNum(2)));
        h.push(&[4; 64]);
        assert_eq!(
            (h.len(), h.oldest(), h.next_seq()),
            (1, SeqNum(3), SeqNum(4))
        );
        h.clear();
        assert_eq!((h.len(), h.next_seq()), (0, SeqNum(0)));
    }

    #[test]
    fn send_history_keeps_send_order_across_the_wrap() {
        let mut s = stream();
        s.s.sender.history.oldest = SeqNum(0xFFFD);
        for i in 0..5 {
            s.send_ump(&[msg(i)]).unwrap();
            payload(&mut s, 0);
        }
        let rq = RetransmitRequest {
            seq: SeqNum(0xFFFE),
            count: 0,
        };
        assert_eq!(s.on_retransmit_request(rq), RequestOutcome::Queued);
        let c = cmds(&payload(&mut s, 0));
        assert_eq!(
            c.iter().map(|c| (c.1, c.2.clone())).collect::<Vec<_>>(),
            [
                (0xFFFE, vec![msg(1)]),
                (0xFFFF, vec![msg(2)]),
                (0, vec![msg(3)]),
                (1, vec![msg(4)])
            ]
        );
    }

    #[test]
    fn reset_empties_the_send_history() {
        let mut s = stream();
        advance(&mut s, 3);
        s.reset();
        assert!(s.sender().history().is_empty());
        assert_eq!(s.sender().next_seq(), SeqNum(0));
    }

    // ---- REQ-NET-RTX-005 ----

    fn resend(s: &mut Stream, seq: u16, count: u16) -> Vec<(u16, Vec<u32>)> {
        let rq = RetransmitRequest {
            seq: SeqNum(seq),
            count,
        };
        assert_eq!(s.on_retransmit_request(rq), RequestOutcome::Queued);
        cmds(&payload(s, 0))
            .into_iter()
            .map(|c| (c.1, c.2))
            .collect()
    }

    #[test]
    fn retransmit_resends_n_commands_from_the_first_number() {
        let mut s = stream();
        advance(&mut s, 10);
        let got = resend(&mut s, 4, 3);
        assert_eq!(
            got,
            [(4, vec![msg(4)]), (5, vec![msg(5)]), (6, vec![msg(6)])]
        );
    }

    #[test]
    fn retransmit_count_zero_or_too_large_resends_up_to_the_newest() {
        let mut s = stream();
        advance(&mut s, 10);
        let got: Vec<u16> = resend(&mut s, 7, 0).into_iter().map(|c| c.0).collect();
        assert_eq!(got, [7, 8, 9]);
        let got: Vec<u16> = resend(&mut s, 8, 50).into_iter().map(|c| c.0).collect();
        assert_eq!(got, [8, 9]);
    }

    // ---- REQ-NET-RTX-006 ----

    #[test]
    fn retransmit_error_for_a_number_not_held() {
        let mut s: TestStream<64, 65, 4> = TestStream::new(StreamConfig::default()).unwrap();
        // Empty history: the field is the next number.
        let e = |seq| {
            RequestOutcome::Error(RetransmitError {
                reason: RetransmitErrorReason::NOT_IN_BUFFER,
                seq: SeqNum(seq),
            })
        };
        let rq = |seq| RetransmitRequest {
            seq: SeqNum(seq),
            count: 1,
        };
        assert_eq!(s.on_retransmit_request(rq(0)), e(0));
        for i in 0..40 {
            s.send_ump(&[msg(i)]).unwrap();
            let mut buf = [0u8; 1400];
            let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
            s.write_data(0, &mut w).unwrap();
        }
        // 65 words hold 32 Commands of 2 words: 8 to 39.
        assert_eq!(s.on_retransmit_request(rq(3)), e(8));
        // Newer than the last written Command.
        assert_eq!(s.on_retransmit_request(rq(40)), e(8));
        assert!(s.sender().queue().is_empty());
        // The Error writes as Table 31.
        let mut buf = [0u8; 16];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        let RequestOutcome::Error(err) = s.on_retransmit_request(rq(3)) else {
            panic!()
        };
        err.write(&mut w).unwrap();
        assert_eq!(
            &buf[4..12],
            [0x81, 0x01, 0x01, 0x00, 0x00, 0x08, 0x00, 0x00]
        );
    }

    #[test]
    fn retransmit_not_supported_when_the_option_is_off() {
        let cfg = StreamConfig {
            retransmit: false,
            ..StreamConfig::default()
        };
        let mut s: Stream = TestStream::new(cfg).unwrap();
        advance(&mut s, 2);
        let rq = RetransmitRequest {
            seq: SeqNum(0),
            count: 1,
        };
        assert_eq!(s.on_retransmit_request(rq), RequestOutcome::NotSupported);
    }

    // ---- REQ-NET-RTX-008 ----

    #[test]
    fn full_queue_and_pending_number_ignore_the_request() {
        let mut s = stream();
        advance(&mut s, 10);
        let rq = |seq| RetransmitRequest {
            seq: SeqNum(seq),
            count: 1,
        };
        for i in 0..4 {
            assert_eq!(s.on_retransmit_request(rq(i)), RequestOutcome::Queued);
        }
        assert_eq!(
            s.on_retransmit_request(rq(1)),
            RequestOutcome::Ignored(Ignored::Pending)
        );
        assert_eq!(
            s.on_retransmit_request(rq(5)),
            RequestOutcome::Ignored(Ignored::Full)
        );
        assert_eq!(seqs(&payload(&mut s, 0)), [0, 1, 2, 3]);
        assert_eq!(s.on_retransmit_request(rq(5)), RequestOutcome::Queued);
    }

    // ---- REQ-NET-RTX-009 ----

    #[test]
    fn retransmit_request_after_delay_d_with_exact_count() {
        let mut s = stream();
        recv_seq(&mut s, 0, 0);
        recv_seq(&mut s, 100, 3);
        recv_seq(&mut s, 101, 5);
        assert_eq!(s.next_deadline(), Some(110));
        assert_eq!(s.poll(109), None);
        assert!(!s.wants_write(109));
        assert_eq!(s.poll(110), None);
        assert!(s.wants_write(110));
        // Oldest missing 0x0001; count from it up to H = 5: 4.
        assert_eq!(cmds(&payload(&mut s, 110)), [(0x80, 1, vec![4 << 16])]);
        assert!(!s.wants_write(110));
    }

    #[test]
    fn no_request_when_the_gap_closes_within_d() {
        let mut s = stream();
        recv_seq(&mut s, 0, 1);
        recv_seq(&mut s, 5, 0);
        assert_eq!(s.next_deadline(), None);
        assert_eq!(s.poll(100), None);
        assert!(payload(&mut s, 100).is_empty());
    }

    #[test]
    fn no_request_when_the_option_is_off() {
        let cfg = StreamConfig {
            retransmit: false,
            ..StreamConfig::default()
        };
        let mut s: Stream = TestStream::new(cfg).unwrap();
        recv_seq(&mut s, 0, 2);
        assert_eq!(s.next_deadline(), None);
        assert!(payload(&mut s, 100).is_empty());
    }

    // ---- REQ-NET-RTX-010 ----

    #[test]
    fn requests_repeat_with_doubled_delay_then_data_loss() {
        let mut s = stream();
        recv_seq(&mut s, 0, 2);
        let mut times = Vec::new();
        let mut loss = None;
        while let Some(t) = s.next_deadline() {
            if let Some(e) = s.poll(t) {
                loss = Some((t, e));
            }
            if !payload(&mut s, t).is_empty() {
                times.push(t);
            }
        }
        assert_eq!(times, [10, 30, 70]);
        assert_eq!(
            loss,
            Some((
                150,
                StreamEvent::DataLoss {
                    range: SeqRange {
                        first: SeqNum(0),
                        count: 2
                    },
                    error: None
                }
            ))
        );
        // Marked as processed: a late copy is skipped.
        assert_eq!(recv_seq(&mut s, 200, 1).class, Class::Duplicate);
    }

    // ---- REQ-NET-RTX-011 ----

    #[test]
    fn nak_stops_requests_and_gives_data_loss() {
        let mut s = stream();
        recv_seq(&mut s, 0, 2);
        s.poll(10);
        payload(&mut s, 10);
        let e = s.on_retransmit_nak();
        assert_eq!(
            e,
            Some(StreamEvent::DataLoss {
                range: SeqRange {
                    first: SeqNum(0),
                    count: 2
                },
                error: None
            })
        );
        assert_eq!(s.next_deadline(), None);
        recv_seq(&mut s, 20, 5);
        assert_eq!(s.next_deadline(), None);
        assert!(!s.requests_enabled());
        s.reset();
        assert!(s.requests_enabled());
    }

    // ---- REQ-NET-RTX-013 ----

    #[test]
    fn retransmit_error_stops_requests_and_gives_data_loss_with_the_error() {
        let mut s = stream();
        recv_seq(&mut s, 0, 3);
        s.poll(10);
        payload(&mut s, 10);
        let err = RetransmitError {
            reason: RetransmitErrorReason::NOT_IN_BUFFER,
            seq: SeqNum(2),
        };
        assert_eq!(
            s.on_retransmit_error(err),
            StreamEvent::DataLoss {
                range: SeqRange {
                    first: SeqNum(0),
                    count: 3
                },
                error: Some(err)
            }
        );
        assert_eq!(s.next_deadline(), None);
        // With no open request, the error is given as it is.
        assert_eq!(
            s.on_retransmit_error(err),
            StreamEvent::RetransmitError(err)
        );
    }

    // ---- REQ-NET-RTX-015 ----

    #[test]
    fn resent_commands_come_before_new_data_and_span_payloads() {
        let mut s = stream();
        for i in 0..10 {
            s.send_ump(&[msg(i); 60]).unwrap();
            payload(&mut s, 0);
        }
        let rq = RetransmitRequest {
            seq: SeqNum(2),
            count: 6,
        };
        assert_eq!(s.on_retransmit_request(rq), RequestOutcome::Queued);
        s.send_ump(&[msg(99)]).unwrap();
        // 244 bytes for each resent Command: 5 fit in 1400 bytes.
        let p1 = payload(&mut s, 0);
        assert_eq!(seqs(&p1), [2, 3, 4, 5, 6]);
        let p2 = payload(&mut s, 0);
        assert_eq!(seqs(&p2), [7, 8, 9, 10]);
        assert!(!s.wants_write(0));
    }

    #[test]
    fn write_data_reports_more_when_the_payload_is_full() {
        let mut s = stream();
        s.send_ump(&[msg(0); 64]).unwrap();
        s.send_ump(&[msg(1); 64]).unwrap();
        let mut buf = [0u8; 300];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        let r = s.write_data(0, &mut w).unwrap();
        assert_eq!(
            r,
            Written {
                commands: 1,
                more: true
            }
        );
        let mut tiny = [0u8; 8];
        let mut w = PayloadWriter::new(&mut tiny, MaxPayload::default());
        assert_eq!(
            s.write_data(0, &mut w),
            Err(StreamError::Packet(PacketError::TooLarge))
        );
    }

    // ---- review fixes ----

    #[test]
    fn join_after_a_full_history_keeps_the_sequence_numbers() {
        // S1: a full history, then two send_ump calls before one write: the join makes room
        // first; correct numbers, no panic, round trip to a receiver.
        let mut tx = stream();
        let mut rx = stream();
        let mut got = Vec::new();
        for i in 0..300u32 {
            tx.send_ump(&[msg(i)]).unwrap();
            let p = payload(&mut tx, 0);
            got.extend(feed(&mut rx, 0, &p).into_iter().flat_map(|d| d.1));
        }
        assert_eq!(tx.sender().history().words(), 512);
        tx.send_ump(&[msg(300)]).unwrap();
        tx.send_ump(&[msg(301)]).unwrap();
        assert!(tx.sender().history().words() <= 512);
        let p = payload(&mut tx, 0);
        let c = cmds(&p);
        assert_eq!(
            c.last().map(|c| (c.1, c.2.clone())),
            Some((300, vec![msg(300), msg(301)]))
        );
        got.extend(feed(&mut rx, 0, &p).into_iter().flat_map(|d| d.1));
        for i in 302..400u32 {
            tx.send_ump(&[msg(i)]).unwrap();
            tx.send_ump(&[msg(i + 1000)]).unwrap();
            let p = payload(&mut tx, 0);
            got.extend(feed(&mut rx, 0, &p).into_iter().flat_map(|d| d.1));
        }
        let mut want: Vec<u32> = (0..302).map(msg).collect();
        for i in 302..400u32 {
            want.extend([msg(i), msg(i + 1000)]);
        }
        assert_eq!(got, want);
        // The history still serves the newest numbers.
        assert_eq!(resend(&mut tx, 398, 1), [(398, vec![msg(399), msg(1399)])]);
    }

    #[test]
    fn new_gap_after_a_request_gets_the_full_request_schedule() {
        // S2: a gap that appears after a request starts the schedule again, so that each
        // missing number gets M requests before the data loss.
        let mut s = stream();
        recv_seq(&mut s, 0, 0);
        recv_seq(&mut s, 0, 2);
        s.poll(10);
        assert_eq!(cmds(&payload(&mut s, 10)), [(0x80, 1, vec![1 << 16])]);
        recv_seq(&mut s, 15, 4);
        assert_eq!(s.next_deadline(), Some(25));
        let mut times = Vec::new();
        let mut loss = None;
        while let Some(t) = s.next_deadline() {
            if let Some(e) = s.poll(t) {
                loss = Some((t, e));
            }
            let p = payload(&mut s, t);
            if !p.is_empty() {
                // Each request holds the number 3.
                assert_eq!(cmds(&p), [(0x80, 1, vec![3 << 16])]);
                times.push(t);
            }
        }
        assert_eq!(times, [25, 45, 85]);
        assert_eq!(
            loss,
            Some((
                165,
                StreamEvent::DataLoss {
                    range: SeqRange {
                        first: SeqNum(1),
                        count: 3
                    },
                    error: None
                }
            ))
        );
    }

    #[test]
    fn retransmit_error_marks_only_the_numbers_of_its_request() {
        // S2: the error answers the request for 1; the number 3 that went missing after
        // it is still requested.
        let mut s = stream();
        recv_seq(&mut s, 0, 0);
        recv_seq(&mut s, 0, 2);
        s.poll(10);
        payload(&mut s, 10);
        recv_seq(&mut s, 15, 4);
        let err = RetransmitError {
            reason: RetransmitErrorReason::NOT_IN_BUFFER,
            seq: SeqNum(0),
        };
        assert_eq!(
            s.on_retransmit_error(err),
            StreamEvent::DataLoss {
                range: SeqRange {
                    first: SeqNum(1),
                    count: 1
                },
                error: Some(err)
            }
        );
        assert_eq!(recv_seq(&mut s, 16, 1).class, Class::Duplicate);
        assert_eq!(s.next_deadline(), Some(25));
        assert_eq!(s.poll(25), None);
        assert_eq!(cmds(&payload(&mut s, 25)), [(0x80, 3, vec![1 << 16])]);
        assert!(recv_seq(&mut s, 26, 3).is_late());
        assert_eq!(s.next_deadline(), None);
    }

    #[test]
    fn command_size_is_checked_and_splits_the_commands() {
        // S3: 1 to 64 words; send_ump uses it; a message longer than it is refused.
        for bad in [0usize, 65] {
            let cfg = StreamConfig {
                max_command_words: bad,
                ..StreamConfig::DEFAULT
            };
            assert_eq!(cfg.check(), Err(StreamError::CommandWords));
            assert!(Stream::new(cfg).is_err());
        }
        let cfg = StreamConfig {
            max_command_words: 4,
            ..StreamConfig::DEFAULT
        };
        let mut s: Stream = TestStream::new(cfg).unwrap();
        let words: Vec<u32> = (0..10).map(msg).collect();
        s.send_ump(&words[..3]).unwrap();
        s.send_ump(&words[3..]).unwrap();
        let c = cmds(&payload(&mut s, 0));
        assert_eq!(
            c.iter().map(|c| (c.1, c.2.len())).collect::<Vec<_>>(),
            [(0, 4), (1, 4), (2, 2)]
        );
        let small = StreamConfig {
            max_command_words: 2,
            ..StreamConfig::DEFAULT
        };
        let mut s: Stream = TestStream::new(small).unwrap();
        assert_eq!(
            s.send_ump(&[0xF000_0000, 1, 2, 3]),
            Err(StreamError::Packet(PacketError::TooLarge))
        );
        assert!(!s.wants_write(0));
    }

    #[test]
    fn too_large_retransmit_is_dropped_and_does_not_block_the_stream() {
        // S3: a resent Command that does not fit in an empty payload drops the request.
        let mut s = stream();
        s.send_ump(&[msg(0); 64]).unwrap();
        payload(&mut s, 0);
        let rq = RetransmitRequest {
            seq: SeqNum(0),
            count: 1,
        };
        assert_eq!(s.on_retransmit_request(rq), RequestOutcome::Queued);
        let mut buf = [0u8; 100];
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        assert_eq!(s.write_data(0, &mut w), Ok(Written::default()));
        assert!(s.sender().queue().is_empty());
        s.send_ump(&[msg(1)]).unwrap();
        let mut w = PayloadWriter::new(&mut buf, MaxPayload::default());
        let r = s.write_data(0, &mut w).unwrap();
        assert_eq!(r.commands, 1);
    }

    #[test]
    fn send_history_accepts_the_serial_half_range() {
        // S4: C up to 32768 words builds (a larger C does not compile).
        let h = std::boxed::Box::new(SendHistory::<32768>::new());
        assert!(h.is_empty());
        assert_eq!(h.next_seq(), SeqNum(0));
    }

    // ---- proptest ----

    #[test]
    fn prop_receiver_never_panics_and_delivers_each_number_once() {
        proptest!(|(start in any::<u16>(), offs in proptest::collection::vec(0u16..300, 0..200))| {
            let mut w = ReceiveWindow::<64>::starting_at(SeqNum(start));
            let mut seen = std::collections::BTreeSet::new();
            for o in offs {
                let s = SeqNum(start.wrapping_add(1).wrapping_add(o));
                match w.accept(s).class {
                    Class::Duplicate => {}
                    _ => {
                        prop_assert!(seen.insert(s.0));
                        prop_assert!(w.is_processed(s));
                    }
                }
            }
        });
    }

    #[test]
    fn prop_stream_never_panics_on_any_sequence_number() {
        proptest!(|(ops in proptest::collection::vec((any::<u16>(), any::<u16>(), 0u8..4), 0..100))| {
            let mut s = stream();
            advance(&mut s, 3);
            for (i, (a, b, k)) in ops.into_iter().enumerate() {
                let now = i as u64 * 7;
                match k {
                    0 => { recv_seq(&mut s, now, a); }
                    1 => { s.on_retransmit_request(RetransmitRequest { seq: SeqNum(a), count: b }); }
                    2 => { s.on_retransmit_error(RetransmitError { reason: RetransmitErrorReason(b as u8), seq: SeqNum(a) }); }
                    _ => { s.poll(now); }
                }
                payload(&mut s, now);
            }
        });
    }

    #[test]
    fn prop_send_receive_round_trip_with_fec() {
        // With R = 2, any loss pattern with no 3 lost payloads in a row loses nothing.
        proptest!(|(lost in proptest::collection::vec(any::<bool>(), 1..60))| {
            let mut tx = stream();
            let mut rx = stream();
            let mut got = Vec::new();
            let mut run = 0;
            for (i, l) in lost.iter().enumerate() {
                tx.send_ump(&[msg(i as u32)]).unwrap();
                let p = payload(&mut tx, 0);
                // The last payload always arrives.
                run = if *l && run < 2 && i + 1 < lost.len() { run + 1 } else { 0 };
                if run == 0 {
                    for it in command_packets(&p).unwrap() {
                        let PacketItem::Command(c) = it else { panic!() };
                        let d = rx.on_ump_data(0, UmpData::read(&c));
                        got.extend(d.messages().map(|m| m.words()[0]));
                    }
                }
            }
            let want: Vec<u32> = (0..lost.len() as u32).map(msg).collect();
            prop_assert_eq!(got, want);
        });
    }
}
