//! L-1 cross-device read state: `POST /channels/{cid}/mark-read` +
//! `GET /channels/read-state`. Persists the caller's per-channel last-seen
//! `(sent_at, id)` cursor server-side so read/unread syncs across devices.
//!
//! Covered:
//! - `mark_read_requires_channel_membership` — a non-member gets a privacy-404,
//!   never a leak that the channel exists.
//! - `mark_read_upsert_keeps_max_cursor` — an OLDER POST does not regress a
//!   newer stored mark (the MAX-cursor rule), but a newer one advances it.
//! - `read_state_round_trips` — a mark written via POST comes back through GET
//!   with the same channel id + cursor.
//! - `mark_read_unknown_channel_404` — an entirely unknown channel id is a
//!   privacy-404 (identical body to the non-member case).

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;

/// Register an owner, create a guild, send a message, and return
/// `(owner_cookie, guild_id, default_text_channel_id, sent_at, message_id)`.
#[cfg(feature = "ssr")]
async fn owner_channel_with_message(
    router: &axum::Router,
) -> (String, String, String, String, String) {
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

    let (status, _, msg) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "first" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let mid = msg["id"].as_str().unwrap().to_string();

    // Read the message back to learn its server-formatted `sent_at` cursor key.
    let (status, _, list) = common::send(
        router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let m0 = &list["messages"][0];
    let sent_at = m0["sent_at"].as_str().unwrap().to_string();
    assert_eq!(m0["id"].as_str().unwrap(), mid);

    (owner, gid, cid, sent_at, mid)
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn mark_read_requires_channel_membership() {
    let a = common::arena().await;
    let (_owner, _gid, cid, sent_at, mid) = owner_channel_with_message(&a.router).await;

    // A second account that is NOT a member of the guild this channel belongs to.
    let outsider = common::register_account(&a.router, "Outsider", "password123").await;
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/mark-read"),
        Some(&outsider),
        Some(&json!({ "sent_at": sent_at, "id": mid })),
    )
    .await;
    // Privacy-404: existence is never leaked to a non-member.
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"].as_str().unwrap(), "channel not found");

    // And the outsider's read-state list stays empty (no row was written).
    let (status, _, rs) = common::send(
        &a.router,
        Method::GET,
        "/channels/read-state",
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(rs["cursors"].as_array().unwrap().is_empty());
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn mark_read_unknown_channel_404() {
    let a = common::arena().await;
    let (owner, _gid, _cid, _sent_at, _mid) = owner_channel_with_message(&a.router).await;

    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        "/channels/doesnotexist/mark-read",
        Some(&owner),
        Some(&json!({ "sent_at": "2026-05-22T12:00:00.000000000Z", "id": "nope" })),
    )
    .await;
    // Identical body to the non-member case — never reveal existence.
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"].as_str().unwrap(), "channel not found");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn read_state_round_trips() {
    let a = common::arena().await;
    let (owner, _gid, cid, sent_at, mid) = owner_channel_with_message(&a.router).await;

    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/mark-read"),
        Some(&owner),
        Some(&json!({ "sent_at": sent_at, "id": mid })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _, rs) = common::send(
        &a.router,
        Method::GET,
        "/channels/read-state",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cursors = rs["cursors"].as_array().unwrap();
    assert_eq!(cursors.len(), 1, "exactly one stored cursor");
    let c = &cursors[0];
    assert_eq!(c["channel_id"].as_str().unwrap(), cid);
    assert_eq!(c["sent_at"].as_str().unwrap(), sent_at);
    assert_eq!(c["id"].as_str().unwrap(), mid);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn mark_read_upsert_keeps_max_cursor() {
    let a = common::arena().await;
    let (owner, _gid, cid, sent_at, mid) = owner_channel_with_message(&a.router).await;

    // Advance to a clearly-newer cursor first.
    let newer = "2026-12-31T23:59:59.000000000Z";
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/mark-read"),
        Some(&owner),
        Some(&json!({ "sent_at": newer, "id": "zzz" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Now POST an OLDER cursor — it must NOT regress the stored mark.
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/mark-read"),
        Some(&owner),
        Some(&json!({ "sent_at": sent_at, "id": mid })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "older POST is accepted (no-op), not an error"
    );

    let (status, _, rs) = common::send(
        &a.router,
        Method::GET,
        "/channels/read-state",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let cursors = rs["cursors"].as_array().unwrap();
    assert_eq!(
        cursors.len(),
        1,
        "still exactly one row (UPSERT, not duplicate)"
    );
    let c = &cursors[0];
    assert_eq!(
        c["sent_at"].as_str().unwrap(),
        newer,
        "older POST must not regress the newer stored cursor"
    );
    assert_eq!(c["id"].as_str().unwrap(), "zzz");
}
