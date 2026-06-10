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
//! - [`reading`] — GET + composite-cursor + MSG_PROJECTION (attachment mimes
//!   join the projection) + typing-name resolution.
//! - [`editing`] — PATCH/DELETE/restore/trash + the own-message gate.
//! - [`typing`] — POST /typing ping (in-memory).
//! - this module: shared `channel_access` (the per-channel layer atop
//!   [`crate::server::access::resolve_membership`]) + the per-message body
//!   constants.

mod editing;
mod posting;
mod read_state;
mod reading;
mod typing;
mod unread;

// Route-table handlers keep their `crate::server::messages::<fn>` paths via
// these re-exports.
pub use self::editing::{delete_message, edit_message, list_deleted_messages, restore_message};
pub use self::posting::post_message;
pub use self::read_state::{mark_read, read_state};
pub use self::reading::{list_messages, ListMessagesQuery};
pub use self::typing::typing_ping;
pub use self::unread::unread;

use surrealdb::types::SurrealValue;

use crate::server::state::AppState;

/// Max characters in a message body (markup included).
pub(super) const MAX_BODY_CHARS: usize = 50_000;

/// Max inline image attachments per message.
pub(super) const MAX_ATTACHMENTS: usize = 100;

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

/// Resolve a channel to its guild + kind, check the caller's guild membership,
/// AND read their per-channel active persona — all in ONE SurrealDB round-trip.
///
/// Splits the three resolutions into a single multi-statement query and folds
/// them into a returned object. Previously this was three sequential round-trips
/// (channel → guild_member → channel_active_persona) on every message poll;
/// W5/H1 collapsed it to one.
///
/// The two unknowns (no such channel / caller not a member) remain distinct
/// internally but the handlers collapse both to a privacy-404. Soft-delete
/// filter is always on here (the messages contract); the shared
/// [`crate::server::access::resolve_membership`] core stays available for the
/// `lorebook` no-filter and `personas` bool-only callers.
pub(super) async fn channel_access(
    state: &AppState,
    cid: &str,
    account: &str,
) -> surrealdb::Result<AccessOutcome> {
    #[derive(SurrealValue)]
    struct CombinedRow {
        /// `kind` of the (live, non-soft-deleted) channel, or `NONE` when the
        /// channel doesn't exist (or it / its guild are soft-deleted).
        chan_kind: Option<String>,
        /// `Some(true)` when the caller is a guild_member; `None` otherwise
        /// (sub-SELECT returns no row). The `IF $chan = NONE` guard skips the
        /// `type::record('guild', NONE)` construction when the channel is
        /// missing — order of evaluation across an `AND` is not contractual
        /// in SurrealQL.
        is_member: Option<bool>,
        /// The caller's worn persona id IN THIS CHANNEL (no row → speak as
        /// the account).
        active_persona: Option<String>,
    }

    // One round-trip, two SurrealQL statements:
    //   stmt 0  LET $chan       (channel + soft-delete gate)
    //   stmt 1  RETURN { ... }  (member check + persona read folded in)
    // Only the RETURN materializes a result row, but the surrealdb driver
    // still indexes through the LET — hence `.take(1)` for the object.
    let sql = "
        LET $chan = (
            SELECT meta::id(guild) AS guild_key, kind
            FROM ONLY type::record('channel', $cid)
            WHERE deleted_at = NONE AND guild.deleted_at = NONE
        );
        RETURN {
            chan_kind: $chan.kind,
            is_member: IF $chan = NONE THEN NONE ELSE
                (SELECT VALUE true FROM ONLY guild_member
                    WHERE guild = type::record('guild', $chan.guild_key)
                      AND account = type::record('account', $account))
            END,
            active_persona: (
                SELECT VALUE meta::id(persona)
                FROM ONLY channel_active_persona
                WHERE channel = type::record('channel', $cid)
                  AND account = type::record('account', $account)
            ),
        };
    ";
    let mut resp = state
        .db
        .query(sql)
        .bind(("cid", cid.to_string()))
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    let row: Option<CombinedRow> = resp.take(1)?;
    let row = row.expect("RETURN always materializes an object");

    match (row.chan_kind, row.is_member) {
        (None, _) => Ok(AccessOutcome::ChannelNotFound),
        (Some(_), None) => Ok(AccessOutcome::NotMember),
        (Some(kind), Some(_)) => Ok(AccessOutcome::Ok(ChannelCtx {
            kind,
            active_persona: row.active_persona,
        })),
    }
}
