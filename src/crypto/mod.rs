//! End-to-end encryption primitives for authlyn-interactive.
//!
//! Built on [`vodozemac`], Matrix's audited Rust implementation of the
//! Olm Double Ratchet (Signal-style).
//!
//! Submodules land in plan order:
//! - [`identity`]: per-device Olm `Account` wrapped as `DeviceAccount`.
//! - [`pickle`]: at-rest encryption keys for serialized session state.
//! - [`prekey`]: wire-format pre-key bundles (publish + verify).
//! - [`olm`]: pairwise Olm sessions (Double Ratchet, used to carry Megolm
//!   group-session keys between two devices).
//! - [`megolm`]: group sessions (sender-keys ratchet) for room messages.
//! - [`attachment`]: per-blob AES-256-CTR + SHA-256 wrappers (Matrix
//!   `m.encrypted` v2) for encrypted media uploads.

pub mod attachment;
pub mod identity;
pub mod megolm;
pub mod olm;
pub mod pickle;
pub mod prekey;

pub use attachment::{AttachmentError, EncryptedAttachment, EncryptedFileRef, Hashes, KeyJwk};
pub use identity::{DeviceAccount, DeviceError};
pub use megolm::{MegolmCiphertext, MegolmError, MegolmInbound, MegolmOutbound};
pub use olm::{OlmEnvelope, OlmSession, OlmSessionError};
pub use pickle::PickleKey;
pub use prekey::{PreKeyBundle, PreKeyError, SignedPreKey};
