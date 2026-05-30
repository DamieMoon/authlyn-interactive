//! Authenticated remote images.
//!
//! Media is auth-gated (`GET /media/{id}` needs the session), and Freya's
//! `ImageViewer` URL fetch is anonymous — so we fetch the bytes through our
//! authenticated `reqwest` client and hand `ImageViewer` an `ImageSource::Bytes`.
//! This is a real Freya `Component` (not a plain fn) so each image gets its own
//! hook scope for the async load; a monogram tile shows while loading / on error.

use bytes::Bytes;
use freya::prelude::*;
use std::hash::{Hash, Hasher};

use crate::native::api::client;
use crate::native::theme;

fn hash_id(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn first_letter(s: &str) -> String {
    s.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}

/// A media blob rendered as an image, loaded over the authenticated session.
#[derive(PartialEq, Clone)]
pub struct RemoteImage {
    pub media_id: String,
    pub size: f32,
    /// Name to monogram while loading / on failure.
    pub fallback: String,
    /// Circular (avatar) vs slightly-rounded (attachment).
    pub circle: bool,
}

impl Component for RemoteImage {
    fn render(&self) -> impl IntoElement {
        let data = use_state(|| None::<Bytes>);
        let id = self.media_id.clone();
        let w = (self.size * 2.0) as u32; // ~retina

        use_hook(move || {
            spawn(async move {
                if let Ok(bytes) = client().get_media_bytes(&id, w).await {
                    *data.write_unchecked() = Some(bytes);
                }
            });
        });

        let radius = if self.circle {
            self.size / 2.0
        } else {
            theme::RADIUS_SM
        };

        let el: Element = match data.read().clone() {
            Some(bytes) => ImageViewer::new(ImageSource::Bytes(hash_id(&self.media_id), bytes))
                .width(Size::px(self.size))
                .height(Size::px(self.size))
                .corner_radius(radius)
                .into(),
            None => rect()
                .width(Size::px(self.size))
                .height(Size::px(self.size))
                .corner_radius(radius)
                .background(theme::AVATAR_TILE)
                .color(theme::INK_SOFT)
                .center()
                .child(first_letter(&self.fallback))
                .into(),
        };
        el
    }
}
