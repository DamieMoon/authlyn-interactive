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

/// Count of live `kind='dm'` channels in the DB (orphan-/duplicate-thread probe).
#[cfg(feature = "ssr")]
async fn live_dm_channel_count(a: &common::Arena) -> usize {
    let mut resp = a
        .db
        .query("SELECT VALUE meta::id(id) FROM channel WHERE kind = 'dm' AND deleted_at = NONE;")
        .await
        .expect("dm channel query")
        .check()
        .expect("dm channel check");
    let ids: Vec<String> = resp.take(0).expect("take dm channel ids");
    ids.len()
}

#[cfg(feature = "ssr")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_one_to_one_creates_converge_on_one_thread() {
    // review H1: the 1:1 dedup must be race-safe. Alice and Bob open the DM at
    // the same instant (the mobile double-tap / both-parties-initiate race) — the
    // dm_pair UNIQUE lock collapses it to ONE thread. A check-then-create read
    // would mint two (disjoint records, nothing for MVCC to arbitrate).
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;

    let alice_members = [bob_id.as_str()];
    let bob_members = [alice_id.as_str()];
    let (r1, r2) = tokio::join!(
        create_dm(&a.router, &alice, &alice_members, None),
        create_dm(&a.router, &bob, &bob_members, None),
    );
    let (st1, dm1) = r1;
    let (st2, dm2) = r2;

    assert!(
        st1.is_success() && st2.is_success(),
        "both concurrent creates succeed: {st1} {dm1:?} / {st2} {dm2:?}"
    );
    assert_eq!(
        dm1["id"], dm2["id"],
        "concurrent creates converge on one thread id"
    );
    assert_eq!(
        list_dms(&a.router, &alice).await.len(),
        1,
        "Alice: one thread"
    );
    assert_eq!(list_dms(&a.router, &bob).await.len(), 1, "Bob: one thread");
    assert_eq!(
        live_dm_channel_count(&a).await,
        1,
        "exactly one DM channel exists — no duplicate, no orphan"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn one_to_one_recreate_after_both_leave_mints_a_new_thread() {
    // review L2: dedup must NOT resurrect a soft-deleted thread. After both
    // members leave (thread soft-deleted, dedup lock released), re-creating the
    // same pair mints a NEW thread (201) — not the dead one.
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;

    let (st1, dm1) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    assert_eq!(st1, StatusCode::CREATED);
    let first = dm1["id"].as_str().unwrap().to_string();

    for who in [&alice, &bob] {
        let (st, _, _) = common::send(
            &a.router,
            Method::DELETE,
            &format!("/dms/{first}/members/me"),
            Some(who),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::NO_CONTENT);
    }

    let (st2, dm2) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    assert_eq!(
        st2,
        StatusCode::CREATED,
        "re-create after both leave mints a new thread, not a 200 dedup"
    );
    assert_ne!(
        dm2["id"].as_str().unwrap(),
        first,
        "the new thread is not the soft-deleted one"
    );
    assert_eq!(
        list_dms(&a.router, &alice).await.len(),
        1,
        "only the new thread is live"
    );
    assert_eq!(
        live_dm_channel_count(&a).await,
        1,
        "the soft-deleted thread is gone from the live set"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn inviting_past_the_member_cap_is_rejected() {
    // review M1: DM_MAX_MEMBERS (16) must hold at invite, not only at create,
    // so a group can't grow unbounded via repeated invites.
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    let (dave, dave_id) = register_with_id(&a.router, "Dave").await;
    let (erin, erin_id) = register_with_id(&a.router, "Erin").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &dave, "Dave").await;
    befriend(&a.router, &alice, &alice_id, &erin, "Erin").await;

    // A group: Alice + Bob + Dave = 3 members.
    let (st, dm) = create_dm(&a.router, &alice, &[&bob_id, &dave_id], Some("Group")).await;
    assert_eq!(st, StatusCode::CREATED, "create group: {dm:?}");
    let tid = dm["id"].as_str().unwrap().to_string();

    // Pad up to the 16-member cap with placeholder members (record links aren't
    // referentially enforced, so fake account ids are fine for the count).
    for i in 0..13 {
        a.db.query(
            "CREATE dm_member SET channel = type::record('channel', $cid),
                account = type::record('account', $a);",
        )
        .bind(("cid", tid.clone()))
        .bind(("a", format!("pad{i}")))
        .await
        .expect("pad transport")
        .check()
        .expect("pad dm_member");
    }

    // At the cap (16) → inviting a 17th (real friend Erin) is rejected.
    let (st, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/dms/{tid}/members"),
        Some(&alice),
        Some(&json!({ "account_id": erin_id })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "inviting past the member cap is rejected: {body:?}"
    );
}

#[cfg(feature = "ssr")]
async fn post_dm(router: &axum::Router, cookie: &str, tid: &str, body: &str) -> StatusCode {
    let (st, _, _) = common::send(
        router,
        Method::POST,
        &format!("/channels/{tid}/messages"),
        Some(cookie),
        Some(&json!({ "body": body })),
    )
    .await;
    st
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn unfriending_locks_the_one_to_one_then_refriending_unlocks_it() {
    // review M2 (owner ruling): unfriending makes the shared 1:1 DM read-only —
    // history stays readable, new posts are server-rejected — and re-friending
    // restores it.
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    let (_, dm) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    let tid = dm["id"].as_str().unwrap().to_string();

    // Both can post while friends.
    assert_eq!(
        post_dm(&a.router, &alice, &tid, "hej").await,
        StatusCode::CREATED
    );
    assert_eq!(
        post_dm(&a.router, &bob, &tid, "hej själv").await,
        StatusCode::CREATED
    );

    // Alice unfriends Bob → the 1:1 locks.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/friends/{bob_id}"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Posting is now rejected for BOTH parties (read-only).
    assert_eq!(
        post_dm(&a.router, &alice, &tid, "still there?").await,
        StatusCode::FORBIDDEN,
        "locked DM rejects the unfriender's posts"
    );
    assert_eq!(
        post_dm(&a.router, &bob, &tid, "hello?").await,
        StatusCode::FORBIDDEN,
        "locked DM rejects the other party's posts"
    );

    // History is still readable, and the thread reports locked=true.
    let (st, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{tid}/messages"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "a locked DM is still readable");
    assert!(
        !list["messages"].as_array().unwrap().is_empty(),
        "the pre-lock history survives"
    );
    let dms = list_dms(&a.router, &alice).await;
    assert_eq!(dms[0]["locked"], json!(true), "thread reports locked=true");

    // Re-friend (Alice requests, Bob accepts) → unlock, posting restored.
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    assert_eq!(
        post_dm(&a.router, &alice, &tid, "we're back").await,
        StatusCode::CREATED,
        "re-friending unlocks the thread"
    );
    let dms = list_dms(&a.router, &alice).await;
    assert_eq!(
        dms[0]["locked"],
        json!(false),
        "thread reports locked=false"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn unfriending_does_not_lock_a_group_thread() {
    // review M2 scope: the lock is 1:1-only (it keys on the dm_pair lock, which
    // groups never have). Unfriending one group member must not freeze the group.
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    let (carol, carol_id) = register_with_id(&a.router, "Carol").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &carol, "Carol").await;

    // A group (Alice + Bob + Carol).
    let (st, dm) = create_dm(&a.router, &alice, &[&bob_id, &carol_id], Some("Group")).await;
    assert_eq!(st, StatusCode::CREATED);
    let tid = dm["id"].as_str().unwrap().to_string();

    // Alice unfriends Bob — they share no 1:1, so there is no pair lock to set.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/friends/{bob_id}"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // The group is unaffected: Bob (still a member) can still post.
    assert_eq!(
        post_dm(&a.router, &bob, &tid, "group lives").await,
        StatusCode::CREATED,
        "an unfriend between two group members must not lock the group"
    );
    let dms = list_dms(&a.router, &bob).await;
    assert_eq!(dms[0]["locked"], json!(false), "group reports locked=false");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn dm_lifecycle_emits_lists_changed_to_members_over_sse() {
    // review M3: the id-only realtime invariant for DMs — create + leave must
    // each deliver a `lists_changed` frame to every affected member's open
    // /events stream (ListsChanged is a bare tag: ids only, no content).
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;

    let (st, _h, mut alice_body) = common::open_sse(&a.router, "/events", Some(&alice)).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _h, mut bob_body) = common::open_sse(&a.router, "/events", Some(&bob)).await;
    assert_eq!(st, StatusCode::OK);

    // Create → both members receive lists_changed.
    let (st, dm) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    assert_eq!(st, StatusCode::CREATED);
    let tid = dm["id"].as_str().unwrap().to_string();
    for (who, body) in [("alice", &mut alice_body), ("bob", &mut bob_body)] {
        match common::next_sse_data(body, std::time::Duration::from_secs(3)).await {
            common::SseRead::Data(v) => assert_eq!(v["type"], "lists_changed", "{who} create"),
            other => panic!("{who} should receive lists_changed on create, got {other:?}"),
        }
    }

    // Leave (Alice) → both remaining + leaver receive lists_changed.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/dms/{tid}/members/me"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    for (who, body) in [("alice", &mut alice_body), ("bob", &mut bob_body)] {
        match common::next_sse_data(body, std::time::Duration::from_secs(3)).await {
            common::SseRead::Data(v) => assert_eq!(v["type"], "lists_changed", "{who} leave"),
            other => panic!("{who} should receive lists_changed on leave, got {other:?}"),
        }
    }
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn dm_privacy_404_body_is_byte_identical_to_the_guild_channel_404() {
    // review L1: an outsider probing a DM thread must get a 404 whose body is
    // byte-identical to the guild-channel 404 — no DM-vs-guild existence oracle.
    // (The existing test pins against a literal; this pins against a LIVE guild
    // 404, so a future drift on either side is caught.)
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    let (_, dm) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    let dm_tid = dm["id"].as_str().unwrap().to_string();

    // A guild + its default channel that the outsider is NOT a member of.
    let owner = common::register_account(&a.router, "GuildOwner", "password123").await;
    let (st, _, guild) = common::send(
        &a.router,
        Method::POST,
        "/guilds",
        Some(&owner),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();
    let (_, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    let guild_cid = detail["channels"][0]["id"].as_str().unwrap().to_string();

    let outsider = common::register_account(&a.router, "Outsider", "password123").await;
    let (st_g, _, body_g) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{guild_cid}/messages"),
        Some(&outsider),
        None,
    )
    .await;
    let (st_d, _, body_d) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{dm_tid}/messages"),
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(st_g, StatusCode::NOT_FOUND);
    assert_eq!(st_d, StatusCode::NOT_FOUND);
    assert_eq!(
        body_d, body_g,
        "a DM non-member 404 must be byte-identical to the guild-channel 404"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn dm_mention_pings_a_thread_member_per_reader() {
    // review L3: resolve_mentions' dm_member arm must resolve @ping inside a DM
    // (guild-members-only would resolve to nobody), and stay per-reader.
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    let (carol, carol_id) = register_with_id(&a.router, "Carol").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &carol, "Carol").await;
    let (_, dm) = create_dm(&a.router, &alice, &[&bob_id, &carol_id], Some("Group")).await;
    let tid = dm["id"].as_str().unwrap().to_string();

    // Alice pings Bob in the DM.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{tid}/messages"),
        Some(&alice),
        Some(&json!({ "body": "look @Bob" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let messages_of = |cookie: String, tid: String| {
        let router = a.router.clone();
        async move {
            let (_, _, body) = common::send(
                &router,
                Method::GET,
                &format!("/channels/{tid}/messages"),
                Some(&cookie),
                None,
            )
            .await;
            body["messages"].as_array().unwrap().clone()
        }
    };

    let bob_view = messages_of(bob.clone(), tid.clone()).await;
    assert_eq!(
        bob_view[0]["is_pinged"], true,
        "the mentioned DM member is pinged"
    );
    let carol_view = messages_of(carol.clone(), tid.clone()).await;
    assert_eq!(
        carol_view[0]["is_pinged"], false,
        "a non-mentioned DM member is not pinged"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn dm_push_notification_info_has_no_guild() {
    // review L4: the push projection over a DM message must resolve (the
    // meta::id(channel.guild) guard means a NONE guild is not a 500) and report
    // guild_key = None — which is what steers notify_inner to the dm_member
    // recipient query + the no-"#channel" DM title.
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;
    let (_, dm) = create_dm(&a.router, &alice, &[&bob_id], None).await;
    let tid = dm["id"].as_str().unwrap().to_string();

    let (st, _, msg) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{tid}/messages"),
        Some(&alice),
        Some(&json!({ "body": "psst" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let mid = msg["id"].as_str().unwrap().to_string();

    let info = authlyn_interactive::server::push::load_notification_info(&a.state, &mid)
        .await
        .expect("notification row read (a NONE guild must not error the projection)")
        .expect("the just-posted DM message must resolve");
    assert!(
        info.guild_key.is_none(),
        "a DM message carries no guild — guild_key must be None, got {:?}",
        info.guild_key
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn self_dm_and_self_invite_are_rejected() {
    // review L5: the obvious adversarial inputs for a social-create endpoint.
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;
    befriend(&a.router, &alice, &alice_id, &bob, "Bob").await;

    // members=[self] collapses to empty → 400.
    let (st, _) = create_dm(&a.router, &alice, &[&alice_id], None).await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "a DM with only yourself is rejected"
    );

    // members=[self, friend] dedups the self out → a normal 2-member 1:1.
    let (st, dm) = create_dm(&a.router, &alice, &[&alice_id, &bob_id], None).await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(
        dm["members"].as_array().unwrap().len(),
        2,
        "self is filtered, leaving exactly the 1:1 pair"
    );
    let tid = dm["id"].as_str().unwrap().to_string();

    // Inviting yourself into a thread you're in → 400.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/dms/{tid}/members"),
        Some(&alice),
        Some(&json!({ "account_id": alice_id })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "inviting yourself is rejected");
}
