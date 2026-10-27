//! Type-checks the BLE part of `examples/esp32s3-ble-midi` (`src/ble.rs`) on the host, with the
//! workspace toolchain. The example itself needs the Espressif Xtensa toolchain (DEC-003:
//! examples are outside the workspace); this test finds an API change of the adapter or of
//! `trouble-host` 0.7 in the example code earlier. The code does not run here: it needs a BLE
//! controller.

/// The example prints with `esp_println::println!`; here with `std::println!`.
macro_rules! log {
    ($($t:tt)*) => {
        std::println!($($t)*)
    };
}

#[allow(dead_code)]
#[path = "../../../examples/esp32s3-ble-midi/src/ble.rs"]
mod ble;

#[test]
fn example_ble_part_compiles() {
    assert_eq!(ble::NAME, "zetaohm-midi");
}
