//! A MIDI Clip File (M2-116) from storage: `ClipReader` over the whole file.
#![no_main]

use libfuzzer_sys::fuzz_target;
use zmidi_file::{ClipReader, FILE_HEADER};

fuzz_target!(|data: &[u8]| {
    // Half of the inputs get the file header, so the fuzzer reaches the event reader at once.
    let mut file = Vec::with_capacity(data.len() + FILE_HEADER.len());
    if data.first().is_some_and(|b| b & 1 == 1) {
        file.extend_from_slice(&FILE_HEADER);
    }
    file.extend_from_slice(data);
    if let Ok(reader) = ClipReader::new(&file) {
        let mut last = 0;
        for event in reader.take(10_000) {
            let Ok(event) = event else { break };
            assert!(event.ticks >= last);
            last = event.ticks;
        }
    }
});
