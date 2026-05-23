//! Integration tests for `POST /media` and `GET /media/{id}`
//! (routing-plan step 9).
//!
//! These hit a real SurrealDB. Run `./scripts/dev-db.sh` first. Each test
//! reserves a fresh namespace/database AND a fresh on-disk media tempdir
//! (via `tests/common::arena`) so concurrent runs don't collide on either
//! axis.
//!
//! The headline test (`one_mib_round_trip`) drives the full
//! encrypt → upload → download → decrypt → SHA-256 + plaintext-match
//! chain the routing plan calls out. The negative tests pin the typed
//! 4xxs the handler module documents in its validation table.

#![cfg(feature = "ssr")]

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use base64::engine::general_purpose::STANDARD_NO_PAD as B64_NP;
use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};
use surrealdb::engine::remote::ws::Client;
use surrealdb::Surreal;
use tower::ServiceExt;

use authlyn_interactive::crypto::attachment;
use authlyn_interactive::protocol::MediaUploadResponse;

mod common;
use common::{arena, get_json, random_id};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Seed a fresh `(user, device)` pair via raw CREATE statements. Mirrors
/// `tests/messages.rs::seed_user_and_device` — these tests don't need
/// real crypto material on the device, just a row the auth stub can
/// resolve via `X-Device-Id`.
async fn seed_user_and_device(db: &Surreal<Client>) -> (String, String) {
    let user_id = random_id();
    let device_id = random_id();
    db.query(
        "CREATE type::record('user', $user_id) SET display_name = '';
         CREATE type::record('device', $device_id)
             SET user = type::record('user', $user_id),
                 identity_curve25519 = $hex,
                 identity_ed25519    = $hex;",
    )
    .bind(("user_id", user_id.clone()))
    .bind(("device_id", device_id.clone()))
    .bind(("hex", "00".repeat(32)))
    .await
    .expect("seed user+device")
    .check()
    .expect("seed user+device check");
    (user_id, device_id)
}

/// Build a multipart/form-data body with the requested fields. Pure
/// byte-assembly — no async, no dependency beyond `Vec<u8>` — so we can
/// drive the multipart handler at the wire level without pulling in a
/// multipart-encoding crate just for tests.
fn build_multipart(boundary: &str, fields: &[(&str, &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(b"--");
        body.extend_from_slice(boundary.as_bytes());
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(b"Content-Disposition: form-data; name=\"");
        body.extend_from_slice(name.as_bytes());
        body.extend_from_slice(b"\"\r\n");
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(value);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");
    body
}

/// POST a multipart body to the router. Returns (status, body bytes).
/// Body is left as raw bytes (not parsed) so each test can decide how to
/// interpret it — JSON for the success/error responses, opaque bytes
/// for the GET reply.
async fn post_multipart(
    router: &Router,
    path: &str,
    headers: &[(&str, &str)],
    boundary: &str,
    body: Vec<u8>,
) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder().method(Method::POST).uri(path).header(
        header::CONTENT_TYPE,
        format!("multipart/form-data; boundary={}", boundary),
    );
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let req = builder.body(Body::from(body)).unwrap();

    let res = router.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    // 2 MiB buffer — well above the 1 MiB happy-path test, well below
    // the 16 MiB body cap; saturates if a test ever grows past 1 MiB.
    let bytes = to_bytes(res.into_body(), 2 * 1024 * 1024)
        .await
        .expect("read body");
    (status, bytes.to_vec())
}

/// GET a raw-byte response (used for the ciphertext fetch — the
/// download handler returns `application/octet-stream`, not JSON).
async fn get_bytes(router: &Router, path: &str, headers: &[(&str, &str)]) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder().method(Method::GET).uri(path);
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    let req = builder.body(Body::empty()).unwrap();
    let res = router.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 2 * 1024 * 1024)
        .await
        .expect("read body");
    (status, bytes.to_vec())
}

const BOUNDARY: &str = "----authlyn-test-boundary-9c3f-";

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

/// Test 1 — the routing-plan acceptance test, end to end.
///
/// Generate 1 MiB of random plaintext. Encrypt via
/// `crypto::attachment::encrypt`. POST the ciphertext as multipart.
/// GET it back. Decrypt with the original key/iv/sha256. Assert that:
///
/// 1. The downloaded bytes match the uploaded ciphertext byte-for-byte
///    (the server is a dumb relay — no transformation).
/// 2. SHA-256 of the downloaded bytes matches the recorded hash (no
///    silent truncation or padding).
/// 3. Decryption yields the original plaintext (the full crypto
///    contract).
#[tokio::test]
async fn one_mib_round_trip() {
    let a = arena().await;
    let (_user, device) = seed_user_and_device(&a.db).await;

    // 1 MiB of random plaintext.
    let mut plaintext = vec![0u8; 1024 * 1024];
    rand::thread_rng().fill(&mut plaintext[..]);

    let att = attachment::encrypt(&plaintext);
    let body = build_multipart(BOUNDARY, &[("ciphertext", &att.ciphertext)]);
    let (status, body_bytes) = post_multipart(
        &a.router,
        "/media",
        &[("X-Device-Id", &device)],
        BOUNDARY,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "upload failed: {body_bytes:?}");

    let resp: MediaUploadResponse =
        serde_json::from_slice(&body_bytes).expect("MediaUploadResponse parse");
    assert!(!resp.id.is_empty());

    // GET the blob back.
    let path = format!("/media/{}", resp.id);
    let (status, downloaded) = get_bytes(&a.router, &path, &[("X-Device-Id", &device)]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        downloaded, att.ciphertext,
        "downloaded ciphertext must match uploaded byte-for-byte"
    );

    // Explicit SHA-256 assertion against the recorded hash. The hash
    // check is also internal to `attachment::decrypt`, but pinning it
    // here means a refactor that drops the internal verification can't
    // green this spec-acceptance test. Equality of base64-unpadded
    // strings is the same shape Matrix peers will compare against.
    let computed_hash = B64_NP.encode(Sha256::digest(&downloaded));
    assert_eq!(
        computed_hash, att.sha256,
        "downloaded ciphertext must hash to the recorded sha256"
    );

    // Decrypt and compare.
    let recovered =
        attachment::decrypt(&downloaded, &att.key, &att.iv, &att.sha256).expect("decrypt succeeds");
    assert_eq!(recovered, plaintext);
}

/// Test 2 — smoke happy path on a tiny payload, to make sure the
/// multipart framing logic isn't gated on body size.
#[tokio::test]
async fn small_payload_round_trip() {
    let a = arena().await;
    let (_user, device) = seed_user_and_device(&a.db).await;

    let att = attachment::encrypt(b"hello world");
    let body = build_multipart(BOUNDARY, &[("ciphertext", &att.ciphertext)]);
    let (status, body_bytes) = post_multipart(
        &a.router,
        "/media",
        &[("X-Device-Id", &device)],
        BOUNDARY,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "upload failed: {body_bytes:?}");
    let resp: MediaUploadResponse = serde_json::from_slice(&body_bytes).unwrap();

    let path = format!("/media/{}", resp.id);
    let (status, bytes) = get_bytes(&a.router, &path, &[("X-Device-Id", &device)]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, att.ciphertext);
}

// ---------------------------------------------------------------------------
// Auth gate
// ---------------------------------------------------------------------------

/// Test 3 — POST without an `X-Device-Id` header is rejected with 401
/// AND the body matches the handler's typed error message.
///
/// Pinning the message string discriminates *this* 401 from an
/// upstream-layer 401 (e.g. a regression where `RequestBodyLimitLayer`
/// or some auth middleware short-circuits before
/// [`server::media::upload_media`] runs). Without the body assertion,
/// any future 401-emitting layer would silently pass this test.
#[tokio::test]
async fn post_without_device_header_is_401() {
    let a = arena().await;
    let body = build_multipart(BOUNDARY, &[("ciphertext", b"x")]);
    let (status, body_bytes) = post_multipart(&a.router, "/media", &[], BOUNDARY, body).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_error_body(&body_bytes, "missing X-Device-Id header");
}

/// Test 4 — POST with an `X-Device-Id` that doesn't resolve to a device
/// row is 401 with the handler's `"unknown caller device"` message.
/// Mirrors the messages/keyshare/rooms shape; the body-string pin
/// distinguishes this 401 from the missing-header 401.
#[tokio::test]
async fn post_with_unknown_device_is_401() {
    let a = arena().await;
    let body = build_multipart(BOUNDARY, &[("ciphertext", b"x")]);
    let (status, body_bytes) = post_multipart(
        &a.router,
        "/media",
        &[("X-Device-Id", "ghost-device-id")],
        BOUNDARY,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_error_body(&body_bytes, "unknown caller device");
}

/// Test 5 — GET without `X-Device-Id` is 401 with the
/// `"missing X-Device-Id header"` body, even for an existing blob.
/// This is the leaked-URL-isn't-a-public-CDN gate the module doc
/// documents; the body-string pin distinguishes the handler's 401
/// from any upstream-layer 401.
#[tokio::test]
async fn get_without_device_header_is_401() {
    let a = arena().await;
    let (_user, device) = seed_user_and_device(&a.db).await;

    // Upload first, then attempt the leak-test GET.
    let att = attachment::encrypt(b"secret");
    let body = build_multipart(BOUNDARY, &[("ciphertext", &att.ciphertext)]);
    let (status, body_bytes) = post_multipart(
        &a.router,
        "/media",
        &[("X-Device-Id", &device)],
        BOUNDARY,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let resp: MediaUploadResponse = serde_json::from_slice(&body_bytes).unwrap();

    let path = format!("/media/{}", resp.id);
    let (status, body_bytes) = get_bytes(&a.router, &path, &[]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_error_body(&body_bytes, "missing X-Device-Id header");
}

/// Test 6 — GET with an unknown id is 404 with the `"media not found"`
/// body. Privacy-conservative: the same body is used for "id doesn't
/// exist" and "file missing on disk", so an attacker can't
/// distinguish; the body-string pin guards that property from a
/// regression that split the two branches.
#[tokio::test]
async fn get_unknown_id_is_404() {
    let a = arena().await;
    let (_user, device) = seed_user_and_device(&a.db).await;
    let path = format!("/media/{}", random_id());
    let (status, body_bytes) = get_bytes(&a.router, &path, &[("X-Device-Id", &device)]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_error_body(&body_bytes, "media not found");
}

/// Helper — assert the response body is the `ErrorBody` shape with the
/// expected `error` string. Used by the negative tests to pin
/// handler-emitted error messages, not just status codes.
#[track_caller]
fn assert_error_body(body: &[u8], expected: &str) {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_else(|e| {
        panic!(
            "expected ErrorBody JSON, got non-JSON: {e}, raw: {:?}",
            String::from_utf8_lossy(body)
        )
    });
    let got = v
        .get("error")
        .and_then(|x| x.as_str())
        .unwrap_or_else(|| panic!("missing/non-string `error` field in {v}"));
    assert_eq!(
        got, expected,
        "error string mismatch (was the handler refactored?)"
    );
}

// ---------------------------------------------------------------------------
// Multipart shape
// ---------------------------------------------------------------------------

/// Test 7 — multipart body with no `ciphertext` field is 400. Other
/// fields (here, `meta`) are tolerated by the handler; only the absence
/// of `ciphertext` is the error.
#[tokio::test]
async fn missing_ciphertext_field_is_400() {
    let a = arena().await;
    let (_user, device) = seed_user_and_device(&a.db).await;

    let body = build_multipart(BOUNDARY, &[("meta", b"some-other-field")]);
    let (status, _) = post_multipart(
        &a.router,
        "/media",
        &[("X-Device-Id", &device)],
        BOUNDARY,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Test 8 — empty `ciphertext` field is 400, separate from the missing
/// case. Encryption of an empty plaintext is legal (see
/// `crypto::attachment::empty_plaintext_round_trip`), but uploading an
/// empty *ciphertext* — i.e. an attacker probing the storage layer
/// without spending any randomness — is rejected at the multipart
/// boundary so we never persist a zero-byte row.
#[tokio::test]
async fn empty_ciphertext_field_is_400() {
    let a = arena().await;
    let (_user, device) = seed_user_and_device(&a.db).await;

    let body = build_multipart(BOUNDARY, &[("ciphertext", b"")]);
    let (status, _) = post_multipart(
        &a.router,
        "/media",
        &[("X-Device-Id", &device)],
        BOUNDARY,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Test 9 — multipart body with extra fields BEFORE the `ciphertext`
/// field is still accepted, AND the bytes that get stored are the
/// `ciphertext` field, not the earlier `filename` field.
///
/// The GET round-trip is load-bearing — without it, a buggy handler
/// that grabbed the FIRST field's bytes (here, `b"photo.png"`) and
/// stored those as the "ciphertext" would still produce a 201 + a
/// valid `MediaUploadResponse` and green this test. Forward-compat
/// with v2 wire shapes (filename, content-type, etc. as preceding
/// multipart fields) only holds if the handler actually picks the
/// right field by name. Discriminating signal is `downloaded ==
/// att.ciphertext`, which is byte-distinguishable from the
/// `filename` field's payload.
#[tokio::test]
async fn extra_fields_before_ciphertext_are_ignored() {
    let a = arena().await;
    let (_user, device) = seed_user_and_device(&a.db).await;

    let att = attachment::encrypt(b"payload");
    let body = build_multipart(
        BOUNDARY,
        &[("filename", b"photo.png"), ("ciphertext", &att.ciphertext)],
    );
    let (status, body_bytes) = post_multipart(
        &a.router,
        "/media",
        &[("X-Device-Id", &device)],
        BOUNDARY,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "upload failed: {body_bytes:?}");
    let resp: MediaUploadResponse = serde_json::from_slice(&body_bytes).unwrap();

    let path = format!("/media/{}", resp.id);
    let (status, downloaded) = get_bytes(&a.router, &path, &[("X-Device-Id", &device)]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        downloaded, att.ciphertext,
        "the stored bytes must be the 'ciphertext' field, not the preceding 'filename' field"
    );
}

// ---------------------------------------------------------------------------
// JSON-error contract (regression: error bodies are still JSON)
// ---------------------------------------------------------------------------

/// Test 10 — error responses use the same `{"error": "..."}` shape every
/// other handler emits. A regression here means a client error-handler
/// branch breaks silently.
#[tokio::test]
async fn error_body_is_json_errorbody_shape() {
    let a = arena().await;
    let (status, body) = get_json(
        &a.router,
        &format!("/media/{}", random_id()),
        &[("X-Device-Id", "missing-dev")],
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        body.get("error").is_some(),
        "expected ErrorBody shape, got {body}"
    );
}
