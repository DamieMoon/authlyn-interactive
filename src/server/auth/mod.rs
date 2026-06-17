//! Username/password accounts + server-side sessions.
//!
//! Wave-3 split of the original `server/auth.rs` into focused submodules.
//! The trust model is a classic server-side session: `POST /auth/register`
//! and `/auth/login` mint a random opaque token, store only its SHA-256 in
//! the `session` table, and hand the token to the browser in an
//! `HttpOnly; Secure; SameSite=Lax` cookie. Every protected handler takes
//! the [`AuthAccount`] extractor, which resolves that cookie back to an
//! account id (or rejects with 401). Passwords are argon2id-hashed;
//! hashing/verification run on the blocking pool so they don't stall the
//! async runtime.
//!
//! ## Stance
//! - Defensive at the boundary: JSON-shape errors → typed 400; unknown
//!   user and wrong password return the **same** 401 body (no enumeration).
//! - The `account_username_ci UNIQUE` index is the source of truth for
//!   "username taken"; a racing duplicate register surfaces as
//!   `is_unique_violation` → 409, same body as the pre-check would give.
//!
//! ## Layout
//! - [`session`] — `AuthAccount` extractor, token issue/resolve/revoke,
//!   session cookie.
//! - [`registration`] — register/login/logout/me handlers.
//! - [`password`] — change-password, security question, public reset flow.
//! - [`admin`] — admin-only password reset.
//! - [`crypto`] — argon2id hashing, sha256, random token, input validators.

mod admin;
mod crypto;
mod password;
mod registration;
mod session;

// `AuthAccount` is imported by ~9 modules as `crate::server::auth::AuthAccount`
// — re-export so the public path stays stable after the split.
pub use self::session::AuthAccount;

// Session-validity primitives for the long-lived `GET /events` stream
// (`server::events`), which re-derives identity for the LIFETIME of its
// connection (review M-05): the cookie name, the token→stored-hash transform,
// and the hash-keyed lookup. Hoisted here (wave-2 follow-up of M-05) so
// events.rs consumes the auth module's own definitions instead of owning
// drift-prone mirror copies.
pub(crate) use self::session::{account_for_token_hash, session_token_hash, SESSION_COOKIE};

// Route handlers referenced by `server/mod.rs::small_body_routes` keep
// their `crate::server::auth::<fn>` paths via these re-exports.
pub use self::admin::admin_reset_password;
pub use self::password::{
    change_password, confirm_password_reset, get_reset_question, set_security_question,
};
pub use self::registration::{login, logout, me, patch_account, register};
