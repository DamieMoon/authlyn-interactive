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
