//! Action layer for the shell — each submodule is hydrate-real + ssr-stub
//! co-located, so the view's handlers call `act::xxx` ungated and the gloo-net
//! client never enters the ssr graph.
//!
//! Grouped by responsibility:
//! - [`prefs`] — localStorage toggle helpers (confirm-delete, compose-preview,
//!   dialogue-style).
//! - [`account`] — logout + password/security-question/admin-reset.
//! - [`guild`] — guild rail: refresh, swap, open, create/rename/delete/restore.
//! - [`channel`] — channel sidebar: open (incl. deep link + session restore),
//!   create/rename/delete/swap/restore.
//! - [`message`] — message read/write: send/edit/delete, the 3-cursor pagination
//!   loop, sync/ingest/unseen, the background poll, mute/last-seen, lore +
//!   friends + member ops + the destructive-action confirm dispatcher.
//! - [`persona`] — wardrobe ops: create/update/remove/leave/swap/share/avatar +
//!   wear/unwear.
//! - [`emoji`] — guild custom-emoji refresh/create/delete + image upload.
//! - [`feedback`] — feedback submit/archive + context builder.
//! - [`notify`] — Web Notifications + Web Push (the ~250-line reflection blob).

pub mod account;
pub mod channel;
pub mod emoji;
pub mod feedback;
pub mod guild;
pub mod message;
pub mod notify;
pub mod persona;
pub mod prefs;

// Re-exports so the view code keeps calling `act::xxx` unchanged.
pub use account::{admin_reset_password, change_password, logout, set_security_question};
pub use channel::{
    create_channel, move_channel_to_bounds, open_channel, open_deep_link, rename_channel,
    restore_channel, restore_session, swap_channel,
};
// `move_channel` (drag drop target) is only reached from a hydrate-gated drag
// handler; re-exporting it on ssr fires dead-code since nothing calls it there.
#[cfg(feature = "hydrate")]
pub use channel::move_channel;
pub use emoji::{create_guild_emoji, delete_guild_emoji, upload_emoji_image};
pub use feedback::{archive_feedback, build_feedback_context, submit_feedback};
pub use guild::{
    create_server, load_deleted_guilds, move_guild_to_bounds, open_server, refresh_guilds,
    rename_server, restore_deleted_guild, swap_guild,
};
// `move_guild` (drag drop target) is hydrate-only — see `move_channel` above.
#[cfg(feature = "hydrate")]
pub use guild::move_guild;
pub use message::{
    accept_friend, add_compose_attachment, add_friend, ask_delete, cancel_delete, confirm_delete,
    copy_message_body, create_lore, delete_lore, delete_message, edit_message, guild_has_unread,
    hydrate_last_seen, invite_member, load_deleted_channels, load_deleted_messages, load_last_seen,
    load_muted, patch_lore, remove_compose_attachment, remove_friend, restore_deleted_message,
    send_message, show_emoji_manager, show_friends, show_members, show_wardrobe, start_sync,
    swap_lore, toggle_mute,
};
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
pub use persona::{
    create_persona, leave_shared_persona, load_persona_sharing, move_persona_to_bounds,
    set_persona_avatar, set_persona_share, swap_persona, unwear, update_persona, wear_persona,
};
// `move_persona` (drag drop target) is hydrate-only — see `move_channel` above.
#[cfg(feature = "hydrate")]
pub use persona::move_persona;
pub use prefs::{
    compose_preview_enabled, confirm_delete_message_enabled, rp_dialogue_style_enabled,
    set_compose_preview, set_confirm_delete_message, set_rp_dialogue_style,
};
