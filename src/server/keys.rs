//! `POST /keys/upload` and `POST /keys/claim/{user}/{device}`.
//!
//! ## Stance
//!
//! - **Defensive** at the HTTP boundary: parse every field, reject early,
//!   never let unverified key material reach the DB.
//! - **Offensive** inside the OTK pool: the `single-use` invariant is
//!   sacred — no two concurrent claimers may ever receive the same
//!   `prekey_otk` row. The atomic `DELETE FROM (SELECT ... LIMIT 1) RETURN
//!   BEFORE` pattern + SurrealDB MVCC make sure of that: the loser of any
//!   race surfaces a retryable write-conflict error rather than a
//!   double-claim.
//! - **Mechanical sympathy** in `pop_one_otk` / `persist_bundle`: write
//!   conflicts are the database asking us to retry with a fresh snapshot,
//!   not a hard failure. Both functions re-issue through
//!   [`with_write_conflict_retry`] with jittered backoff.
//!
//! ## Atomic OTK pop
//!
//! SurrealDB 3.x rejects `DELETE ... LIMIT 1` directly (parse error), so
//! we wrap the LIMIT in a sub-`SELECT`. The sub-`SELECT` is evaluated
//! inside the same statement, and `RETURN BEFORE` gives us the row's
//! contents as it existed pre-delete. Under concurrency, the MVCC
//! arbiter picks a winner per row; losing claimers retry against a fresh
//! snapshot, see the winner's row gone, and pick a different one. End
//! result: every concurrent claimer gets a distinct kid (see the
//! `concurrent_claims_each_get_distinct_otk` integration test).

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use surrealdb::types::SurrealValue;

use crate::crypto::{PreKeyBundle, PreKeyError, SignedPreKey};
use crate::protocol::{
    ClaimKeyResponse, ClaimKind, ErrorBody, UploadKeysRequest, UploadKeysResponse,
};
use crate::server::state::AppState;

/// Header carrying the calling device's ID in the v1 auth stub.
const DEVICE_HEADER: &str = "X-Device-Id";

// ---------------------------------------------------------------------------
// POST /keys/upload
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(device_id))]
pub async fn upload_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    // Take the JSON extraction as a `Result` so we can convert axum's
    // default plain-text rejection bodies into our typed `ErrorBody`.
    payload: Result<Json<UploadKeysRequest>, JsonRejection>,
) -> Response {
    // --- Auth stub: X-Device-Id is required and trusted as-is. ---
    let device_id = match extract_device_id(&headers) {
        Some(id) => id,
        None => {
            return error_response(StatusCode::UNAUTHORIZED, "missing X-Device-Id header");
        }
    };
    tracing::Span::current().record("device_id", &tracing::field::display(&device_id));

    // --- Defensive: surface JSON-extraction failures as typed 400s. ---
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => {
            tracing::warn!(rejection = %rej, "JSON extraction failed");
            return json_rejection_response(rej);
        }
    };

    // --- Defensive: cap OTK count before touching crypto or DB. ---
    if req.bundle.one_time_keys.len() > MAX_OTKS_PER_PUBLISH {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "too many one_time_keys: {} (max {})",
                req.bundle.one_time_keys.len(),
                MAX_OTKS_PER_PUBLISH
            ),
        );
    }

    // --- Defensive: validate the bundle in full before touching the DB. ---
    if let Err(e) = req.bundle.verify_self() {
        tracing::warn!(error = %e, "bundle verification failed");
        return error_response(StatusCode::BAD_REQUEST, format!("{e}"));
    }

    // --- Persist. Upserts are intentional: republish overwrites. ---
    if let Err(e) = persist_bundle(&state, &req.user_id, &device_id, &req.bundle).await {
        tracing::error!(error = %e, "persist_bundle failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }

    let resp = UploadKeysResponse {
        device_id,
        otk_count: req.bundle.one_time_keys.len(),
    };
    (StatusCode::OK, Json(resp)).into_response()
}

/// Upper bound on OTKs accepted per `/keys/upload`. Generous compared to a
/// normal Olm client (which publishes ~50), but bounded so a malicious
/// caller can't push us into a multi-megabyte verify loop.
pub const MAX_OTKS_PER_PUBLISH: usize = 200;

/// Convert an axum `JsonRejection` into a typed `{"error": "..."}` body
/// with status 400. We deliberately do NOT echo the raw rejection string
/// — its phrasing leaks implementation detail and isn't part of any
/// stable contract.
fn json_rejection_response(rej: JsonRejection) -> Response {
    // Pin the status code at 400: even axum's own 415 (missing content
    // type) and 422 (data error) variants are validation failures from
    // our caller's perspective, and the brief calls for a uniform 400.
    let reason: &'static str = match rej {
        JsonRejection::JsonDataError(_) => "invalid JSON body shape",
        JsonRejection::JsonSyntaxError(_) => "malformed JSON",
        JsonRejection::MissingJsonContentType(_) => "missing Content-Type: application/json",
        JsonRejection::BytesRejection(_) => "could not read request body",
        // `JsonRejection` is `#[non_exhaustive]`, so we need a catch-all.
        _ => "invalid JSON request",
    };
    error_response(StatusCode::BAD_REQUEST, reason)
}

fn extract_device_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get(DEVICE_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Upsert user + device rows; replace the OTK pool wholesale; replace the
/// fallback row. The whole sequence runs inside a single SurrealDB
/// transaction (`BEGIN TRANSACTION; ... COMMIT TRANSACTION;`) so a
/// concurrent claim can't observe a half-replaced pool (e.g. seeing the
/// `DELETE prekey_otk WHERE device = $dev` while the new OTKs haven't been
/// `CREATE`d yet, which would spuriously return the fallback key).
///
/// Concurrent claimers landing on the same device while this transaction
/// is in flight will lose the MVCC race and surface a retryable write
/// conflict; both this function and `pop_one_otk` re-issue under
/// [`with_write_conflict_retry`].
async fn persist_bundle(
    state: &AppState,
    user_id: &str,
    device_id: &str,
    bundle: &PreKeyBundle,
) -> surrealdb::Result<()> {
    // We send the OTKs as a single bound array; SurrealQL iterates via
    // `FOR $k IN $otks`. The derive macro doesn't tolerate lifetimes, so
    // we clone the strings into owned values for the bind.
    #[derive(SurrealValue, Clone)]
    struct OtkRow {
        kid: String,
        public_key: String,
        signature: String,
    }
    let otk_rows: Vec<OtkRow> = bundle
        .one_time_keys
        .iter()
        .map(|k| OtkRow {
            kid: k.kid.clone(),
            public_key: k.public_key.clone(),
            signature: k.signature.clone(),
        })
        .collect();

    // BEGIN/COMMIT-wrapped multi-statement block. The SurrealDB 3.x Rust
    // SDK `db.query(...)` honours these markers: if any inner statement
    // errors (or a `THROW` fires), every prior statement in the block is
    // rolled back and the response carries `query_type: Other,
    // results: Err("not executed due to a failed transaction")` for all
    // members of the block. Verified directly: a `BEGIN; CREATE x;
    // THROW; CREATE y; COMMIT;` leaves the table empty (CLI REPL is
    // misleading here — it sends statements individually and does NOT
    // get the same atomicity).
    //
    // Without this wrapper, each top-level statement runs in its own
    // tiny MVCC transaction, so a concurrent claim landing between
    // `DELETE prekey_otk` and the OTK `CREATE` loop would see an empty
    // pool and spuriously serve the fallback key.
    let sql = r#"
        BEGIN TRANSACTION;
        UPSERT type::record("user", $user_id) SET display_name = "";
        UPSERT type::record("device", $device_id)
            SET user = type::record("user", $user_id),
                identity_curve25519 = $id_curve,
                identity_ed25519    = $id_ed;
        LET $dev = type::record("device", $device_id);
        DELETE FROM prekey_otk WHERE device = $dev;
        DELETE FROM prekey_fallback WHERE device = $dev;
        FOR $k IN $otks {
            CREATE prekey_otk SET
                device     = $dev,
                kid        = $k.kid,
                public_key = $k.public_key,
                signature  = $k.signature;
        };
        CREATE prekey_fallback SET
            device     = $dev,
            kid        = $fallback_kid,
            public_key = $fallback_pk,
            signature  = $fallback_sig;
        COMMIT TRANSACTION;
    "#;

    with_write_conflict_retry(|| async {
        state
            .db
            .query(sql)
            .bind(("user_id", user_id.to_string()))
            .bind(("device_id", device_id.to_string()))
            .bind(("id_curve", bundle.identity_curve25519.clone()))
            .bind(("id_ed", bundle.identity_ed25519.clone()))
            .bind(("otks", otk_rows.clone()))
            .bind(("fallback_kid", bundle.fallback_key.kid.clone()))
            .bind(("fallback_pk", bundle.fallback_key.public_key.clone()))
            .bind(("fallback_sig", bundle.fallback_key.signature.clone()))
            .await?
            .check()?;
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// POST /keys/claim/{user}/{device}
// ---------------------------------------------------------------------------

/// Claim endpoint accepts an empty body (`{}`) for forward compatibility;
/// future revisions may carry the caller's identity-key fingerprint.
#[derive(Debug, Default, Deserialize)]
pub struct ClaimRequest {}

#[tracing::instrument(skip(state, _body), fields(target_user = %user, target_device = %device))]
pub async fn claim_key(
    State(state): State<AppState>,
    // Both path params are now load-bearing: `user` is cross-checked against
    // the device row's `user` foreign key, so a peer asking for a device
    // under the wrong user gets a 404 instead of a silent success. Keeps
    // the URL shape self-describing for symmetry with step 5's
    // `/rooms/:id/keyshare` (which has similar disambiguating params).
    Path((user, device)): Path<(String, String)>,
    // Body is optional and forward-compatible. `Option<Json<...>>` already
    // absorbs missing-content-type / missing-body cases into `None` (which
    // is what we want here), so the explicit `Result<.., JsonRejection>`
    // wrapper isn't needed for the current contract.
    _body: Option<Json<ClaimRequest>>,
) -> Response {
    // 1. Look up the device row. 404 if it isn't there.
    let identity = match load_device_identity(&state, &device).await {
        Ok(Some(id)) => id,
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "device not found");
        }
        Err(e) => {
            tracing::error!(error = %e, "load_device_identity failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    // 1b. Cross-check: the device row must belong to the user the caller
    // named in the path. Mismatch is a 404 rather than a 403 because we
    // don't want to leak "this device exists, just under someone else" —
    // from the peer's point of view, the (user, device) tuple they asked
    // for simply isn't there.
    if identity.user_key != user {
        tracing::warn!(
            requested_user = %user,
            actual_user = %identity.user_key,
            "device exists but belongs to a different user"
        );
        return error_response(StatusCode::NOT_FOUND, "device not found for that user");
    }

    // 2. Atomically pop one OTK.
    match pop_one_otk(&state, &device).await {
        Ok(Some(otk)) => {
            let resp = ClaimKeyResponse {
                kind: ClaimKind::Otk,
                device_id: device.clone(),
                identity_curve25519: identity.curve25519_hex,
                identity_ed25519: identity.ed25519_hex,
                key: otk,
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Ok(None) => {
            // No OTKs left — try the fallback.
            match load_fallback(&state, &device).await {
                Ok(Some(fb)) => {
                    tracing::warn!(
                        device = %device,
                        "OTK pool exhausted, returning fallback key"
                    );
                    let resp = ClaimKeyResponse {
                        kind: ClaimKind::Fallback,
                        device_id: device.clone(),
                        identity_curve25519: identity.curve25519_hex,
                        identity_ed25519: identity.ed25519_hex,
                        key: fb,
                    };
                    (StatusCode::OK, Json(resp)).into_response()
                }
                Ok(None) => error_response(StatusCode::SERVICE_UNAVAILABLE, "no keys available"),
                Err(e) => {
                    tracing::error!(error = %e, "load_fallback failed");
                    error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
                }
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "pop_one_otk failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

struct DeviceIdentity {
    curve25519_hex: String,
    ed25519_hex: String,
    /// String form of the device's owning user's record-id key (i.e. the
    /// part after `user:`). Used by [`claim_key`] to cross-check the
    /// `:user` path param.
    user_key: String,
}

async fn load_device_identity(
    state: &AppState,
    device_id: &str,
) -> surrealdb::Result<Option<DeviceIdentity>> {
    // `meta::id(user)` extracts the record-id key as a string, which
    // sidesteps having to teach `SurrealValue` how to deserialize a full
    // `RecordId` (table + typed key enum) when all we want is the key half
    // for an equality check.
    #[derive(SurrealValue)]
    struct Row {
        identity_curve25519: String,
        identity_ed25519: String,
        user_key: String,
    }
    let mut resp = state
        .db
        .query(
            "SELECT identity_curve25519, identity_ed25519, meta::id(user) AS user_key \
             FROM type::record('device', $device_id);",
        )
        .bind(("device_id", device_id.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.map(|r| DeviceIdentity {
        curve25519_hex: r.identity_curve25519,
        ed25519_hex: r.identity_ed25519,
        user_key: r.user_key,
    }))
}

#[derive(SurrealValue)]
struct PreKeyRow {
    kid: String,
    public_key: String,
    signature: String,
}

impl From<PreKeyRow> for SignedPreKey {
    fn from(r: PreKeyRow) -> Self {
        SignedPreKey {
            kid: r.kid,
            public_key: r.public_key,
            signature: r.signature,
        }
    }
}

/// Atomically remove one `prekey_otk` row for this device and return its
/// contents. Returns `Ok(None)` when the pool is empty.
///
/// ## Mechanical sympathy: coordinating with SurrealDB MVCC
///
/// SurrealDB 3.x runs every top-level statement inside an MVCC transaction.
/// Concurrent claimers racing on the same `device` will pick the *same*
/// "first row" inside their respective `SELECT ... LIMIT 1` snapshots,
/// then one of them wins the `DELETE`; the loser surfaces a retryable
/// `"Write conflict, retry the transaction."` error. This isn't pathology
/// — it's the database asking us to back off and try again with a fresh
/// snapshot, which on retry will see the winner's row gone and pick a
/// different one.
///
/// We retry up to [`MAX_WRITE_CONFLICT_ATTEMPTS`] times with jittered
/// backoff. Past that point we surrender and let the original error bubble
/// up — the handler maps it to HTTP 500.
async fn pop_one_otk(state: &AppState, device_id: &str) -> surrealdb::Result<Option<SignedPreKey>> {
    with_write_conflict_retry(|| async {
        let sql = r#"
            LET $dev = type::record("device", $device_id);
            DELETE FROM (SELECT * FROM prekey_otk WHERE device = $dev LIMIT 1) RETURN BEFORE;
        "#;
        let mut resp = state
            .db
            .query(sql)
            .bind(("device_id", device_id.to_string()))
            .await?
            .check()?;
        // Query 0 was the `LET`, the DELETE results are at index 1.
        let rows: Vec<PreKeyRow> = resp.take(1)?;
        Ok(rows.into_iter().next().map(SignedPreKey::from))
    })
    .await
}

async fn load_fallback(
    state: &AppState,
    device_id: &str,
) -> surrealdb::Result<Option<SignedPreKey>> {
    let mut resp = state
        .db
        .query("SELECT kid, public_key, signature FROM prekey_fallback WHERE device = type::record('device', $device_id) LIMIT 1;")
        .bind(("device_id", device_id.to_string()))
        .await?
        .check()?;
    let row: Option<PreKeyRow> = resp.take(0)?;
    Ok(row.map(SignedPreKey::from))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn error_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(ErrorBody::new(msg))).into_response()
}

// ---------------------------------------------------------------------------
// SurrealDB write-conflict retry
// ---------------------------------------------------------------------------

/// Cap on how many times we'll re-issue a SurrealDB statement that got
/// rejected with a retryable write conflict. 5 attempts gives us linear
/// backoff up to ~25ms + jitter — well under any meaningful client
/// timeout, but enough headroom for the contention windows we've seen
/// in concurrent claims (a fraction of a millisecond each).
const MAX_WRITE_CONFLICT_ATTEMPTS: u32 = 5;

/// Base unit of the backoff schedule. Attempt `n` waits
/// `BASE_BACKOFF_MS * n + jitter` ms before retrying.
const BASE_BACKOFF_MS: u64 = 5;

/// Run `op`, retrying when SurrealDB tells us the transaction lost an
/// MVCC race. Other errors propagate immediately.
///
/// The closure is invoked up to [`MAX_WRITE_CONFLICT_ATTEMPTS`] times.
/// Backoff is linear in attempt number plus a small jitter so concurrent
/// retriers desynchronise instead of stampeding the next slot together.
async fn with_write_conflict_retry<F, Fut, T>(mut op: F) -> surrealdb::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = surrealdb::Result<T>>,
{
    let mut last_err: Option<surrealdb::Error> = None;
    for attempt in 1..=MAX_WRITE_CONFLICT_ATTEMPTS {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) if is_write_conflict(&e) && attempt < MAX_WRITE_CONFLICT_ATTEMPTS => {
                // `rand::random::<u64>()` is good enough for jitter; we
                // don't need a cryptographic source here.
                let jitter = rand::random::<u64>() % 20;
                let backoff_ms = BASE_BACKOFF_MS * attempt as u64 + jitter;
                tracing::debug!(
                    attempt,
                    backoff_ms,
                    error = %e,
                    "SurrealDB write conflict, retrying"
                );
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                last_err = Some(e);
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    // We only fall through here if every attempt was a write conflict;
    // surface the last one so the caller can map it to 5xx.
    Err(last_err.expect("loop body always sets last_err before falling through"))
}

/// Identify SurrealDB write-conflict errors via their Display string. The
/// SDK exposes them as plain `surrealdb::Error` values rather than a typed
/// variant, so substring matching is the cheapest reliable test. Both the
/// "Write conflict" and the "retry the transaction" markers appear in the
/// 3.x error text we've observed in production (the full message reads
/// `"Query not executed: Transaction conflict: Write conflict, retry the
/// transaction. This transaction can be retried"`).
///
/// Exposed (`pub`, not `pub(crate)`) so the
/// `is_write_conflict_matches_real_surrealdb_conflict` regression test in
/// `tests/keys.rs` can call it directly. Integration tests are compiled as
/// a separate crate, so `pub(crate)` would not be reachable. That test
/// synthesizes a real MVCC conflict against the dev DB and asserts this
/// predicate still fires. The canary is here because rooms/Megolm in
/// steps 5+6 will copy the same retry pattern, and a silent SurrealDB
/// rename of either substring would disable the retry loop everywhere
/// without any compile-time signal.
pub fn is_write_conflict(err: &surrealdb::Error) -> bool {
    let s = err.to_string();
    s.contains("Write conflict") || s.contains("retry the transaction")
}

// Plumbing so `PreKeyError` flows naturally if we want to surface a more
// specific status later. Kept here, not in protocol.rs, so the WASM bundle
// doesn't drag in axum types.
impl From<PreKeyError> for ErrorBody {
    fn from(value: PreKeyError) -> Self {
        ErrorBody::new(format!("{value}"))
    }
}
