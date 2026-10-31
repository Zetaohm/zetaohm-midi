//! BLE-MIDI packets from a peer: `PacketReader::read` of a sequence of packets.
#![no_main]

use libfuzzer_sys::fuzz_target;
use zmidi_ble::PacketReader;
use zmidi_ump::{Group, StreamProtocol};

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let protocol = if mode & 1 == 0 {
        StreamProtocol::Midi1
    } else {
        StreamProtocol::Midi2
    };
    let mut r = PacketReader::new(Group::default(), protocol);
    for packet in zmidi_fuzz::parts(rest) {
        r.read(packet, |_| {});
    }
});
