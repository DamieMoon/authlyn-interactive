//! W5/P2 orbit-map picker geometry (the diegetic guild/channel chooser; the
//! VISUAL survived the pinch-entry kill — pill-tap is the only entry). ALL
//! geometry derives from the live viewport (UX-equality: no fixed-device pixel
//! math; verified across the POCO C3 floor → Nothing Phone 2). Pure fns — the
//! view reads vw/vh from `window` and feeds them here. Constants are the
//! prototype's (`a-orbit.html` mapGeom/buildMap). No DOM.

/// Resolved geometry for the current viewport.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapGeom {
    /// Channel-orbit ring radius (px).
    pub orbit_radius: f64,
    /// Far-server dock x offset from center (px, positive = right).
    pub far_x: f64,
    /// Far-server dock y offset from center (px, negative = up).
    pub far_y: f64,
}

/// A placed orbit node's center, relative to the orbit center (px).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NodePos {
    pub x: f64,
    pub y: f64,
}

fn clamp_f(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// Derive the map geometry from viewport width/height.
pub fn map_geom(vw: f64, vh: f64) -> MapGeom {
    let orbit_radius = clamp_f((vw / 2.0 - 45.0).min(vh / 2.0 - 160.0), 88.0, 170.0);
    let far_x = vw / 2.0 - 70.0;
    let far_y = -(vh / 2.0 - clamp_f(vh * 0.16, 96.0, 150.0));
    MapGeom {
        orbit_radius,
        far_x,
        far_y,
    }
}

/// The angle (degrees) for channel node `idx` of `count` on the ring. Starts
/// at the top (-90°) and spaces evenly. `count==0` returns -90 (no nodes, but
/// callers guard count first).
pub fn node_angle(idx: usize, count: usize) -> f64 {
    if count == 0 {
        return -90.0;
    }
    idx as f64 * (360.0 / count as f64) - 90.0
}

/// The node center (relative to the orbit center) for `idx` of `count` at the
/// given `radius`. Uses the `node_angle` placement.
pub fn node_pos(idx: usize, count: usize, radius: f64) -> NodePos {
    let a = node_angle(idx, count).to_radians();
    NodePos {
        x: radius * a.cos(),
        y: radius * a.sin(),
    }
}

/// Stable per-string seed (FNV-1a, 64-bit) — deterministic and std-RNG-free, so
/// a guild/channel's orbit is LOCKED across refreshes (owner ruling 2026-06-16:
/// seed per server; behaviour fixed oldest→newest, never reshuffled on render).
pub fn seed_of(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A channel's REALISTIC orbit (the owner's 1:1-or-higher overshoot): inner
/// channels revolve FASTER and outer ones slower (Kepler's third law, period
/// ∝ radius^1.5), with a small chance of a RETROGRADE (opposite-direction)
/// orbit — every parameter derived from `guild_seed ^ seed_of(channel_id)` so
/// the whole system is locked per server and never flickers between renders.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ChannelOrbit {
    /// Node-centre offset from the star at the START angle (px).
    pub x: f64,
    pub y: f64,
    /// Orbit radius (px) — oldest channels inner, newest outer (plus jitter).
    pub radius: f64,
    /// Seconds per revolution (closer = shorter period = faster).
    pub period_s: f64,
    /// True ≈ 17% of the time: this channel orbits the other way.
    pub retrograde: bool,
}

/// Mid-band revolution period (s) at exactly `base_radius`.
const BASE_PERIOD_S: f64 = 84.0;
/// Retrograde chance, in per-mille of channels (~17%).
const RETROGRADE_PERMILLE: u64 = 170;

/// Derive a channel's locked orbit from the guild seed + its id + age-index.
/// `idx` is the channel's position oldest→newest; `base_radius` is `map_geom`'s
/// ring radius (the band centre).
pub fn channel_orbit(
    guild_seed: u64,
    channel_id: &str,
    idx: usize,
    count: usize,
    base_radius: f64,
) -> ChannelOrbit {
    let h = guild_seed ^ seed_of(channel_id);
    // Radius: oldest (idx 0) innermost → newest outermost across a band, plus a
    // small STABLE per-channel jitter so the shells never look mechanical.
    let frac = if count <= 1 {
        0.5
    } else {
        idx as f64 / (count as f64 - 1.0)
    };
    let jitter = ((h >> 8) % 1000) as f64 / 1000.0; // 0..1, stable per channel
    let radius = base_radius * (0.72 + 0.46 * frac + 0.06 * (jitter - 0.5));
    // Kepler's third law: T ∝ r^1.5 → inner fast, outer slow.
    let period_s = BASE_PERIOD_S * (radius / base_radius).powf(1.5);
    // Start angle: evenly spaced from the top, plus a stable ±18° jitter so
    // equal-index ties never overlap.
    let angle = (node_angle(idx, count) + ((h % 37) as f64 - 18.0)).to_radians();
    let retrograde = (h % 1000) < RETROGRADE_PERMILLE;
    ChannelOrbit {
        x: radius * angle.cos(),
        y: radius * angle.sin(),
        radius,
        period_s,
        retrograde,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orbit_radius_clamps_on_the_poco_c3_floor() {
        // POCO C3 360x800: min(360/2-45, 800/2-160) = min(135, 240) = 135 → clamp 135.
        let g = map_geom(360.0, 800.0);
        assert!(
            (g.orbit_radius - 135.0).abs() < 1e-9,
            "got {}",
            g.orbit_radius
        );
    }

    #[test]
    fn orbit_radius_hits_lower_clamp_on_a_short_viewport() {
        // A short viewport drives the min below 88 → clamped up to 88.
        let g = map_geom(300.0, 360.0);
        assert!(
            (g.orbit_radius - 88.0).abs() < 1e-9,
            "got {}",
            g.orbit_radius
        );
    }

    #[test]
    fn orbit_radius_hits_upper_clamp_on_a_large_viewport() {
        // Desktop-ish 1200x900: min(555, 290)=290 → clamped down to 170.
        let g = map_geom(1200.0, 900.0);
        assert!(
            (g.orbit_radius - 170.0).abs() < 1e-9,
            "got {}",
            g.orbit_radius
        );
    }

    #[test]
    fn far_dock_keeps_servers_on_screen() {
        let g = map_geom(360.0, 800.0);
        assert!((g.far_x - 110.0).abs() < 1e-9, "far_x got {}", g.far_x);
        // far_y = -(400 - clamp(128, 96, 150)) = -(400-128) = -272.
        assert!((g.far_y - (-272.0)).abs() < 1e-9, "far_y got {}", g.far_y);
    }

    #[test]
    fn first_node_is_at_top_subsequent_evenly_spaced() {
        assert!((node_angle(0, 4) - (-90.0)).abs() < 1e-9);
        assert!((node_angle(1, 4) - 0.0).abs() < 1e-9);
        assert!((node_angle(2, 4) - 90.0).abs() < 1e-9);
        // The first node sits straight up from center: x≈0, y≈-radius.
        let p = node_pos(0, 4, 100.0);
        assert!(p.x.abs() < 1e-9, "x got {}", p.x);
        assert!((p.y - (-100.0)).abs() < 1e-9, "y got {}", p.y);
        // The second node (idx 1 of 4) is at 0° — straight right of center:
        // x≈+radius, y≈0. Pins an off-axis node placement, not just idx 0.
        let q = node_pos(1, 4, 100.0);
        assert!((q.x - 100.0).abs() < 1e-9, "x got {}", q.x);
        assert!(q.y.abs() < 1e-9, "y got {}", q.y);
    }

    #[test]
    fn node_angle_handles_single_node() {
        assert!((node_angle(0, 1) - (-90.0)).abs() < 1e-9);
    }

    #[test]
    fn channel_orbit_is_locked_per_channel() {
        // Same inputs → identical orbit, every render (owner's no-reshuffle rule).
        let s = seed_of("guild:foersoeksdaeck27");
        let a = channel_orbit(s, "channel:general", 1, 4, 130.0);
        let b = channel_orbit(s, "channel:general", 1, 4, 130.0);
        assert_eq!(a, b);
    }

    #[test]
    fn inner_channels_orbit_faster_than_outer() {
        // Kepler: oldest (idx 0, innermost) has a SHORTER period than the newest.
        let s = seed_of("guild:foersoeksdaeck27");
        let inner = channel_orbit(s, "c0", 0, 4, 130.0);
        let outer = channel_orbit(s, "c3", 3, 4, 130.0);
        assert!(
            inner.radius < outer.radius,
            "inner r {} should be < outer r {}",
            inner.radius,
            outer.radius
        );
        assert!(
            inner.period_s < outer.period_s,
            "inner period {} should be < outer {}",
            inner.period_s,
            outer.period_s
        );
    }

    #[test]
    fn retrograde_is_stable_per_channel() {
        let s = seed_of("guild:foersoeksdaeck27");
        let a = channel_orbit(s, "channel:y", 2, 5, 120.0);
        let b = channel_orbit(s, "channel:y", 2, 5, 120.0);
        assert_eq!(a.retrograde, b.retrograde);
    }

    #[test]
    fn some_channels_orbit_retrograde() {
        // Across a spread of ids, the ~17% retrograde chance must actually fire
        // (the feature exists), and must not fire for ALL (most are prograde).
        let s = seed_of("guild:foersoeksdaeck27");
        let mut retro = 0;
        let n = 200;
        for i in 0..n {
            if channel_orbit(s, &format!("channel:{i}"), i, n, 130.0).retrograde {
                retro += 1;
            }
        }
        assert!(retro > 0, "no channel went retrograde — feature dead");
        assert!(retro < n, "every channel retrograde — chance miscomputed");
    }
}
