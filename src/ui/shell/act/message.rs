//! Message + chat-related actions. This is the largest of the act submodules:
//!
//! - Compose: [`send_message`], [`add_compose_attachment`],
//!   [`remove_compose_attachment`].
//! - Edits + deletes: [`edit_message`], [`delete_message`] (instant
//!   soft-delete, NO modal; the 6s undo toast rides the EXISTING
//!   POST `.../restore` — UX evolution #11), [`restore_deleted_message`],
//!   [`load_deleted_messages`].
//! - Background sync primitives: [`start_poll`] (the SSE fallback loop),
//!   [`refresh_open_channel`], [`refresh_lists`], [`refresh_unread`],
//!   [`sync_messages`], [`reconcile_newest_window`], [`ingest`], [`unseen`]
//!   — driven by [`super::sync`] (the SSE driver that owns `start_sync`).
//! - Three-cursor pagination: [`load_older`] using `cursor` / `oldest` /
//!   `last_seen` with the `seen` HashSet dedupe across all paths.
//! - Mute + last-seen marks: [`load_muted`], [`toggle_mute`],
//!   [`load_last_seen`], [`set_last_seen`].
//! - The destructive-action queue: [`ask_delete`], [`cancel_delete`],
//!   [`confirm_delete`] — `PendingDelete` is data, not a closure, dispatched
//!   by match here.
//! - Pane switchers + friends + lorebook + invites + deleted-channel loads —
//!   parked here rather than spawning a dedicated submodule.
//!
//! Cross-submodule calls: [`confirm_delete`] dispatches into
//! [`super::guild::delete_server`], [`super::channel::delete_channel`],
//! [`super::persona::remove_persona`], and `delete_message` here; the poll loop
//! calls [`super::notify::notify_messages`].

use super::super::{PendingDelete, Shell};

#[cfg(feature = "hydrate")]
use super::super::Pane;
#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use crate::protocol::MessageEnvelope;
// The pure newest-window reconcile core (review M-08) + its tests compile in
// the test graph too; complement the hydrate-gated import above.
#[cfg(all(not(feature = "hydrate"), test))]
use crate::protocol::MessageEnvelope;
#[cfg(feature = "hydrate")]
use gloo_storage::{LocalStorage, Storage};
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

/// Server page size for messages (mirrors `MESSAGES_PAGE_LIMIT` on the
/// server). Below this, the whole channel is loaded in one page and can be
/// reconciled wholesale; at/above it we only append.
#[cfg(feature = "hydrate")]
pub(super) const MESSAGES_PAGE_LIMIT: usize = 100;

// ---- compose ----

#[cfg(feature = "hydrate")]
pub fn send_message(s: Shell) {
    // Edit-in-composer: when an edit is staged, Send/Enter saves the edit
    // (PATCH) instead of posting a new message. Restore the stashed draft and
    // leave edit mode, then dispatch — an empty body keeps edit mode (no-op),
    // mirroring `edit_message`'s own empty guard.
    if let Some(e) = s.composer.editing.get_untracked() {
        let body = s.composer.compose.get_untracked();
        if body.trim().is_empty() {
            return;
        }
        s.composer.compose.set(e.stashed_draft);
        s.composer.editing.set(None);
        edit_message(s, e.cid, e.mid, body);
        return;
    }
    let Some(ch) = s.sel.sel_channel.get_untracked() else {
        return;
    };
    let body = s.composer.compose.get_untracked();
    // Fate Engine intercept (W4/T6): `/roll <expr>`, `/coin`, `/oracle` route
    // to the server-rolled endpoint instead of a normal send. The W4/T5
    // effect picker value is IGNORED for rolls (a roll has no effect), the
    // reply banner clears like any send, and staged attachments stay staged —
    // a roll never carries them, so clearing would silently discard uploads.
    if let Some(expr) = roll_command(body.trim()) {
        s.composer.replying_to.set(None);
        s.composer.effect_mode.set(None);
        s.composer.compose.set(String::new());
        super::channel::save_draft(s, "");
        s.composer.status.set(String::new());
        super::notify::request_notify_permission(s);
        // Same race-proof persona carry as a normal send; the server re-checks
        // can_edit_persona on it either way.
        let persona = s.social.active_persona.get_untracked();
        spawn_local(async move {
            match api::roll(&ch.id, &expr, persona).await {
                Ok(_) => after_send_success(s, &ch.id).await,
                // A 400 (bad expression / bounds) surfaces its server message
                // in the composer status line, like any failed send. (`try_`,
                // review M-10: post-await — logout may have disposed the
                // shell while the POST was in flight.)
                Err(e) => {
                    let _ = s.composer.status.try_set(api::humanize(&e));
                }
            }
        });
        return;
    }
    // The wire SEND request is ids-only; map the staged attachments down,
    // keeping only the ones whose upload has finished (`Ready`) — in-flight or
    // failed slots carry a placeholder id, not a real media id (F-8).
    let attachments: Vec<String> = s
        .composer
        .compose_attachments
        .get_untracked()
        .into_iter()
        .filter(|a| a.status == super::super::state::UploadStatus::Ready)
        .map(|a| a.att.id)
        .collect();
    // A message needs text OR at least one attachment.
    if body.trim().is_empty() && attachments.is_empty() {
        return;
    }
    // Capture + clear the reply target (L-3): the parent id rides as
    // `reply_to_id`, and the banner clears the moment we send.
    let reply_to_id = s.composer.replying_to.get_untracked().map(|r| r.id);
    s.composer.replying_to.set(None);
    // Capture + RESET the delivery effect (W4/T5): an effect is a per-message
    // flourish, not a sticky mode — the picker returns to "no effect" the
    // moment the send is dispatched.
    let effect = s.composer.effect_mode.get_untracked();
    s.composer.effect_mode.set(None);
    s.composer.compose.set(String::new());
    // Drop the now-sent channel's persisted draft (removes the key + persists).
    super::channel::save_draft(s, "");
    s.composer.compose_attachments.set(Vec::new());
    s.composer.status.set(String::new());
    // Sending is a user gesture — a reliable point to request notification
    // permission so background channels can notify later.
    super::notify::request_notify_permission(s);
    // Carry the persona worn in THIS channel so attribution is decided at
    // send time (race-proof) rather than depending on a separately-written
    // per-channel row having committed.
    let persona = s.social.active_persona.get_untracked();
    spawn_local(async move {
        match api::post_message(&ch.id, &body, attachments, persona, reply_to_id, effect).await {
            Ok(_) => after_send_success(s, &ch.id).await,
            // `try_` (review M-10): post-await — logout may have disposed the
            // shell while the POST was in flight.
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

/// Shared post-send success path (normal sends AND rolls): the Send-button
/// pulse plus the immediate catch-up fetch.
///
/// W4/T2: send pulse — flip `.sent` on the Send button for one fx-glow-pulse
/// cycle. Reset on a DETACHED timer so the post-send refresh round-trip below
/// doesn't stretch the pulse. Generation-guarded (the `LongPress` pattern,
/// channel/radial.rs): in a send burst an EARLIER timer firing mid-pulse would
/// otherwise truncate a LATER send's pulse — only the generation's own timer
/// may clear the flag.
#[cfg(feature = "hydrate")]
async fn after_send_success(s: Shell, cid: &str) {
    // Re-entry NEW divider (review): posting — a send OR a roll — means the
    // user has caught up, so the "where I left off" frontier is done
    // (Discord parity: the divider clears on send, not only on channel
    // switch). Guarded on still-being-in-the-channel so a slow POST resolving
    // after a switch can't wipe the INCOMING channel's freshly set divider.
    // `try_` (review M-10): this whole fn runs after the send POST's await —
    // logout may have disposed the shell meanwhile; one try_ read proves it
    // alive for the synchronous stretch below (single-threaded WASM).
    let Some(sel) = s.sel.sel_channel.try_get_untracked() else {
        return;
    };
    if sel.map(|c| c.id).as_deref() == Some(cid) {
        s.msg.new_divider.set(None);
    }
    let gen = s.composer.sent_gen.get_value().wrapping_add(1);
    s.composer.sent_gen.set_value(gen);
    s.composer.sent.set(true);
    // W5/P0 #19 Visual Haptics — first live consumer of the vocabulary: fire a
    // `vh-thud` (weighty land) on the Send button as the send commits. This is
    // REINFORCEMENT of the existing `.send.sent` glow pulse (above), not a
    // replacement; the haptic class removes itself on animationend so a burst
    // of sends re-fires. Resolve the button via the same `query_selector`
    // lookup this module already uses for `.composer textarea`.
    if let Some(el) = leptos::web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".composer .send").ok().flatten())
    {
        super::vh(&el, super::Vh::Thud);
    }
    spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(400).await;
        // Still the newest send? (try_*: the shell may have been
        // disposed while we slept.)
        if s.composer.sent_gen.try_get_value() != Some(gen) {
            return;
        }
        let _ = s.composer.sent.try_set(false);
    });
    let cur = s.msg.cursor.get_untracked();
    if let Ok(l) = api::list_messages(cid, cur.as_ref()).await {
        // Stale-guard (review M-06): the user may have switched channels
        // while the send POST + this catch-up fetch were in flight — the
        // divider clear above already guards this race, but `ingest` would
        // still append the OUTGOING channel's rows (the just-sent message
        // included) onto the INCOMING channel's list and clobber its
        // cursor/last-seen. Same discipline as `refresh_open_channel`
        // (feedback gwiif7xy) — and load-bearing under SSE, where no poll
        // tick reconciles a quiet channel afterwards. `try_` (review M-10):
        // a shell disposed by logout mid-fetch bails the same way.
        let open = s
            .sel
            .sel_channel
            .try_get_untracked()
            .flatten()
            .map(|c| c.id);
        if open.as_deref() != Some(cid) {
            return;
        }
        ingest(s, l.messages);
        // FB10a: advance this channel's last-seen to the new
        // cursor so `refresh_unread` doesn't glow the channel
        // for the user's OWN just-sent message.
        if let Some(cur) = s.msg.cursor.get_untracked() {
            set_last_seen(s, cid, cur);
        }
    }
}

/// Parse a composer body into a Fate Engine expression (W4/T6), or `None` for
/// a normal send. `/roll <expr>` forwards the rest verbatim (the SERVER owns
/// the grammar — an empty or bad tail surfaces its 400 in the status line);
/// `/coin` and `/oracle` match bare or with a trailing space (so `/coined` is
/// NOT a command). A bare `/roll` with no argument defaults to `1d20` (the
/// tabletop convention) instead of posting as literal text.
#[cfg(feature = "hydrate")]
fn roll_command(trimmed: &str) -> Option<String> {
    if trimmed == "/roll" {
        return Some("1d20".to_string());
    }
    if let Some(rest) = trimmed.strip_prefix("/roll ") {
        return Some(rest.trim().to_string());
    }
    for (cmd, expr) in [("/coin", "coin"), ("/oracle", "oracle")] {
        if trimmed == cmd || trimmed.starts_with(&format!("{cmd} ")) {
            return Some(expr.to_string());
        }
    }
    None
}

/// Upload a picked/pasted image or video and stage it as a pending composer
/// attachment (its media id is sent with the next message). The browser's
/// reported MIME (`file.type_()`) is kept locally so the pending thumbnail
/// renders image-vs-video correctly before the message round-trips.
///
/// F-8: the slot is inserted immediately in `Uploading` state and its progress
/// bar is driven from `upload_media_with_progress`; on success it flips to
/// `Ready` (real media id), on failure to `Failed` with a retry button.
#[cfg(feature = "hydrate")]
pub fn add_compose_attachment(s: Shell, file: web_sys::File) {
    s.composer.status.set(String::new());
    let key = next_stage_key();
    let mime = file.type_();
    stash_retry_file(key, file.clone());
    s.composer.compose_attachments.update(|v| {
        v.push(super::super::state::StagedAttachment {
            key,
            att: crate::protocol::Attachment {
                id: format!("pending-{key}"),
                mime,
            },
            status: super::super::state::UploadStatus::Uploading(Some(0.0)),
        });
    });
    spawn_local(async move { upload_staged(s, key, file).await });
}

/// Upload a batch of picked files and stage them **in pick order**. The slots
/// are inserted up front in `files` order so the row layout is stable; each
/// upload then drives its own slot's progress independently (F-8). Concurrency
/// is left to the browser's connection pool. Replaces the prior `join_all`
/// reorder fix (feedback mnjs2ljw…) — order is now guaranteed by inserting the
/// slots before any upload completes, not by awaiting in order.
#[cfg(feature = "hydrate")]
pub fn add_compose_attachments(s: Shell, files: Vec<web_sys::File>) {
    if files.is_empty() {
        return;
    }
    s.composer.status.set(String::new());
    // Insert all slots first (pick order), then kick off the uploads.
    let staged: Vec<(u64, web_sys::File)> = files
        .into_iter()
        .map(|f| {
            let key = next_stage_key();
            let mime = f.type_();
            stash_retry_file(key, f.clone());
            s.composer.compose_attachments.update(|v| {
                v.push(super::super::state::StagedAttachment {
                    key,
                    att: crate::protocol::Attachment {
                        id: format!("pending-{key}"),
                        mime,
                    },
                    status: super::super::state::UploadStatus::Uploading(Some(0.0)),
                });
            });
            (key, f)
        })
        .collect();
    for (key, f) in staged {
        spawn_local(async move { upload_staged(s, key, f).await });
    }
}

/// Run one staged attachment's upload, writing progress into its slot and
/// flipping it to `Ready`/`Failed` on completion (F-8). Shared by the single,
/// batch, and retry entry points.
#[cfg(feature = "hydrate")]
async fn upload_staged(s: Shell, key: u64, file: web_sys::File) {
    let result = api::upload_media_with_progress(&file, move |frac| {
        set_stage_status(
            s,
            key,
            super::super::state::UploadStatus::Uploading(Some(frac)),
        );
    })
    .await;
    match result {
        Ok(id) => {
            forget_retry_file(key);
            // `try_` (review M-10): the upload future can outlive the shell
            // (logout mid-upload) — no staging row left to flip.
            let _ = s.composer.compose_attachments.try_update(|v| {
                if let Some(it) = v.iter_mut().find(|it| it.key == key) {
                    it.att.id = id;
                    it.status = super::super::state::UploadStatus::Ready;
                }
            });
        }
        Err(e) => set_stage_status(
            s,
            key,
            super::super::state::UploadStatus::Failed(api::humanize(&e)),
        ),
    }
}

/// Re-attempt a failed staged upload using the file stashed at stage time
/// (F-8). No-op if the slot or file is gone (already removed).
#[cfg(feature = "hydrate")]
pub fn retry_compose_attachment(s: Shell, key: u64) {
    let Some(file) = peek_retry_file(key) else {
        return;
    };
    set_stage_status(
        s,
        key,
        super::super::state::UploadStatus::Uploading(Some(0.0)),
    );
    spawn_local(async move { upload_staged(s, key, file).await });
}

/// Write a staged attachment's status by key, if it still exists. `try_`
/// (review M-10): also reached from the upload future's progress callback
/// and failure tail, both of which can outlive the shell (logout
/// mid-upload) — disposed degrades to a no-op.
#[cfg(feature = "hydrate")]
fn set_stage_status(s: Shell, key: u64, status: super::super::state::UploadStatus) {
    let _ = s.composer.compose_attachments.try_update(|v| {
        if let Some(it) = v.iter_mut().find(|it| it.key == key) {
            it.status = status;
        }
    });
}

/// Drop one staged attachment before sending, addressed by its stage key
/// (stable across the upload lifecycle, unlike the late-arriving media id).
#[cfg(feature = "hydrate")]
pub fn remove_compose_attachment(s: Shell, key: u64) {
    forget_retry_file(key);
    s.composer
        .compose_attachments
        .update(|v| v.retain(|a| a.key != key));
}

// ---- staged-upload bookkeeping (hydrate-only, F-8) ----

#[cfg(feature = "hydrate")]
thread_local! {
    /// Monotonic counter minting unique stage keys per staged attachment.
    static STAGE_KEY: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Files held for retry, keyed by stage key. `web_sys::File` is `!Send` and
    /// hydrate-only, so it lives here rather than in the shared `Composer`
    /// state. Entries are dropped on success or removal.
    static RETRY_FILES: std::cell::RefCell<std::collections::HashMap<u64, web_sys::File>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

#[cfg(feature = "hydrate")]
fn next_stage_key() -> u64 {
    STAGE_KEY.with(|c| {
        let k = c.get().wrapping_add(1);
        c.set(k);
        k
    })
}

#[cfg(feature = "hydrate")]
fn stash_retry_file(key: u64, file: web_sys::File) {
    RETRY_FILES.with(|m| {
        m.borrow_mut().insert(key, file);
    });
}

#[cfg(feature = "hydrate")]
fn peek_retry_file(key: u64) -> Option<web_sys::File> {
    RETRY_FILES.with(|m| m.borrow().get(&key).cloned())
}

#[cfg(feature = "hydrate")]
fn forget_retry_file(key: u64) {
    RETRY_FILES.with(|m| {
        m.borrow_mut().remove(&key);
    });
}

/// The composer reply banner's parent snippet: the first 100 chars of the
/// body — except a whispered parent, which shows the fixed `(whisper)`
/// placeholder instead (review M-27). The banner is a body-preview surface,
/// so the W4 whisper-mask invariant applies: it must match what the
/// persisted quote will show (`MSG_PROJECTION`'s mask,
/// `server/messages/reading.rs`), never leak the still-veiled spoiler text
/// through the reply button. Pure; unit-tested below.
#[cfg(any(feature = "hydrate", test))]
fn reply_banner_snippet(body: &str, effect: Option<&str>) -> String {
    if effect == Some("whisper") {
        return "(whisper)".to_string();
    }
    body.chars().take(100).collect()
}

/// Begin replying to message `m` (L-3): stash a [`ReplyPreview`] built from the
/// row so the composer banner shows the parent author + snippet, and so the
/// next send carries `reply_to_id`. Single-level — replying to a reply quotes
/// THAT message, never its own parent. Focuses the composer for a fast reply.
#[cfg(feature = "hydrate")]
pub fn start_reply(s: Shell, m: MessageEnvelope) {
    let who = m
        .persona_name
        .clone()
        .unwrap_or_else(|| m.author_display.clone());
    let snippet: String = reply_banner_snippet(&m.body, m.effect.as_deref());
    s.composer
        .replying_to
        .set(Some(crate::protocol::ReplyPreview {
            id: m.id,
            author_display: who,
            body_snippet: snippet,
        }));
    if let Some(el) = leptos::web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".composer textarea").ok().flatten())
    {
        use wasm_bindgen::JsCast;
        if let Ok(input) = el.dyn_into::<leptos::web_sys::HtmlElement>() {
            let _ = input.focus();
        }
    }
}

/// Clear the active reply target (the banner's ✕), reverting the next send to a
/// normal non-reply message.
#[cfg(feature = "hydrate")]
pub fn cancel_reply(s: Shell) {
    s.composer.replying_to.set(None);
}

/// Begin editing message `mid` (own message) in the main composer: stash the
/// current draft, load the message body into the compose box, and enter edit
/// mode so the Send button becomes "Save" and dispatches an edit. Focuses the
/// composer. The stashed draft is restored on save or cancel. While editing,
/// `save_draft` is a no-op so the edit text never clobbers the channel draft.
#[cfg(feature = "hydrate")]
pub fn start_edit(s: Shell, cid: String, mid: String, body: String) {
    let stashed_draft = s.composer.compose.get_untracked();
    s.composer
        .editing
        .set(Some(crate::ui::shell::state::EditingMessage {
            cid,
            mid,
            stashed_draft,
        }));
    s.composer.compose.set(body);
    s.composer.status.set(String::new());
    // Reuse the reply affordance's focus path so the user can type immediately.
    if let Some(el) = leptos::web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".composer textarea").ok().flatten())
    {
        use wasm_bindgen::JsCast;
        if let Ok(input) = el.dyn_into::<leptos::web_sys::HtmlElement>() {
            let _ = input.focus();
        }
    }
}

/// Cancel an in-progress composer edit (the banner's ✕ or Esc): restore the
/// stashed draft and leave edit mode without touching the message.
#[cfg(feature = "hydrate")]
pub fn cancel_edit(s: Shell) {
    if let Some(e) = s.composer.editing.get_untracked() {
        s.composer.compose.set(e.stashed_draft);
        s.composer.editing.set(None);
    }
}

/// Copy a message body to the clipboard as raw markup, **stripping color
/// tokens** so the receiver can re-paste it under their own persona without
/// dragging the original speaker's palette along. Foxtrot feedback row
/// 3szov1qgatobhhrc3mf2 / ctx 019e6f23-fcfc.
///
/// `navigator.clipboard.writeText` is async and reached via reflection so we
/// don't pull the `Clipboard` web-sys feature flag just for this. Success
/// rides the toast primitive in success styling ("Copied" — UX evolution
/// #11's status-line absorption); failures stay on the red status `<p>`,
/// which is for errors only.
#[cfg(feature = "hydrate")]
pub fn copy_message_body(s: Shell, body: String) {
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    let stripped = crate::markup::strip_color_tokens(&body);
    s.composer.status.set(String::new());
    spawn_local(async move {
        let promise = (|| -> Option<js_sys::Promise> {
            let win = leptos::web_sys::window()?;
            let nav = js_sys::Reflect::get(&win, &JsValue::from_str("navigator")).ok()?;
            let clip = js_sys::Reflect::get(&nav, &JsValue::from_str("clipboard")).ok()?;
            let write_fn = js_sys::Reflect::get(&clip, &JsValue::from_str("writeText")).ok()?;
            let func: js_sys::Function = write_fn.dyn_into().ok()?;
            let arg = JsValue::from_str(&stripped);
            func.call1(&clip, &arg).ok()?.dyn_into().ok()
        })();
        match promise {
            Some(p) => match JsFuture::from(p).await {
                // Post-await tails (review M-10): the toast funnel writes
                // `toasts.current` plainly, so prove the shell alive before
                // pushing; the failure line degrades to a no-op. The `None`
                // arm below runs BEFORE the spawned future's first await and
                // stays plain.
                Ok(_) => {
                    if s.sync.polling.try_get_untracked().is_none() {
                        return;
                    }
                    super::toast::show_success_toast(s, "Copied".to_string())
                }
                Err(_) => {
                    let _ = s
                        .composer
                        .status
                        .try_set("Couldn't copy — check clipboard permission".to_string());
                }
            },
            None => s.composer.status.set("Clipboard unavailable".to_string()),
        }
    });
}

// ---- edit / delete / restore ----

/// Edit one of the caller's own messages, then patch `s.msg.messages` in
/// place. `ingest` only appends (dedupes by id), so an edit needs a direct
/// in-place body update — the row's id and cursor position don't change.
#[cfg(feature = "hydrate")]
pub fn edit_message(s: Shell, cid: String, mid: String, body: String) {
    let body = body.trim_end().to_string();
    if body.trim().is_empty() {
        return;
    }
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::edit_message(&cid, &mid, &body).await {
            // `try_` both arms (review M-10): the PATCH may resolve after
            // logout disposed the shell — nothing left to patch or report.
            Ok(()) => {
                let _ = s.msg.messages.try_update(|v| {
                    if let Some(m) = v.iter_mut().find(|m| m.id == mid) {
                        m.body = body.clone();
                    }
                });
            }
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

/// Lifetime of the post-delete undo toast (UX evolution #11). Pure UI grace —
/// the soft-delete is ALREADY committed server-side when the toast appears,
/// so a tab close / PWA kill mid-toast loses nothing: the delete stands, and
/// the row stays restorable from the channel trash until the 1h purge.
#[cfg(feature = "hydrate")]
const UNDO_TOAST_MS: u32 = 6000;

/// Delete one of the caller's own messages — instantly, with NO modal and a
/// 6s regret window (UX evolution #11): the row hides optimistically, the
/// real DELETE fires AT ONCE (it is a soft-delete server-side, review
/// blocker fix — a client-delayed DELETE inverts the failure mode: a tab
/// close mid-grace would silently cancel a user-intended delete and every
/// other member would keep seeing the row), and on success the undo toast
/// offers the EXISTING POST `.../restore` ([`undo_message_delete`]).
///
/// `s.msg.seen` keeps the id while the DELETE is in flight so a racing
/// catch-up fetch can't re-append the row; success removes it (the server no
/// longer returns the row), failure resurfaces the envelope untouched plus
/// an honest error toast. `s.msg.cursor` is left as-is — the composite
/// cursor is a value, not a row reference, so it stays valid after the row
/// is gone and deleting never rewinds the catch-up window.
///
/// Only `kind='user'` rows owned by the viewer ever reach here — both delete
/// affordances flow from the shared `message_actions` predicate
/// (`ui/shell/channel/mod.rs`), so immutable kinds (`roll`, `system`) have
/// no delete button at all; the server 403s them regardless.
#[cfg(feature = "hydrate")]
pub fn delete_message(s: Shell, cid: String, mid: String) {
    s.composer.status.set(String::new());
    // Snapshot the row for the optimistic hide + the toast's Undo reinsert.
    // Reinsert position is derived from its composite `(sent_at, id)` cursor
    // — the list's sort order — not a stored index (an older-history prepend
    // while the toast drains would shift indices).
    let envelope = s
        .msg
        .messages
        .with_untracked(|v| v.iter().find(|m| m.id == mid).cloned());
    if envelope.is_some() {
        s.msg.messages.update(|v| v.retain(|m| m.id != mid));
    }
    spawn_local(async move {
        match api::delete_message(&cid, &mid).await {
            Ok(()) => {
                // `try_update` doubles as the disposal PROOF (review M-10):
                // `None` back means logout disposed the shell while the
                // DELETE was in flight — the soft-delete stands server-side
                // and the toast funnel writes `toasts.current` plainly, so
                // bail instead of pushing.
                if s.msg
                    .seen
                    .try_update(|h| {
                        h.remove(&mid);
                    })
                    .is_none()
                {
                    return;
                }
                // No envelope (row wasn't in the visible list — shouldn't
                // happen, the affordances live on visible rows): nothing to
                // resurface on Undo, so skip the toast; the trash pane can
                // still restore until the purge.
                if let Some(envelope) = envelope {
                    super::toast::show_undo_delete_toast(s, cid, mid, envelope, UNDO_TOAST_MS);
                }
            }
            Err(e) => {
                let msg = api::humanize(&e);
                if let Some(envelope) = envelope {
                    resurface(s, &cid, envelope);
                }
                // Disposal proof before the toast (review M-10): `push`
                // writes `toasts.current` plainly.
                if s.sync.polling.try_get_untracked().is_none() {
                    return;
                }
                super::toast::show_error_toast(s, format!("Couldn't delete — {msg}"));
            }
        }
    });
}

/// Put an optimistically-hidden row back, untouched: re-add its id to `seen`
/// (a full-page reconcile may have rebuilt the set without it — without
/// this, `ingest` could append a duplicate) and reinsert the envelope in
/// composite-cursor order — the list is sorted ASC by the strict
/// `(sent_at, id)` tie-break, and `sent_at` is the fixed-digit lex-monotonic
/// shape, so the String-tuple `partition_point` lands the row exactly where
/// it was (skipped if a reconcile already brought it back). Only when the
/// open channel still is the row's channel — elsewhere the next channel
/// open's fetch shows the true server state anyway. `try_*` throughout: the
/// detached restore/delete futures can outlive the shell (logout mid-toast).
#[cfg(feature = "hydrate")]
fn resurface(s: Shell, cid: &str, envelope: MessageEnvelope) {
    let open = s
        .sel
        .sel_channel
        .try_get_untracked()
        .flatten()
        .map(|c| c.id);
    if open.as_deref() != Some(cid) {
        return;
    }
    let _ = s.msg.seen.try_update(|h| {
        h.insert(envelope.id.clone());
    });
    let _ = s.msg.messages.try_update(|v| {
        if v.iter().any(|m| m.id == envelope.id) {
            return;
        }
        let at = v.partition_point(|m| {
            (m.sent_at.as_str(), m.id.as_str()) < (envelope.sent_at.as_str(), envelope.id.as_str())
        });
        v.insert(at, envelope);
    });
}

/// Undo a just-committed delete (the toast's Undo): POST the EXISTING
/// own-gated `/channels/{cid}/messages/{mid}/restore`
/// (`server/messages/editing.rs` — SSE-notified as a new arrival, which
/// other clients pick up via [`sync_messages`] or the newest-window
/// reconcile as long as the row sits on their newest page; deeper paged-in
/// history re-syncs on their next channel open), then resurface the
/// snapshot envelope in place. The
/// resurface no-ops if the user navigated away meanwhile — the restored row
/// reappears via the next channel open's fetch. On failure (the channel was
/// deleted meanwhile, the 1h purge won, network): honest-state error toast;
/// the row stays deleted.
#[cfg(feature = "hydrate")]
pub(super) fn undo_message_delete(s: Shell, cid: String, mid: String, envelope: MessageEnvelope) {
    spawn_local(async move {
        match api::restore_message(&cid, &mid).await {
            Ok(()) => resurface(s, &cid, envelope),
            Err(e) => {
                // Disposal proof before the toast (review M-10): `push`
                // writes `toasts.current` plainly — a disposed shell has no
                // toast surface left.
                if s.sync.polling.try_get_untracked().is_none() {
                    return;
                }
                super::toast::show_error_toast(
                    s,
                    format!("Couldn't restore — {}", api::humanize(&e)),
                )
            }
        }
    });
}

/// Load soft-deleted messages for the given channel into `s.trash.deleted_messages`.
#[cfg(feature = "hydrate")]
pub fn load_deleted_messages(s: Shell, cid: String) {
    spawn_local(async move {
        match api::list_deleted_messages(&cid).await {
            // `try_` both arms (review M-10): post-await — a disposed shell
            // degrades to a no-op.
            Ok(r) => {
                let _ = s.trash.deleted_messages.try_set(r.messages);
            }
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

/// Restore one of the caller's own deleted messages. On success, remove it
/// from the trash list and reload the channel messages.
#[cfg(feature = "hydrate")]
pub fn restore_deleted_message(s: Shell, cid: String, mid: String) {
    spawn_local(async move {
        match api::restore_message(&cid, &mid).await {
            Ok(()) => {
                // Drop from the trash list immediately (no re-load needed).
                // `try_update` doubles as the disposal proof (review M-10):
                // the POST may resolve after logout disposed the shell.
                if s.trash
                    .deleted_messages
                    .try_update(|v| v.retain(|m| m.id != mid))
                    .is_none()
                {
                    return;
                }
                // Reload channel messages so the restored one reappears.
                if let Ok(l) = api::list_messages(&cid, None).await {
                    // Re-prove after the second await (review M-10); the
                    // remaining writes ride the same tick.
                    if s.msg.messages.try_set(l.messages.clone()).is_some() {
                        return;
                    }
                    s.msg.seen.update(|h| {
                        h.clear();
                        for m in &l.messages {
                            h.insert(m.id.clone());
                        }
                    });
                    s.msg
                        .cursor
                        .set(l.messages.last().map(|m| (m.sent_at.clone(), m.id.clone())));
                }
            }
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

/// Load soft-deleted channels for the given guild into `s.trash.deleted_channels`.
#[cfg(feature = "hydrate")]
pub fn load_deleted_channels(s: Shell, gid: String) {
    spawn_local(async move {
        match api::list_deleted_channels(&gid).await {
            // `try_` both arms (review M-10): post-await — a disposed shell
            // degrades to a no-op.
            Ok(r) => {
                let _ = s.trash.deleted_channels.try_set(r.channels);
            }
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

// ---- destructive-action confirmation ----

/// Queue a destructive action behind the top-level confirm modal: stash the
/// action plus its human prompt. The modal dispatches it via `confirm_delete`.
#[cfg(feature = "hydrate")]
pub fn ask_delete(s: Shell, prompt: String, pending: PendingDelete) {
    s.modals.confirm_prompt.set(Some(prompt));
    s.modals.pending_delete.set(Some(pending));
}

/// Clear a pending confirm without acting (Cancel / backdrop).
#[cfg(feature = "hydrate")]
pub fn cancel_delete(s: Shell) {
    s.modals.pending_delete.set(None);
    s.modals.confirm_prompt.set(None);
}

/// Run the pending destructive action (the modal's "Delete"), then clear it.
/// Message deletes no longer route here (instant + undo toast, UX evolution
/// #11) — the modal covers the heavier channel/server/persona deletes only.
#[cfg(feature = "hydrate")]
pub fn confirm_delete(s: Shell) {
    let pending = s.modals.pending_delete.get_untracked();
    cancel_delete(s);
    match pending {
        Some(PendingDelete::Channel { gid, cid }) => super::channel::delete_channel(s, gid, cid),
        Some(PendingDelete::Server { gid }) => super::guild::delete_server(s, gid),
        Some(PendingDelete::Persona { pid }) => super::persona::remove_persona(s, pid),
        None => {}
    }
}

// ---- pane switchers ----

#[cfg(feature = "hydrate")]
pub fn show_friends(s: Shell) {
    // Re-entry scroll memory (review M-36): leaving the channel via the
    // bottom tabs unmounts ChannelPane just like a channel switch does —
    // capture the reading position FIRST, while the DOM still shows it
    // (no-op when no message list is mounted). `show_current_channel`
    // consumes the mark on the way back.
    super::reentry::capture_scroll_mark(s);
    s.sync.pane.set(Pane::Friends);
    reload_friends(s);
}

/// Open the wardrobe as a dismissible modal popup (F-2) and refresh the
/// persona list. The wardrobe is no longer a full pane — it overlays the
/// current view via `wardrobe_open` and closes on backdrop click / Esc / X.
#[cfg(feature = "hydrate")]
pub fn show_wardrobe(s: Shell) {
    s.sync.wardrobe_open.set(true);
    spawn_local(async move {
        if let Ok(r) = api::list_personas().await {
            // `try_` (review M-10): post-await — the fetch may outlive the
            // shell.
            let _ = s.social.personas.try_set(r.personas);
        }
    });
}

/// Open the per-guild custom-emoji manager. The list is already kept fresh
/// in `s.sel.guild_emoji` (loaded when the guild opens, refreshed on each
/// create/delete), so this only flips the pane.
#[cfg(feature = "hydrate")]
pub fn show_emoji_manager(s: Shell) {
    // Re-entry scroll memory (review M-36): same ChannelPane-unmounting
    // transition as `show_friends` — capture before the pane flips.
    super::reentry::capture_scroll_mark(s);
    s.sync.pane.set(Pane::Emoji);
}

/// Open the member-management pane. The roster is local to the pane and
/// fetched there on mount (an Effect keyed on the selected guild), so this
/// only flips the pane.
#[cfg(feature = "hydrate")]
pub fn show_members(s: Shell) {
    // Re-entry scroll memory (review M-36): same ChannelPane-unmounting
    // transition as `show_friends` — capture before the pane flips.
    super::reentry::capture_scroll_mark(s);
    s.sync.pane.set(Pane::Members);
}

/// Open the DM thread list directly (B3 — owner deck-finding 2026-06-20: DMs
/// were reachable only via Friends). Mirrors `show_friends`: capture the
/// channel scroll position (this unmounts ChannelPane), flip the pane, and
/// refresh the thread list. `DirectMessagesPane` also self-refreshes on mount,
/// so this is belt-and-suspenders if the pane is already shown.
#[cfg(feature = "hydrate")]
pub fn show_dms(s: Shell) {
    super::reentry::capture_scroll_mark(s);
    s.sync.pane.set(Pane::DirectMessages);
    super::dm::refresh_dms(s);
}

// ---- friends + member ops ----

#[cfg(feature = "hydrate")]
pub fn add_friend(s: Shell, username: String) {
    if username.trim().is_empty() {
        return;
    }
    spawn_local(async move {
        match api::add_friend(&username).await {
            // `reload_friends` only spawns (no synchronous signal access) and
            // its own tail is `try_`-guarded, so it is disposal-safe to call.
            Ok(()) => reload_friends(s),
            // `try_` (review M-10): post-await — no status line left after a
            // logout mid-flight.
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

#[cfg(feature = "hydrate")]
pub fn invite_member(s: Shell, gid: String, username: String) {
    let username = username.trim().to_string();
    if username.is_empty() {
        return;
    }
    spawn_local(async move {
        match api::invite_member(&gid, &username).await {
            // Success styling on the toast primitive (UX evolution #11's
            // status-line absorption); the red status <p> stays errors-only.
            // Disposal proof first (review M-10): `push` writes
            // `toasts.current` plainly.
            Ok(()) => {
                if s.sync.polling.try_get_untracked().is_none() {
                    return;
                }
                super::toast::show_success_toast(s, format!("invited {username}"))
            }
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

#[cfg(feature = "hydrate")]
pub fn accept_friend(s: Shell, aid: String) {
    spawn_local(async move {
        let _ = api::accept_friend(&aid).await;
        reload_friends(s);
    });
}

#[cfg(feature = "hydrate")]
pub fn remove_friend(s: Shell, aid: String) {
    spawn_local(async move {
        let _ = api::remove_friend(&aid).await;
        reload_friends(s);
    });
}

// ---- lorebook ----

#[cfg(feature = "hydrate")]
pub fn create_lore(s: Shell, cid: String, keys: Vec<String>, content: String) {
    if cid.is_empty() || content.trim().is_empty() {
        return;
    }
    spawn_local(async move {
        match api::create_lore(&cid, keys, &content).await {
            // `load_lore` only spawns (disposal-safe); the status line gets
            // the `try_` treatment (review M-10).
            Ok(_) => load_lore(s, cid),
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

#[cfg(feature = "hydrate")]
#[allow(clippy::too_many_arguments)]
pub fn patch_lore(
    s: Shell,
    cid: String,
    eid: String,
    title: Option<String>,
    keys: Option<Vec<String>>,
    content: Option<String>,
    enabled: Option<bool>,
    position: Option<i64>,
) {
    use crate::protocol::PatchLorebookEntryRequest;
    spawn_local(async move {
        let req = PatchLorebookEntryRequest {
            title,
            keys,
            content,
            enabled,
            position,
        };
        match api::patch_lore(&cid, &eid, &req).await {
            // Same disposal shape as `create_lore` (review M-10).
            Ok(()) => load_lore(s, cid),
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

/// Swap `eid` with the neighbor above (`up = true`) or below (`up = false`)
/// by exchanging their `position` values, then reload the list.
#[cfg(feature = "hydrate")]
pub fn swap_lore(s: Shell, cid: String, eid: String, position: i64, up: bool) {
    use crate::protocol::PatchLorebookEntryRequest;
    let entries = s.social.lore.get_untracked();
    let neighbor = if up {
        entries
            .iter()
            .filter(|e| e.position < position)
            .max_by_key(|e| e.position)
            .cloned()
    } else {
        entries
            .iter()
            .filter(|e| e.position > position)
            .min_by_key(|e| e.position)
            .cloned()
    };
    let Some(nbr) = neighbor else { return };
    let nbr_pos = nbr.position;
    let nbr_id = nbr.id.clone();
    let cid2 = cid.clone();
    spawn_local(async move {
        let r1 = api::patch_lore(
            &cid,
            &eid,
            &PatchLorebookEntryRequest {
                position: Some(nbr_pos),
                ..Default::default()
            },
        )
        .await;
        let r2 = api::patch_lore(
            &cid2,
            &nbr_id,
            &PatchLorebookEntryRequest {
                position: Some(position),
                ..Default::default()
            },
        )
        .await;
        match (r1, r2) {
            // Same disposal shape as `create_lore` (review M-10).
            (Ok(()), Ok(())) => load_lore(s, cid),
            (Err(e), _) | (_, Err(e)) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

/// Move the lore entry at `idx` to absolute index `target` (the grip-drag drop
/// target), then renumber the list to its array index and PATCH every entry
/// whose stored `position` changed — one PATCH per moved row, over the same
/// absolute-position contract `patch_lore` uses. Renumber-and-persist mirrors
/// the wardrobe `move_persona` flow; no-op when `idx == target` or either is
/// out of range. The server re-derives authorization per PATCH.
#[cfg(feature = "hydrate")]
pub fn move_lore(s: Shell, idx: usize, target: usize) {
    use crate::protocol::PatchLorebookEntryRequest;
    let Some(cid) = s.sel.sel_channel.get_untracked().map(|c| c.id) else {
        return;
    };
    let mut list = s.social.lore.get_untracked();
    if idx >= list.len() || target >= list.len() || idx == target {
        return;
    }
    let item = list.remove(idx);
    list.insert(target, item);
    // Optimistic local reorder, then PATCH only the rows whose stored position
    // drifted from their new array index (lore `position` is a non-option i64).
    s.social.lore.set(list.clone());
    let patches: Vec<(String, i64)> = list
        .iter()
        .enumerate()
        .filter(|(i, e)| e.position != *i as i64)
        .map(|(i, e)| (e.id.clone(), i as i64))
        .collect();
    if patches.is_empty() {
        return;
    }
    spawn_local(async move {
        for (eid, pos) in patches {
            if let Err(e) = api::patch_lore(
                &cid,
                &eid,
                &PatchLorebookEntryRequest {
                    position: Some(pos),
                    ..Default::default()
                },
            )
            .await
            {
                let _ = s.composer.status.try_set(api::humanize(&e));
                break;
            }
        }
        load_lore(s, cid);
    });
}

#[cfg(feature = "hydrate")]
pub fn delete_lore(s: Shell, cid: String, eid: String) {
    spawn_local(async move {
        let _ = api::delete_lore(&cid, &eid).await;
        load_lore(s, cid);
    });
}

// ---- internal: friend + lore reloads, message dedupe ingest ----

#[cfg(feature = "hydrate")]
fn reload_friends(s: Shell) {
    spawn_local(async move {
        if let Ok(f) = api::list_friends().await {
            // `try_` (review M-10): post-await — the fetch may outlive the
            // shell.
            let _ = s.social.friends.try_set(f);
        }
    });
}

#[cfg(feature = "hydrate")]
pub(super) fn load_lore(s: Shell, cid: String) {
    spawn_local(async move {
        if let Ok(l) = api::list_lore(&cid).await {
            // `try_` (review M-10): post-await — the fetch may outlive the
            // shell.
            let _ = s.social.lore.try_set(l.entries);
        }
    });
}

/// Append messages new since the last call, deduped via `s.msg.seen`, and advance
/// `s.msg.cursor` to the latest of them. Used by the initial channel open + the
/// poll's catch-up branch + the post-send fetch. Public to siblings.
#[cfg(feature = "hydrate")]
pub(super) fn ingest(s: Shell, incoming: Vec<MessageEnvelope>) {
    for m in incoming {
        if s.msg.seen.with_untracked(|h| h.contains(&m.id)) {
            continue;
        }
        s.msg.seen.update(|h| {
            h.insert(m.id.clone());
        });
        s.msg.cursor.set(Some((m.sent_at.clone(), m.id.clone())));
        s.msg.messages.update(|v| v.push(m));
    }
}

// ---- muted channels + per-channel last-seen marks ----

// localStorage key for the muted-channel id list.
#[cfg(feature = "hydrate")]
const KEY_MUTED: &str = "authlyn.muted_channels";

/// Load muted channels from localStorage into the reactive set (on mount).
#[cfg(feature = "hydrate")]
pub fn load_muted(s: Shell) {
    let ids = LocalStorage::get::<Vec<String>>(KEY_MUTED).unwrap_or_default();
    s.notify.muted.set(ids.into_iter().collect());
}

/// localStorage key for the per-channel last-seen high-water marks (#23).
#[cfg(feature = "hydrate")]
const KEY_LAST_SEEN: &str = "authlyn.last_seen";

/// Load last-seen marks from localStorage into the reactive map (on mount).
/// This is the OFFLINE fallback; [`hydrate_last_seen`] overlays the
/// server-synced cursors on top when the network fetch succeeds.
#[cfg(feature = "hydrate")]
pub fn load_last_seen(s: Shell) {
    s.notify
        .last_seen
        .set(LocalStorage::get(KEY_LAST_SEEN).unwrap_or_default());
}

/// Hydrate `notify.last_seen` from the SERVER's stored per-channel read cursors
/// (L-1 cross-device sync), so a second device knows what was already read.
/// Runs on shell mount AFTER [`load_last_seen`]: a server cursor wins over the
/// localStorage one only when it is strictly NEWER (the same MAX-cursor rule the
/// server enforces on write), so a stale server row can't rewind a fresher local
/// mark and vice-versa. On fetch failure we keep the localStorage values as the
/// offline fallback. The merged map is written back to localStorage.
#[cfg(feature = "hydrate")]
pub fn hydrate_last_seen(s: Shell) {
    spawn_local(async move {
        let Ok(r) = api::read_state().await else {
            return; // offline — keep the localStorage fallback already loaded.
        };
        let mut changed = false;
        // `try_update` doubles as the disposal proof (review M-10): the
        // fetch can resolve after logout disposed the shell; the same-tick
        // reads below ride this proof.
        let merged = s.notify.last_seen.try_update(|m| {
            for c in r.cursors {
                let incoming = (c.sent_at, c.id);
                let newer = match m.get(&c.channel_id) {
                    // Composite (sent_at, id) compare — String tuple ordering
                    // matches the cursor's strict tie-break (sent_at is the
                    // fixed-9-digit lex-monotonic shape, so this is correct).
                    Some(local) => incoming > *local,
                    None => true,
                };
                if newer {
                    m.insert(c.channel_id, incoming);
                    changed = true;
                }
            }
        });
        if merged.is_none() {
            return;
        }
        if changed {
            let _ = LocalStorage::set(KEY_LAST_SEEN, s.notify.last_seen.get_untracked());
        }
    });
}

/// Record `cur = (sent_at, id)` as the last message seen in `cid`, persist the
/// whole map to localStorage, AND push the mark to the server so read state
/// syncs across devices (L-1) — but ONLY when the cursor actually advanced for
/// this channel, so an idle re-mark (the poll re-asserting the open channel each
/// tick) doesn't spam the endpoint. The server itself also keeps the MAX cursor,
/// so a racing older POST is harmless. Idempotent. Public to siblings.
#[cfg(feature = "hydrate")]
pub(super) fn set_last_seen(s: Shell, cid: &str, cur: (String, String)) {
    // Visibility gate (review M-04): a hidden tab is not a reader. Under SSE
    // a background tab keeps receiving events at full network rate (unlike
    // the poll-era setTimeout, which browsers throttle/freeze when hidden),
    // so marking here would wipe the unread glow on every other device for
    // messages no human saw — the same cross-device class W3 ruled a bug for
    // the sheet flow (reentry.rs's "standing warning"). Skip the local map
    // AND the server POST; the foregrounding wake() pass re-marks via
    // `refresh_unread`'s open-channel prelude the moment the tab is actually
    // visible.
    if super::sync::document_hidden() {
        return;
    }
    let advanced = s
        .notify
        .last_seen
        .with_untracked(|m| m.get(cid).map(|prev| cur > *prev).unwrap_or(true));
    s.notify.last_seen.update(|m| {
        m.insert(cid.to_string(), cur.clone());
    });
    let _ = LocalStorage::set(KEY_LAST_SEEN, s.notify.last_seen.get_untracked());
    if advanced {
        let cid = cid.to_string();
        let (sent_at, id) = cur;
        spawn_local(async move {
            // Fire-and-forget: localStorage is the offline source of truth, the
            // server POST is best-effort cross-device sync.
            let _ = api::mark_read(&cid, &sent_at, &id).await;
        });
    }
}

/// Recompute the unread set across EVERY guild the caller belongs to (#23).
/// The open channel is always considered seen (advance its mark to the live
/// cursor); every other text channel is "unread" iff the batched `GET /unread`
/// summary (W1) says it has messages past the caller's read cursor. A channel
/// this client has never seen is baselined to its current latest (no
/// retroactive glow on first sight). One round-trip total — replaces the old
/// per-channel `list_messages` probe loop.
///
/// Covering all guilds (not only the open one) is what lets the rail glow a
/// guild button whose unread is in a non-open guild — feedback row
/// grt9ohmw8pj2fi4eqb6h; `notify.unread_guilds` is rebuilt fresh from each
/// row's `guild_id` (the per-guild channel cache is lazy now, so it can no
/// longer derive that mapping for never-opened guilds).
#[cfg(feature = "hydrate")]
pub(super) fn refresh_unread(s: Shell) {
    let open = s.sel.sel_channel.get_untracked().map(|c| c.id);
    if let Some(ref oc) = open {
        if let Some(cur) = s.msg.cursor.get_untracked() {
            // `set_last_seen` self-gates on document visibility (review
            // M-04): from a hidden tab this is a no-op, and THIS call —
            // re-reached from wake()'s refresh on foregrounding — is what
            // catches the deferred read-mark back up.
            set_last_seen(s, oc, cur);
        }
        // The open channel is always considered seen: clear its unread glow,
        // ping glow, and count at once (L-4).
        s.notify.unread.update(|u| {
            u.remove(oc);
        });
        s.notify.pinged.update(|p| {
            p.remove(oc);
        });
        s.notify.unread_count.update(|c| {
            c.remove(oc);
        });
    }
    spawn_local(async move {
        let Ok(r) = api::get_unread().await else {
            return;
        };
        // Disposal guard (review M-10): the fetch can resolve after logout
        // disposed the shell — one try_ read proves it alive for the whole
        // synchronous reconcile below (single-threaded WASM).
        if s.sync.polling.try_get_untracked().is_none() {
            return;
        }
        let mut unread_guilds: std::collections::HashSet<String> = std::collections::HashSet::new();
        for row in r.channels {
            if Some(&row.channel_id) == open.as_ref() {
                continue;
            }
            // First sight: this client has no last-seen mark for the channel —
            // baseline it silently to the server-reported latest, don't glow.
            // Only when `row.unread == 0`, though: per the protocol contract
            // unread is 0 whenever the channel has no server-side read cursor,
            // so `unread > 0` PROVES a cursor exists and `!known` merely means
            // local hydration hasn't landed yet (fresh device, get_unread won
            // the race against hydrate_last_seen). Baselining then would
            // mark_read(latest) and silently wipe the unread on ALL devices —
            // instead skip the row; the next pass glows it post-hydration.
            let known = s
                .notify
                .last_seen
                .with_untracked(|m| m.contains_key(&row.channel_id));
            if !known {
                if row.unread == 0 {
                    if let (Some(sent_at), Some(id)) = (row.latest_sent_at, row.latest_id) {
                        set_last_seen(s, &row.channel_id, (sent_at, id));
                    }
                }
                continue;
            }
            let has_new = row.unread > 0;
            // `row.pinged` is per-reader (the server evaluated the unread
            // mentions for THIS caller), so a true here is genuinely a ping
            // for me (L-4).
            let has_ping = row.pinged;
            if has_new {
                // M7/P1: DM rows have no guild (guild_id = None) — they feed the
                // per-channel unread set below, not a guild rail dot.
                if let Some(gid) = &row.guild_id {
                    unread_guilds.insert(gid.clone());
                }
            }
            let marked = s
                .notify
                .unread
                .with_untracked(|u| u.contains(&row.channel_id));
            if has_new != marked {
                s.notify.unread.update(|u| {
                    if has_new {
                        u.insert(row.channel_id.clone());
                    } else {
                        u.remove(&row.channel_id);
                    }
                });
            }
            let pinged_now = s
                .notify
                .pinged
                .with_untracked(|p| p.contains(&row.channel_id));
            if has_ping != pinged_now {
                s.notify.pinged.update(|p| {
                    if has_ping {
                        p.insert(row.channel_id.clone());
                    } else {
                        p.remove(&row.channel_id);
                    }
                });
            }
            let count_now = s
                .notify
                .unread_count
                .with_untracked(|c| c.get(&row.channel_id).copied().unwrap_or(0));
            if row.unread != count_now {
                s.notify.unread_count.update(|c| {
                    if has_new {
                        c.insert(row.channel_id.clone(), row.unread);
                    } else {
                        c.remove(&row.channel_id);
                    }
                });
            }
        }
        // Rebuilt fresh each pass (never incrementally mutated), written only
        // on change so an idle refresh doesn't re-render every rail badge.
        if s.notify
            .unread_guilds
            .with_untracked(|g| *g != unread_guilds)
        {
            s.notify.unread_guilds.set(unread_guilds);
        }
    });
}

/// True iff any text channel in the given guild has unread messages. Pure
/// projection over `notify.unread_guilds` (rebuilt by [`refresh_unread`] from
/// `GET /unread`'s `guild_id` column); called per rail button from a reactive
/// closure so the badge tracks the signal.
#[cfg(feature = "hydrate")]
pub fn guild_has_unread(s: Shell, gid: &str) -> bool {
    s.notify.unread_guilds.with(|g| g.contains(gid))
}

/// Toggle mute for a channel: flip the reactive set + persist. A click is a
/// user gesture, so it's also a good moment to ask for notification permission.
#[cfg(feature = "hydrate")]
pub fn toggle_mute(s: Shell, cid: String) {
    s.notify.muted.update(|m| {
        if !m.remove(&cid) {
            m.insert(cid.clone());
        }
    });
    let ids: Vec<String> = s
        .notify
        .muted
        .with_untracked(|m| m.iter().cloned().collect());
    let _ = LocalStorage::set(KEY_MUTED, &ids);
    super::notify::request_notify_permission(s);
}

// ---- background sync loop + page reconciler ----

/// The subset of `msgs` not yet in `s.msg.seen` — genuinely new this tick.
#[cfg(feature = "hydrate")]
fn unseen(s: Shell, msgs: &[MessageEnvelope]) -> Vec<MessageEnvelope> {
    msgs.iter()
        .filter(|m| !s.msg.seen.with_untracked(|h| h.contains(&m.id)))
        .cloned()
        .collect()
}

/// Full-set reconcile for a channel that fits in one page: reflects new,
/// edited, and deleted messages (including from other people), writing the
/// signal only when something actually changed so an idle poll causes no
/// re-render or scroll jump.
#[cfg(feature = "hydrate")]
fn sync_messages(s: Shell, fresh: Vec<MessageEnvelope>) {
    let changed = s.msg.messages.with_untracked(|cur| {
        cur.len() != fresh.len()
            || cur
                .iter()
                .zip(fresh.iter())
                .any(|(a, b)| a.id != b.id || a.body != b.body || a.persona_name != b.persona_name)
    });
    if !changed {
        return;
    }
    s.msg.seen.update(|h| {
        h.clear();
        for m in &fresh {
            h.insert(m.id.clone());
        }
    });
    s.msg
        .cursor
        .set(fresh.last().map(|m| (m.sent_at.clone(), m.id.clone())));
    s.msg.messages.set(fresh);
}

/// In-place refresh of the guild rail, the friends list, and the SELECTED
/// guild's channel list — each written only when it changed, so things
/// created or removed elsewhere appear/disappear without a manual reload.
///
/// Guild-channel loading is LAZY (W1): only the open guild's detail is
/// fetched (the old cross-guild `join_all` over every guild is gone — rail
/// unread dots now come from `GET /unread`'s `guild_id` via
/// `notify.unread_guilds`, see [`refresh_unread`]). The open guild's channels
/// mirror into `s.sel.channels` so the sidebar/channel-list selector keeps
/// its single-source-of-truth.
#[cfg(feature = "hydrate")]
pub(super) fn refresh_lists(s: Shell) {
    let sel = s.sel.sel_server.get_untracked();
    spawn_local(async move {
        if let Ok(r) = api::list_guilds().await {
            // `try_` (review M-10): every await in this task can resolve
            // after logout disposed the shell — each block re-proves
            // liveness before its same-tick writes.
            let Some(changed) = s.sel.guilds.try_with_untracked(|g| *g != r.guilds) else {
                return;
            };
            if changed {
                s.sel.guilds.set(r.guilds);
            }
        }
        if let Ok(f) = api::list_friends().await {
            let Some(changed) = s.social.friends.try_with_untracked(|cur| *cur != f) else {
                return;
            };
            if changed {
                s.social.friends.set(f);
            }
        }
        // M7/P1: DM threads ride the same lists refresh — a create/invite/leave
        // ListsChanged repaints them alongside guilds + friends.
        if let Ok(d) = api::list_dms().await {
            let Some(changed) = s.sel.dms.try_with_untracked(|cur| *cur != d.dms) else {
                return;
            };
            if changed {
                s.sel.dms.set(d.dms);
            }
        }
        // M7/P2: cameos ride the same lists refresh — an invite/revoke/leave/unfriend
        // ListsChanged repaints the caller's guest channels alongside DMs.
        if let Ok(c) = api::list_cameos().await {
            let Some(changed) = s.sel.cameos.try_with_untracked(|cur| *cur != c.cameos) else {
                return;
            };
            if changed {
                s.sel.cameos.set(c.cameos);
            }
        }
        let Some(gid) = sel else {
            return;
        };
        if let Ok(d) = api::get_guild(&gid).await {
            // Stale-guard: drop this pass's channels if the user switched
            // servers while the fetch was in flight. (`try_`, review M-10:
            // a disposed shell bails the same way.)
            let Some(sel_now) = s.sel.sel_server.try_get_untracked() else {
                return;
            };
            if sel_now.as_deref() != Some(gid.as_str()) {
                return;
            }
            // Keep the per-guild cache warm for the open guild; write both
            // signals only on change to avoid needless re-renders.
            if s.sel
                .guild_channels
                .with_untracked(|m| m.get(&gid) != Some(&d.channels))
            {
                s.sel.guild_channels.update(|m| {
                    m.insert(gid.clone(), d.channels.clone());
                });
            }
            if s.sel.channels.with_untracked(|c| *c != d.channels) {
                s.sel.channels.set(d.channels);
            }
        }
    });
}

/// Backfill older history when the user scrolls near the top: fetch the
/// page immediately before `oldest`, prepend it, and ask the channel pane
/// to re-anchor so the viewport stays put. Guarded against overlap and
/// against running past the start of history.
#[cfg(feature = "hydrate")]
pub fn load_older(s: Shell) {
    if s.msg.loading_older.get_untracked() || !s.msg.more_history.get_untracked() {
        return;
    }
    let Some(oldest) = s.msg.oldest.get_untracked() else {
        return;
    };
    let Some(ch) = s.sel.sel_channel.get_untracked() else {
        return;
    };
    s.msg.loading_older.set(true);
    spawn_local(async move {
        if let Ok(l) = api::list_messages_before(&ch.id, &oldest).await {
            // Disposal guard (review M-10): the fetch can resolve after
            // logout disposed the shell — one try_ read proves it alive for
            // the synchronous prepend below.
            if s.msg.more_history.try_get_untracked().is_none() {
                return;
            }
            if l.messages.len() < MESSAGES_PAGE_LIMIT {
                s.msg.more_history.set(false);
            }
            let fresh: Vec<_> = l
                .messages
                .into_iter()
                .filter(|m| !s.msg.seen.with_untracked(|h| h.contains(&m.id)))
                .collect();
            if !fresh.is_empty() {
                s.msg
                    .oldest
                    .set(fresh.first().map(|m| (m.sent_at.clone(), m.id.clone())));
                s.msg.seen.update(|h| {
                    for m in &fresh {
                        h.insert(m.id.clone());
                    }
                });
                // Anchor to the row currently at the top before everything
                // shifts down, so the viewport doesn't jump.
                let anchor = s
                    .msg
                    .messages
                    .with_untracked(|v| v.first().map(|m| m.id.clone()));
                s.msg.anchor_to.set(anchor);
                s.msg.messages.update(|v| {
                    let mut nw = fresh;
                    nw.append(v);
                    *v = nw;
                });
            }
        }
        // `try_` (review M-10): also reached when the fetch failed, still
        // past the await.
        let _ = s.msg.loading_older.try_set(false);
    });
}

/// One reconcile pass over the OPEN channel's messages + typing names: the
/// short-channel branch reconciles the whole page (reflects edits/deletes),
/// the long-history branch reconciles the newest-page WINDOW when the local
/// tail reaches into it (review M-08 — others' edits/deletes/restores now
/// land in channels past one page too), falling back to the append-only
/// cursor catch-up when the gap exceeds a page. No-op off the Channel
/// pane or with no channel selected. Extracted from the poll tick so the SSE
/// driver ([`super::sync`]) can run it per event; the poll loop still runs it
/// every 1.5s as the fallback cadence.
#[cfg(feature = "hydrate")]
pub(super) async fn refresh_open_channel(s: Shell) {
    // `try_` prelude (review M-10): this fn is awaited from post-await tails
    // (the poll loop's tick, dispatch's spawned tails, the typing-staleness
    // clearer) that can resume after logout disposed the shell — bail
    // instead of panicking, mirroring `sync::refresh_typing_surface`.
    if s.sync.pane.try_get_untracked() != Some(Pane::Channel) {
        return;
    }
    let Some(Some(ch)) = s.sel.sel_channel.try_get_untracked() else {
        return;
    };
    match api::list_messages(&ch.id, None).await {
        Ok(l) if l.messages.len() < MESSAGES_PAGE_LIMIT => {
            // Stale-guard + disposal guard (review M-10): drop this pass's
            // data if the channel changed — or the shell died — while the
            // fetch was in flight (feedback gwiif7xy).
            if s.sel
                .sel_channel
                .try_get_untracked()
                .flatten()
                .map(|c| c.id)
                != Some(ch.id.clone())
            {
                return;
            }
            let fresh = unseen(s, &l.messages);
            s.msg.typing.set(l.typing);
            sync_messages(s, l.messages);
            super::notify::notify_messages(s, &ch, &fresh);
            super::notify::dismiss_open_channel_notifs(&ch, &fresh);
        }
        Ok(l) => {
            // Long history: page 1 is only the newest MESSAGES_PAGE_LIMIT
            // rows. Stale-guard + disposal guard first, same as the short
            // branch (feedback gwiif7xy; review M-10).
            if s.sel
                .sel_channel
                .try_get_untracked()
                .flatten()
                .map(|c| c.id)
                != Some(ch.id.clone())
            {
                return;
            }
            let cur = s.msg.cursor.get_untracked();
            // Does the local tail reach into the fetched window? Composite
            // `(sent_at, id)` compare against the page's first row — the
            // page is ASC, so that row is the window start.
            let connected = match (cur.as_ref(), l.messages.first()) {
                (Some(c), Some(w)) => {
                    (c.0.as_str(), c.1.as_str()) >= (w.sent_at.as_str(), w.id.as_str())
                }
                _ => false,
            };
            if connected {
                // The page is the server's complete truth for everything at
                // or after its first row, so it can express OTHER PEOPLE's
                // edits, soft-deletes, and restores there — reconcile the
                // window (review M-08) instead of the old append-past-the-
                // cursor, which could never remove or rewrite a row (the SSE
                // message_edited/message_deleted events were dead letters
                // beyond one page). New arrivals necessarily live inside the
                // newest page when the tail connects, so the second catch-up
                // fetch is skipped as well.
                let fresh = unseen(s, &l.messages);
                s.msg.typing.set(l.typing);
                reconcile_newest_window(s, l.messages);
                super::notify::notify_messages(s, &ch, &fresh);
                super::notify::dismiss_open_channel_notifs(&ch, &fresh);
            } else if let Ok(l) = api::list_messages(&ch.id, cur.as_ref()).await {
                // Disconnected tail (nothing loaded yet, or more than a full
                // page arrived since): catch up past the cursor exactly as
                // before — appending here can never strand a mid-list gap.
                // Stale-guard + disposal guard again after the second await
                // (feedback gwiif7xy; review M-10).
                if s.sel
                    .sel_channel
                    .try_get_untracked()
                    .flatten()
                    .map(|c| c.id)
                    != Some(ch.id.clone())
                {
                    return;
                }
                let fresh = unseen(s, &l.messages);
                s.msg.typing.set(l.typing);
                ingest(s, l.messages);
                super::notify::notify_messages(s, &ch, &fresh);
                super::notify::dismiss_open_channel_notifs(&ch, &fresh);
            }
        }
        Err(_) => {}
    }
}

/// What one newest-page reconcile pass must do (review M-08). Within the
/// WINDOW — everything at/after the page's first row — the page is the
/// server's full truth, so:
/// - a local row missing from it was soft-DELETED → `removed`;
/// - a local row whose content differs was EDITED → `patched` (compared on
///   body + persona_name, the same fields [`sync_messages`] diffs);
/// - a page row this client has never seen is NEW or RESTORED → `inserts`
///   (`seen`-gated exactly like [`ingest`], so an optimistically-hidden
///   in-flight delete is never re-inserted).
///
/// Local rows OLDER than the window (paged-in history) are out of the page's
/// reach and left untouched — they re-sync on the next channel open. Pure;
/// unit-tested below.
#[cfg(any(feature = "hydrate", test))]
struct WindowPlan {
    /// Ids of in-window local rows the server no longer returns.
    removed: Vec<String>,
    /// Fresh envelopes for in-window local rows whose content changed.
    patched: Vec<MessageEnvelope>,
    /// Never-seen page rows, in page (ASC composite-cursor) order.
    inserts: Vec<MessageEnvelope>,
}

/// Pure decision core of [`reconcile_newest_window`]; see [`WindowPlan`].
#[cfg(any(feature = "hydrate", test))]
fn plan_window_reconcile(
    local: &[MessageEnvelope],
    page: &[MessageEnvelope],
    seen: &std::collections::HashSet<String>,
) -> WindowPlan {
    let mut plan = WindowPlan {
        removed: Vec::new(),
        patched: Vec::new(),
        inserts: Vec::new(),
    };
    let Some(first) = page.first() else {
        return plan; // empty page: nothing the window can claim
    };
    let w = (first.sent_at.as_str(), first.id.as_str());
    let by_id: std::collections::HashMap<&str, &MessageEnvelope> =
        page.iter().map(|m| (m.id.as_str(), m)).collect();
    for m in local {
        if (m.sent_at.as_str(), m.id.as_str()) < w {
            continue; // older than the window: out of the page's reach
        }
        match by_id.get(m.id.as_str()) {
            None => plan.removed.push(m.id.clone()),
            Some(f) => {
                if f.body != m.body || f.persona_name != m.persona_name {
                    plan.patched.push((*f).clone());
                }
            }
        }
    }
    plan.inserts = page
        .iter()
        .filter(|m| !seen.contains(&m.id))
        .cloned()
        .collect();
    plan
}

/// Apply a [`plan_window_reconcile`] pass to the open channel's signals.
/// Inserts land at their composite-cursor position (the `resurface`
/// partition rule), so a restored row reappears exactly where it was;
/// `seen` tracks removals/inserts so a later restore isn't dedupe-blocked
/// and a re-fetch can't duplicate; the cursor only ever ADVANCES (max), so
/// an old restored row can never rewind the catch-up frontier. Every signal
/// is written only when something actually changed: Typing events route
/// through [`refresh_open_channel`] every ~2s while someone types, and an
/// unconditional write would re-render the whole list per ping.
#[cfg(feature = "hydrate")]
fn reconcile_newest_window(s: Shell, page: Vec<MessageEnvelope>) {
    let plan = s.msg.messages.with_untracked(|local| {
        s.msg
            .seen
            .with_untracked(|seen| plan_window_reconcile(local, &page, seen))
    });
    if plan.removed.is_empty() && plan.patched.is_empty() && plan.inserts.is_empty() {
        return; // idle pass: no write, no re-render
    }
    s.msg.messages.update(|v| {
        if !plan.removed.is_empty() {
            v.retain(|m| !plan.removed.contains(&m.id));
        }
        for f in &plan.patched {
            if let Some(m) = v.iter_mut().find(|m| m.id == f.id) {
                *m = f.clone();
            }
        }
        for m in &plan.inserts {
            if v.iter().any(|x| x.id == m.id) {
                continue; // belt-and-braces against a racing reinsert
            }
            let at = v.partition_point(|x| {
                (x.sent_at.as_str(), x.id.as_str()) < (m.sent_at.as_str(), m.id.as_str())
            });
            v.insert(at, m.clone());
        }
    });
    if !plan.removed.is_empty() || !plan.inserts.is_empty() {
        s.msg.seen.update(|h| {
            for id in &plan.removed {
                h.remove(id);
            }
            for m in &plan.inserts {
                h.insert(m.id.clone());
            }
        });
    }
    if let Some(newest) = plan.inserts.last() {
        let nc = (newest.sent_at.clone(), newest.id.clone());
        let advanced = s
            .msg
            .cursor
            .with_untracked(|c| c.as_ref().map(|c| nc > *c).unwrap_or(true));
        if advanced {
            s.msg.cursor.set(Some(nc));
        }
    }
}

/// One Ghost Quill pass for the OPEN channel (W4/T7): fetch other members'
/// live drafts from `GET /typing-drafts` into their own `ghost_drafts` signal
/// (never the real `messages` list). Receiver-side opt-in: with the pref OFF
/// this clears any lingering ghosts and fetches NOTHING — the render is
/// pref-gated too, this is belt-and-braces. Run by the SSE driver on
/// `Typing`/`MessageCreated` events for the open channel and by its
/// staleness clearer; the poll fallback deliberately skips it (SSE-only
/// enhancement — under polling there is no per-keystroke nudge, so ghosts
/// would render seconds-stale and linger a full TTL).
#[cfg(feature = "hydrate")]
pub(super) async fn refresh_ghost_drafts(s: Shell) {
    // `try_` prelude (review M-10): awaited from post-await tails
    // (dispatch's spawned tails, the typing-staleness clearer) that can
    // resume after logout disposed the shell; the pref proof covers the
    // same-tick clear below.
    let Some(enabled) = s.prefs.ghost_quill.try_get_untracked() else {
        return;
    };
    if !enabled {
        if s.msg.ghost_drafts.with_untracked(|g| !g.is_empty()) {
            s.msg.ghost_drafts.set(Vec::new());
        }
        return;
    }
    if s.sync.pane.try_get_untracked() != Some(Pane::Channel) {
        return;
    }
    let Some(Some(ch)) = s.sel.sel_channel.try_get_untracked() else {
        return;
    };
    if let Ok(drafts) = api::get_typing_drafts(&ch.id).await {
        // Stale-guard + disposal guard (review M-10): drop this pass if the
        // channel changed — or the shell died — while the fetch was in
        // flight (same discipline as refresh_open_channel). The
        // unconditional set also clears the signal whenever the fetch
        // returns empty.
        if s.sel
            .sel_channel
            .try_get_untracked()
            .flatten()
            .map(|c| c.id)
            != Some(ch.id.clone())
        {
            return;
        }
        s.msg.ghost_drafts.set(drafts);
    }
}

/// The background sync loop (single instance, guarded by `s.sync.polling`).
/// Every tick it refreshes the open channel's messages; every ~6s it also
/// refreshes the lists. The AUTOMATIC FALLBACK behind the SSE driver
/// ([`super::sync::start_sync`]): it runs when EventSource is missing or
/// demoted — and since the self-healing evolution it self-terminates when
/// the driver generation moves on, i.e. when a resurrection probe got
/// promoted back to SSE. On that handover the `polling` latch stays held
/// (ownership transfers to the promoted driver rather than being released),
/// so belt-and-braces callers like channel-open keep no-opping.
#[cfg(feature = "hydrate")]
pub(super) fn start_poll(s: Shell) {
    if s.sync.polling.get_untracked() {
        return;
    }
    s.sync.polling.set(true);
    let gen = super::sync::current_gen();
    spawn_local(async move {
        let mut tick: u32 = 0;
        loop {
            gloo_timers::future::TimeoutFuture::new(1500).await;
            if super::sync::current_gen() != gen {
                // A promoted SSE driver owns sync now: stop fetching. Checked
                // BEFORE the fetches so a retired loop costs zero requests.
                break;
            }
            // Disposal guard (review M-10): logout's `sync::shutdown()`
            // bumps the generation, but a shell disposed WITHOUT it (context
            // teardown) would sail past the gen check into the refreshes'
            // plain same-tick prelude reads — a dead shell ends the loop.
            if s.sync.polling.try_get_untracked().is_none() {
                break;
            }
            tick = tick.wrapping_add(1);
            if tick.is_multiple_of(4) {
                refresh_lists(s);
                refresh_unread(s);
            }
            refresh_open_channel(s).await;
        }
    });
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
pub fn copy_message_body(_s: Shell, _body: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn guild_has_unread(_s: Shell, _gid: &str) -> bool {
    false
}
#[cfg(not(feature = "hydrate"))]
pub fn send_message(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn add_compose_attachment(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn remove_compose_attachment(_s: Shell, _key: u64) {}
#[cfg(not(feature = "hydrate"))]
pub fn retry_compose_attachment(_s: Shell, _key: u64) {}
#[cfg(not(feature = "hydrate"))]
pub fn start_reply(_s: Shell, _m: crate::protocol::MessageEnvelope) {}
#[cfg(not(feature = "hydrate"))]
pub fn cancel_reply(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn start_edit(_s: Shell, _cid: String, _mid: String, _body: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn cancel_edit(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn delete_message(_s: Shell, _cid: String, _mid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn load_deleted_messages(_s: Shell, _cid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn restore_deleted_message(_s: Shell, _cid: String, _mid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn load_deleted_channels(_s: Shell, _gid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn ask_delete(_s: Shell, _prompt: String, _pending: PendingDelete) {}
#[cfg(not(feature = "hydrate"))]
pub fn cancel_delete(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn confirm_delete(s: Shell) {
    // Mirrors the hydrate dispatch shape so the per-action stubs stay "used".
    match None::<PendingDelete> {
        Some(PendingDelete::Channel { gid, cid }) => super::channel::delete_channel(s, gid, cid),
        Some(PendingDelete::Server { gid }) => super::guild::delete_server(s, gid),
        Some(PendingDelete::Persona { pid }) => super::persona::remove_persona(s, pid),
        None => {}
    }
}
#[cfg(not(feature = "hydrate"))]
pub fn show_friends(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn show_wardrobe(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn show_emoji_manager(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn show_members(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn show_dms(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn add_friend(_s: Shell, _username: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn invite_member(_s: Shell, _gid: String, _username: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn accept_friend(_s: Shell, _aid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn remove_friend(_s: Shell, _aid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn create_lore(_s: Shell, _cid: String, _keys: Vec<String>, _content: String) {}
#[cfg(not(feature = "hydrate"))]
#[allow(clippy::too_many_arguments)]
pub fn patch_lore(
    _s: Shell,
    _cid: String,
    _eid: String,
    _title: Option<String>,
    _keys: Option<Vec<String>>,
    _content: Option<String>,
    _enabled: Option<bool>,
    _position: Option<i64>,
) {
}
#[cfg(not(feature = "hydrate"))]
pub fn swap_lore(_s: Shell, _cid: String, _eid: String, _position: i64, _up: bool) {}
#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
pub fn move_lore(_s: Shell, _idx: usize, _target: usize) {}
#[cfg(not(feature = "hydrate"))]
pub fn delete_lore(_s: Shell, _cid: String, _eid: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn load_muted(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn load_last_seen(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn hydrate_last_seen(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn toggle_mute(_s: Shell, _cid: String) {}
#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
pub fn load_older(_s: Shell) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal envelope with just the fields the fns under test read
    /// (composite cursor + body); everything else is inert filler — the
    /// `reentry` test helper's shape.
    fn env(id: &str, sent_at: &str, body: &str) -> MessageEnvelope {
        MessageEnvelope {
            id: id.into(),
            author_id: "account:a".into(),
            author_name: "a".into(),
            author_display: "A".into(),
            author_avatar_id: None,
            persona_id: None,
            persona_name: None,
            persona_description: None,
            persona_color: None,
            persona_avatar_id: None,
            body: body.into(),
            attachments: Vec::new(),
            tier: "default".into(),
            sent_at: sent_at.into(),
            reply_to: None,
            is_pinged: false,
            kind: "user".into(),
            effect: None,
            guest_cameo: false,
        }
    }

    fn seen_of(msgs: &[MessageEnvelope]) -> std::collections::HashSet<String> {
        msgs.iter().map(|m| m.id.clone()).collect()
    }

    const T1: &str = "2026-06-12T08:00:00.000000000Z";
    const T2: &str = "2026-06-12T09:00:00.000000000Z";
    const T3: &str = "2026-06-12T10:00:00.000000000Z";
    const T4: &str = "2026-06-12T11:00:00.000000000Z";

    #[test]
    fn plan_window_reconcile_removes_only_in_window_rows_the_server_dropped() {
        // Local holds z (OLDER than the page window), a, b; the server's
        // newest page starts at a and no longer returns b (soft-deleted).
        // b must be removed; z is out of the page's reach and must survive.
        let local = vec![
            env("z", T1, "old history"),
            env("a", T2, "x"),
            env("b", T3, "x"),
        ];
        let page = vec![env("a", T2, "x"), env("c", T4, "x")];
        let plan = plan_window_reconcile(&local, &page, &seen_of(&local));
        assert_eq!(plan.removed, vec!["b".to_string()]);
        assert!(plan.patched.is_empty());
        // c is genuinely new (not in seen) — it lands as an insert.
        assert_eq!(
            plan.inserts
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c"]
        );
    }

    #[test]
    fn plan_window_reconcile_patches_an_edited_in_window_row() {
        let local = vec![env("a", T2, "before"), env("b", T3, "same")];
        let page = vec![env("a", T2, "after"), env("b", T3, "same")];
        let plan = plan_window_reconcile(&local, &page, &seen_of(&local));
        assert!(plan.removed.is_empty());
        assert_eq!(plan.patched.len(), 1);
        assert_eq!(plan.patched[0].id, "a");
        assert_eq!(plan.patched[0].body, "after");
        assert!(plan.inserts.is_empty(), "an unchanged page is a no-op");
    }

    #[test]
    fn plan_window_reconcile_never_resurrects_an_optimistically_hidden_row() {
        // The undo-delete flow hides a row from `messages` while keeping its
        // id in `seen` (the in-flight DELETE guard) — the server still
        // returns it until the DELETE commits. It must NOT be re-inserted.
        let local = vec![env("a", T2, "x")];
        let page = vec![env("a", T2, "x"), env("hidden", T3, "x")];
        let mut seen = seen_of(&local);
        seen.insert("hidden".to_string());
        let plan = plan_window_reconcile(&local, &page, &seen);
        assert!(plan.inserts.is_empty());
        assert!(plan.removed.is_empty());
        assert!(plan.patched.is_empty());
    }

    #[test]
    fn plan_window_reconcile_inserts_a_restored_row_behind_the_local_tail() {
        // Cross-client restore: the row was reconciled away on delete (gone
        // from messages AND seen), then restored server-side — it reappears
        // in the page OLDER than the local tail and must come back.
        let local = vec![env("a", T2, "x"), env("c", T4, "x")];
        let page = vec![
            env("a", T2, "x"),
            env("restored", T3, "x"),
            env("c", T4, "x"),
        ];
        let plan = plan_window_reconcile(&local, &page, &seen_of(&local));
        assert_eq!(
            plan.inserts
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["restored"]
        );
        assert!(plan.removed.is_empty());
    }

    #[test]
    fn plan_window_reconcile_is_inert_on_an_empty_page() {
        let local = vec![env("a", T2, "x")];
        let plan = plan_window_reconcile(&local, &[], &seen_of(&local));
        assert!(plan.removed.is_empty() && plan.patched.is_empty() && plan.inserts.is_empty());
    }

    #[test]
    fn reply_banner_snippet_masks_a_whispered_parent_with_the_fixed_placeholder() {
        // W4 whisper-mask invariant (review M-27): the composer reply banner
        // is a body-preview surface — it must show the SAME fixed
        // placeholder the persisted quote will show, never the spoiler text.
        assert_eq!(
            reply_banner_snippet("the hidden secret", Some("whisper")),
            "(whisper)"
        );
    }

    #[test]
    fn reply_banner_snippet_truncates_normal_bodies_to_100_chars() {
        let long: String = "x".repeat(150);
        assert_eq!(reply_banner_snippet(&long, None).chars().count(), 100);
        assert_eq!(reply_banner_snippet("short", Some("shout")), "short");
    }
}
