# oxideav-scribe

Pure-Rust **vector** font shaper + line layout for the
[oxideav](https://github.com/OxideAV) framework. Parses TTF / OTF
tables (via [`oxideav-ttf`](https://github.com/OxideAV/oxideav-ttf)
+ [`oxideav-otf`](https://github.com/OxideAV/oxideav-otf)) and emits
positioned glyphs as [`oxideav-core`](https://github.com/OxideAV/oxideav-core)
`Node` vectors ready for the rasterizer in
[`oxideav-raster`](https://github.com/OxideAV/oxideav-raster).

Scribe contains **no pixel kernel**: outline flattening, scanline AA,
alpha compositing, synthetic bold and stroke dilation all live in
`oxideav-raster`. Producing a rasterised text run is a two-step pipeline:

```rust
use oxideav_core::{Group, Node, VectorFrame};
use oxideav_raster::Renderer;
use oxideav_scribe::{Face, FaceChain, Shaper};

let bytes  = std::fs::read("DejaVuSans.ttf")?;
let face   = Face::from_ttf_bytes(bytes)?;
let chain  = FaceChain::new(face);

// 1. Shape: emit positioned vector glyph nodes.
let placed = Shaper::shape_to_paths(&chain, "Hello, world!", 16.0);

// 2. Wrap into a VectorFrame + render via oxideav-raster.
let mut root = Group::default();
for (_face_idx, glyph_node, transform) in placed {
    root.children.push(Node::Group(Group {
        transform,
        children: vec![glyph_node],
        ..Group::default()
    }));
}
let mut frame = VectorFrame::new(400.0, 80.0);
frame.root = root;
let rgba: oxideav_core::VideoFrame = Renderer::new(400, 80).render(&frame);
```

## Capabilities

- **Outline access** — `Face::glyph_path(gid)` returns a Y-up
  `oxideav_core::Path` (raw `MoveTo` / `LineTo` / `QuadCurveTo` /
  `CubicCurveTo` / `Close`). TT outlines decode quadratics; CFF
  charstrings decode cubics 1:1. `Face::glyph_node(gid, size_px)`
  bakes the Y-flip + scale into a render-ready `Node::Path` (or
  `Node::Image` for CBDT colour glyphs).
- **Shaper** — `cmap` + GSUB type 4 (ligatures) + GPOS type 2 (pair
  kerning) + GPOS type 4/5/6 (mark-to-base, mark-to-mark stacking),
  enough for Latin / Cyrillic / Greek / basic CJK / Vietnamese /
  polytonic Greek.
- **Arabic contextual joining (round 7)** — `shaping::arabic`
  picks `isol` / `init` / `medi` / `fina` per character via the
  Unicode joining-class state machine; `FaceChain::shape` rewrites
  Arabic letters into their Presentation Forms-B equivalents
  (U+FE70..U+FEFF) before cmap so cmap-only fonts render the
  visually-correct contextual shapes (including LAM-ALEF ligatures
  via the existing GSUB pass).
- **Indic complex-script shaping (rounds 8 + 10 + 11)** — `shaping::indic`
  classifies Devanagari (U+0900..U+097F), Bengali (U+0980..U+09FF),
  Tamil (U+0B80..U+0BFF), Gurmukhi (U+0A00..U+0A7F),
  Gujarati (U+0A80..U+0AFF), Telugu (U+0C00..U+0C7F),
  Kannada (U+0C80..U+0CFF), Malayalam (U+0D00..U+0D7F), and
  Oriya (U+0B00..U+0B7F) codepoints, segments runs into
  orthographic clusters, applies per-script pre-base matra reorder,
  and identifies reph where applicable (Tamil + Malayalam are
  reph-disabled — Tamil RA does not form a reph; modern Malayalam
  uses chillu independent half-forms instead). When the active face
  publishes a `rphf` GSUB lookup for the active script, the leading
  RA glyph is rewritten to its reph form via
  `Font::gsub_apply_lookup_type_1` and the halant glyph is dropped
  (round 10). Round 11 also wires cluster-position-aware GSUB
  features: `half` for non-final consonants in conjuncts;
  `pref` / `blwf` / `abvf` / `pstf` (cascaded — first that returns a
  substitute wins) for post-halant consonants; and the presentation-
  pass features `pres` / `psts` / `abvs` / `blws` over every glyph
  in the cluster. Coverage misses pass through unchanged so a font
  without a given lookup degrades gracefully. Per-script reorder
  rules are exposed as `DEVANAGARI_RULES` / `BENGALI_RULES` /
  `TAMIL_RULES` / `GURMUKHI_RULES` / `GUJARATI_RULES` /
  `TELUGU_RULES` / `KANNADA_RULES` / `MALAYALAM_RULES` /
  `ORIYA_RULES` for callers reusing the cluster machine.
- **Variable fonts (round 9)** — `Face::set_variation_coords` /
  `variation_axes` / `named_instances` / `is_variable` surface the
  font's `fvar` declarations and let callers shape against a custom
  axis-coord vector (e.g. `wght=600 / wdth=125` on Inter Variable).
  `Shaper::with_variation_coords(vec![..]).shape_to_paths(&mut chain,
  text, size_px)` is the per-call override path: it installs the
  coords on the primary face, runs the shape, then restores. Glyph
  outlines flow through `oxideav-ttf`'s gvar interpolator so the
  emitted `Path` carries the blended deltas. CFF2 / OTF variable
  fonts are deferred until `oxideav-otf` exposes a CFF2 variation
  pipeline.
- **Vector text API** — `Shaper::shape_to_paths` returns one
  `(face_idx, Node, Transform2D)` per visible glyph. Each node is
  wrapped in an `oxideav_core::Group { cache_key: Some(_), .. }` so the
  downstream rasterizer's bitmap cache memoises the rendered glyph
  across renders, frames, and renderer instances.
- **Italic synthesis** — `style.italic` synthesises a 12° forward
  shear when the face is upright; falls back to the font's own slant
  when one is present. Bold synthesis is deferred to consumer code (or
  a real Bold face).
- **Face chain** — multi-face fallback for missing codepoints; per-glyph
  `face_idx` tells the consumer which face owns each glyph.
- **CBDT/CBLC colour bitmaps** — Noto Color Emoji and friends decode to
  `Node::Image` carrying a `VideoFrame`; the resampling to the requested
  size happens in scribe (bilinear, straight-alpha).
- **Layout** — line measurement + word-wrap (no bidi; round-3 work).

## Out of scope

- **Pixel work** — bitmap rasterisation, alpha compositing, synthetic
  bold dilation, stroke dilation. All in
  [`oxideav-raster`](https://github.com/OxideAV/oxideav-raster).
- **Bidi (UAX #9)**, **Sinhala / Burmese / Khmer / Thai / Lao**
  (Brahmic but with stack-form / split-vowel rules outside the
  Indic2 cluster machine), **variable-font metrics**
  (`MVAR` / `HVAR` / `VVAR` / `STAT`), **CFF2 variable fonts**,
  **TrueType bytecode hinting**, **subpixel LCD filtering**,
  **GPOS cursive attachment** — deferred.

## Test fixtures

Reuses `crates/oxideav-ttf/tests/fixtures/DejaVuSans.ttf` plus
`DejaVuSansMono.ttf` (Bitstream Vera license),
`crates/oxideav-otf/tests/fixtures/SourceSans3-Regular.otf` (SIL OFL),
and a vendored copy of `InterVariable.ttf` (SIL OFL — see
`tests/fixtures/INTER-OFL-LICENSE.txt`) for the round-9 variable-font
suite. Network-gated emoji/CJK fixtures fetch on demand; see
`tests/font_fixtures/` and run with `OXIDEAV_NETWORK_TESTS=1`.

## License

MIT — see [`LICENSE`](LICENSE).
