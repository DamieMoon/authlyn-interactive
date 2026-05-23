//! Encrypted attachments — Matrix `m.encrypted` v2 wire format.
//!
//! Per-blob random AES-256-CTR key + IV, with SHA-256 over the ciphertext
//! for tamper-detection. The key (as a JWK), IV, and hash are bundled into
//! an [`EncryptedFileRef`] and carried inside the Megolm-encrypted message
//! body — the server only ever sees the ciphertext.
//!
//! ## Layering
//!
//! - [`encrypt`] — mints `(key, iv)` randomly, encrypts, returns the
//!   [`EncryptedAttachment`] the sender uses to build the upload + ref.
//! - [`decrypt`] — verifies the SHA-256 *before* applying the keystream,
//!   so a tampered ciphertext is rejected without spending CPU on AES.
//! - [`KeyJwk`] — JWK shape used by [`EncryptedFileRef::key`]. Strict
//!   `alg`/`kty` validation on decode (adversarial-stance default).
//! - [`EncryptedFileRef`] — the wire shape that rides inside the
//!   Megolm-encrypted message body. Server never sees it.
//!
//! ## Stance
//!
//! - **Adversarial.** Default for crypto. The hash check fails closed
//!   (typed error, no keystream applied) and runs before any cipher
//!   operation.
//! - **Defensive at the parse boundary.** JWK fields, base64 inputs, and
//!   length checks happen at the decode/decrypt entry points so the AES
//!   wrappers never see malformed material.
//!
//! ## Counter mode choice
//!
//! Matches libolm / Matrix: `Ctr64BE<Aes256>` — 64-bit big-endian counter
//! in the lower 64 bits of the IV, upper 64 bits as a per-blob nonce. The
//! IV minted by [`encrypt`] keeps its lower 64 bits zeroed so the counter
//! can run for any practical file size before it would touch the nonce.
//!
//! ## What this module does NOT authenticate
//!
//! AES-CTR is unauthenticated. A wrong key just produces garbage plaintext
//! — there is no MAC and `decrypt` cannot tell you the key was wrong. The
//! authentication comes from *outside* this module: the `EncryptedFileRef`
//! ships inside a Megolm-authenticated message body, and the SHA-256
//! check inside [`decrypt`] confirms the ciphertext bytes weren't
//! tampered with in transit. That's the same threat model Matrix
//! `m.encrypted` v2 ships with.

use aes::Aes256;
use base64::engine::general_purpose::{STANDARD_NO_PAD as B64, URL_SAFE_NO_PAD as B64URL};
use base64::Engine;
use ctr::cipher::{KeyIvInit, StreamCipher};
use ctr::Ctr64BE;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

// Wire-format note: `iv` and `hashes.sha256` are unpadded base64
// (`STANDARD_NO_PAD`) per Matrix MSC1420 + matrix-js-sdk's
// `EncryptedFile` interface ("encoded as unpadded base64"). `KeyJwk::k`
// is base64url-no-pad per the JWK spec. Mixing engines is deliberate:
// changing either to padded base64 silently breaks wire compat with
// every other Matrix client.

/// AES-256-CTR cipher with a 64-bit big-endian counter — Matrix /
/// libolm-compatible variant.
type AesCtr = Ctr64BE<Aes256>;

const KEY_LEN: usize = 32;
const IV_LEN: usize = 16;
const SHA256_LEN: usize = 32;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum AttachmentError {
    /// JWK `alg` field was not `"A256CTR"`.
    #[error("JWK alg must be \"A256CTR\"")]
    InvalidJwkAlg,
    /// JWK `kty` field was not `"oct"`.
    #[error("JWK kty must be \"oct\"")]
    InvalidJwkKty,
    /// JWK `k` field did not decode as base64url.
    #[error("JWK k is not valid base64url")]
    InvalidJwkKeyBase64,
    /// JWK `k` decoded to the wrong length (must be 32 bytes for AES-256).
    #[error("JWK k must decode to 32 bytes, got {0}")]
    InvalidJwkKeyLength(usize),
    /// `iv` field did not decode as base64.
    #[error("iv is not valid base64")]
    InvalidIvBase64,
    /// `iv` decoded to the wrong length (must be 16 bytes for AES).
    #[error("iv must decode to 16 bytes, got {0}")]
    InvalidIvLength(usize),
    /// `hashes.sha256` did not decode as base64.
    #[error("sha256 is not valid base64")]
    InvalidHashBase64,
    /// `hashes.sha256` decoded to the wrong length (must be 32 bytes).
    #[error("sha256 must decode to 32 bytes, got {0}")]
    InvalidHashLength(usize),
    /// The SHA-256 of the ciphertext did not match the expected value.
    /// Surfaces as 4xx-equivalent to the caller; never silently
    /// continues to decrypt.
    #[error("sha256 mismatch: ciphertext was tampered with or truncated")]
    HashMismatch,
    /// JWK `ext` field was not `true`. Matrix v2 requires `ext: true`;
    /// rejected at decode time so a peer sending `ext: false` is caught
    /// at the typed-error boundary instead of being silently accepted.
    #[error("JWK ext must be true")]
    JwkExtMustBeTrue,
    /// JWK `key_ops` did not include both `"encrypt"` and `"decrypt"`.
    /// Matrix v2 requires `key_ops` to "at least contain" both
    /// operations; an inbound JWK with `["sign"]`, `["encrypt"]` alone,
    /// or `[]` is rejected.
    #[error("JWK key_ops must contain both 'encrypt' and 'decrypt'")]
    JwkMissingKeyOp,
}

// ---------------------------------------------------------------------------
// JWK
// ---------------------------------------------------------------------------

/// JWK encoding of the per-attachment AES-256 key, matching the
/// `m.encrypted` v2 `file.key` field.
///
/// All four MUST-fields per Matrix v2 are validated strictly on decode:
/// `alg = "A256CTR"`, `kty = "oct"`, `ext = true`, and `key_ops` must
/// contain both `"encrypt"` and `"decrypt"` (additional ops, e.g.
/// `"wrapKey"`, are allowed). A non-conforming inbound JWK is rejected
/// at the typed-error boundary rather than fed to AES against the
/// caller's intent.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyJwk {
    pub alg: String,
    pub ext: bool,
    pub k: String,
    pub key_ops: Vec<String>,
    pub kty: String,
}

impl KeyJwk {
    /// The fixed `alg` value for AES-256-CTR per JWA + Matrix.
    pub const ALG: &'static str = "A256CTR";
    /// The fixed `kty` value for a symmetric key per JWK.
    pub const KTY: &'static str = "oct";
    /// The two `key_ops` values Matrix v2 requires the JWK to advertise.
    const REQUIRED_KEY_OPS: [&'static str; 2] = ["encrypt", "decrypt"];

    fn new_for_key(key: &[u8; KEY_LEN]) -> Self {
        Self {
            alg: Self::ALG.to_string(),
            ext: true,
            k: B64URL.encode(key),
            key_ops: Self::REQUIRED_KEY_OPS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            kty: Self::KTY.to_string(),
        }
    }

    fn decode_key(&self) -> Result<[u8; KEY_LEN], AttachmentError> {
        if self.alg != Self::ALG {
            return Err(AttachmentError::InvalidJwkAlg);
        }
        if self.kty != Self::KTY {
            return Err(AttachmentError::InvalidJwkKty);
        }
        if !self.ext {
            return Err(AttachmentError::JwkExtMustBeTrue);
        }
        for required in Self::REQUIRED_KEY_OPS {
            if !self.key_ops.iter().any(|op| op == required) {
                return Err(AttachmentError::JwkMissingKeyOp);
            }
        }
        let bytes = B64URL
            .decode(self.k.as_bytes())
            .map_err(|_| AttachmentError::InvalidJwkKeyBase64)?;
        let len = bytes.len();
        bytes
            .try_into()
            .map_err(|_| AttachmentError::InvalidJwkKeyLength(len))
    }
}

// ---------------------------------------------------------------------------
// Output of `encrypt`
// ---------------------------------------------------------------------------

/// What [`encrypt`] returns: the ciphertext that goes up to `/media`, plus
/// the key + iv + hash the sender bundles into an [`EncryptedFileRef`].
///
/// `ciphertext` is plain bytes (not base64) — it's what gets POSTed to the
/// server. `iv` and `sha256` are base64-standard-padded; `key` is a
/// [`KeyJwk`] whose `k` field is base64url-no-pad per JWK convention.
#[derive(Clone, Debug)]
pub struct EncryptedAttachment {
    pub ciphertext: Vec<u8>,
    pub key: KeyJwk,
    pub iv: String,
    pub sha256: String,
}

// ---------------------------------------------------------------------------
// Wire shape carried inside Megolm-encrypted message bodies
// ---------------------------------------------------------------------------

/// Matrix `m.encrypted` v2 file reference. Carried inside the
/// Megolm-encrypted message body — the **server never sees it**. The
/// recipient deserializes this after decrypting the Megolm message, then
/// fetches `url` and decrypts with the bundled key/iv/hash.
///
/// Lives in `crypto::attachment` (not `protocol`) because it never
/// crosses the HTTP wire — it's a client-internal type intrinsic to the
/// attachment crypto layer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptedFileRef {
    /// The Matrix `m.encrypted` version. Must be `"v2"`.
    pub v: String,
    /// Server-relative URL of the ciphertext blob, e.g. `/media/<id>`.
    pub url: String,
    pub key: KeyJwk,
    pub iv: String,
    pub hashes: Hashes,
    pub mimetype: String,
}

/// `{ sha256: "<base64>" }` — Matrix's `hashes` object. Kept as a struct
/// (rather than `BTreeMap<String, String>`) because v1 only ever has the
/// one entry and the typed field is what callers will pattern-match
/// against.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hashes {
    pub sha256: String,
}

// ---------------------------------------------------------------------------
// encrypt + decrypt
// ---------------------------------------------------------------------------

/// Encrypt a plaintext blob under a fresh random AES-256-CTR key + IV.
///
/// The IV's lower 64 bits are zeroed so the 64-bit counter can run for
/// any practical file size without touching the nonce half (see module
/// doc, "Counter mode choice"). The returned [`EncryptedAttachment`]
/// holds the ciphertext (to POST), the JWK-encoded key, the base64 IV,
/// and the base64 SHA-256 of the ciphertext.
pub fn encrypt(plaintext: &[u8]) -> EncryptedAttachment {
    let mut key = [0u8; KEY_LEN];
    let mut iv = [0u8; IV_LEN];
    let mut rng = rand::thread_rng();
    rng.fill_bytes(&mut key);
    // Matrix m.encrypted v2: lower 64 bits of the IV must be zero so the
    // 64-bit counter has full headroom. Only fill the upper 64 bits.
    rng.fill_bytes(&mut iv[..8]);

    let mut ciphertext = plaintext.to_vec();
    let mut cipher = AesCtr::new(&key.into(), &iv.into());
    cipher.apply_keystream(&mut ciphertext);

    let sha = Sha256::digest(&ciphertext);

    EncryptedAttachment {
        ciphertext,
        key: KeyJwk::new_for_key(&key),
        iv: B64.encode(iv),
        sha256: B64.encode(sha),
    }
}

/// Decrypt an attachment ciphertext under the supplied key + iv after
/// confirming its SHA-256 matches `expected_sha256_b64`.
///
/// **Hash-check-first.** The SHA-256 verification runs before any
/// keystream application; on mismatch, returns [`AttachmentError::HashMismatch`]
/// without spending CPU on AES. A wrong *key* (or wrong IV) silently
/// produces garbage plaintext — AES-CTR is unauthenticated and this
/// function cannot tell you the key was wrong. The Megolm message body
/// that the JWK rode in on is what authenticates the key+iv+hash bundle;
/// that authentication is the caller's responsibility, not this module's.
pub fn decrypt(
    ciphertext: &[u8],
    key: &KeyJwk,
    iv_b64: &str,
    expected_sha256_b64: &str,
) -> Result<Vec<u8>, AttachmentError> {
    // Hash check first — fail closed before any cipher work.
    let expected = B64
        .decode(expected_sha256_b64.as_bytes())
        .map_err(|_| AttachmentError::InvalidHashBase64)?;
    if expected.len() != SHA256_LEN {
        return Err(AttachmentError::InvalidHashLength(expected.len()));
    }
    let actual = Sha256::digest(ciphertext);
    if actual.as_slice() != expected.as_slice() {
        return Err(AttachmentError::HashMismatch);
    }

    let key_bytes = key.decode_key()?;
    let iv_bytes = B64
        .decode(iv_b64.as_bytes())
        .map_err(|_| AttachmentError::InvalidIvBase64)?;
    let iv_len = iv_bytes.len();
    let iv_arr: [u8; IV_LEN] = iv_bytes
        .try_into()
        .map_err(|_| AttachmentError::InvalidIvLength(iv_len))?;

    let mut plaintext = ciphertext.to_vec();
    let mut cipher = AesCtr::new(&key_bytes.into(), &iv_arr.into());
    cipher.apply_keystream(&mut plaintext);
    Ok(plaintext)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use rand::Rng;

    /// Round-trip a tiny plaintext to lock down the happy-path shape.
    /// Asserts ciphertext is the same length as plaintext (CTR doesn't pad),
    /// the JWK shape is well-formed, and decrypt restores the original.
    #[test]
    fn small_round_trip() {
        let plaintext = b"the quick brown fox jumps over the lazy dog";
        let att = encrypt(plaintext);

        assert_eq!(
            att.ciphertext.len(),
            plaintext.len(),
            "CTR keeps ciphertext length equal to plaintext length"
        );
        assert_eq!(att.key.alg, KeyJwk::ALG);
        assert_eq!(att.key.kty, KeyJwk::KTY);
        assert!(att.key.ext);
        assert_eq!(att.key.key_ops, vec!["encrypt", "decrypt"]);

        let recovered =
            decrypt(&att.ciphertext, &att.key, &att.iv, &att.sha256).expect("decrypt succeeds");
        assert_eq!(recovered, plaintext);
    }

    /// 1 MiB random plaintext, the size the routing-plan acceptance test
    /// calls out. Verifies the CTR keystream + SHA-256 are correct over a
    /// realistic blob, not just toy inputs.
    #[test]
    fn one_mib_random_round_trip() {
        let mut plaintext = vec![0u8; 1024 * 1024];
        rand::thread_rng().fill(&mut plaintext[..]);

        let att = encrypt(&plaintext);
        assert_eq!(att.ciphertext.len(), plaintext.len());
        let recovered =
            decrypt(&att.ciphertext, &att.key, &att.iv, &att.sha256).expect("1 MiB round-trip");
        assert_eq!(recovered, plaintext);
    }

    /// Empty plaintext is a legal input (Matrix doesn't forbid it). Locks
    /// down that the CTR / SHA-256 path doesn't choke on zero bytes.
    #[test]
    fn empty_plaintext_round_trip() {
        let att = encrypt(b"");
        assert!(att.ciphertext.is_empty());
        let recovered = decrypt(&att.ciphertext, &att.key, &att.iv, &att.sha256).expect("decrypt");
        assert!(recovered.is_empty());
    }

    /// Flip one ciphertext byte: SHA-256 check must reject with
    /// [`AttachmentError::HashMismatch`] *before* any keystream applies.
    /// The discriminating signal is the typed error variant — a wrapper
    /// that returned garbage plaintext on mismatch would silently violate
    /// the integrity contract.
    #[test]
    fn tampered_ciphertext_fails_with_hash_mismatch() {
        let plaintext = b"sensitive";
        let att = encrypt(plaintext);
        let mut tampered = att.ciphertext.clone();
        tampered[0] ^= 0x01;

        let err = decrypt(&tampered, &att.key, &att.iv, &att.sha256)
            .expect_err("tampered ciphertext must be rejected");
        assert!(
            matches!(err, AttachmentError::HashMismatch),
            "expected HashMismatch, got {err:?}"
        );
    }

    /// AES-CTR is unauthenticated: a wrong key still "decrypts" to a
    /// non-error plaintext (it'll just be garbage). This test pins that
    /// contract — `decrypt` does NOT raise on wrong key. The authentication
    /// of the key + iv bundle is the caller's responsibility (it rides
    /// inside a Megolm-authenticated message body), per the module doc.
    #[test]
    fn wrong_key_decrypts_to_garbage_not_error() {
        let plaintext = b"hello";
        let att = encrypt(plaintext);

        // Swap in a fresh random key with everything else (iv, ciphertext,
        // sha256) untouched. The sha256 check still passes (we didn't
        // tamper with the ciphertext); the AES path just runs against the
        // wrong keystream.
        let mut wrong = [0u8; KEY_LEN];
        rand::thread_rng().fill_bytes(&mut wrong);
        let wrong_jwk = KeyJwk::new_for_key(&wrong);

        let result = decrypt(&att.ciphertext, &wrong_jwk, &att.iv, &att.sha256);
        let bytes = result.expect("AES-CTR cannot detect wrong key");
        assert_eq!(bytes.len(), plaintext.len());
        assert_ne!(
            bytes, plaintext,
            "with overwhelming probability, wrong key produces different bytes"
        );
    }

    /// JWK `alg` validation is strict — a key that says it's `"A128GCM"`
    /// must be rejected with the typed `InvalidJwkAlg`, not silently
    /// accepted as if it were AES-256-CTR.
    #[test]
    fn jwk_wrong_alg_rejected() {
        let plaintext = b"x";
        let att = encrypt(plaintext);
        let mut bad = att.key.clone();
        bad.alg = "A128GCM".to_string();

        let err = decrypt(&att.ciphertext, &bad, &att.iv, &att.sha256)
            .expect_err("wrong alg must reject");
        assert!(
            matches!(err, AttachmentError::InvalidJwkAlg),
            "expected InvalidJwkAlg, got {err:?}"
        );
    }

    /// JWK `kty` validation is strict — same shape as `alg`.
    #[test]
    fn jwk_wrong_kty_rejected() {
        let plaintext = b"x";
        let att = encrypt(plaintext);
        let mut bad = att.key.clone();
        bad.kty = "RSA".to_string();

        let err = decrypt(&att.ciphertext, &bad, &att.iv, &att.sha256)
            .expect_err("wrong kty must reject");
        assert!(
            matches!(err, AttachmentError::InvalidJwkKty),
            "expected InvalidJwkKty, got {err:?}"
        );
    }

    /// IV must decode to exactly 16 bytes. Anything else is a typed
    /// `InvalidIvLength` — we never feed a wrong-length IV into the AES
    /// init (which would panic in some block-cipher crates).
    #[test]
    fn iv_wrong_length_rejected() {
        let plaintext = b"x";
        let att = encrypt(plaintext);
        let short_iv = B64.encode([0u8; 8]); // only 8 bytes, not 16

        let err = decrypt(&att.ciphertext, &att.key, &short_iv, &att.sha256)
            .expect_err("short IV must reject");
        assert!(
            matches!(err, AttachmentError::InvalidIvLength(8)),
            "expected InvalidIvLength(8), got {err:?}"
        );
    }

    /// Two consecutive `encrypt` calls produce distinct keys + IVs with
    /// overwhelming probability. Pins the contract that the per-blob
    /// randomness is actually random, not a constant from a misuse of
    /// the RNG.
    #[test]
    fn fresh_per_blob_randomness() {
        let a = encrypt(b"same plaintext");
        let b = encrypt(b"same plaintext");
        assert_ne!(a.key.k, b.key.k, "keys must differ between encrypts");
        assert_ne!(a.iv, b.iv, "IVs must differ between encrypts");
        // Same plaintext + different key/iv => different ciphertext.
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    /// JWK serializes to the exact field set Matrix expects (no extras,
    /// no missing fields). Locks down the wire shape so the receiver
    /// in another implementation can parse it.
    #[test]
    fn jwk_serializes_to_expected_field_set() {
        let key = KeyJwk::new_for_key(&[0u8; KEY_LEN]);
        let v = serde_json::to_value(&key).expect("serialize");
        let obj = v.as_object().expect("JWK is a JSON object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, vec!["alg", "ext", "k", "key_ops", "kty"]);
        assert_eq!(obj["alg"], "A256CTR");
        assert_eq!(obj["kty"], "oct");
        assert_eq!(obj["ext"], true);
    }

    /// JWK `k`, `iv`, and `sha256` are all **unpadded** base64. A regression
    /// that flipped any of them to padded base64 would break wire compat
    /// with every other Matrix v2 client. Discriminating signal: absence
    /// of `=` in the emitted strings (32-byte key + 16-byte IV + 32-byte
    /// hash are all sizes where padded base64 would include `=`).
    #[test]
    fn wire_base64_is_unpadded() {
        let att = encrypt(b"x");
        assert!(
            !att.key.k.contains('='),
            "JWK k must be base64url-no-pad, got {}",
            att.key.k
        );
        assert!(
            !att.iv.contains('='),
            "iv must be unpadded base64, got {}",
            att.iv
        );
        assert!(
            !att.sha256.contains('='),
            "sha256 must be unpadded base64, got {}",
            att.sha256
        );
    }

    /// JWK with `ext: false` is rejected at decode time per Matrix v2
    /// MUST ("ext: Must be true."). Discriminating signal is the typed
    /// `JwkExtMustBeTrue` variant — a wrapper that silently accepted
    /// would let a peer downgrade the WebCrypto export hint and would
    /// diverge from spec.
    #[test]
    fn jwk_ext_false_rejected() {
        let plaintext = b"x";
        let att = encrypt(plaintext);
        let mut bad = att.key.clone();
        bad.ext = false;

        let err = decrypt(&att.ciphertext, &bad, &att.iv, &att.sha256)
            .expect_err("ext: false must reject");
        assert!(
            matches!(err, AttachmentError::JwkExtMustBeTrue),
            "expected JwkExtMustBeTrue, got {err:?}"
        );
    }

    /// JWK with `key_ops` missing either `encrypt` or `decrypt` is
    /// rejected. Matrix v2 wording: "Must at least contain `encrypt`
    /// and `decrypt`." Tested both axes (missing one, missing the
    /// other, empty) to lock down that the check is conjunctive.
    #[test]
    fn jwk_key_ops_must_contain_both() {
        let plaintext = b"x";
        let att = encrypt(plaintext);

        for bad_ops in [
            vec![],
            vec!["encrypt".to_string()],
            vec!["decrypt".to_string()],
            vec!["sign".to_string()],
        ] {
            let mut bad = att.key.clone();
            bad.key_ops = bad_ops.clone();
            let err = decrypt(&att.ciphertext, &bad, &att.iv, &att.sha256)
                .expect_err(&format!("key_ops {bad_ops:?} must reject"));
            assert!(
                matches!(err, AttachmentError::JwkMissingKeyOp),
                "expected JwkMissingKeyOp for {bad_ops:?}, got {err:?}"
            );
        }

        // Extra ops beyond the required two are allowed (forward-compat).
        let mut extra = att.key.clone();
        extra.key_ops = vec![
            "encrypt".to_string(),
            "decrypt".to_string(),
            "wrapKey".to_string(),
        ];
        let _ok =
            decrypt(&att.ciphertext, &extra, &att.iv, &att.sha256).expect("extra key_ops allowed");
    }

    /// `EncryptedFileRef` round-trips through JSON unchanged. This is the
    /// contract the Megolm-encrypted message body relies on — the
    /// recipient deserializes one of these after decrypting the body.
    #[test]
    fn encrypted_file_ref_json_round_trip() {
        let att = encrypt(b"payload");
        let r = EncryptedFileRef {
            v: "v2".to_string(),
            url: "/media/abc123".to_string(),
            key: att.key,
            iv: att.iv,
            hashes: Hashes { sha256: att.sha256 },
            mimetype: "image/png".to_string(),
        };
        let json = serde_json::to_string(&r).expect("serialize");
        let r2: EncryptedFileRef = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(r, r2);
    }
}
