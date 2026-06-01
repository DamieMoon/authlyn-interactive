//! Composer color-swatch history: the 3 last-used colors shown inline for a
//! quick swap (feedback rli3tsora4ho7lsi9q31). The move-to-front/dedup/cap fn
//! is pure (compiled in both graphs + unit-tested); load/save are the usual
//! hydrate-real + ssr-stub localStorage helpers, mirroring `prefs.rs`.

/// Max quick-swap swatches kept in the history.
pub(crate) const COLOR_HISTORY_CAP: usize = 3;

/// Record a just-used color name into the most-recent-first history: move it to
/// the front, drop any earlier duplicate, and cap the list at
/// [`COLOR_HISTORY_CAP`]. Pure — no DOM/storage — so it unit-tests cleanly.
pub(crate) fn record_color(history: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::with_capacity(COLOR_HISTORY_CAP);
    out.push(name.to_string());
    for c in history {
        if c != name && out.len() < COLOR_HISTORY_CAP {
            out.push(c.clone());
        }
    }
    out
}

// localStorage key for the quick-swap color history (a JSON array of tag names,
// most-recent-first). Absent → empty history (no quick swatches yet).
#[cfg(feature = "hydrate")]
const KEY_COLOR_HISTORY: &str = "authlyn.compose_colors";

/// Load the persisted color history (most-recent-first). Empty on a fresh user
/// or any parse failure.
#[cfg(feature = "hydrate")]
pub(crate) fn load_color_history() -> Vec<String> {
    use gloo_storage::{LocalStorage, Storage};
    LocalStorage::get::<Vec<String>>(KEY_COLOR_HISTORY).unwrap_or_default()
}

/// Persist the color history.
#[cfg(feature = "hydrate")]
pub(crate) fn save_color_history(history: &[String]) {
    use gloo_storage::{LocalStorage, Storage};
    let _ = LocalStorage::set(KEY_COLOR_HISTORY, history);
}

// ---- ssr stub (no localStorage on the server) ----

#[cfg(not(feature = "hydrate"))]
pub(crate) fn load_color_history() -> Vec<String> {
    Vec::new()
}

#[cfg(not(feature = "hydrate"))]
pub(crate) fn save_color_history(_history: &[String]) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_color_prepends_new_color() {
        let h = record_color(&[], "red");
        assert_eq!(h, vec!["red".to_string()]);
    }

    #[test]
    fn record_color_moves_existing_to_front_dedup() {
        let h = vec!["red".to_string(), "blue".to_string(), "green".to_string()];
        let h = record_color(&h, "green");
        assert_eq!(
            h,
            vec!["green".to_string(), "red".to_string(), "blue".to_string()]
        );
    }

    #[test]
    fn record_color_caps_at_three() {
        let h = vec!["red".to_string(), "blue".to_string(), "green".to_string()];
        let h = record_color(&h, "yellow");
        assert_eq!(
            h,
            vec!["yellow".to_string(), "red".to_string(), "blue".to_string()]
        );
        assert_eq!(h.len(), COLOR_HISTORY_CAP);
    }

    #[test]
    fn record_color_no_duplicates_when_repeating_front() {
        let h = vec!["red".to_string(), "blue".to_string()];
        let h = record_color(&h, "red");
        assert_eq!(h, vec!["red".to_string(), "blue".to_string()]);
    }
}
