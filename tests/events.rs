//! W1 SSE bus: GET /events delivery, privacy filtering, and auth gating.
#![cfg(feature = "ssr")]

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;
use std::time::Duration;

/// Register an owner (under `username`), create a guild, and return
/// `(owner_cookie, guild_id, default_text_channel_id)`.
async fn owner_with_channel(router: &axum::Router, username: &str) -> (String, String, String) {
    let owner = common::register_account(router, username, "password123").await;
    let (st, _, guild) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "Bus" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();
    let (st, _, detail) = common::send(
        router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();
    (owner, gid, cid)
}

#[tokio::test]
async fn events_requires_a_session() {
    let a = common::arena().await;
    let (status, _headers, _body) = common::open_sse(&a.router, "/events", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn member_receives_message_created_over_sse() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router, "EventsOwner").await;

    let (status, headers, mut body) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(axum::http::header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap()),
        Some("text/event-stream"),
        "EventSource hard-fails on a wrong content type"
    );

    // Post a message AFTER subscribing.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "ping over the bus" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let ev = match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        other => panic!("expected an event within the window, got {other:?}"),
    };
    assert_eq!(ev["type"], "message_created");
    assert_eq!(ev["channel_id"], cid.as_str());
}

#[tokio::test]
async fn outsider_never_receives_events_for_a_channel_they_cannot_see() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router, "EventsOwner").await;
    let outsider = common::register_account(&a.router, "EventsOutsider", "password123").await;

    let (status, _h, mut out_body) = common::open_sse(&a.router, "/events", Some(&outsider)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _h, mut own_body) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(status, StatusCode::OK);

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "secret" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // The member sees it…
    let ev = match common::next_sse_data(&mut own_body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        other => panic!("member should receive the event, got {other:?}"),
    };
    assert_eq!(ev["type"], "message_created");

    // …the outsider's stream stays open AND silent: Timeout, not Closed.
    match common::next_sse_data(&mut out_body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        other => panic!("outsider must time out silently, got {other:?}"),
    }
}

/// Typing emits on the bus (Task 6) — this is load-bearing: the per-connection
/// visibility filter is what keeps the cross-guild window silent.
#[tokio::test]
async fn typing_events_do_not_leak_across_guilds() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router, "EventsOwner").await;
    // Unrelated account with their own guild, so their visible set is non-empty.
    let other = common::register_account(&a.router, "EventsOther", "password123").await;
    let (st, _, other_guild) = common::send(
        &a.router,
        Method::POST,
        "/guilds",
        Some(&other),
        Some(&json!({ "name": "Elsewhere" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let other_gid = other_guild["id"].as_str().unwrap().to_string();
    let (_, _, other_detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{other_gid}"),
        Some(&other),
        None,
    )
    .await;
    let other_cid = other_detail["channels"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (_, _h, mut other_body) = common::open_sse(&a.router, "/events", Some(&other)).await;

    // Owner types in THEIR guild — nothing may reach `other`.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/typing"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    match common::next_sse_data(&mut other_body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        other_read => panic!("cross-guild typing leaked: {other_read:?}"),
    }

    // Aliveness proof: the SAME stream still delivers what it IS allowed to see.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{other_cid}/messages"),
        Some(&other),
        Some(&json!({ "body": "proof of life" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let ev = match common::next_sse_data(&mut other_body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        o => panic!("aliveness event should arrive, got {o:?}"),
    };
    assert_eq!(ev["type"], "message_created");
    assert_eq!(ev["channel_id"], other_cid.as_str());
}

#[tokio::test]
async fn edits_deletes_and_typing_reach_members_over_sse() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router, "EventsEditOwner").await;

    // Seed a message BEFORE subscribing (so its create event isn't in the stream).
    let (st, _, msg) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "v1" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let mid = msg["id"].as_str().unwrap().to_string();

    let (_, _h, mut body) = common::open_sse(&a.router, "/events", Some(&owner)).await;

    // Edit → message_edited
    let (st, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        Some(&json!({ "body": "v2" })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let ev = match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        other => panic!("expected message_edited, got {other:?}"),
    };
    assert_eq!(ev["type"], "message_edited");
    assert_eq!(ev["message_id"], mid.as_str());

    // Typing → typing
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/typing"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let ev = match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        other => panic!("expected typing, got {other:?}"),
    };
    assert_eq!(ev["type"], "typing");
    assert_eq!(ev["channel_id"], cid.as_str());

    // Delete → message_deleted
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let ev = match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        other => panic!("expected message_deleted, got {other:?}"),
    };
    assert_eq!(ev["type"], "message_deleted");
    assert_eq!(ev["message_id"], mid.as_str());
}

#[tokio::test]
async fn channel_creation_emits_lists_changed_and_membership_set_refreshes() {
    let a = common::arena().await;
    let (owner, gid, _cid) = owner_with_channel(&a.router, "EventsListsOwner").await;
    let (_, _h, mut body) = common::open_sse(&a.router, "/events", Some(&owner)).await;

    // Create a new channel → lists_changed must arrive.
    let (st, _, chan) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "annex", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let new_cid = chan["id"].as_str().unwrap().to_string();
    let ev = match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        other => panic!("expected lists_changed, got {other:?}"),
    };
    assert_eq!(ev["type"], "lists_changed");

    // …and the connection's visibility set must now include the NEW channel:
    // a message there must reach this same (pre-existing) SSE connection.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{new_cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "born after subscribe" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let ev = match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        other => panic!("expected message_created in the new channel, got {other:?}"),
    };
    assert_eq!(ev["type"], "message_created");
    assert_eq!(ev["channel_id"], new_cid.as_str());
}
