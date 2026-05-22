//! A per-device Olm identity wrapping `vodozemac::olm::Account`.
//!
//! Each logged-in client owns one `DeviceAccount`. The pickled state lives
//! only on the client; the server ever sees only the public identity keys
//! (and, once step 3 lands, published one-time pre-keys).

use thiserror::Error;
use vodozemac::olm::{Account, IdentityKeys};
use vodozemac::{Curve25519PublicKey, Ed25519Signature, KeyId, LibolmPickleError};

use super::pickle::PickleKey;

#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("pickle encoding failed: {0}")]
    Pickle(#[from] LibolmPickleError),
}

pub struct DeviceAccount {
    account: Account,
}

impl DeviceAccount {
    pub fn new() -> Self {
        Self {
            account: Account::new(),
        }
    }

    pub fn identity_keys(&self) -> IdentityKeys {
        self.account.identity_keys()
    }

    /// Encrypt the account state with the given pickle key. The resulting
    /// string is opaque base64; persist it as-is.
    pub fn pickle(&self, key: &PickleKey) -> Result<String, DeviceError> {
        Ok(self.account.to_libolm_pickle(key.as_bytes())?)
    }

    /// Reconstruct an account from a pickled string + pickle key. Returns
    /// `DeviceError::Pickle` if the key is wrong or the data is corrupt.
    pub fn from_pickle(pickle: &str, key: &PickleKey) -> Result<Self, DeviceError> {
        let account = Account::from_libolm_pickle(pickle, key.as_bytes())?;
        Ok(Self { account })
    }

    /// Mint `count` fresh one-time keys.
    ///
    /// They are appended to the account's unpublished set; the caller is
    /// expected to publish them and then call [`Self::mark_keys_as_published`].
    pub fn generate_one_time_keys(&mut self, count: usize) {
        let _ = self.account.generate_one_time_keys(count);
    }

    /// Mint a fresh fallback key, displacing the previous one.
    pub fn generate_fallback_key(&mut self) {
        let _ = self.account.generate_fallback_key();
    }

    /// Sign an arbitrary message with the device's identity Ed25519 key.
    pub fn sign(&self, message: &[u8]) -> Ed25519Signature {
        self.account.sign(message)
    }

    /// Snapshot of OTKs that have been generated locally but not yet
    /// marked as published. Returned as `(KeyId, public_key)` pairs.
    pub fn unpublished_one_time_keys(&self) -> Vec<(KeyId, Curve25519PublicKey)> {
        self.account.one_time_keys().into_iter().collect()
    }

    /// The currently unpublished fallback key, if any. Returns `None` once
    /// it's been marked as published.
    pub fn unpublished_fallback_key(&self) -> Option<(KeyId, Curve25519PublicKey)> {
        self.account.fallback_key().into_iter().next()
    }

    /// Tell the underlying account that everything in the unpublished set
    /// has now been sent to the server. After this, [`Self::unpublished_one_time_keys`]
    /// and [`Self::unpublished_fallback_key`] return empty until new keys
    /// are generated.
    pub fn mark_keys_as_published(&mut self) {
        self.account.mark_keys_as_published();
    }
}

impl Default for DeviceAccount {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_keys_are_stable_within_one_instance() {
        let device = DeviceAccount::new();
        let a = device.identity_keys();
        let b = device.identity_keys();
        assert_eq!(a.curve25519, b.curve25519);
        assert_eq!(a.ed25519, b.ed25519);
    }

    #[test]
    fn pickle_roundtrip_preserves_identity() {
        let key = PickleKey::from_bytes([0xAB; 32]);
        let device = DeviceAccount::new();
        let original = device.identity_keys();

        let pickle = device.pickle(&key).expect("pickle should succeed");
        let restored = DeviceAccount::from_pickle(&pickle, &key).expect("unpickle should succeed");
        let restored_keys = restored.identity_keys();

        assert_eq!(original.curve25519, restored_keys.curve25519);
        assert_eq!(original.ed25519, restored_keys.ed25519);
    }

    #[test]
    fn pickle_unpickle_with_wrong_key_fails() {
        let key = PickleKey::from_bytes([0xAB; 32]);
        let wrong = PickleKey::from_bytes([0xCD; 32]);
        let device = DeviceAccount::new();

        let pickle = device.pickle(&key).unwrap();
        let result = DeviceAccount::from_pickle(&pickle, &wrong);

        assert!(result.is_err(), "decrypting with wrong key should fail");
    }

    #[test]
    fn two_new_accounts_have_distinct_identities() {
        let a = DeviceAccount::new();
        let b = DeviceAccount::new();
        let ka = a.identity_keys();
        let kb = b.identity_keys();
        assert_ne!(ka.curve25519, kb.curve25519);
        assert_ne!(ka.ed25519, kb.ed25519);
    }
}
