//! The General MIDI System messages (M1 v4.2.1 p. 52, RP-003 p. 4, RP-024 4.9 p. 22; Universal
//! Non-Real Time Sub-ID#1 09H; DEC-022). Feature `gm`.
//!
//! - GM1 System On (Sub-ID#2 01H), GM System Off (02H) and GM2 System On (03H), each
//!   `F0 7E <device ID> 09 <sub-ID#2> F7` (REQ-GM-001). RP-003 and RP-024 suggest the Device ID
//!   7FH ([`crate::ALL_DEVICES`]).
//! - Messages only. The receiver state of General MIDI (the kind of each channel, the GM reset and
//!   the GM2 default values) is the `zmidi-io` feature `gm`; the application gives each message
//!   read here to it.
//!
//! No alloc (DEC-002). Never panics.

use crate::universal::{write_universal, NonRealTimeId, SysexError, Universal};

/// Sub-ID#1 09H: General MIDI.
const GENERAL_MIDI: u8 = NonRealTimeId::GeneralMidi.into_bits();

/// The command of a General MIDI System message: its Sub-ID#2 (REQ-GM-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GmCommand {
    /// 01H: General MIDI (Level 1) System On.
    Gm1SystemOn = 0x01,
    /// 02H: General MIDI System Off.
    SystemOff = 0x02,
    /// 03H: General MIDI 2 System On (RP-024 4.9.1).
    Gm2SystemOn = 0x03,
}

impl GmCommand {
    /// The Sub-ID#2 value.
    pub const fn into_bits(self) -> u8 {
        self as u8
    }

    /// The command of a Sub-ID#2 value, or `None` for another value.
    pub const fn from_bits(value: u8) -> Option<Self> {
        Some(match value {
            0x01 => Self::Gm1SystemOn,
            0x02 => Self::SystemOff,
            0x03 => Self::Gm2SystemOn,
            _ => return None,
        })
    }
}

/// One General MIDI System message (REQ-GM-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GeneralMidi {
    /// The Device ID; 7FH addresses all devices.
    pub device_id: u8,
    /// The command.
    pub command: GmCommand,
}

impl GeneralMidi {
    /// Reads the message from a Universal header (Non-Real Time, Sub-ID#1 09H). Gives `None` for
    /// another message or another Sub-ID#2, and [`SysexError::Length`] when bytes follow the
    /// Sub-ID#2.
    pub fn from_universal(u: &Universal<'_>) -> Result<Option<Self>, SysexError> {
        if u.real_time || u.sub_id1 != GENERAL_MIDI {
            return Ok(None);
        }
        let Some(command) = u.sub_id2.and_then(GmCommand::from_bits) else {
            return Ok(None);
        };
        if !u.data.is_empty() {
            return Err(SysexError::Length);
        }
        Ok(Some(Self {
            device_id: u.device_id,
            command,
        }))
    }

    /// Writes `F0 7E <device ID> 09 <sub-ID#2> F7` into `out` and gives the length (REQ-SX-007).
    /// Gives [`SysexError::OutOfRange`] for a Device ID above 7FH.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, SysexError> {
        write_universal(
            out,
            false,
            self.device_id,
            GENERAL_MIDI,
            Some(self.command.into_bits()),
            |_| Ok(()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universal::{Sysex, ALL_DEVICES};

    fn read(bytes: &[u8]) -> Result<Option<GeneralMidi>, SysexError> {
        let s = Sysex::parse(bytes).unwrap();
        GeneralMidi::from_universal(&Universal::parse(&s).unwrap().unwrap())
    }

    #[test]
    fn general_midi_system_messages_bytes() {
        // REQ-GM-001: M1 p. 52 and RP-003 p. 4 (09 01, 09 02), RP-024 4.9.1 p. 22 (09 03).
        let mut out = [0u8; 6];
        for (command, sub) in [
            (GmCommand::Gm1SystemOn, 0x01),
            (GmCommand::SystemOff, 0x02),
            (GmCommand::Gm2SystemOn, 0x03),
        ] {
            let bytes = [0xF0, 0x7E, 0x7F, 0x09, sub, 0xF7];
            let m = GeneralMidi {
                device_id: ALL_DEVICES,
                command,
            };
            assert_eq!(read(&bytes), Ok(Some(m)));
            assert_eq!(m.write(&mut out), Ok(6));
            assert_eq!(out, bytes);
            assert_eq!(GmCommand::from_bits(sub), Some(command));
        }
        // Another Sub-ID#2 is raw; bytes after the Sub-ID#2 are not allowed.
        assert_eq!(read(&[0x7E, 0x7F, 0x09, 0x04]), Ok(None));
        assert_eq!(
            read(&[0x7E, 0x7F, 0x09, 0x01, 0x00]),
            Err(SysexError::Length)
        );
        assert_eq!(read(&[0x7E, 0x7F, 0x06, 0x01]), Ok(None));
        let bad = GeneralMidi {
            device_id: 0x80,
            command: GmCommand::Gm2SystemOn,
        };
        assert_eq!(bad.write(&mut out), Err(SysexError::OutOfRange));
        assert_eq!(
            GeneralMidi {
                device_id: 1,
                ..bad
            }
            .write(&mut out[..5]),
            Err(SysexError::BufferTooSmall { needed: 6 })
        );
    }
}
