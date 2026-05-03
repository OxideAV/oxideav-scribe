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
pub mod compose;
pub mod face;
pub mod layout;
pub mod outline;
pub mod rasterizer;
pub mod shaper;

pub use cache::{CachedGlyph, GlyphCache, GlyphKey};
pub use color::{Rgba, TRANSPARENT, WHITE};
pub use compose::{Composer, RgbaBitmap};
pub use face::Face;
pub use layout::{run_width, wrap_lines};
pub use outline::{flatten, FlatBounds, FlatOutline};
pub use rasterizer::{AlphaBitmap, Rasterizer};
pub use shaper::{PositionedGlyph, Shaper};

/// Errors emitted by the scribe pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The underlying TTF parser rejected the bytes.
    Ttf(oxideav_ttf::Error),
    /// `size_px` was non-positive (negative or NaN).
    InvalidSize,
}

impl From<oxideav_ttf::Error> for Error {
    fn from(e: oxideav_ttf::Error) -> Self {
        Self::Ttf(e)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Ttf(e) => write!(f, "ttf error: {e}"),
            Self::InvalidSize => f.write_str("non-positive font size"),
        }
    }
}

impl std::error::Error for Error {}

/// High-level convenience: shape `text`, rasterise + compose into a
/// freshly allocated `RgbaBitmap` sized to the run's bounds. The
/// resulting bitmap uses straight-alpha RGBA.
///
/// Returns an empty bitmap if the run shapes to zero glyphs (or every
/// glyph is empty / non-rendering — e.g. a string of spaces).
pub fn render_text(
    face: &Face,
    text: &str,
    size_px: f32,
    color: Rgba,
) -> Result<RgbaBitmap, Error> {
    if size_px <= 0.0 || !size_px.is_finite() {
        return Err(Error::InvalidSize);
    }
    let glyphs = Shaper::shape(face, text, size_px)?;
    if glyphs.is_empty() {
        return Ok(RgbaBitmap::default());
    }

    // Pre-rasterise each glyph (once) so we know its bbox + can reuse
    // the bitmap during the compose pass below.
    let mut pen = 0.0_f32;
    let mut x_min = f32::INFINITY;
    let mut y_min = f32::INFINITY;
    let mut x_max = f32::NEG_INFINITY;
    let mut y_max = f32::NEG_INFINITY;
    let mut prepared: Vec<(PositionedGlyph, AlphaBitmap, f32, f32)> =
        Vec::with_capacity(glyphs.len());
    for g in &glyphs {
        let bitmap = Rasterizer::raster_glyph(face, g.glyph_id, size_px)?;
        let (off_x, off_y) = Rasterizer::glyph_offset(face, g.glyph_id, size_px)?;
        if !bitmap.is_empty() {
            let glyph_x = pen + g.x_offset + off_x;
            let glyph_y = g.y_offset + off_y;
            x_min = x_min.min(glyph_x);
            y_min = y_min.min(glyph_y);
            x_max = x_max.max(glyph_x + bitmap.width as f32);
            y_max = y_max.max(glyph_y + bitmap.height as f32);
        }
        prepared.push((*g, bitmap, off_x, off_y));
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
    let mut pen2 = 0.0_f32;
    for (g, bitmap, off_x, off_y) in prepared {
        if !bitmap.is_empty() {
            let glyph_x = pen2 + g.x_offset + off_x - x_origin;
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
        pen2 += g.x_advance;
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
