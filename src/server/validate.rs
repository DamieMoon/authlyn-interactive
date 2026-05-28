//! Shared input-validation helpers for the server handlers.
//!
//! Two name validators live here, and they deliberately do **not** share an
//! implementation:
//!
//! - [`validate_name`] (guilds, personas) bounds length by **character
//!   count** (`name.chars().count()`), so multi-byte names are measured in
//!   user-visible characters.
//! - [`validate_emoji_name`] bounds length by **byte length** (`name.len()`)
//!   and additionally restricts the character class to `[a-z0-9_]`. Because
//!   the allowed bytes are all single-byte ASCII, byte-length and char-count
//!   coincide there — but the two functions are kept distinct so neither
//!   rule silently changes the other's bounds. Do not unify them.

/// Maximum length, in characters, of a guild/channel/persona display name.
const MAX_NAME_CHARS: usize = 100;

/// Validate a guild/channel/persona display name: non-empty, at most
/// [`MAX_NAME_CHARS`] **characters** (Unicode scalar count, not bytes).
pub(crate) fn validate_name(name: &str) -> Result<(), &'static str> {
    let n = name.chars().count();
    if n == 0 {
        return Err("name must not be empty");
    }
    if n > MAX_NAME_CHARS {
        return Err("name too long");
    }
    Ok(())
}

/// `^[a-z0-9_]{2,32}$` validated in Rust (no regex dependency needed).
///
/// Length is measured in **bytes** (`name.len()`); since the allowed
/// character class is single-byte ASCII this equals the character count for
/// any accepted name. Kept separate from [`validate_name`] on purpose — see
/// the module docs.
pub(crate) fn validate_emoji_name(name: &str) -> Result<(), &'static str> {
    let n = name.len();
    if n < 2 {
        return Err("emoji name must be at least 2 characters");
    }
    if n > 32 {
        return Err("emoji name must be at most 32 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err("emoji name must match [a-z0-9_]");
    }
    Ok(())
}
