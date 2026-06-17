//! The lorebook editor pane: list, add, edit, reorder, and remove lore entries.

use leptos::prelude::*;

use super::{act, Shell};
use crate::ui::icons::{IconCheck, IconCircle, IconClose, IconDown, IconUp};

#[component]
pub(crate) fn LorebookPane() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    let keys = RwSignal::new(String::new());
    let content = RwSignal::new(String::new());
    let cid = move || s.sel.sel_channel.get().map(|c| c.id).unwrap_or_default();
    view! {
        <div class="pane">
            <h3>"Lorebook"</h3>
            <div class="lore-list">
                {move || {
                    let entries = s.social.lore.get();
                    let len = entries.len();
                    entries.into_iter().enumerate().map(|(idx, e)| {
                        let entry_cid = cid();
                        let eid = e.id.clone();

                        // Clone ids for each action handler.
                        let cid_toggle = entry_cid.clone();
                        let eid_toggle = eid.clone();
                        let cid_up = entry_cid.clone();
                        let eid_up = eid.clone();
                        let cid_down = entry_cid.clone();
                        let eid_down = eid.clone();
                        let cid_del = entry_cid.clone();
                        let eid_del = eid.clone();
                        let cid_save = entry_cid.clone();
                        let eid_save = eid.clone();

                        // Local edit signals for this entry.
                        let editing = RwSignal::new(false);
                        let edit_title = RwSignal::new(e.title.clone());
                        let edit_keys = RwSignal::new(e.keys.join(", "));
                        let edit_content = RwSignal::new(e.content.clone());

                        let display_title = if e.title.is_empty() {
                            e.keys.join(", ")
                        } else {
                            e.title.clone()
                        };
                        let enabled = e.enabled;
                        let position = e.position;
                        let content_display = e.content.clone();

                        // Suppress spurious "unused" warnings: clippy can't
                        // always trace captures through the view! macro.
                        let _ = (&cid_down, &eid_down, len, idx);

                        view! {
                            <div class="lore-entry" class:lore-disabled=move || !enabled>
                                <div class="lore-head">
                                    // enabled toggle
                                    <button
                                        class="lore-toggle"
                                        title=if enabled { "Disable entry" } else { "Enable entry" }
                                        on:click=move |_|
                                            act::patch_lore(s, cid_toggle.clone(), eid_toggle.clone(),
                                                None, None, None, Some(!enabled), None)>
                                        {if enabled {
                                            view! { <IconCheck/> }.into_any()
                                        } else {
                                            view! { <IconCircle/> }.into_any()
                                        }}
                                    </button>
                                    // reorder up
                                    <button
                                        class="lore-reorder"
                                        title="Move up"
                                        disabled=move || idx == 0
                                        on:click=move |_|
                                            act::swap_lore(s, cid_up.clone(), eid_up.clone(), position, true)>
                                        <IconUp/>
                                    </button>
                                    // reorder down
                                    <button
                                        class="lore-reorder"
                                        title="Move down"
                                        disabled=move || idx == len.saturating_sub(1)
                                        on:click=move |_|
                                            act::swap_lore(s, cid_down.clone(), eid_down.clone(), position, false)>
                                        <IconDown/>
                                    </button>

                                    {move || if editing.get() {
                                        view! {
                                            <input
                                                class="lore-edit-title"
                                                prop:value=move || edit_title.get()
                                                on:input=move |ev| edit_title.set(event_target_value(&ev))
                                                placeholder="title (optional)"/>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <strong class="lore-title">{display_title.clone()}</strong>
                                        }.into_any()
                                    }}

                                    <span class="lore-head-actions">
                                        {move || {
                                            let cid_s = cid_save.clone();
                                            let eid_s = eid_save.clone();
                                            let cid_d = cid_del.clone();
                                            let eid_d = eid_del.clone();
                                            if editing.get() {
                                                view! {
                                                    <button class="lore-save" on:click=move |_| {
                                                        let new_keys: Vec<String> = edit_keys.get_untracked()
                                                            .split(',')
                                                            .map(|k| k.trim().to_string())
                                                            .filter(|k| !k.is_empty())
                                                            .collect();
                                                        act::patch_lore(
                                                            s,
                                                            cid_s.clone(),
                                                            eid_s.clone(),
                                                            Some(edit_title.get_untracked()),
                                                            Some(new_keys),
                                                            Some(edit_content.get_untracked()),
                                                            None,
                                                            None,
                                                        );
                                                        editing.set(false);
                                                    }>"Save"</button>
                                                    <button class="lore-cancel" on:click=move |_| editing.set(false)>"Cancel"</button>
                                                }.into_any()
                                            } else {
                                                view! {
                                                    <button class="lore-edit-btn" on:click=move |_| editing.set(true)>"Edit"</button>
                                                    <button class="lore-delete" on:click=move |_|
                                                        act::delete_lore(s, cid_d.clone(), eid_d.clone())><IconClose/></button>
                                                }.into_any()
                                            }
                                        }}
                                    </span>
                                </div>

                                {move || if editing.get() {
                                    view! {
                                        <div class="lore-edit-fields">
                                            <input
                                                class="lore-edit-keys"
                                                prop:value=move || edit_keys.get()
                                                on:input=move |ev| edit_keys.set(event_target_value(&ev))
                                                placeholder="trigger keywords (comma-separated)"/>
                                            <textarea
                                                class="lore-edit-content"
                                                prop:value=move || edit_content.get()
                                                on:input=move |ev| edit_content.set(event_target_value(&ev))
                                                placeholder="entry content"/>
                                        </div>
                                    }.into_any()
                                } else {
                                    view! {
                                        <div class="lore-content">{content_display.clone()}</div>
                                    }.into_any()
                                }}
                            </div>
                        }.into_any()
                    }).collect::<Vec<_>>()
                }}
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
