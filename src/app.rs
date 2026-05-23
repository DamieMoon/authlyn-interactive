use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::{
    components::{Route, Router, Routes},
    StaticSegment,
};

use crate::client::DeviceClient;

#[cfg(feature = "hydrate")]
use crate::client::{api, store};
#[cfg(feature = "hydrate")]
use crate::crypto::MegolmCiphertext;
#[cfg(feature = "hydrate")]
use crate::protocol::{CreateRoomRequest, JoinRoomRequest, KeyshareDeposit, SendMessageRequest};
#[cfg(feature = "hydrate")]
use leptos::task::spawn_local;

pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone() />
                <HydrationScripts options/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/authlyn-interactive.css"/>
        <Title text="authlyn"/>

        <Router>
            <main>
                <Routes fallback=|| "Page not found.".into_view()>
                    <Route path=StaticSegment("") view=Chat/>
                </Routes>
            </main>
        </Router>
    }
}

/// One rendered chat line.
#[derive(Clone)]
struct DisplayMsg {
    sender: String,
    text: String,
    ts: String,
}

/// Persist the current client snapshot to localStorage. No-op if there's no
/// device yet or the snapshot fails (recoverable on next reload).
#[cfg(feature = "hydrate")]
fn persist(client: StoredValue<Option<DeviceClient>, LocalStorage>) {
    client.with_value(|c| {
        if let Some(c) = c {
            if let Ok(snap) = c.to_snapshot() {
                store::save(&snap);
            }
        }
    });
}

/// The whole step-10 smoke: device setup, room create/join, Megolm key-share,
/// send, and a poll loop that imports key-shares and decrypts incoming
/// messages. Single page; two browser tabs = two devices.
#[component]
fn Chat() -> impl IntoView {
    use std::collections::HashSet;

    // Crypto state — browser-only mutation, held in the LocalStorage arena so
    // the `!Send` vodozemac sessions don't trip the default `Send + Sync` bound.
    let client = StoredValue::new_local(None::<DeviceClient>);
    let cursor = StoredValue::new_local(None::<(String, String)>);
    let seen_ids = StoredValue::new_local(HashSet::<String>::new());

    // Reactive UI state.
    let my_user = RwSignal::new(String::new());
    let my_device = RwSignal::new(String::new());
    let device_id = RwSignal::new(None::<String>);
    let room_name = RwSignal::new(String::new());
    let room_id = RwSignal::new(String::new());
    let peer_user = RwSignal::new(String::new());
    let peer_device = RwSignal::new(String::new());
    let invite_user = RwSignal::new(String::new());
    let compose = RwSignal::new(String::new());
    let messages = RwSignal::new(Vec::<DisplayMsg>::new());
    let status = RwSignal::new("Generate a device to begin.".to_string());
    let polling = RwSignal::new(false);

    // These are only touched inside browser-only async tasks; keep the ssr
    // build from flagging them unused.
    #[cfg(not(feature = "hydrate"))]
    let _ = (client, cursor, seen_ids, polling);

    // Restore a saved device on mount (browser only).
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if let Some(snap) = store::load() {
            match DeviceClient::from_snapshot(snap) {
                Ok(dc) => {
                    my_user.set(dc.user_id.clone());
                    my_device.set(dc.device_id.clone());
                    device_id.set(Some(dc.device_id.clone()));
                    client.set_value(Some(dc));
                    status.set("Loaded saved device.".to_string());
                }
                Err(e) => status.set(format!("Could not load saved device: {e}")),
            }
        }
    });

    let on_generate = move |_| {
        #[cfg(feature = "hydrate")]
        {
            let mut user = my_user.get_untracked();
            let mut device = my_device.get_untracked();
            if user.trim().is_empty() {
                user = format!("user-{:08x}", rand::random::<u32>());
                my_user.set(user.clone());
            }
            if device.trim().is_empty() {
                device = format!("dev-{:08x}", rand::random::<u32>());
                my_device.set(device.clone());
            }
            spawn_local(async move {
                let mut req = None;
                client.update_value(|c| {
                    let mut dc = DeviceClient::new(user.clone(), device.clone());
                    req = Some(dc.build_bundle_request());
                    *c = Some(dc);
                });
                let req = req.expect("bundle request built above");
                match api::upload_keys(&device, &req).await {
                    Ok(resp) => {
                        device_id.set(Some(resp.device_id.clone()));
                        persist(client);
                        status.set(format!(
                            "Published {} OTKs as device {}",
                            resp.otk_count, resp.device_id
                        ));
                    }
                    Err(e) => status.set(format!("Publish failed: {e}")),
                }
            });
        }
    };

    let on_create_room = move |_| {
        #[cfg(feature = "hydrate")]
        {
            let Some(device) = device_id.get_untracked() else {
                status.set("Generate a device first.".into());
                return;
            };
            let name = room_name.get_untracked();
            if name.trim().is_empty() {
                status.set("Enter a room name.".into());
                return;
            }
            spawn_local(async move {
                match api::create_room(&device, &CreateRoomRequest { name }).await {
                    Ok(r) => {
                        room_id.set(r.id.clone());
                        status.set(format!(
                            "Room created — id {} (share it with the peer)",
                            r.id
                        ));
                    }
                    Err(e) => status.set(format!("Create room failed: {e}")),
                }
            });
        }
    };

    let on_invite = move |_| {
        #[cfg(feature = "hydrate")]
        {
            let Some(device) = device_id.get_untracked() else {
                status.set("Generate a device first.".into());
                return;
            };
            let room = room_id.get_untracked();
            let user = invite_user.get_untracked();
            if room.is_empty() || user.trim().is_empty() {
                status.set("Need a room id and a user to invite.".into());
                return;
            }
            spawn_local(async move {
                match api::join_room(&device, &room, &JoinRoomRequest { user }).await {
                    Ok(_) => status.set("Invited user to room.".into()),
                    Err(e) => status.set(format!("Invite failed: {e}")),
                }
            });
        }
    };

    let on_share = move |_| {
        #[cfg(feature = "hydrate")]
        {
            let Some(device) = device_id.get_untracked() else {
                status.set("Generate a device first.".into());
                return;
            };
            let room = room_id.get_untracked();
            let p_user = peer_user.get_untracked();
            let p_device = peer_device.get_untracked();
            if room.is_empty() || p_user.trim().is_empty() || p_device.trim().is_empty() {
                status.set("Need room id + peer user + peer device.".into());
                return;
            }
            spawn_local(async move {
                let mut sk = None;
                client.update_value(|c| {
                    if let Some(c) = c {
                        sk = Some(c.ensure_room_session(&room));
                    }
                });
                let Some((_sid, session_key)) = sk else {
                    status.set("Generate a device first.".into());
                    return;
                };
                let claim = match api::claim_key(&device, &p_user, &p_device).await {
                    Ok(c) => c,
                    Err(e) => {
                        status.set(format!("Claim peer keys failed: {e}"));
                        return;
                    }
                };
                let mut env = None;
                client.with_value(|c| {
                    if let Some(c) = c {
                        env = Some(c.make_keyshare_envelope(&claim, &session_key));
                    }
                });
                let envelope = match env {
                    Some(Ok(e)) => e,
                    Some(Err(e)) => {
                        status.set(format!("Building key-share envelope failed: {e}"));
                        return;
                    }
                    None => {
                        status.set("No device.".into());
                        return;
                    }
                };
                match api::deposit_keyshare(
                    &device,
                    &room,
                    &KeyshareDeposit {
                        recipient_device: p_device.clone(),
                        envelope,
                    },
                )
                .await
                {
                    Ok(_) => {
                        persist(client);
                        status.set("Shared this room's session key with the peer.".into());
                    }
                    Err(e) => status.set(format!("Key-share deposit failed: {e}")),
                }
            });
        }
    };

    let on_send = move |_| {
        #[cfg(feature = "hydrate")]
        {
            let Some(device) = device_id.get_untracked() else {
                status.set("Generate a device first.".into());
                return;
            };
            let room = room_id.get_untracked();
            let text = compose.get_untracked();
            if room.is_empty() || text.is_empty() {
                return;
            }
            spawn_local(async move {
                let mut enc = None;
                client.update_value(|c| {
                    if let Some(c) = c {
                        enc = Some(c.encrypt_for_room(&room, text.as_bytes()));
                    }
                });
                let (megolm_session_id, ct) = match enc.flatten() {
                    Some(x) => x,
                    None => {
                        status.set("Share a session key into this room first.".into());
                        return;
                    }
                };
                match api::post_message(
                    &device,
                    &room,
                    &SendMessageRequest {
                        megolm_session_id,
                        message_index: ct.message_index,
                        ciphertext: ct.ciphertext,
                    },
                )
                .await
                {
                    Ok(resp) => {
                        seen_ids.update_value(|s| {
                            s.insert(resp.id);
                        });
                        messages.update(|m| {
                            m.push(DisplayMsg {
                                sender: "me".into(),
                                text: text.clone(),
                                ts: "now".into(),
                            })
                        });
                        compose.set(String::new());
                        persist(client);
                    }
                    Err(e) => status.set(format!("Send failed: {e}")),
                }
            });
        }
    };

    // Poll loop: once a device + room are set, drain key-shares and fetch new
    // messages every ~1.5s.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let ready = device_id.get().is_some() && !room_id.get().is_empty();
        if ready && !polling.get_untracked() {
            polling.set(true);
            spawn_local(async move {
                loop {
                    gloo_timers::future::TimeoutFuture::new(1500).await;
                    let Some(device) = device_id.get_untracked() else {
                        continue;
                    };
                    let room = room_id.get_untracked();
                    if room.is_empty() {
                        continue;
                    }

                    // 1. Import any pending key-shares (delete-on-read).
                    if let Ok(inbox) = api::drain_inbox(&device, &room).await {
                        for env in inbox.envelopes {
                            let p_user = peer_user.get_untracked();
                            if p_user.trim().is_empty() {
                                continue;
                            }
                            // The sender's identity Curve25519 key is needed to
                            // bind the inbound Olm session — claim it.
                            if let Ok(claim) =
                                api::claim_key(&device, &p_user, &env.sender_device).await
                            {
                                let curve = claim.identity_curve25519;
                                let mut res = None;
                                client.update_value(|c| {
                                    if let Some(c) = c {
                                        res = Some(c.import_keyshare(&curve, &env.envelope));
                                    }
                                });
                                match res {
                                    Some(Ok(sid)) => {
                                        status.set(format!("Imported inbound session {sid}"))
                                    }
                                    Some(Err(e)) => status.set(format!("Key import failed: {e}")),
                                    None => {}
                                }
                                persist(client);
                            }
                        }
                    }

                    // 2. Fetch new messages since the cursor.
                    let cur = cursor.get_value();
                    if let Ok(list) = api::list_messages(&device, &room, cur).await {
                        for env in list.messages {
                            let already = seen_ids.with_value(|s| s.contains(&env.id));
                            let own = client.with_value(|c| {
                                c.as_ref()
                                    .map(|c| c.is_own(&env.megolm_session_id))
                                    .unwrap_or(false)
                            });
                            if already || own {
                                seen_ids.update_value(|s| {
                                    s.insert(env.id.clone());
                                });
                                cursor.set_value(Some((env.sent_at.clone(), env.id.clone())));
                                continue;
                            }
                            let wire = MegolmCiphertext {
                                message_index: env.message_index,
                                ciphertext: env.ciphertext.clone(),
                            };
                            let mut pt = None;
                            client.update_value(|c| {
                                if let Some(c) = c {
                                    pt = Some(c.decrypt(&env.megolm_session_id, &wire));
                                }
                            });
                            match pt.flatten() {
                                Some(bytes) => {
                                    let text = String::from_utf8_lossy(&bytes).to_string();
                                    messages.update(|m| {
                                        m.push(DisplayMsg {
                                            sender: env.sender_device.clone(),
                                            text,
                                            ts: env.sent_at.clone(),
                                        })
                                    });
                                    seen_ids.update_value(|s| {
                                        s.insert(env.id.clone());
                                    });
                                    cursor.set_value(Some((env.sent_at.clone(), env.id.clone())));
                                    persist(client);
                                }
                                None => {
                                    // Unknown session — leave the cursor put and
                                    // retry next tick once the key-share imports.
                                    status.set(
                                        "Waiting for a session key to decrypt incoming messages…"
                                            .into(),
                                    );
                                    break;
                                }
                            }
                        }
                    }
                }
            });
        }
    });

    view! {
        <h1>"authlyn"</h1>
        <p class="status">{move || status.get()}</p>

        <section>
            <h2>"1. Device"</h2>
            <label>"user id " <input prop:value=move || my_user.get()
                on:input=move |ev| my_user.set(event_target_value(&ev))
                placeholder="auto"/></label>
            <label>" device id " <input prop:value=move || my_device.get()
                on:input=move |ev| my_device.set(event_target_value(&ev))
                placeholder="auto"/></label>
            <button on:click=on_generate>"Generate & publish device"</button>
            <p>"Active device: "
                <code>{move || device_id.get().unwrap_or_else(|| "(none)".into())}</code>
            </p>
        </section>

        <section>
            <h2>"2. Room"</h2>
            <label>"name " <input prop:value=move || room_name.get()
                on:input=move |ev| room_name.set(event_target_value(&ev))/></label>
            <button on:click=on_create_room>"Create room"</button>
            <br/>
            <label>"room id " <input prop:value=move || room_id.get()
                on:input=move |ev| room_id.set(event_target_value(&ev))
                placeholder="paste shared room id"/></label>
            <br/>
            <label>"invite user " <input prop:value=move || invite_user.get()
                on:input=move |ev| invite_user.set(event_target_value(&ev))/></label>
            <button on:click=on_invite>"Invite to room"</button>
        </section>

        <section>
            <h2>"3. Key share"</h2>
            <label>"peer user " <input prop:value=move || peer_user.get()
                on:input=move |ev| peer_user.set(event_target_value(&ev))/></label>
            <label>" peer device " <input prop:value=move || peer_device.get()
                on:input=move |ev| peer_device.set(event_target_value(&ev))/></label>
            <button on:click=on_share>"Share session key with peer"</button>
        </section>

        <section>
            <h2>"4. Messages"</h2>
            <ul class="messages">
                {move || messages.get().into_iter().map(|m| view! {
                    <li>
                        <strong>{m.sender}": "</strong>
                        {m.text}
                        " "<em>{m.ts}</em>
                    </li>
                }).collect_view()}
            </ul>
            <input prop:value=move || compose.get()
                on:input=move |ev| compose.set(event_target_value(&ev))
                placeholder="type a message"/>
            <button on:click=on_send>"Send"</button>
        </section>
    }
}
