//! `DeviceClient` — the browser's E2EE state machine.
//!
//! Holds the per-device Olm `DeviceAccount`, one `MegolmOutbound` per room we
//! send into, and the `MegolmInbound` sessions we've imported from peers
//! (keyed by Megolm session id). Every method here is **synchronous** — it
//! only touches crypto + in-memory maps. The async HTTP lives in
//! [`super::api`]; the UI layer interleaves the two (read state → await HTTP →
//! mutate state).
//!
//! This module compiles for **both** `ssr` and `hydrate`: all of `crate::crypto`
//! is target-agnostic, so nothing here is feature-gated. Only the networking
//! ([`super::api`]) and localStorage ([`super::store`]) layers are
//! browser-only.

use std::collections::{HashMap, HashSet};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::crypto::{
    DeviceAccount, MegolmCiphertext, MegolmInbound, MegolmOutbound, OlmEnvelope, OlmSession,
    PickleKey,
};
use crate::protocol::{ClaimKeyResponse, UploadKeysRequest};

/// Number of one-time keys minted per publish. Generous for a single-user
/// smoke; the server caps at `MAX_OTKS_PER_PUBLISH = 200`.
const OTK_COUNT: usize = 50;

/// One device's full client-side crypto state.
pub struct DeviceClient {
    pub user_id: String,
    pub device_id: String,
    pickle_key: PickleKey,
    account: DeviceAccount,
    /// Outbound Megolm session per room we send into.
    outbound: HashMap<String, MegolmOutbound>,
    /// Inbound Megolm sessions we've imported, keyed by Megolm session id.
    inbound: HashMap<String, MegolmInbound>,
    /// Session ids minted by *us* — we can't decrypt our own outbound Megolm
    /// output, so the receive loop skips these.
    own_session_ids: HashSet<String>,
}

impl DeviceClient {
    /// Fresh device: new Olm account + random at-rest pickle key.
    pub fn new(user_id: String, device_id: String) -> Self {
        Self {
            user_id,
            device_id,
            pickle_key: PickleKey::random(),
            account: DeviceAccount::new(),
            outbound: HashMap::new(),
            inbound: HashMap::new(),
            own_session_ids: HashSet::new(),
        }
    }

    /// Mint the OTK pool + fallback and build the publish request for
    /// `POST /keys/upload`.
    pub fn build_bundle_request(&mut self) -> UploadKeysRequest {
        let bundle =
            crate::crypto::prekey::PreKeyBundleBuilder::new().build(&mut self.account, OTK_COUNT);
        UploadKeysRequest {
            user_id: self.user_id.clone(),
            bundle,
        }
    }

    /// Get-or-create our outbound Megolm session for `room`. Returns
    /// `(session_id, session_key_base64)` — the key is what peers need to
    /// instantiate a matching inbound session.
    pub fn ensure_room_session(&mut self, room: &str) -> (String, String) {
        let session = self.outbound.entry(room.to_string()).or_default();
        let session_id = session.session_id();
        self.own_session_ids.insert(session_id.clone());
        (session_id, session.session_key_base64())
    }

    /// Wrap our room session key for a peer: bootstrap a one-shot outbound Olm
    /// session from their claimed bundle and encrypt the key into a PreKey
    /// envelope. The Olm session is discarded — each key-share is a fresh
    /// PreKey message.
    pub fn make_keyshare_envelope(
        &self,
        claim: &ClaimKeyResponse,
        session_key_b64: &str,
    ) -> Result<OlmEnvelope, String> {
        let mut olm =
            OlmSession::outbound_from_claim(&self.account, claim).map_err(|e| e.to_string())?;
        Ok(olm.encrypt(session_key_b64.as_bytes()))
    }

    /// Encrypt `plaintext` for `room`. Returns `(megolm_session_id, ciphertext)`
    /// or `None` if no outbound session exists for the room yet (caller must
    /// share a key first).
    pub fn encrypt_for_room(
        &mut self,
        room: &str,
        plaintext: &[u8],
    ) -> Option<(String, MegolmCiphertext)> {
        let session = self.outbound.get_mut(room)?;
        let session_id = session.session_id();
        let ct = session.encrypt(plaintext);
        Some((session_id, ct))
    }

    /// Accept an inbound key-share envelope: decrypt the Olm PreKey message
    /// (consuming one of our OTKs), recover the peer's Megolm session key, and
    /// install a matching inbound session. Returns the imported session id.
    ///
    /// `sender_identity_curve_hex` is the sender's identity Curve25519 key —
    /// the caller obtains it by claiming the sender's bundle. Vodozemac
    /// cross-checks it against the key inside the PreKey message.
    pub fn import_keyshare(
        &mut self,
        sender_identity_curve_hex: &str,
        envelope: &OlmEnvelope,
    ) -> Result<String, String> {
        let (_olm, plaintext) =
            OlmSession::inbound_from_prekey(&mut self.account, sender_identity_curve_hex, envelope)
                .map_err(|e| e.to_string())?;
        let session_key = String::from_utf8(plaintext)
            .map_err(|e| format!("session key was not valid UTF-8: {e}"))?;
        let inbound =
            MegolmInbound::from_session_key_base64(&session_key).map_err(|e| e.to_string())?;
        let session_id = inbound.session_id();
        self.inbound.insert(session_id.clone(), inbound);
        Ok(session_id)
    }

    /// Decrypt a wire ciphertext routed by `session_id`. `None` if we have no
    /// inbound session for it (key-share not yet imported) or decryption fails.
    pub fn decrypt(&mut self, session_id: &str, wire: &MegolmCiphertext) -> Option<Vec<u8>> {
        let session = self.inbound.get_mut(session_id)?;
        session.decrypt(wire).ok().map(|d| d.plaintext)
    }

    /// True if `session_id` is one we mint outbound (i.e. our own messages).
    pub fn is_own(&self, session_id: &str) -> bool {
        self.own_session_ids.contains(session_id)
    }

    // -- Persistence ---------------------------------------------------------

    /// Serialize the whole client to a `Snapshot` (pickled crypto state) for
    /// localStorage. Returns an error if the account pickle fails.
    pub fn to_snapshot(&self) -> Result<Snapshot, String> {
        let account = self
            .account
            .pickle(&self.pickle_key)
            .map_err(|e| e.to_string())?;
        Ok(Snapshot {
            user_id: self.user_id.clone(),
            device_id: self.device_id.clone(),
            pickle_key_b64: B64.encode(self.pickle_key.as_bytes()),
            account,
            outbound: self
                .outbound
                .iter()
                .map(|(room, s)| (room.clone(), s.pickle(&self.pickle_key)))
                .collect(),
            inbound: self
                .inbound
                .iter()
                .map(|(sid, s)| (sid.clone(), s.pickle(&self.pickle_key)))
                .collect(),
            own_session_ids: self.own_session_ids.iter().cloned().collect(),
        })
    }

    /// Reconstruct a client from a persisted `Snapshot`.
    pub fn from_snapshot(snap: Snapshot) -> Result<Self, String> {
        let key_bytes = B64
            .decode(&snap.pickle_key_b64)
            .map_err(|e| format!("bad pickle key base64: {e}"))?;
        let key_arr: [u8; 32] = key_bytes
            .as_slice()
            .try_into()
            .map_err(|_| "pickle key was not 32 bytes".to_string())?;
        let pickle_key = PickleKey::from_bytes(key_arr);

        let account =
            DeviceAccount::from_pickle(&snap.account, &pickle_key).map_err(|e| e.to_string())?;

        let mut outbound = HashMap::new();
        for (room, p) in &snap.outbound {
            outbound.insert(
                room.clone(),
                MegolmOutbound::from_pickle(p, &pickle_key).map_err(|e| e.to_string())?,
            );
        }
        let mut inbound = HashMap::new();
        for (sid, p) in &snap.inbound {
            inbound.insert(
                sid.clone(),
                MegolmInbound::from_pickle(p, &pickle_key).map_err(|e| e.to_string())?,
            );
        }

        Ok(Self {
            user_id: snap.user_id,
            device_id: snap.device_id,
            pickle_key,
            account,
            outbound,
            inbound,
            own_session_ids: snap.own_session_ids.into_iter().collect(),
        })
    }
}

/// Serializable, at-rest form of a [`DeviceClient`]. Crypto state is pickled
/// (AEAD / libolm) under `pickle_key`; the pickle key itself is stored
/// alongside in base64 — localStorage is the device's trust boundary in the
/// v1 stub (password-derived keys land with the auth follow-up).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub user_id: String,
    pub device_id: String,
    pub pickle_key_b64: String,
    pub account: String,
    pub outbound: Vec<(String, String)>,
    pub inbound: Vec<(String, String)>,
    pub own_session_ids: Vec<String>,
}
