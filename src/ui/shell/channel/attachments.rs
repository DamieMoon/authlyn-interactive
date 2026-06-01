//! Per-message attachment grid + lightbox source. The hydrate variant wires
//! click-to-lightbox; the ssr variant is plain markup so the page still
//! hydrates identically.

use leptos::prelude::*;

use super::LightboxState;
use crate::protocol::Attachment;

/// True for an attachment that opens as a gallery image (everything except
/// video, which keeps its own inline controls). The lightbox gallery navigates
/// images only, so this is the filter used to build the gallery list and to map
/// a clicked thumbnail to its index within that list. Hydrate-only: the ssr
/// grid has no lightbox interaction.
#[cfg(feature = "hydrate")]
fn is_image(att: &Attachment) -> bool {
    !att.mime.starts_with("video/")
}

/// Per-message row cap for the chunked grid: each row holds at most 3 cells,
/// so a 5-image message lays out as [3, 2] and the trailing row stretches to
/// fill the width.
const ROW_CAP: usize = 3;

/// Short human label for a download tile, derived from the stored MIME's base
/// type (e.g. `application/pdf` → "PDF", `application/zip` → "ZIP"). Falls back
/// to "FILE" for an unknown/empty mime. Purely cosmetic — the link still serves
/// the raw blob regardless.
fn file_label(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "application/pdf" => "PDF",
        "application/zip" => "ZIP",
        "text/plain" => "TXT",
        m if m.starts_with("audio/") => "AUDIO",
        _ => "FILE",
    }
}

/// True if an attachment renders as a media tile (`<img>`/`<video>`) rather
/// than a download link. Anything that is neither an image nor a video — PDFs,
/// audio, zips, plain text — is shown as a file download tile.
fn is_media_tile(mime: &str) -> bool {
    mime.starts_with("image/") || mime.starts_with("video/")
}

/// Outer wrapper classes: the message owns its own `.attachments` block, and
/// the lone-image case gets an extra `.lone` flag so the natural-aspect rule
/// applies only when the whole message is one image (not a trailing remainder
/// row in a multi-row layout).
fn wrapper_class(n: usize) -> &'static str {
    if n == 1 {
        "attachments lone"
    } else {
        "attachments"
    }
}

/// Lay attachments into rows so the last row stretches to fill the width.
/// Returns the cell count per row. Single-row layouts (1/2/3) pass through
/// untouched; 4 preserves the historical 2x2 grid; 5+ chunks by [`ROW_CAP`]
/// with the remainder in the trailing row.
fn row_layout(n: usize) -> Vec<usize> {
    match n {
        0 => Vec::new(),
        1 => vec![1],
        2 => vec![2],
        3 => vec![3],
        4 => vec![2, 2],
        _ => {
            let full = n / ROW_CAP;
            let rem = n % ROW_CAP;
            let mut rows = vec![ROW_CAP; full];
            if rem > 0 {
                rows.push(rem);
            }
            rows
        }
    }
}

/// Render a message's inline image attachments as a Discord-style grid: the
/// more images, the more compact (column count climbs, cells go square).
/// Clicking one opens it in the lightbox. Thumbnails pull a downscaled JPEG
/// (`?w=512`); the lightbox loads the full original.
///
/// Rows of up to [`ROW_CAP`] cells; the trailing row's `cols-N` class scales
/// its cells to fill the width (a 5-image message → 3+2 with 1/3-width + 1/2-
/// width cells), so the layout never leaves a half-empty last row.
#[cfg(feature = "hydrate")]
pub(super) fn attachment_grid(
    atts: Vec<Attachment>,
    lightbox: RwSignal<Option<LightboxState>>,
) -> impl IntoView {
    let wrapper_class = wrapper_class(atts.len());
    let rows = row_layout(atts.len());
    // The gallery the lightbox navigates: this message's images only. A clicked
    // image opens at its index within `gallery`; a clicked video opens a
    // single-entry gallery holding just that video (so arrow/swipe no-op).
    let gallery: Vec<Attachment> = atts.iter().filter(|a| is_image(a)).cloned().collect();
    let mut iter = atts.into_iter();
    view! {
        <div class=wrapper_class>
            {rows.into_iter().map(|n| {
                let row: Vec<Attachment> = iter.by_ref().take(n).collect();
                let gallery = gallery.clone();
                view! {
                    <div class=format!("att-row cols-{n}")>
                        {row.into_iter().map(|att| {
                            let is_video = att.mime.starts_with("video/");
                            let id = att.id.clone();
                            if !is_media_tile(&att.mime) {
                                // Non-image/video: a file-icon download tile. The blob is
                                // served with attachment disposition + nosniff (serve_original),
                                // so following the link downloads rather than renders it.
                                let label = file_label(&att.mime);
                                view! {
                                    <a class="att-file" href=format!("/media/{id}") download
                                        title="download attachment">
                                        <span class="att-file-icon">"📄"</span>
                                        <span class="att-file-label">{label}</span>
                                    </a>
                                }.into_any()
                            } else if is_video {
                                // Videos use the raw blob (the `?w=512` thumbnail path is
                                // image-only); play inline and open a lone-video lightbox.
                                let open = att.clone();
                                view! {
                                    <video class="att-thumb" controls preload="metadata"
                                        src=format!("/media/{id}")
                                        on:click=move |_| lightbox.set(Some(LightboxState {
                                            images: vec![open.clone()],
                                            idx: 0,
                                        }))></video>
                                }.into_any()
                            } else {
                                // GIFs must use the raw blob: the `?w=512` thumbnail re-encodes
                                // to a STATIC JPEG (first frame). Other images keep the thumb.
                                let src = if att.mime == "image/gif" {
                                    format!("/media/{id}")
                                } else {
                                    format!("/media/{id}?w=512")
                                };
                                // Shimmer placeholder (F-7): the cell carries
                                // `.att-loading` until the thumb's first
                                // `load`/`error`, when it's removed so the
                                // shimmer stops and the image shows through.
                                let loaded = RwSignal::new(false);
                                let cell_class = move || if loaded.get() {
                                    "att-thumb"
                                } else {
                                    "att-thumb att-loading"
                                };
                                // Index of this image within the gallery (image-only)
                                // list (F-4): a click opens the lightbox at this image
                                // so arrow/swipe navigate the message's other images.
                                let idx = gallery.iter().position(|g| g.id == att.id).unwrap_or(0);
                                let gallery = gallery.clone();
                                view! {
                                    <img class=cell_class loading="lazy" alt="attachment"
                                        src=src
                                        on:load=move |_| loaded.set(true)
                                        on:error=move |_| loaded.set(true)
                                        on:click=move |_| lightbox.set(Some(LightboxState {
                                            images: gallery.clone(),
                                            idx,
                                        }))/>
                                }.into_any()
                            }
                        }).collect_view()}
                    </div>
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
    _lightbox: RwSignal<Option<LightboxState>>,
) -> impl IntoView {
    let wrapper_class = wrapper_class(atts.len());
    let rows = row_layout(atts.len());
    let mut iter = atts.into_iter();
    view! {
        <div class=wrapper_class>
            {rows.into_iter().map(|n| {
                let row: Vec<Attachment> = iter.by_ref().take(n).collect();
                view! {
                    <div class=format!("att-row cols-{n}")>
                        {row.into_iter().map(|att| {
                            let id = att.id.clone();
                            let is_video = att.mime.starts_with("video/");
                            if !is_media_tile(&att.mime) {
                                // Non-image/video: a file-icon download tile (raw blob is
                                // served as an attachment + nosniff by serve_original).
                                let label = file_label(&att.mime);
                                view! {
                                    <a class="att-file" href=format!("/media/{id}") download
                                        title="download attachment">
                                        <span class="att-file-icon">"📄"</span>
                                        <span class="att-file-label">{label}</span>
                                    </a>
                                }.into_any()
                            } else if is_video {
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
            }).collect_view()}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::row_layout;

    #[test]
    fn single_row_layouts_pass_through() {
        assert_eq!(row_layout(0), Vec::<usize>::new());
        assert_eq!(row_layout(1), vec![1]);
        assert_eq!(row_layout(2), vec![2]);
        assert_eq!(row_layout(3), vec![3]);
    }

    #[test]
    fn four_preserves_two_by_two() {
        // Historical layout: 4 images rendered as a 2x2 square block.
        assert_eq!(row_layout(4), vec![2, 2]);
    }

    #[test]
    fn five_chunks_three_then_two() {
        // The Foxtrot ctx-019e6f35 case: row 1 holds 3 cells at 1/3 width,
        // row 2 holds 2 cells at 1/2 width (no half-empty last row).
        assert_eq!(row_layout(5), vec![3, 2]);
    }

    #[test]
    fn larger_counts_chunk_by_three_with_remainder_last() {
        assert_eq!(row_layout(6), vec![3, 3]);
        assert_eq!(row_layout(7), vec![3, 3, 1]);
        assert_eq!(row_layout(8), vec![3, 3, 2]);
        assert_eq!(row_layout(9), vec![3, 3, 3]);
        assert_eq!(row_layout(10), vec![3, 3, 3, 1]);
    }

    #[test]
    fn all_layouts_sum_to_input() {
        // Invariant: the row layout must visit every attachment exactly once.
        for n in 0..=100 {
            let sum: usize = row_layout(n).into_iter().sum();
            assert_eq!(sum, n, "row_layout({n}) cells must sum to n");
        }
    }
}
