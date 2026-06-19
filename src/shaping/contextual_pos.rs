//! GPOS contextual (LookupType 7) and chained-contextual (LookupType 8)
//! positioning.
//!
//! These two lookup types are the positioning analogues of the GSUB
//! contextual / chained-contextual substitution lookups: instead of
//! rewriting the glyph run, they recognise an input context (a glyph
//! sequence, optionally bracketed by backtrack and lookahead windows in
//! the chained variant) and, on a match, dispatch a set of nested
//! per-glyph positioning adjustments. Per the ISO/IEC 14496-22:2019 §6
//! GPOS chapter the nested actions referenced by a contextual lookup's
//! `SequenceLookupRecord`s "can only be positioning adjustments" — i.e.
//! they reference other GPOS LookupList entries by index.
//!
//! ## What the dependency resolves vs. what this module does
//!
//! [`oxideav_ttf::Font::gpos_apply_lookup_type_7`] and
//! [`oxideav_ttf::Font::gpos_apply_lookup_type_8`] decode all three
//! sub-table formats (1 glyph-sequence, 2 class-based, 3 coverage-based),
//! match the input window at a position, and recursively dispatch every
//! `SequenceLookupRecord` into its referenced GPOS lookup — returning the
//! resulting per-glyph adjustments as a `Vec<PosRecord>`. Each
//! [`oxideav_ttf::PosRecord`] carries an **absolute** `glyph_index` into
//! the run plus a [`oxideav_ttf::PosValue`] with the four geometric
//! deltas (xPlacement / yPlacement / xAdvance / yAdvance) in TT font
//! units (Y-up).
//!
//! This module's job is purely the *application*: enumerate the run's
//! contextual-positioning lookups in LookupList order, scan every input
//! position, and **accumulate** the returned deltas onto the matching
//! [`crate::shaper::PositionedGlyph`]s. The ttf record contract is
//! explicit that records are additive ("Multiple records may target the
//! same `glyph_index` if the nested lookups stack adjustments — callers
//! should add (not replace) the deltas"), so this module adds.
//!
//! ## Application order
//!
//! Per §6 common-table-format rules the union of lookups referenced by
//! the active features is applied **in LookupList order**, not feature
//! order. Because a contextual-positioning lookup can reference a lower-
//! or higher-indexed lookup through a `SequenceLookupRecord`, the ttf
//! apply call already walks the nested references at the moment the
//! outer lookup matches; this module only needs to drive the *outer*
//! type-7 / type-8 lookups, and it does so in ascending lookup index
//! (LookupList) order so two contextual lookups that both fire on the
//! same run compose deterministically.
//!
//! ## Field mapping
//!
//! A [`oxideav_ttf::PosValue`] maps onto a positioned glyph the same way
//! the SinglePos (LookupType 1) pass does in
//! [`crate::shaper::shape_run_with_font`]:
//!
//! - `x_placement` shifts the drawn position right → added to `x_offset`.
//! - `y_placement` shifts it up in TT Y-up space → subtracted from
//!   `y_offset` (raster Y-down).
//! - `x_advance` widens / narrows the horizontal advance → added to
//!   `x_advance`.
//! - `y_advance` only affects vertical-layout runs → ignored on this
//!   horizontal pen (kept for parity with the rest of the pipeline).

use crate::shaper::PositionedGlyph;
use oxideav_ttf::Font;

/// Apply every GPOS contextual (LookupType 7) and chained-contextual
/// (LookupType 8) positioning lookup the font publishes to the already-
/// positioned run `out`, in ascending LookupList order.
///
/// `scale` converts TT font units to raster pixels (`size_px / upem`).
/// The pass mutates `out` in place, accumulating each matched
/// [`oxideav_ttf::PosRecord`]'s deltas onto the glyph at its absolute
/// `glyph_index`.
///
/// Gated on the font actually publishing at least one type-7 / type-8
/// GPOS lookup, so plain fonts (the overwhelming majority — DejaVu Sans,
/// Source Sans, most Latin text faces ship none) pay exactly one
/// lookup-list scan and then return.
///
/// The scan is bounded: for each lookup we walk every input position
/// once. A contextual match at position `p` does not re-seed the scan at
/// `p` (the nested records already carry every adjustment the rule
/// emits), so the per-lookup cost is `O(run_len)` apply-calls and the
/// whole pass is `O(num_ctx_lookups * run_len)` — no self-feeding loop
/// is possible because positioning never changes the glyph ids the next
/// lookup matches against.
pub fn apply_contextual_pos(font: &Font<'_>, out: &mut [PositionedGlyph], scale: f32) {
    // An empty run has nothing to match against. A single-glyph run can
    // still match a one-glyph input context, so it is NOT short-circuited
    // here — it flows through the normal path below (where the lookup-list
    // gate makes it a no-op for the common case of a font with no
    // contextual-positioning lookups).
    if out.is_empty() {
        return;
    }

    // Collect the contextual-positioning lookups (effective type 7 or 8
    // after ExtensionPos unwrap, which `gpos_lookup_list` reports) in
    // ascending lookup-index order. The list is already index-ordered;
    // we filter and preserve that order.
    let ctx_lookups: Vec<(u16, u16)> = font
        .gpos_lookup_list()
        .into_iter()
        .filter(|&(_, ty, _)| ty == 7 || ty == 8)
        .map(|(idx, ty, _)| (idx, ty))
        .collect();
    if ctx_lookups.is_empty() {
        return;
    }

    // Snapshot the glyph ids once; positioning never mutates them, so a
    // single buffer serves every lookup and every position.
    let gids: Vec<u16> = out.iter().map(|g| g.glyph_id).collect();

    for (lookup_index, ty) in ctx_lookups {
        for pos in 0..gids.len() {
            let records = match ty {
                7 => font.gpos_apply_lookup_type_7(lookup_index, &gids, pos),
                8 => font.gpos_apply_lookup_type_8(lookup_index, &gids, pos),
                _ => None,
            };
            let Some(records) = records else { continue };
            apply_records(out, &records, scale);
        }
    }
}

/// Accumulate a batch of [`oxideav_ttf::PosRecord`] deltas onto the run.
///
/// Each record's `glyph_index` is an absolute offset into `out`; the
/// four geometric fields (TT font units, Y-up) are scaled to raster
/// pixels and **added** to the target glyph (records are additive per
/// the ttf contract — multiple records may stack on one glyph). The
/// field mapping mirrors the SinglePos (LookupType 1) pass:
/// `x_placement → x_offset`, `y_placement → -y_offset` (Y-up → Y-down),
/// `x_advance → x_advance`; `y_advance` is vertical-layout only and is
/// ignored on the horizontal pen.
///
/// An out-of-range `glyph_index` is skipped defensively — the ttf apply
/// path keeps records in bounds, but a malformed font could in principle
/// emit a stray index.
fn apply_records(out: &mut [PositionedGlyph], records: &[oxideav_ttf::PosRecord], scale: f32) {
    for rec in records {
        let Some(g) = out.get_mut(rec.glyph_index) else {
            continue;
        };
        let v = rec.value;
        g.x_offset += f32::from(v.x_placement) * scale;
        g.y_offset -= f32::from(v.y_placement) * scale;
        g.x_advance += f32::from(v.x_advance) * scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_ttf::{PosRecord, PosValue};

    fn glyph(id: u16) -> PositionedGlyph {
        PositionedGlyph {
            glyph_id: id,
            x_offset: 0.0,
            y_offset: 0.0,
            x_advance: 10.0,
            face_idx: 0,
        }
    }

    /// A single record adjusts exactly its target glyph, with the
    /// SinglePos field mapping: placement → offset (Y negated for the
    /// Y-up→Y-down flip), advance → advance.
    #[test]
    fn one_record_maps_fields_with_y_flip() {
        let mut run = vec![glyph(1), glyph(2), glyph(3)];
        let recs = vec![PosRecord {
            glyph_index: 1,
            value: PosValue {
                x_placement: 100,
                y_placement: 50,
                x_advance: -20,
                y_advance: 0,
            },
        }];
        // scale = 0.5 px/unit.
        apply_records(&mut run, &recs, 0.5);
        // Untouched neighbours.
        assert_eq!(run[0], glyph(1));
        assert_eq!(run[2], glyph(3));
        // Target: x_offset += 100*0.5 = 50; y_offset -= 50*0.5 = -25;
        // x_advance += -20*0.5 = -10 → 10 - 10 = 0.
        assert_eq!(run[1].x_offset, 50.0);
        assert_eq!(run[1].y_offset, -25.0);
        assert_eq!(run[1].x_advance, 0.0);
    }

    /// Multiple records targeting the same glyph stack additively (the
    /// ttf contract: "callers should add (not replace) the deltas").
    #[test]
    fn records_on_same_glyph_accumulate() {
        let mut run = vec![glyph(1), glyph(2)];
        let recs = vec![
            PosRecord {
                glyph_index: 0,
                value: PosValue {
                    x_placement: 10,
                    y_placement: 0,
                    x_advance: 5,
                    y_advance: 0,
                },
            },
            PosRecord {
                glyph_index: 0,
                value: PosValue {
                    x_placement: 4,
                    y_placement: 0,
                    x_advance: 1,
                    y_advance: 0,
                },
            },
        ];
        apply_records(&mut run, &recs, 1.0);
        // 14 px placement, 6 px extra advance.
        assert_eq!(run[0].x_offset, 14.0);
        assert_eq!(run[0].x_advance, 16.0);
    }

    /// An out-of-range `glyph_index` is skipped without panicking and
    /// without disturbing the in-range glyphs.
    #[test]
    fn out_of_range_record_is_skipped() {
        let mut run = vec![glyph(1)];
        let recs = vec![
            PosRecord {
                glyph_index: 5,
                value: PosValue {
                    x_placement: 99,
                    y_placement: 99,
                    x_advance: 99,
                    y_advance: 0,
                },
            },
            PosRecord {
                glyph_index: 0,
                value: PosValue {
                    x_placement: 2,
                    y_placement: 0,
                    x_advance: 0,
                    y_advance: 0,
                },
            },
        ];
        apply_records(&mut run, &recs, 1.0);
        assert_eq!(run[0].x_offset, 2.0);
    }

    /// `y_advance` is vertical-layout only and never touches the
    /// horizontal pen's fields.
    #[test]
    fn y_advance_is_ignored_on_horizontal_pen() {
        let mut run = vec![glyph(1)];
        let recs = vec![PosRecord {
            glyph_index: 0,
            value: PosValue {
                x_placement: 0,
                y_placement: 0,
                x_advance: 0,
                y_advance: 1000,
            },
        }];
        apply_records(&mut run, &recs, 1.0);
        assert_eq!(run[0], glyph(1));
    }

    /// A run shorter than what any context could match is left
    /// untouched, and the pass never panics on the empty run.
    #[test]
    fn empty_and_single_glyph_runs_are_noops() {
        let bytes = include_bytes!("../../tests/fixtures/DejaVuSans.ttf").to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu Sans parses");
        face.with_font(|font| {
            let mut empty: Vec<PositionedGlyph> = Vec::new();
            apply_contextual_pos(font, &mut empty, 0.02);
            assert!(empty.is_empty());

            let mut one = vec![PositionedGlyph {
                glyph_id: 5,
                x_offset: 0.0,
                y_offset: 0.0,
                x_advance: 10.0,
                face_idx: 0,
            }];
            let before = one.clone();
            apply_contextual_pos(font, &mut one, 0.02);
            assert_eq!(one, before);
        })
        .unwrap();
    }

    /// DejaVu Sans publishes no GPOS contextual-positioning lookups, so
    /// a multi-glyph run passes through unchanged. This proves the
    /// "no type-7/8 lookups → exactly one lookup-list scan → identity"
    /// fast path.
    #[test]
    fn font_without_contextual_pos_is_identity() {
        let bytes = include_bytes!("../../tests/fixtures/DejaVuSans.ttf").to_vec();
        let face = crate::Face::from_ttf_bytes(bytes).expect("DejaVu Sans parses");
        face.with_font(|font| {
            let has_ctx = font
                .gpos_lookup_list()
                .iter()
                .any(|&(_, ty, _)| ty == 7 || ty == 8);
            assert!(
                !has_ctx,
                "DejaVu Sans is expected to ship no type-7/8 GPOS lookups"
            );
            let mut run: Vec<PositionedGlyph> = (0..5)
                .map(|i| PositionedGlyph {
                    glyph_id: 10 + i,
                    x_offset: 1.0,
                    y_offset: 2.0,
                    x_advance: 8.0,
                    face_idx: 0,
                })
                .collect();
            let before = run.clone();
            apply_contextual_pos(font, &mut run, 0.02);
            assert_eq!(run, before);
        })
        .unwrap();
    }
}
