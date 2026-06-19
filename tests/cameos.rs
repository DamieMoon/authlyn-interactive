//! M7/P2 integration tests: Guest Cameos. A cameo is a `channel_guest` row that
//! grants an accepted friend scoped, ephemeral read+post access to ONE guild text
//! channel. These pin the lifecycle (`/channels/{cid}/guests`, `/cameos`), the
//! friend-gate, the send-time guest badge (and that it survives revoke), the
//! single-channel privacy-404 (a guest must NOT see sibling channels or the
//! guild), the expiry lazy-check, the unfriend-revoke, and that the inherited
//! message / mention / unread / persona stack works for a guest.

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

/// A guild owned by "Host" with its default text channel `cid` plus a second
/// text channel `cid2`, and a separate "Guest" account befriended with the host
/// but NOT a guild member. Returns
/// `(host_cookie, host_id, gid, cid, cid2, guest_cookie, guest_id)`.
#[cfg(feature = "ssr")]
async fn setup(router: &axum::Router) -> (String, String, String, String, String, String, String) {
    let (host, host_id) = register_with_id(router, "Host").await;
    let (st, _, guild) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(&host),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();

    let (_, _, detail) = common::send(
        router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&host),
        None,
    )
    .await;
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();

    let (st, _, ch2) = common::send(
        router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&host),
        Some(&json!({ "name": "side", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let cid2 = ch2["id"].as_str().unwrap().to_string();

    let (guest, guest_id) = register_with_id(router, "Guest").await;
    befriend(router, &host, &host_id, &guest, "Guest").await;
    (host, host_id, gid, cid, cid2, guest, guest_id)
}

#[cfg(feature = "ssr")]
async fn invite(
    router: &axum::Router,
    cookie: &str,
    cid: &str,
    account_id: &str,
    expires_at: Option<&str>,
) -> (StatusCode, Value) {
    let mut body = json!({ "account_id": account_id });
    if let Some(e) = expires_at {
        body["expires_at"] = json!(e);
    }
    let (st, _, b) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/guests"),
        Some(cookie),
        Some(&body),
    )
    .await;
    (st, b)
}

#[cfg(feature = "ssr")]
async fn post_msg(
    router: &axum::Router,
    cookie: &str,
    cid: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let (st, _, b) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(cookie),
        Some(body),
    )
    .await;
    (st, b)
}

#[cfg(feature = "ssr")]
async fn messages(router: &axum::Router, cookie: &str, cid: &str) -> (StatusCode, Vec<Value>) {
    let (st, _, b) = common::send(
        router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(cookie),
        None,
    )
    .await;
    let msgs = b["messages"].as_array().cloned().unwrap_or_default();
    (st, msgs)
}

#[cfg(feature = "ssr")]
async fn list_cameos(router: &axum::Router, cookie: &str) -> Vec<Value> {
    let (st, _, b) = common::send(router, Method::GET, "/cameos", Some(cookie), None).await;
    assert_eq!(st, StatusCode::OK);
    b["cameos"].as_array().cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn inviting_a_friend_grants_scoped_access_and_badges_the_guest_message() {
    let a = common::arena().await;
    let (host, host_id, _gid, cid, _cid2, guest, guest_id) = setup(&a.router).await;

    let (st, g) = invite(&a.router, &host, &cid, &guest_id, None).await;
    assert_eq!(st, StatusCode::CREATED, "invite: {g:?}");
    assert_eq!(g["account_id"], guest_id);
    assert_eq!(g["invited_by"], host_id);

    // The guest can read AND post in the cameo channel.
    let (st, _) = messages(&a.router, &guest, &cid).await;
    assert_eq!(st, StatusCode::OK, "guest can read the cameo channel");
    let (st, _) = post_msg(
        &a.router,
        &guest,
        &cid,
        &json!({ "body": "hej från gästen" }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "guest can post");
    let (st, _) = post_msg(
        &a.router,
        &host,
        &cid,
        &json!({ "body": "hej från värden" }),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // The host reads both: the guest's message is badged, the host's is not.
    let (_, msgs) = messages(&a.router, &host, &cid).await;
    let guest_msg = msgs
        .iter()
        .find(|m| m["body"] == "hej från gästen")
        .unwrap();
    let host_msg = msgs
        .iter()
        .find(|m| m["body"] == "hej från värden")
        .unwrap();
    assert_eq!(
        guest_msg["guest_cameo"],
        json!(true),
        "guest message is badged"
    );
    assert_eq!(
        host_msg["guest_cameo"],
        json!(false),
        "host message is not badged"
    );

    // The cameo shows up in the guest's /cameos with the host guild's name.
    let cameos = list_cameos(&a.router, &guest).await;
    assert_eq!(cameos.len(), 1, "one active cameo");
    assert_eq!(cameos[0]["channel_id"], cid);
    assert_eq!(cameos[0]["guild_name"], "Guild");
    assert_eq!(cameos[0]["invited_by"], host_id);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn inviting_a_non_friend_is_rejected() {
    let a = common::arena().await;
    let (host, _host_id, _gid, cid, _cid2, _guest, _guest_id) = setup(&a.router).await;
    let (_stranger, stranger_id) = register_with_id(&a.router, "Stranger").await;

    let (st, _) = invite(&a.router, &host, &cid, &stranger_id, None).await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "inviting a non-friend is forbidden"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn inviting_an_existing_guild_member_is_rejected() {
    let a = common::arena().await;
    let (host, host_id, gid, cid, _cid2, _guest, _guest_id) = setup(&a.router).await;
    // A second guild member, also the host's friend.
    let (member, member_id) = register_with_id(&a.router, "Member").await;
    befriend(&a.router, &host, &host_id, &member, "Member").await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&host),
        Some(&json!({ "username": "Member" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let _ = member;

    let (st, _) = invite(&a.router, &host, &cid, &member_id, None).await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "an existing member can't be a guest"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn a_guest_is_confined_to_the_one_channel() {
    let a = common::arena().await;
    let (host, _host_id, gid, cid, cid2, guest, guest_id) = setup(&a.router).await;
    invite(&a.router, &host, &cid, &guest_id, None).await;

    // The cameo channel works...
    let (st, _) = messages(&a.router, &guest, &cid).await;
    assert_eq!(st, StatusCode::OK);

    // ...but a SIBLING channel of the same guild is a privacy-404, byte-identical
    // to a guild non-member's 404 (no sibling-channel existence oracle).
    let outsider = common::register_account(&a.router, "Outsider", "password123").await;
    let (st_g, _, body_g) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid2}/messages"),
        Some(&outsider),
        None,
    )
    .await;
    let (st_guest, _, body_guest) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid2}/messages"),
        Some(&guest),
        None,
    )
    .await;
    assert_eq!(st_g, StatusCode::NOT_FOUND);
    assert_eq!(
        st_guest,
        StatusCode::NOT_FOUND,
        "guest can't see a sibling channel"
    );
    assert_eq!(
        body_guest, body_g,
        "sibling-channel 404 is byte-identical to a non-member's"
    );

    // The guest can't see the guild itself, and can't manage it.
    let (st, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&guest),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "guest can't open the host guild");
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&guest),
        Some(&json!({ "name": "sneaky", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "guest can't manage the guild");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn revoking_a_guest_kills_access_but_keeps_the_badged_history() {
    let a = common::arena().await;
    let (host, _host_id, _gid, cid, _cid2, guest, guest_id) = setup(&a.router).await;
    invite(&a.router, &host, &cid, &guest_id, None).await;
    post_msg(&a.router, &guest, &cid, &json!({ "body": "gästinlägg" })).await;

    // Host revokes the cameo.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/guests/{guest_id}"),
        Some(&host),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "host revokes the guest");

    // The guest loses access, and /cameos is empty.
    let (st, _) = messages(&a.router, &guest, &cid).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "revoked guest is locked out");
    assert!(
        list_cameos(&a.router, &guest).await.is_empty(),
        "no active cameo"
    );

    // The guest's past message stays, still badged (the snapshot survives revoke).
    let (_, msgs) = messages(&a.router, &host, &cid).await;
    let guest_msg = msgs.iter().find(|m| m["body"] == "gästinlägg").unwrap();
    assert_eq!(
        guest_msg["guest_cameo"],
        json!(true),
        "badge survives revoke"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn an_expired_cameo_denies_access_while_a_future_one_grants_it() {
    let a = common::arena().await;
    let (host, _host_id, _gid, cid, cid2, guest, guest_id) = setup(&a.router).await;

    // A cameo that expired in the past: the lazy-check denies access immediately.
    let (st, _) = invite(
        &a.router,
        &host,
        &cid,
        &guest_id,
        Some("2000-01-01T00:00:00Z"),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::CREATED,
        "invite with a past expiry is accepted"
    );
    let (st, _) = messages(&a.router, &guest, &cid).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "an expired cameo denies access");
    assert!(
        list_cameos(&a.router, &guest).await.is_empty(),
        "an expired cameo is not listed"
    );

    // A cameo expiring far in the future grants access.
    let (st, _) = invite(
        &a.router,
        &host,
        &cid2,
        &guest_id,
        Some("2999-01-01T00:00:00Z"),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _) = messages(&a.router, &guest, &cid2).await;
    assert_eq!(st, StatusCode::OK, "a future-expiry cameo grants access");
    assert_eq!(
        list_cameos(&a.router, &guest).await.len(),
        1,
        "only the live one lists"
    );

    // A malformed expiry is a clean 400.
    let (st, _) = invite(&a.router, &host, &cid, &guest_id, Some("not-a-date")).await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "a malformed expiry is rejected"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn unfriending_revokes_only_the_cameo_from_that_inviter() {
    let a = common::arena().await;
    let (host, host_id, gid, cid, cid2, guest, guest_id) = setup(&a.router).await;
    // A second guild member who is ALSO the guest's friend, inviting to cid2.
    let (host2, host2_id) = register_with_id(&a.router, "Host2").await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&host),
        Some(&json!({ "username": "Host2" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    befriend(&a.router, &host2, &host2_id, &guest, "Guest").await;

    invite(&a.router, &host, &cid, &guest_id, None).await; // host → cid
    invite(&a.router, &host2, &cid2, &guest_id, None).await; // host2 → cid2
    assert_eq!(list_cameos(&a.router, &guest).await.len(), 2, "two cameos");

    // Host unfriends the guest → only the host's cameo (cid) is revoked.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/friends/{guest_id}"),
        Some(&host),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let _ = host_id;

    let (st, _) = messages(&a.router, &guest, &cid).await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "host's cameo is revoked on unfriend"
    );
    let (st, _) = messages(&a.router, &guest, &cid2).await;
    assert_eq!(
        st,
        StatusCode::OK,
        "host2's cameo (different inviter) is untouched"
    );
    let cameos = list_cameos(&a.router, &guest).await;
    assert_eq!(cameos.len(), 1, "exactly the surviving cameo");
    assert_eq!(cameos[0]["channel_id"], cid2);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn a_guest_wears_their_own_persona() {
    let a = common::arena().await;
    let (host, _host_id, _gid, cid, _cid2, guest, guest_id) = setup(&a.router).await;
    invite(&a.router, &host, &cid, &guest_id, None).await;

    // The guest's OWN persona (spec §9.7: "own persona").
    let (st, _, persona) = common::send(
        &a.router,
        Method::POST,
        "/personas",
        Some(&guest),
        Some(&json!({ "name": "Gästhjälten", "description": "en gäst" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let pid = persona["id"].as_str().unwrap();

    let (st, _) = post_msg(
        &a.router,
        &guest,
        &cid,
        &json!({ "body": "i roll", "persona_id": pid }),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::CREATED,
        "guest posts wearing their own persona"
    );

    let (_, msgs) = messages(&a.router, &host, &cid).await;
    let m = msgs.iter().find(|m| m["body"] == "i roll").unwrap();
    assert_eq!(
        m["persona_name"], "Gästhjälten",
        "the guest's own persona snapshots"
    );
    assert_eq!(
        m["guest_cameo"],
        json!(true),
        "and it's still a guest message"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn guest_and_member_mention_each_other() {
    let a = common::arena().await;
    let (host, _host_id, _gid, cid, _cid2, guest, guest_id) = setup(&a.router).await;
    invite(&a.router, &host, &cid, &guest_id, None).await;

    // The guest @-mentions the host (a guild member): the host reads it as a ping.
    post_msg(&a.router, &guest, &cid, &json!({ "body": "@Host hallå" })).await;
    let (_, host_view) = messages(&a.router, &host, &cid).await;
    let to_host = host_view
        .iter()
        .find(|m| m["body"] == "@Host hallå")
        .unwrap();
    assert_eq!(to_host["is_pinged"], json!(true), "guest can ping a member");

    // The host @-mentions the guest: the guest reads it as a ping.
    post_msg(
        &a.router,
        &host,
        &cid,
        &json!({ "body": "@Guest välkommen" }),
    )
    .await;
    let (_, guest_view) = messages(&a.router, &guest, &cid).await;
    let to_guest = guest_view
        .iter()
        .find(|m| m["body"] == "@Guest välkommen")
        .unwrap();
    assert_eq!(
        to_guest["is_pinged"],
        json!(true),
        "a member can ping the guest"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn a_cameo_unread_carries_a_null_guild() {
    let a = common::arena().await;
    let (host, _host_id, _gid, cid, _cid2, guest, guest_id) = setup(&a.router).await;
    invite(&a.router, &host, &cid, &guest_id, None).await;
    // The guest baselines the channel, then the host posts something new.
    messages(&a.router, &guest, &cid).await;
    post_msg(&a.router, &host, &cid, &json!({ "body": "nytt" })).await;

    let (st, _, body) = common::send(&a.router, Method::GET, "/unread", Some(&guest), None).await;
    assert_eq!(st, StatusCode::OK);
    let row = body["channels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["channel_id"] == cid)
        .expect("the cameo channel is visible in /unread");
    assert!(
        row["guild_id"].is_null(),
        "a cameo seen as a guest carries no guild"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn leaving_a_cameo_removes_it() {
    let a = common::arena().await;
    let (host, _host_id, _gid, cid, _cid2, guest, guest_id) = setup(&a.router).await;
    invite(&a.router, &host, &cid, &guest_id, None).await;
    assert_eq!(list_cameos(&a.router, &guest).await.len(), 1);

    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/guests/me"),
        Some(&guest),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "the guest leaves their cameo");
    assert!(list_cameos(&a.router, &guest).await.is_empty());
    let (st, _) = messages(&a.router, &guest, &cid).await;
    assert_eq!(st, StatusCode::NOT_FOUND, "access is gone after leaving");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nonmembers_cannot_invite_or_revoke() {
    let a = common::arena().await;
    let (host, _host_id, _gid, cid, _cid2, _guest, guest_id) = setup(&a.router).await;
    invite(&a.router, &host, &cid, &guest_id, None).await;
    let outsider = common::register_account(&a.router, "Outsider", "password123").await;
    let not_found = json!({ "error": "channel not found" });

    // A non-member inviting into the channel → privacy-404.
    let (st, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/guests"),
        Some(&outsider),
        Some(&json!({ "account_id": guest_id })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(body, not_found, "non-member invite is a byte-identical 404");

    // A non-member revoking the guest → privacy-404.
    let (st, _, body) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/guests/{guest_id}"),
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(body, not_found);

    // A non-member listing the guests → privacy-404.
    let (st, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/guests"),
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn soft_deleting_the_host_guild_drops_the_cameo_from_the_guest_list() {
    // M7/P2 review C1: /cameos must apply the same guild-soft-delete filter every
    // access path enforces, or a dead cameo lingers (clickable, 404-on-open) for up
    // to the 30d purge window.
    let a = common::arena().await;
    let (host, _host_id, gid, cid, _cid2, guest, guest_id) = setup(&a.router).await;
    invite(&a.router, &host, &cid, &guest_id, None).await;
    assert_eq!(
        list_cameos(&a.router, &guest).await.len(),
        1,
        "live cameo lists"
    );

    // The host (owner) soft-deletes the guild.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}"),
        Some(&host),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "owner soft-deletes the guild");

    // The cameo vanishes from /cameos AND opening the channel is a privacy-404.
    assert!(
        list_cameos(&a.router, &guest).await.is_empty(),
        "a cameo in a soft-deleted guild is not listed"
    );
    let (st, _) = messages(&a.router, &guest, &cid).await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "and the channel is no longer accessible"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn cameo_lifecycle_emits_lists_changed_to_the_guest_over_sse() {
    // The id-only realtime invariant for cameos: invite + revoke each deliver a
    // `lists_changed` frame to the affected guest's open /events stream.
    let a = common::arena().await;
    let (host, _host_id, _gid, cid, _cid2, guest, guest_id) = setup(&a.router).await;

    let (st, _h, mut guest_body) = common::open_sse(&a.router, "/events", Some(&guest)).await;
    assert_eq!(st, StatusCode::OK);

    let (st, _) = invite(&a.router, &host, &cid, &guest_id, None).await;
    assert_eq!(st, StatusCode::CREATED);
    match common::next_sse_data(&mut guest_body, std::time::Duration::from_secs(3)).await {
        common::SseRead::Data(v) => assert_eq!(v["type"], "lists_changed", "invite"),
        other => panic!("guest should receive lists_changed on invite, got {other:?}"),
    }

    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/guests/{guest_id}"),
        Some(&host),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    match common::next_sse_data(&mut guest_body, std::time::Duration::from_secs(3)).await {
        common::SseRead::Data(v) => assert_eq!(v["type"], "lists_changed", "revoke"),
        other => panic!("guest should receive lists_changed on revoke, got {other:?}"),
    }
}
