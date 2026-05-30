//! Read-path actions — the native port of `src/ui/shell/act/`.
//!
//! Same semantics as the web: load the guild rail, open a guild (→ its channels
//! → first text channel), open a channel (clear view → initial page → ingest),
//! scroll-up backfill, and a 1.5s poll. `RwSignal` ops → Freya `State` writes;
//! `spawn_local`+gloo-timers → Freya `spawn`/`spawn_forever`+`tokio::time::sleep`.
//!
//! Invariant carried over: a **switch-epoch guard** — every channel open bumps
//! `epoch`; a fetch tagged with a now-stale epoch must not write into the
//! freshly-switched channel. We also never call a `spawn`-launching fn from
//! inside a task (Freya's `spawn` ties to the calling component scope); loops
//! inline their fetches instead.

use std::collections::HashSet;
use std::time::Duration;

use freya::prelude::*;

use crate::native::api::client;
use crate::native::state::{NativeState, StagedAttachment};
use crate::protocol::{ChannelSummary, MessageEnvelope};

/// Newest-page size; a short page means the whole channel fits (web parity).
const MESSAGES_PAGE_LIMIT: usize = 100;

/// Mount-time auto-login for dev/headless runs: when `AUTHLYN_NATIVE_USER` is
/// set in the environment, log in (creating the account on first run) and enter
/// the shell directly. With no env creds, this is a no-op and the interactive
/// login form is shown instead.
pub async fn bootstrap(state: NativeState) {
    let Ok(user) = std::env::var("AUTHLYN_NATIVE_USER") else {
        return;
    };
    let pass =
        std::env::var("AUTHLYN_NATIVE_PASS").unwrap_or_else(|_| "native-dev-password".to_string());
    if client().ensure_session(&user, &pass).await.is_err() {
        *state.auth_error.write_unchecked() = "auto-login failed".to_string();
        return;
    }
    *state.authed.write_unchecked() = true;
    post_auth_load(state).await;
}

/// Submit the interactive login/register form: authenticate per the current
/// mode, and on success enter the shell + load its data; on failure surface a
/// friendly message under the form.
pub fn submit_login(state: NativeState) {
    let user = state.auth_user.peek().trim().to_string();
    let pass = state.auth_pass.peek().clone();
    if user.is_empty() || pass.is_empty() {
        *state.auth_error.write_unchecked() = "Enter a username and password.".to_string();
        return;
    }
    if *state.auth_busy.peek() {
        return;
    }
    let register = *state.auth_register.peek();
    *state.auth_busy.write_unchecked() = true;
    *state.auth_error.write_unchecked() = String::new();
    spawn(async move {
        let res = if register {
            client().register(&user, &pass).await
        } else {
            client().login(&user, &pass).await
        };
        *state.auth_busy.write_unchecked() = false;
        match res {
            Ok(_) => {
                *state.auth_pass.write_unchecked() = String::new();
                *state.authed.write_unchecked() = true;
                post_auth_load(state).await;
            }
            Err(e) => {
                *state.auth_error.write_unchecked() = auth_error_message(&e, register);
            }
        }
    });
}

/// Log out: end the session, forget the cookie, and return to the login form
/// with the shell state cleared.
pub fn logout(state: NativeState) {
    spawn(async move {
        let _ = client().logout().await;
        *state.authed.write_unchecked() = false;
        *state.me.write_unchecked() = None;
        *state.guilds.write_unchecked() = Vec::new();
        *state.channels.write_unchecked() = Vec::new();
        *state.sel_server.write_unchecked() = None;
        *state.sel_channel.write_unchecked() = None;
        *state.messages.write_unchecked() = Vec::new();
        *state.seen.write_unchecked() = HashSet::new();
        *state.personas.write_unchecked() = Vec::new();
        *state.active_persona.write_unchecked() = None;
        *state.auth_user.write_unchecked() = String::new();
        *state.auth_pass.write_unchecked() = String::new();
        *state.status.write_unchecked() = "connecting\u{2026}".to_string();
    });
}

/// Load the post-auth shell data: profile, guild rail, personas, and open the
/// first guild's first text channel. Fully inline (no nested `spawn`) so it can
/// be awaited from the mount task or the form-submit task.
async fn post_auth_load(state: NativeState) {
    if let Ok(me) = client().current_user().await {
        *state.me.write_unchecked() = Some(me);
    }
    let Ok(r) = client().list_guilds().await else {
        *state.status.write_unchecked() = "could not load guilds".to_string();
        return;
    };
    let guilds = r.guilds;
    *state.status.write_unchecked() = format!("ready \u{b7} {} guild(s)", guilds.len());
    *state.guilds.write_unchecked() = guilds.clone();

    if let Ok(p) = client().list_personas().await {
        *state.personas.write_unchecked() = p.personas;
    }

    if let Some(g) = guilds.first().cloned() {
        *state.sel_server.write_unchecked() = Some(g.id.clone());
        load_guild_emoji(state, &g.id).await;
        if let Ok(d) = client().get_guild(&g.id).await {
            *state.channels.write_unchecked() = d.channels.clone();
            let first = d
                .channels
                .iter()
                .find(|c| c.kind == "text")
                .or_else(|| d.channels.first())
                .cloned();
            if let Some(ch) = first {
                open_channel_inner(state, ch).await;
            }
        }
    }
}

/// Load the open guild's custom emoji (for `:`-autocomplete). Inline (no nested
/// `spawn`) so it can be awaited from another task.
async fn load_guild_emoji(state: NativeState, gid: &str) {
    if let Ok(r) = client().list_guild_emoji(gid).await {
        *state.guild_emoji.write_unchecked() = r.emoji;
    }
}

/// Map an auth failure to a friendly message for the form.
fn auth_error_message(e: &crate::native::api::ApiError, register: bool) -> String {
    match e.status() {
        Some(401) => "Wrong username or password.".to_string(),
        Some(409) => "That username is taken.".to_string(),
        _ if register => format!("Could not create account: {e}"),
        _ => format!("Could not sign in: {e}"),
    }
}

/// Load the guild rail.
pub fn refresh_guilds(state: NativeState) {
    spawn(async move {
        if let Ok(r) = client().list_guilds().await {
            *state.guilds.write_unchecked() = r.guilds;
        }
    });
}

/// Open a guild: fetch its channels, then open the first text channel.
pub fn open_server(state: NativeState, gid: String) {
    *state.sel_server.write_unchecked() = Some(gid.clone());
    *state.channels.write_unchecked() = Vec::new();
    *state.guild_emoji.write_unchecked() = Vec::new();
    spawn(async move {
        load_guild_emoji(state, &gid).await;
        if let Ok(d) = client().get_guild(&gid).await {
            *state.channels.write_unchecked() = d.channels.clone();
            let first = d
                .channels
                .iter()
                .find(|c| c.kind == "text")
                .or_else(|| d.channels.first())
                .cloned();
            if let Some(ch) = first {
                open_channel_inner(state, ch).await;
            }
        }
    });
}

/// Open a channel from a click handler.
pub fn open_channel(state: NativeState, ch: ChannelSummary) {
    spawn(async move { open_channel_inner(state, ch).await });
}

/// Clear the message view and load the channel's newest page. `async`, no inner
/// `spawn`, so it can be awaited from another task (e.g. `open_server`).
async fn open_channel_inner(state: NativeState, ch: ChannelSummary) {
    let epoch = *state.epoch.peek() + 1;
    *state.epoch.write_unchecked() = epoch;
    *state.sel_channel.write_unchecked() = Some(ch.clone());
    *state.messages.write_unchecked() = Vec::new();
    *state.cursor.write_unchecked() = None;
    *state.oldest.write_unchecked() = None;
    *state.loading_older.write_unchecked() = false;
    *state.more_history.write_unchecked() = true;
    *state.seen.write_unchecked() = HashSet::new();
    *state.typing.write_unchecked() = Vec::new();
    *state.persona_menu.write_unchecked() = false;
    *state.active_persona.write_unchecked() = None;

    if let Ok(l) = client().list_messages(&ch.id, None).await {
        // A newer switch happened while we were fetching — drop this result.
        if *state.epoch.peek() != epoch {
            return;
        }
        *state.more_history.write_unchecked() = l.messages.len() == MESSAGES_PAGE_LIMIT;
        *state.oldest.write_unchecked() = l
            .messages
            .first()
            .map(|m| (m.sent_at.clone(), m.id.clone()));
        *state.typing.write_unchecked() = l.typing;
        // Restore the "speaking as" state for this channel (web parity).
        *state.active_persona.write_unchecked() = l.active_persona;
        ingest(state, l.messages);
    }
}

/// Refresh the caller's persona list (for the composer picker / wardrobe).
pub fn refresh_personas(state: NativeState) {
    spawn(async move {
        if let Ok(p) = client().list_personas().await {
            *state.personas.write_unchecked() = p.personas;
        }
    });
}

/// Wear (`Some`) or take off (`None`) a persona in the open channel. Updates the
/// signal immediately (so sends attribute right away) and persists the
/// per-channel state server-side; closes the picker.
pub fn wear_persona(state: NativeState, persona_id: Option<String>) {
    *state.active_persona.write_unchecked() = persona_id.clone();
    *state.persona_menu.write_unchecked() = false;
    let Some(ch) = state.sel_channel.peek().clone() else {
        return;
    };
    spawn(async move {
        let _ = client()
            .set_channel_active_persona(&ch.id, persona_id)
            .await;
    });
}

/// Dedupe by id via `seen`, append in order, advance `cursor` to the last.
pub fn ingest(state: NativeState, incoming: Vec<MessageEnvelope>) {
    let mut last_cursor = None;
    {
        let mut seen = state.seen.write_unchecked();
        let mut msgs = state.messages.write_unchecked();
        for m in incoming {
            if seen.contains(&m.id) {
                continue;
            }
            seen.insert(m.id.clone());
            last_cursor = Some((m.sent_at.clone(), m.id.clone()));
            msgs.push(m);
        }
    }
    if let Some(c) = last_cursor {
        *state.cursor.write_unchecked() = Some(c);
    }
}

/// Scroll-up backfill: prepend the page strictly older than `oldest`.
pub fn load_older(state: NativeState) {
    if *state.loading_older.peek() || !*state.more_history.peek() {
        return;
    }
    let Some(oldest) = state.oldest.peek().clone() else {
        return;
    };
    let Some(ch) = state.sel_channel.peek().clone() else {
        return;
    };
    *state.loading_older.write_unchecked() = true;
    let epoch = *state.epoch.peek();
    spawn(async move {
        if let Ok(l) = client().list_messages_before(&ch.id, &oldest).await {
            if *state.epoch.peek() == epoch {
                if l.messages.len() < MESSAGES_PAGE_LIMIT {
                    *state.more_history.write_unchecked() = false;
                }
                let fresh: Vec<MessageEnvelope> = l
                    .messages
                    .into_iter()
                    .filter(|m| !state.seen.peek().contains(&m.id))
                    .collect();
                if let Some(first) = fresh.first() {
                    *state.oldest.write_unchecked() =
                        Some((first.sent_at.clone(), first.id.clone()));
                }
                {
                    let mut seen = state.seen.write_unchecked();
                    for m in &fresh {
                        seen.insert(m.id.clone());
                    }
                }
                let mut msgs = state.messages.write_unchecked();
                let mut combined = fresh;
                combined.extend(msgs.drain(..));
                *msgs = combined;
            }
        }
        *state.loading_older.write_unchecked() = false;
    });
}

/// Send `body` to the open channel, wearing the channel's active persona (the
/// server re-validates `can_edit_persona` and falls back to the account).
/// Clears the composer, then pulls the new message in immediately (no poll wait).
pub fn send_message(state: NativeState, body: String) {
    let body = body.trim().to_string();
    let attachment_ids: Vec<String> = state
        .staged_attachments
        .peek()
        .iter()
        .map(|a| a.id.clone())
        .collect();
    // A message with attachments may have an empty body (protocol.rs); only bail
    // when there is truly nothing to send.
    if body.is_empty() && attachment_ids.is_empty() {
        return;
    }
    let Some(ch) = state.sel_channel.peek().clone() else {
        return;
    };
    *state.compose.write_unchecked() = String::new();
    let persona = state.active_persona.peek().clone();
    let epoch = *state.epoch.peek();
    spawn(async move {
        if client()
            .post_message(&ch.id, &body, attachment_ids, persona)
            .await
            .is_ok()
        {
            // Clear staged attachments only after the server accepted them.
            state.staged_attachments.write_unchecked().clear();
            let cursor = state.cursor.peek().clone();
            if let Ok(l) = client().list_messages(&ch.id, cursor.as_ref()).await {
                if *state.epoch.peek() == epoch {
                    ingest(state, l.messages);
                }
            }
        }
    });
}

/// Open the OS file picker (images only), upload each chosen file over the
/// authenticated session, and stage the returned media ids for the next send.
/// Runs in a `spawn` task so the winit/Skia event loop never blocks (rfd's sync
/// `FileDialog` would freeze the window — only `AsyncFileDialog` is safe here).
pub fn pick_and_stage_attachments(state: NativeState) {
    spawn(async move {
        let Some(files) = rfd::AsyncFileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
            .set_title("Attach images")
            .pick_files()
            .await
        else {
            return; // cancelled
        };
        for f in files {
            let name = f.file_name();
            let mime = mime_from_name(&name);
            let bytes = f.read().await;
            if bytes.is_empty() {
                continue;
            }
            match client()
                .upload_media(bytes.clone(), name, mime.clone())
                .await
            {
                Ok(id) => state
                    .staged_attachments
                    .write_unchecked()
                    .push(StagedAttachment {
                        id,
                        bytes: bytes::Bytes::from(bytes),
                        mime,
                    }),
                Err(e) => *state.status.write_unchecked() = format!("attach failed: {e}"),
            }
        }
    });
}

/// Drop a staged attachment (its media id stays on the server, just unreferenced).
pub fn remove_staged_attachment(state: NativeState, id: String) {
    state
        .staged_attachments
        .write_unchecked()
        .retain(|a| a.id != id);
}

/// Replace the trailing `:query` token in the composer with `:name: ` (the
/// chosen emoji shortcode). Native detects the token at the END of the compose
/// string (caret-at-end, the common typing case) rather than at an arbitrary
/// caret, since Freya's `Input` doesn't surface a caret offset.
pub fn apply_emoji(state: NativeState, name: &str) {
    let text = state.compose.peek().clone();
    if let Some((_, start)) = active_shortcode_token(&text) {
        let mut next = text[..start].to_string();
        next.push_str(&format!(":{name}: "));
        *state.compose.write_unchecked() = next;
    }
}

/// Find a trailing `:query` shortcode token at the end of `text` (caret-at-end).
/// Returns `(query, colon_byte_index)` — the query after the colon and the byte
/// offset of the `:`. Mirrors the web `active_shortcode_token` rules: the body
/// is `[a-z0-9_]+`, and the colon must not follow an alphanumeric (blocks
/// `12:30`, `http:smile`). Returns `None` when there's no active token.
pub fn active_shortcode_token(text: &str) -> Option<(String, usize)> {
    let b = text.as_bytes();
    let mut i = b.len();
    while i > 0 && (b[i - 1].is_ascii_lowercase() || b[i - 1].is_ascii_digit() || b[i - 1] == b'_')
    {
        i -= 1;
    }
    if i == b.len() || i == 0 || b[i - 1] != b':' {
        return None;
    }
    let colon = i - 1;
    if colon > 0 && b[colon - 1].is_ascii_alphanumeric() {
        return None;
    }
    Some((text[i..].to_string(), colon))
}

/// Custom guild emoji whose names start with `query` (case-insensitive), capped.
pub fn emoji_suggestions(state: NativeState, query: &str) -> Vec<crate::protocol::CustomEmoji> {
    let q = query.to_lowercase();
    state
        .guild_emoji
        .peek()
        .iter()
        .filter(|e| e.name.to_lowercase().starts_with(&q))
        .take(8)
        .cloned()
        .collect()
}

/// Infer an upload MIME from the file extension, matching the server's image
/// allowlist (`server/media.rs`). The server reads the multipart part's
/// Content-Type and re-validates, rejecting a spoofed extension with 415.
fn mime_from_name(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Edit one of your own messages; updates the row in place on success.
pub fn edit_message(state: NativeState, mid: String, body: String) {
    let body = body.trim().to_string();
    let Some(ch) = state.sel_channel.peek().clone() else {
        return;
    };
    *state.editing.write_unchecked() = None;
    if body.is_empty() {
        return;
    }
    spawn(async move {
        if client().edit_message(&ch.id, &mid, &body).await.is_ok() {
            let mut msgs = state.messages.write_unchecked();
            if let Some(m) = msgs.iter_mut().find(|m| m.id == mid) {
                m.body = body;
            }
        }
    });
}

/// Delete one of your own messages; removes the row on success.
pub fn delete_message(state: NativeState, mid: String) {
    let Some(ch) = state.sel_channel.peek().clone() else {
        return;
    };
    spawn(async move {
        if client().delete_message(&ch.id, &mid).await.is_ok() {
            state.messages.write_unchecked().retain(|m| m.id != mid);
            state.seen.write_unchecked().remove(&mid);
        }
    });
}

// ---------------------------------------------------------------------------
// Phase 4b confirm-action handlers — the destructive ops the modal dialogs
// dispatch. Kept here (not in the leaf modules) because `ui.rs`'s shared
// `modal_view` wiring calls them. Each refreshes the relevant list on success;
// the wardrobe / emoji-manager leaves extend these with their richer flows.
// ---------------------------------------------------------------------------

/// Delete a persona the caller owns, then refresh the wardrobe list. Takes the
/// persona off locally first if it was worn in the open channel (web parity).
pub fn delete_persona(state: NativeState, pid: String) {
    if state.active_persona.peek().as_deref() == Some(pid.as_str()) {
        *state.active_persona.write_unchecked() = None;
    }
    spawn(async move {
        if client().delete_persona(&pid).await.is_ok() {
            if let Ok(p) = client().list_personas().await {
                *state.personas.write_unchecked() = p.personas;
            }
        }
    });
}

/// Remove a gallery image from a persona, then reload the editor's gallery
/// buffer so the thumbnail strip updates.
pub fn remove_gallery_image(state: NativeState, pid: String, img_id: String) {
    spawn(async move {
        if client().remove_gallery_image(&pid, &img_id).await.is_ok() {
            if let Ok(d) = client().get_persona(&pid).await {
                *state.pe_gallery.write_unchecked() = d.gallery;
            }
        }
    });
}

/// Delete a custom emoji from a guild, then reload the guild's emoji list.
pub fn delete_guild_emoji(state: NativeState, gid: String, name: String) {
    spawn(async move {
        if client().delete_emoji(&gid, &name).await.is_ok() {
            if let Ok(r) = client().list_guild_emoji(&gid).await {
                *state.guild_emoji.write_unchecked() = r.emoji;
            }
        }
    });
}

/// Start the 1.5s poll loop (idempotent). Refreshes the open channel's new
/// messages; re-fetches the guild list every ~6s. Inlines its fetches so it
/// never nests a `spawn` inside this task.
pub fn start_poll(state: NativeState) {
    if *state.polling.peek() {
        return;
    }
    *state.polling.write_unchecked() = true;
    spawn_forever(async move {
        let mut tick: u32 = 0;
        loop {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            tick = tick.wrapping_add(1);

            if tick.is_multiple_of(4) {
                if let Ok(r) = client().list_guilds().await {
                    *state.guilds.write_unchecked() = r.guilds;
                }
            }

            let Some(ch) = state.sel_channel.peek().clone() else {
                continue;
            };
            let epoch = *state.epoch.peek();
            match client().list_messages(&ch.id, None).await {
                // Short page: the whole channel fits — ingest any unseen.
                Ok(l) if l.messages.len() < MESSAGES_PAGE_LIMIT => {
                    if *state.epoch.peek() != epoch {
                        continue;
                    }
                    *state.typing.write_unchecked() = l.typing;
                    ingest(state, l.messages);
                }
                // Long history: append only messages past the cursor.
                Ok(_) => {
                    let cursor = state.cursor.peek().clone();
                    if let Ok(l) = client().list_messages(&ch.id, cursor.as_ref()).await {
                        if *state.epoch.peek() != epoch {
                            continue;
                        }
                        *state.typing.write_unchecked() = l.typing;
                        ingest(state, l.messages);
                    }
                }
                Err(_) => {}
            }
        }
    });
}
