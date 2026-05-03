//! Font request style — italic / weight knobs that the shaper +
//! rasterizer honour when producing glyph bitmaps.
//!
//! - **Italic** is synthesised from `post.italicAngle` (a horizontal
//!   shear applied at outline-flatten time when the requested style is
//!   italic but the font itself is upright). See
//!   [`synthetic_italic_shear`].
//! - **Bold** is synthesised by dilating the rasterised glyph alpha
//!   bitmap with a circular kernel whose radius scales with the
//!   weight delta and font size. See [`synthetic_bold_radius`]. Below
//!   [`SYNTHETIC_BOLD_THRESHOLD`] weight delta we don't bother — the
//!   radius would be sub-pixel at any reasonable size.
//!
//! In both cases, callers that have a real Italic / Bold variant of
//! the font available should prefer loading those as separate
//! [`crate::Face`]s — synthetic styles are the fallback for fonts
//! that ship only one cut.
//!
//! ## Why not just a `bool`?
//!
//! Because we want to remember the user's requested weight and italic
//! flag through the shaping + caching pipeline so future synthesis
//! rounds can hook in without a public-API break. `Style` is `Copy` so
//! it travels through arguments cheaply.
//!
//! Default is `Style { italic: false, weight: 400 }` — upright Regular.

/// Font selection / synthesis request. Carried through shape →
/// rasterise → cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    /// Caller wants italic. If the underlying face is already italic
    /// (`Face::italic_angle()` is non-zero), no synthesis is applied —
    /// the font already provides the slant. If the face is upright,
    /// the rasterizer applies a horizontal shear of
    /// `tan(-DEFAULT_SYNTHETIC_ITALIC_DEG)` (`~12°` forward slant) at
    /// outline-flatten time.
    pub italic: bool,
    /// OpenType `usWeightClass` value (100..=1000). When the requested
    /// weight exceeds the loaded face's natural `usWeightClass` by at
    /// least [`SYNTHETIC_BOLD_THRESHOLD`] (round 3 default: 200, i.e.
    /// two weight steps), the rasterised glyph alpha bitmap is
    /// dilated by [`synthetic_bold_radius`] pixels to thicken the
    /// strokes — matching what GDI+ / libass apply when an ASS cue
    /// requests `\b1` against a Regular face. For better visual
    /// quality, callers SHOULD prefer loading a real Bold face when
    /// one is available; synthetic-bold is the fallback.
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

/// Below this `weight_class` delta (request - face) we don't bother
/// synthesising bold — the dilation radius would be sub-pixel at any
/// reasonable text size and the visual change is invisible anyway.
/// 100 = one OpenType weight step (e.g. Regular → Medium).
pub const SYNTHETIC_BOLD_THRESHOLD: i32 = 200;

/// Dilation radius (in pixels) per 1.0 px of font size, per OpenType
/// weight-class step over the face's natural weight. The shaper
/// computes `radius_px = SYNTHETIC_BOLD_PX_PER_WEIGHT_STEP_PER_PX *
/// size_px * weight_delta`; for a Regular face requested as Bold
/// (delta = 300) at 32 px the radius is ~0.96 px, which matches what
/// Microsoft's "GDI synthetic bold" produces (and is what mpv /
/// libass apply when an ASS cue requests `\b1` against a Regular
/// face). At 16 px the same request produces ~0.48 px which rounds
/// to a 1-pixel dilation — visible but not garish.
pub const SYNTHETIC_BOLD_PX_PER_WEIGHT_STEP_PER_PX: f32 = 0.0001;

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

/// Compute the synthetic-bold dilation radius (in pixels) for the
/// requested style at `size_px` against a face whose natural weight
/// class is `face_weight`.
///
/// Returns `0.0` when no synthesis is needed: either the request is
/// at-or-below the face's weight, or the delta is below
/// [`SYNTHETIC_BOLD_THRESHOLD`].
///
/// Otherwise the radius is
/// `SYNTHETIC_BOLD_PX_PER_WEIGHT_STEP_PER_PX * size_px * delta`,
/// then clamped to a minimum of 1.0 pixel (the dilation kernel
/// needs an integer-pixel radius to produce visible thickening).
/// For Regular (400) → Bold (700) at 32 px the un-clamped value is
/// ~0.96 px → clamped to 1 px, matching Microsoft GDI's "synthetic
/// bold" at body sizes; at 64 px it grows to ~1.92 px → clamped to
/// 2 (via the dilation function's own ceiling), giving a heavier
/// look at headlines.
pub fn synthetic_bold_radius(style: Style, face_weight: u16, size_px: f32) -> f32 {
    if size_px <= 0.0 || !size_px.is_finite() {
        return 0.0;
    }
    let req = style.weight as i32;
    let face = face_weight as i32;
    if req <= face {
        return 0.0;
    }
    let delta = req - face;
    if delta < SYNTHETIC_BOLD_THRESHOLD {
        return 0.0;
    }
    let raw = SYNTHETIC_BOLD_PX_PER_WEIGHT_STEP_PER_PX * size_px * delta as f32;
    // The dilation kernel is integer-pixel; clamp up to at least 1
    // so the user sees actual thickening rather than a no-op.
    raw.max(1.0)
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

    // ---- synthetic_bold_radius ---------------------------------------

    #[test]
    fn bold_radius_zero_when_request_at_or_below_face() {
        // Regular request on Regular face → 0.
        assert_eq!(synthetic_bold_radius(Style::REGULAR, 400, 32.0), 0.0);
        // Regular request on Bold face → 0 (we never thin a bold face).
        assert_eq!(synthetic_bold_radius(Style::REGULAR, 700, 32.0), 0.0);
        // Bold request on Bold face → 0 (already bold).
        let bold = Style::REGULAR.with_weight(700);
        assert_eq!(synthetic_bold_radius(bold, 700, 32.0), 0.0);
        // Just-above-face below threshold → 0 (one weight step up).
        let medium = Style::REGULAR.with_weight(500);
        assert_eq!(synthetic_bold_radius(medium, 400, 32.0), 0.0);
    }

    #[test]
    fn bold_radius_grows_with_weight_delta() {
        let bold = Style::REGULAR.with_weight(700);
        let black = Style::REGULAR.with_weight(900);
        let r_bold = synthetic_bold_radius(bold, 400, 32.0);
        let r_black = synthetic_bold_radius(black, 400, 32.0);
        assert!(r_bold > 0.0);
        assert!(r_black > r_bold);
    }

    #[test]
    fn bold_radius_grows_with_size_in_unclamped_regime() {
        let bold = Style::REGULAR.with_weight(700);
        // At small sizes the 1.0-px clamp dominates so we can't check
        // linear scaling there. Pick two sizes well above the clamp
        // threshold (1.0 / 0.0001 / 300 ≈ 33 px).
        let r_med = synthetic_bold_radius(bold, 400, 64.0);
        let r_huge = synthetic_bold_radius(bold, 400, 256.0);
        assert!(r_med > 1.0, "64-px bold should exceed clamp: got {r_med}");
        assert!(r_huge > r_med);
        let ratio = r_huge / r_med;
        assert!(
            (ratio - 4.0).abs() < 1e-3,
            "expected 4× ratio in unclamped regime, got {ratio}"
        );
    }

    #[test]
    fn bold_radius_clamp_kicks_in_at_small_sizes() {
        let bold = Style::REGULAR.with_weight(700);
        // At 16 px the unclamped value is 0.0001 * 16 * 300 = 0.48 —
        // clamped up to 1.0 so the user actually sees thickening.
        let r = synthetic_bold_radius(bold, 400, 16.0);
        assert_eq!(r, 1.0, "small-size bold should clamp to 1.0 px, got {r}");
    }

    #[test]
    fn bold_radius_invalid_size_returns_zero() {
        let bold = Style::REGULAR.with_weight(700);
        assert_eq!(synthetic_bold_radius(bold, 400, 0.0), 0.0);
        assert_eq!(synthetic_bold_radius(bold, 400, -1.0), 0.0);
        assert_eq!(synthetic_bold_radius(bold, 400, f32::NAN), 0.0);
    }
}
