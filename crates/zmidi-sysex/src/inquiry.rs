//! Device Inquiry: the General Information messages (M1 v4.2.1 p. 40; Universal Non-Real Time
//! Sub-ID#1 06H; DEC-022). Feature `inquiry`.
//!
//! - Identity Request (Sub-ID#2 01H): `F0 7E <device ID> 06 01 F7` (REQ-INQ-001).
//! - Identity Reply (Sub-ID#2 02H): `F0 7E <device ID> 06 02 mm ff ff dd dd ss ss ss ss F7`: the
//!   Manufacturer ID (three bytes when the first byte is 00H), the device family code and the
//!   family member code (14 bits each, LSB first) and four bytes of software revision level
//!   (REQ-INQ-002).
//! - [`respond`] answers a request with Device ID 7FH or the own Device ID; the reply carries the
//!   own Device ID (AMB-M1-31, REQ-INQ-003).
//!
//! No alloc (DEC-002). Never panics.

use zmidi_ump::ManufacturerId;

use crate::universal::non_real_time::general_information::{IDENTITY_REPLY, IDENTITY_REQUEST};
use crate::universal::{addressed, write_universal, NonRealTimeId, SysexError, Universal};

/// Sub-ID#1 06H: General Information.
const GENERAL_INFORMATION: u8 = NonRealTimeId::GeneralInformation.into_bits();

/// The identity of a device in an Identity Reply (REQ-INQ-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity {
    /// The Manufacturer ID: one byte, or three bytes when the first byte is 00H.
    pub manufacturer: ManufacturerId,
    /// The device family code, 14 bits (sent LSB first).
    pub family: u16,
    /// The device family member code, 14 bits (sent LSB first).
    pub member: u16,
    /// The software revision level, four 7-bit bytes (format device specific).
    pub revision: [u8; 4],
}

impl Identity {
    /// The identity, or [`SysexError::OutOfRange`] for a code above 14 bits or a revision byte
    /// above 7FH.
    pub const fn new(
        manufacturer: ManufacturerId,
        family: u16,
        member: u16,
        revision: [u8; 4],
    ) -> Result<Self, SysexError> {
        let id = Self {
            manufacturer,
            family,
            member,
            revision,
        };
        if id.valid() {
            Ok(id)
        } else {
            Err(SysexError::OutOfRange)
        }
    }

    const fn valid(&self) -> bool {
        let [a, b, c, d] = self.revision;
        self.family <= 0x3FFF && self.member <= 0x3FFF && (a | b | c | d) <= 0x7F
    }
}

/// One General Information message (REQ-INQ-001, REQ-INQ-002).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InquiryMessage {
    /// Identity Request.
    Request {
        /// The Device ID; 7FH asks all devices.
        device_id: u8,
    },
    /// Identity Reply.
    Reply {
        /// The Device ID of the device that replies.
        device_id: u8,
        /// Its identity.
        identity: Identity,
    },
}

impl InquiryMessage {
    /// Reads the message from a Universal header (Non-Real Time, Sub-ID#1 06H). Gives `None` for
    /// another message, and [`SysexError::Length`] for a length that the format does not allow.
    pub fn from_universal(u: &Universal<'_>) -> Result<Option<Self>, SysexError> {
        if u.real_time || u.sub_id1 != GENERAL_INFORMATION {
            return Ok(None);
        }
        let device_id = u.device_id;
        match u.sub_id2 {
            Some(IDENTITY_REQUEST) if u.data.is_empty() => Ok(Some(Self::Request { device_id })),
            Some(IDENTITY_REQUEST) => Err(SysexError::Length),
            Some(IDENTITY_REPLY) => {
                let n = if u.data.first() == Some(&0) { 3 } else { 1 };
                let (Some(m), Some(rest)) = (u.data.get(..n), u.data.get(n..)) else {
                    return Err(SysexError::Length);
                };
                let &[f0, f1, m0, m1, r0, r1, r2, r3] = rest else {
                    return Err(SysexError::Length);
                };
                let manufacturer =
                    ManufacturerId::from_midi1(m).map_err(|_| SysexError::DataByte)?;
                let identity = Identity {
                    manufacturer,
                    family: f0 as u16 | (f1 as u16) << 7,
                    member: m0 as u16 | (m1 as u16) << 7,
                    revision: [r0, r1, r2, r3],
                };
                Ok(Some(Self::Reply {
                    device_id,
                    identity,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Writes the whole message, F0H to F7H, into `out` and gives the length (REQ-SX-007). Gives
    /// [`SysexError::OutOfRange`] for a Device ID above 7FH or an identity value out of range.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, SysexError> {
        match *self {
            Self::Request { device_id } => write_universal(
                out,
                false,
                device_id,
                GENERAL_INFORMATION,
                Some(IDENTITY_REQUEST),
                |_| Ok(()),
            ),
            Self::Reply {
                device_id,
                identity,
            } => {
                if !identity.valid() {
                    return Err(SysexError::OutOfRange);
                }
                write_universal(
                    out,
                    false,
                    device_id,
                    GENERAL_INFORMATION,
                    Some(IDENTITY_REPLY),
                    |w| {
                        let m = identity.manufacturer.to_midi1();
                        let three = identity.manufacturer.is_three_byte();
                        w.bytes(if three { &m } else { &m[..1] });
                        let (f, d) = (identity.family, identity.member);
                        w.bytes(&[(f & 0x7F) as u8, (f >> 7) as u8]);
                        w.bytes(&[(d & 0x7F) as u8, (d >> 7) as u8]);
                        w.bytes(&identity.revision);
                        Ok(())
                    },
                )
            }
        }
    }
}

/// Answers an Identity Request: writes the Identity Reply into `out` when the request has Device
/// ID 7FH or `own_device_id`, and gives `None` otherwise (also for a message that is not a
/// request). The reply carries `own_device_id`, also for a request with 7FH (AMB-M1-31,
/// REQ-INQ-003).
pub fn respond(
    request: &InquiryMessage,
    own_device_id: u8,
    identity: &Identity,
    out: &mut [u8],
) -> Result<Option<usize>, SysexError> {
    match *request {
        InquiryMessage::Request { device_id } if addressed(device_id, own_device_id) => {
            InquiryMessage::Reply {
                device_id: own_device_id,
                identity: *identity,
            }
            .write(out)
            .map(Some)
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universal::Sysex;

    fn read(bytes: &[u8]) -> Result<Option<InquiryMessage>, SysexError> {
        let s = Sysex::parse(bytes).unwrap();
        InquiryMessage::from_universal(&Universal::parse(&s).unwrap().unwrap())
    }

    #[test]
    fn identity_request_bytes() {
        // REQ-INQ-001: p. 40 "F0 7E <device ID> 06 01 F7".
        let bytes = [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7];
        assert_eq!(
            read(&bytes),
            Ok(Some(InquiryMessage::Request { device_id: 0x7F }))
        );
        let mut out = [0u8; 6];
        let n = InquiryMessage::Request { device_id: 0x7F }
            .write(&mut out)
            .unwrap();
        assert_eq!(&out[..n], &bytes);
        assert_eq!(
            read(&[0x7E, 0x7F, 0x06, 0x01, 0x00]),
            Err(SysexError::Length)
        );
        // Another General Information Sub-ID#2 and another family are not inquiry messages.
        assert_eq!(read(&[0x7E, 0x7F, 0x06, 0x03]), Ok(None));
        assert_eq!(read(&[0x7E, 0x7F, 0x09, 0x01]), Ok(None));
        assert_eq!(read(&[0x7F, 0x7F, 0x06, 0x01]), Ok(None));
        assert_eq!(
            InquiryMessage::Request { device_id: 0x80 }.write(&mut out),
            Err(SysexError::OutOfRange)
        );
    }

    #[test]
    fn identity_reply_one_and_three_byte_ids() {
        // REQ-INQ-002: p. 40 "F0 7E <device ID> 06 02 mm ff ff dd dd ss ss ss ss F7".
        let one = Identity::new(
            ManufacturerId::from_midi1(&[0x43]).unwrap(),
            0x0123,
            0x3FFF,
            [1, 2, 3, 4],
        )
        .unwrap();
        let bytes = [
            0xF0, 0x7E, 0x10, 0x06, 0x02, 0x43, 0x23, 0x02, 0x7F, 0x7F, 1, 2, 3, 4, 0xF7,
        ];
        let reply = InquiryMessage::Reply {
            device_id: 0x10,
            identity: one,
        };
        assert_eq!(read(&bytes), Ok(Some(reply)));
        let mut out = [0u8; 20];
        let n = reply.write(&mut out).unwrap();
        assert_eq!(&out[..n], &bytes);
        let three = Identity {
            manufacturer: ManufacturerId::from_midi1(&[0x00, 0x20, 0x29]).unwrap(),
            ..one
        };
        let bytes = [
            0xF0, 0x7E, 0x10, 0x06, 0x02, 0x00, 0x20, 0x29, 0x23, 0x02, 0x7F, 0x7F, 1, 2, 3, 4,
            0xF7,
        ];
        let reply = InquiryMessage::Reply {
            device_id: 0x10,
            identity: three,
        };
        assert_eq!(read(&bytes), Ok(Some(reply)));
        let n = reply.write(&mut out).unwrap();
        assert_eq!(&out[..n], &bytes);
        // A short or long reply, and out-of-range values.
        assert_eq!(read(&bytes[..15]), Err(SysexError::Length));
        assert_eq!(
            read(&[0x7E, 0x10, 0x06, 0x02, 0x00, 0x20]),
            Err(SysexError::Length)
        );
        let mut long = bytes.to_vec();
        long.insert(16, 5);
        assert_eq!(read(&long), Err(SysexError::Length));
        let m = one.manufacturer;
        assert_eq!(
            Identity::new(m, 0x4000, 0, [0; 4]),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            Identity::new(m, 0, 0x4000, [0; 4]),
            Err(SysexError::OutOfRange)
        );
        assert_eq!(
            Identity::new(m, 0, 0, [0, 0x80, 0, 0]),
            Err(SysexError::OutOfRange)
        );
        let bad = InquiryMessage::Reply {
            device_id: 0x10,
            identity: Identity {
                family: 0x4000,
                ..one
            },
        };
        assert_eq!(bad.write(&mut out), Err(SysexError::OutOfRange));
    }

    #[test]
    fn responder_answers_all_call_and_own_id() {
        // REQ-INQ-003: a request to 7FH or to the own Device ID; the reply has the own Device ID
        // (AMB-M1-31).
        let id = Identity::new(ManufacturerId::from_midi1(&[0x43]).unwrap(), 1, 2, [0; 4]).unwrap();
        let mut out = [0u8; 20];
        for to in [0x7F, 0x05] {
            let n = respond(
                &InquiryMessage::Request { device_id: to },
                0x05,
                &id,
                &mut out,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                read(&out[..n]),
                Ok(Some(InquiryMessage::Reply {
                    device_id: 0x05,
                    identity: id
                }))
            );
        }
        let other = InquiryMessage::Request { device_id: 0x06 };
        assert_eq!(respond(&other, 0x05, &id, &mut out), Ok(None));
        let reply = InquiryMessage::Reply {
            device_id: 0x7F,
            identity: id,
        };
        assert_eq!(respond(&reply, 0x05, &id, &mut out), Ok(None));
        assert_eq!(
            respond(
                &InquiryMessage::Request { device_id: 0x7F },
                0x05,
                &id,
                &mut out[..4]
            ),
            Err(SysexError::BufferTooSmall { needed: 15 })
        );
    }
}
