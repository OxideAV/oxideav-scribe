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
use oxideav_ttf::{NamedInstance, VariationAxis};

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
    /// Current variation coordinates (one entry per `fvar` axis, in
    /// declaration order, in user-space units). Empty for static fonts
    /// or until [`Face::set_variation_coords`] is called. When non-empty
    /// and the font is variable, [`Face::with_font`] re-applies the
    /// vector to every freshly-parsed `Font<'_>` so glyph outline
    /// lookups consume the gvar-blended outline.
    var_coords: Vec<f32>,
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
            var_coords: Vec::new(),
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
            var_coords: Vec::new(),
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
            // OTF / CFF2 variation support is out of scope for the
            // initial round; any caller setting variation coords on an
            // OTF face is a no-op (with_otf_font does not reapply).
            var_coords: Vec::new(),
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
        let mut font = match self.subfont_index {
            Some(i) => {
                oxideav_ttf::Font::from_collection_bytes(&self.bytes, i).map_err(Error::from)?
            }
            None => oxideav_ttf::Font::from_bytes(&self.bytes).map_err(Error::from)?,
        };
        // Re-apply any caller-set variation coords after the parse so
        // glyph_outline (and downstream advances / kerning that consume
        // it) reflect the gvar-blended outline. No-op for static fonts
        // (the underlying setter short-circuits when there is no fvar).
        if !self.var_coords.is_empty() {
            font.set_variation_coords(&self.var_coords);
        }
        Ok(f(&font))
    }

    /// True if this face is the `i`-th subfont of a TrueType Collection
    /// (the `bytes` buffer holds the WHOLE TTC; the subfont is selected
    /// at parse-time). Returns `None` for plain sfnt-flavour faces.
    pub fn subfont_index(&self) -> Option<u32> {
        self.subfont_index
    }

    // ---- variable fonts (fvar / avar / gvar) -----------------------------

    /// `true` if the underlying font ships an `fvar` table — i.e. it
    /// exposes one or more variation axes. `false` for OTF faces and
    /// for static TTF faces.
    pub fn is_variable(&self) -> bool {
        if self.kind != FaceKind::Ttf {
            return false;
        }
        self.with_font(|f| f.is_variable()).unwrap_or(false)
    }

    /// All variation axes the font publishes, cloned out of the
    /// underlying `fvar`. Empty for static / OTF faces. Each
    /// [`VariationAxis`] carries `min` / `default` / `max` plus the
    /// `tag` (`b"wght"` / `b"wdth"` / `b"opsz"` / …) and the `name_id`
    /// for the human-readable axis label.
    pub fn variation_axes(&self) -> Vec<VariationAxis> {
        if self.kind != FaceKind::Ttf {
            return Vec::new();
        }
        self.with_font(|f| f.variation_axes().to_vec())
            .unwrap_or_default()
    }

    /// All named instances (pre-defined axis vectors like "Light",
    /// "Regular", "Bold") the font publishes, in declaration order.
    /// Empty for static / OTF faces. Each [`NamedInstance`] carries
    /// `subfamily_name_id` (a `name`-table id for the subfamily label),
    /// `coords` (one `f32` per axis matching [`Self::variation_axes`]),
    /// and an optional `post_script_name_id`.
    ///
    /// Callers that want to pick an instance by axis vector (e.g. "the
    /// instance whose `wght=900`") iterate this slice and inspect
    /// `coords`. Resolving the human-readable subfamily label requires
    /// reading the `name` table directly via [`Self::with_font`] —
    /// scribe deliberately doesn't surface a bespoke
    /// `name_id → string` accessor.
    pub fn named_instances(&self) -> Vec<NamedInstance> {
        if self.kind != FaceKind::Ttf {
            return Vec::new();
        }
        self.with_font(|f| f.named_instances().to_vec())
            .unwrap_or_default()
    }

    /// Current user-space variation coordinates (one entry per axis,
    /// in `fvar` declaration order). Empty before any
    /// [`Self::set_variation_coords`] call AND for static / OTF faces.
    pub fn variation_coords(&self) -> &[f32] {
        &self.var_coords
    }

    /// Set the user-space variation coordinates that scribe will
    /// re-apply on every [`Self::with_font`] re-parse, so subsequent
    /// glyph outline lookups consume the gvar-blended outline at those
    /// coords. Each entry is in **user-space** units (e.g. `wght` is
    /// 100..900 on Inter).
    ///
    /// The vector is silently length-normalised against the axis count
    /// — shorter vectors leave the trailing axes at their previous
    /// value (or each axis's default for a fresh face), longer vectors
    /// are truncated. Out-of-range values are clamped to each axis's
    /// `[min, max]` *via the underlying parser*, so the value visible
    /// on a subsequent [`Self::variation_coords`] call may differ from
    /// what was passed in. No-op for static / OTF faces.
    ///
    /// Pre-condition: this method works for [`FaceKind::Ttf`] faces
    /// only. Calling it on an OTF face returns `Err(WrongFaceKind)`
    /// (variable CFF2 / OTF is out of scope for the initial round).
    pub fn set_variation_coords(&mut self, coords: &[f32]) -> Result<(), Error> {
        if self.kind != FaceKind::Ttf {
            return Err(Error::WrongFaceKind {
                expected: FaceKind::Ttf,
                actual: self.kind,
            });
        }
        // Round-trip through a freshly-parsed parser so the per-axis
        // length cap + `[min, max]` clamp the underlying setter applies
        // is preserved on round-trip. The freshly-parsed `Font` is
        // discarded after the round-trip — we only persist the clamped
        // f32 vector so subsequent `with_font` re-applies it.
        let mut font = match self.subfont_index {
            Some(i) => {
                oxideav_ttf::Font::from_collection_bytes(&self.bytes, i).map_err(Error::from)?
            }
            None => oxideav_ttf::Font::from_bytes(&self.bytes).map_err(Error::from)?,
        };
        // Seed with whatever the parser exposes as the current vector
        // (axis defaults on a fresh face; the previously-set vector if
        // we re-set with a partial `coords` argument). Then merge the
        // caller-supplied entries on top, then call the parser to
        // clamp + length-cap.
        let mut working = font.variation_coords().to_vec();
        if !self.var_coords.is_empty() {
            for (i, &v) in self.var_coords.iter().enumerate() {
                if i >= working.len() {
                    break;
                }
                working[i] = v;
            }
        }
        for (i, &v) in coords.iter().enumerate() {
            if i >= working.len() {
                break;
            }
            working[i] = v;
        }
        font.set_variation_coords(&working);
        self.var_coords = font.variation_coords().to_vec();
        Ok(())
    }

    /// Reset the variation coordinates to the empty vector — i.e.
    /// subsequent `with_font` re-parses fall back to each axis's
    /// `default` value (the static-font baseline). No-op when no
    /// coords were ever set.
    pub fn clear_variation_coords(&mut self) {
        self.var_coords.clear();
    }

    /// Borrow the raw font bytes the face was constructed from. Used
    /// by [`crate::variations`] to walk tables that the underlying
    /// `oxideav-ttf` / `oxideav-otf` parsers don't surface yet
    /// (`MVAR` / `HVAR` / `VVAR` / `STAT` / `CFF2`). The returned
    /// slice is the WHOLE file (including the sfnt header) so the
    /// caller can call `variations::find_table` against it.
    pub fn raw_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Resolve a `name` table id to its highest-ranked Unicode string
    /// (Windows English first, Mac Roman English second, anything
    /// Unicode-y after, then any remaining record). Used by callers
    /// that consumed an axis or instance name id from
    /// [`Face::variation_axes`] / [`Face::named_instances`] /
    /// [`Face::stat_axes`] / [`Face::stat_axis_values`] and need the
    /// human-readable label.
    ///
    /// Returns `None` if the font has no `name` table for the id, or
    /// (for OTF faces) the name table couldn't be located in the raw
    /// font bytes. Owned `String` rather than `&str` because the
    /// underlying snapshot decodes UTF-16-BE on the fly.
    pub fn name_id(&self, name_id: u16) -> Option<String> {
        let name_bytes = match self.kind {
            FaceKind::Ttf => {
                crate::variations::find_table(&self.bytes, b"name", self.subfont_offset())?
            }
            FaceKind::Otf => crate::variations::find_table(&self.bytes, b"name", 0)?,
        };
        let snap = crate::variations::NameTableSnapshot::parse(name_bytes)?;
        snap.find(name_id).map(|s| s.to_string())
    }

    /// Compute the table-relative offset of the sfnt header for this
    /// face. Plain sfnt fonts return `0`; TTC subfonts walk the
    /// collection header for the per-subfont offset.
    fn subfont_offset(&self) -> usize {
        let i = match self.subfont_index {
            Some(i) => i,
            None => return 0,
        };
        // The TTC header layout is: 'ttcf'(4) + version(4) + numFonts(4) + u32 offsets.
        if self.bytes.len() < 12 {
            return 0;
        }
        if &self.bytes[0..4] != b"ttcf" {
            return 0;
        }
        let off = 12 + (i as usize) * 4;
        if off + 4 > self.bytes.len() {
            return 0;
        }
        u32::from_be_bytes([
            self.bytes[off],
            self.bytes[off + 1],
            self.bytes[off + 2],
            self.bytes[off + 3],
        ]) as usize
    }

    /// Parse and return the `MVAR` (Metrics Variations) table, if
    /// present. Returns `None` for static fonts and for fonts that
    /// don't ship MVAR (most static fonts and many simpler variable
    /// fonts).
    pub fn mvar(&self) -> Option<crate::variations::MvarTable> {
        let bytes = crate::variations::find_table(&self.bytes, b"MVAR", self.subfont_offset())?;
        crate::variations::MvarTable::parse(bytes)
    }

    /// Parse and return the `HVAR` (Horizontal Metrics Variations)
    /// table, if present. Returns `None` for fonts that don't ship
    /// HVAR (every static font and many variable fonts whose advance
    /// widths don't change with the variation coords).
    pub fn hvar(&self) -> Option<crate::variations::AdvanceVariationTable> {
        let bytes = crate::variations::find_table(&self.bytes, b"HVAR", self.subfont_offset())?;
        crate::variations::AdvanceVariationTable::parse(bytes)
    }

    /// Parse and return the `VVAR` (Vertical Metrics Variations)
    /// table, if present. Returns `None` for horizontal-only fonts
    /// (the common case — Inter Variable, Roboto Flex, almost every
    /// Latin variable font).
    pub fn vvar(&self) -> Option<crate::variations::AdvanceVariationTable> {
        let bytes = crate::variations::find_table(&self.bytes, b"VVAR", self.subfont_offset())?;
        crate::variations::AdvanceVariationTable::parse(bytes)
    }

    /// Parse and return the `STAT` (Style Attributes) table, if
    /// present. Returns `None` for fonts that don't ship STAT (every
    /// static font and a handful of older variable fonts).
    pub fn stat(&self) -> Option<crate::variations::StatTable> {
        let bytes = crate::variations::find_table(&self.bytes, b"STAT", self.subfont_offset())?;
        crate::variations::StatTable::parse(bytes)
    }

    /// Parse and return the `CFF2` table, if present. Returns `None`
    /// for TT-flavoured faces and for OTF faces that don't ship CFF2
    /// (every static OTF — they all carry plain `CFF ` instead).
    pub fn cff2(&self) -> Option<crate::variations::Cff2Table> {
        let bytes = crate::variations::find_table(&self.bytes, b"CFF2", 0)?;
        crate::variations::Cff2Table::parse(bytes)
    }

    /// Apply MVAR + the current variation coords (set via
    /// [`Self::set_variation_coords`]) to compute a metric delta in
    /// font units for `tag`. Returns `0.0` when the font has no MVAR,
    /// the tag isn't enumerated, or the coords are at the
    /// per-axis defaults.
    ///
    /// Common tags: `b"hasc"` (horizontal ascender), `b"hdsc"`
    /// (horizontal descender), `b"hcla"` (typo line gap), `b"xhgt"`
    /// (x-height), `b"cpht"` (cap height), `b"undo"` (underline
    /// offset), `b"unds"` (underline size). The full list is in
    /// the OpenType spec §"MVAR — Metrics Variations Table".
    pub fn metric_delta(&self, tag: &[u8; 4]) -> f32 {
        let mvar = match self.mvar() {
            Some(m) => m,
            None => return 0.0,
        };
        let coords = self.normalised_coords();
        mvar.delta(tag, &coords)
    }

    /// Apply HVAR + the current variation coords to compute a
    /// horizontal-advance delta in font units for `gid`. Returns
    /// `0.0` when the font has no HVAR or the coords are at the
    /// per-axis defaults.
    pub fn h_advance_delta(&self, gid: u16) -> f32 {
        let hvar = match self.hvar() {
            Some(h) => h,
            None => return 0.0,
        };
        let coords = self.normalised_coords();
        hvar.advance_delta(gid, &coords)
    }

    /// Apply VVAR + the current variation coords to compute a
    /// vertical-advance delta in font units for `gid`. Returns
    /// `0.0` for horizontal-only fonts.
    pub fn v_advance_delta(&self, gid: u16) -> f32 {
        let vvar = match self.vvar() {
            Some(v) => v,
            None => return 0.0,
        };
        let coords = self.normalised_coords();
        vvar.advance_delta(gid, &coords)
    }

    /// Compute the normalised coord vector for the current variation
    /// coords. Empty for static / OTF faces. The returned vector has
    /// the avar piecewise-linear remap applied (when the font ships
    /// one).
    pub fn normalised_coords(&self) -> Vec<f32> {
        if self.kind != FaceKind::Ttf || self.var_coords.is_empty() {
            return Vec::new();
        }
        self.with_font(|f| f.normalised_coords())
            .unwrap_or_default()
    }

    /// All STAT design axes, in declaration order. Empty for static
    /// fonts or fonts without STAT.
    pub fn stat_axes(&self) -> Vec<crate::variations::StatAxis> {
        match self.stat() {
            Some(s) => s.axes().to_vec(),
            None => Vec::new(),
        }
    }

    /// All STAT axis-value records (one per named point / range / link
    /// / multi-axis combination), in declaration order. Empty for
    /// static fonts or fonts without STAT.
    pub fn stat_axis_values(&self) -> Vec<crate::variations::StatAxisValue> {
        match self.stat() {
            Some(s) => s.axis_values().to_vec(),
            None => Vec::new(),
        }
    }

    /// List the GSUB feature tags this face publishes under `script_tag`
    /// (and, optionally, `lang_tag`). Returns the four-byte feature
    /// identifiers in the order the active LangSys declares them — the
    /// required feature, if any, comes first. Duplicates are preserved
    /// (rare in practice, but possible when a font lists the same tag in
    /// both the default LangSys and a script-specific override that fell
    /// through to default).
    ///
    /// The companion `oxideav-ttf` API also exposes the per-feature
    /// lookup-index list; this accessor surfaces only the tag set, which
    /// is what callers picking which OpenType features to enable
    /// typically need. For the lookup-index detail, drop down via
    /// [`Self::with_font`] and call
    /// [`oxideav_ttf::Font::gsub_features_for_script`] directly.
    ///
    /// Returns `Vec::new()` when:
    /// - the face has no GSUB table (most pre-2000 fonts),
    /// - the requested script tag isn't in the ScriptList,
    /// - `lang_tag` is `Some(_)` and the LangSys isn't in the script,
    ///   and the script has no default LangSys.
    ///
    /// OTF (CFF-flavour) faces also work — `oxideav-otf` shares the
    /// same sfnt container, so the GSUB table sits at the same place.
    /// This accessor uses the TTF path; CFF-only faces fall through to
    /// the empty vec when called (the kind check inside `with_font`
    /// rejects OTF).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use oxideav_scribe::Face;
    /// # fn demo(face: Face) {
    /// // Which features does this font publish for Latin?
    /// let tags = face.gsub_features_for_script(*b"latn", None);
    /// for tag in &tags {
    ///     println!("{}", std::str::from_utf8(tag).unwrap_or("????"));
    /// }
    /// # }
    /// ```
    pub fn gsub_features_for_script(
        &self,
        script_tag: [u8; 4],
        lang_tag: Option<[u8; 4]>,
    ) -> Vec<[u8; 4]> {
        self.with_font(|font| {
            font.gsub_features_for_script(script_tag, lang_tag)
                .into_iter()
                .map(|f| f.tag)
                .collect()
        })
        .unwrap_or_default()
    }

    /// `true` when this face publishes `feature_tag` for `script_tag`
    /// (under the default LangSys). Convenience predicate over
    /// [`Self::gsub_features_for_script`] for callers that just need to
    /// gate behaviour on feature presence without iterating the full
    /// list.
    ///
    /// Cheap for the typical hit-or-miss check — internally this re-runs
    /// the script-walk every call (the same cost as
    /// `gsub_features_for_script(...).iter().any(...)`); cache the full
    /// tag vec if you query many tags against the same script.
    pub fn has_gsub_feature(&self, script_tag: [u8; 4], feature_tag: [u8; 4]) -> bool {
        self.gsub_features_for_script(script_tag, None)
            .contains(&feature_tag)
    }

    /// Shape `text` with the caller-specified GSUB feature tags applied
    /// to the cmap'd glyph run. Returns the post-substitution glyph IDs.
    ///
    /// **Round-89/125/128/156 scope: GSUB LookupType 1 (Single
    /// Substitution), LookupType 2 (Multiple Substitution, Format 1),
    /// LookupType 3 (Alternate Substitution, Format 1; default
    /// `alternateIndex = 0`), and LookupType 4 (Ligature Substitution,
    /// Format 1).** Type 1 Format 1 (delta) and Format 2
    /// (substitute-array), Type 2 Format 1, Type 3 Format 1, and Type
    /// 4 Format 1 are all dispatched through `oxideav-ttf`'s
    /// `gsub_apply_lookup_type_{1,2,3,4}` accessors; ExtensionSubst
    /// LookupType-7 wrappers around any of those lookups are
    /// unwrapped transparently. A Type-2 lookup may change the glyph
    /// count (split one glyph into N, or delete one with
    /// `glyphCount = 0`); a Type-3 lookup is length-preserving (one
    /// alternate per covered slot); a Type-4 lookup *always* shortens
    /// the run (N component glyphs → 1 ligature). The returned `Vec`
    /// reflects the post-substitution length. Lookups of other types
    /// (Contextual, ChainContext, ReverseChainContext) referenced by
    /// the requested features are silently skipped — see
    /// [`crate::shaper::Shaper::shape`] for the full multi-type
    /// pipeline.
    ///
    /// Typical feature tags this is useful for are the display-toggled
    /// features that the always-on round-15 `ccmp` + `calt` passes
    /// don't reach:
    /// - `smcp` / `c2sc` — small caps (from lower / from upper).
    /// - `case` — case-sensitive forms.
    /// - `frac` — fractions (Type-1 component only; the contextual
    ///   `1/2` collapse is a Type-6 rule and skipped here).
    /// - `salt` — stylistic alternates.
    /// - `aalt` — access all alternates (Type-1 + Type-3 mix; the
    ///   Type-3 component returns `alternateIndex = 0` per the
    ///   round-156 default).
    /// - `ss01..ss20` — stylistic sets.
    /// - `sups` / `subs` / `numr` / `dnom` / `ordn` — vertical /
    ///   role-based number forms.
    /// - `cv01..cv99` — per-character variants.
    /// - `zero` — slashed zero.
    /// - `pnum` / `tnum` — proportional / tabular numerals.
    /// - `liga` / `dlig` / `rlig` — standard / discretionary /
    ///   required ligatures (Type-4 ligature substitution as of
    ///   round 128).
    ///
    /// Features are applied in caller order. Each lookup's coverage
    /// table determines per-glyph whether it fires. The script-tag
    /// probe order keeps `latn` → `cyrl` → `grek` → `DFLT` at the
    /// head for backwards-compatibility with the round-89 surface,
    /// then falls through to `arab` / `hebr` / `thai` / `lao ` /
    /// Indic v1+v2 (`deva` / `dev2` / `beng` / `bng2` / ... ) / `khmr`
    /// / `mymr` / `mym2` / CJK (`hang` / `hani` / `kana`) (round 175)
    /// so that non-Latin runs reach GSUB through this caller-driven
    /// surface. The first script whose feature list publishes the
    /// requested tag wins for that feature. Callers that already
    /// know the run's script should drop to
    /// [`Self::shape_text_with_script`] (round 175) to bypass the
    /// probe walk and resolve against one explicit script tag.
    ///
    /// Returns an empty vec for OTF (CFF) faces — GSUB substitution
    /// requires the TTF parser surface; OTF callers must drop to
    /// [`Self::with_otf_font`] (the CFF flavour shares the same GSUB
    /// table layout in principle but scribe doesn't yet route it).
    ///
    /// Empty `text` always returns an empty `Vec`. Empty `features`
    /// returns the pure-cmap output (useful as a baseline).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use oxideav_scribe::Face;
    /// # fn demo(face: Face) {
    /// // Small-caps + case-sensitive punctuation on "Hello".
    /// let gids = face.shape_text("Hello", &[*b"smcp", *b"case"]);
    /// // GIDs are now the small-cap variants where coverage matched.
    /// # let _ = gids;
    /// # }
    /// ```
    pub fn shape_text(&self, text: &str, features: &[[u8; 4]]) -> Vec<u16> {
        self.with_font(|font| crate::shaping::shape_text_with_font(font, text, features))
            .unwrap_or_default()
    }

    /// Shape `text` with the caller-specified GSUB feature tags
    /// applied under the explicit `script_tag`. Returns the post-
    /// substitution glyph IDs.
    ///
    /// Differs from [`Self::shape_text`] in that the script-tag
    /// priority walk is bypassed — every requested feature is
    /// resolved against `script_tag` alone (with the script's
    /// DefaultLangSys). Useful when the caller already knows the
    /// script of the run, e.g.:
    ///
    /// - An Arabic shaper resolving `liga` / `dlig` against `arab`
    ///   rather than letting the auto-probe pick `latn` first when
    ///   the font publishes both.
    /// - A CJK pipeline resolving `vert` / `vrt2` against `hani` /
    ///   `kana` / `hang` to switch a horizontal-form run to the
    ///   vertical-form glyphs.
    /// - An Indic shaper resolving an OT 1.6 v2-tag feature
    ///   (`tml2`, `dev2`, etc.) explicitly when the font ships both
    ///   v1 and v2 script lookups.
    ///
    /// An unknown `script_tag` (or one not present in the font's
    /// ScriptList) yields the pure-cmap output — every requested
    /// feature resolves to an empty lookup list.
    ///
    /// Mirrors [`Self::shape_text`]'s LookupType-1/2/3/4 dispatch
    /// semantics — single / multiple / alternate / ligature
    /// substitution are all wired; contextual / chained-contextual /
    /// reverse-chained lookups referenced by the requested features
    /// are silently skipped (use [`crate::shaper::Shaper::shape`] for
    /// the full multi-type pipeline).
    ///
    /// Returns an empty vec for OTF (CFF) faces — same constraint as
    /// [`Self::shape_text`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use oxideav_scribe::Face;
    /// # fn demo(face: Face) {
    /// // Force-resolve `liga` against the Arabic ScriptList rather
    /// // than letting the auto-probe pick a Latin-side lookup.
    /// let gids = face.shape_text_with_script("لا", *b"arab", &[*b"liga"]);
    /// # let _ = gids;
    /// # }
    /// ```
    pub fn shape_text_with_script(
        &self,
        text: &str,
        script_tag: [u8; 4],
        features: &[[u8; 4]],
    ) -> Vec<u16> {
        self.with_font(|font| {
            crate::shaping::shape_text_with_script_with_font(font, text, script_tag, features)
        })
        .unwrap_or_default()
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
        //
        // Round 6 (#356): use `raster_color_glyph_at` so the bitmap is
        // bilinearly resampled to `size_px` at decode-time. The
        // resulting `Node::Image` carries a bitmap whose dimensions
        // match `bounds.width / .height` 1:1 — downstream rasterizers
        // can blit it without a separate scale step. (Pre-resampling at
        // the decode boundary is also where the cache-key story lives:
        // a 32 px bitmap derived from the 109 px strike is the same
        // every time and can be memoised by callers via the wrapping
        // `Group::cache_key` from `Shaper::shape_to_paths`.)
        if matches!(self.kind, FaceKind::Ttf) && self.has_color_bitmaps() {
            if let Ok(Some(cgb)) = self.raster_color_glyph_at(glyph_id, size_px) {
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
                    // bearing_x / bearing_y / advance from
                    // raster_color_glyph_at are already in raster pixels
                    // at `size_px` (pre-scaled by `size_px / strike_ppem`).
                    let bx = cgb.bearing_x as f32;
                    let by = -(cgb.bearing_y as f32);
                    let bw = w as f32;
                    let bh = h as f32;
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
