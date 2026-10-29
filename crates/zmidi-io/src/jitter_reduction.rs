//! Jitter Reduction Timestamps (M2-104-UM 7.2.2): a sans-IO JR sender and JR receiver.
//!
//! - JR Clock (Message Type 0x0, Status 0x1, 7.2.2.1, Figure 29) gives the Sender's current
//!   time. The Sender sends it at least once every 250 ms, as close as possible to the time in
//!   its Sender Clock Time field. The time increases monotonically except where it wraps.
//! - JR Timestamp (Message Type 0x0, Status 0x2, 7.2.2.2, Figure 31) gives the time of the
//!   following message(s). Messages within one tick of each other can share one JR Timestamp.
//! - Both times are 16-bit values in ticks of 1/31250 s (32 µs). They wrap every 2.09712 s.
//! - The Receiver measures jitter from JR Clock messages and renders each message at its
//!   timestamp plus a delay (7.2.2.1), in receive order (7.2.2.2). Before the first JR Clock it
//!   renders messages as soon as possible (7.2.2.3).
//!
//! The caller gives the local time as microseconds from a monotonic clock. The caller gets
//! "send JR" and "expect JR" from [`crate::Endpoint::sends_jr_timestamps`] and
//! [`crate::Endpoint::expects_jr_timestamps`]. No alloc (DEC-002), no I/O (DEC-003).

use zmidi_ump::{JrClock, JrTimestamp, UtilityMessage};

/// Microseconds in one JR tick (1/31250 s, 7.2.2.1).
const TICK_US: u64 = 32;

/// Longest JR Clock period that 7.2.2.1 permits ("at least once every 250 milliseconds").
const MAX_CLOCK_PERIOD_US: u32 = 250_000;

/// Error of the JR sender (thiserror 2, no_std).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JrError {
    /// The JR Clock period is 0 or more than 250 000 µs (7.2.2.1).
    #[error("JR Clock period must be 1 to 250000 microseconds")]
    ClockPeriod,
    /// A render time earlier than that of the last timestamped message (7.2.2.2).
    #[error("JR-timestamped messages must be sent in render order")]
    OutOfOrder,
}

/// The JR sender state (7.2.2.1, 7.2.2.2). The caller uses it only while JR is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JrSender {
    /// JR Clock period in µs, 1 to 250 000.
    period_us: u64,
    /// Local time of the last JR Clock.
    last_clock_us: Option<u64>,
    /// Tick of the last JR Timestamp.
    last_timestamp_tick: Option<u64>,
    /// Render time of the last timestamped message.
    last_render_us: Option<u64>,
}

impl JrSender {
    /// Creates a sender with a JR Clock period in µs. Returns [`JrError::ClockPeriod`] for 0 or
    /// more than 250 000 (7.2.2.1).
    pub fn new(period_us: u32) -> Result<Self, JrError> {
        if period_us == 0 || period_us > MAX_CLOCK_PERIOD_US {
            return Err(JrError::ClockPeriod);
        }
        Ok(Self {
            period_us: u64::from(period_us),
            last_clock_us: None,
            last_timestamp_tick: None,
            last_render_us: None,
        })
    }

    /// Gives the JR Clock word when the period has elapsed since the last JR Clock, or when no
    /// JR Clock was sent yet. The Sender Clock Time is `now_us` in ticks. Send it at once
    /// (7.2.2.1). A local clock that goes back gives no JR Clock, so the time stays monotonic.
    pub fn poll_clock(&mut self, now_us: u64) -> Option<u32> {
        if let Some(last) = self.last_clock_us {
            if now_us < last.saturating_add(self.period_us) {
                return None;
            }
        }
        self.last_clock_us = Some(now_us);
        Some(word(JrClock::new().with_time(sender_time(now_us))))
    }

    /// Gives the JR Timestamp word to send before the next outgoing message, which renders at
    /// `render_us`. Gives `None` when `render_us` is in the tick of the last JR Timestamp: the
    /// message shares it (7.2.2.2). Returns [`JrError::OutOfOrder`] for a render time earlier
    /// than that of the last timestamped message (7.2.2.2: send in render order).
    pub fn timestamp(&mut self, render_us: u64) -> Result<Option<u32>, JrError> {
        if self.last_render_us.is_some_and(|last| render_us < last) {
            return Err(JrError::OutOfOrder);
        }
        self.last_render_us = Some(render_us);
        let tick = render_us / TICK_US;
        if self.last_timestamp_tick == Some(tick) {
            return Ok(None);
        }
        self.last_timestamp_tick = Some(tick);
        Ok(Some(word(
            JrTimestamp::new().with_timestamp(sender_time(render_us)),
        )))
    }
}

/// What the caller does with one received packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// A JR message: nothing to render.
    Consumed,
    /// Render as soon as possible.
    RenderNow,
    /// Render at this local time in µs.
    RenderAt(u64),
    /// JR was not expected. The JR Timestamp is ignored. Request a configuration without JR
    /// ([`crate::Endpoint::request_configuration`], 7.2.2.3).
    UnexpectedJrTimestamp,
}

/// The JR receiver state (7.2.2.1 to 7.2.2.3), with a window of the last `W` JR Clock offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JrReceiver<const W: usize> {
    /// Offsets (local receive µs minus unwrapped sender µs) of the last `W` JR Clocks.
    offsets: [i64; W],
    /// Number of valid entries in `offsets`.
    len: usize,
    /// Next entry of `offsets` to write.
    next: usize,
    /// Offset of the last JR Clock, used when `W` is 0.
    last_offset: i64,
    /// Unwrapped sender tick of the last JR Clock.
    last_clock_tick: Option<i64>,
    /// The JR Timestamp that applies to the next message.
    timestamp: Option<u16>,
    /// Safety margin in µs added to the measured jitter.
    margin_us: u64,
    /// The last render time given.
    last_render_us: Option<u64>,
}

impl<const W: usize> JrReceiver<W> {
    /// Creates a receiver with a safety margin in µs that is added to the measured jitter.
    pub fn new(margin_us: u64) -> Self {
        Self {
            offsets: [0; W],
            len: 0,
            next: 0,
            last_offset: 0,
            last_clock_tick: None,
            timestamp: None,
            margin_us,
            last_render_us: None,
        }
    }

    /// Takes the words of one received packet, whether JR is expected, and the local time.
    /// Gives what to do with the packet. Never panics.
    pub fn receive(&mut self, words: &[u32], jr_expected: bool, now_us: u64) -> Disposition {
        match UtilityMessage::parse(words) {
            Some(UtilityMessage::JrClock(clock)) => {
                self.clock(clock.time(), now_us);
                Disposition::Consumed
            }
            Some(UtilityMessage::JrTimestamp(ts)) => {
                if jr_expected {
                    self.timestamp = Some(ts.timestamp());
                    Disposition::Consumed
                } else {
                    // 7.2.2.3: render as soon as possible and switch the Sender off JR.
                    self.timestamp = None;
                    Disposition::UnexpectedJrTimestamp
                }
            }
            _ => self.render(jr_expected, now_us),
        }
    }

    /// The delay in µs: the jitter (largest minus smallest offset in the window) plus the
    /// margin (7.2.2.1). The intervals between JR Clocks are not used.
    pub fn delay(&self) -> u64 {
        let window = &self.offsets[..self.len];
        let jitter = match (window.iter().max(), window.iter().min()) {
            (Some(max), Some(min)) => max.abs_diff(*min),
            _ => 0,
        };
        jitter.saturating_add(self.margin_us)
    }

    /// Converts a sender time to local µs (7.2.2.2), with the smallest offset in the window,
    /// the JR Clock with the least transport delay. Gives `None` before the first JR Clock.
    pub fn to_local(&self, sender: u16) -> Option<u64> {
        let tick = unwrap(sender, self.last_clock_tick?);
        let offset = self.offsets[..self.len]
            .iter()
            .min()
            .copied()
            .unwrap_or(self.last_offset);
        let local = tick.saturating_mul(TICK_US as i64).saturating_add(offset);
        Some(local.max(0) as u64)
    }

    /// Records one JR Clock received at `now_us`.
    fn clock(&mut self, time: u16, now_us: u64) {
        let tick = match self.last_clock_tick {
            Some(last) => unwrap(time, last),
            None => i64::from(time),
        };
        self.last_clock_tick = Some(tick);
        let offset = (now_us.min(i64::MAX as u64) as i64)
            .saturating_sub(tick.saturating_mul(TICK_US as i64));
        self.last_offset = offset;
        if W > 0 {
            self.offsets[self.next] = offset;
            self.next = (self.next + 1) % W;
            self.len = (self.len + 1).min(W);
        }
    }

    /// Render time of a message that is not a JR message.
    fn render(&mut self, jr_expected: bool, now_us: u64) -> Disposition {
        let at = match self.timestamp.filter(|_| jr_expected) {
            Some(ts) => self
                .to_local(ts)
                .map(|local| local.saturating_add(self.delay())),
            None => None,
        };
        // 7.2.2.2: render in receive order, so never before the last render time.
        match (at, self.last_render_us) {
            (Some(at), last) => {
                let at = last.map_or(at, |last| at.max(last));
                self.last_render_us = Some(at);
                Disposition::RenderAt(at)
            }
            (None, Some(last)) if last > now_us => Disposition::RenderAt(last),
            (None, _) => Disposition::RenderNow,
        }
    }
}

/// The 16-bit sender time of a local time in µs.
fn sender_time(us: u64) -> u16 {
    (us / TICK_US) as u16
}

/// Unwraps a 16-bit time to the tick nearest to `reference` (half-range rule).
fn unwrap(time: u16, reference: i64) -> i64 {
    let delta = time.wrapping_sub(reference as u16) as i16;
    reference.saturating_add(i64::from(delta))
}

/// The packet word of a one-word Utility message.
fn word<T: zmidi_ump::UmpWords>(message: T) -> u32 {
    let mut out = [0u32; 1];
    let _ = message.to_words(&mut out);
    out[0]
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;

    fn clock(time: u16) -> u32 {
        word(JrClock::new().with_time(time))
    }

    fn timestamp(time: u16) -> u32 {
        word(JrTimestamp::new().with_timestamp(time))
    }

    /// A MIDI 1.0 Channel Voice Note On, Group 0 (Message Type 0x2).
    const NOTE_ON: u32 = 0x2090_3C64;

    #[test]
    fn sender_rejects_clock_period_out_of_range() {
        assert_eq!(JrSender::new(0), Err(JrError::ClockPeriod));
        assert_eq!(JrSender::new(250_001), Err(JrError::ClockPeriod));
        assert!(JrSender::new(250_000).is_ok());
        assert!(JrSender::new(1).is_ok());
    }

    #[test]
    fn sender_sends_clock_once_per_period() {
        let mut s = JrSender::new(250_000).unwrap();
        assert_eq!(s.poll_clock(1_000), Some(0x0010_001F)); // 1000 / 32 = 31 ticks
        assert_eq!(s.poll_clock(250_999), None);
        assert_eq!(s.poll_clock(251_000), Some(clock((251_000 / 32) as u16)));
    }

    #[test]
    fn sender_clock_time_wraps_and_never_goes_back() {
        let mut s = JrSender::new(100_000).unwrap();
        // 65536 ticks = 2.097152 s: the time wraps to 0.
        assert_eq!(s.poll_clock(65_536 * 32), Some(clock(0)));
        // A local clock that goes back gives no JR Clock.
        assert_eq!(s.poll_clock(1_000), None);
    }

    #[test]
    fn sender_shares_timestamp_within_one_tick() {
        let mut s = JrSender::new(250_000).unwrap();
        assert_eq!(s.timestamp(3_200), Ok(Some(0x0020_0064))); // tick 100
        assert_eq!(s.timestamp(3_231), Ok(None)); // same tick
        assert_eq!(s.timestamp(4_800), Ok(Some(timestamp(150))));
    }

    #[test]
    fn sender_rejects_out_of_order_render_time() {
        let mut s = JrSender::new(250_000).unwrap();
        assert!(s.timestamp(6_400).is_ok());
        assert_eq!(s.timestamp(6_399), Err(JrError::OutOfOrder));
        assert_eq!(s.timestamp(6_400), Ok(None));
    }

    #[test]
    fn receiver_renders_now_before_first_clock() {
        let mut r = JrReceiver::<4>::new(500);
        assert_eq!(r.receive(&[timestamp(100)], true, 0), Disposition::Consumed);
        assert_eq!(r.receive(&[NOTE_ON], true, 0), Disposition::RenderNow);
        assert_eq!(r.to_local(100), None);
    }

    #[test]
    fn receiver_applies_last_timestamp_to_following_messages() {
        let mut r = JrReceiver::<4>::new(500);
        // Sender tick 1000 received at local 50 000 µs: offset 18 000 µs.
        assert_eq!(
            r.receive(&[clock(1000)], true, 50_000),
            Disposition::Consumed
        );
        assert_eq!(
            r.receive(&[timestamp(1100)], true, 51_000),
            Disposition::Consumed
        );
        let at = 1100 * 32 + 18_000 + 500;
        assert_eq!(
            r.receive(&[NOTE_ON], true, 51_000),
            Disposition::RenderAt(at)
        );
        // No new JR Timestamp: the last one applies (7.2.2.2).
        assert_eq!(
            r.receive(&[NOTE_ON], true, 51_000),
            Disposition::RenderAt(at)
        );
        // A later JR Timestamp replaces it: the most recent one applies.
        r.receive(&[timestamp(1300)], true, 52_000);
        assert_eq!(
            r.receive(&[NOTE_ON], true, 52_000),
            Disposition::RenderAt(1300 * 32 + 18_000 + 500)
        );
    }

    #[test]
    fn receiver_renders_now_without_timestamp() {
        let mut r = JrReceiver::<4>::new(0);
        r.receive(&[clock(10)], true, 1_000);
        assert_eq!(r.receive(&[NOTE_ON], true, 1_000), Disposition::RenderNow);
        // REQ-JR-012: with no JR Clock received, a message renders now, with or without a JR
        // Timestamp before it.
        let mut r = JrReceiver::<4>::new(0);
        assert_eq!(r.receive(&[NOTE_ON], true, 1_000), Disposition::RenderNow);
        r.receive(&[timestamp(500)], true, 1_000);
        assert_eq!(r.receive(&[NOTE_ON], true, 1_000), Disposition::RenderNow);
    }

    #[test]
    fn receiver_unwraps_sender_time_across_wrap() {
        let mut r = JrReceiver::<4>::new(0);
        r.receive(&[clock(65_000)], true, 2_080_000); // offset 0
        r.receive(&[clock(100)], true, (65_536 + 100) * 32); // wrapped, offset 0
        assert_eq!(r.delay(), 0);
        assert_eq!(r.to_local(200), Some((65_536 + 200) * 32));
        // A time a little before the last clock stays before it.
        assert_eq!(r.to_local(65_500), Some(65_500 * 32));
    }

    #[test]
    fn receiver_delay_ignores_clock_intervals() {
        // Irregular intervals, no jitter: same offset each time (7.2.2.1).
        let mut r = JrReceiver::<8>::new(300);
        for tick in [0u16, 100, 7_000, 7_050, 30_000] {
            r.receive(&[clock(tick)], true, u64::from(tick) * 32 + 5_000);
        }
        assert_eq!(r.delay(), 300);
    }

    #[test]
    fn receiver_delay_is_measured_jitter_plus_margin() {
        let mut r = JrReceiver::<3>::new(100);
        r.receive(&[clock(0)], true, 1_000);
        r.receive(&[clock(1_000)], true, 32_000 + 1_400);
        r.receive(&[clock(2_000)], true, 64_000 + 1_100);
        assert_eq!(r.delay(), 400 + 100);
        // The window holds the last 3 offsets only.
        r.receive(&[clock(3_000)], true, 96_000 + 1_100);
        r.receive(&[clock(4_000)], true, 128_000 + 1_100);
        r.receive(&[clock(5_000)], true, 160_000 + 1_100);
        assert_eq!(r.delay(), 100);
        assert_eq!(r.to_local(5_000), Some(160_000 + 1_100));
    }

    #[test]
    fn receiver_with_empty_window_uses_margin_and_last_offset() {
        let mut r = JrReceiver::<0>::new(250);
        r.receive(&[clock(10)], true, 2_000);
        assert_eq!(r.delay(), 250);
        assert_eq!(r.to_local(10), Some(2_000));
    }

    #[test]
    fn receiver_keeps_receive_order() {
        let mut r = JrReceiver::<4>::new(0);
        r.receive(&[clock(1_000)], true, 32_000);
        r.receive(&[timestamp(1_200)], true, 32_000);
        assert_eq!(
            r.receive(&[NOTE_ON], true, 32_000),
            Disposition::RenderAt(38_400)
        );
        // An earlier timestamp received later renders no earlier (7.2.2.2).
        r.receive(&[timestamp(1_100)], true, 32_100);
        assert_eq!(
            r.receive(&[NOTE_ON], true, 32_100),
            Disposition::RenderAt(38_400)
        );
        // A message with no JR Timestamp before the last render time waits for it; after it, it
        // renders now.
        assert_eq!(
            r.receive(&[NOTE_ON], false, 32_200),
            Disposition::RenderAt(38_400)
        );
        assert_eq!(r.receive(&[NOTE_ON], false, 38_400), Disposition::RenderNow);
    }

    #[test]
    fn receiver_flags_unexpected_timestamp() {
        let mut r = JrReceiver::<4>::new(0);
        r.receive(&[clock(1_000)], false, 32_000);
        assert_eq!(
            r.receive(&[timestamp(1_100)], false, 32_000),
            Disposition::UnexpectedJrTimestamp
        );
        assert_eq!(r.receive(&[NOTE_ON], false, 32_000), Disposition::RenderNow);
    }

    #[test]
    fn prop_receiver_never_panics() {
        proptest!(|(packets in proptest::collection::vec(
            (proptest::collection::vec(any::<u32>(), 0..5), any::<bool>(), any::<u64>()), 0..32))| {
            let mut r = JrReceiver::<4>::new(1_000);
            for (words, expected, now) in &packets {
                let _ = r.receive(words, *expected, *now);
            }
            let _ = r.delay();
            let _ = r.to_local(0);
        });
    }

    #[test]
    fn prop_sender_clock_cadence_holds() {
        proptest!(|(period in 1u32..=250_000, steps in proptest::collection::vec(1u64..300_000, 1..64))| {
            let mut s = JrSender::new(period).unwrap();
            let mut now = 0u64;
            let mut last: Option<u64> = None;
            for step in steps {
                let sent = s.poll_clock(now).is_some();
                match last {
                    // The first poll sends a JR Clock.
                    None => prop_assert!(sent),
                    // A JR Clock exactly when the period has elapsed, never before.
                    Some(l) => prop_assert_eq!(sent, now - l >= u64::from(period)),
                }
                if sent {
                    last = Some(now);
                }
                now += step;
            }
        });
    }
}
