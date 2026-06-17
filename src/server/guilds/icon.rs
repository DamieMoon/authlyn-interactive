//! Guild icon upload + the server-derived per-server accent (M6/P1, effect G).
//!
//! `PUT /guilds/{id}/icon` points `guild.icon` at an already-uploaded media blob
//! (the client POSTs the file to `/media` first, then sends the id here), then
//! re-derives the per-server accent (`guild.accent_color`) from the icon image
//! server-side. Manager-gated like every other guild mutation; the media id is
//! existence-checked (privacy-404) exactly as persona `set_avatar` does.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::protocol::{SetGuildIconRequest, SyncEvent};
use crate::server::auth::AuthAccount;
use crate::server::db_helpers::IdRow;
use crate::server::errors::{error_response, json_rejection_response};
use crate::server::permissions::require_manager;
use crate::server::state::AppState;

/// PUT /guilds/{id}/icon — set the guild's icon and re-derive its per-server
/// accent from the image (manager-gated). The body carries a media id from a
/// prior `POST /media`.
#[tracing::instrument(skip_all, fields(account = %account.0, guild = %gid))]
pub async fn set_guild_icon(
    State(state): State<AppState>,
    Path(gid): Path<String>,
    account: AuthAccount,
    payload: Result<Json<SetGuildIconRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rej) => return json_rejection_response(rej),
    };
    // Owner/admin only; non-members get a privacy-404, plain members 403, and a
    // soft-deleted guild is rejected (require_manager checks liveness).
    if let Err(r) = require_manager(&state, &gid, &account.0).await {
        return r;
    }
    // Existence-check the media id (privacy-404), same contract as persona
    // set_avatar. (Accent derivation from the bytes lands in M6/P1.3.)
    match media_exists(&state, &req.media_id).await {
        Ok(true) => {}
        Ok(false) => return error_response(StatusCode::NOT_FOUND, "media not found"),
        Err(e) => {
            tracing::error!(error = %e, "media_exists failed");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
        }
    }
    if let Err(e) = state
        .db
        .query("UPDATE type::record('guild', $gid) SET icon = type::record('media_blob', $mid);")
        .bind(("gid", gid.clone()))
        .bind(("mid", req.media_id.clone()))
        .await
        .and_then(|r| r.check())
    {
        tracing::error!(error = %e, "set_guild_icon update failed");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "storage error");
    }
    // Re-derive the per-server accent from the new icon (effect G). Best-effort:
    // if the bytes are unreadable/undecodable we keep the icon and leave the
    // accent untouched — a cosmetic derive must never fail the icon set. Policy:
    // OVERWRITE (a new icon defines the server's color); the manual swatch in
    // ServerModal lets the owner reclaim control afterwards.
    if let Some(bytes) = crate::server::media::read_media_blob_bytes(&state, &req.media_id).await {
        if let Ok(Some(accent)) =
            tokio::task::spawn_blocking(move || derive_accent_from_image(&bytes)).await
        {
            if let Err(e) = state
                .db
                .query("UPDATE type::record('guild', $gid) SET accent_color = $accent;")
                .bind(("gid", gid.clone()))
                .bind(("accent", accent))
                .await
                .and_then(|r| r.check())
            {
                // Non-fatal: the icon is set; the accent just stays as it was.
                tracing::error!(error = %e, "set_guild_icon accent derive update failed");
            }
        }
    }

    // Icon + derived accent are part of every member's rail render → broadcast so
    // all members refetch (id-only frame).
    state.emit(SyncEvent::ListsChanged);
    StatusCode::NO_CONTENT.into_response()
}

// ---------------------------------------------------------------------------
// Per-server accent derivation (M6/P1, effect G)
// ---------------------------------------------------------------------------

/// The 7 CHROMATIC accent anchors, IN `ACCENT_PALETTE` order. `"gray"` is
/// deliberately NOT here: gray is the low-saturation bucket, decided solely by
/// the saturation gate — never by RGB nearest-neighbor (a saturated mid-green
/// is closer in raw RGB to the muted-gray token than to the pastel-green token,
/// so including gray here would mis-bucket clearly-colored icons). RGBs MUST
/// stay in lockstep with `src/ui/accent.rs::accent_glow_css` (the UI binds the
/// derived name to exactly these colors); a unit test pins both invariants.
const HUE_ANCHORS: [(&str, [f64; 3]); 7] = [
    ("red", [255.0, 138.0, 150.0]),
    ("orange", [255.0, 180.0, 127.0]),
    ("yellow", [255.0, 212.0, 127.0]),
    ("green", [142.0, 230.0, 200.0]),
    ("blue", [127.0, 182.0, 255.0]),
    ("purple", [196.0, 168.0, 255.0]),
    ("pink", [255.0, 154.0, 213.0]),
];

/// Below this average HSV saturation (0..1) an icon is treated as achromatic and
/// derives to `"gray"` instead of being matched to a hue. Tunable (the owner may
/// adjust it on the headed demo after seeing real icons).
const SAT_THRESHOLD: f64 = 0.12;

/// Derive a per-server accent name (always one of `ACCENT_PALETTE`) from icon
/// bytes, deterministically: decode → downscale to ≤64px → saturation-weighted
/// average color → grayscale gate (→ `"gray"`) → otherwise nearest of the 7 hue
/// anchors. `None` only if the bytes don't decode (the caller then leaves the
/// accent unchanged). CPU-bound; callers run it on a blocking thread. No
/// clustering / RNG / float nondeterminism beyond IEEE arithmetic ⇒ the same
/// image always yields the same name.
fn derive_accent_from_image(bytes: &[u8]) -> Option<String> {
    let small = image::load_from_memory(bytes)
        .ok()?
        .thumbnail(64, 64)
        .to_rgb8();
    let (mut sum_r, mut sum_g, mut sum_b, mut sum_w, mut sat_total) =
        (0f64, 0f64, 0f64, 0f64, 0f64);
    let mut n: u64 = 0;
    for px in small.pixels() {
        let [r, g, b] = px.0;
        let (rf, gf, bf) = (f64::from(r), f64::from(g), f64::from(b));
        let max = rf.max(gf).max(bf);
        let min = rf.min(gf).min(bf);
        // HSV saturation; the average saturation drives the grayscale gate, and
        // the per-pixel saturation weights the average so a colorful logo on a
        // washed-out field derives from the logo, not the field.
        let s = if max <= 0.0 { 0.0 } else { (max - min) / max };
        sum_r += rf * s;
        sum_g += gf * s;
        sum_b += bf * s;
        sum_w += s;
        sat_total += s;
        n += 1;
    }
    if n == 0 {
        return None; // 0px image — nothing to derive from
    }
    let avg_s = sat_total / n as f64;
    if sum_w <= 0.0 || avg_s < SAT_THRESHOLD {
        return Some("gray".to_string());
    }
    Some(nearest_hue([sum_r / sum_w, sum_g / sum_w, sum_b / sum_w]).to_string())
}

/// Nearest of the 7 `HUE_ANCHORS` to `avg` by squared sRGB distance. Ties resolve
/// to the earlier `ACCENT_PALETTE` entry (the loop keeps the first strict
/// minimum), so the mapping is fully deterministic.
fn nearest_hue(avg: [f64; 3]) -> &'static str {
    let mut best = HUE_ANCHORS[0].0;
    let mut best_d = f64::MAX;
    for (name, anchor) in HUE_ANCHORS {
        let d = (avg[0] - anchor[0]).powi(2)
            + (avg[1] - anchor[1]).powi(2)
            + (avg[2] - anchor[2]).powi(2);
        if d < best_d {
            best_d = d;
            best = name;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::accent::ACCENT_PALETTE;

    #[test]
    fn hue_anchors_are_the_chromatic_palette_in_order() {
        // The hue anchors are `ACCENT_PALETTE` minus its final "gray" entry.
        let names: Vec<&str> = HUE_ANCHORS.iter().map(|(n, _)| *n).collect();
        assert_eq!(*ACCENT_PALETTE.last().unwrap(), "gray");
        assert_eq!(
            names.as_slice(),
            &ACCENT_PALETTE[..ACCENT_PALETTE.len() - 1]
        );
    }

    #[test]
    fn each_hue_anchor_maps_to_its_own_name() {
        for (name, rgb) in HUE_ANCHORS {
            assert_eq!(nearest_hue(rgb), name, "anchor {name} must map to itself");
        }
    }

    #[test]
    fn primary_and_saturated_colors_map_to_the_expected_hue() {
        assert_eq!(nearest_hue([255.0, 0.0, 0.0]), "red");
        assert_eq!(nearest_hue([0.0, 255.0, 0.0]), "green");
        assert_eq!(nearest_hue([0.0, 0.0, 255.0]), "blue");
        // A dark, saturated green must still read green (it is NOT near the muted
        // gray token) — the regression that motivated dropping gray as an anchor.
        assert_eq!(nearest_hue([30.0, 180.0, 70.0]), "green");
    }

    #[test]
    fn flat_gray_image_derives_gray_via_the_saturation_gate() {
        let png = solid_test_png(128, 128, 128);
        assert_eq!(derive_accent_from_image(&png).as_deref(), Some("gray"));
    }

    #[test]
    fn saturated_image_derives_its_hue() {
        assert_eq!(
            derive_accent_from_image(&solid_test_png(200, 30, 40)).as_deref(),
            Some("red")
        );
        assert_eq!(
            derive_accent_from_image(&solid_test_png(30, 180, 70)).as_deref(),
            Some("green")
        );
    }

    /// An 8x8 solid-color PNG so the decode→derive path is exercised end-to-end.
    fn solid_test_png(r: u8, g: u8, b: u8) -> Vec<u8> {
        let buf: image::RgbImage = image::ImageBuffer::from_pixel(8, 8, image::Rgb([r, g, b]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(buf)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }
}

/// True iff a `media_blob` row exists for `mid` (mirrors the persona-gallery
/// probe; the path is server-minted so a bad id is a 404, never a 500).
async fn media_exists(state: &AppState, mid: &str) -> surrealdb::Result<bool> {
    let mut resp = state
        .db
        .query("SELECT meta::id(id) AS id_key FROM type::record('media_blob', $mid);")
        .bind(("mid", mid.to_string()))
        .await?
        .check()?;
    Ok(resp.take::<Option<IdRow>>(0)?.is_some())
}
