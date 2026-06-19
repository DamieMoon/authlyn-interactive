//! The friends pane: add by username, plus incoming / outgoing / accepted lists.

use leptos::prelude::*;

use super::{act, Shell};

#[component]
pub(crate) fn FriendsPane() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    let username = RwSignal::new(String::new());
    view! {
        <div class="pane">
            <div class="add-row">
                <h3>"Friends"</h3>
                <button on:click=move |_| s.sync.pane.set(super::Pane::DirectMessages)>
                    "Direct messages →"
                </button>
                <button on:click=move |_| s.sync.pane.set(super::Pane::Cameos)>
                    "Cameos →"
                </button>
            </div>
            <div class="add-row">
                <input prop:value=move || username.get()
                    on:input=move |ev| username.set(event_target_value(&ev))
                    placeholder="add by username"/>
                <button on:click=move |_| {
                    let u = username.get_untracked();
                    username.set(String::new());
                    act::add_friend(s, u);
                }>"Add"</button>
            </div>
            {move || {
                let f = s.social.friends.get();
                view! {
                    <ul class="flist">
                        {f.incoming.into_iter().map(|p| {
                            let aid = p.account_id.clone();
                            view! {
                                <li>
                                    <span class="tag in">"wants to add"</span>" "{p.username}" "
                                    <button on:click=move |_| act::accept_friend(s, aid.clone())>"Accept"</button>
                                </li>
                            }
                        }).collect_view()}
                        {f.outgoing.into_iter().map(|p| view! {
                            <li><span class="tag out">"pending"</span>" "{p.username}</li>
                        }).collect_view()}
                        {f.friends.into_iter().map(|p| {
                            let aid = p.account_id.clone();
                            view! {
                                <li>
                                    <span class="tag ok">"friend"</span>" "{p.username}" "
                                    <button on:click=move |_| act::remove_friend(s, aid.clone())>"Remove"</button>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }
            }}
        </div>
    }
}

/// M7/P1 demo-grade DM surface: existing threads + a friend-picker to start a
/// 1:1 or group. Reachable from FriendsPane; the orbit placement + visual
/// treatment are deck-pass decisions (the function is placement-agnostic — a
/// thread opens through the shared ChannelPane). NOT end-to-end encrypted, so
/// there is deliberately no lock/encryption affordance here.
#[component]
pub(crate) fn DirectMessagesPane() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    // Fresh on mount; `message::refresh_lists` keeps it live via ListsChanged.
    act::refresh_dms(s);
    let title = RwSignal::new(String::new());
    let selected = RwSignal::new(std::collections::HashSet::<String>::new());
    view! {
        <div class="pane">
            <div class="add-row">
                <button on:click=move |_| s.sync.pane.set(super::Pane::Friends)>"← Friends"</button>
                <h3>"Direct messages"</h3>
            </div>
            {move || {
                let dms = s.sel.dms.get();
                view! {
                    <ul class="flist">
                        {dms.into_iter().map(|dm| {
                            let dm_open = dm.clone();
                            let tid = dm.id.clone();
                            let label = dm.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| {
                                dm.members.iter().map(|m| {
                                    if m.display_name.is_empty() { m.username.clone() } else { m.display_name.clone() }
                                }).collect::<Vec<_>>().join(", ")
                            });
                            view! {
                                <li>
                                    <button on:click=move |_| act::open_dm(s, dm_open.clone())>{label}</button>
                                    " "
                                    <button on:click=move |_| act::leave_dm(s, tid.clone())>"Leave"</button>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }
            }}
            <h3>"New DM"</h3>
            <div class="add-row">
                <input prop:value=move || title.get()
                    on:input=move |ev| title.set(event_target_value(&ev))
                    placeholder="group title (optional)"/>
                <button on:click=move |_| {
                    let members: Vec<String> = selected.get_untracked().into_iter().collect();
                    if members.is_empty() { return; }
                    let t = title.get_untracked();
                    let t = (!t.trim().is_empty()).then_some(t);
                    title.set(String::new());
                    selected.set(std::collections::HashSet::new());
                    act::create_dm_thread(s, members, t);
                }>"Start DM"</button>
            </div>
            {move || {
                let friends = s.social.friends.get().friends;
                view! {
                    <ul class="flist">
                        {friends.into_iter().map(|p| {
                            let aid = p.account_id.clone();
                            let aid_checked = aid.clone();
                            view! {
                                <li>
                                    <label>
                                        <input type="checkbox"
                                            prop:checked=move || selected.get().contains(&aid_checked)
                                            on:change=move |_| selected.update(|set| {
                                                if !set.insert(aid.clone()) { set.remove(&aid); }
                                            })/>
                                        " "{p.username}
                                    </label>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }
            }}
        </div>
    }
}

/// M7/P2 demo-grade Guest Cameos surface (guest side): the channels the caller is
/// a guest in. Reachable from FriendsPane; a cameo opens through the shared
/// ChannelPane (a cameo channel IS a guild text channel). The orbit placement +
/// visual treatment are deck-pass decisions. NOT end-to-end encrypted — no
/// encryption affordance. The HOST-side invite/revoke lives in MembersPane.
#[component]
pub(crate) fn CameosPane() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    // Fresh on mount; `message::refresh_lists` keeps it live via ListsChanged.
    act::refresh_cameos(s);
    view! {
        <div class="pane">
            <div class="add-row">
                <button on:click=move |_| s.sync.pane.set(super::Pane::Friends)>"← Friends"</button>
                <h3>"Cameos"</h3>
            </div>
            {move || {
                let cameos = s.sel.cameos.get();
                if cameos.is_empty() {
                    return view! { <p class="empty">"No active cameos."</p> }.into_any();
                }
                view! {
                    <ul class="flist">
                        {cameos.into_iter().map(|c| {
                            let c_open = c.clone();
                            let cid = c.channel_id.clone();
                            let label = match &c.guild_name {
                                Some(g) if !g.is_empty() => format!("{} · {}", c.channel_name, g),
                                _ => c.channel_name.clone(),
                            };
                            view! {
                                <li>
                                    <button on:click=move |_| act::open_cameo(s, c_open.clone())>{label}</button>
                                    " "
                                    <button on:click=move |_| act::leave_cameo(s, cid.clone())>"Leave"</button>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }.into_any()
            }}
        </div>
    }
}
