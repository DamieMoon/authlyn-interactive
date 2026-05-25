//! Step-5 integration tests: lorebook entry CRUD scoped to a lorebook
//! channel, position ordering, the non-lorebook-channel 400, and the
//! membership privacy-404.

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;

/// Create a guild and a lorebook channel; return
/// `(guild_id, default_text_channel_id, lorebook_channel_id)`.
#[cfg(feature = "ssr")]
async fn guild_with_lorebook(router: &axum::Router, cookie: &str) -> (String, String, String) {
    let (_, _, g) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(cookie),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    let gid = g["id"].as_str().unwrap().to_string();
    let (_, _, d) = common::send(
        router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(cookie),
        None,
    )
    .await;
    let text_cid = d["channels"][0]["id"].as_str().unwrap().to_string();
    let (_, _, c) = common::send(
        router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(cookie),
        Some(&json!({ "name": "world", "kind": "lorebook" })),
    )
    .await;
    let lore_cid = c["id"].as_str().unwrap().to_string();
    (gid, text_cid, lore_cid)
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn crud_and_position_ordering() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (_gid, _text, lcid) = guild_with_lorebook(&a.router, &owner).await;

    // Two entries, inserted out of position order.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{lcid}/lorebook"),
        Some(&owner),
        Some(&json!({ "title": "Dragons", "keys": ["dragon", "wyrm"], "content": "they hoard gold", "position": 1 })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let (st, _, b) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{lcid}/lorebook"),
        Some(&owner),
        Some(&json!({ "keys": ["castle"], "content": "the keep on the hill", "position": 0 })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let castle_id = b["id"].as_str().unwrap().to_string();

    // Listed in position order: castle (0) before Dragons (1).
    let (st, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{lcid}/lorebook"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["content"], "the keep on the hill");
    assert_eq!(entries[1]["title"], "Dragons");
    assert_eq!(entries[1]["keys"], json!(["dragon", "wyrm"]));
    assert_eq!(entries[1]["enabled"], true);

    // Patch the castle entry: disable + change content.
    let (st, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/channels/{lcid}/lorebook/{castle_id}"),
        Some(&owner),
        Some(&json!({ "content": "the ruined keep", "enabled": false })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Delete the castle entry.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{lcid}/lorebook/{castle_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (_, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{lcid}/lorebook"),
        Some(&owner),
        None,
    )
    .await;
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["title"], "Dragons");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn lorebook_ops_on_a_text_channel_are_400() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (_gid, text_cid, _lore) = guild_with_lorebook(&a.router, &owner).await;

    let (post, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{text_cid}/lorebook"),
        Some(&owner),
        Some(&json!({ "keys": [], "content": "nope" })),
    )
    .await;
    assert_eq!(post, StatusCode::BAD_REQUEST);

    let (list, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{text_cid}/lorebook"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(list, StatusCode::BAD_REQUEST);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nonmember_cannot_touch_lorebook() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (_gid, _text, lcid) = guild_with_lorebook(&a.router, &owner).await;
    let outsider = common::register_account(&a.router, "Outsider", "password123").await;

    let (list, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{lcid}/lorebook"),
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(list, StatusCode::NOT_FOUND);

    let (post, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{lcid}/lorebook"),
        Some(&outsider),
        Some(&json!({ "keys": ["x"], "content": "intruding" })),
    )
    .await;
    assert_eq!(post, StatusCode::NOT_FOUND);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn empty_content_is_rejected() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let (_gid, _text, lcid) = guild_with_lorebook(&a.router, &owner).await;

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{lcid}/lorebook"),
        Some(&owner),
        Some(&json!({ "keys": ["x"], "content": "   " })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}
