//! Glyph alpha bitmap → RGBA framebuffer compositor.
//!
//! Wraps `oxideav_pixfmt::alpha::blit_alpha_mask`, which already
//! implements the Porter-Duff "over" operator with straight-alpha
//! semantics on both source and destination. This module's
//! responsibility is the per-glyph placement maths: for each
//! [`PositionedGlyph`] we (1) fetch / rasterise its alpha bitmap, (2)
//! convert pen-relative coordinates to absolute destination pixels,
//! and (3) call `blit_alpha_mask` with the colour for the run.

use crate::cache::{subpixel_offset, subpixel_slot, CachedGlyph, GlyphCache, GlyphKey};
use crate::face::Face;
use crate::face_chain::{shear_for, FaceChain};
use crate::rasterizer::Rasterizer;
use crate::shaper::PositionedGlyph;
use crate::stroke::{dilate_alpha, dilate_offset};
use crate::style::{synthetic_bold_radius, Style};
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

    /// Bilinearly resample this bitmap to `(dst_width, dst_height)`.
    ///
    /// Used by the colour-bitmap composer path to scale a CBDT strike
    /// (typically 109 px or 136 px ppem for Noto Color Emoji) down to
    /// the requested raster size. Edge sampling clamps at the source
    /// borders so we never read outside the bitmap. Interpolation is
    /// performed in **straight-alpha space** independently per channel
    /// — the simpler model that matches what FreeType's bitmap-strike
    /// scaling does. Premultiplied interpolation produces sharper
    /// alpha-edge silhouettes but requires un-premultiplying afterwards
    /// to keep downstream `blit_*` straight-alpha consumers happy; for
    /// emoji glyphs at typical body-text sizes the visual difference is
    /// imperceptible.
    ///
    /// Returns the same bitmap unchanged when `dst_width == self.width`
    /// and `dst_height == self.height` (cheap pass-through). Returns an
    /// empty bitmap when either source or destination has a zero
    /// dimension.
    pub fn resample_bilinear(&self, dst_width: u32, dst_height: u32) -> RgbaBitmap {
        if self.is_empty() || dst_width == 0 || dst_height == 0 {
            return RgbaBitmap::default();
        }
        if dst_width == self.width && dst_height == self.height {
            return self.clone();
        }
        let src_w = self.width as usize;
        let src_h = self.height as usize;
        let dw = dst_width as usize;
        let dh = dst_height as usize;
        let mut out = RgbaBitmap::new(dst_width, dst_height);
        // Map each destination pixel centre to a source coordinate via
        // half-pixel offsets so the corner samples land on the source
        // corner pixel centres (the standard "centre-sample" mapping).
        let sx = self.width as f32 / dst_width as f32;
        let sy = self.height as f32 / dst_height as f32;
        for dy in 0..dh {
            // Source Y at the destination pixel centre.
            let src_y = (dy as f32 + 0.5) * sy - 0.5;
            let y0_f = src_y.floor();
            let fy = src_y - y0_f;
            let y0 = (y0_f as i32).clamp(0, src_h as i32 - 1) as usize;
            let y1 = (y0_f as i32 + 1).clamp(0, src_h as i32 - 1) as usize;
            for dx in 0..dw {
                let src_x = (dx as f32 + 0.5) * sx - 0.5;
                let x0_f = src_x.floor();
                let fx = src_x - x0_f;
                let x0 = (x0_f as i32).clamp(0, src_w as i32 - 1) as usize;
                let x1 = (x0_f as i32 + 1).clamp(0, src_w as i32 - 1) as usize;
                let off00 = (y0 * src_w + x0) * 4;
                let off10 = (y0 * src_w + x1) * 4;
                let off01 = (y1 * src_w + x0) * 4;
                let off11 = (y1 * src_w + x1) * 4;
                let dst_off = (dy * dw + dx) * 4;
                let w00 = (1.0 - fx) * (1.0 - fy);
                let w10 = fx * (1.0 - fy);
                let w01 = (1.0 - fx) * fy;
                let w11 = fx * fy;
                for c in 0..4 {
                    let s00 = self.data[off00 + c] as f32;
                    let s10 = self.data[off10 + c] as f32;
                    let s01 = self.data[off01 + c] as f32;
                    let s11 = self.data[off11 + c] as f32;
                    let mixed = s00 * w00 + s10 * w10 + s01 * w01 + s11 * w11;
                    out.data[dst_off + c] = mixed.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        out
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
    /// blitting); `None` is the normal fill. Synthetic bold (when
    /// `style.weight` exceeds the per-face natural weight) is applied
    /// to BOTH passes by adding the bold radius on top of the stroke
    /// radius — that way a Bold + bordered cue gets a thick stroke
    /// surrounding a thick fill, matching how Microsoft GDI+ and
    /// libass compose synthetic-bold-with-border.
    ///
    /// **Colour-bitmap dispatch (round 6 / #356).** For each glyph,
    /// faces that ship CBDT/CBLC for the glyph (typical for emoji
    /// codepoints) take the colour-bitmap branch: the strike is
    /// bilinearly resampled to `size_px` via
    /// [`Face::raster_color_glyph_at`] and blitted directly via
    /// [`compose_color_bitmap_over`] (carries its own colour, so the
    /// run's `color` parameter is ignored for that glyph). Stroke /
    /// synthetic-bold is **skipped** for colour bitmaps — those are
    /// outline-only effects. Outline glyphs continue through the
    /// alpha-mask path with the run colour.
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

            // Round 6 / #356 — colour-bitmap dispatch. The stroke pass
            // (`dilate_radius_px = Some(_)`) is skipped for colour
            // bitmaps because CBDT glyphs don't have a silhouette to
            // stroke; only the fill pass paints them. This means a
            // bordered subtitle line with mixed Latin + emoji gets the
            // border around the Latin glyphs but not around the emoji,
            // matching what libass + Chrome's text renderer do.
            if dilate_radius_px.is_none()
                && matches!(face.kind(), crate::FaceKind::Ttf)
                && face.has_color_bitmaps()
            {
                if let Some(cgb) = face.raster_color_glyph_at(g.glyph_id, size_px)? {
                    if !cgb.bitmap.is_empty() {
                        // Pen-relative placement: bitmap left edge at
                        // pen + bearing_x; bitmap top edge at
                        // baseline - bearing_y (raster Y-down).
                        let target_x = pen_x + g.x_offset;
                        let int_x = target_x.floor();
                        let glyph_x = int_x + cgb.bearing_x as f32;
                        let glyph_y = pen_y + g.y_offset - cgb.bearing_y as f32;
                        compose_color_bitmap_over(
                            dst,
                            glyph_x.round() as i32,
                            glyph_y.round() as i32,
                            &cgb.bitmap,
                        );
                        pen_x += g.x_advance;
                        continue;
                    }
                }
                // No colour bitmap for this glyph — fall through to
                // the outline / alpha-mask path. (Common in fonts that
                // ship both CBDT for emoji and outlines for Latin
                // fallback — Apple Color Emoji's ASCII fallbacks for
                // example.)
            }

            // Sub-pixel positioning: take the fractional part of the
            // desired pre-bbox pen X, snap it to one of SUBPIXEL_STEPS
            // slots, rasterise (with cache) at that sub-pixel offset.
            // The integer part of the pen X drives the blit origin so
            // the bitmap lands at the right whole-pixel position.
            let target_x = pen_x + g.x_offset;
            let int_x = target_x.floor();
            let frac_x = target_x - int_x;
            let slot = subpixel_slot(frac_x);
            let key = GlyphKey::new_subpixel(face.id(), g.glyph_id, size_px, shear, slot);
            let cached = if let Some(c) = self.cache.get(&key) {
                c
            } else {
                let sub_x = subpixel_offset(slot);
                let bitmap =
                    Rasterizer::raster_glyph_subpixel(face, g.glyph_id, size_px, shear, sub_x)?;
                let (off_x, off_y) =
                    Rasterizer::glyph_offset_subpixel(face, g.glyph_id, size_px, shear, sub_x)?;
                let entry = CachedGlyph {
                    bitmap,
                    offset_x: off_x,
                    offset_y: off_y,
                };
                self.cache.insert(key, entry.clone());
                entry
            };

            // Combine stroke radius (if any) with synthetic-bold
            // radius (if the requested weight exceeds the face's). A
            // total radius of 0 skips dilation entirely (cheap path).
            let bold_r = synthetic_bold_radius(style, face.weight_class(), size_px);
            let total_r = dilate_radius_px.unwrap_or(0.0) + bold_r;
            let (blit_bitmap, blit_dx, blit_dy) = if total_r > 0.0 {
                let dil = dilate_alpha(&cached.bitmap, total_r);
                let off = dilate_offset(total_r) as f32;
                (dil, -off, -off)
            } else {
                (cached.bitmap.clone(), 0.0, 0.0)
            };

            // X: the bitmap baked in the sub-pixel offset; only the
            // INTEGER part of the desired position contributes to the
            // blit origin (plus the bbox-relative offset_x and any
            // dilation shift).
            let glyph_x = int_x + cached.offset_x + blit_dx;
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

/// Blit an RGBA8 colour-bitmap glyph into a straight-alpha
/// [`RgbaBitmap`] using Porter-Duff "over". Out-of-bounds destination
/// pixels are clipped; fully-transparent source pixels are skipped
/// (cheap path). Used by the CBDT/CBLC composer dispatch in
/// [`Composer::compose_run_inner`].
fn compose_color_bitmap_over(dst: &mut RgbaBitmap, dst_x: i32, dst_y: i32, src: &RgbaBitmap) {
    if dst.is_empty() || src.is_empty() {
        return;
    }
    let dw = dst.width as i32;
    let dh = dst.height as i32;
    let sw = src.width as i32;
    let sh = src.height as i32;
    // Compute the visible window after clipping against dst bounds.
    let x0 = dst_x.max(0);
    let y0 = dst_y.max(0);
    let x1 = (dst_x + sw).min(dw);
    let y1 = (dst_y + sh).min(dh);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let dst_w_us = dst.width as usize;
    let src_w_us = src.width as usize;
    for y in y0..y1 {
        let src_y = (y - dst_y) as usize;
        let dst_row_off = (y as usize) * dst_w_us * 4;
        let src_row_off = src_y * src_w_us * 4;
        for x in x0..x1 {
            let src_x = (x - dst_x) as usize;
            let s_off = src_row_off + src_x * 4;
            let d_off = dst_row_off + (x as usize) * 4;
            let s = [
                src.data[s_off],
                src.data[s_off + 1],
                src.data[s_off + 2],
                src.data[s_off + 3],
            ];
            if s[3] == 0 {
                continue;
            }
            let d = [
                dst.data[d_off],
                dst.data[d_off + 1],
                dst.data[d_off + 2],
                dst.data[d_off + 3],
            ];
            let out = oxideav_pixfmt::over_straight(s, d);
            dst.data[d_off] = out[0];
            dst.data[d_off + 1] = out[1];
            dst.data[d_off + 2] = out[2];
            dst.data[d_off + 3] = out[3];
        }
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
