//! The guild member-management pane: a roster of the open guild's members.
//!
//! Every member sees a read-only roster (avatar + name + role badge). The
//! guild OWNER additionally gets per-row controls: promote a member to admin /
//! demote an admin back to member, and kick. The owner's own row is fixed
//! (ownership transfer / self-kick are out of scope — the backend rejects
//! both regardless).
//!
//! The roster is LOCAL to this component (`members: RwSignal<Vec<MemberSummary>>`),
//! fetched on mount and whenever the selected guild changes, and refetched
//! after any role change / kick. The fetch + mutations are cfg-split helpers
//! (real on hydrate, no-op on ssr) so the gloo-net client never enters the ssr
//! graph — mirroring `wardrobe.rs`'s inline-action pattern, since the shared
//! `act` module (in mod.rs) is owned by another stream.

use leptos::prelude::*;

use super::Shell;
use crate::protocol::MemberSummary;
use crate::ui::AuthCtx;

// ---------------------------------------------------------------------------
// Member actions (inline, cfg-guarded).
// ---------------------------------------------------------------------------

/// Load the guild's members into `members`, surfacing errors via `s.status`.
#[cfg(feature = "hydrate")]
fn load_members(s: Shell, gid: String, members: RwSignal<Vec<MemberSummary>>) {
    use crate::client::api;
    use leptos::task::spawn_local;
    spawn_local(async move {
        match api::list_members(&gid).await {
            Ok(r) => members.set(r.members),
            Err(e) => s.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(not(feature = "hydrate"))]
fn load_members(_s: Shell, _gid: String, _members: RwSignal<Vec<MemberSummary>>) {}

/// Set a member's role (`"admin"` or `"member"`), then reload the roster.
#[cfg(feature = "hydrate")]
fn set_member_role(
    s: Shell,
    gid: String,
    aid: String,
    role: String,
    members: RwSignal<Vec<MemberSummary>>,
) {
    use crate::client::api;
    use leptos::task::spawn_local;
    s.status.set(String::new());
    spawn_local(async move {
        match api::set_member_role(&gid, &aid, &role).await {
            Ok(()) => load_members(s, gid, members),
            Err(e) => s.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(not(feature = "hydrate"))]
fn set_member_role(
    _s: Shell,
    _gid: String,
    _aid: String,
    _role: String,
    _members: RwSignal<Vec<MemberSummary>>,
) {
}

/// Kick a member, then reload the roster.
#[cfg(feature = "hydrate")]
fn remove_member(s: Shell, gid: String, aid: String, members: RwSignal<Vec<MemberSummary>>) {
    use crate::client::api;
    use leptos::task::spawn_local;
    s.status.set(String::new());
    spawn_local(async move {
        match api::remove_member(&gid, &aid).await {
            Ok(()) => load_members(s, gid, members),
            Err(e) => s.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(not(feature = "hydrate"))]
fn remove_member(_s: Shell, _gid: String, _aid: String, _members: RwSignal<Vec<MemberSummary>>) {}

/// A small circular avatar (or a monogram fallback) for one member.
fn avatar(m: &MemberSummary) -> impl IntoView {
    let label = if m.display_name.trim().is_empty() {
        m.username.clone()
    } else {
        m.display_name.clone()
    };
    match &m.avatar_id {
        Some(id) => {
            let src = format!("/media/{id}?w=64");
            view! { <img class="member-avatar" src=src alt=label/> }.into_any()
        }
        None => {
            let mono = crate::ui::avatar::monogram(&label, '?');
            view! { <span class="member-avatar member-avatar-mono">{mono}</span> }.into_any()
        }
    }
}

#[component]
pub(crate) fn MembersPane(s: Shell) -> impl IntoView {
    let members = RwSignal::new(Vec::<MemberSummary>::new());

    // Current guild id + viewer ownership come from the same Shell signals the
    // other panes use: `s.sel_server` (open guild) and `s.sel_owner` (its owner
    // account id), compared against the authed account from `AuthCtx`. No new
    // Shell field is introduced.
    let gid = move || s.sel_server.get().unwrap_or_default();
    let auth = use_context::<AuthCtx>().expect("AuthCtx");
    let is_owner = move || {
        let me = auth.user.get().map(|u| u.account_id);
        me.is_some() && me == s.sel_owner.get()
    };

    // Fetch on mount and whenever the selected guild changes.
    Effect::new(move |_| {
        let g = gid();
        if !g.is_empty() {
            load_members(s, g, members);
        }
    });

    view! {
        <div class="pane">
            <h3>"Members"</h3>
            <div class="member-list">
                {move || {
                    let owner_view = is_owner();
                    let g = gid();
                    members.get().into_iter().map(|m| {
                        let label = if m.display_name.trim().is_empty() {
                            m.username.clone()
                        } else {
                            m.display_name.clone()
                        };
                        let role = m.role.clone();
                        let is_member_role = role == "member";
                        let is_admin_role = role == "admin";
                        // The owner can mutate every row except the owner's own.
                        let mutable = owner_view && role != "owner";

                        let aid_role = m.account_id.clone();
                        let aid_kick = m.account_id.clone();
                        let g_role = g.clone();
                        let g_kick = g.clone();
                        let next_role = if is_admin_role { "member" } else { "admin" };

                        view! {
                            <div class="member-row">
                                {avatar(&m)}
                                <span class="member-name">{label}</span>
                                <span class=format!("member-role member-role-{role}")>{role.clone()}</span>
                                {mutable.then(|| {
                                    let aid_role = aid_role.clone();
                                    let g_role = g_role.clone();
                                    let aid_kick = aid_kick.clone();
                                    let g_kick = g_kick.clone();
                                    let label_btn = if is_admin_role {
                                        "Demote"
                                    } else {
                                        "Make admin"
                                    };
                                    let _ = is_member_role;
                                    view! {
                                        <span class="member-actions">
                                            <button class="member-role-btn"
                                                on:click=move |_| set_member_role(
                                                    s, g_role.clone(), aid_role.clone(),
                                                    next_role.to_string(), members)>
                                                {label_btn}
                                            </button>
                                            <button class="member-kick" title="kick"
                                                on:click=move |_| remove_member(
                                                    s, g_kick.clone(), aid_kick.clone(), members)>
                                                "✕"
                                            </button>
                                        </span>
                                    }
                                })}
                            </div>
                        }
                    }).collect_view()
                }}
            </div>
        </div>
    }
}
