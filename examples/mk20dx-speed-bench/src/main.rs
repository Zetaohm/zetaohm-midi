//! Cycle counts of the speed-probe benches on the MK20DX256 (FLXS1) at 72 MHz (core 72 MHz,
//! bus 36 MHz, flash 24 MHz: the clock of mk20dx-hal `freeze`). OPT-029 step 2: the factor from the
//! QEMU instruction counts of docs/speeds.md to cycles.
//!
//! The benches are the modules of tools/speed-probe; this crate gives them `bench`, which reads
//! DWT CYCCNT around each bench and prints `BENCH <name> <items> <cycles>` through defmt. The
//! first run of each bench is measured: the flash cache is cold at its start, as in a firmware
//! that runs a path the first time. `cargo run --release` flashes it (probe-rs, J-Link) and
//! shows the log; `scripts/speed-check.py --hw <log>` compares it with the QEMU counts.

#![no_std]
#![no_main]

use core::hint::black_box;

use cortex_m::peripheral::DWT;
use defmt_rtt as _;
use mk20dx_hal::pac;
use mk20dx_hal::prelude::*;
use panic_probe as _;

#[path = "../../../tools/speed-probe/src/benches/mod.rs"]
mod benches;

// `pre_init` is deprecated in cortex-m-rt 0.7.5; flxs1 and the mk20dx-hal docs use it too.
#[allow(deprecated)]
mod early {
    use mk20dx_hal::pac;

    /// K20 RM 23.3.2: the WDOG must be unlocked within 256 bus clock cycles of a reset, which is
    /// before RAM init ends (mk20dx-hal `watchdog`).
    #[cortex_m_rt::pre_init]
    unsafe fn disable_wdog_early() {
        let wdog = &*pac::Wdog::ptr();
        wdog.unlock().write(|w| w.bits(0xC520));
        wdog.unlock().write(|w| w.bits(0xD928));
        // RM 23.3.1 step 2: no update on the bus cycle right after the unlock.
        cortex_m::asm::nop();
        cortex_m::asm::nop();
        wdog.stctrlh().write(|w| w.wdogen().disabled());
    }
}

/// Runs `f` between two reads of CYCCNT and prints its name, its item count and the cycles.
/// `f` gives the number of outputs it made; a value other than `expect` stops the bench.
pub fn bench(name: &str, items: u32, expect: u32, f: impl FnOnce() -> u32) {
    let c0 = DWT::cycle_count();
    let got = f();
    let c1 = DWT::cycle_count();
    defmt::info!(
        "BENCH {=str} {=u32} {=u32}",
        name,
        items,
        c1.wrapping_sub(c0)
    );
    if black_box(got) != expect {
        defmt::panic!(
            "CHECK {=str} gave {=u32} outputs, expected {=u32}",
            name,
            got,
            expect
        );
    }
}

#[cortex_m_rt::entry]
fn main() -> ! {
    let dp = pac::Peripherals::take().unwrap();
    let mut cp = cortex_m::Peripherals::take().unwrap();
    let clocks = dp.mcg.constrain().freeze(dp.osc, &dp.sim);
    let _ = clocks;
    cp.DCB.enable_trace();
    cp.DWT.enable_cycle_counter();
    // The cost of the two reads alone; the script subtracts it from each bench.
    bench("calibrate", 1, 0, || 0);
    benches::run();
    defmt::info!("END");
    loop {
        cortex_m::asm::bkpt();
    }
}
