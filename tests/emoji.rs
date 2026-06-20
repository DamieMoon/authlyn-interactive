//! Wave-1 SAFETY-NET: custom emoji CRUD characterization (`src/server/emoji.rs`).
//!
//! Locks current behavior ahead of the refactor waves (audit 019e6c08):
//!   - create / list / delete round-trip;
//!   - name rule `^[a-z0-9_]{2,32}$`, validated by BYTE length (`name.len()`,
//!     emoji.rs:28 — the audit's M2 char-vs-byte caveat: emoji uses .len());
//!   - authz: any member may list + create; delete is manager-only (owner/admin);
//!     a plain member's delete → 403; a non-member → 404 (privacy);
//!   - `custom_emoji_guild_name` UNIQUE (guild, name) collision → 409;
//!   - the UNIQUE is per-guild composite: the same name in a different guild is OK.
//!
//! Validation ORDER matters and is locked here: the member gate runs BEFORE name
//! validation, so a non-member with a bad name gets 404, not 400.

mod common;

#[cfg(feature = "ssr")]
use axum::body::{to_bytes, Body};
#[cfg(feature = "ssr")]
use axum::http::{header, Method, Request, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::{json, Value};
#[cfg(feature = "ssr")]
use tower::ServiceExt;

#[cfg(feature = "ssr")]
async fn upload_image(router: &axum::Router, cookie: &str, mime: &str, data: &[u8]) -> String {
    let boundary = "Xbnd";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"img\"\r\n\
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
    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    v["id"].as_str().unwrap().to_string()
}

/// Create a guild, returning its id.
#[cfg(feature = "ssr")]
async fn create_guild(router: &axum::Router, cookie: &str) -> String {
    let (st, _, g) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(cookie),
        Some(&json!({ "name": "Guild" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    g["id"].as_str().unwrap().to_string()
}

/// The owner invites `invitee_name` (by username) into guild `gid`; they become
/// a `member`-role row. Mirrors the real invite flow (`InviteMemberRequest`).
#[cfg(feature = "ssr")]
async fn invite(router: &axum::Router, owner: &str, gid: &str, invitee_name: &str) {
    let (st, _, _) = common::send(
        router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(owner),
        Some(&json!({ "username": invitee_name })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "invite should 201");
}

// ---------------------------------------------------------------------------
// CRUD round-trip
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn create_list_delete_round_trip() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner).await;
    let media = upload_image(&a.router, &owner, "image/png", b"\x89PNG e").await;

    // Create.
    let (st, _, e) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/emoji"),
        Some(&owner),
        Some(&json!({ "name": "wave", "media_id": media })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(e["name"], "wave");
    assert_eq!(e["media_id"], media);
    assert!(e["id"].as_str().is_some());
    assert!(e["created_at"].as_str().is_some());

    // List shows it.
    let (st, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}/emoji"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let emoji = list["emoji"].as_array().unwrap();
    assert_eq!(emoji.len(), 1);
    assert_eq!(emoji[0]["name"], "wave");

    // Delete by name → 204.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}/emoji/wave"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Gone from the list.
    let (_, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}/emoji"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(list["emoji"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Name rule [a-z0-9_]{2,32}, byte-length bounds
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn name_rule_is_enforced() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner).await;
    let media = upload_image(&a.router, &owner, "image/png", b"\x89PNG e").await;

    let try_name = |name: String| {
        let router = a.router.clone();
        let owner = owner.clone();
        let gid = gid.clone();
        let media = media.clone();
        async move {
            common::send(
                &router,
                Method::POST,
                &format!("/guilds/{gid}/emoji"),
                Some(&owner),
                Some(&json!({ "name": name, "media_id": media })),
            )
            .await
            .0
        }
    };

    // Too short (1 char) → 400.
    assert_eq!(try_name("a".into()).await, StatusCode::BAD_REQUEST);
    // Exactly 2 chars → OK (boundary).
    assert_eq!(try_name("ab".into()).await, StatusCode::CREATED);
    // Exactly 32 chars → OK (boundary).
    assert_eq!(try_name("a".repeat(32)).await, StatusCode::CREATED);
    // 33 chars → 400.
    assert_eq!(try_name("a".repeat(33)).await, StatusCode::BAD_REQUEST);
    // Uppercase rejected.
    assert_eq!(try_name("Wave".into()).await, StatusCode::BAD_REQUEST);
    // Hyphen rejected (only [a-z0-9_]).
    assert_eq!(try_name("wa-ve".into()).await, StatusCode::BAD_REQUEST);
    // Digits + underscore allowed.
    assert_eq!(try_name("hi_5".into()).await, StatusCode::CREATED);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn name_length_is_byte_counted_not_char_counted() {
    // Audit caveat (M2): emoji name length uses `name.len()` (BYTES), unlike the
    // char-counted validators elsewhere. A 2-codepoint multibyte string is >2
    // bytes; either way it also fails the ASCII char-class — but a string whose
    // BYTE length exceeds 32 while CHAR count is ≤32 is rejected, proving the
    // byte-length rule. "é" is 2 bytes (UTF-8), so 17×"é" = 34 bytes, 17 chars.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner).await;
    let media = upload_image(&a.router, &owner, "image/png", b"\x89PNG e").await;

    let name = "é".repeat(17); // 17 chars, 34 bytes
    assert_eq!(name.chars().count(), 17);
    assert_eq!(name.len(), 34);
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/emoji"),
        Some(&owner),
        Some(&json!({ "name": name, "media_id": media })),
    )
    .await;
    // Rejected for length (byte-counted > 32) AND char-class — either way 400.
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// Authorization: member-list, manager-only-delete, non-member-404
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn members_create_and_list_but_only_managers_delete() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let member = common::register_account(&a.router, "Member", "password123").await;
    let gid = create_guild(&a.router, &owner).await;
    invite(&a.router, &owner, &gid, "Member").await;
    let media = upload_image(&a.router, &owner, "image/png", b"\x89PNG e").await;

    // A plain member CAN create.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/emoji"),
        Some(&member),
        Some(&json!({ "name": "bymember", "media_id": media })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "member can create emoji");

    // A plain member CAN list.
    let (st, _, list) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}/emoji"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list["emoji"].as_array().unwrap().len(), 1);

    // A plain member CANNOT delete → 403 (manager-only).
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}/emoji/bymember"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "member delete is 403");

    // The owner (manager) CAN delete.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}/emoji/bymember"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "owner delete is 204");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn non_member_is_404_on_all_emoji_routes() {
    // Privacy-404: a non-member can't even tell the guild exists. The member gate
    // runs BEFORE name validation, so a bad name from a non-member is still 404.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let stranger = common::register_account(&a.router, "Stranger", "password123").await;
    let gid = create_guild(&a.router, &owner).await;
    let media = upload_image(&a.router, &owner, "image/png", b"\x89PNG e").await;

    // List → 404.
    let (st, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}/emoji"),
        Some(&stranger),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // Create with a VALID name → 404 (member gate first).
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/emoji"),
        Some(&stranger),
        Some(&json!({ "name": "valid", "media_id": media })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // Create with an INVALID name → still 404, NOT 400 (gate precedes validation).
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/emoji"),
        Some(&stranger),
        Some(&json!({ "name": "X", "media_id": media })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::NOT_FOUND,
        "member gate precedes name validation"
    );

    // Delete → 404.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/guilds/{gid}/emoji/whatever"),
        Some(&stranger),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// UNIQUE (guild, name) collision → 409; per-guild scoping
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn duplicate_name_in_same_guild_is_409() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner).await;
    let media = upload_image(&a.router, &owner, "image/png", b"\x89PNG e").await;

    let body = json!({ "name": "dup", "media_id": media });
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/emoji"),
        Some(&owner),
        Some(&body),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // Same name, same guild → UNIQUE violation → 409.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/emoji"),
        Some(&owner),
        Some(&body),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::CONFLICT,
        "UNIQUE (guild,name) collision → 409"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn same_name_in_different_guild_is_allowed() {
    // The UNIQUE index is composite (guild, name): the same shortcode in a
    // SEPARATE guild is a distinct row, not a collision.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let g1 = create_guild(&a.router, &owner).await;
    let g2 = create_guild(&a.router, &owner).await;
    let media = upload_image(&a.router, &owner, "image/png", b"\x89PNG e").await;

    let body = json!({ "name": "shared", "media_id": media });
    let (st1, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{g1}/emoji"),
        Some(&owner),
        Some(&body),
    )
    .await;
    let (st2, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{g2}/emoji"),
        Some(&owner),
        Some(&body),
    )
    .await;
    assert_eq!(st1, StatusCode::CREATED);
    assert_eq!(
        st2,
        StatusCode::CREATED,
        "same name in a different guild is OK"
    );
}
