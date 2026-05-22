//! `POST /rooms`, `POST /rooms/{id}/join`, `POST /rooms/{id}/leave`
//! (routing-plan step 7).
//!
//! Server-side membership state machine. Each transition appends a
//! `room_event` row that step 8's LIVE SELECT subscribers will observe to
//! drive client-side Megolm rotation. No crypto in this step — the server
//! is the source of truth for "who is in this room right now" and the
//! ordered audit trail of transitions; sessions live client-side.
//!
//! ## Stance
//!
//! - **Defensive** at the HTTP boundary: every FK is pre-checked in a
//!   read-only round trip before any write fires, so unknown caller /
//!   room / target failures map to typed 401/404, not a 500 from a
//!   downstream SurrealDB error. Caller identity comes from
//!   `X-Device-Id` only — body fields name the *target*, never the
//!   inviter. Same pattern as `server::keyshare`.
//! - **Mechanical sympathy** for the concurrent-inviter race against the
//!   `room_member_pair (room, user) UNIQUE` index (schema.surql:51): two
//!   inviters can both observe "target not yet a member" inside their
//!   pre-check snapshots and race the write. SurrealDB's MVCC serialises
//!   the writes, one wins, the loser surfaces either a retryable
//!   write conflict (retried via [`with_write_conflict_retry`] against a
//!   fresh snapshot that *does* see the winner's row, which then
//!   resurfaces as a UNIQUE violation) or directly a UNIQUE violation
//!   (when the winner had already committed before the loser's CREATE).
//!   Either way the residual error matches
//!   [`is_unique_violation`](crate::server::retry::is_unique_violation)
//!   and the handler maps it to `409 "user is already a member"` — the
//!   same body as the pre-check 409 (dual-path mapping).
//!
//! ## Privacy 404s
//!
//! Non-member callers attempting `/join` or `/leave` on an existing room
//! receive `404 "room not found"`, NOT `403`. Returning 403 would
//! confirm the room exists; 404 keeps room membership a non-leaky
//! property. Same body shape as "room genuinely does not exist".
//!
//! ## Self-leave only
//!
//! `/leave` operates on `actor == target == caller_user`. Kicks aren't in
//! v1; the schema's `room_event.target: option<record<user>>` is sized
//! to admit them as a follow-up without migration.
//!
//! ## SurrealQL CREATE shape
//!
//! All three handlers use the `(CREATE ... RETURN ...)[0].id_key` form to
//! capture a freshly-created row's id inside a LET binding. The form was
//! verified empirically against SurrealDB 3.1.0-beta.3 (see the
//! plan-required probe at implementation time); the `CREATE ONLY ...`
//! fallback also parses but yields the same shape via `$var.id`, so we
//! stuck with the original form for symmetry with the existing
//! `server::keys` / `server::keyshare` patterns. The `RETURN ...;`
//! statement at the bottom of a `BEGIN/COMMIT` block surfaces at
//! `resp.take(N)` where N is the count of statements between BEGIN and
//! the RETURN inclusive (the BEGIN and COMMIT markers don't get their
//! own indices).

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use surrealdb::types::SurrealValue;

use crate::protocol::{
    CreateRoomRequest, CreateRoomResponse, ErrorBody, JoinRoomRequest, RoomEventResponse,
};
use crate::server::keys::extract_device_id;
use crate::server::retry::{is_unique_violation, with_write_conflict_retry};
use crate::server::state::AppState;

/// Soft cap on the rendered room name. `chars().count()` so multi-byte
/// graphemes count once each, not by byte.
const MAX_ROOM_NAME_CHARS: usize = 200;

// ---------------------------------------------------------------------------
// POST /rooms
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(caller_device, room))]
pub async fn create_room(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<CreateRoomRequest>, JsonRejection>,
) -> Response {
    let device_id = match extract_device_id(&headers) {
        Some(id) => id,
        None => return error_response(StatusCode::UNAUTHORIZED, "missing X-Device-Id header"),
    };
    tracing::Span::current().record("caller_device", &tracing::field::display(&device_id));

    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => {
            tracing::warn!(rejection = %rej, "JSON extraction failed");
            return json_rejection_response(rej);
        }
    };

    let name = req.name.trim();
    if name.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "room name must not be empty");
    }
    if name.chars().count() > MAX_ROOM_NAME_CHARS {
        return error_response(StatusCode::BAD_REQUEST, "room name too long");
    }

    // Resolve caller's user from the device row outside any transaction.
    let caller_user = match load_caller_user(&state, &device_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "unknown caller device"),
        Err(e) => {
            tracing::error!(error = %e, "load_caller_user failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };

    match persist_create_room(&state, &caller_user, name).await {
        Ok(CreateRoomOutcome::Created { room_id, event_id }) => {
            tracing::Span::current().record("room", &tracing::field::display(&room_id));
            (
                StatusCode::CREATED,
                Json(CreateRoomResponse {
                    id: room_id,
                    room_event_id: event_id,
                }),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "persist_create_room failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

enum CreateRoomOutcome {
    Created { room_id: String, event_id: String },
}

/// Atomically create the `room` row, the creator's `room_member` row, and
/// the bootstrapping `'create'` `room_event` row.
///
/// All three CREATEs sit inside a single `BEGIN/COMMIT` block so a
/// crash / cancellation between them can't leave a half-initialised room
/// (e.g. a `room` row with no member, which `/join` would happily try to
/// dispatch into). The whole closure goes through
/// [`with_write_conflict_retry`] because a concurrent CREATE on the same
/// generated id is *technically* possible (SurrealDB's id generator picks
/// from a 160-bit space, so the collision odds are vanishing — but the
/// retry is free and symmetric with the other handlers).
async fn persist_create_room(
    state: &AppState,
    caller_user: &str,
    name: &str,
) -> surrealdb::Result<CreateRoomOutcome> {
    #[derive(SurrealValue)]
    struct Pair {
        room_id: String,
        room_event_id: String,
    }

    // The `RETURN { ... };` statement inside the BEGIN/COMMIT block
    // surfaces a single struct value in the result list; we pick it off
    // with `resp.take(N)`. Statement indices in this block (BEGIN and
    // COMMIT each consume an index, even though they don't produce data
    // — see the empirical probe in step 7 and the `drain_keyshare`
    // precedent at `keyshare.rs:360`):
    //   0  BEGIN
    //   1  LET $user
    //   2  LET $room      (CREATE room ... RETURN ...)[0].id_key
    //   3  CREATE room_member
    //   4  LET $event     (CREATE room_event ... RETURN ...)[0].id_key
    //   5  RETURN { ... };  ← the row we want
    //   6  COMMIT
    let sql = r#"
        BEGIN TRANSACTION;
        LET $user = type::record("user", $user_key);
        LET $room = (CREATE room SET
            name       = $name,
            created_by = $user
            RETURN meta::id(id) AS id_key)[0].id_key;
        CREATE room_member SET
            room = type::record("room", $room),
            user = $user;
        LET $event = (CREATE room_event SET
            room       = type::record("room", $room),
            event_type = "create",
            actor      = $user
            RETURN meta::id(id) AS id_key)[0].id_key;
        RETURN { room_id: $room, room_event_id: $event };
        COMMIT TRANSACTION;
    "#;

    let pair: Option<Pair> = with_write_conflict_retry(|| async {
        let mut resp = state
            .db
            .query(sql)
            .bind(("user_key", caller_user.to_string()))
            .bind(("name", name.to_string()))
            .await?
            .check()?;
        resp.take::<Option<Pair>>(5)
    })
    .await?;

    let pair = pair.ok_or_else(|| {
        surrealdb::Error::thrown("persist_create_room produced no RETURN row".to_string())
    })?;
    Ok(CreateRoomOutcome::Created {
        room_id: pair.room_id,
        event_id: pair.room_event_id,
    })
}

// ---------------------------------------------------------------------------
// POST /rooms/{id}/join
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(caller_device, caller_user, room = %room_id, target_user))]
pub async fn join_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<JoinRoomRequest>, JsonRejection>,
) -> Response {
    let device_id = match extract_device_id(&headers) {
        Some(id) => id,
        None => return error_response(StatusCode::UNAUTHORIZED, "missing X-Device-Id header"),
    };
    tracing::Span::current().record("caller_device", &tracing::field::display(&device_id));

    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => {
            tracing::warn!(rejection = %rej, "JSON extraction failed");
            return json_rejection_response(rej);
        }
    };
    tracing::Span::current().record("target_user", &tracing::field::display(&req.user));

    // Resolve caller's user. Empty → caller's device row doesn't exist.
    let caller_user = match load_caller_user(&state, &device_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "unknown caller device"),
        Err(e) => {
            tracing::error!(error = %e, "load_caller_user failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    tracing::Span::current().record("caller_user", &tracing::field::display(&caller_user));

    // Pre-checks (read-only, outside any transaction). Privacy ordering:
    // unknown room AND non-member caller both surface as the same 404
    // "room not found", before we look at the target. Self-invite (400)
    // is checked first because it's a pure caller-identity property and
    // doesn't leak anything about the room.
    if req.user == caller_user {
        return error_response(StatusCode::BAD_REQUEST, "cannot invite yourself");
    }

    let pre = match join_prechecks(&state, &room_id, &caller_user, &req.user).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "join_prechecks failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    match pre {
        JoinPrecheckOutcome::Ok => {}
        JoinPrecheckOutcome::RoomNotFound | JoinPrecheckOutcome::CallerNotMember => {
            // Same body — see the privacy 404s note in the module doc.
            return error_response(StatusCode::NOT_FOUND, "room not found");
        }
        JoinPrecheckOutcome::TargetUserNotFound => {
            return error_response(StatusCode::NOT_FOUND, "target user not found");
        }
        JoinPrecheckOutcome::TargetAlreadyMember => {
            return error_response(StatusCode::CONFLICT, "user is already a member");
        }
    }

    // Atomic write. The closure body is the unit of write-conflict retry;
    // mapping the UNIQUE-violation residual to 409 lives OUTSIDE the
    // closure because UNIQUE violations are non-retryable (re-issuing
    // the same CREATE against the same key tuple fails identically).
    let result = with_write_conflict_retry(|| async {
        do_join_write(&state, &room_id, &caller_user, &req.user).await
    })
    .await;

    match result {
        Ok(event_id) => (
            StatusCode::OK,
            Json(RoomEventResponse {
                room_event_id: event_id,
            }),
        )
            .into_response(),
        Err(e) if is_unique_violation(&e) => {
            // Concurrent-inviter race: another inviter committed first.
            // Same body as the pre-check 409 — clients shouldn't have to
            // branch on which path fired.
            tracing::info!(
                error = %e,
                "join_room raced a concurrent inviter, returning 409"
            );
            error_response(StatusCode::CONFLICT, "user is already a member")
        }
        Err(e) => {
            tracing::error!(error = %e, "do_join_write failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

enum JoinPrecheckOutcome {
    Ok,
    RoomNotFound,
    CallerNotMember,
    TargetUserNotFound,
    TargetAlreadyMember,
}

async fn join_prechecks(
    state: &AppState,
    room_id: &str,
    caller_user: &str,
    target_user: &str,
) -> surrealdb::Result<JoinPrecheckOutcome> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }

    // The five SELECTs run as one round-trip. Statement indices:
    //   0: room exists?
    //   1: caller is a member?
    //   2: target user exists?
    //   3: target is already a member?
    let sql = r#"
        SELECT meta::id(id) AS id_key FROM type::record("room", $room_id);
        SELECT meta::id(id) AS id_key
            FROM room_member
            WHERE room = type::record("room", $room_id)
              AND user = type::record("user", $caller_user_key);
        SELECT meta::id(id) AS id_key FROM type::record("user", $target_user_key);
        SELECT meta::id(id) AS id_key
            FROM room_member
            WHERE room = type::record("room", $room_id)
              AND user = type::record("user", $target_user_key);
    "#;
    let mut resp = state
        .db
        .query(sql)
        .bind(("room_id", room_id.to_string()))
        .bind(("caller_user_key", caller_user.to_string()))
        .bind(("target_user_key", target_user.to_string()))
        .await?
        .check()?;

    let room: Option<IdRow> = resp.take(0)?;
    if room.is_none() {
        return Ok(JoinPrecheckOutcome::RoomNotFound);
    }
    let caller_membership: Option<IdRow> = resp.take(1)?;
    if caller_membership.is_none() {
        return Ok(JoinPrecheckOutcome::CallerNotMember);
    }
    let target: Option<IdRow> = resp.take(2)?;
    if target.is_none() {
        return Ok(JoinPrecheckOutcome::TargetUserNotFound);
    }
    let already: Option<IdRow> = resp.take(3)?;
    if already.is_some() {
        return Ok(JoinPrecheckOutcome::TargetAlreadyMember);
    }
    Ok(JoinPrecheckOutcome::Ok)
}

/// Issue the CREATE `room_member` + CREATE `room_event` inside one
/// BEGIN/COMMIT transaction. Returns the new `room_event` id on success.
///
/// On `room_member`-pair UNIQUE collision (concurrent inviter committed
/// the same `(room, target)` pair first) the closure returns the raw
/// `surrealdb::Error`; the handler's `is_unique_violation` arm maps it
/// to 409.
async fn do_join_write(
    state: &AppState,
    room_id: &str,
    caller_user: &str,
    target_user: &str,
) -> surrealdb::Result<String> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    // Statement indices (BEGIN and COMMIT each consume an index — see
    // `persist_create_room` for the full table):
    //   0  BEGIN
    //   1  CREATE room_member
    //   2  LET $event = (CREATE room_event ...)[0].id_key
    //   3  RETURN { id_key: $event };
    //   4  COMMIT
    let sql = r#"
        BEGIN TRANSACTION;
        CREATE room_member SET
            room = type::record("room", $room_id),
            user = type::record("user", $target_user_key);
        LET $event = (CREATE room_event SET
            room       = type::record("room", $room_id),
            event_type = "join",
            actor      = type::record("user", $caller_user_key),
            target     = type::record("user", $target_user_key)
            RETURN meta::id(id) AS id_key)[0].id_key;
        RETURN { id_key: $event };
        COMMIT TRANSACTION;
    "#;
    let mut resp = state
        .db
        .query(sql)
        .bind(("room_id", room_id.to_string()))
        .bind(("caller_user_key", caller_user.to_string()))
        .bind(("target_user_key", target_user.to_string()))
        .await?
        .check()?;
    let row: Option<IdRow> = resp.take(3)?;
    row.map(|r| r.id_key)
        .ok_or_else(|| surrealdb::Error::thrown("do_join_write produced no RETURN row".to_string()))
}

// ---------------------------------------------------------------------------
// POST /rooms/{id}/leave
// ---------------------------------------------------------------------------

#[tracing::instrument(skip_all, fields(caller_device, caller_user, room = %room_id))]
pub async fn leave_room(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let device_id = match extract_device_id(&headers) {
        Some(id) => id,
        None => return error_response(StatusCode::UNAUTHORIZED, "missing X-Device-Id header"),
    };
    tracing::Span::current().record("caller_device", &tracing::field::display(&device_id));

    let caller_user = match load_caller_user(&state, &device_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return error_response(StatusCode::UNAUTHORIZED, "unknown caller device"),
        Err(e) => {
            tracing::error!(error = %e, "load_caller_user failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    tracing::Span::current().record("caller_user", &tracing::field::display(&caller_user));

    // Pre-check. Privacy: unknown room AND caller-not-member both surface
    // as the same 404 — see the module doc.
    let pre = match leave_prechecks(&state, &room_id, &caller_user).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "leave_prechecks failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    };
    match pre {
        LeavePrecheckOutcome::Ok => {}
        LeavePrecheckOutcome::RoomNotFound | LeavePrecheckOutcome::CallerNotMember => {
            return error_response(StatusCode::NOT_FOUND, "room not found");
        }
    }

    let result = with_write_conflict_retry(|| async {
        do_leave_write(&state, &room_id, &caller_user).await
    })
    .await;
    match result {
        Ok(event_id) => (
            StatusCode::OK,
            Json(RoomEventResponse {
                room_event_id: event_id,
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "do_leave_write failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error")
        }
    }
}

enum LeavePrecheckOutcome {
    Ok,
    RoomNotFound,
    CallerNotMember,
}

async fn leave_prechecks(
    state: &AppState,
    room_id: &str,
    caller_user: &str,
) -> surrealdb::Result<LeavePrecheckOutcome> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let sql = r#"
        SELECT meta::id(id) AS id_key FROM type::record("room", $room_id);
        SELECT meta::id(id) AS id_key
            FROM room_member
            WHERE room = type::record("room", $room_id)
              AND user = type::record("user", $caller_user_key);
    "#;
    let mut resp = state
        .db
        .query(sql)
        .bind(("room_id", room_id.to_string()))
        .bind(("caller_user_key", caller_user.to_string()))
        .await?
        .check()?;
    let room: Option<IdRow> = resp.take(0)?;
    if room.is_none() {
        return Ok(LeavePrecheckOutcome::RoomNotFound);
    }
    let membership: Option<IdRow> = resp.take(1)?;
    if membership.is_none() {
        return Ok(LeavePrecheckOutcome::CallerNotMember);
    }
    Ok(LeavePrecheckOutcome::Ok)
}

/// DELETE `room_member` + CREATE `room_event{leave, actor=target=caller}`
/// inside one BEGIN/COMMIT. Two simultaneous `/leave` calls from the
/// same caller-in-the-same-room can both return 200 with two `'leave'`
/// event rows — the audit log accurately reflects "the client asked
/// twice", and step 8's LIVE SELECT consumers can de-dupe at the
/// application layer if it matters. The closure deliberately does not
/// re-check membership inside the transaction.
async fn do_leave_write(
    state: &AppState,
    room_id: &str,
    caller_user: &str,
) -> surrealdb::Result<String> {
    #[derive(SurrealValue)]
    struct IdRow {
        id_key: String,
    }
    let sql = r#"
        BEGIN TRANSACTION;
        DELETE FROM room_member
            WHERE room = type::record("room", $room_id)
              AND user = type::record("user", $caller_user_key);
        LET $event = (CREATE room_event SET
            room       = type::record("room", $room_id),
            event_type = "leave",
            actor      = type::record("user", $caller_user_key),
            target     = type::record("user", $caller_user_key)
            RETURN meta::id(id) AS id_key)[0].id_key;
        RETURN { id_key: $event };
        COMMIT TRANSACTION;
    "#;
    let mut resp = state
        .db
        .query(sql)
        .bind(("room_id", room_id.to_string()))
        .bind(("caller_user_key", caller_user.to_string()))
        .await?
        .check()?;
    // Statement indices (BEGIN+COMMIT each consume an index): 0=BEGIN,
    // 1=DELETE, 2=LET $event, 3=RETURN, 4=COMMIT.
    let row: Option<IdRow> = resp.take(3)?;
    row.map(|r| r.id_key).ok_or_else(|| {
        surrealdb::Error::thrown("do_leave_write produced no RETURN row".to_string())
    })
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolve a device id (from `X-Device-Id`) to its owning user id.
/// Returns `Ok(None)` if the device row doesn't exist — handlers map
/// that to 401 `"unknown caller device"`.
async fn load_caller_user(state: &AppState, device_id: &str) -> surrealdb::Result<Option<String>> {
    #[derive(SurrealValue)]
    struct Row {
        user_key: String,
    }
    let mut resp = state
        .db
        .query("SELECT meta::id(user) AS user_key FROM type::record('device', $device_id);")
        .bind(("device_id", device_id.to_string()))
        .await?
        .check()?;
    let row: Option<Row> = resp.take(0)?;
    Ok(row.map(|r| r.user_key))
}

// ---------------------------------------------------------------------------
// HTTP response shaping (identical pattern to keys.rs / keyshare.rs)
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
