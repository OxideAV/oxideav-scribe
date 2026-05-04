//! Font request style — italic knob the shaper honours when emitting
//! glyph paths.
//!
//! - **Italic** is synthesised from `post.italicAngle` (a horizontal
//!   shear applied at outline-flatten time when the requested style is
//!   italic but the font itself is upright). See
//!   [`synthetic_italic_shear`].
//!
//! In every case, callers that have a real Italic / Bold variant of
//! the font available should prefer loading those as separate
//! [`crate::Face`]s — synthetic styles are the fallback for fonts
//! that ship only one cut.
//!
//! ## Why not just a `bool`?
//!
//! Because we want to remember the user's requested weight and italic
//! flag through the shaping pipeline so future synthesis rounds can
//! hook in without a public-API break. `Style` is `Copy` so it travels
//! through arguments cheaply.
//!
//! Default is `Style { italic: false, weight: 400 }` — upright Regular.

/// Font selection / synthesis request. Carried through the shaper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    /// Caller wants italic. If the underlying face is already italic
    /// (`Face::italic_angle()` is non-zero), no synthesis is applied —
    /// the font already provides the slant. If the face is upright,
    /// the consumer applies a horizontal shear of
    /// `tan(-DEFAULT_SYNTHETIC_ITALIC_DEG)` (`~12°` forward slant) at
    /// outline-flatten time.
    pub italic: bool,
    /// OpenType `usWeightClass` value (100..=1000). Consumed by
    /// downstream rasterizers wanting to synthesise bold; the shaper
    /// itself doesn't apply it (true bold should always come from a
    /// real Bold face when one is available).
    pub weight: u16,
}

impl Style {
    /// Upright, regular weight (400).
    pub const REGULAR: Style = Style {
        italic: false,
        weight: 400,
    };

    /// Upright, regular weight — same as `Style::REGULAR`. Convenience
    /// for symmetry with `italic()`.
    pub fn regular() -> Self {
        Self::REGULAR
    }

    /// Italic, regular weight (400).
    pub fn italic() -> Self {
        Self {
            italic: true,
            weight: 400,
        }
    }

    /// Builder: set the italic flag.
    #[must_use]
    pub fn with_italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    /// Builder: set the weight (clamped to 1..=1000 to keep the cache
    /// key well-defined; OpenType allows 100..=1000 in 100 increments
    /// but variable fonts can land any integer in between).
    #[must_use]
    pub fn with_weight(mut self, weight: u16) -> Self {
        self.weight = weight.clamp(1, 1000);
        self
    }
}

impl Default for Style {
    fn default() -> Self {
        Self::REGULAR
    }
}

/// Synthetic-italic shear angle in degrees. `tan(12°) ≈ 0.213`, which
/// matches the slant the major desktop renderers (Quartz, GDI+,
/// FreeType) apply when the font lacks an italic variant. Mirrors
/// historical Type-1 Oblique fonts which ship with `italicAngle = -12`.
pub const DEFAULT_SYNTHETIC_ITALIC_DEG: f32 = 12.0;

/// Threshold (in degrees) under which the font's own `italicAngle` is
/// considered "upright". Some upright faces ship a tiny non-zero value
/// for visual centring; if we sheared on top of that we'd look weird.
pub const ITALIC_ANGLE_EPSILON_DEG: f32 = 0.5;

/// Compute the horizontal shear (`x' = x + shear * y` in TT Y-up
/// coordinates) that synthesises italic for the requested style on a
/// face whose own `italic_angle` is `face_italic_deg` (TT/post
/// convention: negative for forward slant, 0 for upright).
///
/// Returns `0.0` when no synthesis is needed: either the request is
/// upright, or the face is already italic.
pub fn synthetic_italic_shear(style: Style, face_italic_deg: f32) -> f32 {
    if !style.italic {
        return 0.0;
    }
    if face_italic_deg.abs() > ITALIC_ANGLE_EPSILON_DEG {
        // Font is already slanted; honour it instead of double-shearing.
        return 0.0;
    }
    DEFAULT_SYNTHETIC_ITALIC_DEG.to_radians().tan()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_regular_upright() {
        assert_eq!(Style::default(), Style::REGULAR);
        assert!(!Style::default().italic);
        assert_eq!(Style::default().weight, 400);
    }

    #[test]
    fn italic_builder_sets_flag() {
        let s = Style::italic();
        assert!(s.italic);
        assert_eq!(s.weight, 400);
    }

    #[test]
    fn weight_is_clamped() {
        assert_eq!(Style::REGULAR.with_weight(0).weight, 1);
        assert_eq!(Style::REGULAR.with_weight(2_000).weight, 1000);
        assert_eq!(Style::REGULAR.with_weight(700).weight, 700);
    }

    #[test]
    fn upright_request_yields_zero_shear() {
        // Upright request on upright face: 0.
        assert_eq!(synthetic_italic_shear(Style::REGULAR, 0.0), 0.0);
        // Upright request on italic face: still 0 (we never un-italicise).
        assert_eq!(synthetic_italic_shear(Style::REGULAR, -12.0), 0.0);
    }

    #[test]
    fn italic_request_on_upright_face_yields_default_shear() {
        let shear = synthetic_italic_shear(Style::italic(), 0.0);
        let expected = DEFAULT_SYNTHETIC_ITALIC_DEG.to_radians().tan();
        assert!(
            (shear - expected).abs() < 1e-6,
            "shear = {shear}, expected = {expected}"
        );
        // Also ~0.213 (tan 12 deg).
        assert!(shear > 0.20 && shear < 0.22, "shear = {shear}");
    }

    #[test]
    fn italic_request_on_italic_face_yields_zero() {
        // Face already at -12 deg → no synthesis.
        assert_eq!(synthetic_italic_shear(Style::italic(), -12.0), 0.0);
        // Same for positive backslant.
        assert_eq!(synthetic_italic_shear(Style::italic(), 8.0), 0.0);
    }

    #[test]
    fn epsilon_band_still_synthesises() {
        // Tiny font angle (0.3 deg) is treated as upright.
        let shear = synthetic_italic_shear(Style::italic(), 0.3);
        assert!(shear > 0.20);
    }
}
