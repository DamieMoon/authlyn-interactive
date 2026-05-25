//! Step-3 integration tests: channel-scoped message post/list, verbatim
//! markup storage, the >100-message composite-cursor pagination canary,
//! membership privacy-404, and non-text-channel rejection.

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;

/// Register an owner, create a guild, and return
/// `(owner_cookie, guild_id, default_text_channel_id)`.
#[cfg(feature = "ssr")]
async fn owner_with_text_channel(router: &axum::Router) -> (String, String, String) {
    let owner = common::register_account(router, "Owner", "password123").await;
    let (status, _, guild) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();

    let (status, _, detail) = common::send(
        router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();
    (owner, gid, cid)
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn post_and_list_preserves_markup_verbatim() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    let raw = "hello **world** [red]!!![/red] *waves*";
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": raw })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["id"].is_string());

    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(
        msgs[0]["body"], raw,
        "markup is stored and returned verbatim"
    );
    assert!(msgs[0]["author_id"].is_string());
    // No personas exist in step 3 — the author wears none.
    assert!(msgs[0]["persona_id"].is_null());
    assert!(msgs[0]["persona_name"].is_null());
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn empty_body_is_400() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "   " })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nonmember_cannot_post_or_list() {
    let a = common::arena().await;
    let (_owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let outsider = common::register_account(&a.router, "Outsider", "password123").await;

    let (post, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&outsider),
        Some(&json!({ "body": "intruding" })),
    )
    .await;
    assert_eq!(post, StatusCode::NOT_FOUND);

    let (list, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(list, StatusCode::NOT_FOUND);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn posting_to_a_lorebook_channel_is_400() {
    let a = common::arena().await;
    let (owner, gid, _cid) = owner_with_text_channel(&a.router).await;

    let (status, _, lore) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "world", "kind": "lorebook" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let lore_cid = lore["id"].as_str().unwrap();

    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{lore_cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "should be rejected" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn cursor_paginates_past_100_in_order() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    const TOTAL: usize = 150;
    for i in 0..TOTAL {
        let (status, _, _) = common::send(
            &a.router,
            Method::POST,
            &format!("/channels/{cid}/messages"),
            Some(&owner),
            Some(&json!({ "body": format!("m{i}") })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // Page 1: the first 100, ASC by (sent_at, id).
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page1 = body["messages"].as_array().unwrap().clone();
    assert_eq!(page1.len(), 100);

    // Page 2: resume from the last row's composite cursor.
    let last = page1.last().unwrap();
    let since = last["sent_at"].as_str().unwrap();
    let after_id = last["id"].as_str().unwrap();
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages?since={since}&after_id={after_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page2 = body["messages"].as_array().unwrap().clone();
    assert_eq!(page2.len(), 50, "the remaining messages");

    // The two pages together are exactly m0..m149, in order, no dups/gaps.
    let bodies: Vec<String> = page1
        .iter()
        .chain(page2.iter())
        .map(|m| m["body"].as_str().unwrap().to_string())
        .collect();
    let expected: Vec<String> = (0..TOTAL).map(|i| format!("m{i}")).collect();
    assert_eq!(bodies, expected, "cursor pages reassemble in send order");

    let ids: std::collections::HashSet<&str> = page1
        .iter()
        .chain(page2.iter())
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids.len(),
        TOTAL,
        "no duplicate ids across the cursor boundary"
    );
}
