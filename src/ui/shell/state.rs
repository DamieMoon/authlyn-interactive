//! Shell reactive state grouped into 10 sub-structs.
//!
//! `AppShell` (in `mod.rs`) constructs each sub-struct, calls
//! `provide_context::<T>(t)` for each (mirroring the existing `EmojiResolver`
//! pattern), then assembles a flat [`Shell`] handle from the sub-struct
//! handles. The aggregate is what `act::*` and the pane components take as a
//! prop today; M6/C8 migrates the pane consumers to `use_context` and lets
//! the aggregate stay for `act::*` only.
//!
//! Every field is an `RwSignal<T>` — `Copy` and cheap to pass around. The
//! sub-structs themselves derive `Clone, Copy`, so a pane that holds a
//! `Selection` handle is just two pointer-sized signal IDs (per field) plus
//! the struct header.
//!
//! Type imports live here so adding a new state slot only touches `state.rs`
//! and `AppShell`'s constructor.
//!
//! Marked `#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]` because
//! ssr-side these signals are constructed-but-never-read (the shell only
//! renders client-side).

use std::collections::{HashMap, HashSet};

use leptos::prelude::{RwSignal, StoredValue};

use crate::protocol::{
    Attachment, CameoSummary, ChannelSummary, CustomEmoji, DmSummary, GuildSummary,
    ListFriendsResponse, LorebookEntry, MessageEnvelope, PersonaSummary, ReplyPreview,
    TypingDraftEntry,
};

use super::{NavOrigin, Pane, PendingDelete};

/// Server + channel selection, plus the lists they live in.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Selection {
    pub(crate) guilds: RwSignal<Vec<GuildSummary>>,
    pub(crate) sel_server: RwSignal<Option<String>>,
    /// Owner account id of the currently-open server (gates the invite control).
    pub(crate) sel_owner: RwSignal<Option<String>>,
    pub(crate) channels: RwSignal<Vec<ChannelSummary>>,
    /// Per-guild channel cache: guild id → channels. Populated alongside the
    /// guild list (via parallel `get_guild`) so the guild rail can show an
    /// unread badge for ANY guild whose channels carry messages past the
    /// caller's `last_seen` — not only the currently-open guild. Mirrors
    /// `channels` for the open guild's entry; the two stay consistent because
    /// `refresh_lists` writes both.
    pub(crate) guild_channels: RwSignal<HashMap<String, Vec<ChannelSummary>>>,
    /// Custom emoji of the currently-open guild. Powers the composer picker,
    /// `:`-autocomplete, and `:name:` render resolution via the `EmojiResolver`
    /// context built in `AppShell`.
    pub(crate) guild_emoji: RwSignal<Vec<CustomEmoji>>,
    pub(crate) sel_channel: RwSignal<Option<ChannelSummary>>,
    /// M7/P1: the caller's DM threads (1:1 + groups), refreshed alongside the
    /// guild list in `refresh_lists` (so a `ListsChanged` create/invite/leave
    /// repaints them). Opening one routes through `sel_channel`/ChannelPane like
    /// any channel — a DM thread *is* a channel.
    pub(crate) dms: RwSignal<Vec<DmSummary>>,
    /// M7/P2: the caller's active Guest Cameos — guild text channels they're a
    /// guest in (they can't see the host guild's rail). Refreshed on `ListsChanged`
    /// alongside DMs; opening one routes through `sel_channel`/ChannelPane like any
    /// channel.
    pub(crate) cameos: RwSignal<Vec<CameoSummary>>,
}

/// The open channel's message list + the three-cursor pagination state and
/// the live typists.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct MessageView {
    pub(crate) messages: RwSignal<Vec<MessageEnvelope>>,
    pub(crate) cursor: RwSignal<Option<(String, String)>>,
    /// Oldest `(sent_at, id)` currently loaded — the cursor for scroll-up
    /// backfill of older history. `None` until the first page lands.
    pub(crate) oldest: RwSignal<Option<(String, String)>>,
    /// Guards against overlapping scroll-up backfills.
    pub(crate) loading_older: RwSignal<bool>,
    /// `false` once a backfill returns a short page (start of history reached).
    pub(crate) more_history: RwSignal<bool>,
    /// True while the channel's FIRST page is in flight (set on switch, cleared
    /// when the initial `list_messages` lands or fails). Drives the loading
    /// skeleton: skeleton rows show only while this is set AND `messages` is
    /// still empty (F-7). Transient client-only flag — never persisted/sent.
    pub(crate) loading_initial: RwSignal<bool>,
    /// After an older-history prepend, the message id to re-anchor to the top
    /// so the viewport doesn't jump; the channel pane scrolls it into view.
    pub(crate) anchor_to: RwSignal<Option<String>>,
    pub(crate) seen: RwSignal<HashSet<String>>,
    /// Display names of OTHER members currently typing in the open channel
    /// (#19), refreshed from each message-poll response. Cleared on channel
    /// switch; drives the `.typing-indicator` line above the composer.
    pub(crate) typing: RwSignal<Vec<String>>,
    /// Ghost Quill (M4/T7): OTHER members' live drafts in the open channel,
    /// fetched from `GET /typing-drafts` on `Typing`/`MessageCreated` SSE
    /// events when the receiver's pref is on. Deliberately its OWN signal —
    /// ghost rows must never collide with the real `messages` list state.
    /// Cleared on channel switch and whenever a fetch returns empty; rendered
    /// only while `Prefs::ghost_quill` is on. SSE-only enhancement: the poll
    /// fallback never populates it.
    pub(crate) ghost_drafts: RwSignal<Vec<TypingDraftEntry>>,
    /// Re-entry NEW divider (UX evolution #9): the unread baseline captured
    /// when the channel was opened — the composite `(sent_at, id)` last-seen
    /// cursor from BEFORE the open advanced it. `Some` only when the opened
    /// page actually held rows past it; the list renders a virtual "NEW"
    /// divider above the first such row (`act::reentry::first_past_baseline`).
    /// Render-time ornament ONLY: it never enters seen/cursor bookkeeping and
    /// never writes read state. Reset on every channel switch
    /// (`act::channel::open_channel_at`) and cleared when the user posts —
    /// send or roll — into the channel (Discord parity: writing means caught
    /// up; `act::message::after_send_success`).
    pub(crate) new_divider: RwSignal<Option<(String, String)>>,
}

/// Max staged attachments per message (composer cap). Matches the server-side
/// `MAX_ATTACHMENTS` in `src/server/messages/mod.rs` (M7/B1) — the server
/// rejects POSTs over this; the client gates earlier so the user gets a clean
/// toast instead of upload-then-reject. Keep the two in sync by intent.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) const COMPOSER_MAX_ATTACHMENTS: usize = 100;

/// Lifecycle of one staged compose attachment's upload (F-8). Client-only
/// transient state — never serialized; the wire SEND request carries only the
/// media id once `Ready`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) enum UploadStatus {
    /// Bytes are going up; `f32` is the fraction `0.0..=1.0`, or `None` when the
    /// browser can't compute a total (render an indeterminate bar).
    Uploading(Option<f32>),
    /// Upload finished; `att.id` is a real media id ready to send.
    Ready,
    /// Upload failed; the slot shows a retry button. Holds a short message.
    Failed(String),
}

/// A composer attachment plus its transient upload lifecycle (F-8). Wraps the
/// wire [`Attachment`] DTO rather than mutating it, so the serialized shape the
/// server emits is untouched. While `Uploading`/`Failed` the inner
/// `att.id` is a client-only placeholder (the file's object key index, not a
/// media id); it becomes a real media id only on `Ready`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct StagedAttachment {
    /// Stable per-stage key so the view can address a slot for progress
    /// updates / removal / retry independent of the (late-arriving) media id.
    pub(crate) key: u64,
    pub(crate) att: Attachment,
    pub(crate) status: UploadStatus,
}

/// Compose box (draft text + staged attachments + last status line).
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Composer {
    pub(crate) compose: RwSignal<String>,
    /// Staged attachments with per-item upload progress/status (F-8), in pick
    /// order. Only `Ready` items' media ids are sent with the next message.
    pub(crate) compose_attachments: RwSignal<Vec<StagedAttachment>>,
    pub(crate) status: RwSignal<String>,
    /// Per-channel saved drafts (channel id -> in-progress text), stashed on
    /// channel switch so each channel keeps its own draft (feedback fvffwu /
    /// fkqdtp). Client-only: never persisted or sent to the server.
    pub(crate) drafts: RwSignal<HashMap<String, String>>,
    /// Quick-swap color-swatch history (tag names, most-recent-first, capped at
    /// 3) shown inline in the composer toolbar (feedback rli3tsora4ho7lsi9q31).
    /// Persisted to localStorage; client-only, never sent to the server.
    pub(crate) last_used_colors: RwSignal<Vec<String>>,
    /// The message this compose is replying to (L-3), or `None` for a normal
    /// send. Drives the "replying to X" composer banner and rides as
    /// `reply_to_id` on the next send. Reuses the wire [`ReplyPreview`] shape so
    /// the banner shows the parent author + snippet without a lookup. Cleared on
    /// send and on channel switch.
    pub(crate) replying_to: RwSignal<Option<ReplyPreview>>,
    /// When set, the composer is editing an existing message in place of
    /// composing a new one: clicking ✎ loads the message body into the compose
    /// box, the Send button becomes "Save", and Send/Enter dispatches an edit
    /// instead of a post. Drives the "Editing message" banner; the ✕ / Esc
    /// restores the stashed draft. Client-only; never sent or persisted.
    pub(crate) editing: RwSignal<Option<EditingMessage>>,
    /// One-shot send-pulse flag (M4/T2): `act::send_message` flips it true
    /// after a successful post and a detached ~400ms timer resets it, so the
    /// Send button's `.sent` class plays a single `fx-glow-pulse`. Cosmetic
    /// and client-only; never sent or persisted.
    pub(crate) sent: RwSignal<bool>,
    /// Pulse generation, bumped per send: the detached reset timer only
    /// clears [`Composer::sent`] if its generation is still current, so an
    /// EARLIER send's timer can't truncate a LATER send's pulse mid-burst
    /// (the `LongPress` pattern, channel/radial.rs). `StoredValue` (not a
    /// signal) — it's plumbing, not UI.
    pub(crate) sent_gen: StoredValue<u64>,
    /// Delivery-effect mode for the NEXT send (M4/T5): `"whisper"`, `"shout"`,
    /// `"spell"`, or `None` for an ordinary message. Cycled by the composer's
    /// effect picker, sent as `SendMessageRequest::effect`, and RESET to `None`
    /// after each send (an effect is a per-message flourish, not a sticky
    /// mode). Client-only state; the server re-validates the value.
    pub(crate) effect_mode: RwSignal<Option<String>>,
}

/// An in-progress message edit driven through the main composer (see
/// [`Composer::editing`]). `stashed_draft` holds whatever the user was typing
/// when they hit ✎, so cancelling or saving restores it.
#[derive(Clone)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct EditingMessage {
    pub(crate) cid: String,
    pub(crate) mid: String,
    pub(crate) stashed_draft: String,
}

/// Background-sync, current pane selection, the mobile bottom-sheet, and the
/// auth-mirrored account id.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct SyncState {
    /// Latch: true while a background sync driver is running — either the SSE
    /// EventSource (`act::sync::start_sync`) or the legacy poll loop
    /// (`act::message::start_poll`). Both entry points are idempotent through
    /// it. Handover semantics (self-healing evolution): demoting to polling
    /// releases-and-retakes it, while a resurrection probe promoting back to
    /// SSE keeps it held (ownership transfers; the retired poll loop sees the
    /// driver-generation bump and stops on its own).
    pub(crate) polling: RwSignal<bool>,
    /// True while the SSE `EventSource` stream is connected: set on a
    /// current-generation `onopen` (alongside the consecutive-error reset —
    /// they fire together) and cleared on every stream error, at the
    /// poll-fallback demotion, on the constructor-failure path, and when the
    /// wake listener finds the stream terminally CLOSED after a frozen-PWA
    /// resume (`act::sync`). Drives the topbar's honest `● LIVE` /
    /// `● POLLING` chip — state.rs is shared across graphs, but a bare
    /// `RwSignal<bool>` compiles everywhere; only the hydrate-real sync
    /// driver ever WRITES it (ssr constructs it `false` and never reads it,
    /// like every other signal here).
    pub(crate) sse_live: RwSignal<bool>,
    /// The signed-in account's id, mirrored from `AuthCtx` so background tasks
    /// (e.g. the notification poll) can filter out the user's OWN messages
    /// without reaching into reactive context from a spawned future (FB10b).
    pub(crate) me: RwSignal<Option<String>>,
    pub(crate) pane: RwSignal<Pane>,
    /// Whether the wardrobe is open as a dismissible modal popup (F-2). The
    /// wardrobe is no longer a full pane you can only leave by selecting
    /// another pane — it overlays the current view and closes on backdrop
    /// click / Esc / X, and auto-closes when a channel is opened.
    pub(crate) wardrobe_open: RwSignal<bool>,
    /// Whether the orbit MAP overlay (the home/landing surface) is open.
    /// Promoted from a `SkOrbitShell`-local signal so the root-mounted
    /// Account/Server modals — which live OUTSIDE `SkOrbitShell` and can't see a
    /// shell-local signal — can return the user to the map on dismiss via
    /// `act::show_orbit_map`. Only the hydrate orbit shell ever reads/writes it.
    pub(crate) map_open: RwSignal<bool>,
    /// Whether the Station slide-over is open. Promoted from a `SkOrbitShell`-local
    /// signal (same rationale as `map_open`) so the root-mounted Account/Server/
    /// Wardrobe modals can REOPEN Station on dismiss when that's where they were
    /// opened from (Bug 3 one-step-back). The orbit shell aliases it back to a
    /// local `station_open` binding so its in-shell use-sites are unchanged.
    pub(crate) station_open: RwSignal<bool>,
    /// Origin of the current dispatch pane / management modal — where its back /
    /// dismiss pops to (`act::pane_back` / `act::modal_back`). Stamped by each
    /// opener (`act::mark_station_origin` for Station-launched surfaces); reset to
    /// `OrbitMap` by `show_orbit_map` and `open_channel_at` so it self-heals on
    /// any return home or channel descent. Default `OrbitMap` (boot lands here).
    pub(crate) pane_origin: RwSignal<NavOrigin>,
    /// True iff the current pane is a Friends SUB-pane (DMs/Cameos opened from a
    /// Friends in-pane link): back pops to Friends first, which then backs to its
    /// own `pane_origin`. Lets the two-state `pane_origin` express the one nesting
    /// point in the tree without a full history stack.
    pub(crate) pane_via_friends: RwSignal<bool>,
    /// Set during a channel switch to play the warp transition (M4/T3):
    /// `act::open_channel_at` flips it true on entry and a detached ~180ms
    /// timer clears it, driving the `.channel-view.fx-switching` class (rebased
    /// off `.content` in M5/P0 #54). Cosmetic and client-only; never sent or
    /// persisted.
    pub(crate) switching: RwSignal<bool>,
}

/// Friends, the wardrobe, the active worn persona, and the open channel's
/// lorebook entries.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Social {
    pub(crate) friends: RwSignal<ListFriendsResponse>,
    pub(crate) personas: RwSignal<Vec<PersonaSummary>>,
    pub(crate) active_persona: RwSignal<Option<String>>,
    pub(crate) lore: RwSignal<Vec<LorebookEntry>>,
}

/// Destructive action awaiting confirmation, with its human prompt; the
/// top-level confirm modal renders whenever `pending_delete` is `Some`.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Modals {
    pub(crate) pending_delete: RwSignal<Option<PendingDelete>>,
    pub(crate) confirm_prompt: RwSignal<Option<String>>,
}

/// Mute / unread / last-seen tracking for the channel notification badges.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Notify {
    /// Channel ids the user has muted (no new-message notifications). Mirrored
    /// to localStorage so it survives reloads.
    pub(crate) muted: RwSignal<HashSet<String>>,
    /// Channel ids with unread messages — drives the sidebar's white glow (#23).
    /// Recomputed by the background poll against `last_seen`.
    pub(crate) unread: RwSignal<HashSet<String>>,
    /// Per-channel high-water mark this client has seen: channel id →
    /// (sent_at, id) of the last seen message. Persisted to localStorage;
    /// unread = the channel has messages past this mark.
    pub(crate) last_seen: RwSignal<HashMap<String, (String, String)>>,
    /// True once a Web Push subscription has been successfully registered with
    /// the server this session. While true, the poll-loop suppresses its own
    /// client `Notification` (server push already delivers it — see
    /// `notify::notify_messages`); when false the poll path is the fallback.
    pub(crate) web_push_enabled: RwSignal<bool>,
    /// Re-entry scroll memory (UX evolution #9): channel id → the message row
    /// id that was at the top of the viewport when the user last LEFT the
    /// channel (no entry = they left at the tail). Captured on switch-away,
    /// consumed one-shot on the next open UNCONDITIONALLY — even when a
    /// deep-link or the NEW-divider jump outranks it (review: a surviving
    /// mark restored a stale position on a later open) — and persisted to
    /// localStorage like the drafts map (`act::reentry`). Unbounded like that
    /// drafts map (house pattern): entries for channels deleted or never
    /// revisited linger as two small strings each — negligible, and every
    /// revisit self-prunes its own entry via the unconditional consume.
    /// Client-only; never sent to the server and never feeds `last_seen` /
    /// read state.
    pub(crate) scroll_marks: RwSignal<HashMap<String, String>>,
}

/// Soft-deleted-item overlays (#22 Phase 2): deleted channels in the open
/// guild, deleted messages in the open channel, and whether the channel's
/// trash overlay is open.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Trash {
    pub(crate) deleted_channels: RwSignal<Vec<ChannelSummary>>,
    pub(crate) deleted_messages: RwSignal<Vec<MessageEnvelope>>,
    pub(crate) show_msg_trash: RwSignal<bool>,
}

/// The app's toast primitive (UX evolution #11): one transient glass capsule
/// at a time, anchored above the composer/tab bar. A new toast replaces the
/// current one; each auto-dismisses on its own keyed timer (`act::toast`).
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Toasts {
    /// The toast currently shown, or `None`. Written only by `act::toast`
    /// (push / keyed dismiss); read by the `toast_host` view.
    pub(crate) current: RwSignal<Option<Toast>>,
}

/// One transient toast. Client-only and ephemeral — never persisted or sent.
/// (No `PartialEq`: `ToastAction` carries a full `MessageEnvelope` snapshot,
/// and nothing ever compares whole toasts — keyed dismiss compares `key`.)
#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Toast {
    /// Generation key minted per toast so the detached auto-dismiss timer
    /// (and an action-targeted dismiss) only ever clears its OWN toast — the
    /// send-pulse pattern (`Composer::sent_gen`).
    pub(crate) key: u64,
    pub(crate) text: String,
    /// Visual register — `_toast.scss` styles the variants.
    pub(crate) tone: ToastTone,
    /// The single optional action slot, described as data (the
    /// [`super::PendingDelete`] convention — closures don't ride signals);
    /// dispatched by `act::run_toast_action`.
    pub(crate) action: Option<ToastAction>,
    /// Lifetime in ms — drives both the auto-dismiss timer and the CSS
    /// drain bar (`--toast-ms`).
    pub(crate) duration_ms: u32,
}

/// A toast's visual register (UX evolution #11).
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) enum ToastTone {
    /// Calm default chrome (the undo toast).
    Info,
    /// Mint confirmation ("Copied", "invited X") — the status line's success
    /// traffic absorbed in success styling, leaving the red `<p>` for errors.
    Success,
    /// Danger styling (failed delete / restore).
    Danger,
}

/// A toast's action slot, as data.
#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) enum ToastAction {
    /// "Undo" on the message-delete toast: POST the EXISTING own-gated
    /// `/channels/{cid}/messages/{mid}/restore` — the soft-delete already
    /// committed on tap (review blocker fix: a client-delayed DELETE inverts
    /// the failure mode) — then resurface the snapshot `envelope` in place
    /// (`act::message::undo_message_delete`). Self-contained: no client-side
    /// pending registry exists, so a replaced/expired toast strands nothing.
    UndoMessageDelete {
        cid: String,
        mid: String,
        envelope: Box<crate::protocol::MessageEnvelope>,
    },
}

/// Per-user preferences mirrored to localStorage.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Prefs {
    /// When on, `"…"` dialogue is styled at render via a `.dialogue-style`
    /// root class. Persisted to localStorage.
    pub(crate) dialogue_style: RwSignal<bool>,
    /// Ghost Quill (M4/T7): opt-in live co-writer draft preview. Governs BOTH
    /// directions for this client — sending the compose text with the typing
    /// ping AND fetching/rendering other members' ghost rows. Default OFF
    /// (privacy-respecting). Persisted to localStorage.
    pub(crate) ghost_quill: RwSignal<bool>,
    /// M5/P0 #19: whether to mirror visual haptics to navigator.vibrate where
    /// supported (Android). Default OFF; visual feedback is always primary.
    /// Persisted to localStorage as authlyn.haptic_vibrate.
    pub(crate) haptic_vibrate: RwSignal<bool>,
    /// M5/P1: the selected structural UI skeleton id (orbit/deck/hud). Drives
    /// the `.app.sk-*` root class. `None` until the ceremony resolves (pref-less
    /// first run); the render treats `None` as "no sk-* class yet" while the
    /// ceremony modal is up. Persisted to localStorage as authlyn.skeleton.
    pub(crate) skeleton: RwSignal<Option<String>>,
}
