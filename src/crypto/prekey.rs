//! Pre-key bundles: the public material a device publishes so peers can
//! bootstrap an Olm session with it.
//!
//! The wire shape lives in [`PreKeyBundle`]; both ssr and hydrate need it
//! (server validates inbound bundles, client serializes outbound ones), so
//! this module is target-agnostic — no axum, no surrealdb, no tokio.
//!
//! ## Signing rule
//!
//! Each OTK and the fallback key carries an Ed25519 signature over the
//! canonical string `"<kid>:<public_key_hex>"`. The signing key is the
//! device's identity Ed25519 key. The server verifies before storing.
//! Anyone (peer or server) holding the device's `identity_ed25519` can
//! later reverify by recomputing the same canonical string.
//!
//! ## Stance
//!
//! - **Adversarial** when verifying: malicious clients could submit
//!   mismatched (kid, pk, sig) triples, duplicates, wrong-length material.
//!   [`SignedPreKey::verify_against`] is the chokepoint.
//! - **Defensive** at the parse boundary: all hex decoding + length
//!   checks happen here, before any DB row is touched.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use vodozemac::{
    Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature, KeyError, SignatureError,
};

use crate::crypto::identity::DeviceAccount;

/// One signed OTK or fallback key, in the shape published to the server.
///
/// `kid` is the vodozemac [`KeyId`](vodozemac::KeyId) encoded as the library's
/// base64 string (via `String::from(KeyId)`). The server treats it as
/// opaque — there's no path back to a `KeyId` because the vodozemac
/// constructor is private — so we round-trip it as a string only.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedPreKey {
    /// Opaque key identifier from vodozemac. Used by the server only as a
    /// lookup string and by the recipient to disambiguate which OTK was
    /// claimed.
    pub kid: String,
    /// 32-byte Curve25519 public key, hex-encoded (64 hex chars).
    pub public_key: String,
    /// 64-byte Ed25519 signature, hex-encoded (128 hex chars). Signs the
    /// canonical string `"<kid>:<public_key_hex>"`.
    pub signature: String,
}

/// The full publish payload for a device.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreKeyBundle {
    /// 32-byte Curve25519 identity key, hex-encoded.
    pub identity_curve25519: String,
    /// 32-byte Ed25519 identity key, hex-encoded.
    pub identity_ed25519: String,
    /// Signed one-time keys. The server stores the full set and consumes
    /// them one at a time on `/keys/claim`.
    pub one_time_keys: Vec<SignedPreKey>,
    /// Long-lived fallback key, used by the server when the OTK pool runs
    /// dry. Replaced wholesale on each publish.
    pub fallback_key: SignedPreKey,
}

/// Errors raised while building, parsing, or verifying a [`PreKeyBundle`].
#[derive(Debug, Error)]
pub enum PreKeyError {
    /// A hex field could not be decoded (e.g. odd-length string, non-hex
    /// characters).
    #[error("invalid hex in field {field}: {source}")]
    InvalidHex {
        field: &'static str,
        #[source]
        source: hex::FromHexError,
    },
    /// A field's decoded length is not what the wire format requires
    /// (Curve/Ed25519 keys = 32 bytes, signatures = 64 bytes).
    #[error("invalid length for field {field}: expected {expected}, got {got}")]
    InvalidLength {
        field: &'static str,
        expected: usize,
        got: usize,
    },
    /// Vodozemac refused to parse the bytes as the named key type.
    #[error("invalid key in field {field}: {source}")]
    InvalidKey {
        field: &'static str,
        #[source]
        source: KeyError,
    },
    /// Vodozemac refused to parse the bytes as a signature.
    #[error("invalid signature encoding in field {field}: {source}")]
    InvalidSignature {
        field: &'static str,
        #[source]
        source: SignatureError,
    },
    /// The signature didn't verify against the device's identity key.
    #[error("signature verification failed for kid {kid}: {source}")]
    SignatureVerifyFailed {
        kid: String,
        #[source]
        source: SignatureError,
    },
    /// Two OTKs in the bundle share the same `kid`. Each OTK has to be
    /// addressable independently.
    #[error("duplicate kid in one_time_keys: {0}")]
    DuplicateKid(String),
}

impl SignedPreKey {
    /// Build the canonical message that the signature covers.
    /// Keep this in one place so callers and verifiers can't drift.
    pub fn canonical_message(kid: &str, public_key_hex: &str) -> String {
        format!("{kid}:{public_key_hex}")
    }

    /// Validate (a) hex shape + length of the public key, (b) hex shape +
    /// length of the signature, (c) signature parses, (d) signature
    /// verifies under `identity`.
    ///
    /// Returns `Ok(())` if every check passes. The caller is responsible
    /// for funneling the error to the right HTTP status code.
    ///
    /// We don't separately "parse" the Curve25519 public key —
    /// `Curve25519PublicKey::from_bytes` accepts every 32-byte value
    /// (X25519 clamps internally), so a length check on the hex string is
    /// the only meaningful shape gate. The Ed25519 signature verification
    /// transitively confirms the device signed this specific point.
    pub fn verify_against(&self, identity: &Ed25519PublicKey) -> Result<(), PreKeyError> {
        let _ = decode_hex_exact(&self.public_key, 32, "public_key")?;

        let sig_bytes = decode_hex_exact(&self.signature, 64, "signature")?;
        let sig = Ed25519Signature::from_slice(&sig_bytes).map_err(|e| {
            PreKeyError::InvalidSignature {
                field: "signature",
                source: e,
            }
        })?;

        let msg = Self::canonical_message(&self.kid, &self.public_key);
        identity
            .verify(msg.as_bytes(), &sig)
            .map_err(|e| PreKeyError::SignatureVerifyFailed {
                kid: self.kid.clone(),
                source: e,
            })
    }
}

/// Helper: decode a hex string and assert its byte length.
fn decode_hex_exact(s: &str, expected: usize, field: &'static str) -> Result<Vec<u8>, PreKeyError> {
    let bytes = hex::decode(s).map_err(|e| PreKeyError::InvalidHex { field, source: e })?;
    if bytes.len() != expected {
        return Err(PreKeyError::InvalidLength {
            field,
            expected,
            got: bytes.len(),
        });
    }
    Ok(bytes)
}

impl PreKeyBundle {
    /// Decode the identity Ed25519 key out of the bundle. Used by the
    /// server before walking through every OTK to verify signatures.
    pub fn identity_ed25519(&self) -> Result<Ed25519PublicKey, PreKeyError> {
        let bytes = decode_hex_exact(&self.identity_ed25519, 32, "identity_ed25519")?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .expect("length checked by decode_hex_exact");
        Ed25519PublicKey::from_slice(&arr).map_err(|e| PreKeyError::InvalidKey {
            field: "identity_ed25519",
            source: e,
        })
    }

    /// Walk every field of the bundle and confirm:
    ///   - identity keys decode
    ///   - every OTK signature verifies under `identity_ed25519`
    ///   - the fallback key signature verifies
    ///   - OTK kids are unique
    ///
    /// On `Ok(())` the bundle is safe to persist. This is the boundary
    /// where defensive parsing ends and offensive in-process assumptions
    /// begin.
    pub fn verify_self(&self) -> Result<(), PreKeyError> {
        // Identity keys (curve25519 just for shape; signatures only need ed25519).
        let _ = decode_hex_exact(&self.identity_curve25519, 32, "identity_curve25519")?;
        let identity = self.identity_ed25519()?;

        // OTK pool: verify each + check uniqueness.
        let mut seen = std::collections::HashSet::with_capacity(self.one_time_keys.len());
        for otk in &self.one_time_keys {
            otk.verify_against(&identity)?;
            if !seen.insert(otk.kid.clone()) {
                return Err(PreKeyError::DuplicateKid(otk.kid.clone()));
            }
        }

        // Fallback.
        self.fallback_key.verify_against(&identity)?;
        Ok(())
    }
}

/// Builder that turns a freshly-keyed [`DeviceAccount`] into a publishable
/// [`PreKeyBundle`].
///
/// Available on both ssr (used by tests + the eventual server-side builder
/// helpers) and hydrate (clients will publish their own bundles from the
/// browser). Every dependency in here — `vodozemac` types, `DeviceAccount`,
/// `hex` — already compiles cleanly for wasm32, so the gating isn't load-
/// bearing.
pub struct PreKeyBundleBuilder;

impl Default for PreKeyBundleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PreKeyBundleBuilder {
    pub fn new() -> Self {
        Self
    }

    /// Mint `otk_count` fresh OTKs and one fallback key on `device`, sign
    /// each, then mark them as published.
    ///
    /// Marking as published lets the local account drop them from the
    /// "unpublished" set; the server is now authoritative over which OTKs
    /// are live.
    pub fn build(self, device: &mut DeviceAccount, otk_count: usize) -> PreKeyBundle {
        // Generate fresh public/private OTK pairs on the device.
        device.generate_one_time_keys(otk_count);
        // Force a fresh fallback key as well.
        device.generate_fallback_key();

        let identity = device.identity_keys();
        let identity_ed25519 = hex::encode(identity.ed25519.as_bytes());
        let identity_curve25519 = hex::encode(identity.curve25519.as_bytes());

        let sign_one = |kid: String, pk: &Curve25519PublicKey| -> SignedPreKey {
            let public_key_hex = hex::encode(pk.as_bytes());
            let msg = SignedPreKey::canonical_message(&kid, &public_key_hex);
            let sig = device.sign(msg.as_bytes());
            SignedPreKey {
                kid,
                public_key: public_key_hex,
                signature: hex::encode(sig.to_bytes()),
            }
        };

        let one_time_keys: Vec<SignedPreKey> = device
            .unpublished_one_time_keys()
            .into_iter()
            .map(|(kid, pk)| sign_one(String::from(kid), &pk))
            .collect();

        let (fb_kid, fb_pk) = device
            .unpublished_fallback_key()
            .expect("generate_fallback_key was just called, so a fallback exists");
        let fallback_key = sign_one(String::from(fb_kid), &fb_pk);

        // Mark everything as published so future builds don't re-emit the
        // same key material.
        device.mark_keys_as_published();

        PreKeyBundle {
            identity_curve25519,
            identity_ed25519,
            one_time_keys,
            fallback_key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_bundle_verifies() {
        let mut device = DeviceAccount::new();
        let bundle = PreKeyBundleBuilder::new().build(&mut device, 4);
        bundle
            .verify_self()
            .expect("freshly built bundle must verify");
        assert_eq!(bundle.one_time_keys.len(), 4);
        // Sanity: identity hex is 64 chars (32 bytes hex-encoded).
        assert_eq!(bundle.identity_curve25519.len(), 64);
        assert_eq!(bundle.identity_ed25519.len(), 64);
    }

    #[test]
    fn flipping_a_signature_byte_fails_verification() {
        let mut device = DeviceAccount::new();
        let mut bundle = PreKeyBundleBuilder::new().build(&mut device, 1);
        let mut sig = hex::decode(&bundle.one_time_keys[0].signature).unwrap();
        sig[0] ^= 0x01;
        bundle.one_time_keys[0].signature = hex::encode(sig);
        let err = bundle.verify_self().expect_err("must reject corrupted sig");
        assert!(matches!(err, PreKeyError::SignatureVerifyFailed { .. }));
    }

    #[test]
    fn duplicate_kid_rejected() {
        let mut device = DeviceAccount::new();
        let mut bundle = PreKeyBundleBuilder::new().build(&mut device, 2);
        // Replace OTK #1 with a verbatim copy of OTK #0 so the signature
        // still verifies — the duplicate is what we want to flag.
        bundle.one_time_keys[1] = bundle.one_time_keys[0].clone();
        let err = bundle
            .verify_self()
            .expect_err("duplicate kid must be rejected");
        assert!(
            matches!(err, PreKeyError::DuplicateKid(_)),
            "expected DuplicateKid, got {err:?}"
        );
    }

    #[test]
    fn canonical_message_format_is_stable() {
        // Anchor the canonical-string format so signers and verifiers can't drift.
        assert_eq!(
            SignedPreKey::canonical_message("AAAAAAAAAAA", "00".repeat(32).as_str()),
            format!("AAAAAAAAAAA:{}", "00".repeat(32))
        );
    }

    #[test]
    fn invalid_hex_in_public_key_is_typed_error() {
        // The identity Ed25519 key is irrelevant here — verification
        // bails out on `decode_hex_exact` before ever touching it. Any
        // syntactically valid Ed25519 pubkey works; we use the canonical
        // basepoint here so the test runs without spinning up a vodozemac
        // Account (which would force `#[cfg(feature = "ssr")]`).
        //
        // Ed25519 basepoint encoded as a compressed point.
        const BASEPOINT_BYTES: [u8; 32] = [
            0x58, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66,
            0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66,
            0x66, 0x66, 0x66, 0x66,
        ];
        let id = Ed25519PublicKey::from_slice(&BASEPOINT_BYTES)
            .expect("Ed25519 basepoint is a valid public key");
        let sp = SignedPreKey {
            kid: "k".into(),
            public_key: "zzzzz".into(),
            signature: "00".repeat(64),
        };
        let err = sp.verify_against(&id).expect_err("must error");
        assert!(matches!(err, PreKeyError::InvalidHex { .. }));
    }
}
