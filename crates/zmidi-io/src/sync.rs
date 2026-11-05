//! MIDI 1.0 sync receiver: the external MIDI clock of a sequencer or drum machine (M1 v4.2.1 Song
//! Position Pointer and Song Select pp. 27 to 29, System Real Time pp. 30 to 32; DEC-022).
//!
//! - [`SyncReceiver::receive`] takes a typed System message (DIN bytes, USB-MIDI 1.0, BLE-MIDI or a
//!   Message Type 0x1 UMP) and gives at most one [`SyncEvent`].
//! - The position is a count of MIDI clocks: 24 for each quarter note (p. 30), 6 for each Song
//!   Position Pointer step (p. 27). The application multiplies it by its own ticks per clock.
//! - Start and Continue wait for the next Timing Clock; that clock plays (pp. 30, 31). While the
//!   receiver is stopped, a Timing Clock gives nothing (p. 31).
//! - Out of MIDI sync mode the receiver ignores Song Position Pointer and Song Select (pp. 27, 29).
//!   It still gives Start, Stop and Continue: a device on its internal clock decides what to do with
//!   them (p. 30).
//! - A System Reset message changes nothing here; a receiver that implements System Reset calls
//!   [`SyncReceiver::system_reset`] (p. 33, AMB-M1-13).
//!
//! No alloc (DEC-002), no I/O (DEC-003). Never panics.

use zmidi_ump::{SystemMessage, CLOCKS_PER_MIDI_BEAT};

/// The run state of a sync receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SyncRun {
    /// Stopped: a Timing Clock does not advance the position.
    #[default]
    Stopped,
    /// After Start or Continue, until the first Timing Clock (p. 30).
    Waiting,
    /// Playing: each Timing Clock advances the position.
    Playing,
}

/// A start, stop or continue command of the application, for example from a front panel
/// (p. 32: the last command applies).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyncCommand {
    /// Start: position 0, then play at the next Timing Clock.
    Start,
    /// Continue: play from the held position at the next Timing Clock.
    Continue,
    /// Stop: stop at once and keep the position.
    Stop,
}

/// What a message or a command gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyncEvent {
    /// Start: the position is 0 and the receiver waits for the next Timing Clock.
    Start,
    /// Continue: the receiver waits at the held position for the next Timing Clock.
    Continue,
    /// Stop: the receiver stopped and keeps its position.
    Stop,
    /// A Timing Clock while the receiver plays. `position` is the position of this clock; after it
    /// the position is one more.
    Clock {
        /// The position of this clock, in MIDI clocks.
        position: u32,
    },
    /// An accepted Song Position Pointer: the new position.
    Locate {
        /// The new position, in MIDI clocks (the Song Position Pointer value × 6).
        position: u32,
    },
    /// An accepted Song Select: the song for the next Start.
    Song {
        /// The song number, 0 to 127.
        number: u8,
    },
}

/// The sync receiver of one device: run state, position in MIDI clocks, selected song and MIDI
/// sync mode. 8 bytes, all zeros when new, no alloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SyncReceiver {
    /// The position in MIDI clocks.
    position: u32,
    /// The run state.
    run: SyncRun,
    /// The song of the last accepted Song Select.
    song: u8,
    /// True out of MIDI sync mode (stored inverted, so that a new receiver is all zeros).
    internal: bool,
}

impl SyncReceiver {
    /// A receiver that is stopped at position 0, with song 0, in MIDI sync mode.
    pub const fn new() -> Self {
        Self {
            position: 0,
            run: SyncRun::Stopped,
            song: 0,
            internal: false,
        }
    }

    /// Takes one typed System message and gives at most one event (REQ-SYNC-001 to REQ-SYNC-004).
    /// Never panics.
    pub fn receive(&mut self, message: &SystemMessage) -> Option<SyncEvent> {
        match message {
            SystemMessage::Start(_) => self.command(SyncCommand::Start),
            SystemMessage::Continue(_) => self.command(SyncCommand::Continue),
            SystemMessage::Stop(_) => self.command(SyncCommand::Stop),
            SystemMessage::TimingClock(_) => self.clock(),
            SystemMessage::SongPositionPointer(p) if !self.internal => {
                self.position = u32::from(p.position()) * CLOCKS_PER_MIDI_BEAT;
                Some(SyncEvent::Locate {
                    position: self.position,
                })
            }
            SystemMessage::SongSelect(s) if !self.internal => {
                self.song = s.song();
                Some(SyncEvent::Song { number: self.song })
            }
            _ => None,
        }
    }

    /// Takes one command of the application, with the transitions of the MIDI command
    /// (REQ-SYNC-003): a redundant command gives nothing.
    pub fn command(&mut self, command: SyncCommand) -> Option<SyncEvent> {
        match (command, self.run) {
            (SyncCommand::Stop, SyncRun::Stopped) => None,
            (SyncCommand::Stop, _) => {
                self.run = SyncRun::Stopped;
                Some(SyncEvent::Stop)
            }
            (_, SyncRun::Waiting | SyncRun::Playing) => None,
            (SyncCommand::Start, SyncRun::Stopped) => {
                // p. 28: Start is a Song Position Pointer of 0 and a Continue.
                self.position = 0;
                self.run = SyncRun::Waiting;
                Some(SyncEvent::Start)
            }
            (SyncCommand::Continue, SyncRun::Stopped) => {
                self.run = SyncRun::Waiting;
                Some(SyncEvent::Continue)
            }
        }
    }

    fn clock(&mut self) -> Option<SyncEvent> {
        if self.run == SyncRun::Stopped {
            return None;
        }
        self.run = SyncRun::Playing;
        let position = self.position;
        self.position = position.saturating_add(1);
        Some(SyncEvent::Clock { position })
    }

    /// The System Reset action of the application (p. 33): position 0 and stopped; the song and
    /// the MIDI sync mode stay (REQ-SYNC-005).
    pub fn system_reset(&mut self) {
        self.position = 0;
        self.run = SyncRun::Stopped;
    }

    /// The run state.
    pub const fn run(&self) -> SyncRun {
        self.run
    }

    /// The position in MIDI clocks: the position of the next Timing Clock that plays.
    pub const fn position(&self) -> u32 {
        self.position
    }

    /// The song of the last accepted Song Select, or 0.
    pub const fn song(&self) -> u8 {
        self.song
    }

    /// Tells if the receiver is in MIDI sync mode (the default).
    pub const fn sync_mode(&self) -> bool {
        !self.internal
    }

    /// Sets MIDI sync mode on or off; out of it the receiver ignores Song Position Pointer and Song
    /// Select.
    pub fn set_sync_mode(&mut self, on: bool) {
        self.internal = !on;
    }
}

impl Default for SyncReceiver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;
    use zmidi_ump::Group;

    fn m(status: u8, d1: u8, d2: u8) -> SystemMessage {
        let w = 0x1000_0000 | u32::from(status) << 16 | u32::from(d1) << 8 | u32::from(d2);
        SystemMessage::parse(&[w]).unwrap()
    }

    const START: u8 = 0xFA;
    const CONTINUE: u8 = 0xFB;
    const STOP: u8 = 0xFC;
    const CLOCK: u8 = 0xF8;

    fn feed(s: &mut SyncReceiver, bytes: &[u8]) -> Vec<Option<SyncEvent>> {
        bytes.iter().map(|b| s.receive(&m(*b, 0, 0))).collect()
    }

    fn spp(value: u16) -> SystemMessage {
        SystemMessage::SongPositionPointer(
            zmidi_ump::SongPositionPointer::try_new(Group::from_bits(0), value).unwrap(),
        )
    }

    #[test]
    fn new_receiver_is_all_zeros() {
        let s = SyncReceiver::new();
        assert_eq!(s, SyncReceiver::default());
        assert_eq!(
            (s.position, s.run, s.song, s.internal),
            (0, SyncRun::Stopped, 0, false)
        );
        assert!(s.sync_mode());
    }

    #[test]
    fn start_and_continue_wait_for_the_first_clock() {
        // p. 30: Start or Continue does not start the sequence until the next Timing Clock; p. 31:
        // no advance between FA and the first F8.
        let mut s = SyncReceiver::new();
        assert_eq!(s.receive(&m(START, 0, 0)), Some(SyncEvent::Start));
        assert_eq!((s.run(), s.position()), (SyncRun::Waiting, 0));
        assert_eq!(
            s.receive(&m(CLOCK, 0, 0)),
            Some(SyncEvent::Clock { position: 0 })
        );
        assert_eq!((s.run(), s.position()), (SyncRun::Playing, 1));
        s.receive(&m(STOP, 0, 0));
        assert_eq!(s.receive(&m(CONTINUE, 0, 0)), Some(SyncEvent::Continue));
        assert_eq!((s.run(), s.position()), (SyncRun::Waiting, 1));
        assert_eq!(
            s.receive(&m(CLOCK, 0, 0)),
            Some(SyncEvent::Clock { position: 1 })
        );
    }

    #[test]
    fn srt_20_example_ticks() {
        // p. 31 figure, 96 pulses per quarter note (4 ticks per clock): FA F8 ... (16 clocks) FC F8,
        // then FB F8: tick 0 at the first clock after Start, no advance after Stop, tick 64 after
        // Continue.
        let mut s = SyncReceiver::new();
        s.receive(&m(START, 0, 0));
        let ticks: Vec<u32> = feed(&mut s, &[CLOCK; 16])
            .into_iter()
            .flatten()
            .map(|e| match e {
                SyncEvent::Clock { position } => position * 4,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(ticks, (0..16).map(|c| c * 4).collect::<Vec<_>>());
        assert_eq!(feed(&mut s, &[STOP, CLOCK]), [Some(SyncEvent::Stop), None]);
        assert_eq!(
            feed(&mut s, &[CONTINUE, CLOCK]),
            [
                Some(SyncEvent::Continue),
                Some(SyncEvent::Clock { position: 16 })
            ]
        );
        assert_eq!(16 * 4, 64);
    }

    #[test]
    fn stop_keeps_the_position() {
        // p. 31: stop at once and keep the Song Position for a later Continue; Timing Clocks while
        // stopped do not advance it.
        let mut s = SyncReceiver::new();
        feed(&mut s, &[START, CLOCK, CLOCK, CLOCK]);
        assert_eq!(s.receive(&m(STOP, 0, 0)), Some(SyncEvent::Stop));
        assert_eq!(feed(&mut s, &[CLOCK, CLOCK]), [None, None]);
        assert_eq!((s.run(), s.position()), (SyncRun::Stopped, 3));
    }

    #[test]
    fn redundant_commands_are_ignored() {
        // p. 32: Stop while stopped, and Start or Continue while playing, are ignored.
        let mut s = SyncReceiver::new();
        assert_eq!(s.receive(&m(STOP, 0, 0)), None);
        s.receive(&m(START, 0, 0));
        assert_eq!(feed(&mut s, &[START, CONTINUE]), [None, None]);
        s.receive(&m(CLOCK, 0, 0));
        assert_eq!(feed(&mut s, &[START, CONTINUE]), [None, None]);
        assert_eq!(s.position(), 1);
    }

    #[test]
    fn front_panel_commands_have_the_same_transitions() {
        // p. 32: commands from the front panel and from MIDI In; the last one applies.
        let mut a = SyncReceiver::new();
        let mut b = SyncReceiver::new();
        for (cmd, byte) in [
            (SyncCommand::Start, START),
            (SyncCommand::Stop, STOP),
            (SyncCommand::Stop, STOP),
            (SyncCommand::Continue, CONTINUE),
            (SyncCommand::Start, START),
        ] {
            assert_eq!(a.command(cmd), b.receive(&m(byte, 0, 0)));
            assert_eq!(a, b);
        }
        // MIDI Stop after a panel Start.
        let mut s = SyncReceiver::new();
        s.command(SyncCommand::Start);
        assert_eq!(s.receive(&m(STOP, 0, 0)), Some(SyncEvent::Stop));
    }

    #[test]
    fn song_position_pointer_relocates() {
        // p. 27: the position becomes value × 6 clocks.
        let mut s = SyncReceiver::new();
        assert_eq!(
            s.receive(&spp(10)),
            Some(SyncEvent::Locate { position: 60 })
        );
        assert_eq!(s.position(), 60);
        assert_eq!(
            s.receive(&spp(16383)),
            Some(SyncEvent::Locate { position: 98_298 })
        );
        // p. 28: Start is a Song Position Pointer of 0 and a Continue.
        assert_eq!(s.receive(&m(START, 0, 0)), Some(SyncEvent::Start));
        assert_eq!(s.position(), 0);
    }

    #[test]
    fn sc_13_example_gives_27_clocks() {
        // p. 28: SPP 4 (24 clocks), then Continue and three Timing Clocks while the receiver locates:
        // it begins from the 27th clock.
        let mut s = SyncReceiver::new();
        s.receive(&spp(4));
        s.receive(&m(CONTINUE, 0, 0));
        let positions: Vec<_> = feed(&mut s, &[CLOCK; 3]).into_iter().flatten().collect();
        assert_eq!(
            positions,
            [24, 25, 26].map(|position| SyncEvent::Clock { position })
        );
        assert_eq!(s.position(), 27);
    }

    #[test]
    fn song_select_is_held() {
        // p. 29: the song to play at the next Start.
        let mut s = SyncReceiver::new();
        assert_eq!(
            s.receive(&m(0xF3, 5, 0)),
            Some(SyncEvent::Song { number: 5 })
        );
        assert_eq!(s.song(), 5);
        s.receive(&m(START, 0, 0));
        assert_eq!(s.song(), 5);
    }

    #[test]
    fn sync_mode_off_ignores_position_and_song() {
        // pp. 27, 29: ignored when the receiver is not in MIDI sync mode; Start, Stop and Continue
        // still come (p. 30: the device decides).
        let mut s = SyncReceiver::new();
        s.set_sync_mode(false);
        assert!(!s.sync_mode());
        assert_eq!(s.receive(&spp(4)), None);
        assert_eq!(s.receive(&m(0xF3, 5, 0)), None);
        assert_eq!((s.position(), s.song()), (0, 0));
        assert_eq!(s.receive(&m(START, 0, 0)), Some(SyncEvent::Start));
        s.set_sync_mode(true);
        assert_eq!(s.receive(&spp(4)), Some(SyncEvent::Locate { position: 24 }));
    }

    #[test]
    fn system_reset_stops_at_position_0() {
        // p. 33: Set Song Position to 0, Stop playback. A System Reset message alone changes nothing
        // (AMB-M1-13); the song and the sync mode stay.
        let mut s = SyncReceiver::new();
        s.receive(&m(0xF3, 7, 0));
        feed(&mut s, &[START, CLOCK, CLOCK]);
        assert_eq!(s.receive(&m(0xFF, 0, 0)), None);
        assert_eq!((s.run(), s.position()), (SyncRun::Playing, 2));
        s.system_reset();
        assert_eq!(
            (s.run(), s.position(), s.song(), s.sync_mode()),
            (SyncRun::Stopped, 0, 7, true)
        );
    }
}
