//! MIDI 1.0 Active Sensing (M1 v4.2.1 Active Sensing pp. 32, 33; AMB-M1-15; DEC-022): a monitor
//! for the receiver and a sender for the transmitter. Both are sans-IO: the application gives the
//! time of each call in microseconds (DEC-003), as the other sans-IO states of the library do.
//!
//! - [`ActiveSensingMonitor`]: no timeout until the first Active Sensing byte (FEH, p. 32: "If a
//!   device never receives Active Sensing it should operate normally"). After it, each received
//!   byte restarts the timer (the flowchart of p. 33: "Receive Data, Clear Timer"); more than the
//!   timeout (330 ms by default) with no byte gives one timeout, and the monitor waits for FEH
//!   again. At the timeout the device turns off its voices.
//! - [`ActiveSensingSender`]: asks for an FEH byte when no byte was sent for its interval (270 ms by
//!   default, p. 32: "transmit Active Sensing within 270ms").
//!
//! No alloc (DEC-002), no I/O (DEC-003). Never panics.

/// The default timeout of the monitor: 330 ms (p. 32: "receivers judge at over 330ms").
pub const ACTIVE_SENSING_TIMEOUT_US: u32 = 330_000;

/// The default interval of the sender: 270 ms (p. 32: "transmitters transmit Active Sensing within
/// 270ms").
pub const ACTIVE_SENSING_INTERVAL_US: u32 = 270_000;

/// The Active Sensing byte.
const ACTIVE_SENSING: u8 = 0xFE;

/// The Active Sensing monitor of a receiver: the armed flag, the time of the last received byte
/// and the timeout. 16 bytes, no alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActiveSensingMonitor {
    /// The time of the last received byte, in microseconds.
    last_us: u64,
    /// The timeout, in microseconds.
    timeout_us: u32,
    /// True after an FEH byte, until the timeout.
    armed: bool,
}

impl ActiveSensingMonitor {
    /// A monitor with a timeout of `timeout_us` microseconds, not armed.
    pub const fn new(timeout_us: u32) -> Self {
        Self {
            last_us: 0,
            timeout_us,
            armed: false,
        }
    }

    /// Takes one received byte at the time `now_us` (REQ-AS-001): an Active Sensing byte (FEH) arms
    /// the monitor; while it is armed, each byte restarts the timer.
    pub fn receive_byte(&mut self, byte: u8, now_us: u64) {
        self.receive(byte == ACTIVE_SENSING, now_us);
    }

    /// Takes one received message at the time `now_us`, for a transport with no bytes (a UMP, a
    /// USB-MIDI Event Packet): `active_sensing` is true for an Active Sensing message. The same
    /// rules as [`Self::receive_byte`].
    pub fn receive(&mut self, active_sensing: bool, now_us: u64) {
        if active_sensing {
            self.armed = true;
        }
        if self.armed {
            self.last_us = now_us;
        }
    }

    /// Checks the timer at the time `now_us`. Gives true one time when more than the timeout passed
    /// since the last received byte; the monitor is then not armed until the next FEH byte.
    pub fn poll(&mut self, now_us: u64) -> bool {
        if self.armed && now_us.saturating_sub(self.last_us) > u64::from(self.timeout_us) {
            self.armed = false;
            return true;
        }
        false
    }

    /// Tells if the monitor is armed: it received FEH and gave no timeout after it.
    pub const fn is_armed(&self) -> bool {
        self.armed
    }

    /// The timeout, in microseconds.
    pub const fn timeout_us(&self) -> u32 {
        self.timeout_us
    }
}

impl Default for ActiveSensingMonitor {
    /// A monitor with the default timeout of 330 ms.
    fn default() -> Self {
        Self::new(ACTIVE_SENSING_TIMEOUT_US)
    }
}

/// The Active Sensing sender of a transmitter: the time of the last sent byte and the interval.
/// 16 bytes, no alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActiveSensingSender {
    /// The time of the last sent byte, in microseconds.
    last_us: u64,
    /// The interval, in microseconds.
    interval_us: u32,
}

impl ActiveSensingSender {
    /// A sender with an interval of `interval_us` microseconds; the interval starts at time 0.
    pub const fn new(interval_us: u32) -> Self {
        Self {
            last_us: 0,
            interval_us,
        }
    }

    /// Tells the sender that the application sent a byte at the time `now_us`; the interval starts
    /// again (REQ-AS-002).
    pub fn sent(&mut self, now_us: u64) {
        self.last_us = now_us;
    }

    /// Checks the interval at the time `now_us`. Gives true when no byte was sent for the interval:
    /// the application then sends one FEH byte. The interval starts again at `now_us`.
    pub fn poll(&mut self, now_us: u64) -> bool {
        if now_us.saturating_sub(self.last_us) >= u64::from(self.interval_us) {
            self.last_us = now_us;
            return true;
        }
        false
    }

    /// The interval, in microseconds.
    pub const fn interval_us(&self) -> u32 {
        self.interval_us
    }
}

impl Default for ActiveSensingSender {
    /// A sender with the default interval of 270 ms.
    fn default() -> Self {
        Self::new(ACTIVE_SENSING_INTERVAL_US)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const MS: u64 = 1000;

    #[test]
    fn monitor_waits_for_the_first_fe() {
        // p. 32: "If a device never receives Active Sensing it should operate normally."
        let mut m = ActiveSensingMonitor::default();
        m.receive_byte(0x90, 0);
        assert!(!m.poll(10_000 * MS));
        assert!(!m.is_armed());
        m.receive_byte(0xFE, 10_000 * MS);
        assert!(m.is_armed());
    }

    #[test]
    fn monitor_times_out_once_after_330_ms() {
        // p. 32: expect a message of some kind every 300 milliseconds, judge at over 330 ms; the
        // flowchart clears the timer at each received byte, and after the time-out resets its flag.
        let mut m = ActiveSensingMonitor::default();
        assert_eq!(m.timeout_us(), 330_000);
        m.receive_byte(0xFE, 0);
        m.receive_byte(0x90, 300 * MS);
        assert!(
            !m.poll(630 * MS),
            "at exactly 330 ms after the last byte: not over"
        );
        assert!(m.poll(631 * MS));
        assert!(!m.poll(2000 * MS), "one timeout only");
        assert!(!m.is_armed());
        // A byte other than FEH does not arm it again.
        m.receive_byte(0x90, 2000 * MS);
        assert!(!m.poll(5000 * MS));
        // A message transport: the same rules.
        m.receive(true, 6000 * MS);
        m.receive(false, 6100 * MS);
        assert!(!m.poll(6400 * MS));
        assert!(m.poll(6431 * MS));
        // The application can change the timeout.
        let mut short = ActiveSensingMonitor::new(100_000);
        short.receive_byte(0xFE, 0);
        assert!(short.poll(101 * MS));
    }

    #[test]
    fn prop_monitor_times_out_only_after_silence() {
        // The monitor gives a timeout only when armed and more than the timeout passed since the
        // last byte; never two timeouts with no new FEH between them.
        proptest!(|(steps in proptest::collection::vec((0u64..400_000, any::<bool>(), any::<bool>()), 1..40))| {
            let mut m = ActiveSensingMonitor::default();
            let (mut now, mut last, mut armed) = (0u64, 0u64, false);
            for (dt, fe, byte) in steps {
                now += dt;
                if byte {
                    m.receive(fe, now);
                    armed |= fe;
                    if armed { last = now; }
                } else {
                    let want = armed && now - last > 330_000;
                    prop_assert_eq!(m.poll(now), want);
                    if want { armed = false; }
                }
                prop_assert_eq!(m.is_armed(), armed);
            }
        });
    }

    #[test]
    fn sender_asks_after_270_ms_of_silence() {
        // p. 32: transmit Active Sensing within 270 ms "whenever there is no other MIDI data".
        let mut s = ActiveSensingSender::default();
        assert_eq!(s.interval_us(), 270_000);
        assert!(!s.poll(269 * MS));
        assert!(s.poll(270 * MS));
        assert!(!s.poll(500 * MS));
        // Other bytes restart the interval.
        s.sent(530 * MS);
        assert!(!s.poll(799 * MS));
        assert!(s.poll(800 * MS));
        // A time that goes back gives no request.
        assert!(!s.poll(10 * MS));
        let mut fast = ActiveSensingSender::new(100_000);
        assert!(fast.poll(100 * MS));
    }
}
