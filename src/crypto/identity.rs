//! A per-device Olm identity wrapping `vodozemac::olm::Account`.
//!
//! Each logged-in client owns one `DeviceAccount`. The pickled state lives
//! only on the client; the server ever sees only the public identity keys
//! (and, once step 3 lands, published one-time pre-keys).

use thiserror::Error;
use vodozemac::olm::{Account, IdentityKeys};
use vodozemac::LibolmPickleError;

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
