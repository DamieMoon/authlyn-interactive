//! Per-message attachment grid + lightbox source. The hydrate variant wires
//! click-to-lightbox; the ssr variant is plain markup so the page still
//! hydrates identically.

use leptos::prelude::*;

use crate::protocol::Attachment;

/// Per-message row cap for the chunked grid: each row holds at most 3 cells,
/// so a 5-image message lays out as [3, 2] and the trailing row stretches to
/// fill the width.
const ROW_CAP: usize = 3;

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
    lightbox: RwSignal<Option<Attachment>>,
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
                                view! {
                                    <img class=cell_class loading="lazy" alt="attachment"
                                        src=src
                                        on:load=move |_| loaded.set(true)
                                        on:error=move |_| loaded.set(true)
                                        on:click=move |_| lightbox.set(Some(open.clone()))/>
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
    _lightbox: RwSignal<Option<Attachment>>,
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
