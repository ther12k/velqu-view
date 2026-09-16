//! Bundled fonts and a rasterized-glyph cache.
//!
//! M1 bundles DejaVu Sans (regular + bold) so that text rendering is
//! deterministic and independent of system font configuration — fixture
//! hashes must not depend on whatever fonts a machine happens to have.
//! Font files are committed under `assets/fonts/` with their license.
//!
//! Glyph sizes are quantized to whole device pixels: the cache key rounds
//! `logical_size * scale_factor` once, and that same rounded size is what the
//! painter requests, so identical viewports always hit identical rasterization.

use std::collections::HashMap;

use fontdue::Font;

use crate::scene::FontWeight;

const DEJAVU_SANS: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
const DEJAVU_SANS_BOLD: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf");

/// A rasterized glyph bitmap plus its placement metrics, in device pixels.
pub(crate) struct Glyph {
    /// Whole-pixel offset of the bitmap's left edge from the pen position.
    pub xmin: i32,
    /// Whole-pixel offset of the bitmap's bottom edge from the baseline.
    pub ymin: i32,
    /// Bitmap width in pixels.
    pub width: usize,
    /// Bitmap height in pixels.
    pub height: usize,
    /// Horizontal advance in device pixels (subpixel).
    pub advance: f32,
    /// Coverage bitmap, one byte per pixel (0–255), row-major, top-down.
    pub coverage: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    weight: FontWeight,
    ch: char,
    px: u16,
}

/// The renderer's font set: two bundled faces plus a glyph cache.
pub(crate) struct FontStore {
    regular: Font,
    bold: Font,
    cache: HashMap<GlyphKey, Glyph>,
}

impl FontStore {
    /// Loads the bundled DejaVu Sans faces.
    pub(crate) fn bundled() -> Self {
        // fontdue::FontSettings default load flags skip hinting; identical
        // settings on every load keep rasterization reproducible.
        let settings = fontdue::FontSettings::default();
        Self {
            regular: Font::from_bytes(DEJAVU_SANS, settings)
                .expect("bundled DejaVuSans.ttf is valid"),
            bold: Font::from_bytes(DEJAVU_SANS_BOLD, settings)
                .expect("bundled DejaVuSans-Bold.ttf is valid"),
            cache: HashMap::new(),
        }
    }

    fn face(&self, weight: FontWeight) -> &Font {
        match weight {
            FontWeight::Regular => &self.regular,
            FontWeight::Bold => &self.bold,
        }
    }

    /// Ascent (distance above the baseline) in device pixels for `px`-sized text.
    pub(crate) fn ascent(&self, weight: FontWeight, px: u16) -> f32 {
        self.face(weight)
            .vertical_line_metrics(px as f32)
            .map(|line| line.ascent)
            .unwrap_or(px as f32 * 0.8)
    }

    /// Returns the rasterized glyph for `ch`, caching by weight + pixel size.
    pub(crate) fn glyph(&mut self, weight: FontWeight, ch: char, px: u16) -> &Glyph {
        let key = GlyphKey { weight, ch, px };
        if !self.cache.contains_key(&key) {
            let (metrics, coverage) = self.face(weight).rasterize(ch, px as f32);
            let glyph = Glyph {
                xmin: metrics.xmin,
                ymin: metrics.ymin,
                width: metrics.width,
                height: metrics.height,
                advance: metrics.advance_width,
                coverage,
            };
            self.cache.insert(key, glyph);
        }
        self.cache.get(&key).expect("glyph was just inserted")
    }
}

impl std::fmt::Debug for FontStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontStore")
            .field("cached_glyphs", &self.cache.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_faces_rasterize() {
        let mut fonts = FontStore::bundled();
        for weight in [FontWeight::Regular, FontWeight::Bold] {
            let g = fonts.glyph(weight, 'V', 16);
            assert!(g.width > 0 && g.height > 0, "{weight:?} 'V' has a bitmap");
            assert!(g.advance > 0.0);
            assert!(g.coverage.len() >= g.width * g.height);
        }
    }

    #[test]
    fn glyph_cache_is_hit_on_second_request() {
        let mut fonts = FontStore::bundled();
        let first_ptr: *const Glyph = fonts.glyph(FontWeight::Regular, 'a', 12);
        let second_ptr: *const Glyph = fonts.glyph(FontWeight::Regular, 'a', 12);
        assert_eq!(first_ptr, second_ptr);
    }
}
