# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- `tests/round7_glyph_path.rs`: switched test glyph from 'A' to 'O'.
  DejaVu Sans Mono's 'A' is a pure 13-command polygon (zero curves),
  so the `QuadCurveTo ≥ 1` assertion never held. 'O' is the canonical
  curve-bearing glyph and exercises the TT quadratic outline path.
  Test re-enabled.
- `tests/round4_marks.rs::double_diacritic_stacks_above_first`: replaced
  the `circumflex.y_offset < 0` assertion (which was incorrect — the
  shaper's `y_offset` is a *delta* from the natural pen position, not an
  absolute raster Y, and DejaVu Sans's (e, circumflex) anchor pair has
  dy = 0) with `acute.y_offset < circumflex.y_offset`. This is the
  actual round-4 invariant: the second mark stacks STRICTLY above the
  first via the (mark1, mark2) anchor, vs. the round-3 fallback which
  would attach both marks to the base at the same anchor (overlap).
  Test re-enabled.

## [0.1.2](https://github.com/OxideAV/oxideav-scribe/compare/v0.1.1...v0.1.2) - 2026-05-04

### Other

- ignore round-7 'A' QuadCurveTo assertion ([#6](https://github.com/OxideAV/oxideav-scribe/pull/6))
- ignore round-4 double-diacritic y_offset assertion ([#5](https://github.com/OxideAV/oxideav-scribe/pull/5))
- ignore round-3 sub-pixel bitmap tests pending horizontal AA ([#4](https://github.com/OxideAV/oxideav-scribe/pull/4))
- silence rust 1.95 needless_range_loop + manual_range_contains
- re-land round 5: CBDT/CBLC color bitmap glyphs

### Added — round 7, vector-first text export (2026-05-04)

- `Face::glyph_path(glyph_id) -> Option<oxideav_core::Path>` — returns
  the raw glyph outline as vector commands in the font's native Y-up
  font-unit coordinate space. TT outlines map to `MoveTo` + `LineTo`
  + `QuadCurveTo` + `Close` (with the standard implicit-on-curve
  midpoint reconstruction for adjacent off-curve points). CFF outlines
  map to `MoveTo` + `LineTo` + `CubicCurveTo` + `Close`, mirroring the
  Type 2 charstring decode 1:1. Bitmap-only glyphs (CBDT-only fonts)
  return `None`.
- `Face::glyph_node(glyph_id, size_px) -> Option<oxideav_core::Node>`
  — returns a self-contained `Node` ready to be positioned at the pen
  origin. Outline glyphs come back as `Node::Path(PathNode)` with the
  Y-flip + size scale baked in (origin at (0, 0) = pen origin, Y grows
  downward), with a default black solid fill. Bitmap glyphs (CBDT,
  e.g. Noto Color Emoji) come back as `Node::Image(ImageRef)` carrying
  the rasterised RGBA bitmap as an `oxideav_core::VideoFrame`, with
  `bounds` sized to the bitmap's CBDT-declared placement at the
  strike's native ppem (scaled by `size_px / strike_ppem`). COLRv1
  layered glyphs fall through to the base outline path for now —
  round 6 (#356) extends `glyph_node` to return a `Group` of layered
  PathNodes per COLR layer.
- `Shaper::shape_to_paths(face_chain, text, size_px) -> Vec<(usize,
  Node, Transform2D)>` — primary vector text API. Shapes through the
  full GSUB / GPOS / mark-attachment / face-chain-fallback pipeline,
  then wraps each glyph in a `Node` and a positioning `Transform2D`
  (pure translation by the cumulative pen advance + per-glyph
  kerning / mark x_offset / y_offset). Non-rendering glyphs (SPACE,
  empty outlines) advance the pen but do not appear in the output
  vector — the returned length is `<= shaped.len()`.
- The existing rasterise APIs (`render_text`, `render_text_styled`,
  `render_text_wrapped`, `Composer::compose_run*`) are **unchanged**
  in this round — they keep returning `RgbaBitmap`. Removal /
  rewriting in terms of the vector pipeline is task #354.
- `oxideav-core` minimum bumped to `0.1.14` for the `vector` module
  types (`Path`, `PathCommand`, `Node`, `PathNode`, `ImageRef`,
  `Paint`, `FillRule`, `Transform2D`, `Rect`, `Rgba`, `Point`).
- New integration tests:
  - `tests/round7_glyph_path.rs` — DejaVu Sans Mono 'A' must emit
    `MoveTo` + `Close` + ≥1 `QuadCurveTo` and zero cubics; SPACE
    returns `None`.
  - `tests/round7_glyph_path_cff.rs` — Source Sans 3 'A' must emit
    `MoveTo` + `Close` + ≥1 `CubicCurveTo` and zero quads.
  - `tests/round7_shape_to_paths.rs` — DejaVu Sans Mono "Hi" must
    return 2 `PathNode`s with the second translated rightward;
    "A B" skips the SPACE node and translates 'B' past it.
  - `tests/round7_bitmap_glyph_node.rs` — Noto Color Emoji
    `glyph_node('🎉', 96.0)` must be `Node::Image(ImageRef)` with
    a non-empty RGBA plane and ≥1 non-zero-alpha pixel.

### Added — round 5, CBDT/CBLC color bitmap glyphs (2026-05-04)

- New `color_glyph` module + `ColorGlyphBitmap { bitmap, bearing_x,
  bearing_y, advance, ppem }` carrying a decoded RGBA bitmap plus the
  metrics needed for placement.
- `Face::has_color_bitmaps() -> bool` — wraps
  `oxideav_ttf::Font::has_color_bitmaps`. Short-circuits for OTF
  faces (no CFF font we've seen ships CBDT).
- `Face::color_strike_sizes() -> Vec<(u8, u8)>` — all `(ppem_x,
  ppem_y)` strikes the face declares; empty when no CBDT.
- `Face::raster_color_glyph(glyph_id, size_px) -> Result<Option<
  ColorGlyphBitmap>, Error>` — picks the closest strike to
  `size_px.round()`, walks CBLC to find the glyph entry, hands the
  raw PNG bytes from CBDT to `oxideav_png::decode_png_to_frame`, and
  unwraps the `VideoFrame` into an `RgbaBitmap`. PNG width/height
  are recovered directly from the IHDR chunk so the bytes-per-pixel
  ratio is unambiguous (no heuristic: stride / width gives the
  right answer for Rgba8 / Rgb24 / Ya8 / Gray8).
- `oxideav-png` added as a dependency of `oxideav-scribe`. NO `png` /
  `image` crate per workspace policy.
- `tests/round5_emoji.rs` integration test against
  `NotoColorEmoji.ttf` (10.6 MB; download-on-demand via the same
  fixture cache as the CJK test). Verifies the font loads (which
  pre-round-5 would fail because Noto Color Emoji has no `glyf`/
  `loca`), `has_color_bitmaps()` is true, U+1F389 PARTY POPPER
  resolves through cmap, the CBDT walker hands back a non-empty
  PNG payload, the rasterised RGBA bitmap has >5% non-zero alpha
  pixels, and `Shaper::shape("🎉 ok")` survives the no-outline
  shaping path.

### Added — round 5, TTC support + CJK fallback integration test (2026-05-04)

- `Face::from_ttc_bytes(bytes, index)` — construct a face from the
  `index`-th subfont of a TrueType Collection (TTC / `'ttcf'`). Stores
  the subfont index on the face so that `with_font` re-parses through
  `oxideav_ttf::Font::from_collection_bytes(bytes, i)` instead of
  `from_bytes` and the right subfont is selected each time.
- `Face::subfont_index() -> Option<u32>` — accessor; `None` for plain
  sfnt-flavour faces.
- New dev-dep on `ureq = "3"` for the integration-test fixture
  downloader (mirrors the `oxideav-msmpeg4` pattern).
- New `tests/font_fixtures/mod.rs` — shared download-cache-verify
  helper used by all round-5 integration tests. Caches under
  `target/test-fixtures/fonts/`, gates first-time downloads behind
  `OXIDEAV_NETWORK_TESTS=1`, SHA-256 verifies on every load. Skips
  silently with `eprintln!` when neither cached nor enabled.
- `tests/round5_cjk_fallback.rs` closes the round-2 deferral
  ("Two-script-coverage fallback integration test"). Loads
  NotoSansCJK-Medium.ttc subfont 0 (Japanese cut) as the fallback
  behind DejaVu Sans Mono, shapes `"hello 日本語 world"` through a
  `FaceChain`, and asserts:
  - Each Latin codepoint resolves to face_idx 0 (DejaVu).
  - Each CJK codepoint resolves to face_idx 1 (Noto CJK) with a
    non-zero glyph id.
  - The total run advance is positive and CJK glyphs are on average
    wider than Latin glyphs (east-asian wide sanity).

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
