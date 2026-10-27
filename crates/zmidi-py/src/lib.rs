//! The Python extension of zetaohm-midi (DEC-019): PyO3 classes and functions over the Rust crates,
//! imported as `zetaohm_midi._native` by the package in `bindings/python`.
//!
//! - A host-only crate (std); maturin builds it through `bindings/python`.
//! - The module structure is defined by the module specs in `specs/modules/`.

pub mod ble;
pub mod ci;
pub mod file;
pub mod io;
pub mod native;
pub mod net;
pub mod pe;
pub mod ump;
pub mod usb;

use pyo3::PyErr;

/// An [`native::InvalidValueError`] with `msg`.
pub(crate) fn invalid(msg: impl Into<String>) -> PyErr {
    native::InvalidValueError::new_err(msg.into())
}

/// A [`native::MalformedError`] with `msg`.
pub(crate) fn malformed(msg: impl Into<String>) -> PyErr {
    native::MalformedError::new_err(msg.into())
}

/// A [`native::RejectedError`] with `msg`.
pub(crate) fn rejected(msg: impl Into<String>) -> PyErr {
    native::RejectedError::new_err(msg.into())
}
