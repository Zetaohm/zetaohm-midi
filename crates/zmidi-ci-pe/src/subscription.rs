//! Subscriptions (M2-103-UM v1.2 section 11; M2-101-UM 8.11).
//!
//! - [`SubscriptionTable`] keeps the subscriptions of this Device in a table of `N` entries
//!   (DEC-002). Each entry keeps the peer MUID, the role of this Device, the "subscribeId" and
//!   the "start" values "resource", "resId", "mutualEncoding", "offset" and "limit"
//!   (REQ-PE-SUB-005), so that a "notify" can be answered with the same Get (REQ-PE-SUB-014).
//! - The table gives the reply status of each received Subscription message (REQ-PE-SUB-003,
//!   -004, -007, -015) and the headers of the messages to send. Sending and receiving are the
//!   Transaction module's.

use crate::header::{Command, EncodingName, EndedBy, Header, HeaderStr, Status};
use crate::resource_list::ResourceCaps;
use zmidi_ci::Muid;

const ID_MAX: usize = 16;
/// "resource" and "resId": at most 36 characters, ASCII (M2-103 7.2 Table 13, 13.1.1 Table 68),
/// so 36 bytes. A received value of more than 36 bytes has other characters: the responder
/// replies 400 (REQ-PE-SUB-007, OPT-005).
const NAME_MAX: usize = 36;

/// Error of the subscription rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SubscriptionError {
    /// The table is full (REQ-PE-SUB-005).
    #[error("subscription table full")]
    Full,
    /// The reply to a "start" has no "subscribeId" (REQ-PE-SUB-006).
    #[error("no subscribeId in the reply")]
    NoSubscribeId,
    /// The reply to a "start" has no status from 200 to 299 (REQ-PE-SUB-006).
    #[error("start not accepted")]
    NotSuccess,
    /// A command or Property Data that the role does not permit (REQ-PE-SUB-003, -013, -017,
    /// -021).
    #[error("not permitted for this role")]
    NotPermitted,
    /// A header value is longer than the entry buffers, or a header is missing a Property.
    #[error("invalid start header")]
    Invalid,
}

/// The role of this Device in a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// This Device sent the "start".
    Initiator,
    /// This Device accepted the "start".
    Responder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Text<const N: usize> {
    buf: [u8; N],
    len: u8,
}

impl<const N: usize> Text<N> {
    const EMPTY: Self = Text {
        buf: [0; N],
        len: 0,
    };

    fn from_str(s: &HeaderStr<'_>) -> Option<Self> {
        let mut t = Self::EMPTY;
        let (v, cut) = s.decode_into(&mut t.buf);
        if cut {
            return None;
        }
        t.len = v.len() as u8;
        Some(t)
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }
}

/// One subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriptionEntry {
    peer: Muid,
    role: Role,
    id: Text<ID_MAX>,
    resource: Text<NAME_MAX>,
    res_id: Option<Text<NAME_MAX>>,
    mutual_encoding: Option<EncodingName>,
    offset: Option<u32>,
    limit: Option<u32>,
}

impl SubscriptionEntry {
    /// The MUID of the other Device.
    pub fn peer(&self) -> Muid {
        self.peer
    }

    /// The role of this Device.
    pub fn role(&self) -> Role {
        self.role
    }

    /// The "subscribeId".
    pub fn subscribe_id(&self) -> &str {
        self.id.as_str()
    }

    /// The "resource" of the "start".
    pub fn resource(&self) -> &str {
        self.resource.as_str()
    }

    /// The "resId" of the "start".
    pub fn res_id(&self) -> Option<&str> {
        self.res_id.as_ref().map(|t| t.as_str())
    }

    fn from_start(
        peer: Muid,
        role: Role,
        id: &HeaderStr<'_>,
        start: &Header<'_>,
    ) -> Result<Self, SubscriptionError> {
        let res_id = match &start.res_id {
            Some(r) => Some(Text::from_str(r).ok_or(SubscriptionError::Invalid)?),
            None => None,
        };
        Ok(SubscriptionEntry {
            peer,
            role,
            id: Text::from_str(id).ok_or(SubscriptionError::Invalid)?,
            resource: Text::from_str(start.resource.as_ref().ok_or(SubscriptionError::Invalid)?)
                .ok_or(SubscriptionError::Invalid)?,
            res_id,
            mutual_encoding: start.mutual_encoding,
            offset: start.offset,
            limit: start.limit,
        })
    }

    /// The header of an update ("partial", "full" or "notify") that the responder sends
    /// (REQ-PE-SUB-008, -013).
    pub fn update_header(&self, command: Command) -> Result<Header<'_>, SubscriptionError> {
        if self.role != Role::Responder
            || !matches!(command, Command::Partial | Command::Full | Command::Notify)
        {
            return Err(SubscriptionError::NotPermitted);
        }
        Ok(Header {
            command: Some(command),
            subscribe_id: Some(HeaderStr::Text(self.subscribe_id())),
            resource: Some(HeaderStr::Text(self.resource())),
            res_id: self.res_id().map(HeaderStr::Text),
            ..Default::default()
        })
    }

    /// The header of an "end" with "endedBy" by the role of this Device (REQ-PE-SUB-004, -017).
    pub fn end_header(&self) -> Header<'_> {
        Header {
            command: Some(Command::End),
            subscribe_id: Some(HeaderStr::Text(self.subscribe_id())),
            ended_by: Some(match self.role {
                Role::Initiator => EndedBy::Initiator,
                Role::Responder => EndedBy::Responder,
            }),
            ..Default::default()
        }
    }

    /// The header of the Inquiry: Get Property Data that the initiator sends after a "notify",
    /// or after an update that it can not apply: the "start" values and no other request
    /// Property (REQ-PE-SUB-014, -016).
    pub fn get_header(&self) -> Header<'_> {
        Header {
            resource: Some(HeaderStr::Text(self.resource())),
            res_id: self.res_id().map(HeaderStr::Text),
            mutual_encoding: self.mutual_encoding,
            offset: self.offset,
            limit: self.limit,
            ..Default::default()
        }
    }
}

/// The header of a "start" Subscription message: the request Properties of a Get for the same
/// Resource and no Property Data (REQ-PE-SUB-006). `get` holds "resource" and the optional
/// "resId", "mutualEncoding", "offset" and "limit".
pub fn start_header<'a>(get: &Header<'a>) -> Header<'a> {
    Header {
        command: Some(Command::Start),
        resource: get.resource,
        res_id: get.res_id,
        mutual_encoding: get.mutual_encoding,
        offset: get.offset,
        limit: get.limit,
        ..Default::default()
    }
}

/// Checks the Property Data rule of a Subscription message that `role` sends: Property Data
/// only in "partial" and "full" from the responder (REQ-PE-SUB-003, -013, -017, -021).
pub fn check_send(role: Role, command: Command, has_data: bool) -> Result<(), SubscriptionError> {
    let ok = match (role, command) {
        (Role::Initiator, Command::Start | Command::End) => !has_data,
        (Role::Responder, Command::Partial | Command::Full) => true,
        (Role::Responder, Command::Notify | Command::End) => !has_data,
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(SubscriptionError::NotPermitted)
    }
}

/// The update command for a change (REQ-PE-SUB-011, -012): "notify" when the data does not
/// fit in one Chunk for the receiver, else "full" for a Simple Property Resource, else the
/// command that the caller prefers.
pub fn update_command(
    data_len: usize,
    chunk_room: usize,
    simple: bool,
    preferred: Command,
) -> Command {
    if data_len > chunk_room {
        Command::Notify
    } else if simple {
        Command::Full
    } else {
        preferred
    }
}

/// What a received Subscription message is, after [`SubscriptionTable::receive`].
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionInput {
    /// A "partial", "full" or "notify" for the entry at this index (REQ-PE-SUB-009, -010,
    /// -013, -020).
    Update { index: usize, command: Command },
    /// An "end": the entry is removed (REQ-PE-SUB-004, -018).
    Ended(SubscriptionEntry),
    /// An "end" for an unknown "subscribeId": reply 200 and nothing else (REQ-PE-SUB-015).
    EndUnknown,
}

/// The subscriptions of this Device, at most `N` (REQ-PE-SUB-005).
#[derive(Debug, Clone)]
pub struct SubscriptionTable<const N: usize> {
    entries: [Option<SubscriptionEntry>; N],
    // True for an entry that an Invalidate MUID ended: it is not active and waits for
    // `take_ended` in its place, so the table needs no second store (OPT-004).
    ended: [bool; N],
    next_id: u32,
}

#[allow(clippy::new_without_default)]
impl<const N: usize> SubscriptionTable<N> {
    /// An empty table.
    pub const fn new() -> Self {
        SubscriptionTable {
            entries: [None; N],
            ended: [false; N],
            next_id: 0,
        }
    }

    /// The active subscriptions.
    pub fn entries(&self) -> impl Iterator<Item = &SubscriptionEntry> {
        (0..N).filter_map(|i| self.active(i))
    }

    /// The active entry at `index` (from [`SubscriptionInput::Update`]).
    pub fn get(&self, index: usize) -> Option<&SubscriptionEntry> {
        self.active(index)
    }

    fn active(&self, index: usize) -> Option<&SubscriptionEntry> {
        if *self.ended.get(index)? {
            return None;
        }
        self.entries.get(index)?.as_ref()
    }

    /// A free place for a new entry. An ended entry keeps its place until the caller takes it,
    /// so that the caller gets each one (REQ-PE-SUB-019); until then the table can be full
    /// (REQ-PE-SUB-005).
    fn free(&self) -> Option<usize> {
        self.entries.iter().position(Option::is_none)
    }

    /// Writes a new active entry at `i` (a place from [`Self::free`]).
    fn put(&mut self, i: usize, e: SubscriptionEntry) {
        self.entries[i] = Some(e);
        self.ended[i] = false;
    }

    fn find(&self, peer: Muid, role: Role, id: &HeaderStr<'_>) -> Option<usize> {
        (0..N).find(|&i| {
            matches!(self.active(i), Some(e) if e.peer == peer && e.role == role && id.eq_str(e.subscribe_id()))
        })
    }

    /// Initiator: an error while the table is full, so that no "start" is sent
    /// (REQ-PE-SUB-005).
    pub fn can_start(&self) -> Result<(), SubscriptionError> {
        self.free().map(|_| ()).ok_or(SubscriptionError::Full)
    }

    /// Initiator: adds the subscription when the reply to its "start" has a status from 200 to
    /// 299 and a "subscribeId" (REQ-PE-SUB-006). Gives the index of the new entry.
    pub fn add_started(
        &mut self,
        peer: Muid,
        start: &Header<'_>,
        reply: &Header<'_>,
    ) -> Result<usize, SubscriptionError> {
        let id = reply
            .subscribe_id
            .as_ref()
            .ok_or(SubscriptionError::NoSubscribeId)?;
        let i = self.free().ok_or(SubscriptionError::Full)?;
        let e = SubscriptionEntry::from_start(peer, Role::Initiator, id, start)?;
        self.put(i, e);
        Ok(i)
    }

    /// Responder: a received "start". Gives the index of the new entry, with a "subscribeId"
    /// that no other entry uses, or the reply status: 405 without "canSubscribe" or with no
    /// table (`N` = 0: this Device has no subscriptions, M2-103 11), 343 when the table is full,
    /// 400 for a start header that the entry can not keep (REQ-PE-SUB-007, REQ-PE-SUB-022).
    pub fn accept_start(
        &mut self,
        peer: Muid,
        start: &Header<'_>,
        caps: &ResourceCaps,
    ) -> Result<usize, Status> {
        // With N = 0 a 343 ("retry") would make the Initiator try again with no end.
        if N == 0 || !caps.can_subscribe {
            return Err(Status::NOT_ALLOWED);
        }
        let i = self.free().ok_or(Status::TOO_MANY_REQUESTS)?;
        // A new id: "s" and a counter in base 36, at most 8 characters (REQ-PE-SUB-001).
        let mut id = [0u8; 8];
        let n = loop {
            let mut v = self.next_id;
            self.next_id = (self.next_id + 1) % 2_176_782_336; // 36^6: at most 6 digits
            let mut digits = [0u8; 6];
            let mut k = 0;
            loop {
                let d = (v % 36) as u8;
                digits[k] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
                k += 1;
                v /= 36;
                if v == 0 {
                    break;
                }
            }
            id[0] = b's';
            for j in 0..k {
                id[1 + j] = digits[k - 1 - j];
            }
            let text = core::str::from_utf8(&id[..1 + k]).unwrap_or("s");
            if !self.entries().any(|e| e.subscribe_id() == text) {
                break 1 + k;
            }
        };
        let text = core::str::from_utf8(&id[..n]).unwrap_or("s");
        let e = SubscriptionEntry::from_start(peer, Role::Responder, &HeaderStr::Text(text), start)
            .map_err(|_| Status::BAD_REQUEST)?;
        self.put(i, e);
        Ok(i)
    }

    /// A received "partial", "full", "notify" or "end" from `peer` (REQ-PE-SUB-003, -004,
    /// -015, -020). An error is the reply status: 400 for an unknown "subscribeId" or a
    /// command that the sender's role does not permit. A "start" is not handled here.
    pub fn receive(
        &mut self,
        peer: Muid,
        header: &Header<'_>,
    ) -> Result<SubscriptionInput, Status> {
        let id = header.subscribe_id.as_ref().ok_or(Status::BAD_REQUEST)?;
        match header.command {
            Some(c @ (Command::Partial | Command::Full | Command::Notify)) => {
                // Only the responder sends updates: look in the entries where this Device is
                // the initiator.
                let index = self
                    .find(peer, Role::Initiator, id)
                    .ok_or(Status::BAD_REQUEST)?;
                Ok(SubscriptionInput::Update { index, command: c })
            }
            Some(Command::End) => {
                // The sender has the role that "endedBy" gives; this Device has the other one.
                let order = match header.ended_by {
                    Some(EndedBy::Initiator) => [Some(Role::Responder), None],
                    Some(EndedBy::Responder) => [Some(Role::Initiator), None],
                    None => [Some(Role::Responder), Some(Role::Initiator)],
                };
                for role in order.into_iter().flatten() {
                    if let Some(i) = self.find(peer, role, id) {
                        let e = self.entries[i].take();
                        return Ok(
                            e.map_or(SubscriptionInput::EndUnknown, SubscriptionInput::Ended)
                        );
                    }
                }
                Ok(SubscriptionInput::EndUnknown)
            }
            _ => Err(Status::BAD_REQUEST),
        }
    }

    /// Removes the active entry at `index` after this Device sent its "end" (REQ-PE-SUB-018).
    pub fn remove(&mut self, index: usize) -> Option<SubscriptionEntry> {
        self.active(index)?;
        self.entries.get_mut(index).and_then(Option::take)
    }

    /// The indexes of the entries in which this Device is the responder for `resource` and
    /// `res_id`; updates go only to these (REQ-PE-SUB-008, REQ-PE-LINK-009).
    pub fn updates_for<'s>(
        &'s self,
        resource: &'s str,
        res_id: Option<&'s str>,
    ) -> impl Iterator<Item = usize> + 's {
        (0..N).filter(move |&i| {
            self.active(i).is_some_and(|e| {
                e.role == Role::Responder && e.resource() == resource && e.res_id() == res_id
            })
        })
    }

    /// Ends each active entry with `peer`, with no "end" message (an Invalidate MUID of the
    /// peer, REQ-PE-SUB-019). The ended entries wait for [`Self::take_ended`]. Gives their
    /// number.
    pub fn end_peer(&mut self, peer: Muid) -> usize {
        self.end_where(|e| e.peer == peer)
    }

    /// Ends every active entry (an Invalidate MUID of this Device, REQ-PE-SUB-019). Gives their
    /// number.
    pub fn end_all(&mut self) -> usize {
        self.end_where(|_| true)
    }

    fn end_where(&mut self, f: impl Fn(&SubscriptionEntry) -> bool) -> usize {
        let mut n = 0;
        for i in 0..N {
            if self.active(i).is_some_and(&f) {
                self.ended[i] = true;
                n += 1;
            }
        }
        n
    }

    /// The next entry that [`Self::end_peer`] or [`Self::end_all`] ended, in table order; its
    /// place becomes free.
    pub fn take_ended(&mut self) -> Option<SubscriptionEntry> {
        let i = self.ended.iter().position(|&e| e)?;
        self.ended[i] = false;
        self.entries[i].take()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::header::HeaderKind;
    use std::vec::Vec;

    fn peer(v: u32) -> Muid {
        Muid::new(v).unwrap()
    }

    fn get(resource: &'static str) -> Header<'static> {
        Header {
            resource: Some(HeaderStr::Text(resource)),
            res_id: Some(HeaderStr::Text("r1")),
            offset: Some(0),
            limit: Some(10),
            ..Default::default()
        }
    }

    fn reply(id: Option<&'static str>) -> Header<'static> {
        Header {
            status: Some(Status::OK),
            subscribe_id: id.map(HeaderStr::Text),
            ..Default::default()
        }
    }

    fn msg(command: Command, id: &'static str, ended_by: Option<EndedBy>) -> Header<'static> {
        Header {
            command: Some(command),
            subscribe_id: Some(HeaderStr::Text(id)),
            ended_by,
            ..Default::default()
        }
    }

    #[test]
    fn start_header_and_initiator_entry() {
        // M2-103 Table 47; REQ-PE-SUB-005, -006, -014.
        let s = start_header(&get("ProgramList"));
        let mut buf = [0u8; 128];
        let n = s.write(HeaderKind::Subscription, &mut buf).unwrap();
        assert_eq!(
            &buf[..n],
            br#"{"command":"start","resource":"ProgramList","resId":"r1","offset":0,"limit":10}"#
        );
        let mut t = SubscriptionTable::<2>::new();
        assert_eq!(
            t.add_started(peer(5), &s, &reply(None)),
            Err(SubscriptionError::NoSubscribeId)
        );
        let i = t
            .add_started(peer(5), &s, &reply(Some("sub138047")))
            .unwrap();
        let e = t.get(i).unwrap();
        assert_eq!(
            (e.subscribe_id(), e.resource(), e.res_id(), e.role()),
            ("sub138047", "ProgramList", Some("r1"), Role::Initiator)
        );
        let g = e.get_header();
        let n = g.write(HeaderKind::Inquiry, &mut buf).unwrap();
        assert_eq!(
            &buf[..n],
            br#"{"resource":"ProgramList","resId":"r1","offset":0,"limit":10}"#
        );
        t.add_started(peer(6), &s, &reply(Some("b"))).unwrap();
        assert_eq!(t.can_start(), Err(SubscriptionError::Full));
    }

    #[test]
    fn responder_with_no_table_refuses_start_with_405() {
        // REQ-PE-SUB-022: a table of 0 entries refuses every "start" with 405, not 343.
        let mut t = SubscriptionTable::<0>::new();
        let caps = ResourceCaps {
            can_subscribe: true,
            ..ResourceCaps::default()
        };
        let s = start_header(&get("X"));
        assert_eq!(t.accept_start(peer(1), &s, &caps), Err(Status::NOT_ALLOWED));
    }

    #[test]
    fn responder_accepts_start() {
        // REQ-PE-SUB-007.
        let mut t = SubscriptionTable::<2>::new();
        let mut caps = ResourceCaps::default();
        let s = start_header(&get("X"));
        assert_eq!(t.accept_start(peer(1), &s, &caps), Err(Status::NOT_ALLOWED));
        caps.can_subscribe = true;
        let a = t.accept_start(peer(1), &s, &caps).unwrap();
        let b = t.accept_start(peer(1), &s, &caps).unwrap();
        assert_ne!(
            t.get(a).unwrap().subscribe_id(),
            t.get(b).unwrap().subscribe_id(),
            "unique ids"
        );
        assert!(t.get(a).unwrap().subscribe_id().len() <= 8);
        assert_eq!(
            t.accept_start(peer(1), &s, &caps),
            Err(Status::TOO_MANY_REQUESTS)
        );
        let ids: Vec<usize> = t.updates_for("X", Some("r1")).collect();
        assert_eq!(ids, [a, b], "REQ-PE-SUB-008");
        // A "resId" of 36 ASCII characters is kept; one longer than 36 bytes (36 characters
        // that are not ASCII) gets 400 (OPT-005).
        let mut t = SubscriptionTable::<2>::new();
        let long = "r".repeat(36);
        let mut g = get("X");
        g.res_id = Some(HeaderStr::Text(&long));
        let i = t.accept_start(peer(1), &start_header(&g), &caps).unwrap();
        assert_eq!(t.get(i).unwrap().res_id(), Some(long.as_str()));
        let wide = "é".repeat(36);
        g.res_id = Some(HeaderStr::Text(&wide));
        assert_eq!(
            t.accept_start(peer(1), &start_header(&g), &caps),
            Err(Status::BAD_REQUEST)
        );
        assert_eq!(
            t.updates_for("X", None).count(),
            0,
            "REQ-PE-LINK-009: same resId only"
        );
    }

    #[test]
    fn receive_updates_and_ends() {
        // REQ-PE-SUB-003, -004, -015, -018, -020.
        let mut t = SubscriptionTable::<4>::new();
        let s = start_header(&get("X"));
        let i = t.add_started(peer(5), &s, &reply(Some("abc"))).unwrap();
        assert_eq!(
            t.receive(peer(5), &msg(Command::Full, "abc", None)),
            Ok(SubscriptionInput::Update {
                index: i,
                command: Command::Full
            })
        );
        assert_eq!(
            t.receive(peer(5), &msg(Command::Partial, "zzz", None)),
            Err(Status::BAD_REQUEST),
            "unknown id: 400"
        );
        assert_eq!(
            t.receive(peer(6), &msg(Command::Notify, "abc", None)),
            Err(Status::BAD_REQUEST),
            "other peer"
        );
        assert_eq!(
            t.receive(peer(5), &msg(Command::End, "zzz", None)),
            Ok(SubscriptionInput::EndUnknown),
            "unknown end: 200"
        );
        // An "end" from the responder (this Device is the initiator).
        assert!(
            matches!(t.receive(peer(5), &msg(Command::End, "abc", Some(EndedBy::Responder))), Ok(SubscriptionInput::Ended(e)) if e.subscribe_id() == "abc")
        );
        assert_eq!(t.entries().count(), 0);
        // Responder side: an "end" from the initiator; no endedBy also works.
        let caps = ResourceCaps {
            can_subscribe: true,
            ..Default::default()
        };
        let r = t.accept_start(peer(7), &s, &caps).unwrap();
        let id = t.get(r).unwrap().subscribe_id();
        let id: &'static str =
            std::boxed::Box::leak(std::string::String::from(id).into_boxed_str());
        assert_eq!(
            t.receive(peer(7), &msg(Command::Full, id, None)),
            Err(Status::BAD_REQUEST),
            "a responder gets no updates"
        );
        assert!(matches!(
            t.receive(peer(7), &msg(Command::End, id, None)),
            Ok(SubscriptionInput::Ended(_))
        ));
    }

    #[test]
    fn headers_and_data_rules() {
        // REQ-PE-SUB-003, -004, -013, -017, -021, -011, -012.
        let mut t = SubscriptionTable::<2>::new();
        let caps = ResourceCaps {
            can_subscribe: true,
            ..Default::default()
        };
        let r = t
            .accept_start(peer(1), &start_header(&get("X")), &caps)
            .unwrap();
        let e = *t.get(r).unwrap();
        let mut buf = [0u8; 128];
        let n = e
            .end_header()
            .write(HeaderKind::Subscription, &mut buf)
            .unwrap();
        assert!(core::str::from_utf8(&buf[..n])
            .unwrap()
            .ends_with(r#""endedBy":"responder"}"#));
        assert!(e.update_header(Command::Full).is_ok());
        assert_eq!(
            e.update_header(Command::Start),
            Err(SubscriptionError::NotPermitted)
        );
        assert_eq!(
            check_send(Role::Initiator, Command::Start, true),
            Err(SubscriptionError::NotPermitted)
        );
        assert_eq!(
            check_send(Role::Initiator, Command::Full, false),
            Err(SubscriptionError::NotPermitted)
        );
        assert_eq!(
            check_send(Role::Responder, Command::Notify, true),
            Err(SubscriptionError::NotPermitted)
        );
        assert_eq!(
            check_send(Role::Responder, Command::End, true),
            Err(SubscriptionError::NotPermitted)
        );
        assert_eq!(check_send(Role::Responder, Command::Partial, true), Ok(()));
        assert_eq!(
            update_command(600, 400, false, Command::Partial),
            Command::Notify
        );
        assert_eq!(
            update_command(10, 400, true, Command::Partial),
            Command::Full
        );
        assert_eq!(
            update_command(10, 400, false, Command::Partial),
            Command::Partial
        );
    }

    #[test]
    fn invalidate_removes_entries() {
        // REQ-PE-SUB-019.
        let mut t = SubscriptionTable::<4>::new();
        let s = start_header(&get("X"));
        t.add_started(peer(5), &s, &reply(Some("a"))).unwrap();
        t.add_started(peer(6), &s, &reply(Some("b"))).unwrap();
        assert_eq!(t.end_peer(peer(5)), 1);
        assert_eq!(t.entries().count(), 1);
        assert_eq!(t.end_all(), 1);
        assert_eq!(t.entries().count(), 0);
        let removed: Vec<_> = core::iter::from_fn(|| t.take_ended().map(|e| e.peer())).collect();
        assert_eq!(removed, [peer(5), peer(6)]);
        assert!(t.take_ended().is_none());
        // An ended entry is not active, and it keeps its place until the caller takes it: the
        // caller gets each ended entry, and the table is full until then (REQ-PE-SUB-005).
        let mut t = SubscriptionTable::<1>::new();
        t.add_started(peer(5), &s, &reply(Some("a"))).unwrap();
        assert_eq!(t.end_peer(peer(5)), 1);
        assert!(t.get(0).is_none());
        assert_eq!(t.can_start(), Err(SubscriptionError::Full));
        assert_eq!(t.take_ended().unwrap().peer(), peer(5));
        let i = t.add_started(peer(6), &s, &reply(Some("b"))).unwrap();
        assert_eq!(t.get(i).unwrap().peer(), peer(6));
    }
}
