# oxideav-scribe

[![CI](https://github.com/OxideAV/oxideav-scribe/actions/workflows/ci.yml/badge.svg)](https://github.com/OxideAV/oxideav-scribe/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/oxideav-scribe.svg)](https://crates.io/crates/oxideav-scribe) [![docs.rs](https://docs.rs/oxideav-scribe/badge.svg)](https://docs.rs/oxideav-scribe) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

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
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Capabilities

### Outlines and rendering

- **Outline access** — `Face::glyph_path(gid)` returns a Y-up
  `oxideav_core::Path` (`MoveTo` / `LineTo` / `QuadCurveTo` /
  `CubicCurveTo` / `Close`). TT outlines decode quadratics; CFF
  charstrings decode cubics 1:1. `Face::glyph_node(gid, size_px)` bakes
  the Y-flip + scale into a render-ready `Node::Path` (or `Node::Image`
  for CBDT colour glyphs).
- **Parse-once faces** — a `Face` parses its font exactly once, at
  construction, and keeps the parsed view next to the bytes for its
  whole life. Every per-glyph entry point (`glyph_path`, `glyph_node`,
  the colour-bitmap queries, advance / metric deltas, `glyph_name`,
  feature queries, shaping) borrows that view; `with_font` /
  `with_otf_font` are borrows, not parses, so calling them per glyph is
  fine. Installing a variation instance adjusts the view in place. The
  `post` / `MVAR` / `HVAR` / `VVAR` / `STAT` / `name` snapshots are
  parsed on first use and kept. `Face` and `FaceChain` are
  `Send + Sync`.
- **Outline cache (optional)** — attach a shared, bounded
  `OutlineCache` with `Face::set_outline_cache` /
  `FaceChain::set_outline_cache` and each *(face, glyph id, variation
  instance)* is decoded once; drawing it again at any size or position
  scales the kept font-unit path. `Face::glyph_path_shared` hands out
  the cached `Arc<Path>` without copying. The same cache keeps decoded
  colour-bitmap strikes, so an emoji's PNG is inflated once and only
  resampled per draw. Off by default; output is identical with and
  without it. See [Performance](#performance).
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
- **Face chain** — multi-face fallback decided per grapheme cluster
  (a base and its marks, an emoji sequence, a conjunct never split
  across faces); per-glyph `face_idx` tells the consumer which face
  owns each glyph. See [Shaping](#shaping).
- **COLR / CPAL layered colour glyphs** — a `COLR` version 0 colour
  glyph comes back from `Face::glyph_node` as a `Node::Group` of one
  filled path per layer (bottom layer first), coloured from the default
  `CPAL` palette with black standing in for the text foreground colour.
  `Face::color_glyph_node(gid, size_px, palette, foreground)` picks the
  palette and foreground; `Face::color_glyph_layers` /
  `has_color_layers` / `color_palette_count` expose the resolved layer
  list. Version 1 paint graphs are not rendered (see Out of scope).
- **CBDT/CBLC colour bitmaps** — Noto Color Emoji and friends decode to
  `Node::Image` carrying a `VideoFrame`; resampling to the requested
  size happens in scribe (bilinear, straight-alpha).

### Shaping

`FaceChain::shape` / `Shaper::shape` turn text into positioned glyphs
(`PositionedGlyph`: glyph id, face, x / y offset, x / y advance, and
the byte offset of the glyph's cluster in the text). Per call:

1. **Itemise** — script runs by UAX #24 (adjacent scripts that share
   an OpenType script, like Hiragana and Katakana, form one run).
2. **Fall back** — each grapheme cluster (UAX #29) goes to the first
   face of the chain that draws all of it. A cluster with the emoji
   presentation selector U+FE0F prefers a colour face
   (`Face::is_color_font`), one with the text selector U+FE0E a
   monochrome face.
3. **Normalise per font** — base + marks become the precomposed
   character when the font has it (`e` + U+0301 → `é`), a precomposed
   character the font lacks becomes base + marks, Hangul jamo compose
   to syllables; marks are put in canonical order (with the Hebrew
   letter points and the Arabic shadda kept next to their letter, so
   the font's composition and mark-stacking lookups match);
   right-to-left runs get mirrored brackets; a variation selector picks
   the `cmap` variation glyph.
4. **Substitute and position** — the script's default `GSUB` / `GPOS`
   features under its (or the caller's) language system, in the font's
   LookupList order, with `lookupFlag` filtering; advances at the
   installed variation instance (`HVAR`), kerning and mark anchors with
   their variation deltas; the legacy `kern` table when the font's
   `GPOS` has no `kern` feature for the script.

| Scripts | What is applied |
|---------|-----------------|
| Latin, Cyrillic, Greek, Armenian, Georgian and other "standard" scripts | `ccmp`, `locl`, `rlig`, `liga`, `clig`, `calt`, `rclt`, `kern`, `mark`, `mkmk`, `dist`, `curs`, `abvm`, `blwm`; marks attach to the ligature component they follow |
| Arabic, Syriac, N'Ko, Mongolian and other cursive scripts | joining analysis from `Joining_Type` (marks transparent, ZWJ / tatweel join-causing, ZWNJ blocking); `isol` / `fina` / `medi` / `init` each on the glyphs of that form, then `rlig`, `calt`, `liga`, `mset`; `curs`, `kern`, `mark`, `mkmk`; fonts without form features fall back to their Arabic presentation-form glyphs |
| Hebrew | standard features; points reordered so `ccmp` letter + dagesh / shin-dot compositions match; presentation-form composition when the font cannot attach marks |
| Devanagari, Bengali, Gurmukhi, Gujarati, Oriya, Tamil, Telugu, Kannada, Malayalam | the Indic procedure of the OpenType script-development specifications: syllables, base consonant found from the font's own below-base / post-base / pre-base forms, matra and sign reordering by the script's class table, `locl` `nukt` `akhn` `rphf` `rkrf` `pref` `blwf` `abvf` `half` `pstf` `vatu` `cjct` one at a time on their part of the syllable, final reordering (pre-base matra, reph, pre-base-reordering consonant), `init` `pres` `abvs` `blws` `psts` `haln` `calt`, then `kern` `dist` `abvm` `blwm` |
| Thai, Lao | standard features (the scripts are stored in display order) |
| Hangul | jamo composed to syllables when the font has them; standard features |
| Han, Kana | standard features; `vert` alternates, vertical advances and origins in vertical runs; ideographic variation sequences |
| Emoji | ZWJ sequences, skin-tone modifiers, flags, keycaps and tag sequences through the font's `GSUB`; presentation selectors steer the colour / text face |
| Sinhala, Khmer, Myanmar | the character-level cluster model (`shaping::indic`): cluster segmentation, pre-base reordering, per-position single substitutions |

- **Options** — `FaceChain::shape_with_options(text, size, &ShapeOptions)`:

  ```rust
  use oxideav_scribe::ShapeOptions;
  let opts = ShapeOptions::new()
      .with_feature(*b"smcp", true)        // small capitals on
      .with_feature(*b"liga", false)       // standard ligatures off
      .with_feature_value(*b"salt", 2)     // second stylistic alternate
      .with_language(Some(*b"TRK "))       // Turkish language system
      .with_vertical(false);
  ```

  `with_vertical(true)` lays the run out top-to-bottom (`vert`
  substitution, `vmtx` / `VORG` advances and origins, `y_advance` set).
  `FaceChain::shape_with_features` applies *only* the named
  substitution features.
- **Direction** — `shape` returns glyphs in reading order.
  `FaceChain::shape_visual_run(text, size, &opts, rtl)` returns one
  directional run arranged for display (a right-to-left run leftmost
  glyph first, marks and kerning placed for that order);
  `layout::shape_visual_line` does the bidi analysis and calls it per
  level run.
- **Invisible characters** — default-ignorable characters (ZWJ, ZWNJ,
  soft hyphen, variation selectors without a variation glyph, …) come
  out as a zero-advance space glyph; a space character the font lacks
  (en / em / n-per-em / figure / punctuation / ideographic space) is
  drawn with the space glyph at its defined width.
- **CFF faces** shape through the same engine as TrueType ones.
- **Lookup primitives** — the caller-driven surface is unchanged:
  `Face::shape_text(text, features)` and friends apply named `GSUB`
  features to a glyph run through the per-lookup-type entry points
  (types 1–6 and 8), `Face::position_text*` adds a fixed `GPOS` pass
  (single / pair / cursive / mark-to-base / mark-to-ligature /
  mark-to-mark / contextual).
- **Feature-tag introspection** — `Face::gsub_features_for_script` /
  `has_gsub_feature` and their GPOS mirror `gpos_features_for_script` /
  `has_gpos_feature` report the substitution / positioning feature tags
  the active face publishes under an OpenType script tag, for
  higher-level APIs that gate on feature presence (is `kern` there? does
  the font ship `cpsp` capital-spacing?). `layout_features_for_script`
  returns the de-duplicated GSUB ∪ GPOS union so a "which features can I
  toggle for this script?" surface need not care which table realises a
  tag.
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

### Script machinery

- **Joining analysis** — `shaping::compute_forms` /
  `shaping::joining_class` (Arabic) and the engine's `Joining_Type`
  analysis choose `isol` / `init` / `medi` / `fina` per letter;
  `shaping::presentation_form` maps a letter and a form to its Arabic
  Presentation Forms code point for fonts without form lookups.
- **Character-level Indic cluster model** — `shaping::indic` classifies
  Devanagari, Bengali, Tamil, Gurmukhi, Gujarati, Telugu, Kannada,
  Malayalam, Oriya, Sinhala, Khmer, Thai, Lao and Myanmar characters,
  segments runs into orthographic clusters and applies a per-script
  pre-base reorder (`DEVANAGARI_RULES` / … / `BURMESE_RULES`,
  `reorder_cluster_with`). The shaper itself uses it for Sinhala, Khmer
  and Myanmar; the nine scripts listed above use the syllable-level
  procedure instead.
- **Universal cluster model** — `shaping::cluster` implements the
  script-agnostic combining-character-sequence cluster of UAX #24 §5.2
  (base + `Mn`/`Mc`/`Me` marks + ZWJ/ZWNJ join controls) for every
  script without a dedicated machine: `cluster_category`,
  `universal_cluster_boundaries` (half-open spans tiling the input),
  and `cluster_script` (the §5.2 first-real-script resolution).
  The syllable grammar of the Universal Shaping Engine model (for
  Balinese, Javanese, Cham, Tai Tham and the other scripts it covers)
  is not implemented: those scripts shape with the standard feature
  set, without reordering.

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
- **Script-run segmentation (full UAX #24 §5)** —
  `script::script_runs(chars)` / `script_runs_str(text)` itemise a
  string into maximal same-script `ScriptRun`s (char-index ranges +
  resolved `Script`), on top of the per-character resolver
  `script::resolve_scripts`. The resolution implements the complete
  UAX #24 §5 rule set: `Inherited` combining marks join the open run
  unconditionally (§5.2); `Common` punctuation / digits / spaces join
  the open run (a leading `Common` span back-fills onto the first real
  script), so `"abc, def"` and `"123abc"` are each one Latin run;
  characters with a **limited `Script_Extensions` set** continue a run
  only when the run's script is a member (§5.3 — `"アー"` is one
  Katakana run but `"abcー"` splits U+30FC out of Latin, and U+060C
  ARABIC COMMA joins the Arabic side), with consecutive constrained
  characters intersecting their sets; and **paired brackets** resolve
  the closing element to the same script as its opener — the enclosing
  text — via the vendored `BidiBrackets.txt` table (§5.1 —
  `"abc (Ψα) def"` keeps both parentheses in the Latin runs). The
  output is a gap-free partition.
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

- **Baseline metrics** — `Face::has_base_table` / `Face::baseline_coord(
  axis, script_tag, baseline_tag)` surface the `BASE` table's per-script
  design-unit baseline offsets (roman `romn`, ideographic `ideo`,
  hanging `hang`, `math`, …) so a layout engine can sit a Latin run and
  a CJK run on a common line. `BaselineAxis::{Horizontal, Vertical}`
  selects the Y (horizontal-layout) or X (vertical-layout) axis;
  variable TTF faces resolve the coordinate at the current instance
  through the `BASE` `ItemVariationStore`. `default_baseline_tag` /
  `default_baseline_coord` resolve the baseline a script declares as its
  own default (`BaseScript::defaultBaselineIndex`), so a caller can align
  to the font's intent without hard-coding `romn`. Works for both TTF and
  OTF/CFF faces; `None` when the table, axis, script, or baseline tag is
  absent.
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

## Shaping conformance

`tests/shaping_conformance.rs` shapes a corpus of strings per script
with every available font and compares the glyph ids, the absolute
position of every glyph (± 1.5 font units) and the run advance with an
expected run. For the committed fonts (DejaVu Sans, Inter Variable,
Source Sans 3, Noto Sans Arabic) the expectations are committed in
`tests/fixtures/shaping/expected.tsv`, so the comparison runs
everywhere. When the `hb-shape` executable is installed it is run as a
black-box reference as well, also over system fonts and the
network-gated fixtures (cases whose font is absent are skipped).

On the machine this was measured on — 325 runs over 17 fonts (the four
committed ones; Arial, Times New Roman, Tahoma, SF Arabic, SF Hebrew,
Kohinoor Devanagari, Kohinoor Bangla, Noto Sans Kannada, Arial Unicode
MS, Apple SD Gothic Neo, Hiragino Sans; Noto Sans CJK and Noto Color
Emoji from the fixture cache) — runs that agree with the reference:

| Script group (cases) | before | now |
|----------------------|-------:|----:|
| Latin, Cyrillic, Greek: ligatures, kerning (GPOS and `kern` table), precomposed / combining / stacked marks, marks on ligatures, ZWJ / ZWNJ, variation selectors, spaces, `smcp` `c2sc` `onum` `tnum` `frac` `sups` `zero` `dlig` `ss01` `case` `-liga` `-kern`, variation instances (134) | 61 | 133 |
| Arabic: joining forms, non-joiners, lam-alef, marks, shadda stacks, marks on lam-alef, tatweel, ZWJ / ZWNJ, Persian / Urdu letters, digits, punctuation (90) | 29 | 89 |
| Hebrew: letters, points, dagesh, final forms, presentation forms (24) | 18 | 24 |
| Devanagari: pre-base matra, conjuncts, reph, rakar, half forms, matras, signs, explicit halant with ZWJ / ZWNJ (22) | 5 | 19 |
| Bengali: pre-base and split matras, conjuncts, reph, ya-phala (5) | 1 | 5 |
| Kannada (3) | 0 | 2 |
| Thai: tone marks, below vowels, SARA AM, leading vowels (12) | 4 | 7 |
| Hangul: syllables, conjoining jamo L+V, L+V+T, syllable + T (15) | 6 | 15 |
| CJK: horizontal, kana, ideographic variation sequences, vertical with punctuation and Latin (12) | 0 | 12 |
| Emoji: presentation selectors, skin tones, ZWJ sequences, flags, keycaps, tag sequences (8) | 6 | 8 |
| **Total (325)** | **130** | **314** |

The 11 runs that differ are listed, with the reason, in
`KNOWN_DEVIATIONS` in the harness (a listed run that starts to agree
fails the test, so the list cannot go stale):

- **Mark-to-mark before mark-to-base** (DejaVu Sans, two marks on one
  base): the font lists its `mkmk` lookup before its `mark` lookups;
  applied literally in that order the two marks land on the same
  anchor and overlap. This crate applies mark-to-mark after
  mark-to-base (the documented feature order), so the second mark sits
  on the first.
- **Thai** (5 runs, Tahoma and Arial Unicode MS): SARA AM decomposition
  with the nikhahit reordered before a tone mark, and the selection of
  the legacy private-use glyph variants in fonts without Thai `GSUB`,
  are not specified in the staged documentation.
- **Older Indic model** (3 runs, Arial Unicode MS): the matra and reph
  conventions of fonts written for the pre-"2" script tags are not
  specified beyond one sentence.
- **Kannada vowel-sign ligature** (1 run): the font's `abvs` contextual
  lookup is not applied by the font parser.
- **Digits forced right-to-left** (1 run, SF Arabic): the reference
  tool does no bidi analysis; real text puts digits in left-to-right
  runs.

Fonts shaped by Apple Advanced Typography `morx` tables with no `GSUB`
(Thonburi, Apple Color Emoji) are outside OpenType Layout and are
skipped. `SCRIBE_SHAPING_REPORT=1` prints every run's verdict;
`SCRIBE_SHAPING_REGEN=1` rewrites the committed expectations from the
reference tool.

## Performance

A `Face` never re-parses its font. Before this was so, every
`glyph_node` call parsed the whole font (twice for a TrueType face —
once to ask about colour bitmaps, once for the outline), so a frame's
cost was the glyph count times the font's table-parsing time.

`cargo bench --bench hot_paths` (std-only timing, release, median of 9
batches, two runs, on a shared build machine; "before" is the previous
parse-per-call implementation built against the same released
`oxideav-ttf` 0.1.8 / `oxideav-otf` 0.1.5):

| case | before | now | now, outline cache |
|------|-------:|----:|-------------------:|
| UI frame, 486 × `glyph_node`, DejaVu Sans (5 800 glyphs) | 252.8 ms | 0.16 ms | 0.050 ms |
| UI frame, 486 × `glyph_node`, Inter Variable | 109.4 ms | 0.17 ms | 0.054 ms |
| UI frame, 486 × `glyph_node`, Source Sans 3 (CFF) | 1.14 ms | 0.34 ms | 0.047 ms |
| UI frame, 31 × `shape_to_paths`, DejaVu Sans | 179.0 ms | 0.34 ms | 0.24 ms |
| UI frame, 31 × `shape_to_paths`, Inter Variable | 108.5 ms | 0.72 ms | 0.67 ms |
| Latin paragraph (196 glyphs) draw, DejaVu Sans | 60.1 ms | 0.058 ms | 0.018 ms |
| Arabic paragraph (153 glyphs) draw, DejaVu Sans | 51.4 ms | 0.073 ms | 0.014 ms |
| Latin paragraph draw, Inter Variable `wght=700` | 42.9 ms | 0.18 ms | 0.023 ms |
| CJK line (68 glyphs) draw, DejaVu Sans + Noto Sans CJK | 9.32 ms | 0.17 ms | 0.017 ms |
| Colour emoji (16 glyphs) draw, Noto Color Emoji | 6.36 ms | 0.33 ms | 0.012 ms |
| Latin paragraph shape, DejaVu Sans | 0.203 ms | 0.058 ms | — |
| Latin paragraph shape, Inter Variable | 0.336 ms | 0.246 ms | — |

The "UI frame" rows are a text field plus thirty labelled buttons, one
call per glyph per frame: 520 µs → 0.33 µs per `glyph_node` on DejaVu
Sans (about 1 600×), 0.10 µs with the outline cache. What is left per
glyph is the outline walk itself; with a cache, a copy-and-scale of the
kept path (and, for colour emoji, a resample of the kept strike instead
of a PNG decode).

Shaping no longer rebuilds lookup or feature lists per call (a per-face
layout plan keeps them): thirty short labels through `FaceChain::shape`
take 152 µs instead of 204 µs on DejaVu Sans. What remains in shaping
is the per-glyph `GSUB` / `GPOS` lookup matching inside the font
parser — about 85 % of a shaping call in a sampling profile (ligature
lookup, chained-context matching, pair kerning, coverage search).

Face construction is unchanged (about 0.19 ms for DejaVu Sans, 0.11 ms
for Inter Variable with `oxideav-ttf` 0.1.8, which parses every table
up front); it is paid once per face.

### The outline cache

`OutlineCache::new(budget_bytes)` (or `OutlineCache::default()`, 4 MiB)
is a thread-safe memo of font-unit outlines shared by any number of
faces:

```rust
use oxideav_scribe::{Face, FaceChain, OutlineCache};
use std::sync::Arc;

# fn demo(face: Face) {
let cache = Arc::new(OutlineCache::default());
let mut chain = FaceChain::new(face);
chain.set_outline_cache(Some(cache.clone()));
// glyph_path / glyph_node / shape_to_paths now decode each glyph once.
println!("{:?}", cache.stats()); // hits, misses, entries, bytes
# }
```

- **Key** — the face's process-unique `Face::id()` (never reused), the
  glyph id, and the exact bit pattern of the installed variation
  coordinates. A hit can therefore never be another face's glyph or
  another instance's outline, and changing a face's variation
  coordinates needs no flush: the new instance misses, the old one's
  entries are found again if the face returns to it.
- **Colour strikes** — the decoded native-size RGBA of a colour-bitmap
  glyph is kept under the same budget, keyed by face id, glyph id and
  the position of the strike's embedded image in the font file;
  `glyph_node` / `raster_color_glyph_at` resample from it.
- **Memory bound** — entries are charged for their path commands (or
  RGBA bytes — about 70 KiB for a 136 × 128 strike) plus
  a fixed 96-byte allowance and kept in two generations of at most
  half the budget each; when the young one fills, the old one is
  dropped and entries still in use are carried forward. The charged
  total never exceeds the budget. An outline larger than half the
  budget is returned but not kept.
- **Invalidation** — none is needed for correctness. `clear()` empties
  the cache; `forget_face(id)` releases a dropped face's entries sooner
  than ageing would.
- **Threads** — `OutlineCache: Send + Sync`; the decode runs outside
  the internal lock.

## Out of scope

- **Pixel work** — bitmap rasterisation, alpha compositing, synthetic
  bold dilation, stroke dilation. All in
  [`oxideav-raster`](https://github.com/OxideAV/oxideav-raster).
- **Bidi HL1..HL6 higher-level-protocol overrides** — the rule pipeline
  itself is complete; HL overrides remain caller responsibility.
- **COLR version 1 paint graphs** — gradients, transforms and
  composites are not rendered; a glyph with a paint graph is drawn from
  its version 0 layer stack when the font also carries one, else as its
  plain outline.
- **CFF2 variable charstrings** — the `blend` operator is not yet
  emitted (the INDEX walker is parsed for table metadata only).
- **Apple Advanced Typography layout** (`morx` / `kerx`) — fonts that
  ship only AAT tables shape with `cmap` and `hmtx` alone.
- **Universal Shaping Engine scripts**, **Hangul jamo forms** (`ljmo` /
  `vjmo` / `tjmo` for sequences with no precomposed syllable) and
  **Thai SARA AM decomposition** — see
  [Shaping conformance](#shaping-conformance).
- **TrueType bytecode hinting** and **subpixel LCD filtering** —
  deferred.

## Test fixtures

`tests/fixtures/` holds `DejaVuSans.ttf`, `DejaVuSansMono.ttf` and
`DejaVuSansMono-Oblique.ttf` (Bitstream Vera license),
`SourceSans3-Regular.otf` (SIL OFL), `InterVariable.ttf` (SIL OFL —
see `INTER-OFL-LICENSE.txt`) and `NotoSansArabic-Regular.ttf` (SIL OFL
— see `NOTO-ARABIC-OFL-LICENSE.txt`), plus the shaping expectations in
`shaping/expected.tsv` (provenance: the `hb-shape` tool run on those
fonts). Network-gated emoji / CJK fixtures fetch on demand; see
`tests/font_fixtures/` and run with `OXIDEAV_NETWORK_TESTS=1`.

## License

MIT — see [`LICENSE`](LICENSE).
