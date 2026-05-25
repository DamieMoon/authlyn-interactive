//! The wardrobe pane: a gallery of persona *character cards*. Create a persona
//! from the add-row, click a card to open a self-contained detail editor
//! (editable name + description, picture upload, Save), and wear / take off a
//! persona.
//!
//! Each persona carries an `avatar_id` on the wire; when set, the portrait
//! slots render `<img src="/media/{id}">` (inline-styled), otherwise a monogram.
//! The detail editor exposes a file input that uploads to `POST /media` then
//! `PUT /personas/{id}/avatar`.

use leptos::prelude::*;

use super::{act, PendingDelete, Shell};
use crate::ui::markup_view::render_body;

/// A persona portrait: the uploaded avatar image when `avatar_id` is `Some`,
/// otherwise the name's first letter as a monogram. Shared by the card, the
/// detail editor and the info popup. The `<img>` is styled inline so it fills
/// whatever portrait slot it sits in (main.scss is owned by another stream).
fn portrait(avatar_id: &Option<String>, name: &str) -> impl IntoView {
    match avatar_id {
        Some(id) => {
            let src = format!("/media/{id}");
            view! {
                <img src=src alt="persona portrait"
                    style="width:100%;height:100%;object-fit:cover;border-radius:inherit"/>
            }
            .into_any()
        }
        None => {
            let monogram = name
                .chars()
                .next()
                .unwrap_or('?')
                .to_uppercase()
                .to_string();
            view! { {monogram} }.into_any()
        }
    }
}

#[component]
pub(crate) fn WardrobePane(s: Shell) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let desc = RwSignal::new(String::new());
    // Which persona's detail editor is open (by id), if any.
    let selected = RwSignal::new(None::<String>);
    // Which persona's read-only info popup is open (clicking a card name), if any.
    let info = RwSignal::new(None::<crate::protocol::PersonaSummary>);

    view! {
        <div class="pane wardrobe">
            <h3>"Wardrobe"</h3>
            <div class="add-row">
                <input prop:value=move || name.get()
                    on:input=move |ev| name.set(event_target_value(&ev))
                    placeholder="persona name"/>
                <textarea class="add-desc" prop:value=move || desc.get()
                    on:input=move |ev| desc.set(event_target_value(&ev))
                    placeholder="description (Shift+Enter for a new line)"></textarea>
                <button on:click=move |_| {
                    let (n, d) = (name.get_untracked(), desc.get_untracked());
                    name.set(String::new());
                    desc.set(String::new());
                    act::create_persona(s, n, d);
                }>"Create persona"</button>
            </div>
            // Detail editor: a modal shown while a card's Edit is open. Branch
            // with `.into_any()` rather than nesting <Show> to keep view depth low.
            {move || match selected.get() {
                Some(pid) => {
                    // The owner flag drives whether the sharing block appears;
                    // seeded from the grid entry for this persona.
                    let owned = s.personas.get_untracked()
                        .into_iter()
                        .find(|p| p.id == pid)
                        .map(|p| p.owned)
                        .unwrap_or(false);
                    view! { <PersonaDetail s=s pid=pid owned=owned selected=selected/> }.into_any()
                }
                None => ().into_any(),
            }}

            // Read-only info popup — opened by clicking a card's name.
            {move || info.get().map(|p| {
                let desc = (!p.description.trim().is_empty()).then(|| p.description.clone());
                view! {
                    <div class="modal-backdrop" on:click=move |_| info.set(None)>
                        <div class="modal persona-info" on:click=move |_ev| {
                            #[cfg(feature = "hydrate")]
                            _ev.stop_propagation();
                        }>
                            <div class="detail-head">
                                <h4>{p.name.clone()}</h4>
                                <button class="row-edit" title="close"
                                    on:click=move |_| info.set(None)>"✕"</button>
                            </div>
                            <div class="info-portrait" title="persona portrait">
                                {portrait(&p.avatar_id, &p.name)}
                            </div>
                            {match desc {
                                Some(d) => view! { <p class="card-desc">{render_body(&d)}</p> }.into_any(),
                                None => view! { <p class="card-desc muted">"No description."</p> }.into_any(),
                            }}
                        </div>
                    </div>
                }
            })}

            <div class="persona-grid">
                {move || s.personas.get().into_iter().map(|p| {
                    view! { <PersonaCard s=s p=p selected=selected info=info/> }
                }).collect_view()}
            </div>
        </div>
    }
}

/// One character card: name prominent, description blurb, a portrait slot
/// (avatar image or monogram), and a wear/worn toggle. Clicking the name opens
/// a read-only info popup; the Edit button opens the detail editor.
#[component]
fn PersonaCard(
    s: Shell,
    p: crate::protocol::PersonaSummary,
    selected: RwSignal<Option<String>>,
    info: RwSignal<Option<crate::protocol::PersonaSummary>>,
) -> impl IntoView {
    let pid = p.id.clone();
    let pid_worn = pid.clone();
    let pid_wear = pid.clone();
    let pid_edit = pid.clone();
    let pid_remove = pid.clone();
    let pid_leave = pid.clone();
    let worn = Memo::new(move |_| s.active_persona.get().as_deref() == Some(pid_worn.as_str()));
    let desc = p.description.clone();
    let has_desc = !desc.trim().is_empty();
    let owned = p.owned;
    let info_p = p.clone();
    let remove_name = p.name.clone();

    view! {
        <div class="persona-card" class:worn=move || worn.get()>
            // Portrait slot: the uploaded avatar if set, else the monogram.
            <div class="card-portrait" title="persona portrait">
                {portrait(&p.avatar_id, &p.name)}
            </div>
            // Clicking the name/blurb opens the read-only info popup.
            <button class="card-open" title="View persona"
                on:click=move |_| info.set(Some(info_p.clone()))>
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
                // Owner and editor alike may edit (the editor's view hides sharing).
                <button title="edit persona"
                    on:click=move |_| selected.set(Some(pid_edit.clone()))>"Edit"</button>
                // The owner deletes the persona; an editor leaves (drops it from
                // their own list).
                {if owned {
                    view! {
                        <button class="danger" title="delete persona"
                            on:click=move |_| act::ask_delete(
                                s,
                                format!("Delete the persona “{}”? This cannot be undone.", remove_name.clone()),
                                PendingDelete::Persona { pid: pid_remove.clone() },
                            )>
                            "Remove"
                        </button>
                    }.into_any()
                } else {
                    view! {
                        <button class="danger" title="remove from your wardrobe"
                            on:click=move |_| act::leave_shared_persona(s, pid_leave.clone())>
                            "Leave"
                        </button>
                    }.into_any()
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
    // Name used for the monogram fallback in the portrait slot.
    let portrait_name = seed_name.clone();
    // Live avatar for the portrait: re-read `s.personas` so a fresh upload shows
    // without re-opening the editor.
    let pid_portrait = pid.clone();
    let avatar = Memo::new(move |_| {
        s.personas
            .get()
            .into_iter()
            .find(|p| p.id == pid_portrait)
            .and_then(|p| p.avatar_id)
    });

    let edit_name = RwSignal::new(seed_name);
    let edit_desc = RwSignal::new(seed_desc);
    // Owner-only sharing state, loaded on mount: the caller's friends and which
    // of them currently have editor access. Editors leave these empty.
    let friends = RwSignal::new(Vec::<crate::protocol::FriendSummary>::new());
    let editors = RwSignal::new(Vec::<crate::protocol::PersonaEditor>::new());
    if owned {
        act::load_persona_sharing(s, pid.clone(), friends, editors);
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
    let pid_avatar = pid.clone();
    view! {
        // Modal: click the backdrop to close, so a long description can never
        // trap the user. The inner panel scrolls (CSS caps its height).
        <div class="modal-backdrop" on:click=move |_| selected.set(None)>
        <div class="modal persona-detail" on:click=move |_ev| {
            #[cfg(feature = "hydrate")]
            _ev.stop_propagation();
        }>
            <div class="detail-head">
                <h4>{if owned { "Edit persona" } else { "Edit shared persona" }}</h4>
                <button class="row-edit" title="close"
                    on:click=move |_| selected.set(None)>"✕"</button>
            </div>
            // Portrait slot — shows the current avatar (or monogram) and an
            // upload control. Picking a file uploads it and sets it as the
            // avatar; the server gates who may change it.
            <div class="detail-portrait" title="persona portrait">
                {move || portrait(&avatar.get(), &portrait_name)}
            </div>
            <label class="field">
                <span>"Picture"</span>
                <input type="file" accept="image/*"
                    on:change=move |_ev| {
                        #[cfg(feature = "hydrate")]
                        {
                            use leptos::wasm_bindgen::JsCast;
                            if let Some(input) = _ev
                                .target()
                                .and_then(|t| t.dyn_into::<leptos::web_sys::HtmlInputElement>().ok())
                            {
                                if let Some(file) = input.files().and_then(|fl| fl.get(0)) {
                                    act::set_persona_avatar(s, pid_avatar.clone(), file);
                                }
                            }
                        }
                        #[cfg(not(feature = "hydrate"))]
                        act::set_persona_avatar(s, pid_avatar.clone());
                    }/>
            </label>
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
                    placeholder="describe this character (Shift+Enter for a new line)"></textarea>
            </label>
            // Owner-only sharing block: tick a friend to grant edit + wear
            // access, untick to revoke. Editors never reach this branch.
            {if owned {
                view! {
                    <div class="share-block">
                        <span class="muted">"Share with friends"</span>
                        {move || {
                            let granted: Vec<String> = editors.get()
                                .into_iter().map(|e| e.account_id).collect();
                            let list = friends.get();
                            if list.is_empty() {
                                view! {
                                    <span class="muted">"No friends yet — add friends to share."</span>
                                }.into_any()
                            } else {
                                let pid_share = pid_share.clone();
                                list.into_iter().map(|f| {
                                    let aid = f.account_id.clone();
                                    let checked = granted.contains(&aid);
                                    let pid_share = pid_share.clone();
                                    view! {
                                        <label class="share-row">
                                            <input type="checkbox" prop:checked=checked
                                                on:change=move |ev| act::set_persona_share(
                                                    s, pid_share.clone(), aid.clone(),
                                                    event_target_checked(&ev), editors)/>
                                            <span>{f.username}</span>
                                        </label>
                                    }
                                }).collect_view().into_any()
                            }
                        }}
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
        </div>
    }
}
