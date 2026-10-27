//! zmidi-ump: packet parse, MIDI 1.0 to MIDI 2.0 translation and back, SysEx7 reassembly.

use core::hint::black_box;

use zmidi_ump::{
    midi2_to_midi1, to_midi1, Group, Midi1ToMidi2, Midi1ToMidi2Group, SysEx7Event, SysEx7Packets,
    SysEx7Reassembler, Ump, UmpWords,
};

use super::{cv1, SYSEX_PAYLOAD};
use crate::bench;

/// 32 x (MIDI 2.0 Note On, 2 words) + 32 x (MIDI 1.0 Control Change, 1 word): 64 packets, 96 words.
static MIXED_WORDS: [u32; 96] = {
    let mut w = [0u32; 96];
    let mut i = 0;
    while i < 32 {
        w[i * 3] = 0x4090_3C00 | i as u32;
        w[i * 3 + 1] = 0x8000_0000;
        w[i * 3 + 2] = 0x20B0_0700 | i as u32;
        i += 1;
    }
    w
};

/// One packet of each decoded Message Type (M2-104 Table 4): Utility NOOP, System Timing Clock,
/// MIDI 1.0 Note On, SysEx7 Complete, MIDI 2.0 Note On, SysEx8 Complete, Flex Data Set Tempo,
/// UMP Stream Endpoint Discovery: 8 packets, 20 words.
static ALL_TYPES: [u32; 20] = [
    0x0000_0000,
    0x10F8_0000,
    0x2090_3C40,
    0x3002_0102,
    0x0000_0000,
    0x4090_3C00,
    0x8000_0000,
    0x5001_0000,
    0,
    0,
    0,
    0xD010_0000,
    0x02FA_F080,
    0,
    0,
    0xF000_0101,
    0x0000_001F,
    0,
    0,
    0x2080_3C00,
];

/// The SysEx7 packets of `SYSEX_PAYLOAD`: 170 packets of 2 words.
static mut SYSEX_WORDS: [u32; 340] = [0; 340];

/// Reads every packet of `w` with `Ump::parse` into `out`; gives the packet count.
fn parse_all(w: &[u32], out: &mut [Ump]) -> u32 {
    let mut i = 0;
    let mut n = 0;
    while i < w.len() {
        let Ok((u, k)) = Ump::parse(&w[i..]) else {
            break;
        };
        if let Some(slot) = out.get_mut(n) {
            *slot = u;
        }
        i += k;
        n += 1;
    }
    n as u32
}

pub fn run() {
    let mut umps = [Ump::parse(&[0]).unwrap().0; 64];
    bench("ump_parse_mixed", 64, 64, || {
        parse_all(black_box(&MIXED_WORDS[..]), black_box(&mut umps))
    });
    // Each Message Type once, and a MIDI 1.0 Note Off at the end: 9 packets.
    bench("ump_parse_all_types", 9, 9, || {
        parse_all(black_box(&ALL_TYPES[..]), black_box(&mut umps))
    });

    // M2-104 D.3: each MIDI 1.0 Note On gives one MIDI 2.0 Note On.
    let n = parse_all(cv1(), &mut umps);
    assert_eq!(n, 64);
    let umps = black_box(umps);
    bench("ump_translate_midi1_to_midi2_note", 64, 64, || {
        let mut t = Midi1ToMidi2::new();
        let mut n = 0;
        for u in &umps {
            if let Some(m) = t.translate(u) {
                black_box(m);
                n += 1;
            }
        }
        n
    });

    // The words path of the same translation (DEC-018, OPT-037): no typed Ump.
    bench("ump_translate_midi1_to_midi2_words", 64, 64, || {
        let mut t = Midi1ToMidi2Group::new();
        let mut n = 0;
        for &w in black_box(cv1()) {
            if let Some(m) = t.translate_words(w) {
                black_box(m);
                n += 1;
            }
        }
        n
    });

    let mut midi2 = umps;
    let mut t = Midi1ToMidi2::new();
    for (o, u) in midi2.iter_mut().zip(umps.iter()) {
        *o = t.translate(u).unwrap();
    }
    let midi2 = black_box(midi2);
    // M2-104 D.2: each MIDI 2.0 Note On gives one MIDI 1.0 Note On.
    bench("ump_translate_midi2_to_midi1_note", 64, 64, || {
        let mut n = 0;
        for u in &midi2 {
            let t = to_midi1(u);
            black_box(&t);
            n += t.len() as u32;
        }
        n
    });

    // The words path of the same translation (DEC-018): no typed Ump.
    let mut words2 = [0u32; 128];
    for (o, u) in words2.as_chunks_mut::<2>().0.iter_mut().zip(midi2.iter()) {
        u.write(o).unwrap();
    }
    let words2 = black_box(words2);
    bench("ump_translate_midi2_to_midi1_words", 64, 64, || {
        let mut n = 0;
        for w in words2.as_chunks::<2>().0 {
            if let Some(m) = midi2_to_midi1(w) {
                black_box(m.as_slice());
                n += m.as_slice().len() as u32;
            }
        }
        n
    });

    // SAFETY: the probe has one thread and no interrupt; nothing else uses the static.
    let words = unsafe { &mut *core::ptr::addr_of_mut!(SYSEX_WORDS) };
    let mut k = 0;
    for p in SysEx7Packets::try_new(Group::default(), &SYSEX_PAYLOAD).unwrap() {
        k += p.to_words(&mut words[k..]).unwrap();
    }
    assert_eq!(k, 340);
    let words: &[u32] = black_box(&words[..]);
    // M2-104 7.7: Start, 168 Continue, End: one complete message of 1020 bytes.
    bench("ump_sysex7_reassemble_1020", 170, 1020, || {
        let mut r = SysEx7Reassembler::<1024>::new(Group::default());
        let mut n = 0;
        for p in words.chunks(2) {
            if let Some(SysEx7Event::Complete(b)) = r.push(p) {
                n = b.len() as u32;
            }
        }
        n
    });
}
