//! Small avatar helpers shared across the UI.
//!
//! The 4 in-app monogram fallback sites (rail-guild title in shell/mod.rs:rail,
//! chat-avatar in shell/channel/avatar.rs, member-avatar in shell/members.rs,
//! persona-portrait in shell/wardrobe.rs) all compute the same upper-case first
//! character of a name, with a per-site fallback when the name is empty. They
//! each wrap it in different markup (the rail uses a bare `{text}` inside a
//! button, members uses an `<span class="member-avatar member-avatar-mono">`,
//! wardrobe a bare `{text}` inside a `.detail-portrait`/`.card-portrait` slot,
//! channel the inline-styled `.chat-avatar` span) — the structural surrounds
//! stay site-local; only the monogram computation lifts.

/// Upper-case first character of `name`, falling back to `fallback` when the
/// name is empty (or, paranoid, starts with a char that has no upper-case
/// form). Returns a `String` because most call sites push it into a view
/// macro that already wants an owned value.
pub fn monogram(name: &str, fallback: char) -> String {
    name.chars()
        .next()
        .unwrap_or(fallback)
        .to_uppercase()
        .to_string()
}
