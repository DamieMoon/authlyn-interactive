//! W5/P2 composer-orb charge ring (#E + #33 calibration). The ring fills with
//! message LENGTH; the old linear `chars/280` was prose-hostile (#33), so this
//! uses a log curve over WORD count: a one-liner shows a sliver, a paragraph
//! ~60%, only a saga pegs it. Pure math — the SVG `stroke-dashoffset` and the
//! `--charge` custom property are computed from these. No DOM.

/// The send button's progress-ring circumference (52×52 SVG, r≈24 → C≈151),
/// matching the prototype's `CIRC=151`.
pub const CIRC: f64 = 151.0;

/// The word count saturating the ring (a "saga"). `ln(1+250)` is the curve's
/// denominator so 250 words ≈ full.
const SATURATE_WORDS: f64 = 250.0;

/// Charge fraction 0..=1 from the composed text: `ln(1+words)/ln(1+250)`,
/// clamped. Empty/whitespace ⇒ 0. Words split on ASCII/Unicode whitespace.
pub fn charge_fraction(text: &str) -> f64 {
    let words = text.split_whitespace().count() as f64;
    if words <= 0.0 {
        return 0.0;
    }
    ((1.0 + words).ln() / (1.0 + SATURATE_WORDS).ln()).clamp(0.0, 1.0)
}

/// The SVG `stroke-dashoffset` for a given charge fraction: the ring is empty
/// at offset = CIRC and full at offset = 0 (the arc is dashed the full
/// circumference and revealed as the offset shrinks).
pub fn dash_offset(fraction: f64) -> f64 {
    CIRC * (1.0 - fraction.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero_charge() {
        assert_eq!(charge_fraction(""), 0.0);
        assert_eq!(charge_fraction("   \n\t "), 0.0);
    }

    #[test]
    fn one_liner_shows_a_sliver_paragraph_mid_saga_pegs() {
        // Real curve values (ln(1+n)/ln(251)): 1 word ≈ 0.124, 28 words ≈ 0.609
        // ("a paragraph"), 400 words = 1.085 → CLAMPED to 1.0. The saga assertion
        // therefore exercises the clamp, not the raw curve (the curve only
        // reaches 1.0 at exactly 250 words; anything beyond rides the clamp).
        let one = charge_fraction("hi");
        let para = charge_fraction(&"word ".repeat(28));
        let saga = charge_fraction(&"word ".repeat(400));
        assert!(one > 0.0 && one < 0.2, "one-liner is a sliver, got {one}");
        assert!(
            para > 0.55 && para < 0.65,
            "a paragraph (~28 words) ≈ 0.61, got {para}"
        );
        assert!(
            (saga - 1.0).abs() < 1e-9,
            "a saga pegs at the clamp (1.0), got {saga}"
        );
    }

    #[test]
    fn fraction_is_monotonic_in_word_count() {
        let a = charge_fraction("one two");
        let b = charge_fraction("one two three four five");
        assert!(b > a, "more words ⇒ more charge");
    }

    #[test]
    fn dash_offset_maps_empty_to_full_circ_and_full_to_zero() {
        assert!((dash_offset(0.0) - CIRC).abs() < 1e-9);
        assert!(dash_offset(1.0).abs() < 1e-9);
        assert!((dash_offset(0.5) - CIRC * 0.5).abs() < 1e-9);
    }

    #[test]
    fn dash_offset_clamps_out_of_range_fractions() {
        // Over-full saturates to a fully-revealed ring (offset 0), not negative.
        assert!(dash_offset(1.5).abs() < 1e-9, "over 1.0 clamps to 0");
        // Negative clamps to empty (offset == CIRC), not beyond the circumference.
        assert!(
            (dash_offset(-0.2) - CIRC).abs() < 1e-9,
            "below 0 clamps to CIRC"
        );
    }
}
