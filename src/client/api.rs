//! gloo-net Fetch wrappers (hydrate-only). Same-origin requests send the
//! session cookie automatically. Endpoints are added per frontend build slice;
//! this slice covers auth.

use gloo_net::http::{Request, Response};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::protocol::{AuthResponse, ErrorBody, LoginRequest, MeResponse, RegisterRequest};

/// A failed API call.
#[derive(Clone, Debug)]
pub enum ApiError {
    /// The request never got a response (offline, DNS, CORS, …).
    Network(String),
    /// A non-2xx status, with the server's `error` message if present.
    Status(u16, String),
    /// The response body wasn't the shape we expected.
    Codec(String),
}

impl ApiError {
    /// The HTTP status, if the failure was a status error.
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Status(code, _) => Some(*code),
            _ => None,
        }
    }
}

/// A short, user-facing message for an error (e.g. to show under a form).
pub fn humanize(e: &ApiError) -> String {
    match e {
        ApiError::Status(_, msg) => msg.clone(),
        ApiError::Network(_) => "network error — please try again".to_string(),
        ApiError::Codec(_) => "unexpected response from the server".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

pub async fn current_user() -> Result<MeResponse, ApiError> {
    let resp = Request::get("/auth/me").send().await.map_err(net)?;
    decode(resp).await
}

pub async fn register(body: &RegisterRequest) -> Result<AuthResponse, ApiError> {
    post_json("/auth/register", body).await
}

pub async fn login(body: &LoginRequest) -> Result<AuthResponse, ApiError> {
    post_json("/auth/login", body).await
}

pub async fn logout() -> Result<(), ApiError> {
    let resp = Request::post("/auth/logout").send().await.map_err(net)?;
    decode_empty(resp).await
}

// ---------------------------------------------------------------------------
// Low-level helpers (reused by later slices)
// ---------------------------------------------------------------------------

pub(crate) async fn post_json<B: Serialize, T: DeserializeOwned>(
    url: &str,
    body: &B,
) -> Result<T, ApiError> {
    let resp = Request::post(url)
        .json(body)
        .map_err(codec)?
        .send()
        .await
        .map_err(net)?;
    decode(resp).await
}

fn net(e: gloo_net::Error) -> ApiError {
    ApiError::Network(e.to_string())
}

fn codec(e: gloo_net::Error) -> ApiError {
    ApiError::Codec(e.to_string())
}

async fn decode<T: DeserializeOwned>(resp: Response) -> Result<T, ApiError> {
    let status = resp.status();
    if (200..300).contains(&status) {
        resp.json::<T>().await.map_err(codec)
    } else {
        Err(ApiError::Status(status, error_message(resp).await))
    }
}

async fn decode_empty(resp: Response) -> Result<(), ApiError> {
    let status = resp.status();
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(ApiError::Status(status, error_message(resp).await))
    }
}

/// Pull the server's `{"error": "..."}` message, falling back to a generic.
async fn error_message(resp: Response) -> String {
    resp.json::<ErrorBody>()
        .await
        .map(|b| b.error)
        .unwrap_or_else(|_| "request failed".to_string())
}
