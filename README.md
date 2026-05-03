# oxideav-scribe

Pure-Rust font rasterizer + simple shaper + line layout for the
[oxideav](https://github.com/OxideAV) framework. Sits on top of
[`oxideav-ttf`](https://github.com/OxideAV/oxideav-ttf) (outlines,
GSUB/GPOS lookups) and [`oxideav-pixfmt`](https://github.com/OxideAV/oxideav-pixfmt)
(Porter-Duff alpha compositing) to turn a UTF-8 string + a face into an
RGBA bitmap suitable for a subtitle track or a scene compositor.

## Round-2 scope (this release)

Round 2 closes the four deferrals flagged by the subtitle integration:

- **Synthesised italic** — `Style { italic: bool, weight: u16 }` plumbed
  through shaping + rasterisation. When the requested style is italic
  AND the underlying face's `post.italicAngle` is ~0 (upright), the
  rasterizer applies a 12° horizontal shear at outline-flatten time;
  when the face is already italic, no double-shear happens.
- **Per-run colour** — verified end-to-end: cached glyph bitmaps are
  alpha-only, the run colour mixes in at compose time. Two
  back-to-back calls with different colours produce different RGB at
  glyph pixels but identical alpha shape.
- **Font fallback** — `FaceChain` walks an ordered list of faces. For
  each codepoint, the first face whose `glyph_index` returns a real
  (non-`.notdef`) glyph wins; if no face has it, the primary's
  `.notdef` is used. `PositionedGlyph::face_idx` tells the rasterizer
  which face to fetch the outline from.
- **Text outline / stroke** — `compose_run_with_stroke` paints a
  dilated alpha-mask underneath the fill, matching the `\bord`
  semantics that mpv / libass and the ffmpeg `subtitles` filter ship.
  Round 2 uses circular max-filter dilation (fast, looks identical to
  ffmpeg at typical 1–3 px bord values); true offset-curve geometry
  is a round-3 lift.

## Round-1 scope (still in)

- **Outline flattening** — quadratic-Bezier subdivision via the
  classic de Casteljau split (chord tolerance 0.5 px). Implements the
  TrueType implicit-on-curve rule for adjacent off-curve handles, plus
  the Apple "all-off-curve" synthetic-start fallback. Now optionally
  honours a horizontal shear for synthetic italic.
- **Scanline rasterisation** — active-edge-list fill with 4× vertical
  supersampling for anti-aliasing. Even-odd fill rule.
- **Shaper** — `cmap` mapping with `.notdef` fallback for unmapped
  codepoints. Ligature substitution via GSUB type 4. Pair kerning via
  GPOS type 2 with legacy `kern` table fallback.
- **Composer** — Porter-Duff "over" via
  `oxideav_pixfmt::blit_alpha_mask` with straight-alpha destinations.
- **Layout** — line measurement + word-wrap. Tokenises on whitespace
  and falls back to character-level breaks if a single word overflows.
- **LRU cache** — glyph bitmap reuse keyed by
  `(face_id, glyph_id, size_q8, shear_q14)`; default capacity 256
  covers a typical subtitle session at >95% hit rate. Shear key
  component keeps synthesised-italic glyphs out of the upright slot.

## Public API

```rust
use oxideav_scribe::{
    render_text, render_text_styled, render_text_wrapped,
    Composer, Face, FaceChain, RgbaBitmap, Shaper, Style, StrokeStyle, WHITE,
};

let bytes = std::fs::read("DejaVuSans.ttf")?;
let face  = Face::from_ttf_bytes(bytes)?;

// Round-1 entry point — defaults to upright Regular.
let bitmap: RgbaBitmap = render_text(&face, "Hello, world!", 16.0, WHITE)?;

// Round-2: italic + weight via Style.
let italic = render_text_styled(&face, "Hello, world!", 16.0, WHITE, Style::italic())?;

// Word-wrap to a max width; one bitmap per output line.
let lines = render_text_wrapped(&face, "Some long subtitle text", 16.0, WHITE, 200.0)?;

// Lower-level: shape once, then compose into a destination you allocate.
let glyphs = Shaper::shape(&face, "AVATAR", 32.0)?;
let mut dst = RgbaBitmap::new(400, 80);
let mut composer = Composer::new();
composer.compose_run(&glyphs, &face, 32.0, WHITE, &mut dst, 0.0, face.ascent_px(32.0))?;

// Multi-face fallback chain.
let cjk_face = Face::from_ttf_bytes(std::fs::read("NotoCJK.otf")?)?;
let chain = FaceChain::new(face).push_fallback(cjk_face);
let glyphs = chain.shape("Hello 日本語", 16.0)?;

// Stroked subtitle text (\bord 2 + white fill on black border).
let stroke = StrokeStyle::new(2.0, [0, 0, 0, 255]);
composer.compose_run_with_stroke(
    &glyphs, &chain, 16.0, Style::REGULAR,
    [255, 255, 255, 255], Some(stroke),
    &mut dst, 0.0, 32.0,
)?;
```

## Out of scope (round 3+)

- **Bidi (UAX #9)** — left-to-right only; bidi resolution is round 3.
- **Arabic shaping** — joining types, connection tables, mandatory
  ligatures; round 3.
- **Indic conjunct formation** — reordering + half-form selection; round 3.
- **Variable fonts** — `fvar` / `gvar` / `MVAR`; round 3.
- **TrueType bytecode hinting** — modern AA at ≥ 16 px does not need it.
- **CFF / Type 2 charstrings** — `oxideav-otf` carries the cubic-Bezier
  outline pipeline.
- **Mark-to-base / mark-to-mark attachment** (GPOS types 4/5/6) —
  reserved by `PositionedGlyph::y_offset`; round 3.
- **Subpixel positioning** — round 3 once we wire up RGB / BGR LCD
  filtering.
- **Synthetic bold** — `Style.weight` is carried through the cache key
  but no synthesis pass runs yet; round 3 will dilate the alpha mask
  in proportion to the `(weight - 400)` delta.
- **True offset-curve stroke geometry** — current stroke uses
  alpha-mask dilation; round 3 may add geometric Minkowski-sum mode
  for cases where exact corners matter at large bord widths.

## Test fixture

Reuses `crates/oxideav-ttf/tests/fixtures/DejaVuSans.ttf` plus
`DejaVuSansMono.ttf` (Bitstream Vera license). The integration tests
check rasterised output dimensions, shaper glyph counts, kerning
shrinkage on `AVATAR`, the `fi` ligature on `office`, italic-shear
widening of upright glyphs, per-run colour preservation, font-fallback
routing, and stroke dilation. A two-script-coverage fallback test
(Latin primary + CJK fallback) is deferred until a small CJK fixture
lands in `oxideav-ttf` — Noto Sans CJK at 10 MB is too big to vendor
for one test.

## License

MIT — see [`LICENSE`](LICENSE).
