//! The lorebook editor pane: list, add, and remove lore entries.

use leptos::prelude::*;

use super::{act, Shell};

#[component]
pub(crate) fn LorebookPane(s: Shell) -> impl IntoView {
    let keys = RwSignal::new(String::new());
    let content = RwSignal::new(String::new());
    let cid = move || s.sel_channel.get().map(|c| c.id).unwrap_or_default();
    view! {
        <div class="pane">
            <h3>"Lorebook"</h3>
            <div class="lore-list">
                {move || s.lore.get().into_iter().map(|e| {
                    let entry_cid = cid();
                    let eid = e.id.clone();
                    let title = if e.title.is_empty() { e.keys.join(", ") } else { e.title };
                    view! {
                        <div class="lore-entry">
                            <div class="lore-head">
                                <strong>{title}</strong>
                                <button on:click=move |_|
                                    act::delete_lore(s, entry_cid.clone(), eid.clone())>"✕"</button>
                            </div>
                            <div class="lore-content">{e.content}</div>
                        </div>
                    }
                }).collect_view()}
            </div>
            <div class="lore-add">
                <input prop:value=move || keys.get()
                    on:input=move |ev| keys.set(event_target_value(&ev))
                    placeholder="trigger keywords (comma-separated)"/>
                <textarea prop:value=move || content.get()
                    on:input=move |ev| content.set(event_target_value(&ev))
                    placeholder="entry content"></textarea>
                <button on:click=move |_| {
                    let parsed = keys.get_untracked()
                        .split(',')
                        .map(|k| k.trim().to_string())
                        .filter(|k| !k.is_empty())
                        .collect::<Vec<_>>();
                    let body = content.get_untracked();
                    keys.set(String::new());
                    content.set(String::new());
                    act::create_lore(s, cid(), parsed, body);
                }>"Add entry"</button>
            </div>
        </div>
    }
}
