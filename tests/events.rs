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

#[tokio::test]
async fn outsider_never_receives_events_for_a_channel_they_cannot_see() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router).await;
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

/// NOTE: typing does not emit on the bus yet — Task 6 arms it. This passes
/// today for the trivial reason (nothing is broadcast at all) and must KEEP
/// passing once typing emissions land, at which point the filter is what
/// keeps the cross-guild window silent.
#[tokio::test]
async fn typing_events_do_not_leak_across_guilds() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router).await;
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
