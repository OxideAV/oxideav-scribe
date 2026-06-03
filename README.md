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
- **Caller-driven Type-3 alternate-index selection (round 183)** —
  `Face::shape_text_with_alternates(text, &[(feature_tag, alternate_index)])`
  and the explicit-script mirror
  `Face::shape_text_with_script_and_alternates(text, script_tag,
  &[(feature_tag, alternate_index)])` let the caller name the
  `AlternateSet` entry the Type-3 (Alternate Substitution) walker
  picks per feature, instead of the round-156 hardcoded `0`. Useful
  for `salt` / `aalt` / `ss01..ss20` features whose AlternateSets
  ship more than one entry — e.g. `face.shape_text_with_alternates(
  "Ag", &[(*b"salt", 2)])` asks for the third stylistic alternate
  for every `salt`-covered slot. Out-of-range indices fall back to
  cmap-identity per slot (the safe contract for callers that don't
  pre-probe per-font alternate counts; the underlying
  `oxideav-ttf` accessor returns `None` and we leave the slot
  unchanged). Length-preservation per OpenType §6.2.3 is invariant
  across indices. Non-Type-3 lookups (Type 1 / 2 / 4) dispatched by
  the same feature tag silently ignore the index — `liga`'s
  Type-4 ligature collapse still works regardless of which
  alternate-index payload is attached. Two paired entry points
  inherit the round-89 / round-175 auto-probe vs explicit-script
  split: the auto-probe variant walks the broadened script-tag
  priority list, the explicit-script variant resolves against one
  named script tag for deterministic single-script resolution. Index
  0 reproduces the round-156 default byte-for-byte — the round-183
  surface is a strict superset.
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
- **Layout** — line measurement + word-wrap. The bidi character
  pipeline (P → W → N → I → L1 → L2) is available on the public
  `bidi::` module as of round 210; the high-level `layout::*` API
  does not yet drive it automatically — callers wire the
  per-character permutation into their own renderer.
- **BiDi explicit-level / override / isolate stack X1..X9 (round 217)** —
  `bidi::resolve_explicit_levels(classes, paragraph_level) ->
  ExplicitLevels` runs the UAX #9 §3.3.2 explicit-level pass over
  the whole paragraph in one walk, maintaining the spec's
  *directional status stack* of (`level`, override-status,
  isolate-flag) frames plus the three overflow / valid counters
  (overflow_isolate, overflow_embedding, valid_isolate). Returns
  three parallel vectors of length `classes.len()`:
  `levels: Vec<u8>` (per-character embedding level: X6's stack-top
  level for regular characters, the new-scope level for embedding
  initiators, the enclosing scope's level for isolate initiators +
  their matching PDIs, and the paragraph embedding level for B
  characters per X8); `effective_classes: Vec<BidiClass>` (input
  classes with X4 / X5 / X5a / X5b / X6 / X6a override rewrites
  applied — `L` override → `L`, `R` override → `R`, neutral leaves
  classes unchanged); `removed: Vec<bool>` (the X9-removed flag set
  for RLE / LRE / RLO / LRO / PDF / BN; the four isolate-formatting
  characters LRI / RLI / FSI / PDI are *not* removed per the X9
  note). FSI is resolved per X5c by running a P2 / P3 mini-pass
  over the FSI..matching-PDI span and dispatching as RLI or LRI
  accordingly; the in-span P2 itself skips inner LRI..PDI regions
  the same way the top-level `paragraph_level` walker does.
  Overflow events (depth ≥ `MAX_DEPTH = 125`) are absorbed into
  the matching counter per the spec; matching PDFs / PDIs
  decrement their respective overflow counter without popping the
  stack. 19 unit + 23 integration tests cover X1 init, X2..X5
  embedding / override pushes with their least-greater-odd / -even
  level computation, X5a / X5b isolate pushes with their enclosing-
  level initiator reporting, X5c FSI in-span P2 dispatch (strong-L
  → LRI, strong-AL → RLI, no-strong → default LRI, nested-isolate
  skip), X6 override rewriting only non-formatting types, X6a
  unwind + isolate-pop, X7 PDF matching the inner embedding only
  inside an isolate, X8 B-at-paragraph-level, X9 removal flags for
  all six removed types + non-removal for the four isolates, the
  64-RLE-deep MAX_DEPTH = 125 pinning, plus mixed Latin / Hebrew
  paragraphs flowing through `paragraph_level →
  resolve_explicit_levels` end-to-end. The X10 isolating-run-
  sequence partition that consumes X1..X9's output landed in round
  220 (`level_runs` + `isolating_run_sequences`). The bracket-pair
  rule N0 (still blocked on `BidiBrackets.txt`) and the L3 / L4
  mirroring rules remain deferred.
- **BiDi X10 isolating-run-sequence partition (round 220)** —
  `bidi::level_runs(&levels) -> Vec<LevelRun>` returns the UAX #9
  §3.1.2 **BD7** partition of the per-character embedding-level
  vector produced by `resolve_explicit_levels`: a contiguous list
  of `LevelRun { start, end, level }` half-open ranges that fully
  cover `0..levels.len()`. `bidi::isolating_run_sequences(&classes,
  &explicit, paragraph_level) -> Vec<IsolatingRunSequence>` chains
  those level runs into the **BD13** isolating-run-sequence
  partition + derives the per-sequence **X10 step 2** `sos` / `eos`
  directional types. Each `IsolatingRunSequence` carries the
  constituent `runs` (in logical order), the shared `level` across
  every run, and `sos`/`eos` of class `L` or `R`. The chaining rule
  is literal BD13: at every level run whose last non-X9-removed
  character is an isolate initiator (`LRI` / `RLI` / `FSI`), if
  **BD9**'s matching-PDI scan finds a PDI **and** that PDI is the
  first character of its own level run, the next run is appended
  to the current sequence; otherwise the chain closes. Runs whose
  first character is a chained-from-prior PDI cannot seed new
  sequences. The sos / eos derivation faithfully follows X10 step
  2: "compare the level of the first / last character in the
  sequence with the level of the character preceding / following
  it in the paragraph (not counting characters removed by X9), and
  if there is none, with the paragraph embedding level"; the eos
  side also falls back to the paragraph level when the last
  character of the last run is an isolate initiator lacking a
  matching PDI. "If the higher level is odd, the sos or eos is R;
  otherwise, it is L." `IsolatingRunSequence::indices(&removed)`
  yields the constituent character indices in logical order while
  skipping X9-removed positions, ready to drive the in-place W / N
  / I passes per sequence per the X10 step 3 closing note "the
  order that one isolating run sequence is treated relative to
  another does not matter." 17 unit + 16 integration tests cover
  the BD7 partition (empty / uniform-level / multi-change / full
  coverage invariant), the X10 sos / eos derivation under
  paragraph-edge / RLE-bounded / unmatched-isolate / non-zero-base
  paragraph-level conditions, the BD13 chaining across LRI..PDI
  with verification that the matching PDI is the first character
  of its run, the BD9 matching-PDI scan with nested isolates and
  ignored embedding-formatting, the BD13 invariant that every
  level run belongs to exactly one sequence and that all runs in a
  sequence share their embedding level, the `indices` iterator's
  X9-skip behaviour and isolate-format-character preservation, and
  an end-to-end `X → W → N → I` compose driving the existing
  weak / neutral / implicit-level passes per-sequence. **N0
  (bracket-pair resolution per §3.1.3) and L3 / L4 (combining-mark
  reordering + bidi-mirroring per §3.4 / §4.7) remain deferred** —
  N0 blocked on `BidiBrackets.txt` (not yet vendored under
  `docs/text/unicode-bidi/`); L4 blocked on `BidiMirroring.txt`
  (same gap); L3 is conditional on the renderer's mark-attachment
  policy and does not yet fire under scribe's GPOS stacker.
- **BiDi foundation (round 186)** — `bidi::bidi_class(c)` returns the
  UAX #9 §3.2 normative bidirectional class for every code point
  scribe needs today (the 12 explicit-format controls in full, ASCII
  / Latin-1, Hebrew, four core Arabic blocks + Syriac + Thaana +
  N'Ko + the two Arabic Presentation Forms blocks, Combining
  Diacritical Marks + Arabic NSM ranges). `bidi::paragraph_level(s)`
  implements UAX #9 rules **P1 / P2 / P3** end-to-end: walks the
  text, skips the contents of any `LRI` / `RLI` / `FSI` ... `PDI`
  isolate region (nested arbitrarily deep), finds the first strong
  (`L` / `R` / `AL`) character, and returns the paragraph embedding
  level (`0` for LTR or first-strong-L, `1` for first-strong-R-or-AL,
  default `0`). `bidi::split_paragraphs(s)` is P1's split that keeps
  every type-`B` separator with the preceding paragraph (the
  returned slices concatenate back to `s` exactly).  21 unit +
  integration tests cover the explicit-format set, ASCII / Latin-1 /
  Hebrew / Arabic class assignments, isolate-skip with nested
  initiators, embedding initiators *not* skipping (only isolates do),
  and P3 default-when-no-strong-character.
- **BiDi weak-type resolution W1..W7 (round 191)** —
  `bidi::resolve_weak_types(&mut classes, sos, eos)` runs the UAX #9
  §3.3.4 weak-type pass over one isolating run sequence in place.
  `classes` is a mutable slice of `BidiClass` values (the per-
  character classification from `bidi_class`); `sos` / `eos` are the
  start- and end-of-sequence strong types (`L` or `R`, derived from
  the paragraph embedding level by the X1 stack frame in a future
  round — for callers that have not yet wired X1..X10, passing
  `BidiClass::L` for paragraph level 0 and `BidiClass::R` for level
  1 is correct for a single-paragraph no-isolate run).  Rules
  applied in order: **W1** NSM inherits the type of the preceding
  character (or `ON` when the preceding is `LRI` / `RLI` / `FSI` /
  `PDI`; consecutive NSMs all flip to the same type because the
  second NSM, after the first iteration's rewrite, sees the first
  one); **W2** `EN` whose most-recent strong type is `AL` becomes
  `AN`; **W3** every remaining `AL` becomes `R`; **W4** a single
  `ES` or `CS` between two `EN`s collapses to `EN`, a single `CS`
  between two `AN`s collapses to `AN`; **W5** runs of `ET` adjacent
  to an `EN` on either side collapse to `EN`; **W6** every leftover
  `ES` / `ET` / `CS` becomes `ON`; **W7** `EN` whose most-recent
  strong (`L` / `R` / `sos`) is `L` becomes `L`. After the call the
  slice contains no `AL` (W3 ate them) and no leftover `ES` / `ET` /
  `CS` (W6 ate them), so the N-rules can resolve neutrals against
  a clean weak-type vocabulary. 14 unit + 11 integration tests cover
  every rule's spec example (`AL EN → R AN`, `AL NI EN → R NI AN`,
  `EN ES EN → EN EN EN`, `AN CS AN → AN AN AN`, `EN CS AN → EN ON
  AN`, `ET ET EN → EN EN EN`, `AN ET EN → AN EN EN`, `L NI EN → L
  NI L`, `R NI EN → R NI EN`, …), the W1 isolate-initiator → ON
  variant, the W4 negative cases (two consecutive ES don't collapse,
  AN ES AN does not collapse because W4's ES branch is EN-only,
  mixed-type CS doesn't collapse), the W5 non-EN-adjacent case (ET
  next to AN stays ET → W6 → ON), the W2-before-W3 ordering, and a
  full Arabic phone-number-style pipeline. `BidiClass::is_neutral_or_isolate()`
  exposes the §3.3.5 NI alias predicate for the upcoming N-rules.
  N / I / X / L rules remain deferred.
- **BiDi line-level reordering L1 + L2 (round 210)** —
  `bidi::reset_trailing_levels(&orig_classes, &mut levels,
  paragraph_level)` runs the UAX #9 §3.4 rule **L1** in place,
  resetting the embedding level of (a) every segment separator
  `S`, (b) every paragraph separator `B`, (c) every trailing
  whitespace / isolate-formatting run preceding such a separator,
  and (d) every trailing whitespace / isolate-formatting run at
  the end of the line, back to `paragraph_level`. Per the §3.4
  normative note the lookup uses the **original** classes (the
  same slice the caller fed into the W / N / I passes), so the
  function takes the original class slice + the post-I-rules level
  vector as separate arguments. `bidi::reorder_line(&levels) ->
  Vec<usize>` runs rule **L2** and returns a logical-to-visual
  permutation of `0..n`: from the maximum embedding level down to
  the lowest odd level, every maximal contiguous run of positions
  at or above the iteration level reverses, building up the nested
  reversals UAX #9 §3.4 Examples 1..4 illustrate. The pair closes
  the per-character pipeline: a caller chains `bidi_class →
  resolve_weak_types → resolve_neutral_types →
  resolve_implicit_levels → reset_trailing_levels → reorder_line`
  and ends with the visual-order index sequence the line renderer
  walks. 17 unit + 21 integration tests cover every L1 sub-case
  with positive + negative controls, the four §3.4 worked examples
  by their spec-published resolved-level vectors, a permutation-
  invariant sweep over a small set of mixed-level shapes, and
  end-to-end W → N → I → L1 → L2 runs on real Arabic + Latin
  fragments. **X-rules (explicit-embedding / override / isolate
  stack + isolating-run-sequence partition), N0 (bracket-pair
  resolution), and L3 / L4 (combining-mark reordering + bidi-
  mirroring) remain deferred.** L3 is conditional on the
  renderer's mark-attachment policy — scribe's GPOS mark-to-base /
  mark-to-mark stacker keeps logical (post-base) order in both
  directions, so the spec's "If the rendering engine expects them
  to follow the base characters" guard does not fire; L4 needs
  `BidiMirroring.txt` to identify the mirrored set, which is not
  yet vendored alongside the UAX HTML.
- **BiDi implicit-level resolution I1 + I2 (round 204)** —
  `bidi::resolve_implicit_levels(&classes, embedding_level) ->
  Vec<u8>` runs the UAX #9 §3.3.6 implicit-level pass over one
  isolating run sequence and returns the per-character resolved
  embedding level. The slice is expected to be the output of
  `resolve_neutral_types` (no NI left; only `L` / `R` / `EN` / `AN` /
  `NSM` / `BN`). The implementation is the literal Table 5 row
  selection: at an **even** embedding level, `L` stays, `R` goes
  `+1`, and `EN` / `AN` go `+2`; at an **odd** embedding level, `R`
  stays, and `L` / `EN` / `AN` all go `+1`. `BN` is ignored per
  §5.2 — its level stays at the embedding level so the L-rule
  pass folds it. Surviving `NSM` (the rare case where W1 left it
  intact) is treated the same. 8 unit + 15 integration tests
  cover every Table 5 row at multiple embedding levels (even /
  odd, including non-zero base levels up to 124), the `BN` carve-
  out, empty-input no-op, the spec's max-depth-overflow note, and
  full W → N → I pipelines on LTR / RTL paragraph fragments
  (including the §3.3.5 closing prose example "R EN ET EN R" at
  EL 1, which exercises W5's ET-EN collapse + I1's EN-up-two in
  a single end-to-end vector). The I-rule output is the stable
  level vector the round-210 L1 / L2 pair consumes.
- **BiDi neutral-type resolution N1 + N2 (round 198)** —
  `bidi::resolve_neutral_types(&mut classes, embedding_level, sos, eos)`
  runs the UAX #9 §3.3.5 neutral / isolate-formatting pass over one
  isolating run sequence in place. The slice is expected to be the
  output of `resolve_weak_types` (no `AL` left, no leftover `ES` /
  `ET` / `CS`). The implementation walks every maximal contiguous
  run of NI elements (`B` / `S` / `WS` / `ON` / `LRI` / `RLI` /
  `FSI` / `PDI`) and resolves it with **N1** when the strong type
  on both sides agrees — `EN` and `AN` count as `R` per the spec's
  "European and Arabic numbers act as if they were R", and the
  strong-side walk skips over `NSM` / `BN` (which are non-strong
  but also non-NI); falls back to `sos` / `eos` at the sequence
  boundaries — or with **N2** (the embedding direction, `L` for
  even `embedding_level`, `R` for odd) when the strong context
  differs. After the pass every NI has been resolved to a strong
  direction; `NSM` and `BN` survive untouched (the §3.3.6 implicit-
  level pass handles them). 10 unit + 11 integration tests cover
  every spec example (`L NI L → L L L`, `R NI R → R R R`, the full
  R/AN/EN N1 table with EN/AN counting as R, the N2 mismatch table
  at both embedding levels, the full NI alias collapsing in one
  run, `NSM` / `BN` pass-through across NI boundaries, sos / eos
  driving boundary-spanning runs, idempotence on NI-free slices,
  and the compose-with-W realistic Arabic + numbers pipeline).
  **N0 (bracket-pair resolution per §3.1.3 + §3.3.5) is deferred**
  — it requires the Unicode `BidiBrackets.txt` data file to
  identify opening / closing paired brackets, which is not yet
  vendored under `docs/`. The N1 / N2 surface is forward-
  compatible: an N0 implementation lands as a pre-N1 pass that
  turns bracket positions into strong types, after which N1 / N2
  continues to apply unchanged (the spec narrative confirms this
  in the closing N0 note: "if the enclosed text contains no strong
  types the bracket pairs will both resolve to the same level when
  resolved individually using rules N1 and N2"). I / X / L rules
  remain deferred.

## Out of scope

- **Pixel work** — bitmap rasterisation, alpha compositing, synthetic
  bold dilation, stroke dilation. All in
  [`oxideav-raster`](https://github.com/OxideAV/oxideav-raster).
- **Bidi (UAX #9) N0 bracket pairs + L3 / L4** (X1..X9 explicit-
  level pass landed in round 217; X10 isolating-run-sequence
  partition + sos/eos derivation landed in round 220), **CFF2
  variable charstrings**
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
