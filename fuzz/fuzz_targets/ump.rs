//! UMP words from the wire: `Ump::parse` of each packet, the write of each parsed packet, and the
//! SysEx7 reassembler. Nothing may panic on any words.
#![no_main]

use libfuzzer_sys::fuzz_target;
use zmidi_ump::{Group, SysEx7Reassembler, Ump};

fuzz_target!(|data: &[u8]| {
    let words = zmidi_fuzz::words(data);
    let mut at = 0;
    while at < words.len() {
        match Ump::parse(&words[at..]) {
            Ok((ump, used)) => {
                let mut out = [0u32; 4];
                let _ = ump.write(&mut out);
                at += used.max(1);
            }
            Err(_) => at += 1,
        }
    }
    let mut r = SysEx7Reassembler::<8>::new(Group::new(0).unwrap());
    for part in words.chunks(2) {
        let _ = r.push(part);
    }
});
