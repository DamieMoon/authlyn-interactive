//! Shared UI state — the native mirror of `src/ui/shell/state.rs`.
//!
//! `RwSignal<T>` → Freya `State<T>` (a `Copy` generational-box signal). The web
//! groups signals into `Selection`/`MessageView`/`SyncState`; here we keep one
//! flat `NativeState` (itself `Copy`, since every `State<T>` is) created once at
//! the root via [`use_native_state`] and passed by value into the view fns.

use freya::prelude::*;
use std::collections::HashSet;

use crate::protocol::{ChannelSummary, GuildSummary, MeResponse, MessageEnvelope};

/// Composite message cursor `(sent_at, id)` — the same lex-monotonic tie-break
/// key the web client uses (`reading.rs`); never reorder its parts.
pub type Cursor = (String, String);

#[derive(Clone, Copy)]
pub struct NativeState {
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
}

/// Create the root state. MUST be called once, in component context (the app fn).
pub fn use_native_state() -> NativeState {
    NativeState {
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
    }
}
