// Puts memory.x in the linker search path (cortex-m-rt link.x includes it), and makes the fixed
// USB function of the job usb2_fixed (zmidi_usb::fixed::write_rust, DEC-USB-002).
use std::{env, fs, path::PathBuf};

use zmidi_usb::device::{CableCounts, FunctionConfig, Midi1Setting};
use zmidi_usb::BusSpeed;

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::copy("memory.x", out.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");

    // The lean jobs (feature alt0 off) have an empty Alternate Setting 0 (REQ-USB-GEN-014).
    let empty = Midi1Setting::cables(CableCounts::new(0, 0, BusSpeed::Full));
    let mut cfg: FunctionConfig = FunctionConfig::new(0, 1, BusSpeed::Full).unwrap();
    if env::var_os("CARGO_FEATURE_USB2_LEAN").is_some() {
        cfg.midi1 = Some(empty.clone());
    }
    let mut src = String::new();
    zmidi_usb::fixed::write_rust(&cfg, "MIDI_FN", &mut src).unwrap();
    fs::write(out.join("midi_fn.rs"), src).unwrap();

    // The embassy adapter has its own default: OUT 2 and IN 2 on Alternate Setting 1 (DEC-015).
    let mut cfg = zmidi_usb_embassy::default_config(BusSpeed::Full).unwrap();
    if env::var_os("CARGO_FEATURE_EMB2_LEAN").is_some() {
        cfg.midi1 = Some(empty);
    }
    let mut src = String::new();
    zmidi_usb::fixed::write_rust(&cfg, "EMB_FN", &mut src).unwrap();
    fs::write(out.join("emb_fn.rs"), src).unwrap();
}
