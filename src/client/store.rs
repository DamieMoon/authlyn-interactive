//! localStorage persistence of the [`DeviceClient`] snapshot (hydrate build).
//!
//! One JSON blob under a single key. Save after every state mutation; load on
//! mount so a page reload keeps the device identity and any imported sessions
//! (the key-share inbox is delete-on-read, so a reload that lost its inbound
//! sessions could not re-fetch them).

use gloo_storage::{LocalStorage, Storage};

use super::session::Snapshot;

const KEY: &str = "authlyn.device";

/// Persist the snapshot. Errors are swallowed — a failed write just means the
/// next reload starts fresh, which is recoverable.
pub fn save(snapshot: &Snapshot) {
    let _ = LocalStorage::set(KEY, snapshot);
}

/// Load a previously-saved snapshot, if any.
pub fn load() -> Option<Snapshot> {
    LocalStorage::get(KEY).ok()
}

/// Forget the stored device entirely.
pub fn clear() {
    LocalStorage::delete(KEY);
}
