//! Wave-1 SAFETY-NET: `POST /media` + `GET /media/{id}` characterization.
//!
//! Locks the security-critical behavior of the media blob store ahead of the
//! refactor waves (audit 019e6c08, invariant #10):
//!   - path-traversal defense in depth: the on-disk path must canonicalize
//!     inside `media_dir` (media.rs:185); a stored path that escapes is
//!     refused, NEVER served;
//!   - URL-level traversal attempts route to a non-existent id → 404;
//!   - MIME round-trip (POST an image, GET it back, Content-Type preserved);
//!   - unknown id → 404;
//!   - `?w=` thumbnail path with width clamp 16–512;
//!   - the documented per-blob NON-ACL: any authed account may fetch any id
//!     (intentional, trusted-friends model — audit "Open Decisions").
//!
//! These are CURRENT-behavior characterizations, not aspirational specs.

mod common;

#[cfg(feature = "ssr")]
use axum::body::{to_bytes, Body};
#[cfg(feature = "ssr")]
use axum::http::{header, Method, Request, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::Value;
#[cfg(feature = "ssr")]
use tower::ServiceExt;

/// A valid 2x1 RGB PNG so `image::load_from_memory` decodes it for the
/// thumbnail path (verified against `image` 0.25, the server's decoder).
/// Generated with correct chunk CRCs; a hand-rolled bad-CRC PNG makes
/// `make_thumb` fall back to serving the original, which would silently mask
/// the thumbnail branch under test.
#[cfg(feature = "ssr")]
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // signature
    0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR length + type
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, // 2x1
    0x08, 0x02, 0x00, 0x00, 0x00, 0x7B, 0x40, 0xE8, // 8-bit RGB + CRC
    0xDD, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, // IDAT length + type
    0x54, 0x78, 0xDA, 0x63, 0xF8, 0xCF, 0xC0, 0x00, // zlib stream
    0x44, 0x00, 0x08, 0xFE, 0x01, 0xFF, 0x19, 0xC0, // ...
    0x6B, 0xE7, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, // IEND
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// Upload a blob via multipart `POST /media`, asserting 201 and returning the
/// new media id. Mirrors `tests/personas.rs::upload_image`.
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
    assert_eq!(res.status(), StatusCode::CREATED, "media upload should 201");
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    v["id"].as_str().unwrap().to_string()
}

/// GET `/media/{path}` with a cookie; returns (status, content-type, body bytes).
#[cfg(feature = "ssr")]
async fn get_media(
    router: &axum::Router,
    cookie: &str,
    path_and_query: &str,
) -> (StatusCode, Option<String>, Vec<u8>) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/media/{path_and_query}"))
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    let res = router.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    let ct = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap().to_vec();
    (status, ct, bytes)
}

/// Attempt `POST /media` and return only the status (no 201 assertion), for
/// exercising rejection paths.
#[cfg(feature = "ssr")]
async fn try_upload_status(
    router: &axum::Router,
    cookie: &str,
    mime: &str,
    data: &[u8],
) -> StatusCode {
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
    router.clone().oneshot(req).await.expect("oneshot").status()
}

/// GET `/media/{path}` returning the full header map (for asserting security
/// headers like nosniff / Content-Disposition).
#[cfg(feature = "ssr")]
async fn get_media_full(
    router: &axum::Router,
    cookie: &str,
    path_and_query: &str,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/media/{path_and_query}"))
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    let res = router.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap().to_vec();
    (status, headers, bytes)
}

// ---------------------------------------------------------------------------
// MIME round-trip + the documented per-blob non-ACL
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn upload_then_download_round_trips_bytes_and_mime() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;

    let payload = b"\x89PNG\r\n\x1a\nfake-png-body".to_vec();
    let id = upload_image(&a.router, &owner, "image/png", &payload).await;

    let (status, ct, bytes) = get_media(&a.router, &owner, &id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        ct.as_deref(),
        Some("image/png"),
        "stored MIME is served back"
    );
    assert_eq!(bytes, payload, "exact bytes round-trip");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn any_authed_account_may_fetch_any_media_id() {
    // Documented INTENTIONAL non-invariant (audit "Open Decisions"): there is no
    // per-blob ACL — `download_media` ignores the caller (media.rs:164). A second,
    // unrelated account (not the uploader, sharing nothing) can fetch the blob.
    let a = common::arena().await;
    let uploader = common::register_account(&a.router, "Uploader", "password123").await;
    let stranger = common::register_account(&a.router, "Stranger", "password123").await;

    let id = upload_image(&a.router, &uploader, "image/png", b"\x89PNG secret art").await;

    let (status, ct, bytes) = get_media(&a.router, &stranger, &id).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "any authenticated account may fetch any media id (trusted-friends model)"
    );
    assert_eq!(ct.as_deref(), Some("image/png"));
    assert_eq!(bytes, b"\x89PNG secret art");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn unauthenticated_fetch_is_rejected() {
    // The non-ACL is "any *authed* account"; the route still requires a session.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let id = upload_image(&a.router, &owner, "image/png", b"\x89PNG body").await;

    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("/media/{id}"))
        .body(Body::empty())
        .unwrap();
    let res = a.router.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "no session cookie → 401"
    );
}

// ---------------------------------------------------------------------------
// 404 on unknown id
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn unknown_media_id_is_404() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;

    // A well-formed-but-nonexistent id (no such media_blob row).
    let (status, _, _) = get_media(&a.router, &owner, "deadbeefdeadbeefdeadbeefdeadbeef").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Path-traversal defenses (invariant #10)
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn url_path_traversal_attempt_does_not_escape() {
    // The route is `GET /media/{id}` — a single path segment. A percent-encoded
    // traversal payload is captured verbatim as the `id` and looked up as a
    // media_blob record key; no such row exists, so it 404s (and crucially never
    // reads an on-disk path the attacker chose). We assert it is NEVER 200 and
    // NEVER serves bytes from outside the store.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;

    for attempt in [
        "..%2f..%2f..%2fetc%2fpasswd",
        "%2e%2e%2f%2e%2e%2fetc%2fpasswd",
        "....%2f....%2fetc%2fpasswd",
        "%2e%2e",
    ] {
        let (status, _ct, body) = get_media(&a.router, &owner, attempt).await;
        assert_ne!(
            status,
            StatusCode::OK,
            "traversal attempt {attempt:?} must not succeed"
        );
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::BAD_REQUEST,
            "traversal attempt {attempt:?} should 404/400, got {status}"
        );
        // Defense sanity: never leak a system file's contents.
        assert!(
            !body.windows(5).any(|w| w == b"root:"),
            "traversal attempt {attempt:?} leaked file contents"
        );
    }
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn stored_path_outside_media_dir_is_refused() {
    // Directly exercises the security-critical guard at media.rs:185. We forge a
    // media_blob row whose `storage_path` escapes `media_dir` (a real, readable
    // system file outside the store). The `canonical.starts_with(media_dir)`
    // check must REFUSE it — never serve the file. Current behavior: that branch
    // returns 500 "storage error" (it is an internal invariant violation, not a
    // client error), so we assert "not 200, no leaked bytes".
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;

    // Write a real, readable file OUTSIDE media_dir: a sibling of the per-test
    // media tempdir (its canonical parent), so `canonical.starts_with(media_dir)`
    // is false. Cross-platform and deterministic (no reliance on a system path).
    let parent = a
        .media_dir
        .parent()
        .expect("media_dir has a parent")
        .to_path_buf();
    let escape_target = parent.join(format!("authlyn-escape-{}.txt", common::random_id()));
    let secret = b"SECRET-OUTSIDE-MEDIA-DIR";
    std::fs::write(&escape_target, secret).expect("write escape target");
    // Canonicalize for the stored path so it matches what `download_media`
    // canonicalizes to (avoids /var vs /private/var symlink mismatch on macOS).
    let escape_canonical = escape_target.canonicalize().expect("canonicalize target");
    assert!(
        !escape_canonical.starts_with(a.media_dir.as_path()),
        "escape target must live outside media_dir"
    );

    let forged_id = "forgedescape00000000000000000000";
    a.db.query(
        r#"CREATE type::record("media_blob", $id) SET
                uploader     = type::record("account", $uploader),
                mime         = "text/plain",
                size_bytes   = 1,
                storage_path = $path;"#,
    )
    .bind(("id", forged_id.to_string()))
    .bind(("uploader", owner_account_id(&a.router, &owner).await))
    .bind(("path", escape_canonical.to_string_lossy().to_string()))
    .await
    .expect("forge media_blob row")
    .check()
    .expect("forge check");

    let (status, _ct, body) = get_media(&a.router, &owner, forged_id).await;
    let _ = std::fs::remove_file(&escape_target);
    assert_ne!(
        status,
        StatusCode::OK,
        "a stored path escaping media_dir must NOT be served"
    );
    // The escape guard returns 500 (internal invariant violation, not a client
    // error). Lock that exact status.
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "escape guard (media.rs:185) returns 500 'storage error'"
    );
    // The file's contents must never reach the client.
    assert_ne!(
        body.as_slice(),
        secret,
        "media route leaked a file outside media_dir"
    );
}

/// Resolve the caller's own account id (uploader for the forged row) via /auth/me.
#[cfg(feature = "ssr")]
async fn owner_account_id(router: &axum::Router, cookie: &str) -> String {
    let (status, _, body) = common::send(router, Method::GET, "/auth/me", Some(cookie), None).await;
    assert_eq!(status, StatusCode::OK, "/auth/me should 200 for a session");
    body["account_id"]
        .as_str()
        .expect("/auth/me must carry account_id")
        .to_string()
}

// ---------------------------------------------------------------------------
// Thumbnail `?w=` path (width clamp 16–512)
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn thumbnail_query_serves_jpeg_for_image_blob() {
    // `?w=N` on an image blob serves a downscaled JPEG (media.rs:193). Decodable
    // input → Content-Type image/jpeg regardless of the source PNG.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let id = upload_image(&a.router, &owner, "image/png", TINY_PNG).await;

    let (status, ct, bytes) = get_media(&a.router, &owner, &format!("{id}?w=64")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        ct.as_deref(),
        Some("image/jpeg"),
        "thumbnail re-encodes to JPEG"
    );
    assert!(!bytes.is_empty(), "thumbnail body present");
    // It is a real JPEG (SOI marker).
    assert_eq!(&bytes[..2], &[0xFF, 0xD8], "JPEG SOI marker");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn thumbnail_width_is_clamped_low_and_high() {
    // Width clamps to [16, 512]; out-of-range values still produce a valid JPEG
    // (clamped silently) rather than erroring. We characterize that both an
    // absurdly-small and an absurdly-large width succeed and yield JPEG.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let id = upload_image(&a.router, &owner, "image/png", TINY_PNG).await;

    for w in ["1", "100000"] {
        let (status, ct, bytes) = get_media(&a.router, &owner, &format!("{id}?w={w}")).await;
        assert_eq!(status, StatusCode::OK, "w={w} should succeed (clamped)");
        assert_eq!(ct.as_deref(), Some("image/jpeg"), "w={w} yields JPEG");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "w={w} JPEG SOI");
    }
}

// ---------------------------------------------------------------------------
// Stored-XSS hardening (review F-D8-3): image-only uploads + safe serving
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn upload_rejects_non_image_mimes() {
    // The store is image-only. Non-image — and script-capable image/svg+xml —
    // uploads are refused (415) so attacker-controlled active content can never
    // be stored and later served from our origin.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;

    for mime in [
        "text/html",
        "application/pdf",
        "image/svg+xml",
        "application/octet-stream",
    ] {
        let status =
            try_upload_status(&a.router, &owner, mime, b"<script>alert(1)</script>").await;
        assert_eq!(
            status,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{mime} upload must be rejected"
        );
    }

    // A real raster image is still accepted.
    let status = try_upload_status(&a.router, &owner, "image/png", TINY_PNG).await;
    assert_eq!(status, StatusCode::CREATED, "image/png is allowed");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn served_image_carries_nosniff() {
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;
    let id = upload_image(&a.router, &owner, "image/png", TINY_PNG).await;

    let (status, headers, _) = get_media_full(&a.router, &owner, &id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("image/png"),
    );
    assert_eq!(
        headers
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|v| v.to_str().ok()),
        Some("nosniff"),
        "served image must carry X-Content-Type-Options: nosniff"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn legacy_non_image_blob_is_served_as_nosniff_attachment() {
    // Uploads are now image-only, but a row predating that check (or any
    // non-image mime) must be neutralized on serve: forced to an octet-stream
    // attachment with nosniff so stored text/html can never execute on our
    // origin. We forge such a row whose file lives INSIDE media_dir so the path
    // guard passes and it is actually served.
    let a = common::arena().await;
    let owner = common::register_account(&a.router, "Owner", "password123").await;

    let id = "legacyhtml0000000000000000000000";
    let file_path = a.media_dir.join(format!("{id}.bin"));
    let html = b"<script>alert(document.domain)</script>";
    std::fs::write(&file_path, html).expect("write legacy blob");
    let canonical = file_path.canonicalize().expect("canonicalize");
    a.db.query(
        r#"CREATE type::record("media_blob", $id) SET
                uploader     = type::record("account", $uploader),
                mime         = "text/html",
                size_bytes   = 1,
                storage_path = $path;"#,
    )
    .bind(("id", id.to_string()))
    .bind(("uploader", owner_account_id(&a.router, &owner).await))
    .bind(("path", canonical.to_string_lossy().to_string()))
    .await
    .expect("forge row")
    .check()
    .expect("forge check");

    // `?w=` is ignored for a non-image; bytes round-trip but as a safe download.
    let (status, headers, bytes) = get_media_full(&a.router, &owner, &format!("{id}?w=64")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/octet-stream"),
        "non-image must NOT be served as its stored active-content type"
    );
    assert_eq!(
        headers
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|v| v.to_str().ok()),
        Some("nosniff"),
    );
    assert_eq!(
        headers
            .get(header::CONTENT_DISPOSITION)
            .and_then(|v| v.to_str().ok()),
        Some("attachment"),
        "non-image must be a download, never inline"
    );
    assert_eq!(bytes, html, "bytes still round-trip (as a download)");
}
