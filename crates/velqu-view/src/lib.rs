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

use crate::image::ImageStore;
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
    /// A scroll offset passed to [`VelquView::set_scroll_offset`] was not a
    /// finite number. Negative values are allowed (clamped centrally);
    /// NaN/infinite offsets are rejected.
    InvalidScrollOffset {
        /// The rejected x value.
        x: f32,
        /// The rejected y value.
        y: f32,
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
            VelquError::InvalidScrollOffset { x, y } => {
                write!(f, "scroll offset must be finite, got ({x}, {y})")
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

/// Layout instrumentation (ADR 0008): enough to *see* whether whole-tree
/// projection or mutation-driven relayouts get expensive in later
/// milestones, before anyone optimizes anything.
#[derive(Debug, Clone, Copy)]
pub struct LayoutStats {
    /// Layout passes run by this instance (every render and facts call is
    /// one pass).
    pub passes: u64,
    /// Boxes laid out by the most recent pass.
    pub nodes_last_pass: usize,
    /// Wall time of the most recent pass. Indicative only — never part of
    /// pixel output or facts.
    pub duration_last_pass: std::time::Duration,
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
    /// Decoded `<img>` assets for the current document (ADR 0008), keyed by
    /// the `src` reference as written. Cleared when the document changes.
    images: ImageStore,
    image_limits: ImageLimits,
    /// Deterministic image diagnostics from the last prepare pass, in
    /// document order.
    image_diagnostics: Vec<String>,
    /// Runtime scroll offsets (ADR 0008): target key → raw (unclamped)
    /// offset. The empty key is the document-level scroller.
    scroll_offsets: layout::ScrollOffsets,
    // Layout instrumentation (ADR 0008).
    layout_passes: u64,
    layout_nodes_last: usize,
    layout_duration_last: std::time::Duration,
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
            images: ImageStore::new(),
            image_limits: ImageLimits::default(),
            image_diagnostics: Vec::new(),
            scroll_offsets: Vec::new(),
            layout_passes: 0,
            layout_nodes_last: 0,
            layout_duration_last: std::time::Duration::ZERO,
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
    /// This is the seam `<img src>` loading uses internally; exposed so the
    /// host-boundary contract is testable without a renderer.
    pub fn resolve_asset(&self, path: &str) -> Option<Asset> {
        let base = self.document.as_ref().and_then(|doc| doc.base.as_deref());
        self.assets.resolve(AssetRequest { base, path })
    }

    /// Replaces the image decode limits (ADR 0008). The decoded-image cache
    /// is cleared: limits are decode decisions, and a cached success/failure
    /// under old limits must not survive them.
    pub fn set_image_limits(&mut self, limits: ImageLimits) {
        self.image_limits = limits;
        self.images.clear();
    }

    /// Deterministic diagnostics about `<img>` assets from the last render
    /// or layout pass, in document order (ADR 0008). A broken or missing
    /// image still lays out — at the default object size — so this list is
    /// the only place its failure is reported.
    pub fn image_diagnostics(&self) -> Vec<String> {
        self.image_diagnostics.clone()
    }

    /// Resolves and decodes every `<img src>` reference once per document:
    /// requests flow out through the installed resolver with the document's
    /// base, decode is bounded by the image limits, and outcomes (success
    /// and failure) are cached per `src` so later frames never re-decode.
    fn prepare_images(&mut self) {
        self.image_diagnostics.clear();
        let mut srcs: Vec<String> = Vec::new();
        self.dom.walk(|_id, node| {
            if let dom::NodeData::Element { name, attrs, .. } = &node.data {
                if name == "img" {
                    if let Some(src) = attrs
                        .iter()
                        .find(|a| a.name == "src")
                        .map(|a| a.value.clone())
                    {
                        if !srcs.contains(&src) {
                            srcs.push(src);
                        }
                    }
                }
            }
        });
        let base = self.document.as_ref().and_then(|doc| doc.base.clone());
        for src in &srcs {
            // Cached outcomes are re-reported (failures) or skipped
            // (successes) so diagnostics stay deterministic per frame.
            let cached_failure = match self.images.get(src) {
                Some(crate::image::ImageEntry::Failed(failure)) => Some(failure.clone()),
                _ => None,
            };
            if let Some(failure) = cached_failure {
                let message = failure.message();
                self.image_diagnostics
                    .push(format!("image {src:?}: {message}"));
                continue;
            }
            if self.images.get(src).is_some() {
                continue; // cached success from an earlier frame
            }
            let request = AssetRequest {
                base: base.as_deref(),
                path: src,
            };
            match self.assets.resolve(request) {
                None => {
                    self.images.mark_missing(src);
                    let message = crate::image::ImageFailure::Missing.message();
                    self.image_diagnostics
                        .push(format!("image {src:?}: {message}"));
                }
                Some(asset) => {
                    if let Some(failure) = self.images.load(src, &asset.bytes, &self.image_limits) {
                        let message = failure.message();
                        self.image_diagnostics
                            .push(format!("image {src:?}: {message}"));
                    }
                }
            }
        }
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
        // Image identity is per-document, like the DOM: a new document
        // invalidates every decoded asset.
        self.images.clear();
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
        self.prepare_images();

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
        let laid_out = self.run_layout(viewport, &mut cascade);
        let Some(laid) = laid_out else {
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
            painter::paint_document(&laid.display_list, background, viewport, &mut self.fonts)?;
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

    /// Shared cascade+layout pass behind [`VelquView::render`] and
    /// [`VelquView::layout_facts`]; records layout instrumentation.
    fn run_layout(
        &mut self,
        viewport: Viewport,
        cascade: &mut style::Cascade<'_>,
    ) -> Option<layout::LaidOutDocument> {
        let started = std::time::Instant::now();
        let laid = layout::layout_document(
            &self.dom,
            viewport,
            cascade,
            &mut self.fonts,
            &self.images,
            &self.scroll_offsets,
        );
        self.layout_passes += 1;
        self.layout_duration_last = started.elapsed();
        self.layout_nodes_last = laid
            .as_ref()
            .map(|laid| layout::count_box_tree(&laid.root))
            .unwrap_or(0);
        laid
    }

    /// Lays the current document out and returns fixture-facing facts.
    ///
    /// Runs the same cascade+layout pass as [`VelquView::render`] minus
    /// painting. Keys are `data-vv-test` values (ADR 0005).
    pub fn layout_facts(&mut self, viewport: Viewport) -> Result<LayoutFacts, VelquError> {
        if self.document.is_none() {
            return Err(VelquError::DocumentNotLoaded);
        }
        self.prepare_images();
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
        let Some(laid) = self.run_layout(viewport, &mut cascade) else {
            return Ok(LayoutFacts {
                schema_version: layout::LAYOUT_FACTS_SCHEMA_VERSION,
                viewport_width: viewport.width(),
                viewport_height: viewport.height(),
                scale: viewport.scale_factor(),
                nodes: Vec::new(),
                document_scroll_width: viewport.width() as f32,
                document_scroll_height: viewport.height() as f32,
            });
        };
        let mut facts = LayoutFacts {
            schema_version: layout::LAYOUT_FACTS_SCHEMA_VERSION,
            viewport_width: viewport.width(),
            viewport_height: viewport.height(),
            scale: viewport.scale_factor(),
            nodes: Vec::new(),
            document_scroll_width: laid.document_scroll.width,
            document_scroll_height: laid.document_scroll.height,
        };
        layout::collect_facts(&self.dom, &laid.root, viewport, &mut facts);
        Ok(facts)
    }

    /// Sets the scroll offset of a scroll container (ADR 0008).
    ///
    /// `None` targets the document-level scroller (the viewport);
    /// `Some(id)` targets the element with HTML `id="id"` that is a scroll
    /// container (`overflow: auto`/`scroll`). Offsets are **clamped
    /// centrally** at apply time to `0..=extent - scrollport`, so over- and
    /// under-flowing requests are safe. Scroll position is runtime
    /// presentation state: layout facts report unscrolled geometry, and
    /// scrolling never triggers a new layout pass.
    ///
    /// Unknown targets are accepted and simply never apply — matching the
    /// clamping philosophy that offsets are requests, not commands.
    pub fn set_scroll_offset(
        &mut self,
        target: Option<&str>,
        x: f32,
        y: f32,
    ) -> Result<(), VelquError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(VelquError::InvalidScrollOffset { x, y });
        }
        let key = target.unwrap_or("").to_owned();
        let entry = (key, (x.max(0.0), y.max(0.0)));
        match self
            .scroll_offsets
            .iter_mut()
            .find(|(existing, _)| *existing == entry.0)
        {
            Some(slot) => slot.1 = entry.1,
            None => self.scroll_offsets.push(entry),
        }
        Ok(())
    }

    /// Layout instrumentation (ADR 0008): how many layout passes this
    /// instance has run, how many boxes the last pass laid out, and how
    /// long the last pass took. Passes and node counts are deterministic;
    /// the duration is indicative only and never enters pixel output.
    pub fn layout_stats(&self) -> LayoutStats {
        LayoutStats {
            passes: self.layout_passes,
            nodes_last_pass: self.layout_nodes_last,
            duration_last_pass: self.layout_duration_last,
        }
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

    // -- M2c images (ADR 0008) -------------------------------------------

    /// Serves a deterministic 40×20 red PNG to every request, recording it.
    struct ImageRecordingResolver {
        seen: RefCell<Vec<(Option<String>, String)>>,
        bytes: Vec<u8>,
    }

    impl AssetResolver for ImageRecordingResolver {
        fn resolve(&self, request: AssetRequest<'_>) -> Option<Asset> {
            self.seen
                .borrow_mut()
                .push((request.base.map(str::to_owned), request.path.to_owned()));
            Some(Asset {
                id: SourceId::new(request.path),
                bytes: self.bytes.clone(),
            })
        }
    }

    fn red_png() -> Vec<u8> {
        crate::image::test_png(40, 20, [0xef, 0x44, 0x44, 0xff])
    }

    fn img_view(resolver: ImageRecordingResolver) -> VelquView {
        let mut view = VelquView::new();
        view.load_document(
            DocumentSource::new(
                "apps/demo/index.html",
                "<html><body><img data-vv-test=logo src=img/logo.png></body></html>",
            )
            .with_base("apps/demo"),
        )
        .unwrap();
        view.set_asset_resolver(Rc::new(resolver));
        view
    }

    fn fact_of<'a>(facts: &'a LayoutFacts, id: &str) -> &'a LayoutNodeFact {
        facts
            .nodes
            .iter()
            .find(|n| n.fixture_id == id)
            .unwrap_or_else(|| panic!("no fact for {id}"))
    }

    #[test]
    fn img_resolves_through_the_host_and_lays_out_intrinsically() {
        let resolver = ImageRecordingResolver {
            seen: RefCell::new(Vec::new()),
            bytes: red_png(),
        };
        let mut view = img_view(resolver);
        let vp = Viewport::try_new(400, 300, 1.0).unwrap();
        let facts = view.layout_facts(vp).unwrap();
        let logo = fact_of(&facts, "logo");
        assert_eq!((logo.width, logo.height), (40.0, 20.0), "intrinsic size");
        assert!(view.image_diagnostics().is_empty());
    }

    #[test]
    fn img_requests_carry_the_document_base() {
        let resolver = ImageRecordingResolver {
            seen: RefCell::new(Vec::new()),
            bytes: red_png(),
        };
        let mut view = VelquView::new();
        view.load_document(
            DocumentSource::new(
                "apps/demo/index.html",
                "<html><body><img src=img/logo.png></body></html>",
            )
            .with_base("apps/demo"),
        )
        .unwrap();
        let r = Rc::new(resolver);
        view.set_asset_resolver(r.clone());
        let vp = Viewport::try_new(400, 300, 1.0).unwrap();
        view.layout_facts(vp).unwrap();
        assert_eq!(
            r.seen.borrow().as_slice(),
            [(Some("apps/demo".into()), "img/logo.png".into())]
        );
    }

    #[test]
    fn missing_image_is_broken_with_a_deterministic_diagnostic() {
        let mut view = VelquView::new(); // null host: resolves nothing
        view.load_html("<html><body><img data-vv-test=i src=pic.png></body></html>")
            .unwrap();
        let vp = Viewport::try_new(400, 300, 1.0).unwrap();
        let facts = view.layout_facts(vp).unwrap();
        let i = fact_of(&facts, "i");
        assert_eq!(
            (i.width, i.height),
            (300.0, 150.0),
            "broken image keeps the default object size"
        );
        assert_eq!(
            view.image_diagnostics(),
            ["image \"pic.png\": not provided by the host asset resolver"]
        );
        // Repeated frames do not duplicate the diagnostic or re-resolve.
        view.render(vp).unwrap();
        assert_eq!(view.image_diagnostics().len(), 1);
    }

    #[test]
    fn image_limits_bound_decode_and_diagnose() {
        let bytes = red_png();
        let mut view = VelquView::new();
        view.load_html("<html><body><img data-vv-test=i src=pic.png></body></html>")
            .unwrap();
        view.set_asset_resolver(Rc::new(ImageRecordingResolver {
            seen: RefCell::new(Vec::new()),
            bytes: bytes.clone(),
        }));
        // Width cap below the intrinsic 40px: decode refuses, image breaks.
        view.set_image_limits(ImageLimits::try_new(bytes.len(), 10, 4096, 1 << 20).unwrap());
        let vp = Viewport::try_new(400, 300, 1.0).unwrap();
        let facts = view.layout_facts(vp).unwrap();
        assert_eq!(
            (fact_of(&facts, "i").width, fact_of(&facts, "i").height),
            (300.0, 150.0)
        );
        assert_eq!(
            view.image_diagnostics(),
            ["image \"pic.png\": declared dimensions exceed the image size limits"]
        );
    }

    #[test]
    fn corrupt_image_bytes_are_diagnosed_as_decode_failures() {
        let mut view = VelquView::new();
        view.load_html("<html><body><img src=pic.png></body></html>")
            .unwrap();
        view.set_asset_resolver(Rc::new(ImageRecordingResolver {
            seen: RefCell::new(Vec::new()),
            bytes: {
                let mut bytes = red_png();
                bytes.truncate(20); // valid magic, broken body
                bytes
            },
        }));
        let vp = Viewport::try_new(400, 300, 1.0).unwrap();
        view.layout_facts(vp).unwrap();
        assert_eq!(
            view.image_diagnostics(),
            ["image \"pic.png\": image data is corrupt or truncated"]
        );
    }

    #[test]
    fn scroll_offset_is_runtime_state_without_new_layout() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\">\
             <div style=\"height: 300px; background-color: #ef4444\"></div>\
             <div style=\"height: 300px; background-color: #3b82f6\"></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(200, 200, 1.0).unwrap();
        let facts_before = view.layout_facts(vp).unwrap();
        let a = view.render(vp).unwrap();
        let passes_after_layout = view.layout_stats().passes;
        assert_eq!(facts_before.document_scroll_height, 600.0);

        // Scrolling is a state change only: no layout pass runs, and the
        // facts (unscrolled layout truth) are identical afterwards...
        view.set_scroll_offset(None, 0.0, 250.0).unwrap();
        assert_eq!(view.layout_stats().passes, passes_after_layout);
        let facts_after = view.layout_facts(vp).unwrap();
        assert_eq!(facts_before, facts_after);
        // ...but the raster moved (at 250: 50px red, then blue).
        let b = view.render(vp).unwrap();
        assert_ne!(a.frame.sha256_hex(), b.frame.sha256_hex());

        // Offsets are pure presentation state: scrolling back to 0
        // reproduces frame A byte-for-byte.
        view.set_scroll_offset(None, 0.0, 0.0).unwrap();
        let again = view.render(vp).unwrap();
        assert_eq!(a.frame.sha256_hex(), again.frame.sha256_hex());

        // Clamping is central: over-scroll lands exactly on max scroll.
        view.set_scroll_offset(None, 0.0, 400.0).unwrap();
        let at_max = view.render(vp).unwrap();
        view.set_scroll_offset(None, 0.0, 100_000.0).unwrap();
        let clamped = view.render(vp).unwrap();
        assert_eq!(
            at_max.frame.sha256_hex(),
            clamped.frame.sha256_hex(),
            "400 and 100000 clamp to the same maximum offset"
        );
        // Unknown targets never apply; non-finite offsets are rejected.
        view.set_scroll_offset(Some("missing-pane"), 10.0, 10.0)
            .unwrap();
        assert!(matches!(
            view.set_scroll_offset(None, f32::NAN, 0.0),
            Err(VelquError::InvalidScrollOffset { .. })
        ));
    }

    #[test]
    fn layout_stats_count_passes_and_boxes() {
        let mut view = tiny_view();
        assert_eq!(view.layout_stats().passes, 0);
        let vp = Viewport::try_new(200, 200, 1.0).unwrap();
        view.layout_facts(vp).unwrap();
        let after_one = view.layout_stats();
        assert_eq!(after_one.passes, 1);
        assert!(after_one.nodes_last_pass > 0, "boxes were laid out");
        view.render(vp).unwrap();
        assert_eq!(view.layout_stats().passes, 2);
        // Durations are indicative only; just sanity-check they're recorded
        // (any Duration, including zero on fast machines, is valid).
        let _ = after_one.duration_last_pass;
    }
}
