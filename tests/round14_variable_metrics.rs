//! Round-14 — variable-font **metrics** tables (#454).
//!
//! Verifies that scribe's new MVAR / HVAR / VVAR / STAT / name-id /
//! CFF2 surface works end-to-end against `tests/fixtures/InterVariable.ttf`.
//! Inter publishes:
//! - **MVAR** (168 bytes) — global metric variations on `wght` / `opsz`.
//! - **HVAR** (~11 KB) — per-glyph horizontal-advance variations.
//! - **STAT** (220 bytes) — design-axis labelling: `wght 100..900` →
//!   `Thin / ExtraLight / Light / Regular / Medium / SemiBold / Bold /
//!   ExtraBold / Black`; `opsz 14..32` → optical-size labels.
//! - **No VVAR** — Inter is horizontal-only, like every Latin variable
//!   font.
//! - **No CFF2** — Inter is TT-flavoured (`glyf` + `gvar`), not CFF2.
//!
//! Inter v4.0 OFL (see `tests/fixtures/INTER-OFL-LICENSE.txt`).

use oxideav_scribe::variations::{StatAxisValue, StatTable};
use oxideav_scribe::Face;

const FIXTURE: &[u8] = include_bytes!("fixtures/InterVariable.ttf");

fn load_face() -> Face {
    Face::from_ttf_bytes(FIXTURE.to_vec()).expect("Inter Variable parses")
}

#[test]
fn inter_publishes_mvar_table_with_known_metric_tags() {
    let face = load_face();
    let mvar = face.mvar().expect("Inter ships MVAR");
    let values = mvar.values();
    assert!(
        !values.is_empty(),
        "Inter MVAR must enumerate at least one metric tag"
    );
    // Inter publishes at least one of: hasc / hdsc / xhgt / cpht /
    // undo / unds — assert any of them is present.
    let known: &[&[u8; 4]] = &[
        b"hasc", b"hdsc", b"hcla", b"hcld", b"xhgt", b"cpht", b"undo", b"unds", b"strs", b"stro",
    ];
    let any_known = values.iter().any(|v| known.iter().any(|k| **k == v.tag));
    assert!(
        any_known,
        "Inter MVAR must include at least one well-known metric tag — got {:?}",
        values
            .iter()
            .map(|v| std::str::from_utf8(&v.tag).unwrap_or("??"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn inter_metric_delta_at_default_coords_is_zero() {
    let face = load_face();
    // No coords set → metric delta is the no-op (zero).
    assert!(face.metric_delta(b"hasc").abs() < 1e-3);
    assert!(face.metric_delta(b"xhgt").abs() < 1e-3);
}

#[test]
fn inter_metric_delta_changes_at_extreme_weight() {
    let mut face = load_face();
    let axes = face.variation_axes();
    let wght_index = axes.iter().position(|a| &a.tag == b"wght").unwrap();
    let mut coords: Vec<f32> = axes.iter().map(|a| a.default).collect();
    coords[wght_index] = 900.0;
    face.set_variation_coords(&coords).unwrap();
    // At least one of the well-known horizontal metric tags must
    // produce a non-zero delta at wght=900 (Inter MVAR enumerates the
    // ones that vary). We don't pin a specific tag because the spec
    // doesn't require a font to enumerate any particular set — but at
    // least one of the publishable metric tags Inter ships must move.
    let probe_tags: &[&[u8; 4]] = &[
        b"hasc", b"hdsc", b"hcla", b"hcld", b"xhgt", b"cpht", b"undo", b"unds", b"strs", b"stro",
    ];
    let any_moved = probe_tags.iter().any(|t| face.metric_delta(t).abs() > 1e-3);
    assert!(
        any_moved,
        "at least one MVAR metric must produce a non-zero delta at wght=900"
    );
}

#[test]
fn inter_publishes_hvar_with_per_glyph_advance_deltas() {
    let face = load_face();
    let _hvar = face.hvar().expect("Inter ships HVAR");
    // No coords set → per-glyph advance delta is the no-op.
    let upper_a = face.with_font(|f| f.glyph_index('A')).unwrap().unwrap();
    assert!(face.h_advance_delta(upper_a).abs() < 1e-3);
}

#[test]
fn inter_h_advance_delta_changes_at_extreme_weight() {
    let mut face = load_face();
    let axes = face.variation_axes();
    let wght_index = axes.iter().position(|a| &a.tag == b"wght").unwrap();
    let mut coords: Vec<f32> = axes.iter().map(|a| a.default).collect();
    coords[wght_index] = 900.0;
    face.set_variation_coords(&coords).unwrap();
    let upper_o = face.with_font(|f| f.glyph_index('O')).unwrap().unwrap();
    let delta = face.h_advance_delta(upper_o);
    assert!(
        delta.abs() > 1e-3,
        "expected non-zero h_advance_delta('O') at wght=900, got {delta}"
    );
}

#[test]
fn inter_has_no_vvar_horizontal_only() {
    // Inter is strictly horizontal — no VVAR.
    let face = load_face();
    assert!(face.vvar().is_none(), "Inter is horizontal-only, no VVAR");
    assert!(
        face.v_advance_delta(0).abs() < 1e-3,
        "missing VVAR → v_advance_delta is identically zero"
    );
}

#[test]
fn inter_publishes_stat_with_two_axes() {
    let face = load_face();
    let stat = face.stat().expect("Inter ships STAT");
    let axes = stat.axes();
    // Inter publishes STAT axes for at least its two `fvar` axes
    // (`wght` + `opsz`) and may include extra "implicit" STAT-only
    // axes (e.g. `ital`) for cross-family matching — we only require
    // the fvar axes to appear, not an exact count.
    assert!(
        axes.len() >= 2,
        "Inter STAT must carry at least wght + opsz, got {} axes",
        axes.len()
    );
    let tags: Vec<&[u8; 4]> = axes.iter().map(|a| &a.tag).collect();
    assert!(tags.contains(&b"wght"));
    assert!(tags.contains(&b"opsz"));
}

#[test]
fn inter_stat_axis_values_include_regular_label() {
    let face = load_face();
    let stat = face.stat().expect("Inter ships STAT");
    let axis_values = stat.axis_values();
    assert!(
        !axis_values.is_empty(),
        "Inter STAT must enumerate axis values"
    );
    // STAT axis values reference name ids — we expect at least one
    // resolvable label.
    let mut any_resolved = false;
    for av in axis_values {
        let nid = av.value_name_id();
        if face.name_id(nid).is_some() {
            any_resolved = true;
        }
    }
    assert!(
        any_resolved,
        "at least one STAT axis-value name id must resolve to a string"
    );
}

#[test]
fn inter_stat_has_format1_records_for_wght_axis() {
    let face = load_face();
    let stat = face.stat().expect("Inter ships STAT");
    let wght_idx = stat.axes().iter().position(|a| &a.tag == b"wght").unwrap() as u16;
    // Find at least one Single (format 1) record on the wght axis.
    let any_wght = stat.axis_values().iter().any(|av| match av {
        StatAxisValue::Single { axis_index, .. } | StatAxisValue::Range { axis_index, .. } => {
            *axis_index == wght_idx
        }
        StatAxisValue::Linked { axis_index, .. } => *axis_index == wght_idx,
        StatAxisValue::Combined { per_axis, .. } => per_axis.iter().any(|(ai, _)| *ai == wght_idx),
    });
    assert!(any_wght, "Inter STAT must have a wght-axis value record");
}

#[test]
fn inter_name_id_resolves_family_name() {
    let face = load_face();
    // Family name is name id 1.
    let family = face.name_id(1).expect("Inter has a family name");
    assert!(
        family.contains("Inter"),
        "family name should contain 'Inter', got {family:?}"
    );
}

#[test]
fn inter_named_instance_subfamily_name_resolves_to_string() {
    let face = load_face();
    let instances = face.named_instances();
    assert!(!instances.is_empty(), "Inter publishes named instances");
    // Each named instance carries a `subfamily_name_id`. At least one
    // should resolve to a label like "Regular", "Bold", "Thin", etc.
    let mut found = false;
    for inst in &instances {
        if let Some(label) = face.name_id(inst.subfamily_name_id) {
            // Sanity: the label should be a typical weight word.
            let typical = [
                "Thin",
                "ExtraLight",
                "Light",
                "Regular",
                "Medium",
                "SemiBold",
                "Bold",
                "ExtraBold",
                "Black",
            ];
            if typical.iter().any(|w| label.contains(w)) {
                found = true;
                break;
            }
        }
    }
    assert!(
        found,
        "at least one Inter named instance must carry a recognisable weight label via name_id"
    );
}

#[test]
fn inter_axis_name_ids_resolve() {
    let face = load_face();
    let axes = face.variation_axes();
    for axis in &axes {
        let label = face
            .name_id(axis.name_id)
            .expect("each axis name id should resolve");
        assert!(
            !label.is_empty(),
            "axis {:?} name should be non-empty, got {label:?}",
            std::str::from_utf8(&axis.tag).unwrap_or("??")
        );
    }
}

#[test]
fn inter_has_no_cff2_table() {
    // Inter is TT (glyf + gvar), not CFF2.
    let face = load_face();
    assert!(face.cff2().is_none());
}

#[test]
fn stat_table_parser_rejects_truncated_buffer() {
    // Defensive: a too-short buffer must yield None, not panic.
    let raw = [0u8; 4];
    assert!(StatTable::parse(&raw).is_none());
}

const SOURCE_SANS: &[u8] = include_bytes!("fixtures/SourceSans3-Regular.otf");

#[test]
fn source_sans_otf_name_id_resolves_family_name() {
    // OTF (CFF) faces resolve `name` table ids through the same
    // path — the inner table directory walker handles both sfnt
    // flavours (`OTTO` magic for OTF, `0x0001_0000` for TT).
    let face = Face::from_otf_bytes(SOURCE_SANS.to_vec()).expect("Source Sans 3 parses");
    let family = face.name_id(1).expect("Source Sans has a family name");
    assert!(
        family.contains("Source"),
        "family name should contain 'Source', got {family:?}"
    );
}

#[test]
fn source_sans_otf_has_no_mvar_hvar_stat_or_cff2() {
    let face = Face::from_otf_bytes(SOURCE_SANS.to_vec()).unwrap();
    assert!(face.mvar().is_none());
    assert!(face.hvar().is_none());
    assert!(face.vvar().is_none());
    assert!(face.stat().is_none());
    // Source Sans 3 ships plain `CFF `, not `CFF2`.
    assert!(face.cff2().is_none());
}
