//! M1 SSE bus: GET /events delivery, privacy filtering, auth gating, and the
//! mid-stream REVOCATION direction (session logout/reset, kick/leave/guild
//! soft-delete, targeted visibility reloads — reviews M-05/M-07/M-14/M-43).
#![cfg(feature = "ssr")]

mod common;

use authlyn_interactive::protocol::SyncEvent;
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

/// M6/P2: an account profile change (display_name/avatar) is live-resolved on
/// every message the account authored, so it alters `author_display` /
/// `author_avatar_id` on shared messages other members can see — patch_account
/// broadcasts `lists_changed` so every connected client refetches (id-only).
#[tokio::test]
async fn account_profile_change_broadcasts_lists_changed_to_other_members() {
    let a = common::arena().await;
    let (owner, gid, _cid) = owner_with_channel(&a.router, "ProfileOwner").await;

    // A second member shares the guild (and thus sees the owner's messages).
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

    // The member subscribes, THEN the owner renames their account.
    let (status, _h, mut member_body) = common::open_sse(&a.router, "/events", Some(&member)).await;
    assert_eq!(status, StatusCode::OK);

    let (st, _, _) = common::send(
        &a.router,
        Method::PATCH,
        "/account",
        Some(&owner),
        Some(&json!({ "display_name": "Renamed" })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let ev = match common::next_sse_data(&mut member_body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        other => panic!("expected a lists_changed frame, got {other:?}"),
    };
    assert_eq!(ev["type"], "lists_changed");
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

/// M1.5: `mark-read` nudges the SAME account's OTHER devices
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

/// M1.5: every friend mutation (request, accept, remove) reaches EXACTLY the
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

/// C3 (bug hunt 019ef87b): sharing/revoking a persona editor reaches EXACTLY
/// the owner + the editor as `personas_changed`, so an already-mounted
/// recipient session refetches GET /personas instead of showing a stale
/// wardrobe + orbit-station grid. A third account stays silent. Mirrors the
/// friends_changed targeting test.
#[tokio::test]
async fn persona_editor_changes_reach_owner_and_editor_over_sse() {
    let a = common::arena().await;
    let alice = common::register_account(&a.router, "PersonaAlice", "password123").await;
    let (st, _, me) = common::send(&a.router, Method::GET, "/auth/me", Some(&alice), None).await;
    assert_eq!(st, StatusCode::OK);
    let alice_id = me["account_id"].as_str().unwrap().to_string();
    let bob = common::register_account(&a.router, "PersonaBob", "password123").await;
    let (st, _, me) = common::send(&a.router, Method::GET, "/auth/me", Some(&bob), None).await;
    assert_eq!(st, StatusCode::OK);
    let bob_id = me["account_id"].as_str().unwrap().to_string();
    // A bystander with their own guild (non-empty visible set) to prove the
    // stream stays alive afterwards.
    let (carol, _gid, carol_cid) = owner_with_channel(&a.router, "PersonaCarol").await;

    // add_editor requires an accepted friendship — establish it BEFORE opening
    // the streams so no friends_changed frames pollute the personas assertions.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&alice),
        Some(&json!({ "username": "PersonaBob" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/friends/{alice_id}/accept"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // Alice owns a persona (her own create emits nothing to other accounts).
    let (st, _, body) = common::send(
        &a.router,
        Method::POST,
        "/personas",
        Some(&alice),
        Some(&json!({ "name": "Shared One", "description": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let pid = body["id"].as_str().unwrap().to_string();

    let (st, _h, mut alice_body) = common::open_sse(&a.router, "/events", Some(&alice)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _h, mut bob_body) = common::open_sse(&a.router, "/events", Some(&bob)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _h, mut carol_body) = common::open_sse(&a.router, "/events", Some(&carol)).await;
    assert_eq!(st, StatusCode::OK);

    async fn expect_personas_changed(body: &mut axum::body::Body, who: &str) {
        match common::next_sse_data(body, Duration::from_secs(3)).await {
            common::SseRead::Data(v) => {
                assert_eq!(v["type"], "personas_changed", "{who}");
            }
            other => panic!("{who} should receive personas_changed, got {other:?}"),
        }
    }

    // Share: Alice grants Bob editor access → both edge accounts get the nudge
    // (the regression under test: Bob's library gained the persona LIVE).
    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/personas/{pid}/editors/{bob_id}"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    expect_personas_changed(&mut alice_body, "alice (share)").await;
    expect_personas_changed(&mut bob_body, "bob (share)").await;

    // Revoke: Alice removes Bob → both sides again.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/personas/{pid}/editors/{bob_id}"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    expect_personas_changed(&mut alice_body, "alice (revoke)").await;
    expect_personas_changed(&mut bob_body, "bob (revoke)").await;

    // C3 completeness (adversarial review): the OWNER deleting a SHARED persona
    // cascade-drops its persona_editor rows, removing it from each editor's
    // GET /personas — the same membership change as a revoke. Re-share, then
    // delete, and both the owner and the (now-former) editor are nudged.
    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/personas/{pid}/editors/{bob_id}"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    expect_personas_changed(&mut alice_body, "alice (re-share)").await;
    expect_personas_changed(&mut bob_body, "bob (re-share)").await;
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/personas/{pid}"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    expect_personas_changed(&mut alice_body, "alice (delete)").await;
    expect_personas_changed(&mut bob_body, "bob (delete)").await;

    // The bystander saw none of it (Timeout, not Closed)…
    match common::next_sse_data(&mut carol_body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        o => panic!("a third account must not see personas_changed, got {o:?}"),
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

/// A/B (review, content propagation): editing a SHARED persona's grid-projected
/// content — name (patch_persona) or avatar (set_avatar) — reaches every
/// co-viewer (owner + editors) as `personas_changed`, so their mounted grid
/// refetches the new name/avatar instead of going stale. The editor set is
/// unchanged here; this is the content twin of the membership test above. A
/// third account stays silent.
#[tokio::test]
async fn editing_a_shared_persona_reaches_co_viewers_over_sse() {
    let a = common::arena().await;
    let alice = common::register_account(&a.router, "EditAlice", "password123").await;
    let (st, _, me) = common::send(&a.router, Method::GET, "/auth/me", Some(&alice), None).await;
    assert_eq!(st, StatusCode::OK);
    let alice_id = me["account_id"].as_str().unwrap().to_string();
    let bob = common::register_account(&a.router, "EditBob", "password123").await;
    let (st, _, me) = common::send(&a.router, Method::GET, "/auth/me", Some(&bob), None).await;
    assert_eq!(st, StatusCode::OK);
    let bob_id = me["account_id"].as_str().unwrap().to_string();
    let (carol, _gid, carol_cid) = owner_with_channel(&a.router, "EditCarol").await;

    // Friend + share Bob as an editor — all BEFORE the streams open, so neither
    // the friends_changed nor the share personas_changed frames pollute the
    // content assertions below.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&alice),
        Some(&json!({ "username": "EditBob" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/friends/{alice_id}/accept"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, body) = common::send(
        &a.router,
        Method::POST,
        "/personas",
        Some(&alice),
        Some(&json!({ "name": "Original", "description": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let pid = body["id"].as_str().unwrap().to_string();
    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/personas/{pid}/editors/{bob_id}"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // A media blob for the avatar step (set_avatar only checks media existence).
    let avatar_media: String =
        a.db.query(
            "CREATE media_blob SET uploader = type::record('account', $owner), \
             mime = 'image/png', size_bytes = 1, storage_path = 'x' \
             RETURN VALUE meta::id(id);",
        )
        .bind(("owner", alice_id.clone()))
        .await
        .and_then(|mut r| r.take::<Vec<String>>(0))
        .expect("create media + take id")
        .into_iter()
        .next()
        .expect("one media id");

    let (st, _h, mut alice_body) = common::open_sse(&a.router, "/events", Some(&alice)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _h, mut bob_body) = common::open_sse(&a.router, "/events", Some(&bob)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _h, mut carol_body) = common::open_sse(&a.router, "/events", Some(&carol)).await;
    assert_eq!(st, StatusCode::OK);

    async fn expect_personas_changed(body: &mut axum::body::Body, who: &str) {
        match common::next_sse_data(body, Duration::from_secs(3)).await {
            common::SseRead::Data(v) => assert_eq!(v["type"], "personas_changed", "{who}"),
            other => panic!("{who} should receive personas_changed, got {other:?}"),
        }
    }

    // A — rename the shared persona: both viewers' grids must refetch.
    let (st, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/personas/{pid}"),
        Some(&alice),
        Some(&json!({ "name": "Renamed" })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    expect_personas_changed(&mut alice_body, "alice (rename)").await;
    expect_personas_changed(&mut bob_body, "bob (rename)").await;

    // B — change the shared persona's avatar: same.
    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/personas/{pid}/avatar"),
        Some(&alice),
        Some(&json!({ "media_id": avatar_media })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    expect_personas_changed(&mut alice_body, "alice (avatar)").await;
    expect_personas_changed(&mut bob_body, "bob (avatar)").await;

    // The bystander saw none of it (Timeout, not Closed)…
    match common::next_sse_data(&mut carol_body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        o => panic!("a third account must not see personas_changed, got {o:?}"),
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

/// M1.5: reordering YOUR guild rail is a per-user preference — the actor's
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

/// Review M-31: creating a guild is observable ONLY by the creator (they are
/// the sole member at birth) — the actor's connections get a TARGETED
/// `lists_changed`, every other connection gets nothing (this used to
/// broadcast globally: N connections × a visibility reload + three client
/// refetches for an event nobody else can observe — same class as rail
/// reorder above).
#[tokio::test]
async fn create_guild_no_longer_broadcasts() {
    let a = common::arena().await;
    let (owner, _gid, _cid) = owner_with_channel(&a.router, "CreateGuildActor").await;
    // Unrelated account with their own guild (non-empty visible set).
    let (other, _other_gid, other_cid) = owner_with_channel(&a.router, "CreateGuildOther").await;

    let (st, _h, mut owner_body) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _h, mut other_body) = common::open_sse(&a.router, "/events", Some(&other)).await;
    assert_eq!(st, StatusCode::OK);

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "Born Quiet" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // The actor (their other devices) gets the refresh nudge…
    let ev = match common::next_sse_data(&mut owner_body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => v,
        o => panic!("the creator should receive lists_changed, got {o:?}"),
    };
    assert_eq!(ev["type"], "lists_changed");

    // …everyone else stays silent (Timeout, not Closed)…
    match common::next_sse_data(&mut other_body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        o => panic!("guild creation must not broadcast, got {o:?}"),
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

// ---------------------------------------------------------------------------
// Mid-stream revocation (reviews M-05, M-07, M-14, M-43)
// ---------------------------------------------------------------------------

/// Fixture for the visibility-revocation family: `owner_name` owns a guild,
/// `member_name` is invited into it AND owns a guild of their own (for the
/// aliveness proofs), and the member holds an open `/events` stream that has
/// already PROVEN delivery from the owner's channel. Returns
/// `(owner_cookie, owner_gid, owner_cid, member_cookie, member_account_id,
/// member_own_cid, member_stream_body)`.
#[allow(clippy::type_complexity)]
async fn member_with_proven_stream(
    router: &axum::Router,
    owner_name: &str,
    member_name: &str,
) -> (
    String,
    String,
    String,
    String,
    String,
    String,
    axum::body::Body,
) {
    let (owner, gid, cid) = owner_with_channel(router, owner_name).await;
    let (member, _member_gid, member_cid) = owner_with_channel(router, member_name).await;
    let (st, _, me) = common::send(router, Method::GET, "/auth/me", Some(&member), None).await;
    assert_eq!(st, StatusCode::OK);
    let member_id = me["account_id"].as_str().unwrap().to_string();

    // Invite BEFORE the stream opens so the connection's INITIAL visible set
    // already includes the owner's channel (no grow-side lists_changed frame
    // left in the stream to confuse the drain steps below).
    let (st, _, _) = common::send(
        router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": member_name })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let (st, _h, mut body) = common::open_sse(router, "/events", Some(&member)).await;
    assert_eq!(st, StatusCode::OK);

    // Pre-revocation delivery proof: silence later cannot be blamed on a
    // stream that never worked.
    let (st, _, _) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "pre-revocation proof" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => {
            assert_eq!(v["type"], "message_created");
            assert_eq!(v["channel_id"], cid.as_str());
        }
        other => panic!("member should receive events before revocation, got {other:?}"),
    }

    (owner, gid, cid, member, member_id, member_cid, body)
}

/// After a revocation, the stream must go SILENT for the revoked guild
/// (Timeout — not Closed, the connection itself stays up) yet still deliver
/// the member's own guild's events (aliveness: the silence is a filter, not a
/// dead stream).
async fn assert_silent_for_guild_but_alive(
    router: &axum::Router,
    body: &mut axum::body::Body,
    owner: &str,
    revoked_cid: &str,
    member: &str,
    member_cid: &str,
) {
    let (st, _, _) = common::send(
        router,
        Method::POST,
        &format!("/channels/{revoked_cid}/messages"),
        Some(owner),
        Some(&json!({ "body": "post-revocation — must not be delivered" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    match common::next_sse_data(body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        other => panic!("a revoked member must go silent for the guild, got {other:?}"),
    }

    let (st, _, _) = common::send(
        router,
        Method::POST,
        &format!("/channels/{member_cid}/messages"),
        Some(member),
        Some(&json!({ "body": "proof of life" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    match common::next_sse_data(body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => {
            assert_eq!(v["type"], "message_created");
            assert_eq!(v["channel_id"], member_cid);
        }
        other => panic!("aliveness event should arrive, got {other:?}"),
    }
}

/// Reviews M-07/M-14 — the SHRINK direction of the per-connection visibility
/// filter: kicking a member must stop their ALREADY-OPEN stream from
/// receiving the guild's channel events.
#[tokio::test]
async fn kicked_member_stops_receiving_channel_events_mid_stream() {
    let a = common::arena().await;
    let (owner, gid, cid, member, member_id, member_cid, mut body) =
        member_with_proven_stream(&a.router, "KickOwner", "KickedAlice").await;

    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}/members/{member_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    // Drain the broadcast lists_changed the kick emits — it is what triggers
    // the per-connection visibility reload.
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => assert_eq!(v["type"], "lists_changed"),
        other => panic!("expected the kick's lists_changed, got {other:?}"),
    }

    assert_silent_for_guild_but_alive(&a.router, &mut body, &owner, &cid, &member, &member_cid)
        .await;
}

/// Reviews M-07/M-14: same shrink direction, but the member revokes
/// THEMSELVES (self-leave is the `aid == caller` arm of the same endpoint).
#[tokio::test]
async fn member_who_leaves_a_guild_stops_receiving_its_events_mid_stream() {
    let a = common::arena().await;
    let (owner, gid, cid, member, member_id, member_cid, mut body) =
        member_with_proven_stream(&a.router, "LeaveOwner", "LeavingAlice").await;

    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}/members/{member_id}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => assert_eq!(v["type"], "lists_changed"),
        other => panic!("expected the leave's lists_changed, got {other:?}"),
    }

    assert_silent_for_guild_but_alive(&a.router, &mut body, &owner, &cid, &member, &member_cid)
        .await;
}

/// Reviews M-07/M-14: guild soft-delete must silence members' open streams
/// for its channels. HTTP can no longer post into the dead guild (soft-delete
/// gates every mutation), so a stray bus emission is simulated directly on
/// the router's `AppState` — this pins that the PER-CONNECTION filter (not
/// merely the absence of emitters) excludes a soft-deleted guild's channels
/// after the reload.
#[tokio::test]
async fn guild_soft_delete_silences_open_member_streams() {
    let a = common::arena().await;
    let (owner, gid, cid, member, _member_id, member_cid, mut body) =
        member_with_proven_stream(&a.router, "SoftDeleteOwner", "SoftDeleteAlice").await;

    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => assert_eq!(v["type"], "lists_changed"),
        other => panic!("expected the delete's lists_changed, got {other:?}"),
    }

    // A stray emission for the dead guild's channel must be filtered out…
    a.state.emit(SyncEvent::MessageCreated {
        channel_id: cid.clone(),
    });
    match common::next_sse_data(&mut body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        other => panic!("a deleted guild's events must be filtered, got {other:?}"),
    }
    // …while the member's own guild still delivers.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{member_cid}/messages"),
        Some(&member),
        Some(&json!({ "body": "proof of life" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => {
            assert_eq!(v["type"], "message_created");
            assert_eq!(v["channel_id"], member_cid.as_str());
        }
        other => panic!("aliveness event should arrive, got {other:?}"),
    }
}

/// Review M-43: the TARGETED-lane `lists_changed` reload guard (review fix
/// d5c0d33). No production path emits a visibility-SHIFTING targeted
/// lists_changed yet (rail reorder targets the actor but shifts nothing), so
/// the future flow it protects is simulated: membership lands via a direct DB
/// write (no broadcast emit — what an invite-accept flow on the targeted lane
/// would look like), then the targeted nudge fires on the router's own
/// `AppState` (see `Arena::state`).
#[tokio::test]
async fn targeted_lists_changed_reloads_the_connections_visibility_set() {
    let a = common::arena().await;
    let (owner, gid, cid) = owner_with_channel(&a.router, "TargetedOwner").await;
    let member = common::register_account(&a.router, "TargetedMember", "password123").await;
    let (st, _, me) = common::send(&a.router, Method::GET, "/auth/me", Some(&member), None).await;
    assert_eq!(st, StatusCode::OK);
    let member_id = me["account_id"].as_str().unwrap().to_string();

    let (st, _h, mut body) = common::open_sse(&a.router, "/events", Some(&member)).await;
    assert_eq!(st, StatusCode::OK);

    // Membership lands WITHOUT any bus emission…
    a.db.query(
        "CREATE guild_member SET
            guild   = type::record('guild', $gid),
            account = type::record('account', $aid),
            role    = 'member';",
    )
    .bind(("gid", gid.clone()))
    .bind(("aid", member_id.clone()))
    .await
    .expect("grant membership")
    .check()
    .expect("grant membership check");

    // …so the connection's visibility snapshot predates the grant: the
    // guild's events are still filtered out.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "pre-nudge — snapshot is stale" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    match common::next_sse_data(&mut body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        other => panic!("the stale snapshot should still filter, got {other:?}"),
    }

    // The targeted nudge must both DELIVER and reload this connection's set…
    a.state
        .emit_for(vec![member_id.clone()], SyncEvent::ListsChanged);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => assert_eq!(v["type"], "lists_changed"),
        other => panic!("expected the targeted lists_changed, got {other:?}"),
    }

    // …after which the guild's events flow to the pre-existing connection.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "post-nudge — visible now" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => {
            assert_eq!(v["type"], "message_created");
            assert_eq!(v["channel_id"], cid.as_str());
        }
        other => panic!("the reloaded set should now deliver, got {other:?}"),
    }
}

/// Review M-05: identity must hold for the LIFETIME of the stream, not just
/// at connect. Logging a session out must END its live `/events` stream
/// instead of letting it keep draining the account's realtime metadata.
#[tokio::test]
async fn logging_out_a_session_ends_its_live_events_stream() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router, "RevokedOwner").await;
    // A second session on the same account — the device that stays logged in
    // and keeps generating events after the first session is revoked.
    let (st, second, _) = common::send(
        &a.router,
        Method::POST,
        "/auth/login",
        None,
        Some(&json!({ "username": "RevokedOwner", "password": "password123" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let second = second.expect("login must set a session cookie");

    let (st, _h, mut body) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(st, StatusCode::OK);

    // Pre-revocation delivery proof.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&second),
        Some(&json!({ "body": "before the logout" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => assert_eq!(v["type"], "message_created"),
        other => panic!("stream should deliver before the logout, got {other:?}"),
    }

    // Revoke the STREAM'S OWN session (the second one stays valid).
    let (st, _, _) =
        common::send(&a.router, Method::POST, "/auth/logout", Some(&owner), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // The next event must not be delivered: the stream ENDS (Closed — a
    // fail-closed kill, so the reconnect re-authenticates and 401s).
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&second),
        Some(&json!({ "body": "after the logout" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Closed => {}
        other => panic!("a logged-out session must not keep a live stream, got {other:?}"),
    }
}

/// Review M-41 follow-up: restoring an ALREADY-LIVE message is a pinned
/// idempotent 204 (tests/soft_delete.rs) — but the no-op must NOT broadcast
/// `message_created` (each spurious frame fans a full open-channel refetch
/// to every member's connection). A REAL restore (soft-deleted → live) must
/// still broadcast: it is how the reappearance reaches open clients.
#[tokio::test]
async fn restore_of_an_already_live_message_does_not_emit_message_created() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router, "RestoreEmitOwner").await;

    // Seed + soft-delete BEFORE subscribing, so neither event is in the stream.
    let (st, _, msg) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "delete me, then bring me back" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let mid = msg["id"].as_str().unwrap().to_string();
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (st, _h, mut body) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(st, StatusCode::OK);

    // A REAL restore transitions the row → message_created must arrive.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages/{mid}/restore"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => {
            assert_eq!(v["type"], "message_created");
            assert_eq!(v["channel_id"], cid.as_str());
        }
        other => panic!("a real restore must broadcast message_created, got {other:?}"),
    }

    // The idempotent re-restore is a 204 no-op → the stream must stay SILENT
    // (Timeout, not Data — and not Closed, the connection itself stays up).
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages/{mid}/restore"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    match common::next_sse_data(&mut body, Duration::from_millis(1200)).await {
        common::SseRead::Timeout => {}
        other => panic!("a no-op restore must not broadcast, got {other:?}"),
    }

    // Aliveness proof: the silence is a withheld emit, not a dead stream.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "proof of life" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => assert_eq!(v["type"], "message_created"),
        other => panic!("aliveness event should arrive, got {other:?}"),
    }
}

/// Review M-05 follow-up: the per-frame gate only runs when an event is
/// DELIVERED — a revoked session on a fully QUIET stream must still die
/// within ~one re-check period (`AppState::sse_recheck_period`, here shrunk
/// to 100ms), not park in `recv()` holding an authenticated stream open
/// indefinitely. The first half also proves the periodic re-check does NOT
/// kill a live session: several quiet periods elapse, then the stream still
/// delivers.
#[tokio::test]
async fn a_quiet_stream_dies_after_revocation_without_any_event() {
    let a = common::arena_with_sse_recheck_period(Duration::from_millis(100)).await;
    let (owner, _gid, cid) = owner_with_channel(&a.router, "QuietOwner").await;

    let (st, _h, mut body) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(st, StatusCode::OK);

    // Several re-check periods pass on a LIVE session — the stream must
    // survive them and still deliver afterwards.
    tokio::time::sleep(Duration::from_millis(350)).await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "still alive after quiet periods" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Data(v) => assert_eq!(v["type"], "message_created"),
        other => panic!("a live session must survive quiet re-checks, got {other:?}"),
    }

    // Revoke the session — and post NOTHING afterwards. With no event to
    // deliver, only the periodic re-check can end the stream (Closed; a
    // Timeout here would mean it is still parked in `recv()`, the exact
    // leak under test).
    let (st, _, _) =
        common::send(&a.router, Method::POST, "/auth/logout", Some(&owner), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    match common::next_sse_data(&mut body, Duration::from_secs(3)).await {
        common::SseRead::Closed => {}
        other => panic!("a quiet stream must die within ~one period of revocation, got {other:?}"),
    }
}

/// Review M-05 follow-up hardening: the quiet-stream re-check is bound to a
/// DEADLINE, not to bus-receive activity. A busy bus whose every event is
/// INVISIBLE to this connection (account B posting in a guild A cannot see)
/// completes `recv()` at a sub-period cadence; a per-receive timer would
/// re-arm on each filtered `continue` and never fire, and the filtered paths
/// never reach the per-frame gate — so revoked-A's stream would live as long
/// as B keeps typing (the M-05 leak in a narrower disguise). The deadline
/// advances only when a re-check actually runs, so A's revoked stream must
/// still die within ~one period despite the traffic.
#[tokio::test]
async fn a_revoked_stream_dies_even_while_invisible_bus_traffic_keeps_arriving() {
    let a = common::arena_with_sse_recheck_period(Duration::from_millis(100)).await;
    let (victim, _gid_a, _cid_a) = owner_with_channel(&a.router, "DeadlineVictim").await;
    let (busy, _gid_b, cid_b) = owner_with_channel(&a.router, "DeadlineNeighbor").await;

    let (st, _h, mut body) = common::open_sse(&a.router, "/events", Some(&victim)).await;
    assert_eq!(st, StatusCode::OK);

    // Revoke the victim's session, then keep the bus warm with events the
    // victim's connection filters out, every ~40ms (well under the 100ms
    // period) for longer than the read window below.
    let (st, _, _) =
        common::send(&a.router, Method::POST, "/auth/logout", Some(&victim), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let driver = {
        let router = a.router.clone();
        tokio::spawn(async move {
            for i in 0..100 {
                let (st, _, _) = common::send(
                    &router,
                    Method::POST,
                    &format!("/channels/{cid_b}/messages"),
                    Some(&busy),
                    Some(&json!({ "body": format!("invisible noise {i}") })),
                )
                .await;
                assert_eq!(st, StatusCode::CREATED);
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
        })
    };

    // Closed within the window — a Timeout means the invisible traffic kept
    // re-arming the re-check and the revoked stream is still parked; a Data
    // frame would be a privacy-filter failure on top.
    let outcome = common::next_sse_data(&mut body, Duration::from_secs(3)).await;
    driver.abort();
    match outcome {
        common::SseRead::Closed => {}
        other => panic!(
            "revoked stream must die on the re-check deadline despite invisible bus traffic, got {other:?}"
        ),
    }
}
