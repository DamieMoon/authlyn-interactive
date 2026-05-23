//! `POST /rooms/{id}/keyshare` and `GET /rooms/{id}/keyshare/inbox`
//! (routing-plan step 5).
//!
//! Routes Olm-encrypted wire envelopes between devices through the
//! `keyshare_envelope` SurrealDB table. The server is a dumb relay: it
//! stores ciphertext + routing metadata only — never Olm/Megolm session
//! secrets. Trust-model anchor: `src/storage/schema.surql:6-9`.
//!
//! ## Stance
//!
//! - **Defensive** at the HTTP boundary: parse every field, validate FK
//!   existence with explicit pre-checks (so failures map to typed 401/404,
//!   not a SurrealDB FK-violation surfacing as 500), reject early. Sender
//!   identity comes from the `X-Device-Id` header only — never from the
//!   request body.
//! - **Mechanical sympathy** in the inbox drain: SELECT + DELETE share an
//!   MVCC snapshot via a `BEGIN TRANSACTION; … COMMIT TRANSACTION;` block.
//!   The SELECT captures the rows the caller will receive; the DELETE
//!   removes the same id set inside the same transaction. Concurrent
//!   drains by the same recipient contend on the DELETE — the loser's
//!   transaction surfaces a retryable write conflict and re-runs against
//!   a fresh snapshot where the winner's rows are already gone. The whole
//!   block is wrapped in [`with_write_conflict_retry`] for that retry.
//!   (DELETE's own `RETURN <projection>` was rejected: projections evaluate
//!   against the AFTER state, so `meta::id(sender_device)` errors with
//!   "Argument 1 was the wrong type. Expected `record` but found `NONE`".)
//!
//! ## Delete-on-read inbox (known limitation)
//!
//! `GET /rooms/{id}/keyshare/inbox` returns the envelopes AND deletes them
//! in the same transaction. If the HTTP response is lost in transit, those
//! envelopes are gone permanently — the recipient cannot decrypt anything
//! on the affected sender's Megolm session until that sender's next
//! rotation. Acceptable for v1: LIVE SELECT (step 8) is the primary
//! delivery channel, the inbox is for catch-up after disconnect, and
//! rotation is routine. `GET`-then-`DELETE` is the obvious upgrade path;
//! the schema doesn't need to change.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::{Datetime, SurrealValue};

use crate::crypto::OlmEnvelope;
use crate::protocol::{
    ErrorBody, InboxEnvelope, KeyshareDeposit, KeyshareDepositResponse, KeyshareInbox,
};
use crate::server::datetime::to_rfc3339_fixed;
use crate::server::keys::extract_device_id;
use crate::server::retry::with_write_conflict_retry;
use crate::server::state::AppState;

// ---------------------------------------------------------------------------
// POST /rooms/{id}/keyshare
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(sender_device, room = %room_id))]
pub async fn deposit_keyshare(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<KeyshareDeposit>, JsonRejection>,
) -> Response {
    let sender_id = match extract_device_id(&headers) {
        Some(id) => id,
        None => return error_response(StatusCode::UNAUTHORIZED, "missing X-Device-Id header"),
    };
    tracing::Span::current().record("sender_device", tracing::field::display(&sender_id));

    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => {
            tracing::warn!(rejection = %rej, "JSON extraction failed");
            return json_rejection_response(rej);
        }
    };

    // Self-deposit is meaningless and almost certainly a sender bug.
    if req.recipient_device == sender_id {
        return error_response(
            StatusCode::BAD_REQUEST,
            "cannot deposit a keyshare to yourself",
        );
    }

    // Envelope wire-shape checks. The server doesn't hold the recipient's
    // keys and can't tell good ciphertext from bad — base64 well-formedness
    // is the cheapest defense against schema corruption.
    if let Err(msg) = validate_envelope(&req.envelope) {
        return error_response(StatusCode::BAD_REQUEST, msg);
    }

    match persist_envelope(
        &state,
        &sender_id,
        &req.recipient_device,
        &room_id,
        &req.envelope,
    )
    .await
    {
        Ok(DepositOutcome::Created(id)) => {
            (StatusCode::CREATED, Json(KeyshareDepositResponse { id })).into_response()
        }
        Ok(DepositOutcome::UnknownSender) => {
            error_response(StatusCode::UNAUTHORIZED, "unknown sender device")
        }
        Ok(DepositOutcome::UnknownRecipient) => {
            error_response(StatusCode::NOT_FOUND, "recipient device not found")
        }
        Ok(DepositOutcome::UnknownRoom) => error_response(StatusCode::NOT_FOUND, "room not found"),
        Err(e) => {
            tracing::error!(error = %e, "persist_envelope failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

enum DepositOutcome {
    Created(String),
    UnknownSender,
    UnknownRecipient,
    UnknownRoom,
}

/// FK pre-checks (sender/recipient/room) followed by a CREATE.
///
/// **The pre-checks and the CREATE run in separate round-trips on purpose.**
/// SurrealDB does not enforce referential integrity for bare `TYPE
/// record<X>` columns (only `REFERENCE` clauses do, which the schema
/// deliberately avoids), so a CREATE with a dangling sender/recipient/room
/// pointer would succeed and write a junk row. Wrapping the four statements
/// in a single `db.query()` without BEGIN/COMMIT would let the CREATE fire
/// regardless of the pre-check results — the Rust-side `Ok(UnknownSender)`
/// branch would then return 401 to the caller *while* the dangling row was
/// already committed. The two-round-trip split eliminates that side-effect:
/// nothing is written when a pre-check fails.
///
/// There is a small TOCTOU window between the pre-check and the CREATE (an
/// admin could DELETE the device/room row in between). For step 5 v1 with
/// no admin surface, that window is acceptable; if it ever matters,
/// switching to a single BEGIN/COMMIT block with `IF … THROW …` gates is
/// the upgrade path (THROW string parsing in Rust is the cost).
///
/// Step 3's `persist_bundle` doesn't share this concern because it UPSERTs
/// the device row inside the same transaction as the OTKs.
async fn persist_envelope(
    state: &AppState,
    sender_id: &str,
    recipient_id: &str,
    room_id: &str,
    envelope: &OlmEnvelope,
) -> surrealdb::Result<DepositOutcome> {
    #[derive(SurrealValue)]
    struct ExistsRow {
        id_key: String,
    }
    #[derive(SurrealValue)]
    struct CreatedRow {
        id_key: String,
    }

    // Round 1: FK pre-checks (read-only — safe to issue in one query call
    // because none of them write).
    let pre_sql = r#"
        SELECT meta::id(id) AS id_key FROM type::record("device", $sender_id);
        SELECT meta::id(id) AS id_key FROM type::record("device", $recipient_id);
        SELECT meta::id(id) AS id_key FROM type::record("room", $room_id);
    "#;
    let mut pre = state
        .db
        .query(pre_sql)
        .bind(("sender_id", sender_id.to_string()))
        .bind(("recipient_id", recipient_id.to_string()))
        .bind(("room_id", room_id.to_string()))
        .await?
        .check()?;
    let sender: Option<ExistsRow> = pre.take(0)?;
    if sender.is_none() {
        return Ok(DepositOutcome::UnknownSender);
    }
    let recipient: Option<ExistsRow> = pre.take(1)?;
    if recipient.is_none() {
        return Ok(DepositOutcome::UnknownRecipient);
    }
    let room: Option<ExistsRow> = pre.take(2)?;
    if room.is_none() {
        return Ok(DepositOutcome::UnknownRoom);
    }

    // Round 2: CREATE — only reachable after every FK exists.
    let create_sql = r#"
        CREATE keyshare_envelope SET
            sender_device    = type::record("device", $sender_id),
            recipient_device = type::record("device", $recipient_id),
            room             = type::record("room", $room_id),
            olm_message_type = $msg_type,
            olm_message      = $ciphertext
        RETURN meta::id(id) AS id_key;
    "#;
    let mut resp = state
        .db
        .query(create_sql)
        .bind(("sender_id", sender_id.to_string()))
        .bind(("recipient_id", recipient_id.to_string()))
        .bind(("room_id", room_id.to_string()))
        .bind(("msg_type", i64::from(envelope.message_type)))
        .bind(("ciphertext", envelope.ciphertext.clone()))
        .await?
        .check()?;
    let created: Vec<CreatedRow> = resp.take(0)?;
    let id = created
        .into_iter()
        .next()
        .map(|r| r.id_key)
        .ok_or_else(|| {
            // CREATE … RETURN should always emit at least one row on
            // success; if the SDK ever surfaces an empty result instead of
            // erroring, prefer a 500 over a panic.
            surrealdb::Error::thrown("CREATE keyshare_envelope returned no rows".to_string())
        })?;
    Ok(DepositOutcome::Created(id))
}

fn validate_envelope(env: &OlmEnvelope) -> Result<(), &'static str> {
    if env.message_type != OlmEnvelope::TYPE_PREKEY && env.message_type != OlmEnvelope::TYPE_MESSAGE
    {
        return Err("invalid envelope message_type");
    }
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    B64.decode(&env.ciphertext)
        .map(|_| ())
        .map_err(|_| "invalid base64 in envelope ciphertext")
}

// ---------------------------------------------------------------------------
// GET /rooms/{id}/keyshare/inbox
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(recipient_device, room = %room_id))]
pub async fn drain_inbox(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let recipient_id = match extract_device_id(&headers) {
        Some(id) => id,
        None => return error_response(StatusCode::UNAUTHORIZED, "missing X-Device-Id header"),
    };
    tracing::Span::current().record("recipient_device", tracing::field::display(&recipient_id));

    match drain(&state, &recipient_id, &room_id).await {
        Ok(DrainOutcome::Ok(envelopes)) => {
            (StatusCode::OK, Json(KeyshareInbox { envelopes })).into_response()
        }
        Ok(DrainOutcome::UnknownRecipient) => {
            error_response(StatusCode::UNAUTHORIZED, "unknown recipient device")
        }
        Ok(DrainOutcome::UnknownRoom) => error_response(StatusCode::NOT_FOUND, "room not found"),
        Err(e) => {
            tracing::error!(error = %e, "drain_inbox failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

enum DrainOutcome {
    Ok(Vec<InboxEnvelope>),
    UnknownRecipient,
    UnknownRoom,
}

/// `BEGIN/COMMIT`-wrapped SELECT-then-DELETE so the captured rows and the
/// DELETE share one MVCC snapshot. Concurrent CREATEs landing after the
/// snapshot are NOT included in this drain (they remain in the table for
/// the next caller). Concurrent drains by the same recipient race on the
/// DELETE; the loser's transaction surfaces a retryable write conflict and
/// is re-run by `with_write_conflict_retry` against a fresh snapshot,
/// where the winner's rows are already gone.
async fn drain(
    state: &AppState,
    recipient_id: &str,
    room_id: &str,
) -> surrealdb::Result<DrainOutcome> {
    #[derive(SurrealValue)]
    struct ExistsRow {
        id_key: String,
    }
    #[derive(SurrealValue)]
    struct DrainedRow {
        sender_key: String,
        olm_message_type: i64,
        olm_message: String,
        // Raw `datetime` from the projected column — see the module-doc
        // cross-reference in the `BEGIN/COMMIT`-wrapped SELECT below for
        // why the historic `<string>created_at AS created_at` cast was
        // dropped. Converted to a fixed 9-digit RFC 3339 string at
        // envelope build via `to_rfc3339_fixed`. Same convention as
        // `server::messages::MessageRow.sent_at`.
        created_at: Datetime,
    }

    // Pre-check FKs (read-only, two cheap SELECTs, no transaction needed).
    let pre_sql = r#"
        SELECT meta::id(id) AS id_key FROM type::record("device", $recipient_id);
        SELECT meta::id(id) AS id_key FROM type::record("room", $room_id);
    "#;
    let mut pre = state
        .db
        .query(pre_sql)
        .bind(("recipient_id", recipient_id.to_string()))
        .bind(("room_id", room_id.to_string()))
        .await?
        .check()?;
    let rec: Option<ExistsRow> = pre.take(0)?;
    if rec.is_none() {
        return Ok(DrainOutcome::UnknownRecipient);
    }
    let room: Option<ExistsRow> = pre.take(1)?;
    if room.is_none() {
        return Ok(DrainOutcome::UnknownRoom);
    }

    // BEGIN/COMMIT-wrapped SELECT-then-DELETE.
    //
    // The DELETE happens AFTER the SELECT inside the same MVCC transaction,
    // so two concurrent drains contend on the DELETE: one wins, the loser
    // sees a write conflict and retries against a fresh snapshot where the
    // winning drain's rows are already gone. That preserves the
    // at-most-once delivery invariant.
    //
    // DELETE's `RETURN <projection>` is unusable here — projection runs
    // against the AFTER state, which for DELETE is `NONE`, so
    // `meta::id(sender_device)` errors with "Argument 1 was the wrong
    // type. Expected `record` but found `NONE`". The SELECT-then-DELETE
    // pattern dodges that and gives us a place to `ORDER BY created_at`
    // under SurrealDB's native datetime semantics.
    //
    // `created_at` is projected RAW — no `<string>` cast. The historic
    // cast routed the column through `Datetime::Display` →
    // `SecondsFormat::AutoSi`, which emits variable-length sub-second
    // suffixes and lex-mis-orders rows at format-class boundaries (`Z`
    // < `.NNNZ` lexicographically but the other way chronologically).
    // Mirrors `server::messages::load_messages`; same module-doc finding
    // applies (`messages.rs` "Composite cursor empirical findings" §2).
    // Rust-side conversion to the wire `String` happens at envelope
    // build below via `to_rfc3339_fixed`.
    let drain_sql = r#"
        BEGIN TRANSACTION;
        LET $dev  = type::record("device", $recipient_id);
        LET $room = type::record("room", $room_id);
        LET $rows = SELECT
            id,
            meta::id(sender_device) AS sender_key,
            olm_message_type,
            olm_message,
            created_at
        FROM keyshare_envelope
        WHERE recipient_device = $dev AND room = $room
        ORDER BY created_at ASC;
        DELETE keyshare_envelope WHERE id IN $rows.id;
        $rows;
        COMMIT TRANSACTION;
    "#;
    let rows: Vec<DrainedRow> = with_write_conflict_retry(|| async {
        let mut resp = state
            .db
            .query(drain_sql)
            .bind(("recipient_id", recipient_id.to_string()))
            .bind(("room_id", room_id.to_string()))
            .await?
            .check()?;
        // Statement indices in the BEGIN/COMMIT block:
        //   0  BEGIN
        //   1  LET $dev
        //   2  LET $room
        //   3  LET $rows = SELECT ...
        //   4  DELETE ...
        //   5  $rows;            ← the captured rows we want
        //   6  COMMIT
        resp.take::<Vec<DrainedRow>>(5)
    })
    .await?;

    // `olm_message_type` is an `i64` from the DB; deposits validate it to
    // `{0, 1}`, but a future migration / admin write / schema drift could
    // put a value outside `u8` into the column. `as u8` would silently
    // truncate (e.g. `256` → `0`, which would falsely look like a PreKey
    // envelope to recipients and would burn an OTK on garbage). Surface
    // the corruption as a typed error instead.
    let mut envelopes: Vec<InboxEnvelope> = Vec::with_capacity(rows.len());
    for r in rows {
        let message_type = u8::try_from(r.olm_message_type).map_err(|_| {
            surrealdb::Error::thrown(format!(
                "keyshare_envelope row has out-of-range olm_message_type: {}",
                r.olm_message_type
            ))
        })?;
        envelopes.push(InboxEnvelope {
            sender_device: r.sender_key,
            envelope: OlmEnvelope {
                message_type,
                ciphertext: r.olm_message,
            },
            created_at: to_rfc3339_fixed(r.created_at),
        });
    }
    // DELETE has no intrinsic ordering guarantee — sort client-side for
    // deterministic delivery order. `created_at` is a fixed 9-digit
    // RFC 3339 string post-`to_rfc3339_fixed`, so lex compare matches
    // chronological order.
    envelopes.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(DrainOutcome::Ok(envelopes))
}

// ---------------------------------------------------------------------------
// HTTP response shaping (identical pattern to keys.rs)
// ---------------------------------------------------------------------------

fn error_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(ErrorBody::new(msg))).into_response()
}

fn json_rejection_response(rej: JsonRejection) -> Response {
    let reason: &'static str = match rej {
        JsonRejection::JsonDataError(_) => "invalid JSON body shape",
        JsonRejection::JsonSyntaxError(_) => "malformed JSON",
        JsonRejection::MissingJsonContentType(_) => "missing Content-Type: application/json",
        JsonRejection::BytesRejection(_) => "could not read request body",
        // `JsonRejection` is `#[non_exhaustive]`; catch-all required.
        _ => "invalid JSON request",
    };
    error_response(StatusCode::BAD_REQUEST, reason)
}
