//! End-to-end encryption primitives for authlyn-interactive.
//!
//! Built on [`vodozemac`], Matrix's audited Rust implementation of the
//! Olm Double Ratchet (Signal-style).
//!
//! Submodules land in plan order:
//! - [`identity`]: per-device Olm `Account` wrapped as `DeviceAccount`.
//! - [`pickle`]: at-rest encryption keys for serialized session state.
//!
//! Coming next: `prekey`, `olm`, `megolm`, `attachment`.

pub mod identity;
pub mod pickle;

pub use identity::{DeviceAccount, DeviceError};
pub use pickle::PickleKey;
