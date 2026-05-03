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
use crate::rasterizer::Rasterizer;
use crate::shaper::PositionedGlyph;
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
        if dst.is_empty() || glyphs.is_empty() {
            return Ok(());
        }
        let mut pen_x = x;
        let pen_y = y;
        for g in glyphs {
            let key = GlyphKey::new(face.id(), g.glyph_id, size_px);
            let cached = if let Some(c) = self.cache.get(&key) {
                c
            } else {
                let bitmap = Rasterizer::raster_glyph(face, g.glyph_id, size_px)?;
                let (off_x, off_y) = Rasterizer::glyph_offset(face, g.glyph_id, size_px)?;
                let entry = CachedGlyph {
                    bitmap,
                    offset_x: off_x,
                    offset_y: off_y,
                };
                self.cache.insert(key, entry.clone());
                entry
            };

            let glyph_x = pen_x + g.x_offset + cached.offset_x;
            let glyph_y = pen_y + g.y_offset + cached.offset_y;

            if !cached.bitmap.is_empty() {
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
                    &cached.bitmap.data,
                    cached.bitmap.width,
                    cached.bitmap.height,
                    cached.bitmap.width as usize,
                    color,
                );
            }

            pen_x += g.x_advance;
        }
        Ok(())
    }
}
