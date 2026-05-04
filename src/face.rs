//! `Face` — owning wrapper around `oxideav_ttf::Font` /
//! `oxideav_otf::Font` plus per-face identity for the glyph-bitmap
//! cache.
//!
//! `Font<'a>` (in either underlying crate) borrows from the input
//! bytes which makes it awkward to pass around in a higher-level
//! renderer. `Face` owns the bytes via a boxed slice and re-parses
//! on demand through [`Face::with_font`] / [`Face::with_otf_font`].
//! We deliberately avoid `Pin` / self-referential structs (no
//! third-party deps allowed); the cost of a one-line re-parse on
//! each call is ~microseconds and dwarfed by glyph rasterisation.
//!
//! TTF and OTF cohabit through a [`FaceKind`] tag. The TTF path
//! returns quadratic-Bezier outlines (`oxideav_ttf::TtOutline`); the
//! OTF path returns cubic-Bezier outlines (`oxideav_otf::CubicOutline`).
//! Higher-level rasterisation code can dispatch via
//! [`Face::flatten_outline`] which converts whichever native form
//! the face holds into the unified `FlatOutline` polyline.

use crate::Error;
use oxideav_core::{
    FillRule, ImageRef, Node, Paint, Path, PathCommand, PathNode, Point, Rect, Rgba, Transform2D,
    VideoFrame, VideoPlane,
};

/// Which underlying parser this face wraps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceKind {
    /// TrueType / quadratic-Bezier outlines (`oxideav-ttf`).
    Ttf,
    /// OpenType-CFF / cubic-Bezier outlines (`oxideav-otf`).
    Otf,
}

/// Monotonic global id generator for `Face` instances. Used as the
/// primary key when caching rasterised glyph bitmaps so that two
/// faces that happen to share family names don't collide.
fn next_face_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// An owning, re-parseable wrapper around either an
/// `oxideav_ttf::Font` or an `oxideav_otf::Font`. The discriminant
/// is recorded in [`Face::kind`] so callers can pick the right
/// outline path.
#[derive(Debug)]
pub struct Face {
    bytes: Box<[u8]>,
    id: u64,
    kind: FaceKind,
    units_per_em: u16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    family: Option<String>,
    italic_angle: f32,
    weight_class: u16,
    /// `Some(i)` when this face was constructed from a TTC subfont via
    /// `from_ttc_bytes`. `with_font` re-parses through the TTC entry
    /// point so the right subfont is selected each time. `None` for
    /// plain sfnt-flavour faces (the common case).
    subfont_index: Option<u32>,
}

impl Face {
    /// Parse a TTF from owned bytes.
    pub fn from_ttf_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        let bytes: Box<[u8]> = bytes.into_boxed_slice();
        // Snapshot the metadata while we have the borrow.
        let (units_per_em, ascent, descent, line_gap, family, italic_angle, weight_class) = {
            let font = oxideav_ttf::Font::from_bytes(&bytes).map_err(Error::from)?;
            (
                font.units_per_em(),
                font.ascent(),
                font.descent(),
                font.line_gap(),
                font.family_name().map(|s| s.to_string()),
                font.italic_angle(),
                font.weight_class(),
            )
        };
        Ok(Self {
            bytes,
            id: next_face_id(),
            kind: FaceKind::Ttf,
            units_per_em,
            ascent,
            descent,
            line_gap,
            family,
            italic_angle,
            weight_class,
            subfont_index: None,
        })
    }

    /// Parse the `index`-th subfont out of an owned TrueType Collection
    /// (`.ttc` / `'ttcf'`) byte buffer. Convenience wrapper around
    /// `oxideav_ttf::Font::from_collection_bytes`. Index is recorded on
    /// the face so [`Face::with_font`] can re-parse the right subfont.
    pub fn from_ttc_bytes(bytes: Vec<u8>, index: u32) -> Result<Self, Error> {
        let bytes: Box<[u8]> = bytes.into_boxed_slice();
        let (units_per_em, ascent, descent, line_gap, family, italic_angle, weight_class) = {
            let font =
                oxideav_ttf::Font::from_collection_bytes(&bytes, index).map_err(Error::from)?;
            (
                font.units_per_em(),
                font.ascent(),
                font.descent(),
                font.line_gap(),
                font.family_name().map(|s| s.to_string()),
                font.italic_angle(),
                font.weight_class(),
            )
        };
        Ok(Self {
            bytes,
            id: next_face_id(),
            kind: FaceKind::Ttf,
            units_per_em,
            ascent,
            descent,
            line_gap,
            family,
            italic_angle,
            weight_class,
            subfont_index: Some(index),
        })
    }

    /// Parse an OTF (OpenType-CFF) font from owned bytes. Returns
    /// a `Face` whose [`Face::kind`] is [`FaceKind::Otf`] and whose
    /// outlines come back as cubic Beziers via the cubic flattener
    /// in [`crate::outline`].
    pub fn from_otf_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        let bytes: Box<[u8]> = bytes.into_boxed_slice();
        let (units_per_em, ascent, descent, line_gap, family) = {
            let font = oxideav_otf::Font::from_bytes(&bytes).map_err(Error::from)?;
            (
                font.units_per_em(),
                font.ascent(),
                font.descent(),
                font.line_gap(),
                font.family_name().map(|s| s.to_string()),
            )
        };
        Ok(Self {
            bytes,
            id: next_face_id(),
            kind: FaceKind::Otf,
            units_per_em,
            ascent,
            descent,
            line_gap,
            family,
            // OTF (CFF) carries italicAngle in the Top DICT. We
            // don't surface it through the Font public API in
            // round 1 — italic synthesis can fall back to the OS/2
            // (slant) heuristic via weight_class. Defaulting to 0
            // matches "upright".
            italic_angle: 0.0,
            // Round 1 of oxideav-otf doesn't expose OS/2 either;
            // 400 (Regular) is the safe default that avoids
            // synthetic-bold heuristics firing.
            weight_class: 400,
            subfont_index: None,
        })
    }

    /// Underlying parser flavour for this face.
    pub fn kind(&self) -> FaceKind {
        self.kind
    }

    /// Stable per-process id for this face. Used as the first component
    /// of the glyph-bitmap cache key.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Stable, content-derived identity for this face that is **stable
    /// across loads of the same font bytes** (in contrast to [`Face::id`]
    /// which is a per-process counter). Used as the producer-side
    /// component of the [`oxideav_core::Group::cache_key`] emitted by
    /// [`crate::Shaper::shape_to_paths`] so the downstream rasterizer's
    /// bitmap cache reuses the same memoised glyph across renderer
    /// instances and across program restarts.
    ///
    /// Implementation: a `DefaultHasher` digest of the font's leading
    /// bytes (up to 256) plus the byte length, plus the TTC subfont
    /// index when applicable. Two faces parsed from the same bytes get
    /// the same `stable_id`; two distinct fonts almost certainly do
    /// not.
    pub fn stable_id(&self) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut h = DefaultHasher::new();
        // Tag the discriminant + subfont so a TTC's subfont 0 and
        // subfont 1 (which share the outer byte buffer) end up with
        // different ids without us having to hash the entire TTC twice.
        (self.kind as u8).hash(&mut h);
        self.subfont_index.hash(&mut h);
        // Include the total byte length so two fonts that share a
        // common header prefix (rare but possible across stripped
        // variants of the same family) still distinguish.
        (self.bytes.len() as u64).hash(&mut h);
        // Hash the leading bytes — the sfnt header + the table
        // directory both live here and are highly font-specific.
        let prefix = &self.bytes[..self.bytes.len().min(256)];
        prefix.hash(&mut h);
        h.finish()
    }

    /// Family name from the font's `name` table. May be `None` for
    /// stripped or non-standard fonts.
    pub fn family_name(&self) -> Option<&str> {
        self.family.as_deref()
    }

    /// Units per em (`head.unitsPerEm`). Practically always 1024 or
    /// 2048; never zero in valid fonts.
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }

    /// Typographic ascent in raster pixels at `size_px`.
    pub fn ascent_px(&self, size_px: f32) -> f32 {
        self.ascent as f32 * size_px / self.units_per_em as f32
    }

    /// Typographic descent in raster pixels (negative for fonts with
    /// strokes below the baseline).
    pub fn descent_px(&self, size_px: f32) -> f32 {
        self.descent as f32 * size_px / self.units_per_em as f32
    }

    /// Recommended line height: `ascent - descent + line_gap`, in
    /// raster pixels.
    pub fn line_height_px(&self, size_px: f32) -> f32 {
        let units = self.ascent as i32 - self.descent as i32 + self.line_gap as i32;
        units as f32 * size_px / self.units_per_em as f32
    }

    /// `post.italicAngle` in degrees (negative for forward slanted
    /// faces, 0 for upright). Used by [`crate::style`] to decide
    /// whether to synthesise italic for an upright font or honour the
    /// font's own slant.
    pub fn italic_angle(&self) -> f32 {
        self.italic_angle
    }

    /// `OS/2.usWeightClass` (100..=1000). 400 if the font has no
    /// `OS/2` table.
    pub fn weight_class(&self) -> u16 {
        self.weight_class
    }

    /// Run a closure with a freshly-parsed `oxideav_ttf::Font<'_>`
    /// view of the owned bytes. We re-parse on each call instead of
    /// storing a self-referential `Font<'static>` (which would
    /// require unsafe or a third-party crate like `ouroboros`, both
    /// of which we avoid). Re-parsing is read-only header walking —
    /// well under a millisecond on any modern font.
    ///
    /// Returns `Error::WrongFaceKind` if this face was constructed
    /// from OTF bytes; use [`Face::with_otf_font`] in that case.
    pub fn with_font<R>(&self, f: impl FnOnce(&oxideav_ttf::Font<'_>) -> R) -> Result<R, Error> {
        if self.kind != FaceKind::Ttf {
            return Err(Error::WrongFaceKind {
                expected: FaceKind::Ttf,
                actual: self.kind,
            });
        }
        let font = match self.subfont_index {
            Some(i) => {
                oxideav_ttf::Font::from_collection_bytes(&self.bytes, i).map_err(Error::from)?
            }
            None => oxideav_ttf::Font::from_bytes(&self.bytes).map_err(Error::from)?,
        };
        Ok(f(&font))
    }

    /// True if this face is the `i`-th subfont of a TrueType Collection
    /// (the `bytes` buffer holds the WHOLE TTC; the subfont is selected
    /// at parse-time). Returns `None` for plain sfnt-flavour faces.
    pub fn subfont_index(&self) -> Option<u32> {
        self.subfont_index
    }

    /// Run a closure with a freshly-parsed `oxideav_otf::Font<'_>`
    /// view of the owned bytes. Mirrors [`Face::with_font`] for the
    /// CFF / cubic-Bezier path.
    ///
    /// Returns `Error::WrongFaceKind` if this face was constructed
    /// from TTF bytes.
    pub fn with_otf_font<R>(
        &self,
        f: impl FnOnce(&oxideav_otf::Font<'_>) -> R,
    ) -> Result<R, Error> {
        if self.kind != FaceKind::Otf {
            return Err(Error::WrongFaceKind {
                expected: FaceKind::Otf,
                actual: self.kind,
            });
        }
        let font = oxideav_otf::Font::from_bytes(&self.bytes).map_err(Error::from)?;
        Ok(f(&font))
    }

    /// Returns the raw glyph outline as vector commands in the font's
    /// **native Y-up font-unit coordinate space** (no Y-flip, no scaling
    /// applied — the canonical "1 em = `units_per_em` units" frame).
    ///
    /// - TT outlines map to `MoveTo` + `LineTo` + `QuadCurveTo` + `Close`.
    ///   Two consecutive off-curve points expand to an implicit on-curve
    ///   midpoint (the standard TrueType reconstruction rule).
    /// - CFF outlines map to `MoveTo` + `LineTo` + `CubicCurveTo` +
    ///   `Close`, mirroring the Type 2 charstring decode directly.
    /// - Bitmap-only glyphs (CBDT/sbix where the face has no outline
    ///   table) return `None`. Use [`Face::glyph_node`] for a vector
    ///   wrapper that handles the bitmap-vs-outline dispatch.
    /// - COLRv1 layered glyphs (round-6 work) return the *base* outline
    ///   here; the layered group is exposed via [`Face::glyph_node`] in
    ///   round 6.
    ///
    /// The Y-axis convention deliberately stays Y-up so the returned
    /// `Path` is "what the font says". Callers that want Y-down
    /// (oxideav-core's render convention) should compose with
    /// `Transform2D::scale(scale, -scale)` and an appropriate
    /// translation, or use [`Face::glyph_node`] which bakes that flip
    /// into a render-ready `Node`.
    pub fn glyph_path(&self, glyph_id: u16) -> Option<Path> {
        match self.kind {
            FaceKind::Ttf => {
                // CBDT-only glyphs (e.g. emoji) parse as empty outlines.
                let outline = self.with_font(|f| f.glyph_outline(glyph_id)).ok()?.ok()?;
                if outline.contours.is_empty() {
                    return None;
                }
                Some(tt_outline_to_path(&outline))
            }
            FaceKind::Otf => {
                let outline = self
                    .with_otf_font(|f| f.glyph_outline(glyph_id))
                    .ok()?
                    .ok()?;
                if outline.contours.is_empty() {
                    return None;
                }
                Some(cff_outline_to_path(&outline))
            }
        }
    }

    /// Returns a self-contained `Node` for `glyph_id` ready to be
    /// positioned at the pen origin — its local origin (0, 0) is the
    /// glyph's pen origin, X grows rightward, Y grows downward (matching
    /// oxideav-core / SVG / PDF raster conventions). The Y-flip + size
    /// scale are baked into the path so callers don't have to reason
    /// about font units.
    ///
    /// Dispatch:
    /// - **Outline glyph** → `Node::Path(PathNode { path, fill: Some(black), .. })`.
    ///   Replace `fill` if the caller wants colour.
    /// - **Bitmap glyph** (CBDT/sbix from round 5) → `Node::Image(ImageRef { ... })`
    ///   carrying the rasterised RGBA bitmap as a `VideoFrame`. Bounds
    ///   are sized to the bitmap's CBDT-declared placement at the
    ///   strike's native ppem (caller scales via the outer
    ///   `Transform2D` when blitting at a non-strike size).
    /// - **COLRv1 layered glyph** (round 6 — not yet implemented) — for
    ///   now, falls through to the outline path. Round 6 will return a
    ///   `Group` of `PathNode`s here, one per COLR layer.
    ///
    /// Returns `None` for empty / non-rendering glyphs (e.g. SPACE).
    pub fn glyph_node(&self, glyph_id: u16, size_px: f32) -> Option<Node> {
        if size_px <= 0.0 || !size_px.is_finite() {
            return None;
        }

        // Bitmap dispatch first: a face that ships CBDT for this glyph
        // (typical for emoji codepoints) wins over any empty-outline
        // fallback. CBDT-only fonts have no outline at all so this
        // branch is the only path that produces a renderable glyph.
        if matches!(self.kind, FaceKind::Ttf) && self.has_color_bitmaps() {
            if let Ok(Some(cgb)) = self.raster_color_glyph(glyph_id, size_px) {
                if !cgb.bitmap.is_empty() {
                    let w = cgb.bitmap.width;
                    let h = cgb.bitmap.height;
                    // Pack the RGBA8 into a VideoFrame with one plane.
                    let stride = (w as usize) * 4;
                    let frame = VideoFrame {
                        pts: None,
                        planes: vec![VideoPlane {
                            stride,
                            data: cgb.bitmap.data.clone(),
                        }],
                    };
                    // CBDT bearing_x / bearing_y describe the glyph
                    // box at the strike's native ppem (ppem). Convert
                    // to the requested raster size by `size_px / ppem`.
                    let strike_scale = if cgb.ppem > 0 {
                        size_px / cgb.ppem as f32
                    } else {
                        1.0
                    };
                    // Pen-relative placement: the bitmap left edge sits
                    // bearing_x px right of the pen, the bitmap top
                    // edge sits bearing_y px ABOVE the pen (so in
                    // Y-down space, top-edge Y = -bearing_y).
                    let bx = cgb.bearing_x as f32 * strike_scale;
                    let by = -(cgb.bearing_y as f32) * strike_scale;
                    let bw = w as f32 * strike_scale;
                    let bh = h as f32 * strike_scale;
                    return Some(Node::Image(ImageRef {
                        frame: Box::new(frame),
                        bounds: Rect {
                            x: bx,
                            y: by,
                            width: bw,
                            height: bh,
                        },
                        transform: Transform2D::identity(),
                    }));
                }
            }
        }

        // Outline path: take the Y-up native Path and bake in
        // `scale * (1, -1)` so the resulting Path lives directly in
        // Y-down raster pixels at `size_px`. (We could ship a wrapping
        // `Group { transform: scale(scale, -scale), .. }` instead;
        // baking the transform keeps the Node leaf-shaped and lets
        // shape_to_paths emit a pure translation per glyph.)
        let raw = self.glyph_path(glyph_id)?;
        let upem = self.units_per_em.max(1) as f32;
        let scale = size_px / upem;
        let path = scale_and_flip_path(&raw, scale);
        Some(Node::Path(PathNode {
            path,
            fill: Some(Paint::Solid(Rgba::opaque(0, 0, 0))),
            stroke: None,
            fill_rule: FillRule::NonZero,
        }))
    }
}

// -- Outline → vector::Path converters -----------------------------------

/// Apply `(x, y) -> (x*scale, -y*scale)` to every coordinate in `src`.
/// Used by [`Face::glyph_node`] to bake the Y-flip + size-scale into the
/// returned outline so the `Path` is in raster pixels (Y-down) ready to
/// blit, without an enclosing `Group::transform`.
fn scale_and_flip_path(src: &Path, scale: f32) -> Path {
    let mut out = Path {
        commands: Vec::with_capacity(src.commands.len()),
    };
    let map = |p: Point| Point::new(p.x * scale, -p.y * scale);
    for cmd in &src.commands {
        // Glyph outlines never emit ArcTo (TT/CFF have no arc primitive
        // — TT's quadratics + CFF's cubics are it), so the match arms
        // below cover every variant we'll ever see. The wildcard is a
        // forward-compat safety net for the `#[non_exhaustive]` enum.
        let new = match *cmd {
            PathCommand::MoveTo(p) => PathCommand::MoveTo(map(p)),
            PathCommand::LineTo(p) => PathCommand::LineTo(map(p)),
            PathCommand::QuadCurveTo { control, end } => PathCommand::QuadCurveTo {
                control: map(control),
                end: map(end),
            },
            PathCommand::CubicCurveTo { c1, c2, end } => PathCommand::CubicCurveTo {
                c1: map(c1),
                c2: map(c2),
                end: map(end),
            },
            PathCommand::Close => PathCommand::Close,
            other => other,
        };
        out.commands.push(new);
    }
    out
}

/// Convert a TrueType outline (quadratic Beziers in font-unit Y-up
/// coordinates) to a [`Path`] of MoveTo / LineTo / QuadCurveTo / Close.
///
/// Implements the standard TrueType reconstruction:
/// - Pick the first on-curve point of each contour as the starting
///   point (or the midpoint of `pts[0]..pts[1]` if every point is
///   off-curve — the rare "phantom on-curve" Apple-TT case).
/// - On-curve after on-curve → `LineTo`.
/// - On-curve after off-curve → `QuadCurveTo { control: prev_off, end: on }`.
/// - Off-curve after off-curve → emit an implicit on-curve at the
///   midpoint via `QuadCurveTo`, then keep walking with the new
///   off-curve as the next control point.
/// - Trailing off-curve at end-of-contour curves back to the start
///   point.
/// - Each contour terminates with `PathCommand::Close`.
fn tt_outline_to_path(outline: &oxideav_ttf::TtOutline) -> Path {
    let mut out = Path::new();
    for contour in &outline.contours {
        let pts = &contour.points;
        if pts.is_empty() {
            continue;
        }
        let n = pts.len();
        // Find the first on-curve point; if none, synthesise a start at
        // the midpoint of pts[0]..pts[1] (Apple-TT phantom on-curve).
        let start_idx = pts.iter().position(|p| p.on_curve);
        let (start_xy, ordered): (Point, Vec<(Point, bool)>) = if let Some(s) = start_idx {
            let mut ord: Vec<(Point, bool)> = Vec::with_capacity(n);
            for i in 0..n {
                let p = pts[(s + i) % n];
                ord.push((Point::new(p.x as f32, p.y as f32), p.on_curve));
            }
            (ord[0].0, ord)
        } else {
            let p0 = pts[0];
            let p1 = pts[1 % n];
            let mid = Point::new(
                (p0.x as f32 + p1.x as f32) * 0.5,
                (p0.y as f32 + p1.y as f32) * 0.5,
            );
            let mut ord: Vec<(Point, bool)> = Vec::with_capacity(n + 1);
            ord.push((mid, true));
            for p in pts.iter().take(n) {
                ord.push((Point::new(p.x as f32, p.y as f32), p.on_curve));
            }
            (mid, ord)
        };

        out.commands.push(PathCommand::MoveTo(start_xy));
        let mut prev_off: Option<Point> = None;
        for &(xy, on) in ordered.iter().skip(1) {
            if on {
                if let Some(c) = prev_off.take() {
                    out.commands.push(PathCommand::QuadCurveTo {
                        control: c,
                        end: xy,
                    });
                } else {
                    out.commands.push(PathCommand::LineTo(xy));
                }
            } else if let Some(c) = prev_off {
                // Two off-curve points in a row → emit a quadratic to
                // their midpoint, then keep the new off-curve as the
                // next control.
                let mid = Point::new((c.x + xy.x) * 0.5, (c.y + xy.y) * 0.5);
                out.commands.push(PathCommand::QuadCurveTo {
                    control: c,
                    end: mid,
                });
                prev_off = Some(xy);
            } else {
                prev_off = Some(xy);
            }
        }
        // Trailing off-curve curves back to the start.
        if let Some(c) = prev_off.take() {
            out.commands.push(PathCommand::QuadCurveTo {
                control: c,
                end: start_xy,
            });
        }
        out.commands.push(PathCommand::Close);
    }
    out
}

/// Convert a CFF cubic outline (Type 2 charstring decode) to a [`Path`]
/// of MoveTo / LineTo / CubicCurveTo / Close. The CFF segment IR is
/// already explicit, so this is a 1:1 mapping — no on/off-curve dance.
fn cff_outline_to_path(outline: &oxideav_otf::CubicOutline) -> Path {
    let mut out = Path::new();
    for contour in &outline.contours {
        for seg in &contour.segments {
            match *seg {
                oxideav_otf::CubicSegment::MoveTo(p) => {
                    out.commands.push(PathCommand::MoveTo(Point::new(p.x, p.y)));
                }
                oxideav_otf::CubicSegment::LineTo(p) => {
                    out.commands.push(PathCommand::LineTo(Point::new(p.x, p.y)));
                }
                oxideav_otf::CubicSegment::CurveTo { c1, c2, end } => {
                    out.commands.push(PathCommand::CubicCurveTo {
                        c1: Point::new(c1.x, c1.y),
                        c2: Point::new(c2.x, c2.y),
                        end: Point::new(end.x, end.y),
                    });
                }
                oxideav_otf::CubicSegment::ClosePath => {
                    out.commands.push(PathCommand::Close);
                }
            }
        }
    }
    out
}
