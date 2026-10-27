//! zmidi-ci and zmidi-ci-pe: MIDI-CI Discovery (M2-101) and a Property Exchange Get transaction
//! (M2-103), with the engines of two Devices, as in `crates/zmidi-ci-pe/tests/transaction.rs`.
//! Device A is the Initiator, Device B the Responder.

use core::hint::black_box;

use zmidi_ci::{
    Categories, CiDevice, DeviceConfig, DeviceIdentity, Direction, Framing, ManufacturerId, Muid,
    PiCapabilities,
};
use zmidi_ci_pe::{
    EngineConfig, Header, HeaderKind, HeaderStr, Mcoded7Decoder, Mcoded7Encoder, Outgoing,
    PeEngine, PeEvent, ResourceEntry, TxnKind,
};

use super::SYSEX_PAYLOAD;
use crate::bench;

type Dev = CiDevice<'static, 4, 2, 8>;
type Eng = PeEngine<'static, 4, 128, 4, 4>;

const MAX_SYSEX: u32 = 512;
const NOW: u64 = 1000;

/// The reply body of the Get: 400 bytes of JSON.
static BODY: [u8; 400] = {
    let mut b = [b' '; 400];
    b[0] = b'[';
    b[399] = b']';
    b
};

static RESOURCES: [ResourceEntry<'static>; 2] = [
    ResourceEntry::new("DeviceInfo"),
    ResourceEntry::new("X-Patch"),
];

static mut A: Option<(Dev, Eng)> = None;
static mut B: Option<(Dev, Eng)> = None;

fn config(m: u8) -> DeviceConfig<'static> {
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
        max_sysex_size: MAX_SYSEX,
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

/// Up to 8 messages of up to 600 bytes, written by one side for the other.
struct Out {
    msgs: [[u8; 600]; 8],
    lens: [usize; 8],
    n: usize,
}

impl Out {
    const fn new() -> Self {
        Out {
            msgs: [[0; 600]; 8],
            lens: [0; 8],
            n: 0,
        }
    }
    fn push(&mut self) -> &mut [u8; 600] {
        let i = self.n.min(7);
        self.n += 1;
        &mut self.msgs[i]
    }
    fn set_len(&mut self, len: usize) {
        self.lens[(self.n - 1).min(7)] = len;
    }
    fn get(&self, i: usize) -> &[u8] {
        &self.msgs[i][..self.lens[i]]
    }
}

/// Delivers one message to a side: the Device event goes to the engine. Writes the Device
/// replies and then the pending engine messages to `out`. Gives the data bytes of a PE Message
/// event and true when it is the last Chunk.
fn rx(side: &mut (Dev, Eng), msg: &[u8], out: &mut Out) -> (usize, bool) {
    let (dev, eng) = side;
    let mut rng = || 7u32;
    let mut r = dev.receive(0, msg, NOW * 1000, &mut rng);
    let ev = r.event();
    loop {
        let buf = out.push();
        match r.write_next(buf) {
            Some(Ok((_, n))) => out.set_len(n),
            _ => {
                out.n -= 1;
                break;
            }
        }
    }
    // A static: a local array is cleared on each call, a cost of the bench, not of the engine.
    static mut SCRATCH: [u8; 600] = [0; 600];
    // SAFETY: the probe has one thread and no interrupt; only `rx` uses `SCRATCH`.
    let scratch = unsafe { &mut *core::ptr::addr_of_mut!(SCRATCH) };
    let mut got = (0, false);
    if let Some(ev) = ev {
        if let Ok(Some(PeEvent::Message(m))) = eng.receive(dev, &ev, NOW, scratch) {
            got = (m.data.len(), m.end != zmidi_ci::ChunkEnd::More);
        }
    }
    pending(side, out);
    got
}

fn pending(side: &mut (Dev, Eng), out: &mut Out) {
    let (dev, eng) = side;
    loop {
        let buf = out.push();
        match eng.write_pending(dev, NOW, buf) {
            Some(Ok(n)) => out.set_len(n),
            _ => {
                out.n -= 1;
                break;
            }
        }
    }
}

fn muid(side: &(Dev, Eng)) -> Muid {
    side.0.muid()
}

/// A discovers B, and each gets the PE Capabilities of the other.
fn pair(a: &mut (Dev, Eng), b: &mut (Dev, Eng)) {
    let mut buf = [0u8; 600];
    let mut o = Out::new();
    let n = a.0.discovery(0, 0, &mut buf).unwrap();
    rx(b, &buf[..n], &mut o);
    let mut o2 = Out::new();
    rx(a, o.get(0), &mut o2);
    let n = a.0.pe_capabilities_inquiry(muid(b), &mut buf).unwrap();
    let mut o = Out::new();
    rx(b, &buf[..n], &mut o);
    rx(a, o.get(0), &mut Out::new());
    let n = b.0.pe_capabilities_inquiry(muid(a), &mut buf).unwrap();
    let mut o = Out::new();
    rx(a, &buf[..n], &mut o);
    rx(b, o.get(0), &mut Out::new());
}

/// One Get of X-Patch: A's inquiry, B's reply in Chunks of up to `data_room` bytes, A reads
/// each Chunk. Gives the reply data bytes that A received.
fn get(a: &mut (Dev, Eng), b: &mut (Dev, Eng)) -> u32 {
    let (bm, am) = (muid(b), muid(a));
    let header = Header {
        resource: Some(HeaderStr::Text("X-Patch")),
        ..Default::default()
    };
    let Ok(info) = a.1.request(
        &mut a.0,
        bm,
        TxnKind::Get,
        &Outgoing::json(header, 0),
        None,
        NOW,
    ) else {
        return 0;
    };
    let rid = info.request_id;
    let mut m = [0u8; 600];
    let Ok(n) = a.1.send(
        &mut a.0,
        bm,
        rid,
        Direction::Initiator,
        &[],
        true,
        NOW,
        &mut m,
    ) else {
        return 0;
    };
    let mut o = Out::new();
    rx(b, &m[..n], &mut o);
    if b.1
        .reply(
            &mut b.0,
            am,
            rid,
            &Outgoing::json(Header::default(), 0),
            NOW,
        )
        .is_err()
    {
        return 0;
    }
    let mut total = 0;
    let mut rest = &BODY[..];
    loop {
        let room =
            b.1.data_room(&b.0, am, rid, Direction::Responder)
                .unwrap_or(0)
                .min(rest.len());
        let (part, r) = rest.split_at(room);
        rest = r;
        let last = rest.is_empty();
        let Ok(n) = b.1.send(
            &mut b.0,
            am,
            rid,
            Direction::Responder,
            part,
            last,
            NOW,
            &mut m,
        ) else {
            return 0;
        };
        o.n = 0;
        let (k, end) = rx(a, &m[..n], &mut o);
        total += k as u32;
        if end || last {
            break;
        }
    }
    total
}

pub fn run() {
    // SAFETY: the probe has one thread and no interrupt; nothing else uses the statics.
    let (a, b) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(A),
            &mut *core::ptr::addr_of_mut!(B),
        )
    };
    let new = |m: u8, seed: u32| {
        let mut rng = move || seed;
        (
            Dev::new(config(m), &mut rng).unwrap(),
            Eng::new(EngineConfig {
                resources: &RESOURCES,
                zlib_window: 0,
            }),
        )
    };
    let a = a.insert(new(0x41, 0x0111_1111));
    let b = b.insert(new(0x42, 0x0222_2222));

    // M2-101 Discovery: B reads the inquiry and writes its Reply to Discovery.
    let mut buf = [0u8; 600];
    let n = a.0.discovery(0, 0, &mut buf).unwrap();
    let disc = black_box(&buf[..n]);
    bench("ci_device_receive_discovery", 1, 1, || {
        let mut rng = || 7u32;
        let mut r = b.0.receive(0, disc, NOW * 1000, &mut rng);
        black_box(r.event());
        let mut out = [0u8; 600];
        let mut k = 0;
        while let Some(Ok(_)) = r.write_next(&mut out) {
            k += 1;
        }
        k
    });

    pair(a, b);
    // M2-103 Get: one transaction, 400 bytes of reply data.
    bench("pe_get_transaction_400", 1, 400, || get(a, b));

    // M2-103 5: the Header Data of a Get inquiry and of a reply.
    let inquiry =
        black_box(&br#"{"resource":"ProgramList","resId":"bank1","offset":0,"limit":10}"#[..]);
    bench("pe_header_parse_inquiry", 1, 1, || {
        Header::parse(HeaderKind::Inquiry, inquiry).is_ok() as u32
    });
    let reply = black_box(
        &br#"{"status":200,"totalCount":128,"cacheTime":60,"mediaType":"application/json"}"#[..],
    );
    bench("pe_header_parse_reply", 1, 1, || {
        Header::parse(HeaderKind::Reply, reply).is_ok() as u32
    });

    // M2-103 Mcoded7: 1020 bytes give 1166 encoded bytes (8 for each 7).
    let mut enc = [0u8; 1200];
    let mut len = 0;
    bench("pe_mcoded7_encode_1020", 1020, 1166, || {
        let mut e = Mcoded7Encoder::new();
        let n = e
            .encode(black_box(&SYSEX_PAYLOAD[..]), &mut enc)
            .unwrap_or(0);
        len = n + e.finish(&mut enc[n..]).unwrap_or(0);
        len as u32
    });
    let enc = black_box(&enc[..len]);
    let mut dec = [0u8; 1100];
    bench("pe_mcoded7_decode_1020", 1020, 1020, || {
        let mut d = Mcoded7Decoder::new();
        d.decode(enc, &mut dec).unwrap_or(0) as u32
    });
}
