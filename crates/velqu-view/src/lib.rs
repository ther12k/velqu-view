//! # velqu-view
//!
//! VelquView's public renderer API.
//!
//! VelquView is a native application UI runtime for local HTML + (Tailwind-derived)
//! CSS documents. It is **not** a web browser: there is no scripting in the
//! renderer, no network, and no browser compatibility surface (see
//! `docs/architecture.md` and the OKF bundle in `docs/okf/`).
//!
//! This crate owns the Velqu-facing API and — for the M1 milestone — a small
//! deterministic CPU rasterizer used to prove the paint pipeline
//! (rectangle, text, resize, DPI, frame capture). HTML-driven layout and
//! styling land in M2+ behind the same API, so application code never depends
//! on renderer internals; the backend stays replaceable
//! (`docs/decisions/0001-m1-paint-backend.md`).
//!
//! # Milestone status
//!
//! * **M1 (current):** `load_html`/`load_css` retain document source and feed a
//!   deterministic *paint probe* scene; `render` rasterizes it offscreen with
//!   no window required.
//! * **M2 (next):** HTML parsing, style cascade, and block/flex/grid layout
//!   replace the probe scene. The API shape below is intended to survive that
//!   swap unchanged.
//!
//! # Example
//!
//! Render a tiny document offscreen and probe a pixel — this runs in CI with
//! no display server:
//!
//! ```
//! use velqu_view::{Color, Viewport, VelquView};
//!
//! let mut view = VelquView::new();
//! view.load_html("<!doctype html><html><body>Hello</body></html>").unwrap();
//! view.load_css("body { background: #10141c; }").unwrap();
//!
//! let result = view.render(Viewport::new(120, 80, 1.0)).unwrap();
//! assert_eq!(result.frame.width(), 120);
//! assert_eq!(result.frame.height(), 80);
//! // Background probe: deterministic exact color at the top-left corner.
//! assert_eq!(result.frame.pixel(2, 2), Some(Color::from_rgb8(0x10, 0x14, 0x1c)));
//!
//! // Frames are reproducible: same inputs, same bytes.
//! let again = view.render(Viewport::new(120, 80, 1.0)).unwrap();
//! assert_eq!(result.frame.pixels(), again.frame.pixels());
//! ```

mod color;
mod font;
mod painter;
mod probe;
mod scene;

use std::fmt;
use std::fmt::Write as _;
use std::path::Path;

pub use color::{Color, ColorParseError};

use font::FontStore;

/// Which document source an error refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// The HTML document source (`load_html`).
    Html,
    /// A CSS stylesheet source (`load_css`).
    Css,
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SourceKind::Html => "HTML",
            SourceKind::Css => "CSS",
        })
    }
}

/// Errors returned by the VelquView API.
#[derive(Debug)]
pub enum VelquError {
    /// A source string was empty or only whitespace.
    EmptySource {
        /// The kind of source that was rejected.
        kind: SourceKind,
    },
    /// [`VelquView::render`] was called before any HTML was loaded.
    DocumentNotLoaded,
}

impl fmt::Display for VelquError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VelquError::EmptySource { kind } => {
                write!(
                    f,
                    "{kind} source is empty; VelquView does not render blank documents"
                )
            }
            VelquError::DocumentNotLoaded => {
                write!(f, "no document loaded; call load_html() before render()")
            }
        }
    }
}

impl std::error::Error for VelquError {}

/// Physical render target description.
///
/// `width`/`height` are in physical (device) pixels; `scale_factor` is the
/// device-pixels-per-logical-pixel ratio (1.0 = 96-dpi nominal). All document
/// geometry is authored in logical pixels and scaled here, so a DPI change is
/// a viewport change — no hidden global DPI state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Physical pixel width. Must be > 0.
    pub width: u32,
    /// Physical pixel height. Must be > 0.
    pub height: u32,
    /// Device pixels per logical pixel (e.g. 2.0 on a typical 2x display).
    pub scale_factor: f32,
}

impl Viewport {
    /// Creates a viewport from physical dimensions and a scale factor.
    pub const fn new(width: u32, height: u32, scale_factor: f32) -> Self {
        Self {
            width,
            height,
            scale_factor,
        }
    }

    /// Logical width (`width / scale_factor`).
    pub fn logical_width(&self) -> f32 {
        self.width as f32 / self.scale_factor
    }

    /// Logical height (`height / scale_factor`).
    pub fn logical_height(&self) -> f32 {
        self.height as f32 / self.scale_factor
    }
}

/// One rendered frame: a tightly packed RGBA8 image, row-major, top-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Frame {
    /// Builds a frame from a tightly packed RGBA8 buffer.
    ///
    /// # Panics
    ///
    /// Panics if `rgba.len() != width * height * 4` (caller bug).
    pub(crate) fn from_parts(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        assert_eq!(
            rgba.len(),
            width as usize * height as usize * 4,
            "pixel buffer size mismatch"
        );
        Self {
            width,
            height,
            rgba,
        }
    }

    /// Frame width in physical pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in physical pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Raw RGBA8 pixels (4 bytes per pixel, row-major, top-down).
    pub fn pixels(&self) -> &[u8] {
        &self.rgba
    }

    /// Exact color at physical pixel coordinates, or `None` when out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y * self.width + x) as usize * 4;
        Some(Color::from_rgba8(
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ))
    }

    /// Stable content digest (hex SHA-256 of the pixel buffer).
    ///
    /// Used by visual fixtures and evidence records. The hash covers pixels
    /// only, not timings or counters, so it is reproducible across runs.
    pub fn sha256_hex(&self) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(&self.rgba);
        let mut hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            write!(hex, "{byte:02x}").expect("writing to String cannot fail");
        }
        hex
    }

    /// Encodes the frame as a PNG file (RGBA8, no compression tricks).
    ///
    /// Used for fixture baselines and evidence capture; the PNG bytes are a
    /// function of the pixels only.
    pub fn save_png(&self, path: &Path) -> std::io::Result<()> {
        let file = std::fs::File::create(path)?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&self.rgba)?;
        Ok(())
    }
}

/// Non-paint metadata about a rendered frame.
#[derive(Debug, Clone, Copy)]
pub struct RenderStats {
    /// 1-based index of this frame within the [`VelquView`] instance.
    pub frame_index: u64,
    /// Viewport the frame was rendered into.
    pub viewport: Viewport,
    /// Number of scene items considered while painting.
    pub items: usize,
    /// Number of glyph bitmaps composited.
    pub glyphs: usize,
}

/// The result of [`VelquView::render`]: the frame plus paint metadata.
#[derive(Debug, Clone)]
pub struct FrameResult {
    /// The rendered image.
    pub frame: Frame,
    /// Paint metadata for diagnostics and fixtures.
    pub stats: RenderStats,
}

/// The VelquView renderer.
///
/// Backend-independent by contract: no method here exposes windowing,
/// GPU, or engine-specific types. The M1 backend is a deterministic CPU
/// rasterizer; a future GPU backend must satisfy the same API without
/// application-visible changes (`docs/decisions/0003-api-boundary.md`).
///
/// Rendering happens fully offscreen; [`crate::velqu_shell`] (a separate
/// crate) is what puts frames into a native window.
#[derive(Debug)]
pub struct VelquView {
    html: Option<String>,
    css: Vec<String>,
    frame_index: u64,
    fonts: FontStore,
}

impl Default for VelquView {
    fn default() -> Self {
        Self::new()
    }
}

impl VelquView {
    /// Creates an empty view with bundled fonts loaded.
    pub fn new() -> Self {
        Self {
            html: None,
            css: Vec::new(),
            frame_index: 0,
            fonts: FontStore::bundled(),
        }
    }

    /// Loads (replaces) the HTML document source.
    ///
    /// M1: the source is validated and retained; it feeds the deterministic
    /// paint probe. M2 parses it into the Velqu DOM behind this same method.
    pub fn load_html(&mut self, html: &str) -> Result<(), VelquError> {
        if html.trim().is_empty() {
            return Err(VelquError::EmptySource {
                kind: SourceKind::Html,
            });
        }
        self.html = Some(html.to_owned());
        Ok(())
    }

    /// Adds a CSS stylesheet source on top of any previously loaded ones.
    pub fn load_css(&mut self, css: &str) -> Result<(), VelquError> {
        if css.trim().is_empty() {
            return Err(VelquError::EmptySource {
                kind: SourceKind::Css,
            });
        }
        self.css.push(css.to_owned());
        Ok(())
    }

    /// The loaded HTML source, if any.
    pub fn html(&self) -> Option<&str> {
        self.html.as_deref()
    }

    /// The loaded CSS sources, in load order.
    pub fn css(&self) -> &[String] {
        &self.css
    }

    /// Number of frames rendered by this instance so far.
    pub fn frames_rendered(&self) -> u64 {
        self.frame_index
    }

    /// Renders one frame into `viewport`, fully offscreen.
    ///
    /// Deterministic: identical instance state + viewport produce identical
    /// pixels. Requires a loaded document.
    pub fn render(&mut self, viewport: Viewport) -> Result<FrameResult, VelquError> {
        let Some(html) = self.html.as_deref() else {
            return Err(VelquError::DocumentNotLoaded);
        };
        self.frame_index += 1;

        let scene = probe::build(html, &self.css, viewport);
        let (frame, items, glyphs) = painter::paint(&scene, viewport, &mut self.fonts);
        Ok(FrameResult {
            frame,
            stats: RenderStats {
                frame_index: self.frame_index,
                viewport,
                items,
                glyphs,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_view() -> VelquView {
        let mut view = VelquView::new();
        view.load_html("<!doctype html><html><body>hello</body></html>")
            .unwrap();
        view
    }

    #[test]
    fn empty_sources_are_rejected() {
        let mut view = VelquView::new();
        assert!(matches!(
            view.load_html("   "),
            Err(VelquError::EmptySource {
                kind: SourceKind::Html
            })
        ));
        assert!(matches!(
            view.load_css(""),
            Err(VelquError::EmptySource {
                kind: SourceKind::Css
            })
        ));
    }

    #[test]
    fn render_requires_a_document() {
        let mut view = VelquView::new();
        assert!(matches!(
            view.render(Viewport::new(64, 64, 1.0)),
            Err(VelquError::DocumentNotLoaded)
        ));
    }

    #[test]
    fn frames_are_reproducible() {
        let mut view = tiny_view();
        let a = view.render(Viewport::new(320, 200, 1.0)).unwrap();
        let b = view.render(Viewport::new(320, 200, 1.0)).unwrap();
        assert_eq!(a.frame, b.frame);
        assert_eq!(a.frame.sha256_hex(), b.frame.sha256_hex());
        assert_eq!(b.stats.frame_index, 2);
    }

    #[test]
    fn viewport_drives_buffer_dimensions() {
        let mut view = tiny_view();
        let f = view.render(Viewport::new(640, 480, 1.0)).unwrap().frame;
        assert_eq!((f.width(), f.height()), (640, 480));
        assert_eq!(f.pixels().len(), 640 * 480 * 4);

        let f2 = view.render(Viewport::new(1280, 960, 2.0)).unwrap().frame;
        assert_eq!((f2.width(), f2.height()), (1280, 960));
    }

    #[test]
    fn different_document_sizes_paint_differently() {
        let mut a = tiny_view();
        a.load_html("<!doctype html><html><body>short</body></html>")
            .unwrap();
        let mut b = tiny_view();
        b.load_html(
            "<!doctype html><html><body>a considerably longer document body \
             so the probe footer reports different sizes</body></html>",
        )
        .unwrap();
        let vp = Viewport::new(400, 300, 1.0);
        let ha = a.render(vp).unwrap().frame.sha256_hex();
        let hb = b.render(vp).unwrap().frame.sha256_hex();
        assert_ne!(ha, hb);
    }

    #[test]
    fn pixel_access_bounds() {
        let mut view = tiny_view();
        let f = view.render(Viewport::new(50, 40, 1.0)).unwrap().frame;
        assert!(f.pixel(0, 0).is_some());
        assert!(f.pixel(49, 39).is_some());
        assert!(f.pixel(50, 0).is_none());
        assert!(f.pixel(0, 40).is_none());
    }

    #[test]
    fn png_round_trip_header() {
        let mut view = tiny_view();
        let f = view.render(Viewport::new(64, 48, 1.0)).unwrap().frame;
        let dir = std::env::temp_dir().join("velqu-view-png-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("frame.png");
        f.save_png(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        // PNG magic.
        assert_eq!(
            &bytes[..8],
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        );
        std::fs::remove_file(&path).ok();
    }
}
