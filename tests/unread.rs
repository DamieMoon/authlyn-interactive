//! W1: batched GET /unread — cursor math (strict composite tie-break), ping
//! flag, baseline fields, and privacy (only visible channels appear).
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
