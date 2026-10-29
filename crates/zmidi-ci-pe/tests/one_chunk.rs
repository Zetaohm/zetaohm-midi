//! The one-chunk Property Exchange Responder (DEC-PE-005): two zmidi-ci Devices; Device A sends
//! Property Exchange Chunks, Device B answers them with `one_chunk::respond`.

use zmidi_ci::{
    CiDevice, DeviceConfig, DeviceIdentity, Discovery, Event, Framing, ManufacturerId, PeChunk,
    PeKind, PiCapabilities,
};
use zmidi_ci_pe::one_chunk::{respond, respond_flat, Resources};
use zmidi_ci_pe::{Header, HeaderKind, Status};

type Dev = CiDevice<'static, 4, 2, 4>;

fn config(m: u8) -> DeviceConfig<'static> {
    DeviceConfig {
        identity: DeviceIdentity {
            manufacturer: ManufacturerId::OneByte(m),
            family: 1,
            model: 2,
            revision: [0, 0, 0, 1],
        },
        categories: zmidi_ci::Categories {
            property_exchange: true,
            ..Default::default()
        },
        max_sysex_size: 512,
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

/// One Resource "X-State" with a value of up to 64 bytes.
#[derive(Default)]
struct App {
    state: Vec<u8>,
    sets: usize,
}

impl Resources for App {
    fn get(
        &mut self,
        resource: &[u8],
        res_id: Option<&[u8]>,
        out: &mut [u8],
    ) -> Result<usize, Status> {
        if resource != b"X-State" || res_id.is_some_and(|r| r != b"a") {
            return Err(Status::NOT_FOUND);
        }
        let n = self.state.len().min(out.len());
        out[..n].copy_from_slice(&self.state[..n]);
        Ok(n)
    }

    fn set(&mut self, resource: &[u8], _res_id: Option<&[u8]>, data: &[u8]) -> Result<(), Status> {
        if resource != b"X-State" {
            return Err(Status::NOT_FOUND);
        }
        if data.len() > 64 {
            return Err(Status::PAYLOAD_TOO_LARGE);
        }
        self.state = data.to_vec();
        self.sets += 1;
        Ok(())
    }
}

const LIST: &[u8] = br#"[{"resource":"DeviceInfo"},{"resource":"X-State","canSet":"full"}]"#;

struct Pair {
    a: Dev,
    b: Dev,
    app: App,
    /// B answers with `respond_flat` in place of `respond` (REQ-PE-ONE-005).
    flat: bool,
}

impl Pair {
    fn new() -> Self {
        let mut a = Dev::new(config(0x41), &mut || 0x0111_1111).unwrap();
        let mut b = Dev::new(config(0x42), &mut || 0x0222_2222).unwrap();
        let mut buf = [0u8; 600];
        // From the message writer: a Device without the feature `initiator` (DEC-CI-001 of zmidi-ci)
        // does not send Discovery, and the one-chunk Responder does not need it.
        let n = Discovery {
            identity: config(0x41).identity,
            categories: config(0x41).categories,
            max_sysex_size: 512,
            output_path_id: Some(0),
        }
        .write(a.muid(), Framing::Bare, &mut buf)
        .unwrap();
        let discovery = buf[..n].to_vec();
        let mut reply = Vec::new();
        {
            let mut r = b.receive(0, &discovery, 0, &mut || 7);
            while let Some(Ok((_, k))) = r.write_next(&mut buf) {
                reply = buf[..k].to_vec();
            }
        }
        a.receive(0, &reply, 0, &mut || 7);
        Pair {
            a,
            b,
            app: App::default(),
            flat: false,
        }
    }

    /// Delivers `msg` to B and gives the status and the reply messages of `respond`.
    fn send(&mut self, msg: &[u8]) -> (Option<Status>, Vec<Vec<u8>>) {
        let mut sent = Vec::new();
        let mut body = [0u8; 256];
        let mut out = [0u8; 600];
        let r = self.b.receive(0, msg, 0, &mut || 7);
        let Some(ev) = r.event() else {
            return (None, sent);
        };
        let send = |m: &[u8]| sent.push(m.to_vec());
        let status = if self.flat {
            respond_flat(
                &mut self.b,
                &ev,
                LIST,
                &mut self.app,
                &mut body,
                &mut out,
                send,
            )
        } else {
            respond(
                &mut self.b,
                &ev,
                LIST,
                &mut self.app,
                &mut body,
                &mut out,
                send,
            )
        };
        (status, sent)
    }

    fn chunk(
        &self,
        kind: PeKind,
        rid: u8,
        header: &[u8],
        total: u16,
        number: u16,
        data: &[u8],
    ) -> Vec<u8> {
        let mut buf = [0u8; 600];
        // The zmidi-ci writer writes an Inquiry: Get Property Data only as Chunk 1 of 1 with no
        // Property Data (M2-101 Table 33). The Responder must also answer a peer that breaks
        // this rule, so such a Chunk is written as a Set and its Sub-ID#2 is changed to 0x34.
        let odd_get = kind == PeKind::GetInquiry && (total, number, data.len()) != (1, 1, 0);
        let as_kind = if odd_get { PeKind::SetInquiry } else { kind };
        let n = PeChunk {
            request_id: rid,
            header,
            total,
            number,
            data,
        }
        .write(
            as_kind,
            self.a.muid(),
            self.b.muid(),
            Framing::Bare,
            &mut buf,
        )
        .unwrap();
        if odd_get {
            buf[3] = PeKind::GetInquiry.sub_id2();
        }
        buf[..n].to_vec()
    }
}

/// The kind, Request ID, status and Property Data of one reply message.
fn reply(msg: &[u8]) -> (u8, u8, u16, Vec<u8>) {
    let (h, body) = zmidi_ci::Header::parse(msg).unwrap();
    let c = PeChunk::read(&h, body).unwrap();
    let status = Header::parse(HeaderKind::Reply, c.header)
        .unwrap()
        .status
        .unwrap()
        .0;
    assert_eq!((c.total, c.number), (1, 1), "one Chunk");
    (h.sub_id2, c.request_id, status, c.data.to_vec())
}

#[test]
fn get_and_set_in_one_chunk() {
    // REQ-PE-ONE-001: Get and Set in one Chunk; the ResourceList from the constant.
    let mut p = Pair::new();
    let set = p.chunk(
        PeKind::SetInquiry,
        3,
        br#"{"resource":"X-State"}"#,
        1,
        1,
        b"[1,2,3]",
    );
    let (s, r) = p.send(&set);
    assert_eq!(s, Some(Status::OK));
    assert_eq!(reply(&r[0]), (0x37, 3, 200, vec![]));
    let get = p.chunk(
        PeKind::GetInquiry,
        4,
        br#"{ "resource" : "X-State" }"#,
        1,
        1,
        b"",
    );
    let (_, r) = p.send(&get);
    assert_eq!(reply(&r[0]), (0x35, 4, 200, b"[1,2,3]".to_vec()));
    let list = p.chunk(
        PeKind::GetInquiry,
        5,
        br#"{"resource":"ResourceList"}"#,
        1,
        1,
        b"",
    );
    let (_, r) = p.send(&list);
    assert_eq!(reply(&r[0]), (0x35, 5, 200, LIST.to_vec()));
    // Escapes in a header string are decoded (RFC 8259 7).
    let escaped = p.chunk(
        PeKind::GetInquiry,
        6,
        br#"{"resource":"X-State","resId":"a"}"#,
        1,
        1,
        b"",
    );
    assert_eq!(reply(&p.send(&escaped).1[0]).2, 200);
    // The status of the application.
    let other = p.chunk(
        PeKind::GetInquiry,
        7,
        br#"{"resource":"X-Other"}"#,
        1,
        1,
        b"",
    );
    assert_eq!(reply(&p.send(&other).1[0]).2, 404);
}

#[test]
fn request_in_more_chunks_gets_413_once() {
    // REQ-PE-ONE-002: Chunk 1 of 2 gets 413; Chunk 2 gets no reply; the application gets
    // nothing.
    let mut p = Pair::new();
    let c1 = p.chunk(
        PeKind::SetInquiry,
        9,
        br#"{"resource":"X-State"}"#,
        2,
        1,
        b"[1,",
    );
    let (s, r) = p.send(&c1);
    assert_eq!(s, Some(Status::PAYLOAD_TOO_LARGE));
    assert_eq!(reply(&r[0]), (0x37, 9, 413, vec![]));
    let c2 = p.chunk(PeKind::SetInquiry, 9, b"", 2, 2, b"2]");
    assert_eq!(p.send(&c2), (None, vec![]));
    // Number of Chunks 0 (not known) is not one Chunk either.
    let c0 = p.chunk(
        PeKind::GetInquiry,
        10,
        br#"{"resource":"X-State"}"#,
        0,
        1,
        b"",
    );
    assert_eq!(reply(&p.send(&c0).1[0]).2, 413);
    assert_eq!(p.app.sets, 0);
}

#[test]
fn encodings_partial_sets_subscriptions_and_bad_headers() {
    // REQ-PE-ONE-003, REQ-PE-ONE-004.
    bad_headers(false);
}

#[test]
fn flat_responder_gives_the_same_statuses() {
    // REQ-PE-ONE-005: the cases of REQ-PE-ONE-003 and -004 with `respond_flat`, and a request
    // with whitespace or an object value (M2-103 7.1.1 forbids both) gets 400.
    bad_headers(true);
    let mut p = Pair::new();
    p.flat = true;
    for (i, header) in [
        &br#"{"resource": "X-State"}"#[..],
        br#"{"resource":"X-State","x":{"a":1}}"#,
        br#"{"resource":"X-State","x":[1]}"#,
    ]
    .into_iter()
    .enumerate()
    {
        let m = p.chunk(PeKind::GetInquiry, 20 + i as u8, header, 1, 1, b"");
        assert_eq!(p.send(&m).0.map(|s| s.0), Some(400));
    }
    let m = p.chunk(
        PeKind::GetInquiry,
        30,
        br#"{"resource":"X-State"}"#,
        1,
        1,
        b"",
    );
    assert_eq!(p.send(&m).0.map(|s| s.0), Some(200));
}

fn bad_headers(flat: bool) {
    let mut p = Pair::new();
    p.flat = flat;
    let cases: [(PeKind, &[u8], u16); 11] = [
        (
            PeKind::GetInquiry,
            br#"{"resource":"X-State","mutualEncoding":"Mcoded7"}"#,
            415,
        ),
        (
            PeKind::GetInquiry,
            br#"{"resource":"X-State","mutualEncoding":"ascii"}"#,
            200,
        ),
        (
            PeKind::SetInquiry,
            br#"{"resource":"X-State","setPartial":true}"#,
            405,
        ),
        (
            PeKind::Subscription,
            br#"{"command":"start","resource":"X-State"}"#,
            405,
        ),
        (PeKind::GetInquiry, br#"{"resource":"X-State""#, 400),
        (PeKind::GetInquiry, br#"{"resId":"a"}"#, 400),
        (PeKind::GetInquiry, br#"{"resource":5}"#, 400),
        (PeKind::GetInquiry, br#"{"resource":""}"#, 400),
        (
            PeKind::GetInquiry,
            br#"{"resource":"X-State","resId":"0123456789012345678901234567890123456"}"#,
            400,
        ),
        // REQ-PE-ONE-004: a "resId" that is not a string (the application would give 200), and a
        // "resource" of 37 characters (the application would give 404).
        (
            PeKind::GetInquiry,
            br#"{"resource":"X-State","resId":5}"#,
            400,
        ),
        (
            PeKind::GetInquiry,
            br#"{"resource":"X-State0123456789012345678901234567890"}"#,
            400,
        ),
    ];
    for (i, (kind, header, want)) in cases.into_iter().enumerate() {
        let m = p.chunk(kind, i as u8, header, 1, 1, b"");
        let (s, r) = p.send(&m);
        assert_eq!(
            s.map(|s| s.0),
            Some(want),
            "{}",
            String::from_utf8_lossy(header)
        );
        assert_eq!(reply(&r[0]).2, want);
    }
    assert_eq!(
        p.app.sets, 0,
        "a partial Set does not reach the application"
    );
}

#[test]
fn many_requests_do_not_fill_the_request_table() {
    // REQ-PE-ONE-001: each request ends at once, so more requests than the table of zmidi-ci
    // (4 here) all get a reply, also after 413 requests.
    let mut p = Pair::new();
    for rid in 0..40u8 {
        let total = if rid % 3 == 0 { 2 } else { 1 };
        let m = p.chunk(
            PeKind::GetInquiry,
            rid % 128,
            br#"{"resource":"X-State"}"#,
            total,
            1,
            b"",
        );
        let (s, r) = p.send(&m);
        assert!(s.is_some(), "request {rid}");
        assert_eq!(r.len(), 1);
    }
}

#[test]
fn other_events_get_no_reply() {
    let mut p = Pair::new();
    let ev = Event::DiscoveryEnded;
    let (mut body, mut out) = ([0u8; 16], [0u8; 64]);
    let r = respond(&mut p.b, &ev, LIST, &mut p.app, &mut body, &mut out, |_| {
        panic!("no reply")
    });
    assert_eq!(r, None);
}
