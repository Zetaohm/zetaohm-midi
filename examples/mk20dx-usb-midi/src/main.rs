//! USB MIDI echo firmware for the MK20DX256 (Teensy 3.2, FLXS1) with `zmidi-usb-device`.
//!
//! The device has one USB MIDI function (DEC-015): Alternate Setting 0 is USB-MIDI 1.0 (Event
//! Packets) and Alternate Setting 1 is USB MIDI 2.0 (UMP), USB-MIDI v2.0 3.1.1. Both use bulk
//! endpoints OUT 1 and IN 1, as in USB-MIDI v2.0 Appendix B.
//!
//! Each received channel voice or System Exclusive message goes back to the host on the same
//! Group. Other messages (Utility, UMP Stream) are counted and not sent back. The defmt log
//! shows the device state, the Alternate Setting that the host selects, and the counters.
//!
//! `poll()` runs in the USB0 interrupt, with the start-of-frame interrupt on (mk20dx-hal
//! `usb`, interrupt-driven use). Flash with `cargo run --release`; `host/echo.py` checks the
//! echo on the host.

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

use cortex_m::peripheral::NVIC;
use defmt_rtt as _;
use mk20dx_hal::interrupt;
use mk20dx_hal::pac;
use mk20dx_hal::prelude::*;
use mk20dx_hal::usb::{UsbBus, UsbBusExt};
use panic_probe as _;
use usb_device::class_prelude::UsbBusAllocator;
use usb_device::prelude::*;

use zmidi_usb::device::FunctionConfig;
use zmidi_usb::{BusSpeed, LinkConfig};
use zmidi_usb_device::{InterfaceNames, MidiClass};

defmt::timestamp!("{=u32}", 0);

/// pid.codes test VID and PID: a descriptor cache of the host for the FLXS1 PID (0x0485)
/// does not apply to this device.
const VID_PID: UsbVidPid = UsbVidPid(0x1209, 0x0001);

/// UMPs that wait to go back to the host.
const QUEUE: usize = 32;

/// Written by `main` before USB0 is unmasked, then used only by `USB0`.
static mut USB: Option<Usb> = None;

struct Usb {
    dev: UsbDevice<'static, UsbBus>,
    midi: MidiClass<'static, UsbBus>,
    queue: Queue,
}

/// A FIFO of UMPs, at most 4 words each.
struct Queue {
    umps: [([u32; 4], usize); QUEUE],
    head: usize,
    len: usize,
}

impl Queue {
    const fn new() -> Self {
        Queue { umps: [([0; 4], 0); QUEUE], head: 0, len: 0 }
    }

    /// Adds a UMP. False when the queue is full.
    fn push(&mut self, words: [u32; 4], n: usize) -> bool {
        if self.len == QUEUE {
            return false;
        }
        self.umps[(self.head + self.len) % QUEUE] = (words, n);
        self.len += 1;
        true
    }

    /// Removes the first `n` UMPs.
    fn drop_front(&mut self, n: usize) {
        let n = n.min(self.len);
        self.head = (self.head + n) % QUEUE;
        self.len -= n;
    }

    fn iter(&self) -> impl Iterator<Item = &[u32]> {
        (0..self.len).map(move |i| {
            let (w, n) = &self.umps[(self.head + i) % QUEUE];
            &w[..*n]
        })
    }
}

/// UMPs received and sent back, UMPs not sent back, and reader errors.
static RECEIVED: AtomicU32 = AtomicU32::new(0);
static ECHOED: AtomicU32 = AtomicU32::new(0);
static IGNORED: AtomicU32 = AtomicU32::new(0);
static DROPPED: AtomicU32 = AtomicU32::new(0);
static ERRORS: AtomicU32 = AtomicU32::new(0);
/// The device state and the Alternate Setting at the last log line.
static STATE: AtomicU8 = AtomicU8::new(0xFF);
static SETTING: AtomicU8 = AtomicU8::new(0xFF);

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

#[cortex_m_rt::entry]
fn main() -> ! {
    static mut ALLOC: Option<UsbBusAllocator<UsbBus>> = None;
    static STREAMING: [Option<&str>; 2] = [Some("MIDI 1.0"), Some("MIDI 2.0")];

    let dp = pac::Peripherals::take().unwrap();
    let clocks = dp.mcg.constrain().freeze(dp.osc, &dp.sim);

    let bus = dp.usb0.usb_bus(&dp.sim, &clocks);
    bus.enable_sof_interrupt(true);
    let alloc: &'static UsbBusAllocator<UsbBus> = ALLOC.insert(UsbBusAllocator::new(bus));

    // Class first: the allocator gives the interfaces and endpoints in order.
    let cfg = FunctionConfig::new(0, 0, BusSpeed::Full).unwrap();
    let names = InterfaceNames { audio_control: Some("zetaohm-midi"), streaming: &STREAMING };
    let midi = MidiClass::new(alloc, cfg, names, LinkConfig::default()).unwrap();
    let dev = UsbDeviceBuilder::new(alloc, VID_PID)
        .strings(&[StringDescriptors::default()
            .manufacturer("Zetaohm")
            .product("zetaohm-midi echo")
            .serial_number("mk20dx-usb-midi")])
        .unwrap()
        // A 64-byte EP0: with 8, a macOS host stops after SET_ADDRESS (mk20dx-hal `usb`).
        .max_packet_size_0(64)
        .unwrap()
        .build();

    // SAFETY: USB0 is still masked, so the handler cannot run yet.
    unsafe {
        (&raw mut USB).write(Some(Usb { dev, midi, queue: Queue::new() }));
        NVIC::unmask(pac::Interrupt::USB0);
    }
    defmt::info!("mk20dx-usb-midi: USB MIDI function with Alternate Settings 0 and 1");

    loop {
        cortex_m::asm::wfi();
    }
}

#[cortex_m_rt::interrupt]
fn USB0() {
    // SAFETY: main wrote USB before it unmasked USB0, and this handler is the only code that
    // uses it afterwards.
    let usb = unsafe { (&raw mut USB).as_mut().and_then(Option::as_mut).unwrap() };
    usb.dev.poll(&mut [&mut usb.midi]);

    let mut buf = [0u8; 64];
    let queue = &mut usb.queue;
    while usb
        .midi
        .read(&mut buf, |r| match r {
            Ok(r) => {
                RECEIVED.fetch_add(1, Ordering::Relaxed);
                let (words, n) = r.words();
                // Message Type 0x2 and 0x4 (channel voice), 0x3 (System Exclusive 7-bit).
                if matches!(words[0] >> 28, 0x2..=0x4) {
                    if !queue.push(words, n) {
                        DROPPED.fetch_add(1, Ordering::Relaxed);
                    }
                } else {
                    IGNORED.fetch_add(1, Ordering::Relaxed);
                }
            }
            Err(_) => {
                ERRORS.fetch_add(1, Ordering::Relaxed);
            }
        })
        .is_ok()
    {}

    let mut out = [0u8; 64];
    let total = usb.queue.len;
    let mut umps = usb.queue.iter().peekable();
    let sent = usb.midi.write(&mut umps, &mut out);
    let taken = total - umps.count();
    usb.queue.drop_front(taken);
    if let Ok(n) = sent {
        if n > 0 {
            ECHOED.fetch_add(taken as u32, Ordering::Relaxed);
        }
    }

    log_changes(usb);
}

/// Logs the device state and the Alternate Setting when they change.
fn log_changes(usb: &Usb) {
    let state = usb.dev.state() as u8;
    let setting = usb.midi.link().setting().0;
    let old_state = STATE.swap(state, Ordering::Relaxed);
    let old_setting = SETTING.swap(setting, Ordering::Relaxed);
    if old_state != state || old_setting != setting {
        defmt::info!(
            "state {} (0 Default, 1 Addressed, 2 Configured, 3 Suspend), Alternate Setting {}; \
             received {}, echoed {}, ignored {}, dropped {}, errors {}",
            state,
            setting,
            RECEIVED.load(Ordering::Relaxed),
            ECHOED.load(Ordering::Relaxed),
            IGNORED.load(Ordering::Relaxed),
            DROPPED.load(Ordering::Relaxed),
            ERRORS.load(Ordering::Relaxed),
        );
    }
}
