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
use crate::native::state::NativeState;
use crate::protocol::{ChannelSummary, MessageEnvelope};

/// Newest-page size; a short page means the whole channel fits (web parity).
const MESSAGES_PAGE_LIMIT: usize = 100;

/// One-shot mount flow: authenticate, load the profile + guild rail, and open
/// the first guild's first text channel. Fully inline (no nested `spawn`) so it
/// can be awaited from the app's mount task.
pub async fn bootstrap(state: NativeState) {
    let user = std::env::var("AUTHLYN_NATIVE_USER").unwrap_or_else(|_| "native-dev".to_string());
    let pass =
        std::env::var("AUTHLYN_NATIVE_PASS").unwrap_or_else(|_| "native-dev-password".to_string());

    if client().ensure_session(&user, &pass).await.is_err() {
        *state.status.write_unchecked() = "authentication failed".to_string();
        return;
    }
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

    if let Some(g) = guilds.first().cloned() {
        *state.sel_server.write_unchecked() = Some(g.id.clone());
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
    spawn(async move {
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
        ingest(state, l.messages);
    }
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

/// Send `body` to the open channel (as the account; persona-on-send is later).
/// Clears the composer, then pulls the new message in immediately (no poll wait).
pub fn send_message(state: NativeState, body: String) {
    let body = body.trim().to_string();
    if body.is_empty() {
        return;
    }
    let Some(ch) = state.sel_channel.peek().clone() else {
        return;
    };
    *state.compose.write_unchecked() = String::new();
    let epoch = *state.epoch.peek();
    spawn(async move {
        if client()
            .post_message(&ch.id, &body, Vec::new(), None)
            .await
            .is_ok()
        {
            let cursor = state.cursor.peek().clone();
            if let Ok(l) = client().list_messages(&ch.id, cursor.as_ref()).await {
                if *state.epoch.peek() == epoch {
                    ingest(state, l.messages);
                }
            }
        }
    });
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
