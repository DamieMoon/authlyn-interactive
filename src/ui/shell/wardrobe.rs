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

#[cfg(feature = "hydrate")]
use leptos::ev::PointerEvent;

use super::{act, PendingDelete, Shell};
use crate::markup::Color;
use crate::protocol::GalleryImage;
use crate::ui::crest::Crest;
use crate::ui::icons::{IconCheck, IconCircle, IconClose, IconDisc, IconGrip, IconStar};
use crate::ui::markup_view::render_body;
use crate::ui::modal::Modal;

// ---------------------------------------------------------------------------
// Gallery actions (inline, cfg-guarded). The shared `act` module lives in
// mod.rs (owned by another stream), so the per-persona gallery flows are
// implemented here directly, mirroring `act`'s `spawn_local` + `s.composer.status`
// pattern. On ssr these are no-ops so the view still type-checks.
// ---------------------------------------------------------------------------

/// Load a persona's gallery into `gallery`, surfacing errors via `s.composer.status`.
#[cfg(feature = "hydrate")]
fn load_gallery(s: Shell, pid: String, gallery: RwSignal<Vec<GalleryImage>>) {
    use crate::client::api;
    use leptos::task::spawn_local;
    spawn_local(async move {
        match api::get_persona(&pid).await {
            Ok(detail) => gallery.set(detail.gallery),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(not(feature = "hydrate"))]
fn load_gallery(_s: Shell, _pid: String, _gallery: RwSignal<Vec<GalleryImage>>) {}

/// Max gallery images per batch upload (matches the composer's
/// `COMPOSER_MAX_ATTACHMENTS` and the server-side persona-gallery batch sanity
/// ceiling). Drop files past this on the client so the user gets a clean toast
/// instead of an upload-then-server-reject roundtrip.
#[cfg(feature = "hydrate")]
const GALLERY_BATCH_MAX: usize = 100;

/// A picked/pasted gallery file in flight — drives the optimistic upload-in-
/// progress placeholder rendered alongside the loaded gallery thumbnails.
#[cfg(feature = "hydrate")]
#[derive(Clone)]
struct PendingGalleryUpload {
    file_name: String,
}

/// Multi-file gallery upload: client-cap, parallel `upload_media`, then the
/// batch endpoint, then reload the gallery. Shared by the file-picker
/// `on:change` and the `on:paste` handler (B4) so paste is genuinely a thin
/// addition.
///
/// On per-file `upload_media` failure: toast the filename and skip that media
/// id (the rest of the batch still ships). On the final `upload_gallery_images_batch`
/// failure: keep the pending state intact (the upload-in-progress placeholders
/// stay visible) so the user sees the batch did NOT take and can retry.
#[cfg(feature = "hydrate")]
fn gallery_multi_upload(
    s: Shell,
    pid: String,
    files: Vec<web_sys::File>,
    pending: RwSignal<Vec<PendingGalleryUpload>>,
    gallery: RwSignal<Vec<GalleryImage>>,
) {
    use crate::client::api;
    use leptos::task::spawn_local;
    if files.is_empty() {
        return;
    }
    // Client cap — same shape as the composer (W7/B1-client).
    let overflowed = files.len() > GALLERY_BATCH_MAX;
    let files: Vec<web_sys::File> = files.into_iter().take(GALLERY_BATCH_MAX).collect();
    if overflowed {
        s.composer
            .status
            .set(format!("Gallery batch limit ({GALLERY_BATCH_MAX}) reached"));
    } else {
        s.composer.status.set(String::new());
    }
    // Optimistic placeholders: one per accepted file. Cleared on success;
    // retained on batch failure so the user knows the batch didn't ship.
    let placeholders: Vec<PendingGalleryUpload> = files
        .iter()
        .map(|f| PendingGalleryUpload {
            file_name: f.name(),
        })
        .collect();
    pending.update(|v| v.extend(placeholders));
    spawn_local(async move {
        // Parallel uploads — `upload_media` is async, so collecting the futures
        // and `join_all`-ing yields concurrent multipart POSTs without losing
        // input order in the result vector.
        let names: Vec<String> = files.iter().map(|f| f.name()).collect();
        let uploads = files.iter().map(api::upload_media).collect::<Vec<_>>();
        let results = futures_util::future::join_all(uploads).await;
        let mut media_ids: Vec<String> = Vec::with_capacity(results.len());
        for (name, res) in names.iter().zip(results) {
            match res {
                Ok(id) => media_ids.push(id),
                Err(e) => {
                    // Per-file failure: toast the filename + reason. The rest
                    // of the batch still ships (best-effort).
                    s.composer
                        .status
                        .set(format!("Upload failed: {name} — {}", api::humanize(&e)));
                }
            }
        }
        if media_ids.is_empty() {
            // Nothing to commit; drop the placeholders so they don't linger.
            pending.set(Vec::new());
            return;
        }
        match api::upload_gallery_images_batch(&pid, &media_ids).await {
            Ok(_) => {
                pending.set(Vec::new());
                load_gallery(s, pid, gallery);
            }
            Err(e) => {
                // Keep placeholders so the user can retry; surface the batch error.
                s.composer.status.set(api::humanize(&e));
            }
        }
    });
}

// ssr stub: no `web_sys::File` (that crate is hydrate-only). The only call site
// (the file-input `on:change`) is itself hydrate-gated, so on ssr this stub is
// never referenced — keep it for signature parity and silence dead_code.
#[cfg(not(feature = "hydrate"))]
#[allow(dead_code)]
fn gallery_multi_upload(_s: Shell, _pid: String, _gallery: RwSignal<Vec<GalleryImage>>) {}

/// Remove a gallery image, then reload the gallery.
#[cfg(feature = "hydrate")]
fn remove_gallery_image(s: Shell, pid: String, img: String, gallery: RwSignal<Vec<GalleryImage>>) {
    use crate::client::api;
    use leptos::task::spawn_local;
    spawn_local(async move {
        match api::remove_gallery_image(&pid, &img).await {
            Ok(()) => load_gallery(s, pid, gallery),
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(not(feature = "hydrate"))]
fn remove_gallery_image(
    _s: Shell,
    _pid: String,
    _img: String,
    _gallery: RwSignal<Vec<GalleryImage>>,
) {
}

/// Set a gallery image's media as the persona's primary avatar, then reload the
/// wardrobe grid so the portrait updates everywhere.
#[cfg(feature = "hydrate")]
fn set_avatar_from_gallery(s: Shell, pid: String, media_id: String) {
    use crate::client::api;
    use leptos::task::spawn_local;
    s.composer.status.set(String::new());
    spawn_local(async move {
        match api::set_persona_avatar(&pid, &media_id).await {
            Ok(()) => {
                if let Ok(r) = api::list_personas().await {
                    s.social.personas.set(r.personas);
                }
            }
            Err(e) => s.composer.status.set(api::humanize(&e)),
        }
    });
}

#[cfg(not(feature = "hydrate"))]
fn set_avatar_from_gallery(_s: Shell, _pid: String, _media_id: String) {}

/// A persona portrait: the uploaded avatar image when `avatar_id` is `Some`,
/// otherwise (M7/P3) a deterministic heraldic crest derived from the persona's
/// `name` + `debut`. Shared by the card, the detail editor and the info popup.
/// The `<img>` is styled inline so it fills whatever portrait slot it sits in
/// (main.scss is owned by another stream); the crest fills its slot via
/// `style/_crest.scss`.
fn portrait(avatar_id: &Option<String>, name: &str, debut: &str) -> impl IntoView {
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
            // No uploaded avatar → the persona's crest. Two like-named personas
            // differ because the debut date folds into the blazon hash.
            view! { <Crest name=name.to_string() debut=debut.to_string()/> }.into_any()
        }
    }
}

#[component]
pub(crate) fn WardrobePane() -> impl IntoView {
    let s = use_context::<Shell>().expect("Shell provided by AppShell");
    let name = RwSignal::new(String::new());
    let desc = RwSignal::new(String::new());
    // Which persona's detail editor is open (by id), if any.
    let selected = RwSignal::new(None::<String>);
    // Which persona's read-only info popup is open (clicking a card name), if any.
    let info = RwSignal::new(None::<crate::protocol::PersonaSummary>);
    // Client-side search filter over the already-loaded persona list (name +
    // description, case-insensitive). Reorder controls are hidden while a query
    // is active, since card indices then wouldn't map to the full list.
    let search = RwSignal::new(String::new());
    // Finger-drag reorder state, shared across cards (the channel-manager
    // pattern): `drag_from` is the grabbed card's index (set on the grip's
    // pointerdown), `drag_over` the card the finger is currently over (the live
    // drop target). `None` between drags; only live while not filtering (see
    // PersonaCard), so indices map to the full list.
    let drag_from = RwSignal::new(None::<usize>);
    let drag_over = RwSignal::new(None::<usize>);

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
                    let owned = s.social.personas.get_untracked()
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
                    <Modal class="persona-info" close=move || info.set(None)>
                        <div class="detail-head">
                            <h4>{p.name.clone()}</h4>
                            <button class="row-edit" title="close"
                                on:click=move |_| info.set(None)><IconClose/></button>
                        </div>
                        <div class="info-portrait" title="persona portrait">
                            {portrait(&p.avatar_id, &p.name, &p.created_at)}
                        </div>
                        {match desc {
                            Some(d) => view! { <p class="card-desc">{render_body(&d)}</p> }.into_any(),
                            None => view! { <p class="card-desc muted">"No description."</p> }.into_any(),
                        }}
                    </Modal>
                }
            })}

            <input class="persona-search"
                prop:value=move || search.get()
                on:input=move |ev| search.set(event_target_value(&ev))
                placeholder="search personas"/>
            <div class="persona-grid">
                {move || {
                    let q = search.get().trim().to_lowercase();
                    let filtering = !q.is_empty();
                    let all = s.social.personas.get();
                    let len = all.len();
                    all.into_iter()
                        .enumerate()
                        .filter(|(_, p)| {
                            q.is_empty()
                                || p.name.to_lowercase().contains(&q)
                                || p.description.to_lowercase().contains(&q)
                        })
                        .map(|(idx, p)| {
                            view! {
                                <PersonaCard s=s p=p selected=selected info=info
                                    idx=idx len=len reorder=!filtering
                                    drag_from=drag_from drag_over=drag_over/>
                            }
                        })
                        .collect_view()
                }}
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
    // This card's index in the full (unfiltered) wardrobe list, the list
    // length, and whether reorder controls should show (false while searching).
    idx: usize,
    len: usize,
    reorder: bool,
    // Finger-drag reorder (the channel-manager grip pattern): `drag_from` is the
    // grabbed card's index, `drag_over` the card the finger is over. Wired only
    // when `reorder` (filtering off), so indices map to the full list.
    drag_from: RwSignal<Option<usize>>,
    drag_over: RwSignal<Option<usize>>,
) -> impl IntoView {
    let pid = p.id.clone();
    let pid_worn = pid.clone();
    let pid_wear = pid.clone();
    let pid_edit = pid.clone();
    let pid_remove = pid.clone();
    let pid_leave = pid.clone();
    let worn =
        Memo::new(move |_| s.social.active_persona.get().as_deref() == Some(pid_worn.as_str()));
    let desc = p.description.clone();
    let has_desc = !desc.trim().is_empty();
    let owned = p.owned;
    let info_p = p.clone();
    let remove_name = p.name.clone();

    // Finger-drag reorder handlers (hydrate-only; no-op on ssr). The grip
    // captures the pointer on `down`, hit-tests the card under the finger on
    // `move` (elementFromPoint → the `.persona-card[data-idx]` it lands in — a
    // 2-D test for the wrapping grid, vs the channel list's y-only row scan),
    // and commits via `act::move_persona` on `up`. Mirrors `channel/manager.rs`.
    #[cfg(feature = "hydrate")]
    let on_grip_down = move |ev: PointerEvent| {
        use leptos::wasm_bindgen::JsCast as _;
        // Don't let the press bubble to an enclosing swipe-close surface that
        // would set_pointer_capture and steal the move stream (the channel grip
        // guards the same way against the orbit Server window's swipe_close).
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
        if drag_from.get_untracked().is_none() {
            return;
        }
        // Keep the touch on the reorder gesture instead of scrolling the grid.
        ev.prevent_default();
        let Some(doc) = leptos::web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        // elementFromPoint is a pure coordinate hit-test (pointer capture doesn't
        // affect it), so it finds the card under the finger even though the grip
        // captured the pointer. closest() climbs from the hit child to the card.
        if let Some(el) = doc.element_from_point(ev.client_x() as f32, ev.client_y() as f32) {
            if let Ok(Some(card)) = el.closest(".persona-card") {
                if let Some(t) = card
                    .get_attribute("data-idx")
                    .and_then(|v| v.parse::<usize>().ok())
                {
                    drag_over.set(Some(t));
                }
            }
        }
    };
    #[cfg(feature = "hydrate")]
    let on_grip_up = move |_ev: PointerEvent| {
        if let (Some(from), Some(to)) = (drag_from.get_untracked(), drag_over.get_untracked()) {
            if from != to {
                act::move_persona(s, from, to);
            }
        }
        drag_from.set(None);
        drag_over.set(None);
    };

    // Suppress spurious "unused" warnings: clippy can't always trace captures
    // through the view! macro (mirrors the lorebook reorder workaround). `len`
    // is no longer button-driven but kept for the call site; the rest ride the grip.
    let _ = (idx, len, reorder, drag_from, drag_over);

    view! {
        // Finger-drag reorder on the grip (the channel-manager pattern), keyed
        // off `data-idx` for the elementFromPoint hit-test. `.dragging` lifts the
        // grabbed card; `.drag-over` marks the live drop target.
        <div class="persona-card"
            attr:data-idx=move || idx.to_string()
            class:worn=move || worn.get()
            class:dragging=move || drag_from.get() == Some(idx)
            class:drag-over=move || drag_over.get() == Some(idx) && drag_from.get() != Some(idx)>
            // Drag handle — replaces the M3 ↑/↓/⤒/⤓ buttons AND the HTML5 drag
            // (iOS WebKit ignores HTML5 DnD), matching channel reorder (owner
            // directive 2026-06-17). Shown only when not filtering.
            {reorder.then(|| view! {
                <span class="persona-grip" title="Drag to reorder"
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
            })}
            // Portrait slot: the uploaded avatar if set, else the monogram.
            <div class="card-portrait" title="persona portrait">
                {portrait(&p.avatar_id, &p.name, &p.created_at)}
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
                    <button class="worn" on:click=move |_| act::unwear(s)>"Worn "<IconCheck/></button>
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
    let seed = s
        .social
        .personas
        .get_untracked()
        .into_iter()
        .find(|p| p.id == pid);
    let (seed_name, seed_desc, seed_color, seed_debut) = seed
        .map(|p| (p.name, p.description, p.color, p.created_at))
        .unwrap_or_default();
    // Name + debut used for the crest in the portrait slot.
    let portrait_name = seed_name.clone();
    let portrait_debut = seed_debut;
    // Live avatar for the portrait: re-read `s.social.personas` so a fresh upload shows
    // without re-opening the editor.
    let pid_portrait = pid.clone();
    let avatar = Memo::new(move |_| {
        s.social
            .personas
            .get()
            .into_iter()
            .find(|p| p.id == pid_portrait)
            .and_then(|p| p.avatar_id)
    });

    let edit_name = RwSignal::new(seed_name);
    let edit_desc = RwSignal::new(seed_desc);
    // The persona's name-tint (markup palette name, or "" for default).
    let edit_color = RwSignal::new(seed_color);
    // The persona's gallery, loaded on mount; re-loaded after add/remove.
    let gallery = RwSignal::new(Vec::<GalleryImage>::new());
    load_gallery(s, pid.clone(), gallery);
    // Component-scoped (no shell-wide state needed): files awaiting batch
    // commit, rendered as skeleton thumbnails alongside the loaded gallery.
    // Lives next to `gallery` since it shares the same lifetime + reload edge.
    #[cfg(feature = "hydrate")]
    let gallery_pending = RwSignal::new(Vec::<PendingGalleryUpload>::new());
    // A gallery image awaiting a remove confirmation (its id), if any. Local to
    // the editor — the shell-wide `PendingDelete` flow is owned elsewhere, so a
    // gallery thumbnail confirms inline.
    let pending_remove = RwSignal::new(None::<String>);
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
    let pid_gallery_add = pid.clone();
    let pid_gallery_thumbs = pid.clone();
    #[cfg(feature = "hydrate")]
    let pid_gallery_paste = pid.clone();
    view! {
        // Modal: click the backdrop to close, so a long description can never
        // trap the user. The inner panel scrolls (CSS caps its height).
        <Modal class="persona-detail" close=move || selected.set(None)>
            <div class="detail-head">
                <h4>{if owned { "Edit persona" } else { "Edit shared persona" }}</h4>
                <button class="row-edit" title="close"
                    on:click=move |_| selected.set(None)><IconClose/></button>
            </div>
            // Portrait slot — shows the current avatar (or monogram) and an
            // upload control. Picking a file uploads it and sets it as the
            // avatar; the server gates who may change it.
            <div class="detail-portrait" title="persona portrait">
                {move || portrait(&avatar.get(), &portrait_name, &portrait_debut)}
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
            // Gallery: thumbnails of all of this persona's images. Clicking a
            // thumbnail sets it as the primary avatar (the current one is
            // ringed); the ✕ removes it (with an inline confirm). The file
            // input below appends a freshly-uploaded image.
            //
            // `tabindex="0"` makes the gallery region focusable so a user can
            // click into it and Ctrl+V to fan into `gallery_multi_upload`
            // (W7/B4). `on:paste` fires when focus is in the region; text
            // pastes pass through (only `prevent_default()` on image items).
            <div class="field gallery-field" tabindex="0"
                on:paste=move |_ev| {
                    #[cfg(feature = "hydrate")]
                    {
                        let files = crate::ui::clipboard::read_pasted_images(&_ev);
                        if !files.is_empty() {
                            _ev.prevent_default();
                            gallery_multi_upload(
                                s,
                                pid_gallery_paste.clone(),
                                files,
                                gallery_pending,
                                gallery,
                            );
                        }
                    }
                    #[cfg(not(feature = "hydrate"))]
                    let _ = &_ev;
                }>
                <span>"Gallery"</span>
                <div class="gallery-grid">
                    {move || {
                        let imgs = gallery.get();
                        if imgs.is_empty() {
                            return view! {
                                <span class="muted">"No gallery images yet."</span>
                            }.into_any();
                        }
                        let current = avatar.get();
                        let pid_t = pid_gallery_thumbs.clone();
                        imgs.into_iter().map(|g| {
                            let src = format!("/media/{}", g.media_id);
                            let is_avatar = current.as_deref() == Some(g.media_id.as_str());
                            let pid_set = pid_t.clone();
                            let media_set = g.media_id.clone();
                            let img_id = g.id.clone();
                            view! {
                                <div class="gallery-thumb" class:is-avatar=is_avatar>
                                    <button class="gallery-pick"
                                        title=if is_avatar { "Current avatar" } else { "Set as avatar" }
                                        on:click=move |_| set_avatar_from_gallery(
                                            s, pid_set.clone(), media_set.clone())>
                                        <img src=src alt="gallery image"
                                            style="width:100%;height:100%;object-fit:cover;border-radius:inherit"/>
                                        {is_avatar.then(|| view! {
                                            <span class="gallery-badge" title="Current avatar"><IconStar/></span>
                                        })}
                                    </button>
                                    <button class="gallery-remove danger" title="remove image"
                                        on:click=move |_| pending_remove.set(Some(img_id.clone()))>
                                        <IconClose/>
                                    </button>
                                </div>
                            }
                        }).collect_view().into_any()
                    }}
                    // Optimistic upload-in-progress placeholders (W7/B3): one
                    // skeleton thumb per file mid-flight. Cleared on batch
                    // success; retained on batch failure so the user knows the
                    // commit didn't take.
                    {
                        #[cfg(feature = "hydrate")]
                        {
                            view! {
                                {move || {
                                    gallery_pending.get().into_iter().map(|p| view! {
                                        <div class="gallery-thumb gallery-thumb-pending"
                                            title={
                                                let n = p.file_name.clone();
                                                move || format!("Uploading {n}…")
                                            }>
                                            <div class="gallery-pending-skeleton"></div>
                                        </div>
                                    }).collect_view()
                                }}
                            }.into_any()
                        }
                        #[cfg(not(feature = "hydrate"))]
                        { ().into_any() }
                    }
                </div>
            </div>
            <label class="field">
                <span>"Add to gallery"</span>
                // NO `accept` (mirrors the composer's `📎` input): on Android,
                // `accept="image/*"` makes Chrome launch the system photo
                // picker (Google Photos), which on Foxtrot's device picks ONE
                // image at a time even though `multiple` is set. Omitting
                // `accept` gives the generic Files chooser, which honours
                // `multiple`. Non-image picks are dropped client-side below.
                <input type="file" multiple
                    on:change=move |_ev| {
                        #[cfg(feature = "hydrate")]
                        {
                            use leptos::wasm_bindgen::JsCast;
                            if let Some(input) = _ev
                                .target()
                                .and_then(|t| t.dyn_into::<leptos::web_sys::HtmlInputElement>().ok())
                            {
                                if let Some(file_list) = input.files() {
                                    let mut files: Vec<web_sys::File> =
                                        Vec::with_capacity(file_list.length() as usize);
                                    let mut skipped_non_image = false;
                                    for i in 0..file_list.length() {
                                        if let Some(file) = file_list.get(i) {
                                            // Generic picker can return any
                                            // file — gallery is image-only.
                                            if file.type_().starts_with("image/") {
                                                files.push(file);
                                            } else {
                                                skipped_non_image = true;
                                            }
                                        }
                                    }
                                    if skipped_non_image {
                                        s.composer
                                            .status
                                            .set("Skipped non-image files".to_string());
                                    }
                                    if !files.is_empty() {
                                        gallery_multi_upload(
                                            s,
                                            pid_gallery_add.clone(),
                                            files,
                                            gallery_pending,
                                            gallery,
                                        );
                                    }
                                }
                                // Clear so re-picking the same file re-fires change.
                                input.set_value("");
                            }
                        }
                        #[cfg(not(feature = "hydrate"))]
                        let _ = &pid_gallery_add;
                    }/>
            </label>
            // Inline remove confirmation for a gallery image.
            {move || pending_remove.get().map(|img_id| {
                let pid_confirm = pid.clone();
                let img_confirm = img_id.clone();
                view! {
                    <Modal class="confirm-dialog" close=move || pending_remove.set(None)>
                        <p>"Remove this image from the gallery?"</p>
                        <div class="detail-actions">
                            <button class="danger" on:click=move |_| {
                                remove_gallery_image(
                                    s, pid_confirm.clone(), img_confirm.clone(), gallery);
                                pending_remove.set(None);
                            }>"Remove"</button>
                            <button on:click=move |_| pending_remove.set(None)>"Cancel"</button>
                        </div>
                    </Modal>
                }
            })}
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
            // Name color: the palette tinting this persona's name in chat.
            <div class="field">
                <span>"Name color"</span>
                <div class="color-row">
                    <button class="swatch-pick none" title="Default"
                        class:active=move || edit_color.get().is_empty()
                        on:click=move |_| edit_color.set(String::new())><IconCircle/></button>
                    {Color::ALL.into_iter().map(|c| {
                        let name = c.name();
                        let pick = name.to_string();
                        let active_name = name.to_string();
                        view! {
                            <button class=format!("swatch-pick mk-{name}") title=name
                                class:active=move || edit_color.get() == active_name
                                on:click=move |_| edit_color.set(pick.clone())><IconDisc/></button>
                        }
                    }).collect_view()}
                </div>
            </div>
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
                        edit_color.get_untracked(),
                        done,
                    );
                }>"Save"</button>
                <button on:click=move |_| selected.set(None)>"Cancel"</button>
            </div>
        </Modal>
    }
}
