//! Account-management actions: logout + password / security-question /
//! admin-reset. All four are `Shell`-driven (writing status on error) except
//! `logout` which is purely auth-context-driven.

use crate::ui::AuthCtx;

#[cfg(feature = "hydrate")]
use super::super::Shell;
#[cfg(feature = "hydrate")]
use crate::client::api;
#[cfg(feature = "hydrate")]
use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

#[cfg(feature = "hydrate")]
pub fn logout(auth: AuthCtx) {
    let nav = leptos_router::hooks::use_navigate();
    spawn_local(async move {
        let _ = api::logout().await;
        auth.user.set(None);
        nav("/login", Default::default());
    });
}

/// Change the signed-in account's password. The new==confirm check is the
/// caller's (the modal's) job; this just hits the API and reports.
#[cfg(feature = "hydrate")]
pub fn change_password(s: Shell, current: String, new: String) {
    s.status.set(String::new());
    spawn_local(async move {
        match api::change_password(&current, &new).await {
            Ok(()) => s.status.set("password changed".to_string()),
            Err(e) => s.status.set(api::humanize(&e)),
        }
    });
}

/// Set/replace the caller's self-service recovery question + answer.
#[cfg(feature = "hydrate")]
pub fn set_security_question(s: Shell, question: String, answer: String) {
    s.status.set(String::new());
    spawn_local(async move {
        match api::set_security_question(&question, &answer).await {
            Ok(()) => s.status.set("security question saved".to_string()),
            Err(e) => s.status.set(api::humanize(&e)),
        }
    });
}

/// Admin-only: reset another account's password by username.
#[cfg(feature = "hydrate")]
pub fn admin_reset_password(s: Shell, username: String, new_password: String) {
    s.status.set(String::new());
    spawn_local(async move {
        match api::admin_reset_password(&username, &new_password).await {
            Ok(()) => s.status.set(format!("password reset for {username}")),
            Err(e) => s.status.set(api::humanize(&e)),
        }
    });
}

// ---- ssr stubs ----

#[cfg(not(feature = "hydrate"))]
use super::super::Shell;

#[cfg(not(feature = "hydrate"))]
pub fn logout(_auth: AuthCtx) {}
#[cfg(not(feature = "hydrate"))]
pub fn change_password(_s: Shell, _current: String, _new: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn set_security_question(_s: Shell, _question: String, _answer: String) {}
#[cfg(not(feature = "hydrate"))]
pub fn admin_reset_password(_s: Shell, _username: String, _new_password: String) {}
