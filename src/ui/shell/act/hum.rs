//! Corridor hum (UX evolution #4): ambient "someone is active here RIGHT
//! NOW" marks on channel rows, derived purely from the id-only SSE events
//! the client already receives and filters (`act::sync::dispatch`) — zero
//! added requests, no new endpoints, no draft text. The id-only bus is
//! untouched: this module converts an already-delivered fact (a Typing ping
//! or a created message in a visible channel) into an ephemeral per-channel
//! flag (`Notify::humming`) that decays after [`HUM_DECAY_MS`] of silence,
//! mirroring the server's 8s typing TTL.
//!
//! The generation bookkeeping is pure — compiled in every graph and
//! unit-tested, the `compose_colors` pattern — while the timer-spawning
//! [`mark_hum`] is hydrate-only (its sole caller is the SSE dispatch). The
//! poll fallback never lights a hum: graceful absence, like Ghost Quill.

use std::collections::HashMap;

/// Decay window: a hum mark outlives the LAST arming event by this long.
/// Mirrors the server's 8s typing TTL (`AppState.typing` / typing-drafts),
/// so the mark fades on the same clock the in-channel typing line does.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) const HUM_DECAY_MS: u32 = 8_000;

/// Arm (or re-arm) the hum for `cid`: bump its generation and return the new
/// value. The caller schedules a decay timer holding this generation;
/// [`hum_decay_due`] then ignores any timer whose generation went stale, so a
/// re-armed hum is never cut short by an EARLIER event's timer. Pure — no
/// signals, no timers — so it unit-tests cleanly.
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) fn hum_arm(map: &mut HashMap<String, u64>, cid: &str) -> u64 {
    let g = map.entry(cid.to_string()).or_insert(0);
    *g = g.wrapping_add(1);
    *g
}

/// True when the decay timer holding `generation` is still the CURRENT armer
/// of `cid` — i.e. no later event re-armed the hum, and the entry should now
/// be cleared. Pure; see [`hum_arm`].
#[cfg_attr(not(feature = "hydrate"), allow(dead_code))]
pub(crate) fn hum_decay_due(map: &HashMap<String, u64>, cid: &str, generation: u64) -> bool {
    map.get(cid) == Some(&generation)
}

/// Flag `cid` as live-right-now and schedule its decay just past the typing
/// TTL. Each qualifying SSE event re-arms (bumps the generation), so the
/// row mark fades only after [`HUM_DECAY_MS`] of actual silence; a stale
/// timer is a pure no-op (checked untracked, so it never even notifies
/// subscribers). Cosmetic, client-only state — nothing here is sent,
/// fetched, or persisted.
#[cfg(feature = "hydrate")]
pub(crate) fn mark_hum(s: super::super::Shell, cid: String) {
    use leptos::prelude::*;
    use leptos::task::spawn_local;

    let Some(generation) = s.notify.humming.try_update(|m| hum_arm(m, &cid)) else {
        return;
    };
    spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(HUM_DECAY_MS).await;
        // Clear only while OUR arming is still current — and write (notify)
        // only when there is actually something to remove.
        if s.notify
            .humming
            .with_untracked(|m| hum_decay_due(m, &cid, generation))
        {
            s.notify.humming.update(|m| {
                m.remove(&cid);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arming_a_channel_inserts_it_and_returns_its_current_generation() {
        let mut m = HashMap::new();
        let g = hum_arm(&mut m, "channel:a");
        assert_eq!(
            m.get("channel:a"),
            Some(&g),
            "the map entry must hold exactly the generation handed to the decay timer"
        );
    }

    #[test]
    fn rearming_bumps_the_generation_so_the_earlier_timer_goes_stale() {
        let mut m = HashMap::new();
        let g1 = hum_arm(&mut m, "channel:a");
        let g2 = hum_arm(&mut m, "channel:a");
        assert_ne!(g1, g2, "a re-arm must mint a fresh generation");
        assert!(
            !hum_decay_due(&m, "channel:a", g1),
            "the first event's decay timer must be stale after a re-arm — \
             otherwise it would cut a still-live hum short"
        );
        assert!(
            hum_decay_due(&m, "channel:a", g2),
            "the latest armer's timer is the one that clears the mark"
        );
    }

    #[test]
    fn decay_is_due_only_while_the_arming_generation_is_still_current() {
        let mut m = HashMap::new();
        let g = hum_arm(&mut m, "channel:a");
        assert!(hum_decay_due(&m, "channel:a", g));
    }

    #[test]
    fn decay_is_never_due_for_an_already_cleared_or_unknown_channel() {
        let mut m: HashMap<String, u64> = HashMap::new();
        assert!(
            !hum_decay_due(&m, "channel:a", 0),
            "an unknown channel has nothing to clear"
        );
        let g = hum_arm(&mut m, "channel:a");
        m.remove("channel:a");
        assert!(
            !hum_decay_due(&m, "channel:a", g),
            "a cleared entry must not be due again (the update would notify for nothing)"
        );
    }

    #[test]
    fn generations_are_tracked_independently_per_channel() {
        let mut m = HashMap::new();
        let ga = hum_arm(&mut m, "channel:a");
        let _gb1 = hum_arm(&mut m, "channel:b");
        let gb2 = hum_arm(&mut m, "channel:b");
        assert!(
            hum_decay_due(&m, "channel:a", ga),
            "channel:a's timer must not be staled by channel:b's re-arm"
        );
        assert!(hum_decay_due(&m, "channel:b", gb2));
    }
}
