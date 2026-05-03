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
}
