//! Small colour primitives used by the rasterizer + composer.
//!
//! The rest of the workspace standardises on RGBA8 with **straight**
//! alpha (matching `oxideav-pixfmt::alpha::blit_alpha_mask`'s
//! destination convention) so the same convention applies here.

/// Straight-alpha RGBA8 colour. Layout matches `[u8; 4]` so callers can
/// pass either form interchangeably.
pub type Rgba = [u8; 4];

/// Fully opaque white — convenient default for plain white text.
pub const WHITE: Rgba = [255, 255, 255, 255];

/// Fully transparent — used to initialise a brand new RGBA bitmap.
pub const TRANSPARENT: Rgba = [0, 0, 0, 0];
