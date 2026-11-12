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

/// Text weight, resolved to one of the bundled faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FontWeight {
    /// The regular bundled face.
    Regular,
    /// The bold bundled face.
    Bold,
}

const DEJAVU_SANS: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
const DEJAVU_SANS_BOLD: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf");

/// Indices of the two always-present default faces in [`FontStore::faces`].
pub(crate) const DEFAULT_REGULAR: usize = 0;
pub(crate) const DEFAULT_BOLD: usize = 1;

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
    face: usize,
    ch: char,
    px: u16,
}

/// The renderer's font set: the two bundled default faces, any faces the
/// host registered by family name, and a glyph cache.
///
/// Faces live in a flat `Vec` (index = cache key component); the `ids`
/// map resolves `(family, bold)` to an index once per text run, so the
/// per-glyph hot path stays a plain integer lookup.
pub(crate) struct FontStore {
    faces: Vec<Font>,
    ids: HashMap<(String, bool), usize>,
    cache: HashMap<GlyphKey, Glyph>,
}

impl FontStore {
    /// Loads the bundled DejaVu Sans faces.
    pub(crate) fn bundled() -> Self {
        // fontdue::FontSettings default load flags skip hinting; identical
        // settings on every load keep rasterization reproducible.
        let settings = fontdue::FontSettings::default();
        Self {
            faces: vec![
                Font::from_bytes(DEJAVU_SANS, settings).expect("bundled DejaVuSans.ttf is valid"),
                Font::from_bytes(DEJAVU_SANS_BOLD, settings)
                    .expect("bundled DejaVuSans-Bold.ttf is valid"),
            ],
            ids: HashMap::new(),
            cache: HashMap::new(),
        }
    }

    /// Registers a TTF/OTF face under `family` (matched case-insensitively
    /// at resolution time; the stored key is lowercased here). Re-registering
    /// the same `(family, bold)` slot replaces the face in place — indices
    /// (and therefore glyph cache keys) stay stable. Invalid font data is an
    /// error; the previous face (if any) is untouched.
    pub(crate) fn register(&mut self, family: &str, data: &[u8], bold: bool) -> Result<(), String> {
        let key = (family.to_ascii_lowercase(), bold);
        if let Some(&index) = self.ids.get(&key) {
            let font = Font::from_bytes(data, fontdue::FontSettings::default())
                .map_err(|error| format!("font '{family}' (bold={bold}): {error}"))?;
            self.faces[index] = font;
            self.cache.clear();
            return Ok(());
        }
        let font = Font::from_bytes(data, fontdue::FontSettings::default())
            .map_err(|error| format!("font '{family}' (bold={bold}): {error}"))?;
        let index = self.faces.len();
        self.faces.push(font);
        self.ids.insert(key, index);
        Ok(())
    }

    /// Resolves `(family, weight)` to a face index. Unknown families and
    /// `None` fall back to the bundled DejaVu face for the weight, so a
    /// typo in `font-family` can never panic or change metrics silently
    /// beyond the default.
    pub(crate) fn face_index(&self, family: Option<&str>, weight: FontWeight) -> usize {
        let bold = weight == FontWeight::Bold;
        match family {
            Some(name) if !name.is_empty() => {
                let key = (name.to_ascii_lowercase(), bold);
                match self.ids.get(&key) {
                    Some(&index) => index,
                    // No bold face registered for the family: use its
                    // regular face rather than falling back to DejaVu.
                    None if bold => self
                        .ids
                        .get(&(name.to_ascii_lowercase(), false))
                        .copied()
                        .unwrap_or(DEFAULT_BOLD),
                    None => DEFAULT_REGULAR,
                }
            }
            _ => {
                if bold {
                    DEFAULT_BOLD
                } else {
                    DEFAULT_REGULAR
                }
            }
        }
    }

    /// Ascent (distance above the baseline) in device pixels for `px`-sized text.
    /// Unused since layout centers text with constant metric ratios (layout.rs);
    /// kept for the M2b text overhaul which needs real font metrics.
    #[allow(dead_code)]
    pub(crate) fn ascent(&self, face: usize, px: u16) -> f32 {
        self.faces[face]
            .vertical_line_metrics(px as f32)
            .map(|line| line.ascent)
            .unwrap_or(px as f32 * 0.8)
    }

    /// Returns the rasterized glyph for `ch`, caching by face + pixel size.
    pub(crate) fn glyph(&mut self, face: usize, ch: char, px: u16) -> &Glyph {
        let key = GlyphKey { face, ch, px };
        if !self.cache.contains_key(&key) {
            let (metrics, coverage) = self.faces[face].rasterize(ch, px as f32);
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
        for face in [DEFAULT_REGULAR, DEFAULT_BOLD] {
            let g = fonts.glyph(face, 'V', 16);
            assert!(g.width > 0 && g.height > 0, "face {face} 'V' has a bitmap");
            assert!(g.advance > 0.0);
            assert!(g.coverage.len() >= g.width * g.height);
        }
    }

    #[test]
    fn glyph_cache_is_hit_on_second_request() {
        let mut fonts = FontStore::bundled();
        let first_ptr: *const Glyph = fonts.glyph(DEFAULT_REGULAR, 'a', 12);
        let second_ptr: *const Glyph = fonts.glyph(DEFAULT_REGULAR, 'a', 12);
        assert_eq!(first_ptr, second_ptr);
    }
}
