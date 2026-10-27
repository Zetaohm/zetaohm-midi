//! Sends MIDI-CI Initiator messages of `zmidi-ci` to the midi2 C99 responder (`m2_ci_host`, one message
//! per line) and reads its replies with the same `CiDevice`. A secondary check of DEC-001: the
//! specification stays the reference; a difference is a finding to read against M2-101, not a test vector.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use zmidi_ci::{
    Categories, CiDevice, DeviceConfig, DeviceId, DeviceIdentity, Event, Framing, ManufacturerId,
    Muid, NakStatus, PeKind, PiCapabilities,
};
use zmidi_ump::{Group, SysEx7Event, SysEx7Reassembler};

struct Host {
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Host {
    fn start(path: &str) -> Host {
        let mut child = Command::new(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("start m2_ci_host");
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Host {
            _child: child,
            input,
            output,
        }
    }

    /// Sends one message; gives the words of the replies.
    fn send(&mut self, line: &str) -> Vec<String> {
        writeln!(self.input, "{line}").unwrap();
        self.input.flush().unwrap();
        let mut lines = Vec::new();
        loop {
            let mut l = String::new();
            self.output.read_line(&mut l).unwrap();
            let l = l.trim().to_string();
            if l == "END" || l.is_empty() {
                return lines;
            }
            lines.push(l);
        }
    }

    /// Sends a MIDI-CI message and gives each reply message (SysEx7 payload, no F0/F7).
    fn exchange(&mut self, msg: &[u8]) -> Vec<Vec<u8>> {
        let hex: Vec<String> = msg.iter().map(|b| format!("{b:02x}")).collect();
        let words: Vec<u32> = self
            .send(&hex.join(" "))
            .iter()
            .filter_map(|l| l.strip_prefix("W "))
            .map(|w| u32::from_str_radix(w, 16).unwrap())
            .collect();
        let mut asm = SysEx7Reassembler::<1024>::new(Group::from_bits(0));
        let mut out = Vec::new();
        for pair in words.chunks(2) {
            if let Some(SysEx7Event::Complete(m)) = asm.push(pair) {
                out.push(m.to_vec());
            }
        }
        out
    }
}

fn config() -> DeviceConfig<'static> {
    DeviceConfig {
        identity: DeviceIdentity {
            manufacturer: ManufacturerId::OneByte(0x7D),
            family: 1,
            model: 2,
            revision: [0, 0, 0, 1],
        },
        categories: Categories {
            profile_configuration: true,
            property_exchange: true,
            process_inquiry: true,
            ..Default::default()
        },
        max_sysex_size: 512,
        function_block: None,
        first_group: 0,
        group_count: 1,
        channels_in_use: 1,
        product_instance_id: None,
        framing: Framing::Bare,
        process_inquiry: PiCapabilities::default(),
    }
}

type Dev = CiDevice<'static, 4, 2, 4>;

/// Gives each reply to the Device; gives the events.
fn deliver<'m>(dev: &mut Dev, replies: &'m [Vec<u8>]) -> Vec<Event<'m>> {
    let mut events = Vec::new();
    for r in replies {
        let mut rx = dev.receive(0, r, 1_000, &mut || 7);
        if let Some(e) = rx.event() {
            events.push(e);
        }
        let mut buf = [0u8; 64];
        while rx.write_next(&mut buf).is_some() {}
    }
    events
}

struct Report {
    failed: usize,
}

impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: impl std::fmt::Debug) {
        println!("{} {name}: {detail:?}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            self.failed += 1;
        }
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: ci-interop PATH_TO_m2_ci_host");
    let mut host = Host::start(&path);
    let peer_line = host.send("MUID");
    let peer = Muid::new(u32::from_str_radix(peer_line[0].trim_start_matches("MUID "), 16).unwrap()).unwrap();
    let mut dev = Dev::new(config(), &mut || 0x0222_2222).unwrap();
    let mut rep = Report { failed: 0 };
    let mut buf = [0u8; 512];

    // Discovery (Table 6) -> Reply to Discovery (Table 8).
    let n = dev.discovery(0, 0, &mut buf).unwrap();
    let replies = host.exchange(&buf[..n]);
    let ev = deliver(&mut dev, &replies);
    let found = ev.iter().find_map(|e| match e {
        Event::Discovered { muid, reply } => Some((*muid, *reply)),
        _ => None,
    });
    rep.check(
        "Reply to Discovery is read",
        found.is_some_and(|(m, r)| m == peer && r.identity.manufacturer == ManufacturerId::OneByte(0x7D)),
        &found,
    );

    // Inquiry: Endpoint Information: no handler in midi2 C99 -> NAK Not Supported (nak_on_unknown).
    let n = dev.endpoint_info_inquiry(peer, &mut buf).unwrap();
    let replies = host.exchange(&buf[..n]);
    let ev = deliver(&mut dev, &replies);
    let nak = ev.iter().find_map(|e| match e {
        Event::Nak { nak, .. } => Some(nak.status),
        _ => None,
    });
    rep.check("Endpoint Information gets a NAK that is read", nak == Some(NakStatus::NotSupported), (&nak, &ev));

    // Profile Inquiry (Table 17) -> Reply to Profile Inquiry (Table 18).
    let n = dev.profile_inquiry(peer, DeviceId::FunctionBlock, &mut buf).unwrap();
    let replies = host.exchange(&buf[..n]);
    let ev = deliver(&mut dev, &replies);
    let ids = ev.iter().find_map(|e| match e {
        Event::ProfileInquiryReply { reply, .. } => Some((reply.enabled().count(), reply.disabled().count())),
        _ => None,
    });
    rep.check("Reply to Profile Inquiry is read", ids.is_some_and(|(a, b)| a + b == 1), (&ids, &ev));

    // Inquiry: PE Capabilities (Table 30) -> Reply (Table 32).
    let n = dev.pe_capabilities_inquiry(peer, &mut buf).unwrap();
    let replies = host.exchange(&buf[..n]);
    let ev = deliver(&mut dev, &replies);
    let caps = ev.iter().find_map(|e| match e {
        Event::PeCapabilities { capabilities, .. } => Some(*capabilities),
        _ => None,
    });
    rep.check("Reply to PE Capabilities is read", caps.is_some_and(|c| c.simultaneous >= 1), (&caps, &ev));

    // Inquiry: Get Property Data (Table 33) -> Reply (Table 34), one Chunk.
    let (rid, mut w) = dev
        .pe_request(peer, PeKind::GetInquiry, br#"{"resource":"X-Pattern"}"#, Some(0), None)
        .unwrap();
    let n = w.write_next(&[], &mut buf).unwrap();
    let replies = host.exchange(&buf[..n]);
    let ev = deliver(&mut dev, &replies);
    let chunk = ev.iter().find_map(|e| match e {
        Event::PeChunk { kind: PeKind::GetReply, chunk, .. } => Some((chunk.request_id, chunk.header.to_vec(), chunk.data.to_vec())),
        _ => None,
    });
    rep.check(
        "Reply to Get Property Data is read",
        chunk.as_ref().is_some_and(|(r, _, d)| *r == rid && d == b"[1,2,3]"),
        chunk.as_ref().map(|(r, h, d)| (r, String::from_utf8_lossy(h).to_string(), String::from_utf8_lossy(d).to_string())),
    );
    dev.pe_end(peer, rid, zmidi_ci::Direction::Initiator);

    // Inquiry: Process Inquiry Capabilities (Table 40) -> Reply (Table 41).
    let n = dev.pi_capabilities_inquiry(peer, &mut buf).unwrap();
    let replies = host.exchange(&buf[..n]);
    let ev = deliver(&mut dev, &replies);
    let pi = ev.iter().find_map(|e| match e {
        Event::PiCapabilities { capabilities, .. } => Some(*capabilities),
        _ => None,
    });
    rep.check("Reply to Process Inquiry Capabilities is read", pi.is_some(), (&pi, &ev));

    println!("{} check(s) failed", rep.failed);
    std::process::exit(if rep.failed == 0 { 0 } else { 1 });
}
