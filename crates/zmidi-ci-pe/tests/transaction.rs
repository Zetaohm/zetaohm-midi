//! Integration tests of the Property Exchange transaction engine (M2-103-UM v1.2): two zmidi-ci
//! Devices, each with an engine, exchange messages through caller buffers, as an application
//! does. Device A is the initiator, Device B the responder, unless a test says otherwise.
// An Initiator engine against a Responder engine: the Initiator role needs the feature
// `initiator` (DEC-CI-001 of zmidi-ci).
#![cfg(feature = "initiator")]

use zmidi_ci::{
    Ack, AckStatus, Categories, ChunkEnd, CiDevice, Details, DeviceConfig, DeviceIdentity,
    Direction, Framing, ManufacturerId, Muid, Nak, NakStatus, PeChunk, PeKind, PiCapabilities,
};
use zmidi_ci_pe::resource_list::CanSet;
use zmidi_ci_pe::{
    schema_ref, CloseReason, Command, EncodingName, EngineConfig, Header, HeaderKind, HeaderStr,
    Mcoded7Encoder, Outgoing, PartialSetReader, PeEngine, PeEvent, PeHost, ResourceCaps,
    ResourceEntry, Status, SubscriptionError, TransactionError, TxnKind,
};

type Dev = CiDevice<'static, 4, 2, 8>;
type Eng = PeEngine<'static, 4, 128, 4, 4>;

fn config(m: u8, max_sysex: u32) -> DeviceConfig<'static> {
    DeviceConfig {
        identity: DeviceIdentity {
            manufacturer: ManufacturerId::OneByte(m),
            family: 1,
            model: 2,
            revision: [0, 0, 0, 1],
        },
        categories: Categories {
            property_exchange: true,
            ..Default::default()
        },
        max_sysex_size: max_sysex,
        function_block: Some(1),
        first_group: 0,
        group_count: 1,
        channels_in_use: 0,
        product_instance_id: None,
        framing: Framing::Bare,
        process_inquiry: PiCapabilities {
            midi_message_report: false,
        },
    }
}

fn rng(v: u32) -> impl FnMut() -> u32 {
    move || v
}

fn caps() -> ResourceCaps {
    ResourceCaps::default()
}

/// The Resource table of Device B.
fn resources() -> &'static [ResourceEntry<'static>] {
    let mut info = ResourceEntry::new("DeviceInfo");
    info.caps.can_subscribe = false;
    let mut patch = ResourceEntry::new("X-Patch");
    patch.caps = ResourceCaps {
        can_set: CanSet::Full,
        can_subscribe: true,
        ..caps()
    };
    patch.caps.encodings.mcoded7 = true;
    let mut fc = ResourceEntry::new("X-Flow");
    fc.caps = ResourceCaps {
        can_set: CanSet::Full,
        fc_on_get: true,
        fc_on_set: true,
        ..caps()
    };
    fc.caps.encodings.mcoded7 = true;
    let mut list = ResourceEntry::new("ProgramList");
    list.caps.can_paginate = true;
    let mut z = ResourceEntry::new("X-Zip");
    z.caps.can_set = CanSet::Full;
    z.caps.encodings.zlib_mcoded7 = true;
    let mut bin = ResourceEntry::new("X-Bin");
    bin.caps.can_set = CanSet::Full;
    bin.media_types = &["application/octet-stream"];
    Box::leak(Box::new([info, patch, fc, list, z, bin]))
}

struct Side {
    dev: Dev,
    eng: Eng,
    now: u64,
}

/// An owned copy of an engine event, for the asserts.
#[derive(Debug, Clone, PartialEq)]
enum Ev {
    Msg {
        rid: u8,
        role: Direction,
        kind: TxnKind,
        status: Option<u16>,
        header: bool,
        resource: Option<usize>,
        data: Vec<u8>,
        end: ChunkEnd,
        sub: Option<usize>,
        started: Option<Result<usize, SubscriptionError>>,
    },
    Ready(u8),
    Resend(u8, u16),
    Closed(u8, Direction, CloseReason),
    SubStarted(usize),
    SubEnded(String),
    Invalidated(usize),
}

fn own(e: PeEvent<'_, '_>) -> Ev {
    match e {
        PeEvent::Message(m) => Ev::Msg {
            rid: m.request_id,
            role: m.role,
            kind: m.kind,
            status: m.header.and_then(|h| h.status).map(|s| s.0),
            header: m.header.is_some(),
            resource: m.resource,
            data: m.data.to_vec(),
            end: m.end,
            sub: m.subscription,
            started: m.started,
        },
        PeEvent::Ready { request_id, .. } => Ev::Ready(request_id),
        PeEvent::Resend {
            request_id, chunk, ..
        } => Ev::Resend(request_id, chunk),
        PeEvent::Closed {
            request_id,
            role,
            reason,
            ..
        } => Ev::Closed(request_id, role, reason),
        PeEvent::SubscriptionStarted { index } => Ev::SubStarted(index),
        PeEvent::SubscriptionEnded(e) => Ev::SubEnded(e.subscribe_id().to_string()),
        PeEvent::Invalidated { ended, .. } => Ev::Invalidated(ended),
    }
}

impl Side {
    fn new(m: u8, seed: u32, max_sysex: u32, zlib_window: usize) -> Self {
        Side {
            dev: CiDevice::new(config(m, max_sysex), &mut rng(seed)).unwrap(),
            eng: PeEngine::new(EngineConfig {
                resources: resources(),
                zlib_window,
            }),
            now: 1_000,
        }
    }

    fn muid(&self) -> Muid {
        self.dev.muid()
    }

    /// Delivers one message: the Device event goes to the engine. Gives the engine event and
    /// every message written (Device replies, then the engine's pending messages).
    fn rx(&mut self, msg: &[u8]) -> (Option<Ev>, Vec<Vec<u8>>) {
        let mut out = Vec::new();
        let mut buf = [0u8; 600];
        let mut r = self.dev.receive(0, msg, self.now * 1000, &mut rng(7));
        let ev = r.event();
        while let Some(x) = r.write_next(&mut buf) {
            let (_, n) = x.unwrap();
            out.push(buf[..n].to_vec());
        }
        let mut scratch = [0u8; 600];
        let e = match ev {
            Some(ev) => self
                .eng
                .receive(&mut self.dev, &ev, self.now, &mut scratch)
                .unwrap()
                .map(own),
            None => None,
        };
        out.extend(self.pending());
        (e, out)
    }

    fn pending(&mut self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut buf = [0u8; 600];
        while let Some(r) = self.eng.write_pending(&mut self.dev, self.now, &mut buf) {
            out.push(buf[..r.unwrap()].to_vec());
        }
        out
    }

    fn poll(&mut self) -> (Option<Ev>, Vec<Vec<u8>>) {
        let e = self.eng.poll(&mut self.dev, self.now).map(own);
        (e, self.pending())
    }

    #[allow(clippy::too_many_arguments)]
    fn send(&mut self, peer: Muid, rid: u8, role: Direction, part: &[u8], last: bool) -> Vec<u8> {
        let mut buf = [0u8; 600];
        let n = self
            .eng
            .send(
                &mut self.dev,
                peer,
                rid,
                role,
                part,
                last,
                self.now,
                &mut buf,
            )
            .unwrap();
        buf[..n].to_vec()
    }
}

/// A host that reports `.1` as the Number of Simultaneous Requests of every peer.
struct Limited<'d>(&'d mut Dev, u8);

impl PeHost for Limited<'_> {
    fn muid(&self) -> Muid {
        PeHost::muid(&*self.0)
    }
    fn framing(&self) -> Framing {
        self.0.framing()
    }
    fn peer_max_sysex(&self, peer: Muid) -> u32 {
        self.0.peer_max_sysex(peer)
    }
    fn peer_simultaneous(&self, _: Muid) -> Option<u8> {
        Some(self.1)
    }
    fn allocate(&mut self, peer: Muid, kind: PeKind) -> Result<u8, zmidi_ci::DeviceError> {
        self.0.allocate(peer, kind)
    }
    fn end(&mut self, peer: Muid, rid: u8, d: Direction) {
        PeHost::end(self.0, peer, rid, d)
    }
    fn write_ack(
        &self,
        p: Muid,
        a: &Ack<'_>,
        o: &mut [u8],
    ) -> Result<usize, zmidi_ci::DeviceError> {
        self.0.write_ack(p, a, o)
    }
    fn write_nak(
        &self,
        p: Muid,
        n: &Nak<'_>,
        o: &mut [u8],
    ) -> Result<usize, zmidi_ci::DeviceError> {
        self.0.write_nak(p, n, o)
    }
}

/// A discovers B, and each gets the PE Capabilities of the other.
fn pair(max_sysex: u32) -> (Side, Side) {
    let mut a = Side::new(0x41, 0x0111_1111, max_sysex, 0);
    let mut b = Side::new(0x42, 0x0222_2222, max_sysex, 1024);
    let mut buf = [0u8; 600];
    let n = a.dev.discovery(0, 0, &mut buf).unwrap();
    let (_, r) = b.rx(&buf[..n]);
    a.rx(&r[0]);
    let n = a.dev.pe_capabilities_inquiry(b.muid(), &mut buf).unwrap();
    let (_, r) = b.rx(&buf[..n]);
    a.rx(&r[0]);
    let n = b.dev.pe_capabilities_inquiry(a.muid(), &mut buf).unwrap();
    let (_, r) = a.rx(&buf[..n]);
    b.rx(&r[0]);
    (a, b)
}

fn sub_id2(msg: &[u8]) -> u8 {
    zmidi_ci::Header::parse(msg).unwrap().0.sub_id2
}

fn chunk(msg: &[u8]) -> PeChunk<'_> {
    let (h, body) = zmidi_ci::Header::parse(msg).unwrap();
    PeChunk::read(&h, body).unwrap()
}

/// The "status" of a reply message.
fn status(msg: &[u8]) -> u16 {
    Header::parse(HeaderKind::Reply, chunk(msg).header)
        .unwrap()
        .status
        .unwrap()
        .0
}

fn header_text(msg: &[u8]) -> String {
    String::from_utf8(chunk(msg).header.to_vec()).unwrap()
}

fn nak(msg: &[u8]) -> Nak<'_> {
    let (h, body) = zmidi_ci::Header::parse(msg).unwrap();
    assert_eq!(h.sub_id2, 0x7F, "not a NAK");
    Nak::read(&h, body).unwrap()
}

fn ack(msg: &[u8]) -> Ack<'_> {
    let (h, body) = zmidi_ci::Header::parse(msg).unwrap();
    assert_eq!(h.sub_id2, 0x7D, "not an ACK");
    Ack::read(&h, body).unwrap()
}

/// A raw Chunk from `src` to `dst`.
#[allow(clippy::too_many_arguments)]
fn raw(
    kind: PeKind,
    src: Muid,
    dst: Muid,
    rid: u8,
    header: &[u8],
    total: u16,
    number: u16,
    data: &[u8],
) -> Vec<u8> {
    let mut buf = [0u8; 600];
    let n = PeChunk {
        request_id: rid,
        header,
        total,
        number,
        data,
    }
    .write(kind, src, dst, Framing::Bare, &mut buf)
    .unwrap();
    buf[..n].to_vec()
}

fn text(s: &'static str) -> Option<HeaderStr<'static>> {
    Some(HeaderStr::Text(s))
}

fn get(resource: &'static str) -> Header<'static> {
    Header {
        resource: text(resource),
        ..Default::default()
    }
}

/// A asks B for `resource`; gives the Request ID and B's Request event.
fn get_request(
    a: &mut Side,
    b: &mut Side,
    resource: &'static str,
) -> (u8, Option<Ev>, Vec<Vec<u8>>) {
    let info = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(get(resource), 0),
            None,
            a.now,
        )
        .unwrap();
    let m = a.send(b.muid(), info.request_id, Direction::Initiator, &[], true);
    let (e, out) = b.rx(&m);
    (info.request_id, e, out)
}

#[test]
fn get_resource_list_round_trip() {
    // REQ-PE-LIST-002, REQ-PE-GEN-001, REQ-PE-RES-003, REQ-PE-ENC-001: the first Get, the
    // reply in two Chunks, and the ResourceList state.
    let (mut a, mut b) = pair(512);
    let info = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(get("DeviceInfo"), 0),
            None,
            a.now,
        )
        .unwrap();
    assert!(info.resource_list_missing);
    a.eng
        .abort(&mut a.dev, b.muid(), info.request_id, Direction::Initiator)
        .unwrap();
    a.pending();
    let mut buf = [0u8; 600];
    let (rid, n) = a
        .eng
        .request_resource_list(&mut a.dev, b.muid(), a.now, &mut buf)
        .unwrap();
    assert_eq!(header_text(&buf[..n]), r#"{"resource":"ResourceList"}"#);
    assert_eq!(chunk(&buf[..n]).data, b"");
    let (e, out) = b.rx(&buf[..n]);
    assert!(out.is_empty(), "no 404 or 405 for ResourceList");
    let Some(Ev::Msg {
        role,
        header,
        resource,
        end,
        ..
    }) = e
    else {
        panic!("{e:?}")
    };
    assert_eq!(
        (role, header, resource, end),
        (Direction::Responder, true, None, ChunkEnd::Complete)
    );
    // The reply: 1120 bytes in Chunks of the room of a 512-byte message.
    let data: Vec<u8> = br#"[{"resource":"DeviceInfo"},{"resource":"X-Patch","canSet":"full"},{"resource":"ProgramList","canPaginate":true},{"resource":"X-Bin"}]"#.repeat(8);
    let reply = Header {
        status: Some(Status::OK),
        ..Default::default()
    };
    b.eng
        .reply(
            &mut b.dev,
            a.muid(),
            rid,
            &Outgoing::json(reply, data.len()),
            b.now,
        )
        .unwrap();
    let mut sent = 0;
    let mut got = Vec::new();
    let mut ends = Vec::new();
    while sent < data.len() {
        let room = b
            .eng
            .data_room(&b.dev, a.muid(), rid, Direction::Responder)
            .unwrap();
        let part = &data[sent..(sent + room).min(data.len())];
        sent += part.len();
        let m = b.send(
            a.muid(),
            rid,
            Direction::Responder,
            part,
            sent == data.len(),
        );
        assert!(m.len() <= 512);
        let (e, out) = a.rx(&m);
        assert!(out.is_empty());
        let Some(Ev::Msg {
            data, end, kind, ..
        }) = e
        else {
            panic!("{e:?}")
        };
        assert_eq!(kind, TxnKind::Get);
        got.extend(data);
        ends.push(end);
    }
    assert_eq!(got, data);
    assert!(ends.len() >= 3);
    assert_eq!(ends.last(), Some(&ChunkEnd::Complete));
    assert_eq!(b.eng.open(), 0);
    assert_eq!(a.eng.open(), 0);
    assert!(a.eng.has_resource_list(b.muid()));
    let info = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(get("DeviceInfo"), 0),
            None,
            a.now,
        )
        .unwrap();
    assert!(!info.resource_list_missing);
}

#[test]
fn responder_statuses() {
    // REQ-PE-STAT-009, -010, -011, -012, REQ-PE-GEN-004, REQ-PE-STAT-006: the statuses of the
    // responder, each with a "message".
    let (a, mut b) = pair(512);
    let (src, dst) = (a.muid(), b.muid());
    let cases: [(&[u8], PeKind, u16); 8] = [
        (br#"{"resource":"Nope"}"#, PeKind::GetInquiry, 404),
        (br#"{"resource":"DeviceInfo"}"#, PeKind::SetInquiry, 405),
        (br#"{"resource":"DeviceInfo","offset":1}"#, PeKind::GetInquiry, 400),
        (br#"{"resId":"x"}"#, PeKind::GetInquiry, 400),
        (br#"{"resource":"DeviceInfo","mutualEncoding":"UTF-9"}"#, PeKind::GetInquiry, 415),
        (br#"{"resource":"DeviceInfo","mutualEncoding":"Mcoded7"}"#, PeKind::GetInquiry, 415),
        (br#"{"resource":"X-Bin","mediaType":"text/plain"}"#, PeKind::SetInquiry, 415),
        (br#"{"resource":"DeviceInfo","x":"0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789"}"#, PeKind::GetInquiry, 413),
    ];
    for (i, (h, kind, want)) in cases.into_iter().enumerate() {
        let m = raw(kind, src, dst, i as u8, h, 1, 1, b"");
        let (e, out) = b.rx(&m);
        assert_eq!(e, None);
        assert_eq!(out.len(), 1, "{i}");
        assert_eq!(status(&out[0]), want, "{i}");
        assert!(header_text(&out[0]).contains(r#""message":""#), "{i}");
        assert_eq!(chunk(&out[0]).request_id, i as u8);
    }
    // REQ-PE-GEN-004: all 4 slots in use.
    for rid in 20..24 {
        let m = raw(
            PeKind::GetInquiry,
            src,
            dst,
            rid,
            br#"{"resource":"DeviceInfo"}"#,
            1,
            1,
            b"",
        );
        let (e, out) = b.rx(&m);
        assert!(matches!(e, Some(Ev::Msg { .. })) && out.is_empty());
    }
    let m = raw(
        PeKind::GetInquiry,
        src,
        dst,
        30,
        br#"{"resource":"DeviceInfo"}"#,
        1,
        1,
        b"",
    );
    let (_, out) = b.rx(&m);
    assert_eq!(out.len(), 1);
    assert_eq!(status(&out[0]), 343);
}

#[test]
fn initiator_gets_343_and_3xx() {
    // REQ-PE-GEN-004, REQ-PE-STAT-004: a reply 343 or 341 ends the transaction and gives the
    // status; the Request ID is free again (REQ-PE-GEN-005).
    let (mut a, mut b) = pair(512);
    for code in [343u16, 341] {
        let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
        let h = format!(r#"{{"status":{code}}}"#);
        let m = raw(
            PeKind::GetReply,
            b.muid(),
            a.muid(),
            rid,
            h.as_bytes(),
            1,
            1,
            b"",
        );
        let (e, out) = a.rx(&m);
        assert!(out.is_empty());
        assert!(matches!(e, Some(Ev::Msg { status: Some(s), .. }) if s == code));
        assert_eq!(a.eng.open(), 0);
        b.eng
            .abort(&mut b.dev, a.muid(), rid, Direction::Responder)
            .unwrap();
        b.pending();
    }
}

#[test]
fn set_mcoded7_in_chunks() {
    // REQ-PE-ENC-004, REQ-PE-STAT-007: a Set in Mcoded7 over 2 Chunks; B gets the decoded
    // data and replies 200; A accepts 200 and 201 as success.
    let (mut a, mut b) = pair(512);
    let data: Vec<u8> = (0..600u32).map(|i| (i as u8).wrapping_mul(37)).collect();
    let mut e = Mcoded7Encoder::new();
    let mut enc = vec![0u8; 700];
    let mut n = e.encode(&data, &mut enc).unwrap();
    n += e.finish(&mut enc[n..]).unwrap();
    enc.truncate(n);
    let msg = Outgoing {
        header: get("X-Patch"),
        encoding: EncodingName::Mcoded7,
        json: true,
        total_len: Some(enc.len()),
    };
    let info = a
        .eng
        .request(&mut a.dev, b.muid(), TxnKind::Set, &msg, None, a.now)
        .unwrap();
    let rid = info.request_id;
    let mut sent = 0;
    let mut got = Vec::new();
    while sent < enc.len() {
        let room = a
            .eng
            .data_room(&a.dev, b.muid(), rid, Direction::Initiator)
            .unwrap();
        let part = &enc[sent..(sent + room).min(enc.len())];
        sent += part.len();
        let m = a.send(b.muid(), rid, Direction::Initiator, part, sent == enc.len());
        if got.is_empty() {
            assert!(header_text(&m).contains(r#""mutualEncoding":"Mcoded7""#));
        }
        let (ev, out) = b.rx(&m);
        assert!(out.is_empty());
        let Some(Ev::Msg { data, .. }) = ev else {
            panic!("{ev:?}")
        };
        got.extend(data);
    }
    assert_eq!(got, data);
    for code in [200u16, 201] {
        let reply = Header {
            status: Some(Status(code)),
            ..Default::default()
        };
        if code == 201 {
            let info = a
                .eng
                .request(
                    &mut a.dev,
                    b.muid(),
                    TxnKind::Set,
                    &Outgoing::json(get("X-Patch"), 2),
                    None,
                    a.now,
                )
                .unwrap();
            let m = a.send(b.muid(), info.request_id, Direction::Initiator, b"{}", true);
            b.rx(&m);
            assert_eq!(info.request_id, rid, "the Request ID is free again");
        }
        b.eng
            .reply(&mut b.dev, a.muid(), rid, &Outgoing::json(reply, 0), b.now)
            .unwrap();
        let m = b.send(a.muid(), rid, Direction::Responder, &[], true);
        let (ev, _) = a.rx(&m);
        assert!(
            matches!(ev, Some(Ev::Msg { status: Some(s), kind: TxnKind::Set, .. }) if s == code)
        );
        assert_eq!(a.eng.open(), 0);
    }
}

#[test]
fn chunk_gap_and_incomplete_end() {
    // REQ-PE-STAT-008: a gap with no Flow Control gets NAK 0x21 at once and closes; a final
    // Chunk 0x0000 gets 342; data that can not be decoded gets 342.
    let (a, mut b) = pair(512);
    let (src, dst) = (a.muid(), b.muid());
    let m = raw(
        PeKind::SetInquiry,
        src,
        dst,
        1,
        br#"{"resource":"X-Patch"}"#,
        3,
        1,
        b"{",
    );
    assert!(matches!(b.rx(&m).0, Some(Ev::Msg { .. })));
    let m = raw(PeKind::SetInquiry, src, dst, 1, b"", 3, 3, b"}");
    let (e, out) = b.rx(&m);
    assert_eq!(
        e,
        Some(Ev::Closed(
            1,
            Direction::Responder,
            CloseReason::OutOfSequence
        ))
    );
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2),
        (NakStatus::ChunksOutOfSequence, 0x36)
    );
    assert_eq!(
        n.details,
        Details::PropertyExchange {
            request_id: 1,
            chunk: 3
        }
    );
    // Final Chunk 0x0000.
    let m = raw(
        PeKind::SetInquiry,
        src,
        dst,
        2,
        br#"{"resource":"X-Patch"}"#,
        0,
        1,
        b"{",
    );
    b.rx(&m);
    let m = raw(PeKind::SetInquiry, src, dst, 2, b"", 2, 0, b"}");
    let (e, out) = b.rx(&m);
    assert!(matches!(
        e,
        Some(Ev::Msg {
            end: ChunkEnd::Incomplete,
            ..
        })
    ));
    assert_eq!(status(&out[0]), 342);
    // Data that can not be decoded: a zlib stream whose CMF byte is not valid.
    let h = br#"{"resource":"X-Zip","mutualEncoding":"zlib+Mcoded7"}"#;
    let m = raw(
        PeKind::SetInquiry,
        src,
        dst,
        3,
        h,
        1,
        1,
        &[0x00, 0x00, 0x00],
    );
    let (e, out) = b.rx(&m);
    assert_eq!(
        e,
        Some(Ev::Closed(
            3,
            Direction::Responder,
            CloseReason::Refused(Status::BAD_DATA)
        ))
    );
    assert_eq!(status(&out[0]), 342);
    assert_eq!(b.eng.open(), 0);
}

#[test]
fn timeouts_and_waits() {
    // REQ-PE-END-008, -010, -011, -012, -013, -014, REQ-PE-GEN-005, REQ-PE-END-007.
    let (mut a, mut b) = pair(512);
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    // B asks for 250 ms more: ACK 0x10 with 3 (rounded up to 300 ms).
    assert_eq!(
        b.eng.wait(a.muid(), rid, Direction::Responder, 0),
        Err(TransactionError::InvalidWait)
    );
    assert_eq!(
        b.eng.wait(a.muid(), rid, Direction::Responder, 12_701),
        Err(TransactionError::InvalidWait)
    );
    b.eng
        .wait(a.muid(), rid, Direction::Responder, 250)
        .unwrap();
    let out = b.pending();
    let k = ack(&out[0]);
    assert_eq!(
        (k.status, k.original_sub_id2),
        (AckStatus::TimeoutWait(3), 0x34)
    );
    let (e, _) = a.rx(&out[0]);
    assert_eq!(e, None);
    // A: 3 s + 300 ms after its Chunk.
    a.now += 3_300;
    assert_eq!(a.poll(), (None, vec![]));
    // A Notify 100 adds 3 s (sent by a peer; this crate never sends Notify).
    let m = raw(
        PeKind::Notify,
        b.muid(),
        a.muid(),
        rid,
        br#"{"status":100}"#,
        1,
        1,
        b"",
    );
    a.rx(&m);
    a.now += 3_000;
    assert_eq!(a.poll(), (None, vec![]));
    a.now += 1;
    let (e, out) = a.poll();
    assert_eq!(
        e,
        Some(Ev::Closed(rid, Direction::Initiator, CloseReason::Timeout))
    );
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.details),
        (
            NakStatus::Timeout,
            Details::PropertyExchange {
                request_id: rid,
                chunk: 0
            }
        )
    );
    // B: closes on the NAK 0x42, no reply.
    let (e, out) = b.rx(&out[0]);
    assert_eq!(
        e,
        Some(Ev::Closed(
            rid,
            Direction::Responder,
            CloseReason::PeerTimeout
        ))
    );
    assert!(out.is_empty());
    // The Request ID is free again.
    let (rid2, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    assert_eq!(rid2, rid);
    // REQ-PE-END-008: B's deadline is 3 s after the inquiry; a later reply is refused.
    b.now += 3_001;
    let r = Header {
        status: Some(Status::OK),
        ..Default::default()
    };
    assert_eq!(
        b.eng
            .reply(&mut b.dev, a.muid(), rid2, &Outgoing::json(r, 0), b.now),
        Err(TransactionError::Late)
    );
    assert_eq!(b.eng.open(), 0);
}

#[test]
fn notify_and_nak_end_a_transaction() {
    // REQ-PE-END-001 to -006, -015, -016.
    let (mut a, mut b) = pair(512);
    for (code, reason) in [
        (144, CloseReason::Terminated),
        (408, CloseReason::PeerTimeout),
    ] {
        let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
        let h = format!(r#"{{"status":{code}}}"#);
        let m = raw(
            PeKind::Notify,
            b.muid(),
            a.muid(),
            rid,
            h.as_bytes(),
            1,
            1,
            b"",
        );
        let (e, out) = a.rx(&m);
        assert_eq!(e, Some(Ev::Closed(rid, Direction::Initiator, reason)));
        assert!(out.is_empty());
        b.eng
            .abort(&mut b.dev, a.muid(), rid, Direction::Responder)
            .unwrap();
        let out = b.pending();
        let n = nak(&out[0]);
        assert_eq!(
            n.status,
            NakStatus::Terminate {
                by_initiator: false
            }
        );
    }
    // Initiator ends: NAK 0x20 with 0x01; B closes.
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    a.eng
        .abort(&mut a.dev, b.muid(), rid, Direction::Initiator)
        .unwrap();
    let out = a.pending();
    assert_eq!(
        nak(&out[0]).status,
        NakStatus::Terminate { by_initiator: true }
    );
    assert_eq!(
        b.rx(&out[0]).0,
        Some(Ev::Closed(
            rid,
            Direction::Responder,
            CloseReason::Terminated
        ))
    );
    // REQ-PE-END-016: a NAK 0x00 that answers an ACK (the JUCE reply) does not end it (its
    // details are not Property Exchange details); a NAK for an unknown Request ID is ignored.
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    let mut buf = [0u8; 128];
    for (orig, id) in [(0x7D, rid), (0x34, 99)] {
        let nk = Nak {
            original_sub_id2: orig,
            status: NakStatus::Nak,
            details: Details::PropertyExchange {
                request_id: id,
                chunk: 0,
            },
            text: &[],
        };
        let n = nk
            .write(
                "",
                zmidi_ci::DeviceId::FunctionBlock,
                a.muid(),
                b.muid(),
                None,
                Framing::Bare,
                &mut buf,
            )
            .unwrap();
        assert_eq!(b.rx(&buf[..n]).0, None);
    }
    assert_eq!(b.eng.open(), 1);
}

/// A Set to "X-Flow" (fcOnSet) with Flow Control, 3 Chunks.
fn flow_set(a: &mut Side, b: &mut Side) -> (u8, Vec<Vec<u8>>) {
    let fc = resources()[2].caps;
    let data = vec![b'7'; 1100];
    let info = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Set,
            &Outgoing::json(get("X-Flow"), data.len()),
            Some(&fc),
            a.now,
        )
        .unwrap();
    let rid = info.request_id;
    let room1 = a
        .eng
        .data_room(&a.dev, b.muid(), rid, Direction::Initiator)
        .unwrap();
    let mut parts = vec![data[..room1].to_vec()];
    let room = 512 - 22;
    let mut at = room1;
    while at < data.len() {
        parts.push(data[at..(at + room).min(data.len())].to_vec());
        at += room;
    }
    (rid, parts)
}

#[test]
fn flow_control_set() {
    // REQ-PE-FLOW-001, -002, -003, -008, -009, REQ-PE-ENC-001.
    let (mut a, mut b) = pair(512);
    let (rid, parts) = flow_set(&mut a, &mut b);
    assert_eq!(parts.len(), 3);
    for (i, p) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let m = a.send(b.muid(), rid, Direction::Initiator, p, last);
        if i == 0 {
            assert!(header_text(&m).contains(r#""flowControl":true"#));
        }
        if !last {
            // No next Chunk before the ACK (REQ-PE-FLOW-002).
            let mut buf = [0u8; 600];
            assert_eq!(
                a.eng.send(
                    &mut a.dev,
                    b.muid(),
                    rid,
                    Direction::Initiator,
                    &parts[i + 1],
                    false,
                    a.now,
                    &mut buf
                ),
                Err(TransactionError::State)
            );
        }
        let (e, out) = b.rx(&m);
        assert!(matches!(e, Some(Ev::Msg { .. })));
        if last {
            assert!(out.is_empty(), "no ACK for the last Chunk of a Set");
        } else {
            let k = ack(&out[0]);
            assert_eq!(k.status, AckStatus::FlowControlSendNext);
            assert_eq!(k.original_sub_id2, 0x36);
            assert_eq!(
                k.details,
                Details::PropertyExchange {
                    request_id: rid,
                    chunk: i as u16 + 1
                }
            );
            // An ACK with another Chunk number is ignored.
            let mut buf = [0u8; 128];
            let bad = Ack {
                details: Details::PropertyExchange {
                    request_id: rid,
                    chunk: 9,
                },
                ..k
            };
            let n = b
                .dev
                .ack(
                    a.muid(),
                    zmidi_ci::DeviceId::FunctionBlock,
                    &bad,
                    "",
                    &mut buf,
                )
                .unwrap();
            assert_eq!(a.rx(&buf[..n]).0, None);
            assert_eq!(a.rx(&out[0]).0, Some(Ev::Ready(rid)));
        }
    }
    let r = Header {
        status: Some(Status::OK),
        ..Default::default()
    };
    b.eng
        .reply(&mut b.dev, a.muid(), rid, &Outgoing::json(r, 0), b.now)
        .unwrap();
    let m = b.send(a.muid(), rid, Direction::Responder, &[], true);
    let (e, out) = a.rx(&m);
    assert!(matches!(
        e,
        Some(Ev::Msg {
            status: Some(200),
            ..
        })
    ));
    assert!(out.is_empty(), "no ACK 0x11 for a Reply to Set");
    assert_eq!((a.eng.open(), b.eng.open()), (0, 0));
}

#[test]
fn flow_control_get() {
    // REQ-PE-FLOW-004, -005, -006: with Flow Control the initiator ACKs each Chunk, the last
    // one too; the responder closes after its last Chunk; with no Flow Control no ACK.
    let (mut a, mut b) = pair(512);
    for flow in [true, false] {
        let h = Header {
            flow_control: flow,
            ..get("X-Flow")
        };
        let info = a
            .eng
            .request(
                &mut a.dev,
                b.muid(),
                TxnKind::Get,
                &Outgoing::json(h, 0),
                None,
                a.now,
            )
            .unwrap();
        let rid = info.request_id;
        let m = a.send(b.muid(), rid, Direction::Initiator, &[], true);
        assert!(b.rx(&m).1.is_empty(), "no ACK 0x11 for a Get inquiry");
        let data = vec![b'x'; 700];
        let r = Header {
            status: Some(Status::OK),
            ..Default::default()
        };
        b.eng
            .reply(
                &mut b.dev,
                a.muid(),
                rid,
                &Outgoing::json(r, data.len()),
                b.now,
            )
            .unwrap();
        let room1 = b
            .eng
            .data_room(&b.dev, a.muid(), rid, Direction::Responder)
            .unwrap();
        let c1 = b.send(a.muid(), rid, Direction::Responder, &data[..room1], false);
        if flow {
            // Chunk 2 only after the ACK of Chunk 1.
            let mut buf = [0u8; 600];
            assert_eq!(
                b.eng.send(
                    &mut b.dev,
                    a.muid(),
                    rid,
                    Direction::Responder,
                    &data[room1..],
                    true,
                    b.now,
                    &mut buf
                ),
                Err(TransactionError::State)
            );
            let (_, out) = a.rx(&c1);
            assert_eq!(ack(&out[0]).status, AckStatus::FlowControlSendNext);
            assert_eq!(b.rx(&out[0]).0, Some(Ev::Ready(rid)));
            let c2 = b.send(a.muid(), rid, Direction::Responder, &data[room1..], true);
            assert_eq!(b.eng.open(), 0, "closed after the last Chunk");
            let (e, out) = a.rx(&c2);
            assert!(matches!(
                e,
                Some(Ev::Msg {
                    end: ChunkEnd::Complete,
                    ..
                })
            ));
            let k = ack(&out[0]);
            assert_eq!(
                k.details,
                Details::PropertyExchange {
                    request_id: rid,
                    chunk: 2
                }
            );
            // B ignores the ACK of the last Chunk.
            assert_eq!(b.rx(&out[0]).0, None);
        } else {
            // REQ-PE-FLOW-006: Chunk 2 at once; REQ-PE-FLOW-005 order does not matter.
            let c2 = b.send(a.muid(), rid, Direction::Responder, &data[room1..], true);
            assert!(a.rx(&c1).1.is_empty());
            assert!(a.rx(&c2).1.is_empty());
        }
        assert_eq!(a.eng.open(), 0);
    }
}

#[test]
fn flow_control_resend() {
    // REQ-PE-FLOW-010 to -015.
    let (mut a, mut b) = pair(512);
    let (rid, parts) = flow_set(&mut a, &mut b);
    let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    // B: Chunk 1 corrupt (the caller says so): NAK 0x12 with "last good" 0, no ACK.
    b.rx(&raw(
        PeKind::SetInquiry,
        a.muid(),
        b.muid(),
        rid,
        chunk(&c1).header,
        3,
        1,
        &parts[0],
    ));
    b.eng.corrupt(a.muid(), rid, Direction::Responder).unwrap();
    let out = b.pending();
    assert_eq!(out.len(), 1);
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.details),
        (
            NakStatus::FlowControlResend,
            Details::PropertyExchange {
                request_id: rid,
                chunk: 0
            }
        )
    );
    // A sends Chunk 1 again (REQ-PE-FLOW-012).
    assert_eq!(a.rx(&out[0]).0, Some(Ev::Resend(rid, 1)));
    let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    let (_, out) = b.rx(&c1);
    assert_eq!(
        ack(&out[0]).details,
        Details::PropertyExchange {
            request_id: rid,
            chunk: 1
        }
    );
    // REQ-PE-FLOW-015: Chunk 1 again gets the ACK again and no event.
    let (e, out2) = b.rx(&c1);
    assert_eq!(
        (e, ack(&out2[0]).details),
        (
            None,
            Details::PropertyExchange {
                request_id: rid,
                chunk: 1
            }
        )
    );
    a.rx(&out[0]);
    let c2 = a.send(b.muid(), rid, Direction::Initiator, &parts[1], false);
    // REQ-PE-FLOW-013: no ACK for 3 s: send again, 3 times; then NAK 0x20.
    for k in 0..3 {
        a.now += 3_001;
        assert_eq!(a.poll(), (Some(Ev::Resend(rid, 2)), vec![]), "{k}");
        assert_eq!(
            a.send(b.muid(), rid, Direction::Initiator, &parts[1], false),
            c2
        );
    }
    a.now += 3_001;
    let (e, out) = a.poll();
    assert_eq!(
        e,
        Some(Ev::Closed(rid, Direction::Initiator, CloseReason::Timeout))
    );
    assert_eq!(
        nak(&out[0]).status,
        NakStatus::Terminate { by_initiator: true }
    );
    // REQ-PE-FLOW-011: B got Chunk 3 after Chunk 1: NAK 0x12 with 1.
    let c3 = raw(
        PeKind::SetInquiry,
        a.muid(),
        b.muid(),
        rid,
        b"",
        3,
        3,
        &parts[2],
    );
    let (e, out) = b.rx(&c3);
    assert_eq!(e, None);
    assert_eq!(
        nak(&out[0]).details,
        Details::PropertyExchange {
            request_id: rid,
            chunk: 1
        }
    );
    // REQ-PE-FLOW-014: B waits 12 s after its ACK, then closes with NAK 0x20.
    b.now += 12_000;
    assert_eq!(b.poll().0, None);
    b.now += 1;
    let (e, out) = b.poll();
    assert_eq!(
        e,
        Some(Ev::Closed(rid, Direction::Responder, CloseReason::Timeout))
    );
    assert_eq!(
        nak(&out[0]).status,
        NakStatus::Terminate {
            by_initiator: false
        }
    );
}

#[test]
fn flow_control_resend_of_an_old_chunk_ends() {
    // REQ-PE-FLOW-012: a NAK 0x12 for a Chunk that is not the last one sent ends the
    // transaction with NAK 0x20.
    let (mut a, mut b) = pair(512);
    let (rid, parts) = flow_set(&mut a, &mut b);
    let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    let (_, out) = b.rx(&c1);
    a.rx(&out[0]);
    a.send(b.muid(), rid, Direction::Initiator, &parts[1], false);
    let mut buf = [0u8; 128];
    let nk = Nak {
        original_sub_id2: 0x36,
        status: NakStatus::FlowControlResend,
        details: Details::PropertyExchange {
            request_id: rid,
            chunk: 0,
        },
        text: &[],
    };
    let n = b
        .dev
        .nak(
            a.muid(),
            zmidi_ci::DeviceId::FunctionBlock,
            &nk,
            "",
            &mut buf,
        )
        .unwrap();
    let (e, out) = a.rx(&buf[..n]);
    assert_eq!(
        e,
        Some(Ev::Closed(
            rid,
            Direction::Initiator,
            CloseReason::ResendFailed
        ))
    );
    assert_eq!(
        nak(&out[0]).status,
        NakStatus::Terminate { by_initiator: true }
    );
}

#[test]
fn early_406_and_407() {
    // REQ-PE-FLOW-007, -008: the reply after Chunk 1; later Chunks are ignored; the initiator
    // stops its transmission when the reply comes.
    let (mut a, mut b) = pair(512);
    let (src, dst) = (a.muid(), b.muid());
    // 406: "flowControl":true on Get for a Resource with fcOnGet false.
    let m = raw(
        PeKind::GetInquiry,
        src,
        dst,
        5,
        br#"{"resource":"DeviceInfo","flowControl":true}"#,
        1,
        1,
        b"",
    );
    assert_eq!(status(&b.rx(&m).1[0]), 406);
    let m = raw(
        PeKind::SetInquiry,
        src,
        dst,
        6,
        br#"{"resource":"X-Patch","flowControl":true}"#,
        2,
        1,
        b"{",
    );
    assert_eq!(status(&b.rx(&m).1[0]), 406);
    let m = raw(PeKind::SetInquiry, src, dst, 6, b"", 2, 2, b"}");
    assert_eq!(b.rx(&m), (None, vec![]));
    // A final Chunk 0x0000 after the refusal is ignored too.
    let m = raw(PeKind::SetInquiry, src, dst, 6, b"", 2, 0, b"");
    assert_eq!(b.rx(&m), (None, vec![]));
    // 407: A sends no "flowControl" (no caps given) to "X-Flow".
    let data = vec![b'1'; 700];
    let info = a
        .eng
        .request(
            &mut a.dev,
            dst,
            TxnKind::Set,
            &Outgoing::json(get("X-Flow"), data.len()),
            None,
            a.now,
        )
        .unwrap();
    let rid = info.request_id;
    let room = a
        .eng
        .data_room(&a.dev, dst, rid, Direction::Initiator)
        .unwrap();
    let c1 = a.send(dst, rid, Direction::Initiator, &data[..room], false);
    let (_, out) = b.rx(&c1);
    assert_eq!(status(&out[0]), 407);
    let (e, _) = a.rx(&out[0]);
    assert!(matches!(
        e,
        Some(Ev::Msg {
            status: Some(407),
            ..
        })
    ));
    assert_eq!(a.eng.open(), 0);
    let mut buf = [0u8; 600];
    assert_eq!(
        a.eng.send(
            &mut a.dev,
            dst,
            rid,
            Direction::Initiator,
            &data[room..],
            true,
            a.now,
            &mut buf
        ),
        Err(TransactionError::UnknownTransaction)
    );
}

#[test]
fn initiator_rules() {
    // REQ-PE-GEN-003, REQ-PE-ENC-002, REQ-PE-HDR-008, REQ-PE-TXN-007, REQ-PE-STAT-006.
    let (mut a, mut b) = pair(512);
    // The ResourceList of the peer refuses a Set to a Resource with canSet "none".
    let none = resources()[0].caps;
    assert_eq!(
        a.eng.request(
            &mut a.dev,
            b.muid(),
            TxnKind::Set,
            &Outgoing::json(get("DeviceInfo"), 2),
            Some(&none),
            a.now
        ),
        Err(TransactionError::Refused(Status::NOT_ALLOWED))
    );
    // Not JSON needs a mediaType.
    let msg = Outgoing {
        json: false,
        ..Outgoing::json(get("X-Bin"), 4)
    };
    assert_eq!(
        a.eng
            .request(&mut a.dev, b.muid(), TxnKind::Set, &msg, None, a.now),
        Err(TransactionError::NoMediaType)
    );
    // A peer that reported 2 simultaneous requests.
    let mut ids = Vec::new();
    for _ in 0..2 {
        let mut host = Limited(&mut a.dev, 2);
        ids.push(
            a.eng
                .request(
                    &mut host,
                    b.muid(),
                    TxnKind::Get,
                    &Outgoing::json(get("DeviceInfo"), 0),
                    None,
                    a.now,
                )
                .unwrap()
                .request_id,
        );
    }
    let mut host = Limited(&mut a.dev, 2);
    assert_eq!(
        a.eng.request(
            &mut host,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(get("DeviceInfo"), 0),
            None,
            a.now
        ),
        Err(TransactionError::PeerBusy)
    );
    for id in ids {
        a.eng
            .abort(&mut a.dev, b.muid(), id, Direction::Initiator)
            .unwrap();
    }
    a.pending();
    // REQ-PE-HDR-008: a Get with "mutualEncoding":"Mcoded7"; the reply uses it.
    let msg = Outgoing {
        encoding: EncodingName::Mcoded7,
        ..Outgoing::json(get("X-Patch"), 0)
    };
    let info = a
        .eng
        .request(&mut a.dev, b.muid(), TxnKind::Get, &msg, None, a.now)
        .unwrap();
    let m = a.send(b.muid(), info.request_id, Direction::Initiator, &[], true);
    assert!(header_text(&m).contains(r#""mutualEncoding":"Mcoded7""#));
    b.rx(&m);
    let r = Header {
        status: Some(Status::OK),
        ..Default::default()
    };
    assert_eq!(
        b.eng.reply(
            &mut b.dev,
            a.muid(),
            info.request_id,
            &Outgoing::json(r, 2),
            b.now
        ),
        Err(TransactionError::State)
    );
    let msg = Outgoing {
        encoding: EncodingName::Mcoded7,
        ..Outgoing::json(r, 3)
    };
    b.eng
        .reply(&mut b.dev, a.muid(), info.request_id, &msg, b.now)
        .unwrap();
    let m = b.send(
        a.muid(),
        info.request_id,
        Direction::Responder,
        &[0x00, 0x7B, 0x7D],
        true,
    );
    assert!(header_text(&m).contains(r#""mutualEncoding":"Mcoded7""#));
    let (e, _) = a.rx(&m);
    assert!(matches!(e, Some(Ev::Msg { ref data, .. }) if data == b"{}"));
    // REQ-PE-TXN-007: "totalCount" for a Resource with canPaginate.
    let (rid, _, _) = get_request(&mut a, &mut b, "ProgramList");
    assert_eq!(
        b.eng
            .reply(&mut b.dev, a.muid(), rid, &Outgoing::json(r, 2), b.now),
        Err(TransactionError::NoTotalCount)
    );
    let r2 = Header {
        total_count: Some(1),
        ..r
    };
    b.eng
        .reply(&mut b.dev, a.muid(), rid, &Outgoing::json(r2, 2), b.now)
        .unwrap();
    let m = b.send(a.muid(), rid, Direction::Responder, b"[]", true);
    assert!(header_text(&m).contains(r#""totalCount":1"#));
    a.rx(&m);
    // REQ-PE-STAT-006: an error reply from the caller gets a "message".
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    let r = Header {
        status: Some(Status::INTERNAL_ERROR),
        ..Default::default()
    };
    b.eng
        .reply(&mut b.dev, a.muid(), rid, &Outgoing::json(r, 0), b.now)
        .unwrap();
    let m = b.send(a.muid(), rid, Direction::Responder, &[], true);
    assert!(header_text(&m).contains(r#""message":"Internal device error""#));
}

#[test]
fn error_after_the_first_reply_chunk() {
    // REQ-PE-STAT-013: after the first Chunk of its reply, the responder ends with a NAK.
    let (mut a, mut b) = pair(512);
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    let r = Header {
        status: Some(Status::OK),
        ..Default::default()
    };
    b.eng
        .reply(&mut b.dev, a.muid(), rid, &Outgoing::json(r, 1000), b.now)
        .unwrap();
    let room = b
        .eng
        .data_room(&b.dev, a.muid(), rid, Direction::Responder)
        .unwrap();
    let c1 = b.send(
        a.muid(),
        rid,
        Direction::Responder,
        &vec![b' '; room],
        false,
    );
    a.rx(&c1);
    b.eng
        .abort(&mut b.dev, a.muid(), rid, Direction::Responder)
        .unwrap();
    let out = b.pending();
    assert_eq!(
        nak(&out[0]).details,
        Details::PropertyExchange {
            request_id: rid,
            chunk: 0
        }
    );
    assert_eq!(
        a.rx(&out[0]).0,
        Some(Ev::Closed(
            rid,
            Direction::Initiator,
            CloseReason::Terminated
        ))
    );
}

#[test]
fn zlib_window_check() {
    // REQ-PE-STAT-011: a zlib window larger than the window buffer (1024) gets 413; a Device
    // that does not inflate (window 0) refuses "zlib+Mcoded7" with 415.
    let (mut a, mut b) = pair(512);
    let (src, dst) = (a.muid(), b.muid());
    let h = br#"{"resource":"X-Zip","mutualEncoding":"zlib+Mcoded7"}"#;
    // Mcoded7 of [0x78 0x9C]: CMF 0x78 declares 32 KiB.
    let m = raw(
        PeKind::SetInquiry,
        src,
        dst,
        1,
        h,
        1,
        1,
        &[0x20, 0x78, 0x1C],
    );
    let (e, out) = b.rx(&m);
    assert_eq!(
        e,
        Some(Ev::Closed(
            1,
            Direction::Responder,
            CloseReason::Refused(Status::PAYLOAD_TOO_LARGE)
        ))
    );
    assert_eq!(status(&out[0]), 413);
    // CMF 0x18 declares 512: accepted.
    let m = raw(
        PeKind::SetInquiry,
        src,
        dst,
        2,
        h,
        1,
        1,
        &[0x00, 0x18, 0x53],
    );
    assert!(matches!(b.rx(&m).0, Some(Ev::Msg { ref data, .. }) if data == &[0x18, 0x53]));
    let m = raw(
        PeKind::SetInquiry,
        b.muid(),
        a.muid(),
        3,
        h,
        1,
        1,
        &[0x00, 0x18, 0x53],
    );
    let (_, out) = a.rx(&m);
    assert_eq!(status(&out[0]), 415);
}

#[test]
fn schema_ref_values() {
    // REQ-PE-LIST-010.
    assert_eq!(
        schema_ref("midi+jsonschema://patchSchema"),
        Ok(("patchSchema", None))
    );
    assert_eq!(
        schema_ref("midi+jsonschema://s_1#/definitions/x"),
        Ok(("s_1", Some("/definitions/x")))
    );
    for bad in [
        "http://x",
        "midi+jsonschema://",
        "midi+jsonschema://a-b",
        "midi+jsonschema://#x",
    ] {
        assert_eq!(schema_ref(bad), Err(TransactionError::InvalidRef), "{bad}");
    }
}

/// A subscribes to B's "X-Patch"; gives the index in A's table and in B's table.
fn subscribed(a: &mut Side, b: &mut Side) -> (usize, usize) {
    let g = Header {
        mutual_encoding: Some(EncodingName::Mcoded7),
        ..get("X-Patch")
    };
    let info = a
        .eng
        .subscribe(&mut a.dev, b.muid(), &g, None, a.now)
        .unwrap();
    let m = a.send(b.muid(), info.request_id, Direction::Initiator, &[], true);
    assert!(header_text(&m).starts_with(r#"{"command":"start","resource":"X-Patch""#));
    let (e, out) = b.rx(&m);
    let Some(Ev::SubStarted(bi)) = e else {
        panic!("{e:?}")
    };
    assert_eq!(status(&out[0]), 200);
    assert!(header_text(&out[0]).contains(r#""subscribeId":"s"#));
    let (e, _) = a.rx(&out[0]);
    let Some(Ev::Msg {
        started: Some(Ok(ai)),
        ..
    }) = e
    else {
        panic!("{e:?}")
    };
    (ai, bi)
}

#[test]
fn subscription_start_update_end() {
    // REQ-PE-SUB-006, -007, -008, -010, -015, -017, -018, -020.
    let (mut a, mut b) = pair(512);
    let (ai, bi) = subscribed(&mut a, &mut b);
    assert_eq!(a.eng.subscriptions().entries().count(), 1);
    // B sends a "full" with Property Data to its subscriber.
    let data = br#"{"name":"Init"}"#;
    let info = b
        .eng
        .update(
            &mut b.dev,
            bi,
            Command::Full,
            EncodingName::Ascii,
            Some(data.len()),
            b.now,
        )
        .unwrap();
    let m = b.send(a.muid(), info.request_id, Direction::Initiator, data, true);
    assert!(header_text(&m).contains(r#""command":"full""#));
    let (e, out) = a.rx(&m);
    let Some(Ev::Msg {
        sub: Some(s),
        data: d,
        kind: TxnKind::Subscription,
        ..
    }) = e
    else {
        panic!("{e:?}")
    };
    assert_eq!((s, d.as_slice()), (ai, &data[..]));
    assert_eq!(status(&out[0]), 200);
    assert!(!header_text(&out[0]).contains("subscribeId"));
    assert!(matches!(
        b.rx(&out[0]).0,
        Some(Ev::Msg {
            status: Some(200),
            ..
        })
    ));
    // A "partial" in the Partial SET format (REQ-PE-SUB-009).
    let data = br#"{"/name":"Pad"}"#;
    let info = b
        .eng
        .update(
            &mut b.dev,
            bi,
            Command::Partial,
            EncodingName::Ascii,
            Some(data.len()),
            b.now,
        )
        .unwrap();
    let m = b.send(a.muid(), info.request_id, Direction::Initiator, data, true);
    let (e, out) = a.rx(&m);
    b.rx(&out[0]);
    let Some(Ev::Msg { data: d, .. }) = e else {
        panic!("{e:?}")
    };
    let mut r = PartialSetReader::<20>::new();
    let mut items = r.push(&d);
    let item = items.next_item().unwrap().unwrap();
    assert_eq!(item.key, "/name");
    // The initiator ends it; B removes its entry and gives an event.
    let (info, entry) = a.eng.unsubscribe(&mut a.dev, ai, a.now).unwrap();
    let m = a.send(b.muid(), info.request_id, Direction::Initiator, &[], true);
    assert!(header_text(&m).contains(r#""endedBy":"initiator""#));
    let (e, out) = b.rx(&m);
    assert_eq!(e, Some(Ev::SubEnded(entry.subscribe_id().to_string())));
    assert_eq!(status(&out[0]), 200);
    assert_eq!(b.eng.subscriptions().entries().count(), 0);
    assert_eq!(a.eng.subscriptions().entries().count(), 0);
}

#[test]
fn subscription_notify_gets_again() {
    // REQ-PE-SUB-013, -014, -016: a "notify" gets 200 and a Get with the "start" values.
    let (mut a, mut b) = pair(512);
    let (ai, bi) = subscribed(&mut a, &mut b);
    let info = b
        .eng
        .update(
            &mut b.dev,
            bi,
            Command::Notify,
            EncodingName::Ascii,
            Some(0),
            b.now,
        )
        .unwrap();
    let m = b.send(a.muid(), info.request_id, Direction::Initiator, &[], true);
    let (e, out) = a.rx(&m);
    assert!(matches!(e, Some(Ev::Msg { sub: Some(s), .. }) if s == ai));
    assert_eq!(out.len(), 2);
    assert_eq!(status(&out[0]), 200);
    assert_eq!(sub_id2(&out[1]), 0x34);
    assert_eq!(
        header_text(&out[1]),
        r#"{"resource":"X-Patch","mutualEncoding":"Mcoded7"}"#
    );
    // The reply to that Get carries the subscription index.
    let (e, _) = b.rx(&out[1]);
    let Some(Ev::Msg { rid, .. }) = e else {
        panic!("{e:?}")
    };
    let r = Header {
        status: Some(Status::OK),
        ..Default::default()
    };
    b.eng
        .reply(
            &mut b.dev,
            a.muid(),
            rid,
            &Outgoing {
                encoding: EncodingName::Mcoded7,
                ..Outgoing::json(r, 3)
            },
            b.now,
        )
        .unwrap();
    let m = b.send(a.muid(), rid, Direction::Responder, &[0, b'{', b'}'], true);
    assert!(matches!(a.rx(&m).0, Some(Ev::Msg { sub: Some(s), .. }) if s == ai));
    // REQ-PE-SUB-016: the caller can not apply an update: a Get follows.
    a.eng.refresh(ai).unwrap();
    let out = a.pending();
    assert_eq!(sub_id2(&out[0]), 0x34);
}

#[test]
fn subscription_errors() {
    // REQ-PE-SUB-007, -015, -019, -021.
    let (mut a, mut b) = pair(512);
    let (src, dst) = (a.muid(), b.muid());
    // 405 without canSubscribe; 404 for an unknown Resource.
    let m = raw(
        PeKind::Subscription,
        src,
        dst,
        1,
        br#"{"command":"start","resource":"DeviceInfo"}"#,
        1,
        1,
        b"",
    );
    assert_eq!(status(&b.rx(&m).1[0]), 405);
    let m = raw(
        PeKind::Subscription,
        src,
        dst,
        1,
        br#"{"command":"start","resource":"Nope"}"#,
        1,
        1,
        b"",
    );
    assert_eq!(status(&b.rx(&m).1[0]), 404);
    // REQ-PE-SUB-021: the initiator of a subscription can not send an update.
    let (ai, _) = subscribed(&mut a, &mut b);
    assert_eq!(
        a.eng.update(
            &mut a.dev,
            ai,
            Command::Partial,
            EncodingName::Ascii,
            Some(2),
            a.now
        ),
        Err(TransactionError::Subscription(
            SubscriptionError::NotPermitted
        ))
    );
    // 343 when the table is full (4 entries).
    for rid in 10..13 {
        let m = raw(
            PeKind::Subscription,
            src,
            dst,
            rid,
            br#"{"command":"start","resource":"X-Patch"}"#,
            1,
            1,
            b"",
        );
        assert_eq!(status(&b.rx(&m).1[0]), 200);
    }
    let m = raw(
        PeKind::Subscription,
        src,
        dst,
        9,
        br#"{"command":"start","resource":"X-Patch"}"#,
        1,
        1,
        b"",
    );
    assert_eq!(status(&b.rx(&m).1[0]), 343);
    // An unknown "subscribeId": 400 for "full", 200 for "end".
    let m = raw(
        PeKind::Subscription,
        dst,
        src,
        2,
        br#"{"command":"full","subscribeId":"zz","resource":"X-Patch"}"#,
        1,
        1,
        b"{}",
    );
    assert_eq!(status(&a.rx(&m).1[0]), 400);
    let m = raw(
        PeKind::Subscription,
        dst,
        src,
        3,
        br#"{"command":"end","subscribeId":"zz"}"#,
        1,
        1,
        b"",
    );
    let (e, out) = a.rx(&m);
    assert_eq!((e, status(&out[0])), (None, 200));
    // REQ-PE-SUB-019: an Invalidate MUID of A removes B's entries for A, with no message.
    let mut buf = [0u8; 64];
    let n = zmidi_ci::InvalidateMuid { target: a.muid() }
        .write(a.muid(), Framing::Bare, &mut buf)
        .unwrap();
    let (e, out) = b.rx(&buf[..n]);
    assert_eq!(e, Some(Ev::Invalidated(4)));
    assert!(out.is_empty());
    let mut ended = 0;
    while b.eng.take_ended().is_some() {
        ended += 1;
    }
    assert_eq!((ended, b.eng.subscriptions().entries().count()), (4, 0));
}

#[test]
fn set_chunk_deadline_and_incomplete_reply() {
    // REQ-PE-END-009: the responder closes a Set with no next Chunk at the deadline and sends
    // NAK 0x20. REQ-PE-STAT-008: an incomplete reply is an error for the caller.
    let (mut a, mut b) = pair(512);
    let (src, dst) = (a.muid(), b.muid());
    let m = raw(
        PeKind::SetInquiry,
        src,
        dst,
        4,
        br#"{"resource":"X-Patch"}"#,
        2,
        1,
        b"{",
    );
    assert!(matches!(b.rx(&m).0, Some(Ev::Msg { .. })));
    b.now += 3_000;
    assert_eq!(b.poll().0, None);
    b.now += 1;
    let (e, out) = b.poll();
    assert_eq!(
        e,
        Some(Ev::Closed(4, Direction::Responder, CloseReason::Timeout))
    );
    assert_eq!(
        nak(&out[0]).status,
        NakStatus::Terminate {
            by_initiator: false
        }
    );
    // The initiator gets a reply that ends with Chunk 0x0000.
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    let m = raw(
        PeKind::GetReply,
        dst,
        src,
        rid,
        br#"{"status":200}"#,
        2,
        1,
        b"[",
    );
    assert!(matches!(
        a.rx(&m).0,
        Some(Ev::Msg {
            end: ChunkEnd::More,
            ..
        })
    ));
    let m = raw(PeKind::GetReply, dst, src, rid, b"", 2, 0, b"]");
    let (e, out) = a.rx(&m);
    assert!(matches!(
        e,
        Some(Ev::Msg {
            end: ChunkEnd::Incomplete,
            ..
        })
    ));
    assert!(out.is_empty());
    assert_eq!(a.eng.open(), 0);
    // This crate never sends a Notify (REQ-PE-END-007): no engine call writes Sub-ID#2 0x3F.
    for m in b.pending().iter().chain(a.pending().iter()) {
        assert_ne!(sub_id2(m), 0x3F);
    }
}

#[test]
fn same_request_id_on_both_sides() {
    // REQ-PE-END-001, -003, -005, -014: both Devices use Request ID 0 for their own Get; a
    // NAK 0x20 or 0x42 from B as initiator ends A's responder transaction, not A's request.
    let (mut a, mut b) = pair(512);
    let (rid_a, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    let (rid_b, e, _) = get_request(&mut b, &mut a, "DeviceInfo");
    assert_eq!(rid_a, rid_b);
    assert!(matches!(
        e,
        Some(Ev::Msg {
            role: Direction::Responder,
            ..
        })
    ));
    b.eng
        .abort(&mut b.dev, a.muid(), rid_b, Direction::Initiator)
        .unwrap();
    let out = b.pending();
    let (e, _) = a.rx(&out[0]);
    assert_eq!(
        e,
        Some(Ev::Closed(
            rid_b,
            Direction::Responder,
            CloseReason::Terminated
        ))
    );
    assert_eq!(a.eng.open(), 1, "A's own Get is still open");
    let (rid_b, _, _) = get_request(&mut b, &mut a, "DeviceInfo");
    b.now += 3_001;
    // B's responder transaction for A's Get also expires; it closes with no message.
    let mut out = Vec::new();
    while let (Some(_), o) = b.poll() {
        out.extend(o);
    }
    assert_eq!(out.len(), 1);
    assert_eq!(nak(&out[0]).status, NakStatus::Timeout);
    let (e, _) = a.rx(&out[0]);
    assert_eq!(
        e,
        Some(Ev::Closed(
            rid_b,
            Direction::Responder,
            CloseReason::PeerTimeout
        ))
    );
    assert_eq!(a.eng.open(), 1);
}

#[test]
fn corrupt_mcoded7_chunk_is_decoded_again() {
    // REQ-PE-FLOW-011, -012: a Flow Control Set in Mcoded7; the caller marks Chunk 2 corrupt;
    // the resent Chunk 2 decodes from the state before it. The last Chunk can be sent again.
    let (mut a, mut b) = pair(512);
    let fc = resources()[2].caps;
    let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
    let mut e = Mcoded7Encoder::new();
    let mut enc = vec![0u8; 1200];
    let mut n = e.encode(&data, &mut enc).unwrap();
    n += e.finish(&mut enc[n..]).unwrap();
    enc.truncate(n);
    let msg = Outgoing {
        encoding: EncodingName::Mcoded7,
        ..Outgoing::json(get("X-Flow"), enc.len())
    };
    let rid = a
        .eng
        .request(&mut a.dev, b.muid(), TxnKind::Set, &msg, Some(&fc), a.now)
        .unwrap()
        .request_id;
    let room1 = a
        .eng
        .data_room(&a.dev, b.muid(), rid, Direction::Initiator)
        .unwrap();
    let mut parts = vec![enc[..room1].to_vec()];
    for p in enc[room1..].chunks(512 - 22) {
        parts.push(p.to_vec());
    }
    assert_eq!(parts.len(), 3);
    assert_ne!(room1 % 8, 0, "Chunk 1 ends inside an Mcoded7 group");
    let mut got = Vec::new();
    let m = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    let (e, out) = b.rx(&m);
    let Some(Ev::Msg { data: d, .. }) = e else {
        panic!("{e:?}")
    };
    got.extend(d);
    assert_eq!(a.rx(&out[0]).0, Some(Ev::Ready(rid)));
    // Chunk 2, corrupt.
    let m = a.send(b.muid(), rid, Direction::Initiator, &parts[1], false);
    b.rx(&m);
    b.eng.corrupt(a.muid(), rid, Direction::Responder).unwrap();
    let out = b.pending();
    assert_eq!(out.len(), 1, "no ACK for the corrupt Chunk");
    assert_eq!(a.rx(&out[0]).0, Some(Ev::Resend(rid, 2)));
    let m = a.send(b.muid(), rid, Direction::Initiator, &parts[1], false);
    let (e, out) = b.rx(&m);
    let Some(Ev::Msg { data: d, .. }) = e else {
        panic!("{e:?}")
    };
    got.extend(d);
    assert_eq!(a.rx(&out[0]).0, Some(Ev::Ready(rid)));
    // Chunk 3, the last, corrupt: A sends it again after the NAK 0x12.
    let m = a.send(b.muid(), rid, Direction::Initiator, &parts[2], true);
    b.rx(&m);
    b.eng.corrupt(a.muid(), rid, Direction::Responder).unwrap();
    let out = b.pending();
    assert_eq!(
        nak(&out[0]).details,
        Details::PropertyExchange {
            request_id: rid,
            chunk: 2
        }
    );
    assert_eq!(a.rx(&out[0]).0, Some(Ev::Resend(rid, 3)));
    let m = a.send(b.muid(), rid, Direction::Initiator, &parts[2], true);
    let (e, out) = b.rx(&m);
    assert!(out.is_empty());
    let Some(Ev::Msg {
        data: d,
        end: ChunkEnd::Complete,
        ..
    }) = e
    else {
        panic!("{e:?}")
    };
    got.extend(d);
    assert_eq!(got, data);
}

#[test]
fn full_queue_changes_nothing() {
    // A message that comes when the control message queue has less than 3 free entries gives
    // QueueFull and no change; after write_pending it is processed. A scratch buffer that is
    // too small is an error before any change (no slot leak).
    let (a, mut b) = pair(512);
    let mut scratch = [0u8; 600];
    let mut queued = 0;
    for rid in 0..20u8 {
        let m = raw(
            PeKind::GetInquiry,
            a.muid(),
            b.muid(),
            rid,
            br#"{"resource":"Nope"}"#,
            1,
            1,
            b"",
        );
        let ev = b.dev.receive(0, &m, 0, &mut rng(7)).event().unwrap();
        match b.eng.receive(&mut b.dev, &ev, b.now, &mut scratch) {
            Ok(None) => queued += 1,
            Err(TransactionError::QueueFull) => break,
            r => panic!("{r:?}"),
        }
    }
    assert_eq!(queued, 14);
    assert_eq!(b.pending().len(), 14);
    let m = raw(
        PeKind::GetInquiry,
        a.muid(),
        b.muid(),
        40,
        br#"{"resource":"DeviceInfo"}"#,
        1,
        1,
        b"",
    );
    assert!(matches!(b.rx(&m).0, Some(Ev::Msg { .. })));
    let m = raw(
        PeKind::SetInquiry,
        a.muid(),
        b.muid(),
        41,
        br#"{"resource":"X-Patch"}"#,
        1,
        1,
        b"{}",
    );
    let ev = b.dev.receive(0, &m, 0, &mut rng(7)).event().unwrap();
    assert_eq!(
        b.eng.receive(&mut b.dev, &ev, b.now, &mut [0u8; 1]),
        Err(TransactionError::Size)
    );
    assert_eq!(b.eng.open(), 1);
}

#[test]
fn caller_requests_on_a_full_queue_give_queue_full() {
    // A caller request that queues a control message gives QueueFull when the queue has no free
    // entry and changes nothing, so that no message is lost (refresh: REQ-PE-SUB-016; wait:
    // REQ-PE-END-010; abort: REQ-PE-END-009; reply_status: REQ-PE-STAT-011). After
    // write_pending it works.
    let (mut a, mut b) = pair(512);
    let (ai, bi) = subscribed(&mut a, &mut b);
    a.pending();
    let rid = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(get("DeviceInfo"), 0),
            None,
            a.now,
        )
        .unwrap()
        .request_id;
    let open = a.eng.open();
    let mut refreshes = 0;
    while a.eng.refresh(ai).is_ok() {
        refreshes += 1;
        assert!(refreshes <= 64, "refresh never gives QueueFull");
    }
    assert_eq!(a.eng.refresh(ai), Err(TransactionError::QueueFull));
    assert_eq!(
        a.eng.wait(b.muid(), rid, Direction::Initiator, 500),
        Err(TransactionError::QueueFull)
    );
    assert_eq!(
        a.eng.abort(&mut a.dev, b.muid(), rid, Direction::Initiator),
        Err(TransactionError::QueueFull)
    );
    assert_eq!(a.eng.open(), open);
    // write_pending writes the Gets that the peer limit lets go (REQ-PE-SUB-014); the others
    // wait. The free entries then let the requests work.
    assert!(!a.pending().is_empty());
    let open = a.eng.open();
    a.eng
        .wait(b.muid(), rid, Direction::Initiator, 500)
        .unwrap();
    a.eng
        .abort(&mut a.dev, b.muid(), rid, Direction::Initiator)
        .unwrap();
    assert_eq!(a.eng.open(), open - 1);
    let out = a.pending();
    assert!(out.iter().any(|m| sub_id2(m) == 0x7D)); // ACK 0x10, wait
    assert!(out.iter().any(|m| sub_id2(m) == 0x7F)); // NAK, terminate

    // reply_status: b has an open inquiry to reply to.
    let m = raw(
        PeKind::GetInquiry,
        a.muid(),
        b.muid(),
        40,
        br#"{"resource":"DeviceInfo"}"#,
        1,
        1,
        b"",
    );
    let Some(Ev::Msg { rid, .. }) = b.rx(&m).0 else {
        panic!("no inquiry")
    };
    b.pending();
    while b.eng.refresh(bi).is_ok() {}
    let open = b.eng.open();
    assert_eq!(
        b.eng
            .reply_status(&mut b.dev, a.muid(), rid, Status::PAYLOAD_TOO_LARGE, b.now),
        Err(TransactionError::QueueFull)
    );
    assert_eq!(b.eng.open(), open);
    b.pending(); // queued Gets go out and open transactions
    let open = b.eng.open();
    b.eng
        .reply_status(&mut b.dev, a.muid(), rid, Status::PAYLOAD_TOO_LARGE, b.now)
        .unwrap();
    assert_eq!(b.eng.open(), open - 1);
    let out = b.pending();
    assert!(out.iter().any(|m| sub_id2(m) == 0x35 && status(m) == 413));
}

#[test]
fn queue_capacity_follows_q() {
    // OPT-006: the control message queue has Q entries. A received event needs 3 free entries,
    // so Q - 2 refused inquiries are queued before QueueFull (14 for the default 16).
    let (a, mut b) = pair(512);
    let mut eng: PeEngine<'static, 2, 256, 4, 2, 4> = PeEngine::new(EngineConfig {
        resources: resources(),
        zlib_window: 0,
    });
    let mut scratch = [0u8; 600];
    let mut queued = 0;
    for rid in 0..10u8 {
        let m = raw(
            PeKind::GetInquiry,
            a.muid(),
            b.muid(),
            rid,
            br#"{"resource":"Nope"}"#,
            1,
            1,
            b"",
        );
        let ev = b.dev.receive(0, &m, 0, &mut rng(7)).event().unwrap();
        match eng.receive(&mut b.dev, &ev, b.now, &mut scratch) {
            Ok(None) => queued += 1,
            Err(TransactionError::QueueFull) => break,
            r => panic!("{r:?}"),
        }
    }
    assert_eq!(queued, 2);
}

#[test]
fn corrupt_on_a_full_queue_uses_the_place_of_the_removed_ack() {
    // REQ-PE-FLOW-011 with a full queue: corrupt removes the queued ACK of the bad Chunk, so its
    // NAK 0x12 has a place and nothing is lost.
    let (mut a, mut b) = pair(512);
    let (_, bi) = subscribed(&mut a, &mut b);
    b.pending();
    let (rid, parts) = flow_set(&mut a, &mut b);
    let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    let m = raw(
        PeKind::SetInquiry,
        a.muid(),
        b.muid(),
        rid,
        chunk(&c1).header,
        3,
        1,
        &parts[0],
    );
    let ev = b.dev.receive(0, &m, 0, &mut rng(7)).event().unwrap();
    let mut scratch = [0u8; 600];
    b.eng.receive(&mut b.dev, &ev, b.now, &mut scratch).unwrap();
    while b.eng.refresh(bi).is_ok() {}
    b.eng.corrupt(a.muid(), rid, Direction::Responder).unwrap();
    let out = b.pending();
    assert!(
        out.iter().all(|m| sub_id2(m) != 0x7D),
        "no ACK for the corrupt Chunk"
    );
    assert!(out
        .iter()
        .any(|m| sub_id2(m) == 0x7F && nak(m).status == NakStatus::FlowControlResend));
}

#[test]
fn engine_with_no_subscriptions_lists_no_subscribable_resource() {
    // REQ-PE-SUB-022: with S = 0 the ResourceList of the engine has no "canSubscribe":true,
    // although X-Patch has it in its caps; with S > 0 it has.
    fn list(text: &mut Vec<u8>, mut w: zmidi_ci_pe::ResourceListWriter<'_, 512>) {
        let mut buf = [0u8; 64];
        loop {
            let n = w.write_next(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            text.extend_from_slice(&buf[..n]);
        }
    }
    let cfg = || EngineConfig {
        resources: resources(),
        zlib_window: 0,
    };
    let none: PeEngine<'static, 2, 256, 0, 2, 4> = PeEngine::new(cfg());
    let some: PeEngine<'static, 2, 256, 4, 2, 4> = PeEngine::new(cfg());
    let (mut a, mut b) = (Vec::new(), Vec::new());
    list(&mut a, none.resource_list());
    list(&mut b, some.resource_list());
    let has = |t: &[u8]| t.windows(19).any(|w| w == b"\"canSubscribe\":true");
    assert!(!has(&a), "{}", String::from_utf8_lossy(&a));
    assert!(has(&b), "{}", String::from_utf8_lossy(&b));
}
