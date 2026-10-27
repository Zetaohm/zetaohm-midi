//! A one-chunk Property Exchange Responder for small devices (DEC-PE-005, OPT-019).
//!
//! [`respond`] answers Inquiry: Get Property Data and Inquiry: Set Property Data whose request
//! is one Chunk (M2-103 7.2: Number of Chunks 1, Chunk 1), with the Resources of a
//! [`Resources`] implementation and a constant ResourceList. It keeps no transaction, no
//! subscription and no flow control state, and uses the ASCII encoding only, so it is a small
//! part of a firmware; [`crate::PeEngine`] is the full engine (multi-chunk, subscriptions,
//! encodings, the Initiator role).
//!
//! [`respond_flat`] is the same Responder with the flat Header Data reader of M2-103 7.1.1
//! (DEC-PE-006, REQ-PE-ONE-005): about 650 bytes of flash less; a firmware links only the form
//! that it calls.
//!
//! The reply has the Header Data `{"status":NNN}` (M2-103 7.4.1 Table 15):
//!
//! | Request | Status |
//! |---|---|
//! | A Get or Set in one Chunk that the application accepts | 200, the Property Data of a Get |
//! | Number of Chunks other than 1 (Chunk 1; later Chunks get no reply) | 413 |
//! | "mutualEncoding" other than "ASCII" | 415 |
//! | Header Data that is not valid JSON, no "resource" string, or a "resource" or "resId" longer than 36 characters | 400 |
//! | A Set with "setPartial" true | 405 |
//! | A Subscription message | 405 |
//! | An error of the application | its status |
//!
//! A ResourceList that this Responder gives must declare what it does: no "canSubscribe", no
//! "canPaginate", "canSet" "full" or "none", and only "ASCII" in "encodings" (the defaults).

use zmidi_ci::{CiDevice, Direction, Event, PeKind};

use crate::header::Status;
use crate::json::{for_each_flat_member, for_each_member_slice, JsonError, Scan};

/// The most bytes of "resource" and "resId" (M2-103 8.1, 8.2: 36 characters).
pub const MAX_ID: usize = 36;

/// The Resources of the application.
pub trait Resources {
    /// Writes the Property Data of `resource` (and `res_id`) into `out` and gives its length, or
    /// the reply status of a refusal (for example [`Status::NOT_FOUND`]).
    fn get(
        &mut self,
        resource: &[u8],
        res_id: Option<&[u8]>,
        out: &mut [u8],
    ) -> Result<usize, Status>;

    /// Takes the Property Data of a full Set of `resource`, or gives the reply status of a
    /// refusal.
    fn set(&mut self, resource: &[u8], res_id: Option<&[u8]>, data: &[u8]) -> Result<(), Status>;
}

/// The Properties of a request header that this Responder reads.
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
struct Request {
    resource: [u8; MAX_ID],
    resource_len: Option<usize>,
    res_id: [u8; MAX_ID],
    res_id_len: Option<usize>,
    ascii: bool,
    set_partial: bool,
    bad: bool,
}

/// Reads the request Header Data: with the flat reader of M2-103 7.1.1 when `flat`
/// (REQ-PE-ONE-005), else as complete JSON (REQ-PE-HDR-014).
#[inline(always)]
fn read_header(bytes: &[u8], flat: bool) -> Result<Request, JsonError> {
    let mut r = Request {
        resource: [0; MAX_ID],
        resource_len: None,
        res_id: [0; MAX_ID],
        res_id_len: None,
        ascii: true,
        set_partial: false,
        bad: false,
    };
    let member = |name: Option<&[u8]>, raw: &[u8]| -> Result<(), JsonError> {
        let Some(name) = name else {
            return Ok(());
        };
        if name == b"setPartial" {
            r.set_partial = raw == b"true";
            return Ok(());
        }
        // One string decoder call for the three string Properties.
        let mut e = [0u8; 8];
        let (dst, key): (&mut [u8], u8) = match name {
            b"resource" => (&mut r.resource, 0),
            b"resId" => (&mut r.res_id, 1),
            b"mutualEncoding" => (&mut e, 2),
            _ => return Ok(()),
        };
        let n = Scan::decode_into(raw, dst);
        match key {
            0 => {
                r.resource_len = n.filter(|n| *n > 0);
                r.bad |= r.resource_len.is_none();
            }
            1 => {
                r.res_id_len = n;
                r.bad |= n.is_none();
            }
            _ => r.ascii = n.is_some_and(|n| e[..n].eq_ignore_ascii_case(b"ASCII")),
        }
        Ok(())
    };
    if flat {
        for_each_flat_member(bytes, member)?;
    } else {
        for_each_member_slice(bytes, member)?;
    }
    Ok(r)
}

/// The Header Data `{"status":NNN}` of a reply.
fn status_header(status: Status) -> [u8; 14] {
    let mut h = *b"{\"status\":000}";
    let v = status.0.min(999);
    h[10] += (v / 100) as u8;
    h[11] += (v / 10 % 10) as u8;
    h[12] += (v % 10) as u8;
    h
}

/// Answers one event of `dev` (DEC-PE-005). For a Get, Set or Subscription Chunk from a peer,
/// it writes each Chunk of the reply into `out` and gives it to `send`, and gives the status;
/// `None` for every other event and for a later Chunk of a request. `body` holds the Property
/// Data of a Get reply. `list` is the ResourceList (a JSON array) that a Get of "ResourceList"
/// gets. Never panics.
#[allow(clippy::too_many_arguments)]
pub fn respond<const PEERS: usize, const PROFILES: usize, const REQUESTS: usize>(
    dev: &mut CiDevice<'_, PEERS, PROFILES, REQUESTS>,
    event: &Event<'_>,
    list: &[u8],
    resources: &mut impl Resources,
    body: &mut [u8],
    out: &mut [u8],
    send: impl FnMut(&[u8]),
) -> Option<Status> {
    respond_with(dev, event, list, resources, body, out, send, false)
}

/// [`respond`] with the flat Header Data reader of M2-103 7.1.1 (DEC-PE-006, REQ-PE-ONE-005):
/// the Header Data has no whitespace and each value is a string, a number, `true`, `false` or
/// `null`, as M2-103 requires of the sender. It does not check a number or a literal, so a
/// request that breaks the JSON syntax there can get the answer of the request in place of 400;
/// whitespace, an object or an array as a value gives 400. About 650 bytes of flash less than
/// [`respond`] (OPT-051). A firmware links only the form that it calls.
#[allow(clippy::too_many_arguments)]
pub fn respond_flat<const PEERS: usize, const PROFILES: usize, const REQUESTS: usize>(
    dev: &mut CiDevice<'_, PEERS, PROFILES, REQUESTS>,
    event: &Event<'_>,
    list: &[u8],
    resources: &mut impl Resources,
    body: &mut [u8],
    out: &mut [u8],
    send: impl FnMut(&[u8]),
) -> Option<Status> {
    respond_with(dev, event, list, resources, body, out, send, true)
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn respond_with<const PEERS: usize, const PROFILES: usize, const REQUESTS: usize>(
    dev: &mut CiDevice<'_, PEERS, PROFILES, REQUESTS>,
    event: &Event<'_>,
    list: &[u8],
    resources: &mut impl Resources,
    body: &mut [u8],
    out: &mut [u8],
    mut send: impl FnMut(&[u8]),
    flat: bool,
) -> Option<Status> {
    let Event::PeChunk { muid, kind, chunk } = *event else {
        return None;
    };
    let reply = match kind {
        PeKind::GetInquiry => PeKind::GetReply,
        PeKind::SetInquiry => PeKind::SetReply,
        PeKind::Subscription => PeKind::SubscriptionReply,
        _ => return None,
    };
    // zmidi-ci opens a request for each Get or Set Chunk; this Responder ends it at once, also
    // for a later Chunk of a request that got 413 at Chunk 1 (no reply to that Chunk).
    if kind != PeKind::Subscription {
        dev.pe_end(muid, chunk.request_id, Direction::Responder);
    }
    if chunk.number != 1 {
        return None;
    }
    let (status, len) = if kind == PeKind::Subscription {
        (Status::NOT_ALLOWED, 0)
    } else if chunk.total != 1 {
        (Status::PAYLOAD_TOO_LARGE, 0)
    } else {
        match read_header(chunk.header, flat) {
            Err(_) => (Status::BAD_REQUEST, 0),
            Ok(r) if r.bad || r.resource_len.is_none() => (Status::BAD_REQUEST, 0),
            Ok(r) if !r.ascii => (Status::UNSUPPORTED_MEDIA, 0),
            Ok(r) => {
                let resource = &r.resource[..r.resource_len.unwrap_or(0)];
                let res_id = r.res_id_len.map(|n| &r.res_id[..n]);
                match kind {
                    PeKind::GetInquiry if resource == b"ResourceList" => {
                        let n = list.len().min(body.len());
                        body[..n].copy_from_slice(&list[..n]);
                        (Status::OK, n)
                    }
                    PeKind::GetInquiry => match resources.get(resource, res_id, body) {
                        Ok(n) => (Status::OK, n.min(body.len())),
                        Err(s) => (s, 0),
                    },
                    _ if r.set_partial => (Status::NOT_ALLOWED, 0),
                    _ => match resources.set(resource, res_id, chunk.data) {
                        Ok(()) => (Status::OK, 0),
                        Err(s) => (s, 0),
                    },
                }
            }
        }
    };
    let header = status_header(status);
    let Ok(mut writer) = dev.pe_reply(muid, reply, chunk.request_id, &header, Some(len), None)
    else {
        return Some(status);
    };
    let mut at = 0;
    loop {
        let room = writer.data_room().min(len - at);
        match writer.write_next(&body[at..at + room], out) {
            Ok(n) => send(&out[..n]),
            Err(_) => break,
        }
        at += room;
        if at >= len {
            break;
        }
    }
    Some(status)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use proptest::prelude::*;
    use std::string::String;
    use std::vec::Vec;

    /// A request Header Data that follows M2-103 7.1.1: the Resource first, no whitespace, and
    /// each value a string, a number or a boolean; the other Properties in any order, with
    /// escapes in the strings of unknown Properties.
    fn conforming_header() -> impl Strategy<Value = String> {
        let alnum = "[A-Za-z0-9]{1,36}";
        let extra = prop_oneof![
            "[A-Za-z0-9_]{0,40}".prop_map(|v| std::format!(r#""resId":"{v}""#)),
            prop_oneof![
                Just("ASCII"),
                Just("ascii"),
                Just("Mcoded7"),
                Just("zlib+Mcoded7")
            ]
            .prop_map(|v| std::format!(r#""mutualEncoding":"{v}""#)),
            any::<bool>().prop_map(|v| std::format!(r#""setPartial":{v}"#)),
            (0u32..100_000).prop_map(|v| std::format!(r#""offset":{v}"#)),
            "-?(0|[1-9][0-9]{0,4})(\\.[0-9]{1,3})?([eE][+-]?[0-9]{1,2})?"
                .prop_map(|v| std::format!(r#""x":{v}"#)),
            prop_oneof![
                Just(r#"a\"b"#),
                Just(r#"c\\d"#),
                Just(r#"\u00e9"#),
                Just(r#"\/"#),
                Just("")
            ]
            .prop_map(|v| std::format!(r#""note":"{v}""#)),
            Just(r#""y":null"#.into()),
        ];
        (alnum, proptest::collection::vec(extra, 0..6)).prop_map(|(r, rest)| {
            let mut h = std::format!(r#"{{"resource":"{r}""#);
            for p in rest {
                h.push(',');
                h.push_str(&p);
            }
            h.push('}');
            h
        })
    }

    #[test]
    fn prop_flat_reader_equals_full_reader_on_conforming_headers() {
        // REQ-PE-ONE-005: for every Header Data that follows M2-103 7.1.1, the flat reader gives
        // the same request as the complete JSON reader.
        proptest!(|(h in conforming_header())| {
            let full = read_header(h.as_bytes(), false);
            prop_assert!(full.is_ok(), "{h}");
            prop_assert_eq!(read_header(h.as_bytes(), true), full, "{}", h);
        });
    }

    #[test]
    fn flat_reader_refuses_whitespace_objects_and_arrays() {
        // REQ-PE-ONE-005: the forms that M2-103 7.1.1 forbids give a syntax error (400);
        // the complete reader takes whitespace.
        let bad: [&[u8]; 6] = [
            br#"{"resource": "A"}"#,
            br#" {"resource":"A"}"#,
            br#"{"resource":"A","x":{"a":1}}"#,
            br#"{"resource":"A","x":[1]}"#,
            br#"{"resource":"A","x":}"#,
            br#"{"resource":"A"}x"#,
        ];
        for h in bad {
            assert!(
                read_header(h, true).is_err(),
                "{}",
                String::from_utf8_lossy(h)
            );
        }
        assert!(read_header(br#"{"resource": "A"}"#, false).is_ok());
        let v: Vec<u8> = br#"{"resource":"A","x":1}"#.to_vec();
        assert_eq!(read_header(&v, true), read_header(&v, false));
    }
}
