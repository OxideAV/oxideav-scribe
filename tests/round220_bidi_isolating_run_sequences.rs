//! Round 220 — UAX #9 §3.3.3 X10 isolating-run-sequence partition
//! (built on top of BD7 level runs and BD13).
//!
//! Mirrors the per-rule unit tests in `src/bidi.rs` but exercises the
//! public `oxideav_scribe::isolating_run_sequences` re-export so the
//! surface stays stable for external callers.
//!
//! Provenance: every input is constructed by hand from the BD7 /
//! BD9 / BD13 definitions in UAX #9 Revision 50 / Unicode 16.0
//! §3.1.2 + §3.3.3 (the dated snapshot at
//! `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`).

use oxideav_scribe::{
    bidi_class, isolating_run_sequences, level_runs, paragraph_level, resolve_explicit_levels,
    resolve_implicit_levels, resolve_neutral_types, resolve_weak_types, BidiClass,
    IsolatingRunSequence, LevelRun,
};

// --- BD7 level runs -------------------------------------------------

#[test]
fn r220_level_runs_empty() {
    let runs = level_runs(&[]);
    assert!(runs.is_empty());
}

#[test]
fn r220_level_runs_uniform_level_one_run() {
    let runs = level_runs(&[0, 0, 0, 0]);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].start, 0);
    assert_eq!(runs[0].end, 4);
    assert_eq!(runs[0].level, 0);
    assert_eq!(runs[0].len(), 4);
    assert!(!runs[0].is_empty());
}

#[test]
fn r220_level_runs_split_on_each_level_change() {
    let runs = level_runs(&[0, 0, 1, 1, 2, 2, 2, 0]);
    let raw: Vec<_> = runs.iter().map(|r| (r.start, r.end, r.level)).collect();
    assert_eq!(raw, vec![(0, 2, 0), (2, 4, 1), (4, 7, 2), (7, 8, 0)]);
}

#[test]
fn r220_level_runs_coverage_invariant() {
    // BD7: level runs partition the paragraph — every index from 0
    // to len() belongs to exactly one run, ranges are contiguous.
    let levels = [3, 3, 0, 0, 0, 1, 1, 0, 2];
    let runs = level_runs(&levels);
    let mut expect_next = 0;
    for r in &runs {
        assert_eq!(r.start, expect_next);
        expect_next = r.end;
    }
    assert_eq!(expect_next, levels.len());
}

// --- X10 sos / eos derivation ---------------------------------------

#[test]
fn r220_pure_latin_paragraph_emits_one_sequence() {
    let cls: Vec<_> = "Hello".chars().map(bidi_class).collect();
    let pl = paragraph_level("Hello");
    let out = resolve_explicit_levels(&cls, pl);
    let seqs = isolating_run_sequences(&cls, &out, pl);
    assert_eq!(seqs.len(), 1);
    assert_eq!(seqs[0].level, 0);
    assert_eq!(seqs[0].sos, BidiClass::L);
    assert_eq!(seqs[0].eos, BidiClass::L);
    assert_eq!(seqs[0].runs.len(), 1);
    assert_eq!(seqs[0].runs[0].start, 0);
    assert_eq!(seqs[0].runs[0].end, 5);
}

#[test]
fn r220_pure_hebrew_paragraph_level_one_emits_r_sos_eos() {
    let cls: Vec<_> = "\u{05E9}\u{05DC}\u{05D5}\u{05DD}"
        .chars()
        .map(bidi_class)
        .collect();
    let pl = paragraph_level("\u{05E9}\u{05DC}\u{05D5}\u{05DD}");
    assert_eq!(pl, 1);
    let out = resolve_explicit_levels(&cls, pl);
    let seqs = isolating_run_sequences(&cls, &out, pl);
    assert_eq!(seqs.len(), 1);
    assert_eq!(seqs[0].level, 1);
    assert_eq!(seqs[0].sos, BidiClass::R);
    assert_eq!(seqs[0].eos, BidiClass::R);
}

#[test]
fn r220_rle_split_three_sequences() {
    // L RLE L PDF L — three sequences, one per level run.
    let cls = vec![
        BidiClass::L,
        BidiClass::RLE,
        BidiClass::L,
        BidiClass::PDF,
        BidiClass::L,
    ];
    let out = resolve_explicit_levels(&cls, 0);
    let seqs = isolating_run_sequences(&cls, &out, 0);
    assert_eq!(seqs.len(), 3);
    // First: L @ 0, sos L (paragraph), eos R (boundary higher level 1).
    assert_eq!(seqs[0].sos, BidiClass::L);
    assert_eq!(seqs[0].eos, BidiClass::R);
    // Middle: L @ 1, sos = eos = R.
    assert_eq!(seqs[1].sos, BidiClass::R);
    assert_eq!(seqs[1].eos, BidiClass::R);
    // Last: L @ 0, sos R, eos L (paragraph edge).
    assert_eq!(seqs[2].sos, BidiClass::R);
    assert_eq!(seqs[2].eos, BidiClass::L);
}

#[test]
fn r220_lri_chains_to_matching_pdi_run() {
    // L LRI L PDI L — LRI raises to level 2 for the inner L; the
    // chained outer sequence covers (0..2)@0 + (3..5)@0.
    let cls = vec![
        BidiClass::L,
        BidiClass::LRI,
        BidiClass::L,
        BidiClass::PDI,
        BidiClass::L,
    ];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels, vec![0, 0, 2, 0, 0]);
    let seqs = isolating_run_sequences(&cls, &out, 0);
    assert_eq!(seqs.len(), 2);
    // First (chained): runs 0..2 and 3..5 share level 0.
    assert_eq!(seqs[0].level, 0);
    assert_eq!(seqs[0].runs.len(), 2);
    assert_eq!(seqs[0].runs[0].start, 0);
    assert_eq!(seqs[0].runs[0].end, 2);
    assert_eq!(seqs[0].runs[1].start, 3);
    assert_eq!(seqs[0].runs[1].end, 5);
    assert_eq!(seqs[0].sos, BidiClass::L);
    assert_eq!(seqs[0].eos, BidiClass::L);
    // Second (isolate body): run 2..3 at level 2.
    assert_eq!(seqs[1].level, 2);
    assert_eq!(seqs[1].runs.len(), 1);
    assert_eq!(seqs[1].runs[0].start, 2);
    assert_eq!(seqs[1].runs[0].end, 3);
    // sos / eos both rooted by paragraph fallbacks at level 0 +
    // sequence level 2 → higher 2 (even) → L per X10 step 2.
    assert_eq!(seqs[1].sos, BidiClass::L);
    assert_eq!(seqs[1].eos, BidiClass::L);
}

#[test]
fn r220_unmatched_lri_keeps_outer_run_in_its_own_sequence() {
    // L LRI L — LRI raises for the inner L; no matching PDI exists,
    // so the outer sequence does not chain.
    let cls = vec![BidiClass::L, BidiClass::LRI, BidiClass::L];
    let out = resolve_explicit_levels(&cls, 0);
    assert_eq!(out.levels, vec![0, 0, 2]);
    let seqs = isolating_run_sequences(&cls, &out, 0);
    assert_eq!(seqs.len(), 2);
    assert_eq!(seqs[0].runs.len(), 1);
    assert_eq!(seqs[1].runs.len(), 1);
}

#[test]
fn r220_paragraph_level_one_default_fallback_emits_r() {
    let cls = vec![BidiClass::R, BidiClass::R, BidiClass::R];
    let out = resolve_explicit_levels(&cls, 1);
    let seqs = isolating_run_sequences(&cls, &out, 1);
    assert_eq!(seqs.len(), 1);
    assert_eq!(seqs[0].sos, BidiClass::R);
    assert_eq!(seqs[0].eos, BidiClass::R);
}

// --- BD13 invariants ------------------------------------------------

#[test]
fn r220_every_level_run_in_exactly_one_sequence() {
    // BD13 invariant: every level run belongs to exactly one
    // isolating run sequence.
    let cls = vec![
        BidiClass::L,
        BidiClass::RLE,
        BidiClass::AL,
        BidiClass::EN,
        BidiClass::LRI,
        BidiClass::L,
        BidiClass::PDI,
        BidiClass::L,
        BidiClass::PDF,
        BidiClass::L,
    ];
    let out = resolve_explicit_levels(&cls, 0);
    let total_runs = level_runs(&out.levels).len();
    let seqs = isolating_run_sequences(&cls, &out, 0);
    let counted: usize = seqs.iter().map(|s| s.runs.len()).sum();
    assert_eq!(counted, total_runs);
}

#[test]
fn r220_all_runs_in_sequence_share_embedding_level() {
    // BD13 closing note: "all the level runs in an isolating run
    // sequence have the same embedding level."
    let cls = vec![
        BidiClass::L,
        BidiClass::LRI,
        BidiClass::L,
        BidiClass::LRI,
        BidiClass::L,
        BidiClass::PDI,
        BidiClass::L,
        BidiClass::PDI,
        BidiClass::L,
    ];
    let out = resolve_explicit_levels(&cls, 0);
    let seqs = isolating_run_sequences(&cls, &out, 0);
    for s in &seqs {
        for r in &s.runs {
            assert_eq!(r.level, s.level);
        }
    }
}

// --- Indices iterator + X9 removal interaction ----------------------

#[test]
fn r220_indices_iterator_skips_x9_removed_positions() {
    let cls = vec![
        BidiClass::L,
        BidiClass::RLE,
        BidiClass::L,
        BidiClass::PDF,
        BidiClass::L,
    ];
    let out = resolve_explicit_levels(&cls, 0);
    let seqs = isolating_run_sequences(&cls, &out, 0);
    // sequence 1 covers indices 1..3 → drops RLE at 1, keeps L at 2.
    let walk: Vec<usize> = seqs[1].indices(&out.removed).collect();
    assert_eq!(walk, vec![2]);
}

#[test]
fn r220_indices_iterator_preserves_isolate_format_chars() {
    // X9 explicitly does NOT remove LRI / RLI / FSI / PDI: they
    // participate in the W/N/I passes as neutral characters.
    let cls = vec![
        BidiClass::L,
        BidiClass::LRI,
        BidiClass::L,
        BidiClass::PDI,
        BidiClass::L,
    ];
    let out = resolve_explicit_levels(&cls, 0);
    let seqs = isolating_run_sequences(&cls, &out, 0);
    // Chained sequence (level 0) covers logical indices 0, 1, 3, 4.
    let walk: Vec<usize> = seqs[0].indices(&out.removed).collect();
    assert_eq!(walk, vec![0, 1, 3, 4]);
}

// --- End-to-end compose: X → W → N → I via per-sequence sos/eos -----

#[test]
fn r220_end_to_end_pipeline_drives_w_n_i_per_sequence() {
    // L AL EN — paragraph level 0, all on level 0, one sequence.
    // After X: classes [L, AL, EN]. After W3 + W4 (sos = eos = L):
    //   AL → R, EN unchanged. After N: no neutral. After I1 (even):
    //   L stays 0, R goes 1, EN goes 2. The result drives the
    //   line reorderer for the final visual order.
    let cls = vec![BidiClass::L, BidiClass::AL, BidiClass::EN];
    let out = resolve_explicit_levels(&cls, 0);
    let seqs = isolating_run_sequences(&cls, &out, 0);
    assert_eq!(seqs.len(), 1);
    let s = &seqs[0];
    let mut working: Vec<BidiClass> = s
        .indices(&out.removed)
        .map(|i| out.effective_classes[i])
        .collect();
    resolve_weak_types(&mut working, s.sos, s.eos);
    // After W3 the AL must be gone (W3 rewrites AL → R).
    assert!(working.iter().all(|c| !matches!(c, BidiClass::AL)));
    resolve_neutral_types(&mut working, s.level, s.sos, s.eos);
    let resolved = resolve_implicit_levels(&working, s.level);
    // 3 levels emitted.
    assert_eq!(resolved.len(), 3);
    // I1 at even embedding level: L stays even, R goes +1, EN +2.
    assert_eq!(resolved[0], 0);
    assert!(resolved[1] >= 1); // AL → R → +1.
}

#[test]
fn r220_isolating_run_sequence_supports_clone_eq() {
    // Public structural data types implement Clone and PartialEq —
    // needed for downstream renderer pipelines that snapshot
    // sequences for retry or memoisation.
    let cls = vec![BidiClass::L, BidiClass::L, BidiClass::L];
    let out = resolve_explicit_levels(&cls, 0);
    let seqs = isolating_run_sequences(&cls, &out, 0);
    let copy: IsolatingRunSequence = seqs[0].clone();
    assert_eq!(copy, seqs[0]);
    let r: LevelRun = level_runs(&out.levels)[0];
    let r_copy = r;
    assert_eq!(r_copy, r);
}
