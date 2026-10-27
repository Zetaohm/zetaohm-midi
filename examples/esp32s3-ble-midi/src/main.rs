//! BLE-MIDI echo peripheral on an ESP32-S3, with `zmidi-ble` (DEC-016) through the
//! `zmidi-ble-trouble` adapter on `trouble-host` 0.7 and `esp-radio` 1.0.0-beta.1.
//!
//! This file starts the chip and the radio; `ble.rs` has the GATT server and the echo (no
//! ESP32 code, so the adapter tests type-check it on the host).
//!
//! On macOS: Audio MIDI Setup, MIDI Studio, Bluetooth Configuration, connect "zetaohm-midi";
//! then `examples/mk20dx-usb-midi/host/echo.py` checks the echo. The tools in `host/` check the
//! GATT data and the throughput. Flash: `cargo run --release`.

#![no_std]
#![no_main]

use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use trouble_host::prelude::ExternalController;

esp_bootloader_esp_idf::esp_app_desc!();

/// The print macro of `ble.rs`.
macro_rules! log {
    ($($t:tt)*) => {
        esp_println::println!($($t)*)
    };
}

mod ble;

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(size: 72 * 1024);
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);

    // esp-radio 1.0: no `esp_radio::init()`; the connector takes the BT peripheral only.
    let connector = BleConnector::new(p.BT, Default::default()).unwrap();
    let controller: ExternalController<_, 20> = ExternalController::new(connector);
    ble::run(controller).await
}
