//! W5/P1 onboarding ceremony: the first-run three-way skeleton choice. NO
//! silent default — a pref-less device sees this at first authenticated shell
//! mount (new users AND existing users post-update). Selecting a skeleton
//! persists the pref and sets Prefs.skeleton; the ceremony then dismisses.
//! The localStorage-unavailable fallback is handled in AppShell (boots orbit
//! for the session WITHOUT showing this), per spec §1.

use super::{act, Shell};
use leptos::prelude::*;

/// One selectable skeleton card in the ceremony. `title` is the canon theme
/// proper-noun (kept as-is); `blurb` is English UI copy.
struct SkChoice {
    id: &'static str,
    title: &'static str,
    blurb: &'static str,
}

const CHOICES: &[SkChoice] = &[
    SkChoice {
        id: "orbit",
        title: "Omloppsbana",
        blurb: "Spatial — channels orbit your server; swipe between worlds.",
    },
    SkChoice {
        id: "deck",
        title: "Kortdäck",
        blurb: "Layered — scrub through a deck of channels and servers.",
    },
    SkChoice {
        id: "hud",
        title: "Holoterminal",
        blurb: "Zero-chrome — the stream alone; summon panels from the edges.",
    },
];

// `pub(crate)` under a private `mod ceremony` (not `pub mod`): `Shell` is
// `pub(crate)`, so exposing the component at crate-public reach would trip the
// `private_interfaces` lint (the gate runs clippy `-D warnings`). Matches the
// sibling `AccountModal`, which takes `s: Shell` from a private module too.
#[component]
pub(crate) fn SkeletonCeremony(s: Shell) -> impl IntoView {
    let choose = move |id: &'static str| {
        // Persist + apply. If the write somehow fails here the session still
        // gets the chosen class (the signal is set regardless); the ceremony
        // only ever shows when local_storage_writable() returned true.
        let _saved = act::set_skeleton(id);
        s.prefs.skeleton.set(Some(id.to_string()));
    };
    view! {
        <div class="sk-ceremony-scrim" role="dialog" aria-modal="true" aria-label="Choose your interface">
            <div class="sk-ceremony">
                <h2>"Choose your interface"</h2>
                <p class="muted">"You can change this any time in Account \u{2192} Preferences."</p>
                <div class="sk-ceremony-cards">
                    {CHOICES.iter().map(|c| {
                        let id = c.id;
                        view! {
                            <button class=format!("sk-ceremony-card sk-pick-{id}")
                                on:click=move |_| choose(id)>
                                <span class="sk-ceremony-title">{c.title}</span>
                                <span class="sk-ceremony-blurb">{c.blurb}</span>
                            </button>
                        }
                    }).collect_view()}
                </div>
            </div>
        </div>
    }
}
