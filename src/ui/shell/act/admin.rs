//! Admin actions: broadcast a "Nova DOT" system message to every guild.

use super::super::Shell;

#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

/// Broadcast `body` as a Nova DOT system message into every live guild's default
/// channel (admin only). Reports the fan-out result — or the error — via
/// `s.composer.status`.
#[cfg(feature = "hydrate")]
pub fn send_system_broadcast(s: Shell, body: String) {
    use crate::protocol::SendSystemMessageRequest;
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::broadcast_system_message(&SendSystemMessageRequest { body }).await {
            Ok(r) => {
                let skipped = if r.guilds_skipped > 0 {
                    format!(" ({} skipped)", r.guilds_skipped)
                } else {
                    String::new()
                };
                s.composer.status.set(format!(
                    "Broadcast sent to {} server(s){skipped}.",
                    r.messages_sent
                ));
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

// ---- ssr stub ----

#[cfg(not(feature = "hydrate"))]
pub fn send_system_broadcast(_s: Shell, _body: String) {}
