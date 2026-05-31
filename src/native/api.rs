//! Native HTTP client — the `reqwest` port of `src/client/api.rs`'s transport.
//!
//! Mirrors the browser client's shape (`ApiError` + `get`/`post_json`/`decode`/
//! `error_message`) but swaps gloo-net for `reqwest` and carries the session
//! explicitly: a native client is not a same-origin browser, and the server's
//! `authlyn_session` cookie is set `Secure` (`server/auth/session.rs`), which a
//! reqwest cookie jar will NOT replay over dev `http://`. So we capture the
//! cookie's value from the login `Set-Cookie` and attach it manually on every
//! request — preserving the "identity only from the session token, DB stores
//! only its SHA-256" invariant with no backend change. DTOs are reused from
//! [`crate::protocol`], never redefined.

use bytes::Bytes;
use serde::{de::DeserializeOwned, Serialize};
use std::sync::{Mutex, OnceLock};

use crate::protocol::{
    AddGalleryImageRequest, AddGalleryImageResponse, AddGalleryImagesBatchRequest,
    AddGalleryImagesBatchResponse, AuthResponse, ChannelListResponse, ChannelSummary,
    CreateChannelRequest, CreateEmojiRequest, CreateGuildRequest, CreateLorebookEntryRequest,
    CreateLorebookEntryResponse, CreatePersonaRequest, EditMessageRequest, ErrorBody,
    FriendRequest, GuildDetail, GuildSummary, ListEmojiResponse, ListFriendsResponse,
    ListGuildsResponse, ListLorebookResponse, ListMembersResponse, ListMessagesResponse,
    ListPersonaEditorsResponse, ListPersonasResponse, LoginRequest, MeResponse,
    PatchChannelRequest, PatchGuildRequest, PatchLorebookEntryRequest, PatchPersonaRequest,
    PersonaDetail, RailOrderRequest, RegisterRequest, SendMessageRequest, SendMessageResponse,
    SetActivePersonaRequest, SetAvatarRequest, SetMemberRoleRequest,
};

/// The process-global client. One backend + one session for the app's life, so
/// the shared signal-state can stay `Copy` and any spawn can reach the client.
static API: OnceLock<ApiClient> = OnceLock::new();

/// Initialize the global client. Call once in `main()` before `launch`.
pub fn init_client() {
    let _ = API.set(ApiClient::new());
}

/// The global client. Panics if [`init_client`] wasn't called.
pub fn client() -> &'static ApiClient {
    API.get()
        .expect("ApiClient not initialized — call native::api::init_client() in main")
}

/// Name of the session cookie the server sets on login/register.
const SESSION_COOKIE: &str = "authlyn_session";

/// Transport/decoding failure — same three variants as the browser client's
/// `ApiError` so the action layer ports 1:1.
#[derive(Clone, Debug)]
pub enum ApiError {
    /// No HTTP response (connection refused, DNS, timeout).
    Network(String),
    /// A 4xx/5xx response, carrying the server's `{"error": …}` message.
    Status(u16, String),
    /// A 2xx response whose body failed to deserialize.
    Codec(String),
}

impl ApiError {
    /// The HTTP status, when this is a server error response.
    pub fn status(&self) -> Option<u16> {
        match self {
            ApiError::Status(code, _) => Some(*code),
            _ => None,
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Network(m) => write!(f, "network error: {m}"),
            ApiError::Status(code, m) => write!(f, "HTTP {code}: {m}"),
            ApiError::Codec(m) => write!(f, "decode error: {m}"),
        }
    }
}

/// A configured client for one backend. Holds the captured session token so
/// authed requests carry it; `base_url` is configurable for dev vs prod.
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
    session: Mutex<Option<String>>,
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiClient {
    /// Build a client. Base URL comes from `AUTHLYN_NATIVE_URL`, defaulting to
    /// the dev server on loopback. (No TLS backend is compiled in yet — Phase 1
    /// targets loopback http; prod https adds `rustls-tls` later.)
    pub fn new() -> Self {
        let base_url = std::env::var("AUTHLYN_NATIVE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
        Self {
            http: reqwest::Client::new(),
            base_url,
            session: Mutex::new(None),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// `Cookie: authlyn_session=<token>` when a session has been captured.
    fn cookie_header(&self) -> Option<String> {
        self.session
            .lock()
            .unwrap()
            .clone()
            .map(|tok| format!("{SESSION_COOKIE}={tok}"))
    }

    /// Attach the manual session cookie to a request builder, if we have one.
    fn authed(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.cookie_header() {
            Some(cookie) => req.header(reqwest::header::COOKIE, cookie),
            None => req,
        }
    }

    /// Pull the `authlyn_session` value out of a login/register `Set-Cookie`.
    fn capture_session(&self, resp: &reqwest::Response) {
        for hv in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            let Ok(s) = hv.to_str() else { continue };
            if let Some(rest) = s.strip_prefix(&format!("{SESSION_COOKIE}=")) {
                let val = rest.split(';').next().unwrap_or("").to_string();
                if !val.is_empty() {
                    *self.session.lock().unwrap() = Some(val);
                }
                return;
            }
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let mut req = self.http.get(self.url(path));
        if let Some(cookie) = self.cookie_header() {
            req = req.header(reqwest::header::COOKIE, cookie);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        decode(resp).await
    }

    async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        let mut req = self.http.post(self.url(path)).json(body);
        if let Some(cookie) = self.cookie_header() {
            req = req.header(reqwest::header::COOKIE, cookie);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        // Login/register carry the session in Set-Cookie; harmless no-op otherwise.
        self.capture_session(&resp);
        decode(resp).await
    }

    /// POST /auth/login — authenticate and capture the session cookie.
    pub async fn login(&self, username: &str, password: &str) -> Result<AuthResponse, ApiError> {
        self.post_json(
            "/auth/login",
            &LoginRequest {
                username: username.to_string(),
                password: password.to_string(),
            },
        )
        .await
    }

    /// POST /auth/register — create the account and capture the session cookie.
    pub async fn register(&self, username: &str, password: &str) -> Result<AuthResponse, ApiError> {
        self.post_json(
            "/auth/register",
            &RegisterRequest {
                username: username.to_string(),
                password: password.to_string(),
            },
        )
        .await
    }

    /// Log in; bootstrap-register on the first run if the account doesn't exist
    /// yet (401). Mirrors the nova bridge's `ensure_session`.
    pub async fn ensure_session(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AuthResponse, ApiError> {
        match self.login(username, password).await {
            Ok(auth) => Ok(auth),
            Err(e) if e.status() == Some(401) => self.register(username, password).await,
            Err(e) => Err(e),
        }
    }

    /// POST /auth/logout — end the session server-side, then forget the locally
    /// captured cookie so subsequent requests are unauthenticated.
    pub async fn logout(&self) -> Result<(), ApiError> {
        let req = self.http.post(self.url("/auth/logout"));
        let r = self.empty(self.authed(req)).await;
        *self.session.lock().unwrap() = None;
        r
    }

    /// GET /auth/me — the authenticated caller's profile.
    pub async fn current_user(&self) -> Result<MeResponse, ApiError> {
        self.get("/auth/me").await
    }

    /// GET /guilds — the caller's guild rail.
    pub async fn list_guilds(&self) -> Result<ListGuildsResponse, ApiError> {
        self.get("/guilds").await
    }

    /// POST /guilds — create a guild, returning its summary.
    pub async fn create_guild(&self, name: &str) -> Result<GuildSummary, ApiError> {
        self.post_json(
            "/guilds",
            &CreateGuildRequest {
                name: name.to_string(),
            },
        )
        .await
    }

    /// GET /guilds/{gid} — a guild's detail (channels included).
    pub async fn get_guild(&self, gid: &str) -> Result<GuildDetail, ApiError> {
        self.get(&format!("/guilds/{gid}")).await
    }

    // -----------------------------------------------------------------------
    // Guild lifecycle (Phase 4c PR2) — rename/delete/restore/reorder. Mutations
    // are owner/manager gated server-side (privacy-404); the reqwest port mirrors
    // the web `client/api.rs` guild section.
    // -----------------------------------------------------------------------

    /// PATCH /guilds/{gid} — rename a guild (owner/admin).
    pub async fn patch_guild(&self, gid: &str, name: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .patch(self.url(&format!("/guilds/{gid}")))
            .json(&PatchGuildRequest {
                name: Some(name.to_string()),
            });
        self.empty(self.authed(req)).await
    }

    /// DELETE /guilds/{gid} — soft-delete a guild (owner only).
    pub async fn delete_guild(&self, gid: &str) -> Result<(), ApiError> {
        let req = self.http.delete(self.url(&format!("/guilds/{gid}")));
        self.empty(self.authed(req)).await
    }

    /// POST /guilds/{gid}/restore — restore a soft-deleted guild (owner only).
    pub async fn restore_guild(&self, gid: &str) -> Result<(), ApiError> {
        let req = self.http.post(self.url(&format!("/guilds/{gid}/restore")));
        self.empty(self.authed(req)).await
    }

    /// GET /guilds/trash — the caller's soft-deleted guilds (owner-scoped).
    pub async fn list_deleted_guilds(&self) -> Result<ListGuildsResponse, ApiError> {
        self.get("/guilds/trash").await
    }

    /// PUT /rail/order — set the caller's full guild-rail order. Full-list
    /// replacement: the server wipes and rewrites the caller's order rows.
    pub async fn set_rail_order(&self, guild_ids: Vec<String>) -> Result<(), ApiError> {
        let req = self
            .http
            .put(self.url("/rail/order"))
            .json(&RailOrderRequest { guild_ids });
        self.empty(self.authed(req)).await
    }

    // -----------------------------------------------------------------------
    // Channel lifecycle (Phase 4c PR2) — create/rename/reorder/delete/restore.
    // All owner/manager gated server-side.
    // -----------------------------------------------------------------------

    /// POST /guilds/{gid}/channels — create a channel (`kind` = "text" or
    /// "lorebook"); returns its summary (owner/admin).
    pub async fn create_channel(
        &self,
        gid: &str,
        name: &str,
        kind: &str,
    ) -> Result<ChannelSummary, ApiError> {
        self.post_json(
            &format!("/guilds/{gid}/channels"),
            &CreateChannelRequest {
                name: name.to_string(),
                kind: kind.to_string(),
            },
        )
        .await
    }

    /// PATCH /guilds/{gid}/channels/{cid} — rename and/or reposition a channel
    /// (owner/admin). `None` fields are left unchanged; `position` drives the
    /// renumber-and-PATCH reorder (`swap_channel` in `act.rs`).
    pub async fn patch_channel(
        &self,
        gid: &str,
        cid: &str,
        name: Option<String>,
        position: Option<i64>,
    ) -> Result<(), ApiError> {
        let req = self
            .http
            .patch(self.url(&format!("/guilds/{gid}/channels/{cid}")))
            .json(&PatchChannelRequest { name, position });
        self.empty(self.authed(req)).await
    }

    /// DELETE /guilds/{gid}/channels/{cid} — soft-delete a channel (owner/admin).
    pub async fn delete_channel(&self, gid: &str, cid: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .delete(self.url(&format!("/guilds/{gid}/channels/{cid}")));
        self.empty(self.authed(req)).await
    }

    /// POST /guilds/{gid}/channels/{cid}/restore — restore a soft-deleted
    /// channel (owner/admin).
    pub async fn restore_channel(&self, gid: &str, cid: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .post(self.url(&format!("/guilds/{gid}/channels/{cid}/restore")));
        self.empty(self.authed(req)).await
    }

    /// GET /guilds/{gid}/trash/channels — the guild's soft-deleted channels
    /// (owner/admin).
    pub async fn list_deleted_channels(&self, gid: &str) -> Result<ChannelListResponse, ApiError> {
        self.get(&format!("/guilds/{gid}/trash/channels")).await
    }

    /// GET /personas — the caller's personas (owned + shared-as-editor), for the
    /// wardrobe and the composer's "speaking as" picker.
    pub async fn list_personas(&self) -> Result<ListPersonasResponse, ApiError> {
        self.get("/personas").await
    }

    /// GET /guilds/{gid}/emoji — the guild's custom emoji, for `:`-autocomplete.
    pub async fn list_guild_emoji(&self, gid: &str) -> Result<ListEmojiResponse, ApiError> {
        self.get(&format!("/guilds/{gid}/emoji")).await
    }

    /// PUT /channels/{cid}/active-persona — wear (`Some`) or take off (`None`) a
    /// persona in this channel. The send-path also carries the worn id so
    /// attribution is decided at send time; this stores the per-channel state so
    /// it survives a reopen (web parity — `set_channel_active_persona`).
    pub async fn set_channel_active_persona(
        &self,
        cid: &str,
        persona_id: Option<String>,
    ) -> Result<(), ApiError> {
        let req = self
            .http
            .put(self.url(&format!("/channels/{cid}/active-persona")))
            .json(&SetActivePersonaRequest { persona_id });
        self.empty(self.authed(req)).await
    }

    /// GET /channels/{cid}/messages — newest page, or the page after `cursor`
    /// (the `(sent_at, id)` of the last seen message) when polling for new ones.
    pub async fn list_messages(
        &self,
        cid: &str,
        cursor: Option<&(String, String)>,
    ) -> Result<ListMessagesResponse, ApiError> {
        let mut req = self
            .http
            .get(self.url(&format!("/channels/{cid}/messages")));
        if let Some((since, after_id)) = cursor {
            req = req.query(&[("since", since.as_str()), ("after_id", after_id.as_str())]);
        }
        let resp = self
            .authed(req)
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        decode(resp).await
    }

    /// GET /channels/{cid}/messages?before=… — the page of history strictly
    /// older than `before` (scroll-up backfill).
    pub async fn list_messages_before(
        &self,
        cid: &str,
        before: &(String, String),
    ) -> Result<ListMessagesResponse, ApiError> {
        let req = self
            .http
            .get(self.url(&format!("/channels/{cid}/messages")))
            .query(&[
                ("before", before.0.as_str()),
                ("before_id", before.1.as_str()),
            ]);
        let resp = self
            .authed(req)
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        decode(resp).await
    }

    /// POST /channels/{cid}/messages — send a message; returns the new id.
    pub async fn post_message(
        &self,
        cid: &str,
        body: &str,
        attachment_ids: Vec<String>,
        persona_id: Option<String>,
    ) -> Result<SendMessageResponse, ApiError> {
        self.post_json(
            &format!("/channels/{cid}/messages"),
            &SendMessageRequest {
                body: body.to_string(),
                attachment_ids,
                persona_id,
            },
        )
        .await
    }

    /// PATCH /channels/{cid}/messages/{mid} — edit one of your own messages.
    pub async fn edit_message(&self, cid: &str, mid: &str, body: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .patch(self.url(&format!("/channels/{cid}/messages/{mid}")))
            .json(&EditMessageRequest {
                body: body.to_string(),
            });
        self.empty(self.authed(req)).await
    }

    /// DELETE /channels/{cid}/messages/{mid} — soft-delete your own message.
    pub async fn delete_message(&self, cid: &str, mid: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .delete(self.url(&format!("/channels/{cid}/messages/{mid}")));
        self.empty(self.authed(req)).await
    }

    // -----------------------------------------------------------------------
    // Personas + wardrobe (Phase 4b) — the reqwest port of the persona section
    // in `src/client/api.rs`. Same endpoints/verbs/DTOs; the session cookie
    // rides via `self.authed`.
    // -----------------------------------------------------------------------

    /// POST /personas — create a persona owned by the caller.
    pub async fn create_persona(
        &self,
        name: &str,
        description: &str,
    ) -> Result<crate::protocol::PersonaSummary, ApiError> {
        self.post_json(
            "/personas",
            &CreatePersonaRequest {
                name: name.to_string(),
                description: Some(description.to_string()),
                color: None,
            },
        )
        .await
    }

    /// PATCH /personas/{pid} — update a persona's name/description/color/position
    /// (owner/editor). `None` fields are left unchanged; `position` durably
    /// reorders the caller's wardrobe (`reorder_persona` in `wardrobe.rs`).
    pub async fn patch_persona(
        &self,
        pid: &str,
        name: Option<String>,
        description: Option<String>,
        color: Option<String>,
        position: Option<i64>,
    ) -> Result<(), ApiError> {
        let req =
            self.http
                .patch(self.url(&format!("/personas/{pid}")))
                .json(&PatchPersonaRequest {
                    name,
                    description,
                    color,
                    position,
                });
        self.empty(self.authed(req)).await
    }

    /// DELETE /personas/{pid} — delete a persona (owner only).
    pub async fn delete_persona(&self, pid: &str) -> Result<(), ApiError> {
        let req = self.http.delete(self.url(&format!("/personas/{pid}")));
        self.empty(self.authed(req)).await
    }

    /// GET /personas/{pid} — a persona's detail: name/description/avatar plus
    /// its gallery and (for the owner) its share key + editor roster.
    pub async fn get_persona(&self, pid: &str) -> Result<PersonaDetail, ApiError> {
        self.get(&format!("/personas/{pid}")).await
    }

    /// PUT /personas/{pid}/avatar — set the persona's primary avatar to an
    /// already-uploaded media id.
    pub async fn set_persona_avatar(&self, pid: &str, media_id: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .put(self.url(&format!("/personas/{pid}/avatar")))
            .json(&SetAvatarRequest {
                media_id: media_id.to_string(),
            });
        self.empty(self.authed(req)).await
    }

    /// POST /personas/{pid}/gallery — attach one already-uploaded media id to a
    /// persona's gallery. Returns the new gallery-row id.
    pub async fn add_gallery_image(
        &self,
        pid: &str,
        media_id: &str,
    ) -> Result<AddGalleryImageResponse, ApiError> {
        self.post_json(
            &format!("/personas/{pid}/gallery"),
            &AddGalleryImageRequest {
                media_id: media_id.to_string(),
            },
        )
        .await
    }

    /// POST /personas/{pid}/gallery/batch — atomically append multiple media
    /// ids (paste-many). `ids` come back in the same order as `media_ids`.
    pub async fn add_gallery_images_batch(
        &self,
        pid: &str,
        media_ids: &[String],
    ) -> Result<AddGalleryImagesBatchResponse, ApiError> {
        self.post_json(
            &format!("/personas/{pid}/gallery/batch"),
            &AddGalleryImagesBatchRequest {
                media_ids: media_ids.to_vec(),
            },
        )
        .await
    }

    /// DELETE /personas/{pid}/gallery/{img} — remove a gallery image (owner/
    /// editor).
    pub async fn remove_gallery_image(&self, pid: &str, img: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .delete(self.url(&format!("/personas/{pid}/gallery/{img}")));
        self.empty(self.authed(req)).await
    }

    /// DELETE /personas/{pid}/leave — leave a shared persona (editor only),
    /// dropping it from the caller's wardrobe.
    pub async fn leave_persona(&self, pid: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .delete(self.url(&format!("/personas/{pid}/leave")));
        self.empty(self.authed(req)).await
    }

    /// GET /personas/{pid}/editors — the persona's editor roster (owner only).
    pub async fn list_persona_editors(
        &self,
        pid: &str,
    ) -> Result<ListPersonaEditorsResponse, ApiError> {
        self.get(&format!("/personas/{pid}/editors")).await
    }

    /// PUT /personas/{pid}/editors/{aid} — grant a friend editor access (owner
    /// only). Empty body.
    pub async fn set_persona_editor(&self, pid: &str, aid: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .put(self.url(&format!("/personas/{pid}/editors/{aid}")));
        self.empty(self.authed(req)).await
    }

    /// DELETE /personas/{pid}/editors/{aid} — revoke a friend's editor access
    /// (owner only).
    pub async fn remove_persona_editor(&self, pid: &str, aid: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .delete(self.url(&format!("/personas/{pid}/editors/{aid}")));
        self.empty(self.authed(req)).await
    }

    /// GET /friends — the caller's friends + pending requests (the sharing
    /// checklist source and the friends pane).
    pub async fn list_friends(&self) -> Result<ListFriendsResponse, ApiError> {
        self.get("/friends").await
    }

    /// POST /friends — send a friend request by username (the server auto-accepts
    /// when the target already requested the caller). Empty body on success.
    pub async fn add_friend(&self, username: &str) -> Result<(), ApiError> {
        let req = self.http.post(self.url("/friends")).json(&FriendRequest {
            username: username.to_string(),
        });
        self.empty(self.authed(req)).await
    }

    /// POST /friends/{aid}/accept — accept an incoming request from account `aid`.
    pub async fn accept_friend(&self, aid: &str) -> Result<(), ApiError> {
        let req = self.http.post(self.url(&format!("/friends/{aid}/accept")));
        self.empty(self.authed(req)).await
    }

    /// DELETE /friends/{aid} — remove a friend, or cancel/decline a request
    /// (idempotent either direction).
    pub async fn remove_friend(&self, aid: &str) -> Result<(), ApiError> {
        let req = self.http.delete(self.url(&format!("/friends/{aid}")));
        self.empty(self.authed(req)).await
    }

    // -----------------------------------------------------------------------
    // Custom emoji (Phase 4b) — create/delete for the emoji-manager pane.
    // -----------------------------------------------------------------------

    /// POST /guilds/{gid}/emoji — register a named custom emoji against an
    /// already-uploaded media id. `name` must match `^[a-z0-9_]{2,32}$` (the
    /// server re-validates).
    pub async fn create_emoji(
        &self,
        gid: &str,
        name: &str,
        media_id: &str,
    ) -> Result<crate::protocol::CustomEmoji, ApiError> {
        self.post_json(
            &format!("/guilds/{gid}/emoji"),
            &CreateEmojiRequest {
                name: name.to_string(),
                media_id: media_id.to_string(),
            },
        )
        .await
    }

    /// DELETE /guilds/{gid}/emoji/{name} — delete a custom emoji by shortcode
    /// name (manager/admin only — the server enforces).
    pub async fn delete_emoji(&self, gid: &str, name: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .delete(self.url(&format!("/guilds/{gid}/emoji/{name}")));
        self.empty(self.authed(req)).await
    }

    // -----------------------------------------------------------------------
    // Guild members (Phase 4c) — the roster pane. Mutations are owner/admin
    // gated server-side (privacy-404), so a UI gate slip is cosmetic only.
    // -----------------------------------------------------------------------

    /// GET /guilds/{gid}/members — the guild's member roster.
    pub async fn list_members(&self, gid: &str) -> Result<ListMembersResponse, ApiError> {
        self.get(&format!("/guilds/{gid}/members")).await
    }

    /// PUT /guilds/{gid}/members/{aid}/role — set member `aid`'s role
    /// (`"admin"` or `"member"`; the owner's role is fixed).
    pub async fn set_member_role(&self, gid: &str, aid: &str, role: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .put(self.url(&format!("/guilds/{gid}/members/{aid}/role")))
            .json(&SetMemberRoleRequest {
                role: role.to_string(),
            });
        self.empty(self.authed(req)).await
    }

    /// DELETE /guilds/{gid}/members/{aid} — kick member `aid` from the guild.
    pub async fn remove_member(&self, gid: &str, aid: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .delete(self.url(&format!("/guilds/{gid}/members/{aid}")));
        self.empty(self.authed(req)).await
    }

    // -----------------------------------------------------------------------
    // Lorebook (Phase 4c) — entries on a `kind='lorebook'` channel. Any guild
    // member may read/write; entries order by `position` (no datetime cursor).
    // -----------------------------------------------------------------------

    /// GET /channels/{cid}/lorebook — the channel's lore entries (by position).
    pub async fn list_lore(&self, cid: &str) -> Result<ListLorebookResponse, ApiError> {
        self.get(&format!("/channels/{cid}/lorebook")).await
    }

    /// POST /channels/{cid}/lorebook — create an entry from `keys` + `content`
    /// (server assigns the next position). Returns the new id.
    pub async fn create_lore(
        &self,
        cid: &str,
        keys: Vec<String>,
        content: &str,
    ) -> Result<CreateLorebookEntryResponse, ApiError> {
        self.post_json(
            &format!("/channels/{cid}/lorebook"),
            &CreateLorebookEntryRequest {
                title: None,
                keys,
                content: content.to_string(),
                enabled: None,
                position: None,
            },
        )
        .await
    }

    /// PATCH /channels/{cid}/lorebook/{eid} — partial update; the caller chooses
    /// which fields change (enable toggle, inline edit, reorder).
    pub async fn patch_lore(
        &self,
        cid: &str,
        eid: &str,
        body: &PatchLorebookEntryRequest,
    ) -> Result<(), ApiError> {
        let req = self
            .http
            .patch(self.url(&format!("/channels/{cid}/lorebook/{eid}")))
            .json(body);
        self.empty(self.authed(req)).await
    }

    /// DELETE /channels/{cid}/lorebook/{eid} — delete an entry (hard delete).
    pub async fn delete_lore(&self, cid: &str, eid: &str) -> Result<(), ApiError> {
        let req = self
            .http
            .delete(self.url(&format!("/channels/{cid}/lorebook/{eid}")));
        self.empty(self.authed(req)).await
    }

    /// Send a request that returns no body; map non-2xx to `ApiError::Status`.
    async fn empty(&self, req: reqwest::RequestBuilder) -> Result<(), ApiError> {
        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(ApiError::Status(status, error_message(resp).await))
        }
    }

    /// GET /media/{id}?w=N — raw bytes of a media blob (auth-gated; carries the
    /// session cookie). Used for avatars + message attachments.
    pub async fn get_media_bytes(&self, id: &str, w: u32) -> Result<Bytes, ApiError> {
        let req = self
            .http
            .get(self.url(&format!("/media/{id}")))
            .query(&[("w", w)]);
        self.bytes(self.authed(req)).await
    }

    /// POST /media — upload image bytes as multipart/form-data (field `file`,
    /// matching `server/media.rs` FILE_FIELD), carrying the session cookie.
    /// Returns the new media id. The server reads the part's `Content-Type` for
    /// the stored MIME and enforces its image allowlist + the 64 MiB route
    /// limit, so a bad type/size surfaces as `ApiError::Status`.
    pub async fn upload_media(
        &self,
        bytes: Vec<u8>,
        filename: String,
        mime: String,
    ) -> Result<String, ApiError> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename)
            .mime_str(&mime)
            .map_err(|e| ApiError::Codec(format!("bad mime: {e}")))?;
        let form = reqwest::multipart::Form::new().part("file", part);
        let req = self.http.post(self.url("/media")).multipart(form);
        let resp = self
            .authed(req)
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        let body: crate::protocol::MediaUploadResponse = decode(resp).await?;
        Ok(body.id)
    }

    /// GET an arbitrary URL's bytes (no cookie) — markup `Image(url)` nodes.
    pub async fn fetch_bytes(&self, url: &str) -> Result<Bytes, ApiError> {
        self.bytes(self.http.get(url)).await
    }

    async fn bytes(&self, req: reqwest::RequestBuilder) -> Result<Bytes, ApiError> {
        let resp = req
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(ApiError::Status(status, error_message(resp).await));
        }
        resp.bytes()
            .await
            .map_err(|e| ApiError::Codec(e.to_string()))
    }
}

/// Decode a 2xx body as `T`, else surface the server's error message.
async fn decode<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T, ApiError> {
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        resp.json::<T>()
            .await
            .map_err(|e| ApiError::Codec(e.to_string()))
    } else {
        Err(ApiError::Status(status, error_message(resp).await))
    }
}

/// Extract `{"error": "…"}` from a failed response, with a generic fallback.
async fn error_message(resp: reqwest::Response) -> String {
    resp.json::<ErrorBody>()
        .await
        .map(|b| b.error)
        .unwrap_or_else(|_| "request failed".to_string())
}
