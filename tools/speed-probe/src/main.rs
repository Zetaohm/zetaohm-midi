//! Speed probe (docs/optimization-backlog.md, OPT-029). Runs on the QEMU machine mps2-an386
//! (Cortex-M4F). Each bench prints `BENCH <name> <items>` through semihosting, then runs its work
//! between two calls of [`speed_mark`]. The plugin `qemu/insn_mark.c` prints the instruction count
//! at each call; `scripts/speed-check.py` gives the instructions for each item and checks the budgets.
//!
//! The inputs come from statics read through `black_box`, so the compiler cannot fold the work.

#![no_std]
#![no_main]

use core::hint::black_box;

use cortex_m_rt::entry;
use cortex_m_semihosting::{debug, hprintln};

mod benches;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    hprintln!("PANIC {}", info);
    debug::exit(debug::EXIT_FAILURE);
    loop {
        core::hint::spin_loop();
    }
}

/// The marker: the plugin counts the instructions executed before the first instruction of this
/// function. Not inlined, and not merged with another function (it has a side effect).
#[no_mangle]
#[inline(never)]
pub extern "C" fn speed_mark() {
    // SAFETY: a volatile write to a local; it keeps the function body.
    let mut x = 0u32;
    unsafe { core::ptr::write_volatile(&mut x, 1) };
}

/// Runs `f` between two marks and prints its name and its item count first. `f` gives the number of
/// outputs it made (words, bytes, events); a value other than `expect` stops the probe, so a bench
/// that measures no work fails.
pub fn bench(name: &str, items: u32, expect: u32, f: impl FnOnce() -> u32) {
    hprintln!("BENCH {} {}", name, items);
    speed_mark();
    let got = f();
    speed_mark();
    if black_box(got) != expect {
        hprintln!("CHECK {} gave {} outputs, expected {}", name, got, expect);
        debug::exit(debug::EXIT_FAILURE);
    }
}

#[entry]
fn main() -> ! {
    // The cost of the marks alone; the script subtracts it from each bench.
    bench("calibrate", 1, 0, || 0);
    benches::run();
    hprintln!("END");
    debug::exit(debug::EXIT_SUCCESS);
    loop {
        core::hint::spin_loop();
    }
}
