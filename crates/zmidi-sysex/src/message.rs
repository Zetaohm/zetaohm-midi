//! The typed reader of the crate (M1 v4.2.1 Table VIIa pp. T-9, T-10; AMB-M1-60; DEC-017,
//! DEC-022). Always built.
//!
//! [`Message::parse`] takes one whole System Exclusive message, with or without F0H and F7H, and
//! gives each Universal message of an enabled part as its typed message, and every other message
//! raw (REQ-SX-006): a Universal message with an unknown Sub-ID#1 or Sub-ID#2, of a part whose
//! Cargo feature is off, MIDI-CI (Non-Real Time 0DH, which `zmidi-ci` reads) and the families of
//! later parts, as the raw [`Universal`] header and bytes; a manufacturer or Non-Commercial message
//! with its ID and bytes. The part variants exist only with their features (DEC-017: features only
//! at dispatch points).
//!
//! No alloc (DEC-002). Never panics.

use zmidi_ump::ManufacturerId;

#[cfg(feature = "device-control")]
use crate::device_control::DeviceControl;
#[cfg(feature = "gm")]
use crate::gm::GeneralMidi;
#[cfg(feature = "inquiry")]
use crate::inquiry::InquiryMessage;
#[cfg(feature = "mmc-transport")]
use crate::mmc::MmcMessage;
#[cfg(feature = "mtc")]
use crate::mtc::{FullMessage, UserBits};
#[cfg(feature = "notation")]
use crate::notation::{BarMarker, TimeSignature};
#[cfg(feature = "tuning")]
use crate::tuning::TuningMessage;
use crate::universal::{Sysex, SysexError, SysexId, Universal};

/// One System Exclusive message; it borrows the input bytes (REQ-SX-006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message<'a> {
    /// Identity Request or Reply (feature `inquiry`).
    #[cfg(feature = "inquiry")]
    Inquiry(InquiryMessage),
    /// General MIDI System On or Off (feature `gm`).
    #[cfg(feature = "gm")]
    GeneralMidi(GeneralMidi),
    /// Master Volume, Balance, Fine or Coarse Tuning (feature `device-control`).
    #[cfg(feature = "device-control")]
    DeviceControl(DeviceControl),
    /// A MIDI Tuning message (feature `tuning`).
    #[cfg(feature = "tuning")]
    Tuning(TuningMessage<'a>),
    /// The MTC Full Message (feature `mtc`).
    #[cfg(feature = "mtc")]
    MtcFull(FullMessage),
    /// The MTC User Bits message (feature `mtc`).
    #[cfg(feature = "mtc")]
    MtcUserBits(UserBits),
    /// An MMC command or response message (feature `mmc-transport`).
    #[cfg(feature = "mmc-transport")]
    Mmc(MmcMessage<'a>),
    /// The Bar Marker (feature `notation`).
    #[cfg(feature = "notation")]
    BarMarker(BarMarker),
    /// The Time Signature, Immediate or Delayed (feature `notation`).
    #[cfg(feature = "notation")]
    TimeSignature(TimeSignature<'a>),
    /// Any other Universal message: the raw header and bytes.
    Universal(Universal<'a>),
    /// A manufacturer message: its ID and the bytes after it.
    Manufacturer {
        /// The Manufacturer ID.
        id: ManufacturerId,
        /// The bytes after the ID, without F7H.
        data: &'a [u8],
    },
    /// A Non-Commercial (7DH) message: the bytes after the ID, without F7H.
    NonCommercial(&'a [u8]),
}

impl<'a> Message<'a> {
    /// Reads one whole message (REQ-SX-006). Gives the typed message of the enabled part that
    /// reads it, or the raw form; the errors of [`Sysex::parse`], [`Universal::parse`] and of
    /// the part readers (for example a wrong length or a bad checksum) go to the caller. Never
    /// panics.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, SysexError> {
        let s = Sysex::parse(bytes)?;
        let u = match (s.id, Universal::parse(&s)?) {
            (SysexId::Manufacturer(id), _) => {
                return Ok(Self::Manufacturer { id, data: s.data });
            }
            (_, Some(u)) => u,
            (_, None) => return Ok(Self::NonCommercial(s.data)),
        };
        #[cfg(feature = "inquiry")]
        if let Some(m) = InquiryMessage::from_universal(&u)? {
            return Ok(Self::Inquiry(m));
        }
        #[cfg(feature = "gm")]
        if let Some(m) = GeneralMidi::from_universal(&u)? {
            return Ok(Self::GeneralMidi(m));
        }
        #[cfg(feature = "device-control")]
        if let Some(m) = DeviceControl::from_universal(&u)? {
            return Ok(Self::DeviceControl(m));
        }
        #[cfg(feature = "tuning")]
        if let Some(m) = TuningMessage::from_universal(&u)? {
            return Ok(Self::Tuning(m));
        }
        #[cfg(feature = "mtc")]
        if let Some(m) = FullMessage::from_universal(&u)? {
            return Ok(Self::MtcFull(m));
        }
        #[cfg(feature = "mtc")]
        if let Some(m) = UserBits::from_universal(&u)? {
            return Ok(Self::MtcUserBits(m));
        }
        #[cfg(feature = "mmc-transport")]
        if let Some(m) = MmcMessage::from_universal(&u)? {
            return Ok(Self::Mmc(m));
        }
        #[cfg(feature = "notation")]
        if let Some(m) = BarMarker::from_universal(&u)? {
            return Ok(Self::BarMarker(m));
        }
        #[cfg(feature = "notation")]
        if let Some(m) = TimeSignature::from_universal(&u)? {
            return Ok(Self::TimeSignature(m));
        }
        Ok(Self::Universal(u))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(bytes: &[u8]) -> Universal<'_> {
        match Message::parse(bytes) {
            Ok(Message::Universal(u)) => u,
            m => panic!("not raw: {m:?}"),
        }
    }

    #[test]
    fn unknown_sub_ids_are_raw() {
        // REQ-SX-006: an unknown Sub-ID#2 of a part, a family with no part and a later family
        // (AMB-M1-60) are raw.
        let u = raw(&[0xF0, 0x7E, 0x7F, 0x06, 0x03, 0xF7]);
        assert_eq!((u.sub_id1, u.sub_id2), (0x06, Some(0x03)));
        let u = raw(&[0x7E, 0x7F, 0x09, 0x04]);
        assert_eq!((u.sub_id1, u.sub_id2), (0x09, Some(0x04)));
        let u = raw(&[0x7F, 0x7F, 0x04, 0x05, 0, 0]);
        assert_eq!((u.real_time, u.sub_id2), (true, Some(0x05)));
        // Sample Dump Request (03H), File Reference (Non-Real Time 0BH), MTC Cueing (Real Time
        // 05H), and MMC (Real Time 06H) without its part.
        assert_eq!(raw(&[0x7E, 0x00, 0x03, 0x05, 0x00]).data, &[0x05, 0x00]);
        assert_eq!(raw(&[0x7F, 0x7F, 0x05, 0x01, 0x00]).sub_id2, Some(0x01));
        #[cfg(not(feature = "mmc-transport"))]
        assert_eq!(raw(&[0x7F, 0x7F, 0x06, 0x02]).data, &[0x02]);
        #[cfg(not(feature = "mtc"))]
        assert_eq!(
            raw(&[0x7F, 0x7F, 0x01, 0x01, 0x61, 0, 0, 0]).sub_id2,
            Some(0x01)
        );
        assert_eq!(raw(&[0x7E, 0x7F, 0x0B, 0x01, 0x00]).sub_id2, None);
        // MIDI Show Control (Real Time 02H, M1/MSC/01): no Sub-ID#2; the MSC data (command
        // format, command, data) follows Sub-ID#1.
        let u = raw(&[0xF0, 0x7F, 0x01, 0x02, 0x01, 0x01, 0x31, 0xF7]);
        assert_eq!((u.real_time, u.sub_id1, u.sub_id2), (true, 0x02, None));
        assert_eq!(u.data, &[0x01, 0x01, 0x31]);
        #[cfg(not(feature = "notation"))]
        assert_eq!(raw(&[0x7F, 0x7F, 0x03, 0x01, 0, 0]).sub_id2, Some(0x01));
        // Manufacturer and Non-Commercial messages.
        assert_eq!(
            Message::parse(&[0xF0, 0x43, 0x10, 0x4C, 0xF7]),
            Ok(Message::Manufacturer {
                id: ManufacturerId::from_midi1(&[0x43]).unwrap(),
                data: &[0x10, 0x4C]
            })
        );
        assert_eq!(
            Message::parse(&[0x7D, 1, 2]),
            Ok(Message::NonCommercial(&[1, 2]))
        );
        assert_eq!(Message::parse(&[0xF0, 0xF7]), Err(SysexError::NoId));
        // The typed parts, each with its feature.
        #[cfg(feature = "inquiry")]
        assert!(matches!(
            Message::parse(&[0x7E, 0x7F, 0x06, 0x01]),
            Ok(Message::Inquiry(InquiryMessage::Request {
                device_id: 0x7F
            }))
        ));
        #[cfg(feature = "gm")]
        assert!(matches!(
            Message::parse(&[0x7E, 0x7F, 0x09, 0x03]),
            Ok(Message::GeneralMidi(_))
        ));
        #[cfg(not(feature = "gm"))]
        assert_eq!(raw(&[0x7E, 0x7F, 0x09, 0x03]).sub_id2, Some(0x03));
        #[cfg(feature = "device-control")]
        assert!(matches!(
            Message::parse(&[0x7F, 0x7F, 0x04, 0x01, 0x7F, 0x7F]),
            Ok(Message::DeviceControl(_))
        ));
        #[cfg(feature = "tuning")]
        assert!(matches!(
            Message::parse(&[0x7E, 0x7F, 0x08, 0x00, 0x05]),
            Ok(Message::Tuning(_))
        ));
        #[cfg(feature = "mtc")]
        assert!(matches!(
            Message::parse(&[0x7F, 0x7F, 0x01, 0x01, 0x61, 0, 0, 0]),
            Ok(Message::MtcFull(_))
        ));
        #[cfg(feature = "mtc")]
        assert!(matches!(
            Message::parse(&[0x7F, 0x7F, 0x01, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Ok(Message::MtcUserBits(_))
        ));
        #[cfg(feature = "mmc-transport")]
        assert!(matches!(
            Message::parse(&[0xF0, 0x7F, 0x7F, 0x06, 0x02, 0xF7]),
            Ok(Message::Mmc(MmcMessage::Commands {
                device_id: 0x7F,
                ..
            }))
        ));
        #[cfg(feature = "notation")]
        assert!(matches!(
            Message::parse(&[0x7F, 0x7F, 0x03, 0x01, 0x7F, 0x3F]),
            Ok(Message::BarMarker(_))
        ));
        #[cfg(feature = "notation")]
        assert!(matches!(
            Message::parse(&[0x7F, 0x7F, 0x03, 0x42, 4, 4, 2, 24, 8]),
            Ok(Message::TimeSignature(_))
        ));
        // A part reader error goes to the caller.
        #[cfg(feature = "inquiry")]
        assert_eq!(
            Message::parse(&[0x7E, 0x7F, 0x06, 0x01, 0x00]),
            Err(SysexError::Length)
        );
    }

    #[test]
    fn midi_ci_is_raw() {
        // REQ-SX-006: MIDI-CI (Non-Real Time 0DH, M2-101) stays raw; zmidi-ci reads it.
        let discovery = [0xF0, 0x7E, 0x7F, 0x0D, 0x70, 0x02, 0x01, 0x02, 0xF7];
        let u = raw(&discovery);
        assert_eq!((u.sub_id1, u.sub_id2), (0x0D, None));
        assert_eq!(u.data, &[0x70, 0x02, 0x01, 0x02]);
    }
}
