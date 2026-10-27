//! USB MIDI echo firmware for the ESP32-S3 (USB OTG) with `zmidi-usb-embassy` (DEC-003, DEC-015).
//!
//! The device has one USB MIDI function: Alternate Setting 0 is USB-MIDI 1.0 (Event Packets) and
//! Alternate Setting 1 is USB MIDI 2.0 (UMP), USB-MIDI v2.0 3.1.1. On `embassy-usb` each setting
//! has its own endpoint pair (DEC-015): OUT 1 and IN 1 on setting 0, OUT 2 and IN 2 on setting 1
//! (`zmidi_usb_embassy::default_config`). The ESP32-S3 OTG has 6 endpoints after EP0, so 4 fit.
//!
//! Each received channel voice or System Exclusive message goes back to the host on the same
//! Group, as in `examples/mk20dx-usb-midi`. Other messages (Utility, UMP Stream) are counted and
//! not sent back. The serial log shows the Alternate Setting that the host selects and the
//! counters.
//!
//! - USB: the OTG port on GPIO19 (D-) and GPIO20 (D+), the "USB" connector of the
//!   ESP32-S3-DevKitC-1. Flash and log through the other connector ("UART"):
//!   `cargo run --release`.
//! - `examples/mk20dx-usb-midi/host/echo.py` checks the echo on the host.
//! - Two commands for a board with one USB connector (for example the Adafruit ESP32-S3 Reverse
//!   TFT Feather), where the OTG function takes the port of the serial log and of the flash:
//!   - SysEx `F0 7D 73 74 61 74 F7` ("stat"): the answer is a SysEx with the counters as text.
//!   - SysEx `F0 7D 62 6F 6F 74 F7` ("boot"): the chip gives the USB PHY back to USB-Serial-JTAG
//!     and restarts in the ROM download mode, so `espflash` can flash with no BOOT button
//!     (`host/reflash.sh`).

#![no_std]
#![no_main]

use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

use embassy_executor::Spawner;
use embassy_futures::join::join3;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_usb::Builder;
use esp_backtrace as _;
use esp_hal::otg_fs::asynch::{Config as OtgConfig, Driver};
use esp_hal::otg_fs::Usb;
use esp_hal::timer::timg::TimerGroup;
use esp_println::println;
use zmidi_usb::{BusSpeed, LinkConfig};
use zmidi_usb_embassy::{default_config, InterfaceNames, MidiClass, State, FUNCTION_MAX};
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

/// pid.codes test VID and PID, as `examples/mk20dx-usb-midi`.
const VID: u16 = 0x1209;
const PID: u16 = 0x0001;

/// UMPs that wait to go back to the host.
const QUEUE: usize = 32;
/// The most UMPs for one batch of the sender.
const BATCH: usize = 16;
/// The full-speed bulk wMaxPacketSize (USB 2.0 5.8.3).
const PACKET: usize = 64;
/// The most UMPs from one packet: 16 USB-MIDI 1.0 Event Packets, or 16 one-word UMPs.
const PACKET_UMPS: usize = PACKET / 4;

type Mutex = CriticalSectionRawMutex;
/// One cable, the `FunctionConfig` defaults, and one endpoint address in each direction for
/// each of the 2 Alternate Settings.
type Midi = MidiClass<'static, Driver<'static>, Mutex>;
/// A UMP of at most 4 words and its word count.
type Ump = ([u32; 4], usize);

static ECHO: Channel<Mutex, Ump, QUEUE> = Channel::new();

/// UMPs received, sent back and not sent back, UMPs lost on a full queue, and reader errors.
static RECEIVED: AtomicU32 = AtomicU32::new(0);
static ECHOED: AtomicU32 = AtomicU32::new(0);
static IGNORED: AtomicU32 = AtomicU32::new(0);
static DROPPED: AtomicU32 = AtomicU32::new(0);
static ERRORS: AtomicU32 = AtomicU32::new(0);
/// The Alternate Setting that the host selected.
static SETTING: AtomicU8 = AtomicU8::new(0);

fn log_counters(what: &str, setting: u8) {
    println!(
        "{what}: Alternate Setting {setting}; received {}, echoed {}, ignored {}, dropped {}, \
         errors {}",
        RECEIVED.load(Ordering::Relaxed),
        ECHOED.load(Ordering::Relaxed),
        IGNORED.load(Ordering::Relaxed),
        DROPPED.load(Ordering::Relaxed),
        ERRORS.load(Ordering::Relaxed),
    );
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0);

    static EP_OUT: StaticCell<[u8; 1024]> = StaticCell::new();
    static CONFIG_DESC: StaticCell<[u8; FUNCTION_MAX + 64]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 64]> = StaticCell::new();
    static CONTROL: StaticCell<[u8; 256]> = StaticCell::new();
    static MIDI: StaticCell<State<'static, Mutex>> = StaticCell::new();
    static STREAMING: [Option<&str>; 2] = [Some("MIDI 1.0"), Some("MIDI 2.0")];

    let usb = Usb::new(p.USB0, p.GPIO20, p.GPIO19);
    let driver = Driver::new(usb, EP_OUT.init([0; 1024]), OtgConfig::default());
    let mut config = embassy_usb::Config::new(VID, PID);
    config.manufacturer = Some("Zetaohm");
    config.product = Some("zetaohm-midi echo");
    config.serial_number = Some("esp32s3-usb-midi");
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
    println!("esp32s3-usb-midi: USB MIDI function with Alternate Settings 0 and 1");

    let rx = async {
        let mut buf = [0u8; PACKET];
        loop {
            receiver.wait_connection().await;
            SETTING.store(receiver.setting().0, Ordering::Relaxed);
            log_counters("connected", receiver.setting().0);
            let mut pending = heapless::Vec::<Ump, PACKET_UMPS>::new();
            while receiver
                .read_packet(&mut buf, |r| collect_echo(r, &mut pending))
                .await
                .is_ok()
            {
                // Back pressure: wait for room in the queue. The OUT endpoint is not read
                // meanwhile, so the device answers NAK and the host waits (USB 2.0 8.5.1).
                for u in pending.iter() {
                    ECHO.send(*u).await;
                }
                pending.clear();
            }
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
fn collect_echo(
    r: Result<zmidi_usb::Received, zmidi_usb::LinkError>,
    pending: &mut heapless::Vec<Ump, PACKET_UMPS>,
) {
    match r {
        Ok(r) => {
            RECEIVED.fetch_add(1, Ordering::Relaxed);
            let ump = r.words();
            if command(&ump.0[..ump.1], b"boot") {
                download_mode();
            }
            if command(&ump.0[..ump.1], b"stat") {
                send_counters(ump.0[0] >> 24 & 0xF);
                return;
            }
            // Message Type 0x2 and 0x4 (channel voice), 0x3 (System Exclusive 7-bit).
            if matches!(ump.0[0] >> 28, 0x2..=0x4) {
                if pending.push(ump).is_err() {
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

/// True for a whole SysEx7 message (one UMP, status Complete) with the data 7DH and `name`
/// (M2-104 7.7, Figure 37).
fn command(words: &[u32], name: &[u8; 4]) -> bool {
    let [w0, w1] = match words {
        [a, b] => [*a, *b],
        _ => return false,
    };
    let data = [
        (w0 >> 8) as u8,
        w0 as u8,
        (w1 >> 24) as u8,
        (w1 >> 16) as u8,
        (w1 >> 8) as u8,
    ];
    w0 >> 28 == 0x3 && (w0 >> 16) & 0xFF == 0x05 && data[0] == 0x7D && data[1..] == name[..]
}

/// Queues the counters as a SysEx7 message (7DH, then ASCII text) on `group`.
fn send_counters(group: u32) {
    let mut text = heapless::String::<96>::new();
    let _ = write!(
        text,
        "setting {} received {} echoed {} ignored {} dropped {} errors {}",
        SETTING.load(Ordering::Relaxed),
        RECEIVED.load(Ordering::Relaxed),
        ECHOED.load(Ordering::Relaxed),
        IGNORED.load(Ordering::Relaxed),
        DROPPED.load(Ordering::Relaxed),
        ERRORS.load(Ordering::Relaxed),
    );
    let mut data = heapless::Vec::<u8, 97>::new();
    let _ = data.push(0x7D);
    let _ = data.extend_from_slice(text.as_bytes());
    let chunks = data.len().div_ceil(6);
    for (i, chunk) in data.chunks(6).enumerate() {
        // Status: Complete, Start, Continue, End (M2-104 7.7).
        let status = match (i, chunks) {
            (_, 1) => 0x0,
            (0, _) => 0x1,
            (i, n) if i + 1 == n => 0x3,
            _ => 0x2,
        };
        let mut b = [0u8; 6];
        b[..chunk.len()].copy_from_slice(chunk);
        let w0 = 0x3000_0000
            | group << 24
            | status << 20
            | (chunk.len() as u32) << 16
            | u32::from(b[0]) << 8
            | u32::from(b[1]);
        let w1 = u32::from_be_bytes([b[2], b[3], b[4], b[5]]);
        let _ = ECHO.try_send(([w0, w1, 0, 0], 2));
    }
}

/// Restarts the chip in the ROM download mode on USB-Serial-JTAG (ESP32-S3 TRM, chapter "Chip
/// Boot Control"): `esp-hal` gives the PHY to the OTG function in the RTC register `USB_CONF`,
/// which a software reset keeps, so this disconnects from the host and gives the PHY back first.
fn download_mode() -> ! {
    // A USB soft disconnect first (DCTL.SftDiscon of the OTG controller, then 200 ms): without it
    // the host keeps the old device, and the ROM download port (303A:1001) never appears (found on
    // 2026-10-23 and again in T-016 on 2026-10-25).
    esp_hal::peripherals::USB0::regs()
        .dctl()
        .modify(|_, w| w.sftdiscon().set_bit());
    esp_hal::delay::Delay::new().delay_millis(200);
    let rtc = esp_hal::peripherals::LPWR::regs();
    rtc.usb_conf().modify(|_, w| {
        w.sw_hw_usb_phy_sel()
            .clear_bit()
            .sw_usb_phy_sel()
            .clear_bit()
    });
    rtc.option1()
        .modify(|_, w| w.force_download_boot().set_bit());
    esp_hal::system::software_reset()
}
