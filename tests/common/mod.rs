//! Shared test harness for the integration suites.
//!
//! Step 7 hoists what used to be inline copies in `tests/keys.rs` and
//! `tests/keyshare.rs` into one place because a third test binary
//! (`tests/rooms.rs`) needs the same primitives. The pattern Cargo
//! understands is `tests/common/mod.rs` (a `mod.rs` inside a sub-directory
//! is NOT treated as its own test binary, so we don't double-run anything;
//! see <https://doc.rust-lang.org/book/ch11-03-test-organization.html>).
//!
//! Each test binary brings this in with `mod common;` and uses a subset of
//! the exports; `#![allow(dead_code)]` keeps the unused-warning noise down
//! for items a given binary doesn't touch.
//!
//! What's deliberately NOT here: `build_bundle` and `publish_device` —
//! those depend on `crate::crypto`, and only the crypto-touching tests
//! (`tests/keys.rs` and `tests/keyshare.rs`) need them. `tests/rooms.rs`
//! is pure server-surface and brings its own inline `create_test_user_and_device`
//! that issues raw CREATE statements.

#![allow(dead_code)]
#![cfg(feature = "ssr")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use rand::RngCore;
use serde_json::Value;
use surrealdb::engine::remote::ws::{Client, Ws};
use surrealdb::opt::auth::Root;
use surrealdb::Surreal;
use tower::ServiceExt;

use authlyn_interactive::server::{make_router, AppState};
use authlyn_interactive::storage;

/// Monotonic counter so concurrent `cargo test` workers each get distinct
/// SurrealDB namespaces *and* media tempdirs, in addition to the
/// process-PID prefix.
pub static NS_COUNTER: AtomicU64 = AtomicU64::new(0);

/// One isolated test arena: a SurrealDB namespace+database that owns its
/// schema, the axum `Router` wired against it, and a per-arena media
/// storage tempdir. `db` is exposed so tests that need to inspect
/// persisted state (count rows, assert post-conditions) can issue queries
/// directly; `media_dir` is exposed so the media tests can probe the
/// filesystem side directly.
pub struct Arena {
    pub router: Router,
    pub db: Surreal<Client>,
    pub media_dir: PathBuf,
    /// The SAME `AppState` the router was built from. Tests that drive an
    /// ssr core fn directly (e.g. `broadcast_system_message`) must use this
    /// one when they assert on SSE delivery: a freshly constructed
    /// `AppState::new(...)` carries its OWN broadcast channel, so emissions
    /// on it never reach the router's `GET /events` subscribers.
    pub state: AppState,
}

pub async fn arena() -> Arena {
    let db = test_db().await;
    let media_dir = test_media_dir();
    let state = AppState::new(db.clone(), media_dir.clone());
    let router = make_router(state.clone());
    Arena {
        router,
        db,
        media_dir,
        state,
    }
}

/// Like [`arena`] but with the Ghost Quill typing-draft TTL overridden
/// (W4/T7). The TTL is a plain `Copy` field on `AppState`, so it MUST be set
/// before `make_router` clones the state — hence a dedicated constructor
/// rather than mutating `Arena::state` afterwards. Lets the prune tests run
/// in milliseconds instead of sleeping out the 8s production TTL.
pub async fn arena_with_draft_ttl(ttl: std::time::Duration) -> Arena {
    let db = test_db().await;
    let media_dir = test_media_dir();
    let state = AppState::new(db.clone(), media_dir.clone()).with_draft_ttl(ttl);
    let router = make_router(state.clone());
    Arena {
        router,
        db,
        media_dir,
        state,
    }
}

/// Per-arena media-storage tempdir. Uses `random_id()` for uniqueness —
/// 16 bytes of entropy is more than enough to prevent collisions even
/// under aggressive parallel `cargo test` workers. Leaks on drop —
/// for dev runs `/tmp` rotates often enough; CI runners are ephemeral.
/// Cheap enough that the lack of cleanup is fine for v1.
fn test_media_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!("authlyn-test-media-{}", random_id()));
    std::fs::create_dir_all(&path).expect("create test media dir");
    path
}

pub async fn test_db() -> Surreal<Client> {
    let db = raw_db().await;
    db.query(storage::SCHEMA)
        .await
        .expect("apply schema")
        .check()
        .expect("apply schema check");
    db
}

/// Like [`test_db`] but does NOT apply `storage::SCHEMA` — a connection on its
/// own isolated namespace with no schema. For migration tests that apply a
/// custom/"old" schema first, then re-apply the real `storage::SCHEMA` to
/// exercise field additions + backfills over pre-existing rows.
pub async fn raw_db() -> Surreal<Client> {
    let host = std::env::var("SURREAL_URL")
        .unwrap_or_else(|_| "127.0.0.1:8000".into())
        .trim_start_matches("ws://")
        .trim_start_matches("wss://")
        .to_string();
    let user = std::env::var("SURREAL_USER").unwrap_or_else(|_| "root".into());
    let pass = std::env::var("SURREAL_PASS").unwrap_or_else(|_| "root".into());

    let db = Surreal::new::<Ws>(host)
        .await
        .expect("connect to SurrealDB — start it with: surreal start --user root --pass root --bind 127.0.0.1:8000 memory");
    db.signin(Root {
        username: user,
        password: pass,
    })
    .await
    .expect("signin");

    let pid = std::process::id();
    let seq = NS_COUNTER.fetch_add(1, Ordering::Relaxed);
    let ns = format!("test_{}_{}", pid, seq);
    let db_name = format!("test_{}_{}", pid, seq);
    db.use_ns(&ns).use_db(&db_name).await.expect("use ns/db");
    db
}

/// Free-form 16-byte hex string used as device/user/room identifier in v1.
/// Spec calls these "ULIDs", but the v1 auth stub treats them as opaque
/// strings, so random hex is sufficient and avoids pulling in a ulid crate.
pub fn random_id() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Build (but do not send) a JSON request, so several can be fired on cloned
/// routers concurrently via [`status_of`] + `tokio::spawn` (concurrency tests).
pub fn build_json_request(
    method: Method,
    path: &str,
    cookie: Option<&str>,
    body: Option<&Value>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let body = match body {
        Some(v) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(v).unwrap())
        }
        None => Body::empty(),
    };
    builder.body(body).unwrap()
}

/// Drive one prebuilt request through an owned router clone, returning the
/// status — shaped for `tokio::spawn` in concurrency tests.
pub async fn status_of(router: Router, req: Request<Body>) -> StatusCode {
    router.oneshot(req).await.expect("oneshot").status()
}

/// Hit the router with a JSON request. Returns (status, parsed body).
pub async fn post_json(
    router: &Router,
    path: &str,
    headers: &[(&str, &str)],
    body: &Value,
) -> (StatusCode, Value) {
    let mut req_builder = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    for (k, v) in headers {
        req_builder = req_builder.header(*k, *v);
    }
    let req = req_builder
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap();

    let res = router.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.expect("read body");
    let parsed: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "body parse failed: {e}, raw: {:?}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, parsed)
}

/// Low-level request helper that also surfaces the response's first
/// `Set-Cookie` as a bare `name=value` pair (sans attributes) — what you
/// replay as a `Cookie:` header. `cookie` sets the request's `Cookie:`
/// header; `body`, when `Some`, is sent as JSON.
pub async fn send(
    router: &Router,
    method: Method,
    path: &str,
    cookie: Option<&str>,
    body: Option<&Value>,
) -> (StatusCode, Option<String>, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(c) = cookie {
        builder = builder.header(header::COOKIE, c);
    }
    let req = match body {
        Some(b) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(b).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };

    let res = router.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    let set_cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(str::to_string);
    let bytes = to_bytes(res.into_body(), 1 << 20).await.expect("read body");
    let parsed: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, set_cookie, parsed)
}

/// Register a fresh account and return its `authlyn_session=<token>` cookie
/// pair, ready to pass as the `cookie` arg of [`send`] / [`post_json`].
/// Panics unless registration returns 201 with a session cookie.
pub async fn register_account(router: &Router, username: &str, password: &str) -> String {
    let (status, cookie, body) = send(
        router,
        Method::POST,
        "/auth/register",
        None,
        Some(&serde_json::json!({ "username": username, "password": password })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "register({username}) should 201, got {status}: {body:?}"
    );
    cookie.expect("register must set a session cookie")
}

/// Open an SSE response against the in-process router. Returns the status,
/// the response headers, and the still-streaming body — read frames with
/// [`next_sse_data`].
pub async fn open_sse(
    router: &Router,
    path: &str,
    cookie: Option<&str>,
) -> (StatusCode, HeaderMap, Body) {
    let mut req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(header::ACCEPT, "text/event-stream");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let resp = router
        .clone()
        .oneshot(req.body(Body::empty()).expect("request"))
        .await
        .expect("sse oneshot");
    let (parts, body) = resp.into_parts();
    (parts.status, parts.headers, body)
}

/// Outcome of one `next_sse_data` read. Negative (privacy) tests must assert
/// `Timeout` specifically — `Closed` would mean the server dropped the stream,
/// which is NOT proof that an event was withheld.
#[derive(Debug)]
pub enum SseRead {
    /// A `data:` line arrived and parsed as JSON.
    Data(serde_json::Value),
    /// The window elapsed with the stream still open and silent.
    Timeout,
    /// The body stream ended (server closed the connection).
    Closed,
}

/// Read frames until one `data: <json>` line arrives (skipping keep-alive
/// comments), `within` elapses, or the stream ends — see [`SseRead`].
///
/// Parser assumptions: this expects axum's SSE serializer output — one
/// single-line `data: <json>` per event and `: ` keep-alive comment lines.
/// Frames are decoded with per-frame lossy UTF-8, which is fine while
/// `SyncEvent` payloads carry only ASCII ids; a multi-byte char split across
/// frames would be mangled. A `data: ` line whose payload is not valid JSON
/// panics rather than being silently skipped.
pub async fn next_sse_data(body: &mut Body, within: std::time::Duration) -> SseRead {
    use http_body_util::BodyExt;
    let deadline = tokio::time::Instant::now() + within;
    let mut buf = String::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return SseRead::Timeout;
        }
        let frame = match tokio::time::timeout(remaining, body.frame()).await {
            Err(_elapsed) => return SseRead::Timeout,
            Ok(None) => return SseRead::Closed,
            Ok(Some(Err(e))) => panic!("SSE body frame error: {e}"),
            Ok(Some(Ok(frame))) => frame,
        };
        if let Some(bytes) = frame.data_ref() {
            buf.push_str(&String::from_utf8_lossy(bytes));
            // SSE frames are newline-delimited; scan completed lines.
            while let Some(pos) = buf.find('\n') {
                let line: String = buf.drain(..=pos).collect();
                let line = line.trim();
                if let Some(json) = line.strip_prefix("data: ") {
                    match serde_json::from_str(json) {
                        Ok(v) => return SseRead::Data(v),
                        Err(_) => panic!("unparseable SSE data line: {line}"),
                    }
                }
            }
        }
    }
}

/// Hit the router with a GET. Returns (status, parsed body).
pub async fn get_json(
    router: &Router,
    path: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut req_builder = Request::builder().method(Method::GET).uri(path);
    for (k, v) in headers {
        req_builder = req_builder.header(*k, *v);
    }
    let req = req_builder.body(Body::empty()).unwrap();

    let res = router.clone().oneshot(req).await.expect("oneshot");
    let status = res.status();
    let bytes = to_bytes(res.into_body(), 1 << 20).await.expect("read body");
    let parsed: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            panic!(
                "body parse failed: {e}, raw: {:?}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, parsed)
}
