# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
