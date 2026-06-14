//! W5/P2 directional warp sign (deferred from Foundation T0.2). The act layer
//! sets `--warp-dir` (+1 / -1 / 0) from the channel-list index sign of a
//! picker-driven switch; the incoming `.channel-view` slides from
//! `translateX(calc(var(--warp-dir) * 6%))` (`_content.scss:46`). Pure — no DOM.

/// The directional sign for a switch from `from_idx` to `to_idx` in the
/// channel list. Higher destination ⇒ +1 (slide in from the right), lower ⇒
/// -1 (from the left), same index ⇒ 0 (neutral dip). Either index `None`
/// (channel not in the current list — e.g. a cross-guild orbit-map dive) ⇒ 0.
pub fn warp_dir(from_idx: Option<usize>, to_idx: Option<usize>) -> i8 {
    match (from_idx, to_idx) {
        (Some(a), Some(b)) if b > a => 1,
        (Some(a), Some(b)) if b < a => -1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_destination_slides_from_right() {
        assert_eq!(warp_dir(Some(0), Some(3)), 1);
        assert_eq!(warp_dir(Some(2), Some(3)), 1);
    }

    #[test]
    fn lower_destination_slides_from_left() {
        assert_eq!(warp_dir(Some(3), Some(0)), -1);
        assert_eq!(warp_dir(Some(3), Some(2)), -1);
    }

    #[test]
    fn same_or_unknown_index_is_neutral() {
        assert_eq!(warp_dir(Some(2), Some(2)), 0);
        assert_eq!(warp_dir(None, Some(2)), 0);
        assert_eq!(warp_dir(Some(2), None), 0);
        assert_eq!(warp_dir(None, None), 0);
    }
}
