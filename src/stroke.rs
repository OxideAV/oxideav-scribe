//! Glyph stroke / outline synthesis via alpha-bitmap dilation.
//!
//! ## Approach: dilate-the-bitmap
//!
//! Round 2 implements text strokes as a *max-filter dilation* of the
//! glyph's alpha bitmap with a circular kernel. For each output pixel
//! the value is the max of the input pixels within a disc of radius N
//! around it. This is what `mpv`'s libass uses for ASS `\bord`,
//! what `ffmpeg`'s `subtitles` filter does for SRT styling, and what
//! every terminal-grade subtitle renderer ships in production.
//!
//! ### Trade-off vs. true offset-curve geometry
//!
//! A geometrically exact stroke would compute the *Minkowski sum* of
//! the outline with a disc of radius N — that is, every point along
//! the offset curves of every contour. This:
//! * preserves sharp corners exactly (the alpha-dilation rounds them
//!   off — visually identical at small bord values, noticeably softer
//!   at bord ≥ 4 px),
//! * supports miter / bevel / round join styles,
//! * costs O(contours × points × N²) for the geometry generation +
//!   needs a re-rasterisation pass, against O(width × height × N²) for
//!   the alpha dilation.
//!
//! At typical subtitle bord values (1–3 px) the visual difference is
//! sub-pixel; at larger values libass-style dilation is what users
//! expect because that's what they've grown up with. Round 3 may add
//! geometry-mode for cases where exact corners matter.
//!
//! ## API
//!
//! [`dilate_alpha`] is the only entry point. It returns a *new*
//! bitmap that is `2 * radius_ceil + 1` pixels larger in each
//! dimension so the dilated coverage doesn't get clipped at the
//! original bounds. The caller must shift its blit offset by
//! `(-radius_ceil, -radius_ceil)` to align the dilated bitmap with
//! the original glyph's pen origin.

use crate::rasterizer::AlphaBitmap;

/// Dilate `src` by a circular kernel of radius `radius_px`. Returns a
/// new alpha bitmap that is `(2 * ceil(radius_px) + 1)` pixels larger
/// in each dimension, with the original glyph centred — the caller
/// should subtract `ceil(radius_px)` from the blit X/Y offset to align.
///
/// `radius_px <= 0.0` returns the original bitmap unchanged.
///
/// The kernel is a *true circle* (Euclidean distance ≤ r), not a
/// square box: that produces visibly rounder corners than a box
/// dilation, which is what subtitle renderers ship.
pub fn dilate_alpha(src: &AlphaBitmap, radius_px: f32) -> AlphaBitmap {
    if src.is_empty() || radius_px <= 0.0 {
        return src.clone();
    }
    // Round up so a fractional radius doesn't lose the outermost ring.
    let r = radius_px.ceil() as u32;
    let r_sq = (radius_px * radius_px) as i32;
    let r_i = r as i32;

    // Pre-compute the row-by-row kernel half-width: for each `dy` in
    // -r..=r, the kernel reaches out `floor(sqrt(r² - dy²))` pixels in
    // X. This collapses the inner loop from 2D to 1D.
    let half_widths: Vec<i32> = (-r_i..=r_i)
        .map(|dy| {
            let rem = r_sq - dy * dy;
            if rem < 0 {
                -1 // empty row
            } else {
                (rem as f32).sqrt().floor() as i32
            }
        })
        .collect();

    let new_w = src.width + 2 * r;
    let new_h = src.height + 2 * r;
    let mut out = AlphaBitmap::new(new_w, new_h);

    let sw = src.width as i32;
    let sh = src.height as i32;
    let nw = new_w as i32;
    let nh = new_h as i32;

    for oy in 0..nh {
        for ox in 0..nw {
            // Walk the disc kernel, take the max of `src` under it.
            let mut best: u8 = 0;
            // The dst pixel `(ox, oy)` corresponds to source pixel
            // `(ox - r, oy - r)`. Sample sources in a disc around
            // that point.
            let cx = ox - r_i;
            let cy = oy - r_i;
            for dy in -r_i..=r_i {
                let hw = half_widths[(dy + r_i) as usize];
                if hw < 0 {
                    continue;
                }
                let sy = cy + dy;
                if sy < 0 || sy >= sh {
                    continue;
                }
                let row_off = (sy as u32 * src.width) as usize;
                let row = &src.data[row_off..row_off + src.width as usize];
                let x_lo = (cx - hw).max(0);
                let x_hi = (cx + hw).min(sw - 1);
                if x_hi < x_lo {
                    continue;
                }
                for sx in x_lo..=x_hi {
                    let v = row[sx as usize];
                    if v > best {
                        best = v;
                        if best == 255 {
                            break;
                        }
                    }
                }
                if best == 255 {
                    break;
                }
            }
            out.data[(oy as u32 * new_w + ox as u32) as usize] = best;
        }
    }

    out
}

/// Pixel offset that callers should subtract from the un-dilated
/// glyph blit position when compositing a dilated bitmap, so the
/// stroke is centred on the original glyph rather than shifted.
pub fn dilate_offset(radius_px: f32) -> i32 {
    if radius_px <= 0.0 {
        0
    } else {
        radius_px.ceil() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_pixel() -> AlphaBitmap {
        // Single fully-opaque pixel.
        let mut bm = AlphaBitmap::new(1, 1);
        bm.data[0] = 255;
        bm
    }

    #[test]
    fn zero_radius_returns_original() {
        let src = solid_pixel();
        let out = dilate_alpha(&src, 0.0);
        assert_eq!(out.width, 1);
        assert_eq!(out.height, 1);
        assert_eq!(out.data, vec![255]);
    }

    #[test]
    fn radius_1_pixel_grows_to_3x3_disc() {
        let src = solid_pixel();
        // Radius 1: 5×5 output ((1 + 2*1) × (1 + 2*1) = 3×3 — wait, the
        // padding is 2*r so 1 + 2 = 3, hence 3x3).
        // Expected disc-of-r=1 hit pattern (X = 255, . = 0):
        //     X X X
        //     X X X
        //     X X X
        // Because every pixel is within Euclidean distance 1 of the
        // centre when r = 1 ... actually corners are √2 ≈ 1.41 >
        // 1, so corners are 0.
        let out = dilate_alpha(&src, 1.0);
        assert_eq!(out.width, 3);
        assert_eq!(out.height, 3);
        // Corners should be 0 (outside disc).
        assert_eq!(out.get(0, 0), 0, "top-left corner");
        assert_eq!(out.get(2, 0), 0, "top-right corner");
        assert_eq!(out.get(0, 2), 0, "bottom-left corner");
        assert_eq!(out.get(2, 2), 0, "bottom-right corner");
        // Centre + 4-neighbours should be 255.
        assert_eq!(out.get(1, 1), 255, "centre");
        assert_eq!(out.get(0, 1), 255, "left");
        assert_eq!(out.get(2, 1), 255, "right");
        assert_eq!(out.get(1, 0), 255, "top");
        assert_eq!(out.get(1, 2), 255, "bottom");
    }

    #[test]
    fn dilation_preserves_max_value() {
        // Set a single 200-alpha pixel and confirm the dilation keeps
        // 200 (max-filter must not blur the value down).
        let mut src = AlphaBitmap::new(3, 3);
        src.data[4] = 200; // centre
        let out = dilate_alpha(&src, 1.0);
        // 5×5 output. Centre and 4-neighbours should all read 200.
        assert_eq!(out.get(2, 2), 200);
        assert_eq!(out.get(1, 2), 200);
        assert_eq!(out.get(3, 2), 200);
    }

    #[test]
    fn empty_bitmap_is_noop() {
        let src = AlphaBitmap::default();
        let out = dilate_alpha(&src, 2.0);
        assert!(out.is_empty());
    }

    #[test]
    fn dilate_offset_is_ceil_of_radius() {
        assert_eq!(dilate_offset(0.0), 0);
        assert_eq!(dilate_offset(1.0), 1);
        assert_eq!(dilate_offset(1.4), 2);
        assert_eq!(dilate_offset(2.0), 2);
    }
}
