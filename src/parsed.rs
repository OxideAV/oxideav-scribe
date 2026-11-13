//! Face-lifetime parsed-font handle.
//!
//! `oxideav_ttf::Font<'a>` / `oxideav_otf::Font<'a>` borrow the font
//! bytes, and a [`crate::Face`] owns those bytes — so a `Face` that
//! wants to keep the parsed view next to the bytes is a
//! self-referential pair. [`ParsedFont`] is that pair, and the only
//! place in the crate that needs `unsafe`: it parses the bytes exactly
//! once, at construction, and hands out views whose lifetime is tied
//! to the borrow of the handle.
//!
//! ## Why this is sound
//!
//! * The bytes live in a `Vec<u8>` that is never mutated, grown,
//!   shrunk or replaced after construction (the field is private to
//!   this module and no method takes it by `&mut`). A `Vec`'s heap
//!   buffer does not move when the `Vec` value itself is moved, so the
//!   buffer address is stable for the whole life of the handle.
//! * The parsed view is built from a slice of that buffer whose
//!   lifetime is extended to `'static`. That `'static` never leaves
//!   this module: every accessor re-ties the view to the `&self`
//!   borrow, so no caller can keep a view (or anything borrowed from
//!   one) past the handle.
//! * The view is declared before the bytes, so it is dropped first.
//! * Shortening the view's lifetime parameter behind a shared
//!   reference (`&'s Font<'static>` → `&'s Font<'s>`) is done with a
//!   pointer cast rather than relying on variance, so it does not
//!   depend on how the parser crates lay out their lazily-filled
//!   caches. It is sound because everything such a cache can ever hold
//!   is derived from the handle's own bytes, which outlive `'s`.

use crate::Error;

#[cfg(test)]
thread_local! {
    /// Font parses performed on this thread (tests only). Every parse
    /// the crate does goes through [`ParsedFont`], so a count that
    /// stays flat across a call proves the call parsed nothing.
    pub(crate) static PARSE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The parsed view, in whichever flavour the face was built from
/// (boxed: the parser structs are a few kilobytes of table records).
enum View {
    Ttf(Box<oxideav_ttf::Font<'static>>),
    Otf(Box<oxideav_otf::Font<'static>>),
}

/// Font bytes plus the view parsed from them. See the module docs for
/// the invariants.
pub(crate) struct ParsedFont {
    // Field order matters: `view` borrows `bytes` and must drop first.
    view: View,
    bytes: Vec<u8>,
}

impl std::fmt::Debug for ParsedFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParsedFont")
            .field(
                "flavour",
                &match self.view {
                    View::Ttf(_) => "ttf",
                    View::Otf(_) => "otf",
                },
            )
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

/// Extend a borrow of the handle's byte buffer to `'static`.
///
/// # Safety
///
/// The caller must store the returned slice (and anything derived from
/// it) only inside the [`ParsedFont`] that owns `bytes`, must never
/// mutate or reallocate `bytes` afterwards, and must never let the
/// `'static` lifetime escape this module.
unsafe fn extend(bytes: &[u8]) -> &'static [u8] {
    // SAFETY: upheld by the caller as documented above; the pointer
    // and length come from a live slice.
    unsafe { std::slice::from_raw_parts(bytes.as_ptr(), bytes.len()) }
}

fn parse_ttf(
    bytes: &'static [u8],
    subfont: Option<u32>,
) -> Result<oxideav_ttf::Font<'static>, Error> {
    #[cfg(test)]
    PARSE_COUNT.with(|c| c.set(c.get() + 1));
    match subfont {
        Some(i) => oxideav_ttf::Font::from_collection_bytes(bytes, i).map_err(Error::from),
        None => oxideav_ttf::Font::from_bytes(bytes).map_err(Error::from),
    }
}

impl ParsedFont {
    /// Parse `bytes` as a TrueType font (or, with `subfont`, as the
    /// given member of a TrueType Collection).
    pub(crate) fn new_ttf(bytes: Vec<u8>, subfont: Option<u32>) -> Result<Self, Error> {
        // SAFETY: the slice is stored only in `view`, next to the
        // `Vec` it points into, which is never touched again.
        let font = parse_ttf(unsafe { extend(&bytes) }, subfont)?;
        Ok(Self {
            view: View::Ttf(Box::new(font)),
            bytes,
        })
    }

    /// Parse `bytes` as an OpenType-CFF font.
    pub(crate) fn new_otf(bytes: Vec<u8>) -> Result<Self, Error> {
        #[cfg(test)]
        PARSE_COUNT.with(|c| c.set(c.get() + 1));
        // SAFETY: as in `new_ttf`.
        let font = oxideav_otf::Font::from_bytes(unsafe { extend(&bytes) }).map_err(Error::from)?;
        Ok(Self {
            view: View::Otf(Box::new(font)),
            bytes,
        })
    }

    /// The whole font file.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The TrueType view, if this is a TrueType face.
    pub(crate) fn ttf<'s>(&'s self) -> Option<&'s oxideav_ttf::Font<'s>> {
        match &self.view {
            View::Ttf(font) => {
                let font: *const oxideav_ttf::Font<'static> = &**font;
                // SAFETY: only the lifetime parameter changes, and it
                // is shortened to the borrow of `self`, during which
                // the bytes the view points into stay alive and
                // unmodified (module docs).
                Some(unsafe { &*font.cast::<oxideav_ttf::Font<'s>>() })
            }
            View::Otf(_) => None,
        }
    }

    /// The OpenType-CFF view, if this is a CFF face.
    pub(crate) fn otf<'s>(&'s self) -> Option<&'s oxideav_otf::Font<'s>> {
        match &self.view {
            View::Otf(font) => {
                let font: *const oxideav_otf::Font<'static> = &**font;
                // SAFETY: as in `ttf`.
                Some(unsafe { &*font.cast::<oxideav_otf::Font<'s>>() })
            }
            View::Ttf(_) => None,
        }
    }

    /// Adjust the TrueType view in place (used to install a variation
    /// instance). The closure is lifetime-generic, so it cannot store
    /// anything shorter-lived than the handle's bytes into the view.
    /// Returns `None` (without calling `edit`) for a CFF face.
    pub(crate) fn edit_ttf<R>(
        &mut self,
        edit: impl for<'x> FnOnce(&mut oxideav_ttf::Font<'x>) -> R,
    ) -> Option<R> {
        match &mut self.view {
            View::Ttf(font) => Some(edit(font)),
            View::Otf(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEJAVU: &[u8] = include_bytes!("../tests/fixtures/DejaVuSansMono.ttf");
    const SOURCE_SANS: &[u8] = include_bytes!("../tests/fixtures/SourceSans3-Regular.otf");

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn handle_is_send_and_sync() {
        assert_send_sync::<ParsedFont>();
    }

    /// Moving the handle (and so the `Vec` header) leaves the view
    /// valid: the heap buffer it points into does not move.
    #[test]
    fn view_survives_moves_of_the_handle() {
        let a = ParsedFont::new_ttf(DEJAVU.to_vec(), None).unwrap();
        let gid = a.ttf().unwrap().glyph_index('A');
        let boxed = Box::new(a);
        let moved = *boxed;
        let v: Vec<ParsedFont> = std::iter::once(moved).collect();
        let font = v[0].ttf().unwrap();
        assert_eq!(font.glyph_index('A'), gid);
        assert!(font.glyph_outline(gid.unwrap()).is_ok());
        assert!(v[0].otf().is_none());
        assert_eq!(v[0].bytes(), DEJAVU);
    }

    #[test]
    fn flavours_do_not_cross() {
        let o = ParsedFont::new_otf(SOURCE_SANS.to_vec()).unwrap();
        assert!(o.ttf().is_none());
        assert!(o.otf().unwrap().glyph_index('A').is_some());
        let mut o = o;
        assert!(o.edit_ttf(|_| ()).is_none());
        assert!(ParsedFont::new_ttf(vec![0; 64], None).is_err());
        assert!(ParsedFont::new_otf(vec![0; 64]).is_err());
    }

    #[test]
    fn edit_reaches_the_stored_view() {
        let inter = include_bytes!("../tests/fixtures/InterVariable.ttf");
        let mut a = ParsedFont::new_ttf(inter.to_vec(), None).unwrap();
        let defaults = a.ttf().unwrap().variation_coords().to_vec();
        assert!(!defaults.is_empty());
        let axes = a.ttf().unwrap().variation_axes();
        assert_eq!(&axes.last().unwrap().tag, b"wght");
        // The last axis is `wght`; 700 is inside its range.
        let mut bold = defaults.clone();
        *bold.last_mut().unwrap() = 700.0;
        a.edit_ttf(|f| f.set_variation_coords(&bold)).unwrap();
        assert_eq!(a.ttf().unwrap().variation_coords(), &bold[..]);
        a.edit_ttf(|f| f.set_variation_coords(&defaults)).unwrap();
        assert_eq!(a.ttf().unwrap().variation_coords(), &defaults[..]);
    }
}
