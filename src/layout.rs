//! Single-line measurement + word-wrap helpers for round-1.
//!
//! No bidi, no mixed-script reordering — just enough machinery to slice
//! a UTF-8 string into "lines that fit `max_width`" by breaking at
//! whitespace boundaries (or, if a single word overflows, mid-word).
//!
//! The shaper is invoked once per candidate line so kerning and
//! ligatures are correctly accounted for in the width budget.

use crate::bidi::{
    apply_mirroring, bidi_class, level_runs, process_paragraph_classes_with_brackets,
    reorder_combining_marks, reorder_line, reset_trailing_levels, BidiClass,
};
use crate::face::Face;
use crate::face_chain::FaceChain;
use crate::shaper::{PositionedGlyph, Shaper};
use crate::Error;

/// Width of a shaped run in raster pixels: cumulative advance + the
/// trailing glyph's offset (which is normally 0; included for correctness
/// when round-2 mark-to-base attachment lands).
pub fn run_width(glyphs: &[PositionedGlyph]) -> f32 {
    let mut w = 0.0;
    for g in glyphs {
        w += g.x_advance + g.x_offset;
    }
    w
}

/// The visual-order result of driving the full UAX #9 §3 + §3.4
/// bidirectional pipeline over one display line.
///
/// A renderer that wants correct bidirectional text walks
/// [`VisualLine::visual`] left-to-right (the natural rendering
/// direction of the output device), feeding each character to the
/// shaper / cmap and laying the glyphs out in increasing x. The
/// per-character permutation [`VisualLine::logical_to_visual`] and its
/// inverse [`VisualLine::visual_to_logical`] let the caller map a
/// visual glyph back to its source character (cursor hit-testing,
/// selection-rectangle building) and vice versa.
///
/// `visual` already has rule **L4** mirroring applied — every
/// character whose resolved level is odd (right-to-left) and that has
/// a `Bidi_Mirroring_Glyph` pair (a bracket, an angle quotation mark,
/// a mathematical relation, …) is the mirrored code point, not the
/// logical one — so the renderer must *not* mirror again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualLine {
    /// The line's characters in left-to-right visual order, with L4
    /// mirroring applied. `visual.len()` equals the line's character
    /// count.
    pub visual: Vec<char>,
    /// Permutation entry `k` is the logical index of the character
    /// that belongs at visual position `k` (the UAX #9 §3.4 L2
    /// output). `visual[k]` is the L4-mirrored form of the logical
    /// character at index `logical_to_visual[k]`.
    pub logical_to_visual: Vec<usize>,
    /// The inverse permutation: entry `i` is the visual position of
    /// the character whose logical index is `i`. Equivalent to
    /// inverting [`Self::logical_to_visual`]; precomputed for
    /// O(1) logical-to-visual hit-testing.
    pub visual_to_logical: Vec<usize>,
    /// The paragraph embedding level resolved by P2 / P3 (or supplied
    /// by the caller as the HL1 override). `0` for an LTR line, `1`
    /// for an RTL line.
    pub base_level: u8,
}

impl VisualLine {
    /// Collect [`Self::visual`] into a `String` — the line in the
    /// order a left-to-right renderer paints it.
    #[must_use]
    pub fn to_visual_string(&self) -> String {
        self.visual.iter().collect()
    }

    /// Number of characters in the line.
    #[must_use]
    pub fn len(&self) -> usize {
        self.visual.len()
    }

    /// Whether the line is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.visual.is_empty()
    }
}

/// Drive the complete UAX #9 bidirectional pipeline over a single
/// **display line** and return its characters in left-to-right visual
/// order.
///
/// This is the high-level bridge the [`crate::bidi`] module's per-rule entry
/// points compose into: a caller that has already decided where the
/// paragraph breaks into lines (e.g. via [`wrap_lines`]) passes one
/// line here and receives a [`VisualLine`] whose `visual` field is
/// ready to feed glyph-by-glyph into the shaper in rendering order.
///
/// The pipeline run is, per line:
///
/// 1. **§3.2** class assignment + **§3.3 P → X → W → N0 → N1 / N2 →
///    I** via [`process_paragraph_classes_with_brackets`] (the
///    bracket-aware variant, so paired brackets resolve per N0).
/// 2. **§3.4 L1** trailing-whitespace / separator level reset via
///    [`reset_trailing_levels`] over the whole line.
/// 3. **§3.4 L2** the logical-to-visual permutation via
///    [`reorder_line`].
/// 4. **§3.4 L3** combining-mark reordering via
///    [`reorder_combining_marks`], so a base + its marks stay in
///    `base, mark, …` order after the RTL reversal (the contract a
///    renderer that paints marks after the base needs).
/// 5. **§3.4 L4** mirroring via [`apply_mirroring`], applied to the
///    logical characters at their resolved levels, then projected
///    through the L2 / L3 permutation into `visual`.
///
/// `base_level` is the HL1 higher-level-protocol override: `Some(0)`
/// forces an LTR line, `Some(1)` forces RTL, and `None` lets P2 / P3
/// resolve the base from the line's first strong character.
///
/// A line should be a single paragraph's worth of text (no `B`
/// paragraph separator in the middle); callers split on paragraph
/// separators with [`crate::bidi::split_paragraphs`] before line-breaking.
/// An embedded trailing `B` is handled by L1 like any other line.
///
/// Provenance: composed from the §3 / §3.4 per-rule entry points in
/// the [`crate::bidi`] module, each of which cites
/// `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`.
///
/// # Examples
///
/// ```
/// use oxideav_scribe::layout::reorder_line_visual;
///
/// // Pure LTR: visual order equals logical order.
/// let line = reorder_line_visual("abc", None);
/// assert_eq!(line.base_level, 0);
/// assert_eq!(line.to_visual_string(), "abc");
/// assert_eq!(line.logical_to_visual, vec![0, 1, 2]);
/// ```
#[must_use]
pub fn reorder_line_visual(text: &str, base_level: Option<u8>) -> VisualLine {
    let chars: Vec<char> = text.chars().collect();
    let classes: Vec<BidiClass> = chars.iter().copied().map(bidi_class).collect();

    let carrier = process_paragraph_classes_with_brackets(&classes, &chars, base_level);

    // §3.4 L1: reset segment / paragraph separators + trailing
    // whitespace runs to the paragraph level, using the *original*
    // classes per the §3.4 normative note. Work on a clone so the
    // resolved levels used for L4 mirroring stay intact.
    let mut line_levels = carrier.levels.clone();
    reset_trailing_levels(&carrier.classes, &mut line_levels, carrier.paragraph_level);

    // §3.4 L2: the logical-to-visual permutation.
    let mut logical_to_visual = reorder_line(&line_levels);

    // §3.4 L3: keep each base + its combining marks in base-first
    // order after the RTL reversal.
    reorder_combining_marks(&carrier.classes, &line_levels, &mut logical_to_visual);

    // §3.4 L4: mirror odd-resolved-level characters in logical order,
    // then project through the permutation. Mirroring keys off the
    // resolved (post-I) levels, not the L1-reset levels, so a trailing
    // mirrored bracket inside reset whitespace still mirrors correctly.
    let mut mirrored = chars.clone();
    apply_mirroring(&mut mirrored, &carrier.levels);

    let n = chars.len();
    let mut visual = Vec::with_capacity(n);
    let mut visual_to_logical = vec![0usize; n];
    for (vis_pos, &log_idx) in logical_to_visual.iter().enumerate() {
        visual.push(mirrored[log_idx]);
        visual_to_logical[log_idx] = vis_pos;
    }

    VisualLine {
        visual,
        logical_to_visual,
        visual_to_logical,
        base_level: carrier.paragraph_level,
    }
}

/// A bidirectional **display line** shaped into positioned glyphs laid
/// out left-to-right in visual order, the join between the UAX #9
/// reordering pipeline and the OpenType shaper.
///
/// Produced by [`shape_visual_line`]. A renderer paints
/// [`Self::glyphs`] left-to-right starting at pen `x = 0`, advancing by
/// each glyph's `x_advance` and applying its `(x_offset, y_offset)` —
/// exactly the contract of a [`crate::Shaper::shape`] result, but for a
/// mixed-direction line.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedVisualLine {
    /// The shaped glyphs in left-to-right visual order. Glyphs of an
    /// RTL (odd-level) run appear in reversed logical order so the run
    /// reads right-to-left while the pen still advances left-to-right.
    pub glyphs: Vec<PositionedGlyph>,
    /// The paragraph / line embedding level resolved by P2 / P3 (or the
    /// HL1 caller override). `0` for an LTR line, `1` for RTL.
    pub base_level: u8,
}

impl ShapedVisualLine {
    /// Total advance width of the line in raster pixels.
    #[must_use]
    pub fn width(&self) -> f32 {
        run_width(&self.glyphs)
    }

    /// Number of shaped glyphs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    /// Whether the line produced no glyphs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }
}

/// Shape a single bidirectional **display line** into positioned glyphs
/// in left-to-right visual order, bridging the complete UAX #9 §3 / §3.4
/// reordering pipeline and the OpenType shaper.
///
/// Where [`reorder_line_visual`] reorders *characters* and hands the
/// caller a [`VisualLine`] to feed glyph-by-glyph, this entry point does
/// the shaping too — and does it the spec-correct way: shaping runs in
/// **logical** order (so ligatures, cursive joining, and contextual
/// rules see the natural character sequence) and arranging the shaped
/// runs in visual order afterwards.
///
/// The procedure, per line:
///
/// 1. Run the §3 paragraph pipeline (`P → X → W → N0 → N1/N2 → I`) plus
///    the §3.4 **L1** trailing-whitespace / separator reset over the
///    whole line, yielding a per-character resolved level vector.
/// 2. Partition the line into **level runs** (BD7) — maximal
///    same-level substrings.
/// 3. Shape each level run's *logical* substring through `chain`
///    ([`FaceChain::shape`], so face fallback + Arabic joining + Indic
///    clustering all apply). A run whose level is **odd** (RTL) has its
///    shaped-glyph sequence reversed so the run reads right-to-left.
/// 4. Emit the runs in **visual order** — the order the §3.4 **L2**
///    reversal puts them in — concatenating their glyph sequences into
///    one left-to-right stream.
///
/// `base_level` is the HL1 override: `Some(0)` forces LTR, `Some(1)`
/// forces RTL, `None` lets P2 / P3 resolve it.
///
/// Note on mirroring: characters are shaped from the **logical** text,
/// so L4 glyph mirroring (a `(` rendering as `)` inside an RTL run) is
/// the font's / shaper's responsibility via the `rtlm` feature or a
/// mirrored cmap entry — this function does not pre-mirror the
/// characters the way [`reorder_line_visual`] does for its char-level
/// output, because a mirrored *character* would shape to the wrong
/// glyph. Callers needing the visual *character* stream still use
/// [`reorder_line_visual`].
///
/// Provenance: composed from the §3 / §3.4 per-rule entry points in
/// [`crate::bidi`], each citing
/// `docs/text/unicode-bidi/tr9-50-uax9-unicode16.html`.
pub fn shape_visual_line(
    chain: &FaceChain,
    text: &str,
    size_px: f32,
    base_level: Option<u8>,
) -> Result<ShapedVisualLine, Error> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() || size_px <= 0.0 {
        return Ok(ShapedVisualLine {
            glyphs: Vec::new(),
            base_level: base_level.map(|l| l & 1).unwrap_or(0),
        });
    }

    let classes: Vec<BidiClass> = chars.iter().copied().map(bidi_class).collect();
    let carrier = process_paragraph_classes_with_brackets(&classes, &chars, base_level);

    // §3.4 L1: reset separators + trailing whitespace to the paragraph
    // level before partitioning, so trailing spaces of an RTL line sit
    // at the paragraph edge.
    let mut line_levels = carrier.levels.clone();
    reset_trailing_levels(&carrier.classes, &mut line_levels, carrier.paragraph_level);

    // §3.4 L2: the logical-to-visual character permutation. We use it to
    // order the level runs visually (a run's visual rank is the visual
    // position of its first character under L2).
    let logical_to_visual = reorder_line(&line_levels);
    let n = chars.len();
    let mut visual_to_logical = vec![0usize; n];
    for (vis_pos, &log_idx) in logical_to_visual.iter().enumerate() {
        visual_to_logical[log_idx] = vis_pos;
    }

    // BD7 level-run partition.
    let runs = level_runs(&line_levels);

    // Shape each run logically; record (visual_rank, glyphs).
    let mut shaped_runs: Vec<(usize, Vec<PositionedGlyph>)> = Vec::with_capacity(runs.len());
    for run in &runs {
        let run_text: String = chars[run.start..run.end].iter().collect();
        let mut glyphs = chain.shape(&run_text, size_px)?;
        // RTL run: the shaped glyphs read left-to-right in logical
        // order, but the run is laid out right-to-left, so reverse the
        // glyph sequence. Advances stay attached to their glyph, so the
        // pen still accumulates correctly when the reversed sequence is
        // emitted left-to-right.
        if run.level & 1 == 1 {
            glyphs.reverse();
        }
        // The run's visual rank: among all runs, where does this one
        // land left-to-right? Use the minimum visual position of its
        // characters (a contiguous run maps to a contiguous visual
        // block under L2).
        let visual_rank = (run.start..run.end)
            .map(|log_idx| visual_to_logical[log_idx])
            .min()
            .unwrap_or(run.start);
        shaped_runs.push((visual_rank, glyphs));
    }

    // Emit runs in visual order (ascending visual rank).
    shaped_runs.sort_by_key(|(rank, _)| *rank);
    let mut out: Vec<PositionedGlyph> = Vec::new();
    for (_, mut glyphs) in shaped_runs {
        out.append(&mut glyphs);
    }

    Ok(ShapedVisualLine {
        glyphs: out,
        base_level: carrier.paragraph_level,
    })
}

/// Break `text` into lines that fit within `max_width` after shaping.
/// Whitespace runs are the preferred break points; a single word that
/// is wider than `max_width` is broken character-by-character so the
/// caller never receives an over-wide line.
///
/// Returns the line strings (not their shaped output) — the caller
/// usually feeds each line back into [`Shaper::shape`] for the final
/// composition step.
pub fn wrap_lines(
    face: &Face,
    text: &str,
    size_px: f32,
    max_width: f32,
) -> Result<Vec<String>, Error> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if max_width <= 0.0 {
        // Caller didn't constrain width — return one line per actual
        // newline (collapsing them is wrong; preserving them is the
        // least-surprise default).
        return Ok(text.split('\n').map(|s| s.to_string()).collect());
    }

    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        wrap_paragraph(face, paragraph, size_px, max_width, &mut lines)?;
    }
    Ok(lines)
}

/// Wrap `text` to `max_width` **and** shape each produced line into
/// bidi-ordered positioned glyphs — the one-call path from a paragraph
/// of logical text to render-ready visual lines.
///
/// This composes [`wrap_lines`] (the width-constrained break finder,
/// which measures candidate lines with the primary face's shaper) with
/// [`shape_visual_line`] (the per-line UAX #9 reorder + shape). Each
/// returned [`ShapedVisualLine`] is one display line whose `glyphs` a
/// renderer paints left-to-right, stacking lines top-to-bottom by the
/// face's line height.
///
/// `base_level` is the HL1 override applied to **every** line: `Some(0)`
/// forces LTR, `Some(1)` forces RTL, `None` lets each line resolve its
/// own base from its first strong character (P2 / P3). A caller that
/// wants a single paragraph direction for ragged-wrapped RTL text passes
/// `Some(1)` so every visual line shares the paragraph's base level
/// rather than flipping per-line on a line that happens to start with a
/// Latin word.
///
/// Line-breaking itself stays direction-agnostic — breaks are chosen on
/// the logical text by [`wrap_lines`]'s whitespace / hard-break policy,
/// then each resulting logical line is reordered. (UAX #14 break-class
/// line breaking is a separate, larger feature; this entry point uses
/// the existing whitespace breaker.)
///
/// Returns one [`ShapedVisualLine`] per wrapped line, in top-to-bottom
/// reading order.
pub fn wrap_and_shape_lines(
    chain: &FaceChain,
    text: &str,
    size_px: f32,
    max_width: f32,
    base_level: Option<u8>,
) -> Result<Vec<ShapedVisualLine>, Error> {
    let line_strings = wrap_lines(chain.primary(), text, size_px, max_width)?;
    let mut out = Vec::with_capacity(line_strings.len());
    for line in &line_strings {
        out.push(shape_visual_line(chain, line, size_px, base_level)?);
    }
    Ok(out)
}

/// One paragraph's worth of wrapped, bidi-shaped visual lines plus the
/// base embedding level resolved for that paragraph.
///
/// Produced by [`shape_paragraphs`]. A renderer paints the `lines`
/// top-to-bottom; the shared [`Self::base_level`] is the paragraph's
/// resolved direction (relevant for aligning ragged lines: an RTL
/// paragraph is flush-right, an LTR paragraph flush-left).
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedParagraph {
    /// The paragraph's display lines, top-to-bottom.
    pub lines: Vec<ShapedVisualLine>,
    /// The base embedding level P2 / P3 resolved for this paragraph (or
    /// the caller's HL1 override). `0` = LTR, `1` = RTL.
    pub base_level: u8,
}

/// Lay out a full multi-paragraph document: split on **paragraph
/// separators** (the UAX #9 bidi-class-`B` characters — LF `U+000A`, CR
/// `U+000D`, CRLF, NEL `U+0085`, and PARAGRAPH SEPARATOR `U+2029`),
/// resolve **each paragraph's own base direction**, then wrap +
/// bidi-shape every paragraph to `max_width`.
///
/// Note that LINE SEPARATOR `U+2028` is bidi-class `WS` (a *line* break
/// within a paragraph, not a paragraph break) and so does **not** start
/// a new paragraph here — it is wrapped like ordinary whitespace.
///
/// This is the document-level counterpart to [`wrap_and_shape_lines`].
/// Where that entry point applies a single `base_level` to every line of
/// the whole text, `shape_paragraphs` honours UAX #9 P1: each paragraph
/// is an independent bidirectional unit, so a Hebrew paragraph followed
/// by an English one each resolve their own direction. Passing
/// `base_level = None` lets every paragraph auto-resolve from its first
/// strong character (P2 / P3); `Some(0)` / `Some(1)` forces a uniform
/// direction across all paragraphs (HL1).
///
/// Unlike [`wrap_lines`] / [`wrap_and_shape_lines`] — which split only on
/// the ASCII `'\n'` — this driver recognises the full B-class separator
/// set via [`crate::bidi::split_paragraphs`], so a `U+2028` /
/// `U+2029` / NEL in the source forces a paragraph break too. The
/// trailing separator is dropped from each paragraph before wrapping (it
/// is not rendered).
///
/// Returns one [`ShapedParagraph`] per source paragraph, in document
/// order. An empty `text` returns an empty vec.
pub fn shape_paragraphs(
    chain: &FaceChain,
    text: &str,
    size_px: f32,
    max_width: f32,
    base_level: Option<u8>,
) -> Result<Vec<ShapedParagraph>, Error> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for para in crate::bidi::split_paragraphs(text) {
        // Drop the trailing paragraph separator (the B character P1 keeps
        // attached) — it is not part of the rendered run. A bare
        // separator paragraph collapses to an empty line.
        let body: &str = match para.char_indices().next_back() {
            Some((i, c)) if crate::bidi::bidi_class(c) == BidiClass::B => &para[..i],
            _ => para,
        };
        // Per-paragraph base level: the caller override wins; otherwise
        // P2 / P3 resolves this paragraph independently.
        let resolved = base_level.unwrap_or_else(|| crate::bidi::paragraph_level(body));
        let lines = wrap_and_shape_lines(chain, body, size_px, max_width, Some(resolved))?;
        // An empty paragraph (separator-only) still contributes one empty
        // line so the renderer advances a blank line.
        let lines = if lines.is_empty() {
            vec![ShapedVisualLine {
                glyphs: Vec::new(),
                base_level: resolved,
            }]
        } else {
            lines
        };
        out.push(ShapedParagraph {
            lines,
            base_level: resolved,
        });
    }
    Ok(out)
}

fn wrap_paragraph(
    face: &Face,
    text: &str,
    size_px: f32,
    max_width: f32,
    lines: &mut Vec<String>,
) -> Result<(), Error> {
    if text.is_empty() {
        lines.push(String::new());
        return Ok(());
    }

    // Tokenise on whitespace, keeping the spaces attached to the
    // following word so the trailing-space behaviour is consistent.
    let words: Vec<String> = split_keeping_whitespace(text);
    if words.is_empty() {
        lines.push(text.to_string());
        return Ok(());
    }

    let mut current = String::new();
    for word in words {
        let candidate = if current.is_empty() {
            word.trim_start().to_string()
        } else {
            format!("{current}{word}")
        };
        let glyphs = Shaper::shape(face, &candidate, size_px)?;
        if run_width(&glyphs) <= max_width || current.is_empty() {
            current = candidate;
            // If even the first word doesn't fit, hard-break it.
            let cur_glyphs = Shaper::shape(face, &current, size_px)?;
            if run_width(&cur_glyphs) > max_width {
                let (head, tail) = hard_break(face, &current, size_px, max_width)?;
                lines.push(head);
                current = tail;
            }
        } else {
            lines.push(current.clone());
            current = word.trim_start().to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    Ok(())
}

/// Split a string into "word + leading whitespace" tokens. Each
/// returned token starts with zero-or-more whitespace characters
/// followed by zero-or-more non-whitespace characters.
fn split_keeping_whitespace(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut in_word = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if in_word {
                out.push(std::mem::take(&mut buf));
                in_word = false;
            }
            buf.push(ch);
        } else {
            in_word = true;
            buf.push(ch);
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

/// Cut `text` so the prefix shapes within `max_width`. Returns
/// `(head, tail)` — `head` is everything that fit, `tail` is the rest.
fn hard_break(
    face: &Face,
    text: &str,
    size_px: f32,
    max_width: f32,
) -> Result<(String, String), Error> {
    let chars: Vec<char> = text.chars().collect();
    let mut last_good = 0usize;
    for n in 1..=chars.len() {
        let candidate: String = chars[..n].iter().collect();
        let glyphs = Shaper::shape(face, &candidate, size_px)?;
        if run_width(&glyphs) > max_width {
            break;
        }
        last_good = n;
    }
    if last_good == 0 {
        // Even the first character overflows; emit it anyway so we
        // don't loop forever.
        last_good = 1.min(chars.len());
    }
    let head: String = chars[..last_good].iter().collect();
    let tail: String = chars[last_good..].iter().collect();
    Ok((head, tail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeping_whitespace_basic() {
        let v = split_keeping_whitespace("hello world foo");
        assert_eq!(v, vec!["hello", " world", " foo"]);
    }

    #[test]
    fn split_keeping_whitespace_leading_trailing() {
        let v = split_keeping_whitespace("  hi");
        assert_eq!(v, vec!["  hi"]);
    }

    #[test]
    fn empty_text_is_empty_lines() {
        // No face required for empty text.
        // Build a dummy by reusing the Face::from_ttf_bytes path on a
        // real fixture.
        // (No fixture in unit tests — run with the integration test
        // harness for the real measure-and-wrap path.)
    }

    #[test]
    fn visual_ltr_is_identity() {
        let line = reorder_line_visual("abc", None);
        assert_eq!(line.base_level, 0);
        assert_eq!(line.to_visual_string(), "abc");
        assert_eq!(line.logical_to_visual, vec![0, 1, 2]);
        assert_eq!(line.visual_to_logical, vec![0, 1, 2]);
        assert_eq!(line.len(), 3);
        assert!(!line.is_empty());
    }

    #[test]
    fn visual_empty_line() {
        let line = reorder_line_visual("", None);
        assert!(line.is_empty());
        assert_eq!(line.len(), 0);
        assert_eq!(line.to_visual_string(), "");
        assert!(line.logical_to_visual.is_empty());
        assert!(line.visual_to_logical.is_empty());
    }

    #[test]
    fn visual_pure_rtl_reverses() {
        // Three Hebrew letters: a pure-RTL line resolves to base level
        // 1 and the visual order is the logical order reversed.
        let line = reorder_line_visual("\u{05D0}\u{05D1}\u{05D2}", None);
        assert_eq!(line.base_level, 1);
        // Visual = logical reversed.
        assert_eq!(line.logical_to_visual, vec![2, 1, 0]);
        assert_eq!(line.to_visual_string(), "\u{05D2}\u{05D1}\u{05D0}");
        // Inverse permutation is consistent with the forward one.
        for (vis_pos, &log_idx) in line.logical_to_visual.iter().enumerate() {
            assert_eq!(line.visual_to_logical[log_idx], vis_pos);
        }
    }

    #[test]
    fn visual_permutation_is_a_bijection() {
        // Mixed Latin + Hebrew + digits + space: whatever the
        // reordering, the permutation must remain a bijection and the
        // two permutation vectors must invert each other.
        let line = reorder_line_visual("ab \u{05D0}\u{05D1}12", None);
        let n = line.len();
        let mut seen = vec![false; n];
        for &log_idx in &line.logical_to_visual {
            assert!(log_idx < n);
            assert!(!seen[log_idx], "permutation repeats index {log_idx}");
            seen[log_idx] = true;
        }
        assert!(seen.iter().all(|&b| b));
        for (vis_pos, &log_idx) in line.logical_to_visual.iter().enumerate() {
            assert_eq!(line.visual_to_logical[log_idx], vis_pos);
        }
    }

    #[test]
    fn visual_base_level_override() {
        // Forcing base level 1 (RTL) on an all-Latin line flips it to
        // RTL: the line as a whole is laid out right-to-left even though
        // its strong characters are L.
        let ltr = reorder_line_visual("abc", Some(0));
        assert_eq!(ltr.base_level, 0);
        assert_eq!(ltr.to_visual_string(), "abc");

        let rtl = reorder_line_visual("abc", Some(1));
        assert_eq!(rtl.base_level, 1);
        // The Latin run is one level-2 LTR island inside the level-1
        // line, so within the run the characters keep their order.
        assert_eq!(rtl.to_visual_string(), "abc");
        // But the low-bit clamp accepts any odd value as RTL.
        let rtl2 = reorder_line_visual("abc", Some(3));
        assert_eq!(rtl2.base_level, 1);
    }

    #[test]
    fn visual_l4_mirrors_rtl_bracket() {
        // A parenthesis inside a pure-RTL line resolves to an odd level
        // and L4 swaps it for its mirror glyph in the visual output.
        // Logical: he-alef '(' he-bet  ->  the '(' is at an odd level,
        // so the rendered glyph is the mirrored ')'.
        let line = reorder_line_visual("\u{05D0}(\u{05D1}", None);
        assert_eq!(line.base_level, 1);
        let s = line.to_visual_string();
        // The line is reversed and the bracket is mirrored: visual order
        // is bet, mirrored-'(' = ')', alef.
        assert!(
            s.contains(')'),
            "expected mirrored ')' in RTL line, got {s:?}"
        );
        assert!(!s.contains('('), "original '(' should have been mirrored");
    }

    #[test]
    fn visual_ltr_bracket_not_mirrored() {
        // The same bracket in an LTR line stays unmirrored (even level).
        let line = reorder_line_visual("a(b)", None);
        assert_eq!(line.base_level, 0);
        assert_eq!(line.to_visual_string(), "a(b)");
    }
}
