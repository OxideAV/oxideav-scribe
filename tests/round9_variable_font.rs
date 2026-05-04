//! Round-9 variable-font axis selection in shaping.
//!
//! Verifies that scribe's round-9 surface for variable fonts works
//! end-to-end against a real OFL-licensed two-axis font (`opsz`,
//! `wght`):
//!
//! - `Face::variation_axes` / `named_instances` / `is_variable`
//!   surface the `fvar` declarations cloned out of `oxideav-ttf`.
//! - `Face::set_variation_coords` clamps out-of-range entries to each
//!   axis's `[min, max]` per the underlying parser contract.
//! - `Shaper::with_variation_coords(...).shape_to_paths(...)` installs
//!   the override on the primary face for the duration of the call,
//!   producing **different glyph outlines** at `wght=400` vs `wght=900`
//!   while restoring the chain's previous coords on exit.
//!
//! Fixture: `tests/fixtures/InterVariable.ttf` — a copy of the same
//! Inter Variable cut vendored by `oxideav-ttf/tests/fixtures/`. Carries
//! 2 axes (`opsz` 14..32, `wght` 100..900) and 9 named instances
//! (Thin / ExtraLight / Light / Regular / Medium / SemiBold / Bold /
//! ExtraBold / Black). Copyright belongs to The Inter Project Authors;
//! redistribution is governed by `tests/fixtures/INTER-OFL-LICENSE.txt`.

use oxideav_core::{Node, PathCommand};
use oxideav_scribe::{Face, FaceChain, Shaper};

const FIXTURE: &[u8] = include_bytes!("fixtures/InterVariable.ttf");

fn load_chain() -> FaceChain {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).expect("Inter Variable parses");
    FaceChain::new(face)
}

/// Pull every `(x, y)` coordinate point that contributes to the rendered
/// silhouette of a glyph node out of a `Node::Group { children: [PathNode], .. }`
/// envelope (the wrapper added by `Shaper::shape_to_paths`). Used by the
/// "outlines differ" assertion below to compare two variation-coord
/// shape calls without depending on the exact path-command ordering.
fn collect_xy(node: &Node) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    fn walk(n: &Node, out: &mut Vec<(f32, f32)>) {
        match n {
            Node::Group(g) => {
                for c in &g.children {
                    walk(c, out);
                }
            }
            Node::Path(p) => {
                for cmd in &p.path.commands {
                    match *cmd {
                        PathCommand::MoveTo(p) | PathCommand::LineTo(p) => out.push((p.x, p.y)),
                        PathCommand::QuadCurveTo { control, end } => {
                            out.push((control.x, control.y));
                            out.push((end.x, end.y));
                        }
                        PathCommand::CubicCurveTo { c1, c2, end } => {
                            out.push((c1.x, c1.y));
                            out.push((c2.x, c2.y));
                            out.push((end.x, end.y));
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    walk(node, &mut out);
    out
}

#[test]
fn inter_face_publishes_two_axes_and_nine_instances() {
    let face = Face::from_ttf_bytes(FIXTURE.to_vec()).unwrap();
    assert!(face.is_variable(), "Inter Variable must report is_variable");
    let axes = face.variation_axes();
    assert_eq!(axes.len(), 2, "Inter publishes opsz + wght");
    let tags: Vec<&[u8; 4]> = axes.iter().map(|a| &a.tag).collect();
    assert!(tags.contains(&b"wght"), "wght axis present");
    assert!(tags.contains(&b"opsz"), "opsz axis present");
    let instances = face.named_instances();
    assert_eq!(
        instances.len(),
        9,
        "Inter ships 9 named instances (Thin..Black)"
    );
}

#[test]
fn with_variation_coords_400_vs_900_produces_different_outlines() {
    let mut chain = load_chain();
    let axes = chain.face(0).variation_axes();
    let wght_index = axes
        .iter()
        .position(|a| &a.tag == b"wght")
        .expect("wght axis present");
    let opsz_default = axes
        .iter()
        .find(|a| &a.tag == b"opsz")
        .map(|a| a.default)
        .unwrap();

    // Build the wght=400 (default) coords and the wght=900 (heaviest)
    // coords so the only thing changing between the two shape calls is
    // the wght axis.
    let mut coords_regular = vec![opsz_default; axes.len()];
    coords_regular[wght_index] = 400.0;
    let mut coords_heavy = coords_regular.clone();
    coords_heavy[wght_index] = 900.0;

    // Shape "O" at each weight. "O" is the canonical curve-bearing
    // glyph and Inter's gvar deltas demonstrably push its stems wider
    // at higher weights — the per-point coordinates must move between
    // the two calls.
    let regular_paths =
        Shaper::with_variation_coords(coords_regular.clone()).shape_to_paths(&mut chain, "O", 64.0);
    let heavy_paths =
        Shaper::with_variation_coords(coords_heavy).shape_to_paths(&mut chain, "O", 64.0);

    assert_eq!(regular_paths.len(), 1, "one glyph node for 'O' at regular");
    assert_eq!(heavy_paths.len(), 1, "one glyph node for 'O' at heavy");

    let regular_xy = collect_xy(&regular_paths[0].1);
    let heavy_xy = collect_xy(&heavy_paths[0].1);
    assert!(
        !regular_xy.is_empty(),
        "regular outline carries at least one coordinate"
    );
    assert_eq!(
        regular_xy.len(),
        heavy_xy.len(),
        "weight axis must not change topology — same point count"
    );
    let any_diff = regular_xy
        .iter()
        .zip(heavy_xy.iter())
        .any(|(r, h)| (r.0 - h.0).abs() > 1e-3 || (r.1 - h.1).abs() > 1e-3);
    assert!(
        any_diff,
        "wght=400 vs wght=900 must produce at least one point-coordinate delta"
    );

    // After the second shape call the chain should be back at the
    // pre-shape coords (empty, since we never set coords on the chain
    // directly — only via the per-call builder).
    assert!(
        chain.face(0).variation_coords().is_empty(),
        "ShaperBuilder must restore the chain's previous coords on exit"
    );
}

#[test]
fn named_instance_returns_regular_axis_coords() {
    // The "Regular" named instance MUST carry a coord vector matching
    // each axis's `default` value — this is the exact definition of
    // the "Regular" instance for Inter (and for any well-formed
    // variable font that ships one). Use `Shaper::named_instances`
    // for the lookup so we exercise the pass-through accessor.
    let chain = load_chain();
    let instances = Shaper::named_instances(&chain, 0);
    assert!(
        !instances.is_empty(),
        "Inter face publishes named instances"
    );
    let axes = chain.face(0).variation_axes();
    let defaults: Vec<f32> = axes.iter().map(|a| a.default).collect();
    let regular = instances
        .iter()
        .find(|i| {
            i.coords.len() == defaults.len()
                && i.coords
                    .iter()
                    .zip(defaults.iter())
                    .all(|(a, b)| (a - b).abs() < 1e-3)
        })
        .expect("a named instance whose coords match every axis default ('Regular')");
    // Sanity: the instance's coord vector and the axis default vector
    // really do agree elementwise.
    assert_eq!(regular.coords.len(), defaults.len());
    for (i, (c, d)) in regular.coords.iter().zip(defaults.iter()).enumerate() {
        assert!(
            (c - d).abs() < 1e-3,
            "Regular instance axis[{i}] coord {c} != axis default {d}"
        );
    }
}

#[test]
fn out_of_range_coords_clamp_to_axis_min_max() {
    let mut face = Face::from_ttf_bytes(FIXTURE.to_vec()).unwrap();
    let axes = face.variation_axes();
    let wght_index = axes
        .iter()
        .position(|a| &a.tag == b"wght")
        .expect("wght axis");
    let wght_min = axes[wght_index].min;
    let wght_max = axes[wght_index].max;

    // Below-min: must clamp UP to wght.min (100 on Inter).
    let mut below = vec![0.0_f32; axes.len()];
    for (i, a) in axes.iter().enumerate() {
        below[i] = a.default;
    }
    below[wght_index] = -1000.0;
    face.set_variation_coords(&below).unwrap();
    assert_eq!(
        face.variation_coords()[wght_index],
        wght_min,
        "below-min wght must clamp to axis.min"
    );

    // Above-max: must clamp DOWN to wght.max (900 on Inter).
    let mut above = vec![0.0_f32; axes.len()];
    for (i, a) in axes.iter().enumerate() {
        above[i] = a.default;
    }
    above[wght_index] = 5000.0;
    face.set_variation_coords(&above).unwrap();
    assert_eq!(
        face.variation_coords()[wght_index],
        wght_max,
        "above-max wght must clamp to axis.max"
    );

    // In-range: passes through unchanged.
    let mut mid = vec![0.0_f32; axes.len()];
    for (i, a) in axes.iter().enumerate() {
        mid[i] = a.default;
    }
    mid[wght_index] = 600.0;
    face.set_variation_coords(&mid).unwrap();
    assert_eq!(
        face.variation_coords()[wght_index],
        600.0,
        "in-range wght must not be modified"
    );
}

#[test]
fn shape_at_default_coords_matches_static_shape() {
    // Sanity: a shape call with explicit default coords must produce
    // the same glyph node as the un-overridden static path. This is
    // the variable-font equivalent of "the no-op transform leaves the
    // outline alone" and it pins down the round-trip through the
    // builder + restore path.
    let mut chain = load_chain();
    let axes = chain.face(0).variation_axes();
    let defaults: Vec<f32> = axes.iter().map(|a| a.default).collect();

    let static_paths = Shaper::shape_to_paths(&chain, "O", 64.0);
    let default_paths =
        Shaper::with_variation_coords(defaults).shape_to_paths(&mut chain, "O", 64.0);
    assert_eq!(static_paths.len(), default_paths.len());
    let static_xy = collect_xy(&static_paths[0].1);
    let default_xy = collect_xy(&default_paths[0].1);
    assert_eq!(
        static_xy, default_xy,
        "explicit default coords must reproduce the static-path outline exactly"
    );
}
