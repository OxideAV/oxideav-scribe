//! Quadratic-Bezier outline flattening.
//!
//! TrueType glyph outlines are sequences of contours, where each contour
//! is a closed loop of *on-curve* and *off-curve* points. The off-curve
//! points are quadratic-Bezier control handles. Two consecutive
//! off-curve points imply an *implicit* on-curve midpoint at their
//! midpoint — the standard TrueType reconstruction rule documented in
//! the Apple TrueType Reference Manual.
//!
//! This module turns a [`oxideav_ttf::TtOutline`] into a flat polyline
//! (`Vec<Vec<(f32, f32)>>`, one inner Vec per contour) at the
//! rasterisation scale. Each Bezier segment is recursively subdivided by
//! the de Casteljau split until the chord length is below a tolerance
//! (we use 0.5 px which is a standard scanline AA threshold).
//!
//! The de Casteljau subdivision for a quadratic Bezier
//! `B(t) = (1-t)^2 P0 + 2(1-t)t P1 + t^2 P2` at `t = 0.5` yields:
//!
//! ```text
//! M01 = (P0 + P1) / 2
//! M12 = (P1 + P2) / 2
//! M = (M01 + M12) / 2
//! left  = (P0, M01, M)
//! right = (M, M12, P2)
//! ```
//!
//! The two halves can be concatenated to reconstruct the full curve;
//! this is the textbook "split + recurse" algorithm.
//!
//! Coordinate convention: TrueType is Y-up with the origin on the
//! baseline; the rasterizer wants Y-down with the origin at the
//! top-left of the glyph bounding box. Conversion is done at flatten
//! time so downstream code never has to think about it.

use oxideav_ttf::TtOutline;

/// Maximum chord length (in raster pixels) we tolerate before splitting
/// a Bezier segment further.
const FLATTEN_TOLERANCE_PX: f32 = 0.5;

/// Hard cap on subdivision depth — guards against pathological control
/// points that would otherwise loop forever in floating-point. A real
/// glyph at sane sizes converges in 4..6 levels; 16 levels gives 65536
/// intermediate samples which is way past anything sensible.
const MAX_SUBDIV_DEPTH: u8 = 16;

/// A flattened glyph outline ready for the scanline rasterizer.
///
/// Coordinates are in raster pixels with the origin at the **top-left**
/// of `bounds`. The `bounds` field is the bounding box used for that
/// translation (in raster pixels, also Y-down).
#[derive(Debug, Clone, Default)]
pub struct FlatOutline {
    pub contours: Vec<Vec<(f32, f32)>>,
    pub bounds: FlatBounds,
}

/// Bounding box of a flattened outline, in raster pixels (Y-down).
#[derive(Debug, Clone, Copy, Default)]
pub struct FlatBounds {
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
}

impl FlatBounds {
    pub fn width(&self) -> f32 {
        (self.x_max - self.x_min).max(0.0)
    }
    pub fn height(&self) -> f32 {
        (self.y_max - self.y_min).max(0.0)
    }
    /// Pixel width rounded up.
    pub fn width_px(&self) -> u32 {
        self.width().ceil() as u32
    }
    /// Pixel height rounded up.
    pub fn height_px(&self) -> u32 {
        self.height().ceil() as u32
    }
}

/// Flatten a TrueType outline at `scale` (raster-pixel-per-font-unit),
/// translating to a Y-down coordinate system with the origin at the
/// top-left of the glyph bounding box.
///
/// Returns `None` for empty outlines (e.g. the space glyph).
pub fn flatten(outline: &TtOutline, scale: f32) -> Option<FlatOutline> {
    flatten_with_shear(outline, scale, 0.0)
}

/// Flatten with an optional horizontal shear. `shear_x_per_y` is the
/// `tan(angle)` value to apply in TT (Y-up) coordinates: each input
/// point at `(x, y)` becomes `(x + shear * y, y)` *before* the
/// raster-down conversion. Used by the rasterizer when synthesising
/// italic for an upright face — see [`crate::style`].
///
/// `shear_x_per_y == 0.0` is identical to [`flatten`].
///
/// The bounding box is recomputed from the actual mapped points (the
/// font's cached bbox is invalid once shear is applied).
pub fn flatten_with_shear(
    outline: &TtOutline,
    scale: f32,
    shear_x_per_y: f32,
) -> Option<FlatOutline> {
    if outline.contours.is_empty() {
        return None;
    }

    // Without shear we can use the font's cached bbox directly (faster +
    // matches what the original flatten() did exactly so existing tests
    // don't shift by a sub-pixel rounding).
    let bounds = if shear_x_per_y == 0.0 {
        let raw = outline.bounds?;
        FlatBounds {
            x_min: raw.x_min as f32 * scale,
            y_min: -(raw.y_max as f32) * scale,
            x_max: raw.x_max as f32 * scale,
            y_max: -(raw.y_min as f32) * scale,
        }
    } else {
        // Sheared: compute bbox from the sheared points themselves.
        let mut x_min = f32::INFINITY;
        let mut y_min = f32::INFINITY;
        let mut x_max = f32::NEG_INFINITY;
        let mut y_max = f32::NEG_INFINITY;
        for c in &outline.contours {
            for p in &c.points {
                let (sx, sy) = shear_point(p.x as f32, p.y as f32, shear_x_per_y);
                let rx = sx * scale;
                let ry = -sy * scale;
                x_min = x_min.min(rx);
                y_min = y_min.min(ry);
                x_max = x_max.max(rx);
                y_max = y_max.max(ry);
            }
        }
        if !x_min.is_finite() {
            return None;
        }
        FlatBounds {
            x_min,
            y_min,
            x_max,
            y_max,
        }
    };

    let mut contours = Vec::with_capacity(outline.contours.len());
    for c in &outline.contours {
        let pts = flatten_contour(&c.points, scale, &bounds, shear_x_per_y);
        if pts.len() >= 2 {
            contours.push(pts);
        }
    }

    if contours.is_empty() {
        return None;
    }
    Some(FlatOutline { contours, bounds })
}

/// Apply horizontal shear in TT (Y-up) coordinates. Italic-positive y
/// (above the baseline) shifts to the right when `shear_x_per_y > 0`.
#[inline]
fn shear_point(x: f32, y: f32, shear_x_per_y: f32) -> (f32, f32) {
    (x + shear_x_per_y * y, y)
}

/// Apply scale + Y-flip + bounds-relative translation to a single TT
/// point so the result is in raster pixels with origin at top-left of
/// the glyph bbox. Honour an optional horizontal shear (applied in TT
/// Y-up coordinates BEFORE the Y-flip) so synthesised italic produces
/// the visually-expected forward slant.
#[inline]
fn map_point(x: i16, y: i16, scale: f32, bounds: &FlatBounds, shear_x_per_y: f32) -> (f32, f32) {
    let (sx, sy) = shear_point(x as f32, y as f32, shear_x_per_y);
    let rx = sx * scale - bounds.x_min;
    let ry = -sy * scale - bounds.y_min;
    (rx, ry)
}

/// Flatten a single contour (closed loop, mix of on/off-curve points).
///
/// Implements the standard TrueType implicit-on-curve rule: when two
/// off-curve points are adjacent, the implied on-curve midpoint
/// becomes the segment endpoint, then a fresh quadratic continues from
/// it.
/// One ordered point in the rotated contour: (x, y, on-curve flag).
type OrderedPoint = (f32, f32, bool);

fn flatten_contour(
    pts: &[oxideav_ttf::Point],
    scale: f32,
    bounds: &FlatBounds,
    shear_x_per_y: f32,
) -> Vec<(f32, f32)> {
    if pts.is_empty() {
        return Vec::new();
    }
    let n = pts.len();

    // Find a starting on-curve point. If every point is off-curve (rare
    // but legal — Apple's "phantom on-curve" case), synthesise one at
    // the midpoint of pts[0]..pts[1] and rotate.
    let start_idx = pts.iter().position(|p| p.on_curve);
    let (start_xy, ordered): ((f32, f32), Vec<OrderedPoint>) = if let Some(s) = start_idx {
        let mut ord: Vec<OrderedPoint> = Vec::with_capacity(n);
        for i in 0..n {
            let p = pts[(s + i) % n];
            let (x, y) = map_point(p.x, p.y, scale, bounds, shear_x_per_y);
            ord.push((x, y, p.on_curve));
        }
        let s_xy = (ord[0].0, ord[0].1);
        (s_xy, ord)
    } else {
        // All-off-curve: the start is the midpoint of pts[0]..pts[1].
        let p0 = pts[0];
        let p1 = pts[1 % n];
        let (x0, y0) = map_point(p0.x, p0.y, scale, bounds, shear_x_per_y);
        let (x1, y1) = map_point(p1.x, p1.y, scale, bounds, shear_x_per_y);
        let mid = ((x0 + x1) * 0.5, (y0 + y1) * 0.5);
        let mut ord: Vec<OrderedPoint> = Vec::with_capacity(n + 1);
        // Insert the synthetic on-curve start, then walk the original
        // ring in order.
        ord.push((mid.0, mid.1, true));
        for p in pts.iter().take(n) {
            let (x, y) = map_point(p.x, p.y, scale, bounds, shear_x_per_y);
            ord.push((x, y, p.on_curve));
        }
        (mid, ord)
    };

    let mut prev_was_off = false;
    let mut prev_off_xy: Option<(f32, f32)> = None;
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(ordered.len() * 2);
    out.push(start_xy);

    // Iterate the ordered points (skipping index 0, which is the start
    // we already pushed).
    for &(x, y, on) in ordered.iter().skip(1) {
        if on {
            if prev_was_off {
                // Quadratic from out.last() — well, no: from the segment
                // start point — through prev_off_xy to (x, y).
                let p0 = *out.last().expect("started with start_xy");
                let p1 = prev_off_xy.expect("prev_was_off");
                subdivide_quad(&mut out, p0, p1, (x, y), 0);
                prev_was_off = false;
                prev_off_xy = None;
            } else {
                out.push((x, y));
            }
        } else if prev_was_off {
            // Two off-curve points in a row → implicit on-curve at
            // their midpoint terminates the previous quadratic, and a
            // new one starts.
            let prev_off = prev_off_xy.expect("prev_was_off");
            let mid = ((prev_off.0 + x) * 0.5, (prev_off.1 + y) * 0.5);
            let p0 = *out.last().expect("at least start");
            subdivide_quad(&mut out, p0, prev_off, mid, 0);
            prev_off_xy = Some((x, y));
            // prev_was_off stays true.
        } else {
            prev_was_off = true;
            prev_off_xy = Some((x, y));
        }
    }

    // Close the contour: handle a trailing off-curve point by curving
    // back to the start.
    if prev_was_off {
        let p1 = prev_off_xy.expect("prev_was_off");
        let p0 = *out.last().expect("non-empty");
        subdivide_quad(&mut out, p0, p1, start_xy, 0);
    } else {
        // Ensure the contour explicitly closes — the rasterizer doesn't
        // assume implicit closure.
        if let (Some(&last), first) = (out.last(), start_xy) {
            if (last.0 - first.0).abs() > 1e-3 || (last.1 - first.1).abs() > 1e-3 {
                out.push(start_xy);
            }
        }
    }

    out
}

/// Recursively subdivide a quadratic Bezier (`p0`, `p1`, `p2`) until
/// the chord between `p0` and `p2` is below `FLATTEN_TOLERANCE_PX` *and*
/// the off-curve handle `p1` lies within that tolerance of the chord.
/// Pushes one or more *output* points (excluding `p0`, including `p2`).
fn subdivide_quad(
    out: &mut Vec<(f32, f32)>,
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    depth: u8,
) {
    let dx = p2.0 - p0.0;
    let dy = p2.1 - p0.1;
    let chord_sq = dx * dx + dy * dy;
    // Distance from p1 to the chord — perpendicular component of
    // (p1 - p0) projected against the chord normal. We bound this
    // because a tiny chord with a wild control handle still produces a
    // visible bulge.
    let d_pdx = p1.0 - p0.0;
    let d_pdy = p1.1 - p0.1;
    let cross = d_pdx * dy - d_pdy * dx;
    let chord_len = chord_sq.sqrt();
    let perp = if chord_len > 1e-6 {
        (cross / chord_len).abs()
    } else {
        // Degenerate chord: fall back to the direct distance from p0
        // to p1 (which equals "how far the curve might bulge").
        (d_pdx * d_pdx + d_pdy * d_pdy).sqrt()
    };

    if depth >= MAX_SUBDIV_DEPTH
        || (chord_sq <= FLATTEN_TOLERANCE_PX * FLATTEN_TOLERANCE_PX && perp <= FLATTEN_TOLERANCE_PX)
    {
        out.push(p2);
        return;
    }

    // de Casteljau split at t = 0.5.
    let m01 = ((p0.0 + p1.0) * 0.5, (p0.1 + p1.1) * 0.5);
    let m12 = ((p1.0 + p2.0) * 0.5, (p1.1 + p2.1) * 0.5);
    let m = ((m01.0 + m12.0) * 0.5, (m01.1 + m12.1) * 0.5);

    subdivide_quad(out, p0, m01, m, depth + 1);
    subdivide_quad(out, m, m12, p2, depth + 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_ttf::{BBox, Contour, Point as TtPoint};

    fn outline_from(points: Vec<(i16, i16, bool)>) -> TtOutline {
        let pts: Vec<TtPoint> = points
            .into_iter()
            .map(|(x, y, on)| TtPoint { x, y, on_curve: on })
            .collect();
        let mut x_min = i16::MAX;
        let mut y_min = i16::MAX;
        let mut x_max = i16::MIN;
        let mut y_max = i16::MIN;
        for p in &pts {
            x_min = x_min.min(p.x);
            y_min = y_min.min(p.y);
            x_max = x_max.max(p.x);
            y_max = y_max.max(p.y);
        }
        TtOutline {
            contours: vec![Contour { points: pts }],
            bounds: Some(BBox {
                x_min,
                y_min,
                x_max,
                y_max,
            }),
        }
    }

    #[test]
    fn empty_outline_yields_none() {
        let o = TtOutline::default();
        assert!(flatten(&o, 0.05).is_none());
    }

    #[test]
    fn straight_triangle_round_trips() {
        // Triangle with all on-curve points.
        let o = outline_from(vec![(0, 0, true), (100, 0, true), (50, 100, true)]);
        let f = flatten(&o, 1.0).expect("non-empty outline");
        assert_eq!(f.contours.len(), 1);
        // 3 corner points + closing point = 4.
        assert_eq!(f.contours[0].len(), 4);
    }

    #[test]
    fn quadratic_curve_subdivides() {
        // A "tall" curve from (0, 0) to (100, 0) with control at
        // (50, 100) — visible bulge so subdivision must produce
        // intermediate points.
        let o = outline_from(vec![(0, 0, true), (50, 100, false), (100, 0, true)]);
        let f = flatten(&o, 1.0).expect("non-empty outline");
        // Should produce many intermediate points along the curve.
        assert!(
            f.contours[0].len() > 5,
            "expected subdivided curve, got {} points",
            f.contours[0].len()
        );
    }
}
