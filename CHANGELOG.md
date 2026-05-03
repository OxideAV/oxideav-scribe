# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
