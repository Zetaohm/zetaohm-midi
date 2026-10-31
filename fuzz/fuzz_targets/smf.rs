//! A Standard MIDI File (RP-001) from storage: `SmfReader`, each track and each event.
#![no_main]

use libfuzzer_sys::fuzz_target;
use zmidi_file::SmfReader;

/// Half of the inputs get the chunk magic and lengths ("MThd" with 6 bytes, then one "MTrk"
/// chunk for each part), so the fuzzer reaches the event reader at once; the other half are
/// read as they are.
fn file(data: &[u8]) -> Vec<u8> {
    let Some((&mode, rest)) = data.split_first() else {
        return Vec::new();
    };
    if mode & 1 == 0 || rest.len() < 6 {
        return data.to_vec();
    }
    let mut f = Vec::with_capacity(data.len() + 64);
    f.extend_from_slice(b"MThd\0\0\0\x06");
    f.extend_from_slice(&rest[..6]);
    for track in zmidi_fuzz::parts(&rest[6..]) {
        f.extend_from_slice(b"MTrk");
        f.extend_from_slice(&(track.len() as u32).to_be_bytes());
        f.extend_from_slice(track);
    }
    f
}

fuzz_target!(|data: &[u8]| {
    let data = file(data);
    let Ok(r) = SmfReader::new(&data) else {
        return;
    };
    for track in r.tracks().take(64) {
        let Ok(track) = track else { break };
        for event in track.take(100_000) {
            if event.is_err() {
                break;
            }
        }
    }
});
