# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added — `layout::shape_paragraphs`: multi-paragraph document layout (round 374)

The document-level counterpart to `wrap_and_shape_lines`. It splits the
text on UAX #9 bidi-class-`B` **paragraph separators** (`bidi::
split_paragraphs` — LF, CR, CRLF, NEL `U+0085`, `U+2029`), resolves
**each paragraph's own** base direction (P1 / P2 / P3 via
`bidi::paragraph_level`, unless the caller forces a uniform
`base_level`), and wraps + bidi-shapes every paragraph independently —
returning one `ShapedParagraph { lines, base_level }` per source
paragraph. This honours UAX #9 P1 (each paragraph is an independent
bidirectional unit), so a Hebrew paragraph followed by an English one
each resolve their own direction, and recognises the full B-class
separator set rather than the ASCII `'\n'` that `wrap_lines` splits on.
LINE SEPARATOR `U+2028` is bidi-class `WS` (a line break within a
paragraph), so it does not start a new paragraph. Tests:
`tests/round374_shape_paragraphs.rs` (newline / U+2029 / NEL splits,
U+2028 non-split, per-paragraph direction, uniform override, wrap-to
-width, empty-paragraph blank line, single-paragraph equivalence).

### Added — `layout::wrap_and_shape_lines`: wrap + bidi-shape in one call (round 374)

The one-call path from a paragraph of logical text to width-wrapped,
bidi-ordered, render-ready visual lines. Composes `wrap_lines` (the
width-constrained break finder) with `shape_visual_line` (per-line
UAX #9 reorder + shape), returning one `ShapedVisualLine` per display
line in top-to-bottom reading order. `base_level` is applied to every
line, so a caller wrapping RTL text passes `Some(1)` to keep every
visual line on the paragraph's base level rather than flipping per-line
on a line that starts with a Latin token. Line-breaking stays
direction-agnostic (the existing whitespace / hard-break policy on the
logical text); UAX #14 break-class breaking remains a separate future
feature. Tests: `tests/round374_wrap_and_shape.rs` (composition equals
wrap + per-line shape, every line fits the width budget, RTL base
override, hard-newline path).

### Added — bidi-shaped visual line: `layout::shape_visual_line` (round 374)

The crate had a complete UAX #9 reordering pipeline and a complete
OpenType shaper, but the two never met: `reorder_line_visual` reordered
*characters* and handed them back, leaving the caller to shape and
arrange runs. `layout::shape_visual_line(chain, text, size_px,
base_level)` closes the gap. It runs the §3 paragraph pipeline + §3.4 L1
reset, partitions the line into BD7 **level runs**, shapes each run's
*logical* substring through `FaceChain::shape` (so ligatures, Arabic
joining, and Indic clustering see the natural character order), reverses
each RTL (odd-level) run's glyph sequence, and concatenates the runs in
§3.4 L2 **visual** order — returning a `ShapedVisualLine` whose `glyphs`
a renderer paints left-to-right with the pen advancing normally.
`ShapedVisualLine::width()` reports the laid-out advance. Shaping in
logical order (rather than over the pre-mirrored visual char stream
`reorder_line_visual` produces) is what keeps RTL ligatures / joining
correct. Tests: `tests/round374_shape_visual_line.rs` (pure-LTR matches
direct shaping, pure-RTL glyph reversal, LTR-then-RTL run ordering, RTL
base with a Latin island, width sum, base-level override).

### Added — positioned caller-feature shaping: GSUB features through the full GPOS pass (round 362)

The caller-driven GSUB feature surface (`Face::shape_text` and friends)
previously stopped after substitution and returned bare glyph IDs — a
caller requesting `smcp` / `frac` / `sups` / a stylistic set got the
reshaped glyphs but no advances, kerning, mark attachment, or contextual
positioning. The always-on `Shaper::shape` pipeline ran the full GPOS
pass but only over the hard-coded `ccmp` / `liga` / `calt` set, so the
two halves never met.

Round 362 bridges them. The GPOS positioning half of `shape_run_with_font`
is factored into a reusable `shaper::position_run_with_font(font, gids,
component_counts, scale, face_idx)`, and a positioned caller-feature
surface is built on top:

- `Face::position_text(text, size_px, features)` and the explicit-script
  `Face::position_text_with_script` mirror — single-face shaping with
  caller features, returning `PositionedGlyph`s.
- `FaceChain::shape_with_features(text, size_px, features)` — multi-face
  fallback variant; reuses the chain's per-codepoint face assignment plus
  Arabic-joining / Indic-cluster pre-shaping, then positions each per-face
  run with the requested features.
- `shaping::{position_text_with_features_with_font,
  position_text_with_script_and_features_with_font,
  position_text_with_alternates_with_font,
  position_gids_with_features_with_font}` — the lower-level entry points.

The substituted run flows through pair kerning, SinglePos (type 1),
mark-to-base / mark-to-mark / mark-to-ligature attachment (types 4 / 6 /
5), cursive attachment (type 3), and contextual / chained-contextual
positioning (types 7 / 8). Ligature component counts are tracked through
the Type-4 collapse so mark-to-ligature attachment targets the right
component; length-changing contextual rewrites reset the tally to
single-component (graceful mark-to-base fallback). 14 new tests (6 lib +
8 `round362_positioned_features.rs`).

### Added — GSUB contextual / chained / reverse-chained substitution on the caller-driven path (round 353)

`Face::shape_text` and its siblings (`shape_text_with_script`, the
alternate-index variants — all bodied by
`shaping::feature_subst::shape_text_inner`) now dispatch GSUB
**LookupType 5 (Contextual)**, **LookupType 6 (Chained Contexts)**, and
**LookupType 8 (Reverse Chaining Contextual Single)** when a requested
feature references them. Previously these were silently skipped on the
caller-driven surface (only the always-on `ccmp` / `calt` passes covered
them), so a caller explicitly asking for e.g. `calt` or `frac` through
`shape_text` got a contextual no-op. Types 5/6 flow through a new
`apply_context_lookup` left-to-right scan (bounded against a self-feeding
rewrite); type 8 flows through `apply_reverse_chain_lookup`, which walks
the buffer right-to-left per the GSUB chapter's reverse-processing
requirement.

### Fixed — always-on GSUB type-8 reverse-chaining now processes right-to-left (round 353)

`shaping::general::apply_one_lookup` (the always-on `ccmp` / `calt`
walker behind `Shaper::shape`) dispatched LookupType 8 left-to-right, a
documented deferral that could fire a rule on a not-yet-substituted
lookahead glyph. It now uses the same back-to-front walk as the
caller-driven path.

Validated by `tests/round353_contextual_subst.rs` — synthetic TTFs
(`SequenceContextFormat3` / `ChainedSequenceContextFormat3` /
`ReverseChainSingleSubstFormat1`) proving each type fires on its match,
is inert without its context, and (type 8) processes right-to-left, plus
real-fixture Inter smoke tests.

### Added — GPOS contextual + chained-contextual positioning (round 343)

The run-level positioning pass now applies GPOS **LookupType 7
(Contextual Positioning)** and **LookupType 8 (Chained Contexts
Positioning)** — the positioning analogues of the GSUB contextual /
chained-contextual substitution lookups. A new
`shaping::contextual_pos` module enumerates the font's type-7/8 GPOS
lookups in LookupList order, scans every input position, and
accumulates the per-glyph `PosRecord` deltas the dependency's
`gpos_apply_lookup_type_7` / `gpos_apply_lookup_type_8` resolve from
the sub-table match plus the nested `SequenceLookupRecord` recursion.
Field mapping mirrors the SinglePos (LookupType 1) pass —
`xPlacement → x_offset`, `yPlacement → -y_offset` (TT Y-up → raster
Y-down), `xAdvance → x_advance`; `yAdvance` is vertical-layout only and
ignored. Records are additive (multiple records may stack on one
glyph). The pass runs last (step 7), so contextual rules see the
post-kern / post-mark / post-cursive geometry, and is gated on the font
publishing at least one type-7/8 lookup so plain Latin faces pay a
single lookup-list scan. Validated by synthetic-`PosRecord` field-map
unit tests (Y-flip, additive stacking, out-of-range guard,
`yAdvance`-ignored) plus integration tests proving the pass is wired
into `shape` / `shape_to_paths` and is a transparent identity for the
fixtures (which ship no contextual-positioning lookups). This closes
the "more elaborate contextual GSUB/GPOS lookups are explicitly
deferred" caveat on the GPOS side.

### Added — `post` (PostScript) table glyph-name resolution (round 324)

A new `post` module resolves a glyph ID to its PostScript glyph name.
It carries the 258 standard Macintosh glyph names in canonical ordering
(`STANDARD_MAC_GLYPH_NAMES` / `standard_mac_glyph_name`) and parses the
three name-bearing `post` table layouts: format 1.0 (the implied
standard ordering), format 2.0 (`glyphNameIndex` selecting either a
standard name `< 258` or a custom Pascal string `>= 258`), and the
deprecated format 2.5 (a signed per-glyph delta into the standard
ordering). Format 3.0 (no names) parses but reports `has_names() ==
false`. `Face::post()` returns the parsed table for the active subfont
(TTC-aware), and `Face::glyph_name(gid)` is a one-shot convenience.
Validated against the real DejaVuSans `post` table (`A` → "A",
`a` → "a", space → "space", GID 0 → ".notdef") plus synthetic
format-1.0/2.0/2.5/3.0 fixtures.

### Changed — UCD `Bidi_Class` / `Bidi_Mirroring_Glyph` now sourced from the `intl` crate (round 319)

The UAX #9 engine's `Bidi_Class` and `Bidi_Mirroring_Glyph` property
lookups are now delegated to the Karpelès Lab `intl` crate (a pure-Rust
internationalization library) instead of parsing the vendored
`DerivedBidiClass.txt` /
`BidiMirroring.txt` snapshots at runtime. `intl` ships the UCD tables as
compiled const-fn lookups, so the per-call `OnceLock` file-parse on
first use is gone. The §3.2 `@missing` defaults for unassigned code
points (strong `R` / `AL` in the right-to-left script blocks, `ET` in
the Currency Symbols block) are overlaid locally from the fixed
published block list, since `intl` returns `L` for unassigned slots.
This makes the lookup byte-for-byte identical to the previous parser
for every assigned code point (verified exhaustively against the 16.0
`DerivedBidiClass.txt` / `BidiMirroring.txt` snapshots — zero diffs),
while restoring the §3.2 unassigned-block defaults. The
`Bidi_Paired_Bracket` table (`BidiBrackets.txt`) stays vendored because
`intl` does not expose bracket-pair data. The two now-dead UCD `.txt`
files were removed from `src/bidi/`.

### Fixed — pair kerning now adjusts the first glyph's advance (round 319)

GPOS PairPos (LookupType 2) / legacy `kern` pair adjustments are, per
OFF §6.4 (ISO/IEC 14496-22:2019), a change to the **xAdvance of the
first glyph in the pair** — the spec's worked Format 2 example states a
pair is "kerned by reducing the XAdvance of the first glyph." The shaper
previously applied the kern as an `x_offset` on the *right* glyph, which
moved only that one glyph and leaked the adjustment: every glyph
downstream of a kerned pair was placed from the unkerned advance, so the
kern never propagated past the immediate pair and `layout::run_width`
over-counted. The kern is now applied to the left glyph's `x_advance`,
so the pen accumulates it and the whole run shifts correctly. Two
`dejavu_render` tests cover the propagation (a regression guard
asserting the left glyph's advance shrinks and `x_offset` stays 0 for
pure kerning) and the AVATAR run width against per-letter un-kerned
baselines.

## [0.1.9](https://github.com/OxideAV/oxideav-scribe/compare/v0.1.8...v0.1.9) - 2026-06-15

### Other

- refresh to current status, drop per-round changelog cruft
- GPOS LookupType 5 mark-to-ligature attachment (round 304)
- wire GPOS LookupType 1 single adjustment positioning (round 298)
- round 291 — reorder_line_visual high-level bidi bridge
- UAX #9 lookups data-driven from Unicode 16.0 UCD tables (round 283)
- GPOS LookupType 3 cursive attachment (round 276)
- UAX #9 §3.4 L4 bidi mirroring (round 268)
- UAX #9 §3.3.5 N0 bracket-pair resolution (round 257)
- drop release-plz.toml — use release-plz defaults across the workspace
- UAX #9 §3.4 L3 combining-mark reordering (round 247)
- UAX #9 §3.3.1 P1 multi-paragraph driver (round 233)
- r227 test-count correction (14 → 19 unit)
- UAX #9 §3 whole-paragraph driver (round 227)
- UAX #9 §3.3.3 X10 isolating-run-sequence partition + sos/eos (round 220)
- UAX #9 §3.3.2 explicit-level / override / isolate stack X1..X9 (round 217)
- UAX #9 line-level reordering rules L1 + L2 (round 210)
- UAX #9 implicit-level resolution rules I1 + I2 (round 204)

### Added — GPOS LookupType 5 mark-to-ligature attachment (round 304)

The shaper's mark-attachment pass now positions combining marks
against **ligature** base glyphs via GPOS LookupType 5 (MarkLigPos).
Per the OpenType GPOS chapter, a ligature carries "multiple components
(in a virtual sense — not actual glyphs)", each with its own per-class
attachment anchors, and the component a mark associates with "is
dependent on the original character string and subsequent ...
glyph-substitution operations". The shaper recovers that association
from its own ligature-collapse pass: each output glyph now records how
many input glyphs collapsed into it (the component count). When a mark
walks back to a multi-component ligature base, the mark-to-ligature
path runs first — the trailing mark is associated with the **last**
component by default (the "fi + dot-above" case, where the dot
followed the second component), and the probe walks down toward
component 0 so a mark still lands on whichever component publishes a
non-NULL class anchor. A ligature that ships no LookupType-5 anchor
for the mark falls back to mark-to-base; a non-ligature base keeps the
existing mark-to-base path unchanged. Component tracking is reset to
all-single-component when `calt` reshapes the run (a length change
breaks positional alignment), so the type-5 path simply does not fire
on a calt-mutated run rather than mis-indexing. 6 integration tests
build a synthetic TTF (GSUB ligature + GDEF mark class + GPOS
MarkLigPos) and cover last-component attachment, size scaling, the
NULL/out-of-range-component fallback to component 0, the
no-ligature-no-type-5 case, the no-GPOS no-op, and that plain
ligature substitution is undisturbed.

### Added — GPOS LookupType 1 single adjustment positioning (round 298)

The shaper's positioning pipeline now runs a GPOS LookupType 1
(SinglePos) pass between kerning and mark attachment. Per the OpenType
GPOS chapter a SinglePos subtable "is used to adjust the placement or
advance of a single glyph, such as a subscript or superscript"; both
SinglePosFormat1 (one shared `ValueRecord` for every covered glyph)
and SinglePosFormat2 (a per-glyph `ValueRecord` array) are honoured.
The four `ValueRecord` geometric fields map onto a positioned glyph
as: `xPlacement` adds to `x_offset`, `yPlacement` subtracts from
`y_offset` (TT Y-up → raster Y-down), and `xAdvance` widens or
narrows the horizontal advance; `yAdvance` (vertical layout only) is
ignored on the horizontal pen. The pass is gated on the font
publishing a LookupType-1 lookup, so plain fonts pay only one
lookup-list scan, and runs before mark attachment so the
mark-to-base advance accumulation sees the adjusted base advances.
Coverage misses leave the glyph untouched. 6 integration tests cover
Format 1 shared-value application, Format 2 per-glyph independence,
size scaling, additive stacking with the kern `x_offset`, the
no-GPOS no-op, and the uncovered-glyph pass-through.

### Added — high-level bidirectional layout bridge (round 291)

`layout::reorder_line_visual(text, base_level) -> VisualLine` drives
the complete UAX #9 §3 + §3.4 pipeline (P → X → W → N0 → N1 / N2 →
I → L1 → L2 → L3 → L4) over a single display line and returns its
characters in left-to-right visual order, ready to feed glyph-by-glyph
into the shaper. This is the first time the `layout::*` API drives the
bidi engine automatically — previously callers wired the per-character
permutation into their own renderer.

The `VisualLine` carrier publishes `visual: Vec<char>` (L4-mirrored,
render-order), the `logical_to_visual` / `visual_to_logical`
permutation pair (the inverse precomputed for O(1) cursor
hit-testing), and the resolved `base_level`. `base_level: Option<u8>`
is the HL1 higher-level-protocol override (`Some(0)` LTR, `Some(1)`
RTL, `None` lets P2 / P3 resolve from the first strong character). The
internal composition runs `process_paragraph_classes_with_brackets`
(bracket-aware N0) followed by `reset_trailing_levels` (L1),
`reorder_line` (L2), `reorder_combining_marks` (L3) and
`apply_mirroring` (L4); the full per-rule `bidi::` surface stays public
for callers needing finer control. 9 unit + 12 integration tests cover
LTR identity, pure-RTL reversal, embedded-island ordering in both
directions, L4 bracket mirroring in RTL vs LTR context, digits staying
LTR in an RTL line, Arabic-letter base resolution, the HL1 override,
the permutation bijection / inverse-consistency / hit-test round-trip
invariants, and the §3.4 "car means CAR." worked example.

### Added — UAX #9 Unicode 16.0 UCD data tables (round 283)

The three bidi property lookups are now data-driven from the Unicode
16.0 UCD snapshots staged under `docs/text/unicode-bidi/` and
vendored verbatim into `src/bidi/` (embedded via `include_str!`,
parsed once on first lookup into sorted binary-searched range
tables behind `OnceLock`s):

- **`bidi::bidi_class(c)`** — full per-code-point `Bidi_Class` from
  `DerivedBidiClass.txt` (2289 explicit ranges), replacing the
  previous hand-mapped block list. The file's `@missing` lines are
  honoured for unassigned code points: `R` / `AL` defaults in the
  blocks reserved for right-to-left scripts, `ET` in the Currency
  Symbols block, global `L` fallback elsewhere. ASCII punctuation
  such as `!` / `"` that previously fell through to the `L` default
  now resolves to its UCD class (`ON`), and noncharacters resolve
  to `BN`.
- **`bidi::paired_bracket(c)`** — full normative
  `Bidi_Paired_Bracket` / `Bidi_Paired_Bracket_Type` table from
  `BidiBrackets.txt` (64 open/close pairs: ASCII, Tibetan gug
  rtags, square-bracket-with-quill, mathematical, CJK / fullwidth),
  replacing the six-ASCII-bracket seed table from round 257. The
  BD16 walker's close-branch comparison now implements the "where
  U+3009 and U+232A are treated as equivalent"
  canonical-equivalence clause, so mixed U+2329/U+3009 and
  U+3008/U+232A bracket pairs are identified.
- **`bidi::mirrored_glyph(c)`** — full informative
  `Bidi_Mirroring_Glyph` table from `BidiMirroring.txt` (428
  entries — brackets, angle quotation marks, mathematical
  relations and operators, CJK brackets), replacing the
  six-ASCII-bracket seed table from round 268, so rule L4 now
  mirrors `<` ↔ `>`, `«` ↔ `»`, `≤` ↔ `≥`, `〈` ↔ `〉`, … at odd
  resolved levels.

4 unit tests pin the table-size / sortedness / involution /
open-close-symmetry / brackets-are-ON-and-mirrored invariants
straight off the parsed tables; 11 integration tests
(`round283_bidi_unicode_data_tables.rs`) cover explicit class
assignments across planes, the `@missing` fallbacks, noncharacter
BN, the BD16 canonical-equivalence pairings, and N0 + L4
end-to-end through `process_paragraph_with_brackets` with
non-ASCII brackets and mirrors. No public API change — the three
lookup signatures are unchanged.

### Added — GPOS LookupType 3 cursive attachment (round 276)

The shaper's positioning pipeline grows a sixth step: **cursive
attachment** (GPOS LookupType 3, CursivePosFormat1), the
exit-anchor → entry-anchor joining pass that connects adjacent
glyphs in joining scripts. Per the GPOS chapter's CursivePos
section, the two axes work differently and both are implemented:

- **Line-layout direction (X)** — "the layout engine adjusts the
  advance of the first glyph (in logical order)": the first glyph's
  `x_advance` is rewritten so the second glyph's entry anchor lands
  exactly on the first glyph's exit anchor, accounting for both
  glyphs' `x_offset`s and any intervening zero-advance marks.
- **Cross-stream direction (Y)** — "placement of one glyph is
  adjusted to make the anchors align": with the parent lookup's
  RIGHT_TO_LEFT flag clear, the **second** glyph's `y_offset` moves
  to the first glyph's exit height. Adjustments accumulate down a
  connected chain (each second glyph is placed relative to the
  already-adjusted first), producing the cascading-baseline
  behaviour cursive scripts need. Marks attached to the second
  glyph follow it vertically; no X fix-up is needed because the
  advance rewrite shifts the pen of the second glyph and its
  trailing marks equally.

NULL entry/exit anchor offsets skip the pair ("no positioning
adjustment is applied" per spec); fonts without any LookupType-3
lookup skip the pass entirely (one `gpos_lookup_list` probe per
run). The pass runs after mark attachment so cursive chains walk
consecutive **non-mark** glyphs. Scope note: the RIGHT_TO_LEFT
flag-**set** variant (first glyph adjusted cross-stream, chain
anchored to the last glyph's baseline position) needs the GPOS
lookup flag, which `oxideav-ttf`'s public API does not yet expose —
deferred.

Verified by `tests/round276_cursive_attachment.rs`: a synthetic
4-glyph TTF carrying one `curs` feature / CursivePosFormat1 lookup
(byte layout per the staged GPOS / Coverage / Anchor-Format-1 spec
tables) exercises exact anchor alignment, chain accumulation,
px-size scaling, NULL-anchor skips, and the no-GPOS no-op.

### Added — UAX #9 §3.4 L4 bidi mirroring (round 268)

Twelfth UAX #9 surface on scribe and the final entry in the §3.4
line-level pass: rule **L4**, the mirrored-glyph selection for
characters whose resolved directionality is R, shipped — like the
round-257 N0 pass — with the algorithm complete and the lookup
table seeded from the ASCII paired-bracket set pending the
`BidiMirroring.txt` vendoring.

- **`bidi::mirrored_glyph(c: char) -> Option<char>`** — the
  `Bidi_Mirroring_Glyph` acceptable-mirror-pair lookup for the six
  ASCII paired brackets (`(` ↔ `)`, `[` ↔ `]`, `{` ↔ `}`). The
  mapping is an involution (`mirrored_glyph(m) == Some(c)` whenever
  `mirrored_glyph(c) == Some(m)`) and agrees with the round-257
  `paired_bracket` lookup on the seed set. Per the §3.4 L4
  backward-compatibility note, U+FD3E / U+FD3F ORNATE LEFT / RIGHT
  PARENTHESIS are **not** mirrored and return `None`. The full
  mirrored-pair catalogue is the informative `BidiMirroring.txt`
  data file from the UCD, not yet vendored under
  `docs/text/unicode-bidi/`; callers needing the wider set
  (mathematical operators, angle brackets, CJK bracket blocks) can
  run the same per-position loop against their own
  `char -> Option<char>` lookup.
- **`bidi::apply_mirroring(chars: &mut [char], levels: &[u8])`** —
  rule **L4** applied in place to a line's logical character
  sequence: "A character is depicted by a mirrored glyph if and
  only if (a) the resolved directionality of that character is R,
  and (b) the Bidi_Mirrored property value of that character is
  Yes." Condition (a) is read off the resolved level vector (odd
  level ⇔ directionality R per the §3.2 even-LTR / odd-RTL level
  convention); condition (b) is the `mirrored_glyph` lookup, with
  the replacement realising the spec's "depicted by a mirrored
  glyph" requirement through the §7 acceptable-mirror-pair
  substitution. L4 is a per-position glyph selection independent
  of the L2 / L3 permutation; the documented composition applies
  it to the logical sequence and then walks the L2 permutation.
  Because the lookup is an involution, double application restores
  the input — callers run it exactly once per rendered line. The
  HL6 higher-level-protocol override is out of scope. Panics on
  `chars` / `levels` length mismatch.
- **`oxideav_scribe::mirrored_glyph`** +
  **`oxideav_scribe::apply_mirroring`** — public re-exports
  alongside the existing W / N / I / X / L surface.
- **13 unit tests + 19 integration tests**
  (`tests/round268_bidi_l4_mirroring.rs`) cover the seed-pair
  round-trips, the involution + paired-bracket agreement
  invariants, the ornate-parentheses exclusion, the §3.4 worked
  example (`(` even-level vs odd-level), mixed / higher-odd level
  dispatch, non-mirrored pass-through at odd levels, empty input,
  double-application restore, the length-mismatch panic, and
  end-to-end compositions through
  `process_paragraph_with_brackets` + L1 → L2 → L3 → L4 on RTL
  Hebrew lines carrying bracket pairs and combining marks
  (including the display-stream assertion that mirrored brackets
  open toward the enclosed text in visual order). Total scribe
  tests: 861 → 895.

Out-of-scope tail (`README.md`): the `BidiMirroring.txt`
mirrored-pair table needs to land under `docs/text/unicode-bidi/`
before `mirrored_glyph` can grow past the ASCII paired-bracket
seed set — same shape as the `BidiBrackets.txt` gap blocking the
wider N0 table. The L4 algorithm itself is complete; only the
lookup table is partial. With L4 landed, every UAX #9 rule scribe
tracks (P / X / W / N0 / N1 / N2 / I / L1 / L2 / L3 / L4) now has
an implementation; the remaining BiDi work is data-file vendoring
(`BidiBrackets.txt`, `BidiMirroring.txt`, `DerivedBidiClass.txt`)
plus wiring the pipeline into the high-level `layout::*` API.

Provenance: rule L4 and the §7 *Mirroring* acceptable-pair note are
transcribed verbatim from UAX #9 Revision 50 / Unicode 16.0
(`docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`).

### Added — UAX #9 §3.3.5 N0 bracket-pair resolution (round 257)

Eleventh UAX #9 surface on scribe and the first entry to land the
bracket-pair phase of the §3.3.5 neutral pass: rule **N0** with the
**BD14** / **BD15** / **BD16** support definitions, all wired through
a new paragraph-driver variant.

- **`bidi::paired_bracket(c: char) -> Option<(char, BracketKind)>`**
  — the BD14 / BD15 lookup for the six ASCII brackets (`(` ↔ `)`,
  `[` ↔ `]`, `{` ↔ `}`). Returns `Some((paired_char, kind))` where
  `kind` is `BracketKind::Open` or `BracketKind::Close`. The wider
  Unicode `BidiBrackets.txt` table is not yet vendored under
  `docs/text/unicode-bidi/`; callers needing the ~60-entry UCS pair
  set can wrap `bracket_pairs` / `resolve_bracket_pairs` with their
  own `(char, BracketKind)` lookup of the same shape.
- **`bidi::bracket_pairs(chars: &[char], classes: &[BidiClass]) ->
  Vec<(usize, usize)>`** — the **BD16** stack walk over one
  isolating run sequence. Maintains the spec-mandated 63-element
  stack (overflow → empty list per the BD16 "stop processing
  BD16 ... and return an empty list" branch), pairs nested
  brackets by popping through the matching opener inclusively,
  honours the BD14 / BD15 "current bidi class is ON" gate (so
  brackets rewritten to `R` by X6 / RLO are ignored), and sorts
  the result by opener position in ascending logical order — the
  §3.3.5 N0 sequencing invariant.
- **`bidi::resolve_bracket_pairs(classes: &mut [BidiClass],
  pairs: &[(usize, usize)], embedding_level: u8, sos: BidiClass)`**
  — the **N0** rule applied to one isolating run sequence post-W7
  and pre-N1. For each pair, inspects the bracket interior for a
  strong type (EN / AN projected to R per the §3.3.5 note), then
  dispatches the N0 a / b / c.1 / c.2 / d cases in place:
  matching-embedding-inside → both brackets to embedding direction
  (N0 b); opposite-inside + preceding-strong-also-opposite → both
  brackets to that direction (N0 c.1); opposite-inside +
  preceding-strong-matches-embedding → both brackets to embedding
  direction (N0 c.2); no inside-strong → leave the pair untouched
  (N0 d). The trailing-NSM clarification ("any NSM following a
  paired bracket which changed under N0 should change to match the
  bracket") is honoured for the contiguous NSM run after each
  rewritten bracket. Pairs are processed sequentially in opener-
  ascending order so inner pairs see the rewrites of every outer
  pair already processed.
- **`bidi::process_paragraph_classes_with_brackets(classes, chars,
  base_level) -> ParagraphBidi`** + **`bidi::process_paragraph_with_brackets(
  text, base_level) -> (ParagraphBidi, Vec<usize>)`** — §3 paragraph
  driver mirrors of the round-227 / round-233 entry points, with
  N0 wired in between W7 and N1 per the §3.3.5 ordering. The two
  driver variants without `_with_brackets` stay the legacy pass-
  through (N0 not run), so existing callers see no behaviour
  change; the `_with_brackets` variants become the recommended
  default for any caller that handles arbitrary user-supplied
  text containing brackets.
- **`bidi_class('(' / ')' / '[' / ']' / '{' / '}') == BidiClass::ON`**
  — added the six ASCII brackets to the `bidi_class` lookup
  table. They were previously falling through to the L default
  per the unassigned-character convention, which would have made
  every bracket appear strong-L to the W / N walks. Existing test
  suites that assumed ASCII-bracket = L stay passing because the
  new ON class still resolves to the surrounding strong direction
  under N1 / N2 for LTR-only inputs (the only change is in
  bracket-spanning RTL / mixed-direction inputs — exactly what N0
  exists to fix).
- **23 unit tests + 22 integration tests**
  (`tests/round257_bidi_n0_bracket_pairs.rs`) cover every BD16
  worked-example line from the §3.1.3 spec table (`a(b)c → 2-4`,
  `a)b(c → None`, `a(b]c → None`, `a(b]c)d → 2-6`, `a(b)c)d → 2-4`,
  `a(b(c)d → 4-6`, `a(b(c)d) → 2-8, 4-6`, `a(b{c}d) → 2-8, 4-6`),
  every N0 case (b / c.1 / c.2 / d), the EN / AN projection note,
  the 63-stack overflow branch, the non-ON skip gate, the
  sequential-rewrite invariant, and the trailing-NSM clarification.

Out-of-scope tail (`README.md`): the full Unicode `BidiBrackets.txt`
table — ~60 paired-bracket entries across the Mathematical
Operators, Misc Mathematical Symbols, Supplementary Mathematical
Operators, CJK Symbols and Punctuation, and Ornamental Brackets
blocks — needs to land under `docs/text/unicode-bidi/` before
`paired_bracket` can grow past the ASCII set. The N0 algorithm
itself is complete; only the lookup table is partial. The L4
mirroring rule remains deferred for the same reason
(`BidiMirroring.txt` also not vendored).

### Added — UAX #9 §3.4 L3 combining-mark reordering (round 247)

Tenth UAX #9 surface on scribe and the third entry in the §3.4
line-level pass: rule **L3**, the post-L2 combining-mark reordering
that callers running a non-scribe mark-attachment policy need to
turn the L2 permutation into a visually-correct display stream.

- **`bidi::reorder_combining_marks(orig_classes: &[BidiClass],
  levels: &[u8], visual: &mut [usize])`** — in-place permutation
  adjuster. After [`reorder_line`], an RTL run that was originally
  `base, nsm_1, …, nsm_k` in logical order appears as
  `nsm_k, …, nsm_1, base` in visual order (L2 reversed the whole
  odd-level run). `reorder_combining_marks` walks the visual stream
  and reverses each such `[NSM, …, NSM, base]` block back to
  `[base, NSM, …, NSM]` per the §3.4 L3 wording "the ordering of
  the marks and the base character must be reversed". The block is
  identified by the strictly-decreasing logical-index signature
  L2 leaves behind, so the function is **idempotent** (re-running
  on already-rotated visual is a no-op) and multi-cluster RTL runs
  rotate each cluster independently without bleeding into one
  another.
- LTR (even-level) runs are untouched — L2 didn't reverse them so
  marks already follow the base. NSMs whose level differs from the
  following base (rare post-W1 leftover) and orphan NSMs with no
  matching base in the same level-1 block are conservatively left
  alone.
- Scribe's own GPOS mark-to-base + mark-to-mark stacker keeps marks
  in logical (post-base) order in both directions, so callers
  using scribe's renderer can skip the L3 step; the L3 entry point
  is for external callers wiring a different mark-attachment policy
  (the spec's "expects them to follow the base characters" branch).
- 9 unit + 15 integration tests cover the single-base RTL one-mark
  / two-mark / three-mark cases, multi-cluster RTL with same and
  different mark counts, AL (Arabic-letter) base, LTR marks
  untouched, RTL island in an LTR paragraph, LTR island in an RTL
  paragraph (level-2 nested, marks out of scope), empty input,
  orphan leading NSM, idempotency, the L3-yields-a-permutation
  invariant, end-to-end L1 → L2 → L3 composition, and the
  pure-RTL multi-letter word with one mark.
- **N0 (bracket-pair resolution per §3.1.3) and L4 (bidi-mirroring
  per §4.7) remain deferred** — N0 blocked on `BidiBrackets.txt`,
  L4 blocked on `BidiMirroring.txt`, neither yet vendored under
  `docs/text/unicode-bidi/` alongside the UAX HTML.

Provenance: rule L3 is transcribed verbatim from UAX #9 Revision 50
/ Unicode 16.0 §3.4 (`docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`).

### Added — UAX #9 §3.3.1 P1 multi-paragraph driver (round 233)

Ninth UAX #9 surface on scribe, sitting one step above the round 227
§3 whole-paragraph driver: the §3.3.1 **P1** rule "Split the text into
separate paragraphs. A paragraph separator (type B) is kept with the
previous paragraph. Within each paragraph, apply all the other rules
of this algorithm." composed with the per-paragraph driver to take a
whole-document `&str` and return one [`ParagraphBidi`] per paragraph
with whole-input byte / character bookkeeping attached.

- **`bidi::process_text(text: &str, base_level: Option<u8>) ->
  TextBidi`** — top-level entry point. Walks the input via the
  existing [`split_paragraphs`] (P1: trailing B kept with preceding
  paragraph), then dispatches each paragraph slice through
  [`process_paragraph_classes`] independently. Per-paragraph
  `char_byte_offsets` are rebased onto the whole input so callers
  index back into the original `&str` directly without adding a
  paragraph-local offset.
- **`bidi::TextBidi`** — multi-paragraph carrier. Fields:
  - `paragraphs: Vec<ParagraphSlice>` — one entry per paragraph
    found by P1, in logical order.
  - `total_chars: usize` — cumulative character count across all
    paragraphs (equals `text.chars().count()`).
- **`TextBidi::len` / `is_empty`** — paragraph-count accessors.
- **`TextBidi::locate_char(k: usize) -> Option<(usize, usize)>`** —
  given a whole-input logical character index, returns
  `(paragraph_index, paragraph_local_char_index)`. Linear walk over
  paragraphs; the docstring notes a future binary-search swap point
  if profiling demands it. Returns `None` past `total_chars`.
- **`bidi::ParagraphSlice`** — per-paragraph carrier. Fields:
  - `byte_range: Range<usize>` — half-open byte range of the
    paragraph (including its trailing B if any) in the original
    `&str`. Successive paragraph ranges tile the input
    contiguously.
  - `char_offset: usize` — cumulative character offset where this
    paragraph begins in the whole-text logical sequence.
  - `bidi: ParagraphBidi` — the §3 P → X → W → N → I output (round
    227 carrier, unchanged).
  - `char_byte_offsets: Vec<usize>` — whole-input byte index of
    each character in the paragraph.
- **`base_level: Option<u8>` HL1 semantics** — when `Some(_)`, every
  paragraph adopts the same base level (low-bit clamped). When
  `None`, P2 / P3 walk each paragraph independently, so e.g. the
  first paragraph of `"Hi\nשלום"` resolves LTR and the second RTL.
  Callers needing per-paragraph HL1 overrides loop
  [`process_paragraph`] manually.
- **Empty-input contract** — `process_text("")` returns
  `TextBidi { paragraphs: vec![], total_chars: 0 }`; the spec
  applies to no characters.
- **Tests** — 14 unit + 24 integration tests cover: P1 split on
  every class-B codepoint scribe's `bidi_class` recognises (LF, CR,
  NEL, FS, GS, RS, PS); trailing-B-kept-with-preceding-paragraph
  invariant; terminal LF not creating a phantom paragraph;
  consecutive separators producing one paragraph each; per-
  paragraph P2 / P3 independence (mixed Latin + Hebrew document);
  HL1 base-level override + low-bit clamp; whole-input byte-range
  contiguous tiling; `char_offset` accumulation across multi-byte
  inputs (Hebrew); `char_byte_offsets` whole-input indexing
  cross-checked against the original string; `total_chars` matches
  `chars().count()`; `locate_char` round-trip with offset
  arithmetic + out-of-bounds `None`; observational equivalence
  with a manual `split_paragraphs` + `process_paragraph` loop;
  per-paragraph `paragraph_level` cross-check; downstream
  `reorder_paragraph` continues to work via the carrier; manual
  L1 / L2 drive via `reset_trailing_levels` + `reorder_line`
  succeeds for every paragraph; `Clone + Eq + Debug` derive
  smoke tests on both carriers. Total scribe lib tests: 431 → 446.

**Still deferred (no change from round 227):**

- N0 bracket-pair resolution (blocked on `BidiBrackets.txt`).
- L4 bidi mirroring (blocked on `BidiMirroring.txt`).
- L3 combining-mark reordering (conditional on the renderer's
  mark-attachment policy; does not fire under scribe's current
  GPOS stacker).

Provenance: P1 transcribed verbatim from UAX #9 Revision 50 /
Unicode 16.0 §3.3.1 P1 at
`docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`; the per-
paragraph dispatch composes the six per-rule entry points already
in this module, each citing the same dated snapshot directly.

### Added — UAX #9 §3 whole-paragraph driver (round 227)

Eighth UAX #9 surface on scribe, composing the six per-rule entry
points from rounds 186 / 191 / 198 / 204 / 217 / 220 into a single
high-level call. The driver runs §3.2 → §3.3.1 → §3.3.2 → §3.3.3 →
§3.3.4 → §3.3.5 → §3.3.6 on a `&str` or pre-classified `&[BidiClass]`
slice, then publishes the per-character resolved embedding level
ready for §3.4 L1 + L2 to consume per display line.

- **`bidi::process_paragraph(text: &str, base_level: Option<u8>) ->
  (ParagraphBidi, Vec<usize>)`** — text-driving entry point.
  Walks the input once for `char_indices()`, collecting one
  `BidiClass` per character plus a parallel byte-offset vector,
  then dispatches to the class-driven variant. The byte-offset
  vector locates each character in the original UTF-8 input —
  callers that need to slice back into `text` after the bidi
  pass (e.g. to extract per-run glyph clusters) read
  `text[char_byte_offsets[i]..]` to position the cursor at the
  character at logical index `i`.
- **`bidi::process_paragraph_classes(classes: &[BidiClass],
  base_level: Option<u8>) -> ParagraphBidi`** — class-driven
  entry point. Composes:
  - §3.3.1 paragraph level via the new internal
    `paragraph_level_from_classes` walker (mirrors the existing
    `paragraph_level` text walker — first-strong L / R / AL,
    skip LRI..PDI isolate spans per BD8, P3 default 0) unless
    the caller overrides with `base_level = Some(_)` (HL1).
  - §3.3.2 X1..X9 explicit-level pass via
    `resolve_explicit_levels`.
  - §3.3.3 X10 isolating-run-sequence partition via
    `isolating_run_sequences`.
  - §3.3.4 W1..W7 + §3.3.5 N1 + N2 + §3.3.6 I1 + I2
    **per-sequence** (per the X10 step 3 closing note that
    sequence order does not matter). Resolved levels are
    scattered back into the paragraph-wide level vector at the
    original logical offsets; X9-removed positions retain the
    X-rule level since W / N / I skipped them.
- **`bidi::ParagraphBidi`** — the carrier struct. Fields:
  - `paragraph_level: u8` — the §3.3.1 / HL1 result (0 or 1).
  - `classes: Vec<BidiClass>` — input verbatim (L1 needs the
    original types per the §3.4 normative note).
  - `effective_classes: Vec<BidiClass>` — X4 / X5 / X5a / X5b /
    X6 / X6a override-rewritten classes from X1..X9.
  - `removed: Vec<bool>` — X9-removed-flag set.
  - `levels: Vec<u8>` — per-character resolved embedding level
    after the full X → W → N → I sweep.
- **`ParagraphBidi::reorder_paragraph()`** — "whole paragraph as
  one line" convenience. Runs §3.4 L1 + L2 over the carrier and
  returns the logical-to-visual permutation.
- **`ParagraphBidi::reorder_line_range(line: Range<usize>)`** —
  per-line variant. Slices `classes` + `levels` to the given
  half-open range, runs L1 + L2 over the slice, and returns a
  permutation **relative to the line** (caller adds `line.start`
  to map back into the paragraph).
- **`base_level` low-bit clamp** — `base_level = Some(5)` maps to
  paragraph level 1 (the spec only defines two paragraph
  embedding levels). Removes a footgun for callers wiring HL1
  from external integer sources.
- **Tests** — 19 unit + 22 integration tests cover: P2 first-
  strong L / R / AL recognition, P3 no-strong default, BD8
  isolate-span skipping (matched + unmatched LRI), HL1
  base-level override (both directions + low-bit clamp), mixed
  L / R compositions (LTR with embedded RTL block at level 1,
  RTL paragraph lifting embedded LTR block to level 2,
  AL EN → R AN pipeline via W2 + W3 + I1), X9-removed positions
  retaining their X-rule level, text-driving char-byte offset
  advancement on ASCII / Hebrew / Arabic, the carrier's field-
  length invariant, input-class verbatim preservation, the
  `Clone + Eq + Debug` derive set on `ParagraphBidi`, the
  `reorder_paragraph` LTR-identity + RTL-reversal pair, per-line
  `reorder_line_range` on a split LTR paragraph, L1-reset of
  trailing whitespace agreeing with manual L1 + L2 chaining, and
  the §3.4 spec example "car means CAR." resolving to its
  published visual order. The class-driven P2 walker is cross-
  checked against the existing text-driving `paragraph_level` on
  six inputs spanning Latin / Hebrew / Arabic / mixed / LRI..PDI.
  Total scribe lib tests: 412 → 431.

**Still deferred (no change from round 220):**

- N0 bracket-pair resolution (blocked on `BidiBrackets.txt`).
- L4 bidi mirroring (blocked on `BidiMirroring.txt`).
- L3 combining-mark reordering (conditional on the renderer's
  mark-attachment policy; does not fire today under scribe's
  GPOS stacker).

Provenance: the driver composes the six existing per-rule entry
points already in this module; each cites
`docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` (UAX #9
Revision 50 / Unicode 16.0) directly in its own provenance line.
The §3 umbrella anchor lives at §3 "Basic Display Algorithm" of
the same dated snapshot.

### Added — UAX #9 §3.3.3 X10 isolating-run-sequence partition + sos/eos (round 220)

Seventh UAX #9 surface on scribe, sitting between the §3.3.2
explicit-level pass (X1..X9, round 217) and the §3.3.4 weak-type
pass (W1..W7, round 191): the §3.3.3 rule **X10** that walks
X1..X9's per-character embedding-level output, partitions the
paragraph into BD7 level runs, chains those runs across BD13
isolate-initiator → matching-PDI boundaries into isolating run
sequences, and derives per-sequence start-of-sequence (`sos`)
and end-of-sequence (`eos`) directional types from the higher of
the levels on either side of each sequence boundary.

- **`bidi::level_runs(levels: &[u8]) -> Vec<LevelRun>`** — BD7
  partition. Returns a contiguous list of half-open
  `LevelRun { start, end, level }` ranges fully covering
  `0..levels.len()`. X9-removed positions are included in their
  containing level run; W / N / I phases consume them via
  `IsolatingRunSequence::indices` which performs the X9 skip.
- **`bidi::isolating_run_sequences(classes: &[BidiClass],
  explicit: &ExplicitLevels, paragraph_level: u8) ->
  Vec<IsolatingRunSequence>`** — BD13 chaining + X10 step 2
  sos / eos derivation. For every level run whose last non-X9-
  removed character is an isolate initiator (LRI / RLI / FSI), if
  BD9's matching-PDI scan finds a PDI **and** that PDI is the
  first character of its own level run, the next run is appended
  to the current sequence; otherwise the chain closes. The sos /
  eos pair on each sequence reflects the X10 step 2 prose: scan
  outward past X9-removed characters; if no non-removed character
  exists on either side, OR the eos side is an unmatched isolate
  initiator, fall back to the paragraph embedding level. "If the
  higher level is odd, the sos or eos is R; otherwise, it is L."
- **`bidi::IsolatingRunSequence::indices(&removed)`** — iterator
  over the constituent character indices in logical order while
  skipping X9-removed positions. Drives the in-place W / N / I
  passes per sequence. Per the X10 step 3 closing note "the order
  that one isolating run sequence is treated relative to another
  does not matter."
- **New public types** — `LevelRun { start, end, level }` and
  `IsolatingRunSequence { runs, level, sos, eos }`. Both
  `Clone + PartialEq + Eq + Debug` for downstream snapshot /
  memoisation use cases.
- **Tests** — 17 unit + 16 integration tests cover the BD7
  partition (empty / uniform / multi-change / coverage invariant),
  the X10 sos / eos derivation under paragraph-edge / RLE-bounded
  / unmatched-isolate / RTL-base-paragraph conditions, the BD13
  chaining across LRI..PDI verifying that the matching PDI is the
  first character of its run, the BD9 matching-PDI scan with
  nested isolates and ignored embedding-formatting characters, the
  BD13 invariants (every level run belongs to exactly one
  sequence; all runs in a sequence share their embedding level),
  the `indices` iterator's X9-skip + isolate-format-character
  preservation, and an end-to-end X → W → N → I pipeline composed
  per-sequence. Total scribe lib tests: 395 → 412.

**Out of scope (still deferred):**

- **N0 bracket-pair resolution** (§3.1.3 / §3.3.5) — blocked on
  `BidiBrackets.txt` from the Unicode Character Database; not yet
  vendored under `docs/text/unicode-bidi/`.
- **L4 bidi mirroring** (§3.4 / §4.7) — blocked on
  `BidiMirroring.txt` from the same UCD release; not yet vendored.
- **L3 combining-mark reordering** (§3.4) — conditional on the
  rendering engine's mark-attachment policy per the §3.4 guard;
  does not fire today because scribe's GPOS mark-to-base /
  mark-to-mark stacker keeps logical (post-base) order in both
  directions.

Provenance: transcribed verbatim from UAX #9 Revision 50 /
Unicode 16.0 §3.1.2 (BD7, BD9, BD13) + §3.3.3 (X10) at
`docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`.

### Added — UAX #9 §3.3.2 explicit-level / override / isolate stack pass (X1..X9, round 217)

Sixth UAX #9 surface on scribe, slotting in front of the existing
W / N / I / L pipeline: the §3.3.2 explicit-level rules **X1..X9**
that walk a paragraph's bidi class slice while maintaining the
spec's *directional status stack* + the three overflow / valid
counters, and emit a per-character embedding level vector ready
for the X10 isolating-run-sequence partition.

- **`bidi::resolve_explicit_levels(classes: &[BidiClass],
  paragraph_level: u8) -> ExplicitLevels`** — single-pass walk
  over the paragraph implementing X1 (stack init), X2..X5
  (RLE / LRE / RLO / LRO embedding / override pushes), X5a / X5b
  (RLI / LRI isolate pushes), X5c (FSI resolved via P2 / P3 over
  the FSI..matching-PDI span and treated as RLI or LRI
  accordingly), X6 (regular-character level assignment + override
  rewrite), X6a (PDI: unwind embeddings down to + including the
  matched isolate frame), X7 (PDF: pop the matching embedding,
  with overflow-counter bookkeeping for unmatched / overflow
  cases), X8 (B characters always at the paragraph level), and X9
  (mark RLE / LRE / RLO / LRO / PDF / BN as removed; isolate-
  formatting characters LRI / RLI / FSI / PDI are *not* removed
  per the X9 note).
- **`bidi::ExplicitLevels`** — the return struct, carrying three
  parallel vectors of length `classes.len()`:
  - `levels: Vec<u8>` — per-character embedding level. Index
    preservation lets the downstream X10 partition + W / N / I
    phases map back to the original logical offsets.
  - `effective_classes: Vec<BidiClass>` — input classes with X4 /
    X5 / X5a / X5b / X6 / X6a override rewrites applied. Under
    an `L` override every non-formatting character is rewritten
    to `L`; under an `R` override to `R`; under neutral (the
    starting state, embeddings and isolates) classes are
    unchanged.
  - `removed: Vec<bool>` — `true` for the six X9-removed types.
- **`bidi::MAX_DEPTH`** — `pub const MAX_DEPTH: u8 = 125;` per
  UAX #9 §3.1.2 BD2 ("this specification now guarantees that the
  value of 125 for max_depth will not be increased (or
  decreased) in future versions"). Embedding initiators that
  would push past this depth become *overflow* events tracked
  through the overflow_embedding / overflow_isolate counters.
- **`oxideav_scribe::resolve_explicit_levels`** +
  **`oxideav_scribe::ExplicitLevels`** + **`oxideav_scribe::MAX_DEPTH`**
  — public re-exports alongside the existing W / N / I / L
  surface.

The X10 isolating-run-sequence partition (BD13) is *not* part of
this round — it runs on top of X1..X9's level output and feeds
the W / N / I passes per sequence. The bracket-pair rule N0 (still
blocked on `BidiBrackets.txt`) and the L3 / L4 mirroring rules
remain deferred.

42 new tests (19 unit + 23 integration via
`tests/round217_bidi_explicit_levels.rs`):

- **X1 init** — empty-paragraph no-op, plain Latin / Arabic
  paragraph-level dispatch, paragraph-level=1 path.
- **X2..X5** — RLE / LRE / RLO / LRO each pushing the spec's
  least-greater-odd / -even level above the stack top, the
  embedding-initiator's reported level reflecting the new scope,
  the override status correctly rewriting inner classes.
- **X5a / X5b** — RLI / LRI pushing the same least-greater-
  odd / -even levels but as isolate frames, the initiator's own
  level reflecting the *enclosing* scope per the spec text, the
  X9 non-removal of the four isolate-formatting characters.
- **X5c** — FSI resolved via the in-span P2 + P3 mini-pass:
  strong-L-inside → LRI, strong-AL-inside → RLI, no-strong →
  default LRI, P2's inner isolate-skip working through nested
  LRI..PDI inside the FSI span.
- **X6** — override rewriting only non-formatting types.
- **X6a** — PDI matching the enclosing isolate frame: unwind
  embeddings above it, pop the isolate, unmatched PDI ignored.
- **X7** — PDF popping the matching embedding, PDF at top level
  ignored, PDF inside an overflow scope absorbed by the overflow
  counter.
- **X8** — B inside any embedding still reported at the paragraph
  level.
- **X9** — RLE / LRE / RLO / LRO / BN / PDF marked removed; LRI /
  RLI / FSI / PDI not removed.
- **Overflow** — 64-RLE-deep chain pinning the inner level at
  MAX_DEPTH = 125; doubly-nested RLI..PDI..PDI pair both popping
  cleanly within bounds.
- **Public surface** — re-export shape sanity check, MAX_DEPTH
  constant value, plus mixed Latin / Hebrew paragraph fed through
  `paragraph_level → resolve_explicit_levels` end-to-end.

Provenance: rules transcribed verbatim from
`docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` §3.3.2 (UAX
#9 Revision 50, Unicode 16.0). BD2's `max_depth = 125` constant
also pinned to the same file.

### Added — UAX #9 §3.4 line-level reordering rules L1 + L2 (round 210)

Fifth UAX #9 surface on scribe, layered on top of the round 204
implicit-level pass: the §3.4 line-level reordering rules L1 and
L2. The pair closes the per-character pipeline (P → W → N → I →
L1) and emits the logical-to-visual permutation a renderer walks
to lay glyphs out in display order.

- **`bidi::reset_trailing_levels(orig_classes: &[BidiClass],
  levels: &mut [u8], paragraph_level: u8)`** — in-place pass over
  one line implementing UAX #9 §3.4 rule **L1**. Resets the
  embedding level of:
  - every segment separator (class `S`) per case (1),
  - every paragraph separator (class `B`) per case (2),
  - every maximal run of whitespace (`WS`) and/or isolate-
    formatting characters (`LRI` / `RLI` / `FSI` / `PDI`)
    immediately preceding such a separator per case (3),
  - the same trailing-filler run at the end of the line per
    case (4),
  back to `paragraph_level`. Per UAX #9 §3.4 the lookup uses the
  **original** bidi classes ("The types of characters used here
  are the *original* types, not those modified by the previous
  phase."), so the caller passes the input class slice alongside
  the post-I-rules level vector. Panics on length mismatch.
- **`bidi::reorder_line(levels: &[u8]) -> Vec<usize>`** —
  permutation pass implementing UAX #9 §3.4 rule **L2**. Walks
  from the maximum level in `levels` down to the lowest odd
  level, and for each iteration level reverses every maximal
  contiguous run of positions whose level is `>= iteration_level`.
  Returns a `Vec<usize>` of length `levels.len()` mapping visual
  position to logical index (i.e. `visual[v] == logical_index`),
  the form a renderer / line builder consumes when emitting
  glyphs in display order. Returns the identity for empty input
  or input with no odd levels.
- **`oxideav_scribe::reset_trailing_levels`** +
  **`oxideav_scribe::reorder_line`** — public re-exports alongside
  the existing W / N / I pass entry points.

The pair completes the per-character UAX #9 algorithm scribe
needs to drive mixed-direction layout: a caller feeds a line
through `bidi_class` → `resolve_weak_types` →
`resolve_neutral_types` → `resolve_implicit_levels` →
`reset_trailing_levels` → `reorder_line` and ends with the
visual-order index sequence the line renderer walks. The X-rules
(explicit-embedding / override / isolate stack machinery + the
isolating-run-sequence partition), the N0 bracket-pair rule, and
the L3 / L4 mirroring rules remain deferred.

**38 new tests** (17 unit + 21 integration via
`tests/round210_bidi_line_reordering.rs`): every L1 sub-case (S /
B separators in cases 1 + 2; WS-before-separator and
isolate-formatting-before-separator in case 3; trailing-WS at end
of line in case 4); the §3.4 normative "original classes"
clause; interior whitespace and leading whitespace negative
controls; length-mismatch panic; empty input; multiple
separators on one line; identity and full-reverse for L2; UAX #9
§3.4 worked examples 1, 2, 3, and 4 reproduced by their
resolved-level vectors (the spec's "Resolved levels" row) and
verified against the spec's "Display" row; permutation-invariant
sweep over a small set of mixed-level shapes; and end-to-end
`W → N → I → L1 → L2` pipelines on real Arabic text, including
the §3.4 Example-1 line "car means CAR." reconstructed from real
characters with the paragraph level detected by `paragraph_level`.

Provenance: rules transcribed verbatim from
`docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` §3.4 (UAX #9
Revision 50, Unicode 16.0).

### Added — UAX #9 §3.3.6 implicit-level resolution rules I1 + I2 (round 204)

Fourth UAX #9 surface on scribe, layered on top of the round 198
neutral-type pass: the §3.3.6 implicit-level resolution rules I1
and I2. The phase computes the per-character resolved embedding
level the L-rule reordering pass consumes; it is the final
character-level transformation in the algorithm before line-level
work begins.

- **`bidi::resolve_implicit_levels(classes: &[BidiClass],
  embedding_level: u8) -> Vec<u8>`** — pass over one isolating run
  sequence (the same slice already mutated through
  `resolve_weak_types` + `resolve_neutral_types`) returning a
  per-character level vector. The implementation is UAX #9 §3.3.6
  Table 5 verbatim:
  - **I1** (even embedding levels): `R` → EL+1, `EN` / `AN` → EL+2.
  - **I2** (odd embedding levels): `L` / `EN` / `AN` → EL+1.
  - `BN` is ignored per §5.2 ("In rules I1 and I2, ignore BN.") —
    its level stays at `embedding_level`. A surviving `NSM` (the
    rare case where W1 left it intact) is treated the same; the
    L-rule pass folds both.
- **`oxideav_scribe::resolve_implicit_levels`** +
  `oxideav_scribe::bidi::resolve_implicit_levels` — public
  re-exports alongside the W / N pass entry points.

After the call the caller has the level vector the §3.4 / §3.4.1
L-rule reordering pass needs. The X-rules (paragraph + embedding /
isolate stack machinery + the isolating-run-sequence partition)
remain deferred; callers that already know their embedding level
pass it directly, which matches the same contract `paragraph_level`
returned in round 186 for the outer level.

**23 new tests** (8 unit + 15 integration via
`tests/round204_bidi_implicit_levels.rs`): every Table 5 row at
multiple embedding levels (even / odd, base levels 0 / 1 / 2 / 3
plus EL = 124 for the max-depth overflow note), the `BN`
carve-out at both even and odd embedding levels, empty-input
no-op, output-length-equals-input-length defensive sweep, and
full W → N → I pipelines on LTR / RTL paragraph fragments —
including the §3.3.5 closing prose example "IT IS A bmw 500, OK."
reduced to its embedded `R EN ET EN R` run at EL 1, which
exercises W5's ET-EN collapse + I1's EN-up-two in a single
end-to-end level vector.

Cleanup: removed two stale provenance lines in `src/variations.rs`
and `src/shaping/arabic_pf.rs`; the substantive provenance
citation (Microsoft OpenType chapters for variations,
`UnicodeData.txt` decomposition mappings for Arabic presentation
forms) is unchanged.

Provenance: rules transcribed verbatim from
`docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` §3.3.6 (UAX #9
Revision 50, Unicode 16.0).

### Added — UAX #9 §3.3.5 neutral-type resolution rules N1 + N2 (round 198)

Third concrete UAX #9 surface on scribe, layered on top of the
round 191 weak-type pass: the §3.3.5 neutral / isolate-formatting
resolution rules N1 and N2. The phase finishes resolving every
former neutral or isolate-formatting position to a strong
direction, completing the input vocabulary the §3.3.6 implicit-
level pass (I1 / I2) will consume in a follow-up round.

- **`bidi::resolve_neutral_types(classes: &mut [BidiClass],
  embedding_level: u8, sos: BidiClass, eos: BidiClass)`** — in-place
  pass over one isolating run sequence (the same slice already
  mutated by `resolve_weak_types`) applying:
  - **N1** — every maximal contiguous run of NI elements (`B` /
    `S` / `WS` / `ON` / `LRI` / `RLI` / `FSI` / `PDI`) collapses to
    the strong type on either side when both sides agree, with `EN`
    and `AN` counting as `R` per the spec's "European and Arabic
    numbers act as if they were R". `NSM` / `BN` are skipped by the
    strong-side walk (they are non-strong but also non-NI). `sos` /
    `eos` provide the strong type at sequence boundaries.
  - **N2** — every NI run whose strong neighbours disagree
    collapses to the embedding direction (`L` for even
    `embedding_level`, `R` for odd).
- **`oxideav_scribe::resolve_neutral_types`** + the matching
  `oxideav_scribe::bidi::resolve_neutral_types` — public re-exports
  alongside `resolve_weak_types`.

After the pass the slice contains no NI; `NSM` and `BN` survive
untouched (they are not in the NI alias and the §3.3.6 implicit-
level rules handle them).

**N0 (bracket-pair resolution per §3.1.3 + §3.3.5) is deferred** —
it requires the Unicode `BidiBrackets.txt` data file to identify
opening / closing paired brackets, which is not yet vendored under
`docs/`. The N1 / N2 surface is forward-compatible: an N0
implementation lands as a pre-N1 pass that turns bracket positions
into strong types, after which N1 / N2 continues to apply
unchanged.

**21 new tests** (10 unit + 11 integration via
`tests/round198_bidi_neutral_types.rs`): every spec example —
`L NI L → L L L`, `R NI R → R R R`, the full R / AN / EN N1 table
with EN / AN counting as R, the N2 mismatch table at both
embedding levels, the full NI alias collapsing in one run,
`NSM` / `BN` pass-through across NI boundaries, `sos` / `eos`
driving boundary-spanning runs, idempotence on NI-free slices,
and the compose-with-W realistic Arabic + numbers pipeline.

Provenance: rules transcribed verbatim from
`docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` §3.3.5 (UAX #9
Revision 50, Unicode 16.0). No external library source was
consulted at any point.

## [0.1.8](https://github.com/OxideAV/oxideav-scribe/compare/v0.1.7...v0.1.8) - 2026-05-30

### Other

- UAX #9 weak-type resolution rules W1..W7 (round 191)
- UAX #9 BiDi foundation — character classes + P1/P2/P3 paragraph level (round 186)
- caller-driven Type-3 alternate-index selection (round 183)
- round175 tests: use scribe-local DejaVu fixture (CI fix)
- explicit-script-tag entry point + broadened auto-probe (round 175)
- GSUB LookupType 3 (Alternate Substitution) wired into shape_text (round 156)
- GSUB LookupType 4 (Ligature Substitution) wired into shape_text (round 128)
- GSUB LookupType 2 (Multiple Substitution) wired into shape_text (round 125)
- GSUB LookupType 1 caller-driven feature application (round 89)
- GSUB feature-tag introspection on Face (round 88)
- general-script ccmp + calt GSUB pass (round 15)
- hygiene round 75 (Error::source, lib.rs doc refresh, no_run doctests)
- drop committed Cargo.lock + relax oxideav-core to "0.1"
- backfill Unreleased entry for round 14 + round 13
- variable fonts (CFF2/MVAR/HVAR/VVAR/STAT/name_id) + Brahmic round 13 (Burmese + Lao)

### Added — UAX #9 §3.3.4 weak-type resolution rules W1..W7 on the BiDi foundation (round 191)

Second concrete UAX #9 surface on scribe lands on top of the round
186 character-class + paragraph-level foundation: the complete W
phase from §3.3.4. The phase resolves one isolating run sequence
end-to-end, leaving the slice with no `AL` (W3 collapses every one
to `R`) and no leftover `ES` / `ET` / `CS` (W6 collapses every
remainder to `ON`) — i.e. a clean vocabulary for the upcoming
N-rules and the §3.3.6 implicit-level rules.

- **`bidi::resolve_weak_types(classes: &mut [BidiClass], sos:
  BidiClass, eos: BidiClass)`** — in-place pass over one isolating
  run sequence applying the seven rules in order:
  - **W1** — `NSM` inherits the type of the immediately-preceding
    character. NSM at the start of the sequence takes the `sos` type.
    NSM after an isolate initiator (`LRI` / `RLI` / `FSI`) or after
    `PDI` becomes `ON`. The forward pass means consecutive NSMs all
    flip to the same final type (per the spec example `AL NSM NSM →
    AL AL AL`).
  - **W2** — `EN` whose most-recent strong type (walking backward
    through neutrals etc., with `sos` as the implicit start) is `AL`
    becomes `AN`.
  - **W3** — every remaining `AL` becomes `R`. (Runs after W2 so the
    AL marker is still visible to the EN rewriter.)
  - **W4** — a single `ES` between two `EN`s collapses to `EN`. A
    single `CS` between two same-type numbers collapses to that
    type (`EN CS EN → EN EN EN` and `AN CS AN → AN AN AN`). Mixed-
    type CS (`EN CS AN`) and double separators (`EN ES ES EN`) do
    NOT collapse — W4 demands a *single* separator with a *same-type*
    number on each side.
  - **W5** — runs of `ET` adjacent to an `EN` on either side
    collapse to `EN`. ETs adjacent to `AN` (not `EN`) do not flip;
    isolated ET runs with no EN neighbour fall through to W6.
  - **W6** — every leftover `ES` / `ET` / `CS` becomes `ON`. This is
    the catch-all that drops separators which did not get absorbed
    into a number, so the N-rules see only `L` / `R` / `EN` / `AN` /
    `NSM` / `BN` / neutrals.
  - **W7** — `EN` whose most-recent strong type (`L` / `R` / `sos`)
    is `L` becomes `L`. After W3 the strong-type vocabulary is
    `{L, R, sos}` only — the spec example `L NI EN → L NI L` is
    asserted directly. `R` as the most-recent strong leaves the EN
    untouched.
- **`BidiClass::is_neutral_or_isolate()`** — predicate for the
  UAX #9 §3.3.5 / §3.3.6 NI alias (`B | S | WS | ON | FSI | LRI |
  RLI | PDI`). Surfaced now because W7's narration uses NI, and
  because every subsequent rule (N0..N2, I1..I2) dispatches on the
  same predicate.

Coverage: 14 unit tests in `src/bidi.rs` (every rule's positive +
negative spec examples plus a full-pipeline composition check) + 11
integration tests in `tests/round191_bidi_weak_types.rs` (the same
rule examples asserted through the `oxideav_scribe::resolve_weak_types`
public re-export). The realistic-pipeline test walks an
`AL NSM EN ET EN CS EN` sequence through every rule's footprint.

The `sos` / `eos` parameters keep the signature symmetric with the
forthcoming N-rules (which read both), even though W1..W7 actually
only read `sos` (via W1's start-of-sequence default, W2's
most-recent-strong implicit start, and W7's same). Callers without
X1..X10 wired up yet can pass `BidiClass::L` for paragraph level 0
and `BidiClass::R` for level 1 — that produces a correct W pass for
a single-paragraph no-isolate input.

Out of scope for this round (each is its own follow-up):

- **N0** bracket-pair-aware neutral resolution (needs
  `BidiBrackets.txt` from the UCD).
- **N1, N2** neutral type resolution against surrounding strong
  types + paragraph embedding direction.
- **I1, I2** implicit embedding level resolution.
- **X1..X10** explicit embedding / override / isolate stack
  machinery + the isolating-run-sequence partition + overflow
  counters.
- **L1..L4** line-level reordering + mirroring
  (`BidiMirroring.txt`).

Provenance: every rule transcribed verbatim from the dated snapshot
at `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` §3.3.4 (UAX
#9 Revision 50, Unicode 16.0, fetched 2026-05-29). Every test input
comes directly from the spec's per-rule example blocks. No external
library source — HarfBuzz, ICU, FreeType, rustybuzz, the
`unicode-bidi` crate, etc. — was consulted at any point.

### Added — Unicode Bidirectional Algorithm foundation: UAX #9 character classes + P1/P2/P3 paragraph-level resolution (round 186)

First concrete UAX #9 (*Unicode Bidirectional Algorithm*) surface
landed on scribe. The crate had no BiDi at all before this round —
`layout` was advertised as "no bidi" and the `shaping::arabic`
module documented "logical-order input is assumed (post-bidi)" with
no helper to produce that ordering. Round 186 lands the **foundation**
surface every subsequent UBA rule (W / N / I / X / L) will dispatch
against:

- **`bidi::BidiClass`** — the 23 normative bidirectional character
  types from UAX #9 §3.2 Table 4 (3 Strong: `L` / `R` / `AL`; 7 Weak:
  `EN` / `ES` / `ET` / `AN` / `CS` / `NSM` / `BN`; 4 Neutral: `B` /
  `S` / `WS` / `ON`; 9 Explicit Formatting: `LRE` / `LRO` / `RLE` /
  `RLO` / `PDF` / `LRI` / `RLI` / `FSI` / `PDI`).
- **`bidi::bidi_class(c: char) -> BidiClass`** — `char` → class
  lookup. Coverage: the 12 explicit-formatting control characters
  (LRM, RLM, ALM, LRE, RLE, PDF, LRO, RLO, LRI, RLI, FSI, PDI)
  exhaustively; the paragraph / segment / line separators (LF / CR /
  NEL / U+2029 / TAB / VT / U+001F / FF / SPACE / U+1680 / U+2028 /
  U+2000..U+200A / U+202F / U+205F / U+3000); ASCII digits (EN),
  separators (`+ -` ES; `, . / :` CS; `# $ %` ET) and letters (L);
  C0 / DEL boundary-neutral controls; the Latin-1 supplement (NBSP =
  CS, currency / degree = ET, SOFT HYPHEN = BN, Latin-1 letters = L);
  the Combining Diacritical Marks (U+0300..U+036F = NSM); the Hebrew
  block (U+0590..U+05FF = R) and Hebrew Presentation Forms
  (U+FB1D..U+FB4F = R); the four core Arabic blocks (U+0600..U+06FF
  with `U+0660..U+0669` split out as AN, `U+06F0..U+06F9` as EN, and
  the documented Arabic NSM ranges separated out) and Syriac +
  Arabic Supplement + Arabic Presentation Forms A and B (all AL);
  Thaana (AL); N'Ko (R); ZWJ / ZWNJ / word-joiner family (BN); object
  replacement character U+FFFC (ON). Unmapped code points fall back
  to `L` per the UAX #9 §3.2 default ("Unassigned characters are
  given strong types in the algorithm.").
- **`bidi::paragraph_level(text: &str) -> u8`** — UAX #9 **P1 + P2
  + P3** in a single call. Walks the text, tracks isolate depth so
  the contents of any `LRI` / `RLI` / `FSI` ... `PDI` region are
  skipped (nested arbitrarily — overflow / overflow-isolate
  accounting from X5a..X5c is **not** implemented because P2 only
  needs to skip), finds the first strong character (L / R / AL),
  and returns 1 if it is R or AL, 0 otherwise. The default when no
  strong character is found is also 0. An unmatched `PDI` at top
  level is ignored (the spec achieves this by "skip until matching
  PDI or end of paragraph" but the top-level case is symmetric).
  Embedding initiators (`LRE` / `RLE` / `LRO` / `RLO` / `PDF`) are
  *not* skipped by P2, only isolates are — asserted by a dedicated
  test pair.
- **`bidi::split_paragraphs(text: &str) -> Vec<&str>`** — UAX #9
  **P1**. Splits at every type-`B` character and **keeps the
  separator with the preceding paragraph** (per the spec's wording
  "A paragraph separator (type B) is kept with the previous
  paragraph."). The returned slices concatenate back to the input
  byte-for-byte; two adjacent `B` characters yield an empty
  middle paragraph.
- **Predicates on `BidiClass`** — `is_strong()` (L / R / AL) and
  `is_isolate_initiator()` (LRI / RLI / FSI) for use by P2 + the
  forthcoming X-rules.

Coverage: 15 unit tests in `src/bidi.rs` (explicit-format set,
isolate-initiator + strong predicates, ASCII + Latin-1 + Hebrew +
Arabic + Combining Diacriticals class assignments, P1 split, P2
isolate-skip including nested initiators, P3 LTR/RTL defaults, P2
non-skip-of-embedding-initiators, unmatched-PDI tolerance) + 6
integration tests in `tests/round186_bidi_paragraph_level.rs` (the
same contracts asserted through the public re-export surface
`oxideav_scribe::{BidiClass, bidi_class, paragraph_level,
split_paragraphs}`).

Out of scope for this round (each is its own follow-up):

- **W1..W7** weak type resolution (NSM + EN/ES/CS + AN/ET joining
  rules).
- **N0..N2** neutral type resolution, including the §3.1.3 bracket
  pairs algorithm (`BidiBrackets.txt`).
- **I1..I2** implicit embedding level resolution.
- **X1..X10** explicit embedding / override / isolate stack
  machinery + the isolating-run-sequence partition + overflow
  counters.
- **L1..L4** line-level reordering + mirroring (`BidiMirroring.txt`).
- Filling out the per-code-point class table beyond the listed
  ranges — needs `DerivedBidiClass.txt` from the Unicode Character
  Database (UAX #9 references but does not include this file).

Provenance: every class assignment and every rule mirrors the
dated snapshot at
`docs/text/unicode-bidi/tr9-50-uax9-unicode16.html` (UAX #9
Revision 50, Unicode 16.0, fetched 2026-05-29). No external
library source was consulted at any point.

### Added — Caller-driven Type-3 alternate-index selection (round 183)

A pair of new `Face` methods that let callers pick which
`AlternateSet` entry the GSUB Type-3 (Alternate Substitution) walker
applies per feature, instead of the round-156 hardcoded
`alternateIndex = 0`:

- **`Face::shape_text_with_alternates(text, feature_alternates)`** —
  auto-probe script-resolution variant. `feature_alternates` is a
  list of `(feature_tag, alternate_index)` pairs; the set of features
  applied is the union of the tags listed.
- **`Face::shape_text_with_script_and_alternates(text, script_tag,
  feature_alternates)`** — explicit-script variant. Mirrors the
  round-175 `shape_text_with_script` resolution semantics; every
  feature resolves against `script_tag` alone (no priority walk).

Contract:

1. **Index 0 reproduces the round-156 default** — the round-183
   surface is a strict superset, not a re-derivation. Asserted by
   `round183_index_zero_matches_round156_aalt_on_inter` /
   `_on_dejavu` against the existing fixtures.
2. **Out-of-range index falls back to cmap-identity per slot.**
   When `alternate_index` exceeds a covered glyph's `AlternateSet`
   entry count, `oxideav_ttf::Font::gsub_apply_lookup_type_3`
   returns `None` and we leave the slot's GID unchanged. Safe
   fallback for callers that don't pre-probe per-font alternate
   counts.
3. **Type-3 length-preservation is invariant across indices** —
   per OpenType §6.2.3, the run length matches `cmap_only.len()`
   regardless of which `alternate_index` the caller requests.
4. **Non-Type-3 features ignore the index.** Features that mix
   Type-3 with Type-1 (Single) / Type-2 (Multiple) / Type-4
   (Ligature) lookups still dispatch the non-Type-3 components
   with their existing walkers — `liga`'s Type-4 ligature collapse
   produces identical output whether the caller passes
   `(*b"liga", 0)` or `(*b"liga", u16::MAX)`.

Underlying mechanism: the new methods route through
`crate::shaping::shape_text_with_alternates_with_font` /
`shape_text_with_script_and_alternates_with_font`, which call the
existing private `shape_text_inner` with a per-call
`feature_alternates: &[(feature_tag, alternate_index)]` slice.
Inside the dispatch loop, the per-feature alternate index is
looked up via a linear scan (the list is short in practice) and
passed to `font.gsub_apply_lookup_type_3(lookup_idx, gid,
alt_index)` instead of the hardcoded `0`. Types 1, 2, and 4 are
dispatched unchanged.

Coverage: 7 new unit tests in `src/shaping/feature_subst.rs`
(empty-list contract, index-0-matches-156 contract, out-of-range
fallback, explicit-script unknown-tag contract, explicit-script
index-0 contract, non-Type-3 ignore-index contract, multi-sample
length-preservation matrix) + 13 new integration tests in
`tests/round183_alternate_index.rs` (the same contracts asserted
against the public `Face` surface, including a multi-feature mixed-
index walk).

### Added — Explicit-script-tag entry point + broadened script-tag probe list (round 175)

Two paired changes that broaden the caller-driven GSUB surface to
non-Latin scripts:

1. **`Face::shape_text_with_script(text, script_tag, features)`** —
   the deterministic-resolution mirror of `Face::shape_text`. Every
   requested feature tag is resolved against the explicit
   `script_tag` alone (with the script's DefaultLangSys), bypassing
   the script-tag priority walk. Useful when the caller already
   knows the script of the run — typically because the run came out
   of a script-segmenter or because the caller is shaping a
   known-language string — and the auto-probe walk's cross-script
   collision risk is unacceptable (e.g. `liga` published under both
   `latn` and `arab`). LookupType-1/2/3/4 dispatch semantics mirror
   `shape_text` exactly; lookup types 5/6/8 referenced by the
   requested features are silently skipped (same caller-driven
   contract as round 89/125/128/156).
2. **Broadened script-tag probe list in `shape_text`** — the
   underlying `resolve_feature_lookups` helper used by the
   auto-probing surface now walks `latn` → `cyrl` → `grek` → `DFLT`
   → `arab` → `hebr` → `thai` → `lao ` → Indic v1+v2 (`deva` /
   `dev2`, `beng` / `bng2`, `taml` / `tml2`, `gujr` / `gjr2`, `guru`
   / `gur2`, `knda` / `knd2`, `mlym` / `mlm2`, `orya` / `ory2`,
   `telu` / `tel2`, `sinh`) → `khmr` → `mymr` / `mym2` → `hang` /
   `hani` / `kana`. The round-15 four-tag prefix is preserved
   verbatim at the head of the list — every Latin / Cyrillic / Greek
   / DFLT resolution sees identical behaviour (no-regression
   guarantee, asserted by
   `round175_broadened_probe_preserves_latn_smcp_result`). Non-Latin
   tags resolve only when none of the round-15 four publishes the
   requested feature, which is the typical case for non-Latin runs.

The two together close a long-standing gap: until round 175, the
caller-driven `shape_text` API was limited to scripts that publish
the requested feature under one of `latn` / `cyrl` / `grek` /
`DFLT`. A font publishing `liga` only under `arab` (typical for
Arabic-only fonts) or a CJK font publishing `vert` under `hani` /
`kana` / `hang` was unreachable. Round 175 makes both paths work
without requiring callers to drop to `with_font` and call the raw
`oxideav-ttf` accessors.

A new export `shape_text_with_script_with_font` mirrors
`shape_text_with_font` at the function level for callers using
`Face::with_font` directly.

Tests:

- `src/shaping/feature_subst.rs` adds 6 in-module tests covering
  the probe-list invariants
  (`round175_probe_list_prefix_is_round15_priority`,
  `round175_probe_list_covers_broadened_scripts`), the explicit-
  script API contracts
  (`round175_explicit_unknown_script_is_cmap_identity`,
  `round175_explicit_empty_features_is_cmap_identity`,
  `round175_explicit_latn_matches_auto_probe_on_smcp`,
  `round175_explicit_dflt_unknown_feature_is_cmap_identity`), and
  the no-regression guarantee on the broadened auto-probe
  (`round175_broadened_probe_preserves_latn_smcp_result`).
- `tests/round175_shape_text_with_script.rs` adds 8 integration
  tests covering the public `Face::shape_text_with_script` surface
  (auto-probe agreement on `smcp` / `liga`, unknown-script
  cmap-identity, empty-text / empty-features baselines, `DFLT`
  resolution, caller-order preservation, and the cmap-baseline
  agreement with the auto-probe path).

Doc updates: README "Capabilities" gets a round-175 paragraph;
`lib.rs` re-exports `shape_text_with_script_with_font`;
`Face::shape_text` docstring is updated to describe the broadened
probe list.

### Added — GSUB LookupType 3 (Alternate Substitution) in `shape_text` (round 156)

`Face::shape_text` now dispatches GSUB LookupType 3 (Alternate
Substitution, Format 1) alongside the round-89 LookupType 1
(Single), round-125 LookupType 2 (Multiple), and round-128
LookupType 4 (Ligature) paths. For each covered slot the shaper
calls `oxideav-ttf::Font::gsub_apply_lookup_type_3(idx, gid, 0)`
to pick the first entry of the slot's `AlternateSet` and rewrites
the slot in place — Type 3 is length-preserving so the walker
mirrors the Type-1 single-substitution walker exactly.

The OpenType spec (§6.2.3 "Alternate Substitution Subtable")
defines exactly one format: Coverage on each input glyph plus a
per-coverage `AlternateSet` listing one or more
`alternateGlyphIDs[]`. The spec deliberately leaves "the
application of the OpenType Layout engine selects an alternate"
unpinned — we default to `alternateIndex = 0`, which is what the
`aalt` and `salt` features are designed to produce when consulted
without a user-specified pick. A higher-level surface that wanted
to expose user-driven indices would belong above this layer (the
existing `oxideav-ttf` accessor already takes `alternate_index`
per call).

The headline use case is `aalt` (Access All Alternates) — a
near-universal OpenType feature that publishes a Type-1 component
(single substitution into the designer-selected principal
alternate) plus a Type-3 component (the full per-glyph
`AlternateSet` for ad-hoc alternate access). Every test-fixture
font in `tests/fixtures/` ships an `aalt` feature with a Type-3
lookup; pre-round-156 the Type-3 component was silently skipped,
so a glyph covered *only* by the Type-3 lookup passed through
unchanged. Round 156 wires it into the caller-driven surface.

Tests: `tests/round156_alternate_subst.rs` adds 7 integration
tests covering both rich-coverage (Inter Variable, 37 Type-3 hits
across ASCII probes) and sparse-coverage (DejaVu Sans, 5 hits at
'I', 'J', 'a', 'l', 'y') fonts:

- `inter_aalt_substitutes_via_lookup_type_3` — the headline
  contract: `shape_text("a", &[aalt])` reshapes via the Type-3
  alternate-0 (different gid from `cmap('a')`, length 1) where
  previously the Type-3 lookup was silently skipped.
- `inter_aalt_reshapes_many_lowercase_slots` — bulk-coverage
  check: at least 5 of 7 lowercase ASCII letters reshape via the
  Type-3 lookup.
- `dejavu_aalt_is_pure_type_3` — `aalt` on DejaVu is a single
  Type-3 lookup; round-156 reshapes 'I' / 'a' / 'l' / 'y' /
  duplicates in `"Iaaly"`.
- `dejavu_aalt_outside_coverage_is_cmap_identity` — uncovered
  glyphs ('b' / 'c' / 'd' / 'e' / 'f' / 'g' on DejaVu) pass through
  unchanged.
- `aalt_is_idempotent_on_inter` — coverage is on the input glyphs,
  not the substitutes, so re-applying `aalt` is a no-op.
- `aalt_does_not_affect_run_length` — Type 3 is length-preserving
  across both fixtures and several input compositions.
- `font_without_aalt_is_cmap_identity` — unknown feature tag
  (`zzzz`) doesn't accidentally pull in a Type-3 lookup.

Plus two unit tests in `src/shaping/feature_subst.rs`
(`aalt_dispatches_lookup_type_3_on_inter`,
`aalt_is_idempotent_on_dejavu`) mirroring the existing per-module
test convention.

Lookups of the remaining declared types (5 / 6 / 8) referenced by
the requested features are still silently skipped on the
caller-driven surface — contextual / chained-contextual /
reverse-chained substitutions continue to flow through
`Shaper::shape` / `FaceChain::shape` via `shaping::general`.

### Added — GSUB LookupType 4 (Ligature Substitution) in `shape_text` (round 128)

`Face::shape_text` now dispatches GSUB LookupType 4 (Ligature
Substitution, Format 1) alongside the round-89 LookupType 1
(Single Substitution) and round-125 LookupType 2 (Multiple
Substitution) paths. The shaper walks the cmap'd run
left-to-right; at each cursor position it asks
`oxideav-ttf::Font::gsub_apply_lookup_type_4(idx, &gids[pos..])`
whether the lookup matches a prefix starting at the cursor.
On a hit the `consumed` glyphs `gids[pos..pos+consumed]` are
spliced to the single replacement glyph and the cursor advances
by 1 (past the new ligature). The advance-by-1 is what
`Shaper::shape`'s round-1 ligature pass already does and is
the natural mirror of the round-125 Type-2 walker.

The canonical use case is `liga` / `dlig` / `rlig` collapsing
multi-glyph component sequences into a single ligature glyph
(e.g. fi / fl / ffi / ffl on DejaVu Sans, or the historic
ct / st discretionary ligatures on serif text fonts). The
`Shaper::shape` / `FaceChain::shape` pipeline already collapsed
ligatures via the round-1 `lookup_ligature` walker; round 128
brings the same capability to the caller-driven
`shape_text(text, features)` surface so explicit per-call feature
lists can now express ligature substitution as well.

Tests: `tests/round128_ligature_subst.rs` adds 17 new
integration tests around a per-byte synthetic TTF (no external
library consulted; every byte layout follows the Microsoft
Typography OpenType spec — chapter 6 §6.2.4 LigatureSubstFormat1
plus the common-table Coverage Format 1 layout, transcribed
from the published spec):

- A parameterised builder that produces a 5-glyph TTF
  (`.notdef` + up to 4 letters + ligature glyphs) with a
  Format-12 cmap and a GSUB table publishing one `liga` feature
  under script `DFLT` with one LigatureSubstFormat1 lookup
  populated from a caller-supplied `LigatureSet` table.
- `synthetic_font_cmap_routes_a_b_c`,
  `synthetic_font_publishes_liga_under_dflt`, and
  `synthetic_font_has_one_lookup_type_4` lock the parsed shape
  of the synthetic font (cmap mapping + feature publication +
  GSUB lookup count / type).
- `liga_collapses_two_components_into_ligature` is the headline
  contract: shape_text("ab", [liga]) returns the single
  replacement glyph instead of the cmap'd [1, 2].
- `liga_is_noop_on_uncovered_prefix` /
  `liga_is_noop_when_tail_doesnt_match` cover the two miss
  paths (first-component-not-in-coverage and
  first-matches-but-tail-doesn't).
- `liga_mixed_input_collapses_only_matching_prefix` verifies
  multiple ligature matches in a single run.
- `liga_partial_then_match` walks past an uncovered glyph and
  fires the ligature at the next position.
- `liga_does_not_re_match_its_own_output` is the idempotence
  guard.
- `liga_empty_text_yields_empty_run`,
  `liga_empty_features_is_cmap_identity_on_ab`, and
  `unknown_feature_skips_type_4_lookup` are the empty-input and
  unknown-feature baselines.
- `liga_longest_match_first_picks_3_glyph_ligature` /
  `liga_longest_match_first_falls_back_when_tail_missing` cover
  the spec-mandated longest-match-first ordering within a single
  LigatureSet (a 3-glyph ligature with the same first component
  must win over a 2-glyph one when the trailing components are
  present, and fall back to the 2-glyph one when they aren't).
- `liga_two_sets_each_fires_independently` /
  `liga_two_sets_first_set_only_on_partial_input` cover
  multiple LigatureSets routed by coverage (different first
  components, each with their own ligature record).
- `liga_single_component_record_behaves_like_single_subst`
  covers the spec-legal-but-rare `componentCount = 1` edge case
  (effectively a Single Substitution dressed as a ligature).

Plus three new in-module tests in
`src/shaping/feature_subst.rs` (existing 9 → 11) replacing the
pre-128 `liga_is_skipped_because_it_is_lookup_type_4` test with
three new tests against real DejaVu Sans:
`liga_collapses_fi_via_lookup_type_4` (fi → single ligature
glyph), `liga_leaves_uncovered_glyphs_alone` (mixed "abfi" → 3
glyphs not 4), and `liga_is_identity_on_uncovered_run` ("abc"
unchanged because no f/l/i prefix matches).

The pre-existing `tests/round89_single_subst.rs ::
dejavu_liga_is_skipped_because_lookup_type_is_4` test was
renamed to `dejavu_liga_is_now_dispatched_as_lookup_type_4`
and its assertions flipped to lock the new round-128 contract
(fi cmap'd to 2 glyphs, liga collapses to 1 different glyph).

Lookups of any other declared type (Alternate = 3, Context = 5,
ChainContext = 6, ReverseChainContext = 8) referenced by the
requested features remain silently skipped on the `shape_text`
surface; the broader `apply_one_lookup` walker in
`shaping::general` continues to dispatch all declared types end-
to-end through the always-on `ccmp` / `calt` passes.

Clean-room note: OpenType §6.2.4 LigatureSubstFormat1 byte
layout is implemented inside `oxideav-ttf` (already shipped; no
scribe-side decode work this round); scribe's round-128 layer
is a thin dispatcher plus a per-byte synthetic-fixture builder
transcribed from the published OpenType spec. No HarfBuzz /
FreeType / Pango / Skia source was consulted; no WebSearch /
WebFetch was invoked during this round.

### Added — GSUB LookupType 2 (Multiple Substitution) in `shape_text` (round 125)

`Face::shape_text` now dispatches GSUB LookupType 2 (Multiple
Substitution, Format 1) alongside the round-89 LookupType 1
(Single Substitution) path. The shaper walks the cmap'd run
left-to-right; when a Type-2 lookup's coverage matches a slot,
the single input glyph is spliced out and replaced by the
`Sequence` record's `glyphCount` substituteGlyphIDs, and the
cursor advances past the inserted run so the same lookup
doesn't re-match its own output. The OpenType spec (§6.2.2)
explicitly permits `glyphCount = 0` as a deletion form; the
returned `Vec<u16>` reflects the post-substitution length,
which may be 0, shorter than, equal to, or longer than the
input.

The canonical use case is `ccmp` "split a precomposed glyph
into base + combining mark" so a subsequent GPOS mark-
attachment pass can position the mark independently. The
`shaping::general` `ccmp` / `calt` pipeline already dispatched
Type 2 since round 15; round 125 brings the same capability
to the caller-driven `shape_text(text, features)` surface so
explicit per-call feature lists (`smcp`, `c2sc`, `frac`,
`salt`, `ss01..ss20`, `cv01..cv99`, etc.) can now express
multiple-substitution lookups as well.

Tests: `tests/round125_multi_subst.rs` adds 11 new integration
tests around a per-byte synthetic TTF (no external library
consulted; every byte layout follows the Microsoft Typography
OpenType spec):

- A minimal 4-glyph TTF (`.notdef` + 'a' + 'b' + 'c') with a
  Format-4 cmap mapping 'a'/'b'/'c' to GIDs 1/2/3 and a GSUB
  table publishing one `ccmp` feature under script `DFLT`
  with one MultipleSubstFormat1 lookup. The lookup's
  coverage and Sequence record are parameterised, so the
  same builder produces both the split-variant (gid 1 →
  [2, 3]) and the deletion-variant (gid 1 → []).
- `synthetic_font_cmap_routes_a_b_c`,
  `synthetic_font_publishes_ccmp_under_dflt`, and
  `synthetic_font_has_one_lookup_type_2` lock the parsed shape
  of the synthetic font (cmap mapping + feature publication +
  GSUB lookup count / type).
- `ccmp_splits_a_into_b_c_via_lookup_type_2` is the headline
  contract: shape_text("a", [ccmp]) returns [2, 3] instead of
  the cmap [1].
- `ccmp_is_noop_on_uncovered_glyph` asserts coverage gating
  (gid 2 isn't in the lookup's coverage; shaping "b" is
  identity).
- `ccmp_mixed_input_expands_only_covered_slot` verifies "ab"
  expands the 'a' slot and leaves 'b' intact → output is
  `[2, 3, 2]`, length 3.
- `ccmp_walker_does_not_re_match_its_own_output` /
  `ccmp_lookup_type_2_glyph_count_zero_deletes_input` cover
  the walker's advance-past-substitution contract and the
  spec-legal `glyphCount = 0` deletion form.
- `ccmp_empty_text_yields_empty_run`,
  `ccmp_empty_features_is_cmap_identity_on_a`, and
  `unknown_feature_skips_type_2_lookup` are the empty-input
  and unknown-feature baselines.

Plus three new in-module tests in
`src/shaping/feature_subst.rs` (existing 9 → 12) updating the
`liga`-is-skipped contract docstring to reflect the
LookupType-1/2 surface (LookupType 4 ligature work still
flows through `Shaper::shape` / `FaceChain::shape`).

Lookups of any other declared type (Alternate = 3, Ligature
= 4, Context = 5, ChainContext = 6, ReverseChainContext = 8)
referenced by the requested features remain silently skipped
on the `shape_text` surface; the broader `apply_one_lookup`
walker in `shaping::general` continues to dispatch all
declared types.

Clean-room note: OpenType §6.2.2 MultipleSubstFormat1 byte
layout is implemented inside `oxideav-ttf` (already shipped;
no scribe-side decode work this round); scribe's round-125
layer is a thin dispatcher plus a per-byte synthetic-fixture
builder transcribed from the published OpenType spec. No
HarfBuzz / FreeType / Pango / Skia source was consulted; no
WebSearch / WebFetch was invoked during this round.

### Added — caller-driven GSUB LookupType 1 application (round 89)

A new public surface — `Face::shape_text(text, features) -> Vec<u16>` —
cmap's the input text and applies every **GSUB LookupType 1 (Single
Substitution)** lookup the requested feature tags reference under
`latn` / `cyrl` / `grek` / `DFLT`. Format 1 (delta) and Format 2
(substitute-array) sub-tables per OpenType §6.2.1 are both
dispatched through `oxideav_ttf::Font::gsub_apply_lookup_type_1`;
LookupType-7 ExtensionSubst wrappers around a Type-1 lookup are
unwrapped transparently.

Round-89 scope is single-substitution only. Lookups of other
declared types (Multiple, Alternate, Ligature, Contextual,
ChainContext, ReverseChainContext) referenced by the requested
features are silently skipped — ligature collapsing and contextual
rules remain on the existing `Shaper::shape` / `FaceChain::shape`
pipeline.

Typical feature tags this surface unlocks (none of which the
always-on round-15 `ccmp` / `calt` passes touch):

- `smcp` / `c2sc` — small caps (from lower / from upper).
- `case` — case-sensitive forms.
- `salt` / `ss01..ss20` / `cv01..cv99` — stylistic alternates,
  sets, and per-character variants.
- `sups` / `subs` / `numr` / `dnom` / `ordn` — vertical /
  role-based number forms.
- `frac` — fractions (the Type-1 digit reshape component only;
  the contextual `1/2` collapse rule is a Type-4/5 lookup and
  remains a TODO for a later round).
- `zero` — slashed zero.
- `pnum` / `tnum` — proportional / tabular numerals.

Round-89 test coverage adds 20 new tests (9 in
`src/shaping/feature_subst.rs`, 11 in
`tests/round89_single_subst.rs`):

- Empty-text and empty-features baselines (cmap identity).
- Inter Variable `smcp` reshapes lowercase ASCII (`"Hi"` →
  `[H_gid, smcp(i_gid)]` — upper-case stays unchanged because it's
  outside `smcp` coverage; lowercase reshapes to small-cap variant).
- Inter `sups` reshapes most digit slots; `subs` is independent
  from `sups`.
- Inter `salt` is well-defined on lowercase.
- Caller-ordered features: `[*b"smcp", *b"case"]` on lowercase
  matches `[*b"smcp"]` alone (case is a no-op for letters);
  `[*b"smcp", *b"smcp"]` is idempotent (re-applying smcp doesn't
  match the post-substitution glyphs).
- DejaVu Sans's `liga` (LookupType 4) is the documented no-op —
  proves the Type-1-only contract.
- Unknown feature tag (`zzzz`) and font without the requested
  feature (DejaVu's missing `smcp`) both pass through unchanged.

Clean-room note: OpenType §6.2.1 Format 1/2 byte layouts are
implemented inside `oxideav-ttf` (which has its own clean-room
provenance); scribe's round-89 layer is a thin dispatcher and does
not re-derive the table format itself. No HarfBuzz / FreeType /
Pango / Skia source was consulted; no WebSearch / WebFetch was
invoked during this round.

### Added — GSUB feature-tag introspection (round 88)

Two new accessors on `Face` surface the OpenType feature-tag set the
underlying GSUB table publishes:

- `Face::gsub_features_for_script(script_tag, lang_tag)` → `Vec<[u8; 4]>` —
  returns the four-byte feature tags published under a script (in
  declaration order, required-feature first per the underlying
  `oxideav-ttf` contract).
- `Face::has_gsub_feature(script_tag, feature_tag)` → `bool` —
  convenience predicate for callers that just need to gate on
  feature presence.

Both are pure pass-through accessors over
`oxideav_ttf::Font::gsub_features_for_script`; semantics live in
`oxideav-ttf`. Returns an empty vec / `false` for OTF faces, fonts
without a GSUB table, and unknown / absent script tags.

Round-88 test coverage adds 10 new tests in
`tests/round88_gsub_features.rs`:

- Introspection: unknown-script / unknown-feature empty cases.
- Introspection: DejaVu Sans `latn` publishes `ccmp` and `liga`;
  every tag is printable ASCII.
- Introspection: Inter Variable `latn` snapshots the full tag set
  (38 tags including `aalt` / `c2sc` / `calt` / `case` / `ccmp` /
  `cv01..cv13` / `dlig` / `dnom` / `frac` / `numr` / `ordn` / `pnum` /
  `salt` / `sinf` / `smcp` / `ss01..ss08` / `subs` / `sups` / `tnum` /
  `zero`). Documents Inter's deliberate omission of `liga` (lives in
  `dlig`/`calt`) and `kern` (GPOS-only — a future GPOS introspection
  accessor is needed).
- Round-15 pipeline on a second font: Inter Variable's `ccmp` lookup
  substitutes the dotless-i variant before a combining-above mark, and
  a 56-char ASCII pangram is byte-identical to the cmap-only output
  (no calt false-positives).

### Added — general-script `ccmp` + `calt` (round 15)

Wires the OpenType **required-feature** `ccmp` (Glyph Composition /
Decomposition) as a pre-ligature pass and `calt` (Contextual
Alternates) as a post-ligature pass into the post-cmap path of
`shape_run_with_font`. Both passes resolve features through the new
`shaping::general` module, which probes the script-tag list `latn` /
`cyrl` / `grek` / `DFLT` in priority order and dispatches every lookup
under the chosen feature according to its declared GSUB LookupType:

- LookupType 1 (single substitution) — already used by Indic.
- **LookupType 2 (multiple substitution)** — now reachable for the
  first time. The canonical decomposition site: `ç → c + combining
  cedilla`, etc.
- LookupType 3 (alternate substitution) — picks the spec-default
  alternate index 0 (user-driven indices belong on a higher API).
- LookupType 4 (ligature substitution) — feature-tag-aware variant of
  the existing untargeted walker.
- LookupType 5 / 6 (contextual / chained context) — already used by
  Indic; now reachable for Latin too. Fonts use type-6 chained context
  to make `ccmp` rules condition on whether a following mark is
  present (e.g. DejaVu Sans rewrites `i → dotless-i` only before a
  combining-above mark).
- LookupType 8 (reverse chained context) — single-position dispatch.

**Measured delta against DejaVu Sans:**

- `chain.shape("i\u{0307}")` previously produced `[gid_i, gid_dot]`
  (visually wrong — dot of "i" collides with combining dot). Now
  produces `[gid_dotless_i, gid_dot]`, matching the font's published
  ccmp rule.
- `chain.shape("I\u{0307}")` previously produced
  `[gid_I, gid_dot_default]`; now produces
  `[gid_I, gid_dot_capital_variant]`, matching the case-specific
  ccmp lookup.
- ASCII-only runs (`"Hello, world!"`) are bit-identical to round-14
  output — the coverage tables ensure the new pass is a no-op for any
  glyph not in the `ccmp` / `calt` coverage.

Four new integration tests (`tests/round15_ccmp_calt.rs`) lock the
positive cases AND the negative-control (`"a\u{0301}"` is a pass-through
because DejaVu Sans publishes no rule for that pair). Total test count:
lib 268→270, integration 64→68.

No external library source was consulted. The feature-application
ordering follows the spec's "required features first" rule + the
public OpenType registry's documented feature semantics for `ccmp` /
`calt`; the round-15 lookup-dispatch code is built from `oxideav-ttf`'s
public per-type entry points and the lookup-type metadata returned by
`Font::gsub_lookup_list`.

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
