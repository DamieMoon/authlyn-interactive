//! Dev hot-reload over SSE — `POST /admin/dev/reload`.
//!
//! When a new release is deployed to the test deck (which runs the COMPILED
//! binary and so has no cargo-leptos live-reload), an admin nudge broadcasts a
//! payload-free `Reload` over the existing SSE bus; every connected client then
//! calls `location.reload()`. This mirrors the `tests/system_messages.rs`
//! convention: the admin-ALLOWED path can't be driven through HTTP (the
//! `is_admin` env read races parallel workers), so the broadcast LOGIC is
//! exercised directly via the `broadcast_reload` core fn, while only the
//! fail-closed gate (non-admin → 403, unauth → 401) is checked through the
//! router.
//!
//! The load-bearing SSE property — that `Reload` reaches a connection whose
//! visible-channel set is EMPTY, as a DISTINCT NAMED `event: reload` frame —
//! is asserted with a tiny raw-frame reader here, because the shared
//! `next_sse_data` helper only surfaces `data:` lines and discards the event
//! name.

mod common;

#[cfg(feature = "ssr")]
use authlyn_interactive::server::dev_reload::broadcast_reload;
#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;

// ---------------------------------------------------------------------------
// Broadcast core — the global reload nudge reaches EVERY connection as a
// distinct named frame, bypassing the per-connection visibility filter.
// ---------------------------------------------------------------------------

/// A freshly-registered account with NO guild membership has an EMPTY
/// visible-channel set, so it would never receive a channel-scoped event. The
/// reload nudge must reach it anyway (global, filter-bypassing) AND arrive as a
/// `event: reload` frame distinct from the generic `data:`-only message frames.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn reload_reaches_a_connection_with_no_visible_channels_as_a_named_frame() {
    let a = common::arena().await;

    // A user in zero guilds: `visible_channels` is empty for this connection,
    // proving the delivery below bypasses the channel-visibility filter.
    let lonely = common::register_account(&a.router, "Lonely", "password123").await;

    // Subscribe BEFORE broadcasting (the harness contract).
    let (st, _h, mut body) = common::open_sse(&a.router, "/events", Some(&lonely)).await;
    assert_eq!(st, StatusCode::OK);

    // Driven on `a.state` (the ROUTER's bus) so the emission lands on the same
    // bus the `GET /events` stream subscribes to.
    broadcast_reload(&a.state);

    let frame = read_named_frame(&mut body, std::time::Duration::from_secs(3)).await;
    assert_eq!(
        frame.event.as_deref(),
        Some("reload"),
        "the reload nudge is a DISTINCT named SSE frame, not a generic data-only message"
    );
}

/// The reload frame carries no information (id-only bus invariant): its `data:`
/// is a content-free empty-object sentinel — never a payload.
#[cfg(feature = "ssr")]
#[tokio::test]
async fn reload_frame_is_payload_free() {
    let a = common::arena().await;
    let user = common::register_account(&a.router, "Payload", "password123").await;
    let (st, _h, mut body) = common::open_sse(&a.router, "/events", Some(&user)).await;
    assert_eq!(st, StatusCode::OK);

    broadcast_reload(&a.state);

    let frame = read_named_frame(&mut body, std::time::Duration::from_secs(3)).await;
    assert_eq!(frame.event.as_deref(), Some("reload"));
    // Content-free: an empty JSON object is the entire data line. The signal is
    // the FRAME ITSELF; nothing rides it (the bus stays id-only by design).
    assert_eq!(
        frame.data.as_deref(),
        Some("{}"),
        "the reload nudge must stay payload-free"
    );
}

// ---------------------------------------------------------------------------
// Admin gate — fail-closed (empty admin set authorizes no one)
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn dev_reload_is_403_for_non_admin() {
    let a = common::arena().await;
    let user = common::register_account(&a.router, "NotAdmin", "password123").await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/admin/dev/reload",
        Some(&user),
        Some(&json!({})),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "no admins configured → every caller is non-admin → 403"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn dev_reload_requires_auth() {
    let a = common::arena().await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/admin/dev/reload",
        None,
        Some(&json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Local raw-frame reader: like `common::next_sse_data` but surfaces the
// `event:` name too (the load-bearing property the shared helper discards).
// ---------------------------------------------------------------------------

/// One fully-parsed SSE frame (the fields this suite asserts on).
#[cfg(feature = "ssr")]
#[derive(Debug, Default)]
struct NamedFrame {
    event: Option<String>,
    data: Option<String>,
}

/// Read frames until a blank line terminates one carrying an `event:` line, or
/// `within` elapses (panics on timeout — the reload nudge must arrive). Parses
/// axum's SSE serializer output: `event: <name>` / `data: <payload>` lines,
/// `\n`-delimited, a blank line ending the frame; `: ` keep-alive comments are
/// skipped.
#[cfg(feature = "ssr")]
async fn read_named_frame(body: &mut axum::body::Body, within: std::time::Duration) -> NamedFrame {
    use http_body_util::BodyExt;
    let deadline = tokio::time::Instant::now() + within;
    let mut buf = String::new();
    let mut frame = NamedFrame::default();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for a named SSE frame");
        }
        let chunk = match tokio::time::timeout(remaining, body.frame()).await {
            Err(_elapsed) => panic!("timed out waiting for a named SSE frame"),
            Ok(None) => panic!("SSE stream closed before a named frame arrived"),
            Ok(Some(Err(e))) => panic!("SSE body frame error: {e}"),
            Ok(Some(Ok(chunk))) => chunk,
        };
        if let Some(bytes) = chunk.data_ref() {
            buf.push_str(&String::from_utf8_lossy(bytes));
            while let Some(pos) = buf.find('\n') {
                let line: String = buf.drain(..=pos).collect();
                let line = line.trim_end_matches(['\r', '\n']);
                if line.is_empty() {
                    // Blank line ends a frame: return it iff it was named.
                    if frame.event.is_some() {
                        return frame;
                    }
                    frame = NamedFrame::default();
                } else if let Some(name) = line.strip_prefix("event:") {
                    frame.event = Some(name.trim().to_string());
                } else if let Some(payload) = line.strip_prefix("data:") {
                    frame.data = Some(payload.trim().to_string());
                }
                // `: ` keep-alive comments and other fields are ignored.
            }
        }
    }
}
