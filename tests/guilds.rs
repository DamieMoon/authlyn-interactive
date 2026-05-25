//! Step-2 integration tests: guild create/list/detail, the membership
//! privacy-404, owner-gated channel creation, and the concurrent-invite race
//! against the `guild_member_pair` UNIQUE index.

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;

#[cfg(feature = "ssr")]
async fn create_guild(router: &axum::Router, cookie: &str, name: &str) -> String {
    let (status, _, body) = common::send(
        router,
        Method::POST,
        "/guilds",
        Some(cookie),
        Some(&json!({ "name": name })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create guild: {body:?}");
    body["id"].as_str().expect("guild id").to_string()
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn create_lists_and_details_with_default_channel() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "My Guild").await;

    let (status, _, body) =
        common::send(&a.router, Method::GET, "/guilds", Some(&owner), None).await;
    assert_eq!(status, StatusCode::OK);
    let guilds = body["guilds"].as_array().unwrap();
    assert_eq!(guilds.len(), 1);
    assert_eq!(guilds[0]["name"], "My Guild");

    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "My Guild");
    let channels = body["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 1, "a fresh guild has one default channel");
    assert_eq!(channels[0]["name"], "general");
    assert_eq!(channels[0]["kind"], "text");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn nonmember_get_guild_is_404() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Secret").await;

    let outsider = common::register_account(&a.router, "Outsider", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&outsider),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "non-members get a privacy 404"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn channel_create_is_owner_gated() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;

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

    // A plain member cannot create channels.
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&member),
        Some(&json!({ "name": "lore", "kind": "lorebook" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The owner can.
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "lore", "kind": "lorebook" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["kind"], "lorebook");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn invite_unknown_user_is_404() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;

    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "ghost" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn concurrent_invite_yields_one_member_row() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;

    // Register the target directly so we can grab its account id for the
    // post-condition DB assertion.
    let (status, _, body) = common::send(
        &a.router,
        Method::POST,
        "/auth/register",
        None,
        Some(&json!({ "username": "Target", "password": "password123" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let target_id = body["account_id"].as_str().unwrap().to_string();

    let url = format!("/guilds/{gid}/members");
    let invite = json!({ "username": "Target" });
    let (r1, r2) = tokio::join!(
        common::send(&a.router, Method::POST, &url, Some(&owner), Some(&invite)),
        common::send(&a.router, Method::POST, &url, Some(&owner), Some(&invite)),
    );
    let statuses = [r1.0, r2.0];
    assert!(
        statuses.contains(&StatusCode::CREATED),
        "exactly one invite should 201: {statuses:?}"
    );
    assert!(
        statuses.contains(&StatusCode::CONFLICT),
        "the racing invite should 409: {statuses:?}"
    );

    // The UNIQUE index must leave exactly one membership row.
    let mut resp =
        a.db.query(
            "SELECT VALUE meta::id(id) FROM guild_member
                WHERE guild = type::record('guild', $gid)
                  AND account = type::record('account', $tid);",
        )
        .bind(("gid", gid.clone()))
        .bind(("tid", target_id.clone()))
        .await
        .unwrap()
        .check()
        .unwrap();
    let ids: Vec<String> = resp.take(0).unwrap();
    assert_eq!(ids.len(), 1, "exactly one guild_member row for the target");
}

/// Register an account, returning `(session_cookie, account_id)`.
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
    assert_eq!(status, StatusCode::CREATED);
    (
        cookie.unwrap(),
        body["account_id"].as_str().unwrap().to_string(),
    )
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn promoting_a_member_to_admin_lets_them_manage() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;
    let (member, member_id) = register_with_id(&a.router, "Member").await;

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/members"),
        Some(&owner),
        Some(&json!({ "username": "Member" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // A plain member can't create channels.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&member),
        Some(&json!({ "name": "x", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Owner grants admin — the easy path to share control.
    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/members/{member_id}/role"),
        Some(&owner),
        Some(&json!({ "role": "admin" })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // Now the (promoted) admin can manage.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&member),
        Some(&json!({ "name": "x", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn plain_member_cannot_grant_admin() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;
    let (member, _) = register_with_id(&a.router, "Member").await;
    let (_, third_id) = register_with_id(&a.router, "Third").await;

    for name in ["Member", "Third"] {
        let (st, _, _) = common::send(
            &a.router,
            Method::POST,
            &format!("/guilds/{gid}/members"),
            Some(&owner),
            Some(&json!({ "username": name })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED);
    }

    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/members/{third_id}/role"),
        Some(&member),
        Some(&json!({ "role": "admin" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn owner_role_cannot_be_changed() {
    let a = common::arena().await;
    let (owner, owner_id) = register_with_id(&a.router, "Owner").await;
    let gid = create_guild(&a.router, &owner, "Guild").await;

    let (st, _, _) = common::send(
        &a.router,
        Method::PUT,
        &format!("/guilds/{gid}/members/{owner_id}/role"),
        Some(&owner),
        Some(&json!({ "role": "member" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}
