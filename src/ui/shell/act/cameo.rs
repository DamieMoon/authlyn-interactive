//! Guest-cameo actions (hydrate-real / ssr no-op). M7/P2.
//!
//! These are the GUEST-side actions that touch the shell's `sel.cameos` signal:
//! list, open, leave. A cameo is a guild text channel the caller is a guest in, so
//! opening one routes through the shared [`super::channel::open_channel`] (kind
//! `'text'` → ChannelPane). The HOST-side invite/revoke is channel-local UI (no
//! shell signal to refresh) and lives inline in the cameo invite component.

use super::super::Shell;

#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use crate::protocol::{CameoSummary, ChannelSummary};
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

/// Project a cameo onto a `ChannelSummary` so the shared channel pane can render
/// it. A cameo channel is a guild text channel; its header name is the channel name.
#[cfg(feature = "hydrate")]
fn cameo_as_channel(c: &CameoSummary) -> ChannelSummary {
    ChannelSummary {
        id: c.channel_id.clone(),
        name: c.channel_name.clone(),
        kind: "text".to_string(),
        position: 0,
    }
}

/// Open a cameo channel in the shared channel pane.
#[cfg(feature = "hydrate")]
pub fn open_cameo(s: Shell, cameo: CameoSummary) {
    super::channel::open_channel(s, cameo_as_channel(&cameo));
}

/// Refetch the caller's active cameos into `sel.cameos`.
#[cfg(feature = "hydrate")]
pub fn refresh_cameos(s: Shell) {
    spawn_local(async move {
        if let Ok(c) = api::list_cameos().await {
            // `try_` (review M-10): the await can resolve after logout.
            let _ = s.sel.cameos.try_set(c.cameos);
        }
    });
}

/// Leave a cameo; if it was the open channel, drop the selection back to the
/// cameo list.
#[cfg(feature = "hydrate")]
pub fn leave_cameo(s: Shell, cid: String) {
    spawn_local(async move {
        match api::leave_cameo(&cid).await {
            Ok(()) => {
                if s.sel
                    .sel_channel
                    .try_get_untracked()
                    .flatten()
                    .map(|c| c.id)
                    .as_deref()
                    == Some(cid.as_str())
                {
                    let _ = s.sel.sel_channel.try_set(None);
                    let _ = s.sync.pane.try_set(super::super::Pane::Cameos);
                }
                refresh_cameos(s);
            }
            Err(e) => {
                let _ = s.composer.status.try_set(api::humanize(&e));
            }
        }
    });
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
pub fn open_cameo(_s: Shell, _cameo: crate::protocol::CameoSummary) {}
#[cfg(not(feature = "hydrate"))]
pub fn refresh_cameos(_s: Shell) {}
#[cfg(not(feature = "hydrate"))]
pub fn leave_cameo(_s: Shell, _cid: String) {}
