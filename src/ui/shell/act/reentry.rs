//! Re-entry aids (UX evolution #9): the unread-frontier "NEW" divider, the
//! date-separator labels, and per-channel scroll memory.
//!
//! Pure decision fns ([`first_past_baseline`], [`utc_date_label`]) compile in both
//! graphs and unit-test under ssr; the localStorage / DOM pieces follow the
//! house hydrate-real + ssr-stub pairing (the `compose_colors` pattern).
//! Nothing here ever WRITES read state: the divider only READS the prior
//! last-seen cursor captured at channel open, and the scroll marks are
//! render-side memory — `mark_read` / `set_last_seen` semantics are untouched
//! (the M3 cross-device unread-wipe review finding is the standing warning).

use std::collections::HashMap;

use crate::protocol::MessageEnvelope;

#[cfg(feature = "hydrate")]
use super::super::Shell;
#[cfg(feature = "hydrate")]
use gloo_storage::{LocalStorage, Storage};
#[cfg(feature = "hydrate")]
use leptos::prelude::*;

/// Anchor sentinel for "scroll to the NEW divider": stored into
/// `MessageView::anchor_to` by the unread jump and resolved by the channel
/// pane's anchor effect to the divider row's own dom id (which is this same
/// string). Deliberately NOT `msg-`-prefixed, so it can never collide with —
/// or be resolved as — a real message row by the delegated radial handlers or
/// the message-anchor lookups (sentinel discipline, the skeleton-row rule).
pub(crate) const NEW_DIVIDER_ANCHOR: &str = "new-divider";

/// Id of the first message strictly past the `prior` last-seen BASELINE — the
/// row the NEW divider renders above (and L-4's unread-jump target). Named
/// for the baseline, NOT "first_unread": the pane-local `first_unread_id`
/// signal in `channel/mod.rs` is a different frontier (session appends while
/// scrolled up) and the two must never be conflated (review). Strict
/// composite `(sent_at, id)` tuple compare matching the server cursor's
/// tie-break exactly: `sent_at` is the fixed-9-digit lex-monotonic RFC 3339
/// shape, so String ordering is correct — the same rule `hydrate_last_seen`
/// relies on. The list is composite-ordered ASC and never re-sorted
/// client-side, so the first match IS the frontier. Pure; unit-tested below.
pub(crate) fn first_past_baseline(
    msgs: &[MessageEnvelope],
    prior: &(String, String),
) -> Option<String> {
    msgs.iter()
        .find(|m| (m.sent_at.as_str(), m.id.as_str()) > (prior.0.as_str(), prior.1.as_str()))
        .map(|m| m.id.clone())
}

/// The date part of an RFC 3339 timestamp (`YYYY-MM-DD`), no timezone math —
/// the ssr fallback for [`date_label`] and the hydrate parse-failure
/// fallback. Unparseable input passes through unchanged (the
/// `format_local_time` rule: never render an "Invalid Date"). Pure;
/// unit-tested below.
pub(crate) fn utc_date_label(sent_at: &str) -> &str {
    sent_at.split('T').next().unwrap_or(sent_at)
}

/// The viewer-LOCAL calendar date of `sent_at` as an ISO `YYYY-MM-DD` label —
/// the date-separator text. ISO is deliberate: identical on every device
/// (locale-stable, no Intl variance between the table's phones) and the
/// native Swedish date shape. Hydrate parses through `js_sys::Date` so the
/// day boundary is the VIEWER's local midnight; ssr has no browser timezone
/// and falls back to the UTC date part (the shell only renders client-side —
/// same note as `format_local_time`).
#[cfg(feature = "hydrate")]
pub(crate) fn date_label(sent_at: &str) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(sent_at));
    if date.get_time().is_nan() {
        return utc_date_label(sent_at).to_string();
    }
    format!(
        "{:04}-{:02}-{:02}",
        date.get_full_year(),
        // 0-based month, 1-based day-of-month — JavaScript's Date quirk.
        date.get_month() + 1,
        date.get_date()
    )
}

#[cfg(not(feature = "hydrate"))]
pub(crate) fn date_label(sent_at: &str) -> String {
    utc_date_label(sent_at).to_string()
}

// ---- per-channel scroll memory (hydrate-real; the drafts localStorage
// pattern). Capture/restore are hydrate-only with NO ssr stubs: their only
// caller is the hydrate-gated `open_channel_at`. ----

/// localStorage key for the per-channel scroll marks (channel id → the
/// message row id last at the top of the viewport). Absent → no memory yet.
#[cfg(feature = "hydrate")]
const KEY_SCROLL_MARKS: &str = "authlyn.scroll_marks";

/// Px slack under which leaving a channel counts as at-the-tail: a user there
/// wants the tail again on re-entry, so the mark is CLEARED instead of
/// stored. Same order of magnitude as the pane's own near-bottom slack.
#[cfg(feature = "hydrate")]
const AT_BOTTOM_SLACK_PX: f64 = 40.0;

/// Load the persisted scroll marks (on shell mount). Empty on a fresh device
/// or any parse failure.
#[cfg(feature = "hydrate")]
pub(crate) fn load_scroll_marks() -> HashMap<String, String> {
    LocalStorage::get(KEY_SCROLL_MARKS).unwrap_or_default()
}

#[cfg(not(feature = "hydrate"))]
pub(crate) fn load_scroll_marks() -> HashMap<String, String> {
    HashMap::new()
}

/// Write (`Some`) or clear (`None`) one channel's mark + persist the map.
/// `try_*` throughout: callers can run from teardown paths where the shell
/// signals are already disposed.
#[cfg(feature = "hydrate")]
fn save_mark(s: Shell, cid: &str, mid: Option<String>) {
    let _ = s.notify.scroll_marks.try_update(|m| match mid {
        Some(mid) => {
            m.insert(cid.to_string(), mid);
        }
        None => {
            m.remove(cid);
        }
    });
    if let Some(map) = s.notify.scroll_marks.try_get_untracked() {
        let _ = LocalStorage::set(KEY_SCROLL_MARKS, &map);
    }
}

/// Record where the user stands in the still-open channel — called by
/// `open_channel_at` BEFORE any state is cleared, while the DOM still shows
/// the OUTGOING channel. At/near the tail clears the mark (re-entry should
/// land at the tail again); otherwise the mark is the topmost REAL message
/// row (`msg-` dom ids only — skeletons/ghosts/dividers excluded by the
/// selector) still visible in the viewport. Row-id granularity is deliberate
/// (the proposal said "id + pixel offset"): restore rides the proven
/// `anchor_to`/`scroll_into_view` path, and stored pixel offsets are exactly
/// the hardcoded-pixel math the fluid-geometry rule forbids — a viewport or
/// font change between visits would make them lie. No-op when no message
/// list is mounted (a sheet pick from another pane keeps the previous mark)
/// or when it measures zero (detached mid-teardown).
#[cfg(feature = "hydrate")]
pub(crate) fn capture_scroll_mark(s: Shell) {
    use wasm_bindgen::JsCast;
    let Some(cid) = s
        .sel
        .sel_channel
        .try_get_untracked()
        .flatten()
        .map(|c| c.id)
    else {
        return;
    };
    let Some(list) = leptos::web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector("ul.messages").ok().flatten())
    else {
        return;
    };
    let scroll_height = f64::from(list.scroll_height());
    let client_height = f64::from(list.client_height());
    // A detached / not-yet-laid-out list measures 0 — no real position.
    if scroll_height <= 0.0 || client_height <= 0.0 {
        return;
    }
    let dist = scroll_height - f64::from(list.scroll_top()) - client_height;
    if dist <= AT_BOTTOM_SLACK_PX {
        save_mark(s, &cid, None);
        return;
    }
    // Topmost message row still visible: bounding rects (not offset math, so
    // padding/transform contexts can't skew it). NodeList is document order =
    // vertical order (the list is composite-ordered, never re-sorted), so the
    // first row whose bottom edge sits below the list's top edge wins.
    let list_top = list.get_bounding_client_rect().top();
    let Ok(rows) = list.query_selector_all("li[id^='msg-']") else {
        return;
    };
    for i in 0..rows.length() {
        let Some(el) = rows
            .get(i)
            .and_then(|n| n.dyn_into::<leptos::web_sys::Element>().ok())
        else {
            continue;
        };
        if el.get_bounding_client_rect().bottom() > list_top {
            if let Some(mid) = el.id().strip_prefix("msg-") {
                save_mark(s, &cid, Some(mid.to_string()));
            }
            return;
        }
    }
}

/// The saved re-entry anchor for `cid`, CONSUMED (one-shot per OPEN): returns
/// the remembered row id only when that row is on the just-loaded page, and
/// removes the entry either way — a consumed mark is re-captured fresh on the
/// next switch-away, and a missing row (fell off the newest page) prunes
/// itself instead of returning a dead anchor. The caller MUST call this
/// unconditionally on every open and use the value only when no deep-link /
/// NEW-divider jump outranks it (`open_channel_at`) — consulting it lazily
/// from the precedence chain let a mark outlive the open it was saved for
/// and restore a stale position days later (review). Must run AFTER the
/// page's `ingest` (it validates against `messages`).
#[cfg(feature = "hydrate")]
pub(crate) fn take_restore_anchor(s: Shell, cid: &str) -> Option<String> {
    let saved = s
        .notify
        .scroll_marks
        .with_untracked(|m| m.get(cid).cloned())?;
    save_mark(s, cid, None);
    s.msg
        .messages
        .with_untracked(|v| v.iter().any(|m| m.id == saved))
        .then_some(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal envelope with just the composite-cursor fields the fns under
    /// test read; everything else is inert filler.
    fn env(id: &str, sent_at: &str) -> MessageEnvelope {
        MessageEnvelope {
            id: id.into(),
            author_id: "account:a".into(),
            author_name: "a".into(),
            author_display: "A".into(),
            author_avatar_id: None,
            persona_id: None,
            persona_name: None,
            persona_description: None,
            persona_color: None,
            persona_avatar_id: None,
            body: "hello".into(),
            attachments: Vec::new(),
            tier: "default".into(),
            sent_at: sent_at.into(),
            reply_to: None,
            is_pinged: false,
            kind: "user".into(),
            effect: None,
            guest_cameo: false,
        }
    }

    fn cur(sent_at: &str, id: &str) -> (String, String) {
        (sent_at.to_string(), id.to_string())
    }

    #[test]
    fn first_past_baseline_finds_the_first_row_strictly_past_it() {
        let msgs = vec![
            env("aaa", "2026-06-10T08:00:00.000000000Z"),
            env("bbb", "2026-06-11T09:00:00.000000000Z"),
            env("ccc", "2026-06-11T10:00:00.000000000Z"),
        ];
        let prior = cur("2026-06-10T08:00:00.000000000Z", "aaa");
        assert_eq!(first_past_baseline(&msgs, &prior), Some("bbb".to_string()));
    }

    #[test]
    fn first_past_baseline_breaks_sent_at_ties_strictly_on_id() {
        // Two rows share the baseline's sent_at: the id-equal row is READ
        // (not strictly past), the id-greater row is the frontier — the
        // composite cursor's strict tie-break, no off-by-one duplicates.
        let msgs = vec![
            env("aaa", "2026-06-11T09:00:00.000000000Z"),
            env("bbb", "2026-06-11T09:00:00.000000000Z"),
        ];
        let prior = cur("2026-06-11T09:00:00.000000000Z", "aaa");
        assert_eq!(first_past_baseline(&msgs, &prior), Some("bbb".to_string()));
    }

    #[test]
    fn first_past_baseline_returns_none_when_nothing_is_past_it() {
        let msgs = vec![
            env("aaa", "2026-06-10T08:00:00.000000000Z"),
            env("bbb", "2026-06-11T09:00:00.000000000Z"),
        ];
        // Baseline AT the newest row: fully read, no divider.
        let prior = cur("2026-06-11T09:00:00.000000000Z", "bbb");
        assert_eq!(first_past_baseline(&msgs, &prior), None);
        // …and on an empty page.
        assert_eq!(first_past_baseline(&[], &prior), None);
    }

    #[test]
    fn utc_date_label_slices_the_rfc3339_date_part() {
        assert_eq!(
            utc_date_label("2026-06-11T20:15:30.123456789Z"),
            "2026-06-11"
        );
    }

    #[test]
    fn utc_date_label_passes_garbage_through_unchanged() {
        assert_eq!(utc_date_label("not a timestamp"), "not a timestamp");
        assert_eq!(utc_date_label(""), "");
    }

    #[test]
    fn date_label_groups_rows_by_their_date_part_on_ssr() {
        // The ssr fallback is the UTC date slice (hydrate swaps in the
        // viewer-local date); two same-day rows share a label, a row across
        // midnight gets a new one — the separator predicate.
        let a = date_label("2026-06-11T08:00:00.000000000Z");
        let b = date_label("2026-06-11T23:59:59.999999999Z");
        let c = date_label("2026-06-12T00:00:00.000000000Z");
        assert_eq!(a, b);
        assert_ne!(b, c);
        assert_eq!(c, "2026-06-12");
    }
}
