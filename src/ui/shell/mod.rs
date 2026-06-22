//! The authed orbit shell: an immersive pill/station landing whose orbit-map
//! overlay is the home surface, with content panes that switch between channel
//! messages, the lorebook editor, the wardrobe, and the friends list. Orbit is
//! the sole + default shell (no rail/sidebar fallback) — the station opens the
//! shared, skeleton-independent Account & Server modals mounted here.
//!
//! State is signal-driven (a `Copy` `Shell` handle); deep-link URLs are a
//! later polish. All data-fetching lives in the `act` module, defined twice —
//! real on hydrate, no-op stubs on ssr — so the view's handlers call it
//! ungated and the gloo-net client never enters the ssr graph.
//!
//! The content panes each live in their own submodule (`channel`, `wardrobe`,
//! `lorebook`, `friends`); this module owns the shared `Shell` state, the
//! orbit shell mount (`AppShell`), and the [`act`] action layer.

use std::collections::{HashMap, HashSet};

use leptos::prelude::*;

use crate::protocol::ListFriendsResponse;

use crate::ui::emoji::EmojiResolver;
use crate::ui::modal::{Modal, ModalHead};
use crate::ui::AuthCtx;

mod account;
mod channel;
mod emoji_manager;
mod friends;
pub mod holopanel;
mod lorebook;
mod members;
mod server;
pub mod sk_orbit;
mod state;
mod toast;
mod wardrobe;

#[cfg(feature = "hydrate")]
pub(crate) use state::COMPOSER_MAX_ATTACHMENTS;
pub(crate) use state::{
    Composer, MessageView, Modals, Notify, Prefs, Selection, Social, SyncState, Toasts, Trash,
};

use account::AccountModal;
use server::ServerModal;
use sk_orbit::SkOrbitShell;
use toast::toast_host;
use wardrobe::WardrobePane;

#[component]
pub fn Home() -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");

    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if !auth.loading.get() && auth.user.get().is_none() {
            leptos_router::hooks::use_navigate()("/login", Default::default());
        }
    });

    view! {
        <Show
            when=move || auth.is_authed()
            fallback=|| view! { <p class="loading">"Loading…"</p> }
        >
            <AppShell/>
        </Show>
    }
}

#[derive(Clone, Copy, PartialEq)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) enum Pane {
    Friends,
    Channel,
    Lorebook,
    Emoji,
    Members,
    /// M7/P1: the DM thread list (demo-grade orbit surface; placement is a
    /// deck-pass decision).
    DirectMessages,
    /// M7/P2: the Guest Cameos list — channels the caller is a guest in
    /// (demo-grade; placement is a deck-pass decision).
    Cameos,
}

/// A destructive action awaiting confirmation. Stored in `Shell::pending_delete`
/// (with a human prompt in `confirm_prompt`); the top-level confirm modal in
/// `AppShell` dispatches the matching `act::` fn when the user confirms. Storing
/// a closure in a signal is awkward in Leptos, so we describe the action as data.
///
/// Message deletes no longer queue here (UX evolution #11): they act
/// instantly with a 6s undo toast instead (`act::delete_message`); the
/// confirm modal stays for the heavier owner-gated restores below.
#[derive(Clone)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) enum PendingDelete {
    Channel { gid: String, cid: String },
    Server { gid: String },
    Persona { pid: String },
}

/// Aggregate of the shell's reactive state, grouped into 10 sub-structs.
///
/// Each sub-struct is also `provide_context`'d in `AppShell` so a deeper
/// component can pull just the slice it needs via `use_context::<Selection>()`
/// (the pane-component migration in M6/C8). `act::*` keeps taking the full
/// aggregate handle so action functions stay short and uncluttered.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) struct Shell {
    pub(crate) sel: Selection,
    pub(crate) msg: MessageView,
    pub(crate) composer: Composer,
    pub(crate) sync: SyncState,
    pub(crate) social: Social,
    pub(crate) modals: Modals,
    pub(crate) notify: Notify,
    pub(crate) trash: Trash,
    pub(crate) prefs: Prefs,
    pub(crate) toasts: Toasts,
}

#[component]
fn AppShell() -> impl IntoView {
    let auth = use_context::<AuthCtx>().expect("AuthCtx provided at root");

    // Construct each state sub-struct, then make each available via
    // `provide_context` so pane components (M6/C8) can pull just the slice
    // they need without taking the full Shell aggregate as a prop.
    let sel = Selection {
        guilds: RwSignal::new(Vec::new()),
        sel_server: RwSignal::new(None),
        sel_owner: RwSignal::new(None),
        channels: RwSignal::new(Vec::new()),
        guild_channels: RwSignal::new(HashMap::new()),
        guild_emoji: RwSignal::new(Vec::new()),
        sel_channel: RwSignal::new(None),
        dms: RwSignal::new(Vec::new()),
        cameos: RwSignal::new(Vec::new()),
    };
    provide_context(sel);

    let msg = MessageView {
        messages: RwSignal::new(Vec::new()),
        cursor: RwSignal::new(None),
        oldest: RwSignal::new(None),
        loading_older: RwSignal::new(false),
        more_history: RwSignal::new(true),
        loading_initial: RwSignal::new(false),
        anchor_to: RwSignal::new(None),
        seen: RwSignal::new(HashSet::new()),
        typing: RwSignal::new(Vec::new()),
        ghost_drafts: RwSignal::new(Vec::new()),
        new_divider: RwSignal::new(None),
    };
    provide_context(msg);

    let composer = Composer {
        compose: RwSignal::new(String::new()),
        compose_attachments: RwSignal::new(Vec::new()),
        status: RwSignal::new(String::new()),
        drafts: RwSignal::new(crate::ui::shell::act::channel::load_drafts()),
        last_used_colors: RwSignal::new(act::load_color_history()),
        replying_to: RwSignal::new(None),
        editing: RwSignal::new(None),
        sent: RwSignal::new(false),
        sent_gen: StoredValue::new(0),
        effect_mode: RwSignal::new(None),
    };
    provide_context(composer);

    let sync = SyncState {
        polling: RwSignal::new(false),
        sse_live: RwSignal::new(false),
        me: RwSignal::new(auth.user.get_untracked().map(|u| u.account_id)),
        pane: RwSignal::new(Pane::Friends),
        wardrobe_open: RwSignal::new(false),
        map_open: RwSignal::new(false),
        switching: RwSignal::new(false),
    };
    provide_context(sync);

    let social = Social {
        friends: RwSignal::new(ListFriendsResponse {
            friends: Vec::new(),
            incoming: Vec::new(),
            outgoing: Vec::new(),
        }),
        personas: RwSignal::new(Vec::new()),
        active_persona: RwSignal::new(None),
        lore: RwSignal::new(Vec::new()),
    };
    provide_context(social);

    let modals = Modals {
        pending_delete: RwSignal::new(None),
        confirm_prompt: RwSignal::new(None),
    };
    provide_context(modals);

    let notify = Notify {
        muted: RwSignal::new(HashSet::new()),
        unread: RwSignal::new(HashSet::new()),
        last_seen: RwSignal::new(HashMap::new()),
        web_push_enabled: RwSignal::new(false),
        scroll_marks: RwSignal::new(act::reentry::load_scroll_marks()),
    };
    provide_context(notify);

    let trash = Trash {
        deleted_channels: RwSignal::new(Vec::new()),
        deleted_messages: RwSignal::new(Vec::new()),
        show_msg_trash: RwSignal::new(false),
    };
    provide_context(trash);

    let prefs = Prefs {
        dialogue_style: RwSignal::new(act::rp_dialogue_style_enabled()),
        ghost_quill: RwSignal::new(act::ghost_quill_enabled()),
        haptic_vibrate: RwSignal::new(act::haptic_vibrate_enabled()),
        // v27 (M5/P2): orbit is the sole + default shell — forced here
        // unconditionally (no ceremony, no chooser, no M3 fallback path). The
        // `skeleton` signal is kept (vestigial reads in channel/* + the
        // skeleton_switch.rs surface) but can no longer vary.
        skeleton: RwSignal::new(Some(act::SKELETON_FALLBACK.to_string())),
    };
    provide_context(prefs);

    let toasts = Toasts {
        current: RwSignal::new(None),
    };
    provide_context(toasts);

    let s = Shell {
        sel,
        msg,
        composer,
        sync,
        social,
        modals,
        notify,
        trash,
        prefs,
        toasts,
    };
    // Make the aggregate available to pane components (M6/C8) so they can drop
    // their `s: Shell` prop in favour of `use_context::<Shell>()`.
    provide_context(s);

    // Keep `s.sync.me` in sync with the auth context (it resolves async after mount).
    Effect::new(move |_| {
        s.sync.me.set(auth.user.get().map(|u| u.account_id));
    });
    // M7/P2 deck-pass fix (UI-5): the inline `composer.status` line is a
    // per-action transient (e.g. a cameo-invite "already a member of this guild"
    // surfaced from MembersPane). It used to bleed across panes — visible while
    // navigating Members -> Friends -> Cameos. Pane navigation is the natural
    // reset point, so clear it on every pane switch. One Effect covers ALL
    // transitions (rail nav + the in-pane back/forward buttons), not just the
    // friends.rs sites. Reading `pane` subscribes; writing `status` doesn't, so
    // there's no feedback loop. Client-only (Effects don't run on ssr).
    Effect::new(move |_| {
        let _ = s.sync.pane.get();
        s.composer.status.set(String::new());
    });
    // Provide the emoji resolver to the whole shell subtree so the markup
    // renderer turns `:shortcode:` into a custom-emoji image or a unicode glyph
    // without threading a parameter through every render call site.
    let emoji_map = Memo::new(move |_| {
        s.sel
            .guild_emoji
            .get()
            .into_iter()
            .map(|e| (e.name, e.media_id))
            .collect::<HashMap<String, String>>()
    });
    provide_context(EmojiResolver::new(emoji_map));
    // M7/D1: kick off the lazy `/emoji.json` fetch at shell mount so the
    // picker and `:shortcode:` resolver are warm by the time the first
    // composer renders. No-op if already loaded or in flight.
    crate::ui::emoji::data::warm();
    // Account-management modal visibility (change password, future options).
    let account_open = RwSignal::new(false);
    // Server-management modal visibility (owner-gated: accent, invitations,
    // channels). Mirrors `account_open` — a shared, skeleton-independent window
    // rendered below and opened from the orbit station's "Server settings"
    // button.
    let server_open = RwSignal::new(false);
    // The invite/manage controls show only to the owner of the open server.
    let is_owner = move || {
        let me = auth.user.get().map(|u| u.account_id);
        me.is_some() && me == s.sel.sel_owner.get()
    };
    // The open guild's accent name (empty = default), derived from the rail
    // list so it auto-updates on a set-accent patch.
    let accent_name = move || {
        let sid = s.sel.sel_server.get();
        s.sel
            .guilds
            .get()
            .into_iter()
            .find(|g| Some(&g.id) == sid.as_ref())
            .map(|g| g.accent_color)
            .unwrap_or_default()
    };

    // A notification deep-link arrives as `/?server=&channel=&m=` (set by the
    // service worker's notificationclick handler). Read it once at mount; when
    // present it wins over the stored-session restore. Router-driven, so it
    // needs no extra web-sys features. None on ssr (the Effect is client-only).
    #[cfg(feature = "hydrate")]
    let deep_link: Option<(String, String, Option<String>)> = {
        let q = leptos_router::hooks::use_query_map().get_untracked();
        match (q.get("channel"), q.get("server")) {
            (Some(cid), Some(gid)) => Some((gid, cid, q.get("m"))),
            _ => None,
        }
    };
    #[cfg(not(feature = "hydrate"))]
    let deep_link: Option<(String, String, Option<String>)> = None;
    #[cfg(feature = "hydrate")]
    let nav = leptos_router::hooks::use_navigate();

    // On mount: load the guild rail, then either follow a notification
    // deep-link or restore the last session (falling back to the Friends home).
    // A deep-linked/restored channel wins; we don't show Friends over it.
    // (No-ops on ssr; the stub `restore_session` returns false so ssr still
    // lands on Friends.)
    Effect::new(move |_| {
        act::refresh_guilds(s);
        if let Some((gid, cid, message)) = deep_link.clone() {
            act::open_deep_link(s, gid, cid, message);
            // Strip the query so a manual refresh doesn't yank us back here.
            #[cfg(feature = "hydrate")]
            nav(
                "/",
                leptos_router::NavigateOptions {
                    replace: true,
                    scroll: false,
                    ..Default::default()
                },
            );
        } else if !act::restore_session(s) {
            act::show_friends(s);
        }
        // Keep the rail/sidebar/friends + open channel live (idempotent).
        act::start_sync(s);
        act::load_muted(s);
        // Load the offline localStorage marks first, then overlay the
        // server-synced read cursors on top (L-1 cross-device sync): a newer
        // server cursor wins, a failed fetch falls back to localStorage.
        act::load_last_seen(s);
        act::hydrate_last_seen(s);
        // Window-focus listener: when the user returns to the tab with a
        // channel already open, clear any tray notifications that arrived
        // for that channel while we were backgrounded (feedback row
        // kx24k2cwftdppidhmh0e).
        #[cfg(feature = "hydrate")]
        act::wire_focus_clears_notifs(s);
        // SW message listener: a push notification clicked from a backgrounded
        // PWA routes via the SW's `client.navigate()`, which throws in some
        // standalone contexts; its fallback posts a NOTIFICATION_CLICK message
        // to this window. Register the listener so that payload deep-links the
        // app instead of being silently dropped (feedback br3ebxgjj1lh3qfbz3n8).
        #[cfg(feature = "hydrate")]
        act::wire_notification_click(s);
    });

    view! {
        <div class="app fx-max"
            class:dialogue-style=move || s.prefs.dialogue_style.get()
            class:sk-orbit=move || s.prefs.skeleton.get().as_deref() == Some("orbit")
            style:--glow-accent=move || crate::ui::accent::accent_glow_css(&accent_name())
            style:--accent=move || crate::ui::accent::accent_var_css(&accent_name())
        >
            // Orbit is the sole + default shell for v27 (M5/P2; owner
            // ruling 2026-06-17 — orbit is the default with NO fallback, and the
            // legacy M3 rail/sidebar/bottom-tabs chrome was retired here).
            // `account_open` / `server_open` are the shared, skeleton-independent
            // windows the station opens (mounted below this branch).
            <SkOrbitShell account_open=account_open server_open=server_open/>


            {move || if account_open.get() {
                view! { <AccountModal s=s open=account_open/> }.into_any()
            } else {
                ().into_any()
            }}

            // Server-management window (owner-gated): the guild-owner sibling of
            // the AccountModal, opened from the orbit station's "Server
            // settings" button. The is_owner gate is a UX affordance; each
            // server route the modal calls re-validates require_manager.
            {move || (server_open.get() && is_owner()).then(|| view! {
                <ServerModal s=s open=server_open/>
            })}

            // Wardrobe popup (F-2): a dismissible Modal — backdrop click, Esc, or
            // the X close it. Auto-closes when a channel is opened (act::open_channel
            // clears `wardrobe_open`). The wide variant widens the dialog for the
            // persona grid; nested modals inside `WardrobePane` (detail editor, info
            // popup) keep their own `stop_propagation` so inner backdrop clicks only
            // dismiss the inner modal, not this one.
            {move || s.sync.wardrobe_open.get().then(|| {
                view! {
                    <Modal class="wardrobe-modal" swipe_close=true
                        close=move || { s.sync.wardrobe_open.set(false); act::show_orbit_map(s); }>
                        <ModalHead title="Wardrobe"
                            on_close=move || { s.sync.wardrobe_open.set(false); act::show_orbit_map(s); }/>
                        <WardrobePane/>
                    </Modal>
                }
            })}

            // Top-level confirm dialog for destructive actions. Shown whenever a
            // `PendingDelete` is queued; backdrop/Cancel clears it without acting,
            // "Delete" dispatches the queued action (see `act::confirm_delete`).
            {move || s.modals.pending_delete.get().is_some().then(|| {
                let prompt = s.modals.confirm_prompt.get().unwrap_or_default();
                view! {
                    <Modal class="confirm-modal" close=move || act::cancel_delete(s)>
                        <h3>"Confirm delete"</h3>
                        <p>{prompt}</p>
                        <div class="confirm-actions">
                            <button on:click=move |_| act::cancel_delete(s)>"Cancel"</button>
                            <button class="danger"
                                on:click=move |_| act::confirm_delete(s)>"Delete"</button>
                        </div>
                    </Modal>
                }
            })}

            // The toast layer (UX evolution #11): one transient glass capsule
            // at a time, fixed above the composer/tab bar. Born for the
            // undo-able message delete; the host renders empty (and eats no
            // taps) while no toast is up.
            {toast_host(s)}

        </div>
    }
}

// ---------------------------------------------------------------------------
// Actions — real on hydrate, no-op stubs on ssr (so the view calls them
// ungated and gloo-net never enters the ssr graph). Defined in `act/` so the
// view stays focused on layout and each action cluster lives in its own file.
// ---------------------------------------------------------------------------

pub mod act;
