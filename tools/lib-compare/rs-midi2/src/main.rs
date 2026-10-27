#![no_std]
#![no_main]
use core::hint::black_box;
use core::ptr::{addr_of, read_volatile};
use mk20dx_hal as _;
#[allow(unused_imports)]
use midi2::prelude::*;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { loop {} }
#[no_mangle]
static mut INPUT: [u32; 64] = [0; 64];

#[cortex_m_rt::entry]
fn main() -> ! {
    loop {
        let w = unsafe { read_volatile(addr_of!(INPUT)) };
        #[cfg(feature = "lean")]
        lean(&w);
        #[cfg(feature = "full")]
        full(&w);
        black_box(&w);
    }
}

#[cfg(feature = "lean")]
#[inline(never)]
fn lean(w: &[u32; 64]) {
    use midi2::channel_voice2::ChannelVoice2 as C;
    match w[0] >> 28 {
        4 => match C::try_from(&w[..2]) {
            Ok(C::AssignableController(m)) => { black_box((1u8, m.channel(), m.bank(), m.index(), m.controller_data())); }
            Ok(C::RelativeAssignableController(m)) => { black_box((2u8, m.channel(), m.bank(), m.index(), m.controller_data())); }
            Ok(C::ProgramChange(m)) => { black_box((3u8, m.channel(), m.program())); }
            Ok(C::ControlChange(m)) => { black_box((4u8, m.channel(), m.control(), m.control_change_data())); }
            Ok(C::NoteOn(m)) => { black_box((5u8, m.channel(), m.note_number(), m.velocity(), m.attribute())); }
            Ok(C::NoteOff(m)) => { black_box((6u8, m.channel(), m.note_number())); }
            Ok(C::PerNotePitchBend(m)) => { black_box((7u8, m.channel(), m.note_number(), m.pitch_bend_data())); }
            Ok(C::ChannelPitchBend(m)) => { black_box((8u8, m.channel(), m.pitch_bend_data())); }
            _ => {}
        },
        1 => match midi2::system_common::SystemCommon::try_from(&w[..1]) {
            Ok(m) => { black_box(m.data()); }
            Err(_) => {}
        },
        _ => {}
    }
}

#[cfg(feature = "full")]
#[inline(never)]
fn full(w: &[u32; 64]) {
    if let Ok(m) = UmpMessage::try_from(&w[..]) {
        black_box(m.data());
    }
}
