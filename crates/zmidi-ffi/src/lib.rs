//! The C ABI of zetaohm-midi (DEC-019): one header, `zmidi_` names, for C, C++ and the language
//! bindings in `bindings/`.
//!
//! - The crate is `no_std` and needs no `alloc` (DEC-002). The program gives the storage of each
//!   object; the C ABI has no callbacks (DEC-019 rules 1 to 6).
//! - Builds: `cargo rustc --lib --crate-type staticlib` (or `cdylib`) with the feature
//!   `panic-handler` for bare metal and WASM, and with the feature `std` for the host.
//!
//! The module structure is defined by the module specs in `specs/modules/`.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "ble")]
pub mod ble;
#[cfg(feature = "ci-device")]
pub mod ci;
#[cfg(feature = "file")]
pub mod file;
#[cfg(feature = "io")]
pub mod io;
#[cfg(feature = "net")]
pub mod net;
#[cfg(feature = "ci-property-exchange")]
pub mod pe;
pub mod status;
#[cfg(feature = "ump")]
pub mod ump;
#[cfg(feature = "usb")]
pub mod usb;

/// Stops on a panic. The C ABI does not panic on input of the program; this handler is for a
/// defect only.
#[cfg(all(
    feature = "panic-handler",
    not(feature = "std"),
    any(target_os = "none", target_family = "wasm")
))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
