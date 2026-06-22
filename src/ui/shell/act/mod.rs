//! Action layer for the shell — each submodule is hydrate-real + ssr-stub
//! co-located, so the view's handlers call `act::xxx` ungated and the gloo-net
//! client never enters the ssr graph.
//!
//! Grouped by responsibility:
//! - [`prefs`] — localStorage toggle helpers (confirm-delete, compose-preview,
//!   dialogue-style).
//! - [`account`] — logout, change-password, admin-reset, profile (display name + avatar).
//! - [`guild`] — guild rail: refresh, swap, open, create/rename/delete/restore.
//! - [`channel`] — channel sidebar: open (incl. deep link + session restore),
//!   create/rename/delete/swap/restore.
//! - [`compose_colors`] — composer quick-swap color-swatch history
//!   (move-to-front/dedup/cap + localStorage load/save).
//! - [`message`] — message read/write: send/edit/delete, the 3-cursor pagination
//!   loop, sync/ingest/unseen, the background poll, mute/last-seen, lore +
//!   friends + member ops + the destructive-action confirm dispatcher.
//! - [`sync`] — the M1 sync driver (`start_sync`): EventSource on `/events`
//!   dispatching `message`'s refresh primitives, with the poll loop as the
//!   automatic fallback.
//! - [`persona`] — wardrobe ops: create/update/remove/leave/swap/share/avatar +
//!   wear/unwear.
//! - [`reentry`] — re-entry aids (UX evolution #9): the unread-frontier NEW
//!   divider baseline, date-separator labels, and per-channel scroll memory
//!   (pure decision fns + localStorage / DOM capture).
//! - [`toast`] — the one-at-a-time toast capsule: push/keyed-dismiss + the
//!   action dispatcher (UX evolution #11).
//! - [`emoji`] — guild custom-emoji refresh/create/delete + image upload.
//! - [`feedback`] — feedback submit/archive + context builder.
//! - [`notify`] — Web Notifications + Web Push (the ~250-line reflection blob).
//!
//! M5/P1: this module is `pub` (was `mod act`) so the theme-switch guards in
//! `tests/skeleton_switch.rs` can reach the skeleton-pref helpers at the stable
//! path `ui::shell::act::*` (the plan's public test path). Most action fns take
//! the crate-internal `Shell` (a `pub(crate)` type), so lifting the module to
//! `pub` makes `private_interfaces` fire on every `act::*(s: Shell)` signature.
//! That is lint noise, not a real leak: `Shell`/`PendingDelete`/`ToastAction`
//! stay `pub(crate)`, so no external crate can ever name them or call these fns
//! with a real argument — only same-workspace integration tests reach the pref
//! helpers, which take no `Shell`. Allow it module-wide rather than churn ~86
//! signatures or widen `Shell` to `pub`.
#![allow(private_interfaces)]

pub mod account;
pub mod admin;
pub mod cameo;
pub mod channel;
pub mod compose_colors;
pub mod dm;
pub mod emoji;
pub mod feedback;
pub mod guild;
pub mod haptics;
pub mod message;
pub mod notify;
pub mod persona;
pub mod prefs;
pub mod reentry;
pub mod sync;
pub mod toast;

// Re-exports so the view code keeps calling `act::xxx` unchanged.
pub use account::{
    admin_reset_password, change_password, logout, save_display_name, set_account_avatar,
};
pub use admin::send_system_broadcast;
pub use cameo::{leave_cameo, open_cameo, refresh_cameos};
pub use channel::{
    create_channel, move_channel_to_bounds, open_channel, open_deep_link, rename_channel,
    restore_channel, restore_session, show_orbit_map, swap_channel,
};
pub(crate) use compose_colors::{load_color_history, record_color, save_color_history};
pub use dm::{create_dm_thread, invite_to_dm, leave_dm, open_dm, refresh_dms};
// `move_channel` (drag drop target) is only reached from a hydrate-gated drag
// handler; re-exporting it on ssr fires dead-code since nothing calls it there.
#[cfg(feature = "hydrate")]
pub use channel::move_channel;
pub use emoji::{create_guild_emoji, delete_guild_emoji, upload_emoji_image};
pub use feedback::{archive_feedback, build_feedback_context, submit_feedback};
pub use guild::{
    create_server, move_guild_to_bounds, open_server, refresh_guilds, rename_server,
    select_server_for_sheet, set_guild_accent, set_guild_icon, swap_guild,
};
// `move_guild` (drag drop target) is hydrate-only — see `move_channel` above.
#[cfg(feature = "hydrate")]
pub use guild::move_guild;
pub use message::{
    accept_friend, add_compose_attachment, add_friend, ask_delete, cancel_delete, cancel_edit,
    cancel_reply, confirm_delete, copy_message_body, create_lore, delete_lore, delete_message,
    hydrate_last_seen, invite_member, load_deleted_channels, load_deleted_messages, load_last_seen,
    load_muted, move_lore, patch_lore, remove_compose_attachment, remove_friend,
    restore_deleted_message, retry_compose_attachment, send_message, show_dms, show_emoji_manager,
    show_friends, show_members, show_wardrobe, start_edit, start_reply, swap_lore, toggle_mute,
};
pub use sync::start_sync;
pub use toast::run_toast_action;
// `load_older` is only reachable through a hydrate-gated branch in `channel`;
// re-exporting it on ssr fires "unused import" because nothing calls it there.
#[cfg(feature = "hydrate")]
pub use message::load_older;
// `add_compose_attachments` (batch picker upload) is hydrate-only — the ssr
// picker branch calls the single-file `add_compose_attachment` stub instead.
#[cfg(feature = "hydrate")]
pub use message::add_compose_attachments;
// Notification-tray bookkeeping (feedback row kx24k2cwftdppidhmh0e). Hydrate-
// only. `clear_notifs_for_channel` is referenced via `super::notify::…` from
// `channel.rs`; only the one-time AppShell-mount installer is re-exported.
#[cfg(feature = "hydrate")]
pub use notify::wire_focus_clears_notifs;
// SW `NOTIFICATION_CLICK` postMessage listener: deep-links the app when a push
// notification is clicked from a backgrounded PWA and the SW's `client.navigate`
// fallback fires (feedback row br3ebxgjj1lh3qfbz3n8). Hydrate-only.
#[cfg(feature = "hydrate")]
pub use notify::wire_notification_click;
pub use persona::{
    create_persona, leave_shared_persona, load_persona_sharing, move_persona_to_bounds,
    set_persona_avatar, set_persona_share, swap_persona, unwear, update_persona, wear_persona,
};
// `move_persona` (drag drop target) is hydrate-only — see `move_channel` above.
#[cfg(feature = "hydrate")]
pub use persona::move_persona;
pub use prefs::{
    clear_skeleton, compose_preview_enabled, ghost_quill_enabled, is_valid_skeleton,
    local_storage_writable, rp_dialogue_style_enabled, set_compose_preview, set_ghost_quill,
    set_rp_dialogue_style, set_skeleton, skeleton_pref, SKELETON_FALLBACK, SKELETON_IDS,
};
// M5/P0 #19 Visual Haptics. `vh` is the hydrate-real fire helper (no ssr stub —
// the ssr graph never animates a DOM element) and `Vh` is its kind argument, so
// both are re-exported hydrate-only (like `move_channel` above — re-exporting on
// ssr fires unused-import since nothing there fires a visual haptic). The
// localStorage toggle helpers ARE ungated: `haptic_vibrate_enabled` seeds the
// Prefs signal and `set_haptic_vibrate` backs the account toggle, both of which
// compile in the ssr graph too.
pub use haptics::{haptic_vibrate_enabled, set_haptic_vibrate};
#[cfg(feature = "hydrate")]
pub use haptics::{vh, Vh};
