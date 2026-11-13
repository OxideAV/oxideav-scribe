//! `Face` — owning wrapper around `oxideav_ttf::Font` /
//! `oxideav_otf::Font` plus per-face identity for the glyph-bitmap
//! cache.
//!
//! `Font<'a>` (in either underlying crate) borrows from the input
//! bytes which makes it awkward to pass around in a higher-level
//! renderer. `Face` owns the bytes **and the view parsed from them**:
//! the font is parsed exactly once, when the `Face` is constructed
//! (and once more each time a variation instance is installed), and
//! every per-glyph entry point — outlines, colour bitmaps, metrics,
//! shaping — borrows that one view through [`Face::with_font`] /
//! [`Face::with_otf_font`]. Nothing is re-parsed per call. The
//! scribe-side table snapshots (`post`, `MVAR`, `HVAR`, `VVAR`,
//! `STAT`, `name`) are likewise parsed on first use and kept for the
//! life of the face.
//!
//! A `Face` is `Send + Sync`; all of its `&self` entry points may be
//! called from several threads at once.
//!
//! TTF and OTF cohabit through a [`FaceKind`] tag. The TTF path
//! returns quadratic-Bezier outlines (`oxideav_ttf::TtOutline`); the
//! OTF path returns cubic-Bezier outlines (`oxideav_otf::CubicOutline`).
//! Higher-level rasterisation code can dispatch via
//! [`Face::flatten_outline`] which converts whichever native form
//! the face holds into the unified `FlatOutline` polyline.

use crate::Error;
use crate::outline_cache::{InstanceKey, OutlineCache};
use crate::parsed::ParsedFont;
use crate::shaping::plan::LayoutPlan;
use oxideav_core::{
    FillRule, ImageRef, Node, Paint, Path, PathCommand, PathNode, Point, Rect, Rgba, Transform2D,
    VideoFrame, VideoPlane,
};
use oxideav_ttf::{NamedInstance, VariationAxis};
use std::sync::{Arc, OnceLock};

/// Which underlying parser this face wraps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FaceKind {
    /// TrueType / quadratic-Bezier outlines (`oxideav-ttf`).
    Ttf,
    /// OpenType-CFF / cubic-Bezier outlines (`oxideav-otf`).
    Otf,
}

/// Which `BASE`-table layout axis a baseline coordinate is queried on
/// (ISO/IEC 14496-22:2019 §6.3.1). A `HorizAxis` baseline is a **Y**
/// coordinate used when laying text out horizontally; a `VertAxis`
/// baseline is an **X** coordinate used for vertical writing modes. A
/// font may ship one axis, both, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BaselineAxis {
    /// `HorizAxis` — baseline values are Y coordinates (horizontal text).
    Horizontal,
    /// `VertAxis` — baseline values are X coordinates (vertical text).
    Vertical,
}

/// Monotonic global id generator for `Face` instances. Used as the
/// primary key when caching rasterised glyph bitmaps so that two
/// faces that happen to share family names don't collide.
fn next_face_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Scribe-side table snapshots, each parsed on first use and kept for
/// the life of the face (the tables do not depend on the variation
/// instance).
#[derive(Default)]
struct LazyTables {
    post: OnceLock<Option<crate::post::PostTable>>,
    mvar: OnceLock<Option<crate::variations::MvarTable>>,
    hvar: OnceLock<Option<crate::variations::AdvanceVariationTable>>,
    vvar: OnceLock<Option<crate::variations::AdvanceVariationTable>>,
    stat: OnceLock<Option<crate::variations::StatTable>>,
    names: OnceLock<Option<crate::variations::NameTableSnapshot>>,
}

impl std::fmt::Debug for LazyTables {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyTables")
            .field("post", &self.post.get().is_some())
            .field("mvar", &self.mvar.get().is_some())
            .field("hvar", &self.hvar.get().is_some())
            .field("vvar", &self.vvar.get().is_some())
            .field("stat", &self.stat.get().is_some())
            .field("names", &self.names.get().is_some())
            .finish()
    }
}

/// An owning wrapper around either an `oxideav_ttf::Font` or an
/// `oxideav_otf::Font`, parsed once at construction. The discriminant
/// is recorded in [`Face::kind`] so callers can pick the right
/// outline path.
#[derive(Debug)]
pub struct Face {
    /// The font bytes and the view parsed from them.
    font: ParsedFont,
    id: u64,
    /// Content-derived identity, computed once (see [`Face::stable_id`]).
    stable_id: u64,
    kind: FaceKind,
    units_per_em: u16,
    ascent: i16,
    descent: i16,
    line_gap: i16,
    family: Option<String>,
    italic_angle: f32,
    weight_class: u16,
    /// `Some(i)` when this face was constructed from a TTC subfont via
    /// `from_ttc_bytes`. `None` for plain sfnt-flavour faces (the
    /// common case).
    subfont_index: Option<u32>,
    /// Byte offset of this face's sfnt header inside the file (`0`
    /// unless the face is a collection member).
    subfont_offset: usize,
    /// Current variation coordinates (one entry per `fvar` axis, in
    /// declaration order, in user-space units). Empty for static fonts
    /// or until [`Face::set_variation_coords`] is called. The parsed
    /// view always has exactly this instance installed.
    var_coords: Vec<f32>,
    /// Normalised (avar-mapped) form of `var_coords`; empty whenever
    /// `var_coords` is.
    norm_coords: Vec<f32>,
    /// The per-axis default coordinates (what a freshly parsed view
    /// reports); empty for static and CFF faces.
    default_coords: Vec<f32>,
    /// `var_coords` as an outline-cache key component.
    instance: InstanceKey,
    tables: LazyTables,
    /// Lookup-list / feature-list facts shaping asks for on every
    /// call, filled on first use (they do not depend on the instance).
    plan: LayoutPlan,
    /// Shared outline cache, when the caller attached one.
    outline_cache: Option<Arc<OutlineCache>>,
}

/// Content-derived identity: a digest of the font's leading bytes (up
/// to 256) plus the byte length, the flavour and the collection index.
fn compute_stable_id(bytes: &[u8], kind: FaceKind, subfont_index: Option<u32>) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    // Tag the discriminant + subfont so a TTC's subfont 0 and
    // subfont 1 (which share the outer byte buffer) end up with
    // different ids without us having to hash the entire TTC twice.
    (kind as u8).hash(&mut h);
    subfont_index.hash(&mut h);
    // Include the total byte length so two fonts that share a
    // common header prefix (rare but possible across stripped
    // variants of the same family) still distinguish.
    (bytes.len() as u64).hash(&mut h);
    // Hash the leading bytes — the sfnt header + the table
    // directory both live here and are highly font-specific.
    let prefix = &bytes[..bytes.len().min(256)];
    prefix.hash(&mut h);
    h.finish()
}

/// Byte offset of collection member `index`'s sfnt header, or `0` when
/// the buffer is not a collection or the index is out of range.
fn collection_member_offset(bytes: &[u8], index: Option<u32>) -> usize {
    let Some(i) = index else {
        return 0;
    };
    // The TTC header layout is: 'ttcf'(4) + version(4) + numFonts(4) + u32 offsets.
    if bytes.len() < 12 || &bytes[0..4] != b"ttcf" {
        return 0;
    }
    let off = 12 + (i as usize) * 4;
    match bytes.get(off..off + 4) {
        Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize,
        None => 0,
    }
}

impl Face {
    /// Parse a TTF from owned bytes.
    pub fn from_ttf_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        Self::new_ttf(bytes, None)
    }

    /// Parse the `index`-th subfont out of an owned TrueType Collection
    /// (`.ttc` / `'ttcf'`) byte buffer. Convenience wrapper around
    /// `oxideav_ttf::Font::from_collection_bytes`.
    pub fn from_ttc_bytes(bytes: Vec<u8>, index: u32) -> Result<Self, Error> {
        Self::new_ttf(bytes, Some(index))
    }

    fn new_ttf(bytes: Vec<u8>, subfont_index: Option<u32>) -> Result<Self, Error> {
        let font = ParsedFont::new_ttf(bytes, subfont_index)?;
        // Snapshot the metadata from the parsed view.
        let (
            units_per_em,
            ascent,
            descent,
            line_gap,
            family,
            italic_angle,
            weight_class,
            default_coords,
        ) = {
            let f = font.ttf().expect("constructed as TrueType");
            (
                f.units_per_em(),
                f.ascent(),
                f.descent(),
                f.line_gap(),
                f.family_name().map(|s| s.to_string()),
                f.italic_angle(),
                f.weight_class(),
                f.variation_coords().to_vec(),
            )
        };
        Ok(Self {
            stable_id: compute_stable_id(font.bytes(), FaceKind::Ttf, subfont_index),
            subfont_offset: collection_member_offset(font.bytes(), subfont_index),
            font,
            id: next_face_id(),
            kind: FaceKind::Ttf,
            units_per_em,
            ascent,
            descent,
            line_gap,
            family,
            italic_angle,
            weight_class,
            subfont_index,
            var_coords: Vec::new(),
            norm_coords: Vec::new(),
            default_coords,
            instance: InstanceKey::default(),
            tables: LazyTables::default(),
            plan: LayoutPlan::new(),
            outline_cache: None,
        })
    }

    /// Parse an OTF (OpenType-CFF) font from owned bytes. Returns
    /// a `Face` whose [`Face::kind`] is [`FaceKind::Otf`] and whose
    /// outlines come back as cubic Beziers via the cubic flattener
    /// in [`crate::outline`].
    pub fn from_otf_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        let font = ParsedFont::new_otf(bytes)?;
        let (units_per_em, ascent, descent, line_gap, family) = {
            let f = font.otf().expect("constructed as CFF");
            (
                f.units_per_em(),
                f.ascent(),
                f.descent(),
                f.line_gap(),
                f.family_name().map(|s| s.to_string()),
            )
        };
        Ok(Self {
            stable_id: compute_stable_id(font.bytes(), FaceKind::Otf, None),
            subfont_offset: 0,
            font,
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
            // OTF face gets `WrongFaceKind`.
            var_coords: Vec::new(),
            norm_coords: Vec::new(),
            default_coords: Vec::new(),
            instance: InstanceKey::default(),
            tables: LazyTables::default(),
            plan: LayoutPlan::new(),
            outline_cache: None,
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
    /// not. Computed once at construction; this accessor is a field
    /// read.
    pub fn stable_id(&self) -> u64 {
        self.stable_id
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

    /// Run a closure with the face's parsed `oxideav_ttf::Font<'_>`
    /// view. The view was built when the face was constructed (with
    /// the current variation instance installed), so this is a borrow,
    /// not a parse — calling it once per glyph is fine.
    ///
    /// Returns `Error::WrongFaceKind` if this face was constructed
    /// from OTF bytes; use [`Face::with_otf_font`] in that case.
    pub fn with_font<R>(&self, f: impl FnOnce(&oxideav_ttf::Font<'_>) -> R) -> Result<R, Error> {
        Ok(f(self.ttf()?))
    }

    /// The face's parsed TrueType view, with the face's variation
    /// coordinates installed. A plain borrow: nothing is parsed.
    pub(crate) fn ttf(&self) -> Result<&oxideav_ttf::Font<'_>, Error> {
        self.font.ttf().ok_or(Error::WrongFaceKind {
            expected: FaceKind::Ttf,
            actual: self.kind,
        })
    }

    /// The face's kept shaping plan (only meaningful together with
    /// [`Self::ttf`]'s view).
    pub(crate) fn layout_plan(&self) -> &LayoutPlan {
        &self.plan
    }

    /// The face's parsed OpenType-CFF view. A plain borrow.
    pub(crate) fn otf(&self) -> Result<&oxideav_otf::Font<'_>, Error> {
        self.font.otf().ok_or(Error::WrongFaceKind {
            expected: FaceKind::Otf,
            actual: self.kind,
        })
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
        self.ttf().map(|f| f.is_variable()).unwrap_or(false)
    }

    /// All variation axes the font publishes, cloned out of the
    /// underlying `fvar`. Empty for static / OTF faces. Each
    /// [`VariationAxis`] carries `min` / `default` / `max` plus the
    /// `tag` (`b"wght"` / `b"wdth"` / `b"opsz"` / …) and the `name_id`
    /// for the human-readable axis label.
    pub fn variation_axes(&self) -> Vec<VariationAxis> {
        self.ttf()
            .map(|f| f.variation_axes().to_vec())
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
        self.ttf()
            .map(|f| f.named_instances().to_vec())
            .unwrap_or_default()
    }

    /// Current user-space variation coordinates (one entry per axis,
    /// in `fvar` declaration order). Empty before any
    /// [`Self::set_variation_coords`] call AND for static / OTF faces.
    pub fn variation_coords(&self) -> &[f32] {
        &self.var_coords
    }

    /// Set the user-space variation coordinates of this face, so
    /// subsequent glyph outline lookups (and shaping) consume the
    /// gvar-blended outline at those coords. Each entry is in
    /// **user-space** units (e.g. `wght` is 100..900 on Inter).
    ///
    /// The vector is silently length-normalised against the axis count
    /// — shorter vectors leave the trailing axes at their previous
    /// value (or each axis's default for a fresh face), longer vectors
    /// are truncated. Out-of-range values are clamped to each axis's
    /// `[min, max]` *via the underlying parser*, so the value visible
    /// on a subsequent [`Self::variation_coords`] call may differ from
    /// what was passed in. No-op for static / OTF faces.
    ///
    /// The instance is installed on the face's parsed view in place —
    /// nothing is re-parsed. An attached [`OutlineCache`] needs no
    /// flushing: its keys carry the instance, so outlines of the
    /// previous instance are simply not found under the new one (and
    /// are found again if the face returns to it).
    ///
    /// Pre-condition: this method works for [`FaceKind::Ttf`] faces
    /// only. Calling it on an OTF face returns `Err(WrongFaceKind)`
    /// (variable CFF2 / OTF is out of scope for the initial round).
    pub fn set_variation_coords(&mut self, coords: &[f32]) -> Result<(), Error> {
        let (installed, normalised) = self
            .font
            .edit_ttf(|font| {
                // The view's current vector is the axis defaults on a
                // fresh face, or the previously-installed instance.
                // Overlay the caller-supplied entries and let the
                // parser clamp + length-cap.
                let mut working = font.variation_coords().to_vec();
                for (slot, &v) in working.iter_mut().zip(coords.iter()) {
                    *slot = v;
                }
                font.set_variation_coords(&working);
                let installed = font.variation_coords().to_vec();
                let normalised = if installed.is_empty() {
                    Vec::new()
                } else {
                    font.normalised_coords()
                };
                (installed, normalised)
            })
            .ok_or(Error::WrongFaceKind {
                expected: FaceKind::Ttf,
                actual: self.kind,
            })?;
        self.instance = InstanceKey::from_coords(&installed);
        self.var_coords = installed;
        self.norm_coords = normalised;
        Ok(())
    }

    /// Reset the variation coordinates to the empty vector — i.e. the
    /// face goes back to each axis's `default` value (the static-font
    /// baseline). No-op when no coords were ever set.
    pub fn clear_variation_coords(&mut self) {
        if self.var_coords.is_empty() {
            return;
        }
        let defaults = &self.default_coords;
        self.font
            .edit_ttf(|font| font.set_variation_coords(defaults));
        self.var_coords.clear();
        self.norm_coords.clear();
        self.instance = InstanceKey::default();
    }

    /// Borrow the raw font bytes the face was constructed from. Used
    /// by [`crate::variations`] to walk tables that the underlying
    /// `oxideav-ttf` / `oxideav-otf` parsers don't surface yet
    /// (`MVAR` / `HVAR` / `VVAR` / `STAT` / `CFF2`). The returned
    /// slice is the WHOLE file (including the sfnt header) so the
    /// caller can call `variations::find_table` against it.
    pub fn raw_bytes(&self) -> &[u8] {
        self.font.bytes()
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
        let snap = self
            .tables
            .names
            .get_or_init(|| crate::variations::NameTableSnapshot::parse(self.table(b"name")?))
            .as_ref()?;
        snap.find(name_id).map(|s| s.to_string())
    }

    /// The raw bytes of table `tag` in this face's table directory.
    fn table(&self, tag: &[u8; 4]) -> Option<&[u8]> {
        crate::variations::find_table(self.font.bytes(), tag, self.subfont_offset)
    }

    fn mvar_ref(&self) -> Option<&crate::variations::MvarTable> {
        self.tables
            .mvar
            .get_or_init(|| crate::variations::MvarTable::parse(self.table(b"MVAR")?))
            .as_ref()
    }

    fn hvar_ref(&self) -> Option<&crate::variations::AdvanceVariationTable> {
        self.tables
            .hvar
            .get_or_init(|| crate::variations::AdvanceVariationTable::parse(self.table(b"HVAR")?))
            .as_ref()
    }

    fn vvar_ref(&self) -> Option<&crate::variations::AdvanceVariationTable> {
        self.tables
            .vvar
            .get_or_init(|| crate::variations::AdvanceVariationTable::parse(self.table(b"VVAR")?))
            .as_ref()
    }

    fn stat_ref(&self) -> Option<&crate::variations::StatTable> {
        self.tables
            .stat
            .get_or_init(|| crate::variations::StatTable::parse(self.table(b"STAT")?))
            .as_ref()
    }

    fn post_ref(&self) -> Option<&crate::post::PostTable> {
        self.tables
            .post
            .get_or_init(|| crate::post::PostTable::parse(self.table(b"post")?))
            .as_ref()
    }

    /// Return the `MVAR` (Metrics Variations) table, if present.
    /// Returns `None` for static fonts and for fonts that don't ship
    /// MVAR (most static fonts and many simpler variable fonts).
    ///
    /// The table is parsed on first use and kept; this accessor hands
    /// back a copy. [`Self::metric_delta`] reads the kept table
    /// directly.
    pub fn mvar(&self) -> Option<crate::variations::MvarTable> {
        self.mvar_ref().cloned()
    }

    /// Return the `HVAR` (Horizontal Metrics Variations) table, if
    /// present. Returns `None` for fonts that don't ship HVAR (every
    /// static font and many variable fonts whose advance widths don't
    /// change with the variation coords). Parsed on first use and
    /// kept; this accessor hands back a copy.
    pub fn hvar(&self) -> Option<crate::variations::AdvanceVariationTable> {
        self.hvar_ref().cloned()
    }

    /// Return the `VVAR` (Vertical Metrics Variations) table, if
    /// present. Returns `None` for horizontal-only fonts (the common
    /// case — Inter Variable, Roboto Flex, almost every Latin variable
    /// font). Parsed on first use and kept; this accessor hands back a
    /// copy.
    pub fn vvar(&self) -> Option<crate::variations::AdvanceVariationTable> {
        self.vvar_ref().cloned()
    }

    /// Return the `STAT` (Style Attributes) table, if present. Returns
    /// `None` for fonts that don't ship STAT (every static font and a
    /// handful of older variable fonts). Parsed on first use and kept;
    /// this accessor hands back a copy.
    pub fn stat(&self) -> Option<crate::variations::StatTable> {
        self.stat_ref().cloned()
    }

    /// Parse and return the `CFF2` table, if present. Returns `None`
    /// for TT-flavoured faces and for OTF faces that don't ship CFF2
    /// (every static OTF — they all carry plain `CFF ` instead).
    pub fn cff2(&self) -> Option<crate::variations::Cff2Table> {
        let bytes = crate::variations::find_table(self.font.bytes(), b"CFF2", 0)?;
        crate::variations::Cff2Table::parse(bytes)
    }

    /// Return the `post` (PostScript) table, if present. Returns
    /// `None` for fonts that ship no `post` table, or whose `post`
    /// table is too short to parse. A format-3.0 `post` parses
    /// successfully but supplies no glyph names — see
    /// [`crate::post::PostTable::has_names`].
    ///
    /// The table is parsed on first use and kept; this accessor hands
    /// back a copy (every glyph name), so prefer [`Self::glyph_name`]
    /// for single lookups.
    pub fn post(&self) -> Option<crate::post::PostTable> {
        self.post_ref().cloned()
    }

    /// Resolve a glyph ID to its PostScript name via the `post` table.
    /// Returns `None` when the font has no `post` table, the table
    /// supplies no names (format 3.0), or the glyph ID is unnamed.
    /// The table is parsed once per face, on the first call.
    pub fn glyph_name(&self, gid: u16) -> Option<String> {
        self.post_ref()?.glyph_name(gid).map(|s| s.to_string())
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
        match self.mvar_ref() {
            Some(mvar) => mvar.delta(tag, &self.norm_coords),
            None => 0.0,
        }
    }

    /// Apply HVAR + the current variation coords to compute a
    /// horizontal-advance delta in font units for `gid`. Returns
    /// `0.0` when the font has no HVAR or the coords are at the
    /// per-axis defaults.
    pub fn h_advance_delta(&self, gid: u16) -> f32 {
        match self.hvar_ref() {
            Some(hvar) => hvar.advance_delta(gid, &self.norm_coords),
            None => 0.0,
        }
    }

    /// Apply VVAR + the current variation coords to compute a
    /// vertical-advance delta in font units for `gid`. Returns
    /// `0.0` for horizontal-only fonts.
    pub fn v_advance_delta(&self, gid: u16) -> f32 {
        match self.vvar_ref() {
            Some(vvar) => vvar.advance_delta(gid, &self.norm_coords),
            None => 0.0,
        }
    }

    /// Compute the normalised coord vector for the current variation
    /// coords. Empty for static / OTF faces. The returned vector has
    /// the avar piecewise-linear remap applied (when the font ships
    /// one). Computed once, when the instance is installed; this
    /// accessor copies the kept vector.
    pub fn normalised_coords(&self) -> Vec<f32> {
        self.norm_coords.clone()
    }

    /// All STAT design axes, in declaration order. Empty for static
    /// fonts or fonts without STAT.
    pub fn stat_axes(&self) -> Vec<crate::variations::StatAxis> {
        match self.stat_ref() {
            Some(s) => s.axes().to_vec(),
            None => Vec::new(),
        }
    }

    /// All STAT axis-value records (one per named point / range / link
    /// / multi-axis combination), in declaration order. Empty for
    /// static fonts or fonts without STAT.
    pub fn stat_axis_values(&self) -> Vec<crate::variations::StatAxisValue> {
        match self.stat_ref() {
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

    /// GPOS sibling of [`Self::gsub_features_for_script`]: the positioning
    /// feature tags this face publishes under `script_tag` (and, when
    /// `lang_tag` is `Some(_)`, that language system's LangSys — falling
    /// back to the script's default LangSys when the tag is absent).
    ///
    /// Callers gating positioning behaviour typically want just the tag
    /// set — e.g. is `kern` present? does the face publish `cpsp`
    /// (capital-spacing) or `mark` / `mkmk` attachment? For the
    /// per-feature lookup-index detail, drop down through
    /// [`Self::with_font`] and call
    /// [`oxideav_ttf::Font::gpos_features_for_script`] directly.
    ///
    /// Returns `Vec::new()` when the face has no GPOS table, the script
    /// tag is not in the GPOS ScriptList, or `lang_tag` is `Some(_)` with
    /// no matching LangSys and no default LangSys. As with the GSUB
    /// accessor this uses the TTF path; CFF-only OTF faces fall through to
    /// the empty vec.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use oxideav_scribe::Face;
    /// # fn demo(face: Face) {
    /// // Does this font kern Latin?
    /// if face.gpos_features_for_script(*b"latn", None).contains(b"kern") {
    ///     // enable pair kerning
    /// }
    /// # }
    /// ```
    pub fn gpos_features_for_script(
        &self,
        script_tag: [u8; 4],
        lang_tag: Option<[u8; 4]>,
    ) -> Vec<[u8; 4]> {
        self.with_font(|font| {
            font.gpos_features_for_script(script_tag, lang_tag)
                .into_iter()
                .map(|f| f.tag)
                .collect()
        })
        .unwrap_or_default()
    }

    /// `true` when this face publishes GPOS `feature_tag` for `script_tag`
    /// (under the default LangSys). GPOS mirror of
    /// [`Self::has_gsub_feature`]; convenience over
    /// [`Self::gpos_features_for_script`] for a single hit-or-miss check.
    pub fn has_gpos_feature(&self, script_tag: [u8; 4], feature_tag: [u8; 4]) -> bool {
        self.gpos_features_for_script(script_tag, None)
            .contains(&feature_tag)
    }

    /// The union of the GSUB and GPOS feature tags this face publishes
    /// under `script_tag` / `lang_tag`, de-duplicated and sorted. A
    /// convenience for higher-level APIs that present "which OpenType
    /// features can I toggle for this script?" without caring whether a
    /// given tag is realised through substitution, positioning, or both
    /// (`kern` is GPOS, `liga` is GSUB, `locl` can be either). Returns an
    /// empty `Vec` when neither table lists the script.
    pub fn layout_features_for_script(
        &self,
        script_tag: [u8; 4],
        lang_tag: Option<[u8; 4]>,
    ) -> Vec<[u8; 4]> {
        let mut tags = self.gsub_features_for_script(script_tag, lang_tag);
        tags.extend(self.gpos_features_for_script(script_tag, lang_tag));
        tags.sort_unstable();
        tags.dedup();
        tags
    }

    // ---- baseline metrics (BASE) -----------------------------------------

    /// `true` when the face ships a `BASE` table (ISO/IEC 14496-22:2019
    /// §6.3.1) — the per-script baseline registry a layout engine
    /// consults to align glyphs from different scripts (roman, hanging,
    /// ideographic, mathematical, …) on a common line. Works for both
    /// TTF and OTF/CFF faces. `false` for the many fonts that ship no
    /// `BASE` table (a client then falls back to the `OS/2` /`hhea`
    /// ascent-descent line metrics).
    pub fn has_base_table(&self) -> bool {
        match self.kind {
            FaceKind::Ttf => self.with_font(|f| f.has_base()).unwrap_or(false),
            FaceKind::Otf => self.with_otf_font(|f| f.base().is_some()).unwrap_or(false),
        }
    }

    /// The design-unit baseline coordinate for `(script_tag,
    /// baseline_tag)` on the requested [`BaselineAxis`], from the face's
    /// `BASE` table (§6.3.1.3 `BaseValues`). On the horizontal axis the
    /// value is a **Y** offset from the font's origin baseline; on the
    /// vertical axis it is an **X** offset. Common `baseline_tag`s come
    /// from the OpenType *Baseline Tags* registry — `romn` (roman /
    /// alphabetic), `ideo` (ideographic em-box bottom), `hang` (hanging),
    /// `math`, `icfb` / `icft` (ideographic character face).
    ///
    /// For a variable TTF face the coordinate is resolved at the
    /// currently-set variation instance (a `BaseCoordFormat3`
    /// VariationIndex delta is folded through the `BASE`
    /// `ItemVariationStore`); set the instance with
    /// [`Self::set_variation_coords`] first.
    ///
    /// Returns `None` when:
    /// - the face has no `BASE` table;
    /// - the requested axis is absent (a horizontal-only Latin font has
    ///   no `VertAxis`, and vice-versa);
    /// - `script_tag` is not listed in that axis's `BaseScriptList`
    ///   (§6.3.1.3: an unlisted script is laid out with the whole-font
    ///   default metrics);
    /// - the axis has no `BaseTagList`, or `baseline_tag` is not in it.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use oxideav_scribe::{Face, BaselineAxis};
    /// # fn demo(face: Face) {
    /// // Ideographic vs roman baseline offset for Latin, so a CJK run
    /// // and a Latin run share a line.
    /// let romn = face.baseline_coord(BaselineAxis::Horizontal, *b"latn", *b"romn");
    /// let ideo = face.baseline_coord(BaselineAxis::Horizontal, *b"latn", *b"ideo");
    /// # let _ = (romn, ideo);
    /// # }
    /// ```
    pub fn baseline_coord(
        &self,
        axis: BaselineAxis,
        script_tag: [u8; 4],
        baseline_tag: [u8; 4],
    ) -> Option<i16> {
        match self.kind {
            FaceKind::Ttf => self
                .with_font(|f| match axis {
                    BaselineAxis::Horizontal => {
                        f.base_horiz_y_for_script_baseline_var(script_tag, baseline_tag)
                    }
                    BaselineAxis::Vertical => {
                        f.base_vert_x_for_script_baseline_var(script_tag, baseline_tag)
                    }
                })
                .ok()
                .flatten(),
            FaceKind::Otf => {
                let otf_axis = match axis {
                    BaselineAxis::Horizontal => oxideav_otf::BaseAxis::Horizontal,
                    BaselineAxis::Vertical => oxideav_otf::BaseAxis::Vertical,
                };
                self.with_otf_font(|f| f.baseline_coord(otf_axis, &script_tag, &baseline_tag))
                    .ok()
                    .flatten()
            }
        }
    }

    /// The baseline tag a `script_tag` declares as its **default** on the
    /// given [`BaselineAxis`] (§6.3.1.3 `BaseScript::defaultBaselineIndex`
    /// into the axis's `BaseTagList`). This is the baseline the font's
    /// author wants text of that script to sit on when the layout client
    /// has no stronger preference — `romn` for Latin/Greek/Cyrillic,
    /// `ideo` (or `icfb`) for CJK, etc. — so a caller can align to the
    /// font's own intent instead of hard-coding a tag.
    ///
    /// Returns `None` when the face has no `BASE` table, the axis is
    /// absent, `script_tag` is unlisted, or the axis carries no
    /// `BaseTagList`.
    pub fn default_baseline_tag(&self, axis: BaselineAxis, script_tag: [u8; 4]) -> Option<[u8; 4]> {
        match self.kind {
            FaceKind::Ttf => self
                .with_font(|f| {
                    let base = f.base_table()?;
                    let ax = match axis {
                        BaselineAxis::Horizontal => base.horiz_axis.as_ref(),
                        BaselineAxis::Vertical => base.vert_axis.as_ref(),
                    }?;
                    let bs = ax.base_script_for_tag(script_tag)?;
                    let idx = bs.base_values.as_ref()?.default_baseline_index as usize;
                    ax.baseline_tags.as_ref()?.get(idx).copied()
                })
                .ok()
                .flatten(),
            FaceKind::Otf => {
                let otf_axis = match axis {
                    BaselineAxis::Horizontal => oxideav_otf::BaseAxis::Horizontal,
                    BaselineAxis::Vertical => oxideav_otf::BaseAxis::Vertical,
                };
                self.with_otf_font(|f| {
                    let ax = f.base()?.axis(otf_axis)?;
                    let bs = ax.script(&script_tag)?;
                    ax.baseline_tags()
                        .get(bs.default_baseline_index as usize)
                        .copied()
                })
                .ok()
                .flatten()
            }
        }
    }

    /// The design-unit coordinate of `script_tag`'s **default** baseline
    /// on `axis` — [`Self::default_baseline_tag`] resolved through
    /// [`Self::baseline_coord`]. `None` under the same conditions as
    /// those two. Convenience for "give me the baseline this script wants,
    /// and where it sits" without naming the tag.
    pub fn default_baseline_coord(&self, axis: BaselineAxis, script_tag: [u8; 4]) -> Option<i16> {
        let tag = self.default_baseline_tag(axis, script_tag)?;
        self.baseline_coord(axis, script_tag, tag)
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
        self.shape_text_planned(text, features, None, &[])
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
        self.shape_text_planned(text, features, Some(script_tag), &[])
    }

    /// Round 183 — shape `text` with caller-specified GSUB feature
    /// tags and per-feature **alternate-index** selection for Type-3
    /// (Alternate Substitution) lookups.
    ///
    /// `feature_alternates` is a list of `(feature_tag, alternate_index)`
    /// pairs. The set of features applied is the union of the tags
    /// listed — callers don't need a separate `features` argument.
    /// Feature application order is the order entries appear in
    /// `feature_alternates`.
    ///
    /// **What the alternate index changes:** the OpenType spec §6.2.3
    /// (Alternate Substitution) doesn't pin which entry of a coverage
    /// glyph's `AlternateSet` the layout engine picks. The round-156
    /// default ([`Self::shape_text`] and friends) hardcodes
    /// `alternateIndex = 0` — the first listed alternate. Round 183
    /// gives callers explicit control: requesting
    /// `(*b"salt", 2)` picks the third alternate for every Type-3
    /// covered slot under `salt`.
    ///
    /// **Out-of-range index → cmap-identity for that slot.** When a
    /// covered glyph's `AlternateSet` has fewer entries than the
    /// requested `alternate_index`, the slot passes through unchanged
    /// rather than panicking or rolling over to index 0 (the
    /// underlying `oxideav_ttf::Font::gsub_apply_lookup_type_3`
    /// returns `None` and we leave the slot at its current GID). This
    /// is the safest fallback for callers that don't pre-probe the
    /// per-font alternate count — request alternate-3 unconditionally
    /// and get "alternate-3 if the font has one, default cmap glyph
    /// otherwise".
    ///
    /// **Only Type-3 lookups consult the alternate index.** Features
    /// that mix Type-3 with Type-1 (Single), Type-2 (Multiple), or
    /// Type-4 (Ligature) lookups — `aalt` is the canonical example —
    /// still dispatch the non-Type-3 components with their existing
    /// length-preserving / length-changing walkers. The alternate
    /// index is silently ignored for those.
    ///
    /// Auto-probes the broadened script-tag priority list — see
    /// [`Self::shape_text`] for the resolution order. Drop to
    /// [`Self::shape_text_with_script_and_alternates`] for the
    /// deterministic-resolution mirror that bypasses the priority
    /// walk.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use oxideav_scribe::Face;
    /// # fn demo(face: Face) {
    /// // Ask for `salt` alternate-2 (third stylistic alternate).
    /// let gids = face.shape_text_with_alternates("Ag", &[(*b"salt", 2)]);
    /// // Falls back to cmap glyph for slots whose `salt`
    /// // AlternateSet has fewer than 3 entries.
    /// # let _ = gids;
    /// # }
    /// ```
    pub fn shape_text_with_alternates(
        &self,
        text: &str,
        feature_alternates: &[([u8; 4], u16)],
    ) -> Vec<u16> {
        let features: Vec<[u8; 4]> = feature_alternates.iter().map(|(tag, _)| *tag).collect();
        self.shape_text_planned(text, &features, None, feature_alternates)
    }

    /// Round 183 — explicit-script-tag mirror of
    /// [`Self::shape_text_with_alternates`]. Every requested feature
    /// resolves against `script_tag` alone (no priority walk); per-
    /// feature Type-3 alternate indices come from
    /// `feature_alternates`.
    ///
    /// Combines the determinism of
    /// [`Self::shape_text_with_script`] (round 175) with the caller-
    /// driven alternate-index selection of
    /// [`Self::shape_text_with_alternates`] (round 183). Use it when
    /// you already know the run's script and also want non-default
    /// Type-3 alternates — for example, a CJK pipeline forcing `aalt`
    /// against `hani` with `alternate_index = 1` to expose the font's
    /// second-stylistic-set glyph for a particular CJK codepoint
    /// without risking a `latn`/`aalt` cross-script collision from
    /// the auto-probe walk.
    ///
    /// Unknown `script_tag` yields cmap-identity (every feature
    /// resolves to an empty lookup list); out-of-range alternate
    /// index falls back to cmap-identity per slot; non-Type-3
    /// lookups dispatch with their existing semantics, all matching
    /// the round-183 contract on the auto-probe variant.
    pub fn shape_text_with_script_and_alternates(
        &self,
        text: &str,
        script_tag: [u8; 4],
        feature_alternates: &[([u8; 4], u16)],
    ) -> Vec<u16> {
        let features: Vec<[u8; 4]> = feature_alternates.iter().map(|(tag, _)| *tag).collect();
        self.shape_text_planned(text, &features, Some(script_tag), feature_alternates)
    }

    /// Round 362 — shape `text` with the caller-requested GSUB features
    /// applied **and positioned** through the full GPOS pass, returning
    /// [`crate::shaper::PositionedGlyph`]s with per-glyph advances and
    /// offsets.
    ///
    /// Where [`Self::shape_text`] stops after GSUB substitution and hands
    /// back bare glyph IDs, this method bridges the substituted run to the
    /// positioning pipeline (pair kerning, SinglePos, mark-to-base /
    /// mark-to-mark / mark-to-ligature attachment, cursive attachment, and
    /// contextual / chained-contextual positioning). The result is a
    /// render-ready run — the same shape [`crate::Shaper::shape`]
    /// produces, but with the caller's optional/discretionary features
    /// (`smcp` / `frac` / `sups` / `onum` / stylistic sets / …) folded in.
    ///
    /// `size_px` and the face's `unitsPerEm` set the design-unit→pixel
    /// scale; the output `x_advance` / `x_offset` / `y_offset` are in
    /// raster pixels. Every glyph is tagged `face_idx = 0` (single-face);
    /// use [`crate::FaceChain::shape_with_features`] for the multi-face
    /// fallback variant.
    ///
    /// Returns an empty vec for OTF (CFF) faces, empty `text`, or
    /// `size_px <= 0.0` — same constraints as [`Self::shape_text`] plus
    /// the size guard.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use oxideav_scribe::Face;
    /// # fn demo(face: Face) {
    /// // Small caps + slashed zero, positioned at 16px.
    /// let placed = face.position_text("Hello 0", 16.0, &[*b"smcp", *b"zero"]);
    /// # let _ = placed;
    /// # }
    /// ```
    pub fn position_text(
        &self,
        text: &str,
        size_px: f32,
        features: &[[u8; 4]],
    ) -> Vec<crate::shaper::PositionedGlyph> {
        self.position_text_planned(text, size_px, features, None)
    }

    /// Round 362 — explicit-script-tag mirror of [`Self::position_text`].
    ///
    /// Every requested feature is resolved against `script_tag` alone (no
    /// priority walk), then the substituted run is positioned through the
    /// full GPOS pass. Use this when the caller already knows the run's
    /// script so a feature published under two scripts resolves
    /// deterministically. Same return / degenerate-input contract as
    /// [`Self::position_text`].
    pub fn position_text_with_script(
        &self,
        text: &str,
        size_px: f32,
        script_tag: [u8; 4],
        features: &[[u8; 4]],
    ) -> Vec<crate::shaper::PositionedGlyph> {
        self.position_text_planned(text, size_px, features, Some(script_tag))
    }

    /// GSUB feature substitution over the kept view and plan; empty
    /// for a CFF face.
    fn shape_text_planned(
        &self,
        text: &str,
        features: &[[u8; 4]],
        script_tag: Option<[u8; 4]>,
        feature_alternates: &[([u8; 4], u16)],
    ) -> Vec<u16> {
        match self.ttf() {
            Ok(font) => crate::shaping::feature_subst::shape_text_inner(
                font,
                &self.plan,
                text,
                features,
                script_tag,
                feature_alternates,
            ),
            Err(_) => Vec::new(),
        }
    }

    /// GSUB feature substitution + GPOS positioning over the kept view
    /// and plan; empty for a CFF face.
    fn position_text_planned(
        &self,
        text: &str,
        size_px: f32,
        features: &[[u8; 4]],
        script_tag: Option<[u8; 4]>,
    ) -> Vec<crate::shaper::PositionedGlyph> {
        match self.ttf() {
            Ok(font) => crate::shaping::feature_subst::position_text_inner(
                font,
                &self.plan,
                text,
                size_px,
                features,
                script_tag,
                &[],
                0,
            ),
            Err(_) => Vec::new(),
        }
    }

    /// Round 377 — resolve a Unicode [`intl::unicode::script::Script`] to
    /// the OpenType script tag **this font actually registers**.
    ///
    /// [`crate::script::ot_script_tags`] returns the registry's tag list
    /// for a script (modern "v.2" tag first, legacy tag second for the
    /// Indic scripts). This method walks that list and returns the first
    /// tag the font publishes a `GSUB` `ScriptList` entry for — so a
    /// Devanagari run resolves to `dev2` on a modern font but falls back
    /// to `deva` on a legacy-only font. A font registers a script when
    /// [`oxideav_ttf::Font::gsub_features_for_script`] returns a non-empty
    /// feature list for the tag (or the script's default-language-system
    /// is present even with no features — checked via the same call with
    /// `lang_tag = None`).
    ///
    /// When the font registers **none** of the script's tags (e.g. a
    /// Latin-only font asked about a Devanagari run, or an OTF/CFF face
    /// with no parsed GSUB here), the **primary** registry tag
    /// ([`crate::script::ot_script_tag`]) is returned unchanged — the
    /// shaper then simply finds no script-specific lookups and falls back
    /// to cmap-only positioning, which is the correct graceful
    /// degradation.
    pub fn resolve_ot_script_tag(&self, script: intl::unicode::script::Script) -> [u8; 4] {
        let tags = crate::script::ot_script_tags(script);
        // OTF (CFF) faces don't expose GSUB here; return the primary
        // tag and let cmap-only shaping handle it.
        let Ok(font) = self.ttf() else {
            return tags[0];
        };
        tags.iter()
            .copied()
            .find(|&tag| !self.plan.gsub_features(font, tag).is_empty())
            .unwrap_or(tags[0])
    }

    /// Round 377 — shape a **mixed-script** string by itemising it into
    /// per-script runs and positioning each run under its own OpenType
    /// script tag.
    ///
    /// Where [`Self::position_text_with_script`] applies a single caller-
    /// supplied script tag to the whole input, this method runs
    /// [`crate::script::script_runs`] over `text`, resolves each run's
    /// Unicode script to its OpenType tag via
    /// [`crate::script::ot_script_tag`], and positions each run under that
    /// tag through the full GSUB-feature + GPOS pipeline
    /// ([`Self::position_text_with_script`]). The per-run glyph streams
    /// are concatenated in logical order into one
    /// [`crate::shaper::PositionedGlyph`] vector.
    ///
    /// This is the segmenter-driven shaping path: a string like
    /// `"Hello \u{0939}\u{093F}"` (Latin then Devanagari) selects `latn`
    /// for the first run and the Devanagari tag for the second, so a
    /// feature published under one script does not leak into the other.
    /// The per-run tag is resolved through [`Self::resolve_ot_script_tag`],
    /// which prefers the modern "v.2" tag but falls back to the legacy
    /// tag (e.g. `deva`) when the font only registers that one — so the
    /// path shapes correctly on both modern and legacy Indic fonts.
    ///
    /// Each run is positioned starting at pen x = 0; the per-glyph
    /// `x_advance` values mean a renderer accumulates the pen across run
    /// boundaries exactly as it does within a run — no inter-run gap is
    /// inserted. The same `features` set is offered to every run (a run
    /// whose script does not register a requested feature simply ignores
    /// it). Bidi reordering is **not** applied here — this is the
    /// logical-order itemised-shaping primitive; callers wanting visual
    /// order pass each [`crate::layout::ShapedVisualLine`] run through
    /// the bidi pipeline first.
    ///
    /// Returns an empty vec for OTF (CFF) faces, empty `text`, or
    /// `size_px <= 0.0`.
    pub fn position_text_itemized(
        &self,
        text: &str,
        size_px: f32,
        features: &[[u8; 4]],
    ) -> Vec<crate::shaper::PositionedGlyph> {
        if text.is_empty() || size_px <= 0.0 || !size_px.is_finite() {
            return Vec::new();
        }
        let chars: Vec<char> = text.chars().collect();
        let runs = crate::script::script_runs(&chars);
        let mut out: Vec<crate::shaper::PositionedGlyph> = Vec::new();
        for run in runs {
            let run_text: String = chars[run.start..run.end].iter().collect();
            let tag = self.resolve_ot_script_tag(run.script);
            let mut placed = self.position_text_with_script(&run_text, size_px, tag, features);
            out.append(&mut placed);
        }
        out
    }

    /// Round 377 — the substitution-only counterpart of
    /// [`Self::position_text_itemized`]: itemise `text` into per-script
    /// runs and run GSUB substitution on each run under its resolved
    /// OpenType script tag, returning the concatenated post-substitution
    /// glyph IDs in logical order.
    ///
    /// Where [`Self::shape_text`] auto-probes a single script-tag priority
    /// list for the whole input, this method picks the right tag *per
    /// run* (via [`Self::resolve_ot_script_tag`], modern-v.2-preferred
    /// with legacy fallback), so a Latin word and a Devanagari word in the
    /// same string each resolve their features against their own script.
    /// The same `features` set is offered to every run.
    ///
    /// Returns an empty vec for OTF (CFF) faces or empty `text`. This is
    /// the gid-level API (no positioning); use
    /// [`Self::position_text_itemized`] for render-ready
    /// [`crate::shaper::PositionedGlyph`]s.
    pub fn shape_text_itemized(&self, text: &str, features: &[[u8; 4]]) -> Vec<u16> {
        if text.is_empty() {
            return Vec::new();
        }
        let chars: Vec<char> = text.chars().collect();
        let mut out: Vec<u16> = Vec::new();
        for run in crate::script::script_runs(&chars) {
            let run_text: String = chars[run.start..run.end].iter().collect();
            let tag = self.resolve_ot_script_tag(run.script);
            out.extend(self.shape_text_with_script(&run_text, tag, features));
        }
        out
    }

    /// Round 377 — itemise `text` and pair each [`crate::script::ScriptRun`]
    /// with the OpenType script tag **this font** resolves for it
    /// ([`Self::resolve_ot_script_tag`]).
    ///
    /// A convenience for callers driving their own per-run shaping (or
    /// debugging itemisation): returns one `(ScriptRun, [u8; 4])` per run,
    /// in logical order, partitioning `0..text.chars().count()`. The
    /// run's `start` / `end` are char indices into `text`.
    pub fn script_run_tags(&self, text: &str) -> Vec<(crate::script::ScriptRun, [u8; 4])> {
        let chars: Vec<char> = text.chars().collect();
        crate::script::script_runs(&chars)
            .into_iter()
            .map(|run| {
                let tag = self.resolve_ot_script_tag(run.script);
                (run, tag)
            })
            .collect()
    }

    /// Run a closure with the face's parsed `oxideav_otf::Font<'_>`
    /// view. Mirrors [`Face::with_font`] for the CFF / cubic-Bezier
    /// path: a borrow of the view built at construction, not a parse.
    ///
    /// Returns `Error::WrongFaceKind` if this face was constructed
    /// from TTF bytes.
    pub fn with_otf_font<R>(
        &self,
        f: impl FnOnce(&oxideav_otf::Font<'_>) -> R,
    ) -> Result<R, Error> {
        Ok(f(self.otf()?))
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
    ///
    /// With an [`OutlineCache`] attached the outline is decoded once
    /// per *(glyph, variation instance)* and this returns a copy of
    /// the kept path; use [`Face::glyph_path_shared`] to share it
    /// without copying.
    pub fn glyph_path(&self, glyph_id: u16) -> Option<Path> {
        match &self.outline_cache {
            Some(cache) => self
                .cached_outline(cache, glyph_id)
                .map(|shared| Path::clone(&shared)),
            None => self.decode_outline(glyph_id),
        }
    }

    /// [`Face::glyph_path`] behind an `Arc`: with an [`OutlineCache`]
    /// attached this hands out the cache's own allocation (no copy of
    /// the commands); without one it decodes the outline and wraps it.
    /// The path is in the same Y-up font-unit space.
    pub fn glyph_path_shared(&self, glyph_id: u16) -> Option<Arc<Path>> {
        match &self.outline_cache {
            Some(cache) => self.cached_outline(cache, glyph_id),
            None => self.decode_outline(glyph_id).map(Arc::new),
        }
    }

    /// Attach (or, with `None`, detach) a shared [`OutlineCache`].
    /// While one is attached, [`Face::glyph_path`],
    /// [`Face::glyph_path_shared`] and the outline branch of
    /// [`Face::glyph_node`] decode each *(glyph, variation instance)*
    /// once and reuse the font-unit path at every size and position.
    /// Output is identical with and without a cache.
    ///
    /// No cache is attached by default. See [`crate::outline_cache`]
    /// for the key, the memory bound and the threading rules.
    pub fn set_outline_cache(&mut self, cache: Option<Arc<OutlineCache>>) {
        self.outline_cache = cache;
    }

    /// Builder form of [`Face::set_outline_cache`].
    #[must_use]
    pub fn with_outline_cache(mut self, cache: Arc<OutlineCache>) -> Self {
        self.outline_cache = Some(cache);
        self
    }

    /// The attached [`OutlineCache`], if any.
    pub fn outline_cache(&self) -> Option<&Arc<OutlineCache>> {
        self.outline_cache.as_ref()
    }

    fn cached_outline(&self, cache: &OutlineCache, glyph_id: u16) -> Option<Arc<Path>> {
        cache.get_or_decode(self.id, glyph_id, &self.instance, || {
            self.decode_outline(glyph_id)
        })
    }

    /// Decode `glyph_id`'s outline from the parsed view (at the
    /// installed variation instance) into a font-unit path.
    fn decode_outline(&self, glyph_id: u16) -> Option<Path> {
        if let Some(font) = self.font.ttf() {
            // CBDT-only glyphs (e.g. emoji) parse as empty outlines.
            let outline = font.glyph_outline(glyph_id).ok()?;
            if outline.contours.is_empty() {
                return None;
            }
            Some(tt_outline_to_path(&outline))
        } else {
            let outline = self.font.otf()?.glyph_outline(glyph_id).ok()?;
            if outline.contours.is_empty() {
                return None;
            }
            Some(cff_outline_to_path(&outline))
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
        if self.has_color_bitmaps()
            && let Ok(Some(cgb)) = self.raster_color_glyph_at(glyph_id, size_px)
            && !cgb.bitmap.is_empty()
        {
            let w = cgb.bitmap.width;
            let h = cgb.bitmap.height;
            // Pack the RGBA8 into a VideoFrame with one plane.
            let stride = (w as usize) * 4;
            let frame = VideoFrame {
                pts: None,
                planes: vec![VideoPlane {
                    stride,
                    data: cgb.bitmap.data,
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

        // Outline path: take the Y-up native Path and bake in
        // `scale * (1, -1)` so the resulting Path lives directly in
        // Y-down raster pixels at `size_px`. (We could ship a wrapping
        // `Group { transform: scale(scale, -scale), .. }` instead;
        // baking the transform keeps the Node leaf-shaped and lets
        // shape_to_paths emit a pure translation per glyph.)
        let upem = self.units_per_em.max(1) as f32;
        let scale = size_px / upem;
        let path = match &self.outline_cache {
            // The kept font-unit path is shared; scale a copy of it.
            Some(cache) => {
                let shared = self.cached_outline(cache, glyph_id)?;
                scale_and_flip_path(&shared, scale)
            }
            // Nobody else sees the freshly decoded path: scale it
            // where it is.
            None => {
                let mut raw = self.decode_outline(glyph_id)?;
                scale_and_flip_in_place(&mut raw, scale);
                raw
            }
        };
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
    Path {
        commands: src
            .commands
            .iter()
            .map(|cmd| scale_and_flip_command(*cmd, scale))
            .collect(),
    }
}

/// [`scale_and_flip_path`] over a path the caller owns, reusing its
/// allocation.
fn scale_and_flip_in_place(path: &mut Path, scale: f32) {
    for cmd in &mut path.commands {
        *cmd = scale_and_flip_command(*cmd, scale);
    }
}

fn scale_and_flip_command(cmd: PathCommand, scale: f32) -> PathCommand {
    let map = |p: Point| Point::new(p.x * scale, -p.y * scale);
    // Glyph outlines never emit ArcTo (TT/CFF have no arc primitive
    // — TT's quadratics + CFF's cubics are it), so the match arms
    // below cover every variant we'll ever see. The wildcard is a
    // forward-compat safety net for the `#[non_exhaustive]` enum.
    match cmd {
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
    }
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
    // Every point yields at most one command, plus MoveTo, a possible
    // closing curve and Close per contour.
    let capacity: usize = outline.contours.iter().map(|c| c.points.len() + 3).sum();
    let mut out = Path {
        commands: Vec::with_capacity(capacity),
    };
    let xy = |p: &oxideav_ttf::Point| Point::new(p.x as f32, p.y as f32);
    for contour in &outline.contours {
        let pts = &contour.points;
        if pts.is_empty() {
            continue;
        }
        let n = pts.len();
        // Start at the first on-curve point and walk the remaining
        // points cyclically from there; if there is none, synthesise a
        // start at the midpoint of pts[0]..pts[1] (Apple-TT phantom
        // on-curve) and walk every point.
        let (start_xy, first, count) = match pts.iter().position(|p| p.on_curve) {
            Some(s) => (xy(&pts[s]), s + 1, n - 1),
            None => {
                let p0 = pts[0];
                let p1 = pts[1 % n];
                let mid = Point::new(
                    (p0.x as f32 + p1.x as f32) * 0.5,
                    (p0.y as f32 + p1.y as f32) * 0.5,
                );
                (mid, 0, n)
            }
        };

        out.commands.push(PathCommand::MoveTo(start_xy));
        let mut prev_off: Option<Point> = None;
        for p in pts.iter().cycle().skip(first).take(count) {
            let xy = xy(p);
            if p.on_curve {
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
    let capacity: usize = outline.contours.iter().map(|c| c.segments.len()).sum();
    let mut out = Path {
        commands: Vec::with_capacity(capacity),
    };
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

#[cfg(test)]
mod parse_once_tests {
    use super::*;
    use crate::parsed::PARSE_COUNT;

    fn parses_during(f: impl FnOnce()) -> usize {
        let before = PARSE_COUNT.with(|c| c.get());
        f();
        PARSE_COUNT.with(|c| c.get()) - before
    }

    #[test]
    fn face_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Face>();
        check::<crate::FaceChain>();
    }

    /// Construction is the only parse: drawing, colour queries,
    /// metrics, feature queries, shaping and installing variation
    /// instances all reuse the view the face already holds.
    #[test]
    fn nothing_parses_after_construction() {
        let mut face = None;
        let n = parses_during(|| {
            face = Some(
                Face::from_ttf_bytes(
                    include_bytes!("../tests/fixtures/InterVariable.ttf").to_vec(),
                )
                .unwrap(),
            );
        });
        assert_eq!(n, 1, "construction parses exactly once");
        let mut face = face.unwrap();

        let n = parses_during(|| {
            for round in 0..3 {
                for gid in 0..200u16 {
                    let _ = face.glyph_node(gid, 14.0);
                    let _ = face.glyph_path(gid);
                    let _ = face.glyph_path_shared(gid);
                    let _ = face.raster_color_glyph(gid, 14.0);
                    let _ = face.h_advance_delta(gid);
                    let _ = face.v_advance_delta(gid);
                    let _ = face.glyph_name(gid);
                }
                let _ = face.has_color_bitmaps();
                let _ = face.color_strike_sizes();
                let _ = face.is_variable();
                let _ = face.variation_axes();
                let _ = face.named_instances();
                let _ = face.normalised_coords();
                let _ = face.metric_delta(b"xhgt");
                let _ = face.stat_axes();
                let _ = face.name_id(1);
                let _ = face.has_base_table();
                let _ = face.baseline_coord(BaselineAxis::Horizontal, *b"latn", *b"romn");
                let _ = face.default_baseline_tag(BaselineAxis::Horizontal, *b"latn");
                let _ = face.gsub_features_for_script(*b"latn", None);
                let _ = face.layout_features_for_script(*b"latn", None);
                let _ = face.resolve_ot_script_tag(intl::unicode::script::Script::Latin);
                let _ = face.shape_text("office", &[*b"liga"]);
                let _ = face.position_text_itemized("Hello \u{0939}\u{093F}", 16.0, &[]);
                let _ = crate::Shaper::shape(&face, "Hello", 16.0);
                let _ = face.with_font(|f| f.glyph_index('A'));
                if round == 0 {
                    face.set_variation_coords(&[20.0, 650.0]).unwrap();
                    face.set_outline_cache(Some(Arc::new(OutlineCache::default())));
                } else {
                    face.clear_variation_coords();
                }
            }
        });
        assert_eq!(n, 0, "{n} font parses after construction");
    }

    #[test]
    fn cff_face_parses_once_too() {
        let mut face = None;
        let n = parses_during(|| {
            face = Some(
                Face::from_otf_bytes(
                    include_bytes!("../tests/fixtures/SourceSans3-Regular.otf").to_vec(),
                )
                .unwrap(),
            );
        });
        assert_eq!(n, 1);
        let face = face.unwrap();
        let n = parses_during(|| {
            for gid in 0..200u16 {
                let _ = face.glyph_node(gid, 14.0);
                let _ = face.glyph_path(gid);
            }
            let _ = face.has_base_table();
            let _ = face.baseline_coord(BaselineAxis::Horizontal, *b"latn", *b"romn");
            let _ = face.glyph_name(5);
            let _ = face.with_otf_font(|f| f.glyph_index('A'));
        });
        assert_eq!(n, 0);
    }

    /// The variation setters keep their observable contract: partial
    /// vectors overlay the installed instance, values clamp, and
    /// clearing returns to the default instance.
    #[test]
    fn instance_install_overlays_clamps_and_clears() {
        let mut face =
            Face::from_ttf_bytes(include_bytes!("../tests/fixtures/InterVariable.ttf").to_vec())
                .unwrap();
        let axes = face.variation_axes();
        assert_eq!(axes.len(), 2);
        assert!(face.variation_coords().is_empty());
        assert!(face.normalised_coords().is_empty());

        face.set_variation_coords(&[20.0, 700.0]).unwrap();
        assert_eq!(face.variation_coords(), &[20.0, 700.0]);
        assert_eq!(face.normalised_coords().len(), 2);
        // A shorter vector leaves the trailing axis where it was.
        face.set_variation_coords(&[24.0]).unwrap();
        assert_eq!(face.variation_coords(), &[24.0, 700.0]);
        // Out-of-range values clamp to the axis range.
        face.set_variation_coords(&[1.0e6, -5.0]).unwrap();
        assert_eq!(face.variation_coords(), &[axes[0].max, axes[1].min]);

        let commands = |f: &Face| f.glyph_path(40).map(|p| p.commands);
        let bold_a = commands(&face);
        face.clear_variation_coords();
        assert!(face.variation_coords().is_empty());
        assert!(face.normalised_coords().is_empty());
        let fresh =
            Face::from_ttf_bytes(include_bytes!("../tests/fixtures/InterVariable.ttf").to_vec())
                .unwrap();
        assert_eq!(commands(&face), commands(&fresh));
        assert_ne!(commands(&face), bold_a);

        // Static and CFF faces: a no-op and a typed error.
        let mut fixed =
            Face::from_ttf_bytes(include_bytes!("../tests/fixtures/DejaVuSans.ttf").to_vec())
                .unwrap();
        fixed.set_variation_coords(&[700.0]).unwrap();
        assert!(fixed.variation_coords().is_empty());
        let mut cff = Face::from_otf_bytes(
            include_bytes!("../tests/fixtures/SourceSans3-Regular.otf").to_vec(),
        )
        .unwrap();
        assert!(matches!(
            cff.set_variation_coords(&[700.0]),
            Err(Error::WrongFaceKind { .. })
        ));
    }
}
