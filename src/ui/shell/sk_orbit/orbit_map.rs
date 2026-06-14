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
}
