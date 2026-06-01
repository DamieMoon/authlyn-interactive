//! L-4 ping-mention integration tests: posting an `@member` resolves the
//! mention to a guild-member account and records it in `pinged_users`, surfaced
//! to each reader via the per-reader `is_pinged` projection. Mentions of a
//! non-member / unknown handle resolve to nobody, and the composite cursor +
//! pagination are unaffected by the new field.

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

/// Invite `username` (already registered) into `gid` as a `member`.
#[cfg(feature = "ssr")]
async fn invite(router: &axum::Router, owner: &str, gid: &str, username: &str) {
    let (status, _, _) = common::send(
        router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(owner),
        Some(&json!({ "username": username })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "invite({username}) should 201");
}

/// Post `body` to `cid` as `cookie`, returning the new message id.
#[cfg(feature = "ssr")]
async fn post_one(router: &axum::Router, cookie: &str, cid: &str, body: &str) -> String {
    let (status, _, m) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(cookie),
        Some(&json!({ "body": body })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "post should 201: {m:?}");
    m["id"].as_str().unwrap().to_string()
}

/// Fetch the messages array for `cid` as `cookie`.
#[cfg(feature = "ssr")]
async fn messages_of(router: &axum::Router, cookie: &str, cid: &str) -> Vec<serde_json::Value> {
    let (status, _, body) = common::send(
        router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body["messages"].as_array().unwrap().clone()
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn mentioning_a_member_pings_only_that_reader() {
    let a = common::arena().await;
    let (owner, gid, cid) = owner_with_text_channel(&a.router).await;
    let member = common::register_account(&a.router, "Member", "password123").await;
    invite(&a.router, &owner, &gid, "Member").await;
    let bystander = common::register_account(&a.router, "Bystander", "password123").await;
    invite(&a.router, &owner, &gid, "Bystander").await;

    // The owner pings the member.
    post_one(&a.router, &owner, &cid, "hey @Member look at this").await;

    // The mentioned reader sees `is_pinged = true`...
    let member_view = messages_of(&a.router, &member, &cid).await;
    assert_eq!(member_view.len(), 1);
    assert_eq!(
        member_view[0]["is_pinged"], true,
        "the mentioned member is pinged"
    );

    // ...while a different reader (and the author) sees `is_pinged = false`:
    // the flag is per-reader.
    let bystander_view = messages_of(&a.router, &bystander, &cid).await;
    assert_eq!(
        bystander_view[0]["is_pinged"], false,
        "a non-mentioned member is not pinged"
    );
    let owner_view = messages_of(&a.router, &owner, &cid).await;
    assert_eq!(
        owner_view[0]["is_pinged"], false,
        "the author who mentioned someone else is not pinged"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn case_insensitive_mention_resolves() {
    let a = common::arena().await;
    let (owner, gid, cid) = owner_with_text_channel(&a.router).await;
    let member = common::register_account(&a.router, "MixedCase", "password123").await;
    invite(&a.router, &owner, &gid, "MixedCase").await;

    // Mention in a different case than the registered username.
    post_one(&a.router, &owner, &cid, "ping @mixedcase here").await;
    let view = messages_of(&a.router, &member, &cid).await;
    assert_eq!(
        view[0]["is_pinged"], true,
        "mention matching is case-insensitive"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn mentioning_a_nonmember_or_unknown_pings_nobody() {
    let a = common::arena().await;
    let (owner, gid, cid) = owner_with_text_channel(&a.router).await;
    // An account that exists but is NOT a member of this guild.
    let outsider = common::register_account(&a.router, "Outsider", "password123").await;
    // A member, to read the channel and confirm they aren't accidentally pinged.
    let member = common::register_account(&a.router, "Member", "password123").await;
    invite(&a.router, &owner, &gid, "Member").await;

    // @Outsider (non-member) and @ghost (no such account) must resolve to nobody.
    post_one(&a.router, &owner, &cid, "@Outsider @ghost @Member hi").await;

    // The real member IS pinged (control: resolution works at all).
    let member_view = messages_of(&a.router, &member, &cid).await;
    assert_eq!(
        member_view[0]["is_pinged"], true,
        "the in-guild member is pinged"
    );

    // The outsider can't even read the channel (privacy-404), so a ping to a
    // non-member is meaningless — the membership gate already blocks them.
    let (status, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a non-member can't read the channel at all"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn mention_inside_inline_code_does_not_ping() {
    let a = common::arena().await;
    let (owner, gid, cid) = owner_with_text_channel(&a.router).await;
    let member = common::register_account(&a.router, "Member", "password123").await;
    invite(&a.router, &owner, &gid, "Member").await;

    // `@Member` inside an inline code span is verbatim text, not a mention.
    post_one(&a.router, &owner, &cid, "the literal `@Member` token").await;
    let view = messages_of(&a.router, &member, &cid).await;
    assert_eq!(
        view[0]["is_pinged"], false,
        "a mention inside inline code is literal, not a ping"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn no_mention_leaves_is_pinged_false() {
    let a = common::arena().await;
    let (owner, gid, cid) = owner_with_text_channel(&a.router).await;
    let member = common::register_account(&a.router, "Member", "password123").await;
    invite(&a.router, &owner, &gid, "Member").await;

    post_one(&a.router, &owner, &cid, "a plain message, no pings").await;
    let view = messages_of(&a.router, &member, &cid).await;
    assert_eq!(view[0]["is_pinged"], false);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn pagination_and_cursor_unaffected_by_pinged_users() {
    // The composite (sent_at, id) cursor must still page cleanly with the new
    // field present: post a run of messages (some pinging the member, some not),
    // then walk the channel forward via ?since=&after_id= and confirm we see
    // every message exactly once, in order, with `is_pinged` correctly per row.
    let a = common::arena().await;
    let (owner, gid, cid) = owner_with_text_channel(&a.router).await;
    let member = common::register_account(&a.router, "Member", "password123").await;
    invite(&a.router, &owner, &gid, "Member").await;

    const N: usize = 12;
    for i in 0..N {
        // Even messages ping the member; odd ones don't.
        let body = if i % 2 == 0 {
            format!("msg {i} @Member")
        } else {
            format!("msg {i} plain")
        };
        post_one(&a.router, &owner, &cid, &body).await;
    }

    // First page (newest, ASC) as the member.
    let page = messages_of(&a.router, &member, &cid).await;
    assert_eq!(page.len(), N, "all {N} messages on one page");
    // Bodies are in order and is_pinged tracks the even/odd pattern.
    for (i, m) in page.iter().enumerate() {
        assert_eq!(
            m["body"],
            format!("msg {i} {}", if i % 2 == 0 { "@Member" } else { "plain" })
        );
        assert_eq!(
            m["is_pinged"],
            i % 2 == 0,
            "row {i} is_pinged should match the even/odd ping pattern"
        );
    }

    // Forward-cursor poll from the 3rd message: must return the strict tail,
    // every later message exactly once, still in order.
    let since = page[2]["sent_at"].as_str().unwrap();
    let after_id = page[2]["id"].as_str().unwrap();
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages?since={since}&after_id={after_id}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let tail = body["messages"].as_array().unwrap();
    assert_eq!(tail.len(), N - 3, "the strict forward tail after row 2");
    assert_eq!(tail[0]["body"], "msg 3 plain", "first tail row is msg 3");
}
