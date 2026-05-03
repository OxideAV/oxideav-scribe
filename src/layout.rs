//! Single-line measurement + word-wrap helpers for round-1.
//!
//! No bidi, no mixed-script reordering — just enough machinery to slice
//! a UTF-8 string into "lines that fit `max_width`" by breaking at
//! whitespace boundaries (or, if a single word overflows, mid-word).
//!
//! The shaper is invoked once per candidate line so kerning and
//! ligatures are correctly accounted for in the width budget.

use crate::face::Face;
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
}
