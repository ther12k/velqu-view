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
//! # Milestone status
//!
//! * **M1–M2c (done):** document/stylesheet sources with identity, cascade,
//!   box tree, Taffy-backed block/flex/grid layout, deterministic text and
//!   images, display list, paint-side scrolling, and pixel-hash fixtures
//!   (`docs/decisions/0005`–`0008`).
//! * **M3 (current, phase 1 complete):** the Tailwind-compatible utility
//!   pipeline ([ADR 0009](docs/decisions/0009-m3-tailwind-pipeline.md)) —
//!   `enable_tailwind()` compiles the document's utility classes into a
//!   generated stylesheet, so plain HTML with Tailwind classes renders with
//!   no CSS files, no node, and no network.
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
mod input;
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
pub use input::{Event, HitTarget};
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
    /// Tailwind utility pipeline (ADR 0009): when enabled, the `class`
    /// attributes of the loaded document are compiled into a generated
    /// stylesheet appended after the author sheets in cascade order.
    tailwind_enabled: bool,
    tailwind_css: Option<String>,
    tailwind_diagnostics_list: Vec<String>,
    /// `<style>` block text extracted from the document, in order.
    style_blocks: Vec<String>,
    // Input gate state (M4a, ADR 0010). Interaction state is runtime
    // presentation state — it never changes layout facts.
    /// The most recent layout pass, kept for hit testing. Refreshed by
    /// every render/layout_facts call; input methods use it read-only.
    last_laid: Option<layout::LaidOutDocument>,
    last_viewport: Option<Viewport>,
    /// Element `id` under the pointer (hover), if any element with an id.
    hover: Option<String>,
    /// Focused element `id`, if any.
    focus: Option<String>,
    /// Element `id` of the pointer press target (click tracking).
    pressed: Option<Option<String>>,
    /// Interaction events since the last [`VelquView::take_events`].
    events: Vec<Event>,
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
            tailwind_enabled: false,
            tailwind_css: None,
            tailwind_diagnostics_list: Vec::new(),
            style_blocks: Vec::new(),
            last_laid: None,
            last_viewport: None,
            hover: None,
            focus: None,
            pressed: None,
            events: Vec::new(),
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

    /// Enables the Tailwind utility pipeline (ADR 0009): the document's
    /// `class` attributes are compiled by [`velqu_tailwind`] into a
    /// generated stylesheet that participates in the cascade after every
    /// author sheet (utilities beat element styles; inline styles and later
    /// sheets still win as CSS specifies). Unsupported classes are reported
    /// through [`VelquView::tailwind_diagnostics`] — never dropped
    /// silently.
    pub fn enable_tailwind(&mut self) {
        self.tailwind_enabled = true;
        self.rebuild_tailwind();
    }

    /// Whether the Tailwind utility pipeline is enabled.
    pub fn tailwind_enabled(&self) -> bool {
        self.tailwind_enabled
    }

    /// Deterministic diagnostics for utility classes the pipeline could not
    /// compile, in first-use order (ADR 0009).
    pub fn tailwind_diagnostics(&self) -> Vec<String> {
        self.tailwind_diagnostics_list.clone()
    }

    /// Re-compiles the utility classes of the loaded document. Runs on
    /// [`VelquView::enable_tailwind`] and on every document load while
    /// enabled; output is a deterministic function of the class list.
    fn rebuild_tailwind(&mut self) {
        self.tailwind_css = None;
        self.tailwind_diagnostics_list.clear();
        if !self.tailwind_enabled {
            return;
        }
        let mut classes: Vec<String> = Vec::new();
        self.dom.walk(|_id, node| {
            if let dom::NodeData::Element { attrs, .. } = &node.data {
                if let Some(class_attr) = attrs
                    .iter()
                    .find(|a| a.name == "class")
                    .map(|a| a.value.clone())
                {
                    for class in class_attr.split_ascii_whitespace() {
                        if !classes.iter().any(|existing| existing == class) {
                            classes.push(class.to_owned());
                        }
                    }
                }
            }
        });
        if classes.is_empty() {
            return;
        }
        let refs: Vec<&str> = classes.iter().map(String::as_str).collect();
        let build = velqu_tailwind::compile_utilities(&refs);
        self.tailwind_diagnostics_list = build
            .diagnostics
            .iter()
            .map(|d| format!("class {:?}: {}", d.class, d.message))
            .collect();
        if build.rules > 0 {
            self.tailwind_css = Some(build.css);
        }
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
        // invalidates every decoded asset — and the input state belongs to
        // the old tree (M4a).
        self.images.clear();
        self.last_laid = None;
        self.hover = None;
        self.focus = None;
        self.pressed = None;
        self.events.clear();
        // `<style>` blocks travel with the document (review fix: they were
        // silently dropped before M3's review).
        self.collect_style_blocks();
        // Utility classes are per-document too: recompile when enabled.
        self.rebuild_tailwind();
        Ok(())
    }

    /// Extracts `<style>` element text from the parsed DOM, in document
    /// order. These are author sheets: they sort after host-installed
    /// stylesheets and override the generated utilities at equal
    /// specificity (ADR 0009).
    fn collect_style_blocks(&mut self) {
        self.style_blocks.clear();
        let mut blocks: Vec<String> = Vec::new();
        self.dom.walk(|id, _| {
            if self.dom.tag_name(id) == Some("style") {
                let text = self.dom.descendant_text(id);
                if !text.trim().is_empty() {
                    blocks.push(text);
                }
            }
        });
        self.style_blocks = blocks;
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
        let mut parsed_author = Vec::with_capacity(self.parsed_css.len() + 1);
        // The generated utility sheet participates FIRST among author
        // sheets: utilities still beat element rules via class specificity,
        // while author class rules override utilities at equal specificity —
        // Tailwind's layered-utilities semantics, where the author's own CSS
        // wins (ADR 0009).
        if let Some(text) = &self.tailwind_css {
            let source = StylesheetSource::new("velqu:tailwind", text.clone());
            let parsed = css::parse(&source, order);
            order += parsed.rules.len() as u32;
            parsed_author.push(parsed);
        }
        for sheet in &self.stylesheets {
            let parsed = css::parse(sheet, order);
            order += parsed.rules.len() as u32;
            parsed_author.push(parsed);
        }
        for (i, block) in self.style_blocks.iter().enumerate() {
            let source = StylesheetSource::new(format!("velqu:style-{i}"), block.clone());
            let parsed = css::parse(&source, order);
            order += parsed.rules.len() as u32;
            parsed_author.push(parsed);
        }

        let mut cascade = style::Cascade::new(&ua_parsed.rules, &parsed_author);
        self.run_layout(viewport, &mut cascade);
        if self.last_laid.is_none() {
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
        }

        let background = document_background(&parsed_author);
        // Disjoint-field borrows: the laid-out display list (immutable)
        // and the font store (mutable) never overlap.
        let display_list = &self.last_laid.as_ref().expect("checked above").display_list;
        let (frame, items, glyphs) =
            painter::paint_document(display_list, background, viewport, &mut self.fonts)?;
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
    fn run_layout(&mut self, viewport: Viewport, cascade: &mut style::Cascade<'_>) {
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
        // Keep the freshest layout for input hit testing (M4a). Callers
        // read `self.last_laid` (disjoint-field borrows keep painting
        // clone-free).
        self.last_viewport = Some(viewport);
        self.last_laid = laid;
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
        let mut parsed_author = Vec::with_capacity(self.parsed_css.len() + 1);
        // Generated utility sheet first among author sheets (ADR 0009):
        // author class rules override utilities at equal specificity.
        if let Some(text) = &self.tailwind_css {
            let source = StylesheetSource::new("velqu:tailwind", text.clone());
            let parsed = css::parse(&source, order);
            order += parsed.rules.len() as u32;
            parsed_author.push(parsed);
        }
        for sheet in &self.stylesheets {
            let parsed = css::parse(sheet, order);
            order += parsed.rules.len() as u32;
            parsed_author.push(parsed);
        }
        for (i, block) in self.style_blocks.iter().enumerate() {
            let source = StylesheetSource::new(format!("velqu:style-{i}"), block.clone());
            let parsed = css::parse(&source, order);
            order += parsed.rules.len() as u32;
            parsed_author.push(parsed);
        }
        let mut cascade = style::Cascade::new(&ua_parsed.rules, &parsed_author);
        self.run_layout(viewport, &mut cascade);
        let Some(laid) = self.last_laid.as_ref() else {
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
        // Cached geometry has stale applied offsets; the next render
        // re-applies them with zero layout passes.
        self.last_laid = None;
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

    // -- input gate (M4a, ADR 0010) ---------------------------------------

    /// Validates that the cached layout matches `viewport` (the input
    /// geometry must correspond to the point space callers use).
    fn cached_layout(&self, viewport: Viewport) -> Option<&layout::LaidOutDocument> {
        let laid = self.last_laid.as_ref()?;
        let cached = self.last_viewport?;
        if cached.width() == viewport.width()
            && cached.height() == viewport.height()
            && cached.scale_factor() == viewport.scale_factor()
        {
            Some(laid)
        } else {
            None
        }
    }

    /// Hit tests the most recent layout at `(x, y)` — **viewport** device
    /// pixels, same space as frame pixels — returning the topmost element
    /// under the point, or `None` when nothing is there or no layout is
    /// cached for this viewport (render first).
    ///
    /// Honors paint order (later siblings on top), clip scopes, and
    /// per-container scroll offsets; the point never triggers layout.
    pub fn hit_test(&self, viewport: Viewport, x: f32, y: f32) -> Option<HitTarget> {
        let laid = self.cached_layout(viewport)?;
        let node = input::hit_at(&laid.root, laid.root_offset, x, y)?;
        Some(HitTarget {
            element_id: node.element_id.clone(),
            tag: node.tag.clone(),
            scroll_container: node.style.overflow_y.is_scroll_container(),
        })
    }

    /// Moves the pointer to `(x, y)` (viewport device px): updates hover
    /// and emits [`Event::PointerLeave`]/[`Event::PointerEnter`] when the
    /// hovered element id changed.
    pub fn pointer_move(&mut self, viewport: Viewport, x: f32, y: f32) {
        let hit = self.hit_test(viewport, x, y);
        let new_hover = hit.and_then(|target| target.element_id);
        if new_hover == self.hover {
            return;
        }
        if let Some(old) = self.hover.take() {
            self.events.push(Event::PointerLeave { element: Some(old) });
        }
        if let Some(new) = new_hover.clone() {
            self.events.push(Event::PointerEnter { element: Some(new) });
        }
        self.hover = new_hover;
    }

    /// Presses at `(x, y)` (viewport device px). Remembers the press
    /// target for click tracking; a later [`VelquView::pointer_release`]
    /// over the same element emits [`Event::Click`].
    pub fn pointer_press(&mut self, viewport: Viewport, x: f32, y: f32) {
        let hit = self.hit_test(viewport, x, y);
        self.pressed = Some(hit.and_then(|target| target.element_id));
    }

    /// Releases at `(x, y)` (viewport device px). If the press and release
    /// hit the same element, emits [`Event::Click`]; elements with an `id`
    /// also take focus on click.
    pub fn pointer_release(&mut self, viewport: Viewport, x: f32, y: f32) {
        let Some(pressed) = self.pressed.take() else {
            return;
        };
        let hit = self.hit_test(viewport, x, y);
        let released = hit.and_then(|target| target.element_id);
        if pressed.is_some() && pressed == released {
            self.events.push(Event::Click {
                element: released.clone(),
            });
            if let Some(id) = released {
                self.set_focus_impl(Some(id));
            }
        }
    }

    /// Scrolls the wheel at `(x, y)` (viewport device px): the gesture
    /// scrolls the nearest scrollable ancestor of the element under the
    /// pointer, or the document-level scroller when none is found.
    ///
    /// `dx`/`dy` are the wheel delta in **device px**, browser-signed:
    /// positive dy = view moves down (a standard wheel-down scrolls
    /// forward), positive dx = view moves right. Platform shells convert
    /// their native deltas (winit's are opposite-signed; line deltas
    /// scale by a line height). Offsets are clamped centrally like
    /// [`VelquView::set_scroll_offset`]; a change emits
    /// [`Event::Scrolled`] and is baked into the cached layout, so
    /// consecutive events accumulate between frames and the next render
    /// paints the new offset with **zero** additional layout passes.
    pub fn wheel(&mut self, viewport: Viewport, x: f32, y: f32, dx: f32, dy: f32) {
        let Some(laid) = self.cached_layout(viewport) else {
            return;
        };
        let ctx = input::WheelContext {
            root: &laid.root,
            document_offset: laid.root_offset,
            document_extent: laid.document_scroll,
            viewport,
        };
        let Some((target, new_x, new_y)) = input::wheel_target(&ctx, x, y, dx, dy) else {
            return;
        };
        let changed = match self.scroll_offset_of(target.as_deref()) {
            Some((current_x, current_y)) => {
                (current_x - new_x).abs() > f32::EPSILON || (current_y - new_y).abs() > f32::EPSILON
            }
            None => (new_x.abs() + new_y.abs()) > f32::EPSILON,
        };
        if !changed {
            return;
        }
        let key = target.clone().unwrap_or_default();
        match self
            .scroll_offsets
            .iter_mut()
            .find(|(existing, _)| *existing == key)
        {
            Some(slot) => slot.1 = (new_x, new_y),
            None => self.scroll_offsets.push((key, (new_x, new_y))),
        }
        // Bake the clamped offset into the cached tree so consecutive
        // wheel events (a real pointer delivers many between frames)
        // accumulate and hit tests stay coherent — no invalidation, no
        // relayout; the next render reproduces the same values from the
        // stored offsets.
        if let Some(laid) = self.last_laid.as_mut() {
            match &target {
                Some(id) => {
                    bake_scroll(&mut laid.root, id, (new_x, new_y));
                }
                None => laid.root_offset = (new_x, new_y),
            }
        }
        self.events.push(Event::Scrolled {
            target,
            x: new_x,
            y: new_y,
        });
    }

    /// The stored (raw) offset for a scroll key; `None` when never set.
    fn scroll_offset_of(&self, key: Option<&str>) -> Option<(f32, f32)> {
        let key = key.unwrap_or("");
        self.scroll_offsets
            .iter()
            .find(|(existing, _)| existing == key)
            .map(|(_, offset)| *offset)
    }

    /// Moves focus to the next element with an `id` in document order,
    /// wrapping around (Tab semantics). Emits
    /// [`Event::FocusChanged`] when focus moved.
    pub fn focus_next(&mut self) {
        let mut ids: Vec<String> = Vec::new();
        self.dom.walk(|id, node| {
            if let dom::NodeData::Element { attrs, .. } = &node.data {
                if let Some(value) = attrs
                    .iter()
                    .find(|a| a.name == "id")
                    .map(|a| a.value.clone())
                {
                    if !ids.contains(&value) {
                        ids.push(value);
                    }
                }
            }
            let _ = id;
        });
        let next: Option<String> = match &self.focus {
            Some(current) => match ids.iter().position(|id| id == current) {
                Some(index) => ids
                    .get((index + 1) % ids.len().max(1))
                    .cloned()
                    .or_else(|| Some(current.clone())),
                None => ids.first().cloned(),
            },
            None => ids.first().cloned(),
        };
        self.set_focus_impl(next);
    }

    /// Sets focus directly (`None` clears it) and emits
    /// [`Event::FocusChanged`] when it moved.
    pub fn set_focus(&mut self, element: Option<&str>) {
        self.set_focus_impl(element.map(str::to_owned));
    }

    fn set_focus_impl(&mut self, to: Option<String>) {
        if to == self.focus {
            return;
        }
        let from = self.focus.take();
        self.focus = to.clone();
        self.events.push(Event::FocusChanged { from, to });
    }

    /// Reports the pointer leaving the window: clears hover and emits
    /// [`Event::PointerLeave`] when an element was hovered.
    pub fn pointer_exit(&mut self) {
        if let Some(old) = self.hover.take() {
            self.events.push(Event::PointerLeave { element: Some(old) });
        }
    }

    /// The currently focused element id, if any.
    pub fn focused(&self) -> Option<&str> {
        self.focus.as_deref()
    }

    /// The element id currently under the pointer (hover), if any.
    pub fn hovered(&self) -> Option<&str> {
        self.hover.as_deref()
    }

    /// Drains the interaction events accumulated since the last call, in
    /// the order they occurred.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

/// Bakes a clamped scroll offset into the cached box tree so input stays
/// coherent between renders (ADR 0010): the container's applied scroll
/// updates in place, mirroring what the next render's
/// `apply_scroll_offsets` will compute from the stored request. Returns
/// once the id is found.
fn bake_scroll(node: &mut layout::BoxNode, id: &str, offset: (f32, f32)) -> bool {
    if node.element_id.as_deref() == Some(id) {
        node.applied_scroll = offset;
        return true;
    }
    node.children
        .iter_mut()
        .any(|child| bake_scroll(child, id, offset))
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

    // -- M4a input gate (ADR 0010) ----------------------------------------

    /// A two-pane document: pane A (id "a", scrollable, with a wide child
    /// and a late sibling that paints on top of its left edge), then pane B
    /// (id "b") stacked below.
    fn input_view() -> VelquView {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\">\
             <div id=a style=\"overflow: auto; width: 200px; height: 100px\">\
             <div style=\"width: 400px; height: 60px; background-color: #ef4444\"></div>\
             <div style=\"width: 50px; height: 40px; margin: 0; background-color: #22c55e\"></div>\
             </div>\
             <div id=b style=\"width: 200px; height: 100px; background-color: #3b82f6\"></div>\
             </body></html>",
        )
        .unwrap();
        view
    }

    #[test]
    fn hit_test_reports_topmost_and_clip_scoped_elements() {
        let mut view = input_view();
        let vp = Viewport::try_new(300, 300, 1.0).unwrap();
        view.render(vp).unwrap();

        // Inside pane A over its wide red child: the child paints above
        // pane A's own background, so the topmost box is the child
        // itself (no id) — not the scroll container behind it.
        let over_red = view.hit_test(vp, 100.0, 30.0).expect("hit");
        assert_eq!(over_red.tag, "div");
        assert!(!over_red.scroll_container);

        // Over the green sibling (later in pane A's paint order).
        let over_green = view.hit_test(vp, 25.0, 80.0).expect("hit");
        assert_eq!(over_green.tag, "div");

        // Clipped away: (250, 30) is inside the red child's own geometry
        // (content x=250 < 400) but outside pane A's 200px clip — the
        // walk must not descend into the clipped subtree.
        let clipped_out = view.hit_test(vp, 250.0, 30.0).expect("over the canvas");
        assert!(
            clipped_out.element_id.is_none(),
            "clipped-away content must not be hit"
        );

        // Pane B: plain background hit.
        let over_b = view.hit_test(vp, 50.0, 150.0).expect("hit inside b");
        assert_eq!(over_b.element_id.as_deref(), Some("b"));
        assert!(!over_b.scroll_container);

        // Outside every content box (past the 200px document): nothing
        // identifiable is under the pointer.
        let past = view.hit_test(vp, 290.0, 290.0);
        assert!(past.is_none_or(|t| t.element_id.is_none()));
    }

    #[test]
    fn hit_test_honors_scrolled_offsets() {
        let mut view = input_view();
        // Scroll pane a right by 150: the green sibling (at content x=0)
        // moves under viewport x=-150..-100 — off-clip; the point that was
        // over green now shows the red child.
        view.set_scroll_offset(Some("a"), 150.0, 0.0).unwrap();
        let vp = Viewport::try_new(300, 300, 1.0).unwrap();
        view.render(vp).unwrap();

        // Content x=150+60=210 → viewport x=60: red child area.
        let target = view.hit_test(vp, 60.0, 30.0).expect("hit");
        assert!(!target.scroll_container);
        // Green moved left out of the clip: (210, 80) viewport now lands
        // on content x=360 — still inside the red child (400 wide).
        let still_red = view.hit_test(vp, 210.0, 30.0).expect("hit");
        assert!(!still_red.scroll_container);
    }

    #[test]
    fn pointer_events_track_hover_and_click() {
        let mut view = input_view();
        let vp = Viewport::try_new(300, 300, 1.0).unwrap();
        view.render(vp).unwrap();

        view.pointer_move(vp, 50.0, 150.0); // over pane b
        let events = view.take_events();
        assert_eq!(
            events,
            vec![Event::PointerEnter {
                element: Some("b".into())
            }]
        );

        view.pointer_move(vp, 60.0, 150.0); // still over b, same id: no events
        assert!(view.take_events().is_empty());

        view.pointer_move(vp, 500.0, 500.0); // off-document: leave
        let events = view.take_events();
        assert_eq!(
            events,
            vec![Event::PointerLeave {
                element: Some("b".into())
            }]
        );

        // Press on b, release on b: click + focus (b has an id).
        view.pointer_press(vp, 50.0, 150.0);
        view.pointer_release(vp, 50.0, 150.0);
        let events = view.take_events();
        assert!(events.contains(&Event::Click {
            element: Some("b".into())
        }));
        assert!(events.contains(&Event::FocusChanged {
            from: None,
            to: Some("b".into())
        }));
        assert_eq!(view.focused(), Some("b"));

        // Press on b, release elsewhere: no click (and no focus change).
        view.pointer_press(vp, 50.0, 150.0);
        view.pointer_release(vp, 500.0, 500.0);
        let events = view.take_events();
        assert!(!events.iter().any(|e| matches!(e, Event::Click { .. })));
        assert_eq!(view.focused(), Some("b"));
    }

    #[test]
    fn wheel_scrolls_nearest_container_with_zero_layout() {
        let mut view = input_view();
        // Viewport shorter than the 200px document so the document-level
        // scroller has range (extent 200 − scrollport 150 = 50 ≥ one 40px
        // wheel step).
        let vp = Viewport::try_new(300, 150, 1.0).unwrap();
        view.render(vp).unwrap();
        let passes_before = view.layout_stats().passes;

        // Wheel over pane b — no scrollable ancestor → document scroller.
        // One wheel notch = 40 device px (the shell's line-height).
        view.wheel(vp, 50.0, 150.0, 0.0, 40.0);
        let events = view.take_events();
        assert_eq!(
            events,
            vec![Event::Scrolled {
                target: None,
                x: 0.0,
                y: 40.0
            }]
        );
        // No layout pass ran for the wheel itself.
        assert_eq!(view.layout_stats().passes, passes_before);
        // The next render paints the new offset — still zero new passes
        // beyond the frame's own single pass.
        view.render(vp).unwrap();
        assert_eq!(view.layout_stats().passes, passes_before + 1);

        // Wheel over the scrollable pane a: the pane scrolls, not the page.
        view.render(vp).unwrap(); // paint the document scroll (one pass)
        view.pointer_move(vp, 100.0, 50.0); // hover tracking for realism
        let _ = view.take_events();
        view.wheel(vp, 100.0, 50.0, 40.0, 0.0);
        let events = view.take_events();
        assert_eq!(
            events,
            vec![Event::Scrolled {
                target: Some("a".into()),
                x: 40.0,
                y: 0.0
            }]
        );

        // Over-scroll clamps to the extent (pane a content 400 wide,
        // scrollport 200 → max 200; 40 per tick, so 6+ ticks clamp).
        for _ in 0..10 {
            view.render(vp).unwrap();
            view.wheel(vp, 100.0, 50.0, 40.0, 0.0);
            let _ = view.take_events();
        }
        view.render(vp).unwrap();
        let facts = view.layout_facts(vp).unwrap();
        // Facts stay the unscrolled truth regardless of all that scrolling.
        assert_eq!(facts.viewport_width, 300);
    }

    #[test]
    fn focus_cycles_and_reports() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body>\
             <div id=first></div><div><div id=second></div></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(200, 200, 1.0).unwrap();
        view.render(vp).unwrap();

        assert_eq!(view.focused(), None);
        view.focus_next();
        assert_eq!(view.focused(), Some("first"));
        view.focus_next();
        assert_eq!(view.focused(), Some("second"));
        view.focus_next();
        assert_eq!(view.focused(), Some("first"), "wraps around");

        let events = view.take_events();
        assert_eq!(
            events,
            vec![
                Event::FocusChanged {
                    from: None,
                    to: Some("first".into())
                },
                Event::FocusChanged {
                    from: Some("first".into()),
                    to: Some("second".into())
                },
                Event::FocusChanged {
                    from: Some("second".into()),
                    to: Some("first".into())
                },
            ]
        );

        // Direct set with no change emits nothing.
        view.set_focus(Some("first"));
        assert!(view.take_events().is_empty());
        view.set_focus(None);
        assert_eq!(view.take_events().len(), 1);
    }

    #[test]
    fn input_state_resets_with_the_document() {
        let mut view = input_view();
        let vp = Viewport::try_new(300, 300, 1.0).unwrap();
        view.render(vp).unwrap();
        view.pointer_move(vp, 50.0, 150.0);
        view.pointer_press(vp, 50.0, 150.0);
        let _ = view.take_events();

        view.load_html("<!doctype html><html><body>fresh</body></html>")
            .unwrap();
        assert_eq!(view.hovered(), None);
        assert_eq!(view.focused(), None);
        // Stale cache is gone: hit testing without a render finds nothing.
        assert!(view.hit_test(vp, 50.0, 150.0).is_none());
        assert!(view.take_events().is_empty());
    }

    // -- M3 Tailwind utility pipeline (ADR 0009) --------------------------

    #[test]
    fn tailwind_utilities_reach_layout_and_paint() {
        let mut view = VelquView::new();
        view.enable_tailwind();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\">\
             <div data-vv-test=card class=\"bg-rose-600 w-64 h-20 p-4\"></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(320, 200, 1.0).unwrap();
        let facts = view.layout_facts(vp).unwrap();
        let card = fact_of(&facts, "card");
        // w-64 = 16rem = 256px; h-20 = 80px; classes compose.
        assert_eq!((card.width, card.height), (256.0, 80.0));
        assert!(view.tailwind_diagnostics().is_empty());
        let result = view.render(vp).unwrap();
        let rose = Color::from_hex("#e11d48").unwrap();
        assert_eq!(result.frame.pixel(10, 10), Some(rose), "bg-rose-600 paints");
    }

    #[test]
    fn tailwind_utilities_lose_to_inline_but_beat_element_rules() {
        let mut view = VelquView::new();
        view.enable_tailwind();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\">\
             <style>div { width: 100px }</style>\
             <div data-vv-test=a class=\"w-32 h-8\"></div>\
             <div data-vv-test=b class=\"w-32 h-8\" style=\"width: 40px\"></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(320, 200, 1.0).unwrap();
        let facts = view.layout_facts(vp).unwrap();
        // The utility (class specificity) beats the element rule…
        assert_eq!(fact_of(&facts, "a").width, 128.0);
        // …and an inline style beats the utility, as CSS specifies.
        assert_eq!(fact_of(&facts, "b").width, 40.0);
    }

    #[test]
    fn tailwind_author_css_overrides_utilities() {
        // Tailwind's layered-utilities semantics: the author's own CSS
        // beats a utility at equal specificity (ADR 0009, review
        // correction — the first implementation had this backwards).
        let mut view = VelquView::new();
        view.enable_tailwind();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\">\
             <style>.card { width: 96px }</style>\
             <div data-vv-test=card class=\"card w-32 h-8\"></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(320, 200, 1.0).unwrap();
        let facts = view.layout_facts(vp).unwrap();
        // .card wins the tie against .w-32 (author sheet sorts after the
        // generated one), while .h-8 still applies unopposed.
        assert_eq!(fact_of(&facts, "card").width, 96.0);
        assert_eq!(fact_of(&facts, "card").height, 32.0);
    }

    #[test]
    fn tailwind_whitespace_utilities_map_into_the_profile() {
        // white-space is a supported renderer property; the utilities must
        // compile (review finding: they were missing from v0).
        let mut view = VelquView::new();
        view.enable_tailwind();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\">\
             <p data-vv-test=p class=\"w-40 whitespace-nowrap\">\
             aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk</p>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(320, 200, 1.0).unwrap();
        let facts = view.layout_facts(vp).unwrap();
        // nowrap keeps everything on one line that would otherwise wrap.
        assert_eq!(
            fact_of(&facts, "p").text_runs.len(),
            1,
            "{:?}",
            fact_of(&facts, "p").text_runs
        );
        assert!(view.tailwind_diagnostics().is_empty());
    }

    #[test]
    fn tailwind_is_opt_in_and_reports_unsupported_classes() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\">\
             <div data-vv-test=a class=\"w-64 h-8 shadow-md hover:flex\"></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(400, 200, 1.0).unwrap();
        // Without opt-in, utilities do nothing and produce no diagnostics.
        let facts = view.layout_facts(vp).unwrap();
        assert_eq!(fact_of(&facts, "a").width, 400.0, "block fills the body");
        assert!(view.tailwind_diagnostics().is_empty());

        view.enable_tailwind();
        let facts = view.layout_facts(vp).unwrap();
        assert_eq!(fact_of(&facts, "a").width, 256.0, "w-64 now applies");
        let diagnostics = view.tailwind_diagnostics();
        assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
        assert!(diagnostics[0].starts_with("class \"shadow-md\":"));
        assert!(diagnostics[1].starts_with("class \"hover:flex\":"));
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
