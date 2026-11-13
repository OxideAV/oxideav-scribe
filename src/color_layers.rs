//! Layered colour glyphs — `COLR` version 0 layer stacks coloured from
//! `CPAL`.
//!
//! A version-0 colour glyph is a base glyph id mapped to a run of
//! layer records, each naming an ordinary outline glyph and a palette
//! entry. The layers are painted in order, the first at the bottom of
//! the z-order; palette entry `0xFFFF` stands for the text foreground
//! colour, which the application supplies. Palette `0` is the font's
//! default palette. (Structures: the staged `COLR` reference,
//! `docs/text/opentype/otspec-colr-v1-paint-graph.md` §2–§3, and the
//! `CPAL` chapter, `docs/text/opentype/otspec-cpal.html`.)
//!
//! [`Face::color_glyph_layers`] resolves a glyph to its coloured layer
//! list; [`Face::color_glyph_node`] builds the render-ready vector
//! node — a [`Group`] with one filled path per layer — and
//! [`Face::glyph_node`] returns that node for colour glyphs (default
//! palette, black foreground).
//!
//! Version-1 paint graphs (gradients, transforms, composites) are not
//! rendered here: a glyph that has both a paint graph and a version-0
//! layer stack is drawn from the layer stack; one that has only a
//! paint graph is drawn as its plain outline.

use crate::color::Rgba;
use crate::face::Face;
use oxideav_core::{FillRule, Group, Node, Paint, PathNode};

/// `paletteIndex` value meaning "the text foreground colour".
const FOREGROUND_ENTRY: u16 = 0xFFFF;

/// How one layer of a colour glyph is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayerColor {
    /// A palette colour, straight-alpha RGBA.
    Palette(Rgba),
    /// The application's text foreground colour (palette entry
    /// `0xFFFF`).
    Foreground,
}

/// One layer of a version-0 colour glyph: an outline glyph and the
/// colour it is filled with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ColorGlyphLayer {
    /// The outline glyph painted for this layer (fetch it with
    /// [`Face::glyph_path`]).
    pub glyph_id: u16,
    /// The layer's fill.
    pub color: LayerColor,
}

impl Face {
    /// `true` when this face ships a `COLR` + `CPAL` pair — layered
    /// colour glyphs. `false` for outline-only, bitmap-colour-only and
    /// CFF faces.
    pub fn has_color_layers(&self) -> bool {
        self.ttf().map(|f| f.has_color_layers()).unwrap_or(false)
    }

    /// Number of palettes in the face's `CPAL` table (`0` without
    /// one). Palette `0` is the default.
    pub fn color_palette_count(&self) -> u16 {
        self.ttf().map(|f| f.cpal_num_palettes()).unwrap_or(0)
    }

    /// The colour layers of `glyph_id` coloured from palette
    /// `palette`, bottom of the z-order first. Empty when the face has
    /// no layered colour glyphs or `glyph_id` is not a colour base
    /// glyph (an ordinary outline glyph, or a glyph only used as a
    /// layer of others).
    ///
    /// A layer whose palette entry does not exist in `palette` cannot
    /// be painted and is left out.
    pub fn color_glyph_layers(&self, glyph_id: u16, palette: u16) -> Vec<ColorGlyphLayer> {
        let Ok(font) = self.ttf() else {
            return Vec::new();
        };
        if !font.has_color_layers() {
            return Vec::new();
        }
        font.color_layers(glyph_id)
            .into_iter()
            .filter_map(|layer| {
                let color = if layer.palette_index == FOREGROUND_ENTRY {
                    LayerColor::Foreground
                } else {
                    LayerColor::Palette(font.cpal_color(palette, layer.palette_index)?)
                };
                Some(ColorGlyphLayer {
                    glyph_id: layer.layer_glyph_id,
                    color,
                })
            })
            .collect()
    }

    /// The layered colour glyph `glyph_id` as a render-ready node: a
    /// [`Group`] holding one filled [`Node::Path`] per layer, bottom
    /// layer first, in the same Y-down pixel space at `size_px` as
    /// [`Face::glyph_node`]. Layers are coloured from palette
    /// `palette`; foreground layers are filled with `foreground`
    /// (straight-alpha RGBA).
    ///
    /// Returns `None` when `glyph_id` is not a colour base glyph, when
    /// none of its layers has an outline, or for a non-positive /
    /// non-finite `size_px` — the caller then draws the glyph as an
    /// ordinary outline. Layer outlines go through the attached
    /// [`crate::OutlineCache`], if any.
    pub fn color_glyph_node(
        &self,
        glyph_id: u16,
        size_px: f32,
        palette: u16,
        foreground: Rgba,
    ) -> Option<Node> {
        if size_px <= 0.0 || !size_px.is_finite() {
            return None;
        }
        let layers = self.color_glyph_layers(glyph_id, palette);
        let children: Vec<Node> = layers
            .iter()
            .filter_map(|layer| {
                let [r, g, b, a] = match layer.color {
                    LayerColor::Palette(rgba) => rgba,
                    LayerColor::Foreground => foreground,
                };
                Some(Node::Path(PathNode {
                    path: self.scaled_outline(layer.glyph_id, size_px)?,
                    fill: Some(Paint::Solid(oxideav_core::Rgba::new(r, g, b, a))),
                    stroke: None,
                    fill_rule: FillRule::NonZero,
                }))
            })
            .collect();
        if children.is_empty() {
            return None;
        }
        Some(Node::Group(Group {
            children,
            ..Group::default()
        }))
    }
}
