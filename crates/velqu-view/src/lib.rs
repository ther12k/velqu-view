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
//! # Sources and assets
//!
//! Content carries identity: documents and stylesheets load as
//! [`DocumentSource`]/[`StylesheetSource`] with host-chosen [`SourceId`]s, so
//! stylesheets can be *replaced* (hot reload) and diagnostics can name the
//! offending source. Relative asset references resolve through a
//! host-installed [`AssetResolver`] — the core performs no I/O of its own
//! (ADR 0004). `load_html`/`load_css` remain as auto-id conveniences.
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
//! let viewport = Viewport::try_new(120, 80, 1.0).unwrap();
//! let result = view.render(viewport).unwrap();
//! assert_eq!(result.frame.width(), 120);
//! assert_eq!(result.frame.height(), 80);
//! // Background probe: deterministic exact color at the top-left corner.
//! assert_eq!(result.frame.pixel(2, 2), Some(Color::from_rgb8(0x10, 0x14, 0x1c)));
//!
//! // Frames are reproducible: same inputs, same bytes.
//! let again = view.render(viewport).unwrap();
//! assert_eq!(result.frame.pixels(), again.frame.pixels());
//! ```

mod color;
mod css;
mod display_list;
mod dom;
mod font;
mod html;
mod image;
mod layout;
mod painter;
mod source;
mod style;
mod taffy_backend;
mod text;
mod viewport;

use std::fmt;
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

pub use color::{Color, ColorParseError};
pub use image::{ImageLimits, InvalidImageLimits, InvalidImageLimitsReason};
pub use layout::{LAYOUT_FACTS_SCHEMA_VERSION, LayoutFacts, LayoutNodeFact};
pub use source::{
    Asset, AssetRequest, AssetResolver, DocumentSource, NullAssetResolver, SharedAssetResolver,
    SourceId, StylesheetSource,
};
pub use viewport::{InvalidViewport, InvalidViewportReason, MAX_PIXELS, Viewport};

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
    /// A viewport violated its invariants.
    InvalidViewport(InvalidViewport),
    /// The frame buffer could not be allocated (allocation refused by the
    /// allocator). Viewports are pixel-bounded at construction
    /// ([`MAX_PIXELS`]); this covers the residual OOM path gracefully.
    FrameAllocationFailed {
        /// Frame width that was being allocated.
        width: u32,
        /// Frame height that was being allocated.
        height: u32,
    },
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
            VelquError::InvalidViewport(invalid) => write!(f, "{invalid}"),
            VelquError::FrameAllocationFailed { width, height } => {
                write!(f, "cannot allocate {width}x{height} frame buffer")
            }
        }
    }
}

impl std::error::Error for VelquError {}

impl From<InvalidViewport> for VelquError {
    fn from(invalid: InvalidViewport) -> Self {
        VelquError::InvalidViewport(invalid)
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
pub struct VelquView {
    document: Option<DocumentSource>,
    /// Parsed DOM of the loaded document (rebuilt on every load).
    dom: dom::Dom,
    stylesheets: Vec<StylesheetSource>,
    /// Parsed form of `stylesheets`, kept index-aligned.
    parsed_css: Vec<css::Stylesheet>,
    next_auto_sheet: u32,
    frame_index: u64,
    fonts: FontStore,
    assets: SharedAssetResolver,
    custom_assets: bool,
}

impl Default for VelquView {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for VelquView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VelquView")
            .field("document", &self.document)
            .field("stylesheets", &self.stylesheets)
            .field("frame_index", &self.frame_index)
            .field("fonts", &self.fonts)
            .field("custom_asset_resolver", &self.custom_assets)
            .finish()
    }
}

impl VelquView {
    /// Creates an empty view with bundled fonts loaded and no asset
    /// resolution (the null host: every asset request resolves to nothing).
    pub fn new() -> Self {
        Self {
            document: None,
            dom: dom::Dom::empty(),
            stylesheets: Vec::new(),
            parsed_css: Vec::new(),
            next_auto_sheet: 0,
            frame_index: 0,
            fonts: FontStore::bundled(),
            assets: Rc::new(NullAssetResolver),
            custom_assets: false,
        }
    }

    /// Installs the host's asset resolver.
    ///
    /// The core renderer never performs I/O itself; hosts provide bytes
    /// through this trait (ADR 0004). Installing is a host action — e.g.
    /// `velqu-lab` maps requests onto the app directory.
    pub fn set_asset_resolver(&mut self, resolver: SharedAssetResolver) {
        self.assets = resolver;
        self.custom_assets = true;
    }

    /// Resolves one relative asset reference through the installed resolver,
    /// using the loaded document's base.
    ///
    /// This is the seam M2 image loading will call; exposed now so the
    /// host-boundary contract is testable without a renderer.
    pub fn resolve_asset(&self, path: &str) -> Option<Asset> {
        let base = self.document.as_ref().and_then(|doc| doc.base.as_deref());
        self.assets.resolve(AssetRequest { base, path })
    }

    /// Loads (replaces) the HTML document source.
    ///
    /// Convenience wrapper around [`VelquView::load_document`] with the
    /// stable id `"document"` and no base.
    pub fn load_html(&mut self, html: &str) -> Result<(), VelquError> {
        self.load_document(DocumentSource::new("document", html))
    }

    /// Loads (replaces) the HTML document with identity and base.
    pub fn load_document(&mut self, source: DocumentSource) -> Result<(), VelquError> {
        if source.html.trim().is_empty() {
            return Err(VelquError::EmptySource {
                kind: SourceKind::Html,
            });
        }
        self.dom = html::parse(&source.html);
        self.document = Some(source);
        Ok(())
    }

    /// Adds a CSS stylesheet source on top of any previously loaded ones,
    /// with an auto-generated unique id.
    ///
    /// See [`VelquView::load_stylesheet`] when the host wants to control
    /// identity (replacement, diagnostics).
    pub fn load_css(&mut self, css: &str) -> Result<(), VelquError> {
        let id = format!("stylesheet-{}", self.next_auto_sheet);
        self.next_auto_sheet += 1;
        self.load_stylesheet(StylesheetSource::new(id, css))
    }

    /// Loads a stylesheet with explicit identity: **upserts** by id.
    ///
    /// If a stylesheet with the same [`SourceId`] is already loaded, its
    /// content is replaced in place (same position in cascade order) —
    /// the primitive M6 hot reload builds on. Otherwise the sheet is
    /// appended.
    pub fn load_stylesheet(&mut self, source: StylesheetSource) -> Result<(), VelquError> {
        if source.css.trim().is_empty() {
            return Err(VelquError::EmptySource {
                kind: SourceKind::Css,
            });
        }
        match self
            .stylesheets
            .iter_mut()
            .find(|sheet| sheet.id == source.id)
        {
            Some(existing) => *existing = source,
            None => self.stylesheets.push(source),
        }
        self.rebuild_css();
        Ok(())
    }

    /// Re-parses every stylesheet source (cssparser is fast; sheets are
    /// small). Rule `order` values stay globally consistent across sheets.
    fn rebuild_css(&mut self) {
        let mut order = 0;
        self.parsed_css = self
            .stylesheets
            .iter()
            .map(|sheet| {
                let parsed = css::parse(sheet, order);
                order += parsed.rules.len() as u32;
                parsed
            })
            .collect();
    }

    /// Diagnostics from parsing all loaded stylesheets (deterministic, in
    /// source order) — the base the M3 checker and Lab console build on.
    pub fn css_diagnostics(&self) -> Vec<String> {
        self.parsed_css
            .iter()
            .flat_map(|sheet| {
                let source = sheet.source.as_str().to_owned();
                sheet
                    .diagnostics
                    .iter()
                    .map(move |d| format!("{source}: {d}"))
            })
            .collect()
    }

    /// The loaded HTML source text, if any.
    pub fn html(&self) -> Option<&str> {
        self.document.as_ref().map(|doc| doc.html.as_str())
    }

    /// The loaded document (identity + base), if any.
    pub fn document(&self) -> Option<&DocumentSource> {
        self.document.as_ref()
    }

    /// The loaded stylesheets, in cascade (load) order.
    pub fn stylesheets(&self) -> &[StylesheetSource] {
        &self.stylesheets
    }

    /// Number of frames rendered by this instance so far.
    pub fn frames_rendered(&self) -> u64 {
        self.frame_index
    }

    /// Renders one frame into `viewport`, fully offscreen.
    ///
    /// Deterministic: identical instance state + viewport produce identical
    /// pixels. Requires a loaded document. The pipeline is
    /// DOM → cascade → block layout → display list → paint (ADR 0005); the
    /// M1 probe scene is retired.
    pub fn render(&mut self, viewport: Viewport) -> Result<FrameResult, VelquError> {
        if self.document.is_none() {
            return Err(VelquError::DocumentNotLoaded);
        }
        self.frame_index += 1;

        // UA defaults (M2a baseline: body margin, heading sizes, hidden
        // head elements) then author sheets, in order.
        let ua_sheet = StylesheetSource::new("velqu:ua", UA_STYLESHEET);
        let ua_parsed = css::parse(&ua_sheet, 0);
        let mut order = ua_parsed.rules.len() as u32;
        let mut parsed_author = Vec::with_capacity(self.parsed_css.len());
        for sheet in &self.stylesheets {
            let parsed = css::parse(sheet, order);
            order += parsed.rules.len() as u32;
            parsed_author.push(parsed);
        }

        let mut cascade = style::Cascade::new(&ua_parsed.rules, &parsed_author);
        let Some((_root_box, display_list)) =
            layout::layout_document(&self.dom, viewport, &mut cascade, &mut self.fonts)
        else {
            // Nothing visible (e.g. an all-hidden document): paint the
            // author background only.
            let background = document_background(&parsed_author);
            let (frame, items, glyphs) = painter::paint_document(
                &display_list::DisplayList::default(),
                background,
                viewport,
                &mut self.fonts,
            )?;
            return Ok(FrameResult {
                frame,
                stats: RenderStats {
                    frame_index: self.frame_index,
                    viewport,
                    items,
                    glyphs,
                },
            });
        };

        let background = document_background(&parsed_author);
        let (frame, items, glyphs) =
            painter::paint_document(&display_list, background, viewport, &mut self.fonts)?;
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

    /// Lays the current document out and returns fixture-facing facts.
    ///
    /// Runs the same cascade+layout pass as [`VelquView::render`] minus
    /// painting. Keys are `data-vv-test` values (ADR 0005).
    pub fn layout_facts(&mut self, viewport: Viewport) -> Result<LayoutFacts, VelquError> {
        if self.document.is_none() {
            return Err(VelquError::DocumentNotLoaded);
        }
        let ua_sheet = StylesheetSource::new("velqu:ua", UA_STYLESHEET);
        let ua_parsed = css::parse(&ua_sheet, 0);
        let mut order = ua_parsed.rules.len() as u32;
        let mut parsed_author = Vec::with_capacity(self.parsed_css.len());
        for sheet in &self.stylesheets {
            let parsed = css::parse(sheet, order);
            order += parsed.rules.len() as u32;
            parsed_author.push(parsed);
        }
        let mut cascade = style::Cascade::new(&ua_parsed.rules, &parsed_author);
        let mut fonts = font::FontStore::bundled();
        let Some((root, _)) =
            layout::layout_document(&self.dom, viewport, &mut cascade, &mut fonts)
        else {
            return Ok(LayoutFacts {
                schema_version: layout::LAYOUT_FACTS_SCHEMA_VERSION,
                viewport_width: viewport.width(),
                viewport_height: viewport.height(),
                scale: viewport.scale_factor(),
                nodes: Vec::new(),
            });
        };
        let mut facts = LayoutFacts {
            schema_version: layout::LAYOUT_FACTS_SCHEMA_VERSION,
            viewport_width: viewport.width(),
            viewport_height: viewport.height(),
            scale: viewport.scale_factor(),
            nodes: Vec::new(),
        };
        layout::collect_facts(&self.dom, &root, viewport, &mut facts);
        Ok(facts)
    }
}

/// The author-declared page background: `html`'s, then `body`'s, then white
/// (CSS background propagation, simplified).
fn document_background(sheets: &[css::Stylesheet]) -> Color {
    let mut body_background = None;
    for sheet in sheets {
        for rule in &sheet.rules {
            for declaration in &rule.declarations {
                if declaration.property == "background-color"
                    || declaration.property == "background"
                {
                    if let Some(color) = style::parse_color(&declaration.value) {
                        let matches_root = rule.selectors.iter().any(|selector| {
                            selector.segments.len() == 1
                                && selector.segments[0]
                                    .compound
                                    .simples
                                    .iter()
                                    .any(|s| {
                                        matches!(s, css::Simple::Type(tag) if tag == "html" || tag == "body")
                                    })
                        });
                        if matches_root && body_background.is_none() {
                            body_background = Some(color);
                        }
                    }
                }
            }
        }
    }
    body_background.unwrap_or(Color::WHITE)
}

/// The M2a UA stylesheet: minimal defaults the cascade cannot express as
/// per-tag Rust defaults alone (here: nothing beyond what `ua_default`
/// covers; kept as a hook for spec-derived UA rules).
const UA_STYLESHEET: &str = "";

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

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
            view.render(Viewport::try_new(64, 64, 1.0).unwrap()),
            Err(VelquError::DocumentNotLoaded)
        ));
    }

    #[test]
    fn frames_are_reproducible() {
        let mut view = tiny_view();
        let vp = Viewport::try_new(320, 200, 1.0).unwrap();
        let a = view.render(vp).unwrap();
        let b = view.render(vp).unwrap();
        assert_eq!(a.frame, b.frame);
        assert_eq!(a.frame.sha256_hex(), b.frame.sha256_hex());
        assert_eq!(b.stats.frame_index, 2);
    }

    #[test]
    fn viewport_drives_buffer_dimensions() {
        let mut view = tiny_view();
        let f = view
            .render(Viewport::try_new(640, 480, 1.0).unwrap())
            .unwrap()
            .frame;
        assert_eq!((f.width(), f.height()), (640, 480));
        assert_eq!(f.pixels().len(), 640 * 480 * 4);

        let f2 = view
            .render(Viewport::try_new(1280, 960, 2.0).unwrap())
            .unwrap()
            .frame;
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
        let vp = Viewport::try_new(400, 300, 1.0).unwrap();
        let ha = a.render(vp).unwrap().frame.sha256_hex();
        let hb = b.render(vp).unwrap().frame.sha256_hex();
        assert_ne!(ha, hb);
    }

    #[test]
    fn pixel_access_bounds() {
        let mut view = tiny_view();
        let f = view
            .render(Viewport::try_new(50, 40, 1.0).unwrap())
            .unwrap()
            .frame;
        assert!(f.pixel(0, 0).is_some());
        assert!(f.pixel(49, 39).is_some());
        assert!(f.pixel(50, 0).is_none());
        assert!(f.pixel(0, 40).is_none());
    }

    #[test]
    fn png_round_trip_header() {
        let mut view = tiny_view();
        let f = view
            .render(Viewport::try_new(64, 48, 1.0).unwrap())
            .unwrap()
            .frame;
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

    #[test]
    fn load_html_is_a_named_document() {
        let view = tiny_view();
        let doc = view.document().unwrap();
        assert_eq!(doc.id.as_str(), "document");
        assert_eq!(doc.base, None);
    }

    #[test]
    fn load_document_carries_identity_and_base() {
        let mut view = VelquView::new();
        view.load_document(
            DocumentSource::new("apps/demo/index.html", "<html></html>").with_base("apps/demo"),
        )
        .unwrap();
        let doc = view.document().unwrap();
        assert_eq!(doc.id.as_str(), "apps/demo/index.html");
        assert_eq!(doc.base.as_deref(), Some("apps/demo"));
        // Replacing the document swaps identity wholesale.
        view.load_document(DocumentSource::new(
            "other.html",
            "<html><body>x</body></html>",
        ))
        .unwrap();
        assert_eq!(view.document().unwrap().id.as_str(), "other.html");
    }

    #[test]
    fn load_css_generates_unique_ids() {
        let mut view = tiny_view();
        view.load_css("a { color: red }").unwrap();
        view.load_css("b { color: blue }").unwrap();
        let ids: Vec<&str> = view.stylesheets().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["stylesheet-0", "stylesheet-1"]);
    }

    #[test]
    fn load_stylesheet_upserts_by_id_in_place() {
        let mut view = tiny_view();
        view.load_stylesheet(StylesheetSource::new("app.css", "a { color: red }"))
            .unwrap();
        view.load_stylesheet(StylesheetSource::new("extra.css", "b { color: blue }"))
            .unwrap();
        // Replace app.css: same id, new content, cascade position preserved.
        view.load_stylesheet(StylesheetSource::new("app.css", "a { color: green }"))
            .unwrap();

        let sheets = view.stylesheets();
        assert_eq!(sheets.len(), 2, "replacement must not append");
        assert_eq!(sheets[0].id.as_str(), "app.css");
        assert_eq!(sheets[0].css, "a { color: green }");
        assert_eq!(sheets[1].id.as_str(), "extra.css");
    }

    #[test]
    fn stylesheet_content_reaches_the_frame() {
        // Tall enough that the probe's document/css-size status line (y≈208
        // logical) is actually inside the viewport.
        let vp = Viewport::try_new(400, 300, 1.0).unwrap();
        let mut before = tiny_view();
        before.load_css("body { margin: 0 }").unwrap();
        let hash_before = before.render(vp).unwrap().frame.sha256_hex();

        let mut after = tiny_view();
        after
            .load_stylesheet(StylesheetSource::new(
                "stylesheet-0",
                "body { margin: 4px }",
            ))
            .unwrap();
        let hash_after = after.render(vp).unwrap().frame.sha256_hex();

        // The probe echoes document/css byte totals, so equal-length but
        // different stylesheet content yields a different frame digest.
        assert_ne!(hash_before, hash_after);
    }

    struct RecordingResolver {
        seen: RefCell<Vec<(Option<String>, String)>>,
    }

    impl AssetResolver for RecordingResolver {
        fn resolve(&self, request: AssetRequest<'_>) -> Option<Asset> {
            self.seen
                .borrow_mut()
                .push((request.base.map(str::to_owned), request.path.to_owned()));
            Some(Asset {
                id: SourceId::new(request.path),
                bytes: b"png-bytes".to_vec(),
            })
        }
    }

    #[test]
    fn default_host_resolves_nothing() {
        let view = tiny_view();
        assert!(view.resolve_asset("logo.png").is_none());
    }

    #[test]
    fn resolve_asset_threads_document_base_to_the_host() {
        let resolver = Rc::new(RecordingResolver {
            seen: RefCell::new(Vec::new()),
        });
        let mut view = VelquView::new();
        view.load_document(
            DocumentSource::new("apps/demo/index.html", "<html></html>").with_base("apps/demo"),
        )
        .unwrap();
        view.set_asset_resolver(resolver.clone());

        let asset = view.resolve_asset("img/logo.png").unwrap();
        assert_eq!(asset.bytes, b"png-bytes");
        assert_eq!(asset.id.as_str(), "img/logo.png");
        assert_eq!(
            resolver.seen.borrow().as_slice(),
            [(Some("apps/demo".into()), "img/logo.png".into())]
        );
    }

    #[test]
    fn debug_mentions_resolver_installation() {
        let mut view = tiny_view();
        let plain = format!("{:?}", view);
        assert!(plain.contains("custom_asset_resolver: false"));
        view.set_asset_resolver(Rc::new(RecordingResolver {
            seen: RefCell::new(Vec::new()),
        }));
        assert!(format!("{:?}", view).contains("custom_asset_resolver: true"));
    }
}
