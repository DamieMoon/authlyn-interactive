//! Shared HTTP error-response helpers for the server handlers.
//!
//! Every JSON handler builds its 4xx/5xx replies through these two functions,
//! which were previously copied byte-for-byte into each handler module. The
//! wire shape is the canonical [`ErrorBody`] (`{"error": "<reason>"}`), so the
//! response body is identical across every route.

use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::protocol::ErrorBody;

/// Build a JSON error response: `(status, {"error": msg})`.
pub(crate) fn error_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(ErrorBody::new(msg))).into_response()
}

/// Map an axum [`JsonRejection`] (malformed/missing JSON body) to a stable
/// `400 Bad Request` with a human-readable reason. The reason strings are part
/// of the API surface — keep them unchanged.
pub(crate) fn json_rejection_response(rej: JsonRejection) -> Response {
    let reason: &'static str = match rej {
        JsonRejection::JsonDataError(_) => "invalid JSON body shape",
        JsonRejection::JsonSyntaxError(_) => "malformed JSON",
        JsonRejection::MissingJsonContentType(_) => "missing Content-Type: application/json",
        JsonRejection::BytesRejection(_) => "could not read request body",
        _ => "invalid JSON request",
    };
    error_response(StatusCode::BAD_REQUEST, reason)
}
