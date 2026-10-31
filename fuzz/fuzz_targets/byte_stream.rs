//! The MIDI 1.0 byte stream (DIN, the byte form of BLE and USB): `BytesToUmp` with each protocol.
#![no_main]

use libfuzzer_sys::fuzz_target;
use zmidi_io::BytesToUmp;
use zmidi_ump::{Group, StreamProtocol};

fuzz_target!(|data: &[u8]| {
    let Some((&mode, bytes)) = data.split_first() else {
        return;
    };
    let protocol = if mode & 1 == 0 {
        StreamProtocol::Midi1
    } else {
        StreamProtocol::Midi2
    };
    let mut r = BytesToUmp::new(Group::new(mode >> 4).unwrap(), protocol);
    for &b in bytes {
        assert!(r.push(b).count() <= 2);
    }
});
