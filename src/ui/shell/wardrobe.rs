//! The wardrobe pane: a gallery of persona *character cards*. Create a persona
//! from the add-row, click a card to open a self-contained detail editor
//! (editable name + description, Save), and wear / take off a persona.
//!
//! Images are deliberately deferred: a persona carries an `avatar_id` on the
//! wire and the backend has avatar/gallery endpoints, but no upload UI is wired
//! here yet. The card reserves a slot (`.card-portrait`) for a future portrait.

use leptos::prelude::*;

use super::{act, Shell};
use crate::ui::markup_view::render_body;

/// Select the full text of the focused input (so the owner can one-click copy
/// the share key). Hydrate-only; a no-op on the server render.
#[cfg(feature = "hydrate")]
fn select_all(ev: &leptos::ev::FocusEvent) {
    use leptos::wasm_bindgen::JsCast;
    if let Some(t) = ev.target() {
        if let Ok(i) = t.dyn_into::<leptos::web_sys::HtmlInputElement>() {
            i.select();
        }
    }
}

#[cfg(not(feature = "hydrate"))]
fn select_all(_ev: &leptos::ev::FocusEvent) {}

#[component]
pub(crate) fn WardrobePane(s: Shell) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let desc = RwSignal::new(String::new());
    // The "add a shared persona by key" input.
    let redeem_key = RwSignal::new(String::new());
    // Which persona's detail editor is open (by id), if any.
    let selected = RwSignal::new(None::<String>);

    view! {
        <div class="pane wardrobe">
            <h3>"Wardrobe"</h3>
            <div class="add-row">
                <input prop:value=move || name.get()
                    on:input=move |ev| name.set(event_target_value(&ev))
                    placeholder="persona name"/>
                <input prop:value=move || desc.get()
                    on:input=move |ev| desc.set(event_target_value(&ev))
                    placeholder="description"/>
                <button on:click=move |_| {
                    let (n, d) = (name.get_untracked(), desc.get_untracked());
                    name.set(String::new());
                    desc.set(String::new());
                    act::create_persona(s, n, d);
                }>"Create persona"</button>
            </div>
            // Redeem a share key → gain edit + wear access to someone else's persona.
            <div class="add-row redeem-row">
                <input prop:value=move || redeem_key.get()
                    on:input=move |ev| redeem_key.set(event_target_value(&ev))
                    placeholder="paste a persona share key"/>
                <button on:click=move |_| {
                    let k = redeem_key.get_untracked();
                    redeem_key.set(String::new());
                    act::redeem_persona(s, k);
                }>"Add by key"</button>
            </div>

            // Detail editor: shown only while a card is selected. Self-contained
            // (no modal library) — an expanded panel above the grid. Branch with
            // `.into_any()` rather than nesting <Show> to keep view depth low.
            {move || match selected.get() {
                Some(pid) => {
                    // The owner flag drives whether the share key + editor roster
                    // appear; seeded from the grid entry for this persona.
                    let owned = s.personas.get_untracked()
                        .into_iter()
                        .find(|p| p.id == pid)
                        .map(|p| p.owned)
                        .unwrap_or(false);
                    view! { <PersonaDetail s=s pid=pid owned=owned selected=selected/> }.into_any()
                }
                None => ().into_any(),
            }}

            <div class="persona-grid">
                {move || s.personas.get().into_iter().map(|p| {
                    view! { <PersonaCard s=s p=p selected=selected/> }
                }).collect_view()}
            </div>
        </div>
    }
}

/// One character card: name prominent, description blurb, a reserved portrait
/// slot (image upload deferred), and a wear/worn toggle. Clicking the card body
/// opens the detail editor.
#[component]
fn PersonaCard(
    s: Shell,
    p: crate::protocol::PersonaSummary,
    selected: RwSignal<Option<String>>,
) -> impl IntoView {
    let pid = p.id.clone();
    let pid_worn = pid.clone();
    let pid_wear = pid.clone();
    let pid_open = pid.clone();
    let pid_remove = pid.clone();
    let worn = Memo::new(move |_| s.active_persona.get().as_deref() == Some(pid_worn.as_str()));
    let desc = p.description.clone();
    let has_desc = !desc.trim().is_empty();
    let owned = p.owned;

    view! {
        <div class="persona-card" class:worn=move || worn.get()>
            // Reserved portrait slot — image upload is deferred (see module docs).
            <div class="card-portrait" title="image upload coming soon">
                {p.name.chars().next().unwrap_or('?').to_uppercase().to_string()}
            </div>
            <button class="card-open" title="Edit persona"
                on:click=move |_| selected.set(Some(pid_open.clone()))>
                <span class="card-name">{p.name.clone()}</span>
                {if has_desc {
                    // Description renders the same markup as chat (#18).
                    view! { <span class="card-desc">{render_body(&desc)}</span> }.into_any()
                } else {
                    view! { <span class="card-desc muted">"No description yet."</span> }.into_any()
                }}
            </button>
            <div class="card-actions">
                <Show when=move || worn.get()
                    fallback=move || {
                        let pid = pid_wear.clone();
                        view! {
                            <button on:click=move |_| act::wear_persona(s, pid.clone())>
                                "Wear"
                            </button>
                        }
                    }>
                    <button class="worn" on:click=move |_| act::unwear(s)>"Worn ✓"</button>
                </Show>
                // Only the owner may delete; editors (key-redeemed) cannot.
                {if owned {
                    view! {
                        <button class="danger" title="remove persona"
                            on:click=move |_| act::remove_persona(s, pid_remove.clone())>
                            "Remove"
                        </button>
                    }.into_any()
                } else {
                    view! { <span class="shared-tag" title="shared with you">"Shared"</span> }
                        .into_any()
                }}
            </div>
        </div>
    }
}

/// The detail editor for one persona: editable name + description with a Save
/// button (calls the PATCH endpoint) and a Close. Seeded from the summary in
/// the grid; on save it reloads the grid and closes itself.
#[component]
fn PersonaDetail(
    s: Shell,
    pid: String,
    owned: bool,
    selected: RwSignal<Option<String>>,
) -> impl IntoView {
    // Seed the form from the current grid entry for this persona.
    let seed = s.personas.get_untracked().into_iter().find(|p| p.id == pid);
    let (seed_name, seed_desc) = seed.map(|p| (p.name, p.description)).unwrap_or_default();

    let edit_name = RwSignal::new(seed_name);
    let edit_desc = RwSignal::new(seed_desc);
    // Owner-only sharing state, loaded on mount; editors leave these empty.
    let share_key = RwSignal::new(None::<String>);
    let editors = RwSignal::new(Vec::<crate::protocol::PersonaEditor>::new());
    if owned {
        act::load_persona_share(s, pid.clone(), share_key, editors);
    }
    // Flipped true by `act::update_persona` on success → closes the editor.
    let done = RwSignal::new(false);
    Effect::new(move |_| {
        if done.get() {
            selected.set(None);
        }
    });

    let pid_save = pid.clone();
    let pid_share = pid.clone();
    view! {
        <div class="persona-detail">
            <div class="detail-head">
                <h4>{if owned { "Edit persona" } else { "Edit shared persona" }}</h4>
                <button class="row-edit" title="close"
                    on:click=move |_| selected.set(None)>"✕"</button>
            </div>
            // Portrait slot — image upload deferred.
            <div class="detail-portrait muted" title="image upload coming soon">
                "Portrait — coming soon"
            </div>
            <label class="field">
                <span>"Name"</span>
                <input prop:value=move || edit_name.get()
                    on:input=move |ev| edit_name.set(event_target_value(&ev))
                    placeholder="persona name"/>
            </label>
            <label class="field">
                <span>"Description"</span>
                <textarea prop:value=move || edit_desc.get()
                    on:input=move |ev| edit_desc.set(event_target_value(&ev))
                    placeholder="describe this character"></textarea>
            </label>
            // Owner-only sharing block: the share key (read-only, selectable) +
            // the editor roster. Editors never reach this branch.
            {if owned {
                view! {
                    <div class="share-block">
                        <label class="field">
                            <span>"Share key"</span>
                            <input class="share-key" readonly=true
                                prop:value=move || share_key.get().unwrap_or_default()
                                on:focus=move |ev| select_all(&ev)/>
                        </label>
                        <div class="editor-list">
                            <span class="muted">"Editors"</span>
                            {move || {
                                let list = editors.get();
                                if list.is_empty() {
                                    view! {
                                        <span class="muted">"No one else yet."</span>
                                    }.into_any()
                                } else {
                                    let pid_share = pid_share.clone();
                                    list.into_iter().map(|e| {
                                        let aid = e.account_id.clone();
                                        let pid_share = pid_share.clone();
                                        view! {
                                            <div class="editor-row">
                                                <span>{e.username}</span>
                                                <button class="danger" title="revoke access"
                                                    on:click=move |_| act::remove_persona_editor(
                                                        s, pid_share.clone(), aid.clone(), editors)>
                                                    "Remove"
                                                </button>
                                            </div>
                                        }
                                    }).collect_view().into_any()
                                }
                            }}
                        </div>
                    </div>
                }.into_any()
            } else {
                ().into_any()
            }}
            <div class="detail-actions">
                <button class="save" on:click=move |_| {
                    act::update_persona(
                        s,
                        pid_save.clone(),
                        edit_name.get_untracked(),
                        edit_desc.get_untracked(),
                        done,
                    );
                }>"Save"</button>
                <button on:click=move |_| selected.set(None)>"Cancel"</button>
            </div>
        </div>
    }
}
