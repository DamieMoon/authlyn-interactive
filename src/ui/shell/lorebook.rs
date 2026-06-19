//! The lorebook editor pane: list, add, edit, reorder, and remove lore entries.

use leptos::prelude::*;

#[cfg(feature = "hydrate")]
use leptos::ev::PointerEvent;

use super::{act, Shell};
use crate::ui::icons::{IconCheck, IconCircle, IconClose, IconGrip};

#[component]
pub(crate) fn LorebookPane() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    let keys = RwSignal::new(String::new());
    let content = RwSignal::new(String::new());
    let cid = move || s.sel.sel_channel.get().map(|c| c.id).unwrap_or_default();
    // Finger-drag reorder state shared across entry rows (the wardrobe /
    // channel-manager grip pattern): `drag_from` = the grabbed entry's index,
    // `drag_over` = the entry the finger is currently over. None between drags.
    let drag_from = RwSignal::new(None::<usize>);
    let drag_over = RwSignal::new(None::<usize>);
    view! {
        <div class="pane">
            <h3>"Lorebook"</h3>
            <div class="lore-list">
                {move || {
                    let entries = s.social.lore.get();
                    entries.into_iter().enumerate().map(|(idx, e)| {
                        let entry_cid = cid();
                        let eid = e.id.clone();

                        // Clone ids for each action handler.
                        let cid_toggle = entry_cid.clone();
                        let eid_toggle = eid.clone();
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
                        let content_display = e.content.clone();

                        // Finger-drag reorder handlers (hydrate-only; no-op on
                        // ssr). The grip captures the pointer on `down`, hit-tests
                        // the entry under the finger by bounding box on `move`
                        // (NodeList order = list order, so the row index IS the
                        // target index — no DOM attribute round-trip), and commits
                        // via act::move_lore on `up`. Mirrors channel/manager.rs.
                        #[cfg(feature = "hydrate")]
                        let on_grip_down = move |ev: PointerEvent| {
                            use leptos::wasm_bindgen::JsCast as _;
                            // Stop the press bubbling to an enclosing swipe-close
                            // surface whose pointer engine would set_pointer_capture
                            // on bubble and steal the move stream.
                            ev.stop_propagation();
                            if let Some(el) = ev
                                .current_target()
                                .and_then(|t| t.dyn_into::<leptos::web_sys::Element>().ok())
                            {
                                let _ = el.set_pointer_capture(ev.pointer_id());
                            }
                            drag_from.set(Some(idx));
                            drag_over.set(Some(idx));
                            ev.prevent_default();
                        };
                        #[cfg(feature = "hydrate")]
                        let on_grip_move = move |ev: PointerEvent| {
                            use leptos::wasm_bindgen::JsCast as _;
                            if drag_from.get_untracked().is_none() {
                                return;
                            }
                            // Keep the touch on the reorder instead of scrolling.
                            ev.prevent_default();
                            let y = ev.client_y() as f64;
                            let Some(rows) = leptos::web_sys::window()
                                .and_then(|w| w.document())
                                .and_then(|d| d.query_selector_all(".lore-list .lore-entry").ok())
                            else {
                                return;
                            };
                            for i in 0..rows.length() {
                                let Some(el) = rows
                                    .item(i)
                                    .and_then(|n| n.dyn_into::<leptos::web_sys::Element>().ok())
                                else {
                                    continue;
                                };
                                let r = el.get_bounding_client_rect();
                                if y >= r.top() && y <= r.bottom() {
                                    drag_over.set(Some(i as usize));
                                    break;
                                }
                            }
                        };
                        #[cfg(feature = "hydrate")]
                        let on_grip_up = move |_ev: PointerEvent| {
                            if let (Some(from), Some(to)) =
                                (drag_from.get_untracked(), drag_over.get_untracked())
                            {
                                if from != to {
                                    act::move_lore(s, from, to);
                                }
                            }
                            drag_from.set(None);
                            drag_over.set(None);
                        };

                        // clippy can't always trace captures through the view!
                        // macro (these flow only into attr:/class: closures).
                        let _ = (idx, drag_from, drag_over);

                        view! {
                            <div class="lore-entry"
                                attr:data-idx=move || idx.to_string()
                                class:lore-disabled=move || !enabled
                                class:dragging=move || drag_from.get() == Some(idx)
                                class:drag-over=move || drag_over.get() == Some(idx) && drag_from.get() != Some(idx)>
                                <div class="lore-head">
                                    // Drag-to-reorder grip — replaces the M3 ↑/↓
                                    // buttons (matches wardrobe / channel-manager;
                                    // iOS WebKit ignores HTML5 DnD).
                                    <span class="lore-grip" title="Drag to reorder"
                                        on:pointerdown=move |_ev| {
                                            #[cfg(feature = "hydrate")] on_grip_down(_ev);
                                        }
                                        on:pointermove=move |_ev| {
                                            #[cfg(feature = "hydrate")] on_grip_move(_ev);
                                        }
                                        on:pointerup=move |_ev| {
                                            #[cfg(feature = "hydrate")] on_grip_up(_ev);
                                        }
                                        on:pointercancel=move |_ev| {
                                            #[cfg(feature = "hydrate")] on_grip_up(_ev);
                                        }><IconGrip/></span>
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
