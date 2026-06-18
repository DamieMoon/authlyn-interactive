//! Direct-message thread actions (hydrate-real / ssr no-op). M7/P1.
//!
//! A DM thread is a channel, so opening one routes through the shared
//! [`super::channel::open_channel`] (`kind='dm'` → `Pane::Channel`/ChannelPane).
//! The list lives in `sel.dms`, refreshed here and by `message::refresh_lists`
//! (so a server `ListsChanged` from a create/invite/leave repaints it). Errors
//! surface on `composer.status`, the same status line guild actions use.

use super::super::Shell;

#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use crate::protocol::{ChannelSummary, DmSummary};
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

/// Project a DM thread onto a `ChannelSummary` so the shared channel pane can
/// render it. The header name is the group title, else the members' names.
#[cfg(feature = "hydrate")]
fn dm_as_channel(dm: &DmSummary) -> ChannelSummary {
    let name = dm
        .title
        .clone()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            dm.members
                .iter()
                .map(|m| {
                    if m.display_name.is_empty() {
                        m.username.clone()
                    } else {
                        m.display_name.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        });
    ChannelSummary {
        id: dm.id.clone(),
        name,
        kind: "dm".to_string(),
        position: 0,
    }
}

/// Open a DM thread in the shared channel pane.
#[cfg(feature = "hydrate")]
pub fn open_dm(s: Shell, dm: DmSummary) {
    super::channel::open_channel(s, dm_as_channel(&dm));
}

/// Refetch the caller's DM threads into `sel.dms`.
#[cfg(feature = "hydrate")]
pub fn refresh_dms(s: Shell) {
    spawn_local(async move {
        if let Ok(d) = api::list_dms().await {
            // `try_` (review M-10): the await can resolve after logout.
            let _ = s.sel.dms.try_set(d.dms);
        }
    });
}

/// Create a 1:1 (one member) or group (2+) DM with friends, then open it. The
/// server emits `ListsChanged` to every member, so their lists repaint too.
#[cfg(feature = "hydrate")]
pub fn create_dm_thread(s: Shell, members: Vec<String>, title: Option<String>) {
    if members.is_empty() {
        return;
    }
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::create_dm(members, title).await {
            Ok(dm) => {
                refresh_dms(s);
                open_dm(s, dm);
            }
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

/// Invite an accepted friend into a thread the caller belongs to.
#[cfg(feature = "hydrate")]
pub fn invite_to_dm(s: Shell, tid: String, account_id: String) {
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::invite_to_dm(&tid, &account_id).await {
            Ok(_) => refresh_dms(s),
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

/// Leave a thread; if it was the open channel, drop the selection back to the
/// DM list.
#[cfg(feature = "hydrate")]
pub fn leave_dm(s: Shell, tid: String) {
    spawn_local(async move {
        match api::leave_dm(&tid).await {
            Ok(()) => {
                if s.sel
                    .sel_channel
                    .try_get_untracked()
                    .flatten()
                    .map(|c| c.id)
                    .as_deref()
                    == Some(tid.as_str())
                {
                    let _ = s.sel.sel_channel.try_set(None);
                    let _ = s.sync.pane.try_set(super::super::Pane::DirectMessages);
                }
                refresh_dms(s);
            }
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
pub fn open_dm(_s: Shell, _dm: crate::protocol::DmSummary) {}
#[cfg(not(feature = "hydrate"))]
pub fn refresh_dms(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn create_dm_thread(_s: Shell, _members: Vec<String>, _title: Option<String>) {}
#[cfg(not(feature = "hydrate"))]
pub fn invite_to_dm(_s: Shell, _tid: String, _account_id: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn leave_dm(_s: Shell, _tid: String) {}
