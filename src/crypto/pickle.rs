//! Pickle keys for at-rest encryption of vodozemac session state.
//!
//! v0 uses `vodozemac::olm::Account::to_libolm_pickle` (AES-CBC + HMAC-SHA256)
//! with a 32-byte symmetric key the client holds locally. Password-derived
//! keys (argon2id over the user password) land in the auth follow-up plan;
//! the `PickleKey` newtype hides the representation so we can swap in a
//! modern AEAD or change the derivation without touching callers.

use rand::RngCore;

#[derive(Clone)]
pub struct PickleKey([u8; 32]);

impl PickleKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn random() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
