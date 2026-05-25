//! Step-1 integration tests: registration, login, sessions, the AuthAccount
//! extractor, and logout. Each test owns its own SurrealDB arena (namespace),
//! so usernames can collide freely across tests.

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;

#[cfg(feature = "ssr")]
#[tokio::test]
async fn register_sets_cookie_and_me_resolves_it() {
    let a = common::arena().await;
    let cookie = common::register_account(&a.router, "Alice", "hunter2hunter2").await;

    let (status, _, body) =
        common::send(&a.router, Method::GET, "/auth/me", Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["username"], "Alice");
    assert!(body["account_id"].is_string());
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn me_without_cookie_is_401() {
    let a = common::arena().await;
    let (status, _, _) = common::send(&a.router, Method::GET, "/auth/me", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn me_with_garbage_cookie_is_401() {
    let a = common::arena().await;
    let (status, _, _) = common::send(
        &a.router,
        Method::GET,
        "/auth/me",
        Some("authlyn_session=deadbeefnotarealtoken"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn duplicate_username_is_409_case_insensitive() {
    let a = common::arena().await;
    let _ = common::register_account(&a.router, "Bob", "correcthorse").await;

    // Different case, same case-insensitive key → must collide.
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        "/auth/register",
        None,
        Some(&json!({ "username": "BOB", "password": "anotherpassword" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn login_good_and_bad_credentials() {
    let a = common::arena().await;
    let _ = common::register_account(&a.router, "Carol", "swordfish99").await;

    // Good credentials → 200 + a fresh session cookie.
    let (status, cookie, _) = common::send(
        &a.router,
        Method::POST,
        "/auth/login",
        None,
        Some(&json!({ "username": "Carol", "password": "swordfish99" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(cookie.is_some(), "login must set a session cookie");

    // Wrong password and unknown user both 401 (identical body, no enumeration).
    let (wrong_pw, _, body_pw) = common::send(
        &a.router,
        Method::POST,
        "/auth/login",
        None,
        Some(&json!({ "username": "Carol", "password": "wrong" })),
    )
    .await;
    let (unknown, _, body_unknown) = common::send(
        &a.router,
        Method::POST,
        "/auth/login",
        None,
        Some(&json!({ "username": "Nobody", "password": "whatever12" })),
    )
    .await;
    assert_eq!(wrong_pw, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown, StatusCode::UNAUTHORIZED);
    assert_eq!(body_pw, body_unknown, "401 bodies must be identical");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn logout_invalidates_the_session() {
    let a = common::arena().await;
    let cookie = common::register_account(&a.router, "Dave", "battery-staple").await;

    // Sanity: the session works before logout.
    let (before, _, _) =
        common::send(&a.router, Method::GET, "/auth/me", Some(&cookie), None).await;
    assert_eq!(before, StatusCode::OK);

    let (logout, _, _) =
        common::send(&a.router, Method::POST, "/auth/logout", Some(&cookie), None).await;
    assert_eq!(logout, StatusCode::NO_CONTENT);

    // The same cookie is now dead (session row deleted).
    let (after, _, _) = common::send(&a.router, Method::GET, "/auth/me", Some(&cookie), None).await;
    assert_eq!(after, StatusCode::UNAUTHORIZED);
}
