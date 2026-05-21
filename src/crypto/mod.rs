//! End-to-end encryption primitives for authlyn-interactive.
//!
//! Built on [`vodozemac`], Matrix's audited Rust implementation of the
//! Olm Double Ratchet (Signal-style).
//!
//! The flow is intentionally minimal at this stage:
//!
//! 1. Each participant owns an [`Identity`] (long-term Curve25519 + Ed25519 keys).
//! 2. Two participants establish a [`Session`] from one side's pre-key bundle.
//! 3. The session encrypts and decrypts messages with per-message forward secrecy.
//!
//! TODO: pre-key bundle exchange, session persistence, message routing.

use vodozemac::olm::{Account, IdentityKeys, OlmMessage, Session};

pub struct Identity {
    account: Account,
}

impl Identity {
    pub fn new() -> Self {
        Self {
            account: Account::new(),
        }
    }

    pub fn keys(&self) -> IdentityKeys {
        self.account.identity_keys()
    }

    pub fn account_mut(&mut self) -> &mut Account {
        &mut self.account
    }
}

impl Default for Identity {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Channel {
    session: Session,
}

impl Channel {
    pub fn from_session(session: Session) -> Self {
        Self { session }
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> OlmMessage {
        self.session.encrypt(plaintext)
    }

    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_has_stable_keys() {
        let id = Identity::new();
        let keys_a = id.keys();
        let keys_b = id.keys();
        assert_eq!(keys_a.curve25519, keys_b.curve25519);
        assert_eq!(keys_a.ed25519, keys_b.ed25519);
    }
}
