//! Wire-shape pins for the SSE `SyncEvent` enum (W1). These are serde-only
//! tests (no server), but live in the integration tree per repo convention.
#![cfg(feature = "ssr")]

use authlyn_interactive::protocol::SyncEvent;

#[tokio::test]
async fn sync_event_serializes_with_snake_case_type_tags() {
    let ev = SyncEvent::MessageCreated {
        channel_id: "abc".into(),
    };
    let json = serde_json::to_string(&ev).unwrap();
    assert_eq!(json, r#"{"type":"message_created","channel_id":"abc"}"#);

    let ev = SyncEvent::ListsChanged;
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r#"{"type":"lists_changed"}"#
    );

    let back: SyncEvent = serde_json::from_str(r#"{"type":"typing","channel_id":"c1"}"#).unwrap();
    assert_eq!(
        back,
        SyncEvent::Typing {
            channel_id: "c1".into()
        }
    );

    let ev = SyncEvent::MessageEdited {
        channel_id: "c1".into(),
        message_id: "m1".into(),
    };
    assert_eq!(
        serde_json::to_string(&ev).unwrap(),
        r#"{"type":"message_edited","channel_id":"c1","message_id":"m1"}"#
    );

    // A future server's unknown event type must decode to Unknown, not error.
    let future: SyncEvent = serde_json::from_str(r#"{"type":"warp_initiated"}"#).unwrap();
    assert_eq!(future, SyncEvent::Unknown);
    assert_eq!(future.channel_id(), None);
}

/// W1.5 wire pins for the account-targeted variants. Both are NEW types on an
/// already-shipped wire: a stale client deserializes them through the
/// `#[serde(other)] Unknown` catch-all (pinned above with `warp_initiated`),
/// so adding them is wire-compatible by construction.
#[tokio::test]
async fn targeted_sync_events_pin_their_wire_shape() {
    // read_state_changed round-trips with its channel id…
    let ev = SyncEvent::ReadStateChanged {
        channel_id: "c1".into(),
    };
    let json = serde_json::to_string(&ev).unwrap();
    assert_eq!(json, r#"{"type":"read_state_changed","channel_id":"c1"}"#);
    let back: SyncEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ev);
    // …but is NOT channel-visibility-scoped: it is account-targeted on the
    // server (the targeted lane bypasses visibility filtering entirely), so
    // `channel_id()` deliberately reports None.
    assert_eq!(ev.channel_id(), None);

    // friends_changed is a bare tag.
    let ev = SyncEvent::FriendsChanged;
    let json = serde_json::to_string(&ev).unwrap();
    assert_eq!(json, r#"{"type":"friends_changed"}"#);
    let back: SyncEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ev);
    assert_eq!(ev.channel_id(), None);
}
