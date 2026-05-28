//! Per-message attachment grid + lightbox source. The hydrate variant wires
//! click-to-lightbox; the ssr variant is plain markup so the page still
//! hydrates identically.

use leptos::prelude::*;

use crate::protocol::Attachment;

/// Render a message's inline image attachments as a Discord-style grid: the
/// more images, the more compact (column count climbs, cells go square).
/// Clicking one opens it in the lightbox. Thumbnails pull a downscaled JPEG
/// (`?w=512`); the lightbox loads the full original.
#[cfg(feature = "hydrate")]
pub(super) fn attachment_grid(
    atts: Vec<Attachment>,
    lightbox: RwSignal<Option<Attachment>>,
) -> impl IntoView {
    let cols = match atts.len() {
        1 => 1,
        2 | 4 => 2,
        _ => 3,
    };
    view! {
        <div class=format!("attachments cols-{cols}")>
            {atts.into_iter().map(|att| {
                let open = att.clone();
                let is_video = att.mime.starts_with("video/");
                let id = att.id.clone();
                if is_video {
                    // Videos use the raw blob (the `?w=512` thumbnail path is
                    // image-only); play inline and open the lightbox on click.
                    view! {
                        <video class="att-thumb" controls preload="metadata"
                            src=format!("/media/{id}")
                            on:click=move |_| lightbox.set(Some(open.clone()))></video>
                    }.into_any()
                } else {
                    // GIFs must use the raw blob: the `?w=512` thumbnail re-encodes
                    // to a STATIC JPEG (first frame). Other images keep the thumb.
                    let src = if att.mime == "image/gif" {
                        format!("/media/{id}")
                    } else {
                        format!("/media/{id}?w=512")
                    };
                    view! {
                        <img class="att-thumb" loading="lazy" alt="attachment"
                            src=src
                            on:click=move |_| lightbox.set(Some(open.clone()))/>
                    }.into_any()
                }
            }).collect_view()}
        </div>
    }
}

/// SSR build has no lightbox interaction; render the grid as plain links so the
/// markup still hydrates identically.
#[cfg(not(feature = "hydrate"))]
pub(super) fn attachment_grid(
    atts: Vec<Attachment>,
    _lightbox: RwSignal<Option<Attachment>>,
) -> impl IntoView {
    let cols = match atts.len() {
        1 => 1,
        2 | 4 => 2,
        _ => 3,
    };
    view! {
        <div class=format!("attachments cols-{cols}")>
            {atts.into_iter().map(|att| {
                let id = att.id.clone();
                let is_video = att.mime.starts_with("video/");
                if is_video {
                    view! {
                        <video class="att-thumb" controls preload="metadata"
                            src=format!("/media/{id}")></video>
                    }.into_any()
                } else {
                    // GIFs use the raw blob (the thumbnail re-encodes to a static
                    // JPEG); other images use the downscaled thumb.
                    let src = if att.mime == "image/gif" {
                        format!("/media/{id}")
                    } else {
                        format!("/media/{id}?w=512")
                    };
                    view! {
                        <img class="att-thumb" alt="attachment" src=src/>
                    }.into_any()
                }
            }).collect_view()}
        </div>
    }
}
