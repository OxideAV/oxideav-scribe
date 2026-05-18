# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.8](https://github.com/OxideAV/oxideav-scribe/compare/v0.1.7...v0.1.8) - 2026-05-18

### Other

- hygiene round 75 (Error::source, lib.rs doc refresh, no_run doctests)
- drop committed Cargo.lock + relax oxideav-core to "0.1"
- backfill Unreleased entry for round 14 + round 13
- variable fonts (CFF2/MVAR/HVAR/VVAR/STAT/name_id) + Brahmic round 13 (Burmese + Lao)

### Changed — hygiene (round 75)

Internal-only hygiene round. No new spec material consulted; the
shaper's behavioural surface is unchanged.

- `Error::source()` is now wired so callers walking the source chain
  (anyhow, thiserror's `#[from]` walker, the `?` operator with
  `dyn Error` conversion) see the underlying `oxideav_ttf::Error` /
  `oxideav_otf::Error` rather than just the wrapper. Previously the
  `From` impls preserved the inner error but the `source()` method
  returned `None`.
- Crate-level `lib.rs` doc-comment is brought current with rounds
  13 (Burmese + Lao + multi-glyph context-aware GSUB) and 14
  (`Face::mvar` / `hvar` / `vvar` / `stat` / `cff2` / `name_id`),
  matching the README's already-updated capabilities tour. Same
  text — the README was already in sync; the lib doc had drifted.
- The two `Shaper::with_variation_coords` doctest snippets are
  converted from `ignore` to `no_run`. They now compile-check
  against the public API (catching any future signature drift) but
  still skip execution because both need a non-trivial `Face`
  instance to be meaningful.
- Added a small `error_tests` module covering the `Error` /
  `Display` / `From` surface (4 new lib tests; total 264 → 268).
  Round-trips `Face::from_ttf_bytes` / `from_otf_bytes` with 4-byte
  garbage to confirm the wrapper variants carry the inner parser
  error.

### Added — variable-font metrics + style attributes (round 14, #454)

Round 14 closes the variable-font metrics gap that the round-9 outline
work left open. The four metric-variation tables now have first-class
parsers + per-coord lookup methods on `Face`:

- **MVAR** (Metrics Variations) — `Face::mvar()` parses the table;
  `Face::metric_delta(b"hasc")` returns the ascender delta in font
  units at the current variation coords (similar accessors via tag
  for `cpht` cap-height, `xhgt` x-height, `undo` underline offset,
  `unds` underline size, `strs` strikeout size, `stro` strikeout
  offset, etc. — every tag in the OpenType MVAR ValueTag registry).
- **HVAR** (Horizontal-Advance Variations) — `Face::hvar()` parses
  the table; `Face::h_advance_delta(gid)` returns the per-glyph
  advance-width delta. The implicit `glyphID → (0, glyphID)`
  identity (used by HVAR tables that omit the optional
  advanceWidthMapping) is handled transparently.
- **VVAR** (Vertical-Advance Variations) — `Face::vvar()` /
  `v_advance_delta(gid)` mirrors HVAR for vertical layout. Returns
  `None` / `0.0` for the horizontal-only fonts that omit VVAR
  (the common case for Latin fonts).
- **STAT** (Style Attributes) — `Face::stat()` parses the table;
  `stat_axes()` enumerates the design axes (each carries a `name`-
  table id for the axis label); `stat_axis_values()` enumerates
  every axis-value record across all four spec formats:
  `Single` (one named point on one axis — e.g. `wght=400 →
  "Regular"`), `Range` (e.g. `wght 600..700 → "SemiBold/Bold"`),
  `Linked` (named value plus a "linked" value used for the
  bold/italic toggle), and `Combined` (multi-axis named
  combinations — e.g. `wght=700 + wdth=75 → "Bold Condensed"`).

The four metric tables share two pieces of plumbing:

- **`ItemVariationStore`** — the OpenType-spec delta-storage
  primitive. One ItemVariationStore can hold multiple sub-tables;
  each sub-table publishes a region-index list, a delta-set count,
  and a packed delta array. `resolve_delta(outer, inner, &coords)`
  walks the regions, computes the per-region scalar via the
  spec's "(coord - start) / (peak - start)" ramp + sign / zero
  rules, and accumulates the scaled delta sum.
- **`DeltaSetIndexMap`** — short (format 0) + long (format 1)
  variants both supported. Resolves an item key (glyph id for
  HVAR/VVAR, or an implicit 0 for MVAR's per-tag lookup) into
  an `(outer_index, inner_index)` pair that addresses the IVS.

`Face::name_id(nid)` resolves any `name`-table id to the
highest-ranked Unicode string, with the same priority the
underlying TTF parser uses (Windows English first, Mac Roman
English second, anything Unicode-y after, then any remaining
record). Closes the surface for callers that consumed an
`axis_name_id` / `subfamily_name_id` (from `fvar`) or a
`value_name_id` (from STAT) and need the human-readable label —
they no longer have to reach into `Face::with_font` for it.

`Face::cff2()` parses the CFF2 INDEX walker (header + Top DICT
+ Global Subrs + CharStrings INDEX) and reports the table's
glyph count + variation-axis count + `has_charstrings` boolean.
Full Type 2 v3 charstring evaluation under variations (with the
`blend` operator) is the deferred follow-up; once the underlying
`oxideav-otf` crate exposes a CFF2 charstring interpreter,
scribe will lift it onto `Face::glyph_path` for OTF / CFF2
variable fonts the same way it does for TT / gvar today.

All variation tables are parsed locally in scribe (`crate::variations`)
from raw font bytes obtained via `Face::raw_bytes()` rather than from
new APIs on `oxideav-ttf` / `oxideav-otf` — keeps the round
self-contained without coordination on the producer crates' release
cadence.

Test coverage:
- `crate::variations::tests` — 5 new unit tests (synthetic-bytes
  coverage of `ItemVariationStore`, `DeltaSetIndexMap`, `StatTable`
  format-1 round-trip, `NameTableSnapshot` UTF-16 decode, `CFF2`
  INDEX walker on empty + non-empty INDEX bytes).
- `tests/round14_variable_metrics.rs` — 16 new integration tests
  against `tests/fixtures/InterVariable.ttf` (Inter ships MVAR +
  HVAR + STAT with two `fvar` axes — `wght` 100..900 + `opsz`
  14..32 — plus 9 named instances). MVAR enumerates well-known
  metric tags and produces a non-zero delta at `wght=900`. HVAR
  produces a non-zero per-glyph advance delta at `wght=900`.
  VVAR is correctly absent (Inter is horizontal-only). STAT
  enumerates ≥ 2 design axes (`wght` + `opsz`), with at least
  one Single-format record on the wght axis. `name_id` resolves
  the family name, every `axis_name_id`, and at least one named-
  instance subfamily label. CFF2 is correctly absent (Inter is
  TT-flavoured). Plus 2 OTF-flavour tests against
  `tests/fixtures/SourceSans3-Regular.otf` confirming the
  `name_id` resolver works on OTF magic and that a static OTF
  font reports `mvar / hvar / vvar / stat / cff2 == None`.

The implementations are clean-room readings of:

- Microsoft OpenType §"MVAR — Metrics Variations Table".
- Microsoft OpenType §"HVAR — Horizontal Metrics Variations Table".
- Microsoft OpenType §"VVAR — Vertical Metrics Variations Table".
- Microsoft OpenType §"STAT — Style Attributes Table".
- Microsoft OpenType §"Item Variation Store Header and Item Variation
  Subtables".
- Microsoft OpenType §"Delta Set Index Map Table".
- Microsoft OpenType §"name — Naming Table".
- Adobe Technical Note #5176 §"CFF2 charstring format" and TN5177.

No HarfBuzz / FreeType / fontTools / pango source consulted.

## [0.1.7](https://github.com/OxideAV/oxideav-scribe/compare/v0.1.6...v0.1.7) - 2026-05-05

### Other

- Sinhala + Khmer + Thai (round 12, Brahmic non-Indic)

### Added — three Brahmic non-Indic scripts (round 12)

Round 12 extends the cluster machine across the script-family boundary
to three of the five Brahmic non-Indic scripts. The round-11
`ReorderRules` template carried straight over: each script ships its
own categorisation table + `*_RULES` constant + feature-tag list, and
the existing `cluster_boundaries_with` / `reorder_cluster_with`
implementations consume them unchanged.

- **Sinhala** (U+0D80..U+0DFF) — closest of the three to the Indic
  shape. U+0DCA "AL-LAKUNA" plays the halant role (suppresses inherent
  vowel; glues the next consonant into a conjunct cluster). Pre-base
  matras: U+0DD9 (sign-e), U+0DDA (sign-ee), U+0DDB (sign-ai). The
  precomposed two-part vowels U+0DDC / U+0DDD / U+0DDE (sign-o /
  sign-oo / sign-au) carry pre-base components after canonical
  decomposition — classified as `PreBaseMatra` so a cluster machine
  on raw input still emits a pre-base reorder. `reph_enabled = false`
  — Sinhala has no superscript reph rendering; RA + al-lakuna stays
  in-line. Feature-tag list omits `rphf` and adds `pref` / `blwf` /
  `pstf` for the two-part vowel decompositions.
- **Khmer** (U+1780..U+17FF) — uniquely complex among the round-12
  scripts. U+17D2 "KHMER SIGN COENG" plays the halant role and stacks
  the following consonant as a subjoined letter underneath the base.
  Khmer subjoined chains run two- to three-deep in Pali borrowings;
  the cluster machine glues them into a single cluster via the
  standard halant-skip rule. Pre-base matras: U+17BE (oe), U+17BF
  (ya), U+17C0 (ie), U+17C1 (e), U+17C2 (ae), U+17C3 (ai), plus the
  precomposed U+17C4 / U+17C5 (oo / au). `reph_enabled = false` —
  Khmer's RA + COENG renders as subjoined RA, not as a reph. Feature-
  tag list omits `rphf` and adds the Khmer-specific `cfar` (coeng-ra
  final reordering).
- **Thai** (U+0E00..U+0E7F) — the structural outlier. Thai has **no
  halant** and **no conjunct formation** — every consonant starts a
  new cluster. Thai's pre-base vowels U+0E40..U+0E44 (SARA E / AE /
  O / AI MAIMUAN / AI MAIMALAI) appear in **storage / keyboard order
  BEFORE** their consonant — the only Indic-family script where this
  is the case — so their visual position already matches storage and
  no reorder is needed. The cluster machine treats them as `Vowel`
  (which starts a new cluster) and the existing segmenter does the
  rest. Tone marks U+0E48..U+0E4B (mai ek / mai tho / mai tri / mai
  chattawa) and signs U+0E4C..U+0E4E (thanthakhat / nikhahit /
  yamakkan) are classified as `Bindu` so they attach to the cluster
  end. Above-base vowel signs U+0E31, U+0E34..U+0E37, U+0E47 and
  below-base signs U+0E38..U+0E3A are `Matra`. Feature-tag list is
  minimal — `locl` / `ccmp` plus the four presentation-pass features.
- **`Script::{Sinhala, Khmer, Thai}`** added to the
  `shaping::arabic::Script` enum. The shared `script_of(char)`
  classifier now returns the right variant for every codepoint in the
  three new blocks.
- **`script_indic_tags`** extended to map each new variant to its
  OpenType script tag pair: `(sinh, sinh)` / `(khmr, khmr)` /
  `(thai, thai)` (Sinhala / Khmer / Thai have a single Indic2 tag —
  no v1/v2 split, since they were added after the Indic2 transition).
- **Crate-root re-exports** for the new categorisation functions
  (`sinhala_category` / `khmer_category` / `thai_category`),
  feature-tag functions (`sinhala_feature_tags` / `khmer_feature_tags`
  / `thai_feature_tags`), and rules constants (`SINHALA_RULES` /
  `KHMER_RULES` / `THAI_RULES`).

The implementations are clean-room readings of:

- Unicode 15.1 Standard Annex #15 (Indic / Brahmic syllabic
  categories); UAX #29 (cluster boundaries); the per-block charts in
  the Unicode 15.1 core spec.
- Microsoft OpenType Layout — *Creating and supporting OpenType fonts
  for South-East Asian scripts* (Khmer / Thai); *Creating and
  supporting OpenType fonts for the Sinhala script*.

No HarfBuzz / FreeType / pango / ICU layout source consulted.

Test coverage:
- `shaping::indic::tests` — 36 new unit tests covering the three new
  scripts (categorisation per category + per-script pre-base reorder +
  reph-disabled assertions + cluster-boundary cases for halant chains
  / coeng subjoined chains / Thai vowel-break + feature-tag list
  shape). Total `shaping::indic` test count: 117 (up from 81).
- `face_chain::tests` — 7 new unit tests covering the multi-script
  cluster-span pass: per-script reorder + cluster-span script tag for
  each new script + Khmer three-deep subjoined chain + Thai storage-
  order preservation + a mixed Devanagari / Thai run that segments
  cleanly at the script boundary.
- `tests/round12_sinhala_cluster.rs` /
  `tests/round12_khmer_cluster.rs` / `tests/round12_thai_cluster.rs`
  — three new integration tests. Sinhala + Khmer follow the round-10
  / round-11 fixture-skip pattern (DejaVuSans does not cover
  U+0D80..U+0DFF or U+1780..U+17FF; tests `eprintln!` and return when
  the fixture cmap is empty for the script in question). The Thai
  test runs **without skip** — DejaVuSans does cover the Thai block,
  so the test asserts SARA E + KO KAI gid sequence is preserved (no
  reorder happens) and tone marks pass through.

Followup tasks deferred:
- **Burmese (U+1000..U+109F) + Lao (U+0E80..U+0EFF)** — the remaining
  two Brahmic non-Indic scripts. Burmese has medial consonants
  U+103B..U+103E that chain like Khmer subjoined letters but via a
  distinct mechanism (no coeng equivalent — the medials are encoded
  as standalone codepoints), plus complex tone marks. Lao mirrors
  Thai structurally but with distinct codepoints (U+0EB1 above-base
  vowel sign etc.). Both deferred to a future round.
- **Multi-glyph-context GSUB features** (`locl` / `nukt` / `akhn` /
  `cjct` / `init` / `haln`) still pending from #481. The round-11
  cluster-position pass implements `half` + `pref|blwf|abvf|pstf` +
  presentation features but the listed features need multi-glyph
  context (e.g. `cjct` matches a halant + consonant pair and emits a
  single conjunct gid) which the current `gsub_apply_lookup_type_1`
  call doesn't carry.
- **Indic font fixtures on the CDN.** The round-10 / round-11 / round-12
  Sinhala + Khmer integration tests still skip when the fixture font
  lacks coverage; landing `NotoSansSinhala` / `NotoSansKhmer` / et al.
  on `samples.oxideav.org/fonts/` via the existing `font_fixtures`
  helper would activate them.

## [0.1.6](https://github.com/OxideAV/oxideav-scribe/compare/v0.1.5...v0.1.6) - 2026-05-05

### Other

- 6 more Indic scripts + cluster-position GSUB (round 11)

### Added — six more Indic scripts + cluster-position GSUB wiring (round 11)

Round 11 brings the remaining Indic scripts under the same cluster
machine + adds cluster-position-aware GSUB feature dispatch on top of
the round-10 `rphf` pattern.

- **Six new scripts.** `shaping::indic::{gurmukhi,gujarati,telugu,
  kannada,malayalam,oriya}_category(char) -> IndicCategory` per-script
  syllabic categorisation tables, plus matching
  `{GURMUKHI,GUJARATI,TELUGU,KANNADA,MALAYALAM,ORIYA}_RULES`
  `ReorderRules` constants and `*_feature_tags()` functions:
  - **Gurmukhi** (U+0A00..U+0A7F) — Punjabi. Halant-driven (U+0A4D)
    + pre-base matra "i" U+0A3F + reph rule on RA U+0A30 (rare in
    modern usage; `rphf` lookup fires only when the font ships one).
  - **Gujarati** (U+0A80..U+0AFF) — closest to Devanagari. Halant
    U+0ACD + pre-base matra "i" U+0ABF + reph on RA U+0AB0.
  - **Telugu** (U+0C00..U+0C7F) — pre-base matras U+0C46 / U+0C47 /
    U+0C48 (e / ee / ai); reph on RA U+0C30; halant U+0C4D.
    Feature-tag list adds `pref` / `pstf` / `abvf` (the
    Telugu/Kannada/Malayalam family per-position GSUB features) on
    top of the Devanagari list.
  - **Kannada** (U+0C80..U+0CFF) — same Telugu family with own
    codepoints + halant U+0CCD; pre-base matras U+0CC6 / U+0CC7 /
    U+0CC8.
  - **Malayalam** (U+0D00..U+0D7F) — pre-base matras U+0D46 / U+0D47 /
    U+0D48; halant U+0D4D; chillu (independent half-form)
    characters U+0D7A..U+0D7F classified as `Consonant` (NFC-stable
    in modern orthography). `reph_enabled = false` because chillu
    replaces the historic reph rendering — the feature-tag list
    drops `rphf`.
  - **Oriya** (U+0B00..U+0B7F) — pre-base matras U+0B47 / U+0B48 plus
    the precomposed o / au matras U+0B4B / U+0B4C (which carry
    pre-base components after canonical decomposition); reph on RA
    U+0B30; halant U+0B4D.
- **`Script::{Gurmukhi,Gujarati,Telugu,Kannada,Malayalam,Oriya}`** —
  added to the `shaping::arabic::Script` enum. The shared
  `script_of(char)` classifier now returns the right variant for
  every codepoint in U+0A00..U+0D7F (the contiguous Indic range).
- **`script_indic_tags`** extended to map each new `Script` variant
  to its `(modern_indic2_tag, legacy_v1_tag)` OpenType script tag
  pair: `(gur2, guru)` / `(gjr2, gujr)` / `(tel2, telu)` /
  `(knd2, knda)` / `(mlm2, mlym)` / `(ory2, orya)`.
- **`FaceChain::shape` cluster-position-aware GSUB pass.** A new
  pass after the round-10 reph substitution walks each Indic
  cluster (recorded as a `ClusterSpan` sidecar from
  `apply_indic_reorder`) and dispatches the position-driven GSUB
  features:
  - `half` — applied to a base consonant immediately followed by a
    halant when the cluster has more characters after the halant.
  - `pref` / `blwf` / `abvf` / `pstf` — cascaded on a post-halant
    consonant; first lookup that returns a substitute wins.
    The cascade lets the font's form-position table dictate which
    feature applies to a given conjunct component without the
    shaper hard-coding per-script position rules.
  - `pres` / `psts` / `abvs` / `blws` — presentation-pass single
    substitutions applied to every glyph in the cluster.
  All of these use `Font::gsub_apply_lookup_type_1` against the
  active script tag pair (modern Indic2 tried first, legacy v1
  fallback). Coverage misses pass through unchanged — fonts without
  a given lookup degrade gracefully to the round-10 behaviour.
- **Crate-root re-exports** for the new categorisation functions,
  feature-tag functions, and rules constants.

The implementations are clean-room readings of:

- Unicode 15.1 Standard Annex #15 (Indic syllabic categories).
- Microsoft OpenType Layout — *Creating and supporting OpenType
  fonts for Indic scripts* (Gurmukhi / Gujarati / Telugu / Kannada /
  Malayalam / Oriya — the per-script *Shaping* informative examples
  drove the per-position feature cascade).

No HarfBuzz / FreeType / pango / ICU layout source consulted.

Test coverage:
- `shaping::indic::tests` — 32 new unit tests covering the six new
  scripts (categorisation + per-script pre-base matra reorder + reph
  identification + feature-tag list shape + chillu classification +
  feature-list assertions). Total `shaping::indic` test count: 81
  (up from 49).
- `face_chain::tests` — 8 new unit tests covering the multi-script
  cluster-span pass: per-script reorder + cluster-span script tags
  for every new script + the chillu cluster-boundary case + an
  `adjust_cluster_spans` helper test that verifies subsequent
  spans shift down after a reph drop.
- `tests/round11_telugu_cluster.rs` /
  `tests/round11_kannada_cluster.rs` /
  `tests/round11_gujarati_cluster.rs` — three new integration tests
  that follow the round-10 fixture-skip pattern (DejaVuSans does not
  cover U+0A80..U+0CFF; tests `eprintln!` and return when the
  fixture cmap is empty for the script in question). Activate once a
  Noto Sans Telugu / Kannada / Gujarati font lands in
  `samples.oxideav.org/fonts/` via the `font_fixtures` helper.

Followup tasks deferred:
- **Indic font fixtures on the CDN.** Activating the round-11
  integration tests + the round-8 / round-10 ones for Devanagari /
  Bengali / Tamil needs `NotoSansDevanagari` / `NotoSansBengali` /
  `NotoSansTamil` / `NotoSansTelugu` / `NotoSansKannada` /
  `NotoSansGujarati` / `NotoSansGurmukhi` / `NotoSansMalayalam` /
  `NotoSansOriya` (~280-330 KB each, OFL) on
  `samples.oxideav.org/fonts/`. The `font_fixtures` helper plumbing
  is already in place.
- **Brahmic non-Indic scripts.** Sinhala (U+0D80..U+0DFF), Burmese
  (U+1000..U+109F), Khmer (U+1780..U+17FF), Thai (U+0E00..U+0E7F),
  Lao (U+0E80..U+0EFF) all use cluster-based shaping but have
  stack-form / split-vowel rules outside the Indic2 cluster machine
  (e.g. Sinhala's vowel signs decompose to up to 3 components;
  Burmese's medial consonants chain via U+103B..U+103E).
- **Cluster-position feature ORDER.** Round 11 fires `half` then the
  `pref|blwf|abvf|pstf` cascade then the presentation features for
  every cluster. The OpenType Indic2 spec calls for a per-cluster
  feature application order (locl → ccmp → nukt → akhn → rphf →
  pref → blwf → half → abvf → pstf → cjct → init → pres → abvs →
  blws → psts → haln); the round-11 pass implements the inner
  half + position cascade + presentation block but does NOT yet
  apply locl / nukt / akhn / cjct / init / haln (those need
  multi-glyph context which the current single-substitution pass
  doesn't carry).

## [0.1.5](https://github.com/OxideAV/oxideav-scribe/compare/v0.1.4...v0.1.5) - 2026-05-04

### Other

- Bengali + Tamil + reph GSUB wiring (round 10)
- variable-font axis selection (round 9)
- add Arabic + Devanagari rows to capability list
- Devanagari complex-script cluster reorder (round 8)
- Arabic contextual joining (round 7)
- drop pixel pipeline — vector-only refactor ([#354](https://github.com/OxideAV/oxideav-scribe/pull/354))

### Added — Bengali + Tamil shaping + reph GSUB wiring (round 10)

Round 10 extends Indic complex-script shaping beyond Devanagari
(round 8 baseline) to two more scripts and wires the long-deferred
reph GSUB substitution.

- `shaping::indic::bengali_category(char) -> IndicCategory` — Bengali
  (U+0980..U+09FF) syllabic categorisation. Bengali shares
  Devanagari's structural shape (halant U+09CD glues consonants into
  conjuncts; bindus attach to the cluster end) but has THREE pre-base
  reordering matras (U+09BF "i", U+09C7 "e", U+09C8 "ai") instead of
  Devanagari's one. The reph rule is the same — RA U+09B0 + halant +
  consonant.
- `shaping::indic::tamil_category(char) -> IndicCategory` — Tamil
  (U+0B80..U+0BFF) syllabic categorisation. Minimal cluster machine:
  no nukta (no U+0BBC slot), no reph (Tamil RA does not form a
  superscript), no conjunct formation in the modern orthography.
  Three pre-base matras (U+0BC6 / U+0BC7 / U+0BC8 — e / ee / ai).
- `shaping::indic::ReorderRules { category, ra_codepoint, reph_enabled }`
  + `DEVANAGARI_RULES` / `BENGALI_RULES` / `TAMIL_RULES` constants —
  per-script reorder rule descriptors. The cluster machine
  (`reorder_cluster_with`) and segmenter (`cluster_boundaries_with`)
  are now generic over a `ReorderRules` reference, so adding
  Telugu / Gujarati / Gurmukhi / Kannada / Malayalam / Oriya in
  future rounds is a one-table change rather than a re-implementation.
- `shaping::indic::bengali_feature_tags()` /
  `shaping::indic::tamil_feature_tags()` — per-script OpenType GSUB
  feature application order. Bengali shares Devanagari's tag list
  one-to-one (same Indic family rules); Tamil's list omits `rphf` /
  `cjct` / `vatu` and adds the Tamil-specific `pref` (pre-base form,
  reorders the pre-base component of a precomposed two-part vowel
  sign).
- `shaping::indic::script_indic_tags(Script) -> Option<([u8; 4], [u8; 4])>`
  — maps a script to its `(modern, legacy)` OpenType script tag pair
  (`dev2` / `deva`, `bng2` / `beng`, `tml2` / `taml`). The `rphf`
  feature lookup walks BOTH tags so older fonts that only ship the
  v1 tag still get the reph substitution.
- `Script::Bengali` / `Script::Tamil` — added to
  `shaping::arabic::Script`. The shared `script_of(char)` classifier
  now returns them for the U+0980..U+09FF / U+0B80..U+0BFF blocks;
  `feature_tags_for_run(Script::Bengali / Script::Tamil)` returns
  the matching tag list.
- `FaceChain::shape` (and `shape_styled`) gained a generalised
  pre-cmap pass `apply_indic_reorder` that finds contiguous Indic
  runs of one script at a time and dispatches per-script reorder
  rules. The previous round-8 `apply_devanagari_reorder` is replaced.
- **Reph GSUB substitution wired (round 8 followup).** When the
  cluster machine flags `ClusterFlags::has_reph` AND the face
  assigned to the RA glyph publishes a `rphf` GSUB feature for the
  active script, `Font::gsub_apply_lookup_type_1` is applied to the
  RA glyph; on success, the RA gid is replaced with the reph form
  and the halant glyph is removed from the run. Faces without an
  `rphf` lookup fall back to in-line RA + halant + base rendering
  (the round-8 behaviour). The substitution is back-to-front order
  so multiple reph clusters in one run don't shift the indexing of
  pending marks.

The implementations are clean-room readings of:

- Unicode 15.1 Standard Annex #15 (Indic syllabic categories).
- Microsoft OpenType Layout — *Creating and supporting OpenType
  fonts for Indic scripts* (Bengali / Tamil / Telugu / Gujarati /
  Gurmukhi / Kannada / Malayalam / Oriya).

No HarfBuzz / FreeType / pango / ICU layout source consulted.

Test coverage:
- `shaping::indic::tests` — 21 new unit tests covering Bengali +
  Tamil categorisation, per-script pre-base matra reorder, Bengali
  reph identification, Tamil reph-disabled assertion, per-script
  feature-tag lists, and `script_indic_tags` mapping. Total
  `shaping::indic` test count: 49 (up from 26).
- `face_chain::tests` — 7 new unit tests covering the multi-script
  pre-cmap pass: Bengali pre-base matra reorder, Bengali reph mark
  with the right script tag, Tamil pre-base reorder, Tamil
  no-reph-mark assertion, Devanagari reph mark indexing (with and
  without a coexisting pre-base matra reorder), and a mixed
  Devanagari + Bengali run that segments cleanly at the script
  boundary.
- `tests/round10_bengali_cluster.rs` /
  `tests/round10_tamil_cluster.rs` — integration tests skip with
  `eprintln!` on the current vendored fonts (DejaVuSans does not
  cover Bengali / Tamil), exactly mirroring the round-8 Devanagari
  pattern. Activates once `NotoSansBengali-Regular.ttf` (~290 KB,
  OFL) / `NotoSansTamil-Regular.ttf` (~330 KB, OFL) lands in
  `samples.oxideav.org/fonts/` via the `font_fixtures` helper.

Followup tasks deferred:
- **Devanagari fixture font on the CDN.** Activating the round-8
  `ki_cluster_reorders_pre_base_matra_before_base` integration test
  needs `NotoSansDevanagari-Regular.ttf` (~280 KB, OFL) on
  `samples.oxideav.org/fonts/`. The `font_fixtures` helper plumbing
  is in place — adding the fixture is one entry in the `pub const`
  table plus a `tests/round8_devanagari_cluster.rs` switch from
  `include_bytes!` to `load_fixture(..)`. Same for the round-10
  Bengali / Tamil tests.
- **Remaining Indic scripts.** Telugu (U+0C00..U+0C7F — split
  vowels including U+0C46 + U+0C56), Gujarati (U+0A80..U+0AFF —
  closest to Devanagari), Gurmukhi (U+0A00..U+0A7F), Kannada
  (U+0C80..U+0CFF), Malayalam (U+0D00..U+0D7F), Oriya
  (U+0B00..U+0B7F). Each is a per-script categorisation table +
  `ReorderRules` constant.
- **Other GSUB features (`pref`, `pres`, `blwf`, `blws`, `half`,
  `cjct`, etc.).** Round 10 wires only `rphf`. The remaining
  Indic substitution features follow the same gsub_features_for_script
  + `gsub_apply_lookup_type_1` pattern but need cluster-position
  awareness (e.g. `half` only applies to non-final consonants).

### Added — variable-font axis selection in shaping (round 9)

scribe now lets callers shape a run against a specific
`fvar`-axis-coord vector, so a single Inter Variable / Roboto Flex /
Source Sans 3 VF font can be rendered at e.g. `wght=600 / wdth=125`
without loading a separate static cut. The path-output reflects the
gvar deltas applied at the chosen coords.

- `Face::is_variable()` / `Face::variation_axes()` /
  `Face::named_instances()` / `Face::variation_coords()` /
  `Face::set_variation_coords(coords)` / `Face::clear_variation_coords()`
  — variation-coord state lives on `Face`. The vector is stored
  alongside the owned font bytes and re-applied on every
  `Face::with_font` re-parse so subsequent `glyph_path` /
  `glyph_node` / `glyph_advance` lookups see the gvar-blended
  outline. `set_variation_coords` round-trips through the underlying
  parser to preserve its `[min, max]` clamp + per-axis length cap.
- `FaceChain::set_variation_coords(coords)` /
  `FaceChain::variation_axes(face_index)` /
  `FaceChain::named_instances(face_index)` /
  `FaceChain::face_mut(idx)` — chain-level mirrors. The setter
  targets the **primary** face only; fallback faces typically cover a
  different script and are loaded from a static cut, so flipping a
  fallback's coords requires `chain.face_mut(idx).set_variation_coords(..)`
  explicitly.
- `Shaper::with_variation_coords(Vec<f32>) -> ShaperBuilder` —
  per-call override builder. `ShaperBuilder::shape` /
  `ShaperBuilder::shape_to_paths` install the coords on the primary
  face for the duration of the call, run the shape, then restore the
  pre-call coord vector (or clear it when the chain had never had
  any). The static `Shaper::shape` / `Shaper::shape_to_paths` entry
  points are unchanged — they continue to use whatever coords are
  currently installed on the chain (axis defaults if none).
- `Shaper::named_instances(face_chain, face_index)` — convenience
  pass-through accessor that returns `Vec<NamedInstance>` for the
  chosen face. Each `NamedInstance` carries a `coords` vector that
  matches the face's `variation_axes` one-to-one; callers pick the
  vector that defines "Regular" / "Bold" / etc and pass it to
  `Face::set_variation_coords` or `Shaper::with_variation_coords`.
  Resolving the human-readable subfamily label requires reading the
  `name` table directly via `Face::with_font` — scribe deliberately
  doesn't surface a `name_id → string` accessor (the underlying ttf
  `Font::name` field is private).
- `oxideav_ttf::VariationAxis` and `oxideav_ttf::NamedInstance` are
  re-exported from the crate root so callers don't need to depend on
  `oxideav-ttf` directly.

The implementation is a clean-room consumer of `oxideav-ttf`'s
existing `fvar` / `avar` / `gvar` stack (which was a clean-room
implementation of OpenType 1.9 §6.2 / §6.3 / §6.5). No HarfBuzz /
FreeType / fontTools / Skia variable-font source consulted.

Integration test (`tests/round9_variable_font.rs`) loads
`InterVariable.ttf` (vendored OFL fixture) and verifies:
- Axes / named-instance counts match Inter (2 axes, 9 instances).
- `with_variation_coords([opsz_default, 400.0]).shape_to_paths(..)`
  vs `[opsz_default, 900.0]` produces glyph outlines whose
  per-coordinate points differ (gvar deltas applied).
- The chain's pre-call coord vector is restored after the builder
  returns (empty stays empty).
- A `Shaper::named_instances` lookup picks out the instance whose
  coords match every axis default (the canonical "Regular").
- `Face::set_variation_coords` clamps below-min / above-max coords
  to each axis's `[min, max]` and leaves in-range values untouched.
- Shaping against the explicit axis-default coords reproduces the
  static `Shaper::shape_to_paths` output bit-exactly.

Followup tasks deferred:
- Variable-CFF2 / OTF support — `Face::set_variation_coords` rejects
  OTF faces (`WrongFaceKind`) until `oxideav-otf` exposes a CFF2
  variation pipeline.
- Variable-font *metrics* (`MVAR`, `HVAR`, `VVAR`, `STAT`) —
  `Face::ascent_px` / `descent_px` / `line_height_px` /
  `glyph_advance` (via the shaper) currently use `head` / `hhea` /
  `OS/2` static values. Once `oxideav-ttf` exposes `MVAR` / `HVAR`
  blending, scribe will route these through the same coord vector.
- Per-fallback-face variation coords convenience — currently
  callers reach through `FaceChain::face_mut(idx)`.

Test fixture: `tests/fixtures/InterVariable.ttf` — a copy of the
same Inter Variable cut vendored by `oxideav-ttf/tests/fixtures/`,
plus the matching `INTER-OFL-LICENSE.txt`. ~843 KB.

### Added — Devanagari complex-script shaping (round 8)

New `shaping::indic` module covering Devanagari (Hindi / Marathi /
Sanskrit / Nepali) cluster-based shaping:

- `shaping::indic::IndicCategory` + `devanagari_category(char)` —
  Devanagari syllabic categorisation derived from the Unicode
  `IndicSyllabicCategory.txt` + `IndicPositionalCategory.txt`
  properties, condensed to the categories the cluster machine
  distinguishes (Consonant, Vowel, Halant, PreBaseMatra, Matra,
  Nukta, Bindu, Symbol, Other).
- `shaping::indic::cluster_boundaries(&[char]) -> Vec<(usize, usize)>`
  — segments the input into Devanagari orthographic clusters.
  Halant glues the next consonant into the same cluster (forming a
  conjunct); independent vowels and consonants without a preceding
  halant start new clusters; danda + non-Indic codepoints always
  form cluster boundaries.
- `shaping::indic::reorder_cluster(&[char]) -> (Vec<char>, ClusterFlags)`
  — applies the round-8 cluster transformations: pre-base matra
  reordering (U+093F moves to the front of the cluster, ahead of its
  base consonant) and reph identification (a leading
  RA + halant + consonant sets `ClusterFlags::has_reph`).
- `shaping::indic::devanagari_feature_tags()` — the spec-mandated
  Devanagari OpenType feature tag list in application order
  (`locl`, `ccmp`, `nukt`, `akhn`, `rphf`, `blwf`, `half`, `vatu`,
  `cjct`, `init`, `pres`, `abvs`, `blws`, `psts`, `haln`).
  Returned as `Vec<[u8; 4]>` so the future GSUB feature-lookup pass
  can iterate without re-deriving the order.
- `Script::Devanagari` — added to `shaping::arabic::Script`. The
  shared `script_of(char)` classifier now returns it for the
  U+0900..U+097F block; `feature_tags_for_run(Script::Devanagari)`
  returns the Devanagari feature-tag list.
- `FaceChain::shape` (and `shape_styled`) gained a second pre-cmap
  shaping pass that runs after the round-7 Arabic substitution.
  Devanagari runs are segmented into clusters and each cluster's
  characters are reordered into visual order before face-chain cmap
  lookup. A simple cmap-only Devanagari font therefore renders
  "कि" (KA + sign-i) with the matra correctly placed visually before
  the base consonant — without needing feature-tagged GSUB lookups.

The reph glyph substitution (the `rphf` GSUB feature) is *flagged*
but not yet emitted; the substitution requires `oxideav-ttf` to
expose feature-tagged GSUB lookup type 1 (single substitution),
which is being implemented in parallel. Once that lands, scribe will
consume the `has_reph` flag plus the `devanagari_feature_tags()`
feature list to drive the substitution.

The implementation is a clean-room reading of:

- Unicode 15.1 Standard Annex #15 (Indic syllabic categories)
- Unicode 15.1 Standard Annex #29 (text segmentation)
- Microsoft OpenType Layout — *Creating and supporting OpenType
  fonts for the Devanagari script*

No HarfBuzz / FreeType / pango / ICU layout source consulted.

Integration test (`tests/round8_devanagari_cluster.rs`) is in place
but *skips* on the current vendored fonts — DejaVuSans does not
cover the Devanagari block. Adding `NotoSansDevanagari-Regular.ttf`
(OFL-licensed; ~280 KB) to the `samples.oxideav.org/fonts/`
network-fetch CDN via the `font_fixtures` helper will activate the
test. Unit tests cover the cluster machine + reorder
exhaustively (26 new tests in `shaping::indic::tests` plus 5 in
`face_chain::tests`).

Followup tasks deferred:
- Other Indic scripts (Bengali U+0980..U+09FF, Gurmukhi U+0A00..U+0A7F,
  Gujarati U+0A80..U+0AFF, Tamil U+0B80..U+0BFF, Telugu U+0C00..U+0C7F,
  Kannada U+0C80..U+0CFF, Malayalam U+0D00..U+0D7F) — same broad
  cluster-machine shape but per-script categories, pre-base
  reordering matras, and feature lists.
- Reph glyph substitution via `rphf` GSUB once `oxideav-ttf`
  exposes feature-tagged GSUB lookup type 1.
- Vertical text (`vert` / `vrt2` features for CJK).
- GPOS cursive attachment (`curs` feature) — needed for proper
  Arabic / Devanagari mark positioning beyond simple anchor delta.

### Added — Arabic contextual joining (round 7)

New `shaping` module covering Arabic / Hebrew RTL contextual shaping:

- `shaping::arabic` — Unicode joining-class lookup (`JoiningClass::{U,
  L, R, D, C, T}`) covering the Arabic + Syriac + Arabic Supplement +
  Arabic Extended-A blocks plus combining-mark transparents, and the
  joining-adjacency state machine `compute_forms(&[char]) ->
  Vec<JoiningForm>` that picks `Isol` / `Init` / `Medi` / `Fina` per
  character (transparent marks inherit the form of the preceding base;
  ZWJ acts as joining-causing; ZWNJ breaks the chain).
- `shaping::arabic::script_of(char) -> Script` and
  `feature_tags_for_run(Script) -> Vec<[u8; 4]>` — script
  classification + the OpenType feature-tag list for a run (Arabic gets
  `isol` / `init` / `medi` / `fina`).
- `shaping::arabic_pf::presentation_form(base, JoiningForm) ->
  Option<char>` — translates an Arabic base codepoint + chosen form
  into its Arabic Presentation Forms-B equivalent (U+FE70..U+FEFF) per
  the UCD `UnicodeData.txt` `<initial>` / `<medial>` / `<final>` /
  `<isolated>` decomposition tags.
- `FaceChain::shape` (and `shape_styled`) now run a pre-cmap shaping
  pass that finds Arabic codepoint runs, picks the contextual form per
  letter, and rewrites to the PF-B equivalent before face-chain cmap
  lookup. The shaper's existing GSUB ligature pass then collapses
  LAM-medi + ALEF-fina into the LAM-ALEF FINAL ligature (U+FEFC) for
  fonts that ship it (DejaVuSans does). Faces that lack a PF-B glyph
  fall back to the original base codepoint — the shaper degrades to
  the round-6 isolated-form behaviour rather than emitting `.notdef`.

The substitution is a clean-room implementation of the algorithm
described in Unicode core specification §9.2 and the OpenType layout
spec for the four Arabic feature tags. No HarfBuzz / FreeType / pango
/ ICU layout source consulted.

Integration test (`tests/round7_arabic_joining.rs`) shapes "السلام"
against DejaVuSans and asserts the resulting glyph IDs match the
PF-B + LAM-ALEF-ligature gids, demonstrably differing from the naive
isolated-form lookup.

Followup tasks deferred:
- Full feature-tagged GSUB lookup table (lookup type 1 single
  substitution) — needed for fonts that don't ship the PF-B block but
  do ship feature lookups (e.g. modern Noto Sans Arabic UI).
- Mark reordering + GPOS mark-attachment per Arabic shaper rules.
- Indic complex-script shaping (round 8 candidate).

### Removed — vector-only refactor (#354)

Scribe is now a pure vector shaper. All pixel-pipeline code moved to
[`oxideav-raster`](https://github.com/OxideAV/oxideav-raster); consumers
that need a rasterised text run should call `Shaper::shape_to_paths`,
wrap the resulting nodes in a `VectorFrame`, and hand it to
`oxideav_raster::Renderer::render`.

Removed modules: `cache`, `compose`, `outline`, `rasterizer`, `stroke`.

Removed public APIs:
- `render_text` / `render_text_styled` / `render_text_wrapped`
- `Composer` / `Composer::compose_run` / `compose_run_styled` /
  `compose_run_with_stroke` / `with_capacity` / `cache` accessors
- `StrokeStyle`
- `Rasterizer` / `Rasterizer::raster_glyph` / `raster_glyph_styled` /
  `raster_glyph_subpixel` / `glyph_offset*`
- `AlphaBitmap`
- `RgbaBitmap` (the public re-export from the crate root — the type
  itself still lives at `oxideav_scribe::color_glyph::RgbaBitmap` as
  the carrier for the CBDT decode path; vector consumers will not need it)
- `outline::flatten` / `flatten_with_shear` / `flatten_with_shear_offset` /
  `flatten_cubic` / `flatten_cubic_with_shear` / `FlatOutline` / `FlatBounds`
- `cache::GlyphKey` / `GlyphCache` / `CachedGlyph` /
  `subpixel_slot` / `subpixel_offset` / `SUBPIXEL_STEPS`
- `stroke::dilate_alpha` / `dilate_offset`
- `style::synthetic_bold_radius` /
  `style::SYNTHETIC_BOLD_THRESHOLD` /
  `style::SYNTHETIC_BOLD_PX_PER_WEIGHT_STEP_PER_PX`
- `face_chain::shear_for`

Removed dependency: `oxideav-pixfmt` (the alpha-blit responsibility
moves to the rasterizer).

`Style` keeps `weight: u16` for downstream rasterizers wanting to
synthesise bold; scribe itself no longer applies bold dilation.

## [0.1.4](https://github.com/OxideAV/oxideav-scribe/compare/v0.1.3...v0.1.4) - 2026-05-04

### Other

- bilinear-resample + composer dispatch for CBDT colour glyphs ([#356](https://github.com/OxideAV/oxideav-scribe/pull/356))

## [0.1.3](https://github.com/OxideAV/oxideav-scribe/compare/v0.1.2...v0.1.3) - 2026-05-04

### Other

- wrap shape_to_paths glyphs in cache-keyed Group ([#357](https://github.com/OxideAV/oxideav-scribe/pull/357))
- implement trapezoidal horizontal coverage for sub-pixel AA
- re-enable round4 double-diacritic, assert acute < circumflex
- re-enable round7 path test, switch glyph 'A' -> 'O'

### Added

- `Face::stable_id()` — content-derived face identity (DefaultHasher
  digest of the leading sfnt bytes + length + subfont index) that is
  stable across loads of the same font bytes. Distinct from the
  per-process `Face::id()` counter, this is the right input for any
  cache key persisted across renderer instances or program runs.
- `Shaper::shape_to_paths` now wraps each emitted glyph node in a
  `Node::Group { cache_key: Some(_), children: vec![glyph_node], .. }`
  carrier with a deterministic `cache_key` derived from
  `(face_stable_id, glyph_id, size_q8)`. Combined with oxideav-raster's
  composite-key bitmap cache (which mixes the producer key with the
  effective transform), this lets the rasterizer reuse the same
  bitmap for repeated glyph instances across a run, across renderers,
  and across program restarts. Closes #357.

### Changed

- Bumped `oxideav-core` minimum to `0.1.15` for the `Group::cache_key`
  field used by the new `shape_to_paths` wrapper.
- `tests/round7_shape_to_paths.rs`: updated to expect the new
  `Node::Group { cache_key: Some(_), children: [PathNode] }`
  envelope around each glyph instead of the raw `Node::Path`.

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

### Changed

- Rasterizer: replaced binary `floor`/`ceil` horizontal scanline fill
  with **analytical trapezoidal coverage**. Each non-vertical active-
  edge pair now contributes per-pixel fractional coverage
  `clamp(min(x1, px+1) - max(x0, px), 0, 1)` (mapped to a 0..=255
  byte), accumulated into the supersample buffer instead of a hard
  `0`/`1`. The 4× vertical supersample is unchanged. Effect: 16
  sub-pixel x-slots now produce 16 visually distinct glyph bitmaps
  (previously they collapsed to ~2 because horizontal edges were
  binary). Re-enables `round3_subpixel::different_slots_produce_*` and
  `each_slot_produces_a_distinct_bitmap`.
- `outline::flatten_with_shear_offset`: consolidated bbox computation
  so the bitmap dimensions are *independent* of `x_subpixel`. The
  bitmap left edge is always `floor(raw.x_min * scale)` and the bitmap
  right edge always reserves a 1-px slack column for the trapezoidal
  rightmost partial-coverage. Without this every sub-pixel slot would
  pick a different bitmap width as the silhouette spilled across pixel
  boundaries. Glyph bitmaps are typically 1 px wider than before
  (round-2 callers see at most a 1-column right-edge zero pad).

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
