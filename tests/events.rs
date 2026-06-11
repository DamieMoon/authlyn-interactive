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

/// W1.5: `mark-read` nudges the SAME account's OTHER devices
/// (`read_state_changed`, account-targeted) and nobody else.
#[tokio::test]
async fn read_state_changes_reach_only_the_same_account() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router, "ReadStateOwner").await;

    // Seed a message so there is a (sent_at, id) cursor to mark read.
    let (st, _, msg) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "to be read" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let mid = msg["id"].as_str().unwrap().to_string();
    let (st, _, page) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let sent_at = page["messages"][0]["sent_at"].as_str().unwrap().to_string();

    // A different account WITH a guild of their own (non-empty visibility set,
    // so silence below can't be blamed on an empty filter).
    let (other, _other_gid, other_cid) = owner_with_channel(&a.router, "ReadStateOutsider").await;

    // TWO streams on the same cookie = the same account's two devices.
    let (st, _h, mut device_a) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _h, mut device_b) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _h, mut other_body) = common::open_sse(&a.router, "/events", Some(&other)).await;
    assert_eq!(st, StatusCode::OK);

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/mark-read"),
        Some(&owner),
        Some(&json!({ "sent_at": sent_at, "id": mid })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // BOTH of the caller's devices get the nudge…
    for (name, body) in [("device A", &mut device_a), ("device B", &mut device_b)] {
        let ev = match common::next_sse_data(body, Duration::from_secs(3)).await {
            common::SseRead::Data(v) => v,
            other => panic!("{name} should receive read_state_changed, got {other:?}"),
        };
        assert_eq!(ev["type"], "read_state_changed");
        assert_eq!(ev["channel_id"], cid.as_str());
    }

    // …the unrelated account gets NOTHING (Timeout, not Closed)…
    match common::next_sse_data(&mut other_body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        o => panic!("another account must not see read_state_changed, got {o:?}"),
    }

    // …and that silence is not a dead stream: it still delivers its own events.
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
}

/// W1.5: every friend mutation (request, accept, remove) reaches EXACTLY the
/// two accounts of the friendship edge as `friends_changed` — a third account
/// stays silent.
#[tokio::test]
async fn friend_mutations_reach_both_parties_over_sse() {
    let a = common::arena().await;
    let alice = common::register_account(&a.router, "FriendsAlice", "password123").await;
    let (st, _, me) = common::send(&a.router, Method::GET, "/auth/me", Some(&alice), None).await;
    assert_eq!(st, StatusCode::OK);
    let alice_id = me["account_id"].as_str().unwrap().to_string();
    let bob = common::register_account(&a.router, "FriendsBob", "password123").await;
    let (st, _, me) = common::send(&a.router, Method::GET, "/auth/me", Some(&bob), None).await;
    assert_eq!(st, StatusCode::OK);
    let bob_id = me["account_id"].as_str().unwrap().to_string();
    // The bystander owns a guild so their visible set is non-empty and the
    // stream can prove aliveness afterwards.
    let (carol, _gid, carol_cid) = owner_with_channel(&a.router, "FriendsCarol").await;

    let (st, _h, mut alice_body) = common::open_sse(&a.router, "/events", Some(&alice)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _h, mut bob_body) = common::open_sse(&a.router, "/events", Some(&bob)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _h, mut carol_body) = common::open_sse(&a.router, "/events", Some(&carol)).await;
    assert_eq!(st, StatusCode::OK);

    async fn expect_friends_changed(body: &mut axum::body::Body, who: &str) {
        match common::next_sse_data(body, Duration::from_secs(3)).await {
            common::SseRead::Data(v) => {
                assert_eq!(v["type"], "friends_changed", "{who}");
            }
            other => panic!("{who} should receive friends_changed, got {other:?}"),
        }
    }

    // Request: Alice → Bob. Both edge accounts get the nudge (this is the
    // regression under test: an incoming request must be visible LIVE).
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&alice),
        Some(&json!({ "username": "FriendsBob" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    expect_friends_changed(&mut alice_body, "alice (request)").await;
    expect_friends_changed(&mut bob_body, "bob (request)").await;

    // Accept: Bob accepts Alice's request → both sides again.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/friends/{alice_id}/accept"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    expect_friends_changed(&mut alice_body, "alice (accept)").await;
    expect_friends_changed(&mut bob_body, "bob (accept)").await;

    // Remove: Alice unfriends Bob → both sides again.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/friends/{bob_id}"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    expect_friends_changed(&mut alice_body, "alice (remove)").await;
    expect_friends_changed(&mut bob_body, "bob (remove)").await;

    // The bystander saw none of it (Timeout, not Closed)…
    match common::next_sse_data(&mut carol_body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        o => panic!("a third account must not see friends_changed, got {o:?}"),
    }
    // …and the silence is not a dead stream.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{carol_cid}/messages"),
        Some(&carol),
        Some(&json!({ "body": "proof of life" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let ev = match common::next_sse_data(&mut carol_body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        o => panic!("aliveness event should arrive, got {o:?}"),
    };
    assert_eq!(ev["type"], "message_created");
}

/// W1.5: reordering YOUR guild rail is a per-user preference — the actor's
/// own (other) devices get `lists_changed`, every other connection gets
/// nothing (this used to broadcast globally: N×M amplification).
#[tokio::test]
async fn rail_reorder_no_longer_broadcasts() {
    let a = common::arena().await;
    let (owner, gid, _cid) = owner_with_channel(&a.router, "RailOwner").await;
    // Unrelated account with their own guild (non-empty visible set).
    let (other, _other_gid, other_cid) = owner_with_channel(&a.router, "RailOther").await;

    let (st, _h, mut owner_body) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _h, mut other_body) = common::open_sse(&a.router, "/events", Some(&other)).await;
    assert_eq!(st, StatusCode::OK);

    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        "/rail/order",
        Some(&owner),
        Some(&json!({ "guild_ids": [gid] })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // The actor (their other devices) still gets the refresh nudge…
    let ev = match common::next_sse_data(&mut owner_body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        o => panic!("the actor should receive lists_changed, got {o:?}"),
    };
    assert_eq!(ev["type"], "lists_changed");

    // …everyone else stays silent (Timeout, not Closed)…
    match common::next_sse_data(&mut other_body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        o => panic!("rail reorder must not broadcast, got {o:?}"),
    }
    // …with the usual aliveness proof.
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
