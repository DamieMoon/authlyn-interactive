//! M5/P2: guild.accent_color over the REST surface — manager-gated write,
//! palette validation (400 on junk), round-trips through GET /guilds/{id}.
//! Mirrors tests/guilds.rs's harness use (common::arena / register_account /
//! send) — no new common helpers.
#![cfg(feature = "ssr")]

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;

/// Create a guild as `cookie`, returning its id (mirrors tests/guilds.rs).
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

#[tokio::test]
async fn patch_guild_accent_is_manager_gated_and_palette_validated() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "owner-a", "password123").await;
    let gid = create_guild(&a.router, &owner, "Accent Guild").await;

    // 1. A valid palette accent is accepted (204) and reads back on the detail.
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}"),
        Some(&owner),
        Some(&json!({ "accent_color": "purple" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "owner setting a valid accent must be 204"
    );
    let (status, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["accent_color"], "purple",
        "accent must round-trip on GET /guilds/{{id}}"
    );

    // 2. Empty clears it back to the default.
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}"),
        Some(&owner),
        Some(&json!({ "accent_color": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/guilds/{gid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        body["accent_color"], "",
        "empty accent clears back to default"
    );

    // 3. An out-of-palette value is rejected with 400 (server/accent.rs gate).
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}"),
        Some(&owner),
        Some(&json!({ "accent_color": "chartreuse" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "out-of-palette accent must be 400"
    );

    // 4. A non-member cannot set it — the privacy-404 (resolve_membership) /
    //    require_manager gate. (A registered intruder is never a member of gid.)
    let intruder = common::register_account(&a.router, "intruder", "password123").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/guilds/{gid}"),
        Some(&intruder),
        Some(&json!({ "accent_color": "red" })),
    )
    .await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
        "a non-manager must not set the accent (privacy-404 or 403), got {status}"
    );
}
