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

/// M6/P2 identity invariant: ACCOUNT identity (display_name + avatar) resolves
/// LIVE at read on every message — a rename / new avatar reflects even on OLD
/// rows — while a worn persona's name stays SNAPSHOTTED at send. Pins the new
/// live `author_avatar_id` projection on both a bare and a persona-worn message.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn account_identity_resolves_live_while_persona_name_stays_frozen() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    // A persona, then two messages: one wearing it (snapshots persona_name) and
    // one bare (account identity only).
    let (status, _, persona) = common::send(
        &a.router,
        Method::POST,
        "/personas",
        Some(&owner),
        Some(&json!({ "name": "Hero" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let pid = persona["id"].as_str().unwrap().to_string();

    let (status, _, m) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "in character", "persona_id": pid })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let in_char = m["id"].as_str().unwrap().to_string();
    let bare = post_one(&a.router, &owner, &cid, "bare").await;

    // Rename the account AND set its avatar AFTER both messages were sent.
    let avatar_mid = upload_media(&a.router, &owner, "image/png", b"\x89PNG\r\n\x1a\nfake").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        "/account",
        Some(&owner),
        Some(&json!({ "display_name": "Renamed", "avatar": avatar_mid })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Re-read the SAME old messages.
    let (_, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    let msgs = body["messages"].as_array().unwrap();
    let find = |id: &str| msgs.iter().find(|m| m["id"] == id).unwrap().clone();
    let ic = find(&in_char);
    let bm = find(&bare);

    // Account identity is LIVE on BOTH old rows (rename + new avatar reflected).
    assert_eq!(bm["author_display"], "Renamed");
    assert_eq!(bm["author_avatar_id"], avatar_mid);
    assert_eq!(ic["author_display"], "Renamed");
    assert_eq!(
        ic["author_avatar_id"], avatar_mid,
        "the account behind a worn persona still resolves its live avatar"
    );

    // The bare message has no persona; the worn-persona message keeps its
    // send-time persona_name snapshot, unaffected by the account change.
    assert!(bm["persona_id"].is_null());
    assert_eq!(ic["persona_name"], "Hero");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn malformed_cursor_is_400_not_500() {
    // F-D4-3: a cursor with valid RFC3339 separators but a malformed tail must
    // yield a deterministic 400, never a 500 from a type::datetime parse failure.
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let (status, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages?before=2026-05-22T12:00:00Xbogus&before_id=abc"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "malformed cursor must 400, not 500"
    );
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

/// L-2: a message body carrying hyperlinks round-trips through the store
/// verbatim, and re-parsing the stored body yields `Node::Link` nodes (both an
/// explicit `[text](url)` and a bare autolink), while a `javascript:` pseudo-URL
/// is NOT linkified — proving the http/https whitelist holds end-to-end.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn hyperlinks_round_trip_and_reject_unsafe_schemes() {
    use authlyn_interactive::markup::{parse, Node};

    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    let raw = "see [docs](https://example.com) and http://bare.test [x](javascript:alert(1))";
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
    let stored = msgs[0]["body"].as_str().unwrap();
    assert_eq!(stored, raw, "link markup is stored and returned verbatim");

    // Re-parsing the stored body proves the link grammar + scheme whitelist.
    let ast = parse(stored);
    assert!(
        ast.iter()
            .any(|n| matches!(n, Node::Link(t, u) if t == "docs" && u == "https://example.com")),
        "explicit markdown link parses to a Link node: {ast:?}"
    );
    assert!(
        ast.iter().any(
            |n| matches!(n, Node::Link(t, u) if t == "http://bare.test" && u == "http://bare.test")
        ),
        "bare http URL autolinks: {ast:?}"
    );
    // The javascript: pseudo-link must NOT linkify — it stays literal text.
    assert!(
        !ast.iter()
            .any(|n| matches!(n, Node::Link(_, u) if u.contains("javascript"))),
        "javascript: is never emitted as a Link: {ast:?}"
    );
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

/// F-D12-1: the `MAX_BODY_CHARS` (50_000) cap is enforced on POST; a body one
/// char over the cap must 400 "message body too long". Guards the cap against a
/// silent refactor removing it (its empty-body / attachment-cap siblings are
/// tested; this one was not).
#[cfg(feature = "ssr")]
#[tokio::test]
async fn post_body_over_char_cap_is_400() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let over_cap = "x".repeat(50_001);
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": over_cap })),
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

/// Post a message as `cookie` to `cid` and return its id.
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
    assert_eq!(status, StatusCode::CREATED);
    m["id"].as_str().unwrap().to_string()
}

/// Fetch the single message in a channel as `cookie`.
#[cfg(feature = "ssr")]
async fn first_message(
    router: &axum::Router,
    cookie: &str,
    cid: &str,
) -> Option<serde_json::Value> {
    let (status, _, body) = common::send(
        router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body["messages"].as_array().unwrap().first().cloned()
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn author_edits_own_message() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let mid = post_one(&a.router, &owner, &cid, "before").await;

    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        Some(&json!({ "body": "after **edit**" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let m = first_message(&a.router, &owner, &cid).await.unwrap();
    assert_eq!(m["body"], "after **edit**", "edit is observable via list");
    assert_eq!(m["id"].as_str().unwrap(), mid, "id is unchanged");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn empty_edit_body_is_400() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let mid = post_one(&a.router, &owner, &cid, "keep me").await;

    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        Some(&json!({ "body": "   " })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The original body survives a rejected edit.
    let m = first_message(&a.router, &owner, &cid).await.unwrap();
    assert_eq!(m["body"], "keep me");
}

/// F-D12-2: the same `MAX_BODY_CHARS` cap on the edit path (PATCH) — a distinct
/// code path from POST, so it needs its own guard. Patching a small message to
/// one char over the cap must 400 "message body too long".
#[cfg(feature = "ssr")]
#[tokio::test]
async fn edit_body_over_char_cap_is_400() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let mid = post_one(&a.router, &owner, &cid, "small").await;

    let over_cap = "x".repeat(50_001);
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        Some(&json!({ "body": over_cap })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The original body survives the rejected over-cap edit.
    let m = first_message(&a.router, &owner, &cid).await.unwrap();
    assert_eq!(m["body"], "small");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn author_deletes_own_message() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let mid = post_one(&a.router, &owner, &cid, "delete me").await;

    let (status, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert!(
        first_message(&a.router, &owner, &cid).await.is_none(),
        "message is gone after delete"
    );

    // Re-deleting your own message is idempotent: soft-delete leaves the row in
    // place and `message_author` does not filter `deleted_at`, so it's still
    // found and still yours — the second DELETE soft-deletes again and 204s.
    let (status, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn other_member_cannot_edit_or_delete() {
    let a = common::arena().await;
    let (owner, gid, cid) = owner_with_text_channel(&a.router).await;
    let mid = post_one(&a.router, &owner, &cid, "owner's words").await;

    // A second member of the same guild (so the channel is visible to them).
    let member = common::register_account(&a.router, "Member", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "Member" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (edit, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&member),
        Some(&json!({ "body": "hijacked" })),
    )
    .await;
    assert_eq!(edit, StatusCode::FORBIDDEN);

    let (del, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(del, StatusCode::FORBIDDEN);

    // The message is untouched.
    let m = first_message(&a.router, &owner, &cid).await.unwrap();
    assert_eq!(m["body"], "owner's words");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nonmember_edit_is_privacy_404() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let mid = post_one(&a.router, &owner, &cid, "private").await;
    let outsider = common::register_account(&a.router, "Outsider", "password123").await;

    let (edit, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&outsider),
        Some(&json!({ "body": "x" })),
    )
    .await;
    assert_eq!(edit, StatusCode::NOT_FOUND);

    let (del, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(del, StatusCode::NOT_FOUND);
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

    // Page 1: no cursor returns the NEWEST 100 (the channel opens at the newest
    // page — commit 8175a95), displayed oldest-first → m50..m149.
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
    assert_eq!(
        page1.first().unwrap()["body"],
        "m50",
        "newest page starts at m50"
    );
    assert_eq!(
        page1.last().unwrap()["body"],
        "m149",
        "newest page ends at m149"
    );

    // Page 2: scroll-up backfill — older history BEFORE page1's first row, via
    // the (before, before_id) composite cursor → m0..m49.
    let first = page1.first().unwrap();
    let before = first["sent_at"].as_str().unwrap();
    let before_id = first["id"].as_str().unwrap();
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages?before={before}&before_id={before_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page2 = body["messages"].as_array().unwrap().clone();
    assert_eq!(page2.len(), 50, "the remaining older messages");

    // The two pages together are exactly m0..m149, in order, no dups/gaps:
    // older backfill (page2 = m0..m49) precedes the newest page (page1 = m50..m149).
    let bodies: Vec<String> = page2
        .iter()
        .chain(page1.iter())
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

/// Force a message's `sent_at` to an exact instant via direct DB write — the
/// API stamps `time::now()`, so equal-`sent_at` tie groups (the boundary the
/// strict composite cursor tie-break exists for) can only be built this way.
/// Parameterized: the datetime rides `type::datetime($at)`, never a
/// `<string>` cast (the cursor invariant).
#[cfg(feature = "ssr")]
async fn force_sent_at(a: &common::Arena, mid: &str, at: &str) {
    a.db.query("UPDATE type::record('message', $mid) SET sent_at = type::datetime($at);")
        .bind(("mid", mid.to_string()))
        .bind(("at", at.to_string()))
        .await
        .expect("force sent_at transport")
        .check()
        .expect("force sent_at");
}

/// GET a message page and return its ids in display order.
#[cfg(feature = "ssr")]
async fn page_ids(router: &axum::Router, cookie: &str, cid: &str, query: &str) -> Vec<String> {
    let (status, _, body) = common::send(
        router,
        Method::GET,
        &format!("/channels/{cid}/messages{query}"),
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect()
}

/// Review M-11/M-12: the composite cursor at an equal-`sent_at` tie group, in
/// BOTH directions. Three early rows, a five-row tie group sharing ONE
/// instant, three late rows; the cursor walks the tie group by id. Pins the
/// exact page membership + order the index-friendly split (equality-boundary
/// statement + open-range statement) must reassemble: strictly-greater /
/// strictly-lesser tie ids only, the cursor row itself never included.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn cursor_tie_break_with_equal_sent_at_is_strict_in_both_directions() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    const T_TIE: &str = "2026-01-01T10:00:10Z";
    let mut early: Vec<String> = Vec::new();
    for i in 0..3 {
        let id = post_one(&a.router, &owner, &cid, &format!("early {i}")).await;
        force_sent_at(&a, &id, &format!("2026-01-01T10:00:0{i}Z")).await;
        early.push(id);
    }
    let mut ties: Vec<String> = Vec::new();
    for i in 0..5 {
        let id = post_one(&a.router, &owner, &cid, &format!("tie {i}")).await;
        force_sent_at(&a, &id, T_TIE).await;
        ties.push(id);
    }
    let mut late: Vec<String> = Vec::new();
    for i in 0..3 {
        let id = post_one(&a.router, &owner, &cid, &format!("late {i}")).await;
        force_sent_at(&a, &id, &format!("2026-01-01T10:00:2{i}Z")).await;
        late.push(id);
    }
    // Within the tie group display order is the id tie-break (lexical).
    ties.sort();

    // Forward (catch-up) from the MIDDLE tie id: the strictly-greater tie ids
    // in id order, then the late rows — the cursor row itself excluded.
    let got = page_ids(
        &a.router,
        &owner,
        &cid,
        &format!("?since={T_TIE}&after_id={}", ties[2]),
    )
    .await;
    let expected: Vec<String> = ties[3..].iter().chain(late.iter()).cloned().collect();
    assert_eq!(got, expected, "catch-up from the middle of the tie group");

    // Forward from the TOP tie id: the tie group is exhausted.
    let got = page_ids(
        &a.router,
        &owner,
        &cid,
        &format!("?since={T_TIE}&after_id={}", ties[4]),
    )
    .await;
    assert_eq!(got, late, "catch-up from the top of the tie group");

    // Backward (scroll-up) from the middle tie id: the early rows + the
    // strictly-lesser tie ids, ASC display order, cursor row excluded.
    let got = page_ids(
        &a.router,
        &owner,
        &cid,
        &format!("?before={T_TIE}&before_id={}", ties[2]),
    )
    .await;
    let expected: Vec<String> = early.iter().chain(ties[..2].iter()).cloned().collect();
    assert_eq!(got, expected, "backfill from the middle of the tie group");

    // Backward from the BOTTOM tie id: no tie rows at all.
    let got = page_ids(
        &a.router,
        &owner,
        &cid,
        &format!("?before={T_TIE}&before_id={}", ties[0]),
    )
    .await;
    assert_eq!(got, early, "backfill from the bottom of the tie group");
}

/// Review M-12: when MORE than a page matches a cursor, both directions keep
/// the 100 rows NEAREST the cursor — catch-up keeps the oldest 100 past it,
/// backfill keeps the newest 100 before it. NOTE: every row here carries a
/// distinct API-stamped `sent_at`, so the tie-boundary statement returns zero
/// rows and the range statement self-limits to exactly the page — this test
/// pins the range arm's LIMIT direction only. The `rows.truncate(...)` line
/// itself is pinned by `cursor_truncation_keeps_tie_rows_nearest_the_cursor_
/// when_tie_and_range_overflow_the_page` below, where BOTH statements return
/// rows.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn cursor_pages_keep_the_hundred_rows_nearest_the_cursor_when_more_match() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    const TOTAL: usize = 121; // m0..m120 — one more than a page on each side
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

    // Recover the m0 and m120 envelopes: the newest page holds m21..m120, a
    // backfill before its first row holds m0..m20.
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let newest = body["messages"].as_array().unwrap().clone();
    assert_eq!(newest.first().unwrap()["body"], "m21");
    let m120 = newest.last().unwrap().clone();
    assert_eq!(m120["body"], "m120");
    let first = newest.first().unwrap();
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!(
            "/channels/{cid}/messages?before={}&before_id={}",
            first["sent_at"].as_str().unwrap(),
            first["id"].as_str().unwrap()
        ),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let oldest = body["messages"].as_array().unwrap().clone();
    let m0 = oldest.first().unwrap().clone();
    assert_eq!(m0["body"], "m0");

    // Catch-up from m0: 120 rows match; the page is the oldest 100 of them
    // (m1..m100), nearest AFTER the cursor.
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!(
            "/channels/{cid}/messages?since={}&after_id={}",
            m0["sent_at"].as_str().unwrap(),
            m0["id"].as_str().unwrap()
        ),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page = body["messages"].as_array().unwrap();
    assert_eq!(page.len(), 100);
    assert_eq!(page.first().unwrap()["body"], "m1");
    assert_eq!(page.last().unwrap()["body"], "m100");

    // Backfill from m120: 120 rows match; the page is the newest 100 of them
    // (m20..m119), nearest BEFORE the cursor.
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!(
            "/channels/{cid}/messages?before={}&before_id={}",
            m120["sent_at"].as_str().unwrap(),
            m120["id"].as_str().unwrap()
        ),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page = body["messages"].as_array().unwrap();
    assert_eq!(page.len(), 100);
    assert_eq!(page.first().unwrap()["body"], "m20");
    assert_eq!(page.last().unwrap()["body"], "m119");
}

/// Review M-12 (re-review): pins the ONLY correctness-load-bearing Rust line
/// of the split-statement rewrite — `rows.truncate(MESSAGES_PAGE_LIMIT)`
/// BEFORE the display flip (`reading.rs::load_messages`). A page of distinct
/// timestamps can never exercise it (the tie statement returns zero rows and
/// the range statement self-limits to exactly 100, so the truncate is a
/// no-op); here BOTH statements return rows: a 5-row tie group at the cursor
/// instant plus 120 range rows past it → 104 candidates for a 100-row page.
/// Kills both surviving mutants:
/// - truncate DELETED → a 104-row page leaks (the length assert);
/// - truncate moved AFTER `out.reverse()` → the Before page keeps the FAR
///   oldest rows and drops the 4 tie rows nearest the cursor (silently
///   skipped history — the membership asserts).
/// Layout: low tie group at T_LOW, 120 distinct-stamp range rows between,
/// high tie group at T_HIGH. Catch-up from the bottom low-tie id must return
/// the 4 greater low-tie ids + the oldest 96 range rows; backfill from the
/// top high-tie id must return the newest 96 range rows + the 4 lesser
/// high-tie ids (ASC display order).
#[cfg(feature = "ssr")]
#[tokio::test]
async fn cursor_truncation_keeps_tie_rows_nearest_the_cursor_when_tie_and_range_overflow_the_page()
{
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    const T_LOW: &str = "2026-02-01T10:00:00Z";
    const T_HIGH: &str = "2026-02-01T12:00:00Z";

    let mut low_ties: Vec<String> = Vec::new();
    for i in 0..5 {
        let id = post_one(&a.router, &owner, &cid, &format!("low tie {i}")).await;
        force_sent_at(&a, &id, T_LOW).await;
        low_ties.push(id);
    }
    let mut range: Vec<String> = Vec::new();
    for i in 0..120 {
        let id = post_one(&a.router, &owner, &cid, &format!("range {i}")).await;
        // Distinct strictly-increasing instants between the tie groups:
        // 11:00:00 .. 11:01:59.
        force_sent_at(
            &a,
            &id,
            &format!("2026-02-01T11:{:02}:{:02}Z", i / 60, i % 60),
        )
        .await;
        range.push(id);
    }
    let mut high_ties: Vec<String> = Vec::new();
    for i in 0..5 {
        let id = post_one(&a.router, &owner, &cid, &format!("high tie {i}")).await;
        force_sent_at(&a, &id, T_HIGH).await;
        high_ties.push(id);
    }
    // Within a tie group display order is the id tie-break (lexical).
    low_ties.sort();
    high_ties.sort();

    // Catch-up from the BOTTOM low-tie id: 124 rows match (4 ties + 120
    // range); the page is the 100 nearest AFTER the cursor — all 4 greater
    // tie ids first, then the oldest 96 range rows. A dropped truncate leaks
    // 104 rows here.
    let got = page_ids(
        &a.router,
        &owner,
        &cid,
        &format!("?since={T_LOW}&after_id={}", low_ties[0]),
    )
    .await;
    let expected: Vec<String> = low_ties[1..]
        .iter()
        .chain(range[..96].iter())
        .cloned()
        .collect();
    assert_eq!(got.len(), 100, "catch-up page is exactly one page");
    assert_eq!(
        got, expected,
        "catch-up keeps the tie rows nearest the cursor"
    );

    // Backfill from the TOP high-tie id: 124 rows match (4 ties + 120 range);
    // the page is the 100 nearest BEFORE the cursor — the newest 96 range
    // rows, then all 4 lesser tie ids, ASC display order. Truncating AFTER
    // the display reverse keeps the far-oldest range rows instead and drops
    // the tie rows entirely.
    let got = page_ids(
        &a.router,
        &owner,
        &cid,
        &format!("?before={T_HIGH}&before_id={}", high_ties[4]),
    )
    .await;
    let expected: Vec<String> = range[24..]
        .iter()
        .chain(high_ties[..4].iter())
        .cloned()
        .collect();
    assert_eq!(got.len(), 100, "backfill page is exactly one page");
    assert_eq!(
        got, expected,
        "backfill keeps the tie rows nearest the cursor, not the far end of the range"
    );
}

/// Locks down the W5/H2 batch typing-name resolution: two typists are pinged
/// into the channel; a third member polls and sees both names — and never
/// themselves — using their username (no `display_name`, no worn persona,
/// exercises the `??`-fallback chain). Indirectly proves the batched
/// `(account IN-list) + (channel_active_persona IN-list)` query merges
/// correctly in Rust.
/// Bulk-CREATE `n` `media_blob` rows owned by `account_id` via direct DB,
/// returning their ids in CREATE order. Avoids 100x multipart round-trips when
/// a test only needs media ids to exist (e.g. probing the attachment cap).
///
/// One query with N stacked CREATEs is simpler than wrestling with FOR-loop
/// result indexing in 3.1.0-beta.3 — each CREATE produces its own response
/// statement, so we take(i) once per row.
#[cfg(feature = "ssr")]
async fn bulk_create_media_rows(
    db: &surrealdb::Surreal<surrealdb::engine::remote::ws::Client>,
    account_id: &str,
    n: usize,
) -> Vec<String> {
    use surrealdb::types::SurrealValue;
    #[derive(SurrealValue)]
    struct IdKey {
        id_key: String,
    }
    let mut sql = String::with_capacity(n * 200);
    for _ in 0..n {
        sql.push_str(
            "CREATE media_blob SET \
                uploader = type::record('account', $owner), \
                mime = 'image/png', \
                size_bytes = 1, \
                storage_path = 'x' \
                RETURN meta::id(id) AS id_key;\n",
        );
    }
    let mut resp = db
        .query(sql)
        .bind(("owner", account_id.to_string()))
        .await
        .expect("bulk media create")
        .check()
        .expect("bulk media check");
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let row: Option<IdKey> = resp.take(i).expect("bulk media take");
        ids.push(row.expect("each CREATE returns one row").id_key);
    }
    ids
}

/// Register and return both the session cookie and the new account id.
#[cfg(feature = "ssr")]
async fn register_with_id(router: &axum::Router, name: &str) -> (String, String) {
    let (status, cookie, body) = common::send(
        router,
        axum::http::Method::POST,
        "/auth/register",
        None,
        Some(&serde_json::json!({ "username": name, "password": "password123" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    (
        cookie.expect("register sets session cookie"),
        body["account_id"]
            .as_str()
            .expect("register returns account_id")
            .to_string(),
    )
}

/// Boundary of the attachment cap: 100 attachments POST cleanly (201); 101 is
/// rejected (400 "too many attachments"). Locks down the W7/B1 cap-raise
/// (10 → 100) so a future drift back is caught.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn attachment_cap_boundary_is_100() {
    let a = common::arena().await;
    let (owner_cookie, owner_id) = register_with_id(&a.router, "Owner").await;
    let (_status, _, guild) = common::send(
        &a.router,
        Method::POST,
        "/guilds",
        Some(&owner_cookie),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    let gid = guild["id"].as_str().unwrap().to_string();
    let (_status, _, detail) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner_cookie),
        None,
    )
    .await;
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();

    // Pre-create 101 media_blob rows so we can probe both sides of the cap
    // without 101 multipart round-trips. The handler dedupes attachments
    // before counting, so we need 101 distinct ids — not 1 id repeated.
    let ids = bulk_create_media_rows(&a.db, &owner_id, 101).await;
    assert_eq!(ids.len(), 101, "bulk media create returned 101 ids");

    // Exactly 100 → 201 CREATED.
    let at_cap: Vec<&str> = ids[..100].iter().map(String::as_str).collect();
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner_cookie),
        Some(&json!({ "body": "max attachments", "attachment_ids": at_cap })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "100 attachments must be accepted: {body:?}"
    );

    // 101 → 400 with the canonical message.
    let over_cap: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner_cookie),
        Some(&json!({ "body": "one too many", "attachment_ids": over_cap })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "101 attachments must 400");
    assert_eq!(
        body["error"], "too many attachments",
        "rejection carries the canonical message"
    );
}

/// Upload `data` as `mime` via multipart `POST /media`, asserting 201 and
/// returning the new media id. Used to stage a real non-image attachment.
#[cfg(feature = "ssr")]
async fn upload_media(router: &axum::Router, cookie: &str, mime: &str, data: &[u8]) -> String {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request};
    use tower::ServiceExt;

    let boundary = "Xbnd";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"f\"\r\n\
         Content-Type: {mime}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let req = Request::builder()
        .method(Method::POST)
        .uri("/media")
        .header(header::COOKIE, cookie)
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let res = router.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(res.status(), StatusCode::CREATED, "media upload should 201");
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    v["id"].as_str().unwrap().to_string()
}

/// A message carrying a PDF attachment sends (201) and reads back cleanly: the
/// attachment surfaces in the list with its stored MIME, proving arbitrary
/// (non-image) files flow through the send/read path unchanged.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn message_with_pdf_attachment_sends_and_reads() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    let pdf_id = upload_media(&a.router, &owner, "application/pdf", b"%PDF-1.4\nbody\n").await;

    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "see attached", "attachment_ids": [pdf_id.clone()] })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "pdf-attached message: {body:?}"
    );

    let (status, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let msg = &list["messages"][0];
    assert_eq!(msg["body"], "see attached");
    let atts = msg["attachments"].as_array().expect("attachments array");
    assert_eq!(atts.len(), 1, "exactly one attachment");
    assert_eq!(atts[0]["id"], pdf_id, "attachment id round-trips");
    assert_eq!(
        atts[0]["mime"], "application/pdf",
        "stored MIME round-trips on read"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn typing_indicator_lists_other_typists_and_excludes_caller() {
    let a = common::arena().await;
    let (owner, gid, cid) = owner_with_text_channel(&a.router).await;

    // Two extra members so the batch IN-list carries >1 id.
    let alice = common::register_account(&a.router, "Alice", "password123").await;
    let bob = common::register_account(&a.router, "Bob", "password123").await;
    for name in ["Alice", "Bob"] {
        let (status, _, _) = common::send(
            &a.router,
            Method::POST,
            &format!("/guilds/{gid}/members"),
            Some(&owner),
            Some(&json!({ "username": name })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    // Alice and Bob each ping typing; owner also pings (must not see self).
    for cookie in [&alice, &bob, &owner] {
        let (status, _, _) = common::send(
            &a.router,
            Method::POST,
            &format!("/channels/{cid}/typing"),
            Some(cookie),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let typing: Vec<String> = body["typing"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut sorted = typing.clone();
    sorted.sort();
    assert_eq!(
        sorted,
        vec!["Alice".to_string(), "Bob".to_string()],
        "both other typists are resolved by username (display_name = '' → username fallback); caller is excluded"
    );
}

// ---------------------------------------------------------------------------
// Reply-to-message (L-3)
// ---------------------------------------------------------------------------

/// Post a reply to `parent` in `cid` and return the raw `(status, body)`.
#[cfg(feature = "ssr")]
async fn reply_to(
    router: &axum::Router,
    cookie: &str,
    cid: &str,
    body: &str,
    parent: &str,
) -> (StatusCode, serde_json::Value) {
    let (status, _, m) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(cookie),
        Some(&json!({ "body": body, "reply_to_id": parent })),
    )
    .await;
    (status, m)
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn reply_to_same_channel_persists() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let parent = post_one(&a.router, &owner, &cid, "the original").await;

    let (status, m) = reply_to(&a.router, &owner, &cid, "a reply", &parent).await;
    assert_eq!(status, StatusCode::CREATED);
    let reply_id = m["id"].as_str().unwrap().to_string();

    // The reply is returned with a live preview of its parent.
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
    let reply = msgs.iter().find(|m| m["id"] == reply_id).unwrap();
    assert_eq!(
        reply["reply_to"]["id"].as_str().unwrap(),
        parent,
        "reply preview points at the parent message"
    );
    assert_eq!(reply["reply_to"]["body_snippet"], "the original");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn reply_to_other_channel_is_400() {
    let a = common::arena().await;
    let (owner, gid, cid) = owner_with_text_channel(&a.router).await;
    let parent = post_one(&a.router, &owner, &cid, "lives in channel 1").await;

    // A second text channel in the same guild.
    let (status, _, second) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "two", "kind": "text" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cid2 = second["id"].as_str().unwrap();

    // Replying in channel 2 to a parent in channel 1 is rejected.
    let (status, _) = reply_to(&a.router, &owner, cid2, "cross-channel reply", &parent).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a reply target in a different channel must 400"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn reply_to_soft_deleted_parent_is_400() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let parent = post_one(&a.router, &owner, &cid, "doomed").await;

    // Soft-delete the parent (DELETE soft-deletes the author's own message).
    let (status, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{parent}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = reply_to(&a.router, &owner, &cid, "reply to a tombstone", &parent).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a soft-deleted reply target must 400"
    );
}

/// M-26: validation ORDER is security-load-bearing. `reply_target_valid` and
/// `all_media_exist` are DB probes, so the membership gate (privacy-404) must
/// run BEFORE them — otherwise a non-member POSTing a reply gets 400 "invalid
/// reply target" when the target is absent from the channel but 404 when it
/// exists there, a 400-vs-404 existence oracle on (cid, rid) pairs. Every
/// non-member probe variant must collapse to the byte-identical privacy-404.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn nonmember_post_probes_collapse_to_the_identical_privacy_404() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let parent = post_one(&a.router, &owner, &cid, "members only").await;
    let outsider = common::register_account(&a.router, "Outsider", "password123").await;

    // (a) Reply target that EXISTS in the channel — already 404 today.
    let (st_real, body_real) = reply_to(&a.router, &outsider, &cid, "probe", &parent).await;
    // (b) Reply target absent from the channel — the oracle's other face:
    // must be the SAME 404, never the validator's 400.
    let (st_bogus, body_bogus) = reply_to(&a.router, &outsider, &cid, "probe", "nosuchmsg").await;
    // (c) Unknown attachment id — `all_media_exist` is a DB probe too.
    let (st_media, _, body_media) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&outsider),
        Some(&json!({ "body": "probe", "attachment_ids": ["nosuchblob"] })),
    )
    .await;
    // (d) The canonical privacy-404 every channel handler emits.
    let (st_canon, _, body_canon) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&outsider),
        None,
    )
    .await;

    assert_eq!(st_real, StatusCode::NOT_FOUND);
    assert_eq!(
        st_bogus,
        StatusCode::NOT_FOUND,
        "a non-member's bogus reply target must hit the membership 404, not the 400 validator"
    );
    assert_eq!(
        st_media,
        StatusCode::NOT_FOUND,
        "a non-member's unknown attachment must hit the membership 404, not the 400 validator"
    );
    assert_eq!(st_canon, StatusCode::NOT_FOUND);
    assert_eq!(
        body_real, body_bogus,
        "existing vs missing reply target must be indistinguishable to a non-member"
    );
    assert_eq!(
        body_bogus, body_media,
        "reply-target and attachment probes must share one privacy-404 body"
    );
    assert_eq!(
        body_media, body_canon,
        "the POST privacy-404 body must be byte-identical to the list handler's"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn reply_preview_renders_author_and_snippet() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    // A long parent body so the snippet is truncated to ~100 chars.
    let long = "x".repeat(250);
    let parent = post_one(&a.router, &owner, &cid, &long).await;
    let (status, _m) = reply_to(&a.router, &owner, &cid, "see above", &parent).await;
    assert_eq!(status, StatusCode::CREATED);

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
    let reply = msgs.iter().find(|m| m["body"] == "see above").unwrap();
    let preview = &reply["reply_to"];
    // author_display falls back to the username ("Owner") since no display_name.
    assert_eq!(preview["author_display"], "Owner");
    let snippet = preview["body_snippet"].as_str().unwrap();
    assert_eq!(snippet.chars().count(), 100, "snippet capped at ~100 chars");
    assert!(snippet.chars().all(|c| c == 'x'));
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn reply_to_deleted_after_send_null_joins_gracefully() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    let parent = post_one(&a.router, &owner, &cid, "soon gone").await;
    let (status, m) = reply_to(&a.router, &owner, &cid, "still here", &parent).await;
    assert_eq!(status, StatusCode::CREATED);
    let reply_id = m["id"].as_str().unwrap().to_string();

    // Soft-delete the PARENT after the reply already exists.
    let (status, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{parent}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The reply still lists; its preview degrades to null rather than dangling.
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
    let reply = msgs.iter().find(|m| m["id"] == reply_id).unwrap();
    assert!(
        reply["reply_to"].is_null(),
        "a parent soft-deleted after send null-joins the preview, not a 500/dangle"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn composite_cursor_unaffected_by_reply_to() {
    // Reply rows participate in the same (sent_at, id) cursor as any other
    // message; a page of replies paginates in send order without dups/gaps.
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    const TOTAL: usize = 150;
    // First message is a plain root; every subsequent one replies to the root,
    // so the cursor must still order all 150 by send order.
    let root = post_one(&a.router, &owner, &cid, "m0").await;
    for i in 1..TOTAL {
        let (status, _) = reply_to(&a.router, &owner, &cid, &format!("m{i}"), &root).await;
        assert_eq!(status, StatusCode::CREATED);
    }

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
    assert_eq!(page1.first().unwrap()["body"], "m50");
    assert_eq!(page1.last().unwrap()["body"], "m149");

    let first = page1.first().unwrap();
    let before = first["sent_at"].as_str().unwrap();
    let before_id = first["id"].as_str().unwrap();
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages?before={before}&before_id={before_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let page2 = body["messages"].as_array().unwrap().clone();
    assert_eq!(page2.len(), 50);

    let bodies: Vec<String> = page2
        .iter()
        .chain(page1.iter())
        .map(|m| m["body"].as_str().unwrap().to_string())
        .collect();
    let expected: Vec<String> = (0..TOTAL).map(|i| format!("m{i}")).collect();
    assert_eq!(bodies, expected, "reply rows reassemble in send order");
}

/// T10 behavior pin for folding attachment-MIME resolution into the page
/// projection: a single message carrying BOTH a PNG and a PDF lists back with
/// each attachment bearing its OWN stored MIME — pinning the per-id merge
/// semantics (not just "some mime appears"), which the implementation swap
/// underneath must preserve. Strengthens the single-attachment assertion in
/// `message_with_pdf_attachment_sends_and_reads`.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn list_messages_returns_attachment_mime_in_the_page_response() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    let png_id = upload_media(&a.router, &owner, "image/png", b"\x89PNG\r\n\x1a\nfake").await;
    let pdf_id = upload_media(&a.router, &owner, "application/pdf", b"%PDF-1.4\nbody\n").await;

    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({
            "body": "mixed attachments",
            "attachment_ids": [png_id.clone(), pdf_id.clone()]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "attached message: {body:?}");

    let (status, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let atts = list["messages"][0]["attachments"]
        .as_array()
        .expect("attachments array");
    assert_eq!(atts.len(), 2, "both attachments ride the page");
    let mime_of = |id: &str| {
        atts.iter()
            .find(|a| a["id"] == id)
            .unwrap_or_else(|| panic!("attachment {id} missing from page"))["mime"]
            .clone()
    };
    assert_eq!(
        mime_of(&png_id),
        "image/png",
        "PNG mime rides the page response"
    );
    assert_eq!(
        mime_of(&pdf_id),
        "application/pdf",
        "each attachment id resolves to ITS OWN mime"
    );
}

// ---------------------------------------------------------------------------
// Message effects (W4/T5): whisper / shout / spell
// ---------------------------------------------------------------------------

/// W4/T5: a message posted with a valid `effect` round-trips — the value rides
/// MSG_PROJECTION onto the list envelope — while an effect-less post and an
/// empty-string `effect` (treated as absent, mirroring `reply_to_id`) both
/// list `effect` as null.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn message_effect_round_trips_through_post_and_list() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    // 1) plain, 2) whispered, 3) empty-string effect (= absent).
    post_one(&a.router, &owner, &cid, "plain").await;
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "psst", "effect": "whisper" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "whispered post: {body:?}");
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "blank effect", "effect": "" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "empty effect = absent: {body:?}"
    );

    let (status, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let msgs = list["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    assert!(
        msgs[0]["effect"].is_null(),
        "an effect-less message lists effect = null, got {:?}",
        msgs[0]["effect"]
    );
    assert_eq!(
        msgs[1]["effect"], "whisper",
        "the effect rides the list envelope"
    );
    assert!(
        msgs[2]["effect"].is_null(),
        "an empty-string effect is treated as absent, got {:?}",
        msgs[2]["effect"]
    );
}

/// W4/T5: an out-of-set `effect` is rejected with a 400 BEFORE any write —
/// server-side validation mirrors the body checks rather than leaning on the
/// DB ASSERT (which would surface as a 500), and nothing is persisted.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn unknown_message_effect_is_400_and_persists_nothing() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "boom", "effect": "sparkle-bomb" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a garbage effect must 400, got {status}: {body:?}"
    );

    let (status, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        list["messages"].as_array().unwrap().len(),
        0,
        "the rejected message must not be persisted"
    );
}

/// W4/T5 spoiler-leak guard: replying to a whispered message must NOT leak the
/// hidden text through the reply-quote preview — the projection masks the
/// snippet with a fixed placeholder when the parent carries effect='whisper'.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn reply_preview_masks_whispered_parent_snippet() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "the hidden secret", "effect": "whisper" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let parent = body["id"].as_str().unwrap().to_string();

    let (status, m) = reply_to(&a.router, &owner, &cid, "what was that?", &parent).await;
    assert_eq!(status, StatusCode::CREATED);
    let reply_id = m["id"].as_str().unwrap().to_string();

    let (status, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let msgs = list["messages"].as_array().unwrap();
    let reply = msgs.iter().find(|m| m["id"] == reply_id).unwrap();
    let snippet = reply["reply_to"]["body_snippet"].as_str().unwrap();
    assert!(
        !snippet.contains("hidden secret"),
        "whispered parent text must not leak via the reply quote, got {snippet:?}"
    );
    assert_eq!(snippet, "(whisper)", "masked with the fixed placeholder");
}

/// Review M-12 (newest-page arm): the cursorless newest page must be the top
/// `MESSAGES_PAGE_LIMIT` rows under the strict `(sent_at, id)` order —
/// including when an equal-`sent_at` tie group STRADDLES the page boundary:
/// the tie group is cut by id (highest ids stay in the page) and the cut-off
/// rows remain reachable through the before-cursor, so no row is ever lost at
/// the seam. Pins the exact semantics the index-friendly boundary-probe split
/// (LET boundary + open range + boundary tie group) must reproduce.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn newest_page_boundary_tie_group_is_cut_by_id_and_loses_no_row() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;

    const T_TIE: &str = "2026-01-01T09:59:59Z";
    let mut ties: Vec<String> = Vec::new();
    for i in 0..6 {
        let id = post_one(&a.router, &owner, &cid, &format!("tie {i}")).await;
        force_sent_at(&a, &id, T_TIE).await;
        ties.push(id);
    }
    let mut later: Vec<String> = Vec::new();
    for i in 0..97 {
        let id = post_one(&a.router, &owner, &cid, &format!("later {i}")).await;
        force_sent_at(
            &a,
            &id,
            &format!("2026-01-01T10:{:02}:{:02}Z", i / 60, i % 60),
        )
        .await;
        later.push(id);
    }
    // Within the tie group display order is the id tie-break (lexical).
    ties.sort();

    // Newest page: 103 live rows, the top 100 under (sent_at, id) = the three
    // HIGHEST tie ids, then every later row — ASC display order.
    let got = page_ids(&a.router, &owner, &cid, "").await;
    let expected: Vec<String> = ties[3..].iter().chain(later.iter()).cloned().collect();
    assert_eq!(got.len(), 100);
    assert_eq!(
        got, expected,
        "the newest page must cut the tie group by id"
    );

    // The three cut-off tie rows are exactly the backfill page before the
    // newest page's first row — the seam loses nothing.
    let got = page_ids(
        &a.router,
        &owner,
        &cid,
        &format!("?before={T_TIE}&before_id={}", ties[3]),
    )
    .await;
    assert_eq!(got, ties[..3].to_vec(), "the seam must lose no tie row");
}

/// Review M-12 (newest-page arm): the boundary probe of the restructured
/// newest page must degrade cleanly when the channel holds NO live rows —
/// both a virgin channel and one whose every message is soft-deleted return
/// an empty page (never an error).
#[cfg(feature = "ssr")]
#[tokio::test]
async fn newest_page_is_empty_for_virgin_and_fully_deleted_channels() {
    let a = common::arena().await;
    let (owner, gid, cid) = owner_with_text_channel(&a.router).await;

    // Virgin channel: no message has ever existed.
    let (status, _, chan) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "virgin", "kind": "text" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let virgin_cid = chan["id"].as_str().unwrap().to_string();
    let got = page_ids(&a.router, &owner, &virgin_cid, "").await;
    assert!(got.is_empty(), "a virgin channel's page must be empty");

    // Fully soft-deleted channel: rows exist, none of them live.
    let mid = post_one(&a.router, &owner, &cid, "soon gone").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let got = page_ids(&a.router, &owner, &cid, "").await;
    assert!(
        got.is_empty(),
        "a fully soft-deleted channel's page must be empty"
    );
}
