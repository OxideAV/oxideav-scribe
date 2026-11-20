//! Paragraph layout: line breaking at Unicode line-break
//! opportunities, alignment and justification, tab stops, horizontal
//! or vertical lines.
//!
//! [`layout_text`] is the document-level entry point. For every
//! paragraph (UAX #9 rule P1) it
//!
//! 1. resolves the paragraph's base direction (rules P2 / P3, unless
//!    the caller fixes it);
//! 2. finds the break opportunities of UAX #14 (through the `intl`
//!    crate's line-break segmentation: spaces, hyphens, between
//!    ideographs, never inside a grapheme cluster, mandatory breaks at
//!    line separators) and fills lines greedily to the measure, the
//!    spaces at a line's end hanging outside it; a segment wider than
//!    the measure is cut between grapheme clusters;
//! 3. shapes each line in visual order ([`crate::layout::shape_visual_line_with`]);
//! 4. advances tabs to the next tab stop, then aligns or justifies
//!    the line.
//!
//! Filling is linear in the text length: each segment between two
//! opportunities is shaped once and its width remembered (documents
//! repeat their words), the pair interaction at each joint likewise.
//!
//! [`crate::layout::wrap_lines`] and friends keep their simpler
//! whitespace-only breaking.

use crate::Error;
use crate::face_chain::FaceChain;
use crate::shaper::{PositionedGlyph, ShapeOptions};
use std::collections::HashMap;
use std::ops::Range;

/// How a line sits in the measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Align {
    /// At the paragraph's start edge: left for a left-to-right
    /// paragraph, right for a right-to-left one.
    #[default]
    Start,
    /// At the paragraph's end edge.
    End,
    /// At the left edge whatever the direction.
    Left,
    /// At the right edge whatever the direction.
    Right,
    /// Centred.
    Center,
    /// Stretched to the measure by widening the spaces between words;
    /// the last line of a paragraph and lines ended by a forced break
    /// are set at the start edge.
    Justify,
}

/// Parameters of [`layout_text`].
///
/// ```
/// use oxideav_scribe::{Align, LayoutOptions, ShapeOptions};
/// let opts = LayoutOptions::new(480.0)
///     .with_align(Align::Justify)
///     .with_tab_stops(vec![120.0, 240.0])
///     .with_shape_options(ShapeOptions::new().with_feature(*b"onum", true));
/// assert_eq!(opts.max_extent(), 480.0);
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LayoutOptions {
    pub(crate) max_extent: f32,
    pub(crate) align: Align,
    pub(crate) base_level: Option<u8>,
    pub(crate) tab_stops: Vec<f32>,
    pub(crate) tab_interval: Option<f32>,
    pub(crate) shape: ShapeOptions,
}

impl LayoutOptions {
    /// Lay out to lines at most `max_extent` pixels long (wide, or
    /// tall for vertical text). Zero or a negative value means "no
    /// limit": lines end only at forced breaks.
    pub fn new(max_extent: f32) -> Self {
        Self {
            max_extent,
            align: Align::Start,
            base_level: None,
            tab_stops: Vec::new(),
            tab_interval: None,
            shape: ShapeOptions::new(),
        }
    }

    /// Alignment of every line.
    #[must_use]
    pub fn with_align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// Fix the base direction of every paragraph (`Some(0)`
    /// left-to-right, `Some(1)` right-to-left) instead of resolving it
    /// per paragraph from its first strong character.
    #[must_use]
    pub fn with_base_level(mut self, base_level: Option<u8>) -> Self {
        self.base_level = base_level.map(|l| l & 1);
        self
    }

    /// Explicit tab stops, as distances from the line's start edge.
    /// A tab past the last explicit stop goes to the next multiple of
    /// the tab interval.
    #[must_use]
    pub fn with_tab_stops(mut self, mut stops: Vec<f32>) -> Self {
        stops.retain(|s| s.is_finite() && *s > 0.0);
        stops.sort_by(f32::total_cmp);
        self.tab_stops = stops;
        self
    }

    /// Distance between the implicit tab stops; the default is eight
    /// space widths of the primary face.
    #[must_use]
    pub fn with_tab_interval(mut self, interval: f32) -> Self {
        self.tab_interval = (interval.is_finite() && interval > 0.0).then_some(interval);
        self
    }

    /// Shaping options (features, language system, vertical layout).
    /// With [`ShapeOptions::with_vertical`] lines run top to bottom
    /// and `max_extent` limits their height.
    #[must_use]
    pub fn with_shape_options(mut self, shape: ShapeOptions) -> Self {
        self.shape = shape;
        self
    }

    /// The line length limit.
    pub fn max_extent(&self) -> f32 {
        self.max_extent
    }

    /// The alignment.
    pub fn align(&self) -> Align {
        self.align
    }
}

/// One line produced by [`layout_text`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LaidOutLine {
    /// The line's glyphs in visual order; paint them in sequence, the
    /// pen starting at [`Self::offset`] and advancing by each glyph's
    /// `x_advance` (`y_advance` for vertical text). Clusters are byte
    /// offsets into the text given to [`layout_text`].
    pub glyphs: Vec<PositionedGlyph>,
    /// Byte range of the line in the text, hanging spaces included,
    /// the paragraph separator excluded.
    pub text_range: Range<usize>,
    /// Where the line starts along its direction, from the start of
    /// the measure (alignment).
    pub offset: f32,
    /// Advance of the line's glyphs (after justification), hanging
    /// spaces excluded.
    pub extent: f32,
    /// The paragraph's base level: `0` left-to-right, `1`
    /// right-to-left.
    pub base_level: u8,
    /// The line ends its paragraph or ends at a forced break.
    pub forced_break: bool,
    /// Index of the paragraph the line belongs to.
    pub paragraph: usize,
}

/// Segment widths and joint interactions, remembered for a document.
struct Widths<'a> {
    chain: &'a FaceChain,
    size_px: f32,
    shape: &'a ShapeOptions,
    segments: HashMap<String, (f32, f32)>,
    joints: HashMap<(char, char), f32>,
}

impl Widths<'_> {
    fn advance(&self, g: &PositionedGlyph) -> f32 {
        if self.shape.is_vertical() {
            g.y_advance
        } else {
            g.x_advance
        }
    }

    fn measure(&self, text: &str) -> Result<f32, Error> {
        let glyphs = self
            .chain
            .shape_with_options(text, self.size_px, self.shape)?;
        Ok(glyphs.iter().map(|g| self.advance(g)).sum())
    }

    /// `(width, width without the trailing white space)` of a segment
    /// shaped on its own.
    fn segment(&mut self, text: &str) -> Result<(f32, f32), Error> {
        if let Some(&w) = self.segments.get(text) {
            return Ok(w);
        }
        let full = self.measure(text)?;
        let visible = text.trim_end_matches(char::is_whitespace);
        let w = if visible.len() == text.len() {
            (full, full)
        } else {
            (full, self.measure(visible)?)
        };
        if self.segments.len() < 65_536 {
            self.segments.insert(text.to_string(), w);
        }
        Ok(w)
    }

    /// What putting `b` right after `a` adds to their separate widths.
    fn joint(&mut self, a: char, b: char) -> Result<f32, Error> {
        if a.is_whitespace() && b.is_whitespace() {
            return Ok(0.0);
        }
        if let Some(&d) = self.joints.get(&(a, b)) {
            return Ok(d);
        }
        let pair: String = [a, b].into_iter().collect();
        let mut one = [0u8; 4];
        let d = self.measure(&pair)?
            - self.measure(a.encode_utf8(&mut one))?
            - self.measure(b.encode_utf8(&mut one))?;
        if self.joints.len() < 65_536 {
            self.joints.insert((a, b), d);
        }
        Ok(d)
    }
}

/// Lay `text` out as lines: paragraphs, line breaking, bidi, shaping,
/// tabs, alignment. See the module docs for the procedure and
/// [`LayoutOptions`] for the parameters.
///
/// Lines come back in reading order (top to bottom for horizontal
/// text; for vertical text each "line" is a column). An empty
/// paragraph yields one empty line.
pub fn layout_text(
    chain: &FaceChain,
    text: &str,
    size_px: f32,
    opts: &LayoutOptions,
) -> Result<Vec<LaidOutLine>, Error> {
    let mut lines = Vec::new();
    if text.is_empty() || size_px <= 0.0 || !size_px.is_finite() {
        return Ok(lines);
    }
    let mut widths = Widths {
        chain,
        size_px,
        shape: &opts.shape,
        segments: HashMap::new(),
        joints: HashMap::new(),
    };
    let limit = (opts.max_extent > 0.0 && opts.max_extent.is_finite()).then_some(opts.max_extent);
    let mut index = 0usize;
    let mut after_cr = false;
    for para in crate::bidi::split_paragraphs(text) {
        // CR LF is one line end: the LF does not open a paragraph.
        if para == "\n" && std::mem::take(&mut after_cr) {
            continue;
        }
        after_cr = para.ends_with('\r');
        let offset = para.as_ptr() as usize - text.as_ptr() as usize;
        // Drop the paragraph separator: it is not rendered.
        let body = match para.char_indices().next_back() {
            Some((i, c)) if crate::bidi::bidi_class(c) == crate::bidi::BidiClass::B => &para[..i],
            _ => para,
        };
        let level = opts
            .base_level
            .unwrap_or_else(|| crate::bidi::paragraph_level(body));
        let first = lines.len();
        for (range, forced) in break_lines(&mut widths, body, limit)? {
            lines.push(set_line(
                chain, text, size_px, opts, &widths, offset, range, level, forced, index,
            )?);
        }
        if lines.len() == first {
            lines.push(LaidOutLine {
                glyphs: Vec::new(),
                text_range: offset..offset,
                offset: 0.0,
                extent: 0.0,
                base_level: level,
                forced_break: true,
                paragraph: index,
            });
        }
        if let Some(last) = lines.last_mut() {
            last.forced_break = true;
        }
        if opts.align == Align::Justify
            && let Some(limit) = limit
        {
            for line in &mut lines[first..] {
                justify(line, text, limit, opts.shape.is_vertical());
            }
        }
        if let Some(limit) = limit {
            for line in &mut lines[first..] {
                line.offset = match (opts.align, line.base_level & 1 == 1) {
                    (Align::Left, _)
                    | (Align::Start | Align::Justify, false)
                    | (Align::End, true) => 0.0,
                    (Align::Center, _) => (limit - line.extent) / 2.0,
                    _ => limit - line.extent,
                };
            }
        }
        index += 1;
    }
    Ok(lines)
}

/// Greedy line filling of one paragraph: byte ranges of its lines and
/// whether each ends at a forced break.
fn break_lines(
    widths: &mut Widths<'_>,
    body: &str,
    limit: Option<f32>,
) -> Result<Vec<(Range<usize>, bool)>, Error> {
    let mut lines: Vec<(Range<usize>, bool)> = Vec::new();
    let mut start = 0usize;
    // Width of the line so far, trailing white space included.
    let mut width = 0.0f32;
    let mut at = 0usize;
    for seg in intl::unicode::segment::line_breaks(body) {
        let seg_range = at..at + seg.text.len();
        at = seg_range.end;
        let (full, visible) = widths.segment(seg.text)?;
        let joint = match (
            body[start..seg_range.start].chars().next_back(),
            seg.text.chars().next(),
        ) {
            (Some(a), Some(b)) => widths.joint(a, b)?,
            _ => 0.0,
        };
        if let Some(limit) = limit
            && width + joint + visible > limit
        {
            if seg_range.start > start {
                // The segment goes to the next line.
                lines.push((start..seg_range.start, false));
                start = seg_range.start;
            }
            if visible > limit {
                // Wider than a whole line: cut between clusters.
                let mut piece = start;
                for cut in emergency_cuts(widths, &body[seg_range.clone()], limit)? {
                    lines.push((piece..seg_range.start + cut, false));
                    piece = seg_range.start + cut;
                }
                start = piece;
                width = widths.segment(&body[start..seg_range.end])?.0;
            } else {
                width = full;
            }
        } else {
            width += joint + full;
        }
        if seg.mandatory {
            lines.push((start..seg_range.end, true));
            start = seg_range.end;
            width = 0.0;
        }
    }
    if start < body.len() {
        lines.push((start..body.len(), true));
    }
    Ok(lines)
}

/// Where to cut a segment wider than the measure: byte offsets (in
/// the segment) at grapheme-cluster boundaries such that every piece
/// but the last is as long as fits (and at least one cluster).
fn emergency_cuts(widths: &Widths<'_>, segment: &str, limit: f32) -> Result<Vec<usize>, Error> {
    let glyphs = widths
        .chain
        .shape_with_options(segment, widths.size_px, widths.shape)?;
    // Advance per cluster, in text order.
    let mut per_cluster: Vec<(usize, f32)> = Vec::new();
    for g in &glyphs {
        let c = g.cluster as usize;
        match per_cluster.iter_mut().find(|(start, _)| *start == c) {
            Some((_, w)) => *w += widths.advance(g),
            None => per_cluster.push((c, widths.advance(g))),
        }
    }
    per_cluster.sort_by_key(|&(start, _)| start);
    let mut cuts = Vec::new();
    let mut run = 0.0f32;
    let mut clusters_in_piece = 0usize;
    for &(start, w) in &per_cluster {
        if clusters_in_piece > 0 && run + w > limit {
            cuts.push(start);
            run = 0.0;
            clusters_in_piece = 0;
        }
        run += w;
        clusters_in_piece += 1;
    }
    Ok(cuts)
}

/// Shape one line and place its tabs.
#[allow(clippy::too_many_arguments)] // one call site; a struct would only rename the list
fn set_line(
    chain: &FaceChain,
    text: &str,
    size_px: f32,
    opts: &LayoutOptions,
    widths: &Widths<'_>,
    para_offset: usize,
    range: Range<usize>,
    level: u8,
    forced: bool,
    paragraph: usize,
) -> Result<LaidOutLine, Error> {
    let text_range = para_offset + range.start..para_offset + range.end;
    let line_text = &text[text_range.clone()];
    // White space at the end of a line hangs outside it; a forced
    // break character is not drawn either.
    let visible = line_text.trim_end_matches(|c: char| c.is_whitespace() && c != '\t');
    let shaped =
        crate::layout::shape_visual_line_with(chain, visible, size_px, Some(level), &opts.shape)?;
    let mut glyphs = shaped.glyphs;
    let shift = text_range.start as u32;
    for g in &mut glyphs {
        g.cluster += shift;
    }
    let vertical = opts.shape.is_vertical();
    if !vertical && visible.contains('\t') {
        place_tabs(chain, text, size_px, opts, &mut glyphs, level & 1 == 1)?;
    }
    let extent = glyphs.iter().map(|g| widths.advance(g)).sum();
    Ok(LaidOutLine {
        glyphs,
        text_range,
        offset: 0.0,
        extent,
        base_level: level,
        forced_break: forced,
        paragraph,
    })
}

/// Give every tab glyph the advance that takes the pen to the next
/// tab stop, measured from the line's start edge (the left for a
/// left-to-right paragraph, the right for a right-to-left one). The
/// tab is drawn with the space glyph.
fn place_tabs(
    chain: &FaceChain,
    text: &str,
    size_px: f32,
    opts: &LayoutOptions,
    glyphs: &mut [PositionedGlyph],
    rtl: bool,
) -> Result<(), Error> {
    let space = chain.shape(" ", size_px)?;
    let (space_gid, space_face, space_advance) = space.first().map_or((0, 0, size_px / 4.0), |g| {
        (g.glyph_id, g.face_idx, g.x_advance)
    });
    let interval = opts
        .tab_interval
        .unwrap_or(space_advance * 8.0)
        .max(f32::EPSILON);
    let next_stop = |pen: f32| -> f32 {
        // Strictly beyond the pen (with a little slack for rounding).
        let past = pen + 0.01;
        opts.tab_stops
            .iter()
            .copied()
            .find(|&s| s > past)
            .unwrap_or_else(|| ((past / interval).floor() + 1.0) * interval)
    };
    let is_tab = |g: &PositionedGlyph| text.as_bytes().get(g.cluster as usize) == Some(&b'\t');
    let mut pen = 0.0f32;
    let order: Box<dyn Iterator<Item = usize>> = if rtl {
        Box::new((0..glyphs.len()).rev())
    } else {
        Box::new(0..glyphs.len())
    };
    for i in order {
        if is_tab(&glyphs[i]) {
            let g = &mut glyphs[i];
            g.glyph_id = space_gid;
            g.face_idx = space_face;
            g.x_offset = 0.0;
            g.y_offset = 0.0;
            g.x_advance = next_stop(pen) - pen;
        }
        pen += glyphs[i].x_advance;
    }
    Ok(())
}

/// Stretch a line to `limit` by widening its word spaces.
fn justify(line: &mut LaidOutLine, text: &str, limit: f32, vertical: bool) {
    if line.forced_break || line.extent >= limit {
        return;
    }
    let is_space = |g: &PositionedGlyph| {
        matches!(
            text[g.cluster as usize..].chars().next(),
            Some(' ' | '\u{a0}')
        )
    };
    let spaces = line.glyphs.iter().filter(|g| is_space(g)).count();
    if spaces == 0 {
        return;
    }
    let each = (limit - line.extent) / spaces as f32;
    for g in line.glyphs.iter_mut().filter(|g| is_space(g)) {
        if vertical {
            g.y_advance += each;
        } else {
            g.x_advance += each;
        }
    }
    line.extent = limit;
}
