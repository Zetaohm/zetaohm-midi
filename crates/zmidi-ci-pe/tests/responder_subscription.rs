//! A responder-only engine (no feature `initiator`) with Subscriptions: it accepts a "start",
//! sends updates to its subscriber and reads the replies to them (M2-103-UM v1.2 section 9;
//! GitHub issue #1). The subscriber is raw messages, so the test needs no Initiator role. It runs
//! with every feature set, also with `--no-default-features`.

use zmidi_ci::{
    Categories, CiDevice, DeviceConfig, DeviceIdentity, Direction, Framing, ManufacturerId, Muid,
    PeChunk, PeKind, PiCapabilities,
};
use zmidi_ci_pe::{
    Command, EncodingName, EngineConfig, Header, HeaderKind, PeEngine, PeEvent, ResourceCaps,
    ResourceEntry, TransactionError, TxnKind,
};

type Dev = CiDevice<'static, 4, 2, 8>;
type Eng = PeEngine<'static, 4, 128, 4, 4>;

/// The MUID of the subscriber, which exists only as raw messages.
const SUBSCRIBER: u32 = 0x0123_4567;

fn device() -> Dev {
    let config = DeviceConfig {
        identity: DeviceIdentity {
            manufacturer: ManufacturerId::OneByte(0x42),
            family: 1,
            model: 2,
            revision: [0, 0, 0, 1],
        },
        categories: Categories {
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
    };
    CiDevice::new(config, &mut || 0x0222_2222).unwrap()
}

fn resources() -> &'static [ResourceEntry<'static>] {
    let mut patch = ResourceEntry::new("X-Patch");
    patch.caps = ResourceCaps {
        can_subscribe: true,
        ..ResourceCaps::default()
    };
    Box::leak(Box::new([patch]))
}

/// A raw Chunk from the subscriber to the Device.
fn raw(kind: PeKind, dst: Muid, rid: u8, header: &[u8]) -> Vec<u8> {
    let mut buf = [0u8; 300];
    let n = PeChunk {
        request_id: rid,
        header,
        total: 1,
        number: 1,
        data: &[],
    }
    .write(
        kind,
        Muid::new(SUBSCRIBER).unwrap(),
        dst,
        Framing::Bare,
        &mut buf,
    )
    .unwrap();
    buf[..n].to_vec()
}

fn reply_status(msg: &[u8]) -> u16 {
    let (h, body) = zmidi_ci::Header::parse(msg).unwrap();
    let chunk = PeChunk::read(&h, body).unwrap();
    Header::parse(HeaderKind::Reply, chunk.header)
        .unwrap()
        .status
        .unwrap()
        .0
}

struct Responder {
    dev: Dev,
    eng: Eng,
    now: u64,
}

/// What the engine gave for one message: the event kind and status of a reply.
#[derive(Debug, PartialEq)]
enum Got {
    Started(usize),
    Reply(TxnKind, Option<u16>),
    Other,
    Nothing,
}

impl Responder {
    /// Delivers one message; gives the engine event and every message written.
    fn rx(&mut self, msg: &[u8]) -> (Got, Vec<Vec<u8>>) {
        let mut out = Vec::new();
        let mut buf = [0u8; 600];
        let mut r = self.dev.receive(0, msg, self.now * 1000, &mut || 7);
        let ev = r.event();
        while let Some(x) = r.write_next(&mut buf) {
            out.push(buf[..x.unwrap().1].to_vec());
        }
        let mut scratch = [0u8; 600];
        let got = match ev {
            Some(ev) => match self
                .eng
                .receive(&mut self.dev, &ev, self.now, &mut scratch)
                .unwrap()
            {
                Some(PeEvent::SubscriptionStarted { index }) => Got::Started(index),
                Some(PeEvent::Message(m)) => {
                    Got::Reply(m.kind, m.header.and_then(|h| h.status).map(|s| s.0))
                }
                Some(_) => Got::Other,
                None => Got::Nothing,
            },
            None => Got::Nothing,
        };
        while let Some(n) = self.eng.write_pending(&mut self.dev, self.now, &mut buf) {
            out.push(buf[..n.unwrap()].to_vec());
        }
        (got, out)
    }

    /// Sends a Subscription update with no Property Data; gives its message and Request ID.
    fn update(&mut self, index: usize, command: Command) -> (Vec<u8>, u8) {
        let info = self
            .eng
            .update(
                &mut self.dev,
                index,
                command,
                EncodingName::Ascii,
                Some(0),
                self.now,
            )
            .unwrap();
        let mut buf = [0u8; 600];
        let n = self
            .eng
            .send(
                &mut self.dev,
                Muid::new(SUBSCRIBER).unwrap(),
                info.request_id,
                Direction::Initiator,
                &[],
                true,
                self.now,
                &mut buf,
            )
            .unwrap();
        (buf[..n].to_vec(), info.request_id)
    }
}

#[test]
fn responder_reads_the_replies_to_its_subscription_updates() {
    // REQ-PE-SUB-006, -010, -013, -015: a "start" gets 200; the reply to each update (200 or an
    // error status) ends its transaction, so the Request ID is free again.
    let mut b = Responder {
        dev: device(),
        eng: PeEngine::new(EngineConfig {
            resources: resources(),
            zlib_window: 0,
        }),
        now: 1_000,
    };
    let me = b.dev.muid();
    // The subscriber discovers the Device, then asks for its PE Capabilities (M2-101 8.5); the
    // Device keeps the subscriber's own values from that Inquiry (REQ-CI-PEF-045).
    let mut buf = [0u8; 64];
    let n = zmidi_ci::Discovery {
        identity: DeviceIdentity {
            manufacturer: ManufacturerId::OneByte(0x41),
            family: 1,
            model: 2,
            revision: [0, 0, 0, 1],
        },
        categories: Categories {
            property_exchange: true,
            ..Default::default()
        },
        max_sysex_size: 512,
        output_path_id: Some(0),
    }
    .write(Muid::new(SUBSCRIBER).unwrap(), Framing::Bare, &mut buf)
    .unwrap();
    b.rx(&buf[..n]);
    let n = zmidi_ci::PeCapabilities {
        simultaneous: 2,
        major: None,
        minor: None,
    }
    .write_inquiry(Muid::new(SUBSCRIBER).unwrap(), me, Framing::Bare, &mut buf)
    .unwrap();
    let (_, out) = b.rx(&buf[..n]);
    assert_eq!(out.len(), 1, "the Reply to Inquiry: PE Capabilities");
    let start = raw(
        PeKind::Subscription,
        me,
        1,
        br#"{"command":"start","resource":"X-Patch"}"#,
    );
    let (got, out) = b.rx(&start);
    let Got::Started(index) = got else {
        panic!("{got:?}")
    };
    assert_eq!(reply_status(&out[0]), 200);

    // REQ-PE-SUB-023: "notify", "full", "partial" and "end"; each reply frees the Request ID,
    // so every update uses the same one.
    let mut rids = Vec::new();
    for (command, status) in [
        (Command::Notify, 200),
        (Command::Full, 200),
        (Command::Partial, 200),
        (Command::Notify, 500),
    ] {
        let (m, rid) = b.update(index, command);
        let header = String::from_utf8(chunk_header(&m)).unwrap();
        assert!(
            header.starts_with(&format!(r#"{{"command":"{}""#, command.as_str())),
            "{header}"
        );
        rids.push(rid);
        let reply = format!(r#"{{"status":{status}}}"#);
        let (got, out) = b.rx(&raw(PeKind::SubscriptionReply, me, rid, reply.as_bytes()));
        assert_eq!(got, Got::Reply(TxnKind::Subscription, Some(status)));
        assert!(out.is_empty(), "no message answers a reply: {out:?}");
        // The transaction is closed: the same reply again gets no event.
        let (got, _) = b.rx(&raw(PeKind::SubscriptionReply, me, rid, reply.as_bytes()));
        assert_eq!(got, Got::Nothing);
    }
    // REQ-PE-GEN-003: the subscriber reported 2 simultaneous requests; a third open update is
    // refused.
    let open: Vec<u8> = (0..2).map(|_| b.update(index, Command::Notify).1).collect();
    assert_eq!(
        b.eng
            .update(
                &mut b.dev,
                index,
                Command::Notify,
                EncodingName::Ascii,
                Some(0),
                b.now
            )
            .err(),
        Some(TransactionError::PeerBusy)
    );
    for rid in open {
        let (got, _) = b.rx(&raw(
            PeKind::SubscriptionReply,
            me,
            rid,
            br#"{"status":200}"#,
        ));
        assert_eq!(got, Got::Reply(TxnKind::Subscription, Some(200)));
    }
    assert_eq!(b.eng.subscriptions().entries().count(), 1);
    // The responder ends the subscription.
    let (info, entry) = b.eng.unsubscribe(&mut b.dev, index, b.now).unwrap();
    assert_eq!(b.eng.subscriptions().entries().count(), 0);
    let mut buf = [0u8; 600];
    let n = b
        .eng
        .send(
            &mut b.dev,
            Muid::new(SUBSCRIBER).unwrap(),
            info.request_id,
            Direction::Initiator,
            &[],
            true,
            b.now,
            &mut buf,
        )
        .unwrap();
    assert_eq!(
        chunk_header(&buf[..n]),
        format!(
            r#"{{"command":"end","subscribeId":"{}","endedBy":"responder"}}"#,
            entry.subscribe_id()
        )
        .into_bytes()
    );
    rids.push(info.request_id);
    let (got, out) = b.rx(&raw(
        PeKind::SubscriptionReply,
        me,
        info.request_id,
        br#"{"status":200}"#,
    ));
    assert_eq!(got, Got::Reply(TxnKind::Subscription, Some(200)));
    assert!(out.is_empty());
    assert_eq!(b.eng.open(), 0);
    assert!(rids.iter().all(|r| *r == rids[0]), "{rids:?}");
}

/// The Header Data of one Chunk message.
fn chunk_header(msg: &[u8]) -> Vec<u8> {
    let (h, body) = zmidi_ci::Header::parse(msg).unwrap();
    PeChunk::read(&h, body).unwrap().header.to_vec()
}
