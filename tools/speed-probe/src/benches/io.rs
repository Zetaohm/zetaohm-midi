//! zmidi-io: the MIDI 1.0 byte stream reader and writer (DIN).

use core::hint::black_box;

use zmidi_io::{BytesToUmp, BytesToUmpCore, UmpToBytes};
use zmidi_ump::{Group, StreamProtocol};

use super::{cv1, NOTE_BYTES};
use crate::bench;

/// Gives each byte to a new reader; gives the number of UMPs out.
fn bytes_to_ump(protocol: StreamProtocol, bytes: &[u8]) -> u32 {
    let mut r = BytesToUmp::new(Group::default(), protocol);
    let mut n = 0;
    for &b in bytes {
        for p in r.push(b) {
            black_box(p);
            n += 1;
        }
    }
    n
}

pub fn run() {
    // One UMP for each message: Message Type 0x2 (MIDI 1.0 Protocol) or 0x4 (MIDI 2.0 Protocol, D.3).
    bench("io_bytes_to_ump_midi1_note", 64, 64, || {
        bytes_to_ump(StreamProtocol::Midi1, black_box(&NOTE_BYTES[..]))
    });
    // The callback path of the same reader (MIDI 1.0 Protocol): no Pushed iterator for each byte.
    bench("io_bytes_to_ump_midi1_note_feed", 64, 64, || {
        let mut r = BytesToUmpCore::new(Group::default());
        let mut n = 0;
        for &b in black_box(&NOTE_BYTES[..]) {
            r.feed(b, |w| {
                black_box(w);
                n += 1;
            });
        }
        n
    });
    bench("io_bytes_to_ump_midi2_note", 64, 64, || {
        bytes_to_ump(StreamProtocol::Midi2, black_box(&NOTE_BYTES[..]))
    });
    // Running status on: 1 status byte, then 2 data bytes for each message.
    // The callback path of the reader with the D.3 translator (OPT-036).
    bench("io_bytes_to_ump_midi2_note_feed", 64, 64, || {
        let mut r = BytesToUmp::new(Group::default(), StreamProtocol::Midi2);
        let mut n = 0;
        for &b in black_box(&NOTE_BYTES[..]) {
            r.feed(b, |w| {
                black_box(w);
                n += 1;
            });
        }
        n
    });
    bench("io_ump_to_bytes_note_running", 64, 129, || {
        let mut t = UmpToBytes::new(true);
        let mut n = 0;
        for w in cv1().chunks(1) {
            t.write_each(w, |b| {
                black_box(b);
                n += 1;
            });
        }
        n
    });
}
