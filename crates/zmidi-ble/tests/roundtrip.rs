//! Writer to reader round trip, and "never panics" for any input (REQ-BLE-GEN-001).

use proptest::prelude::*;
use zmidi_ble::zmidi_ump::{Group, StreamProtocol, Ump};
use zmidi_ble::{PacketReader, PacketWriter, Received, TimedUmp};
use zmidi_io::BytesToUmp;

/// UMPs with their times.
type Timed = Vec<(u64, Vec<u32>)>;

/// One MIDI 1.0 event: its time step and its bytes (a whole message, or a SysEx message that
/// can hold System Real-Time bytes).
fn event() -> impl Strategy<Value = (u64, Vec<u8>)> {
    let channel = (0x80u8..=0xEF, 0u8..0x80, 0u8..0x80).prop_map(|(s, a, b)| match s {
        0xC0..=0xDF => vec![s, a],
        _ => vec![s, a, b],
    });
    let common = prop_oneof![
        (0u8..0x80).prop_map(|a| vec![0xF1, a]),
        (0u8..0x80, 0u8..0x80).prop_map(|(a, b)| vec![0xF2, a, b]),
        (0u8..0x80).prop_map(|a| vec![0xF3, a]),
        Just(vec![0xF6]),
    ];
    let realtime =
        prop::sample::select(vec![0xF8u8, 0xFA, 0xFB, 0xFC, 0xFE, 0xFF]).prop_map(|b| vec![b]);
    let sysex = prop::collection::vec(
        prop_oneof![9 => 0u8..0x80, 1 => prop::sample::select(vec![0xF8u8, 0xFE])],
        0..60,
    )
    .prop_map(|mut d| {
        d.insert(0, 0xF0);
        d.push(0xF7);
        d
    });
    (
        0u64..60,
        prop_oneof![4 => channel, 1 => common, 2 => realtime, 1 => sysex],
    )
}

/// The UMPs of the events (zmidi-io, Group 0, MIDI 1.0 Protocol) with their times.
fn umps(events: &[(u64, Vec<u8>)], start: u64) -> Timed {
    let mut b = BytesToUmp::midi1(Group::default());
    let mut t = start;
    let mut out = Vec::new();
    for (dt, bytes) in events {
        t += dt;
        for &x in bytes {
            for p in b.push(x) {
                out.push((t, p.words().to_vec()));
            }
        }
    }
    out
}

#[test]
fn prop_writer_packets_read_back_as_the_same_umps_and_times() {
    proptest!(|(
        events in prop::collection::vec(event(), 0..80),
        start in 0u64..20_000,
        mtu in 23u16..120,
        running in any::<bool>(),
    )| {
        let input = umps(&events, start);
        let mut w = PacketWriter::new(mtu).unwrap();
        w.set_running_status(running);
        let mut it = input
            .iter()
            .map(|(t, words)| TimedUmp { millis: *t, words })
            .peekable();
        let mut r = PacketReader::default();
        let mut got = Vec::new();
        let mut buf = vec![0u8; w.packet_len()];
        loop {
            let n = w.write_packet(&mut it, &mut buf).unwrap();
            if n == 0 {
                break;
            }
            prop_assert!(n <= usize::from(mtu) - 3);
            prop_assert_eq!(buf[0] & 0xC0, 0x80, "header byte");
            r.read(&buf[..n], |x| {
                let rx = x.expect("no read error");
                let mut words = [0u32; 4];
                let k = rx.ump.write(&mut words).unwrap();
                got.push((u64::from(rx.timestamp.millis()), words[..k].to_vec()));
            });
        }
        let want: Timed =
            input.iter().map(|(t, words)| (t % 8192, words.clone())).collect();
        // zmidi-io gives a full SysEx7 chunk only at the next data byte, so a System Real-Time
        // message right after the chunk can come before it; M2-104 7.7.1 allows Real Time
        // between the UMPs of a SysEx message. The order inside each class is kept.
        let split = |v: &[(u64, Vec<u32>)]| -> (Timed, Timed) {
            v.iter().cloned().partition(|(_, w)| w[0] >> 28 == 0x3)
        };
        prop_assert_eq!(got.len(), want.len());
        prop_assert_eq!(split(&got), split(&want));
        prop_assert!(!r.in_sysex());
    });
}

#[test]
fn prop_reader_never_panics() {
    proptest!(|(packets in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..64), 0..8))| {
        let mut r = PacketReader::new(Group::default(), StreamProtocol::Midi2);
        for p in &packets {
            r.read(p, |_| {});
        }
    });
}

#[test]
fn prop_writer_never_panics() {
    proptest!(|(
        umps in prop::collection::vec((any::<u64>(), prop::collection::vec(any::<u32>(), 0..5)), 0..40),
        mtu in any::<u16>(),
        len in 0usize..600,
    )| {
        let mut w = PacketWriter::new(mtu).unwrap_or_default();
        let mut it = umps
            .iter()
            .map(|(t, words)| TimedUmp { millis: *t, words })
            .peekable();
        let mut buf = vec![0u8; len];
        for _ in 0..200 {
            match w.write_packet(&mut it, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => prop_assert!(n <= w.packet_len()),
            }
        }
    });
}

/// A UMP with a random Message Type and random content: valid, invalid and out-of-order
/// SysEx7 packets, MIDI 2.0 Channel Voice (translated by zmidi-io), Utility and others.
fn any_ump() -> impl Strategy<Value = Vec<u32>> {
    (
        prop_oneof![1 => 0u32..16, 3 => prop::sample::select(vec![1u32, 2, 3, 4])],
        any::<[u32; 4]>(),
    )
        .prop_map(|(mt, mut w)| {
            w[0] = (w[0] & 0x0FFF_FFFF) | mt << 28;
            match mt {
                // A status byte with bit 7 set, as a valid message has.
                0x1 | 0x2 | 0x4 => w[0] |= 0x0080_0000,
                // A SysEx7 byte count of 0 to 6.
                0x3 => w[0] = (w[0] & 0xFFF0_FFFF) | ((w[0] >> 16 & 0xF) % 7) << 16,
                _ => {}
            }
            let n = match mt {
                0x0..=0x2 | 0x6 | 0x7 => 1,
                0x3 | 0x4 | 0x8..=0xA => 2,
                0xB | 0xC => 3,
                _ => 4,
            };
            w[..n].to_vec()
        })
}

#[test]
fn prop_every_writer_packet_reads_with_no_error() {
    proptest!(|(
        umps in prop::collection::vec((0u64..300, any_ump()), 0..60),
        mtu in 23u16..80,
        running in any::<bool>(),
    )| {
        let mut w = PacketWriter::new(mtu).unwrap();
        w.set_running_status(running);
        let mut t = 0u64;
        let timed: Vec<(u64, Vec<u32>)> = umps.into_iter().map(|(dt, u)| { t += dt; (t, u) }).collect();
        let mut it = timed.iter().map(|(t, words)| TimedUmp { millis: *t, words }).peekable();
        let mut r = PacketReader::default();
        let mut buf = vec![0u8; w.packet_len()];
        let mut errors = 0;

        loop {
            let n = w.write_packet(&mut it, &mut buf).unwrap();
            if n == 0 {
                break;
            }
            r.read(&buf[..n], |x| match x {
                Ok(_) => {}
                Err(_) => errors += 1,
            });
        }
        prop_assert_eq!(errors, 0);
        prop_assert!(it.next().is_none());
        // A SysEx that the input does not end stays open in both, so the writer has nothing held.
        prop_assert!(!w.has_held());
    });
}

/// DEC-BLE-002: `read_words` followed by `Ump::parse` gives the same items and errors, in the
/// same order, as `read`, also across packets (open SysEx, timestamps).
#[test]
fn prop_read_words_matches_read() {
    proptest!(|(
        packets in prop::collection::vec((0x80u8..=0xBF, prop::collection::vec(any::<u8>(), 0..40)), 0..8),
        midi2 in any::<bool>(),
    )| {
        let protocol = if midi2 { StreamProtocol::Midi2 } else { StreamProtocol::Midi1 };
        let mut typed = PacketReader::new(Group::default(), protocol);
        let mut words = PacketReader::new(Group::default(), protocol);
        for (header, rest) in &packets {
            let mut p = vec![*header];
            p.extend_from_slice(rest);
            let mut a = Vec::new();
            typed.read(&p, |x| a.push(x));
            let mut b = Vec::new();
            words.read_words(&p, |x| {
                b.push(x.map(|w| {
                    let (ump, n) = Ump::parse(w.words).unwrap();
                    assert_eq!(n, w.words.len());
                    Received { timestamp: w.timestamp, ump }
                }))
            });
            prop_assert_eq!(a, b);
            prop_assert_eq!(typed.in_sysex(), words.in_sysex());
        }
    });
}
