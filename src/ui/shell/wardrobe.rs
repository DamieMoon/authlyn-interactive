//! The wardrobe pane: create personas and wear / take them off.

use leptos::prelude::*;

use super::{act, Shell};

#[component]
pub(crate) fn WardrobePane(s: Shell) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let desc = RwSignal::new(String::new());
    view! {
        <div class="pane">
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
            <div class="persona-grid">
                {move || s.personas.get().into_iter().map(|p| {
                    let pid = p.id.clone();
                    let pid_worn = pid.clone();
                    let worn = move || s.active_persona.get().as_deref() == Some(pid_worn.as_str());
                    let pid_wear = pid.clone();
                    view! {
                        <div class="persona-card">
                            <span class="pname">{p.name}</span>
                            <Show when=worn
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
                        </div>
                    }
                }).collect_view()}
            </div>
        </div>
    }
}
