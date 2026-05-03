//! Glyph alpha bitmap → RGBA framebuffer compositor.
//!
//! Wraps `oxideav_pixfmt::alpha::blit_alpha_mask`, which already
//! implements the Porter-Duff "over" operator with straight-alpha
//! semantics on both source and destination. This module's
//! responsibility is the per-glyph placement maths: for each
//! [`PositionedGlyph`] we (1) fetch / rasterise its alpha bitmap, (2)
//! convert pen-relative coordinates to absolute destination pixels,
//! and (3) call `blit_alpha_mask` with the colour for the run.

use crate::cache::{CachedGlyph, GlyphCache, GlyphKey};
use crate::face::Face;
use crate::face_chain::{shear_for, FaceChain};
use crate::rasterizer::Rasterizer;
use crate::shaper::PositionedGlyph;
use crate::stroke::{dilate_alpha, dilate_offset};
use crate::style::Style;
use crate::{Error, Rgba};

/// An RGBA8 bitmap with straight alpha. Stride is `width * 4`.
#[derive(Debug, Clone, Default)]
pub struct RgbaBitmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaBitmap {
    /// Allocate a fully-transparent (alpha = 0) bitmap.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    /// Stride in bytes (`width * 4`).
    pub fn stride(&self) -> usize {
        (self.width as usize) * 4
    }

    /// True if the bitmap holds zero pixels.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Read RGBA at `(x, y)`. Out-of-range reads return `[0; 4]`.
    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let off = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        [
            self.data[off],
            self.data[off + 1],
            self.data[off + 2],
            self.data[off + 3],
        ]
    }

    /// Number of pixels with non-zero alpha.
    pub fn nonzero_alpha_count(&self) -> usize {
        self.data.chunks_exact(4).filter(|p| p[3] != 0).count()
    }
}

/// Glyph composer. Owns the LRU cache; multiple `compose_run` calls
/// reuse the same rasterised glyph bitmaps.
#[derive(Debug, Default)]
pub struct Composer {
    cache: GlyphCache,
}

impl Composer {
    /// Construct a composer with the default LRU capacity (256).
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with a specific cache capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            cache: GlyphCache::new(cap),
        }
    }

    /// Direct access to the underlying cache (mostly for diagnostics).
    pub fn cache(&self) -> &GlyphCache {
        &self.cache
    }

    /// Mutable cache access (lets callers reset hit/miss counters
    /// between renders if they want).
    pub fn cache_mut(&mut self) -> &mut GlyphCache {
        &mut self.cache
    }

    /// Compose a sequence of positioned glyphs onto `dst` starting at
    /// pen origin `(x, y)`. `color` is straight-alpha RGBA; the
    /// destination is also treated as straight-alpha RGBA.
    ///
    /// Single-face, upright (`Style::REGULAR`) — the round-1 entry
    /// point. Use [`Composer::compose_run_styled`] for italic / chain
    /// fallback, or [`Composer::compose_run_with_stroke`] for stroked
    /// borders.
    ///
    /// Per-run colour is honoured correctly: the cached glyph bitmap
    /// is grayscale alpha-only, and `color` is mixed in by
    /// `oxideav_pixfmt::blit_alpha_mask` at compose time. Calling this
    /// twice with different `color` values produces visibly different
    /// RGB values at glyph pixels with identical alpha shape.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_run(
        &mut self,
        glyphs: &[PositionedGlyph],
        face: &Face,
        size_px: f32,
        color: Rgba,
        dst: &mut RgbaBitmap,
        x: f32,
        y: f32,
    ) -> Result<(), Error> {
        // Wrap the single face in a length-1 chain and forward.
        let face_ref = SingleFace::Borrowed(face);
        self.compose_run_inner(
            glyphs,
            ChainRef::Single(face_ref),
            size_px,
            Style::REGULAR,
            color,
            None,
            dst,
            x,
            y,
        )
    }

    /// Styled multi-face variant. `chain` is consulted by
    /// `glyph.face_idx`; `style` controls italic synthesis (the
    /// composer applies the per-face shear derived from
    /// [`crate::style::synthetic_italic_shear`]).
    #[allow(clippy::too_many_arguments)]
    pub fn compose_run_styled(
        &mut self,
        glyphs: &[PositionedGlyph],
        chain: &FaceChain,
        size_px: f32,
        style: Style,
        color: Rgba,
        dst: &mut RgbaBitmap,
        x: f32,
        y: f32,
    ) -> Result<(), Error> {
        self.compose_run_inner(
            glyphs,
            ChainRef::Chain(chain),
            size_px,
            style,
            color,
            None,
            dst,
            x,
            y,
        )
    }

    /// Compose with an optional border stroke painted *under* the
    /// fill. The stroke is the alpha-dilated glyph silhouette in
    /// `stroke.color`; the fill is the original glyph in `fill_color`.
    /// Implements the ASS `\bord` semantics that mpv / ffmpeg use.
    #[allow(clippy::too_many_arguments)]
    pub fn compose_run_with_stroke(
        &mut self,
        glyphs: &[PositionedGlyph],
        chain: &FaceChain,
        size_px: f32,
        style: Style,
        fill_color: Rgba,
        stroke: Option<StrokeStyle>,
        dst: &mut RgbaBitmap,
        x: f32,
        y: f32,
    ) -> Result<(), Error> {
        // First pass: paint the stroke (if any) underneath the fill.
        if let Some(s) = stroke {
            if s.width_px > 0.0 && s.color[3] > 0 {
                self.compose_run_inner(
                    glyphs,
                    ChainRef::Chain(chain),
                    size_px,
                    style,
                    s.color,
                    Some(s.width_px),
                    dst,
                    x,
                    y,
                )?;
            }
        }
        // Second pass: paint the fill on top.
        self.compose_run_inner(
            glyphs,
            ChainRef::Chain(chain),
            size_px,
            style,
            fill_color,
            None,
            dst,
            x,
            y,
        )
    }

    /// Core compositing loop. `dilate_radius_px = Some(r)` requests a
    /// stroke pass (each glyph's alpha bitmap is dilated by r before
    /// blitting); `None` is the normal fill.
    #[allow(clippy::too_many_arguments)]
    fn compose_run_inner(
        &mut self,
        glyphs: &[PositionedGlyph],
        chain: ChainRef<'_>,
        size_px: f32,
        style: Style,
        color: Rgba,
        dilate_radius_px: Option<f32>,
        dst: &mut RgbaBitmap,
        x: f32,
        y: f32,
    ) -> Result<(), Error> {
        if dst.is_empty() || glyphs.is_empty() {
            return Ok(());
        }
        let mut pen_x = x;
        let pen_y = y;
        for g in glyphs {
            let face = chain.face(g.face_idx);
            let shear = shear_for(face, style);
            let key = GlyphKey::new_styled(face.id(), g.glyph_id, size_px, shear);
            let cached = if let Some(c) = self.cache.get(&key) {
                c
            } else {
                let bitmap =
                    Rasterizer::raster_glyph_styled(face, g.glyph_id, size_px, shear)?;
                let (off_x, off_y) =
                    Rasterizer::glyph_offset_styled(face, g.glyph_id, size_px, shear)?;
                let entry = CachedGlyph {
                    bitmap,
                    offset_x: off_x,
                    offset_y: off_y,
                };
                self.cache.insert(key, entry.clone());
                entry
            };

            // For the stroke pass, dilate the cached alpha bitmap on
            // the fly. Dilation is NOT cached because radius is
            // typically a per-cue parameter that varies by ASS style;
            // the alpha-mask sources stay cached.
            let (blit_bitmap, blit_dx, blit_dy) = if let Some(r) = dilate_radius_px {
                let dil = dilate_alpha(&cached.bitmap, r);
                let off = dilate_offset(r) as f32;
                (dil, -off, -off)
            } else {
                (cached.bitmap.clone(), 0.0, 0.0)
            };

            let glyph_x = pen_x + g.x_offset + cached.offset_x + blit_dx;
            let glyph_y = pen_y + g.y_offset + cached.offset_y + blit_dy;

            if !blit_bitmap.is_empty() {
                let dw = dst.width;
                let dh = dst.height;
                let ds = dst.stride();
                oxideav_pixfmt::blit_alpha_mask(
                    &mut dst.data,
                    dw,
                    dh,
                    ds,
                    glyph_x.round() as i32,
                    glyph_y.round() as i32,
                    &blit_bitmap.data,
                    blit_bitmap.width,
                    blit_bitmap.height,
                    blit_bitmap.width as usize,
                    color,
                );
            }

            pen_x += g.x_advance;
        }
        Ok(())
    }
}

/// Border stroke configuration for [`Composer::compose_run_with_stroke`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeStyle {
    /// Stroke half-width in pixels (the visible stroke thickness on
    /// each side of the glyph outline). 0 disables the stroke.
    pub width_px: f32,
    /// Straight-alpha RGBA stroke colour. Painted under the fill so
    /// it shows around the glyph silhouette, not on top.
    pub color: Rgba,
}

impl StrokeStyle {
    /// Convenience constructor.
    pub fn new(width_px: f32, color: Rgba) -> Self {
        Self { width_px, color }
    }
}

/// Internal helper: lets [`Composer::compose_run`] keep its single-face
/// signature without forcing every caller to allocate a `FaceChain`.
enum SingleFace<'a> {
    Borrowed(&'a Face),
}

/// Internal helper: unify "borrowed single face" + "borrowed chain"
/// in `compose_run_inner` so the loop body doesn't have to special-case.
enum ChainRef<'a> {
    Single(SingleFace<'a>),
    Chain(&'a FaceChain),
}

impl<'a> ChainRef<'a> {
    fn face(&self, idx: u16) -> &Face {
        match self {
            ChainRef::Single(SingleFace::Borrowed(f)) => f,
            ChainRef::Chain(c) => c.face(idx),
        }
    }
}
