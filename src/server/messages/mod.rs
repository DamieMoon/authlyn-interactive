//! `POST /channels/{cid}/messages` and `GET /channels/{cid}/messages`,
//! plus per-message edit/delete/restore + the typing ping.
//!
//! Wave-3 split of the original `server/messages.rs` into focused submodules.
//! Channel-scoped, server-trusted (plaintext) messages with the proven
//! `(sent_at, id)` composite-cursor pagination. The author comes from the
//! session (`AuthAccount`); the "speaking-as" persona is PER-CHANNEL — the
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
//! - `posting` — POST + persist + attachment-existence check.
//! - `reading` — GET + composite-cursor + MSG_PROJECTION (attachment mimes
//!   join the projection) + typing-name resolution.
//! - `editing` — PATCH/DELETE/restore/trash + the own-message gate (and the
//!   roll-immutability 403s).
//! - `rolling` — POST /roll (M4/T6 Fate Engine: server-authoritative dice).
//! - `typing` — POST /typing ping + GET /typing-drafts (M4/T7 Ghost Quill;
//!   both in-memory).
//! - this module: shared `channel_access` (the per-channel layer atop
//!   `crate::server::access::resolve_membership`) + the per-message body
//!   constants.

mod editing;
mod posting;
mod read_state;
mod reading;
mod rolling;
mod typing;
mod unread;

// Route-table handlers keep their `crate::server::messages::<fn>` paths via
// these re-exports.
pub use self::editing::{delete_message, edit_message, list_deleted_messages, restore_message};
pub use self::posting::post_message;
pub use self::read_state::{mark_read, read_state};
pub use self::reading::{list_messages, ListMessagesQuery};
pub use self::rolling::{roll_message, ORACLE_ANSWERS};
pub use self::typing::{typing_drafts, typing_ping};
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
    /// M7/P1 (review M2): true when the channel is read-only (a 1:1 DM whose
    /// friends unfriended). `post_message` rejects writes; reads are unaffected.
    pub locked: bool,
    /// M7/P2: true when the caller reaches this channel as a GUEST (an active
    /// `channel_guest`) and is NOT a `guild_member` — so `post_message` snapshots
    /// `message.guest_cameo = true` without a second round-trip. Always false for a
    /// DM or for a real guild member (a member who is also somehow a guest is
    /// treated as a member).
    pub via_guest: bool,
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
/// M5/H1 collapsed it to one.
///
/// The two unknowns (no such channel / caller not a member) remain distinct
/// internally but the handlers collapse both to a privacy-404. Soft-delete
/// filter is always on here (the messages contract); the shared
/// `crate::server::access::resolve_membership` core stays available for the
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
        /// `Some(true)` when the caller is a member — `guild_member` for a guild
        /// channel, `dm_member` for a `kind='dm'` thread (M7/P1), or an active
        /// `channel_guest` for a guild channel (M7/P2 Guest Cameos) — `None`
        /// otherwise (sub-SELECT returns no row). The `IF $chan = NONE` guard
        /// skips the membership lookup when the channel is missing — order of
        /// evaluation across an `AND` is not contractual in SurrealQL.
        is_member: Option<bool>,
        /// The caller's worn persona id IN THIS CHANNEL (no row → speak as
        /// the account).
        active_persona: Option<String>,
        /// M7/P1 (review M2): true when the channel carries a `locked_at` stamp
        /// (a read-only 1:1 DM). False for a missing channel or any live channel.
        locked: bool,
        /// M7/P2: true when the caller is an active guest (channel_guest) and NOT
        /// a guild_member of this channel's guild → drives the send-time badge.
        via_guest: bool,
    }

    // One round-trip, four SurrealQL statements:
    //   stmt 0  LET $chan   (channel + soft-delete gate)
    //   stmt 1  LET $gm     (guild_member presence, false for a DM/missing chan)
    //   stmt 2  LET $cg     (active channel_guest presence — M7/P2)
    //   stmt 3  RETURN { ... }  (member check + via_guest + persona read folded in)
    // Only the RETURN materializes a result row, but the surrealdb driver
    // still indexes through each LET — hence `.take(3)` for the object.
    let sql = "
        LET $chan = (
            SELECT (IF guild != NONE THEN meta::id(guild) ELSE NONE END) AS guild_key, kind, locked_at
            FROM ONLY type::record('channel', $cid)
            WHERE deleted_at = NONE AND (guild = NONE OR guild.deleted_at = NONE)
        );
        LET $gm = (IF $chan = NONE OR $chan.kind = 'dm' THEN false ELSE
            ((SELECT VALUE true FROM ONLY guild_member
                WHERE guild = type::record('guild', $chan.guild_key)
                  AND account = type::record('account', $account)) == true) END);
        LET $cg = (IF $chan = NONE OR $chan.kind = 'dm' THEN false ELSE
            ((SELECT VALUE true FROM ONLY channel_guest
                WHERE channel = type::record('channel', $cid)
                  AND account = type::record('account', $account)
                  AND (expires_at = NONE OR expires_at > time::now())) == true) END);
        RETURN {
            chan_kind: $chan.kind,
            locked: (IF $chan = NONE THEN false ELSE $chan.locked_at != NONE END),
            is_member: IF $chan = NONE THEN NONE ELSE
                (IF $chan.kind = 'dm' THEN
                    (SELECT VALUE true FROM ONLY dm_member
                        WHERE channel = type::record('channel', $cid)
                          AND account = type::record('account', $account))
                ELSE
                    (IF ($gm OR $cg) THEN true ELSE NONE END)
                END)
            END,
            via_guest: ($cg AND !$gm),
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
    let row: Option<CombinedRow> = resp.take(3)?;
    let row = row.expect("RETURN always materializes an object");

    match (row.chan_kind, row.is_member) {
        (None, _) => Ok(AccessOutcome::ChannelNotFound),
        (Some(_), None) => Ok(AccessOutcome::NotMember),
        (Some(kind), Some(_)) => Ok(AccessOutcome::Ok(ChannelCtx {
            kind,
            active_persona: row.active_persona,
            locked: row.locked,
            via_guest: row.via_guest,
        })),
    }
}
