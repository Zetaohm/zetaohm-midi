//! The benches, one module for each crate. Each input is a table of the size that a real stream
//! gives in one burst. Each bench gives the number of outputs it made, and [`crate::bench`] checks
//! it. Inputs that another part of the library makes (for example the packets of a writer for its
//! reader) are made before the first mark.

mod ble;
mod file;
mod io;
mod net;
mod pe;
mod ump;
mod usb;

use core::hint::black_box;

pub fn run() {
    ump::run();
    io::run();
    usb::run();
    ble::run();
    file::run();
    net::run();
    pe::run();
}

/// 64 MIDI 1.0 Channel Voice UMPs (Note On, Group 0, channel 1, notes 0 to 63), 1 word each.
pub static CV1_WORDS: [u32; 64] = {
    let mut w = [0u32; 64];
    let mut i = 0;
    while i < 64 {
        w[i] = 0x2090_0040 | ((i as u32) << 8);
        i += 1;
    }
    w
};

/// 64 MIDI 1.0 Note On messages, each with its status byte: 192 bytes.
pub static NOTE_BYTES: [u8; 192] = {
    let mut b = [0u8; 192];
    let mut i = 0;
    while i < 64 {
        b[i * 3] = 0x90;
        b[i * 3 + 1] = i as u8;
        b[i * 3 + 2] = 0x40;
        i += 1;
    }
    b
};

/// A SysEx payload of 1020 data bytes (0x00 to 0x7F, repeated): 170 SysEx7 packets of 6 bytes.
pub static SYSEX_PAYLOAD: [u8; 1020] = {
    let mut b = [0u8; 1020];
    let mut i = 0;
    while i < 1020 {
        b[i] = (i % 128) as u8;
        i += 1;
    }
    b
};

/// The words of `CV1_WORDS`, seen through `black_box`.
pub fn cv1() -> &'static [u32] {
    black_box(&CV1_WORDS[..])
}
