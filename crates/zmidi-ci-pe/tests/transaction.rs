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
    ResourceEntry, ScalarValue, Status, SubscriptionError, TransactionError, TxnKind,
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
    // Mcoded7 only: no "ASCII" (as the X-Pattern Resource of FLXS1).
    let mut m7 = ResourceEntry::new("X-M7");
    m7.caps.can_set = CanSet::Full;
    m7.caps.encodings.ascii = false;
    m7.caps.encodings.mcoded7 = true;
    Box::leak(Box::new([info, patch, fc, list, z, bin, m7]))
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
        /// "mediaType" of the header (REQ-PE-ENC-001).
        media: Option<String>,
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
            media: m
                .header
                .and_then(|h| h.media_type)
                .map(|t| t.chars().collect()),
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

    /// Delivers one message as `rx` does, but writes no pending message of the engine.
    fn rx_held(&mut self, msg: &[u8]) -> Option<Ev> {
        let ev = self
            .dev
            .receive(0, msg, self.now * 1000, &mut rng(7))
            .event()?;
        let mut scratch = [0u8; 600];
        self.eng
            .receive(&mut self.dev, &ev, self.now, &mut scratch)
            .unwrap()
            .map(own)
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

/// A MIDI-CI NAK from `from` to `to` for Request ID `rid`.
fn nak_from(from: &Side, to: Muid, orig: u8, status: NakStatus, rid: u8, chunk: u16) -> Vec<u8> {
    let nk = Nak {
        original_sub_id2: orig,
        status,
        details: Details::PropertyExchange {
            request_id: rid,
            chunk,
        },
        text: &[],
    };
    let mut buf = [0u8; 128];
    let n = from
        .dev
        .nak(to, zmidi_ci::DeviceId::FunctionBlock, &nk, "", &mut buf)
        .unwrap();
    buf[..n].to_vec()
}

/// A MIDI-CI ACK from `from` to `to` for Request ID `rid`.
fn ack_from(from: &Side, to: Muid, orig: u8, status: AckStatus, rid: u8, chunk: u16) -> Vec<u8> {
    let k = Ack {
        original_sub_id2: orig,
        status,
        details: Details::PropertyExchange {
            request_id: rid,
            chunk,
        },
        text: &[],
    };
    let mut buf = [0u8; 128];
    let n = from
        .dev
        .ack(to, zmidi_ci::DeviceId::FunctionBlock, &k, "", &mut buf)
        .unwrap();
    buf[..n].to_vec()
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
    let media76 = format!(
        r#"{{"resource":"X-Bin","mediaType":"application/{}"}}"#,
        "x".repeat(64)
    );
    let cases: [(&[u8], PeKind, u16); 14] = [
        // REQ-PE-STAT-009: a header that the parser refuses, in a Set and in a Subscription.
        (br#"{"resource":"X-Patch","offset":1}"#, PeKind::SetInquiry, 400),
        (br#"{"command":"start"}"#, PeKind::Subscription, 400),
        (br#"{"resource":"Nope"}"#, PeKind::GetInquiry, 404),
        (br#"{"resource":"Nope"}"#, PeKind::SetInquiry, 404),
        // A "mediaType" of 76 characters, which the header parser refuses.
        (media76.as_bytes(), PeKind::SetInquiry, 415),
        (br#"{"resource":"DeviceInfo"}"#, PeKind::SetInquiry, 405),
        (br#"{"resource":"DeviceInfo","offset":1}"#, PeKind::GetInquiry, 400),
        (br#"{"resId":"x"}"#, PeKind::GetInquiry, 400),
        (br#"{"resource":"DeviceInfo","mutualEncoding":"UTF-9"}"#, PeKind::GetInquiry, 415),
        (br#"{"resource":"DeviceInfo","mutualEncoding":"Mcoded7"}"#, PeKind::GetInquiry, 415),
        (br#"{"resource":"X-Bin","mediaType":"text/plain"}"#, PeKind::SetInquiry, 415),
        // No "mutualEncoding" is "ASCII", which the Resource does not list.
        (br#"{"resource":"X-M7"}"#, PeKind::GetInquiry, 415),
        (br#"{"resource":"X-M7"}"#, PeKind::SetInquiry, 415),
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
        // REQ-PE-STAT-012: no Property Data in the reply.
        assert_eq!(
            (chunk(&out[0]).total, chunk(&out[0]).data),
            (1, &b""[..]),
            "{i}"
        );
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
    // The initiator: zlib data that can not be decoded gives an error to the caller. B (with a
    // zlib window) asks A.
    let (mut a, mut b) = pair(512);
    let msg = Outgoing {
        encoding: EncodingName::ZlibMcoded7,
        ..Outgoing::json(get("X-Zip"), 0)
    };
    let info = b
        .eng
        .request(&mut b.dev, a.muid(), TxnKind::Get, &msg, None, b.now)
        .unwrap();
    let rid = info.request_id;
    let m = b.send(a.muid(), rid, Direction::Initiator, &[], true);
    a.rx(&m);
    let m = raw(
        PeKind::GetReply,
        a.muid(),
        b.muid(),
        rid,
        br#"{"status":200,"mutualEncoding":"zlib+Mcoded7"}"#,
        1,
        1,
        &[0x00, 0x00, 0x00],
    );
    let (e, _) = b.rx(&m);
    assert_eq!(
        e,
        Some(Ev::Closed(
            rid,
            Direction::Initiator,
            CloseReason::Refused(Status::BAD_DATA)
        ))
    );
    assert_eq!(b.eng.open(), 0);
    a.eng
        .abort(&mut a.dev, b.muid(), rid, Direction::Responder)
        .unwrap();
    a.pending();
    // The initiator: a Chunk gap in a reply gets NAK 0x21 and closes.
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    let m = raw(
        PeKind::GetReply,
        b.muid(),
        a.muid(),
        rid,
        br#"{"status":200}"#,
        3,
        1,
        b"{",
    );
    assert!(matches!(a.rx(&m).0, Some(Ev::Msg { .. })));
    let m = raw(PeKind::GetReply, b.muid(), a.muid(), rid, b"", 3, 3, b"}");
    let (e, out) = a.rx(&m);
    assert_eq!(
        e,
        Some(Ev::Closed(
            rid,
            Direction::Initiator,
            CloseReason::OutOfSequence
        ))
    );
    // The Sub-ID#2 of the inquiry (0x34), as for every NAK of the transaction.
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (NakStatus::ChunksOutOfSequence, 0x34, pe(rid, 3))
    );
    assert_eq!(a.eng.open(), 0);
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
    // REQ-PE-END-010: the Request ID in the ACK details (and the last Chunk received).
    assert_eq!(
        k.details,
        Details::PropertyExchange {
            request_id: rid,
            chunk: 1
        }
    );
    let (e, _) = a.rx(&out[0]);
    assert_eq!(e, None);
    // REQ-PE-END-010: 6 s is Status Data 0x3C (Table 13 example); 12.7 s is the largest wait.
    // These ACKs do not go to A.
    for (ms, units) in [(6_000, 0x3C), (12_700, 0x7F)] {
        b.eng.wait(a.muid(), rid, Direction::Responder, ms).unwrap();
        let out = b.pending();
        assert_eq!(ack(&out[0]).status, AckStatus::TimeoutWait(units), "{ms}");
    }
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
fn ack_and_nak_of_another_device_are_ignored() {
    // REQ-PE-END-001: the transaction is found by the MUID of the sender and the Request ID.
    let (mut a, mut b) = pair(512);
    let mut c = Side::new(0x43, 0x0333_3333, 512, 0);
    let mut buf = [0u8; 600];
    let n = c.dev.discovery(0, 0, &mut buf).unwrap();
    let (_, r) = a.rx(&buf[..n]);
    c.rx(&r[0]);
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    assert_eq!(a.eng.open(), 1);
    let terminate = NakStatus::Terminate {
        by_initiator: false,
    };
    for m in [
        ack_from(&c, a.muid(), 0x34, AckStatus::TimeoutWait(10), rid, 1),
        nak_from(&c, a.muid(), 0x34, terminate, rid, 1),
    ] {
        let (e, out) = a.rx(&m);
        assert_eq!(e, None);
        assert!(out.is_empty());
        assert_eq!(a.eng.open(), 1);
    }
    // The same NAK from B ends the transaction.
    let (e, _) = a.rx(&nak_from(&b, a.muid(), 0x34, terminate, rid, 1));
    assert_eq!(
        e,
        Some(Ev::Closed(
            rid,
            Direction::Initiator,
            CloseReason::Terminated
        ))
    );
    assert_eq!(a.eng.open(), 0);
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
    // REQ-PE-FLOW-009: an ACK 0x11 for the last Chunk is ignored; the reply still ends it.
    let k = ack_from(&b, a.muid(), 0x36, AckStatus::FlowControlSendNext, rid, 3);
    assert_eq!(a.rx(&k), (None, vec![]));
    assert_eq!(a.eng.open(), 1);
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
            // REQ-PE-FLOW-003: the Sub-ID#2 of a Get inquiry, the Request ID and Chunk 1.
            assert_eq!(
                (ack(&out[0]).original_sub_id2, ack(&out[0]).details),
                (
                    0x34,
                    Details::PropertyExchange {
                        request_id: rid,
                        chunk: 1
                    }
                )
            );
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
        (n.status, n.original_sub_id2, n.details),
        (NakStatus::FlowControlResend, 0x36, pe(rid, 0))
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
    // REQ-PE-FLOW-011: NAK 0x12 (not 0x21) for the Set Inquiry 0x36.
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (NakStatus::FlowControlResend, 0x36, pe(rid, 1))
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
    // B ignores the later Chunks of that Request ID.
    let c2 = raw(PeKind::SetInquiry, src, dst, rid, b"", 2, 2, &data[room..]);
    assert_eq!(b.rx(&c2), (None, vec![]));
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
    // REQ-PE-RES-001: "requireResId":true needs a "resId".
    let rid_caps = ResourceCaps {
        require_res_id: true,
        ..caps()
    };
    assert_eq!(
        a.eng.request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(get("DeviceInfo"), 0),
            Some(&rid_caps),
            a.now
        ),
        Err(TransactionError::Refused(Status::BAD_REQUEST))
    );
    let with_id = Header {
        res_id: text("a"),
        ..get("DeviceInfo")
    };
    let info = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(with_id, 0),
            Some(&rid_caps),
            a.now,
        )
        .unwrap();
    let m = a.send(b.muid(), info.request_id, Direction::Initiator, &[], true);
    assert_eq!(header_text(&m), r#"{"resource":"DeviceInfo","resId":"a"}"#);
    a.eng
        .abort(&mut a.dev, b.muid(), info.request_id, Direction::Initiator)
        .unwrap();
    a.pending();
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
    // An <id> longer than 36 characters (REQ-PE-HDR-007).
    let long = format!("midi+jsonschema://{}", "s".repeat(37));
    assert_eq!(schema_ref(&long), Err(TransactionError::InvalidRef));
    // Table 88: the Header Data of the Inquiry: Get for the "$ref".
    let (mut a, b) = pair(512);
    let (id, _) = schema_ref("midi+jsonschema://patchSchema#/x").unwrap();
    let h = Header {
        res_id: Some(HeaderStr::Text(id)),
        ..get("JSONSchema")
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
    let m = a.send(b.muid(), info.request_id, Direction::Initiator, &[], true);
    assert_eq!(
        header_text(&m),
        r#"{"resource":"JSONSchema","resId":"patchSchema"}"#
    );
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
    // REQ-PE-SUB-006: the Reply to Subscription of a "start" has no Property Data.
    assert_eq!((chunk(&out[0]).total, chunk(&out[0]).data), (1, &b""[..]));
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
    assert_eq!(d, data);
    let mut r = PartialSetReader::<20>::new();
    let mut items = r.push(&d);
    let item = items.next_item().unwrap().unwrap();
    assert_eq!(
        (item.key, item.value),
        (
            "/name",
            ScalarValue::String {
                text: "Pad",
                truncated: false
            }
        )
    );
    assert!(items.next_item().is_none());
    r.finish().unwrap();
    // A "partial" whose data is not in the Partial SET format: the engine gives it with the
    // subscription and replies 200 (REQ-PE-SUB-016); the Partial SET reader refuses the key.
    let data = br#"{"name":"Pad"}"#;
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
    assert_eq!(status(&out[0]), 200);
    b.rx(&out[0]);
    let Some(Ev::Msg {
        data: d,
        sub: Some(s),
        ..
    }) = e
    else {
        panic!("{e:?}")
    };
    assert_eq!((s, d.as_slice()), (ai, &data[..]));
    let mut r = PartialSetReader::<20>::new();
    assert_eq!(
        r.push(&d).next_item().map(|i| i.map(|_| ())),
        Some(Err(zmidi_ci_pe::PropertyDataError::InvalidKey))
    );
    // The initiator ends it; B removes its entry and gives an event.
    let (info, entry) = a.eng.unsubscribe(&mut a.dev, ai, a.now).unwrap();
    let m = a.send(b.muid(), info.request_id, Direction::Initiator, &[], true);
    // REQ-PE-SUB-004, -017: "command", "subscribeId" and "endedBy", no Property Data.
    assert_eq!(
        header_text(&m),
        format!(
            r#"{{"command":"end","subscribeId":"{}","endedBy":"initiator"}}"#,
            entry.subscribe_id()
        )
    );
    assert_eq!(chunk(&m).data, b"");
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
    // Every message that the engines write, for REQ-PE-END-007.
    let mut sent: Vec<Vec<u8>> = Vec::new();
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
    let (e, out) = b.rx(&m);
    assert!(matches!(e, Some(Ev::Msg { .. })));
    sent.extend(out);
    b.now += 3_000;
    let (e, out) = b.poll();
    assert_eq!(e, None);
    sent.extend(out);
    b.now += 1;
    let (e, out) = b.poll();
    sent.extend(out.clone());
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
    let (rid, _, out) = get_request(&mut a, &mut b, "DeviceInfo");
    sent.extend(out);
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
    let (e, out) = a.rx(&m);
    assert!(matches!(
        e,
        Some(Ev::Msg {
            end: ChunkEnd::More,
            ..
        })
    ));
    sent.extend(out);
    let m = raw(PeKind::GetReply, dst, src, rid, b"", 2, 0, b"]");
    let (e, out) = a.rx(&m);
    sent.extend(out.clone());
    assert!(matches!(
        e,
        Some(Ev::Msg {
            end: ChunkEnd::Incomplete,
            ..
        })
    ));
    assert!(out.is_empty());
    assert_eq!(a.eng.open(), 0);
    // This crate never sends a Notify (REQ-PE-END-007): no message above has Sub-ID#2 0x3F.
    // At the deadline of the Set, the NAK 0x20 in place of a Notify 144; nothing else.
    assert_eq!(sent.len(), 1);
    assert_eq!(sub_id2(&sent[0]), 0x7F);
    assert!(sent.iter().all(|m| sub_id2(m) != 0x3F));
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
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (NakStatus::FlowControlResend, 0x36, pe(rid, 2))
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

/// A starts a Set of `len` bytes to `resource`; gives the Request ID and the Chunk parts.
fn set_parts(
    a: &mut Side,
    b: &Side,
    resource: &'static str,
    len: usize,
    caps: Option<&ResourceCaps>,
) -> (u8, Vec<Vec<u8>>) {
    let data = vec![b' '; len];
    let rid = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Set,
            &Outgoing::json(get(resource), len),
            caps,
            a.now,
        )
        .unwrap()
        .request_id;
    let room1 = a
        .eng
        .data_room(&a.dev, b.muid(), rid, Direction::Initiator)
        .unwrap();
    let mut parts = vec![data[..room1].to_vec()];
    for p in data[room1..].chunks(512 - 22) {
        parts.push(p.to_vec());
    }
    (rid, parts)
}

/// B starts a reply of `len` bytes to the Get `rid` of A; gives the room of Chunk 1.
fn get_reply(a: &Side, b: &mut Side, rid: u8, len: usize) -> usize {
    let r = Header {
        status: Some(Status::OK),
        ..Default::default()
    };
    b.eng
        .reply(&mut b.dev, a.muid(), rid, &Outgoing::json(r, len), b.now)
        .unwrap();
    b.eng
        .data_room(&b.dev, a.muid(), rid, Direction::Responder)
        .unwrap()
}

fn terminate(by_initiator: bool) -> NakStatus {
    NakStatus::Terminate { by_initiator }
}

fn pe(request_id: u8, chunk: u16) -> Details {
    Details::PropertyExchange { request_id, chunk }
}

#[test]
fn timeout_counts_from_the_last_chunk() {
    // REQ-PE-END-011: an ACK 0x10 that comes 1 s after the last Chunk adds its wait to the
    // timeout of that Chunk (3.3 s after it); it does not start the 3 s again. A NAK 0x00 that
    // answers an ACK does not start them again either. REQ-PE-END-012: a Notify 100 that comes
    // 1 s after the last Chunk adds 3 s (6 s after it). REQ-PE-END-007: the ACK 0x10 and the
    // NAK 0x42 are sent, no Notify. REQ-PE-GEN-005: the Request ID is free after the timeout.
    let (mut a, mut b) = pair(512);
    let mut sent: Vec<Vec<u8>> = Vec::new();
    let t0 = a.now;
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    b.eng
        .wait(a.muid(), rid, Direction::Responder, 250)
        .unwrap();
    let out = b.pending();
    sent.extend(out.clone());
    a.now = t0 + 1_000;
    assert_eq!(a.rx(&out[0]), (None, vec![]));
    let juce = nak_from(&b, a.muid(), 0x7D, NakStatus::Nak, rid, 0);
    assert_eq!(a.rx(&juce), (None, vec![]));
    a.now = t0 + 3_300;
    assert_eq!(a.poll(), (None, vec![]));
    a.now += 1;
    let (e, out) = a.poll();
    assert_eq!(
        e,
        Some(Ev::Closed(rid, Direction::Initiator, CloseReason::Timeout))
    );
    assert_eq!(
        (nak(&out[0]).status, nak(&out[0]).original_sub_id2),
        (NakStatus::Timeout, 0x34)
    );
    sent.extend(out.clone());
    b.rx(&out[0]);
    let t1 = a.now;
    let (rid2, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    assert_eq!(rid2, rid);
    a.now = t1 + 1_000;
    let m = raw(
        PeKind::Notify,
        b.muid(),
        a.muid(),
        rid2,
        br#"{"status":100}"#,
        1,
        1,
        b"",
    );
    assert_eq!(a.rx(&m), (None, vec![]));
    a.now = t1 + 6_000;
    assert_eq!(a.poll(), (None, vec![]));
    a.now += 1;
    let (e, out) = a.poll();
    assert_eq!(
        e,
        Some(Ev::Closed(rid2, Direction::Initiator, CloseReason::Timeout))
    );
    sent.extend(out);
    assert_eq!(
        sent.iter().map(|m| sub_id2(m)).collect::<Vec<_>>(),
        [0x7D, 0x7F, 0x7F]
    );
}

#[test]
fn reply_deadline_counts_from_the_last_inquiry_chunk_and_waits() {
    // REQ-PE-END-008: the deadline of the reply is 3 s after the last Chunk of the inquiry (here
    // 2 s after Chunk 1), plus the 300 ms of the ACK 0x10 that B sent; a reply after it gives
    // Late and is not sent.
    let (a, mut b) = pair(512);
    let (src, dst) = (a.muid(), b.muid());
    let t0 = b.now;
    for rid in [1u8, 2] {
        let m = raw(
            PeKind::SetInquiry,
            src,
            dst,
            rid,
            br#"{"resource":"X-Patch"}"#,
            2,
            1,
            b"{",
        );
        assert!(matches!(b.rx(&m).0, Some(Ev::Msg { .. })));
    }
    b.now = t0 + 2_000;
    for rid in [1u8, 2] {
        let m = raw(PeKind::SetInquiry, src, dst, rid, b"", 2, 2, b"}");
        assert!(matches!(
            b.rx(&m).0,
            Some(Ev::Msg {
                end: ChunkEnd::Complete,
                ..
            })
        ));
    }
    for rid in [1u8, 2] {
        b.eng.wait(src, rid, Direction::Responder, 250).unwrap();
    }
    assert_eq!(b.pending().len(), 2);
    let ok = Outgoing::json(
        Header {
            status: Some(Status::OK),
            ..Default::default()
        },
        0,
    );
    b.now = t0 + 5_300;
    b.eng.reply(&mut b.dev, src, 1, &ok, b.now).unwrap();
    let m = b.send(src, 1, Direction::Responder, &[], true);
    assert_eq!((sub_id2(&m), status(&m)), (0x37, 200));
    b.now += 1;
    assert_eq!(
        b.eng.reply(&mut b.dev, src, 2, &ok, b.now),
        Err(TransactionError::Late)
    );
    assert_eq!(b.pending(), Vec::<Vec<u8>>::new());
    assert_eq!(b.eng.open(), 0);
}

#[test]
fn next_chunk_deadline_of_the_sender() {
    // REQ-PE-END-009: the deadline of the next Chunk is 3 s after the Chunk before, plus the
    // wait of each ACK 0x10 of the responder. A Chunk after it gives Late and is not sent; the
    // transaction ends with NAK 0x20 (Status Data 0x01 as initiator, 0x00 as responder).
    let (mut a, mut b) = pair(512);
    let mut buf = [0u8; 600];
    let (rid, parts) = set_parts(&mut a, &b, "X-Patch", 700, None);
    assert_eq!(parts.len(), 2);
    let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    b.rx(&c1);
    a.now += 3_001;
    assert_eq!(
        a.eng.send(
            &mut a.dev,
            b.muid(),
            rid,
            Direction::Initiator,
            &parts[1],
            true,
            a.now,
            &mut buf
        ),
        Err(TransactionError::Late)
    );
    let out = a.pending();
    assert_eq!(out.len(), 1, "the NAK, not the Chunk");
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (terminate(true), 0x36, pe(rid, 0))
    );
    assert_eq!(
        b.rx(&out[0]).0,
        Some(Ev::Closed(
            rid,
            Direction::Responder,
            CloseReason::Terminated
        ))
    );
    // An ACK 0x10 of 500 ms from B, 100 ms after Chunk 1: the deadline is 3.5 s after Chunk 1.
    for (extra, late) in [(3_500u64, false), (3_501, true)] {
        let t = a.now;
        let (rid, parts) = set_parts(&mut a, &b, "X-Patch", 700, None);
        let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
        b.rx(&c1);
        b.eng
            .wait(a.muid(), rid, Direction::Responder, 500)
            .unwrap();
        let out = b.pending();
        assert_eq!(ack(&out[0]).status, AckStatus::TimeoutWait(5));
        a.now = t + 100;
        assert_eq!(a.rx(&out[0]).0, None);
        a.now = t + extra;
        let r = a.eng.send(
            &mut a.dev,
            b.muid(),
            rid,
            Direction::Initiator,
            &parts[1],
            true,
            a.now,
            &mut buf,
        );
        if late {
            assert_eq!(r, Err(TransactionError::Late));
            let out = a.pending();
            assert_eq!(nak(&out[0]).status, terminate(true));
            b.rx(&out[0]);
        } else {
            let n = r.unwrap();
            assert!(matches!(
                b.rx(&buf[..n]).0,
                Some(Ev::Msg {
                    end: ChunkEnd::Complete,
                    ..
                })
            ));
            b.eng
                .reply_status(&mut b.dev, a.muid(), rid, Status::OK, b.now)
                .unwrap();
            let out = b.pending();
            a.rx(&out[0]);
        }
    }
    assert_eq!((a.eng.open(), b.eng.open()), (0, 0));
    // The responder: a reply in 2 Chunks.
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    let data = vec![b' '; 700];
    let room = get_reply(&a, &mut b, rid, data.len());
    b.send(a.muid(), rid, Direction::Responder, &data[..room], false);
    b.now += 3_001;
    assert_eq!(
        b.eng.send(
            &mut b.dev,
            a.muid(),
            rid,
            Direction::Responder,
            &data[room..],
            true,
            b.now,
            &mut buf
        ),
        Err(TransactionError::Late)
    );
    let out = b.pending();
    assert_eq!(out.len(), 1, "the NAK, not the Chunk");
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (terminate(false), 0x34, pe(rid, 0))
    );
}

#[test]
fn responder_nak_0x20_stops_the_initiator() {
    // REQ-PE-END-002: NAK 0x20 with Status Data 0x00, the Sub-ID#2 of the inquiry and the
    // Request ID. REQ-PE-END-003: for 0x34 A gives no later reply Chunk to the caller, for 0x36
    // A sends no more Chunks; A closes with no reply and the Request ID is free at once.
    let (mut a, mut b) = pair(512);
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    let data = vec![b' '; 700];
    let room = get_reply(&a, &mut b, rid, data.len());
    let c1 = b.send(a.muid(), rid, Direction::Responder, &data[..room], false);
    assert!(matches!(
        a.rx(&c1).0,
        Some(Ev::Msg {
            end: ChunkEnd::More,
            ..
        })
    ));
    b.eng
        .abort(&mut b.dev, a.muid(), rid, Direction::Responder)
        .unwrap();
    let out = b.pending();
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (terminate(false), 0x34, pe(rid, 0))
    );
    assert_eq!(
        a.rx(&out[0]),
        (
            Some(Ev::Closed(
                rid,
                Direction::Initiator,
                CloseReason::Terminated
            )),
            vec![]
        )
    );
    let c2 = raw(PeKind::GetReply, b.muid(), a.muid(), rid, b"", 2, 2, b" ");
    assert_eq!(a.rx(&c2), (None, vec![]));
    let (rid2, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    assert_eq!(rid2, rid, "the Request ID is free again");
    // 0x36: A sends no more Chunks of its Set.
    let (rid, parts) = set_parts(&mut a, &b, "X-Patch", 700, None);
    let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    b.rx(&c1);
    b.eng
        .abort(&mut b.dev, a.muid(), rid, Direction::Responder)
        .unwrap();
    let out = b.pending();
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (terminate(false), 0x36, pe(rid, 0))
    );
    assert_eq!(
        a.rx(&out[0]),
        (
            Some(Ev::Closed(
                rid,
                Direction::Initiator,
                CloseReason::Terminated
            )),
            vec![]
        )
    );
    let mut buf = [0u8; 600];
    assert_eq!(
        a.eng.send(
            &mut a.dev,
            b.muid(),
            rid,
            Direction::Initiator,
            &parts[1],
            true,
            a.now,
            &mut buf
        ),
        Err(TransactionError::UnknownTransaction)
    );
}

#[test]
fn initiator_nak_0x20_stops_the_responder() {
    // REQ-PE-END-004: NAK 0x20 with Status Data 0x01, the Sub-ID#2 of the inquiry and the
    // Request ID. REQ-PE-END-005: for 0x34 B sends no more Chunks, for 0x36 B gives no later
    // Chunk to the caller; B closes with no reply.
    let (mut a, mut b) = pair(512);
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    let data = vec![b' '; 700];
    let room = get_reply(&a, &mut b, rid, data.len());
    let c1 = b.send(a.muid(), rid, Direction::Responder, &data[..room], false);
    a.rx(&c1);
    a.eng
        .abort(&mut a.dev, b.muid(), rid, Direction::Initiator)
        .unwrap();
    let out = a.pending();
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (terminate(true), 0x34, pe(rid, 0))
    );
    assert_eq!(
        b.rx(&out[0]),
        (
            Some(Ev::Closed(
                rid,
                Direction::Responder,
                CloseReason::Terminated
            )),
            vec![]
        )
    );
    let mut buf = [0u8; 600];
    assert_eq!(
        b.eng.send(
            &mut b.dev,
            a.muid(),
            rid,
            Direction::Responder,
            &data[room..],
            true,
            b.now,
            &mut buf
        ),
        Err(TransactionError::UnknownTransaction)
    );
    // 0x36: B gives no later Chunk of the Set to the caller.
    let (rid, parts) = set_parts(&mut a, &b, "X-Patch", 700, None);
    let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    assert!(matches!(b.rx(&c1).0, Some(Ev::Msg { .. })));
    a.eng
        .abort(&mut a.dev, b.muid(), rid, Direction::Initiator)
        .unwrap();
    let out = a.pending();
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (terminate(true), 0x36, pe(rid, 0))
    );
    assert_eq!(
        b.rx(&out[0]),
        (
            Some(Ev::Closed(
                rid,
                Direction::Responder,
                CloseReason::Terminated
            )),
            vec![]
        )
    );
    let c2 = raw(
        PeKind::SetInquiry,
        a.muid(),
        b.muid(),
        rid,
        b"",
        2,
        2,
        &parts[1],
    );
    assert_eq!(b.rx(&c2), (None, vec![]));
    assert_eq!((a.eng.open(), b.eng.open()), (0, 0));
}

#[test]
fn notify_144_stops_both_directions() {
    // REQ-PE-END-006: after a Notify 144, no later Chunk of that Request ID goes to the caller,
    // no more Chunks are sent, nothing answers it, and the Request ID is free.
    let (mut a, mut b) = pair(512);
    let notify = |from: Muid, to: Muid, rid: u8| {
        raw(
            PeKind::Notify,
            from,
            to,
            rid,
            br#"{"status":144}"#,
            1,
            1,
            b"",
        )
    };
    // During a reply: A receives no more.
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    let data = vec![b' '; 700];
    let room = get_reply(&a, &mut b, rid, data.len());
    let c1 = b.send(a.muid(), rid, Direction::Responder, &data[..room], false);
    a.rx(&c1);
    assert_eq!(
        a.rx(&notify(b.muid(), a.muid(), rid)),
        (
            Some(Ev::Closed(
                rid,
                Direction::Initiator,
                CloseReason::Terminated
            )),
            vec![]
        )
    );
    let c2 = b.send(a.muid(), rid, Direction::Responder, &data[room..], true);
    assert_eq!(a.rx(&c2), (None, vec![]));
    let (rid2, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    assert_eq!(rid2, rid, "the Request ID is free again");
    // During a Set: A sends no more; B (from a Notify of A) receives no more.
    let (rid, parts) = set_parts(&mut a, &b, "X-Patch", 700, None);
    let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    b.rx(&c1);
    assert_eq!(
        a.rx(&notify(b.muid(), a.muid(), rid)).0,
        Some(Ev::Closed(
            rid,
            Direction::Initiator,
            CloseReason::Terminated
        ))
    );
    let mut buf = [0u8; 600];
    assert_eq!(
        a.eng.send(
            &mut a.dev,
            b.muid(),
            rid,
            Direction::Initiator,
            &parts[1],
            true,
            a.now,
            &mut buf
        ),
        Err(TransactionError::UnknownTransaction)
    );
    assert_eq!(
        b.rx(&notify(a.muid(), b.muid(), rid)),
        (
            Some(Ev::Closed(
                rid,
                Direction::Responder,
                CloseReason::Terminated
            )),
            vec![]
        )
    );
    let c2 = raw(
        PeKind::SetInquiry,
        a.muid(),
        b.muid(),
        rid,
        b"",
        2,
        2,
        &parts[1],
    );
    assert_eq!(b.rx(&c2), (None, vec![]));
}

#[test]
fn nak_0x42_stops_the_responder() {
    // REQ-PE-END-013, -014: A times out during a reply in 2 Chunks and sends NAK 0x42; B closes
    // with no reply and sends no more Chunks.
    let (mut a, mut b) = pair(512);
    let (rid, _, _) = get_request(&mut a, &mut b, "DeviceInfo");
    let data = vec![b' '; 700];
    let room = get_reply(&a, &mut b, rid, data.len());
    let c1 = b.send(a.muid(), rid, Direction::Responder, &data[..room], false);
    a.rx(&c1);
    a.now += 3_001;
    let (e, out) = a.poll();
    assert_eq!(
        e,
        Some(Ev::Closed(rid, Direction::Initiator, CloseReason::Timeout))
    );
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (NakStatus::Timeout, 0x34, pe(rid, 0))
    );
    assert_eq!(
        b.rx(&out[0]),
        (
            Some(Ev::Closed(
                rid,
                Direction::Responder,
                CloseReason::PeerTimeout
            )),
            vec![]
        )
    );
    let mut buf = [0u8; 600];
    assert_eq!(
        b.eng.send(
            &mut b.dev,
            a.muid(),
            rid,
            Direction::Responder,
            &data[room..],
            true,
            b.now,
            &mut buf
        ),
        Err(TransactionError::UnknownTransaction)
    );
}

#[test]
fn flow_control_get_next_chunk_before_the_ack() {
    // REQ-PE-FLOW-005: A takes Chunk 2 before it sends the ACK of Chunk 1; then both ACKs go out
    // (REQ-PE-FLOW-003, -004: 0x34, the Request ID and each Chunk number).
    let (mut a, b) = pair(512);
    let h = Header {
        flow_control: true,
        ..get("X-Flow")
    };
    let rid = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(h, 0),
            None,
            a.now,
        )
        .unwrap()
        .request_id;
    a.send(b.muid(), rid, Direction::Initiator, &[], true);
    let c1 = raw(
        PeKind::GetReply,
        b.muid(),
        a.muid(),
        rid,
        br#"{"status":200}"#,
        2,
        1,
        b"ab",
    );
    let c2 = raw(PeKind::GetReply, b.muid(), a.muid(), rid, b"", 2, 2, b"cd");
    let Some(Ev::Msg { data, end, .. }) = a.rx_held(&c1) else {
        panic!()
    };
    assert_eq!((data.as_slice(), end), (&b"ab"[..], ChunkEnd::More));
    let Some(Ev::Msg { data, end, .. }) = a.rx_held(&c2) else {
        panic!()
    };
    assert_eq!((data.as_slice(), end), (&b"cd"[..], ChunkEnd::Complete));
    let out = a.pending();
    assert_eq!(out.len(), 2);
    for (i, m) in out.iter().enumerate() {
        let k = ack(m);
        assert_eq!(
            (k.status, k.original_sub_id2, k.details),
            (AckStatus::FlowControlSendNext, 0x34, pe(rid, i as u16 + 1))
        );
    }
    assert_eq!(a.eng.open(), 0);
}

#[test]
fn flow_control_get_responder_resends() {
    // REQ-PE-FLOW-012, -013 with the responder as the sender (a Get with "flowControl":true): a
    // NAK 0x12 gets the Chunk again; a NAK 0x12 for a Chunk that is not the last one sent ends
    // the transaction with NAK 0x20, Status Data 0x00; no ACK for 3 s gets the Chunk again, 3
    // times, then NAK 0x20 with Status Data 0x00.
    let (mut a, mut b) = pair(512);
    let data = vec![b'x'; 1100];
    let start = |a: &mut Side, b: &mut Side| -> (u8, usize) {
        let h = Header {
            flow_control: true,
            ..get("X-Flow")
        };
        let rid = a
            .eng
            .request(
                &mut a.dev,
                b.muid(),
                TxnKind::Get,
                &Outgoing::json(h, 0),
                None,
                a.now,
            )
            .unwrap()
            .request_id;
        let m = a.send(b.muid(), rid, Direction::Initiator, &[], true);
        b.rx(&m);
        (rid, get_reply(a, b, rid, 1100))
    };
    let (rid, room1) = start(&mut a, &mut b);
    let c1 = b.send(a.muid(), rid, Direction::Responder, &data[..room1], false);
    let resend = nak_from(&a, b.muid(), 0x34, NakStatus::FlowControlResend, rid, 0);
    assert_eq!(b.rx(&resend), (Some(Ev::Resend(rid, 1)), vec![]));
    assert_eq!(
        b.send(a.muid(), rid, Direction::Responder, &data[..room1], false),
        c1
    );
    let (_, out) = a.rx(&c1);
    assert_eq!(b.rx(&out[0]).0, Some(Ev::Ready(rid)));
    b.send(
        a.muid(),
        rid,
        Direction::Responder,
        &data[room1..room1 + 490],
        false,
    );
    // Chunk 1 again: it is not the last Chunk sent.
    let (e, out) = b.rx(&resend);
    assert_eq!(
        e,
        Some(Ev::Closed(
            rid,
            Direction::Responder,
            CloseReason::ResendFailed
        ))
    );
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (terminate(false), 0x34, pe(rid, 0))
    );
    // No ACK: 3 resends, then NAK 0x20.
    let (rid, room1) = start(&mut a, &mut b);
    let c1 = b.send(a.muid(), rid, Direction::Responder, &data[..room1], false);
    for k in 0..3 {
        b.now += 3_000;
        assert_eq!(b.poll(), (None, vec![]), "{k}");
        b.now += 1;
        assert_eq!(b.poll(), (Some(Ev::Resend(rid, 1)), vec![]), "{k}");
        assert_eq!(
            b.send(a.muid(), rid, Direction::Responder, &data[..room1], false),
            c1
        );
    }
    b.now += 3_001;
    let (e, out) = b.poll();
    assert_eq!(
        e,
        Some(Ev::Closed(rid, Direction::Responder, CloseReason::Timeout))
    );
    let n = nak(&out[0]);
    assert_eq!(
        (n.status, n.original_sub_id2, n.details),
        (terminate(false), 0x34, pe(rid, 0))
    );
}

#[test]
fn flow_control_waits() {
    // REQ-PE-FLOW-014. The sender counts the 3 s of REQ-PE-END-009 from the ACK 0x11 (1 s after
    // the Chunk). An ACK 0x10 of 500 ms adds to the ACK wait of the sender. The receiver closes
    // 12 s after its ACK 0x11, plus the 500 ms of the ACK 0x10 that it sent.
    let (mut a, mut b) = pair(512);
    let (rid, parts) = flow_set(&mut a, &mut b);
    let t = a.now;
    let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    let (_, out) = b.rx(&c1);
    a.now = t + 1_000;
    assert_eq!(a.rx(&out[0]).0, Some(Ev::Ready(rid)));
    a.now = t + 4_000;
    assert_eq!(a.poll(), (None, vec![]));
    a.now += 1;
    let (e, out) = a.poll();
    assert_eq!(
        e,
        Some(Ev::Closed(rid, Direction::Initiator, CloseReason::Timeout))
    );
    assert_eq!(nak(&out[0]).status, terminate(true));
    b.rx(&out[0]);
    let (rid, parts) = flow_set(&mut a, &mut b);
    let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    let (_, out) = b.rx(&c1);
    a.rx(&out[0]);
    let (ta, tb) = (a.now, b.now);
    let c2 = a.send(b.muid(), rid, Direction::Initiator, &parts[1], false);
    let (_, out) = b.rx(&c2);
    assert_eq!(ack(&out[0]).details, pe(rid, 2), "not given to A");
    b.eng
        .wait(a.muid(), rid, Direction::Responder, 500)
        .unwrap();
    let out = b.pending();
    let k = ack(&out[0]);
    assert_eq!(
        (k.status, k.original_sub_id2),
        (AckStatus::TimeoutWait(5), 0x36)
    );
    assert_eq!(a.rx(&out[0]), (None, vec![]));
    a.now = ta + 3_500;
    assert_eq!(a.poll(), (None, vec![]));
    a.now += 1;
    assert_eq!(a.poll(), (Some(Ev::Resend(rid, 2)), vec![]));
    b.now = tb + 12_500;
    assert_eq!(b.poll(), (None, vec![]));
    b.now += 1;
    let (e, out) = b.poll();
    assert_eq!(
        e,
        Some(Ev::Closed(rid, Direction::Responder, CloseReason::Timeout))
    );
    assert_eq!(nak(&out[0]).status, terminate(false));
}

#[test]
fn caller_refuses_set_data_with_413() {
    // REQ-PE-STAT-011: the caller refuses the Property Data of a Set as too large: B replies 413
    // with no Property Data and ignores the later Chunks.
    let (a, mut b) = pair(512);
    let (src, dst) = (a.muid(), b.muid());
    let m = raw(
        PeKind::SetInquiry,
        src,
        dst,
        7,
        br#"{"resource":"X-Patch"}"#,
        2,
        1,
        b"{",
    );
    assert!(matches!(
        b.rx(&m).0,
        Some(Ev::Msg {
            end: ChunkEnd::More,
            ..
        })
    ));
    b.eng
        .reply_status(&mut b.dev, src, 7, Status::PAYLOAD_TOO_LARGE, b.now)
        .unwrap();
    let out = b.pending();
    assert_eq!(out.len(), 1);
    assert_eq!((sub_id2(&out[0]), chunk(&out[0]).request_id), (0x37, 7));
    assert_eq!(
        header_text(&out[0]),
        r#"{"status":413,"message":"Payload too large"}"#
    );
    assert_eq!(chunk(&out[0]).data, b"");
    let m = raw(PeKind::SetInquiry, src, dst, 7, b"", 2, 2, b"}");
    assert_eq!(b.rx(&m), (None, vec![]));
    assert_eq!(b.eng.open(), 0);
}

#[test]
fn data_with_a_media_type() {
    // REQ-PE-ENC-002: data that is not JSON: the header has the "mediaType" that the caller
    // gives; with none, the request and the reply give NoMediaType. REQ-PE-ENC-001: the reader
    // gives such data with the "mediaType" value; with no "mediaType" the data is JSON.
    let (mut a, mut b) = pair(512);
    let bytes = [0x00u8, 0x7F, 0x01, 0x22];
    let h = Header {
        media_type: text("application/octet-stream"),
        ..get("X-Bin")
    };
    let msg = Outgoing {
        json: false,
        ..Outgoing::json(h, bytes.len())
    };
    let rid = a
        .eng
        .request(&mut a.dev, b.muid(), TxnKind::Set, &msg, None, a.now)
        .unwrap()
        .request_id;
    let m = a.send(b.muid(), rid, Direction::Initiator, &bytes, true);
    assert_eq!(
        header_text(&m),
        r#"{"resource":"X-Bin","mediaType":"application/octet-stream"}"#
    );
    let (e, out) = b.rx(&m);
    assert!(out.is_empty());
    let Some(Ev::Msg { media, data, .. }) = e else {
        panic!("{e:?}")
    };
    assert_eq!(
        (media.as_deref(), data.as_slice()),
        (Some("application/octet-stream"), &bytes[..])
    );
    b.eng
        .reply_status(&mut b.dev, a.muid(), rid, Status::OK, b.now)
        .unwrap();
    let out = b.pending();
    a.rx(&out[0]);
    // The reply to a Get.
    let (rid, e, _) = get_request(&mut a, &mut b, "X-Bin");
    assert!(matches!(e, Some(Ev::Msg { media: None, .. })));
    let r = Header {
        status: Some(Status::OK),
        ..Default::default()
    };
    assert_eq!(
        b.eng.reply(
            &mut b.dev,
            a.muid(),
            rid,
            &Outgoing {
                json: false,
                ..Outgoing::json(r, bytes.len())
            },
            b.now
        ),
        Err(TransactionError::NoMediaType)
    );
    let r = Header {
        media_type: text("application/octet-stream"),
        ..r
    };
    b.eng
        .reply(
            &mut b.dev,
            a.muid(),
            rid,
            &Outgoing {
                json: false,
                ..Outgoing::json(r, bytes.len())
            },
            b.now,
        )
        .unwrap();
    let m = b.send(a.muid(), rid, Direction::Responder, &bytes, true);
    assert_eq!(
        header_text(&m),
        r#"{"status":200,"mediaType":"application/octet-stream"}"#
    );
    let Some(Ev::Msg { media, data, .. }) = a.rx(&m).0 else {
        panic!()
    };
    assert_eq!(
        (media.as_deref(), data.as_slice()),
        (Some("application/octet-stream"), &bytes[..])
    );
}

#[test]
fn resource_list_get_ignores_offset_and_limit() {
    // REQ-PE-LIST-001: a Get for "ResourceList" with "offset" and "limit" is a normal request:
    // no error reply, and the reply needs no "totalCount".
    let (mut a, mut b) = pair(512);
    let m = raw(
        PeKind::GetInquiry,
        a.muid(),
        b.muid(),
        3,
        br#"{"resource":"ResourceList","offset":5,"limit":1}"#,
        1,
        1,
        b"",
    );
    let (e, out) = b.rx(&m);
    assert!(out.is_empty());
    assert!(matches!(
        e,
        Some(Ev::Msg {
            role: Direction::Responder,
            resource: None,
            end: ChunkEnd::Complete,
            ..
        })
    ));
    let r = Header {
        status: Some(Status::OK),
        ..Default::default()
    };
    b.eng
        .reply(&mut b.dev, a.muid(), 3, &Outgoing::json(r, 2), b.now)
        .unwrap();
    let m = b.send(a.muid(), 3, Direction::Responder, b"[]", true);
    assert_eq!(header_text(&m), r#"{"status":200}"#);
    let _ = a.rx(&m);
}

#[test]
fn initiator_flow_control_needs_the_resource_list_entry() {
    // REQ-PE-HDR-010: "flowControl":true is refused for a Get without "fcOnGet" and a Set
    // without "fcOnSet" (status 406), and written where the entry gives them.
    let (mut a, b) = pair(512);
    let info = resources()[0].caps;
    let patch = resources()[1].caps;
    let fc = resources()[2].caps;
    for (kind, resource, caps) in [
        (TxnKind::Get, "DeviceInfo", info),
        (TxnKind::Set, "X-Patch", patch),
    ] {
        let h = Header {
            flow_control: true,
            ..get(resource)
        };
        assert_eq!(
            a.eng.request(
                &mut a.dev,
                b.muid(),
                kind,
                &Outgoing::json(h, 2),
                Some(&caps),
                a.now
            ),
            Err(TransactionError::Refused(
                Status::FLOW_CONTROL_NOT_SUPPORTED
            )),
            "{resource}"
        );
    }
    assert_eq!(a.eng.open(), 0);
    let h = Header {
        flow_control: true,
        ..get("X-Flow")
    };
    let rid = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(h, 0),
            Some(&fc),
            a.now,
        )
        .unwrap()
        .request_id;
    let m = a.send(b.muid(), rid, Direction::Initiator, &[], true);
    assert_eq!(
        header_text(&m),
        r#"{"resource":"X-Flow","flowControl":true}"#
    );
}

#[test]
fn subscription_start_reply_errors_add_no_entry() {
    // REQ-PE-SUB-006: a reply outside 200-299, or with no "subscribeId", adds no entry and
    // gives the error; 201 with a "subscribeId" adds one.
    let (mut a, b) = pair(512);
    let g = get("X-Patch");
    for (reply, want) in [
        (
            &br#"{"status":405,"subscribeId":"s9"}"#[..],
            Err(SubscriptionError::NotSuccess),
        ),
        (br#"{"status":200}"#, Err(SubscriptionError::NoSubscribeId)),
        (br#"{"status":201,"subscribeId":"s9"}"#, Ok(0)),
    ] {
        let rid = a
            .eng
            .subscribe(&mut a.dev, b.muid(), &g, None, a.now)
            .unwrap()
            .request_id;
        a.send(b.muid(), rid, Direction::Initiator, &[], true);
        let m = raw(
            PeKind::SubscriptionReply,
            b.muid(),
            a.muid(),
            rid,
            reply,
            1,
            1,
            b"",
        );
        let (e, out) = a.rx(&m);
        assert!(out.is_empty());
        let Some(Ev::Msg { started, .. }) = e else {
            panic!("{e:?}")
        };
        assert_eq!(started, Some(want));
        assert_eq!(
            a.eng.subscriptions().entries().count(),
            usize::from(want.is_ok())
        );
        assert_eq!(a.eng.open(), 0);
    }
    let e = a.eng.subscriptions().get(0).unwrap();
    assert_eq!((e.subscribe_id(), e.resource()), ("s9", "X-Patch"));
}

#[test]
fn subscription_data_in_notify_and_end_is_ignored() {
    // REQ-PE-SUB-013: the Property Data of a received "notify" is ignored: 200, then the Get.
    // REQ-PE-SUB-017: the Property Data of a received "end" is ignored: 200, the entry ends.
    let (mut a, mut b) = pair(512);
    let (ai, bi) = subscribed(&mut a, &mut b);
    let id = b
        .eng
        .subscriptions()
        .get(bi)
        .unwrap()
        .subscribe_id()
        .to_string();
    let h = format!(r#"{{"command":"notify","subscribeId":"{id}"}}"#);
    let m = raw(
        PeKind::Subscription,
        b.muid(),
        a.muid(),
        50,
        h.as_bytes(),
        1,
        1,
        br#"{"x":1}"#,
    );
    let (e, out) = a.rx(&m);
    let Some(Ev::Msg { data, sub, .. }) = e else {
        panic!("{e:?}")
    };
    assert_eq!((data.as_slice(), sub), (&b""[..], Some(ai)));
    assert_eq!(out.len(), 2);
    assert_eq!((sub_id2(&out[0]), status(&out[0])), (0x39, 200));
    assert_eq!(
        (sub_id2(&out[1]), header_text(&out[1])),
        (
            0x34,
            String::from(r#"{"resource":"X-Patch","mutualEncoding":"Mcoded7"}"#)
        )
    );
    let h = format!(r#"{{"command":"end","subscribeId":"{id}","endedBy":"responder"}}"#);
    let m = raw(
        PeKind::Subscription,
        b.muid(),
        a.muid(),
        51,
        h.as_bytes(),
        1,
        1,
        b"{}",
    );
    let (e, out) = a.rx(&m);
    assert_eq!(e, Some(Ev::SubEnded(id)));
    assert_eq!(out.len(), 1);
    assert_eq!(header_text(&out[0]), r#"{"status":200}"#);
    assert_eq!(a.eng.subscriptions().entries().count(), 0);
}

#[test]
fn subscription_update_that_the_caller_can_not_apply() {
    // REQ-PE-SUB-016: A replies 200 to a "full" that its caller can not apply; the caller asks
    // again, and A sends a Get with the "start" values.
    let (mut a, mut b) = pair(512);
    let (ai, bi) = subscribed(&mut a, &mut b);
    let data = br#"{"name":"far too long for the caller"}"#;
    let rid = b
        .eng
        .update(
            &mut b.dev,
            bi,
            Command::Full,
            EncodingName::Ascii,
            Some(data.len()),
            b.now,
        )
        .unwrap()
        .request_id;
    let m = b.send(a.muid(), rid, Direction::Initiator, data, true);
    let (e, out) = a.rx(&m);
    assert!(matches!(e, Some(Ev::Msg { sub: Some(s), .. }) if s == ai));
    assert_eq!(out.len(), 1);
    assert_eq!((sub_id2(&out[0]), status(&out[0])), (0x39, 200));
    a.eng.refresh(ai).unwrap();
    let out = a.pending();
    assert_eq!(out.len(), 1);
    assert_eq!(sub_id2(&out[0]), 0x34);
    assert_eq!(
        header_text(&out[0]),
        r#"{"resource":"X-Patch","mutualEncoding":"Mcoded7"}"#
    );
}

// ----- limits found by mutation testing -----

#[test]
fn default_message_of_each_status() {
    // REQ-PE-STAT-006: the "message" of each status other than 200.
    for (s, m) in [
        (201, "Accepted"),
        (341, "Resource currently unavailable"),
        (342, "Bad data or unexpected end of data"),
        (343, "Too many requests"),
        (400, "Bad request"),
        (403, "Not authorized"),
        (404, "Resource not found"),
        (405, "Resource not allowed"),
        (406, "Flow Control not supported"),
        (407, "Flow Control is required"),
        (413, "Payload too large"),
        (415, "Unsupported media type or encoding"),
        (445, "Invalid version of data"),
        (500, "Internal device error"),
        (299, "Internal device error"),
    ] {
        assert_eq!(zmidi_ci_pe::default_message(Status(s)), m, "{s}");
    }
}

#[test]
fn schema_ref_id_of_36_characters() {
    // REQ-PE-HDR-007: 1 to 36 characters.
    let id = "a".repeat(36);
    let v = format!("midi+jsonschema://{id}");
    assert_eq!(schema_ref(&v), Ok((id.as_str(), None)));
    let v = format!("midi+jsonschema://{}", "a".repeat(37));
    assert_eq!(schema_ref(&v), Err(TransactionError::InvalidRef));
}

/// Gives a Chunk to B with a scratch buffer of `scratch` bytes.
fn rx_scratch(b: &mut Side, msg: &[u8], scratch: usize) -> Result<Option<Ev>, TransactionError> {
    let ev = b
        .dev
        .receive(0, msg, b.now * 1000, &mut rng(7))
        .event()
        .unwrap();
    let mut buf = vec![0u8; scratch];
    b.eng
        .receive(&mut b.dev, &ev, b.now, &mut buf)
        .map(|e| e.map(own))
}

#[test]
fn scratch_of_the_data_length_and_header_of_h_bytes() {
    // The scratch buffer needs the length of the Property Data, no more.
    let (a, mut b) = pair(512);
    let data = br#"{"v":1}"#;
    let m = raw(
        PeKind::SetInquiry,
        a.muid(),
        b.muid(),
        1,
        br#"{"resource":"X-Patch"}"#,
        1,
        1,
        data,
    );
    assert_eq!(
        rx_scratch(&mut b, &m, data.len() - 1),
        Err(TransactionError::Size)
    );
    let e = rx_scratch(&mut b, &m, data.len()).unwrap();
    assert!(matches!(e, Some(Ev::Msg { .. })), "{e:?}");
    // REQ-PE-STAT-011: a header of H = 128 bytes is read; 129 bytes get 413.
    for (len, refused) in [(128, false), (129, true)] {
        let (a, mut b) = pair(512);
        let fixed = r#"{"resource":"DeviceInfo","x":""}"#.len();
        let h = format!(
            r#"{{"resource":"DeviceInfo","x":"{}"}}"#,
            "p".repeat(len - fixed)
        );
        assert_eq!(h.len(), len);
        let m = raw(
            PeKind::GetInquiry,
            a.muid(),
            b.muid(),
            1,
            h.as_bytes(),
            1,
            1,
            b"",
        );
        let (e, out) = b.rx(&m);
        if refused {
            assert_eq!((e, status(&out[0])), (None, 413));
        } else {
            assert!(
                matches!(
                    e,
                    Some(Ev::Msg {
                        resource: Some(0),
                        ..
                    })
                ),
                "{e:?}"
            );
        }
    }
}

#[test]
fn json_media_type_on_a_resource_with_no_media_types() {
    // REQ-PE-ENC-001, REQ-PE-STAT-012: with no "mediaTypes" a Resource takes
    // "application/json" only.
    for (media, ok) in [("application/json", true), ("text/plain", false)] {
        let (a, mut b) = pair(512);
        let h = format!(r#"{{"resource":"X-Patch","mediaType":"{media}"}}"#);
        let m = raw(
            PeKind::SetInquiry,
            a.muid(),
            b.muid(),
            1,
            h.as_bytes(),
            1,
            1,
            b"{}",
        );
        let (e, out) = b.rx(&m);
        if ok {
            assert!(
                matches!(
                    e,
                    Some(Ev::Msg {
                        resource: Some(1),
                        ..
                    })
                ),
                "{e:?}"
            );
        } else {
            assert_eq!((e, status(&out[0])), (None, 415));
        }
    }
}

#[test]
fn request_rules_of_the_initiator() {
    let (mut a, b) = pair(512);
    let end_with_data = Header {
        command: Some(Command::End),
        subscribe_id: text("s1"),
        ..get("X-Patch")
    };
    // REQ-PE-SUB-005: an "end" has no Property Data.
    let r = a.eng.request(
        &mut a.dev,
        b.muid(),
        TxnKind::Subscription,
        &Outgoing::json(end_with_data, 3),
        None,
        a.now,
    );
    assert_eq!(
        r.err(),
        Some(TransactionError::Subscription(
            SubscriptionError::NotPermitted
        ))
    );
    // REQ-PE-FLOW-008: "fcOnSet" sets "flowControl" on a Set only.
    let fc = ResourceCaps {
        fc_on_set: true,
        ..caps()
    };
    let info = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(get("X-Flow"), 0),
            Some(&fc),
            a.now,
        )
        .unwrap();
    let m = a.send(b.muid(), info.request_id, Direction::Initiator, &[], true);
    assert_eq!(header_text(&m), r#"{"resource":"X-Flow"}"#);
    // A ResourceList request is not missing the ResourceList.
    let (mut a, b) = pair(512);
    let info = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(get("ResourceList"), 0),
            None,
            a.now,
        )
        .unwrap();
    assert!(!info.resource_list_missing);
}

/// Discovery and the PE Capabilities between `a` and `b`, as `pair` does.
fn link(a: &mut Side, b: &mut Side) {
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
}

/// A gets the ResourceList of `b` (an empty list).
fn list_from(a: &mut Side, b: &mut Side) {
    let mut buf = [0u8; 600];
    let (rid, n) = a
        .eng
        .request_resource_list(&mut a.dev, b.muid(), a.now, &mut buf)
        .unwrap();
    b.rx(&buf[..n]);
    let ok = Header {
        status: Some(Status::OK),
        ..Default::default()
    };
    b.eng
        .reply(&mut b.dev, a.muid(), rid, &Outgoing::json(ok, 2), b.now)
        .unwrap();
    let m = b.send(a.muid(), rid, Direction::Responder, b"[]", true);
    a.rx(&m);
    assert!(a.eng.has_resource_list(b.muid()));
}

/// An open Get of A to `b`; gives its Request ID.
fn open_get(a: &mut Side, b: &Side) -> u8 {
    a.eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(get("DeviceInfo"), 0),
            None,
            a.now,
        )
        .unwrap()
        .request_id
}

#[test]
fn invalidation_closes_the_transactions_of_its_muid_only() {
    // REQ-PE-SUB-019, REQ-CI-INV-005: a peer MUID ends that peer's transactions and
    // ResourceList state; a new MUID of this Device ends all of them.
    let (mut a, mut b) = pair(512);
    let mut c = Side::new(0x43, 0x0333_3333, 512, 1024);
    link(&mut a, &mut c);
    list_from(&mut a, &mut b);
    list_from(&mut a, &mut c);
    let rb = open_get(&mut a, &b);
    open_get(&mut a, &c);
    assert_eq!(a.eng.open(), 2);
    let mut buf = [0u8; 64];
    let n = c.dev.invalidate(&mut rng(9), &mut buf).unwrap();
    let old_c = zmidi_ci::Header::parse(&buf[..n]).unwrap().0.source;
    let (e, _) = a.rx(&buf[..n]);
    assert_eq!(e, Some(Ev::Invalidated(0)));
    assert_eq!(a.eng.open(), 1);
    assert!(a.eng.has_resource_list(b.muid()));
    assert!(!a.eng.has_resource_list(old_c));
    // B's transaction is the one still open.
    assert!(a
        .eng
        .abort(&mut a.dev, b.muid(), rb, Direction::Initiator)
        .is_ok());
    open_get(&mut a, &b);
    // A new MUID of A (REQ-CI-INV-005): all transactions and ResourceList states end.
    let n = zmidi_ci::InvalidateMuid { target: a.muid() }
        .write(b.muid(), Framing::Bare, &mut buf)
        .unwrap();
    let (e, _) = a.rx(&buf[..n]);
    assert_eq!(e, Some(Ev::Invalidated(0)));
    assert_eq!(a.eng.open(), 0);
    assert!(!a.eng.has_resource_list(b.muid()));
}

#[test]
fn open_request_limit_is_for_each_peer() {
    // REQ-PE-GEN-003: the Number of Simultaneous Requests of one peer.
    let (mut a, b) = pair(512);
    let mut c = Side::new(0x43, 0x0333_3333, 512, 1024);
    link(&mut a, &mut c);
    let mut host = Limited(&mut a.dev, 1);
    let req = |eng: &mut Eng, host: &mut Limited<'_>, peer| {
        eng.request(
            host,
            peer,
            TxnKind::Get,
            &Outgoing::json(get("DeviceInfo"), 0),
            None,
            1_000,
        )
        .map(|_| ())
    };
    assert_eq!(req(&mut a.eng, &mut host, b.muid()), Ok(()));
    assert_eq!(
        req(&mut a.eng, &mut host, b.muid()),
        Err(TransactionError::PeerBusy)
    );
    assert_eq!(req(&mut a.eng, &mut host, c.muid()), Ok(()));
}

#[test]
fn unsent_request_times_out_with_no_message() {
    // Nothing was sent: the peer does not know the transaction, so no NAK goes out.
    let (mut a, b) = pair(512);
    let rid = open_get(&mut a, &b);
    a.now += zmidi_ci_pe::transaction::TIMEOUT_MS;
    assert_eq!(a.poll(), (None, vec![]));
    a.now += 1;
    let (e, out) = a.poll();
    assert_eq!(
        e,
        Some(Ev::Closed(rid, Direction::Initiator, CloseReason::Timeout))
    );
    assert!(out.is_empty(), "{out:?}");
}

#[test]
fn each_retransmit_waits_a_full_timeout() {
    // REQ-PE-FLOW-013, -014: the Chunk goes again after each timeout with no ACK.
    let (mut a, mut b) = pair(512);
    let (rid, parts) = flow_set(&mut a, &mut b);
    a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    for _ in 0..2 {
        a.now += zmidi_ci_pe::transaction::TIMEOUT_MS;
        assert_eq!(a.poll().0, None);
        a.now += 1;
        assert_eq!(a.poll().0, Some(Ev::Resend(rid, 1)));
        a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    }
}

/// A host of `dev` with SysEx framing.
struct SysExHost<'d>(&'d mut Dev);

impl PeHost for SysExHost<'_> {
    fn muid(&self) -> Muid {
        PeHost::muid(&*self.0)
    }
    fn framing(&self) -> Framing {
        Framing::SysEx
    }
    fn peer_max_sysex(&self, peer: Muid) -> u32 {
        self.0.peer_max_sysex(peer)
    }
    fn peer_simultaneous(&self, peer: Muid) -> Option<u8> {
        self.0.peer_simultaneous(peer)
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

#[test]
fn chunk_count_follows_the_room_of_the_framing() {
    // A message of 512 bytes holds 512 - 22 bytes of header and data, 2 fewer with the
    // SysEx framing bytes. Header {"resource":"X-Patch"} is 22 bytes.
    let (mut a, b) = pair(512);
    fn chunks<D: PeHost>(eng: &mut Eng, host: &mut D, peer: Muid, len: usize) -> u16 {
        let msg = Outgoing::json(get("X-Patch"), len);
        let rid = eng
            .request(host, peer, TxnKind::Set, &msg, None, 1_000)
            .unwrap()
            .request_id;
        let room = eng
            .data_room(host, peer, rid, Direction::Initiator)
            .unwrap();
        let mut buf = [0u8; 600];
        let part = vec![b'1'; room.min(len)];
        let n = eng
            .send(
                host,
                peer,
                rid,
                Direction::Initiator,
                &part,
                len <= room,
                1_000,
                &mut buf,
            )
            .unwrap();
        eng.abort(host, peer, rid, Direction::Initiator).ok();
        let mut out = [0u8; 600];
        while eng.write_pending(host, 1_000, &mut out).is_some() {}
        let (h, body) = zmidi_ci::Header::parse(&buf[..n]).unwrap();
        PeChunk::read(&h, body).unwrap().total
    }
    let peer = b.muid();
    let total = |a: &mut Side, sysex: bool, len: usize| {
        if sysex {
            chunks(&mut a.eng, &mut SysExHost(&mut a.dev), peer, len)
        } else {
            chunks(&mut a.eng, &mut a.dev, peer, len)
        }
    };
    assert_eq!(total(&mut a, false, 468), 1);
    assert_eq!(total(&mut a, false, 469), 2);
    assert_eq!(total(&mut a, true, 466), 1);
    assert_eq!(total(&mut a, true, 467), 2);
    assert_eq!(total(&mut a, false, 468 + 2 * 490), 3);
    assert_eq!(total(&mut a, false, 468 + 2 * 490 + 1), 4);
}

#[test]
fn nak_finds_the_transaction_of_its_role_only() {
    // REQ-PE-END-001, -003: NAK 0x20 "by responder" and NAK 0x12 of a Get reply are for the
    // Initiator and the data sender: a transaction in the other role keeps going.
    let (mut a, mut b) = pair(512);
    let (rid, _, _) = get_request(&mut b, &mut a, "DeviceInfo");
    assert_eq!(a.eng.open(), 1);
    let m = nak_from(&b, a.muid(), 0x34, terminate(false), rid, 0);
    assert_eq!(a.rx(&m), (None, vec![]));
    assert_eq!(a.eng.open(), 1);
    // A Get with flow control that A did not send yet: a NAK 0x12 for the Get data is for
    // the Responder, so A's request stays.
    let g = Header {
        flow_control: true,
        ..get("X-Flow")
    };
    let rid = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Get,
            &Outgoing::json(g, 0),
            None,
            a.now,
        )
        .unwrap()
        .request_id;
    let m = nak_from(&b, a.muid(), 0x34, NakStatus::FlowControlResend, rid, 0);
    assert_eq!(a.rx(&m), (None, vec![]));
    assert_eq!(a.eng.open(), 2);
}

#[test]
fn ack_counts_once_for_the_last_chunk() {
    // REQ-PE-FLOW-002: the ACK of the last Chunk sent, once.
    let (mut a, mut b) = pair(512);
    let (rid, parts) = flow_set(&mut a, &mut b);
    let c1 = a.send(b.muid(), rid, Direction::Initiator, &parts[0], false);
    let (_, out) = b.rx(&c1);
    assert_eq!(a.rx(&out[0]).0, Some(Ev::Ready(rid)));
    assert_eq!(a.rx(&out[0]).0, None, "the same ACK again");
    a.send(b.muid(), rid, Direction::Initiator, &parts[1], false);
    let other = ack_from(&b, a.muid(), 0x36, AckStatus::FlowControlSendNext, rid, 3);
    assert_eq!(a.rx(&other).0, None, "the ACK of another Chunk");
}

#[test]
fn resend_nak_needs_flow_control() {
    // REQ-PE-FLOW-010, -012: NAK 0x12 is for a transaction with flow control. With no flow
    // control it is ignored.
    let (mut a, b) = pair(512);
    let data = vec![b'1'; 700];
    let info = a
        .eng
        .request(
            &mut a.dev,
            b.muid(),
            TxnKind::Set,
            &Outgoing::json(get("X-Patch"), data.len()),
            None,
            a.now,
        )
        .unwrap();
    let rid = info.request_id;
    let room = a
        .eng
        .data_room(&a.dev, b.muid(), rid, Direction::Initiator)
        .unwrap();
    a.send(b.muid(), rid, Direction::Initiator, &data[..room], false);
    let m = nak_from(&b, a.muid(), 0x36, NakStatus::FlowControlResend, rid, 0);
    assert_eq!(a.rx(&m), (None, vec![]));
    assert_eq!(a.eng.open(), 1);
}

#[test]
fn chunk_1_again_after_the_inquiry_is_ignored() {
    // REQ-PE-FLOW-007: a Chunk of a transaction that is not receiving changes nothing.
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
    let m = a.send(b.muid(), info.request_id, Direction::Initiator, &[], true);
    assert!(matches!(b.rx(&m).0, Some(Ev::Msg { .. })));
    assert_eq!(b.rx(&m), (None, vec![]));
    assert_eq!(b.eng.open(), 1);
}

#[test]
fn get_after_notify_waits_for_a_free_request_of_that_peer() {
    // REQ-PE-SUB-014: the Get waits while the peer has its maximum of open requests from
    // this Device; requests to other peers and requests of the peer do not count.
    let (mut a, mut b) = pair(512);
    let mut c = Side::new(0x43, 0x0333_3333, 512, 1024);
    link(&mut a, &mut c);
    let (_, bi) = subscribed(&mut a, &mut b);
    // A request of B to A (A is its Responder) and a request of A to C.
    get_request(&mut b, &mut a, "DeviceInfo");
    open_get(&mut a, &c);
    let rid_b = open_get(&mut a, &b);
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
    let ev = a
        .dev
        .receive(0, &m, a.now * 1000, &mut rng(7))
        .event()
        .unwrap();
    let mut s = [0u8; 600];
    let e = a
        .eng
        .receive(&mut Limited(&mut a.dev, 1), &ev, a.now, &mut s)
        .unwrap()
        .map(own);
    assert!(matches!(e, Some(Ev::Msg { .. })), "{e:?}");
    let pending = |a: &mut Side| {
        let mut out = Vec::new();
        let mut buf = [0u8; 600];
        let mut host = Limited(&mut a.dev, 1);
        while let Some(r) = a.eng.write_pending(&mut host, a.now, &mut buf) {
            out.push(sub_id2(&buf[..r.unwrap()]));
        }
        out
    };
    assert_eq!(pending(&mut a), [0x39], "the 200 reply; the Get waits");
    a.eng
        .abort(&mut a.dev, b.muid(), rid_b, Direction::Initiator)
        .unwrap();
    assert_eq!(
        pending(&mut a).last(),
        Some(&0x34),
        "the Get after the abort"
    );
}

#[test]
fn engine_host_framing_is_the_device_framing() {
    let cfg = DeviceConfig {
        framing: Framing::SysEx,
        ..config(0x41, 512)
    };
    let dev: Dev = CiDevice::new(cfg, &mut rng(1)).unwrap();
    assert_eq!(PeHost::framing(&dev), Framing::SysEx);
}

#[test]
fn subscription_commands_with_data_are_refused() {
    // REQ-PE-SUB-005, -013: "start" and "notify" carry no Property Data.
    let (mut a, b) = pair(512);
    for (command, sub_id) in [(Command::Start, None), (Command::Notify, text("s1"))] {
        let h = Header {
            command: Some(command),
            subscribe_id: sub_id,
            ..get("X-Patch")
        };
        let r = a.eng.request(
            &mut a.dev,
            b.muid(),
            TxnKind::Subscription,
            &Outgoing::json(h, 3),
            None,
            a.now,
        );
        assert_eq!(
            r.err(),
            Some(TransactionError::Subscription(
                SubscriptionError::NotPermitted
            )),
            "{command:?}"
        );
    }
}

#[test]
fn resource_list_states_replace_the_oldest_when_full() {
    // The engine keeps the ResourceList state of P peers; with P = 1 the next peer takes the
    // place.
    type Eng1 = PeEngine<'static, 4, 128, 4, 1>;
    fn list1(e1: &mut Eng1, a: &mut Side, b: &mut Side) {
        let mut buf = [0u8; 600];
        let (rid, n) = e1
            .request_resource_list(&mut a.dev, b.muid(), a.now, &mut buf)
            .unwrap();
        b.rx(&buf[..n]);
        let ok = Header {
            status: Some(Status::OK),
            ..Default::default()
        };
        b.eng
            .reply(&mut b.dev, a.muid(), rid, &Outgoing::json(ok, 2), b.now)
            .unwrap();
        let m = b.send(a.muid(), rid, Direction::Responder, b"[]", true);
        let ev = a
            .dev
            .receive(0, &m, a.now * 1000, &mut rng(7))
            .event()
            .unwrap();
        let mut s = [0u8; 600];
        e1.receive(&mut a.dev, &ev, a.now, &mut s).unwrap();
    }
    let (mut a, mut b) = pair(512);
    let mut c = Side::new(0x43, 0x0333_3333, 512, 1024);
    link(&mut a, &mut c);
    let mut e1 = Eng1::new(EngineConfig {
        resources: resources(),
        zlib_window: 0,
    });
    list1(&mut e1, &mut a, &mut b);
    assert!(e1.has_resource_list(b.muid()));
    list1(&mut e1, &mut a, &mut c);
    assert!(e1.has_resource_list(c.muid()));
    assert!(!e1.has_resource_list(b.muid()));
}
