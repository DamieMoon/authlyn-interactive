//! Wave-1 SAFETY-NET: feedback submit + admin-gate characterization
//! (`src/server/feedback.rs`, audit 019e6c08, invariant #11).
//!
//! Locks current behavior:
//!   - `POST /feedback` (any authed): body 1–4000 chars (trimmed); empty → 400,
//!     over-4000 → 400; `kind` is COERCED to {bug,idea,other} (never rejected);
//!     success → 201;
//!   - admin-gate FAIL-CLOSED: with no `AUTHLYN_ADMIN_USERNAMES` in the test env
//!     the admin set is empty → EVERY caller is non-admin → `GET /feedback` and
//!     `DELETE /feedback/{id}` both → 403. (The admin-ALLOWED path needs a
//!     process-wide env var that would race the parallel test workers, so it is
//!     intentionally NOT exercised here — see note below.)
//!   - archived (`status = 'deleted'`) rows are filtered from the inbox: the
//!     list handler's WHERE clause (`status != 'deleted'`, feedback.rs:133) is
//!     characterized directly against the DB, since the list route itself is
//!     admin-gated and unreachable without the env var above.

mod common;

#[cfg(feature = "ssr")]
use axum::http::{Method, StatusCode};
#[cfg(feature = "ssr")]
use serde_json::json;
#[cfg(feature = "ssr")]
use surrealdb::types::SurrealValue;

// ---------------------------------------------------------------------------
// POST /feedback — body bounds + kind coercion
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn submit_feedback_accepts_valid_kinds() {
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;

    for kind in ["bug", "idea", "other"] {
        let (st, _, _) = common::send(
            &a.router,
            Method::POST,
            "/feedback",
            Some(&user),
            Some(&json!({ "kind": kind, "body": "something is off" })),
        )
        .await;
        assert_eq!(st, StatusCode::CREATED, "kind={kind} should 201");
    }
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn submit_feedback_coerces_unknown_kind() {
    // `coerce_kind` maps anything outside {bug,idea} to "other" — an unknown kind
    // is NEVER a 400. Characterize via the stored row's `kind` field.
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;

    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/feedback",
        Some(&user),
        Some(&json!({ "kind": "wat-is-this", "body": "coerce me" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "unknown kind is accepted, not 400");

    #[derive(SurrealValue)]
    struct KindRow {
        kind: String,
    }
    let mut resp =
        a.db.query("SELECT kind FROM feedback WHERE body = 'coerce me';")
            .await
            .expect("query")
            .check()
            .expect("check");
    let row: Option<KindRow> = resp.take(0).expect("take");
    assert_eq!(
        row.expect("feedback row exists").kind,
        "other",
        "unknown kind coerced to 'other'"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn submit_feedback_body_bounds() {
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;

    // Empty body → 400.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/feedback",
        Some(&user),
        Some(&json!({ "kind": "bug", "body": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "empty body → 400");

    // Whitespace-only body trims to empty → 400.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/feedback",
        Some(&user),
        Some(&json!({ "kind": "bug", "body": "   \n\t " })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "whitespace body → 400");

    // Exactly 4000 chars (char-counted) → OK (boundary).
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/feedback",
        Some(&user),
        Some(&json!({ "kind": "bug", "body": "x".repeat(4000) })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "4000 chars is the upper boundary");

    // 4001 chars → 400.
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/feedback",
        Some(&user),
        Some(&json!({ "kind": "bug", "body": "x".repeat(4001) })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "4001 chars → 400");
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn submit_feedback_requires_auth() {
    let a = common::arena().await;
    let (st, _, _) = common::send(
        &a.router,
        Method::POST,
        "/feedback",
        None,
        Some(&json!({ "kind": "bug", "body": "anon" })),
    )
    .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

// ---------------------------------------------------------------------------
// Admin gate — fail-closed (empty admin set authorizes no one)
// ---------------------------------------------------------------------------
//
// NOTE: the admin-ALLOWED path is deliberately untested. `is_admin` reads the
// `AUTHLYN_ADMIN_USERNAMES`/`AUTHLYN_ADMIN_USERNAME` *process* env at call time;
// setting it would leak across the parallel test workers in this binary (and the
// whole suite), making other tests' fail-closed assertions flaky. The fail-closed
// branch is the security-critical one (invariant #11) and is fully covered here.

#[cfg(feature = "ssr")]
#[tokio::test]
async fn list_feedback_is_403_for_non_admin() {
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;

    // Even after submitting, the submitter is not an admin → cannot read the inbox.
    common::send(
        &a.router,
        Method::POST,
        "/feedback",
        Some(&user),
        Some(&json!({ "kind": "bug", "body": "mine" })),
    )
    .await;

    let (st, _, _) = common::send(&a.router, Method::GET, "/feedback", Some(&user), None).await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "no admins configured → every caller is non-admin → 403"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn delete_feedback_is_403_for_non_admin() {
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;

    // Create a real feedback row to target.
    common::send(
        &a.router,
        Method::POST,
        "/feedback",
        Some(&user),
        Some(&json!({ "kind": "bug", "body": "deletable" })),
    )
    .await;
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let mut resp =
        a.db.query("SELECT meta::id(id) AS id_key FROM feedback WHERE body = 'deletable';")
            .await
            .expect("query")
            .check()
            .expect("check");
    let id = resp
        .take::<Option<IdRow>>(0)
        .expect("take")
        .expect("row")
        .id_key;

    let (st, _, _) = common::send(
        &a.router,
        Method::DELETE,
        &format!("/feedback/{id}"),
        Some(&user),
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::FORBIDDEN,
        "delete is admin-gated; non-admin → 403"
    );

    // The row was NOT archived (still status='new'): the gate fired before the write.
    #[derive(SurrealValue)]
    struct StatusRow {
        status: String,
    }
    let mut resp =
        a.db.query("SELECT status FROM feedback WHERE body = 'deletable';")
            .await
            .expect("query")
            .check()
            .expect("check");
    let row: Option<StatusRow> = resp.take(0).expect("take");
    assert_eq!(
        row.expect("row").status,
        "new",
        "403 means the soft-delete write never ran"
    );
}

#[cfg(feature = "ssr")]
#[tokio::test]
async fn list_and_delete_require_auth() {
    let a = common::arena().await;
    let (st, _, _) = common::send(&a.router, Method::GET, "/feedback", None, None).await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "GET /feedback needs a session"
    );
    let (st, _, _) =
        common::send(&a.router, Method::DELETE, "/feedback/whatever", None, None).await;
    assert_eq!(
        st,
        StatusCode::UNAUTHORIZED,
        "DELETE /feedback/{{id}} needs a session"
    );
}

// ---------------------------------------------------------------------------
// Archived (status='deleted') rows are filtered from the inbox
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
#[tokio::test]
async fn archived_feedback_is_filtered_from_list_query() {
    // The list route is admin-gated (unreachable without the env var that would
    // race other tests). We characterize the EXACT inbox filter the handler uses
    // (`WHERE status != 'deleted'`, feedback.rs:133) directly against the DB:
    // submit two items, archive one, assert the filter returns only the live one.
    let a = common::arena().await;
    let user = common::register_account(&a.router, "User", "password123").await;

    for body in ["live-item", "archived-item"] {
        common::send(
            &a.router,
            Method::POST,
            "/feedback",
            Some(&user),
            Some(&json!({ "kind": "idea", "body": body })),
        )
        .await;
    }

    // Archive one (what `delete_feedback` does: flip status to 'deleted').
    a.db.query("UPDATE feedback SET status = 'deleted' WHERE body = 'archived-item';")
        .await
        .expect("archive")
        .check()
        .expect("archive check");

    // Replicate the handler's inbox query and assert the archived row is gone.
    #[derive(SurrealValue)]
    struct BodyRow {
        body: String,
    }
    // (The handler ORDERs BY created_at; we only assert set membership, so we
    // omit ORDER BY — SurrealDB requires an ORDER idiom to also be projected.)
    let mut resp =
        a.db.query("SELECT body FROM feedback WHERE status != 'deleted';")
            .await
            .expect("list query")
            .check()
            .expect("list check");
    let rows: Vec<BodyRow> = resp.take(0).expect("take");
    let bodies: Vec<String> = rows.into_iter().map(|r| r.body).collect();
    assert!(
        bodies.contains(&"live-item".to_string()),
        "live item present"
    );
    assert!(
        !bodies.contains(&"archived-item".to_string()),
        "archived (status='deleted') item filtered out of the inbox"
    );
}
