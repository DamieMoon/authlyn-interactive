//! Megolm group sessions — sender-keys-style group ratchet wrappers around
//! `vodozemac::megolm::{GroupSession, InboundGroupSession}`.
//!
//! ## Surface
//!
//! - [`MegolmOutbound`] — `GroupSession` wrapper. The sender holds one per
//!   active sending session in a room; rotation policy lives one layer up
//!   (step 7).
//! - [`MegolmInbound`] — `InboundGroupSession` wrapper, one per
//!   `(room, sender_device, session_id)` triple the receiver knows about.
//! - [`MegolmCiphertext`] — wire shape for a single encrypted message.
//!   Field types mirror the `message` table columns (`message_index: int`,
//!   `ciphertext: string`) so step 8 can serialize straight onto a row;
//!   `megolm_session_id` rides separately on the row (callers fetch it via
//!   [`MegolmOutbound::session_id`]).
//! - [`DecryptedMessage`] — re-export from vodozemac. It's an inert value
//!   type (`plaintext: Vec<u8>` + `message_index: u32`); wrapping adds no
//!   defence.
//!
//! Pure in-process — no HTTP, no DB. Key-share fanout uses the
//! [`OlmSession`](crate::crypto::OlmSession) wrapper from step 4 to ship
//! [`MegolmOutbound::session_key_base64`] across to peers; the routing rows
//! that carry those Olm-wrapped envelopes are step 5's `keyshare_envelope`.
//!
//! ## Stance
//!
//! - **Adversarial.** Default for crypto code. Every cross-device input is
//!   hostile until proven otherwise.
//! - **Defensive at the parse boundary.** Base64 and type checks happen at
//!   the constructor entry points so vodozemac is never fed garbage.
//!   [`SessionKey::from_base64`] is itself defensive (signature-verifies the
//!   inner Ed25519 sig on bootstrap), and [`ExportedSessionKey::from_base64`]
//!   deliberately is not (catch-up exports are unsigned by design).
//! - **Offensive inside the wrapper.** Once we hold a `GroupSession` /
//!   `InboundGroupSession`, vodozemac's invariants hold; we don't
//!   second-guess them.
//!
//! ## Version pin
//!
//! Every session is constructed with `SessionConfig::version_1()`. Vodozemac's
//! `SessionConfig::default()` is V2; pinning V1 explicitly at every
//! construction site keeps outbound and inbound from drifting (a V1 outbound
//! against a default-config inbound would surface as a confusing
//! `InvalidMACLength`, not a "configurations don't match" error).

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use thiserror::Error;
pub use vodozemac::megolm::{DecryptedMessage, ExportedSessionKey};
use vodozemac::megolm::{
    DecryptionError, GroupSession, GroupSessionPickle, InboundGroupSession,
    InboundGroupSessionPickle, MegolmMessage, SessionConfig, SessionKey, SessionKeyDecodeError,
};
use vodozemac::{DecodeError, PickleError};

use crate::crypto::pickle::PickleKey;

// ---------------------------------------------------------------------------
// Wire ciphertext
// ---------------------------------------------------------------------------

/// Wire shape for a Megolm ciphertext.
///
/// `message_index` is the sender's outbound ratchet position at the moment
/// of encrypt (vodozemac increments after each `encrypt`). `ciphertext` is
/// the base64-encoded [`MegolmMessage`] (standard alphabet, padded).
///
/// Field shapes are chosen to match the `message` SurrealDB columns
/// (`message_index: int`, `ciphertext: string`), so step 8 can hand a
/// `MegolmCiphertext` to the DB with no translation layer. The
/// `megolm_session_id` column rides separately on the row — callers fetch
/// it via [`MegolmOutbound::session_id`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MegolmCiphertext {
    pub message_index: u32,
    pub ciphertext: String,
}

impl MegolmCiphertext {
    fn from_megolm_message(msg: MegolmMessage) -> Self {
        Self {
            message_index: msg.message_index(),
            ciphertext: B64.encode(msg.to_bytes()),
        }
    }

    fn into_megolm_message(&self) -> Result<MegolmMessage, MegolmError> {
        // Vodozemac's `MegolmMessage::from_base64` handles both the base64
        // decode and the inner version/proto parse; `DecodeError` already
        // carries a `Base64` variant so one wrapper arm covers both.
        MegolmMessage::from_base64(&self.ciphertext)
            .map_err(|source| MegolmError::InvalidMessageBase64 { source })
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors raised by the [`MegolmOutbound`] / [`MegolmInbound`] wrappers.
///
/// Decode-side variants split by which constructor produced them so callers
/// can tell "I tried to bootstrap from a session key" from "I tried to
/// catch-up import an exported key" from "I tried to decrypt a wire
/// message". The decrypt-side variant is intentionally a single arm —
/// vodozemac's `DecryptionError` already discriminates signature / MAC /
/// padding / unknown-index internally, and callers shouldn't branch on
/// those at the routing layer.
#[derive(Debug, Error)]
pub enum MegolmError {
    /// `MegolmInbound::from_session_key_base64` was given a string that
    /// vodozemac refused to parse as a `SessionKey`. Covers base64 errors,
    /// version mismatch, bad signature, and bad inner public key.
    #[error("invalid session-key base64: {source}")]
    InvalidSessionKeyBase64 {
        #[source]
        source: SessionKeyDecodeError,
    },
    /// `MegolmInbound::from_exported_base64` was given a string that
    /// vodozemac refused to parse as an `ExportedSessionKey`. Same source
    /// type as above; the discriminating variant tells the caller which
    /// constructor failed.
    #[error("invalid exported-key base64: {source}")]
    InvalidExportedKeyBase64 {
        #[source]
        source: SessionKeyDecodeError,
    },
    /// A `MegolmCiphertext`'s `ciphertext` field could not be parsed as a
    /// vodozemac `MegolmMessage`. Vodozemac's `DecodeError` distinguishes
    /// base64 failures from inner protobuf / version / length failures
    /// internally; callers shouldn't branch on those at the routing layer.
    #[error("ciphertext did not parse as a MegolmMessage: {source}")]
    InvalidMessageBase64 {
        #[source]
        source: DecodeError,
    },
    /// Vodozemac rejected a decrypt — bad signature, bad MAC, bad padding,
    /// or an index the session no longer has the key material for.
    #[error("decrypt failed: {source}")]
    Decrypt {
        #[source]
        source: DecryptionError,
    },
    /// Pickle decryption failed — wrong key or corrupted ciphertext.
    #[error("pickle decrypt failed: {source}")]
    Unpickle {
        #[source]
        source: PickleError,
    },
}

// ---------------------------------------------------------------------------
// Outbound (sender-side) session
// ---------------------------------------------------------------------------

/// The sender's side of a Megolm group session. One per active sending
/// session per room. Wraps `vodozemac::megolm::GroupSession`.
pub struct MegolmOutbound {
    session: GroupSession,
}

/// Opaque Debug — never prints session state. The signing keypair and
/// ratchet bytes are the entire security of the session; leaking them into
/// a log defeats the point.
impl std::fmt::Debug for MegolmOutbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MegolmOutbound").finish_non_exhaustive()
    }
}

impl MegolmOutbound {
    /// Mint a fresh outbound session, version-1 explicitly.
    pub fn new() -> Self {
        Self {
            session: GroupSession::new(SessionConfig::version_1()),
        }
    }

    /// The session ID. Globally unique with overwhelming probability —
    /// derived from the public half of the per-session Ed25519 keypair, so
    /// two new sessions on the same device still have distinct IDs.
    /// Recipients use this to route ciphertexts to the right
    /// [`MegolmInbound`].
    pub fn session_id(&self) -> String {
        self.session.session_id()
    }

    /// The bootstrap key, base64-encoded. Ship this to each recipient via
    /// an Olm-pairwise channel (step 4 / step 5); they instantiate a
    /// matching [`MegolmInbound`] with [`MegolmInbound::from_session_key_base64`].
    ///
    /// Vodozemac signs the key with the session's Ed25519 keypair on its
    /// way out, and `SessionKey::from_base64` verifies that signature on
    /// the way in — so a network attacker can't substitute a different
    /// session key for the same `session_id`.
    pub fn session_key_base64(&self) -> String {
        self.session.session_key().to_base64()
    }

    /// Encrypt a plaintext into a wire ciphertext. The sender's outbound
    /// ratchet advances by one position per call; the returned
    /// [`MegolmCiphertext`]'s `message_index` is the position *at the time
    /// of encrypt* (i.e. the index recipients will use to derive the
    /// matching message key).
    pub fn encrypt(&mut self, plaintext: &[u8]) -> MegolmCiphertext {
        MegolmCiphertext::from_megolm_message(self.session.encrypt(plaintext))
    }

    /// Serialize the session and encrypt under a pickle key. Uses
    /// vodozemac's modern AEAD pickle — same key shape as
    /// [`crate::crypto::OlmSession::pickle`] and [`DeviceAccount::pickle`],
    /// so a device can stash its account, all Olm sessions, and all
    /// outbound Megolm sessions under one per-device secret.
    pub fn pickle(&self, key: &PickleKey) -> String {
        self.session.pickle().encrypt(key.as_bytes())
    }

    /// Restore an outbound session from a pickle string. Wrong key or
    /// corrupted data surfaces as [`MegolmError::Unpickle`].
    pub fn from_pickle(pickle: &str, key: &PickleKey) -> Result<Self, MegolmError> {
        let p = GroupSessionPickle::from_encrypted(pickle, key.as_bytes())
            .map_err(|source| MegolmError::Unpickle { source })?;
        Ok(Self {
            session: GroupSession::from_pickle(p),
        })
    }
}

impl Default for MegolmOutbound {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Inbound (recipient-side) session
// ---------------------------------------------------------------------------

/// The recipient's side of a Megolm group session. One per
/// `(room, sender_device, session_id)` triple the recipient knows about
/// (the bookkeeping lives in step 7). Wraps
/// `vodozemac::megolm::InboundGroupSession`.
pub struct MegolmInbound {
    session: InboundGroupSession,
}

/// Opaque Debug — never prints ratchet state.
impl std::fmt::Debug for MegolmInbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MegolmInbound").finish_non_exhaustive()
    }
}

impl MegolmInbound {
    /// Bootstrap an inbound session from a signed session key (the kind
    /// [`MegolmOutbound::session_key_base64`] emits). The signature on the
    /// key is verified by vodozemac during `SessionKey::from_base64`.
    pub fn from_session_key_base64(s: &str) -> Result<Self, MegolmError> {
        let key = SessionKey::from_base64(s)
            .map_err(|source| MegolmError::InvalidSessionKeyBase64 { source })?;
        Ok(Self {
            session: InboundGroupSession::new(&key, SessionConfig::version_1()),
        })
    }

    /// Catch-up import from another recipient's [`Self::export_at_base64`].
    /// Unsigned by design — the caller is responsible for trusting the
    /// channel the exported key arrived over.
    pub fn from_exported_base64(s: &str) -> Result<Self, MegolmError> {
        let key = ExportedSessionKey::from_base64(s)
            .map_err(|source| MegolmError::InvalidExportedKeyBase64 { source })?;
        Ok(Self {
            session: InboundGroupSession::import(&key, SessionConfig::version_1()),
        })
    }

    /// The session ID. Must agree byte-for-byte with the sending
    /// [`MegolmOutbound::session_id`] for any ciphertext the recipient
    /// expects to decrypt.
    pub fn session_id(&self) -> String {
        self.session.session_id()
    }

    /// Decrypt a wire ciphertext. Returns plaintext + the encrypter's
    /// message-index on success. Vodozemac verifies the message signature
    /// against the session's Ed25519 public key (cached at bootstrap) and
    /// the per-message MAC against the derived chain key before unsealing
    /// the AES ciphertext; if either fails the call returns
    /// [`MegolmError::Decrypt`] and the session's skip-ahead cache is
    /// unaffected (`InboundGroupSession::decrypt` only mutates state on
    /// success).
    pub fn decrypt(&mut self, wire: &MegolmCiphertext) -> Result<DecryptedMessage, MegolmError> {
        let msg = wire.into_megolm_message()?;
        self.session
            .decrypt(&msg)
            .map_err(|source| MegolmError::Decrypt { source })
    }

    /// Export the session at the given message index for catch-up sharing
    /// with another recipient. Returns `None` if the session has already
    /// been ratcheted past `index` and the chain key for that point is
    /// gone — vodozemac's `InboundGroupSession::export_at` is forward-only.
    pub fn export_at_base64(&mut self, index: u32) -> Option<String> {
        self.session.export_at(index).map(|k| k.to_base64())
    }

    /// Serialize the inbound session and encrypt under a pickle key.
    pub fn pickle(&self, key: &PickleKey) -> String {
        self.session.pickle().encrypt(key.as_bytes())
    }

    /// Restore an inbound session from a pickle string. Wrong key or
    /// corrupted data surfaces as [`MegolmError::Unpickle`].
    pub fn from_pickle(pickle: &str, key: &PickleKey) -> Result<Self, MegolmError> {
        let p = InboundGroupSessionPickle::from_encrypted(pickle, key.as_bytes())
            .map_err(|source| MegolmError::Unpickle { source })?;
        Ok(Self {
            session: InboundGroupSession::from_pickle(p),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::identity::DeviceAccount;
    use crate::crypto::olm::{OlmEnvelope, OlmSession};
    use crate::crypto::prekey::PreKeyBundleBuilder;
    use crate::protocol::{ClaimKeyResponse, ClaimKind};

    /// Build a synthetic claim response from one of `device`'s freshly-built
    /// bundles. Mirrors the shape `/keys/claim` returns, without taking a
    /// dependency on axum/SurrealDB. Identical in spirit to the helper of
    /// the same name in `crypto::olm::tests`.
    fn make_claim(device: &mut DeviceAccount, device_id: &str) -> ClaimKeyResponse {
        let bundle = PreKeyBundleBuilder::new().build(device, 3);
        ClaimKeyResponse {
            kind: ClaimKind::Otk,
            device_id: device_id.into(),
            identity_curve25519: bundle.identity_curve25519,
            identity_ed25519: bundle.identity_ed25519,
            key: bundle.one_time_keys[0].clone(),
        }
    }

    fn identity_curve_hex(account: &DeviceAccount) -> String {
        hex::encode(account.identity_keys().curve25519.as_bytes())
    }

    /// Test 1 — bootstrap inbound from outbound's session_key_base64 and
    /// confirm both sides report the same session_id. The bootstrap key
    /// uniquely identifies the session; recipients can route messages to
    /// the right inbound session.
    #[test]
    fn outbound_inbound_session_ids_match() {
        let alice = MegolmOutbound::new();
        let key = alice.session_key_base64();
        let bob = MegolmInbound::from_session_key_base64(&key).expect("valid session key");
        assert_eq!(alice.session_id(), bob.session_id());
    }

    /// Test 2 — three messages from Alice, decrypted in order by Bob.
    /// Assert the plaintext round-trips and `decrypted.message_index` is
    /// 0, 1, 2. The wire `message_index` is what step 8 will persist on
    /// the `message` row, so we lock it down here.
    #[test]
    fn three_messages_alice_to_bob_decrypt_in_order() {
        let mut alice = MegolmOutbound::new();
        let mut bob = MegolmInbound::from_session_key_base64(&alice.session_key_base64()).unwrap();

        let m0 = alice.encrypt(b"m1");
        let m1 = alice.encrypt(b"m2");
        let m2 = alice.encrypt(b"m3");

        // Wire-level message_index is set at encrypt time.
        assert_eq!(m0.message_index, 0);
        assert_eq!(m1.message_index, 1);
        assert_eq!(m2.message_index, 2);

        let d0 = bob.decrypt(&m0).expect("m0");
        let d1 = bob.decrypt(&m1).expect("m1");
        let d2 = bob.decrypt(&m2).expect("m2");

        assert_eq!(d0.plaintext, b"m1");
        assert_eq!(d1.plaintext, b"m2");
        assert_eq!(d2.plaintext, b"m3");
        assert_eq!(d0.message_index, 0);
        assert_eq!(d1.message_index, 1);
        assert_eq!(d2.message_index, 2);
    }

    /// Test 3 — full end-to-end through the real step-4 Olm wrapper.
    ///
    /// Alice and Bob each hold a `DeviceAccount` and a published pre-key
    /// bundle. Each side bootstraps a pairwise [`OlmSession`] against the
    /// other via the step-4 wrappers (`outbound_from_claim` /
    /// `inbound_from_prekey`). Each side mints its own
    /// [`MegolmOutbound`], ships the session-key-base64 across the Olm
    /// channel, and the other side instantiates a matching
    /// [`MegolmInbound`].
    ///
    /// Both sides then encrypt three messages on the Megolm channel and
    /// decrypt the peer's three. *Invariant:* the wrapper composes with
    /// step 4's Olm wrapper end-to-end — no hidden coupling, no API
    /// mismatch. This is the load-bearing integration test for step 7's
    /// key-share fanout (which will sit on top of these same calls but
    /// add room bookkeeping and the `keyshare_envelope` round-trip).
    #[test]
    fn three_messages_each_direction_via_olm_keyshare() {
        let mut alice = DeviceAccount::new();
        let mut bob = DeviceAccount::new();
        let alice_curve_hex = identity_curve_hex(&alice);
        let bob_curve_hex = identity_curve_hex(&bob);

        // --- Olm channel A→B: Alice bootstraps from Bob's claim ---
        let bob_claim = make_claim(&mut bob, "bob-device");
        let mut alice_olm_to_bob =
            OlmSession::outbound_from_claim(&alice, &bob_claim).expect("alice→bob outbound");

        // Alice mints her outbound Megolm and ships the session key across
        // the Olm channel (Olm-PreKey envelope, since it's the first
        // ciphertext on this side).
        let mut alice_megolm = MegolmOutbound::new();
        let env_a_keyshare = alice_olm_to_bob.encrypt(alice_megolm.session_key_base64().as_bytes());
        assert_eq!(env_a_keyshare.message_type, OlmEnvelope::TYPE_PREKEY);

        // Bob accepts the PreKey envelope on his side, recovers the Megolm
        // session key, and instantiates the matching inbound Megolm.
        let (mut bob_olm_to_alice, recovered) =
            OlmSession::inbound_from_prekey(&mut bob, &alice_curve_hex, &env_a_keyshare)
                .expect("bob inbound olm");
        let alice_key_str = std::str::from_utf8(&recovered).expect("ascii session key");
        let mut bob_megolm_inbound_for_alice =
            MegolmInbound::from_session_key_base64(alice_key_str).expect("alice's key parses");
        assert_eq!(
            bob_megolm_inbound_for_alice.session_id(),
            alice_megolm.session_id()
        );

        // --- Olm channel B→A: Alice accepts Bob's claim ---
        let alice_claim = make_claim(&mut alice, "alice-device");
        let mut bob_olm_to_alice_outbound =
            OlmSession::outbound_from_claim(&bob, &alice_claim).expect("bob→alice outbound");

        // Bob mints his outbound Megolm and ships its key over the
        // outbound Olm session. The first ciphertext on that side will
        // be a PreKey envelope to Alice.
        let mut bob_megolm = MegolmOutbound::new();
        let env_b_keyshare =
            bob_olm_to_alice_outbound.encrypt(bob_megolm.session_key_base64().as_bytes());
        assert_eq!(env_b_keyshare.message_type, OlmEnvelope::TYPE_PREKEY);

        let (_alice_olm_inbound_from_bob, recovered) =
            OlmSession::inbound_from_prekey(&mut alice, &bob_curve_hex, &env_b_keyshare)
                .expect("alice inbound olm");
        let bob_key_str = std::str::from_utf8(&recovered).expect("ascii session key");
        let mut alice_megolm_inbound_for_bob =
            MegolmInbound::from_session_key_base64(bob_key_str).expect("bob's key parses");
        assert_eq!(
            alice_megolm_inbound_for_bob.session_id(),
            bob_megolm.session_id()
        );

        // --- Three Megolm messages each direction ---
        let a_msgs: Vec<MegolmCiphertext> = (0..3)
            .map(|i| alice_megolm.encrypt(format!("alice-{i}").as_bytes()))
            .collect();
        let b_msgs: Vec<MegolmCiphertext> = (0..3)
            .map(|i| bob_megolm.encrypt(format!("bob-{i}").as_bytes()))
            .collect();

        for (i, m) in a_msgs.iter().enumerate() {
            let d = bob_megolm_inbound_for_alice
                .decrypt(m)
                .expect("bob decrypts alice");
            assert_eq!(d.plaintext, format!("alice-{i}").as_bytes());
            assert_eq!(d.message_index as usize, i);
        }
        for (i, m) in b_msgs.iter().enumerate() {
            let d = alice_megolm_inbound_for_bob
                .decrypt(m)
                .expect("alice decrypts bob");
            assert_eq!(d.plaintext, format!("bob-{i}").as_bytes());
            assert_eq!(d.message_index as usize, i);
        }

        // Sanity: keep the bob→alice Olm session referenced so the inbound
        // bootstrap above isn't optimized away by future refactors that
        // change the assertion shape.
        let _ = &mut bob_olm_to_alice;
    }

    /// Test 4 — out-of-order decrypt then in-order decrypt.
    /// Alice encrypts m0, m1, m2. Bob decrypts m2 first, then m0, then m1.
    /// All three must succeed with correct `message_index`. *Invariant:*
    /// Megolm's skip-ahead cache works through the wrapper — receiving a
    /// later message before earlier ones doesn't lose the ability to
    /// decrypt the earlier ones.
    #[test]
    fn out_of_order_decrypt_then_in_order_decrypt() {
        let mut alice = MegolmOutbound::new();
        let mut bob = MegolmInbound::from_session_key_base64(&alice.session_key_base64()).unwrap();

        let m0 = alice.encrypt(b"m0");
        let m1 = alice.encrypt(b"m1");
        let m2 = alice.encrypt(b"m2");

        let d2 = bob.decrypt(&m2).expect("m2 first");
        assert_eq!(d2.plaintext, b"m2");
        assert_eq!(d2.message_index, 2);

        let d0 = bob.decrypt(&m0).expect("m0 after m2");
        assert_eq!(d0.plaintext, b"m0");
        assert_eq!(d0.message_index, 0);

        let d1 = bob.decrypt(&m1).expect("m1 after m0");
        assert_eq!(d1.plaintext, b"m1");
        assert_eq!(d1.message_index, 1);
    }

    /// Test 5 — outbound pickle preserves the message counter, not just
    /// identity. Alice encrypts m0, Bob decrypts. Pickle Alice's outbound,
    /// drop, restore under same key; restored outbound encrypts m1; Bob
    /// decrypts m1 with `message_index = 1`. If pickle only restored the
    /// signing key but reset the ratchet index, the restored side would
    /// produce a second message tagged 0 and Bob would either decrypt it
    /// as garbage (wrong message key) or reject the MAC.
    #[test]
    fn outbound_pickle_round_trip_preserves_continuation() {
        let key = PickleKey::from_bytes([0x42; 32]);
        let mut alice = MegolmOutbound::new();
        let mut bob = MegolmInbound::from_session_key_base64(&alice.session_key_base64()).unwrap();

        let m0 = alice.encrypt(b"m0");
        assert_eq!(m0.message_index, 0);
        let d0 = bob.decrypt(&m0).expect("m0");
        assert_eq!(d0.plaintext, b"m0");

        let pickle = alice.pickle(&key);
        let original_session_id = alice.session_id();
        drop(alice);

        let mut alice_restored = MegolmOutbound::from_pickle(&pickle, &key).expect("restore");
        assert_eq!(alice_restored.session_id(), original_session_id);

        let m1 = alice_restored.encrypt(b"m1");
        assert_eq!(
            m1.message_index, 1,
            "restored outbound must continue counter, not reset to 0"
        );
        let d1 = bob.decrypt(&m1).expect("m1 after restore");
        assert_eq!(d1.plaintext, b"m1");
        assert_eq!(d1.message_index, 1);
    }

    /// Test 6 — inbound pickle round-trip preserves enough state to keep
    /// decrypting messages produced after the pickle point. Bob decrypts
    /// m0, m1; pickle Bob's inbound, drop, restore; restored inbound
    /// decrypts m2 (which Alice encrypts after the pickle) at
    /// `message_index = 2`, and `session_id()` survives the round-trip.
    ///
    /// What this proves: pickle preserves `signing_key` and the
    /// `initial_ratchet`, which together are enough to authenticate and
    /// decrypt any message at or after the restore point. A pickle that
    /// dropped `signing_key` or mangled `initial_ratchet` would fail at
    /// the m2 decrypt; a pickle that fabricated a fresh session entirely
    /// would fail the `session_id()` equality check.
    ///
    /// What this does NOT prove (and why): vodozemac-0.9.0's
    /// `InboundGroupSession` pickle (`src/megolm/inbound_group_session.rs`
    /// lines 496-503) deliberately serializes only `initial_ratchet`;
    /// `From<InboundGroupSessionPickle> for InboundGroupSession` then
    /// resets `latest_ratchet = initial_ratchet.clone()` on restore. The
    /// "skip-ahead" / latest-ratchet cache is a perf optimization (lets
    /// repeated in-order decrypts skip re-ratcheting from index 0), not a
    /// security property. It is also not observable from the public API,
    /// so any test claiming to assert cache preservation would be
    /// unprovable. We assert only the invariant that is both real and
    /// observable: round-tripped sessions still decrypt subsequent
    /// messages under the same session id.
    #[test]
    fn inbound_pickle_round_trip_decrypts_subsequent_messages() {
        let key = PickleKey::from_bytes([0x77; 32]);
        let mut alice = MegolmOutbound::new();
        let mut bob = MegolmInbound::from_session_key_base64(&alice.session_key_base64()).unwrap();

        let m0 = alice.encrypt(b"m0");
        let m1 = alice.encrypt(b"m1");
        bob.decrypt(&m0).expect("m0");
        bob.decrypt(&m1).expect("m1");

        let original_session_id = bob.session_id();
        let pickle = bob.pickle(&key);
        drop(bob);
        let mut bob_restored = MegolmInbound::from_pickle(&pickle, &key).expect("restore");
        assert_eq!(bob_restored.session_id(), original_session_id);

        let m2 = alice.encrypt(b"m2");
        let d2 = bob_restored.decrypt(&m2).expect("m2 after restore");
        assert_eq!(d2.plaintext, b"m2");
        assert_eq!(d2.message_index, 2);
    }

    /// Test 7a — outbound pickled under key A, unpickled with key B,
    /// returns `MegolmError::Unpickle` (not a panic, not a generic
    /// error). The typed-error contract is what callers will branch on
    /// when surfacing pickle failures to the auth layer.
    #[test]
    fn outbound_pickle_wrong_key_fails_typed() {
        let key_a = PickleKey::from_bytes([0xAA; 32]);
        let key_b = PickleKey::from_bytes([0xBB; 32]);
        let alice = MegolmOutbound::new();
        let pickle = alice.pickle(&key_a);

        let err = MegolmOutbound::from_pickle(&pickle, &key_b).expect_err("wrong key must fail");
        assert!(
            matches!(err, MegolmError::Unpickle { .. }),
            "expected MegolmError::Unpickle, got {err:?}"
        );
    }

    /// Test 7b — inbound pickled under key A, unpickled with key B,
    /// returns `MegolmError::Unpickle`. Same contract as 7a but on the
    /// recipient side.
    #[test]
    fn inbound_pickle_wrong_key_fails_typed() {
        let key_a = PickleKey::from_bytes([0xAA; 32]);
        let key_b = PickleKey::from_bytes([0xBB; 32]);
        let alice = MegolmOutbound::new();
        let bob = MegolmInbound::from_session_key_base64(&alice.session_key_base64()).unwrap();
        let pickle = bob.pickle(&key_a);

        let err = MegolmInbound::from_pickle(&pickle, &key_b).expect_err("wrong key must fail");
        assert!(
            matches!(err, MegolmError::Unpickle { .. }),
            "expected MegolmError::Unpickle, got {err:?}"
        );
    }

    /// Test 8 — a failed verify under out-of-order decrypt does not corrupt
    /// the skip-ahead cache.
    ///
    /// Megolm-specific contract (not the Olm pattern): `decrypt` here is
    /// verify-then-decrypt against a chain key deterministically derived
    /// from the initial ratchet, so the only mutable side-effect on
    /// success is populating the skip-ahead cache. The receiving-chain
    /// "did we consume MK?" concern from Olm doesn't apply.
    ///
    /// Setup: Alice encrypts m0, m1, m2. Bob decrypts m2 *first* —
    /// vodozemac advances `latest_ratchet` to index 2 and populates the
    /// cache for indices 0..=2. Bob then attempts a corrupted copy of m1
    /// (we flip the last byte of the on-wire encoding; per the message
    /// layout that's a signature byte, but for the assertion shape what
    /// matters is that `verify` rejects it). The cache for index 1 must
    /// still be intact, so a subsequent untampered m1 decrypts cleanly.
    #[test]
    fn corrupted_decrypt_does_not_corrupt_skip_ahead_cache() {
        let mut alice = MegolmOutbound::new();
        let mut bob = MegolmInbound::from_session_key_base64(&alice.session_key_base64()).unwrap();

        let _m0 = alice.encrypt(b"m0");
        let m1 = alice.encrypt(b"m1");
        let m2 = alice.encrypt(b"m2");

        // Decrypt m2 first — this populates Bob's skip-ahead cache for
        // indices 0..=2.
        let d2 = bob.decrypt(&m2).expect("m2 first");
        assert_eq!(d2.plaintext, b"m2");

        // Build a corrupted copy of m1: base64-decode, flip the last
        // byte, base64-re-encode, same `message_index`. The MegolmMessage
        // on-wire layout is V|payload|MAC|signature, so the last byte is
        // the signature; vodozemac rejects with `DecryptionError::Signature`,
        // which our wrapper surfaces as `MegolmError::Decrypt`. The
        // discriminating assertion below is *not* the error variant —
        // it's that the cached key for index 1 is still usable.
        let mut raw = B64.decode(&m1.ciphertext).expect("base64");
        *raw.last_mut().expect("ciphertext non-empty") ^= 0x01;
        let m1_corrupt = MegolmCiphertext {
            message_index: m1.message_index,
            ciphertext: B64.encode(&raw),
        };

        let err = bob
            .decrypt(&m1_corrupt)
            .expect_err("corrupted m1 must be rejected");
        assert!(
            matches!(err, MegolmError::Decrypt { .. }),
            "expected MegolmError::Decrypt, got {err:?}"
        );

        // The Megolm-distinctive invariant: the failed verify did not
        // corrupt the cached chain key for index 1.
        let d1 = bob
            .decrypt(&m1)
            .expect("untampered m1 must still decrypt after failed verify");
        assert_eq!(d1.plaintext, b"m1");
        assert_eq!(d1.message_index, 1);
    }

    /// Test 9 — `export_at_base64` + `from_exported_base64` is forward-only.
    /// Alice encrypts m0, m1, m2. Bob decrypts m0 (advancing his
    /// `latest_ratchet`), then exports at index 1. A fresh inbound built
    /// via `from_exported_base64` must *fail* on m0 (it's behind the
    /// imported initial index) and *succeed* on m1 + m2 (at or after it).
    /// This is the catch-up contract: the export forgets earlier message
    /// keys by design, so a compromised exporter cannot let the importer
    /// read history prior to the export point.
    #[test]
    fn export_at_then_import_only_decrypts_from_that_index_forward() {
        let mut alice = MegolmOutbound::new();
        let mut bob = MegolmInbound::from_session_key_base64(&alice.session_key_base64()).unwrap();

        let m0 = alice.encrypt(b"m0");
        let m1 = alice.encrypt(b"m1");
        let m2 = alice.encrypt(b"m2");

        // Bob has decrypted m0, so his `latest_ratchet` is at index 1.
        // Exporting at index 1 yields a key that covers indices 1 and on.
        let d0 = bob.decrypt(&m0).expect("m0");
        assert_eq!(d0.message_index, 0);

        let exported = bob.export_at_base64(1).expect("export at 1");
        let mut catchup =
            MegolmInbound::from_exported_base64(&exported).expect("import valid export");
        assert_eq!(catchup.session_id(), bob.session_id());

        let err = catchup
            .decrypt(&m0)
            .expect_err("imported session must not decrypt before its initial index");
        assert!(
            matches!(err, MegolmError::Decrypt { .. }),
            "expected MegolmError::Decrypt, got {err:?}"
        );

        let d1 = catchup.decrypt(&m1).expect("m1 at the initial index");
        assert_eq!(d1.plaintext, b"m1");
        assert_eq!(d1.message_index, 1);
        let d2 = catchup.decrypt(&m2).expect("m2 after initial index");
        assert_eq!(d2.plaintext, b"m2");
        assert_eq!(d2.message_index, 2);
    }

    /// Test 10 — an inbound built from session X's key cannot decrypt
    /// ciphertext produced by session Y. The session_id mismatch isn't
    /// just metadata: each `MegolmMessage` carries an Ed25519 signature
    /// under the *sender's* per-session keypair, and the inbound's
    /// `signing_key` is fixed at bootstrap from the bootstrap key — so
    /// vodozemac rejects with a signature error (surfaced here as
    /// `MegolmError::Decrypt`) before any chain-key derivation is even
    /// attempted.
    #[test]
    fn wrong_session_key_inbound_cannot_decrypt() {
        let mut session_x = MegolmOutbound::new();
        let session_y = MegolmOutbound::new();
        assert_ne!(
            session_x.session_id(),
            session_y.session_id(),
            "fresh sessions must have distinct ids"
        );

        // Instantiate inbound from Y's key, attempt to decrypt X's ciphertext.
        let mut inbound_for_y =
            MegolmInbound::from_session_key_base64(&session_y.session_key_base64()).unwrap();
        let from_x = session_x.encrypt(b"from-x");

        let err = inbound_for_y
            .decrypt(&from_x)
            .expect_err("inbound for Y must reject ciphertext from X");
        assert!(
            matches!(err, MegolmError::Decrypt { .. }),
            "expected MegolmError::Decrypt, got {err:?}"
        );
    }
}
