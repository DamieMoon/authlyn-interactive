//! M7/P1 integration tests: direct messages (1:1 + groups). DM threads are
//! channels with `kind='dm'` and no guild; these pin the lifecycle (`/dms`),
//! the friend-gate, 1:1 dedup, leave + last-member soft-delete, the privacy-404
//! parity for non-members, and that the inherited channel-scoped message /
//! unread / persona stack works on a DM thread.

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::{json, Value};

#[cfg(feature = "ssr")]
async fn register_with_id(router: &axum::Router, name: &str) -> (String, String) {
    let (status, cookie, body) = common::send(
        router,
        Method::POST,
        "/auth/register",
        None,
        Some(&json!({ "username": name, "password": "password123" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "register({name}): {body:?}");
    (
        cookie.unwrap(),
        body["account_id"].as_str().unwrap().to_string(),
    )
}

/// `requester` sends a friend request to `addressee_name`, then `addressee`
/// accepts `requester_id` — leaving an accepted friendship between the two.
#[cfg(feature = "ssr")]
async fn befriend(
    router: &axum::Router,
    requester_cookie: &str,
    requester_id: &str,
    addressee_cookie: &str,
    addressee_name: &str,
) {
    let (st, _, _) = common::send(
        router,
        Method::POST,
        "/friends",
        Some(requester_cookie),
        Some(&json!({ "username": addressee_name })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "friend request");
    let (st, _, _) = common::send(
        router,
        Method::POST,
        &format!("/friends/{requester_id}/accept"),
        Some(addressee_cookie),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "friend accept");
}

#[cfg(feature = "ssr")]
async fn create_dm(
    router: &axum::Router,
    cookie: &str,
    members: &[&str],
    title: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({ "members": members });
    if let Some(t) = title {
        body["title"] = json!(t);
    }
    let (st, _, b) = common::send(router, Method::POST, "/dms", Some(cookie), Some(&body)).await;
    (st, b)
}

#[cfg(feature = "ssr")]
async fn list_dms(router: &axum::Router, cookie: &str) -> Vec<Value> {
    let (st, _, b) = common::send(router, Method::GET, "/dms", Some(cookie), None).await;
    assert_eq!(st, StatusCode::OK);
    b["dms"].as_array().cloned().unwrap_or_default()
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn create_one_to_one_dm_between_friends() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;

    let (st, dm) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    assert_eq!(st, StatusCode::CREATED, "create 1:1: {dm:?}");
    assert!(dm["id"].as_str().is_some(), "thread has a channel id");
    assert!(dm["title"].is_null(), "1:1 has no title");
    let members = dm["members"].as_array().unwrap();
    assert_eq!(members.len(), 2, "creator + the one other member");

    // Both list it.
    assert_eq!(list_dms(&a.router, &alice).await.len(), 1);
    assert_eq!(list_dms(&a.router, &bob).await.len(), 1);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn one_to_one_dm_is_deduped() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;

    let (st1, dm1) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    assert_eq!(st1, StatusCode::CREATED);
    // A second create for the same pair returns the SAME thread (200, not a dup).
    let (st2, dm2) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    assert_eq!(st2, StatusCode::OK, "dedup returns the existing thread");
    assert_eq!(dm1["id"], dm2["id"], "same channel id");
    // Bob initiating it the other way also dedups to the same thread.
    let (st3, dm3) = create_dm(&a.router, &bob, &[&alice_id], None).await;
    assert_eq!(st3, StatusCode::OK);
    assert_eq!(dm1["id"], dm3["id"]);
    assert_eq!(
        list_dms(&a.router, &alice).await.len(),
        1,
        "still one thread"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn create_group_dm_with_title() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    let (carol, carol_id) = register_with_id(&a.router, "Carol").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &carol, "Carol").await;

    let (st, dm) = create_dm(&a.router, &alice, &[&bob_id, &carol_id], Some("Squad")).await;
    assert_eq!(st, StatusCode::CREATED, "create group: {dm:?}");
    assert_eq!(dm["title"], "Squad");
    assert_eq!(dm["members"].as_array().unwrap().len(), 3);
    // A group is NOT deduped — same members again makes a second thread.
    let (st2, _) = create_dm(&a.router, &alice, &[&bob_id, &carol_id], Some("Squad")).await;
    assert_eq!(st2, StatusCode::CREATED);
    assert_eq!(list_dms(&a.router, &alice).await.len(), 2);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn cannot_dm_a_non_friend() {
    let a = common::arena().await;
    let (alice, _alice_id) = register_with_id(&a.router, "Alice").await;
    let (_bob, bob_id) = register_with_id(&a.router, "Bob").await;
    // No friendship.
    let (st, body) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "non-friend DM rejected: {body:?}"
    );
    assert!(list_dms(&a.router, &alice).await.is_empty());
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn dm_message_round_trips_via_channel_route() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    let (_, dm) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    let tid = dm["id"].as_str().unwrap();

    // Alice posts through the inherited channel-scoped route.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{tid}/messages"),
        Some(&alice),
        Some(&json!({ "body": "hej Bob" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "DM message accepted");

    // Bob (the other member) reads it.
    let (st, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{tid}/messages"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let msgs = body["messages"].as_array().unwrap();
    assert!(msgs.iter().any(|m| m["body"] == "hej Bob"), "Bob sees it");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nonmember_dm_access_is_privacy_404() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    let (carol, carol_id) = register_with_id(&a.router, "Carol").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &carol, "Carol").await;
    let (_, dm) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    let tid = dm["id"].as_str().unwrap();

    // The privacy-404 body all non-membership probes collapse to.
    let not_found = json!({ "error": "channel not found" });

    // Carol is not a member: reading messages 404s with the identical body.
    let (st, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{tid}/messages"),
        Some(&carol),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(body, not_found, "byte-identical privacy-404");

    // Inviting / leaving a thread she's not in is the same 404 (no existence leak).
    let (st, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/dms/{tid}/members"),
        Some(&carol),
        Some(&json!({ "account_id": carol_id })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(body, not_found);

    let (st, _, body) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/dms/{tid}/members/me"),
        Some(&carol),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(body, not_found);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn invite_friend_to_group_then_they_can_post() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    let (carol, carol_id) = register_with_id(&a.router, "Carol").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &carol, "Carol").await;
    let (_, dm) = create_dm(&a.router, &alice, &[&bob_id], Some("Group")).await;
    let tid = dm["id"].as_str().unwrap();

    // Carol can't see it before being invited.
    let (st, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{tid}/messages"),
        Some(&carol),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // Alice (a member, Carol's friend) invites Carol.
    let (st, _, dm) = common::send(
        &a.router,
        Method::POST,
        &format!("/dms/{tid}/members"),
        Some(&alice),
        Some(&json!({ "account_id": carol_id })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "invite accepted: {dm:?}");
    assert_eq!(dm["members"].as_array().unwrap().len(), 3);

    // Now Carol can post.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{tid}/messages"),
        Some(&carol),
        Some(&json!({ "body": "hi all" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(list_dms(&a.router, &carol).await.len(), 1);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn cannot_invite_a_non_friend_of_the_inviter() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    let (_dave, dave_id) = register_with_id(&a.router, "Dave").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    let (_, dm) = create_dm(&a.router, &alice, &[&bob_id], Some("Group")).await;
    let tid = dm["id"].as_str().unwrap();

    // Dave is not Alice's friend → invite forbidden.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/dms/{tid}/members"),
        Some(&alice),
        Some(&json!({ "account_id": dave_id })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn leaving_drops_you_and_last_member_soft_deletes_the_thread() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    let (_, dm) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    let tid = dm["id"].as_str().unwrap().to_string();

    // Alice leaves: gone from her list, still on Bob's.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/dms/{tid}/members/me"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(list_dms(&a.router, &alice).await.is_empty(), "Alice left");
    assert_eq!(list_dms(&a.router, &bob).await.len(), 1, "Bob still has it");

    // Bob leaves (last member) → thread soft-deleted: gone from his list, and
    // message access now 404s (the channel is no longer live).
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/dms/{tid}/members/me"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(list_dms(&a.router, &bob).await.is_empty(), "Bob left too");
    let (st, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{tid}/messages"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "soft-deleted thread is gone");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn dm_unread_appears_with_null_guild() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    let (_, dm) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    let tid = dm["id"].as_str().unwrap().to_string();

    // Bob posts; Alice sees the DM in /unread with guild_id null.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{tid}/messages"),
        Some(&bob),
        Some(&json!({ "body": "ping" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let (st, _, body) = common::send(&a.router, Method::GET, "/unread", Some(&alice), None).await;
    assert_eq!(st, StatusCode::OK);
    let row = body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["channel_id"] == tid)
        .expect("DM thread is a visible channel in /unread");
    assert!(row["guild_id"].is_null(), "DM unread row has no guild");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn persona_wear_works_per_dm_channel() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    let (_, dm) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    let tid = dm["id"].as_str().unwrap().to_string();

    // Alice owns a persona and wears it in the DM (the channel-scoped route a
    // DM member is gated for via resolve_membership).
    let (st, _, persona) = common::send(
        &a.router,
        Method::POST,
        "/personas",
        Some(&alice),
        Some(&json!({ "name": "Shadow" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let pid = persona["id"].as_str().unwrap();
    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/channels/{tid}/active-persona"),
        Some(&alice),
        Some(&json!({ "persona_id": pid })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NO_CONTENT,
        "wearing a persona in a DM works"
    );

    // A non-member cannot wear a persona there (privacy-404).
    let (carol, _carol_id) = register_with_id(&a.router, "Carol").await;
    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/channels/{tid}/active-persona"),
        Some(&carol),
        Some(&json!({ "persona_id": pid })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}
