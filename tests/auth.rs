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
    // M6/P2: /auth/me carries the live profile — display_name (empty until set)
    // and avatar_id (null until an avatar is set).
    assert_eq!(body["display_name"], "");
    assert!(
        body["avatar_id"].is_null(),
        "no avatar set → avatar_id null"
    );
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
async fn patch_account_updates_display_name() {
    let a = common::arena().await;
    let cookie = common::register_account(&a.router, "Alice", "hunter2hunter2").await;

    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        "/account",
        Some(&cookie),
        Some(&json!({ "display_name": "  Alice the Bold  " })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // /auth/me reflects the trimmed display name.
    let (_, _, body) = common::send(&a.router, Method::GET, "/auth/me", Some(&cookie), None).await;
    assert_eq!(body["display_name"], "Alice the Bold");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn patch_account_rejects_bad_display_name() {
    let a = common::arena().await;
    let cookie = common::register_account(&a.router, "Alice", "hunter2hunter2").await;

    for bad in ["".to_string(), "   ".to_string(), "x".repeat(33)] {
        let (status, _, _) = common::send(
            &a.router,
            Method::PATCH,
            "/account",
            Some(&cookie),
            Some(&json!({ "display_name": &bad })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "display_name {bad:?} must be rejected"
        );
    }
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn patch_account_unknown_avatar_is_404() {
    let a = common::arena().await;
    let cookie = common::register_account(&a.router, "Alice", "hunter2hunter2").await;
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        "/account",
        Some(&cookie),
        Some(&json!({ "avatar": "deadbeefdeadbeefdeadbeefdeadbeef" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown avatar media 404s");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn patch_account_without_cookie_is_401() {
    let a = common::arena().await;
    let (status, _, _) = common::send(
        &a.router,
        Method::PATCH,
        "/account",
        None,
        Some(&json!({ "display_name": "Nope" })),
    )
    .await;
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
async fn change_password_rotates_the_login_credential() {
    let a = common::arena().await;
    let cookie = common::register_account(&a.router, "Erin", "oldpassword1").await;

    // Correct current password → 204, no body.
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        "/auth/change-password",
        Some(&cookie),
        Some(&json!({ "current_password": "oldpassword1", "new_password": "newpassword2" })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The new password now logs in.
    let (new_ok, new_cookie, _) = common::send(
        &a.router,
        Method::POST,
        "/auth/login",
        None,
        Some(&json!({ "username": "Erin", "password": "newpassword2" })),
    )
    .await;
    assert_eq!(new_ok, StatusCode::OK);
    assert!(new_cookie.is_some());

    // The old password no longer works.
    let (old_fail, _, _) = common::send(
        &a.router,
        Method::POST,
        "/auth/login",
        None,
        Some(&json!({ "username": "Erin", "password": "oldpassword1" })),
    )
    .await;
    assert_eq!(old_fail, StatusCode::UNAUTHORIZED);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn change_password_wrong_current_is_rejected() {
    let a = common::arena().await;
    let cookie = common::register_account(&a.router, "Frank", "frankspass1").await;

    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        "/auth/change-password",
        Some(&cookie),
        Some(&json!({ "current_password": "notmypassword", "new_password": "newpassword2" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // The original password is untouched.
    let (login, _, _) = common::send(
        &a.router,
        Method::POST,
        "/auth/login",
        None,
        Some(&json!({ "username": "Frank", "password": "frankspass1" })),
    )
    .await;
    assert_eq!(login, StatusCode::OK);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn change_password_rejects_too_short_new_password() {
    let a = common::arena().await;
    let cookie = common::register_account(&a.router, "Grace", "gracespass1").await;

    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        "/auth/change-password",
        Some(&cookie),
        Some(&json!({ "current_password": "gracespass1", "new_password": "short" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn change_password_requires_authentication() {
    let a = common::arena().await;
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        "/auth/change-password",
        None,
        Some(&json!({ "current_password": "whatever1", "new_password": "newpassword2" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn register_rejects_password_under_8_characters_even_when_8_bytes() {
    // F-D5-2: the length rule must count CHARACTERS (matching its "at least 8
    // characters" message and the username check), not bytes. Three lock emojis
    // are 3 characters but 12 UTF-8 bytes; a byte-based check would wrongly
    // accept them as "8+ characters", so this asserts the char-count gate 400s.
    let a = common::arena().await;
    let (status, _, _) = common::send(
        &a.router,
        Method::POST,
        "/auth/register",
        None,
        Some(&json!({ "username": "Multibyte", "password": "🔒🔒🔒" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[cfg(feature = "ssr")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_register_same_username_never_500s() {
    // F-D6-1: two simultaneous registrations of the same username resolve to
    // exactly one 201 + one 409 — never a 500 from an un-retried MVCC write
    // conflict on the account_username_ci UNIQUE index (with_write_conflict_retry).
    let a = common::arena().await;
    let body = json!({ "username": "Racer", "password": "password123" });
    let req1 = common::build_json_request(Method::POST, "/auth/register", None, Some(&body));
    let req2 = common::build_json_request(Method::POST, "/auth/register", None, Some(&body));
    let h1 = tokio::spawn(common::status_of(a.router.clone(), req1));
    let h2 = tokio::spawn(common::status_of(a.router.clone(), req2));
    let mut got = [h1.await.unwrap(), h2.await.unwrap()];
    assert!(
        !got.contains(&StatusCode::INTERNAL_SERVER_ERROR),
        "register race must never 500: {got:?}"
    );
    got.sort_by_key(|s| s.as_u16());
    assert_eq!(
        got,
        [StatusCode::CREATED, StatusCode::CONFLICT],
        "exactly one 201 and one 409"
    );
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
