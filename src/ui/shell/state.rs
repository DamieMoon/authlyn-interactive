//! Shell reactive state grouped into 9 sub-structs.
//!
//! `AppShell` (in `mod.rs`) constructs each sub-struct, calls
//! `provide_context::<T>(t)` for each (mirroring the existing `EmojiResolver`
//! pattern), then assembles a flat [`Shell`] handle from the sub-struct
//! handles. The aggregate is what `act::*` and the pane components take as a
//! prop today; W6/C8 migrates the pane consumers to `use_context` and lets
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

use leptos::prelude::RwSignal;

use crate::protocol::{
    Attachment, ChannelSummary, CustomEmoji, GuildSummary, ListFriendsResponse, LorebookEntry,
    MessageEnvelope, PersonaSummary,
};

use super::{Pane, PendingDelete};

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
    /// After an older-history prepend, the message id to re-anchor to the top
    /// so the viewport doesn't jump; the channel pane scrolls it into view.
    pub(crate) anchor_to: RwSignal<Option<String>>,
    pub(crate) seen: RwSignal<HashSet<String>>,
    /// Display names of OTHER members currently typing in the open channel
    /// (#19), refreshed from each message-poll response. Cleared on channel
    /// switch; drives the `.typing-indicator` line above the composer.
    pub(crate) typing: RwSignal<Vec<String>>,
}

/// Max staged attachments per message (composer cap). Matches the server-side
/// `MAX_ATTACHMENTS` in `src/server/messages/mod.rs` (W7/B1) — the server
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
}

/// Background-sync, current pane selection, mobile drawer, and the
/// auth-mirrored account id.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct SyncState {
    pub(crate) polling: RwSignal<bool>,
    /// The signed-in account's id, mirrored from `AuthCtx` so background tasks
    /// (e.g. the notification poll) can filter out the user's OWN messages
    /// without reaching into reactive context from a spawned future (FB10b).
    pub(crate) me: RwSignal<Option<String>>,
    pub(crate) pane: RwSignal<Pane>,
    /// Mobile-only: whether the off-canvas rail+sidebar drawer is open.
    pub(crate) nav_open: RwSignal<bool>,
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
    /// Channel ids with unread messages — drives the sidebar glow (#23).
    /// Recomputed by the background poll against `last_seen`.
    pub(crate) unread: RwSignal<HashSet<String>>,
    /// Per-channel high-water mark this client has seen: channel id →
    /// (sent_at, id) of the last seen message. Persisted to localStorage;
    /// unread = the channel has messages past this mark.
    pub(crate) last_seen: RwSignal<HashMap<String, (String, String)>>,
}

/// Soft-deleted-item overlays (#22 Phase 2): own deleted guilds, deleted
/// channels in the open guild, deleted messages in the open channel, and
/// whether the channel's trash overlay is open.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Trash {
    pub(crate) deleted_guilds: RwSignal<Vec<GuildSummary>>,
    pub(crate) deleted_channels: RwSignal<Vec<ChannelSummary>>,
    pub(crate) deleted_messages: RwSignal<Vec<MessageEnvelope>>,
    pub(crate) show_msg_trash: RwSignal<bool>,
}

/// Per-user preferences mirrored to localStorage.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Prefs {
    /// When on, `"…"` dialogue is styled at render via a `.dialogue-style`
    /// root class. Persisted to localStorage.
    pub(crate) dialogue_style: RwSignal<bool>,
}
