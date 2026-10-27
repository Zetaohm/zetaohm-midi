//! zmidi-ble: the BLE-MIDI packet writer and reader (BLE-MIDI v1.0).

use core::hint::black_box;

use zmidi_ble::{PacketReader, PacketWriter, TimedUmp};
use zmidi_ump::{Group, StreamProtocol};

use super::cv1;
use crate::bench;

/// ATT_MTU 247 (data length 251): packets of 244 bytes.
const ATT_MTU: u16 = 247;

pub fn run() {
    let mut out = [0u8; 512];
    let mut lens = [0usize; 4];
    // BLE-MIDI v1.0 running status: all messages have the same time, so after the first message the
    // writer omits the timestamp and the status: header, timestamp, status, 64 x (note, velocity)
    // = 131 bytes in one packet.
    bench("ble_write_note", 64, 131, || {
        let mut w = PacketWriter::new(ATT_MTU).unwrap();
        w.set_running_status(true);
        let mut umps = cv1()
            .chunks(1)
            .map(|words| TimedUmp {
                millis: 1000,
                words,
            })
            .peekable();
        let mut total = 0;
        let mut i = 0;
        loop {
            let n = w.write_packet(&mut umps, &mut out[total..]).unwrap_or(0);
            if n == 0 {
                break;
            }
            if let Some(l) = lens.get_mut(i) {
                *l = n;
            }
            i += 1;
            total += n;
        }
        total as u32
    });
    let (out, lens) = black_box((out, lens));
    bench("ble_read_note_midi1", 64, 64, || {
        let mut r = PacketReader::new(Group::default(), StreamProtocol::Midi1);
        let mut n = 0;
        let mut start = 0;
        for &l in lens.iter().take_while(|&&l| l > 0) {
            r.read(&out[start..start + l], |x| {
                black_box(&x);
                n += x.is_ok() as u32;
            });
            start += l;
        }
        n
    });
}
