# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added — round 4, oblique fixture for italic-on-italic suppression (2026-05-04)

- `tests/fixtures/DejaVuSansMono-Oblique.ttf` (252 KB) — DejaVu Sans
  Mono Oblique with `post.italicAngle = -11.0°`. Shipped under the
  same Bitstream Vera license as the other DejaVu fixtures
  (DEJAVU-LICENSE already in tests/fixtures/).
- New integration test `round4_oblique.rs` verifies that the
  round-2 deferral is closed:
  - `Face::italic_angle()` round-trips `-11.0°` from the parser.
  - `synthetic_italic_shear(Style::italic(), face.italic_angle())`
    returns 0 on the oblique face — no double-shear synthesis.
  - `render_text_styled(face, "I", 32, WHITE, Style::REGULAR)` and
    `render_text_styled(face, "I", 32, WHITE, Style::italic())`
    produce *bit-identical* RGBA bitmaps on the oblique face.
  - The same italic() request on the matching upright fixture
    DOES synthesise a wider 'I' — confirming the asymmetry survives
    end-to-end.
  - A REGULAR request on the oblique face still renders the font's
    own forward slant (top-right quadrant alpha sum exceeds the
    upright fixture's).

### Added — round 4, GPOS mark-to-mark stacking (2026-05-04)

- Shaper now runs a 5th pass after mark-to-base: for each consecutive
  `(mark_prev, mark_new)` pair where both glyphs are GDEF marks,
  `oxideav_ttf::Font::lookup_mark_to_mark(prev, new)` is consulted.
  If the font ships an anchor for the pair, the new mark is positioned
  relative to the previous mark's *post-attachment* position (which
  already sits on the base). This handles double-diacritic stacks
  like Vietnamese `ê + acute → ế` and polytonic Greek
  `α + tonos + dialytika`.
- Round-3 mark-to-base remains the fallback for any pair the font
  doesn't cover with a mark-to-mark anchor (most fonts ship a sparse
  set of mark-to-mark anchors compared to mark-to-base).
- New integration test `round4_marks.rs` verifies that
  `e + COMBINING CIRCUMFLEX + COMBINING ACUTE` stacks the acute
  ABOVE the circumflex (proving mark-to-mark fired, not the
  round-3 fallback which would overlap them); that the gap scales
  linearly with `size_px`; and that the round-3 single-mark path is
  unaffected.

### Added — round 3, synthetic bold via alpha dilation (2026-05-04)

- `style::synthetic_bold_radius(style, face_weight, size_px) -> f32`
  — when the requested `style.weight` exceeds the face's natural
  `usWeightClass` by at least `SYNTHETIC_BOLD_THRESHOLD = 200` (two
  weight steps), returns the per-side dilation radius in pixels.
  Formula: `0.0001 * size_px * weight_delta`, clamped up to 1.0 px
  so the dilation kernel produces visible thickening at small body
  sizes. For Regular (400) → Bold (700) at 32 px this yields ~1.0
  px (clamped from 0.96), at 64 px ~1.92 px, matching what
  Microsoft GDI+ and libass produce for ASS `\b1` against a Regular
  face.
- `style::SYNTHETIC_BOLD_THRESHOLD = 200` and
  `SYNTHETIC_BOLD_PX_PER_WEIGHT_STEP_PER_PX = 0.0001` — tunable
  constants exposed for callers that need a different aesthetic.
- `Composer::compose_run` / `compose_run_styled` /
  `compose_run_with_stroke` — apply the synthetic-bold radius to
  the cached glyph bitmap on the fly, combined additively with any
  stroke radius, so a Bold + bordered cue gets a thick stroke
  surrounding a thick fill.
- `render_text_styled` — top-level entry point honours synthetic
  bold via the same dilation path.
- Caveat: callers that have a real Bold face available SHOULD prefer
  loading it as a separate `Face`; synthetic bold is the fallback
  for fonts that ship only one cut. The cache key already includes
  `weight` indirectly via the face id, so loading a real Bold face
  produces a separate cache slot from the Regular one.

### Added — round 3, GPOS mark-to-base attachment (2026-05-04)

- Shaper now runs a 4th pass after kerning: for each `(base, mark)`
  pair where `mark` is classified as a mark by the font's `GDEF`
  table, it queries `oxideav_ttf::Font::lookup_mark_to_base(base,
  mark)` and applies the returned anchor delta to the mark's
  `x_offset` / `y_offset`. The mark's `x_advance` is zeroed so a
  following glyph lands at the post-base pen position, matching what
  every desktop shaper does. Multi-mark stacks (e.g. polytonic
  Greek `α + tonos + dialytika`) work because each mark walks back
  through previous marks until a base is found.
- Y axis is converted from TT (Y-up) to raster (Y-down) at the
  shaper boundary so consumers don't have to think about it.
- No public-API change: `PositionedGlyph` already had `y_offset`
  from round 1.
- New integration test `round3_marks.rs` verifies `'A' + U+0301` →
  combining acute lifts above the baseline (negative raster Y
  offset), the mark's advance is zeroed, the offset scales linearly
  with `size_px`, and pure-base runs are untouched.

### Added — round 3, sub-pixel positioning (2026-05-04)

- `cache::SUBPIXEL_STEPS = 16` + `subpixel_slot(x_fract)` +
  `subpixel_offset(slot)` — quantise the fractional part of a pen X
  position to one of 16 sub-pixel positions per pixel. The composer
  uses these to bake the sub-pixel placement into the cached glyph
  bitmap, then blits at `floor(pen_x)`. Result: visibly cleaner edges
  at 12–14 px body sizes than naive integer rounding.
- `GlyphKey { ..., x_subpixel_q4: u8 }` + `GlyphKey::new_subpixel(...)`
  — extends the LRU key with the sub-pixel slot. Each unique
  `(face, glyph, size, shear)` tuple now occupies up to 16 cache
  slots; in practice ~3-4 are touched per glyph in typical text and
  the 256-entry default still holds an entire cue.
- `Rasterizer::raster_glyph_subpixel(face, gid, size_px, shear,
  x_subpixel)` + `Rasterizer::glyph_offset_subpixel(...)` — outline
  is shifted right by `x_subpixel` pixels before flatten, so the
  resulting alpha pattern reflects the sub-pixel placement. The
  bitmap left edge stays floor-aligned so the composer can blit at
  integer X.
- `outline::flatten_with_shear_offset(...)` — the underlying flatten
  helper that takes the shear + sub-pixel arguments. Bit-identical to
  `flatten_with_shear` when `x_subpixel == 0.0`.
- `render_text_styled` now uses sub-pixel positioning automatically;
  `Composer::compose_run` / `compose_run_styled` /
  `compose_run_with_stroke` likewise. Existing callers see a quality
  improvement with no API changes.

## [0.1.0](https://github.com/OxideAV/oxideav-scribe/compare/v0.0.1...v0.1.0) - 2026-05-03

### Other

- update deps and promote to 0.1

### Added — OTF / CFF integration (2026-05-03)

- `Face::from_otf_bytes(Vec<u8>)` — construct a face from an
  OpenType-CFF font (Adobe TN5176 / TN5177 via the new sibling
  `oxideav-otf` crate). Mirrors the existing `Face::from_ttf_bytes`
  TTF entry point.
- `Face::kind() -> FaceKind` + `FaceKind { Ttf, Otf }` — runtime
  discriminant for callers that need to dispatch on the underlying
  outline format.
- `Face::with_otf_font(|font| ...)` — re-parse-on-demand handle to
  the underlying `oxideav_otf::Font<'_>`. Mirrors `with_font` but
  rejects TTF-flavoured faces with `Error::WrongFaceKind`. The
  pre-existing `with_font` now also enforces this and rejects
  OTF-flavoured faces.
- `outline::flatten_cubic` + `outline::flatten_cubic_with_shear` —
  cubic-Bezier de Casteljau flattener that mirrors the existing
  quadratic path. Accepts a `oxideav_otf::CubicOutline` (explicit
  `MoveTo` / `LineTo` / `CurveTo` / `ClosePath` segments) and emits
  a polyline-`FlatOutline` with the same Y-down, top-left-origin
  convention. Tolerance + max-depth match the quadratic path.
- New `Error` variants: `Otf(oxideav_otf::Error)` and
  `WrongFaceKind { expected, actual }`.

### Added — round 2 (2026-05-03)

- `Style { italic: bool, weight: u16 }` — font request style carried
  through the shape → rasterise → cache pipeline. `Style::REGULAR` is
  the default (upright, 400). Builder helpers: `Style::italic()`,
  `Style::REGULAR.with_weight(700)`.
- `synthetic_italic_shear(style, face_italic_deg)` — derives the
  per-face shear value. Returns 0 when the request is upright OR when
  the face is already italic (within `ITALIC_ANGLE_EPSILON_DEG = 0.5
  deg`); otherwise returns `tan(12°) ≈ 0.213`. Documented in
  `style.rs`.
- `Face::italic_angle()`, `Face::weight_class()` — accessors that
  cache the values from `oxideav-ttf` at face construction.
- `outline::flatten_with_shear(outline, scale, shear_x_per_y)` —
  flattens a glyph outline with an optional horizontal shear applied
  in TT (Y-up) coordinates. The bbox is recomputed from the actual
  sheared points so the rasterizer sizes the bitmap correctly.
- `Rasterizer::raster_glyph_styled` + `Rasterizer::glyph_offset_styled`
  — sheared-aware variants of the round-1 entry points.
- `cache::GlyphKey::new_styled(face_id, glyph_id, size_px,
  shear_x_per_y)` — extends the LRU key with a `shear_q14` component
  so synthesised italic glyphs do not collide with upright ones.
- `FaceChain { faces: Vec<Face> }` — ordered-fallback chain.
  `FaceChain::new(primary).push_fallback(face)` (chainable).
  `FaceChain::shape(text, size_px)` walks the chain per codepoint and
  returns positioned glyphs whose `face_idx` identifies the source
  face.
- `PositionedGlyph::face_idx: u16` — new field. 0 for the primary;
  rasterizer reads it to pick the right face out of a chain.
- `shaper::shape_run_with_font(font, glyph_ids, size_px, face_idx)` —
  pre-cmap'd entry point used by `FaceChain` so each run-of-one-face
  does its own GSUB + GPOS pass.
- `Composer::compose_run_styled(glyphs, chain, size_px, style, color,
  dst, x, y)` — multi-face + italic-aware compose. Picks the per-face
  shear from `style` and `Face::italic_angle()` automatically.
- `Composer::compose_run_with_stroke(...)` — paints a dilated stroke
  under the fill, matching ASS `\bord` semantics.
- `StrokeStyle { width_px, color }` — stroke configuration.
- `stroke::dilate_alpha(bitmap, radius_px)` — circular max-filter
  dilation. Output bitmap is `2 * ceil(radius)` pixels larger in each
  dimension; caller subtracts `dilate_offset(radius)` from the blit
  origin to align.
- `render_text_styled(face, text, size_px, color, style)` — top-level
  styled render. `render_text(...)` keeps round-1 signature, defaults
  to `Style::REGULAR`.

### Changed

- `GlyphKey` now carries a `shear_q14: i32` component. Existing call
  sites that build keys via `GlyphKey::new(...)` keep their
  zero-shear behaviour unchanged.
- `PositionedGlyph` gained `face_idx: u16`. Round-1 callers using
  `Shaper::shape(face, ...)` get `face_idx = 0` for every glyph and
  see no behavioural change.
- `Composer::compose_run` is now a thin wrapper over a private
  `compose_run_inner` that handles single-face + chain + stroke in
  one place. Public signature is unchanged.

### Deferred (round 3)

- **DejaVuSans-Oblique fixture for "request italic on already-italic
  font"** — code path is unit-tested at `style::synthetic_italic_shear`
  level; integration test deferred until a small italic fixture lands.
- **Two-script-coverage fallback integration test** — needs a small
  CJK or emoji fixture in `oxideav-ttf`. Noto Sans CJK is ~10 MB,
  too big to vendor for one test.
- **Synthetic bold** — `Style.weight` is carried through but no
  dilation pass runs against the alpha mask yet.
- **True offset-curve stroke geometry** — current stroke uses alpha
  dilation (libass / ffmpeg-style). Round 3 may add a Minkowski-sum
  geometric mode for sharp corners at large bord widths.

## [0.0.1] - 2026-05-02

### Added

- Initial round-1 release of the pure-Rust font rasterizer + simple
  shaper + line layout crate.
- `Face` (`face.rs`) — owning wrapper around `oxideav_ttf::Font` with
  per-process face id for cache keying. Exposes `family_name`,
  `units_per_em`, `ascent_px`, `descent_px`, `line_height_px`.
- Outline flattening (`outline.rs`) — quadratic-Bezier subdivision
  via de Casteljau split, 0.5 px chord tolerance, with the standard
  TrueType implicit-on-curve reconstruction rule (and the all-off-
  curve synthetic-start fallback).
- Scanline rasterizer (`rasterizer.rs`) — active-edge-list fill at
  4× vertical supersampling for anti-aliasing. Even-odd fill rule.
- Round-1 shaper (`shaper.rs`) — cmap mapping with `.notdef`
  fallback, GSUB type 4 ligature substitution, GPOS type 2 pair
  kerning (with legacy `kern` table fallback inherited from
  `oxideav-ttf`).
- Composer (`compose.rs`) — wraps `oxideav_pixfmt::blit_alpha_mask`
  with per-glyph placement maths.
- Layout helpers (`layout.rs`) — `run_width` measurement +
  `wrap_lines` whitespace-aware word wrap with character-level
  fallback.
- LRU glyph bitmap cache (`cache.rs`) — `(face_id, glyph_id, size_q8)`
  keyed, default 256 entries.
- High-level convenience entry points: `render_text` (autosized
  single-line) + `render_text_wrapped` (multi-line word-wrapped).

### Deferred (round 2+)

- Bidi (UAX #9), Arabic shaping, Indic conjunct formation.
- Variable fonts (`fvar`/`gvar`/`MVAR`).
- TrueType bytecode hinting.
- CFF / Type 2 charstrings (will land via a sibling `oxideav-otf` crate).
- Mark-to-base / mark-to-mark attachment (GPOS types 4/5/6).
- Subpixel positioning + LCD filter.
