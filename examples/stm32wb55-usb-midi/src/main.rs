//! USB MIDI echo firmware for the STM32WB55 (USB FS device) with `zmidi-usb-embassy` (DEC-003,
//! DEC-015).
//!
//! The device has one USB MIDI function: Alternate Setting 0 is USB-MIDI 1.0 (Event Packets) and
//! Alternate Setting 1 is USB MIDI 2.0 (UMP), USB-MIDI v2.0 3.1.1. On `embassy-usb` each setting
//! has its own endpoint pair (DEC-015): OUT 1 and IN 1 on setting 0, OUT 2 and IN 2 on setting 1
//! (`zmidi_usb_embassy::default_config`). The WB55 USB peripheral has 8 endpoints with EP0 and
//! 1024 bytes of packet memory, so 4 bulk endpoints of 64 bytes fit.
//!
//! Each received channel voice or System Exclusive message goes back to the host on the same
//! Group, as in `examples/esp32s3-usb-midi` and `examples/mk20dx-usb-midi`. Other messages
//! (Utility, UMP Stream) are counted and not sent back. The defmt log shows the Alternate Setting
//! that the host selects and the counters.
//!
//! - Board: P-NUCLEO-WB55 (STM32WB55RG). USB: the "USB USER" connector, PA11 (D-) and PA12 (D+).
//!   Flash and log through the ST-LINK connector: `cargo run --release` (probe-rs).
//! - Clock: the 32 MHz HSE crystal, PLL R 64 MHz for the system, PLL Q 48 MHz for USB
//!   (`embassy_stm32::rcc::WPAN_DEFAULT`).
//! - `examples/mk20dx-usb-midi/host/echo.py` checks the echo on the host.

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};

use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::join::join3;
use embassy_stm32::rcc::mux::Clk48sel;
use embassy_stm32::usb::{Driver, InterruptHandler};
use embassy_stm32::{bind_interrupts, peripherals};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_usb::Builder;
use zmidi_usb::{BusSpeed, LinkConfig};
use zmidi_usb_embassy::{default_config, InterfaceNames, MidiClass, State, FUNCTION_MAX};
use panic_probe as _;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    USB_LP => InterruptHandler<peripherals::USB>;
});

/// pid.codes test VID and PID, as `examples/mk20dx-usb-midi`.
const VID: u16 = 0x1209;
const PID: u16 = 0x0001;

/// UMPs that wait to go back to the host.
const QUEUE: usize = 32;
/// The most UMPs for one batch of the sender.
const BATCH: usize = 16;
/// The full-speed bulk wMaxPacketSize (USB 2.0 5.8.3).
const PACKET: usize = 64;

type Mutex = CriticalSectionRawMutex;
/// One cable, the `FunctionConfig` defaults, and one endpoint address in each direction for
/// each of the 2 Alternate Settings.
type Midi = MidiClass<'static, Driver<'static, peripherals::USB>, Mutex>;
/// A UMP of at most 4 words and its word count.
type Ump = ([u32; 4], usize);

static ECHO: Channel<Mutex, Ump, QUEUE> = Channel::new();

/// UMPs received, sent back and not sent back, UMPs lost on a full queue, and reader errors.
static RECEIVED: AtomicU32 = AtomicU32::new(0);
static ECHOED: AtomicU32 = AtomicU32::new(0);
static IGNORED: AtomicU32 = AtomicU32::new(0);
static DROPPED: AtomicU32 = AtomicU32::new(0);
static ERRORS: AtomicU32 = AtomicU32::new(0);

fn log_counters(what: &str, setting: u8) {
    defmt::info!(
        "{}: Alternate Setting {}; received {}, echoed {}, ignored {}, dropped {}, errors {}",
        what,
        setting,
        RECEIVED.load(Ordering::Relaxed),
        ECHOED.load(Ordering::Relaxed),
        IGNORED.load(Ordering::Relaxed),
        DROPPED.load(Ordering::Relaxed),
        ERRORS.load(Ordering::Relaxed),
    );
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) -> ! {
    let mut rcc = embassy_stm32::rcc::WPAN_DEFAULT;
    // The USB clock must be 48 MHz (embassy-stm32 checks it): PLL Q of WPAN_DEFAULT.
    rcc.mux.clk48sel = Clk48sel::PLL1_Q;
    let mut config = embassy_stm32::Config::default();
    config.rcc = rcc;
    let p = embassy_stm32::init(config);

    static CONFIG_DESC: StaticCell<[u8; FUNCTION_MAX + 64]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 64]> = StaticCell::new();
    static CONTROL: StaticCell<[u8; 256]> = StaticCell::new();
    static MIDI: StaticCell<State<'static, Mutex>> = StaticCell::new();
    static STREAMING: [Option<&str>; 2] = [Some("MIDI 1.0"), Some("MIDI 2.0")];

    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);
    let mut config = embassy_usb::Config::new(VID, PID);
    config.manufacturer = Some("Zetaohm");
    config.product = Some("zetaohm-midi echo");
    config.serial_number = Some("stm32wb55-usb-midi");
    let mut builder = Builder::new(
        driver,
        config,
        CONFIG_DESC.init([0; FUNCTION_MAX + 64]),
        BOS_DESC.init([0; 64]),
        &mut [],
        CONTROL.init([0; 256]),
    );
    let names = InterfaceNames {
        audio_control: Some("zetaohm-midi"),
        streaming: &STREAMING,
    };
    let cfg = default_config(BusSpeed::Full).unwrap();
    let midi = Midi::new(
        &mut builder,
        MIDI.init(State::new()),
        cfg,
        names,
        LinkConfig::default(),
    )
    .unwrap();
    let (mut sender, mut receiver) = midi.split();
    let mut device = builder.build();
    defmt::info!("stm32wb55-usb-midi: USB MIDI function with Alternate Settings 0 and 1");

    let rx = async {
        let mut buf = [0u8; PACKET];
        loop {
            receiver.wait_connection().await;
            log_counters("connected", receiver.setting().0);
            while receiver.read_packet(&mut buf, queue_echo).await.is_ok() {}
            log_counters("disconnected", receiver.setting().0);
        }
    };

    let tx = async {
        let mut buf = [0u8; PACKET];
        let mut batch = [([0u32; 4], 0usize); BATCH];
        loop {
            sender.wait_connection().await;
            'connected: loop {
                batch[0] = ECHO.receive().await;
                let mut n = 1;
                // Check the room first: a UMP taken from the queue must go into the batch.
                while n < BATCH {
                    let Ok(u) = ECHO.try_receive() else { break };
                    batch[n] = u;
                    n += 1;
                }
                let size = sender.packet_size().min(PACKET);
                let mut umps = batch[..n].iter().map(|(w, len)| &w[..*len]).peekable();
                // Ok(0): nothing more to send (after the zero-length packet, if one is due).
                loop {
                    match sender.write_packet(&mut umps, &mut buf[..size]).await {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(_) => break 'connected,
                    }
                }
                ECHOED.fetch_add((n - umps.count()) as u32, Ordering::Relaxed);
            }
        }
    };

    let (never, _, _) = join3(device.run(), rx, tx).await;
    never
}

/// The sink of the receiver: runs with the adapter mutex held, so it only queues.
fn queue_echo(r: Result<zmidi_usb::Received, zmidi_usb::LinkError>) {
    match r {
        Ok(r) => {
            RECEIVED.fetch_add(1, Ordering::Relaxed);
            let ump = r.words();
            // Message Type 0x2 and 0x4 (channel voice), 0x3 (System Exclusive 7-bit).
            if matches!(ump.0[0] >> 28, 0x2..=0x4) {
                if ECHO.try_send(ump).is_err() {
                    DROPPED.fetch_add(1, Ordering::Relaxed);
                }
            } else {
                IGNORED.fetch_add(1, Ordering::Relaxed);
            }
        }
        Err(_) => {
            ERRORS.fetch_add(1, Ordering::Relaxed);
        }
    }
}
