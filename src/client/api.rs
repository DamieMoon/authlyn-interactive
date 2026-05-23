//! Typed Fetch wrappers over the REST API, for the browser (hydrate) build.
//!
//! Every call attaches the `X-Device-Id` header (the v1 auth stub) and
//! (de)serializes the shared `crate::protocol` DTOs. Non-2xx responses are
//! decoded into the server's `ErrorBody` and surfaced as [`ApiError::Status`].

use gloo_net::http::{Request, Response};
use serde::de::DeserializeOwned;

use crate::protocol::{
    ClaimKeyResponse, CreateRoomRequest, CreateRoomResponse, ErrorBody, JoinRoomRequest,
    KeyshareDeposit, KeyshareDepositResponse, KeyshareInbox, ListMessagesResponse,
    RoomEventResponse, SendMessageRequest, SendMessageResponse, UploadKeysRequest,
    UploadKeysResponse,
};

const DEVICE_HEADER: &str = "X-Device-Id";

/// Failure modes of a REST call.
#[derive(Debug)]
pub enum ApiError {
    /// The Fetch itself failed (offline, CORS, DNS, …).
    Network(String),
    /// A non-2xx response; carries the status and the server's error message.
    Status(u16, String),
    /// Request body or response body (de)serialization failed.
    Codec(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Network(e) => write!(f, "network error: {e}"),
            ApiError::Status(code, msg) => write!(f, "HTTP {code}: {msg}"),
            ApiError::Codec(e) => write!(f, "encoding error: {e}"),
        }
    }
}

/// Parse a `Response`: decode JSON on 2xx, else decode `ErrorBody`.
async fn parse<R: DeserializeOwned>(resp: Response) -> Result<R, ApiError> {
    let status = resp.status();
    if (200..300).contains(&status) {
        resp.json::<R>()
            .await
            .map_err(|e| ApiError::Codec(e.to_string()))
    } else {
        let msg = match resp.json::<ErrorBody>().await {
            Ok(body) => body.error,
            Err(_) => "unknown error".to_string(),
        };
        Err(ApiError::Status(status, msg))
    }
}

/// `POST /keys/upload`.
pub async fn upload_keys(
    device_id: &str,
    body: &UploadKeysRequest,
) -> Result<UploadKeysResponse, ApiError> {
    let resp = Request::post("/keys/upload")
        .header(DEVICE_HEADER, device_id)
        .json(body)
        .map_err(|e| ApiError::Codec(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    parse(resp).await
}

/// `POST /keys/claim/{user}/{device}` (empty body).
pub async fn claim_key(
    device_id: &str,
    user: &str,
    device: &str,
) -> Result<ClaimKeyResponse, ApiError> {
    let resp = Request::post(&format!("/keys/claim/{user}/{device}"))
        .header(DEVICE_HEADER, device_id)
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    parse(resp).await
}

/// `POST /rooms`.
pub async fn create_room(
    device_id: &str,
    body: &CreateRoomRequest,
) -> Result<CreateRoomResponse, ApiError> {
    let resp = Request::post("/rooms")
        .header(DEVICE_HEADER, device_id)
        .json(body)
        .map_err(|e| ApiError::Codec(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    parse(resp).await
}

/// `POST /rooms/{id}/join` — invite `body.user` to the room.
pub async fn join_room(
    device_id: &str,
    room: &str,
    body: &JoinRoomRequest,
) -> Result<RoomEventResponse, ApiError> {
    let resp = Request::post(&format!("/rooms/{room}/join"))
        .header(DEVICE_HEADER, device_id)
        .json(body)
        .map_err(|e| ApiError::Codec(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    parse(resp).await
}

/// `POST /rooms/{id}/keyshare`.
pub async fn deposit_keyshare(
    device_id: &str,
    room: &str,
    body: &KeyshareDeposit,
) -> Result<KeyshareDepositResponse, ApiError> {
    let resp = Request::post(&format!("/rooms/{room}/keyshare"))
        .header(DEVICE_HEADER, device_id)
        .json(body)
        .map_err(|e| ApiError::Codec(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    parse(resp).await
}

/// `GET /rooms/{id}/keyshare/inbox` (delete-on-read).
pub async fn drain_inbox(device_id: &str, room: &str) -> Result<KeyshareInbox, ApiError> {
    let resp = Request::get(&format!("/rooms/{room}/keyshare/inbox"))
        .header(DEVICE_HEADER, device_id)
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    parse(resp).await
}

/// `POST /rooms/{id}/messages`.
pub async fn post_message(
    device_id: &str,
    room: &str,
    body: &SendMessageRequest,
) -> Result<SendMessageResponse, ApiError> {
    let resp = Request::post(&format!("/rooms/{room}/messages"))
        .header(DEVICE_HEADER, device_id)
        .json(body)
        .map_err(|e| ApiError::Codec(e.to_string()))?
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    parse(resp).await
}

/// `GET /rooms/{id}/messages?since=&after_id=`. `cursor` is the composite
/// `(sent_at, id)` of the last seen row; `None` fetches from the start.
pub async fn list_messages(
    device_id: &str,
    room: &str,
    cursor: Option<(String, String)>,
) -> Result<ListMessagesResponse, ApiError> {
    let url = match &cursor {
        Some((since, after_id)) => {
            format!("/rooms/{room}/messages?since={since}&after_id={after_id}")
        }
        None => format!("/rooms/{room}/messages"),
    };
    let resp = Request::get(&url)
        .header(DEVICE_HEADER, device_id)
        .send()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
    parse(resp).await
}
