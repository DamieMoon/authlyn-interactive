//! Wave-1 SAFETY-NET (optional): the no-store cache layer (server/mod.rs:179).
//!
//! Dynamic JSON API responses must never be cached (a cached message list once
//! flashed ancient messages on cold open). The `map_response` layer stamps
//! `Cache-Control: no-store` on the small-body (JSON) route group. This locks
//! that header onto API responses. It also characterizes that the media route
//! group — a SEPARATE router without that layer — is never stamped no-store
//! (it carries its own immutable Cache-Control instead, set per-response in
//! media.rs), so a refactor can't silently move/drop the layer unnoticed.

mod common;

#[cfg(feature = "ssr")]
use axum::body::Body;
#[cfg(feature = "ssr")]
use axum::http::{header, Method, Request, StatusCode};
#[cfg(feature = "ssr")]
use tower::ServiceExt;

/// GET `path`, returning (status, Cache-Control header value).
#[cfg(feature = "ssr")]
async fn cache_control_of(
    router: &axum::Router,
    cookie: Option<&str>,
    path: &str,
) -> (StatusCode, Option<String>) {
    let mut b = Request::builder().method(Method::GET).uri(path);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    let res = router
        .clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .expect("oneshot");
    let status = res.status();
    let cc = res
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    (status, cc)
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn json_api_responses_are_no_store() {
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;

    // A successful authed JSON GET.
    let (st, cc) = cache_control_of(&a.router, Some(&user), "/guilds").await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        cc.as_deref(),
        Some("no-store"),
        "JSON API responses carry Cache-Control: no-store"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn no_store_applies_even_to_error_responses() {
    // The layer is a blanket map_response, so it stamps no-store on a 401 too
    // (the SW/browser must not cache an auth error either).
    let a = common::arena().await;
    let (st, cc) = cache_control_of(&a.router, None, "/guilds").await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert_eq!(
        cc.as_deref(),
        Some("no-store"),
        "error responses on the JSON group are also no-store"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn media_route_group_is_not_no_store() {
    // The media routes are a SEPARATE Router (server/mod.rs::media_routes) without
    // the no-store layer — so blobs remain cacheable. Characterize the current
    // split so a refactor that merges the groups can't silently change caching.
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;

    // Upload then GET a blob; assert the media response is NOT no-store.
    let boundary = "Xbnd";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"i\"\r\n\
         Content-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(b"\x89PNG body");
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let res = a
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/media")
                .header(header::COOKIE, &user)
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let id = v["id"].as_str().unwrap();

    let (st, cc) = cache_control_of(&a.router, Some(&user), &format!("/media/{id}")).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        cc.as_deref(),
        Some("private, max-age=31536000, immutable"),
        "media responses carry their own immutable-but-PRIVATE Cache-Control \
         (session-gated, so never `public` — review M-29), never the JSON \
         group's no-store (separate route group)"
    );
}
