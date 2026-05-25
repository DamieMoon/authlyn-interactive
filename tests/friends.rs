//! Step-6 integration tests: friend request -> pending -> accept, the
//! reverse-request auto-accept, duplicate-request 409, unfriend, and the
//! self / unknown-user guards.

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::{json, Value};

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
fn has_user(list: &Value, username: &str) -> bool {
    list.as_array()
        .unwrap()
        .iter()
        .any(|x| x["username"] == username)
}

#[cfg(feature = "ssr")]
async fn friends_of(router: &axum::Router, cookie: &str) -> Value {
    let (status, _, body) = common::send(router, Method::GET, "/friends", Some(cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    body
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn request_pending_then_accept() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, _bob_id) = register_with_id(&a.router, "Bob").await;

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&alice),
        Some(&json!({ "username": "Bob" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // Alice sees Bob as outgoing; Bob sees Alice as incoming.
    let af = friends_of(&a.router, &alice).await;
    assert!(has_user(&af["outgoing"], "Bob"));
    assert!(af["friends"].as_array().unwrap().is_empty());
    let bf = friends_of(&a.router, &bob).await;
    assert!(has_user(&bf["incoming"], "Alice"));

    // Bob accepts.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/friends/{alice_id}/accept"),
        Some(&bob),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // Both now list each other as friends, with no pending.
    let af = friends_of(&a.router, &alice).await;
    assert!(has_user(&af["friends"], "Bob"));
    assert!(af["outgoing"].as_array().unwrap().is_empty());
    let bf = friends_of(&a.router, &bob).await;
    assert!(has_user(&bf["friends"], "Alice"));
    assert!(bf["incoming"].as_array().unwrap().is_empty());
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn reverse_request_auto_accepts() {
    let a = common::arena().await;
    let (alice, _) = register_with_id(&a.router, "Alice").await;
    let (bob, _) = register_with_id(&a.router, "Bob").await;

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&alice),
        Some(&json!({ "username": "Bob" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // Bob requesting Alice back accepts the existing request (200, not 201).
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&bob),
        Some(&json!({ "username": "Alice" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    assert!(has_user(
        &friends_of(&a.router, &alice).await["friends"],
        "Bob"
    ));
    assert!(has_user(
        &friends_of(&a.router, &bob).await["friends"],
        "Alice"
    ));
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn duplicate_request_is_409() {
    let a = common::arena().await;
    let (alice, _) = register_with_id(&a.router, "Alice").await;
    let (_bob, _) = register_with_id(&a.router, "Bob").await;

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&alice),
        Some(&json!({ "username": "Bob" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&alice),
        Some(&json!({ "username": "Bob" })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn unfriend_removes_the_relationship() {
    let a = common::arena().await;
    let (alice, alice_id) = register_with_id(&a.router, "Alice").await;
    let (bob, bob_id) = register_with_id(&a.router, "Bob").await;

    common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&alice),
        Some(&json!({ "username": "Bob" })),
    )
    .await;
    common::send(
        &a.router,
        Method::POST,
        &format!("/friends/{alice_id}/accept"),
        Some(&bob),
        None,
    )
    .await;

    // Alice unfriends Bob.
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/friends/{bob_id}"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    assert!(friends_of(&a.router, &alice).await["friends"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(friends_of(&a.router, &bob).await["friends"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn self_and_unknown_user_are_rejected() {
    let a = common::arena().await;
    let (alice, _) = register_with_id(&a.router, "Alice").await;

    let (self_req, _, _) = common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&alice),
        Some(&json!({ "username": "Alice" })),
    )
    .await;
    assert_eq!(self_req, StatusCode::BAD_REQUEST);

    let (unknown, _, _) = common::send(
        &a.router,
        Method::POST,
        "/friends",
        Some(&alice),
        Some(&json!({ "username": "ghost" })),
    )
    .await;
    assert_eq!(unknown, StatusCode::NOT_FOUND);
}
