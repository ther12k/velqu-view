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
use std::rc::Rc;

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
    faces: Vec<Rc<Font>>,
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
                Rc::new(
                    Font::from_bytes(DEJAVU_SANS, settings)
                        .expect("bundled DejaVuSans.ttf is valid"),
                ),
                Rc::new(
                    Font::from_bytes(DEJAVU_SANS_BOLD, settings)
                        .expect("bundled DejaVuSans-Bold.ttf is valid"),
                ),
            ],
            ids: HashMap::new(),
            cache: HashMap::new(),
        }
    }

    /// Shares the registered faces with a reload candidate, but not its
    /// mutable glyph cache. A failed reload leaves this store untouched.
    pub(crate) fn fork(&self) -> Self {
        Self {
            faces: self.faces.clone(),
            ids: self.ids.clone(),
            cache: HashMap::new(),
        }
    }

    /// Parse both faces before replacing either slot, so an invalid font
    /// leaves the registry and the current layout intact.
    pub(crate) fn register_family(
        &mut self,
        family: &str,
        regular: &[u8],
        bold: Option<&[u8]>,
    ) -> Result<(), String> {
        let parse = |data: &[u8], bold: bool| {
            Font::from_bytes(data, fontdue::FontSettings::default())
                .map(Rc::new)
                .map_err(|error| format!("font '{family}' (bold={bold}): {error}"))
        };
        let regular = parse(regular, false)?;
        let bold = bold.map(|data| parse(data, true)).transpose()?;
        let family = family.to_ascii_lowercase();
        self.set_face((family.clone(), false), regular);
        if let Some(bold) = bold {
            self.set_face((family, true), bold);
        }
        self.cache.clear();
        Ok(())
    }

    fn set_face(&mut self, key: (String, bool), face: Rc<Font>) {
        if let Some(&index) = self.ids.get(&key) {
            self.faces[index] = face;
        } else {
            self.ids.insert(key, self.faces.len());
            self.faces.push(face);
        }
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

    /// The face's vertical metrics at `px`: ascent (≥ 0, above the
    /// baseline) and descent (≤ 0, below) — from the font's horizontal
    /// (hhea) metrics, the ones horizontal text layout uses (Latin fonts
    /// rarely carry vmtx). Text baselines center this content range in
    /// the line box (layout.rs), matching the painter's baseline
    /// semantics; a face without metrics falls back to a constant model
    /// so unknown families keep the old behavior.
    pub(crate) fn verticals(&self, face: usize, px: u16) -> (f32, f32) {
        self.faces[face]
            .horizontal_line_metrics(px as f32)
            .map(|line| (line.ascent, line.descent))
            .unwrap_or((px as f32 * 0.8, px as f32 * -0.2))
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
