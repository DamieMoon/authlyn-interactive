//! Wave-1 SAFETY-NET: soft-delete + restore + purge cascade characterization
//! (#22; audit 019e6c08, invariant #14). Locks current behavior:
//!
//!   - guild / channel / message soft-delete: the item leaves its normal read
//!     (list / detail / message page), appears in the matching trash listing,
//!     and a restore brings it back into the normal read;
//!   - purge_soft_deleted hard-deletes rows past their rollback window
//!     (message 1h / channel 1d / guild 30d) and CASCADES (a purged guild takes
//!     its channels + members + messages; a purged channel takes its messages).
//!     We backdate `deleted_at` past the window, then invoke the (otherwise
//!     hourly) sweep deterministically.

mod common;

#[cfg(feature = "ssr")]
use authlyn_interactive::server::{purge_soft_deleted, AppState};
#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::{json, Value};
#[cfg(feature = "ssr")]
use surrealdb::types::SurrealValue;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a guild and return `(guild_id, default_text_channel_id)`.
#[cfg(feature = "ssr")]
async fn guild_with_channel(router: &axum::Router, cookie: &str) -> (String, String) {
    let (st, _, g) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(cookie),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = g["id"].as_str().unwrap().to_string();
    let (_, _, d) = common::send(
        router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(cookie),
        None,
    )
    .await;
    let cid = d["channels"][0]["id"].as_str().unwrap().to_string();
    (gid, cid)
}

/// Post a message, returning its id.
#[cfg(feature = "ssr")]
async fn post_message(router: &axum::Router, cookie: &str, cid: &str, body: &str) -> String {
    let (st, _, m) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(cookie),
        Some(&json!({ "body": body })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    m["id"].as_str().unwrap().to_string()
}

/// Ids of the messages currently in the normal (live) page of a channel.
#[cfg(feature = "ssr")]
fn message_ids(list: &Value) -> Vec<String> {
    list["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect()
}

/// Does a table still hold a row with the given record id? (post-purge check)
#[cfg(feature = "ssr")]
async fn row_exists(
    db: &surrealdb::Surreal<surrealdb::engine::remote::ws::Client>,
    table: &str,
    id: &str,
) -> bool {
    let sql = format!("SELECT VALUE meta::id(id) FROM type::record('{table}', $id);");
    let mut resp = db
        .query(sql)
        .bind(("id", id.to_string()))
        .await
        .expect("exists query")
        .check()
        .expect("exists check");
    let rows: Vec<String> = resp.take(0).expect("take");
    !rows.is_empty()
}

// ---------------------------------------------------------------------------
// Guild soft-delete → trash → restore
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn guild_soft_delete_then_restore() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (gid, _cid) = guild_with_channel(&a.router, &owner).await;

    // Present in the normal list.
    let (_, _, list) = common::send(&a.router, Method::GET, "/guilds", Some(&owner), None).await;
    assert_eq!(list["guilds"].as_array().unwrap().len(), 1);

    // Soft-delete (owner) → 204.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Gone from the normal list.
    let (_, _, list) = common::send(&a.router, Method::GET, "/guilds", Some(&owner), None).await;
    assert_eq!(
        list["guilds"].as_array().unwrap().len(),
        0,
        "soft-deleted guild leaves the normal list"
    );

    // Detail now 404s (membership row survives, but load filters deleted_at).
    let (st, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "soft-deleted guild detail → 404");

    // Present in the trash.
    let (st, _, trash) =
        common::send(&a.router, Method::GET, "/guilds/trash", Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    let trashed = trash["guilds"].as_array().unwrap();
    assert_eq!(trashed.len(), 1);
    assert_eq!(trashed[0]["id"], gid);

    // Restore → 204.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/restore"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Back in the normal list, detail 200, gone from trash.
    let (_, _, list) = common::send(&a.router, Method::GET, "/guilds", Some(&owner), None).await;
    assert_eq!(list["guilds"].as_array().unwrap().len(), 1);
    let (st, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "restored guild detail → 200");
    let (_, _, trash) =
        common::send(&a.router, Method::GET, "/guilds/trash", Some(&owner), None).await;
    assert_eq!(trash["guilds"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Channel soft-delete → trash → restore
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn channel_soft_delete_then_restore() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (gid, _default_cid) = guild_with_channel(&a.router, &owner).await;

    // Add a second channel so deleting one still leaves the guild non-empty.
    let (st, _, c) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "extra", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let cid = c["id"].as_str().unwrap().to_string();

    let chan_ids = |detail: &Value| -> Vec<String> {
        detail["channels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_string())
            .collect()
    };

    // Present in the guild detail.
    let (_, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert!(chan_ids(&detail).contains(&cid));

    // Soft-delete the channel (manager) → 204.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}/channels/{cid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Gone from the guild detail.
    let (_, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert!(
        !chan_ids(&detail).contains(&cid),
        "soft-deleted channel leaves the guild detail"
    );

    // Present in the channel trash.
    let (st, _, trash) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}/trash/channels"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let trashed: Vec<String> = trash["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        trashed.contains(&cid),
        "soft-deleted channel is in the trash"
    );

    // Restore → 204, back in the detail, gone from trash.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels/{cid}/restore"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert!(chan_ids(&detail).contains(&cid), "restored channel is back");
    let (_, _, trash) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}/trash/channels"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(trash["channels"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Message soft-delete → trash → restore
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn message_soft_delete_then_restore() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (_gid, cid) = guild_with_channel(&a.router, &owner).await;

    let keep = post_message(&a.router, &owner, &cid, "keep me").await;
    let drop = post_message(&a.router, &owner, &cid, "delete me").await;

    // Both present.
    let (_, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    let ids = message_ids(&list);
    assert!(ids.contains(&keep) && ids.contains(&drop));

    // Soft-delete one → 204.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{drop}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Gone from the normal page; the kept one stays.
    let (_, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    let ids = message_ids(&list);
    assert!(ids.contains(&keep));
    assert!(
        !ids.contains(&drop),
        "soft-deleted message leaves the normal page"
    );

    // Present in the message trash.
    let (st, _, trash) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages/trash"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let trashed = message_ids(&trash);
    assert!(
        trashed.contains(&drop),
        "soft-deleted message is in the trash"
    );
    assert!(!trashed.contains(&keep), "live message is not in the trash");

    // Re-deleting an already-deleted message is idempotent → 204 (audit: the
    // author gate does not filter deleted_at, so re-delete returns 204 not 404).
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{drop}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "re-delete is idempotent (204)");

    // Restore → 204, back in the normal page, gone from trash.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages/{drop}/restore"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert!(
        message_ids(&list).contains(&drop),
        "restored message is back"
    );
    let (_, _, trash) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages/trash"),
        Some(&owner),
        None,
    )
    .await;
    assert!(!message_ids(&trash).contains(&drop));
}

// ---------------------------------------------------------------------------
// Restore authorization (M-09/M-41): the undo-toast hot path's adversarial
// family. POST .../restore rides the shared require_own_message gate, but the
// UPDATE behind it is UNSCOPED (`UPDATE type::record('message', $mid) SET
// deleted_at = NONE`) — the gate is the ONLY thing between any authenticated
// user and un-deleting someone else's message, so every arm of the 403/404
// matrix is pinned here.
// ---------------------------------------------------------------------------

/// Soft-delete `mid` as its author, asserting the 204.
#[cfg(feature = "ssr")]
async fn soft_delete_message(router: &axum::Router, cookie: &str, cid: &str, mid: &str) {
    let (st, _, _) = common::send(
        router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NO_CONTENT,
        "soft-delete the fixture message"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn restoring_someone_elses_deleted_message_is_403() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (gid, cid) = guild_with_channel(&a.router, &owner).await;
    let mid = post_message(&a.router, &owner, &cid, "deliberately deleted").await;
    soft_delete_message(&a.router, &owner, &cid, &mid).await;

    // A second member of the same guild — the channel IS visible to them.
    let member = common::register_account(&a.router, "Member", "password123").await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "Member" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages/{mid}/restore"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "another member must not resurrect a message its author deleted"
    );

    // The message stays deleted: absent from the live page, still in the trash.
    let (_, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert!(
        !message_ids(&list).contains(&mid),
        "the rejected restore must not un-delete the row"
    );
    let (_, _, trash) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages/trash"),
        Some(&owner),
        None,
    )
    .await;
    assert!(message_ids(&trash).contains(&mid));
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn restore_collapses_to_privacy_404_for_non_members_and_unknown_channels() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (_gid, cid) = guild_with_channel(&a.router, &owner).await;
    let mid = post_message(&a.router, &owner, &cid, "private history").await;
    soft_delete_message(&a.router, &owner, &cid, &mid).await;

    let outsider = common::register_account(&a.router, "Outsider", "password123").await;

    // Non-member on the real (cid, mid) pair.
    let (st_real, _, body_real) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages/{mid}/restore"),
        Some(&outsider),
        None,
    )
    .await;
    // Any caller on an unknown channel.
    let (st_unknown, _, body_unknown) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/nosuchchannel/messages/{mid}/restore"),
        Some(&outsider),
        None,
    )
    .await;
    // The canonical privacy-404 every other channel handler emits.
    let (st_canon, _, body_canon) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&outsider),
        None,
    )
    .await;

    assert_eq!(st_real, StatusCode::NOT_FOUND);
    assert_eq!(st_unknown, StatusCode::NOT_FOUND);
    assert_eq!(st_canon, StatusCode::NOT_FOUND);
    assert_eq!(
        body_real, body_unknown,
        "non-member and unknown-channel restore bodies must be indistinguishable"
    );
    assert_eq!(
        body_real, body_canon,
        "the restore privacy-404 body must be byte-identical to the list handler's"
    );

    // And the probe must not have restored anything.
    let (_, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert!(!message_ids(&list).contains(&mid));
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn restoring_a_purged_message_is_403_not_500() {
    // After the hard-delete sweep the row is gone; the gate's "missing message"
    // arm collapses to the same 403 as "not yours" (no existence probe by mid).
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (_gid, cid) = guild_with_channel(&a.router, &owner).await;
    let mid = post_message(&a.router, &owner, &cid, "soon purged").await;
    soft_delete_message(&a.router, &owner, &cid, &mid).await;

    a.db.query("UPDATE type::record('message', $mid) SET deleted_at = time::now() - 2h;")
        .bind(("mid", mid.clone()))
        .await
        .expect("backdate")
        .check()
        .expect("backdate check");
    let state = AppState::new(a.db.clone(), a.media_dir.clone());
    purge_soft_deleted(&state).await.expect("purge");
    assert!(!row_exists(&a.db, "message", &mid).await, "fixture purged");

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages/{mid}/restore"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "restoring a purged (hard-deleted) message must 403, never 500"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn restore_of_an_already_live_message_is_an_idempotent_204() {
    // The undo toast can race its own timeout — a double restore (and a restore
    // of a never-deleted message) must stay a 204 no-op, not an error.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (_gid, cid) = guild_with_channel(&a.router, &owner).await;
    let mid = post_message(&a.router, &owner, &cid, "bounces back").await;
    soft_delete_message(&a.router, &owner, &cid, &mid).await;

    for label in ["first restore", "second (already-live) restore"] {
        let (st, _, _) = common::send(
            &a.router,
            Method::POST,
            &format!("/channels/{cid}/messages/{mid}/restore"),
            Some(&owner),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::NO_CONTENT, "{label} must 204");
    }

    // A restore of a message that was never deleted is the same no-op.
    let never_deleted = post_message(&a.router, &owner, &cid, "never deleted").await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages/{never_deleted}/restore"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // The restored message appears exactly once in the live page.
    let (_, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    let ids = message_ids(&list);
    assert_eq!(
        ids.iter().filter(|id| **id == mid).count(),
        1,
        "double restore must not duplicate the row"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn restore_with_a_cross_channel_message_id_is_403() {
    // The author gate is channel-scoped (`WHERE channel = $cid`), so a valid
    // mid presented under a DIFFERENT channel id must not be found — 403, and
    // the unscoped UPDATE behind the gate must never run.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (gid, cid1) = guild_with_channel(&a.router, &owner).await;
    let (st, _, c) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "second", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let cid2 = c["id"].as_str().unwrap().to_string();

    let mid = post_message(&a.router, &owner, &cid1, "lives in channel 1").await;
    soft_delete_message(&a.router, &owner, &cid1, &mid).await;

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid2}/messages/{mid}/restore"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "a cross-channel (cid, mid) pair must not pass the channel-scoped gate"
    );

    // Still deleted in its real channel.
    let (_, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid1}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert!(
        !message_ids(&list).contains(&mid),
        "the cross-channel restore must not have touched the row"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn restore_in_a_soft_deleted_channel_or_guild_is_privacy_404() {
    // channel_access filters `deleted_at` on both the channel AND its guild,
    // so a trashed container collapses the restore to the privacy-404 — a
    // deleted channel's history must not be mutable through the undo path.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;

    // Arm 1: the message's CHANNEL is soft-deleted (an extra channel, so the
    // guild stays non-empty).
    let (gid, _default_cid) = guild_with_channel(&a.router, &owner).await;
    let (st, _, c) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "doomed", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let cid = c["id"].as_str().unwrap().to_string();
    let mid = post_message(&a.router, &owner, &cid, "trapped").await;
    soft_delete_message(&a.router, &owner, &cid, &mid).await;
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}/channels/{cid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "soft-delete the channel");

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages/{mid}/restore"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "restore inside a soft-deleted channel must privacy-404"
    );

    // Arm 2: the GUILD is soft-deleted; restore via its (live) default channel.
    let (gid2, cid2) = guild_with_channel(&a.router, &owner).await;
    let mid2 = post_message(&a.router, &owner, &cid2, "guild goes down").await;
    soft_delete_message(&a.router, &owner, &cid2, &mid2).await;
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid2}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "soft-delete the guild");

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid2}/messages/{mid2}/restore"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "restore inside a soft-deleted guild must privacy-404"
    );
}

// ---------------------------------------------------------------------------
// purge_soft_deleted: windowed hard-delete + cascade
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn purge_hard_deletes_message_past_window_only() {
    // The sweep hard-deletes a soft-deleted message only once it is past the 1h
    // window. We backdate one beyond it and leave another freshly soft-deleted;
    // only the stale one is purged.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (_gid, cid) = guild_with_channel(&a.router, &owner).await;

    let stale = post_message(&a.router, &owner, &cid, "stale").await;
    let fresh = post_message(&a.router, &owner, &cid, "fresh").await;

    // Soft-delete both via the API.
    for mid in [&stale, &fresh] {
        common::send(
            &a.router,
            Method::DELETE,
            &format!("/channels/{cid}/messages/{mid}"),
            Some(&owner),
            None,
        )
        .await;
    }
    // Backdate the stale one's deleted_at past the 1h window.
    a.db.query("UPDATE type::record('message', $mid) SET deleted_at = time::now() - 2h;")
        .bind(("mid", stale.clone()))
        .await
        .expect("backdate")
        .check()
        .expect("backdate check");

    let state = AppState::new(a.db.clone(), a.media_dir.clone());
    purge_soft_deleted(&state).await.expect("purge");

    assert!(
        !row_exists(&a.db, "message", &stale).await,
        "stale soft-deleted message is hard-deleted by the sweep"
    );
    assert!(
        row_exists(&a.db, "message", &fresh).await,
        "freshly soft-deleted message (within 1h) survives the sweep"
    );
}

/// Soft-delete a guild and backdate its `deleted_at` past the 30d window,
/// returning `(guild_id, channel_id, message_id, owner_member_row_id)` so the
/// caller can assert the cascade. Shared by the two guild-purge tests below.
#[cfg(feature = "ssr")]
async fn doomed_guild(a: &common::Arena, owner: &str) -> (String, String, String, String) {
    let (gid, cid) = guild_with_channel(&a.router, owner).await;
    let mid = post_message(&a.router, owner, &cid, "in the doomed guild").await;

    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp =
        a.db.query(
            "SELECT meta::id(id) AS id_key FROM guild_member
                WHERE guild = type::record('guild', $gid);",
        )
        .bind(("gid", gid.clone()))
        .await
        .expect("member query")
        .check()
        .expect("member check");
    let member_id = resp
        .take::<Vec<IdRow>>(0)
        .expect("take")
        .into_iter()
        .next()
        .expect("owner membership exists")
        .id_key;

    common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}"),
        Some(owner),
        None,
    )
    .await;
    a.db.query("UPDATE type::record('guild', $gid) SET deleted_at = time::now() - 31d;")
        .bind(("gid", gid.clone()))
        .await
        .expect("backdate")
        .check()
        .expect("backdate check");

    (gid, cid, mid, member_id)
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn purge_cascades_guild_to_channels_and_messages() {
    // A guild past its 30d window is hard-deleted along with its channels and
    // messages (mod.rs:233-236), even though those children were never
    // individually soft-deleted. (Membership-row cascade is covered — and
    // currently FAILING — by the ignored test below.)
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (gid, cid, mid, _member_id) = doomed_guild(&a, &owner).await;

    let state = AppState::new(a.db.clone(), a.media_dir.clone());
    purge_soft_deleted(&state).await.expect("purge");

    assert!(
        !row_exists(&a.db, "guild", &gid).await,
        "guild past 30d window is hard-deleted"
    );
    assert!(
        !row_exists(&a.db, "channel", &cid).await,
        "cascade: the guild's channel is hard-deleted"
    );
    assert!(
        !row_exists(&a.db, "message", &mid).await,
        "cascade: the guild's message is hard-deleted"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn manager_mutations_on_a_soft_deleted_guild_are_404() {
    // F-D1a-1: a soft-deleted guild is invisible to reads and must be immutable.
    // require_manager refuses management mutations on a trashed guild (404),
    // rather than letting an owner/admin keep writing to a guild treated as gone.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (gid, _cid) = guild_with_channel(&a.router, &owner).await;

    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "soft-delete guild");

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "ghost", "kind": "text" })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "create_channel on a trashed guild must 404"
    );

    let (st, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}"),
        Some(&owner),
        Some(&json!({ "name": "ghost-name" })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "patch_guild on a trashed guild must 404"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn purge_cascades_guild_to_all_child_tables() {
    // F-D7-1/2: the 30d guild purge must also sweep guild/channel children it
    // previously orphaned — custom_emoji, user_guild_order, lorebook_entry,
    // channel_active_persona — not just channels/members/messages.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (gid, cid, _mid, _member) = doomed_guild(&a, &owner).await;

    // Seed one row in each child table keyed to the doomed guild/channel.
    // Referential existence is not enforced, so placeholder FK targets are fine.
    a.db.query(
        r#"
        CREATE type::record('custom_emoji', 'ce1') SET
            guild = type::record('guild', $gid), name = 'e',
            media = type::record('media_blob', 'm1'),
            creator = type::record('account', 'a1');
        CREATE type::record('user_guild_order', 'ugo1') SET
            account = type::record('account', 'a1'),
            guild = type::record('guild', $gid), position = 0;
        CREATE type::record('lorebook_entry', 'le1') SET
            channel = type::record('channel', $cid), keys = [], content = '';
        CREATE type::record('channel_active_persona', 'cap1') SET
            account = type::record('account', 'a1'),
            channel = type::record('channel', $cid),
            persona = type::record('persona', 'p1');
        "#,
    )
    .bind(("gid", gid.clone()))
    .bind(("cid", cid.clone()))
    .await
    .expect("seed children")
    .check()
    .expect("seed check");

    let children = [
        ("custom_emoji", "ce1"),
        ("user_guild_order", "ugo1"),
        ("lorebook_entry", "le1"),
        ("channel_active_persona", "cap1"),
    ];
    for (t, id) in children {
        assert!(row_exists(&a.db, t, id).await, "{t} seeded before purge");
    }

    let state = AppState::new(a.db.clone(), a.media_dir.clone());
    purge_soft_deleted(&state).await.expect("purge");

    for (t, id) in children {
        assert!(
            !row_exists(&a.db, t, id).await,
            "purge must sweep the orphaned {t} row of a purged guild"
        );
    }
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn purge_should_cascade_guild_member_rows() {
    // Regression guard for the guild_member cascade. This previously FAILED: the
    // 30d guild purge left orphan guild_member rows because SurrealDB 3.1.0-beta.3
    // mis-plans DELETE on a composite-index leading column + IN + a LET var. Fixed
    // in server/mod.rs by inlining the guild subquery; this asserts the correct
    // contract — membership rows are hard-deleted along with the purged guild.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (_gid, _cid, _mid, member_id) = doomed_guild(&a, &owner).await;

    let state = AppState::new(a.db.clone(), a.media_dir.clone());
    purge_soft_deleted(&state).await.expect("purge");

    assert!(
        !row_exists(&a.db, "guild_member", &member_id).await,
        "cascade: the purged guild's membership row should be hard-deleted"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn purge_cascades_channel_to_its_messages() {
    // A channel past its 1d window is hard-deleted along with its messages
    // (mod.rs:230-232), even messages that were never individually soft-deleted.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (gid, cid) = guild_with_channel(&a.router, &owner).await;
    let mid = post_message(&a.router, &owner, &cid, "in the doomed channel").await;

    common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}/channels/{cid}"),
        Some(&owner),
        None,
    )
    .await;
    a.db.query("UPDATE type::record('channel', $cid) SET deleted_at = time::now() - 2d;")
        .bind(("cid", cid.clone()))
        .await
        .expect("backdate")
        .check()
        .expect("backdate check");

    let state = AppState::new(a.db.clone(), a.media_dir.clone());
    purge_soft_deleted(&state).await.expect("purge");

    assert!(
        !row_exists(&a.db, "channel", &cid).await,
        "channel past 1d window is hard-deleted"
    );
    assert!(
        !row_exists(&a.db, "message", &mid).await,
        "cascade: the channel's message is hard-deleted (even un-soft-deleted)"
    );
    // The guild itself survives (only the channel was trashed).
    assert!(
        row_exists(&a.db, "guild", &gid).await,
        "the live guild is untouched by a channel purge"
    );
}

// ---------------------------------------------------------------------------
// DM purge cascade (M7/P1): a purged kind='dm' channel must take its dm_member
// rows, symmetric with the guild_member cascade. The LEAVE path hard-deletes
// each leaver's row and soft-deletes the thread only at zero members, so it
// never orphans — the first test pins that. The cascade itself guards any
// NON-leave soft-delete path (a future admin thread-delete / moderation tool /
// an unfriend-driven soft-delete), exercised directly in the second test so it
// cannot silently regress; without the purge's `DELETE dm_member` arm the rows
// would survive their channel.
// ---------------------------------------------------------------------------

/// Register an account, returning `(session_cookie, account_id)`.
#[cfg(feature = "ssr")]
async fn register_with_id(router: &axum::Router, name: &str) -> (String, String) {
    let (st, cookie, body) = common::send(
        router,
        Method::POST,
        "/auth/register",
        None,
        Some(&json!({ "username": name, "password": "password123" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "register({name})");
    (
        cookie.unwrap(),
        body["account_id"].as_str().unwrap().to_string(),
    )
}

/// The dm_member row ids still attached to a channel.
#[cfg(feature = "ssr")]
async fn dm_member_ids(
    db: &surrealdb::Surreal<surrealdb::engine::remote::ws::Client>,
    cid: &str,
) -> Vec<String> {
    let mut resp = db
        .query(
            "SELECT VALUE meta::id(id) FROM dm_member
                WHERE channel = type::record('channel', $cid);",
        )
        .bind(("cid", cid.to_string()))
        .await
        .expect("dm_member query")
        .check()
        .expect("dm_member check");
    resp.take(0).expect("take dm_member ids")
}

/// Register Alice + Bob, make them friends, and open a 1:1 DM. Returns
/// `(alice_cookie, bob_cookie, dm_channel_id)`.
#[cfg(feature = "ssr")]
async fn one_to_one_dm(a: &common::Arena) -> (String, String, String) {
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    // Alice friend-requests Bob; Bob accepts.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&alice),
        Some(&json!({ "username": "Bob" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "friend request");
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/friends/{alice_id}/accept"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "friend accept");
    let (st, _, dm) = common::send(
        &a.router,
        Method::POST,
        "/dms",
        Some(&alice),
        Some(&json!({ "members": [bob_id] })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "create 1:1 DM: {dm:?}");
    let cid = dm["id"].as_str().unwrap().to_string();
    (alice, bob, cid)
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn leaving_a_dm_removes_member_rows_so_the_leave_path_never_orphans() {
    // leave_dm hard-deletes the leaver's dm_member row; the thread is soft-deleted
    // only once membership hits zero. So at the moment the channel is soft-deleted
    // there are already zero dm_member rows — the leave path cannot orphan any.
    let a = common::arena().await;
    let (alice, bob, cid) = one_to_one_dm(&a).await;
    assert_eq!(dm_member_ids(&a.db, &cid).await.len(), 2, "two members");

    for who in [&alice, &bob] {
        let (st, _, _) = common::send(
            &a.router,
            Method::DELETE,
            &format!("/dms/{cid}/members/me"),
            Some(who),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::NO_CONTENT);
    }

    assert!(
        dm_member_ids(&a.db, &cid).await.is_empty(),
        "both leaves hard-deleted every dm_member row before soft-delete"
    );
    // The channel row itself still exists (soft-deleted, not yet purged); only
    // its membership rows are gone.
    assert!(
        row_exists(&a.db, "channel", &cid).await,
        "the soft-deleted DM channel row survives until purge"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn purge_should_cascade_dm_member_rows() {
    // Cascade guard: a kind='dm' channel soft-deleted while it STILL has members
    // (the shape a non-leave soft-delete path produces) must take its dm_member
    // rows on purge — exactly as the 30d guild purge takes guild_member. Without
    // the purge's `DELETE dm_member` arm these rows would orphan onto the
    // dm_member_account index forever.
    let a = common::arena().await;
    let (alice, _bob, cid) = one_to_one_dm(&a).await;
    let mid = post_message(&a.router, &alice, &cid, "in the doomed DM").await;

    let members = dm_member_ids(&a.db, &cid).await;
    assert_eq!(members.len(), 2, "two dm_member rows before purge");

    // Soft-delete the channel directly, members still attached, and backdate it
    // past the 1d window (members present is the contrast with the leave path).
    a.db.query("UPDATE type::record('channel', $cid) SET deleted_at = time::now() - 2d;")
        .bind(("cid", cid.clone()))
        .await
        .expect("backdate")
        .check()
        .expect("backdate check");

    let state = AppState::new(a.db.clone(), a.media_dir.clone());
    purge_soft_deleted(&state).await.expect("purge");

    assert!(
        !row_exists(&a.db, "channel", &cid).await,
        "the DM channel past 1d is hard-deleted"
    );
    assert!(
        !row_exists(&a.db, "message", &mid).await,
        "cascade: the DM's message is hard-deleted"
    );
    for id in &members {
        assert!(
            !row_exists(&a.db, "dm_member", id).await,
            "cascade: the purged DM's dm_member row {id} must be hard-deleted"
        );
    }
    assert!(
        dm_member_ids(&a.db, &cid).await.is_empty(),
        "no orphan dm_member rows survive the purged DM channel"
    );
}
