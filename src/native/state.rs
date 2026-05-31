//! Shared UI state — the native mirror of `src/ui/shell/state.rs`.
//!
//! `RwSignal<T>` → Freya `State<T>` (a `Copy` generational-box signal). The web
//! groups signals into `Selection`/`MessageView`/`SyncState`; here we keep one
//! flat `NativeState` (itself `Copy`, since every `State<T>` is) created once at
//! the root via [`use_native_state`] and passed by value into the view fns.

use freya::prelude::*;
use std::collections::HashSet;

use crate::protocol::{
    ChannelSummary, CustomEmoji, GuildSummary, MeResponse, MessageEnvelope, PersonaSummary,
};

/// Which pane the 3rd column shows. The native mirror of the web's "active view"
/// routing: the channel reader, the account-scoped wardrobe, or the guild-scoped
/// custom-emoji manager. `Copy`/`PartialEq` so it sits in a `State<NativeView>`
/// and compares cheaply in the `ui.rs` dispatch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NativeView {
    /// The existing 3-pane channel reader/composer.
    Channel,
    /// The persona wardrobe (account-scoped).
    Wardrobe,
    /// The per-guild custom-emoji manager (guild-scoped).
    EmojiManager,
}

/// A confirm/edit overlay rendered over the shell — the native mirror of the
/// web's `PendingDelete` confirm flow plus the persona detail editor. The leaves
/// open one by writing `Some(..)` into [`NativeState::modal`] and close it by
/// writing `None`; the confirm handlers dispatch their `act` fn then clear it.
#[derive(Clone, Debug)]
pub enum NativeModal {
    /// The persona detail editor for persona `pid` (name/description/color/
    /// gallery/avatar/sharing). The working buffers live on [`NativeState`]
    /// (`pe_*`); this variant only carries which persona is open.
    PersonaEditor { pid: String },
    /// Confirm deleting (owner) the persona `pid`; `name` is shown in the prompt.
    ConfirmDeletePersona { pid: String, name: String },
    /// Confirm removing gallery image `img_id` (a `persona_image` row id) from
    /// persona `pid`.
    ConfirmDeleteGalleryImage { pid: String, img_id: String },
    /// Confirm deleting custom emoji `name` from guild `gid`.
    ConfirmDeleteEmoji { gid: String, name: String },
}

/// Composite message cursor `(sent_at, id)` — the same lex-monotonic tie-break
/// key the web client uses (`reading.rs`); never reorder its parts.
pub type Cursor = (String, String);

/// A composer attachment already uploaded to `/media`, awaiting send. `bytes`
/// are kept for an instant local thumbnail (no auth round-trip); `id` is what
/// goes into `SendMessageRequest.attachment_ids`.
#[derive(Clone)]
pub struct StagedAttachment {
    pub id: String,
    pub bytes: bytes::Bytes,
    pub mime: String,
}

#[derive(Clone, Copy)]
pub struct NativeState {
    // Auth gate (pre-shell login/register form)
    /// True once a session is established → render the shell; false → login form.
    pub authed: State<bool>,
    pub auth_user: State<String>,
    pub auth_pass: State<String>,
    /// Login (false) vs register (true) mode for the form.
    pub auth_register: State<bool>,
    /// Last auth error to show under the form (empty = none).
    pub auth_error: State<String>,
    /// An auth request is in flight (disables the submit button).
    pub auth_busy: State<bool>,

    // Sync / identity
    pub me: State<Option<MeResponse>>,
    pub status: State<String>,
    pub polling: State<bool>,

    // Selection
    pub guilds: State<Vec<GuildSummary>>,
    pub sel_server: State<Option<String>>,
    pub channels: State<Vec<ChannelSummary>>,
    pub sel_channel: State<Option<ChannelSummary>>,

    // Message view
    pub messages: State<Vec<MessageEnvelope>>,
    pub cursor: State<Option<Cursor>>,
    pub oldest: State<Option<Cursor>>,
    pub loading_older: State<bool>,
    pub more_history: State<bool>,
    pub seen: State<HashSet<String>>,
    pub typing: State<Vec<String>>,

    /// Bumped on every channel switch; a poll/fetch tagged with a stale epoch
    /// must not ingest into the freshly-switched channel (the web's switch guard).
    pub epoch: State<u64>,

    // Composer / edit (write path)
    pub compose: State<String>,
    /// Id of the message currently being edited inline, if any.
    pub editing: State<Option<String>>,
    pub edit_buf: State<String>,

    // Personas (worn-persona-on-send)
    /// The caller's personas (owned + shared-as-editor), for the picker.
    pub personas: State<Vec<PersonaSummary>>,
    /// The persona worn in the OPEN channel (`None` = speaking as the account).
    /// Restored from the message list's `active_persona` on channel open; sent
    /// with each message so attribution is race-proof (web parity).
    pub active_persona: State<Option<String>>,
    /// Whether the composer's "speaking as" picker panel is open.
    pub persona_menu: State<bool>,

    /// Image attachments uploaded and staged for the next send, in display order.
    pub staged_attachments: State<Vec<StagedAttachment>>,

    /// Custom emoji of the open guild — powers the composer `:`-autocomplete
    /// (and, later, `:name:` render resolution). Reloaded on guild open.
    pub guild_emoji: State<Vec<CustomEmoji>>,

    // ---- Phase 4b: wardrobe + emoji-manager panes ----
    /// Which pane the 3rd column renders (channel / wardrobe / emoji manager).
    pub view: State<NativeView>,
    /// The confirm/edit overlay rendered over the shell, if any (`None` = closed).
    pub modal: State<Option<NativeModal>>,

    // Persona detail-editor working buffers (the `PersonaEditor` modal binds
    // these). Seeded from the grid + `get_persona`/sharing fetches when the
    // editor opens; cleared/ignored when it closes.
    /// Editable persona name.
    pub pe_name: State<String>,
    /// Editable persona description (markup-capable).
    pub pe_description: State<String>,
    /// The persona's name-tint markup palette name (empty = default).
    pub pe_color: State<String>,
    /// The persona's gallery images, loaded on open; reloaded after add/remove.
    pub pe_gallery: State<Vec<crate::protocol::GalleryImage>>,
    /// The persona's current primary-avatar media id (drives the portrait + the
    /// gallery "current" ring), if any.
    pub pe_avatar_id: State<Option<String>>,
    /// Accounts granted editor access (owner-only sharing checklist).
    pub pe_editors: State<Vec<crate::protocol::PersonaEditor>>,
    /// The caller's friends (owner-only sharing checklist source).
    pub pe_friends: State<Vec<crate::protocol::FriendSummary>>,

    // Emoji-manager add-row buffers (the manager pane binds these).
    /// Media id of an uploaded-but-unnamed emoji image, staged for "Add".
    pub emoji_staged_media: State<Option<String>>,
    /// Raw bytes of the staged emoji image, for an instant local preview.
    pub emoji_staged_bytes: State<Option<bytes::Bytes>>,
    /// The new emoji's shortcode name being typed.
    pub emoji_new_name: State<String>,
}

/// Create the root state. MUST be called once, in component context (the app fn).
pub fn use_native_state() -> NativeState {
    NativeState {
        authed: use_state(|| false),
        auth_user: use_state(String::new),
        auth_pass: use_state(String::new),
        auth_register: use_state(|| false),
        auth_error: use_state(String::new),
        auth_busy: use_state(|| false),
        me: use_state(|| None),
        status: use_state(|| "connecting…".to_string()),
        polling: use_state(|| false),
        guilds: use_state(Vec::new),
        sel_server: use_state(|| None),
        channels: use_state(Vec::new),
        sel_channel: use_state(|| None),
        messages: use_state(Vec::new),
        cursor: use_state(|| None),
        oldest: use_state(|| None),
        loading_older: use_state(|| false),
        more_history: use_state(|| true),
        seen: use_state(HashSet::new),
        typing: use_state(Vec::new),
        epoch: use_state(|| 0u64),
        compose: use_state(String::new),
        editing: use_state(|| None),
        edit_buf: use_state(String::new),
        personas: use_state(Vec::new),
        active_persona: use_state(|| None),
        persona_menu: use_state(|| false),
        staged_attachments: use_state(Vec::new),
        guild_emoji: use_state(Vec::new),
        view: use_state(|| NativeView::Channel),
        modal: use_state(|| None),
        pe_name: use_state(String::new),
        pe_description: use_state(String::new),
        pe_color: use_state(String::new),
        pe_gallery: use_state(Vec::new),
        pe_avatar_id: use_state(|| None),
        pe_editors: use_state(Vec::new),
        pe_friends: use_state(Vec::new),
        emoji_staged_media: use_state(|| None),
        emoji_staged_bytes: use_state(|| None),
        emoji_new_name: use_state(String::new),
    }
}
