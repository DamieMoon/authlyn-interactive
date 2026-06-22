//! Auth-side crypto/token primitives + shared input validators.
//!
//! Split from `server/auth.rs` in Wave 3 of the systems-audit; behavior
//! preserved verbatim. Kept here because `registration`, `password`, and
//! `session` all depend on the same hashing/verification + length-rule pair.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::http::StatusCode;
use axum::response::Response;

use crate::server::errors::error_response;

/// Username length bounds, counted in CHARACTERS (not bytes) — the
/// inclusive 3..=32 range enforced by [`validate_credentials`].
pub(super) const MIN_USERNAME_CHARS: usize = 3;
pub(super) const MAX_USERNAME_CHARS: usize = 32;
/// Minimum password length, counted in CHARACTERS to match the user-facing
/// "at least 8 characters" message and the char-based username rule below. A
/// byte count would let a sub-8-character multibyte password (e.g. three lock
/// emoji = 3 chars but 12 bytes) slip past the gate (review F-D5-2).
pub(super) const MIN_PASSWORD_CHARS: usize = 8;
/// Maximum password length, counted in BYTES: this is a DoS / argon2-input
/// bound, and bytes is the correct unit for capping the work fed to the hasher.
pub(super) const MAX_PASSWORD_BYTES: usize = 4096;

/// A fresh opaque session token: 32 bytes of CSPRNG entropy, hex-encoded.
/// Handed to the browser once (in the cookie); only its SHA-256 is stored.
pub(super) fn random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 of `input`, hex-encoded. The one-way transform applied to a session
/// token before it touches the DB, so a leaked `session` row can't be replayed
/// as a cookie.
pub(super) fn sha256_hex(input: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input);
    hex::encode(hasher.finalize())
}

/// argon2id hash on the blocking pool (it's tens of ms of CPU). Maps task /
/// hashing failures to a 500 response so callers can `?`-style early-return.
pub(super) async fn hash_on_blocking_pool(password: String) -> Result<String, Response> {
    match tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
    })
    .await
    {
        Ok(Ok(hash)) => Ok(hash),
        Ok(Err(e)) => {
            tracing::error!(error = %e, "argon2 hashing failed");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "hashing failed",
            ))
        }
        Err(e) => {
            tracing::error!(error = %e, "hash task join failed");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "hashing failed",
            ))
        }
    }
}

/// Verify `password` against a stored argon2 PHC string on the blocking pool
/// (CPU-bound). An **unparseable** PHC verifies as `false`, not an error — so a
/// non-credential sentinel hash (the `nova_dot` seed's `password_hash = '!'`,
/// see `storage/schema.surql`) makes that account login-impossible via a clean
/// verify-fail rather than a 500. Only a task-join failure maps to a 500.
pub(super) async fn verify_on_blocking_pool(
    password: String,
    phc: String,
) -> Result<bool, Response> {
    match tokio::task::spawn_blocking(move || match PasswordHash::new(&phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    })
    .await
    {
        Ok(verified) => Ok(verified),
        Err(e) => {
            tracing::error!(error = %e, "verify task join failed");
            Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "verification failed",
            ))
        }
    }
}

/// Validate a register payload: username length (`MIN_USERNAME_CHARS..=
/// MAX_USERNAME_CHARS`, char-counted) and whitespace-free, then defers to
/// [`validate_password`]. Returns the user-facing 400 message on the first
/// failure.
pub(super) fn validate_credentials(username: &str, password: &str) -> Result<(), &'static str> {
    let n = username.chars().count();
    if !(MIN_USERNAME_CHARS..=MAX_USERNAME_CHARS).contains(&n) {
        return Err("username must be 3-32 characters");
    }
    if username.chars().any(char::is_whitespace) {
        return Err("username must not contain whitespace");
    }
    validate_password(password)
}

/// The password length rule shared by register and change-password.
pub(super) fn validate_password(password: &str) -> Result<(), &'static str> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err("password must be at least 8 characters");
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err("password too long");
    }
    Ok(())
}
