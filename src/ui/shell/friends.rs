//! The friends pane: add by username, plus incoming / outgoing / accepted lists.

use leptos::prelude::*;

use super::{act, Shell};

#[component]
pub(crate) fn FriendsPane(s: Shell) -> impl IntoView {
    let username = RwSignal::new(String::new());
    view! {
        <div class="pane">
            <h3>"Friends"</h3>
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
