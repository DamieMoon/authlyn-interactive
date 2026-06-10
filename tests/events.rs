//! W1 SSE bus: GET /events delivery, privacy filtering, and auth gating.
#![cfg(feature = "ssr")]

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;
use std::time::Duration;

/// Register an owner, create a guild, and return
/// `(owner_cookie, guild_id, default_text_channel_id)`.
async fn owner_with_channel(router: &axum::Router) -> (String, String, String) {
    let owner = common::register_account(router, "EventsOwner", "password123").await;
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
    let (owner, _gid, cid) = owner_with_channel(&a.router).await;

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
        other => panic!("expected an event within 3s, got {other:?}"),
    };
    assert_eq!(ev["type"], "message_created");
    assert_eq!(ev["channel_id"], cid.as_str());
}
