//! Ephemeral loading-skeleton rows for the channel pane.
//!
//! While a channel's first page is in flight the message list is empty, so the
//! pane would otherwise flash blank between the channel switch and the
//! response. This module renders a fixed run of CSS-only shimmer placeholder
//! rows (generic avatar circle + body bars) during that window; Leptos diffing
//! drops them the instant `sync_messages`/`ingest` lands real rows.
//!
//! CRITICAL: these rows are EPHEMERAL DOM ONLY. They use sentinel `skeleton-N`
//! ids and never touch `s.msg.seen`, the cursor, `oldest`, or `anchor_to`, so
//! they can never collide with dedup/backfill. The shimmer is pure CSS
//! (`@keyframes shimmer` in `style/_skeleton.scss`) — no per-frame JS.
//!
//! Shared across both build graphs (no ssr/hydrate crates), so the skeleton
//! markup hydrates identically.

use leptos::prelude::*;

/// How many placeholder rows to paint while loading. Fixed (not derived from
/// the incoming page size, which isn't known yet); 6 comfortably fills a pane
/// without overflowing short viewports.
const SKELETON_ROWS: usize = 6;

/// Whether to show the loading skeleton: only when a first-page load is
/// in-flight AND no real messages are rendered yet. Once `sync_messages`
/// pushes real rows the list is non-empty and the skeletons vanish; the flag
/// is also cleared by the load's completion path. A pure predicate so it can
/// be unit-tested without a reactive runtime.
pub(super) fn should_show_skeletons(loading_initial: bool, message_count: usize) -> bool {
    loading_initial && message_count == 0
}

/// One shimmer placeholder row: a generic circular avatar plus a couple of
/// body bars, mirroring the real `.msg` layout so the swap-in doesn't shift
/// the pane. `i` only feeds the sentinel id + a small width jitter so the rows
/// don't look mechanically identical.
fn skeleton_row(i: usize) -> impl IntoView {
    // Subtle per-row body-width variation (last bar shorter on alternating
    // rows) so the run reads as "content loading" rather than a flat block.
    let short = i.is_multiple_of(2);
    view! {
        <li class="msg msg-skeleton" id=format!("skeleton-{i}") aria-hidden="true">
            <span class="skel-avatar"></span>
            <div class="skel-body">
                <span class="skel-bar skel-name"></span>
                <span class="skel-bar skel-line"></span>
                <span class=if short { "skel-bar skel-line skel-short" } else { "skel-bar skel-line" }></span>
            </div>
        </li>
    }
}

/// The full skeleton run. Rendered inside `.messages` in place of the real
/// rows while [`should_show_skeletons`] holds.
pub(super) fn skeleton_rows() -> impl IntoView {
    view! {
        <>{(0..SKELETON_ROWS).map(skeleton_row).collect_view()}</>
    }
}

#[cfg(test)]
mod tests {
    use super::should_show_skeletons;

    #[test]
    fn shows_only_while_loading_and_empty() {
        // In-flight load with nothing rendered yet → show skeletons.
        assert!(should_show_skeletons(true, 0));
    }

    #[test]
    fn hidden_once_real_messages_land() {
        // Real rows present → never skeletons, even if a flag is still set
        // (Leptos diffing also removes them; the predicate is the guard).
        assert!(!should_show_skeletons(true, 5));
        assert!(!should_show_skeletons(false, 5));
    }

    #[test]
    fn hidden_when_idle_and_empty() {
        // Genuinely empty channel, no load in flight → no skeletons (so an
        // empty channel reads as empty, not perpetually loading).
        assert!(!should_show_skeletons(false, 0));
    }
}
