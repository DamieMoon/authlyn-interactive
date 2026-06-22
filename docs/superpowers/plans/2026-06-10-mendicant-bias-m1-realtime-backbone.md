# Mendicant Bias M1: Realtime Backbone Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the 1.5s client polling with an SSE event bus (`GET /events`), add a batched `GET /unread` endpoint, and land the M1 perf fixes (media cache headers, MIME folded into the message projection, lazy guild-channel loading) — dropping idle traffic from ~150–200 req/min to keep-alives only.

**Architecture:** Notify-and-fetch. The server broadcasts tiny id-only `SyncEvent`s over a `tokio::sync::broadcast` hub in `AppState`; each SSE connection filters events against a per-connection visible-channel set (reloaded on `lists_changed`). Clients react to events by refetching through the EXISTING permission-checked endpoints, so the push path carries no content and adds no new authorization surface. The old polling loop remains as an automatic fallback.

**Tech Stack:** axum 0.8 SSE (core, no new feature), `tokio::sync::broadcast`, `futures_util::stream::unfold` (ssr), web-sys `EventSource` (hydrate), SurrealDB loop-indexed batched statements (sanctioned by the parameterized-SQL invariant), existing `tests/common` harness + new streaming helper.

**Spec:** `docs/superpowers/specs/2026-06-10-mendicant-bias-design.md` §6 (re-architecture), §12 M1.

**Invariants in play:** session-cookie-only identity (SSE uses `AuthAccount`); privacy-404 / no-information-leak (event filtering must be tested adversarially); parameterized SQL only (loop-index bind names are explicitly sanctioned); soft-delete hidden on read.

**Prerequisites:** local SurrealDB running (`surreal start --user root --pass root --bind 127.0.0.1:8000 memory`). All commits on branch `mendicant-bias`. NEVER push to `main`.

---

### Task 0: Branch + baseline

**Files:** none (git only)

- [ ] **Step 0.1: Create the work branch**

```bash
cd /Users/damien/Developer/authlyn-interactive
git checkout main && git pull --ff-only 2>/dev/null || true
git checkout -b mendicant-bias
```

- [ ] **Step 0.2: Verify the baseline is green**

Run: `cargo test --features ssr 2>&1 | tail -5`
Expected: `test result: ok.` lines for all 16 suites (144 tests). If SurrealDB is not running, start it first (see Prerequisites).

- [ ] **Step 0.3: No commit** (nothing changed)

---

### Task 1: `SyncEvent` wire type in `protocol.rs`

**Files:**
- Modify: `src/protocol.rs` (append at end of file)
- Test: `tests/sync_events.rs` (create)

- [ ] **Step 1.1: Write the failing serde-shape test**

Create `tests/sync_events.rs`:

```rust
//! Wire-shape pins for the SSE `SyncEvent` enum (M1). These are serde-only
//! tests (no server), but live in the integration tree per repo convention.
#![cfg(feature = "ssr")]

use authlyn_interactive::protocol::SyncEvent;

#[tokio::test]
async fn sync_event_serializes_with_snake_case_type_tags() {
    let ev = SyncEvent::MessageCreated { channel_id: "abc".into() };
    let json = serde_json::to_string(&ev).unwrap();
    assert_eq!(json, r#"{"type":"message_created","channel_id":"abc"}"#);

    let ev = SyncEvent::ListsChanged;
    assert_eq!(serde_json::to_string(&ev).unwrap(), r#"{"type":"lists_changed"}"#);

    let back: SyncEvent =
        serde_json::from_str(r#"{"type":"typing","channel_id":"c1"}"#).unwrap();
    assert_eq!(back, SyncEvent::Typing { channel_id: "c1".into() });
}
```

- [ ] **Step 1.2: Run it to verify it fails**

Run: `cargo test --features ssr --test sync_events 2>&1 | tail -5`
Expected: COMPILE ERROR — `no SyncEvent in protocol`.

- [ ] **Step 1.3: Implement the enum**

Append to `src/protocol.rs`:

```rust
/// M1 realtime: the id-only event vocabulary broadcast over `GET /events`.
/// Deliberately content-free (notify-and-fetch): clients react by refetching
/// through the existing permission-checked endpoints, so this enum never
/// becomes an authorization surface. Shared by ssr (emitter) and hydrate
/// (EventSource consumer); always-on like every other wire DTO here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEvent {
    /// A message was created in this channel.
    MessageCreated { channel_id: String },
    /// A message was edited in this channel.
    MessageEdited { channel_id: String, message_id: String },
    /// A message was soft-deleted in this channel.
    MessageDeleted { channel_id: String, message_id: String },
    /// Someone (not necessarily you) pinged "typing" in this channel.
    Typing { channel_id: String },
    /// Guild/channel/membership metadata changed somewhere visible to you —
    /// refetch lists. Also used as a generic "resync" nudge after broadcast lag.
    ListsChanged,
}

impl SyncEvent {
    /// The channel this event is scoped to, if any. `None` (ListsChanged) means
    /// "deliver to everyone and let the refetch re-derive visibility".
    pub fn channel_id(&self) -> Option<&str> {
        match self {
            SyncEvent::MessageCreated { channel_id }
            | SyncEvent::MessageEdited { channel_id, .. }
            | SyncEvent::MessageDeleted { channel_id, .. }
            | SyncEvent::Typing { channel_id } => Some(channel_id),
            SyncEvent::ListsChanged => None,
        }
    }
}
```

(`Serialize`/`Deserialize` are already imported at the top of `protocol.rs`.)

- [ ] **Step 1.4: Run the test to verify it passes**

Run: `cargo test --features ssr --test sync_events 2>&1 | tail -5`
Expected: `test result: ok. 1 passed`

- [ ] **Step 1.5: Commit**

```bash
git add src/protocol.rs tests/sync_events.rs
git commit -m "feat(protocol): SyncEvent wire enum for the M1 SSE bus

Id-only, content-free by design (notify-and-fetch): the push path must
never become an authorization surface. snake_case type tags pinned.

Tests: sync_event_serializes_with_snake_case_type_tags"
```

---

### Task 2: Broadcast hub in `AppState`

**Files:**
- Modify: `Cargo.toml` (deps + ssr feature)
- Modify: `src/server/state.rs`

- [ ] **Step 2.1: Add dependencies**

In `Cargo.toml`, next to the other ssr-only deps (each line carries a purpose comment per repo convention):

```toml
# M1 SSE bus: BroadcastStream-free hand-rolled stream needs unfold; ssr-only.
futures-util = { version = "0.3", optional = true, default-features = false, features = ["std"] }
```

NOTE: `futures-util` already exists as an optional dep (hydrate graph). Do NOT add a duplicate entry — instead just add `"dep:futures-util"` to the `ssr` feature list in `[features]`:

```toml
ssr = [
    # ... existing entries unchanged ...
    "dep:futures-util",
]
```

And add a dev-dependency for streaming test bodies:

```toml
[dev-dependencies]
# (append to existing block)
http-body-util = "0.1"   # frame-by-frame reading of SSE test responses
```

- [ ] **Step 2.2: Add the hub to AppState**

In `src/server/state.rs`, add the field to the struct (after `typing`):

```rust
    /// M1 realtime: the process-wide SSE event bus. Every mutation handler
    /// best-effort `send()`s a `SyncEvent`; every `GET /events` connection
    /// subscribes. Capacity 256: laggards get `RecvError::Lagged` and are
    /// nudged to resync — events are droppable by design (notify-and-fetch).
    pub events: tokio::sync::broadcast::Sender<crate::protocol::SyncEvent>,
```

Initialize it in EVERY constructor in `state.rs` (both `with_leptos` and the test-facing `new` — locate all `Self {` literals in this file):

```rust
            events: tokio::sync::broadcast::channel(256).0,
```

- [ ] **Step 2.3: Verify it compiles**

Run: `cargo check --features ssr 2>&1 | tail -3`
Expected: `Finished` (warnings ok, errors none). If a constructor was missed the struct literal errors will name it.

- [ ] **Step 2.4: Run the full suite (no behavior change expected)**

Run: `cargo test --features ssr 2>&1 | tail -3`
Expected: all green.

- [ ] **Step 2.5: Commit**

```bash
git add Cargo.toml Cargo.lock src/server/state.rs
git commit -m "feat(server): tokio broadcast event hub on AppState

Capacity-256 sender; subscribers are GET /events connections (next
commit). Laggards are droppable by design under notify-and-fetch.

Tests: covered by existing suite (no behavior change)"
```

---

### Task 3: Streaming test helper + first failing `/events` test

**Files:**
- Modify: `tests/common/mod.rs` (append helpers)
- Test: `tests/events.rs` (create)

- [ ] **Step 3.1: Add SSE helpers to the harness**

Append to `tests/common/mod.rs`:

```rust
/// Open an SSE response against the in-process router. Returns the status and
/// the still-streaming body — read frames with `next_sse_data`.
pub async fn open_sse(
    router: &axum::Router,
    path: &str,
    cookie: Option<&str>,
) -> (StatusCode, axum::body::Body) {
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
    (resp.status(), resp.into_body())
}

/// Read frames until one `data: <json>` line arrives (skipping keep-alive
/// comments), or `within` elapses. Returns the parsed JSON, or None on timeout.
pub async fn next_sse_data(
    body: &mut axum::body::Body,
    within: std::time::Duration,
) -> Option<serde_json::Value> {
    use http_body_util::BodyExt;
    let deadline = tokio::time::Instant::now() + within;
    let mut buf = String::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let frame = tokio::time::timeout(remaining, body.frame()).await.ok()??;
        let frame = frame.ok()?;
        if let Some(bytes) = frame.data_ref() {
            buf.push_str(&String::from_utf8_lossy(bytes));
            // SSE frames are newline-delimited; scan completed lines.
            while let Some(pos) = buf.find('\n') {
                let line: String = buf.drain(..=pos).collect();
                let line = line.trim();
                if let Some(json) = line.strip_prefix("data: ") {
                    if let Ok(v) = serde_json::from_str(json) {
                        return Some(v);
                    }
                }
            }
        }
    }
}
```

(`StatusCode`, `Request`, `Method`, `Body`, `header`, `tower::ServiceExt` are already imported in this file; add `use tower::ServiceExt as _;` only if `oneshot` is not already in scope — check the existing imports first.)

- [ ] **Step 3.2: Write the first failing test**

Create `tests/events.rs`:

```rust
//! M1 SSE bus: GET /events delivery, privacy filtering, and auth gating.
#![cfg(feature = "ssr")]

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;
use std::time::Duration;

/// Register an owner, create a guild, return (cookie, gid, first channel id).
async fn owner_with_channel(router: &axum::Router) -> (String, String, String) {
    let owner = common::register_account(router, "EventsOwner", "password123").await;
    let (st, _, guild) = common::send(
        router, Method::POST, "/guilds", Some(&owner), Some(&json!({ "name": "Bus" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();
    let (st, _, detail) =
        common::send(router, Method::GET, &format!("/guilds/{gid}"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();
    (owner, gid, cid)
}

#[tokio::test]
async fn events_requires_a_session() {
    let a = common::arena().await;
    let (status, _body) = common::open_sse(&a.router, "/events", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn member_receives_message_created_over_sse() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router).await;

    let (status, mut body) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(status, StatusCode::OK);

    // Post a message AFTER subscribing.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "ping over the bus" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let ev = common::next_sse_data(&mut body, Duration::from_secs(3))
        .await
        .expect("an event should arrive within 3s");
    assert_eq!(ev["type"], "message_created");
    assert_eq!(ev["channel_id"], cid.as_str());
}
```

- [ ] **Step 3.3: Run to verify failure**

Run: `cargo test --features ssr --test events 2>&1 | tail -8`
Expected: both tests FAIL — `/events` returns 404 (route does not exist), so `events_requires_a_session` asserts 401 ≠ 404 and the member test gets status 404.

- [ ] **Step 3.4: Commit the red tests**

```bash
git add tests/common/mod.rs tests/events.rs
git commit -m "test(events): failing SSE delivery + auth tests and streaming harness helpers

open_sse/next_sse_data read axum bodies frame-by-frame so SSE responses
can be asserted without draining the infinite stream.

Tests: events_requires_a_session, member_receives_message_created_over_sse (red)"
```

---

### Task 4: Minimal `GET /events` endpoint (unfiltered) + first emission

**Files:**
- Create: `src/server/events.rs`
- Modify: `src/server/mod.rs` (module decl + route)
- Modify: `src/server/messages/posting.rs` (emit on create)

- [ ] **Step 4.1: Implement the SSE handler**

Create `src/server/events.rs`:

```rust
//! GET /events — the M1 SSE bus (ssr-only). Auth via the session cookie
//! (`AuthAccount`), exactly like every JSON route. Wire format: unnamed SSE
//! `data:` frames each carrying one serialized `protocol::SyncEvent`.
//! Filtering (privacy) is per-connection: see `visible_channels` below.

use crate::protocol::SyncEvent;
use crate::server::auth::session::AuthAccount;
use crate::server::state::AppState;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::Stream;
use std::collections::HashSet;
use std::convert::Infallible;
use tokio::sync::broadcast;

/// Load the channel ids the account may currently see (live text channels in
/// guilds where they are a member). Two parameterized statements, one
/// round-trip. Returns (channel_id, guild_id) pairs; M1 consumers only need
/// the channel ids but /unread (same helper) wants the guild mapping too.
pub(crate) async fn visible_channels(
    state: &AppState,
    account: &str,
) -> surrealdb::Result<Vec<(String, String)>> {
    #[derive(surrealdb::types::SurrealValue)]
    struct Row {
        channel_id: String,
        guild_id: String,
    }
    let mut resp = state
        .db
        .query(
            "LET $gids = (SELECT VALUE guild FROM guild_member
                 WHERE account = type::record('account', $account));
             SELECT meta::id(id) AS channel_id, meta::id(guild) AS guild_id FROM channel
                 WHERE deleted_at = NONE AND kind = 'text'
                   AND guild IN $gids AND guild.deleted_at = NONE;",
        )
        .bind(("account", account.to_string()))
        .await?
        .check()?;
    let rows: Vec<Row> = resp.take(1)?;
    Ok(rows.into_iter().map(|r| (r.channel_id, r.guild_id)).collect())
}

/// Per-connection stream state for the unfold below.
struct Conn {
    rx: broadcast::Receiver<SyncEvent>,
    visible: HashSet<String>,
    state: AppState,
    account: String,
}

impl Conn {
    async fn reload_visible(&mut self) {
        if let Ok(rows) = visible_channels(&self.state, &self.account).await {
            self.visible = rows.into_iter().map(|(c, _g)| c).collect();
        }
        // On DB error: keep the stale set. Fail-closed enough (no new grants
        // leak in), and the next lists_changed retries.
    }
}

fn sse_frame(ev: &SyncEvent) -> Event {
    // Serialization of a unit-tagged enum cannot fail.
    Event::default().data(serde_json::to_string(ev).expect("SyncEvent serializes"))
}

pub async fn events(
    State(state): State<AppState>,
    account: AuthAccount,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe BEFORE loading visibility so no event in between is missed;
    // an event for a channel created in that gap is recovered by the
    // lists_changed → reload path.
    let rx = state.events.subscribe();
    let mut conn = Conn { rx, visible: HashSet::new(), state, account: account.0 };
    conn.reload_visible().await;

    let stream = futures_util::stream::unfold(conn, |mut conn| async move {
        loop {
            match conn.rx.recv().await {
                Ok(ev) => match ev.channel_id() {
                    Some(cid) if !conn.visible.contains(cid) => continue, // privacy filter
                    Some(_) => return Some((Ok(sse_frame(&ev)), conn)),
                    None => {
                        // lists_changed: visibility may have shifted under us.
                        conn.reload_visible().await;
                        return Some((Ok(sse_frame(&ev)), conn));
                    }
                },
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Dropped events: nudge the client to a full resync.
                    conn.reload_visible().await;
                    return Some((Ok(sse_frame(&SyncEvent::ListsChanged)), conn));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

- [ ] **Step 4.2: Wire module + route**

In `src/server/mod.rs`: add `pub mod events;` next to the other module declarations, and register the route inside `small_body_routes()` (it is a GET with no body; the 512 KiB request cap is irrelevant but harmless, and the `no-store` response layer is CORRECT for SSE):

```rust
        .route("/events", get(events::events))
```

(add `events::` to the existing `use` list or reference it fully-qualified, matching file style.)

- [ ] **Step 4.3: Emit `MessageCreated` from the posting path**

In `src/server/messages/posting.rs`, locate the success branch of the message-create handler — the point where the CREATE succeeded and the handler is about to build its 201 response (grep: `grep -n "CREATED" src/server/messages/posting.rs`). Immediately before returning success, add:

```rust
    // M1 bus: best-effort, never fails the request (send() errs only when no
    // subscriber exists, which is the idle case).
    let _ = state
        .events
        .send(crate::protocol::SyncEvent::MessageCreated { channel_id: cid.clone() });
```

(If the success branch consumes `cid`, clone earlier; match the variable name actually in scope — it is the validated channel id, not raw user input.)

- [ ] **Step 4.4: Run the events tests**

Run: `cargo test --features ssr --test events 2>&1 | tail -8`
Expected: BOTH PASS (`events_requires_a_session` now hits the `AuthAccount` 401; the member test receives `message_created`).

- [ ] **Step 4.5: Full suite + commit**

Run: `cargo test --features ssr 2>&1 | tail -3` — expected green.

```bash
git add src/server/events.rs src/server/mod.rs src/server/messages/posting.rs
git commit -m "feat(server): GET /events SSE bus with per-connection visibility set

Notify-and-fetch: frames carry id-only SyncEvents; the posting path
emits message_created. Subscribe-before-load avoids missed events;
broadcast lag degrades to a lists_changed resync nudge.

Tests: events_requires_a_session, member_receives_message_created_over_sse"
```

---

### Task 5: Adversarial privacy filtering

**Files:**
- Test: `tests/events.rs` (append)

- [ ] **Step 5.1: Write the failing-or-proving privacy tests**

Append to `tests/events.rs`:

```rust
#[tokio::test]
async fn outsider_never_receives_events_for_a_channel_they_cannot_see() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router).await;
    let outsider = common::register_account(&a.router, "EventsOutsider", "password123").await;

    let (status, mut out_body) = common::open_sse(&a.router, "/events", Some(&outsider)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, mut own_body) = common::open_sse(&a.router, "/events", Some(&owner)).await;
    assert_eq!(status, StatusCode::OK);

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "secret" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // The member sees it…
    let ev = common::next_sse_data(&mut own_body, Duration::from_secs(3))
        .await
        .expect("member receives the event");
    assert_eq!(ev["type"], "message_created");

    // …the outsider gets NOTHING channel-scoped (keep-alive comments are
    // filtered by next_sse_data; lists_changed would also be a leak-free shape,
    // but message events must never cross the membership line).
    let leaked = common::next_sse_data(&mut out_body, Duration::from_millis(1200)).await;
    assert!(
        leaked.is_none(),
        "outsider must not receive channel-scoped events, got: {leaked:?}"
    );
}

#[tokio::test]
async fn typing_events_do_not_leak_across_guilds() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router).await;
    // Second, unrelated account with their own guild (so their visible set is non-empty).
    let other = common::register_account(&a.router, "EventsOther", "password123").await;
    let (st, _, _) = common::send(
        &a.router, Method::POST, "/guilds", Some(&other), Some(&json!({ "name": "Elsewhere" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let (_, mut other_body) = common::open_sse(&a.router, "/events", Some(&other)).await;

    let (st, _, _) = common::send(
        &a.router, Method::POST, &format!("/channels/{cid}/typing"), Some(&owner), None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let leaked = common::next_sse_data(&mut other_body, Duration::from_millis(1200)).await;
    assert!(leaked.is_none(), "typing in a foreign guild leaked: {leaked:?}");
}
```

- [ ] **Step 5.2: Run them**

Run: `cargo test --features ssr --test events 2>&1 | tail -8`
Expected: `outsider_never_receives…` PASSES already (Task 4 filter). `typing_events_do_not_leak…` FAILS — typing does not emit yet (no event at all → assert passes trivially!). To make it a REAL test it must fail for the right reason: it will pass vacuously now and be armed by Task 6's typing emission. Run it again AFTER Task 6 and confirm it still passes. Note this explicitly in the Task 6 verification step.

- [ ] **Step 5.3: Commit**

```bash
git add tests/events.rs
git commit -m "test(events): adversarial privacy — channel-scoped events never cross membership

Tests: outsider_never_receives_events_for_a_channel_they_cannot_see, typing_events_do_not_leak_across_guilds"
```

---

### Task 6: Emit from edit, delete, and typing paths

**Files:**
- Modify: `src/server/messages/editing.rs` (or wherever the PATCH/DELETE handlers live — locate first)
- Modify: `src/server/messages/typing.rs`
- Test: `tests/events.rs` (append)

- [ ] **Step 6.1: Locate the handlers**

Run: `grep -n "pub async fn" src/server/messages/*.rs`
Expected output names the edit + delete handlers (registered in `mod.rs` as `messages::edit_message` / `messages::delete_message` or similar on the `/channels/{cid}/messages/{mid}` route) and `typing_ping`.

- [ ] **Step 6.2: Write the failing tests**

Append to `tests/events.rs`:

```rust
#[tokio::test]
async fn edits_deletes_and_typing_reach_members_over_sse() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_channel(&a.router).await;

    // Seed a message BEFORE subscribing (so its create event isn't in the stream).
    let (st, _, msg) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "v1" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let mid = msg["id"].as_str().unwrap().to_string();

    let (_, mut body) = common::open_sse(&a.router, "/events", Some(&owner)).await;

    // Edit → message_edited
    let (st, _, _) = common::send(
        &a.router,
        Method::PATCH,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        Some(&json!({ "body": "v2" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let ev = common::next_sse_data(&mut body, Duration::from_secs(3)).await.unwrap();
    assert_eq!(ev["type"], "message_edited");
    assert_eq!(ev["message_id"], mid.as_str());

    // Typing → typing
    let (st, _, _) = common::send(
        &a.router, Method::POST, &format!("/channels/{cid}/typing"), Some(&owner), None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let ev = common::next_sse_data(&mut body, Duration::from_secs(3)).await.unwrap();
    assert_eq!(ev["type"], "typing");
    assert_eq!(ev["channel_id"], cid.as_str());

    // Delete → message_deleted
    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/channels/{cid}/messages/{mid}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let ev = common::next_sse_data(&mut body, Duration::from_secs(3)).await.unwrap();
    assert_eq!(ev["type"], "message_deleted");
}
```

- [ ] **Step 6.3: Run to verify failure**

Run: `cargo test --features ssr --test events edits_deletes_and_typing 2>&1 | tail -5`
Expected: FAIL — times out waiting for `message_edited` (nothing emits yet).

- [ ] **Step 6.4: Add the three emissions**

In each located handler's success branch (same pattern as Task 4 — immediately before the success response, with the in-scope channel/message id variables):

```rust
    let _ = state.events.send(crate::protocol::SyncEvent::MessageEdited {
        channel_id: cid.clone(),
        message_id: mid.clone(),
    });
```

```rust
    let _ = state.events.send(crate::protocol::SyncEvent::MessageDeleted {
        channel_id: cid.clone(),
        message_id: mid.clone(),
    });
```

And in `src/server/messages/typing.rs::typing_ping`, after the mutex insert block and before `StatusCode::NO_CONTENT.into_response()`:

```rust
    let _ = state
        .events
        .send(crate::protocol::SyncEvent::Typing { channel_id: cid.clone() });
```

- [ ] **Step 6.5: Run events suite — including the armed Task 5 test**

Run: `cargo test --features ssr --test events 2>&1 | tail -8`
Expected: ALL PASS — and `typing_events_do_not_leak_across_guilds` is now load-bearing (typing emits, the outsider still gets nothing).

- [ ] **Step 6.6: Commit**

```bash
git add src/server/messages/ tests/events.rs
git commit -m "feat(server): emit message_edited/message_deleted/typing on the bus

Best-effort sends in each success branch; the cross-guild typing privacy
test is now armed (events flow, the outsider still receives none).

Tests: edits_deletes_and_typing_reach_members_over_sse"
```

---

### Task 7: `lists_changed` from guild/channel mutations

**Files:**
- Modify: `src/server/state.rs` (helper)
- Modify: guild + channel mutation handlers (locate)
- Test: `tests/events.rs` (append)

- [ ] **Step 7.1: Failing test**

Append to `tests/events.rs`:

```rust
#[tokio::test]
async fn channel_creation_emits_lists_changed_and_membership_set_refreshes() {
    let a = common::arena().await;
    let (owner, gid, _cid) = owner_with_channel(&a.router).await;
    let (_, mut body) = common::open_sse(&a.router, "/events", Some(&owner)).await;

    // Create a new channel → lists_changed must arrive.
    let (st, _, chan) = common::send(
        &a.router,
        Method::POST,
        &format!("/guilds/{gid}/channels"),
        Some(&owner),
        Some(&json!({ "name": "annex", "kind": "text" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let new_cid = chan["id"].as_str().unwrap().to_string();
    let ev = common::next_sse_data(&mut body, Duration::from_secs(3)).await.unwrap();
    assert_eq!(ev["type"], "lists_changed");

    // …and the connection's visibility set must now include the NEW channel:
    // a message there must reach this same (pre-existing) SSE connection.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{new_cid}/messages"),
        Some(&owner),
        Some(&json!({ "body": "born after subscribe" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let ev = common::next_sse_data(&mut body, Duration::from_secs(3)).await.unwrap();
    assert_eq!(ev["type"], "message_created");
    assert_eq!(ev["channel_id"], new_cid.as_str());
}
```

NOTE: if the channel-creation route shape differs (check `src/server/mod.rs` route table — it may be `POST /guilds/{gid}/channels` or a `channel-manager` path), adjust the path and body to the REAL route; the assertion logic stays identical.

- [ ] **Step 7.2: Run to verify failure**

Run: `cargo test --features ssr --test events channel_creation_emits 2>&1 | tail -5`
Expected: FAIL — timeout waiting for `lists_changed`.

- [ ] **Step 7.3: Implement the helper + emissions**

In `src/server/state.rs`, add to `impl AppState` (create the impl block if the only one is constructors):

```rust
    /// Best-effort lists_changed nudge — call after any guild/channel/membership
    /// mutation commits. Cheap (one enum over a broadcast), never fails the request.
    pub fn notify_lists_changed(&self) {
        let _ = self.events.send(crate::protocol::SyncEvent::ListsChanged);
    }
```

Locate every guild/channel mutation success branch:

Run: `grep -rn "StatusCode::CREATED\|NO_CONTENT\|StatusCode::OK" src/server/guilds* src/server/channels* src/server/guild/ src/server/channel/ 2>/dev/null` (adjust to the actual domain dirs from `ls src/server/`).

Add `state.notify_lists_changed();` to the success branches of, at minimum: guild create, guild rename, guild delete/restore, channel create, channel rename, channel delete/restore, channel reorder, member add/remove (if such a handler exists in this phase).

- [ ] **Step 7.4: Run + commit**

Run: `cargo test --features ssr --test events 2>&1 | tail -5` — expected ALL PASS.
Run: `cargo test --features ssr 2>&1 | tail -3` — full suite green.

```bash
git add src/server/
git commit -m "feat(server): lists_changed on guild/channel mutations; SSE visibility refresh proven

The pre-existing-connection test pins the reload path: a channel born
after subscribe still delivers its events to the same stream.

Tests: channel_creation_emits_lists_changed_and_membership_set_refreshes"
```

---

### Task 8: Batched `GET /unread`

**Files:**
- Modify: `src/protocol.rs` (DTOs)
- Create: `src/server/messages/unread.rs`
- Modify: `src/server/messages/mod.rs` (module + re-export), `src/server/mod.rs` (route)
- Test: `tests/unread.rs` (create)

- [ ] **Step 8.1: Add the DTOs**

Append to `src/protocol.rs`:

```rust
/// M1: one row per visible text channel in `GET /unread`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelUnread {
    pub channel_id: String,
    pub guild_id: String,
    /// Messages newer than the caller's read cursor (capped at 100). 0 when
    /// the channel has no cursor yet — the client baselines instead of glowing.
    pub unread: usize,
    /// True iff any unread message pings the caller.
    pub pinged: bool,
    /// Latest live message's cursor pair, for client-side baselining of
    /// never-visited channels. None when the channel is empty.
    #[serde(default)]
    pub latest_sent_at: Option<String>,
    #[serde(default)]
    pub latest_id: Option<String>,
}

/// M1: response of `GET /unread`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnreadResponse {
    pub channels: Vec<ChannelUnread>,
}
```

- [ ] **Step 8.2: Write the failing tests**

Create `tests/unread.rs`:

```rust
//! M1: batched GET /unread — cursor math (strict composite tie-break), ping
//! flag, baseline fields, and privacy (only visible channels appear).
#![cfg(feature = "ssr")]

mod common;

use axum::http::{Method, StatusCode};
use serde_json::json;

async fn setup(router: &axum::Router) -> (String, String, String) {
    let owner = common::register_account(router, "UnreadOwner", "password123").await;
    let (st, _, guild) = common::send(
        router, Method::POST, "/guilds", Some(&owner), Some(&json!({ "name": "U" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let gid = guild["id"].as_str().unwrap().to_string();
    let (_, _, detail) =
        common::send(router, Method::GET, &format!("/guilds/{gid}"), Some(&owner), None).await;
    let cid = detail["channels"][0]["id"].as_str().unwrap().to_string();
    (owner, gid, cid)
}

async fn post_msg(router: &axum::Router, cookie: &str, cid: &str, body: &str) -> serde_json::Value {
    let (st, _, m) = common::send(
        router,
        Method::POST,
        &format!("/channels/{cid}/messages"),
        Some(cookie),
        Some(&json!({ "body": body })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    m
}

#[tokio::test]
async fn unread_counts_messages_past_the_cursor_and_baselines_unvisited() {
    let a = common::arena().await;
    let (owner, gid, cid) = setup(&a.router).await;

    let m1 = post_msg(&a.router, &owner, &cid, "one").await;
    let _m2 = post_msg(&a.router, &owner, &cid, "two").await;
    let m3 = post_msg(&a.router, &owner, &cid, "three").await;

    // No cursor yet → unread 0, but latest_* exposes the baseline.
    let (st, _, body) = common::send(&a.router, Method::GET, "/unread", Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    let rows = body["channels"].as_array().unwrap();
    let row = rows.iter().find(|r| r["channel_id"] == cid.as_str()).unwrap();
    assert_eq!(row["guild_id"], gid.as_str());
    assert_eq!(row["unread"], 0);
    assert_eq!(row["pinged"], false);
    assert_eq!(row["latest_id"], m3["id"]);

    // Mark read at m1 → exactly 2 unread (strict tie-break: m1 itself excluded).
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/mark-read"),
        Some(&owner),
        Some(&json!({ "sent_at": m1["sent_at"], "id": m1["id"] })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    let (_, _, body) = common::send(&a.router, Method::GET, "/unread", Some(&owner), None).await;
    let rows = body["channels"].as_array().unwrap();
    let row = rows.iter().find(|r| r["channel_id"] == cid.as_str()).unwrap();
    assert_eq!(row["unread"], 2, "m2 and m3 are unread; m1 (the cursor) is not");
}

#[tokio::test]
async fn unread_ping_flag_fires_only_on_unread_mentions() {
    let a = common::arena().await;
    let (owner, _gid, cid) = setup(&a.router).await;
    // A second member who will be pinged.
    let buddy = common::register_account(&a.router, "UnreadBuddy", "password123").await;
    // Join flow: use the real membership route (check src/server/mod.rs for the
    // invite/join shape; in tests other suites add members via the join/invite
    // endpoint — copy the pattern from tests/guilds.rs).
    common::join_guild(&a.router, &buddy, &cid).await;

    let m1 = post_msg(&a.router, &owner, &cid, "hello").await;
    // Baseline buddy at m1, then ping them.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        &format!("/channels/{cid}/mark-read"),
        Some(&buddy),
        Some(&json!({ "sent_at": m1["sent_at"], "id": m1["id"] })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    post_msg(&a.router, &owner, &cid, "@UnreadBuddy the watch begins").await;

    let (_, _, body) = common::send(&a.router, Method::GET, "/unread", Some(&buddy), None).await;
    let rows = body["channels"].as_array().unwrap();
    let row = rows.iter().find(|r| r["channel_id"] == cid.as_str()).unwrap();
    assert_eq!(row["unread"], 1);
    assert_eq!(row["pinged"], true);
}

#[tokio::test]
async fn unread_lists_only_channels_the_caller_can_see() {
    let a = common::arena().await;
    let (_owner, _gid, cid) = setup(&a.router).await;
    let outsider = common::register_account(&a.router, "UnreadOutsider", "password123").await;

    let (st, _, body) =
        common::send(&a.router, Method::GET, "/unread", Some(&outsider), None).await;
    assert_eq!(st, StatusCode::OK);
    let rows = body["channels"].as_array().unwrap();
    assert!(
        rows.iter().all(|r| r["channel_id"] != cid.as_str()),
        "foreign channels must not appear in /unread"
    );
}
```

NOTE on `common::join_guild`: check `tests/guilds.rs` / `tests/mentions.rs` for how a second member joins (invite endpoint or direct `guild_member` CREATE via `a.db`). If no helper exists, add one to `tests/common/mod.rs` replicating the existing suites' pattern EXACTLY — `tests/mentions.rs` must already create multi-member channels for ping tests; copy that mechanism verbatim into a shared helper and refactor the borrowing suite to use it.

- [ ] **Step 8.3: Run to verify failure**

Run: `cargo test --features ssr --test unread 2>&1 | tail -8`
Expected: FAIL — 404 on `/unread` (no route).

- [ ] **Step 8.4: Implement the endpoint**

Create `src/server/messages/unread.rs`:

```rust
//! GET /unread — M1 batched unread/ping summary for every visible text
//! channel, in ONE WebSocket round-trip. Replaces the client's N-per-channel
//! message probes. Statement batch is built with loop-INDEXED bind names only
//! (sanctioned by the parameterized-SQL invariant — no user value is ever
//! spliced into query text).

use crate::protocol::{ChannelUnread, UnreadResponse};
use crate::server::auth::session::AuthAccount;
use crate::server::events::visible_channels;
use crate::server::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};

use super::read_state::to_rfc3339_fixed; // make pub(super) if private (step 8.5)

#[derive(surrealdb::types::SurrealValue)]
struct CursorRow {
    channel_id: String,
    last_seen_at: surrealdb::types::Datetime,
    last_seen_id: String,
}

#[derive(surrealdb::types::SurrealValue)]
struct LatestRow {
    id_key: String,
    sent_at: surrealdb::types::Datetime,
}

pub async fn unread(State(state): State<AppState>, account: AuthAccount) -> Response {
    let result: surrealdb::Result<UnreadResponse> = async {
        let channels = visible_channels(&state, &account.0).await?;

        // Round-trip 1: the caller's cursors.
        let mut resp = state
            .db
            .query(
                "SELECT meta::id(channel) AS channel_id, last_seen_at, last_seen_id
                     FROM channel_read_state
                     WHERE account = type::record('account', $account);",
            )
            .bind(("account", account.0.clone()))
            .await?
            .check()?;
        let cursors: Vec<CursorRow> = resp.take(0)?;
        let cursor_by_channel: std::collections::HashMap<String, (surrealdb::types::Datetime, String)> =
            cursors
                .into_iter()
                .map(|c| (c.channel_id, (c.last_seen_at, c.last_seen_id)))
                .collect();

        // Round-trip 2: one batched multi-statement query. Per channel WITH a
        // cursor: unread ids (LIMIT 100) + ping probe (LIMIT 1). Per channel
        // WITHOUT: latest row for client baselining. Bind names use the loop
        // index; statement kinds are tracked for take()-ordering.
        enum Stmt {
            Unread(usize), // index into `out`
            Ping(usize),
            Latest(usize),
        }
        let mut sql = String::new();
        let mut kinds: Vec<Stmt> = Vec::new();
        let mut out: Vec<ChannelUnread> = channels
            .iter()
            .map(|(cid, gid)| ChannelUnread {
                channel_id: cid.clone(),
                guild_id: gid.clone(),
                unread: 0,
                pinged: false,
                latest_sent_at: None,
                latest_id: None,
            })
            .collect();

        let mut q = state.db.query(""); // placeholder; rebuilt below
        // Build SQL + bind list first, then create the query once.
        let mut binds: Vec<(String, surrealdb::types::Value)> = Vec::new();
        let _ = q; // discard placeholder pattern note

        for (i, (cid, _gid)) in channels.iter().enumerate() {
            if let Some((at, id)) = cursor_by_channel.get(cid) {
                sql.push_str(&format!(
                    "SELECT VALUE meta::id(id) FROM message \
                     WHERE channel = type::record('channel', $cid_{i}) AND deleted_at = NONE \
                       AND (sent_at > $at_{i} OR (sent_at = $at_{i} AND meta::id(id) > $mid_{i})) \
                     LIMIT 100;\n"
                ));
                kinds.push(Stmt::Unread(i));
                sql.push_str(&format!(
                    "SELECT VALUE meta::id(id) FROM message \
                     WHERE channel = type::record('channel', $cid_{i}) AND deleted_at = NONE \
                       AND (sent_at > $at_{i} OR (sent_at = $at_{i} AND meta::id(id) > $mid_{i})) \
                       AND type::record('account', $acct) IN (pinged_users ?? []) \
                     LIMIT 1;\n"
                ));
                kinds.push(Stmt::Ping(i));
                binds.push((format!("at_{i}"), at.clone().into()));
                binds.push((format!("mid_{i}"), id.clone().into()));
            } else {
                sql.push_str(&format!(
                    "SELECT meta::id(id) AS id_key, sent_at FROM message \
                     WHERE channel = type::record('channel', $cid_{i}) AND deleted_at = NONE \
                     ORDER BY sent_at DESC, id_key DESC LIMIT 1;\n"
                ));
                kinds.push(Stmt::Latest(i));
            }
            binds.push((format!("cid_{i}"), cid.clone().into()));
        }

        if !sql.is_empty() {
            let mut query = state.db.query(sql).bind(("acct", account.0.clone()));
            for (name, value) in binds {
                query = query.bind((name, value));
            }
            let mut resp = query.await?.check()?;
            for (stmt_idx, kind) in kinds.iter().enumerate() {
                match kind {
                    Stmt::Unread(i) => {
                        let ids: Vec<String> = resp.take(stmt_idx)?;
                        out[*i].unread = ids.len();
                    }
                    Stmt::Ping(i) => {
                        let ids: Vec<String> = resp.take(stmt_idx)?;
                        out[*i].pinged = !ids.is_empty();
                    }
                    Stmt::Latest(i) => {
                        let latest: Option<LatestRow> = resp.take(stmt_idx)?;
                        if let Some(l) = latest {
                            out[*i].latest_sent_at = Some(to_rfc3339_fixed(l.sent_at));
                            out[*i].latest_id = Some(l.id_key);
                        }
                    }
                }
            }
        }

        Ok(UnreadResponse { channels: out })
    }
    .await;

    match result {
        Ok(body) => Json(body).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "unread failed");
            crate::server::error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}
```

ADAPTATION NOTES (deterministic, not optional): (a) the `.into()` conversions of `Datetime`/`String` into bind values must match the SDK's bind API — if `query.bind((String, T))` accepts typed values directly (it does elsewhere in this codebase via tuples), bind `at.clone()` / `id.clone()` without the `Value` indirection and type `binds` accordingly; copy the exact bind style from `read_state.rs`. (b) `error_response` lives where the other handlers import it from — copy their `use` line. (c) Remove the placeholder `let mut q`/`let _ = q` lines — they exist only in this plan text to flag the build-then-bind order; the final code builds `sql`+`binds` first and constructs the query once, exactly as shown below them.

- [ ] **Step 8.5: Wire visibility + route + helper visibility**

- In `src/server/messages/mod.rs`: add `pub mod unread;` (match the existing submodule declarations) and re-export if the file re-exports handlers (`pub use unread::unread;`).
- In `src/server/mod.rs` `small_body_routes()`: `.route("/unread", get(messages::unread::unread))` (or via the re-export, matching style).
- In `src/server/messages/read_state.rs`: change `fn to_rfc3339_fixed` to `pub(super) fn to_rfc3339_fixed` if it is private.

- [ ] **Step 8.6: Run the unread suite to green, then the world**

Run: `cargo test --features ssr --test unread 2>&1 | tail -8` — expected: 3 PASS.
Run: `cargo test --features ssr 2>&1 | tail -3` — expected: green.

- [ ] **Step 8.7: Commit**

```bash
git add src/protocol.rs src/server/ tests/unread.rs tests/common/mod.rs
git commit -m "feat(server): batched GET /unread — one round-trip for all visible channels

Loop-indexed bind names (sanctioned splice form), strict composite
cursor tie-break matching the message cursor invariant, ping probe via
pinged_users, latest-row baseline fields for never-visited channels.

Tests: unread_counts_messages_past_the_cursor_and_baselines_unvisited, unread_ping_flag_fires_only_on_unread_mentions, unread_lists_only_channels_the_caller_can_see"
```

---

### Task 9: Media `Cache-Control` headers

**Files:**
- Modify: `src/server/media.rs:301–340` (`serve_original`, `jpeg_response`)
- Test: `tests/media.rs` (append)

- [ ] **Step 9.1: Failing test**

Append to `tests/media.rs` (reuse that file's existing upload helper to obtain a media id — copy the upload pattern from its first test):

```rust
#[tokio::test]
async fn media_responses_are_immutably_cacheable() {
    let a = common::arena().await;
    // Reuse this suite's existing helper/pattern to register + upload a PNG and
    // get back the media id (copy from the upload test above this one).
    let (cookie, media_id) = upload_test_png(&a).await;

    let req = common::build_json_request(
        Method::GET,
        &format!("/media/{media_id}"),
        Some(&cookie),
        None,
    );
    let resp = a.router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let cc = resp
        .headers()
        .get(axum::http::header::CACHE_CONTROL)
        .expect("media must carry Cache-Control")
        .to_str()
        .unwrap();
    assert_eq!(cc, "public, max-age=31536000, immutable");
}
```

(If `tests/media.rs` has no reusable upload helper, extract one from its first upload test — same multipart body, same assertions — rather than inventing a new pattern. `upload_test_png` returns `(session_cookie, media_id)`.)

- [ ] **Step 9.2: Run to verify failure**

Run: `cargo test --features ssr --test media media_responses_are_immutably 2>&1 | tail -5`
Expected: FAIL — header missing.

- [ ] **Step 9.3: Implement**

Media ids are server-minted random 16-byte ids and blobs are never replaced in place — responses are immutable by construction. In `src/server/media.rs`, add the header to BOTH response builders:

In `serve_original` (both arms) and `jpeg_response`, extend the header tuple arrays:

```rust
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable".to_string()),
```

(in `jpeg_response` the literals are `&'static str` — use `"public, max-age=31536000, immutable"` to match the array's type.)

IMPORTANT: media routes live in `media_routes()`, NOT under `small_body_routes()`'s `no-store` response layer — verify by reading `src/server/mod.rs` (Task 4 touched it) and confirm `tests/cache_control.rs` (which pins `no-store` on JSON) still passes.

- [ ] **Step 9.4: Run + commit**

Run: `cargo test --features ssr --test media 2>&1 | tail -3` and `cargo test --features ssr --test cache_control 2>&1 | tail -3` — both green.

```bash
git add src/server/media.rs tests/media.rs
git commit -m "perf(media): immutable Cache-Control on blob and thumbnail responses

Ids are random and content-addressed-in-practice (never replaced), so
1y/immutable is safe; PWA avatar refetch chatter disappears.

Tests: media_responses_are_immutably_cacheable"
```

---

### Task 10: Fold attachment MIME into the message page query

**Files:**
- Modify: `src/server/messages/reading.rs` (projection + row mapping; delete `resolve_attachment_mimes`)
- Test: `tests/messages.rs` (append)

- [ ] **Step 10.1: Failing-or-pinning test**

Append to `tests/messages.rs` (reuse the existing attachment-upload pattern in this file — it already posts messages with attachments):

```rust
#[tokio::test]
async fn list_messages_returns_attachment_mime_without_a_second_query_path() {
    let a = common::arena().await;
    let (owner, _gid, cid) = owner_with_text_channel(&a.router).await;
    // Reuse this suite's existing helper to upload a PNG and post a message
    // carrying it as an attachment (copy the pattern from the attachment tests
    // above), then:
    let (st, _, body) = common::send(
        &a.router,
        Method::GET,
        &format!("/channels/{cid}/messages"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let att = &body["messages"][0]["attachments"][0];
    assert_eq!(att["mime"], "image/png", "mime must ride the page response");
}
```

(This may already pass via `resolve_attachment_mimes` — it then acts as the behavior pin while the implementation is swapped underneath.)

- [ ] **Step 10.2: Run — expected PASS (pin), note it**

Run: `cargo test --features ssr --test messages list_messages_returns_attachment_mime 2>&1 | tail -3`

- [ ] **Step 10.3: Swap the implementation**

In `src/server/messages/reading.rs`:

1. Add one projected field to `MSG_PROJECTION` (inside the const, after the `attachments` line):

```
        (SELECT meta::id(id) AS id, mime FROM media_blob
            WHERE meta::id(id) IN ($parent.attachments ?? [])) AS attachment_mimes,
```

(If `$parent` is not accepted in this SurrealDB version's projection subquery, the equivalent accepted form is `WHERE meta::id(id) IN (attachments ?? [])` — try `$parent` first, fall back, and KEEP whichever passes the pin test.)

2. Extend the row struct that `MSG_PROJECTION` deserializes into (find the `#[derive(SurrealValue)]` struct with `id_key`, `author_key`, …) with:

```rust
    #[surreal(default)]
    attachment_mimes: Vec<MimePair>,
```

```rust
#[derive(surrealdb::types::SurrealValue, Default)]
struct MimePair {
    id: String,
    mime: String,
}
```

(match the actual derive-attribute spelling used elsewhere in the file for defaults; if none exists, make the field `Option<Vec<MimePair>>`.)

3. In the envelope-mapping code (where the row becomes `MessageEnvelope`), merge mimes exactly as `resolve_attachment_mimes` did:

```rust
        let mimes: std::collections::HashMap<String, String> = row
            .attachment_mimes
            .into_iter()
            .map(|p| (p.id, p.mime))
            .collect();
        for att in envelope.attachments.iter_mut() {
            if let Some(m) = mimes.get(&att.id) {
                att.mime = m.clone();
            }
        }
```

4. Delete `resolve_attachment_mimes` and all its call sites (grep: `grep -rn "resolve_attachment_mimes" src/`).

- [ ] **Step 10.4: Run the pins**

Run: `cargo test --features ssr --test messages 2>&1 | tail -3` — green (especially the new pin + every existing attachment test).
Run: `cargo test --features ssr 2>&1 | tail -3` — green.

- [ ] **Step 10.5: Commit**

```bash
git add src/server/messages/reading.rs tests/messages.rs
git commit -m "perf(messages): attachment MIME joins the page projection; second round-trip removed

resolve_attachment_mimes deleted; merge semantics preserved and pinned.

Tests: list_messages_returns_attachment_mime_without_a_second_query_path"
```

---

### Task 11: Client plumbing — `get_unread`, `SyncEvent` parsing, EventSource features

**Files:**
- Modify: `Cargo.toml` (web-sys features)
- Modify: `src/client/api.rs`

- [ ] **Step 11.1: web-sys features**

In `Cargo.toml`'s `web-sys` feature list, append:

```toml
    "EventSource",            # M1 SSE consumer
    "MessageEvent",           # SSE onmessage payloads
```

- [ ] **Step 11.2: API function**

Append to `src/client/api.rs` (next to the other GET wrappers):

```rust
/// GET /unread — batched unread/ping summary for every visible text channel (M1).
pub async fn get_unread() -> Result<crate::protocol::UnreadResponse, ApiError> {
    get("/unread").await
}
```

- [ ] **Step 11.3: Verify the hydrate graph compiles**

Run: `cargo clippy --features hydrate --target wasm32-unknown-unknown 2>&1 | tail -3`
Expected: no errors.

- [ ] **Step 11.4: Commit**

```bash
git add Cargo.toml Cargo.lock src/client/api.rs
git commit -m "feat(client): /unread API wrapper + EventSource web-sys features

Tests: hydrate graph clippy-clean (UI verification lands with the sync driver)"
```

---

### Task 12: Client sync driver — SSE with polling fallback, batched unread, lazy lists

**Files:**
- Create: `src/ui/shell/act/sync.rs`
- Modify: `src/ui/shell/act/mod.rs` (module + re-export)
- Modify: `src/ui/shell/act/message.rs` (extract `refresh_open_channel`; rewrite `refresh_unread`; slim `refresh_lists`; keep `start_poll` as fallback)

This task is hydrate-side: there is no wasm test harness, so verification is (a) `cargo clippy --features hydrate --target wasm32-unknown-unknown`, (b) `cargo leptos build` succeeding, (c) the manual smoke in Task 13. Keep every function in the established hydrate-real/ssr-stub pair pattern.

- [ ] **Step 12.1: Extract the open-channel refresh from the poll body**

In `src/ui/shell/act/message.rs`, extract the body of the per-tick message fetch (the `match api::list_messages(&ch.id, None).await { … }` block including both reconcile paths and stale-guards, lines ~1158–1188) into:

```rust
/// One open-channel sync pass: full reconcile for short histories, cursor
/// append for long ones. Safe to call from the poll loop AND from SSE events.
#[cfg(feature = "hydrate")]
pub(super) async fn refresh_open_channel(s: Shell) {
    if s.sync.pane.get_untracked() != Pane::Channel {
        return;
    }
    let Some(ch) = s.sel.sel_channel.get_untracked() else {
        return;
    };
    // <moved body, verbatim, with `continue` → `return`>
}
```

and make `start_poll`'s loop call `refresh_open_channel(s).await;` where the block used to be. NO behavior change.

- [ ] **Step 12.2: Rewrite `refresh_unread` on top of `/unread`**

Replace the per-channel loop body of `refresh_unread` (message.rs:848–946) with one call (the open-channel "always seen" prelude at the top of the function stays VERBATIM):

```rust
    spawn_local(async move {
        let Ok(r) = api::get_unread().await else {
            return;
        };
        for row in r.channels {
            if Some(&row.channel_id) == open.as_ref() {
                continue;
            }
            let has_cursor = s
                .notify
                .last_seen
                .with_untracked(|m| m.contains_key(&row.channel_id));
            if !has_cursor {
                // Never-visited: baseline silently from the server's latest.
                if let (Some(at), Some(id)) = (row.latest_sent_at, row.latest_id) {
                    set_last_seen(s, &row.channel_id, (at, id));
                }
                continue;
            }
            let has_new = row.unread > 0;
            s.notify.unread.update(|u| {
                if has_new { u.insert(row.channel_id.clone()); } else { u.remove(&row.channel_id); }
            });
            s.notify.pinged.update(|p| {
                if row.pinged { p.insert(row.channel_id.clone()); } else { p.remove(&row.channel_id); }
            });
            s.notify.unread_count.update(|c| {
                if has_new { c.insert(row.channel_id.clone(), row.unread); } else { c.remove(&row.channel_id); }
            });
        }
    });
```

NOTE: the server already computes unread against ITS cursor store; the client `last_seen` check above only gates baselining semantics (preserving today's "first sight never glows" behavior). `guild_has_unread` (rail dots) currently derives guild state from the channel cache — find it (`grep -n "guild_has_unread" src/ui/shell/`) and, if it walks `guild_channels`, add a parallel map `s.notify.unread_guilds: RwSignal<HashSet<String>>` populated in the loop above from `row.guild_id`, and switch `guild_has_unread` to read it. Add the signal to `Notify` in `state.rs` following the existing field style.

- [ ] **Step 12.3: Slim `refresh_lists`**

In `refresh_lists` (message.rs:1020–1076): DELETE the cross-guild `join_all(get_guild)` block (the `gids`/`details`/`next` section). Keep guilds + friends refresh. Then fetch ONLY the selected guild's channels:

```rust
        if let Some(gid) = sel {
            if let Ok(d) = api::get_guild(&gid).await {
                s.sel
                    .guild_channels
                    .update(|m| { m.insert(gid.clone(), d.channels.clone()); });
                if s.sel.channels.with_untracked(|c| *c != d.channels) {
                    s.sel.channels.set(d.channels);
                }
            }
        }
```

(`open_server` already fetches detail on guild switch; this keeps the open guild fresh on lists_changed.)

- [ ] **Step 12.4: The SSE driver**

Create `src/ui/shell/act/sync.rs`:

```rust
//! M1 sync driver (hydrate-real / ssr no-op): an EventSource on /events
//! dispatches notify-and-fetch refreshes; the legacy 1.5s poll loop remains
//! the automatic fallback when SSE cannot hold a connection.

#[cfg(feature = "hydrate")]
mod real {
    use super::super::message;
    use super::super::Shell;
    use crate::client::api;
    use crate::protocol::SyncEvent;
    use leptos::task::spawn_local;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;

    /// Consecutive EventSource error threshold before we declare SSE dead and
    /// fall back to polling. The browser auto-reconnects between errors.
    const MAX_CONSECUTIVE_ERRORS: u32 = 5;

    pub fn start_sync(s: Shell) {
        if s.sync.polling.get_untracked() {
            return; // someone already started a driver (idempotent, like start_poll)
        }
        match web_sys::EventSource::new("/events") {
            Ok(es) => wire(s, es),
            Err(_) => message::start_poll(s), // ancient browser: legacy loop
        }
    }

    fn wire(s: Shell, es: web_sys::EventSource) {
        s.sync.polling.set(true); // reuse the existing "a driver runs" latch

        let errors = std::rc::Rc::new(std::cell::Cell::new(0u32));

        let on_message = {
            let errors = errors.clone();
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
                errors.set(0);
                let Some(txt) = ev.data().as_string() else { return };
                let Ok(event) = serde_json::from_str::<SyncEvent>(&txt) else { return };
                dispatch(s, event);
            })
        };
        es.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        let on_error = {
            let errors = errors.clone();
            let es2 = es.clone();
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                let n = errors.get() + 1;
                errors.set(n);
                if n >= MAX_CONSECUTIVE_ERRORS {
                    es2.close();
                    s.sync.polling.set(false); // release the latch…
                    message::start_poll(s); // …and hand over to the legacy loop
                }
            })
        };
        es.set_onerror(Some(on_error.as_ref().unchecked_ref()));

        // The EventSource lives for the whole session; one instance per shell
        // mount. Leak the closures deliberately (bounded: created once).
        on_message.forget();
        on_error.forget();

        // Initial sync: lists + unread once, immediately.
        message::refresh_lists_pub(s);
        message::refresh_unread_pub(s);
    }

    fn dispatch(s: Shell, event: SyncEvent) {
        match event {
            SyncEvent::MessageCreated { channel_id }
            | SyncEvent::MessageEdited { channel_id, .. }
            | SyncEvent::MessageDeleted { channel_id, .. }
            | SyncEvent::Typing { channel_id } => {
                let open = s.sel.sel_channel.get_untracked().map(|c| c.id);
                if open.as_deref() == Some(channel_id.as_str()) {
                    spawn_local(async move { message::refresh_open_channel(s).await });
                } else {
                    message::refresh_unread_pub(s);
                }
            }
            SyncEvent::ListsChanged => {
                message::refresh_lists_pub(s);
                message::refresh_unread_pub(s);
            }
        }
    }
}

#[cfg(feature = "hydrate")]
pub use real::start_sync;

#[cfg(not(feature = "hydrate"))]
pub fn start_sync(_s: super::Shell) {}
```

Supporting changes in `message.rs`: expose thin `pub(super)` wrappers `refresh_lists_pub(s)` / `refresh_unread_pub(s)` that call the private fns (or make the originals `pub(super)`) — match the module's existing visibility style. Update `act/mod.rs`: declare `mod sync;`, re-export `pub use sync::start_sync;`, and change the existing `start_sync` wrapper in `message.rs` (lines 1193–1198) — DELETE it (the new module owns the name) while keeping `start_poll` itself intact as the fallback.

- [ ] **Step 12.5: Compile gates**

Run: `cargo clippy --features hydrate --target wasm32-unknown-unknown 2>&1 | tail -5` — no errors.
Run: `cargo clippy --features ssr 2>&1 | tail -5` — no errors (ssr stubs line up).
Run: `cargo test --features ssr 2>&1 | tail -3` — green.

- [ ] **Step 12.6: Commit**

```bash
git add src/ui/shell/act/ src/ui/shell/state.rs
git commit -m "feat(client): SSE sync driver with automatic polling fallback

EventSource on /events dispatches notify-and-fetch refreshes;
refresh_unread now rides one batched /unread call (was N per-channel
probes) with guild rail dots from row.guild_id; refresh_lists no longer
fans get_guild across every guild each ~6s. start_poll survives as the
fallback after 5 consecutive SSE errors.

Tests: clippy clean on hydrate+ssr graphs; behavior smoke in M1 verification"
```

---

### Task 13: M1 verification gate

**Files:** none (verification only)

- [ ] **Step 13.1: Full quality gate**

```bash
cargo fmt --all
cargo test --features ssr 2>&1 | tail -5
cargo clippy --features ssr 2>&1 | tail -3
cargo clippy --features hydrate --target wasm32-unknown-unknown 2>&1 | tail -3
cargo clippy --features freya 2>&1 | tail -3
cargo build --bin authlyn-native --features freya 2>&1 | tail -3
```

Expected: fmt makes no diff (or commit it), all tests pass, all three clippy graphs clean, native builds.

- [ ] **Step 13.2: Live smoke (dev server)**

```bash
cargo leptos watch &
sleep 60   # first build is slow; wait for "listening on 127.0.0.1:3000"
```

Then verify with the browser (Playwright or manual): open http://127.0.0.1:3000, register, create a guild, open DevTools → Network: confirm ONE pending `/events` stream and NO repeating 1.5s `/channels/*/messages` polling while idle; post a message from a second browser profile and confirm it appears without a poll tick; kill the server briefly and confirm the client recovers (SSE reconnect or fallback polling). **Local dev only — never against prod.**

- [ ] **Step 13.3: Update CLAUDE.md's realtime line**

CLAUDE.md "Architecture" states: "Real-time is client polling + in-memory typing state … NOT LIVE SELECT". Update that sentence to:

```
- Real-time is **SSE (`GET /events`, tokio broadcast in `AppState.events`) with automatic client fallback to legacy polling**; typing remains in-memory (`AppState.typing`), broadcast on POST. NOT LIVE SELECT (tests only).
```

- [ ] **Step 13.4: Final M1 commit**

```bash
git add CLAUDE.md
git commit -m "docs(claude-md): realtime is now SSE with polling fallback (M1)"
```

---

## Done = M1 exit criteria

1. Idle client network: keep-alives only (verified in Step 13.2).
2. `tests/events.rs` (5 tests incl. two adversarial privacy tests), `tests/unread.rs` (3), `tests/sync_events.rs` (1), media cache pin, MIME pin — all green alongside the original 144.
3. All three clippy graphs clean; native client still builds.
4. CLAUDE.md realtime description updated.
5. Branch `mendicant-bias` NOT pushed (owner pushes/merges; push = deploy).

**Next plan:** `2026-06-10-mendicant-bias-m2-design-system.md` (written after M1 lands, against the then-current tree).
