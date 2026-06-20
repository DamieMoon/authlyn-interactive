//! Browser REST client (hydrate-only). All functions wrap `gloo-net` Fetch and
//! share the same envelope: 2xx → typed body or `()`, otherwise [`ApiError`].
//!
//! Requests are same-origin, so the session cookie rides along automatically —
//! callers never touch headers. The thin transport layer at the bottom of the
//! file (`get`, `post_empty`, `post_json`, `post_json_empty`, `delete_empty`,
//! `put_json`, `put_empty`, `patch_json`) funnels every call through
//! `decode` / `decode_empty`, which lift the server's `{"error": "..."}` body
//! into [`ApiError::Status`]. DTOs live in [`crate::protocol`].

use gloo_net::http::{Request, Response};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::protocol::{
    AddGalleryImageRequest, AddGalleryImageResponse, AddGalleryImagesBatchRequest,
    AddGalleryImagesBatchResponse, AdminResetPasswordRequest, AuthResponse, ChangePasswordRequest,
    ChannelListResponse, ChannelSummary, CreateChannelRequest, CreateDmRequest, CreateEmojiRequest,
    CreateGuildRequest, CreateLorebookEntryRequest, CreateLorebookEntryResponse,
    CreatePersonaRequest, DmSummary, EditMessageRequest, ErrorBody, FriendRequest, GuestSummary,
    GuildDetail, GuildSummary, InviteGuestRequest, InviteMemberRequest, InviteToDmRequest,
    ListCameosResponse, ListDmsResponse, ListEmojiResponse, ListFeedbackResponse,
    ListFriendsResponse, ListGuestsResponse, ListGuildsResponse, ListLorebookResponse,
    ListMembersResponse, ListMessagesResponse, ListPersonaEditorsResponse, ListPersonasResponse,
    LoginRequest, MarkReadRequest, MeResponse, PatchChannelRequest, PatchGuildRequest,
    PatchLorebookEntryRequest, PatchPersonaRequest, PersonaDetail, PersonaSummary,
    PushSubscribeRequest, RailOrderRequest, ReadStateResponse, RegisterRequest, RollRequest,
    SendMessageRequest, SendMessageResponse, SendSystemMessageRequest, SetActivePersonaRequest,
    SetMemberRoleRequest, SubmitFeedbackRequest, SystemBroadcastResult, TypingDraftEntry,
    TypingPingRequest, UnreadResponse, VapidKeyResponse,
};

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

/// GET /auth/me — resolve the current session cookie to the signed-in account.
pub async fn current_user() -> Result<MeResponse, ApiError> {
    let resp = Request::get("/auth/me").send().await.map_err(net)?;
    decode(resp).await
}

/// POST /auth/register — create a new account and start a session.
pub async fn register(body: &RegisterRequest) -> Result<AuthResponse, ApiError> {
    post_json("/auth/register", body).await
}

/// POST /auth/login — exchange username + password for a session cookie.
pub async fn login(body: &LoginRequest) -> Result<AuthResponse, ApiError> {
    post_json("/auth/login", body).await
}

/// POST /auth/logout — drop the server-side session and clear the cookie.
pub async fn logout() -> Result<(), ApiError> {
    let resp = Request::post("/auth/logout").send().await.map_err(net)?;
    decode_empty(resp).await
}

/// POST /auth/change-password — change the signed-in account's password.
pub async fn change_password(current: &str, new: &str) -> Result<(), ApiError> {
    post_json_empty(
        "/auth/change-password",
        &ChangePasswordRequest {
            current_password: current.to_string(),
            new_password: new.to_string(),
        },
    )
    .await
}

/// PATCH /account — update the signed-in account's profile (M6). Either field may
/// be `None` (left untouched); `avatar` is a media id from a prior `POST /media`.
pub async fn patch_account(
    display_name: Option<&str>,
    avatar: Option<&str>,
) -> Result<(), ApiError> {
    patch_json(
        "/account",
        &crate::protocol::PatchAccountRequest {
            display_name: display_name.map(str::to_string),
            avatar: avatar.map(str::to_string),
        },
    )
    .await
}

/// POST /auth/admin/reset-password — set another user's password by username.
/// Admin-only.
pub async fn admin_reset_password(username: &str, new_password: &str) -> Result<(), ApiError> {
    post_json_empty(
        "/auth/admin/reset-password",
        &AdminResetPasswordRequest {
            username: username.to_string(),
            new_password: new_password.to_string(),
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// Guilds + channels
// ---------------------------------------------------------------------------

/// GET /guilds — list every guild the viewer is a member of.
pub async fn list_guilds() -> Result<ListGuildsResponse, ApiError> {
    get("/guilds").await
}

/// PUT /rail/order — persist the caller's personal guild-rail order (#17/FB2).
/// `guild_ids` is the full rail top-to-bottom; the server replaces the caller's
/// order rows.
pub async fn set_rail_order(guild_ids: Vec<String>) -> Result<(), ApiError> {
    put_json("/rail/order", &RailOrderRequest { guild_ids }).await
}

/// POST /guilds — create a new guild owned by the viewer.
pub async fn create_guild(name: &str) -> Result<GuildSummary, ApiError> {
    post_json(
        "/guilds",
        &CreateGuildRequest {
            name: name.to_string(),
        },
    )
    .await
}

/// GET /guilds/{gid} — fetch a guild's detail (channels included).
pub async fn get_guild(gid: &str) -> Result<GuildDetail, ApiError> {
    get(&format!("/guilds/{gid}")).await
}

/// POST /guilds/{gid}/members — invite a user to a guild by username
/// (owner/admin only).
pub async fn invite_member(gid: &str, username: &str) -> Result<(), ApiError> {
    post_json_empty(
        &format!("/guilds/{gid}/members"),
        &InviteMemberRequest {
            username: username.to_string(),
        },
    )
    .await
}

/// GET /guilds/{gid}/members — list a guild's members (any member may read).
/// The owner-only mutations (`set_member_role`/`remove_member`) gate themselves
/// server-side.
pub async fn list_members(gid: &str) -> Result<ListMembersResponse, ApiError> {
    get(&format!("/guilds/{gid}/members")).await
}

/// PUT /guilds/{gid}/members/{aid}/role — promote/demote a member (`role` is
/// `"admin"` or `"member"`; owner/admin only, owner's role is fixed).
pub async fn set_member_role(gid: &str, aid: &str, role: &str) -> Result<(), ApiError> {
    put_json(
        &format!("/guilds/{gid}/members/{aid}/role"),
        &SetMemberRoleRequest {
            role: role.to_string(),
        },
    )
    .await
}

/// DELETE /guilds/{gid}/members/{aid} — kick a member (owner/admin only; the
/// owner can't be removed).
pub async fn remove_member(gid: &str, aid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/guilds/{gid}/members/{aid}")).await
}

/// PATCH /guilds/{gid} — rename a guild (owner/admin only).
pub async fn patch_guild(gid: &str, name: &str) -> Result<(), ApiError> {
    patch_json(
        &format!("/guilds/{gid}"),
        &PatchGuildRequest {
            name: Some(name.to_string()),
            ..Default::default()
        },
    )
    .await
}

/// PATCH /guilds/{gid} — set the per-server accent (owner/admin only). An
/// empty string clears it back to the default. Sends ONLY accent_color.
pub async fn set_guild_accent(gid: &str, accent: &str) -> Result<(), ApiError> {
    patch_json(
        &format!("/guilds/{gid}"),
        &PatchGuildRequest {
            accent_color: Some(accent.to_string()),
            ..Default::default()
        },
    )
    .await
}

/// PATCH /guilds/{gid}/channels/{cid} — rename a channel (owner/admin only).
pub async fn patch_channel(gid: &str, cid: &str, name: &str) -> Result<(), ApiError> {
    patch_json(
        &format!("/guilds/{gid}/channels/{cid}"),
        &PatchChannelRequest {
            name: Some(name.to_string()),
            ..Default::default()
        },
    )
    .await
}

/// PATCH /guilds/{gid}/channels/{cid} — persist a channel's per-guild display
/// order (owner/admin only). Used by the reorder ↑/↓ controls, which swap two
/// channels' positions.
pub async fn set_channel_position(gid: &str, cid: &str, position: i64) -> Result<(), ApiError> {
    patch_json(
        &format!("/guilds/{gid}/channels/{cid}"),
        &PatchChannelRequest {
            position: Some(position),
            ..Default::default()
        },
    )
    .await
}

/// DELETE /guilds/{gid} — soft-delete a guild (owner/admin only).
pub async fn delete_guild(gid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/guilds/{gid}")).await
}

/// DELETE /guilds/{gid}/channels/{cid} — soft-delete a channel (owner/admin
/// only).
pub async fn delete_channel(gid: &str, cid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/guilds/{gid}/channels/{cid}")).await
}

/// POST /guilds/{gid}/channels — create a channel under a guild
/// (owner/admin only). `kind` is the channel type string.
pub async fn create_channel(gid: &str, name: &str, kind: &str) -> Result<ChannelSummary, ApiError> {
    post_json(
        &format!("/guilds/{gid}/channels"),
        &CreateChannelRequest {
            name: name.to_string(),
            kind: kind.to_string(),
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// GET /channels/{cid}/messages — list messages, optionally resuming from a
/// `(sent_at, id)` cursor (the live-poll tail).
pub async fn list_messages(
    cid: &str,
    cursor: Option<&(String, String)>,
) -> Result<ListMessagesResponse, ApiError> {
    let url = match cursor {
        Some((since, after_id)) => {
            format!("/channels/{cid}/messages?since={since}&after_id={after_id}")
        }
        None => format!("/channels/{cid}/messages"),
    };
    get(&url).await
}

/// GET /channels/{cid}/messages?before={…} — load the page of older history
/// immediately before a `(sent_at, id)` cursor (scroll-up backfill). Returned
/// ASC, ready to prepend.
pub async fn list_messages_before(
    cid: &str,
    before: &(String, String),
) -> Result<ListMessagesResponse, ApiError> {
    let (before_ts, before_id) = before;
    get(&format!(
        "/channels/{cid}/messages?before={before_ts}&before_id={before_id}"
    ))
    .await
}

/// POST /channels/{cid}/messages — send a message with optional attachments,
/// a worn persona, an optional reply-to parent (L-3), and an optional delivery
/// effect (M4/T5: whisper/shout/spell; server-validated).
pub async fn post_message(
    cid: &str,
    body: &str,
    attachment_ids: Vec<String>,
    persona_id: Option<String>,
    reply_to_id: Option<String>,
    effect: Option<String>,
) -> Result<SendMessageResponse, ApiError> {
    post_json(
        &format!("/channels/{cid}/messages"),
        &SendMessageRequest {
            body: body.to_string(),
            attachment_ids,
            persona_id,
            reply_to_id,
            effect,
        },
    )
    .await
}

/// POST /channels/{cid}/roll — the Fate Engine (M4/T6): send a roll
/// EXPRESSION (`NdM(+|-K)?`, `coin`, `oracle`) and let the SERVER roll it —
/// the client never computes an outcome. The result lands as an immutable
/// `kind='roll'` message; a bad expression is a 400 whose message the
/// composer status line surfaces.
pub async fn roll(
    cid: &str,
    expr: &str,
    persona: Option<String>,
) -> Result<SendMessageResponse, ApiError> {
    post_json(
        &format!("/channels/{cid}/roll"),
        &RollRequest {
            expr: expr.to_string(),
            persona,
        },
    )
    .await
}

/// POST /channels/{cid}/typing — ping "I am typing" in a channel (#19).
/// Fire-and-forget: the composer calls this at most every ~2s while typing;
/// errors are ignored by the caller. `draft` is the Ghost Quill opt-in
/// (M4/T7): `Some(compose text)` only when the SENDER's own pref is on
/// (the server stores it ephemerally for other members to fetch);
/// `None` sends the classic body-less ping, which also clears any draft
/// the server still holds for this caller. `effect` is the composer's
/// currently ARMED delivery effect (review M-01) — the server pre-masks a
/// whisper-armed draft to the fixed `(whisper)` placeholder BEFORE storing
/// it, so spoiler text never streams live to the audience it will land
/// veiled from. Only meaningful alongside a `draft`; the bare ping has no
/// body to mask.
pub async fn post_typing(
    cid: &str,
    draft: Option<String>,
    effect: Option<String>,
) -> Result<(), ApiError> {
    match draft {
        Some(draft) => {
            post_json_empty(
                &format!("/channels/{cid}/typing"),
                &TypingPingRequest {
                    draft: Some(draft),
                    effect,
                },
            )
            .await
        }
        None => post_empty(&format!("/channels/{cid}/typing")).await,
    }
}

/// GET /channels/{cid}/typing-drafts — other members' live Ghost Quill drafts
/// (M4/T7). Called on `Typing`/`MessageCreated` SSE events for the OPEN
/// channel, and only when the RECEIVER's pref is on — draft text rides this
/// permission-checked fetch, never the id-only SSE bus.
pub async fn get_typing_drafts(cid: &str) -> Result<Vec<TypingDraftEntry>, ApiError> {
    get(&format!("/channels/{cid}/typing-drafts")).await
}

/// PATCH /channels/{cid}/messages/{mid} — edit one of your own messages.
pub async fn edit_message(cid: &str, mid: &str, body: &str) -> Result<(), ApiError> {
    patch_json(
        &format!("/channels/{cid}/messages/{mid}"),
        &EditMessageRequest {
            body: body.to_string(),
        },
    )
    .await
}

/// DELETE /channels/{cid}/messages/{mid} — soft-delete one of your own
/// messages.
pub async fn delete_message(cid: &str, mid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/channels/{cid}/messages/{mid}")).await
}

// ---------------------------------------------------------------------------
// Cross-device read state (L-1)
// ---------------------------------------------------------------------------

/// POST /channels/{cid}/mark-read — persist the caller's last-seen `(sent_at, id)`
/// cursor for this channel so read/unread syncs across devices. Fire-and-forget
/// from the caller's side (the local mark + localStorage write is the offline
/// source of truth); the server keeps the MAX cursor so an older mark can't regress.
pub async fn mark_read(cid: &str, sent_at: &str, id: &str) -> Result<(), ApiError> {
    post_json_empty(
        &format!("/channels/{cid}/mark-read"),
        &MarkReadRequest {
            sent_at: sent_at.to_string(),
            id: id.to_string(),
        },
    )
    .await
}

/// GET /channels/read-state — the caller's stored per-channel read cursors,
/// used to hydrate `notify.last_seen` on shell mount (cross-device sync).
pub async fn read_state() -> Result<ReadStateResponse, ApiError> {
    get("/channels/read-state").await
}

/// GET /unread — batched unread/ping summary for every visible text channel (M1).
pub async fn get_unread() -> Result<UnreadResponse, ApiError> {
    get("/unread").await
}

// ---------------------------------------------------------------------------
// Trash + restore (#22 soft-delete)
// ---------------------------------------------------------------------------

/// GET /guilds/trash — list the caller's own soft-deleted guilds.
pub async fn list_deleted_guilds() -> Result<ListGuildsResponse, ApiError> {
    get("/guilds/trash").await
}

/// POST /guilds/{gid}/restore — restore a soft-deleted guild (owner only).
pub async fn restore_guild(gid: &str) -> Result<(), ApiError> {
    post_empty(&format!("/guilds/{gid}/restore")).await
}

/// GET /guilds/{gid}/trash/channels — list soft-deleted channels in a guild
/// (owner/admin only).
pub async fn list_deleted_channels(gid: &str) -> Result<ChannelListResponse, ApiError> {
    get(&format!("/guilds/{gid}/trash/channels")).await
}

/// POST /guilds/{gid}/channels/{cid}/restore — restore a soft-deleted channel
/// (owner/admin only).
pub async fn restore_channel(gid: &str, cid: &str) -> Result<(), ApiError> {
    post_empty(&format!("/guilds/{gid}/channels/{cid}/restore")).await
}

/// GET /channels/{cid}/messages/trash — list soft-deleted messages in a
/// channel (any member).
pub async fn list_deleted_messages(cid: &str) -> Result<ListMessagesResponse, ApiError> {
    get(&format!("/channels/{cid}/messages/trash")).await
}

/// POST /channels/{cid}/messages/{mid}/restore — restore one of your own
/// soft-deleted messages.
pub async fn restore_message(cid: &str, mid: &str) -> Result<(), ApiError> {
    post_empty(&format!("/channels/{cid}/messages/{mid}/restore")).await
}

// ---------------------------------------------------------------------------
// Personas + wardrobe
// ---------------------------------------------------------------------------

/// GET /personas — list the viewer's wardrobe (owned + shared-as-editor).
pub async fn list_personas() -> Result<ListPersonasResponse, ApiError> {
    get("/personas").await
}

/// POST /personas — create a persona owned by the viewer.
pub async fn create_persona(name: &str, description: &str) -> Result<PersonaSummary, ApiError> {
    post_json(
        "/personas",
        &CreatePersonaRequest {
            name: name.to_string(),
            description: Some(description.to_string()),
            color: None,
        },
    )
    .await
}

/// PATCH /personas/{pid} — update a persona's name, description and/or color
/// (owner/editor).
pub async fn patch_persona(
    pid: &str,
    name: Option<String>,
    description: Option<String>,
    color: Option<String>,
) -> Result<(), ApiError> {
    patch_json(
        &format!("/personas/{pid}"),
        &PatchPersonaRequest {
            name,
            description,
            color,
            position: None,
        },
    )
    .await
}

/// PATCH /personas/{pid} — persist a persona's wardrobe display order
/// (owner/editor). Used by the reorder ↑/↓ controls, which swap two personas'
/// positions.
pub async fn set_persona_position(pid: &str, position: i64) -> Result<(), ApiError> {
    patch_json(
        &format!("/personas/{pid}"),
        &PatchPersonaRequest {
            position: Some(position),
            ..Default::default()
        },
    )
    .await
}

/// DELETE /personas/{pid} — delete a persona (owner only).
pub async fn delete_persona(pid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/personas/{pid}")).await
}

/// DELETE /personas/{pid}/leave — leave a shared persona; drop it from the
/// caller's list (editor only).
pub async fn leave_persona(pid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/personas/{pid}/leave")).await
}

/// GET /personas/{pid}/editors — list the editors of a persona (owner only).
pub async fn list_persona_editors(pid: &str) -> Result<ListPersonaEditorsResponse, ApiError> {
    get(&format!("/personas/{pid}/editors")).await
}

/// PUT /personas/{pid}/editors/{aid} — share a persona with a friend, granting
/// editor access (owner only).
pub async fn add_persona_editor(pid: &str, aid: &str) -> Result<(), ApiError> {
    put_empty(&format!("/personas/{pid}/editors/{aid}")).await
}

/// DELETE /personas/{pid}/editors/{aid} — revoke an editor's access to a
/// persona (owner only).
pub async fn remove_persona_editor(pid: &str, aid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/personas/{pid}/editors/{aid}")).await
}

/// PUT /guilds/{gid}/active-persona — wear (`Some`) or take off (`None`) a
/// persona in a guild.
///
/// DEPRECATED: superseded by the per-channel `set_channel_active_persona`. The
/// guild endpoint + `guild_member.active_persona` field remain server-side but
/// the client no longer calls this. Kept to avoid churn.
#[allow(dead_code)]
pub async fn set_active_persona(gid: &str, persona_id: Option<String>) -> Result<(), ApiError> {
    put_json(
        &format!("/guilds/{gid}/active-persona"),
        &SetActivePersonaRequest { persona_id },
    )
    .await
}

/// PUT /channels/{cid}/active-persona — wear (`Some`) or take off (`None`) a
/// persona in a specific channel (per-channel worn persona, #persona).
pub async fn set_channel_active_persona(
    cid: &str,
    persona_id: Option<String>,
) -> Result<(), ApiError> {
    put_json(
        &format!("/channels/{cid}/active-persona"),
        &SetActivePersonaRequest { persona_id },
    )
    .await
}

/// GET /personas/{pid} — fetch a persona's detail (name, description, avatar,
/// gallery, and — for the owner — its share key + editor roster).
pub async fn get_persona(pid: &str) -> Result<PersonaDetail, ApiError> {
    get(&format!("/personas/{pid}")).await
}

/// POST /personas/{pid}/gallery — attach an already-uploaded media id to a
/// persona's gallery (owner/editor). Returns the new gallery image id.
pub async fn add_gallery_image(
    pid: &str,
    media_id: &str,
) -> Result<AddGalleryImageResponse, ApiError> {
    post_json(
        &format!("/personas/{pid}/gallery"),
        &AddGalleryImageRequest {
            media_id: media_id.to_string(),
        },
    )
    .await
}

/// POST /api/personas/{id}/gallery/batch — atomically append multiple media_ids
/// to a persona's gallery. Single SurrealDB transaction: all inserts succeed or
/// the whole batch fails. The returned `ids` are in the same order as the input
/// `media_ids`, so the client can correlate each new gallery row with the
/// media id it asked for.
pub async fn upload_gallery_images_batch(
    pid: &str,
    media_ids: &[String],
) -> Result<AddGalleryImagesBatchResponse, ApiError> {
    post_json(
        &format!("/personas/{pid}/gallery/batch"),
        &AddGalleryImagesBatchRequest {
            media_ids: media_ids.to_vec(),
        },
    )
    .await
}

/// DELETE /personas/{pid}/gallery/{img} — remove a gallery image from a
/// persona (owner/editor).
pub async fn remove_gallery_image(pid: &str, img: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/personas/{pid}/gallery/{img}")).await
}

/// PUT /personas/{pid}/avatar — set a persona's primary avatar to an
/// already-uploaded media id.
pub async fn set_persona_avatar(pid: &str, media_id: &str) -> Result<(), ApiError> {
    put_json(
        &format!("/personas/{pid}/avatar"),
        &crate::protocol::SetAvatarRequest {
            media_id: media_id.to_string(),
        },
    )
    .await
}

/// PUT /guilds/{gid}/icon — set a guild's icon to an already-uploaded media id.
/// The server re-derives the per-server accent from the image (M6, effect G).
pub async fn set_guild_icon(gid: &str, media_id: &str) -> Result<(), ApiError> {
    put_json(
        &format!("/guilds/{gid}/icon"),
        &crate::protocol::SetGuildIconRequest {
            media_id: media_id.to_string(),
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------

/// POST /media — upload a browser `File`/`Blob` as multipart/form-data (field
/// `file`); returns the new media id from the `{ "id": "..." }` body.
///
/// Thin wrapper over [`upload_media_with_progress`] with a no-op progress sink,
/// for callers (e.g. the wardrobe) that don't render an upload bar.
pub async fn upload_media(file: &web_sys::File) -> Result<String, ApiError> {
    upload_media_with_progress(file, |_| {}).await
}

/// POST /media with upload-progress callbacks (F-8). `on_progress` is invoked
/// with a fraction `0.0..=1.0` as the body bytes go up; if the browser can't
/// compute a total it's never called (the caller shows an indeterminate bar).
///
/// gloo-net's Fetch transport exposes no upload-progress event, so this drives
/// a raw `XMLHttpRequest` and subscribes to `xhr.upload.onprogress`. The async
/// completion is bridged through a `oneshot` resolved by the `load`/`error`/
/// `abort` handlers. Same multipart shape + same `{ "id": ".." }` envelope as
/// the Fetch path, so the server is none the wiser.
pub async fn upload_media_with_progress<F>(
    file: &web_sys::File,
    on_progress: F,
) -> Result<String, ApiError>
where
    F: Fn(f32) + 'static,
{
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    let form = web_sys::FormData::new()
        .map_err(|e| ApiError::Codec(format!("FormData unavailable: {e:?}")))?;
    form.append_with_blob("file", file)
        .map_err(|e| ApiError::Codec(format!("FormData append failed: {e:?}")))?;

    let xhr = web_sys::XmlHttpRequest::new()
        .map_err(|e| ApiError::Codec(format!("XMLHttpRequest unavailable: {e:?}")))?;
    xhr.open_with_async("POST", "/media", true)
        .map_err(|e| ApiError::Network(format!("open failed: {e:?}")))?;

    // Upload-progress: write the fraction into the caller's sink. `length_computable`
    // is false for chunked/unknown-length bodies → leave the bar indeterminate.
    let upload = xhr
        .upload()
        .map_err(|e| ApiError::Network(format!("upload stream unavailable: {e:?}")))?;
    let on_progress_cb =
        Closure::<dyn FnMut(web_sys::ProgressEvent)>::new(move |ev: web_sys::ProgressEvent| {
            if ev.length_computable() && ev.total() > 0.0 {
                let frac = (ev.loaded() / ev.total()) as f32;
                on_progress(frac.clamp(0.0, 1.0));
            }
        });
    upload.set_onprogress(Some(on_progress_cb.as_ref().unchecked_ref()));

    // Bridge XHR's event-driven completion to async/await via a `Promise` (the
    // `futures` oneshot crate isn't in the hydrate dep graph). The promise
    // executor hands us its `resolve` fn; the load handler resolves with the
    // status code, error/abort reject. Each handler fires at most once.
    use wasm_bindgen::JsValue;
    let resolve_cell: std::rc::Rc<std::cell::RefCell<Option<js_sys::Function>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let reject_cell: std::rc::Rc<std::cell::RefCell<Option<js_sys::Function>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let promise = {
        let resolve_cell = resolve_cell.clone();
        let reject_cell = reject_cell.clone();
        js_sys::Promise::new(&mut move |resolve, reject| {
            *resolve_cell.borrow_mut() = Some(resolve);
            *reject_cell.borrow_mut() = Some(reject);
        })
    };

    let on_load = {
        let resolve_cell = resolve_cell.clone();
        Closure::<dyn FnMut()>::new(move || {
            if let Some(resolve) = resolve_cell.borrow_mut().take() {
                let _ = resolve.call1(&JsValue::NULL, &JsValue::NULL);
            }
        })
    };
    let on_fail = {
        let reject_cell = reject_cell.clone();
        Closure::<dyn FnMut()>::new(move || {
            if let Some(reject) = reject_cell.borrow_mut().take() {
                let _ = reject.call1(&JsValue::NULL, &JsValue::NULL);
            }
        })
    };
    xhr.set_onload(Some(on_load.as_ref().unchecked_ref()));
    xhr.set_onerror(Some(on_fail.as_ref().unchecked_ref()));
    xhr.set_onabort(Some(on_fail.as_ref().unchecked_ref()));

    xhr.send_with_opt_form_data(Some(&form))
        .map_err(|e| ApiError::Network(format!("send failed: {e:?}")))?;

    // Await completion; keep the closures alive until the request settles.
    let settled = wasm_bindgen_futures::JsFuture::from(promise).await;
    drop(on_progress_cb);
    drop(on_load);
    drop(on_fail);
    if settled.is_err() {
        return Err(ApiError::Network("upload failed".to_string()));
    }

    // 2xx → parse the `{ "id": ".." }` envelope; otherwise lift the server's
    // `{ "error": ".." }` body into a Status error, mirroring `decode`.
    let status = xhr.status().unwrap_or(0);
    let text = xhr.response_text().ok().flatten().unwrap_or_default();
    if (200..300).contains(&status) {
        serde_json::from_str::<MediaUploadResponse>(&text)
            .map(|b| b.id)
            .map_err(|e| ApiError::Codec(e.to_string()))
    } else {
        let msg = serde_json::from_str::<ErrorBody>(&text)
            .map(|b| b.error)
            .unwrap_or_else(|_| "request failed".to_string());
        Err(ApiError::Status(status, msg))
    }
}

#[derive(serde::Deserialize)]
struct MediaUploadResponse {
    id: String,
}

// ---------------------------------------------------------------------------
// Lorebook
// ---------------------------------------------------------------------------

/// GET /channels/{cid}/lorebook — list a channel's lorebook entries.
pub async fn list_lore(cid: &str) -> Result<ListLorebookResponse, ApiError> {
    get(&format!("/channels/{cid}/lorebook")).await
}

/// POST /channels/{cid}/lorebook — create a lorebook entry (`keys` + `content`).
pub async fn create_lore(
    cid: &str,
    keys: Vec<String>,
    content: &str,
) -> Result<CreateLorebookEntryResponse, ApiError> {
    post_json(
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

/// PATCH /channels/{cid}/lorebook/{eid} — update fields of a lorebook entry.
pub async fn patch_lore(
    cid: &str,
    eid: &str,
    req: &PatchLorebookEntryRequest,
) -> Result<(), ApiError> {
    patch_json(&format!("/channels/{cid}/lorebook/{eid}"), req).await
}

/// DELETE /channels/{cid}/lorebook/{eid} — delete a lorebook entry.
pub async fn delete_lore(cid: &str, eid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/channels/{cid}/lorebook/{eid}")).await
}

// ---------------------------------------------------------------------------
// Friends
// ---------------------------------------------------------------------------

/// GET /friends — list the viewer's friends (accepted) and pending requests.
pub async fn list_friends() -> Result<ListFriendsResponse, ApiError> {
    get("/friends").await
}

/// POST /friends — send a friend request to a user by username.
pub async fn add_friend(username: &str) -> Result<(), ApiError> {
    post_json_empty(
        "/friends",
        &FriendRequest {
            username: username.to_string(),
        },
    )
    .await
}

/// POST /friends/{aid}/accept — accept a pending incoming friend request.
pub async fn accept_friend(aid: &str) -> Result<(), ApiError> {
    post_empty(&format!("/friends/{aid}/accept")).await
}

/// DELETE /friends/{aid} — unfriend, or reject a pending request.
pub async fn remove_friend(aid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/friends/{aid}")).await
}

// ---------------------------------------------------------------------------
// Direct messages (M7/P1)
// ---------------------------------------------------------------------------

/// GET /dms — the caller's DM threads (1:1 + groups).
pub async fn list_dms() -> Result<ListDmsResponse, ApiError> {
    get("/dms").await
}

/// POST /dms — start a DM with friends (one member = 1:1, deduped; 2+ = group).
pub async fn create_dm(members: Vec<String>, title: Option<String>) -> Result<DmSummary, ApiError> {
    post_json("/dms", &CreateDmRequest { members, title }).await
}

/// POST /dms/{tid}/members — invite an accepted friend into a thread.
pub async fn invite_to_dm(tid: &str, account_id: &str) -> Result<DmSummary, ApiError> {
    post_json(
        &format!("/dms/{tid}/members"),
        &InviteToDmRequest {
            account_id: account_id.to_string(),
        },
    )
    .await
}

/// DELETE /dms/{tid}/members/me — leave a thread.
pub async fn leave_dm(tid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/dms/{tid}/members/me")).await
}

// ---------------------------------------------------------------------------
// Guest cameos (M7/P2)
// ---------------------------------------------------------------------------

/// GET /cameos — the caller's active cameos (guest-side standalone list).
pub async fn list_cameos() -> Result<ListCameosResponse, ApiError> {
    get("/cameos").await
}

/// POST /channels/{cid}/guests — invite an accepted friend as a guest in this
/// guild text channel, with an optional RFC3339 expiry.
pub async fn invite_guest(
    cid: &str,
    account_id: &str,
    expires_at: Option<String>,
) -> Result<GuestSummary, ApiError> {
    post_json(
        &format!("/channels/{cid}/guests"),
        &InviteGuestRequest {
            account_id: account_id.to_string(),
            expires_at,
        },
    )
    .await
}

/// GET /channels/{cid}/guests — the channel's active guests (host view).
pub async fn list_guests(cid: &str) -> Result<ListGuestsResponse, ApiError> {
    get(&format!("/channels/{cid}/guests")).await
}

/// DELETE /channels/{cid}/guests/{aid} — revoke a guest (inviter or manager).
pub async fn revoke_guest(cid: &str, aid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/channels/{cid}/guests/{aid}")).await
}

/// DELETE /channels/{cid}/guests/me — leave a cameo (the guest ends it).
pub async fn leave_cameo(cid: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/channels/{cid}/guests/me")).await
}

// ---------------------------------------------------------------------------
// Custom emoji
// ---------------------------------------------------------------------------

/// GET /guilds/{guild_id}/emoji — list all custom emoji in a guild (member
/// required).
pub async fn list_emoji(guild_id: &str) -> Result<ListEmojiResponse, ApiError> {
    get(&format!("/guilds/{guild_id}/emoji")).await
}

/// POST /guilds/{guild_id}/emoji — create a custom emoji. `req.media_id` must
/// be an id returned by a prior `POST /media` upload. `req.name` must match
/// `^[a-z0-9_]{2,32}$`. Member required.
pub async fn create_emoji(
    guild_id: &str,
    req: &CreateEmojiRequest,
) -> Result<crate::protocol::CustomEmoji, ApiError> {
    post_json(&format!("/guilds/{guild_id}/emoji"), req).await
}

/// DELETE /guilds/{guild_id}/emoji/{name} — delete a custom emoji by its
/// shortcode name (manager/admin required).
pub async fn delete_emoji(guild_id: &str, name: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/guilds/{guild_id}/emoji/{name}")).await
}

// ---------------------------------------------------------------------------
// Web Push (#30)
// ---------------------------------------------------------------------------

/// GET /push/vapid-key — fetch the server's VAPID public key. Returns
/// `Err(Status(404, _))` when push isn't configured server-side — callers
/// treat that as "push unavailable" and skip subscribing.
pub async fn push_vapid_key() -> Result<VapidKeyResponse, ApiError> {
    get("/push/vapid-key").await
}

/// POST /push/subscribe — register this browser's push subscription with the
/// server.
pub async fn push_subscribe(req: &PushSubscribeRequest) -> Result<(), ApiError> {
    post_json_empty("/push/subscribe", req).await
}

// ---------------------------------------------------------------------------
// Feedback / bug reports (#31)
// ---------------------------------------------------------------------------

/// GET /feedback — list submitted feedback (admin only — the server gates on
/// `AUTHLYN_ADMIN_USERNAMES`). Non-admins get a 403, surfaced as an `ApiError`
/// the caller can treat as "no inbox for you".
pub async fn list_feedback() -> Result<ListFeedbackResponse, ApiError> {
    get("/feedback").await
}

/// POST /feedback — submit a feedback item (bug | idea | other).
pub async fn submit_feedback(req: &SubmitFeedbackRequest) -> Result<(), ApiError> {
    post_json_empty("/feedback", req).await
}

/// DELETE /feedback/{id} — soft-delete (archive) a feedback item (admin only).
pub async fn delete_feedback(id: &str) -> Result<(), ApiError> {
    delete_empty(&format!("/feedback/{id}")).await
}

// ---------------------------------------------------------------------------
// App-admin system broadcast (Nova DOT)
// ---------------------------------------------------------------------------

/// POST /admin/system-message — broadcast a "Nova DOT" system message into every
/// guild's default channel (admin only — the server gates on
/// `AUTHLYN_ADMIN_USERNAMES`; non-admins get a 403).
pub async fn broadcast_system_message(
    req: &SendSystemMessageRequest,
) -> Result<SystemBroadcastResult, ApiError> {
    post_json("/admin/system-message", req).await
}

// ---------------------------------------------------------------------------
// Low-level helpers (reused by later slices)
// ---------------------------------------------------------------------------

/// GET `url`; deserialize the JSON response as `T`.
pub(crate) async fn get<T: DeserializeOwned>(url: &str) -> Result<T, ApiError> {
    let resp = Request::get(url).send().await.map_err(net)?;
    decode(resp).await
}

/// POST `url` with no body; decode a 2xx no-body response as `()`.
async fn post_empty(url: &str) -> Result<(), ApiError> {
    let resp = Request::post(url).send().await.map_err(net)?;
    decode_empty(resp).await
}

/// DELETE `url`; decode a 2xx no-body response as `()`.
async fn delete_empty(url: &str) -> Result<(), ApiError> {
    let resp = Request::delete(url).send().await.map_err(net)?;
    decode_empty(resp).await
}

/// POST `url` with a JSON body; deserialize the JSON response as `T`.
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

/// PATCH `url` with a JSON body; decode a 2xx no-body response as `()`.
/// Mirrors [`post_json`] for the dozen+ PATCH-with-204 sites.
async fn patch_json<B: Serialize>(url: &str, body: &B) -> Result<(), ApiError> {
    let resp = Request::patch(url)
        .json(body)
        .map_err(codec)?
        .send()
        .await
        .map_err(net)?;
    decode_empty(resp).await
}

/// PUT `url` with a JSON body; decode a 2xx no-body response as `()`.
async fn put_json<B: Serialize>(url: &str, body: &B) -> Result<(), ApiError> {
    let resp = Request::put(url)
        .json(body)
        .map_err(codec)?
        .send()
        .await
        .map_err(net)?;
    decode_empty(resp).await
}

/// PUT `url` with no body; decode a 2xx no-body response as `()`.
async fn put_empty(url: &str) -> Result<(), ApiError> {
    let resp = Request::put(url).send().await.map_err(net)?;
    decode_empty(resp).await
}

/// POST `url` with a JSON body; decode a 2xx no-body response as `()`.
/// Mirrors [`post_json`] for sites that POST a body but want only the status.
async fn post_json_empty<B: Serialize>(url: &str, body: &B) -> Result<(), ApiError> {
    let resp = Request::post(url)
        .json(body)
        .map_err(codec)?
        .send()
        .await
        .map_err(net)?;
    decode_empty(resp).await
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
