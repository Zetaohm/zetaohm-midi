//! zmidi-usb: USB-MIDI 1.0 Event Packets (Alternate Setting 0) and the UMP words of Alternate
//! Setting 1.

use core::hint::black_box;

use zmidi_ump::StreamProtocol;
use zmidi_usb::event_packet::{PacketReader, PacketWriter, WriterOptions};
use zmidi_usb::ump_usb::{ump_words, word_from_bytes, word_to_bytes};

use super::cv1;
use crate::bench;

pub fn run() {
    let mut packets = [0u8; 256];
    // USB-MIDI 1.0 4: one 4-byte Event Packet (CIN 0x9) for each Note On.
    bench("usb_event_write_note", 64, 256, || {
        let mut w = PacketWriter::<1>::new(WriterOptions::default());
        let mut n = 0;
        for u in cv1().chunks(1) {
            n += w.write(u, &mut packets[n..]).unwrap_or(0);
        }
        n as u32
    });
    let packets = black_box(packets);
    bench("usb_event_read_note_midi1", 64, 64, || {
        let mut r = PacketReader::<1>::new(StreamProtocol::Midi1).unwrap();
        let mut n = 0;
        r.read_with(&packets, |p| {
            black_box(&p);
            n += p.is_ok() as u32;
        });
        n
    });
    bench("usb_event_read_note_midi2", 64, 64, || {
        let mut r = PacketReader::<1, 1>::new(StreamProtocol::Midi2).unwrap();
        let mut n = 0;
        r.read_with(&packets, |p| {
            black_box(&p);
            n += p.is_ok() as u32;
        });
        n
    });

    // USB-MIDI 2.0 Alternate Setting 1: the UMP words as little-endian bytes, and back.
    let mut bytes = [0u8; 256];
    bench("usb_ump_words_to_bytes", 64, 64, || {
        let mut n = 0;
        for (o, &w) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(cv1()) {
            o.copy_from_slice(&word_to_bytes(w));
            n += 1;
        }
        n
    });
    let bytes = black_box(bytes);
    bench("usb_ump_bytes_to_words", 64, 64, || {
        let mut i = 0;
        let mut n = 0;
        while i + 4 <= bytes.len() {
            let w0 = word_from_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
            let k = ump_words(w0);
            black_box(w0);
            i += 4 * k;
            n += 1;
        }
        n
    });
}
