//! Scanline rasterizer with 4× vertical supersampling.
//!
//! # Algorithm
//!
//! The flattened outline (a set of closed polygonal contours from
//! [`crate::outline::flatten`]) is rasterised at 4× vertical
//! resolution into a temporary single-bit buffer using the
//! standard *active edge list* scanline fill algorithm:
//!
//! 1. Build an "edge table": one entry per outline segment with its
//!    `y_min`, `y_max`, `x` at `y_min`, and `dx/dy` slope.
//! 2. Sort by `y_min`, then walk scanlines top-to-bottom.
//! 3. At each scanline:
//!    - Move edges with `y_min == scanline` from the edge table into
//!      the active list.
//!    - Drop edges with `y_max == scanline` from the active list.
//!    - Sort the active list by current `x`.
//!    - Pair adjacent x-coordinates → fill pixels between each pair
//!      (even-odd rule).
//!    - Step every active edge by its slope.
//!
//! The supersampled buffer is then box-averaged 4 rows down to produce
//! 8-bit alpha. This trades ~2× memory for clean anti-aliasing without
//! any of the trig (perpendicular distance, signed area) that fancier
//! analytical AA rasterisers use; the output is visually
//! indistinguishable at typical type sizes.
//!
//! # Coordinate convention
//!
//! Input polylines are in raster pixels, Y-down, origin at the
//! top-left of the glyph bounding box. Output `AlphaBitmap` is the
//! same orientation.
//!
//! # Even-odd fill
//!
//! TrueType uses the non-zero winding rule in theory, but for
//! correctly-wound outlines (which most production fonts ship)
//! even-odd produces identical fills with simpler edge tracking. We
//! use even-odd; if a real-world font surfaces winding-rule failures
//! they get fixed in round 2.

use crate::face::Face;
use crate::outline::{flatten, FlatBounds};
use crate::Error;

/// Vertical supersampling factor for AA.
const SUPERSAMPLE: u32 = 4;

/// A grayscale alpha bitmap (one byte per pixel). Row-major,
/// stride = width.
#[derive(Debug, Clone, Default)]
pub struct AlphaBitmap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl AlphaBitmap {
    /// Construct an empty `width × height` alpha bitmap. All pixels are 0.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; (width as usize) * (height as usize)],
        }
    }

    /// True if the bitmap has zero pixels.
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Read the alpha at `(x, y)`. Out-of-range reads return 0.
    pub fn get(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.data[(y * self.width + x) as usize]
    }

    /// Total non-zero pixel count — handy for tests.
    pub fn nonzero_pixel_count(&self) -> usize {
        self.data.iter().filter(|&&b| b != 0).count()
    }
}

/// Per-glyph rasterisation entry-point. Accepts a `Face`, a glyph id
/// and a target pixel size; returns the AA glyph alpha bitmap.
///
/// The bitmap is empty when the glyph has no outline (e.g. the space
/// glyph) or when `size_px` rounds to zero.
#[derive(Debug)]
pub struct Rasterizer;

impl Rasterizer {
    /// Rasterise a single glyph at `size_px` pixels per em.
    pub fn raster_glyph(face: &Face, glyph_id: u16, size_px: f32) -> Result<AlphaBitmap, Error> {
        if size_px <= 0.0 {
            return Ok(AlphaBitmap::default());
        }
        let upem = face.units_per_em().max(1) as f32;
        let scale = size_px / upem;

        // Pull the outline.
        let outline = face.with_font(|font| font.glyph_outline(glyph_id))??;
        let flat = match flatten(&outline, scale) {
            Some(f) => f,
            None => return Ok(AlphaBitmap::default()),
        };

        Ok(rasterise_flat(&flat))
    }

    /// Compute the offset from the glyph's pen position to the
    /// top-left of the rasterised bitmap, in raster pixels (Y-down).
    /// Required when composing multiple glyphs onto a baseline because
    /// rasterised bitmaps live in their own bbox-local frame.
    ///
    /// Returns `(left_bearing_px, top_offset_px)` where
    /// `left_bearing_px = bounds.x_min` and
    /// `top_offset_px = bounds.y_min` (both relative to the glyph
    /// origin, after the Y-flip into raster space).
    pub fn glyph_offset(face: &Face, glyph_id: u16, size_px: f32) -> Result<(f32, f32), Error> {
        if size_px <= 0.0 {
            return Ok((0.0, 0.0));
        }
        let upem = face.units_per_em().max(1) as f32;
        let scale = size_px / upem;
        let outline = face.with_font(|font| font.glyph_outline(glyph_id))??;
        let flat = match flatten(&outline, scale) {
            Some(f) => f,
            None => return Ok((0.0, 0.0)),
        };
        Ok((flat.bounds.x_min, flat.bounds.y_min))
    }
}

/// Workhorse: rasterise a flattened outline into an alpha bitmap.
fn rasterise_flat(flat: &crate::outline::FlatOutline) -> AlphaBitmap {
    let w = flat.bounds.width_px();
    let h = flat.bounds.height_px();
    if w == 0 || h == 0 {
        return AlphaBitmap::default();
    }

    // Supersample only vertically: rasterise into a 4× tall buffer and
    // average 4 rows together for the final AA value.
    let ss_h = h * SUPERSAMPLE;

    // Build the edge table. Each edge is a non-horizontal line segment
    // (we drop horizontal edges — they contribute nothing to the
    // even-odd parity flip).
    //
    // Coordinates are scaled from the input (pixel-space, but bounds-
    // relative) to the supersampled grid in Y. X stays in pixel
    // space.
    let mut edges: Vec<Edge> = Vec::new();
    for contour in &flat.contours {
        if contour.len() < 2 {
            continue;
        }
        for i in 0..contour.len() {
            let (x0, y0) = contour[i];
            let (x1, y1) = contour[(i + 1) % contour.len()];
            // Last point of a closed contour usually equals the first;
            // we want to skip that explicit closure to avoid emitting
            // a duplicate zero-length edge.
            if (x0 - x1).abs() < 1e-6 && (y0 - y1).abs() < 1e-6 {
                continue;
            }
            let yss0 = y0 * SUPERSAMPLE as f32;
            let yss1 = y1 * SUPERSAMPLE as f32;
            if (yss0 - yss1).abs() < 1e-6 {
                continue; // horizontal: ignored for even-odd parity flip
            }
            // Order so y_min < y_max.
            let (mx0, my0, mx1, my1) = if yss0 < yss1 {
                (x0, yss0, x1, yss1)
            } else {
                (x1, yss1, x0, yss0)
            };
            let dydx_inv = (mx1 - mx0) / (my1 - my0); // dx per unit y
            edges.push(Edge {
                y_min: my0,
                y_max: my1,
                x_at_y_min: mx0,
                dxdy: dydx_inv,
            });
        }
    }

    if edges.is_empty() {
        return AlphaBitmap::new(w, h);
    }

    // Sort edges by y_min for predictable activation order.
    edges.sort_by(|a, b| {
        a.y_min
            .partial_cmp(&b.y_min)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Allocate the supersampled coverage buffer (one bit per cell, but
    // we use a Vec<u8> packed at one byte per row-cell for simplicity;
    // memory cost is 4× the final bitmap, easily affordable for
    // glyphs).
    let mut coverage: Vec<u8> = vec![0; (w as usize) * (ss_h as usize)];

    // Active edge list — re-built per scanline.
    let mut active: Vec<ActiveEdge> = Vec::new();
    let mut next_edge = 0usize;

    for ss_y in 0..ss_h {
        // Sample at the centre of each supersample row.
        let y = ss_y as f32 + 0.5;

        // Activate edges whose y_min has passed.
        while next_edge < edges.len() && edges[next_edge].y_min <= y {
            let e = &edges[next_edge];
            // Only keep if the edge spans this scanline.
            if e.y_max > y {
                let x = e.x_at_y_min + (y - e.y_min) * e.dxdy;
                active.push(ActiveEdge {
                    x,
                    y_max: e.y_max,
                    dxdy: e.dxdy,
                });
            }
            next_edge += 1;
        }
        // Drop edges that have ended.
        active.retain(|e| e.y_max > y);

        if active.is_empty() {
            continue;
        }

        // Sort by current x.
        active.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));

        // Pair-fill (even-odd).
        let row = &mut coverage[(ss_y as usize) * (w as usize)..(ss_y as usize + 1) * (w as usize)];
        let n = active.len();
        let mut i = 0;
        while i + 1 < n {
            let x0 = active[i].x;
            let x1 = active[i + 1].x;
            let lo = x0.max(0.0).floor() as i64;
            let hi = (x1.min(w as f32)).ceil() as i64;
            if hi > lo {
                let lo = lo.max(0) as usize;
                let hi = (hi as usize).min(w as usize);
                if hi > lo {
                    for px in &mut row[lo..hi] {
                        *px = 1;
                    }
                }
            }
            i += 2;
        }

        // Step every active edge by 1 unit-y for the next scanline.
        for e in &mut active {
            e.x += e.dxdy;
        }
    }

    // Box-average the supersampled buffer down to 8-bit AA.
    let mut bitmap = AlphaBitmap::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0u32;
            for s in 0..SUPERSAMPLE {
                let row = (y * SUPERSAMPLE + s) as usize;
                let idx = row * (w as usize) + (x as usize);
                sum += coverage[idx] as u32;
            }
            // Each cell is 0 or 1 → sum in 0..=SUPERSAMPLE → scale to
            // 0..=255 with rounding.
            let alpha = (sum * 255 + (SUPERSAMPLE / 2)) / SUPERSAMPLE;
            bitmap.data[(y * w + x) as usize] = alpha.min(255) as u8;
        }
    }

    bitmap
}

#[derive(Debug, Clone, Copy)]
struct Edge {
    y_min: f32,
    y_max: f32,
    x_at_y_min: f32,
    dxdy: f32,
}

#[derive(Debug, Clone, Copy)]
struct ActiveEdge {
    x: f32,
    y_max: f32,
    dxdy: f32,
}

// Quiet a clippy false positive: `FlatBounds` is referenced through
// `flat.bounds.{width_px, height_px}` further up; the import keeps the
// line concise even though the type isn't named directly.
#[allow(dead_code)]
fn _bounds_type_hint(_: FlatBounds) {}
