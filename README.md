# oxideav-scribe

Pure-Rust font rasterizer + simple shaper + line layout for the
[oxideav](https://github.com/OxideAV) framework. Sits on top of
[`oxideav-ttf`](https://github.com/OxideAV/oxideav-ttf) (outlines,
GSUB/GPOS lookups) and [`oxideav-pixfmt`](https://github.com/OxideAV/oxideav-pixfmt)
(Porter-Duff alpha compositing) to turn a UTF-8 string + a face into an
RGBA bitmap suitable for a subtitle track or a scene compositor.

## Round-1 scope (this release)

- **Outline flattening** — quadratic-Bezier subdivision via the
  classic de Casteljau split (chord tolerance 0.5 px). Implements the
  TrueType implicit-on-curve rule for adjacent off-curve handles, plus
  the Apple "all-off-curve" synthetic-start fallback.
- **Scanline rasterisation** — active-edge-list fill with 4× vertical
  supersampling for anti-aliasing. Even-odd fill rule.
- **Shaper** — `cmap` mapping with `.notdef` fallback for unmapped
  codepoints (basic CJK works because the codepoint walk is the same;
  whether actual glyphs are present depends on the loaded font).
  Ligature substitution via GSUB type 4. Pair kerning via GPOS type 2
  with legacy `kern` table fallback.
- **Composer** — Porter-Duff "over" via
  `oxideav_pixfmt::blit_alpha_mask` with straight-alpha destinations.
- **Layout** — line measurement + word-wrap. Tokenises on whitespace
  and falls back to character-level breaks if a single word overflows.
- **LRU cache** — glyph bitmap reuse keyed by
  `(face_id, glyph_id, size_q8)`; default capacity 256 covers a
  typical subtitle session at >95% hit rate.

## Public API

```rust
use oxideav_scribe::{render_text, render_text_wrapped, Face, Shaper, Composer, RgbaBitmap, WHITE};

let bytes = std::fs::read("DejaVuSans.ttf")?;
let face  = Face::from_ttf_bytes(bytes)?;

// One-call convenience: shape + raster + compose, autosized to the run.
let bitmap: RgbaBitmap = render_text(&face, "Hello, world!", 16.0, WHITE)?;
let _ = bitmap.width;
let _ = bitmap.data;       // straight-alpha RGBA8

// Word-wrap to a max width; one bitmap per output line.
let lines = render_text_wrapped(&face, "Some long subtitle text", 16.0, WHITE, 200.0)?;

// Lower-level: shape once, then compose into a destination you allocate.
let glyphs = Shaper::shape(&face, "AVATAR", 32.0)?;
let mut dst = RgbaBitmap::new(400, 80);
let mut composer = Composer::new();
composer.compose_run(&glyphs, &face, 32.0, WHITE, &mut dst, 0.0, face.ascent_px(32.0))?;
```

## Out of scope (round 2+)

- **Bidi (UAX #9)** — left-to-right only; bidi resolution is round 3.
- **Arabic shaping** — joining types, connection tables, mandatory
  ligatures; round 3.
- **Indic conjunct formation** — reordering + half-form selection; round 3.
- **Variable fonts** — `fvar` / `gvar` / `MVAR`; round 3.
- **TrueType bytecode hinting** — modern AA at ≥ 16 px does not need it.
- **CFF / Type 2 charstrings** — cubic Beziers from OTF will live in a
  sibling `oxideav-otf` crate (round 2).
- **Mark-to-base / mark-to-mark attachment** (GPOS types 4/5/6) —
  reserved by `PositionedGlyph::y_offset`; round 3.
- **Subpixel positioning** — round 2 once we wire up RGB / BGR LCD
  filtering.

## Test fixture

Reuses `crates/oxideav-ttf/tests/fixtures/DejaVuSans.ttf` (Bitstream Vera
license). The integration tests check rasterised output dimensions,
shaper glyph counts, kerning shrinkage on `AVATAR`, and the `fi`
ligature on `office`.

## License

MIT — see [`LICENSE`](LICENSE).
