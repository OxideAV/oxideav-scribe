//! `Face` — owning wrapper around `oxideav_ttf::Font` plus per-face
//! identity for the glyph-bitmap cache.
//!
//! `Font<'a>` borrows from the input bytes which makes it awkward to
//! pass around in a higher-level renderer. `Face` owns the bytes via a
//! boxed slice and re-parses on demand through [`Face::with_font`]. We
//! deliberately avoid `Pin` / self-referential structs (no third-party
//! deps allowed); the cost of a one-line re-parse on each call is
//! ~microseconds and dwarfed by glyph rasterisation.

use crate::Error;

/// Monotonic global id generator for `Face` instances. Used as the
/// primary key when caching rasterised glyph bitmaps so that two
/// faces that happen to share family names don't collide.
fn next_face_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// An owning, re-parseable wrapper around `oxideav_ttf::Font`.
#[derive(Debug)]
pub struct Face {
    bytes: Box<[u8]>,
    id: u64,
    units_per_em: u16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    family: Option<String>,
    italic_angle: f32,
    weight_class: u16,
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
            units_per_em,
            ascent,
            descent,
            line_gap,
            family,
            italic_angle,
            weight_class,
        })
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

    /// Run a closure with a freshly-parsed `Font<'_>` view of the
    /// owned bytes. We re-parse on each call instead of storing a
    /// self-referential `Font<'static>` (which would require unsafe or
    /// a third-party crate like `ouroboros`, both of which we avoid).
    /// Re-parsing is read-only header walking — well under a
    /// millisecond on any modern font.
    pub fn with_font<R>(&self, f: impl FnOnce(&oxideav_ttf::Font<'_>) -> R) -> Result<R, Error> {
        let font = oxideav_ttf::Font::from_bytes(&self.bytes).map_err(Error::from)?;
        Ok(f(&font))
    }
}
