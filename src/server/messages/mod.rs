//! `POST /channels/{cid}/messages` and `GET /channels/{cid}/messages`,
//! plus per-message edit/delete/restore + the typing ping.
//!
//! Wave-3 split of the original `server/messages.rs` into focused submodules.
//! Channel-scoped, server-trusted (plaintext) messages with the proven
//! `(sent_at, id)` composite-cursor pagination. The author comes from the
//! session ([`AuthAccount`]); the "speaking-as" persona is PER-CHANNEL — the
//! client sends the persona it's wearing in this channel and the server snapshots
//! it after validating the caller may use it (`can_edit_persona`), falling back to
//! the stored `channel_active_persona` row, else the account. `body` is stored
//! verbatim (it may contain [`crate::markup`] formatting, rendered client-side).
//!
//! ## Privacy 404s
//! Unknown channel and caller-not-a-member-of-the-channel's-guild both
//! surface as `404 "channel not found"` — membership stays non-leaky.
//!
//! ## Composite cursor (SurrealDB 3.1.0-beta.3)
//! Carried over verbatim from the retired room messages: bind `$since`
//! through `type::datetime(...)` (a plain string compares lexically and
//! re-delivers the boundary row), project `sent_at` RAW (never `<string>`
//! cast — that lex-mis-orders at sub-second format boundaries; see
//! `server::datetime`), and `ORDER BY` the projected aliases.
//!
//! ## Layout
//! - [`posting`] — POST + persist + attachment-existence check.
//! - [`reading`] — GET + composite-cursor + MSG_PROJECTION + mime batch +
//!   typing-name resolution.
//! - [`editing`] — PATCH/DELETE/restore/trash + the own-message gate.
//! - [`typing`] — POST /typing ping (in-memory).
//! - this module: shared `channel_access` (the per-channel layer atop
//!   [`crate::server::access::resolve_membership`]) + the per-message body
//!   constants.

mod editing;
mod posting;
mod reading;
mod typing;

// Route-table handlers keep their `crate::server::messages::<fn>` paths via
// these re-exports.
pub use self::editing::{delete_message, edit_message, list_deleted_messages, restore_message};
pub use self::posting::post_message;
pub use self::reading::{list_messages, ListMessagesQuery};
pub use self::typing::typing_ping;

use surrealdb::types::SurrealValue;

use crate::server::access::{resolve_membership, Membership};
use crate::server::state::AppState;

/// Max characters in a message body (markup included).
pub(super) const MAX_BODY_CHARS: usize = 50_000;

/// Max inline image attachments per message.
pub(super) const MAX_ATTACHMENTS: usize = 10;

// ---------------------------------------------------------------------------
// Shared: channel access (membership gate + kind + active persona)
// ---------------------------------------------------------------------------

pub(super) struct ChannelCtx {
    pub kind: String,
    pub active_persona: Option<String>,
}

pub(super) enum AccessOutcome {
    Ok(ChannelCtx),
    ChannelNotFound,
    NotMember,
}

/// Resolve a channel to its guild + kind, then check the caller's membership
/// of that guild and read their active persona for it. The two unknowns
/// (no such channel / caller not a member) are distinct internally but the
/// handlers collapse both to a privacy-404.
///
/// The resolve + membership check is the shared [`crate::server::access`] core
/// (with the soft-delete filter on); this layers the per-channel active-persona
/// read on top.
pub(super) async fn channel_access(
    state: &AppState,
    cid: &str,
    account: &str,
) -> surrealdb::Result<AccessOutcome> {
    #[derive(SurrealValue)]
    struct PersonaRow {
        persona_id: String,
    }

    let kind = match resolve_membership(state, cid, account, true).await? {
        Membership::Member { kind } => kind,
        Membership::ChannelNotFound => return Ok(AccessOutcome::ChannelNotFound),
        Membership::NotMember => return Ok(AccessOutcome::NotMember),
    };

    // Membership gate is per-guild (handled by the core). The worn persona,
    // however, is per-CHANNEL (channel_active_persona): no row → speak as the
    // account.
    let mut resp = state
        .db
        .query(
            "SELECT meta::id(persona) AS persona_id
                FROM channel_active_persona
                WHERE channel = type::record('channel', $cid)
                  AND account = type::record('account', $account);",
        )
        .bind(("cid", cid.to_string()))
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    let active_persona = resp.take::<Option<PersonaRow>>(0)?.map(|r| r.persona_id);

    Ok(AccessOutcome::Ok(ChannelCtx {
        kind,
        active_persona,
    }))
}
