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

### Outlines and rendering

- **Outline access** — `Face::glyph_path(gid)` returns a Y-up
  `oxideav_core::Path` (`MoveTo` / `LineTo` / `QuadCurveTo` /
  `CubicCurveTo` / `Close`). TT outlines decode quadratics; CFF
  charstrings decode cubics 1:1. `Face::glyph_node(gid, size_px)` bakes
  the Y-flip + scale into a render-ready `Node::Path` (or `Node::Image`
  for CBDT colour glyphs).
- **PostScript glyph names** — `Face::glyph_name(gid)` (and the lower
  level `Face::post()` / `crate::post::PostTable`) resolve a glyph ID to
  its `post`-table PostScript name. The 258 standard Macintosh names are
  carried as `STANDARD_MAC_GLYPH_NAMES`; the parser handles `post`
  formats 1.0 (implied standard ordering), 2.0 (per-glyph standard +
  custom Pascal strings), and the deprecated 2.5 (signed delta into the
  standard set). Format 3.0 reports no names.
- **Vector text API** — `Shaper::shape_to_paths` returns one
  `(face_idx, Node, Transform2D)` per visible glyph. Each node is
  wrapped in an `oxideav_core::Group { cache_key: Some(_), .. }` so the
  downstream rasterizer's bitmap cache memoises the rendered glyph
  across renders, frames, and renderer instances.
- **Italic synthesis** — `style.italic` synthesises a 12° forward shear
  when the face is upright; falls back to the font's own slant when one
  is present. Bold synthesis is deferred to consumer code (or a real
  Bold face).
- **Face chain** — multi-face fallback for missing codepoints; per-glyph
  `face_idx` tells the consumer which face owns each glyph.
- **CBDT/CBLC colour bitmaps** — Noto Color Emoji and friends decode to
  `Node::Image` carrying a `VideoFrame`; resampling to the requested
  size happens in scribe (bilinear, straight-alpha).

### Shaping (GSUB / GPOS)

- **GSUB substitution** — LookupType 1 (single), 2 (multiple), 3
  (alternate), 4 (ligature), 5 (contextual), 6 (chained-contextual), and
  8 (reverse-chaining contextual single) are all applied through the
  caller-driven `Face::shape_text(text, features)` surface as well as
  through the auto-probe `Shaper::shape` / `FaceChain::shape` path. The
  contextual types dispatch each rule's nested `SequenceLookupRecord`
  sub-lookups; type 8 is processed right-to-left per the GSUB chapter's
  reverse-processing requirement. The required-feature `ccmp` runs as a
  pre-ligature pass and `calt` as a post-ligature pass for every Latin /
  Cyrillic / Greek / DFLT run. Worked examples:
  `face.shape_text("fi", &[*b"liga"])` returns a single fi-ligature
  glyph; `face.shape_text("Hi", &[*b"smcp"])` returns small-caps where
  the font ships them; a `frac`/`calt` contextual rule the caller
  requests now fires instead of passing through.
- **GPOS positioning** — single adjustment (type 1), pair kerning
  (type 2), cursive attachment (type 3, flag-clear semantics),
  mark-to-base (type 4), mark-to-ligature (type 5), mark-to-mark
  stacking (type 6), and contextual / chained-contextual positioning
  (types 7 / 8). The contextual pass runs last so its nested per-glyph
  adjustments layer on the post-kern / post-mark / post-cursive
  geometry; it accumulates the `PosRecord` deltas the GPOS lookup
  dispatches and is gated so plain Latin faces pay one lookup-list scan.
  Together this is enough for Latin / Cyrillic / Greek / basic CJK /
  Vietnamese / polytonic Greek.
- **Feature-tag introspection** — `Face::gsub_features_for_script` /
  `has_gsub_feature` report the feature tags the active face publishes
  under an OpenType script tag, for higher-level APIs that gate on
  feature presence. (A GPOS introspection mirror is a follow-up.)
- **Explicit-script + alternate-index shaping** —
  `shape_text_with_script` resolves features against one named script
  tag (no priority walk, avoiding cross-script collisions like `liga`
  under both `latn` and `arab`); `shape_text_with_alternates` /
  `shape_text_with_script_and_alternates` name the `AlternateSet` entry
  the Type-3 walker picks. The auto-probe `shape_text` walks a broad
  script-tag priority list (Latin / Cyrillic / Greek / DFLT / Arabic /
  Hebrew / Thai / Lao / the Indic v1+v2 scripts / Khmer / Myanmar /
  Hangul / Han / Kana).
- **Positioned caller-feature shaping** — `Face::position_text(text,
  size_px, features)` (and the explicit-script `position_text_with_script`
  mirror, plus `FaceChain::shape_with_features` for multi-face fallback)
  run the caller's requested optional/discretionary features (`smcp`,
  `frac`, `sups` / `subs`, `onum` / `lnum` / `pnum` / `tnum`, `zero`,
  stylistic sets, …) through GSUB substitution **and then** the full GPOS
  positioning pass, returning render-ready `PositionedGlyph`s with
  per-glyph advances and offsets. Previously the caller-feature surface
  stopped at GSUB and handed back bare glyph IDs; the substituted run now
  gets pair kerning, SinglePos, mark-to-base / mark-to-mark /
  mark-to-ligature attachment, cursive attachment, and contextual
  positioning. Ligature component counts are tracked through the Type-4
  collapse so mark-to-ligature attachment targets the right component.
- **Itemised mixed-script shaping** — `Face::position_text_itemized(text,
  size_px, features)` runs the `script` segmenter (below) over the input,
  resolves each run's Unicode script to the OpenType tag the font
  registers (`Face::resolve_ot_script_tag`: modern "v.2" tag preferred,
  legacy tag fallback, e.g. `dev2` on a modern font but `deva` on a
  legacy-only one), and positions each run under that tag through the
  GSUB-feature + GPOS pipeline, concatenating the per-run glyphs in
  logical order. A `"Hello हि"` string selects `latn` for the Latin run
  and the Devanagari tag for the second, so a feature published under one
  script does not leak into the other. For a single-script input the
  result is identical to `position_text_with_script` under the resolved
  tag.

### Complex-script shaping

- **Arabic contextual joining** — `shaping::arabic` picks `isol` /
  `init` / `medi` / `fina` per character via the joining-class state
  machine; `FaceChain::shape` rewrites Arabic letters into their
  Presentation Forms-B equivalents before cmap so cmap-only fonts
  render the correct contextual shapes (including LAM-ALEF ligatures).
- **Indic + Brahmic non-Indic shaping** — `shaping::indic` classifies
  Devanagari, Bengali, Tamil, Gurmukhi, Gujarati, Telugu, Kannada,
  Malayalam, Oriya, Sinhala, Khmer, Thai, Lao, and Myanmar / Burmese,
  segments runs into orthographic clusters, applies per-script pre-base
  matra reorder, identifies reph (or the Burmese kinzi), rewrites the
  leading RA to its reph form via `rphf`, and wires the
  cluster-position-aware GSUB features (`half`, `pref` / `blwf` /
  `abvf` / `pstf`, the presentation features `pres` / `psts` / `abvs` /
  `blws`, and the context-aware `locl` / `nukt` / `akhn` / `cjct` /
  `init` / `haln`). Per-script reorder rules are exposed as
  `DEVANAGARI_RULES` / `BENGALI_RULES` / … / `BURMESE_RULES` for callers
  reusing the cluster machine. Coverage misses pass through unchanged.

### Script itemisation

- **Unicode script → OpenType tag** — `script::ot_script_tag(s)` /
  `ot_script_tags(s)` map a Unicode `Script` (from the `intl` UCD
  tables) to its OpenType `ScriptList` tag(s). The Indic scripts that
  register both a legacy and a "v.2" shaping tag return the pair
  modern-first (`deva` → `[dev2, deva]`, `taml` → `[tml2, taml]`, …) so a
  shaper can prefer the v.2 form and fall back for older fonts. The
  tables are transcribed from the OpenType *Script Tags* registry
  (`docs/text/opentype/registries/script-tags.html`, CC-BY-4.0);
  `Common` / `Inherited` / `Unknown` resolve to the Default tag `DFLT`.
- **Script-run segmentation** — `script::script_runs(chars)` /
  `script_runs_str(text)` itemise a string into maximal same-script
  `ScriptRun`s (char-index ranges + resolved `Script`). `Inherited`
  combining marks always join the preceding run; `Common` punctuation /
  digits / spaces join the open run (and a leading `Common` span
  back-fills onto the first real script), so `"abc, def"` is one Latin
  run and `"123abc"` is one Latin run. The output is a gap-free
  partition. Full UAX #24 §5.1 bracket-pairing / `Script_Extensions`
  refinement is layered on later.
- **Font-aware itemised shaping** — `Face::resolve_ot_script_tag(script)`
  picks the tag the font actually registers (v.2 preferred, legacy
  fallback); `Face::script_run_tags(text)` pairs each `ScriptRun` with
  that resolved tag; `Face::shape_text_itemized(text, features)` (gids)
  and `Face::position_text_itemized(text, size_px, features)`
  (render-ready glyphs) shape a mixed-script string run-by-run under the
  resolved tags.

### Variable fonts

- **Outline interpolation** — `Face::set_variation_coords` /
  `variation_axes` / `named_instances` / `is_variable` surface the
  `fvar` declarations and let callers shape against a custom axis-coord
  vector. `Shaper::with_variation_coords(..)` is the per-call override
  path. Glyph outlines flow through the gvar interpolator so the emitted
  `Path` carries the blended deltas. CFF2 variable charstrings (the
  `blend` operator) are not yet emitted — scribe parses the CFF2 INDEX
  for table presence / axis count / glyph count via `Face::cff2()` only.
- **Metric-variation tables** — `Face::mvar()` / `metric_delta(tag)`
  (global metrics), `Face::hvar()` / `h_advance_delta(gid)` (horizontal
  advance), `Face::vvar()` / `v_advance_delta(gid)` (vertical), and
  `Face::stat()` / `stat_axes()` / `stat_axis_values()` (Style
  Attributes) all resolve at the current variation coords. They share an
  `ItemVariationStore` + `DeltaSetIndexMap` parser in `crate::variations`.
- **`name`-id resolution** — `Face::name_id(nid)` returns the
  highest-ranked Unicode string for a `name`-table id, resolving
  `axis_name_id` / `subfamily_name_id` / `value_name_id`.

### Layout and bidirectional text

- **Line layout** — line measurement + word-wrap. `layout::wrap_lines`
  breaks logical text to a pixel width; `layout::wrap_and_shape_lines(
  chain, text, size_px, max_width, base_level)` is the one-call path that
  wraps **and** shapes each produced line into a `ShapedVisualLine`
  (bidi-ordered, render-ready), one per display line top-to-bottom.
- **Document layout** — `layout::shape_paragraphs(chain, text, size_px,
  max_width, base_level)` is the multi-paragraph driver: it splits the
  text on UAX #9 bidi-class-`B` paragraph separators (LF, CR, CRLF, NEL
  `U+0085`, `U+2029`), resolves **each paragraph's own** base direction
  (P1 / P2 / P3, unless a uniform `base_level` is forced), and wraps +
  bidi-shapes every paragraph independently — returning one
  `ShapedParagraph { lines, base_level }` per source paragraph. (LINE
  SEPARATOR `U+2028` is class `WS`, a line break within a paragraph, so
  it does not start a new one.)
- **Bidi-shaped visual line** — `layout::shape_visual_line(chain, text,
  size_px, base_level) -> ShapedVisualLine` is the join between the UAX
  #9 reordering pipeline and the OpenType shaper. It partitions the line
  into bidi **level runs**, shapes each run's *logical* substring through
  the face chain (so ligatures / Arabic joining / Indic clustering see
  the natural character sequence), reverses each RTL run's glyph
  sequence, and concatenates the runs in §3.4 L2 **visual** order. The
  result is a `Vec<PositionedGlyph>` a renderer paints left-to-right with
  the pen advancing normally — correct mixed-direction layout without the
  caller hand-rolling the run arrangement. `ShapedVisualLine::width()`
  reports the laid-out advance.
- **High-level bidi bridge** — `layout::reorder_line_visual(text,
  base_level) -> VisualLine` drives the complete UAX #9 pipeline over
  one display line (class assignment → P → X → W → N0 → N1/N2 → I → L1
  → L2 → L3 → L4) and returns the characters in left-to-right visual
  order ready to feed glyph-by-glyph into the shaper. `VisualLine`
  publishes `visual: Vec<char>` (L4-mirrored, render order), the
  `logical_to_visual` / `visual_to_logical` permutation pair (the latter
  precomputed for O(1) cursor hit-testing), and the resolved
  `base_level`. `base_level: Option<u8>` is the HL1 override.
- **Whole-text / paragraph drivers** — `bidi::process_text(text,
  base_level) -> TextBidi` splits a document into paragraphs (P1) and
  resolves each independently; `bidi::process_paragraph(text,
  base_level)` and `process_paragraph_with_brackets(..)` (N0 wired in
  between W7 and N1) compose the per-rule passes into one
  `ParagraphBidi` carrier with `reorder_paragraph()` /
  `reorder_line_range(start..end)` helpers.
- **Per-rule UAX #9 surface** — the complete rule pipeline is also
  exposed as individual public functions for callers needing finer
  control: `bidi_class` (full `Bidi_Class` coverage via the `intl`
  crate's compiled UCD tables, plus the UAX #9 §3.2 unassigned-block
  defaults), `paragraph_level` / `split_paragraphs`
  (P1/P2/P3), `resolve_explicit_levels` (X1..X9 stack), `level_runs` /
  `isolating_run_sequences` (X10 BD7/BD13 partition + sos/eos),
  `resolve_weak_types` (W1..W7), `paired_bracket` / `bracket_pairs` /
  `resolve_bracket_pairs` (N0, full `BidiBrackets.txt`),
  `resolve_neutral_types` (N1/N2), `resolve_implicit_levels` (I1/I2),
  `reset_trailing_levels` / `reorder_line` (L1/L2),
  `reorder_combining_marks` (L3), and `mirrored_glyph` /
  `apply_mirroring` (L4, `Bidi_Mirroring_Glyph` via the `intl` crate).

## Out of scope

- **Pixel work** — bitmap rasterisation, alpha compositing, synthetic
  bold dilation, stroke dilation. All in
  [`oxideav-raster`](https://github.com/OxideAV/oxideav-raster).
- **Bidi HL1..HL6 higher-level-protocol overrides** — the rule pipeline
  itself is complete; HL overrides remain caller responsibility.
- **CFF2 variable charstrings** — the `blend` operator is not yet
  emitted (the INDEX walker is parsed for table metadata only).
- **TrueType bytecode hinting**, **subpixel LCD filtering**, and the
  **GPOS cursive attachment RIGHT_TO_LEFT flag-set variant** (needs
  lookup-flag exposure in `oxideav-ttf`'s public GPOS API) — deferred.

## Test fixtures

Reuses `crates/oxideav-ttf/tests/fixtures/DejaVuSans.ttf` plus
`DejaVuSansMono.ttf` (Bitstream Vera license),
`crates/oxideav-otf/tests/fixtures/SourceSans3-Regular.otf` (SIL OFL),
and a vendored copy of `InterVariable.ttf` (SIL OFL — see
`tests/fixtures/INTER-OFL-LICENSE.txt`) for the variable-font suite.
Network-gated emoji/CJK fixtures fetch on demand; see
`tests/font_fixtures/` and run with `OXIDEAV_NETWORK_TESTS=1`.

## License

MIT — see [`LICENSE`](LICENSE).
