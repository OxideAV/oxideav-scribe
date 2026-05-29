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
- **GSUB feature-tag introspection (round 88)** — `Face::gsub_features_for_script(script_tag, lang_tag)`
  returns the four-byte feature tags the active face publishes under
  an OpenType script tag (in declaration order, required-feature
  first); `Face::has_gsub_feature(script_tag, feature_tag)` is the
  one-shot predicate variant. Useful for higher-level shaping APIs
  that gate behaviour on feature presence (e.g. enabling `smcp`
  small-caps only when the font ships it). Pure pass-through over
  `oxideav-ttf`'s GSUB walker; OTF / GSUB-less faces return an empty
  vec. A GPOS introspection mirror is a known follow-up — GPOS-only
  features like `kern` are not visible through this accessor.
- **Explicit-script-tag caller-driven shaping (round 175)** —
  `Face::shape_text_with_script(text, script_tag, features) -> Vec<u16>`
  is the deterministic-resolution mirror of `Face::shape_text`: every
  requested feature is resolved against the explicit `script_tag` alone
  (no priority walk), so callers that already know the script of the
  run get a one-tag resolution without the cross-script collision risk
  the auto-probe walk has when two scripts publish the same feature
  tag (e.g. `liga` under both `latn` and `arab`). Two concrete uses: an
  Arabic shaper forcing `liga` / `dlig` against `arab`, and a CJK
  pipeline forcing `vert` / `vrt2` against `hani` / `kana` / `hang`. An
  unknown `script_tag` yields cmap-identity. The companion auto-probe
  surface `Face::shape_text` now walks a broadened script-tag priority
  list — `latn` → `cyrl` → `grek` → `DFLT` → `arab` → `hebr` → `thai`
  / `lao ` → Indic v1+v2 (`deva` / `dev2`, `beng` / `bng2`, `taml` /
  `tml2`, `gujr` / `gjr2`, `guru` / `gur2`, `knda` / `knd2`, `mlym`
  / `mlm2`, `orya` / `ory2`, `telu` / `tel2`, `sinh`) → `khmr` →
  `mymr` / `mym2` → `hang` / `hani` / `kana` — so non-Latin runs reach
  GSUB through the caller-driven surface for the first time. The
  round-15 four-tag prefix is preserved verbatim so existing Latin /
  Cyrillic / Greek / DFLT callers see no behaviour change.
- **Caller-driven GSUB LookupType 1 + 2 + 3 + 4 application (rounds
  89 + 125 + 128 + 156)** — `Face::shape_text(text, features) -> Vec<u16>`
  cmap's the text, then applies every **single-substitution** (Type
  1), **multiple-substitution** (Type 2), **alternate-substitution**
  (Type 3, default `alternateIndex = 0`), and **ligature-substitution**
  (Type 4) lookup the requested feature tags reference under `latn` /
  `cyrl` / `grek` / `DFLT`. OpenType §6.2.1 Format 1 (delta) + Format
  2 (substitute-array), §6.2.2 Format 1 (Sequence-record splice,
  including the spec-legal `glyphCount = 0` deletion form), §6.2.3
  Format 1 (AlternateSet, first entry picked), and §6.2.4 Format 1
  (LigatureSet / Ligature records, longest-match-first per the spec
  ordering rule) are dispatched through `oxideav-ttf`'s
  `gsub_apply_lookup_type_{1,2,3,4}`. A Type-2 hit may change the
  returned glyph count (split / delete); a Type-3 hit is
  length-preserving (one alternate per covered slot); a Type-4 hit
  *always* shortens the run (N components → 1 ligature). `ccmp`
  "split precomposed glyph → base + combining mark" rules express
  through `shape_text` as well as the always-on
  `shaping::general::apply_ccmp` pass; `liga` / `dlig` / `rlig`
  collapse the standard / discretionary / required ligatures (e.g.
  fi / fl / ffi / ffl on DejaVu Sans) end-to-end on the caller-
  driven surface; `aalt` (Access All Alternates) now reshapes the
  slots its Type-3 lookup covers (e.g. the lowercase 'a' single-storey
  alternate on Inter Variable, the small set of `aalt` entries on
  DejaVu Sans). Useful for the display-toggled catalogue the round-15
  `ccmp` / `calt` passes don't reach: `smcp` / `c2sc`, `case`,
  `salt`, `aalt`, `frac`, `sups` / `subs` / `numr` / `dnom` /
  `ordn`, `ss01..ss20`, `cv01..cv99`, `zero`, `pnum` / `tnum`, plus
  `liga` / `dlig` / `rlig` as of round 128. Features are applied in
  caller order. Lookups of other declared types (5 / 6 / 8)
  referenced by the requested features are silently skipped —
  contextual / chained-contextual / reverse-chained substitutions
  still flow through `Shaper::shape` / `FaceChain::shape`. Worked
  examples: on Inter Variable, `face.shape_text("Hi", &[*b"smcp"])`
  returns `[cmap("H"), smcp(cmap("i"))]`; on DejaVu Sans,
  `face.shape_text("fi", &[*b"liga"])` returns a single
  ligature-glyph id (the fi-ligature) instead of the 2-glyph cmap
  output; on Inter Variable, `face.shape_text("a", &[*b"aalt"])`
  reshapes via the Type-3 alternate-0 (different glyph id from
  `cmap('a')`, length still 1) where previously the Type-3 lookup
  was silently skipped.
- **General-script GSUB features (round 15)** — `shaping::general`
  wires the OpenType **required-feature** `ccmp` (Glyph Composition /
  Decomposition) as a pre-ligature pass and `calt` (Contextual
  Alternates) as a post-ligature pass into `shape_run_with_font` for
  every Latin / Cyrillic / Greek / DFLT run. Lookups are dispatched
  per their declared GSUB type — types **1 / 2 / 3 / 4 / 5 / 6 / 8**
  are all routed via the appropriate `Font::gsub_apply_lookup_type_N`
  entry points (previously only type 4 ligatures were touched on Latin
  runs, and types 1 / 5 / 6 only via the per-script Indic / Arabic
  dispatchers). Concrete win against DejaVu Sans: `chain.shape("i\u{0307}")`
  now substitutes the dotless-i variant before the combining-above
  mark (matching the font's published `ccmp` rule). Coverage tables
  decide per-glyph whether each lookup fires — fonts without a `ccmp`
  / `calt` feature, or runs whose glyphs aren't in the lookup's
  coverage, are a no-op.
- **Arabic contextual joining (round 7)** — `shaping::arabic`
  picks `isol` / `init` / `medi` / `fina` per character via the
  Unicode joining-class state machine; `FaceChain::shape` rewrites
  Arabic letters into their Presentation Forms-B equivalents
  (U+FE70..U+FEFF) before cmap so cmap-only fonts render the
  visually-correct contextual shapes (including LAM-ALEF ligatures
  via the existing GSUB pass).
- **Indic + Brahmic non-Indic complex-script shaping (rounds 8 + 10 +
  11 + 12 + 13)** — `shaping::indic` classifies Devanagari
  (U+0900..U+097F), Bengali (U+0980..U+09FF), Tamil (U+0B80..U+0BFF),
  Gurmukhi (U+0A00..U+0A7F), Gujarati (U+0A80..U+0AFF),
  Telugu (U+0C00..U+0C7F), Kannada (U+0C80..U+0CFF),
  Malayalam (U+0D00..U+0D7F), Oriya (U+0B00..U+0B7F),
  Sinhala (U+0D80..U+0DFF), Khmer (U+1780..U+17FF),
  Thai (U+0E00..U+0E7F), Lao (U+0E80..U+0EFF), and
  Myanmar / Burmese (U+1000..U+109F) codepoints, segments runs into
  orthographic clusters, applies per-script pre-base matra reorder,
  and identifies reph where applicable (Tamil + Malayalam + Sinhala +
  Khmer + Thai + Lao are reph-disabled; Burmese identifies a kinzi
  (NGA + Asat + Virama + Cons) instead via `RephKind::BurmeseKinzi`
  on `BURMESE_RULES`). When the active face publishes a `rphf` GSUB
  lookup for the active script, the leading RA glyph is rewritten to
  its reph form via `Font::gsub_apply_lookup_type_1` and the halant
  glyph is dropped (round 10). Round 11 wires cluster-position-aware
  GSUB features: `half` for non-final consonants in conjuncts;
  `pref` / `blwf` / `abvf` / `pstf` (cascaded — first that returns a
  substitute wins) for post-halant consonants; and the presentation-
  pass features `pres` / `psts` / `abvs` / `blws` over every glyph
  in the cluster. Round 12 generalises the cluster machine over Khmer
  (where U+17D2 COENG plays the halant role and stacks subjoined
  consonants 2-3 deep in Pali borrowings), Sinhala (Indic-shaped halant
  with U+0DCA AL-LAKUNA), and Thai (no halant — pre-base vowels
  U+0E40..U+0E44 already in storage order). Round 13 adds Lao
  (structural twin of Thai) and Burmese (Asat U+103A killer + Virama
  U+1039 halant + medials U+103B..U+103E + pre-base sign-e U+1031 +
  kinzi reph-equivalent), plus a multi-glyph **context-aware** GSUB
  pass (`apply_cluster_context_substitutions`) that dispatches `locl` /
  `nukt` / `akhn` / `cjct` / `init` / `haln` via
  `Font::gsub_apply_lookup_type_5` (Contextual) +
  `gsub_apply_lookup_type_6` (Chained Context) at every position in
  every cluster. Coverage misses pass through unchanged so a font
  without a given lookup degrades gracefully. Per-script reorder rules
  are exposed as `DEVANAGARI_RULES` / `BENGALI_RULES` / `TAMIL_RULES` /
  `GURMUKHI_RULES` / `GUJARATI_RULES` / `TELUGU_RULES` /
  `KANNADA_RULES` / `MALAYALAM_RULES` / `ORIYA_RULES` /
  `SINHALA_RULES` / `KHMER_RULES` / `THAI_RULES` / `LAO_RULES` /
  `BURMESE_RULES` for callers reusing the cluster machine.
- **Variable fonts (round 9)** — `Face::set_variation_coords` /
  `variation_axes` / `named_instances` / `is_variable` surface the
  font's `fvar` declarations and let callers shape against a custom
  axis-coord vector (e.g. `wght=600 / wdth=125` on Inter Variable).
  `Shaper::with_variation_coords(vec![..]).shape_to_paths(&mut chain,
  text, size_px)` is the per-call override path: it installs the
  coords on the primary face, runs the shape, then restores. Glyph
  outlines flow through `oxideav-ttf`'s gvar interpolator so the
  emitted `Path` carries the blended deltas. CFF2 charstring
  evaluation (TN5177 v3 with the `blend` operator) is deferred —
  scribe parses the CFF2 INDEX walker for table presence + axis
  count + glyph count via `Face::cff2()`, but doesn't yet emit
  variation-blended cubic outlines from CFF2 charstrings.
- **Variable-font metrics + style attributes (round 14 / #454)** —
  full surface for the four metric-variation tables. `Face::mvar()`
  parses the global metric-variation table; `metric_delta(b"hasc")`
  returns the ascender delta in font units at the current variation
  coords (similar accessors for `cpht` cap-height, `xhgt` x-height,
  `undo` underline offset, `unds` underline size, and the rest of
  the OpenType MVAR ValueTag set). `Face::hvar()` /
  `h_advance_delta(gid)` exposes per-glyph horizontal-advance
  variations; `Face::vvar()` / `v_advance_delta(gid)` is the
  vertical mirror (returns `None` / `0.0` for the horizontal-only
  fonts that omit VVAR). `Face::stat()` / `stat_axes()` /
  `stat_axis_values()` parse the Style Attributes table — design
  axis labelling (`wght 400 → "Regular"`, `wght 700 → "Bold"`)
  surfaced as metadata for downstream callers. All four tables
  share an `ItemVariationStore` parser + a `DeltaSetIndexMap`
  parser — both live in `crate::variations`.
- **`name`-id resolution (round 14)** — `Face::name_id(nid)`
  returns the highest-ranked Unicode string for a `name`-table
  id, with the same priority the underlying TTF parser uses
  (Windows English first, Mac Roman English second, anything
  Unicode-y after, then any remaining record). Resolves
  `axis_name_id` / `subfamily_name_id` (from `fvar`) and
  `value_name_id` (from STAT) without forcing callers to reach
  into `Face::with_font`.
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
- **Bidi (UAX #9)**, **CFF2 variable charstrings**
  (the `blend` operator in TN5177 v3 — scribe parses the CFF2
  INDEX walker, but doesn't yet emit variation-blended cubic
  outlines), **TrueType bytecode hinting**, **subpixel LCD
  filtering**, **GPOS cursive attachment** — deferred.

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
