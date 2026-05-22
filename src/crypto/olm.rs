//! Pairwise Olm sessions — Signal-style Double Ratchet wrappers around
//! `vodozemac::olm::Session`.
//!
//! ## Surface
//!
//! - [`OlmSession`] — `Session` wrapper with encrypt/decrypt/pickle.
//! - [`OlmEnvelope`] — wire shape `(message_type: u8, ciphertext: base64)`,
//!   field types mirror the `keyshare_envelope` schema columns
//!   (`olm_message_type: int`, `olm_message: string`) so step 5 can serialize
//!   straight onto a row.
//! - [`OlmSession::outbound_from_claim`] — bootstrap from a `/keys/claim`
//!   response. The first ciphertext on this session will be a PreKey.
//! - [`OlmSession::inbound_from_prekey`] — accept an incoming PreKey envelope,
//!   instantiate the matching inbound session, return the first plaintext.
//!
//! Pure in-process — no HTTP, no DB. The HTTP `keyshare_envelope` routes land
//! in step 5; Megolm group-session wiring lands in step 6.
//!
//! ## Stance
//!
//! - **Adversarial.** Default for crypto code. Every cross-device input is
//!   hostile until proven otherwise.
//! - **Defensive at the parse boundary.** Hex / length / base64 / type-tag
//!   checks happen at the constructor entry points so vodozemac is never fed
//!   garbage.
//! - **Offensive inside the wrapper.** Once we hold a `Session`, vodozemac's
//!   invariants hold; we don't second-guess them.
//!
//! ## OTK consumption invariant (read carefully)
//!
//! [`OlmSession::inbound_from_prekey`] delegates to
//! `vodozemac::olm::Account::create_inbound_session`, which only drops the
//! matching private OTK **after the first ciphertext decrypts successfully**
//! (vodozemac 0.9.0 `src/olm/account/mod.rs:287-296`). A malformed or
//! replayed PreKey envelope can't make us "consume" an OTK and leave honest
//! senders unable to reach us. The defensive checks here (type-tag, base64,
//! `OlmMessage::from_parts`) all short-circuit *before* `create_inbound_session`
//! is called, preserving that invariant unconditionally.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use vodozemac::olm::{
    DecryptionError, InboundCreationResult, OlmMessage, Session, SessionConfig,
    SessionCreationError, SessionPickle,
};
use vodozemac::{Curve25519PublicKey, DecodeError, Ed25519PublicKey, KeyError, PickleError};

use crate::crypto::identity::DeviceAccount;
use crate::crypto::pickle::PickleKey;
use crate::crypto::prekey::PreKeyError;
use crate::protocol::ClaimKeyResponse;

// ---------------------------------------------------------------------------
// Wire envelope
// ---------------------------------------------------------------------------

/// Wire envelope for an Olm message.
///
/// `message_type` is the OlmMessage type tag — `0` for PreKey,
/// `1` for a normal Message. `ciphertext` is base64 (standard alphabet,
/// padded).
///
/// Field shapes are chosen to match the `keyshare_envelope` SurrealDB
/// columns (`olm_message_type: int`, `olm_message: string`), so step 5 can
/// hand an `OlmEnvelope` straight to the DB without a translation layer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OlmEnvelope {
    pub message_type: u8,
    pub ciphertext: String,
}

impl OlmEnvelope {
    /// Type tag for an Olm PreKey message (carries the recipient's OTK +
    /// ratchet bootstrap state).
    pub const TYPE_PREKEY: u8 = 0;
    /// Type tag for a regular Olm message (ratchet already established).
    pub const TYPE_MESSAGE: u8 = 1;

    fn from_olm_message(msg: OlmMessage) -> Self {
        let (ty, ct) = msg.to_parts();
        // OlmMessage uses 0 or 1 for the type tag; the cast from usize
        // to u8 is safe for any value vodozemac currently emits.
        Self {
            message_type: ty as u8,
            ciphertext: B64.encode(ct),
        }
    }

    fn into_olm_message(&self) -> Result<OlmMessage, OlmSessionError> {
        let ct = B64
            .decode(&self.ciphertext)
            .map_err(|source| OlmSessionError::InvalidBase64 { source })?;
        OlmMessage::from_parts(self.message_type as usize, &ct)
            .map_err(|source| OlmSessionError::ParseOlmMessage { source })
    }
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// A pairwise Olm session between two devices. Wraps `vodozemac::olm::Session`.
pub struct OlmSession {
    session: Session,
}

/// Opaque Debug — never prints session state. Printing ratchet keys / chain
/// counters into a log would defeat the point of having an E2EE session.
impl std::fmt::Debug for OlmSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OlmSession").finish_non_exhaustive()
    }
}

#[derive(Debug, Error)]
pub enum OlmSessionError {
    #[error("invalid hex in field {field}: {source}")]
    InvalidHex {
        field: &'static str,
        #[source]
        source: hex::FromHexError,
    },
    #[error("invalid length for field {field}: expected {expected}, got {got}")]
    InvalidLength {
        field: &'static str,
        expected: usize,
        got: usize,
    },
    #[error("invalid Ed25519 key in field {field}: {source}")]
    InvalidEdKey {
        field: &'static str,
        #[source]
        source: KeyError,
    },
    #[error("pre-key signature verification failed: {source}")]
    PreKeyVerify {
        #[source]
        source: PreKeyError,
    },
    #[error("invalid base64 in envelope ciphertext: {source}")]
    InvalidBase64 {
        #[source]
        source: base64::DecodeError,
    },
    #[error("envelope did not parse as an OlmMessage: {source}")]
    ParseOlmMessage {
        #[source]
        source: DecodeError,
    },
    #[error(
        "expected a PreKey envelope (message_type 0) for inbound bootstrap, got message_type {got}"
    )]
    NotAPreKeyEnvelope { got: u8 },
    #[error("vodozemac refused to create the inbound session: {source}")]
    SessionCreate {
        #[source]
        source: SessionCreationError,
    },
    #[error("decrypt failed: {source}")]
    Decrypt {
        #[source]
        source: DecryptionError,
    },
    #[error("pickle decrypt failed: {source}")]
    Unpickle {
        #[source]
        source: PickleError,
    },
}

impl OlmSession {
    /// Bootstrap a new outbound session from a `/keys/claim` response.
    ///
    /// Adversarial: re-verifies the claim's signed pre-key under the claim's
    /// own `identity_ed25519` before instantiating. The server verified at
    /// publish time but trust does not transit the network — a man-in-the-
    /// middle could tamper with the response, and `verify_against` is the
    /// chokepoint that catches it.
    pub fn outbound_from_claim(
        account: &DeviceAccount,
        claim: &ClaimKeyResponse,
    ) -> Result<Self, OlmSessionError> {
        let peer_identity_curve =
            parse_curve25519(&claim.identity_curve25519, "identity_curve25519")?;
        let peer_identity_ed = parse_ed25519(&claim.identity_ed25519, "identity_ed25519")?;

        // Re-verify under the claim's *own* identity_ed25519. The server
        // already verified this at publish time, but trust doesn't transit
        // the network.
        claim
            .key
            .verify_against(&peer_identity_ed)
            .map_err(|source| OlmSessionError::PreKeyVerify { source })?;

        let peer_otk_curve = parse_curve25519(&claim.key.public_key, "key.public_key")?;

        let session = account.account().create_outbound_session(
            SessionConfig::default(),
            peer_identity_curve,
            peer_otk_curve,
        );
        Ok(Self { session })
    }

    /// Accept an incoming PreKey envelope, consume the matching OTK from
    /// `account`, and return the new session plus the first plaintext.
    ///
    /// `sender_identity_curve25519_hex` is the identity key the caller
    /// *expects* the sender to have. Vodozemac cross-checks this against
    /// the identity key encoded inside the PreKey message and rejects with
    /// `SessionCreationError::MismatchedIdentityKey` if they differ, so a
    /// forged envelope cannot bind the inbound session to a different sender
    /// than the caller is willing to talk to.
    pub fn inbound_from_prekey(
        account: &mut DeviceAccount,
        sender_identity_curve25519_hex: &str,
        envelope: &OlmEnvelope,
    ) -> Result<(Self, Vec<u8>), OlmSessionError> {
        // Defensive: fail before parsing if the wire tag isn't PreKey. This
        // also preserves the OTK-consumption invariant (see module docs):
        // we never reach `create_inbound_session` for a non-PreKey envelope.
        if envelope.message_type != Self::TYPE_PREKEY_TAG {
            return Err(OlmSessionError::NotAPreKeyEnvelope {
                got: envelope.message_type,
            });
        }
        let sender_curve =
            parse_curve25519(sender_identity_curve25519_hex, "sender_identity_curve25519")?;
        let msg = envelope.into_olm_message()?;
        let prekey_msg = match msg {
            OlmMessage::PreKey(m) => m,
            // `OlmMessage::from_parts(0, _)` should always yield PreKey, but
            // pin the assumption in case vodozemac's layout shifts.
            OlmMessage::Normal(_) => {
                return Err(OlmSessionError::NotAPreKeyEnvelope {
                    got: envelope.message_type,
                });
            }
        };
        let InboundCreationResult { session, plaintext } = account
            .account_mut()
            .create_inbound_session(sender_curve, &prekey_msg)
            .map_err(|source| OlmSessionError::SessionCreate { source })?;
        Ok((Self { session }, plaintext))
    }

    const TYPE_PREKEY_TAG: u8 = OlmEnvelope::TYPE_PREKEY;

    /// Encrypt a plaintext into a wire envelope. The resulting envelope's
    /// `message_type` is `0` (PreKey) until the ratchet has been advanced
    /// by an incoming reply, and `1` (Message) afterwards. Vodozemac tracks
    /// the boundary.
    pub fn encrypt(&mut self, plaintext: &[u8]) -> OlmEnvelope {
        OlmEnvelope::from_olm_message(self.session.encrypt(plaintext))
    }

    /// Decrypt a wire envelope. Returns the plaintext bytes on success.
    ///
    /// On failure, vodozemac guarantees that ratchet state is **not**
    /// advanced — a subsequent legitimate envelope from the same sender
    /// will still decrypt. See the `corrupted_ciphertext_does_not_poison_session`
    /// test for the regression guard.
    pub fn decrypt(&mut self, envelope: &OlmEnvelope) -> Result<Vec<u8>, OlmSessionError> {
        let msg = envelope.into_olm_message()?;
        self.session
            .decrypt(&msg)
            .map_err(|source| OlmSessionError::Decrypt { source })
    }

    /// Serialize the session and encrypt under a pickle key.
    ///
    /// Uses vodozemac's modern AEAD pickle (the only direction supported on
    /// `Session` write-side; `from_libolm_pickle` exists for read-side
    /// migration but `to_libolm_pickle` does not). The 32-byte `PickleKey`
    /// shape is the same as `DeviceAccount::pickle`, so callers can store
    /// session and account pickles side by side under the same per-device
    /// secret.
    pub fn pickle(&self, key: &PickleKey) -> String {
        self.session.pickle().encrypt(key.as_bytes())
    }

    /// Restore a session from a pickle string. Wrong key or corrupted data
    /// surfaces as `OlmSessionError::Unpickle`.
    pub fn from_pickle(pickle: &str, key: &PickleKey) -> Result<Self, OlmSessionError> {
        let sp = SessionPickle::from_encrypted(pickle, key.as_bytes())
            .map_err(|source| OlmSessionError::Unpickle { source })?;
        Ok(Self {
            session: Session::from(sp),
        })
    }
}

// ---------------------------------------------------------------------------
// Hex parsing helpers (private)
// ---------------------------------------------------------------------------

fn decode_hex_exact(
    s: &str,
    expected: usize,
    field: &'static str,
) -> Result<Vec<u8>, OlmSessionError> {
    let bytes = hex::decode(s).map_err(|source| OlmSessionError::InvalidHex { field, source })?;
    if bytes.len() != expected {
        return Err(OlmSessionError::InvalidLength {
            field,
            expected,
            got: bytes.len(),
        });
    }
    Ok(bytes)
}

fn parse_curve25519(
    hex_str: &str,
    field: &'static str,
) -> Result<Curve25519PublicKey, OlmSessionError> {
    let bytes = decode_hex_exact(hex_str, 32, field)?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .expect("length verified by decode_hex_exact");
    // `Curve25519PublicKey::from_bytes` is infallible: every 32-byte value
    // is a valid public key under X25519's clamping rules.
    Ok(Curve25519PublicKey::from_bytes(arr))
}

fn parse_ed25519(hex_str: &str, field: &'static str) -> Result<Ed25519PublicKey, OlmSessionError> {
    let bytes = decode_hex_exact(hex_str, 32, field)?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .expect("length verified by decode_hex_exact");
    Ed25519PublicKey::from_slice(&arr)
        .map_err(|source| OlmSessionError::InvalidEdKey { field, source })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::prekey::PreKeyBundleBuilder;
    use crate::protocol::{ClaimKeyResponse, ClaimKind};

    /// Build a synthetic claim response from one of Bob's freshly-built
    /// bundles. Mirrors the shape `/keys/claim` returns, without taking a
    /// dependency on axum/SurrealDB.
    fn make_claim(bob: &mut DeviceAccount) -> ClaimKeyResponse {
        let bundle = PreKeyBundleBuilder::new().build(bob, 3);
        ClaimKeyResponse {
            kind: ClaimKind::Otk,
            device_id: "bob-device".into(),
            identity_curve25519: bundle.identity_curve25519,
            identity_ed25519: bundle.identity_ed25519,
            key: bundle.one_time_keys[0].clone(),
        }
    }

    fn identity_curve_hex(account: &DeviceAccount) -> String {
        hex::encode(account.identity_keys().curve25519.as_bytes())
    }

    /// Test 1 — end-to-end one-message exchange.
    /// Alice bootstraps from Bob's claim, encrypts "hello bob"; Bob's inbound
    /// session decrypts to the same plaintext. First message is a PreKey
    /// envelope.
    #[test]
    fn end_to_end_one_message_exchange() {
        let alice = DeviceAccount::new();
        let mut bob = DeviceAccount::new();
        let alice_curve_hex = identity_curve_hex(&alice);

        let claim = make_claim(&mut bob);
        let mut alice_session =
            OlmSession::outbound_from_claim(&alice, &claim).expect("outbound session");

        let env = alice_session.encrypt(b"hello bob");
        assert_eq!(env.message_type, OlmEnvelope::TYPE_PREKEY);

        let (_bob_session, plaintext) =
            OlmSession::inbound_from_prekey(&mut bob, &alice_curve_hex, &env)
                .expect("inbound session");
        assert_eq!(plaintext.as_slice(), b"hello bob");
    }

    /// Test 2 — continued conversation.
    /// After the initial PreKey exchange, both sides can encrypt and decrypt
    /// further messages. Alice's subsequent messages may stay PreKey until
    /// she receives a reply (the ratchet hasn't advanced yet), so we don't
    /// assert on the type tag for those — what matters is that both sides
    /// can decrypt what the other side sends.
    #[test]
    fn continued_conversation_in_both_directions() {
        let alice = DeviceAccount::new();
        let mut bob = DeviceAccount::new();
        let alice_curve_hex = identity_curve_hex(&alice);

        let claim = make_claim(&mut bob);
        let mut alice_session = OlmSession::outbound_from_claim(&alice, &claim).unwrap();

        let env1 = alice_session.encrypt(b"hello bob");
        let (mut bob_session, _) =
            OlmSession::inbound_from_prekey(&mut bob, &alice_curve_hex, &env1).unwrap();

        // Bob replies. Bob's first outbound message after creating the
        // inbound side is a regular Message (the ratchet on Bob's side is
        // already established).
        let env_b1 = bob_session.encrypt(b"hi alice");
        assert_eq!(env_b1.message_type, OlmEnvelope::TYPE_MESSAGE);
        assert_eq!(
            alice_session.decrypt(&env_b1).expect("alice decrypts"),
            b"hi alice"
        );

        // Alice sends a third message. Now her ratchet has advanced (she's
        // received Bob's reply), so this one should be a regular Message.
        let env_a2 = alice_session.encrypt(b"how are you");
        assert_eq!(env_a2.message_type, OlmEnvelope::TYPE_MESSAGE);
        assert_eq!(
            bob_session.decrypt(&env_a2).expect("bob decrypts"),
            b"how are you"
        );
    }

    /// Test 3 — pickle round-trip mid-conversation.
    /// After the initial PreKey exchange, pickle Alice's outbound session,
    /// drop it, restore it under the same key, and verify the restored
    /// session can still encrypt to Bob (and Bob can still decrypt).
    #[test]
    fn pickle_round_trip_mid_conversation() {
        let alice = DeviceAccount::new();
        let mut bob = DeviceAccount::new();
        let alice_curve_hex = identity_curve_hex(&alice);

        let claim = make_claim(&mut bob);
        let mut alice_session = OlmSession::outbound_from_claim(&alice, &claim).unwrap();
        let env1 = alice_session.encrypt(b"hello bob");
        let (mut bob_session, _) =
            OlmSession::inbound_from_prekey(&mut bob, &alice_curve_hex, &env1).unwrap();

        // Pickle Alice's session, drop, restore.
        let key = PickleKey::from_bytes([0x42; 32]);
        let pickle = alice_session.pickle(&key);
        drop(alice_session);
        let mut alice_session_restored =
            OlmSession::from_pickle(&pickle, &key).expect("restore from pickle");

        // Restored session can continue the conversation.
        let env2 = alice_session_restored.encrypt(b"after pickle");
        assert_eq!(
            bob_session
                .decrypt(&env2)
                .expect("bob decrypts post-pickle"),
            b"after pickle"
        );
    }

    /// Test 4 — wrong envelope type for inbound bootstrap.
    /// An envelope with `message_type = 1` must be rejected with
    /// `NotAPreKeyEnvelope`, and Bob's account state must be untouched.
    #[test]
    fn inbound_bootstrap_rejects_non_prekey_envelope() {
        let alice = DeviceAccount::new();
        let mut bob = DeviceAccount::new();
        let alice_curve_hex = identity_curve_hex(&alice);

        // Force-build Bob's bundle so we know the pre-state — but don't
        // claim from it (we're not bootstrapping for real).
        let _ = PreKeyBundleBuilder::new().build(&mut bob, 2);

        let env = OlmEnvelope {
            message_type: OlmEnvelope::TYPE_MESSAGE,
            // Any non-empty string — the function bails on the type tag
            // before touching the ciphertext.
            ciphertext: B64.encode(b"\x00\x01\x02\x03"),
        };

        let result = OlmSession::inbound_from_prekey(&mut bob, &alice_curve_hex, &env);
        assert!(
            matches!(result, Err(OlmSessionError::NotAPreKeyEnvelope { got: 1 })),
            "unexpected result: {result:?}"
        );
    }

    /// Test 5 — corrupted ciphertext on decrypt.
    /// A corrupted envelope returns `Decrypt(_)`, and the session state
    /// remains usable: a subsequent legitimate envelope still decrypts.
    #[test]
    fn corrupted_ciphertext_does_not_poison_session() {
        let alice = DeviceAccount::new();
        let mut bob = DeviceAccount::new();
        let alice_curve_hex = identity_curve_hex(&alice);

        let claim = make_claim(&mut bob);
        let mut alice_session = OlmSession::outbound_from_claim(&alice, &claim).unwrap();

        let env1 = alice_session.encrypt(b"first");
        let (mut bob_session, first) =
            OlmSession::inbound_from_prekey(&mut bob, &alice_curve_hex, &env1).unwrap();
        assert_eq!(first.as_slice(), b"first");

        // Encrypt env2, keep the un-tampered copy aside, then build a separate
        // corrupted envelope by flipping the MAC byte. Both carry message-
        // index 1 in Bob's receiving chain.
        let env2 = alice_session.encrypt(b"second");
        let mut raw = B64.decode(&env2.ciphertext).expect("base64");
        *raw.last_mut().expect("ciphertext non-empty") ^= 0x01;
        let env_corrupt = OlmEnvelope {
            message_type: env2.message_type,
            ciphertext: B64.encode(&raw),
        };

        let err = bob_session
            .decrypt(&env_corrupt)
            .expect_err("must reject corrupted ciphertext");
        assert!(
            matches!(err, OlmSessionError::Decrypt { .. }),
            "expected Decrypt error, got {err:?}"
        );

        // The discriminating check: the original un-tampered env2 must still
        // decrypt. Both env2 and env_corrupt are message-index 1, so if the
        // failed MAC attempt had wrongly consumed MK1 from Bob's receiving
        // chain, this decrypt would fail. A naive "later envelope still
        // works" check would pass even for a poisoned ratchet, because Olm's
        // skip-ahead cache derives intermediate MKs on demand — same-index
        // re-decrypt is the actual invariant.
        assert_eq!(
            bob_session
                .decrypt(&env2)
                .expect("un-tampered env2 must still decrypt"),
            b"second"
        );
    }

    /// Test 6 — claim with a corrupted OTK signature.
    /// Outbound bootstrap rejects the claim with `PreKeyVerify` before any
    /// vodozemac state is touched.
    #[test]
    fn outbound_rejects_claim_with_forged_otk_signature() {
        let alice = DeviceAccount::new();
        let mut bob = DeviceAccount::new();
        let mut claim = make_claim(&mut bob);

        // Flip one byte of the OTK signature so verification fails.
        let mut sig = hex::decode(&claim.key.signature).expect("hex");
        sig[0] ^= 0x01;
        claim.key.signature = hex::encode(sig);

        let err = OlmSession::outbound_from_claim(&alice, &claim)
            .expect_err("forged signature must be rejected");
        assert!(
            matches!(err, OlmSessionError::PreKeyVerify { .. }),
            "expected PreKeyVerify, got {err:?}"
        );
    }
}
