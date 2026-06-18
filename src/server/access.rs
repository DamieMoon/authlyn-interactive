//! Shared channel-membership and visibility resolution.
//!
//! Three handlers gate on "is this caller a member of the guild that owns this
//! channel, and what kind of channel is it?" — `messages` (needs the channel
//! kind + the caller's per-channel active persona), `personas` (just a bool),
//! and `lorebook` (also asserts `kind == "lorebook"`). They previously each
//! re-implemented the resolve + membership check with subtly different SQL.
//! [`visible_channels`] answers the account-wide form of the same question
//! (every live text channel the caller may see), shared by `GET /events`
//! (filtering) and `GET /unread`.
//!
//! [`resolve_membership`] is the common core: resolve channel → (guild, kind),
//! then check `guild_member`. Each caller layers its own specifics on top —
//! none of their public contracts (return type, status codes, error bodies)
//! change.
//!
//! ## The one behavioral knob: `filter_deleted`
//! `messages` and `personas` resolve the channel **only if neither it nor its
//! guild is soft-deleted** (`deleted_at = NONE AND guild.deleted_at = NONE`);
//! `lorebook` historically resolved the channel with **no** soft-delete filter.
//! That difference is preserved verbatim via the `filter_deleted` flag — do not
//! collapse it without a deliberate behavior decision.

use surrealdb::types::SurrealValue;

use crate::server::state::AppState;

/// Outcome of resolving a channel and checking the caller's guild membership.
/// `ChannelNotFound` and `NotMember` are kept distinct for callers that want
/// them (none currently do — every call site collapses both to a privacy-404 /
/// `false`), mirroring the original `messages::AccessOutcome` split.
pub(crate) enum Membership {
    /// Caller is a member; the channel's `kind` is carried out.
    Member { kind: String },
    /// No such (live, per `filter_deleted`) channel.
    ChannelNotFound,
    /// Channel exists but the caller is not a member of its guild.
    NotMember,
}

/// Resolve `cid` → its guild + `kind`, then check whether `account` is a member
/// of that guild.
///
/// When `filter_deleted` is true the channel resolves only if neither it nor
/// its guild is soft-deleted; when false the channel resolves regardless of
/// soft-delete state (the `lorebook` contract).
pub(crate) async fn resolve_membership(
    state: &AppState,
    cid: &str,
    account: &str,
    filter_deleted: bool,
) -> surrealdb::Result<Membership> {
    #[derive(SurrealValue)]
    struct ChanRow {
        // M7/P1: NONE for a DM thread (a channel with no guild).
        guild_key: Option<String>,
        kind: String,
    }
    #[derive(SurrealValue)]
    struct MemRow {
        member: bool,
    }

    // The soft-delete filter is the only varying fragment; both branches are
    // static SQL (no user input interpolated). M7/P1: `guild = NONE OR …` keeps
    // guild-less DM threads live (a DM is never "guild-soft-deleted").
    // `meta::id(guild)` ERRORS on a DM row (guild = NONE), so it's guarded — NONE
    // guild_key for a DM thread, the id for a guild channel.
    let chan_sql = if filter_deleted {
        "SELECT (IF guild != NONE THEN meta::id(guild) ELSE NONE END) AS guild_key, kind
            FROM type::record('channel', $cid)
            WHERE deleted_at = NONE AND (guild = NONE OR guild.deleted_at = NONE);"
    } else {
        "SELECT (IF guild != NONE THEN meta::id(guild) ELSE NONE END) AS guild_key, kind
            FROM type::record('channel', $cid);"
    };

    let mut resp = state
        .db
        .query(chan_sql)
        .bind(("cid", cid.to_string()))
        .await?
        .check()?;
    let Some(chan) = resp.take::<Option<ChanRow>>(0)? else {
        return Ok(Membership::ChannelNotFound);
    };

    // M7/P1: membership lives in a different table per channel kind. A DM thread
    // (`kind='dm'`, no guild) is gated by `dm_member`; a guild text/lorebook
    // channel by `guild_member`. The three-outcome contract is identical, so the
    // privacy-404 callers don't change.
    let is_member = if chan.kind == "dm" {
        let mut resp = state
            .db
            .query(
                "SELECT true AS member
                    FROM dm_member
                    WHERE channel = type::record('channel', $cid)
                      AND account = type::record('account', $account);",
            )
            .bind(("cid", cid.to_string()))
            .bind(("account", account.to_string()))
            .await?
            .check()?;
        resp.take::<Option<MemRow>>(0)?.is_some()
    } else {
        // A guild channel always has a guild; defend against a malformed row.
        let Some(gid) = chan.guild_key else {
            return Ok(Membership::NotMember);
        };
        let mut resp = state
            .db
            .query(
                "SELECT true AS member
                    FROM guild_member
                    WHERE guild = type::record('guild', $gid)
                      AND account = type::record('account', $account);",
            )
            .bind(("gid", gid))
            .bind(("account", account.to_string()))
            .await?
            .check()?;
        resp.take::<Option<MemRow>>(0)?.is_some()
    };
    if !is_member {
        return Ok(Membership::NotMember);
    }

    Ok(Membership::Member { kind: chan.kind })
}

/// One channel the account may currently see: a live `kind='text'` channel in a
/// guild where they are a member, or (M7/P1) a live `kind='dm'` thread where they
/// are a `dm_member` (`guild_id` then `None`). Shared by GET /events (filtering)
/// and GET /unread (Task 8, aggregation).
#[derive(SurrealValue)]
pub(crate) struct VisibleChannel {
    pub(crate) channel_id: String,
    // M7/P1: NONE for a DM thread (a channel with no guild).
    pub(crate) guild_id: Option<String>,
}

/// Load every [`VisibleChannel`] for `account`. Two parameterized statements,
/// one round-trip. The account-only `guild_member` lookup is served by the
/// `guild_member_account` index (review M-37 — the `(guild, account)` UNIQUE
/// composite can't serve a non-prefix predicate, so this planned as a
/// TableScan on every /events connect, ListsChanged visibility reload, and
/// GET /unread).
pub(crate) async fn visible_channels(
    state: &AppState,
    account: &str,
) -> surrealdb::Result<Vec<VisibleChannel>> {
    let mut resp = state
        .db
        .query(
            "LET $gids = (SELECT VALUE guild FROM guild_member
                 WHERE account = type::record('account', $account));
             LET $dms = (SELECT VALUE channel FROM dm_member
                 WHERE account = type::record('account', $account));
             SELECT meta::id(id) AS channel_id,
                    (IF guild != NONE THEN meta::id(guild) ELSE NONE END) AS guild_id
                 FROM channel
                 WHERE deleted_at = NONE
                   AND ( (kind = 'text' AND guild IN $gids AND guild.deleted_at = NONE)
                         OR (kind = 'dm' AND id IN $dms) );",
        )
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    // Statements 0 and 1 are the LETs (no materialized rows); the SELECT is
    // take(2). DM rows project `guild_id = NONE` (no guild) → Option::None.
    resp.take(2)
}
