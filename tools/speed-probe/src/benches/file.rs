//! zmidi-file: the MIDI Clip File writer and reader (M2-116).

use core::hint::black_box;

use zmidi_file::{ClipConfig, ClipReader, ClipWriter};
use zmidi_ump::{Group, Ump};

use super::cv1;
use crate::bench;

pub fn run() {
    let mut buf = [0u8; 2048];
    let config = ClipConfig {
        ticks_per_quarter_note: 96,
        profiles: &[],
        tempo: None,
        time_signature: None,
        group: Group::default(),
    };
    let mut len = 0;
    // M2-116 3: one Delta Clockstamp and one UMP for each note, 12 ticks apart.
    bench("file_clip_write_note", 64, 64, || {
        let Ok(mut w) = ClipWriter::new(&mut buf, black_box(&config)) else {
            return 0;
        };
        if w.start(0, None, None).is_err() {
            return 0;
        }
        let mut n = 0;
        for (i, u) in cv1().chunks(1).enumerate() {
            n += w.event(12 * i as u64, u).is_ok() as u32;
        }
        len = w.finish(12 * 64).unwrap_or(0);
        n
    });
    let data = black_box(&buf[..len]);
    // The reader gives the 64 notes and the End of Clip (and the DCTPQ and Start of Clip).
    bench("file_clip_read_note", 64, 64, || {
        let Ok(r) = ClipReader::new(data) else {
            return 0;
        };
        let mut n = 0;
        for e in r {
            let Ok(e) = e else { break };
            if matches!(e.ump, Ump::Midi1ChannelVoice(_)) {
                n += 1;
            }
            black_box(e);
        }
        n
    });
}
