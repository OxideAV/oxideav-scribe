//! Pure-Rust font rasterizer + simple shaper + layout for the
//! [oxideav](https://github.com/OxideAV) framework.
//!
//! Round-1 scope:
//! - **Outline flattening** — quadratic-Bezier subdivision via the
//!   classic de Casteljau split (chord tolerance 0.5 px).
//! - **Scanline rasterisation** — active-edge-list fill with 4×
//!   vertical supersampling for anti-aliasing.
//! - **Shaper** — `cmap` + GSUB type 4 (ligatures) + GPOS type 2
//!   (pair kerning), enough for Latin / Cyrillic / Greek / basic CJK.
//! - **Composer** — Porter-Duff "over" via
//!   [`oxideav_pixfmt::blit_alpha_mask`] with straight-alpha
//!   destinations.
//! - **Layout** — line measurement + word-wrap (no bidi).
//! - **LRU cache** — glyph bitmap reuse keyed by
//!   `(face_id, glyph_id, size_q8)`.
//!
//! See `README.md` for a tour and the round-2/3 deferral list.

#![deny(missing_debug_implementations)]
#![warn(rust_2018_idioms)]

pub mod cache;
pub mod color;
pub mod color_glyph;
pub mod compose;
pub mod face;
pub mod face_chain;
pub mod layout;
pub mod outline;
pub mod rasterizer;
pub mod shaper;
pub mod stroke;
pub mod style;

pub use cache::{
    subpixel_offset, subpixel_slot, CachedGlyph, GlyphCache, GlyphKey, SUBPIXEL_STEPS,
};
pub use color::{Rgba, TRANSPARENT, WHITE};
pub use color_glyph::ColorGlyphBitmap;
pub use compose::{Composer, RgbaBitmap, StrokeStyle};
pub use face::{Face, FaceKind};
pub use face_chain::{shear_for, FaceChain};
pub use layout::{run_width, wrap_lines};
pub use outline::{
    flatten, flatten_cubic, flatten_cubic_with_shear, flatten_with_shear,
    flatten_with_shear_offset, FlatBounds, FlatOutline,
};
pub use rasterizer::{AlphaBitmap, Rasterizer};
pub use shaper::{PositionedGlyph, Shaper};
pub use stroke::{dilate_alpha, dilate_offset};
pub use style::{
    synthetic_bold_radius, synthetic_italic_shear, Style, DEFAULT_SYNTHETIC_ITALIC_DEG,
    ITALIC_ANGLE_EPSILON_DEG, SYNTHETIC_BOLD_PX_PER_WEIGHT_STEP_PER_PX, SYNTHETIC_BOLD_THRESHOLD,
};

/// Errors emitted by the scribe pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The underlying TTF parser rejected the bytes.
    Ttf(oxideav_ttf::Error),
    /// The underlying OTF (CFF) parser rejected the bytes.
    Otf(oxideav_otf::Error),
    /// `size_px` was non-positive (negative or NaN).
    InvalidSize,
    /// A `with_font` / `with_otf_font` call was made on a face of
    /// the wrong flavour.
    WrongFaceKind {
        expected: FaceKind,
        actual: FaceKind,
    },
}

impl From<oxideav_ttf::Error> for Error {
    fn from(e: oxideav_ttf::Error) -> Self {
        Self::Ttf(e)
    }
}

impl From<oxideav_otf::Error> for Error {
    fn from(e: oxideav_otf::Error) -> Self {
        Self::Otf(e)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ttf(e) => write!(f, "ttf error: {e}"),
            Self::Otf(e) => write!(f, "otf error: {e}"),
            Self::InvalidSize => f.write_str("non-positive font size"),
            Self::WrongFaceKind { expected, actual } => {
                write!(f, "wrong face kind: expected {expected:?}, got {actual:?}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// High-level convenience: shape `text`, rasterise + compose into a
/// freshly allocated `RgbaBitmap` sized to the run's bounds. The
/// resulting bitmap uses straight-alpha RGBA. Defaults to upright,
/// regular weight ([`Style::REGULAR`]); for italic / bold use
/// [`render_text_styled`].
///
/// Returns an empty bitmap if the run shapes to zero glyphs (or every
/// glyph is empty / non-rendering — e.g. a string of spaces).
pub fn render_text(
    face: &Face,
    text: &str,
    size_px: f32,
    color: Rgba,
) -> Result<RgbaBitmap, Error> {
    render_text_styled(face, text, size_px, color, Style::REGULAR)
}

/// Styled variant of [`render_text`]. Honours `style.italic` (applies
/// a synthetic horizontal shear when the requested style is italic
/// and the underlying face is upright; otherwise the font's own slant
/// is honoured) and `style.weight` (when the requested weight exceeds
/// the face's natural `usWeightClass` by at least
/// [`SYNTHETIC_BOLD_THRESHOLD`], the rasterised glyph alpha is
/// dilated for synthetic bold).
pub fn render_text_styled(
    face: &Face,
    text: &str,
    size_px: f32,
    color: Rgba,
    style: Style,
) -> Result<RgbaBitmap, Error> {
    if size_px <= 0.0 || !size_px.is_finite() {
        return Err(Error::InvalidSize);
    }
    let glyphs = Shaper::shape(face, text, size_px)?;
    if glyphs.is_empty() {
        return Ok(RgbaBitmap::default());
    }

    let shear = synthetic_italic_shear(style, face.italic_angle());
    let bold_r = crate::style::synthetic_bold_radius(style, face.weight_class(), size_px);

    // Pre-rasterise each glyph (once) so we know its bbox + can reuse
    // the bitmap during the compose pass below. Sub-pixel positioning:
    // each glyph is rasterised with the fractional part of its target
    // pen position baked into the outline, so the per-glyph alpha
    // pattern reflects its sub-pixel placement (sharper edges at small
    // body sizes than naive integer rounding). Synthetic-bold (when
    // `style.weight` > face's natural weight) dilates each bitmap
    // by `bold_r` pixels — the same path the composer uses.
    let mut pen = 0.0_f32;
    let mut x_min = f32::INFINITY;
    let mut y_min = f32::INFINITY;
    let mut x_max = f32::NEG_INFINITY;
    let mut y_max = f32::NEG_INFINITY;
    let mut prepared: Vec<(PositionedGlyph, AlphaBitmap, f32, f32, f32)> =
        Vec::with_capacity(glyphs.len());
    for g in &glyphs {
        let target = pen + g.x_offset;
        let int_x = target.floor();
        let frac_x = target - int_x;
        let slot = crate::cache::subpixel_slot(frac_x);
        let sub_x = crate::cache::subpixel_offset(slot);
        let mut bitmap =
            Rasterizer::raster_glyph_subpixel(face, g.glyph_id, size_px, shear, sub_x)?;
        let (mut off_x, mut off_y) =
            Rasterizer::glyph_offset_subpixel(face, g.glyph_id, size_px, shear, sub_x)?;
        if bold_r > 0.0 && !bitmap.is_empty() {
            bitmap = crate::stroke::dilate_alpha(&bitmap, bold_r);
            let off = crate::stroke::dilate_offset(bold_r) as f32;
            off_x -= off;
            off_y -= off;
        }
        if !bitmap.is_empty() {
            let glyph_x = int_x + off_x;
            let glyph_y = g.y_offset + off_y;
            x_min = x_min.min(glyph_x);
            y_min = y_min.min(glyph_y);
            x_max = x_max.max(glyph_x + bitmap.width as f32);
            y_max = y_max.max(glyph_y + bitmap.height as f32);
        }
        prepared.push((*g, bitmap, off_x, off_y, int_x));
        pen += g.x_advance;
    }

    if !x_min.is_finite() {
        // Every glyph was empty (whitespace-only).
        return Ok(RgbaBitmap::default());
    }

    // Round bounds outward to whole pixels and shift everything so
    // (x_min, y_min) lands at (0, 0) of the output bitmap.
    let x_origin = x_min.floor();
    let y_origin = y_min.floor();
    let width = (x_max.ceil() - x_origin).max(0.0) as u32;
    let height = (y_max.ceil() - y_origin).max(0.0) as u32;
    if width == 0 || height == 0 {
        return Ok(RgbaBitmap::default());
    }

    let mut dst = RgbaBitmap::new(width, height);
    let dw = dst.width;
    let dh = dst.height;
    let ds = dst.stride();
    for (g, bitmap, off_x, off_y, int_x) in prepared {
        if !bitmap.is_empty() {
            let glyph_x = int_x + off_x - x_origin;
            let glyph_y = g.y_offset + off_y - y_origin;
            oxideav_pixfmt::blit_alpha_mask(
                &mut dst.data,
                dw,
                dh,
                ds,
                glyph_x.round() as i32,
                glyph_y.round() as i32,
                &bitmap.data,
                bitmap.width,
                bitmap.height,
                bitmap.width as usize,
                color,
            );
        }
    }

    Ok(dst)
}

/// Multi-line variant: word-wrap to `max_width`, returns one bitmap
/// per line. Each line is independently sized to its own glyph bounds.
pub fn render_text_wrapped(
    face: &Face,
    text: &str,
    size_px: f32,
    color: Rgba,
    max_width: f32,
) -> Result<Vec<RgbaBitmap>, Error> {
    let lines = wrap_lines(face, text, size_px, max_width)?;
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        out.push(render_text(face, &line, size_px, color)?);
    }
    Ok(out)
}
