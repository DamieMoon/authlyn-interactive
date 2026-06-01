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
