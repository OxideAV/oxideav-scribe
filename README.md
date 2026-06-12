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
  kerning) + GPOS type 3 (cursive attachment) + GPOS type 4/5/6
  (mark-to-base, mark-to-mark stacking), enough for Latin / Cyrillic
  / Greek / basic CJK / Vietnamese / polytonic Greek.
- **BiDi Unicode 16.0 UCD data tables (round 283)** — the three UAX
  #9 property lookups are now data-driven from the Unicode 16.0 UCD
  snapshots staged under `docs/text/unicode-bidi/` and vendored
  verbatim into `src/bidi/` (embedded via `include_str!`, parsed
  once on first lookup into sorted binary-searched range tables).
  `bidi::bidi_class(c)` covers every assigned code point from
  `DerivedBidiClass.txt` (2289 ranges) plus the file's `@missing`
  defaults for unassigned code points — `R` / `AL` in the
  right-to-left script blocks, `ET` in the Currency Symbols block —
  replacing the previous hand-mapped block list.
  `bidi::paired_bracket(c)` carries the full normative
  `BidiBrackets.txt` table (64 open/close pairs: ASCII, Tibetan gug
  rtags, square-bracket-with-quill, mathematical, CJK / fullwidth),
  and the BD16 walker now honours the close-branch "U+3009 and
  U+232A are treated as equivalent" canonical-equivalence clause.
  `bidi::mirrored_glyph(c)` carries the full informative
  `BidiMirroring.txt` table (428 entries — brackets, angle
  quotation marks, mathematical relations, CJK brackets), so rule
  L4 mirrors far beyond the previous ASCII seed set. 4 unit + 11
  integration tests pin the table-size / involution / open-close
  symmetry invariants, the `@missing` fallbacks, the BD16
  equivalence pairings, and N0 + L4 end-to-end through the
  paragraph driver with non-ASCII brackets and mirrors.
- **GPOS cursive attachment (round 276)** — the shaper's positioning
  pipeline runs a CursivePosFormat1 pass over consecutive non-mark
  glyph pairs: when the first glyph publishes an **exit** anchor and
  the second an **entry** anchor, the first glyph's advance is
  rewritten so the anchors align in the line-layout direction, and
  the second glyph's `y_offset` moves so they align cross-stream
  (the RIGHT_TO_LEFT-flag-clear semantics — second glyph adjusted).
  Cross-stream adjustments accumulate down a connected chain (the
  cascading-baseline behaviour joining scripts need), marks attached
  to a cursive glyph follow it vertically, and NULL entry/exit
  anchor offsets skip the pair per spec. The flag-set variant (first
  glyph adjusted, chain anchored at the last glyph) is deferred
  until `oxideav-ttf` exposes GPOS lookup flags.
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
  220 (`level_runs` + `isolating_run_sequences`); L3 combining-
  mark reordering landed in round 247; the bracket-pair rule N0
  landed in round 257 and L4 mirroring in round 268, both with
  full Unicode 16.0 data tables as of round 283.
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
  another does not matter." L3 combining-mark reordering landed in
  round 247 — see the dedicated entry above. 17 unit + 16 integration tests cover
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
  weak / neutral / implicit-level passes per-sequence. L3
  combining-mark reordering (§3.4) landed in round 247 — see the
  dedicated entry. N0 (bracket-pair resolution per §3.1.3) landed
  in round 257 and L4 (bidi-mirroring) in round 268; both consume
  the full Unicode 16.0 `BidiBrackets.txt` / `BidiMirroring.txt`
  data tables as of round 283.
- **BiDi §3.3.1 P1 multi-paragraph driver (round 233)** —
  `bidi::process_text(text, base_level) -> TextBidi` is the top-
  level entry point for whole-document text. It walks the P1 split
  (paragraph separator type B kept with the previous paragraph per
  the §3.3.1 prose: LF, CR, NEL, FS / GS / RS, PS — every codepoint
  scribe's `bidi_class()` table assigns to class B) via the existing
  `split_paragraphs` and dispatches each paragraph slice through
  `process_paragraph_classes` independently. The returned
  [`TextBidi`] carrier publishes `paragraphs: Vec<ParagraphSlice>`
  (one entry per paragraph, in logical order) plus `total_chars:
  usize` (whole-input character count). Each `ParagraphSlice` carries
  the paragraph's `byte_range: Range<usize>` (half-open, including
  any trailing B), `char_offset: usize` (cumulative whole-text
  character offset where the paragraph starts), the round-227
  `bidi: ParagraphBidi` (the §3 P → X → W → N → I result), and
  `char_byte_offsets: Vec<usize>` (whole-input byte index of every
  character — pre-rebased so callers index back into the original
  `&str` without paragraph-local offset arithmetic). `TextBidi::len`
  / `is_empty` / `locate_char(k) -> Option<(usize, usize)>` are
  paragraph-count + character-position accessors;
  `locate_char` returns `(paragraph_index,
  paragraph_local_char_index)` for any whole-input character index
  below `total_chars`, `None` past it. `base_level: Option<u8>`
  applies the HL1 override uniformly across paragraphs (low-bit
  clamped); `None` lets P2 / P3 walk each paragraph independently,
  so the LTR + RTL paragraphs of `"Hi\nשלום"` resolve to levels 0
  and 1 respectively without caller bookkeeping. 14 unit + 24
  integration tests cover P1 split on every recognised class-B
  separator, the trailing-B-kept-with-preceding-paragraph invariant,
  terminal-LF not creating a phantom paragraph, consecutive
  separators producing one paragraph each, per-paragraph P2 / P3
  independence, HL1 uniform application + low-bit clamp,
  contiguous-tiling byte ranges, multi-byte char_byte_offsets
  (Hebrew alef = 2 UTF-8 bytes), `total_chars == chars().count()`,
  `locate_char` round-trip with offset arithmetic + out-of-bounds
  `None`, observational equivalence with a manual `split_paragraphs`
  + `process_paragraph` loop, per-paragraph `paragraph_level` cross-
  check against the text walker, downstream
  `ParagraphBidi::reorder_paragraph` continuing to work via the
  carrier, manual L1 / L2 drive via `reset_trailing_levels` +
  `reorder_line` succeeding per paragraph, and Clone + Eq + Debug
  derive smoke tests on both carrier types. L3 landed in round 247;
  N0 in round 257; L4 in round 268.
- **BiDi §3 whole-paragraph driver (round 227)** —
  `bidi::process_paragraph(text, base_level) -> (ParagraphBidi,
  Vec<usize>)` and the class-driven mirror
  `bidi::process_paragraph_classes(classes, base_level) ->
  ParagraphBidi` compose the six existing per-rule passes from
  rounds 186 / 191 / 198 / 204 / 217 / 220 into one entry point:
  §3.2 class assignment → §3.3.1 P2 + P3 first-strong walk (or the
  HL1 caller override when `base_level` is `Some(_)`) → §3.3.2
  X1..X9 explicit-level / override / isolate stack → §3.3.3 X10
  BD7 + BD13 isolating-run-sequence partition + sos / eos
  derivation → per-sequence §3.3.4 W1..W7 → §3.3.5 N1 + N2 →
  §3.3.6 I1 + I2. The returned [`ParagraphBidi`] carrier publishes
  four paragraph-wide parallel vectors: `paragraph_level: u8` (the
  P2 / P3 / HL1 outcome), `classes: Vec<BidiClass>` (input
  preserved for §3.4 L1's "original types" note), `effective_classes:
  Vec<BidiClass>` (X4 / X5 / X5a / X5b / X6 / X6a override-rewritten
  variant), `removed: Vec<bool>` (X9 removed-flag set), and
  `levels: Vec<u8>` (per-character resolved embedding level after
  the full X → W → N → I sweep — X9-removed positions keep their
  X-rule level since W / N / I skipped them per §3.3.2 X9). A
  `base_level` mask of `& 1` enforces the §3.3.1 paragraph-level
  convention {0, 1} regardless of caller-supplied value. The
  text-driving variant returns a parallel `Vec<usize>` of byte
  offsets so a `text[char_byte_offsets[i]..]` slice locates the
  character at logical index `i` (essential for UTF-8 multi-byte
  scripts — Hebrew U+05D0..U+05D2 advance by 2 bytes each, Arabic
  U+0627..U+06FF by 2 bytes, U+1F600 by 4). `ParagraphBidi::reorder_paragraph()`
  is the "treat the whole paragraph as one line" convenience that
  runs §3.4 L1 + L2 over the carrier in place;
  `ParagraphBidi::reorder_line_range(start..end)` is the per-line
  variant a real line-breaker calls after deciding where the
  paragraph wraps. 19 unit + 22 integration tests cover P2 / P3
  fallbacks (first-strong L / R / AL, no-strong, BD8 isolate-span
  skipping for both LRI..PDI and unmatched-LRI variants), the HL1
  base-level override (both directions + low-bit clamp), full
  end-to-end mixed L / R compositions (LTR-with-RTL-block,
  RTL-with-LTR-block lifted to level 2, AL EN → R AN pipeline),
  X9 removed-position level preservation, text-driving char-byte
  offset advancement on ASCII / Hebrew / Arabic, the §3.4
  reorder_paragraph identity / RTL-reversal pair, per-line
  reorder_line_range on a split paragraph, and the §3.4 spec
  example "car means CAR." resolving to its published visual order.
  L3 landed in round 247; N0 in round 257; L4 in round 268.
- **BiDi foundation (round 186)** — `bidi::bidi_class(c)` returns the
  UAX #9 §3.2 normative bidirectional class (originally a
  hand-mapped block list; full `DerivedBidiClass.txt`-driven
  per-code-point coverage as of round 283 — see the dedicated
  entry). `bidi::paragraph_level(s)`
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
  exposes the §3.3.5 NI alias predicate the N-rules consume (N / I
  / X / L rules have all since landed — see their entries).
- **BiDi §3.3.5 N0 bracket-pair resolution (round 257)** —
  `bidi::paired_bracket(c)` returns the BD14 / BD15 paired-bracket
  lookup (full `BidiBrackets.txt` table as of round 283) as
  `Some((paired_char, BracketKind::{Open, Close}))`;
  `bidi::bracket_pairs(chars, classes)` runs the **BD16**
  (§3.1.3) stack walk over one isolating run sequence, returning
  the list of `(open_pos, close_pos)` pairs sorted by opener in
  ascending logical order (the N0 sequencing invariant). The
  walker maintains the spec-mandated 63-element stack — overflow
  triggers the BD16 "stop processing and return an empty list"
  branch — and honours the BD14 / BD15 "current bidi class is ON"
  gate, so brackets whose class was rewritten by X6 / RLO are
  ignored. `bidi::resolve_bracket_pairs(classes, pairs,
  embedding_level, sos)` applies **N0** in place to one isolating
  run sequence post-W7 and pre-N1: each pair's interior is scanned
  for a strong type (EN / AN projected to R per the §3.3.5 note),
  then the N0 b / c.1 / c.2 / d branches are dispatched —
  matching-embedding-inside → both brackets to embedding direction
  (b); opposite-inside + preceding-strong-also-opposite → both to
  that direction (c.1); opposite-inside + preceding-strong-
  matches-embedding → both to embedding direction (c.2); nothing
  strong inside → leave the pair untouched for N1 / N2 (d). The
  trailing-NSM clarification ("any NSM following a paired bracket
  which changed under N0 should change to match the bracket") is
  honoured for the contiguous NSM run after each rewritten
  bracket. Pairs are processed sequentially in opener-ascending
  order so an inner pair sees the rewrites of every outer pair
  already processed (the §3.3.5 "sequentially in the logical
  order of the text positions of the opening paired brackets"
  invariant). `bidi::process_paragraph_with_brackets(text,
  base_level)` and `bidi::process_paragraph_classes_with_brackets(
  classes, chars, base_level)` are paragraph-driver mirrors of
  the round-227 entry points with N0 wired in between W7 and N1;
  the non-`with_brackets` variants stay the legacy
  pass-through (N0 not run) so existing callers see no
  behaviour change. The six ASCII brackets are now classified
  `BidiClass::ON` in `bidi::bidi_class` (previously they fell
  through to the L default). 23 unit + 22 integration tests cover
  every BD16 worked-example line from the §3.1.3 spec table, every
  N0 case (b / c.1 / c.2 / d), the EN / AN projection note, the
  63-stack overflow branch, the non-ON skip gate, the sequential-
  rewrite invariant, and the trailing-NSM clarification. Round
  283 replaced the original six-ASCII-bracket seed table with the
  full normative Unicode 16.0 `BidiBrackets.txt` table (64 pairs)
  and added the BD16 U+3009 ≡ U+232A canonical-equivalence
  comparison — see the dedicated entry.
- **BiDi §3.4 L4 mirroring (round 268)** — `bidi::mirrored_glyph(c)`
  returns the `Bidi_Mirroring_Glyph` acceptable-mirror-pair
  character (full `BidiMirroring.txt` table as of round 283); the
  lookup is an involution and returns `None` for characters without
  a mirror pair, including the §3.4-excluded backward-compatibility
  pair U+FD3E / U+FD3F ornate parentheses ("for backward
  compatibility ... not mirrored"). `bidi::apply_mirroring(chars,
  levels)` applies
  rule **L4** in place over a line's logical character sequence:
  every position whose resolved level is odd (resolved
  directionality R, per the §3.2 even-LTR / odd-RTL level
  convention) and whose character has a mirror pair is replaced by
  the mirrored character — the spec's worked example "U+0028 LEFT
  PARENTHESIS appears as `(` when its resolved level is even, and as
  the mirrored glyph `)` when its resolved level is odd" holds
  verbatim. L4 is a per-position glyph selection independent of the
  L2 / L3 permutation; applying it to the logical sequence and then
  walking the L2 permutation is the straightforward composition (and
  because the lookup is an involution, callers run it exactly once
  per line). The HL6 higher-level-protocol override is out of scope.
  13 unit + 19 integration tests cover the seed-pair round-trips,
  the involution + paired-bracket agreement invariants, the ornate-
  parentheses exclusion, the even / odd / mixed / higher-odd level
  dispatch, non-mirrored pass-through, double-application restore,
  the length-mismatch panic, and end-to-end compositions through
  `process_paragraph_with_brackets` + L1 → L2 → L3 → L4 on RTL
  Hebrew lines with brackets and combining marks. Round 283
  replaced the original six-ASCII-bracket seed table with the full
  Unicode 16.0 `BidiMirroring.txt` table (428 entries) — see the
  dedicated entry.
- **BiDi §3.4 L3 combining-mark reordering (round 247)** —
  `bidi::reorder_combining_marks(orig_classes, levels, &mut visual)`
  applies UAX #9 §3.4 rule **L3** in place to the visual permutation
  returned by `reorder_line`: every `[NSM, …, NSM, base]` block in
  visual order whose logical indices form an L2-reversed strictly-
  decreasing sequence (i.e. the post-L2 footprint of one RTL
  `base + marks` cluster) is wholesale reversed back to
  `[base, NSM, …, NSM]` so the marks follow the base in the final
  display stream — the contract a renderer that paints marks with
  rightward overhangs (the spec's "expects them to follow"
  alternative) requires. The block is reversed wholesale per the
  spec wording "the ordering of the marks and the base character
  must be reversed", which restores the marks to their original
  logical-source order behind the base. The decreasing-logical-
  index check uniquely identifies the L2-reversed shape, so L3 is
  idempotent (re-running on already-rotated visual is a no-op) and
  multi-cluster RTL runs rotate each cluster independently. Even-
  level (LTR) runs are untouched — L2 didn't reverse them, so
  marks are already after the base. NSMs whose level differs from
  the following base (rare post-W1 leftover) and orphan NSMs with
  no matching base in the same level-1 block are conservatively
  left alone. 9 unit + 15 integration tests cover the single-base
  RTL one-mark / two-mark / three-mark cases, multi-cluster RTL
  with same and different mark counts, AL (Arabic-letter) base,
  LTR-marks-untouched, RTL-island in an LTR paragraph, LTR-island
  in an RTL paragraph (level-2 nested, marks out of scope), empty
  input, orphan leading NSM, idempotency, the
  L3-yields-a-permutation invariant, end-to-end L1 → L2 → L3
  composition, and the pure-RTL multi-letter word with one mark.
  Scribe's GPOS mark-to-base / mark-to-mark stacker keeps marks in
  logical (post-base) order on both LTR and RTL, so callers using
  scribe's own renderer can skip the L3 step; the L3 entry point is
  for external callers wiring a different mark-attachment policy.
  The L4 mirroring rule landed in round 268 — see the dedicated
  entry above; the full `BidiMirroring.txt` table landed in round
  283.
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
  fragments. The X-rules pair (X1..X9 stack + X10 isolating-run-
  sequence partition) landed in rounds 217 + 220; L3 (combining-
  mark reordering) landed in round 247 — see the entry above.
  The N0 algorithm landed in round 257 and the L4 mirroring rule
  in round 268; the full Unicode 16.0 `BidiBrackets.txt` /
  `BidiMirroring.txt` / `DerivedBidiClass.txt` data tables landed
  in round 283.
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
  N0 (bracket-pair resolution per §3.1.3 + §3.3.5) landed in
  round 257 exactly as anticipated here — a pre-N1 pass that
  turns bracket positions into strong types, after which N1 / N2
  apply unchanged (the spec narrative confirms this in the
  closing N0 note: "if the enclosed text contains no strong
  types the bracket pairs will both resolve to the same level when
  resolved individually using rules N1 and N2"). The I / X / L
  rules have all since landed — see their entries.

## Out of scope

- **Pixel work** — bitmap rasterisation, alpha compositing, synthetic
  bold dilation, stroke dilation. All in
  [`oxideav-raster`](https://github.com/OxideAV/oxideav-raster).
- **Bidi (UAX #9) HL1..HL6 higher-level-protocol overrides** (the
  rule pipeline itself is complete: X1..X9 round 217, X10 round
  220, W round 191, N0 round 257, N1/N2 round 198, I round 204, L1
  + L2 round 210, L3 round 247, L4 round 268, full Unicode 16.0
  data tables round 283 — HL overrides remain caller
  responsibility), **CFF2 variable charstrings** (the
  `blend` operator in TN5177 v3 — scribe parses the CFF2 INDEX
  walker, but doesn't yet emit variation-blended cubic outlines),
  **TrueType bytecode hinting**, **subpixel LCD filtering**,
  **GPOS cursive attachment RIGHT_TO_LEFT-flag semantics** (the
  flag-clear pass landed in round 276; the flag-set cross-stream
  variant needs lookup-flag exposure in `oxideav-ttf`'s public GPOS
  API) — deferred.

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
