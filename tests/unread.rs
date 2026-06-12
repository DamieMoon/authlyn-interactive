//! W1: batched GET /unread — cursor math (strict composite tie-break), ping
//! flag, baseline fields, privacy (only visible channels appear), and the
//! soft-delete exclusions (deleted messages/channels/guilds never count —
//! review M-16). The equal-`sent_at` tie-group test (review M-11) pins the
//! strict boundary semantics across the index-friendly split of the unread
//! predicate into an equality + open-range statement pair.
#![cfg(feature = "ssr")]

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;

/// Register an owner, create a guild, and return
/// `(owner_cookie, guild_id, default_text_channel_id)`.
async fn setup(router: &axum::Router) -> (String, String, String) {
    let owner = common::register_account(router, "UnreadOwner", "password123").await;
    let (st, _, guild) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "U" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();
    let (_, _, detail) = common::send(
        router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();
    (owner, gid, cid)
}

/// Invite `username` (already registered) into `gid` as a `member` — the same
/// membership mechanism `tests/mentions.rs` uses.
async fn invite(router: &axum::Router, owner: &str, gid: &str, username: &str) {
    let (status, _, _) = common::send(
        router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(owner),
        Some(&json!({ "username": username })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "invite({username}) should 201");
}

/// Post `body` to `cid` and return the full message envelope. POST returns
/// only `{ id }`, so the envelope (with its server-formatted `sent_at` cursor
/// key) is read back over GET — the `tests/read_state.rs` pattern.
async fn post_msg(router: &axum::Router, cookie: &str, cid: &str, body: &str) -> serde_json::Value {
    let (st, _, m) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(cookie),
        Some(&json!({ "body": body })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let id = m["id"].as_str().unwrap().to_string();
    let (st, _, page) = common::send(
        router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    page["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id.as_str())
        .expect("posted message must appear in the page")
        .clone()
}

/// Force a message's `sent_at` to an exact instant via direct DB write. The
/// API stamps `time::now()` at send, so an equal-`sent_at` tie group — the
/// boundary the strict composite cursor tie-break exists for — can only be
/// built this way. Parameterized: the datetime rides `type::datetime($at)`,
/// never a `<string>` cast (the cursor invariant).
async fn force_sent_at(a: &common::Arena, mid: &str, at: &str) {
    a.db.query("UPDATE type::record('message', $mid) SET sent_at = type::datetime($at);")
        .bind(("mid", mid.to_string()))
        .bind(("at", at.to_string()))
        .await
        .expect("force sent_at transport")
        .check()
        .expect("force sent_at");
}

/// POST /channels/{cid}/mark-read at an explicit `(sent_at, id)` cursor and
/// assert the 204.
async fn mark_read_at(router: &axum::Router, cookie: &str, cid: &str, sent_at: &str, id: &str) {
    let (st, _, _) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/mark-read"),
        Some(cookie),
        Some(&json!({ "sent_at": sent_at, "id": id })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
}

/// GET /unread and return the row for `cid` (panics when absent — the
/// presence-negative tests scan the array themselves).
async fn unread_row(router: &axum::Router, cookie: &str, cid: &str) -> serde_json::Value {
    let (st, _, body) = common::send(router, Method::GET, "/unread", Some(cookie), None).await;
    assert_eq!(st, StatusCode::OK);
    body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["channel_id"] == cid)
        .expect("channel row in /unread")
        .clone()
}

#[tokio::test]
async fn unread_counts_messages_past_the_cursor_and_baselines_unvisited() {
    let a = common::arena().await;
    let (owner, gid, cid) = setup(&a.router).await;

    let m1 = post_msg(&a.router, &owner, &cid, "one").await;
    let _m2 = post_msg(&a.router, &owner, &cid, "two").await;
    let m3 = post_msg(&a.router, &owner, &cid, "three").await;

    // No cursor yet → unread 0, but latest_* exposes the baseline.
    let (st, _, body) = common::send(&a.router, Method::GET, "/unread", Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    let rows = body["channels"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|r| r["channel_id"] == cid.as_str())
        .unwrap();
    assert_eq!(row["guild_id"], gid.as_str());
    assert_eq!(row["unread"], 0);
    assert_eq!(row["pinged"], false);
    assert_eq!(row["latest_id"], m3["id"]);
    assert_eq!(row["latest_sent_at"], m3["sent_at"]);

    // Mark read at m1 → exactly 2 unread (strict tie-break: m1 itself excluded).
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/mark-read"),
        Some(&owner),
        Some(&json!({ "sent_at": m1["sent_at"], "id": m1["id"] })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (_, _, body) = common::send(&a.router, Method::GET, "/unread", Some(&owner), None).await;
    let rows = body["channels"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|r| r["channel_id"] == cid.as_str())
        .unwrap();
    assert_eq!(
        row["unread"], 2,
        "m2 and m3 are unread; m1 (the cursor) is not"
    );
}

#[tokio::test]
async fn unread_ping_flag_fires_only_on_unread_mentions() {
    let a = common::arena().await;
    let (owner, gid, cid) = setup(&a.router).await;
    let buddy = common::register_account(&a.router, "UnreadBuddy", "password123").await;
    invite(&a.router, &owner, &gid, "UnreadBuddy").await;

    let m1 = post_msg(&a.router, &owner, &cid, "hello").await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/mark-read"),
        Some(&buddy),
        Some(&json!({ "sent_at": m1["sent_at"], "id": m1["id"] })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    post_msg(&a.router, &owner, &cid, "@UnreadBuddy the watch begins").await;

    let (_, _, body) = common::send(&a.router, Method::GET, "/unread", Some(&buddy), None).await;
    let rows = body["channels"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|r| r["channel_id"] == cid.as_str())
        .unwrap();
    assert_eq!(row["unread"], 1);
    assert_eq!(row["pinged"], true);
}

#[tokio::test]
async fn unread_mixed_batch_keeps_statement_indices_aligned() {
    let a = common::arena().await;
    let (owner, gid, cid_general) = setup(&a.router).await;
    // Two more channels → three visible channels in ONE batch:
    // general (cursored, 2 unread + ping), annex (cursorless, baseline),
    // archive (cursored, fully read → 0 unread).
    let (st, _, annex) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "annex", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let cid_annex = annex["id"].as_str().unwrap().to_string();
    let (st, _, archive) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "archive", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let cid_archive = archive["id"].as_str().unwrap().to_string();

    // general: baseline at m1, then a plain message + a ping → 2 unread,
    // pinged. Two unread vs the LIMIT-1 ping probe keeps the counts
    // DIFFERENT, so a swapped Unread/Ping statement mapping cannot
    // masquerade as correct (unread would read the probe and report 1).
    let m1 = post_msg(&a.router, &owner, &cid_general, "before").await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid_general}/mark-read"),
        Some(&owner),
        Some(&json!({ "sent_at": m1["sent_at"], "id": m1["id"] })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    post_msg(&a.router, &owner, &cid_general, "filler").await;
    post_msg(&a.router, &owner, &cid_general, "@UnreadOwner ping").await;

    // annex: two messages, never visited → unread 0 + latest baseline.
    post_msg(&a.router, &owner, &cid_annex, "a1").await;
    let a2 = post_msg(&a.router, &owner, &cid_annex, "a2").await;

    // archive: one message, fully read → cursored with 0 unread.
    let r1 = post_msg(&a.router, &owner, &cid_archive, "r1").await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid_archive}/mark-read"),
        Some(&owner),
        Some(&json!({ "sent_at": r1["sent_at"], "id": r1["id"] })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (st, _, body) = common::send(&a.router, Method::GET, "/unread", Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    let rows = body["channels"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "all three visible channels appear");
    let find = |cid: &str| rows.iter().find(|r| r["channel_id"] == cid).unwrap();

    let g = find(&cid_general);
    assert_eq!(g["unread"], 2);
    assert_eq!(g["pinged"], true);

    let an = find(&cid_annex);
    assert_eq!(an["unread"], 0);
    assert_eq!(an["pinged"], false);
    assert_eq!(an["latest_id"], a2["id"]);

    let ar = find(&cid_archive);
    assert_eq!(ar["unread"], 0);
    assert_eq!(ar["pinged"], false);
}

#[tokio::test]
async fn unread_lists_only_channels_the_caller_can_see() {
    let a = common::arena().await;
    let (_owner, _gid, cid) = setup(&a.router).await;
    let outsider = common::register_account(&a.router, "UnreadOutsider", "password123").await;

    let (st, _, body) =
        common::send(&a.router, Method::GET, "/unread", Some(&outsider), None).await;
    assert_eq!(st, StatusCode::OK);
    let rows = body["channels"].as_array().unwrap();
    assert!(
        rows.iter().all(|r| r["channel_id"] != cid.as_str()),
        "foreign channels must not appear in /unread"
    );
}

/// Review M-11: the strict composite tie-break at an equal-`sent_at` tie
/// group, for BOTH the unread count and the ping probe. Five mention-carrying
/// messages share ONE instant; the cursor walks the tie group by id. Pins the
/// boundary semantics the split (equality + open-range) statements must
/// reassemble exactly: the cursor row and lexically-lesser tie ids are read,
/// strictly-greater tie ids and later rows are unread.
#[tokio::test]
async fn unread_tie_break_at_equal_sent_at_counts_only_strictly_higher_ids() {
    let a = common::arena().await;
    let (owner, gid, cid) = setup(&a.router).await;
    let buddy = common::register_account(&a.router, "UnreadTieBuddy", "password123").await;
    invite(&a.router, &owner, &gid, "UnreadTieBuddy").await;

    const T0: &str = "2026-01-01T10:00:00Z";
    const T1: &str = "2026-01-01T10:00:01Z";
    const T2: &str = "2026-01-01T10:00:02Z";

    // Five tie-group messages, every one mentioning buddy (so the ping probe
    // is exercised at the same boundary), then two later plain messages.
    let mut ties: Vec<String> = Vec::new();
    for i in 0..5 {
        let m = post_msg(&a.router, &owner, &cid, &format!("@UnreadTieBuddy tie {i}")).await;
        ties.push(m["id"].as_str().unwrap().to_string());
    }
    let later1 = post_msg(&a.router, &owner, &cid, "after one").await;
    let later2 = post_msg(&a.router, &owner, &cid, "after two").await;
    let later2_id = later2["id"].as_str().unwrap().to_string();
    for id in &ties {
        force_sent_at(&a, id, T0).await;
    }
    force_sent_at(&a, later1["id"].as_str().unwrap(), T1).await;
    force_sent_at(&a, &later2_id, T2).await;
    // The tie-break compares id strings; walk the cursor along the sorted ids.
    ties.sort();

    // Cursor ON the middle tie row: the two strictly-greater tie ids + the
    // two later rows are unread; the cursor row itself and the lesser tie ids
    // are not. The greater tie rows carry mentions → pinged.
    mark_read_at(&a.router, &buddy, &cid, T0, &ties[2]).await;
    let row = unread_row(&a.router, &buddy, &cid).await;
    assert_eq!(
        row["unread"], 4,
        "two greater tie ids + two later messages; the cursor row is read"
    );
    assert_eq!(
        row["pinged"], true,
        "unread tie rows above the cursor id mention buddy"
    );

    // Cursor on the TOP tie id: the whole tie group is read; only the two
    // later (mention-free) rows remain.
    mark_read_at(&a.router, &buddy, &cid, T0, &ties[4]).await;
    let row = unread_row(&a.router, &buddy, &cid).await;
    assert_eq!(row["unread"], 2, "the whole tie group is behind the cursor");
    assert_eq!(
        row["pinged"], false,
        "every mention sits at or below the cursor"
    );

    // Fully caught up (cursor = the newest row): zero unread — the exact case
    // that must not degrade into a whole-channel walk.
    mark_read_at(&a.router, &buddy, &cid, T2, &later2_id).await;
    let row = unread_row(&a.router, &buddy, &cid).await;
    assert_eq!(row["unread"], 0);
    assert_eq!(row["pinged"], false);
}

/// Review M-16: a soft-deleted message stops counting toward unread (and
/// stops pinging) IMMEDIATELY, and a restore (the undo-toast round trip)
/// brings both back — the `deleted_at = NONE` filter in the unread and ping
/// statements is load-bearing, not decorative.
#[tokio::test]
async fn soft_deleted_message_stops_counting_toward_unread_until_restored() {
    let a = common::arena().await;
    let (owner, gid, cid) = setup(&a.router).await;
    let buddy = common::register_account(&a.router, "UnreadTrashBuddy", "password123").await;
    invite(&a.router, &owner, &gid, "UnreadTrashBuddy").await;

    let m1 = post_msg(&a.router, &owner, &cid, "baseline").await;
    mark_read_at(
        &a.router,
        &buddy,
        &cid,
        m1["sent_at"].as_str().unwrap(),
        m1["id"].as_str().unwrap(),
    )
    .await;
    let m2 = post_msg(&a.router, &owner, &cid, "@UnreadTrashBuddy soon gone").await;
    let m2_id = m2["id"].as_str().unwrap().to_string();

    let row = unread_row(&a.router, &buddy, &cid).await;
    assert_eq!(row["unread"], 1);
    assert_eq!(row["pinged"], true);

    // Soft-delete → the badge and the ping must drop it immediately.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{m2_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let row = unread_row(&a.router, &buddy, &cid).await;
    assert_eq!(
        row["unread"], 0,
        "a soft-deleted message must not count toward unread"
    );
    assert_eq!(row["pinged"], false, "a soft-deleted mention must not ping");

    // Restore → it counts (and pings) again.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages/{m2_id}/restore"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let row = unread_row(&a.router, &buddy, &cid).await;
    assert_eq!(
        row["unread"], 1,
        "a restored message counts toward unread again"
    );
    assert_eq!(row["pinged"], true, "a restored mention pings again");
}

/// Review M-16: a soft-deleted channel's row disappears from /unread (its
/// messages must not inflate any badge), while sibling live channels stay.
#[tokio::test]
async fn soft_deleted_channel_disappears_from_unread() {
    let a = common::arena().await;
    let (owner, gid, cid_general) = setup(&a.router).await;
    let (st, _, annex) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "annex", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let cid_annex = annex["id"].as_str().unwrap().to_string();
    post_msg(&a.router, &owner, &cid_annex, "doomed channel content").await;

    // Present while live…
    unread_row(&a.router, &owner, &cid_annex).await;

    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}/channels/{cid_annex}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // …gone once trashed; the live sibling keeps its row.
    let (st, _, body) = common::send(&a.router, Method::GET, "/unread", Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    let rows = body["channels"].as_array().unwrap();
    assert!(
        rows.iter().all(|r| r["channel_id"] != cid_annex.as_str()),
        "a soft-deleted channel must not appear in /unread"
    );
    assert!(
        rows.iter().any(|r| r["channel_id"] == cid_general.as_str()),
        "the live sibling channel still appears"
    );
}

/// Review M-16: soft-deleting a GUILD drops every one of its channels from
/// /unread — the `guild.deleted_at = NONE` clause in `visible_channels`, the
/// filter /unread (and SSE) seed from.
#[tokio::test]
async fn soft_deleted_guild_drops_its_channels_from_unread() {
    let a = common::arena().await;
    let (owner, gid, cid) = setup(&a.router).await;
    post_msg(&a.router, &owner, &cid, "guild going down").await;

    // Present while live…
    unread_row(&a.router, &owner, &cid).await;

    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (st, _, body) = common::send(&a.router, Method::GET, "/unread", Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    let rows = body["channels"].as_array().unwrap();
    assert!(
        rows.iter().all(|r| r["guild_id"] != gid.as_str()),
        "a soft-deleted guild's channels must not appear in /unread"
    );
}
