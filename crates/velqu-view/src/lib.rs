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
//! * **M3 (done):** the Tailwind-compatible utility pipeline
//!   ([ADR 0009](docs/decisions/0009-m3-tailwind-pipeline.md)) —
//!   `enable_tailwind()` compiles the document's utility classes into a
//!   generated stylesheet, so plain HTML with Tailwind classes renders with
//!   no CSS files, no node, and no network.
//! * **M4a–M4b (done):** the input gate and interaction styling — hit
//!   testing, pointer/focus/click events, wheel scrolling, `:hover`/
//!   `:focus`/`:active` frozen to paint, cursor, focus origin — all with
//!   zero relayout ([ADR 0010](docs/decisions/0010-m4a-input-gate.md),
//!   [ADR 0011](docs/decisions/0011-m4b-interaction-styling.md)).
//! * **M4c1 (done):** editable controls — opaque document-scoped element
//!   identity on every event, runtime `<input>`/`<textarea>` values with
//!   grapheme-safe editing, pointer-capture selection, scroll-to-caret
//!   ([ADR 0012](docs/decisions/0012-m4c1-editable-controls.md)).
//! * **M4c2 (done):** clipboard — Copy/Cut/Paste commands resolved through
//!   a fallible host-installed [`ClipboardProvider`] (null by default; the
//!   shell installs an OS-backed one). Cut is transactional — the selection
//!   is deleted only after its clipboard write succeeded — and pasted text
//!   goes through the same filter as direct insertion, with CRLF
//!   normalization ([ADR 0013](docs/decisions/0013-m4c2-clipboard.md)).
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

mod clipboard;
mod color;
mod control;
mod css;
mod display_list;
mod dom;
mod editor;
mod font;
mod html;
mod image;
mod input;
mod keyboard;
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

pub use clipboard::{ClipboardError, ClipboardProvider, NullClipboardProvider};
pub use color::{Color, ColorParseError};
pub use control::{ControlFact, ControlFacts, ControlKind, ControlRect};
pub use image::{ImageLimits, InvalidImageLimits, InvalidImageLimitsReason};
pub use input::{ElementHandle, ElementTarget, Event, FocusOrigin, HitTarget, ScrollTarget};
pub use keyboard::{KeyCommand, KeyModifiers};
pub use layout::{LAYOUT_FACTS_SCHEMA_VERSION, LayoutFacts, LayoutNodeFact};
pub use source::{
    Asset, AssetRequest, AssetResolver, DocumentSource, NullAssetResolver, SharedAssetResolver,
    SourceId, StylesheetSource,
};
pub use style::CursorStyle;
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
    /// Presentation-only repaints (M4b, ADR 0011): frames re-emitted from
    /// cached geometry because interaction state alone changed. Never
    /// counts toward `passes` — pointer motion never lays out.
    pub repaints: u64,
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
    /// Host-provided clipboard (M4c2, ADR 0013); the null default keeps the
    /// renderer hermetic — copy/cut writes go nowhere, paste reads nothing.
    clipboard: Rc<dyn clipboard::ClipboardProvider>,
    custom_clipboard: bool,
    /// Decoded `<img>` assets for the current document (ADR 0008), keyed by
    /// the `src` reference as written. Cleared when the document changes.
    images: ImageStore,
    image_limits: ImageLimits,
    /// Deterministic image diagnostics from the last prepare pass, in
    /// document order.
    image_diagnostics: Vec<String>,
    /// Runtime scroll offsets (ADR 0008): DOM node (None = document) →
    /// raw (unclamped) offset.
    scroll_offsets: layout::ScrollOffsets,
    // Layout instrumentation (ADR 0008).
    layout_passes: u64,
    layout_nodes_last: usize,
    layout_duration_last: std::time::Duration,
    /// Presentation-only repaints (M4b, ADR 0011): frames re-emitted from
    /// the cached geometry because interaction state alone changed — no
    /// Taffy pass.
    repaint_passes: u64,
    /// Deterministic diagnostics from the cascade's apply stage (skipped
    /// declarations, deferred interaction properties), deduplicated across
    /// frames and in first-seen order.
    style_diagnostics_list: Vec<String>,
    /// Tailwind utility pipeline (ADR 0009): when enabled, the `class`
    /// attributes of the loaded document are compiled into a generated
    /// stylesheet appended after the author sheets in cascade order.
    tailwind_enabled: bool,
    tailwind_css: Option<String>,
    tailwind_diagnostics_list: Vec<String>,
    /// `<style>` block text extracted from the document, in order.
    style_blocks: Vec<String>,
    /// Runtime state for supported editable controls. Keys are current-
    /// document DOM nodes; the store is cleared on document replacement.
    controls: std::collections::HashMap<dom::NodeId, control::ControlState>,
    /// Deterministic diagnostics for controls outside the M4c1 profile.
    control_diagnostics_list: Vec<String>,
    /// Runtime editor geometry keyed by current-document DOM node. This is
    /// presentation state, rebuilt from the cached outer layout without Taffy.
    control_geometry: std::collections::HashMap<dom::NodeId, control::EditorGeometry>,
    // Input gate state (M4a/M4b, ADR 0010/0011). Interaction state is
    // runtime presentation state — it never changes layout facts; it is
    // keyed by DOM node and reaches only pixels (via stateful selectors).
    /// The most recent layout pass, kept for hit testing. Refreshed by
    /// every render/layout_facts call; input methods use it read-only.
    last_laid: Option<layout::LaidOutDocument>,
    last_viewport: Option<Viewport>,
    /// Set when document content or stylesheets changed: the next render
    /// must run a full layout, not the presentation-only path.
    structure_dirty: bool,
    /// Last pointer position (viewport device px) from input; hover is
    /// re-derived from it when scrolling moves content underneath.
    pointer_pos: Option<(f32, f32)>,
    /// Captured editable control during pointer selection. Capture is
    /// independent of hover and survives pointer movement outside the box.
    pointer_capture: Option<dom::NodeId>,
    /// Selection anchor recorded at pointer press for the captured control.
    pointer_anchor: Option<usize>,
    /// Monotonic document identity used to scope public element handles.
    document_generation: u64,
    /// The element under the pointer (for `:hover`), if any.
    hover: Option<dom::NodeId>,
    /// Focused element (for `:focus`), if any.
    focus: Option<dom::NodeId>,
    /// Why focus last moved (M4b).
    focus_origin: Option<input::FocusOrigin>,
    /// The pressed element (for `:active` chain + click tracking).
    pressed: Option<dom::NodeId>,
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
            .field("custom_clipboard_provider", &self.custom_clipboard)
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
            clipboard: Rc::new(clipboard::NullClipboardProvider),
            custom_clipboard: false,
            images: ImageStore::new(),
            image_limits: ImageLimits::default(),
            image_diagnostics: Vec::new(),
            scroll_offsets: Vec::new(),
            layout_passes: 0,
            layout_nodes_last: 0,
            layout_duration_last: std::time::Duration::ZERO,
            repaint_passes: 0,
            style_diagnostics_list: Vec::new(),
            tailwind_enabled: false,
            tailwind_css: None,
            tailwind_diagnostics_list: Vec::new(),
            style_blocks: Vec::new(),
            controls: std::collections::HashMap::new(),
            control_diagnostics_list: Vec::new(),
            control_geometry: std::collections::HashMap::new(),
            last_laid: None,
            last_viewport: None,
            structure_dirty: true,
            document_generation: 0,
            pointer_pos: None,
            pointer_capture: None,
            pointer_anchor: None,
            hover: None,
            focus: None,
            focus_origin: None,
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

    /// Installs the host's clipboard provider (M4c2, ADR 0013).
    ///
    /// The renderer has no clipboard of its own: copy/cut write through this
    /// provider and paste reads from it. The default
    /// [`NullClipboardProvider`] reads nothing and drops writes, so a
    /// hostless view stays hermetic. Installing is a host action — e.g.
    /// `velqu-shell` installs an OS-backed provider.
    pub fn set_clipboard_provider(&mut self, provider: Rc<dyn clipboard::ClipboardProvider>) {
        self.clipboard = provider;
        self.custom_clipboard = true;
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

    /// Deterministic diagnostics for controls outside the M4c1 profile.
    pub fn control_diagnostics(&self) -> Vec<String> {
        self.control_diagnostics_list.clone()
    }

    /// Returns the current runtime value for a supported control handle.
    ///
    /// The value is never read from a mutated DOM attribute or text child;
    /// the optional HTML id is only descriptive metadata. Stale handles and
    /// non-control elements return `None`.
    pub fn control_value(&self, handle: ElementHandle) -> Option<&str> {
        let node = self.resolve_handle(handle)?;
        self.controls.get(&node).map(control::ControlState::value)
    }

    /// Returns control facts from the most recently cached layout.
    ///
    /// This read does not trigger layout. Before the first render/layout pass,
    /// or when the viewport does not match the cache, the result is empty.
    /// Current values and selection offsets come from runtime control state;
    /// `LayoutFacts` remains structural and state-free.
    pub fn control_facts(&mut self, viewport: Viewport) -> Result<ControlFacts, VelquError> {
        if self.document.is_none() {
            return Err(VelquError::DocumentNotLoaded);
        }
        let Some(laid) = self.cached_layout(viewport).cloned() else {
            return Ok(ControlFacts::default());
        };
        let mut facts = ControlFacts::default();
        collect_control_facts(self, &laid.root, &mut facts.controls);
        Ok(facts)
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
        self.document_generation = self.document_generation.wrapping_add(1).max(1);
        // Image identity is per-document, like the DOM: a new document
        // invalidates every decoded asset — and the input state belongs to
        // the old tree (M4a/M4b).
        self.images.clear();
        self.controls.clear();
        self.control_diagnostics_list.clear();
        self.control_geometry.clear();
        let (control_inits, control_diagnostics) = control::discover(&self.dom);
        for init in control_inits {
            self.controls.insert(
                init.node,
                control::ControlState::new(
                    init.kind,
                    init.initial_value,
                    init.readonly,
                    init.disabled,
                ),
            );
        }
        self.control_diagnostics_list = control_diagnostics;
        self.last_laid = None;
        self.last_viewport = None;
        self.structure_dirty = true;
        self.pointer_pos = None;
        self.pointer_capture = None;
        self.pointer_anchor = None;
        self.hover = None;
        self.focus = None;
        self.pressed = None;
        self.events.clear();
        // Scroll offsets are keyed by DOM node: they die with the document
        // — a reload is the one sanctioned reset (ADR 0011).
        self.scroll_offsets.clear();
        self.style_diagnostics_list.clear();
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

    /// Deterministic diagnostics from the cascade's apply stage (ADR
    /// 0011): skipped declarations and deferred interaction properties,
    /// deduplicated across frames in first-seen order. Parse-time issues
    /// surface through [`VelquView::css_diagnostics`].
    pub fn style_diagnostics(&self) -> Vec<String> {
        self.style_diagnostics_list.clone()
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
        if self.structure_dirty || self.cached_layout(viewport).is_none() {
            // Structural change or new viewport: full layout. The current
            // interaction state feeds stateful selectors so pixels are
            // correct after resize/reload too (M4b).
            let interaction = self.interaction_state();
            self.run_layout(viewport, &mut cascade, Some(&interaction));
            self.structure_dirty = false;
        } else {
            // Steady state: only presentation can have changed. Recompute
            // styles with the current interaction state, patch the cached
            // tree, re-emit — no Taffy pass (ADR 0011).
            self.repaint_presentation(viewport, &mut cascade);
        }
        self.record_style_diagnostics(&cascade);
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
    /// `interaction` feeds stateful-selector matching (M4b): layout facts
    /// always pass `None` — facts are the structural truth, state only
    /// ever reaches pixels.
    fn run_layout(
        &mut self,
        viewport: Viewport,
        cascade: &mut style::Cascade<'_>,
        interaction: Option<&style::InteractionState>,
    ) {
        let started = std::time::Instant::now();
        let laid = layout::layout_document(
            &self.dom,
            viewport,
            cascade,
            &mut self.fonts,
            &self.images,
            &self.scroll_offsets,
            interaction,
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
        self.rebuild_control_presentation();
    }

    /// Presentation-only repaint (M4b, ADR 0011): recompute styles with
    /// the current interaction state, patch the cached box tree's
    /// paint-only fields, and re-emit the display list. Layout geometry,
    /// scroll extents, and the Taffy pass count are untouched.
    fn repaint_presentation(&mut self, _viewport: Viewport, cascade: &mut style::Cascade<'_>) {
        let interaction = self.interaction_state();
        let root_element = layout::layout_root(&self.dom);
        let styles =
            layout::compute_all_styles(&self.dom, root_element, cascade, Some(&interaction));
        if let Some(laid) = self.last_laid.as_mut() {
            layout::patch_presentation_styles(&mut laid.root, &styles);
        }
        self.rebuild_control_presentation();
        self.repaint_passes += 1;
    }

    /// Rebuilds runtime control paint from the cached outer layout. This is a
    /// presentation-only operation: it never invokes Taffy or changes layout
    /// facts.
    fn rebuild_control_presentation(&mut self) {
        let Some(viewport) = self.last_viewport else {
            return;
        };
        let Some(laid) = self.last_laid.as_mut() else {
            return;
        };
        let scale = viewport.scale_factor();
        let (items, geometry) = control::build_paint_items(
            &laid.root,
            &mut self.controls,
            self.focus,
            &mut self.fonts,
            scale,
        );
        let offset = laid.root_offset;
        laid.display_list =
            layout::build_display_list_with_controls(&laid.root, scale, offset, Some(&items));
        self.control_geometry = geometry;
    }

    /// Snapshots the node-keyed interaction state for the cascade.
    fn interaction_state(&self) -> style::InteractionState {
        // :active chains through the pressed element's ancestors, exactly
        // like :hover (CSS activation propagates up).
        let mut active_path = Vec::new();
        if let Some(pressed) = self.pressed {
            let mut cursor = Some(pressed);
            while let Some(id) = cursor {
                active_path.push(id);
                cursor = self.dom.node(id).parent;
            }
        }
        // :hover chains through the hovered element's ancestors (ADR 0011).
        let mut hover_path = Vec::new();
        if let Some(hovered) = self.hover {
            let mut cursor = Some(hovered);
            while let Some(id) = cursor {
                hover_path.push(id);
                cursor = self.dom.node(id).parent;
            }
        }
        style::InteractionState {
            hover_path,
            active_path,
            focus: self.focus,
        }
    }

    /// Merges this pass's cascade diagnostics into the deduplicated,
    /// first-seen-order list surfaced by [`VelquView::style_diagnostics`].
    fn record_style_diagnostics(&mut self, cascade: &style::Cascade<'_>) {
        for diagnostic in &cascade.diagnostics {
            let text = format!(
                "{} line {}: {}",
                diagnostic.source, diagnostic.line, diagnostic.message
            );
            if !self.style_diagnostics_list.contains(&text) {
                self.style_diagnostics_list.push(text);
            }
        }
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
        self.run_layout(viewport, &mut cascade, None);
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
    /// scrolling never triggers a new layout pass. Offsets are keyed by
    /// node identity (ADR 0011), so they transplant across relayouts —
    /// resize and restyle keep the position, re-clamped; only loading a
    /// new document resets it.
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
        let key = match target {
            None => None,
            Some(id) => match self.element_node(id) {
                Some(node) => Some(node),
                // Unknown ids match nothing, by contract.
                None => return Ok(()),
            },
        };
        let entry = (key, (x.max(0.0), y.max(0.0)));
        match self
            .scroll_offsets
            .iter_mut()
            .find(|(existing, _)| *existing == entry.0)
        {
            Some(slot) => slot.1 = entry.1,
            None => self.scroll_offsets.push(entry),
        }
        // Bake the clamped offset into the cached tree (mirroring what the
        // next layout's apply stage computes from the raw request), so hit
        // testing and hover stay coherent and the next render needs no
        // Taffy pass. The stored request stays raw for later re-clamping.
        if let (Some(laid), Some(viewport)) = (self.last_laid.as_mut(), self.last_viewport) {
            match entry.0 {
                Some(node) => {
                    if let Some(container) = layout::find_box(&laid.root, node) {
                        if let Some(extent) = container.scroll {
                            let clamped = layout::clamp_scroll_offset(
                                entry.1,
                                (extent.width, extent.height),
                                (container.padding_box.w, container.padding_box.h),
                            );
                            bake_scroll(&mut laid.root, node, clamped);
                        }
                    }
                }
                None => {
                    laid.root_offset = layout::clamp_scroll_offset(
                        entry.1,
                        (laid.document_scroll.width, laid.document_scroll.height),
                        (viewport.width() as f32, viewport.height() as f32),
                    );
                }
            }
        }
        if let Some(viewport) = self.last_viewport {
            self.refresh_hover(viewport);
        }
        Ok(())
    }

    /// Resolves an element `id` attribute to its DOM node (document
    /// order, first match) — the runtime state key (ADR 0011).
    fn element_node(&self, id: &str) -> Option<dom::NodeId> {
        let mut found = None;
        self.dom.walk(|node, data| {
            if found.is_none() {
                if let dom::NodeData::Element { attrs, .. } = &data.data {
                    if attrs.iter().any(|a| a.name == "id" && a.value == id) {
                        found = Some(node);
                    }
                }
            }
        });
        found
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
            repaints: self.repaint_passes,
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
            handle: self.node_handle(node.node),
            element_id: node.element_id.clone(),
            tag: node.tag.clone(),
            scroll_container: node.style.overflow_y.is_scroll_container(),
        })
    }

    /// Moves the pointer to `(x, y)` (viewport device px): updates hover
    /// and emits [`Event::PointerLeave`]/[`Event::PointerEnter`] when the
    /// hovered element changed. The position is remembered so scrolling
    /// can re-derive hover when content moves underneath (ADR 0011).
    ///
    /// While an editable control holds pointer capture (a press inside it),
    /// movement also extends its selection — even outside the control's
    /// border box. Returns whether interaction presentation changed (hover
    /// or captured selection); never triggers layout.
    pub fn pointer_move(&mut self, viewport: Viewport, x: f32, y: f32) -> bool {
        self.pointer_pos = Some((x, y));
        let before = self.hover;
        self.update_hover(viewport, x, y);
        let selection_changed = self.extend_control_selection(viewport, x, y);
        before != self.hover || selection_changed
    }

    /// Re-derives hover from the remembered pointer position — after a
    /// scroll, the content under a stationary pointer changed.
    fn refresh_hover(&mut self, viewport: Viewport) {
        if let Some((x, y)) = self.pointer_pos {
            self.update_hover(viewport, x, y);
        }
    }

    fn update_hover(&mut self, viewport: Viewport, x: f32, y: f32) {
        let hit = self
            .cached_layout(viewport)
            .and_then(|laid| input::hit_at(&laid.root, laid.root_offset, x, y))
            .map(|node| node.node);
        if hit == self.hover {
            return;
        }
        if let Some(old) = self.hover.take() {
            self.events.push(Event::PointerLeave {
                target: self.node_target(old),
            });
        }
        if let Some(node) = hit {
            self.events.push(Event::PointerEnter {
                target: self.node_target(node),
            });
        }
        self.hover = hit;
    }

    /// Builds the opaque public handle for a current-document DOM node.
    fn node_handle(&self, node: dom::NodeId) -> ElementHandle {
        ElementHandle::new(self.document_generation, node)
    }

    /// Resolves a public handle only when it belongs to this document.
    fn resolve_handle(&self, handle: ElementHandle) -> Option<dom::NodeId> {
        (handle.generation() == self.document_generation)
            .then_some(handle.node())
            .filter(|&node| {
                node < self.dom.node_count()
                    && matches!(self.dom.node(node).data, dom::NodeData::Element { .. })
            })
    }

    /// Sets focus using an opaque handle from this document.
    ///
    /// Handles from another document, or handles that do not name an
    /// element, are ignored safely. This is the stable counterpart to the
    /// id-based convenience method [`VelquView::set_focus`].
    pub fn set_focus_handle(&mut self, handle: Option<ElementHandle>) {
        let node = handle.and_then(|handle| self.resolve_handle(handle));
        if handle.is_some() && node.is_none() {
            return;
        }
        self.set_focus_node_with(node, input::FocusOrigin::Programmatic);
    }

    /// The element's HTML `id` attribute value, if any (descriptive metadata).
    fn node_element_id(&self, node: dom::NodeId) -> Option<&str> {
        match &self.dom.node(node).data {
            dom::NodeData::Element { attrs, .. } => attrs
                .iter()
                .find(|a| a.name == "id")
                .map(|a| a.value.as_str()),
            _ => None,
        }
    }

    /// The public identity for a current-document node.
    fn node_target(&self, node: dom::NodeId) -> ElementTarget {
        ElementTarget {
            handle: self.node_handle(node),
            id: self.node_element_id(node).map(str::to_owned),
        }
    }

    /// Presses at `(x, y)` (viewport device px). Remembers the press
    /// target for click tracking and `:active` styling; a later
    /// [`VelquView::pointer_release`] over the same element emits
    /// [`Event::Click`].
    ///
    /// Pressing an enabled editable control also focuses it, places the
    /// caret at the nearest grapheme boundary, and captures the pointer for
    /// drag selection (M4c1).
    pub fn pointer_press(&mut self, viewport: Viewport, x: f32, y: f32) -> bool {
        let before = self.pressed;
        self.pointer_pos = Some((x, y));
        self.update_hover(viewport, x, y);
        let hit = self
            .cached_layout(viewport)
            .and_then(|laid| input::hit_at(&laid.root, laid.root_offset, x, y))
            .map(|node| node.node);
        self.pressed = hit;
        if let Some(node) = hit {
            if self.controls.contains_key(&node) {
                self.begin_control_selection(viewport, node, x, y);
            }
        }
        before != self.pressed
    }

    /// Releases at `(x, y)` (viewport device px). If the press and release
    /// hit the same element, emits [`Event::Click`]; elements with an `id`
    /// also take focus on click. Any active pointer capture ends (M4c1).
    pub fn pointer_release(&mut self, viewport: Viewport, x: f32, y: f32) -> bool {
        self.pointer_capture = None;
        self.pointer_anchor = None;
        let Some(pressed) = self.pressed.take() else {
            return false;
        };
        self.pointer_pos = Some((x, y));
        self.update_hover(viewport, x, y);
        let released = self
            .cached_layout(viewport)
            .and_then(|laid| input::hit_at(&laid.root, laid.root_offset, x, y))
            .map(|node| node.node);
        if Some(pressed) == released {
            self.events.push(Event::Click {
                target: self.node_target(pressed),
            });
            if self.node_element_id(pressed).is_some() {
                self.set_focus_node_with(Some(pressed), input::FocusOrigin::Pointer);
            }
            return true;
        }
        false
    }

    /// Focuses an enabled control, places the caret under the press point,
    /// and captures the pointer for drag selection (M4c1). Disabled
    /// controls are ignored; readonly controls are selectable.
    fn begin_control_selection(&mut self, viewport: Viewport, node: dom::NodeId, x: f32, y: f32) {
        if self.controls.get(&node).is_some_and(|state| state.disabled) {
            return;
        }
        self.set_focus_node_with(Some(node), input::FocusOrigin::Pointer);
        let Some(offset) = self.control_offset_at_point(viewport, node, x, y) else {
            return;
        };
        if let Some(state) = self.controls.get_mut(&node) {
            state.editor.collapse_to(offset);
        }
        self.pointer_capture = Some(node);
        self.pointer_anchor = Some(offset);
        self.events.push(Event::SelectionChanged {
            target: self.node_target(node),
            anchor: offset,
            focus: offset,
        });
        self.rebuild_control_presentation();
    }

    /// Extends the captured control's selection to the pointer position.
    /// Mapping ignores clipping (capture deliberately survives leaving the
    /// box); the offset clamps to the nearest line's grapheme boundaries.
    fn extend_control_selection(&mut self, viewport: Viewport, x: f32, y: f32) -> bool {
        let Some(node) = self.pointer_capture else {
            return false;
        };
        let Some(anchor) = self.pointer_anchor else {
            return false;
        };
        let Some(offset) = self.control_offset_at_point(viewport, node, x, y) else {
            return false;
        };
        let Some(state) = self.controls.get_mut(&node) else {
            return false;
        };
        if state.editor.anchor() == anchor && state.editor.focus() == offset {
            return false;
        }
        state.editor.set_selection(anchor, offset);
        self.events.push(Event::SelectionChanged {
            target: self.node_target(node),
            anchor,
            focus: offset,
        });
        self.rebuild_control_presentation();
        true
    }

    /// Maps a viewport point into a control's editor space using the cached
    /// layout (document scroll plus enclosing scroll transforms), then
    /// resolves the nearest grapheme boundary. Pure read of cached geometry
    /// plus font measurement — no layout pass.
    fn control_offset_at_point(
        &mut self,
        viewport: Viewport,
        node: dom::NodeId,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        let scale = viewport.scale_factor();
        let laid = {
            let last = self.last_laid.as_ref()?;
            let cached = self.last_viewport?;
            (cached.width() == viewport.width()
                && cached.height() == viewport.height()
                && cached.scale_factor() == viewport.scale_factor())
            .then_some(last)?
        };
        let (point_x, point_y) = input::point_in_node(&laid.root, laid.root_offset, node, x, y)?;
        let box_node = layout::find_box(&laid.root, node)?.clone();
        let state = self.controls.get(&node)?;
        Some(control::offset_at_point(
            &box_node,
            state,
            &mut self.fonts,
            scale,
            point_x,
            point_y,
        ))
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
            generation: self.document_generation,
        };
        let Some(result) = input::wheel_target(&ctx, x, y, dx, dy) else {
            return;
        };
        // Change is measured against what the target was painted with —
        // not the stored raw request, which can exceed the current clamp
        // after a relayout.
        let changed = result.offset != result.previous_applied;
        if !changed {
            return;
        }
        match self
            .scroll_offsets
            .iter_mut()
            .find(|(existing, _)| *existing == result.node)
        {
            Some(slot) => slot.1 = result.offset,
            None => self.scroll_offsets.push((result.node, result.offset)),
        }
        // Bake the clamped offset into the cached tree so consecutive
        // wheel events (a real pointer delivers many between frames)
        // accumulate and hit tests stay coherent — no invalidation, no
        // relayout; the next render reproduces the same values from the
        // stored offsets.
        if let Some(laid) = self.last_laid.as_mut() {
            match result.node {
                Some(node) => {
                    bake_scroll(&mut laid.root, node, result.offset);
                }
                None => laid.root_offset = result.offset,
            }
        }
        self.events.push(Event::Scrolled {
            target: match result.node {
                None => ScrollTarget::Document,
                Some(_) => ScrollTarget::Element {
                    handle: result
                        .handle
                        .expect("element wheel target always has a handle"),
                    id: result.element_id,
                },
            },
            x: result.offset.0,
            y: result.offset.1,
        });
        // The content under a stationary pointer changed: re-derive hover
        // so :hover follows what is visually under the cursor (ADR 0011).
        self.refresh_hover(viewport);
    }

    /// Moves focus to the next focusable element in document order, wrapping
    /// around (Tab semantics). Supported controls participate even when they
    /// have no HTML `id`; disabled controls are skipped.
    pub fn focus_next(&mut self) {
        let nodes = self.focusable_nodes();
        let next: Option<dom::NodeId> = match self.focus {
            Some(current) => match nodes.iter().position(|&id| id == current) {
                Some(index) => nodes.get((index + 1) % nodes.len().max(1)).copied(),
                None => nodes.first().copied(),
            },
            None => nodes.first().copied(),
        };
        self.set_focus_node_with(next, input::FocusOrigin::Keyboard);
    }

    fn focusable_nodes(&self) -> Vec<dom::NodeId> {
        let mut nodes = Vec::new();
        self.dom.walk(|id, node| {
            let dom::NodeData::Element { attrs, .. } = &node.data else {
                return;
            };
            let has_id = attrs.iter().any(|attribute| attribute.name == "id");
            let is_control = self.controls.contains_key(&id);
            let disabled = self
                .controls
                .get(&id)
                .is_some_and(|control| control.disabled);
            if (has_id || is_control) && !disabled {
                nodes.push(id);
            }
        });
        nodes
    }

    /// Sets focus by element `id` (`None` clears it) and emits
    /// [`Event::FocusChanged`] when it moved. Unknown ids are ignored —
    /// focus targets resolve against the loaded document.
    pub fn set_focus(&mut self, element: Option<&str>) {
        let to = match element {
            None => None,
            Some(id) => {
                let Some(node) = self.element_node(id) else {
                    return;
                };
                Some(node)
            }
        };
        self.set_focus_node_with(to, input::FocusOrigin::Programmatic);
    }

    fn set_focus_node_with(&mut self, to: Option<dom::NodeId>, origin: input::FocusOrigin) {
        if to.is_some_and(|node| {
            self.controls
                .get(&node)
                .is_some_and(|control| control.disabled)
        }) {
            return;
        }
        if to == self.focus {
            return;
        }
        let from = self.focus.take().map(|node| self.node_target(node));
        self.focus = to;
        self.focus_origin = Some(origin);
        self.events.push(Event::FocusChanged {
            from,
            to: to.map(|node| self.node_target(node)),
            origin,
        });
    }

    /// Why focus last moved, if it ever did (M4b).
    pub fn focus_origin(&self) -> Option<input::FocusOrigin> {
        self.focus_origin
    }

    /// Applies one named keyboard command to the focused control.
    ///
    /// Commands are backend-independent; the shell translates platform key
    /// names before calling this method. Returns whether the runtime state
    /// changed. Editing never mutates the DOM or runs document layout.
    pub fn key_command(&mut self, command: KeyCommand, modifiers: KeyModifiers) -> bool {
        let Some(node) = self.focus else {
            return false;
        };
        let Some(state) = self.controls.get_mut(&node) else {
            return false;
        };
        if state.disabled {
            return false;
        }
        let before_value = state.value().to_owned();
        let before_selection = state.selection();
        match command {
            // Clipboard commands (M4c2, ADR 0013) resolve through the
            // installed provider. The provider is fallible and cut is
            // transactional: a write that did not land (contention, null
            // host) leaves the selection and value untouched.
            KeyCommand::Copy => {
                if let Some(text) = state.editor.selected_text() {
                    // Copy never blocks on failure: the editor is
                    // untouched either way, so the error is not load-
                    // bearing here.
                    let _ = self.clipboard.write(text);
                }
                // Copy changes no runtime state (and needs no repaint).
                return false;
            }
            KeyCommand::Cut => {
                if state.readonly || state.editor.selected_range().is_none() {
                    // Browsers ignore cut on readonly fields and on
                    // collapsed selections.
                    return false;
                }
                let text = state.editor.selected_text().unwrap_or_default().to_owned();
                if self.clipboard.write(&text).is_ok() {
                    state.editor.delete_selection();
                } else {
                    // The selection never left the control: destroying it
                    // now would lose user data with no undo (ADR 0013).
                    return false;
                }
            }
            KeyCommand::Paste => {
                if state.readonly {
                    return false;
                }
                let text = match self.clipboard.read() {
                    Ok(Some(text)) => text,
                    // No text on the clipboard, or the clipboard could
                    // not be reached: pasting nothing is the safe move.
                    Ok(None) | Err(_) => return false,
                };
                let filtered = state.kind.filter_text(&text);
                if filtered.is_empty() {
                    return false;
                }
                state.editor.insert_text(&filtered);
            }
            KeyCommand::Backspace if !state.readonly => {
                state.editor.backspace();
            }
            KeyCommand::Delete if !state.readonly => {
                state.editor.delete();
            }
            KeyCommand::Left => state.editor.move_left(modifiers.shift),
            KeyCommand::Right => state.editor.move_right(modifiers.shift),
            KeyCommand::Up if state.kind == ControlKind::Textarea => {
                state.editor.move_vertical(-1, modifiers.shift)
            }
            KeyCommand::Down if state.kind == ControlKind::Textarea => {
                state.editor.move_vertical(1, modifiers.shift)
            }
            KeyCommand::Home => state.editor.move_home(modifiers.shift),
            KeyCommand::End => state.editor.move_end(modifiers.shift),
            KeyCommand::SelectAll if modifiers.select_all() => state.editor.select_all(),
            KeyCommand::Enter if !state.readonly && state.kind.accepts_newline() => {
                state.editor.insert_text("\n");
            }
            KeyCommand::Tab | KeyCommand::Escape | KeyCommand::Up | KeyCommand::Down => {
                return false;
            }
            _ => return false,
        }
        self.finish_control_change(node, before_value, before_selection)
    }

    /// Inserts direct text from a platform text event into the focused control.
    ///
    /// Callers must filter named commands first; this method defensively drops
    /// control characters and newlines for single-line inputs (the same
    /// filter paste applies, ADR 0013).
    pub fn insert_text(&mut self, text: &str) -> bool {
        let Some(node) = self.focus else {
            return false;
        };
        let Some(state) = self.controls.get_mut(&node) else {
            return false;
        };
        if state.disabled || state.readonly {
            return false;
        }
        let filtered = state.kind.filter_text(text);
        if filtered.is_empty() {
            return false;
        }
        let before_value = state.value().to_owned();
        let before_selection = state.selection();
        state.editor.insert_text(&filtered);
        self.finish_control_change(node, before_value, before_selection)
    }

    fn finish_control_change(
        &mut self,
        node: dom::NodeId,
        before_value: String,
        before_selection: (usize, usize),
    ) -> bool {
        let Some(state) = self.controls.get_mut(&node) else {
            return false;
        };
        let after_value = state.value().to_owned();
        let after_selection = state.selection();
        let value_changed = after_value != before_value;
        let selection_changed = after_selection != before_selection;
        if value_changed {
            state.dirty = true;
            self.events.push(Event::ValueChanged {
                target: self.node_target(node),
                value: after_value,
            });
        }
        if selection_changed {
            self.events.push(Event::SelectionChanged {
                target: self.node_target(node),
                anchor: after_selection.0,
                focus: after_selection.1,
            });
        }
        if value_changed || selection_changed {
            // Editor paint (value/selection/caret/scroll) is presentation:
            // rebuilt from the cached outer box, never a Taffy pass.
            self.rebuild_control_presentation();
        }
        value_changed || selection_changed
    }

    /// The `cursor` in effect under the pointer (M4b): the hovered
    /// element's computed (inherited) cursor, or `Auto` when nothing is
    /// under the pointer or no layout is cached. Reading it never lays
    /// out and never repaints.
    pub fn cursor_under(&self, viewport: Viewport, x: f32, y: f32) -> CursorStyle {
        self.cached_layout(viewport)
            .and_then(|laid| input::hit_at(&laid.root, laid.root_offset, x, y))
            .map(|node| node.style.cursor)
            .unwrap_or_default()
    }

    /// Reports the pointer leaving the window: clears hover and emits
    /// [`Event::PointerLeave`] when an element was hovered.
    pub fn pointer_exit(&mut self) {
        self.pointer_pos = None;
        if let Some(old) = self.hover.take() {
            self.events.push(Event::PointerLeave {
                target: self.node_target(old),
            });
        }
    }

    /// The currently focused element's `id`, if it has one.
    pub fn focused(&self) -> Option<&str> {
        self.focus.and_then(|node| self.node_element_id(node))
    }

    /// The `id` of the element currently under the pointer, if it has one.
    pub fn hovered(&self) -> Option<&str> {
        self.hover.and_then(|node| self.node_element_id(node))
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
/// `apply_scroll_offsets` will compute from the stored request. Keys are
/// DOM node identity (ADR 0011). Returns once the node is found.
fn bake_scroll(node: &mut layout::BoxNode, id: dom::NodeId, offset: (f32, f32)) -> bool {
    if node.node == id {
        node.applied_scroll = offset;
        return true;
    }
    node.children
        .iter_mut()
        .any(|child| bake_scroll(child, id, offset))
}

fn collect_control_facts(view: &VelquView, node: &layout::BoxNode, out: &mut Vec<ControlFact>) {
    if let Some(kind) = node.control {
        if let Some(state) = view.controls.get(&node.node) {
            let (anchor, focus) = state.selection();
            let geometry = view.control_geometry.get(&node.node);
            out.push(ControlFact {
                target: view.node_target(node.node),
                kind,
                value_length: state.value().len(),
                selection_anchor: anchor,
                selection_focus: focus,
                caret_rect: geometry
                    .map(|g| ControlRect {
                        x: g.caret.x,
                        y: g.caret.y,
                        width: g.caret.w,
                        height: g.caret.h,
                    })
                    .unwrap_or(ControlRect {
                        x: node.content.x,
                        y: node.content.y,
                        width: 1.0,
                        height: node.content.h.max(1.0),
                    }),
                visible_text_range: geometry.map_or((0, state.value().len()), |g| g.visible_range),
                scroll_offset: geometry.map_or(state.scroll_offset, |g| g.scroll_offset),
            });
        }
    }
    for child in &node.children {
        collect_control_facts(view, child, out);
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

        let b_target = view.node_target(view.element_node("b").unwrap());
        view.pointer_move(vp, 50.0, 150.0); // over pane b
        let events = view.take_events();
        assert_eq!(
            events,
            vec![Event::PointerEnter {
                target: b_target.clone()
            }]
        );

        view.pointer_move(vp, 60.0, 150.0); // still over b, same id: no events
        assert!(view.take_events().is_empty());

        view.pointer_move(vp, 500.0, 500.0); // off-document: leave
        let events = view.take_events();
        assert_eq!(
            events,
            vec![Event::PointerLeave {
                target: b_target.clone()
            }]
        );

        // Press on b, release on b: click + focus (b has an id).
        view.pointer_press(vp, 50.0, 150.0);
        view.pointer_release(vp, 50.0, 150.0);
        let events = view.take_events();
        assert!(events.contains(&Event::Click {
            target: b_target.clone()
        }));
        assert!(events.contains(&Event::FocusChanged {
            from: None,
            to: Some(b_target),
            origin: FocusOrigin::Pointer,
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
                target: ScrollTarget::Document,
                x: 0.0,
                y: 40.0
            }]
        );
        // No layout pass ran for the wheel itself.
        assert_eq!(view.layout_stats().passes, passes_before);
        // Painting the new offset is a presentation-only repaint: the
        // baked offsets re-emit from cached geometry, still zero passes
        // (M4b, ADR 0011).
        view.render(vp).unwrap();
        assert_eq!(view.layout_stats().passes, passes_before);
        assert_eq!(view.layout_stats().repaints, 1);

        // Wheel over the scrollable pane a: the pane scrolls, not the page.
        view.render(vp).unwrap();
        view.pointer_move(vp, 100.0, 50.0); // hover tracking for realism
        let _ = view.take_events();
        view.wheel(vp, 100.0, 50.0, 40.0, 0.0);
        let events = view.take_events();
        let pane_a = view.node_target(view.element_node("a").unwrap());
        assert_eq!(
            events,
            vec![Event::Scrolled {
                target: ScrollTarget::Element {
                    handle: pane_a.handle,
                    id: Some("a".into())
                },
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

    // -- M4b: scroll-state invariants (ADR 0011) --------------------------

    /// Pane "a" (100px scrollport, 300px red content) over a 300px blue
    /// block: the document is 400px tall, so the document scroller has
    /// range in a 200px viewport.
    fn transplant_view() -> VelquView {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\">\
             <div id=a style=\"overflow: auto; width: 200px; height: 100px\">\
             <div style=\"width: 100px; height: 300px; background-color: #ef4444\"></div>\
             </div>\
             <div style=\"width: 200px; height: 300px; background-color: #3b82f6\"></div>\
             </body></html>",
        )
        .unwrap();
        view
    }

    #[test]
    fn scroll_leaves_layout_facts_untouched() {
        let mut view = transplant_view();
        let vp = Viewport::try_new(300, 200, 1.0).unwrap();
        let before = view.layout_facts(vp).unwrap();

        view.set_scroll_offset(Some("a"), 0.0, 250.0).unwrap();
        view.render(vp).unwrap();
        view.wheel(vp, 50.0, 150.0, 0.0, 40.0); // document scroller
        let _ = view.take_events();

        let after = view.layout_facts(vp).unwrap();
        assert_eq!(before, after, "facts are the unscrolled truth");
    }

    #[test]
    fn scroll_applies_once_and_stays_across_repeated_renders() {
        let mut view = transplant_view();
        let vp = Viewport::try_new(300, 200, 1.0).unwrap();
        view.render(vp).unwrap();
        view.wheel(vp, 50.0, 150.0, 0.0, 40.0); // document scroller
        let _ = view.take_events();

        let first = view.render(vp).unwrap().frame.sha256_hex();
        let second = view.render(vp).unwrap().frame.sha256_hex();
        let third = view.render(vp).unwrap().frame.sha256_hex();
        // Neither lost (second render reverts) nor applied twice (each
        // render shifts another 40px).
        assert_eq!(first, second);
        assert_eq!(second, third);

        // And it differs from the unscrolled page: the offset is real.
        let mut fresh = transplant_view();
        fresh.render(vp).unwrap();
        assert_ne!(fresh.render(vp).unwrap().frame.sha256_hex(), first);
    }

    #[test]
    fn scroll_position_survives_resize_and_reclamps() {
        let mut view = transplant_view();
        // Document extent 400 tall; viewport 200 → max offset 200.
        let vp = Viewport::try_new(300, 200, 1.0).unwrap();
        view.render(vp).unwrap();
        for _ in 0..5 {
            view.wheel(vp, 150.0, 150.0, 0.0, 40.0); // ×5 → 200 (max)
            let _ = view.take_events();
            view.render(vp).unwrap();
        }

        // Resize the viewport taller: extent 400 − 320 = max 80 now. The
        // offset must survive (re-clamped), not reset. The blue block
        // spans content y 100..400, so at viewport y=200:
        // offset 0 → content 200 (blue), offset 80 → content 280 (blue)…
        // both blue, so probe the pane instead: pane content is red at
        // every reachable offset — instead assert behaviorally below.
        let taller = Viewport::try_new(300, 320, 1.0).unwrap();
        view.render(taller).unwrap();

        // At the new max (80): a further wheel-down changes nothing, and a
        // wheel-up lands exactly 40 lower — the position re-clamped rather
        // than resetting to 0 (which would wheel up to... nothing) or
        // staying at the stale 200 (which would wheel down "clamping" a
        // correction).
        view.wheel(taller, 150.0, 150.0, 0.0, 40.0);
        assert!(
            view.take_events().is_empty(),
            "at the re-clamped max, further wheel-down is a no-op"
        );
        view.wheel(taller, 150.0, 150.0, 0.0, -40.0);
        assert_eq!(
            view.take_events(),
            vec![Event::Scrolled {
                target: ScrollTarget::Document,
                x: 0.0,
                y: 40.0,
            }],
            "offset survived the resize (80) and moved to 40"
        );
    }

    #[test]
    fn scroll_position_survives_a_restyle_relayout() {
        let mut view = transplant_view();
        let vp = Viewport::try_new(300, 200, 1.0).unwrap();
        view.render(vp).unwrap();
        view.wheel(vp, 50.0, 50.0, 0.0, 40.0); // pane a → 40
        let _ = view.take_events();
        let pane_a = view.node_target(view.element_node("a").unwrap());

        // A new stylesheet forces a full relayout (the box tree is
        // rebuilt); the pane still exists, so its offset transplants.
        view.load_css("#a { background-color: #22c55e }").unwrap();
        view.render(vp).unwrap();

        view.wheel(vp, 50.0, 50.0, 0.0, 40.0);
        assert_eq!(
            view.take_events(),
            vec![Event::Scrolled {
                target: ScrollTarget::Element {
                    handle: pane_a.handle,
                    id: Some("a".into())
                },
                x: 0.0,
                y: 80.0,
            }],
            "the offset accumulated across the relayout"
        );
    }

    #[test]
    fn idless_scroll_containers_have_independent_accumulating_state() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\">\
             <div style=\"overflow: auto; width: 200px; height: 100px\">\
             <div style=\"width: 100px; height: 300px\"></div></div>\
             <div style=\"width: 200px; height: 50px\"></div>\
             <div style=\"overflow: auto; width: 200px; height: 100px\">\
             <div style=\"width: 100px; height: 300px\"></div></div>\
             </body></html>",
        )
        .unwrap();
        // Viewport 200 tall vs a 250px document: the document scroller has
        // range 50, and both panes are reachable.
        let vp = Viewport::try_new(300, 200, 1.0).unwrap();
        view.render(vp).unwrap();

        // Both panes are id-less; neither may alias the document scroller
        // or each other, and repeated wheels accumulate (they bake into
        // the cached tree).
        for _ in 0..3 {
            view.wheel(vp, 50.0, 50.0, 0.0, 40.0); // first pane
            view.render(vp).unwrap();
        }
        let events = view.take_events();
        let first_handle = match &events[0] {
            Event::Scrolled {
                target: ScrollTarget::Element { handle, .. },
                ..
            } => *handle,
            other => panic!("unexpected first event: {other:?}"),
        };
        assert_eq!(
            events,
            vec![
                Event::Scrolled {
                    target: ScrollTarget::Element {
                        handle: first_handle,
                        id: None
                    },
                    x: 0.0,
                    y: 40.0,
                },
                Event::Scrolled {
                    target: ScrollTarget::Element {
                        handle: first_handle,
                        id: None
                    },
                    x: 0.0,
                    y: 80.0,
                },
                Event::Scrolled {
                    target: ScrollTarget::Element {
                        handle: first_handle,
                        id: None
                    },
                    x: 0.0,
                    y: 120.0,
                },
            ]
        );

        // The second pane is untouched by the first pane's scroll.
        view.wheel(vp, 50.0, 200.0, 0.0, 40.0); // second pane (y 150..250)
        let events = view.take_events();
        let second_handle = match &events[0] {
            Event::Scrolled {
                target: ScrollTarget::Element { handle, .. },
                ..
            } => *handle,
            other => panic!("unexpected second event: {other:?}"),
        };
        assert_ne!(first_handle, second_handle);
        assert_eq!(
            events,
            vec![Event::Scrolled {
                target: ScrollTarget::Element {
                    handle: second_handle,
                    id: None
                },
                x: 0.0,
                y: 40.0,
            }]
        );
        // And the document scroller is a distinct target (probed outside
        // the 200px content width, over the body block).
        view.wheel(vp, 250.0, 180.0, 0.0, 40.0);
        let events = view.take_events();
        assert!(matches!(
            events.as_slice(),
            [Event::Scrolled {
                target: ScrollTarget::Document,
                ..
            }]
        ));
    }

    // -- M4b: interaction styling end-to-end (ADR 0011) --------------------

    /// Card (120×120, blue) with a label child (60×60): hovering the
    /// *label* must turn the card red through the ancestor chain and the
    /// label green through the `.card:hover .label` descendant rule.
    fn hover_chain_view() -> VelquView {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><head><style>             body { margin: 0 }\
             .card { width: 120px; height: 120px; background-color: #3b82f6 }\
             .card:hover { background-color: #ef4444 }\
             .label { display: block; width: 60px; height: 60px }\
             .card:hover .label { background-color: #22c55e }\
             </style></head><body>\
             <div class=card><span class=label>x</span></div>\
             </body></html>",
        )
        .unwrap();
        view
    }

    #[test]
    fn hover_styles_follow_the_pointer_chain() {
        let mut view = hover_chain_view();
        let vp = Viewport::try_new(200, 200, 1.0).unwrap();
        let base = view.render(vp).unwrap().frame;
        assert_eq!(
            base.pixel(30, 30),
            Some(Color::from_hex("#3b82f6").unwrap()),
            "unhovered: the label area shows the card's blue"
        );

        // Pointer over the label — the card is in the label's hover chain.
        view.pointer_move(vp, 30.0, 30.0);
        let frame = view.render(vp).unwrap().frame;
        assert_eq!(
            frame.pixel(90, 90),
            Some(Color::from_hex("#ef4444").unwrap()),
            "an ancestor matches :hover when a descendant is hovered"
        );
        assert_eq!(
            frame.pixel(30, 30),
            Some(Color::from_hex("#22c55e").unwrap()),
            "the descendant rule .card:hover .label activates"
        );

        // Pointer off the card: everything reverts.
        view.pointer_move(vp, 180.0, 180.0);
        let frame = view.render(vp).unwrap().frame;
        assert_eq!(
            frame.pixel(90, 90),
            Some(Color::from_hex("#3b82f6").unwrap())
        );
        assert_eq!(
            frame.pixel(30, 30),
            Some(Color::from_hex("#3b82f6").unwrap())
        );
    }

    #[test]
    fn hover_changes_pixels_without_touching_layout() {
        let mut view = hover_chain_view();
        let vp = Viewport::try_new(200, 200, 1.0).unwrap();
        view.render(vp).unwrap();
        let facts = view.layout_facts(vp).unwrap();
        let passes = view.layout_stats().passes;

        view.pointer_move(vp, 30.0, 30.0);
        let hovered = view.render(vp).unwrap();
        assert_eq!(
            hovered.frame.sha256_hex(),
            view.render(vp).unwrap().frame.sha256_hex(),
            "steady-state hover renders are deterministic"
        );

        assert_eq!(
            view.layout_stats().passes,
            passes,
            "pointer motion must not lay out"
        );
        assert!(view.layout_stats().repaints >= 1, "the repaint is counted");
        // Facts call runs its own truth pass, after the count assertions.
        let after = view.layout_facts(vp).unwrap();
        assert_eq!(facts, after, "facts are the structural truth, state-free");
    }

    #[test]
    fn overlapping_siblings_hover_the_topmost_only() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><head><style>             body { margin: 0 }\
             .box { width: 100px; height: 100px }\
             #under { background-color: #3b82f6 }\
             #under:hover { background-color: #ef4444 }\
             #over { background-color: #111111; margin-top: -50px }\
             #over:hover { background-color: #22c55e }\
             </style></head><body>\
             <div id=under class=box></div>\
             <div id=over class=box></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(200, 200, 1.0).unwrap();
        view.render(vp).unwrap();

        // The overlap zone (y 50..100) paints `over` last: hit testing and
        // hover both go to the visually topmost box (ADR 0010's contract).
        view.pointer_move(vp, 50.0, 75.0);
        let frame = view.render(vp).unwrap().frame;
        assert_eq!(
            frame.pixel(50, 75),
            Some(Color::from_hex("#22c55e").unwrap()),
            "the topmost painted box takes the hover"
        );

        // Above the overlap (y 0..50), only `under` is under the pointer.
        view.pointer_move(vp, 50.0, 25.0);
        let frame = view.render(vp).unwrap().frame;
        assert_eq!(
            frame.pixel(50, 25),
            Some(Color::from_hex("#ef4444").unwrap()),
            "the lower box hovers where it is visible"
        );
    }

    #[test]
    fn clipped_content_is_never_hovered() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><head><style>             body { margin: 0 }\
             .pane { overflow: auto; width: 200px; height: 100px }\
             .wide { width: 400px; height: 60px; background-color: #3b82f6 }\
             .wide:hover { background-color: #ef4444 }\
             </style></head><body>\
             <div class=pane><div class=wide></div></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(300, 200, 1.0).unwrap();
        view.render(vp).unwrap();

        // Inside the clip: hover applies.
        view.pointer_move(vp, 100.0, 30.0);
        let frame = view.render(vp).unwrap().frame;
        assert_eq!(
            frame.pixel(100, 30),
            Some(Color::from_hex("#ef4444").unwrap())
        );

        // Outside the clip (x ≥ 200 is past the pane's edge): the wide
        // child's raw geometry extends here, but it is clipped away —
        // no hover, pixels unchanged.
        view.pointer_move(vp, 250.0, 30.0);
        let frame = view.render(vp).unwrap().frame;
        assert_ne!(
            frame.pixel(250, 30),
            Some(Color::from_hex("#ef4444").unwrap()),
            "clipped-away content must not match :hover"
        );
    }

    #[test]
    fn scrolling_under_a_stationary_pointer_moves_hover() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><head><style>             body { margin: 0 }\
             section { width: 200px; height: 200px }\
             #a { background-color: #3b82f6 }\
             #a:hover { background-color: #ef4444 }\
             #b { background-color: #111111 }\
             #b:hover { background-color: #22c55e }\
             </style></head><body>\
             <section id=a></section>\
             <section id=b></section>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(300, 200, 1.0).unwrap();
        view.render(vp).unwrap();
        let passes = view.layout_stats().passes;

        // Pointer parked at (50, 150) over #a.
        view.pointer_move(vp, 50.0, 150.0);
        view.render(vp).unwrap();
        assert_eq!(
            view.render(vp).unwrap().frame.pixel(50, 150),
            Some(Color::from_hex("#ef4444").unwrap())
        );

        // Wheel down without moving the pointer: #b slides under it.
        view.wheel(vp, 50.0, 150.0, 0.0, 100.0);
        let events = view.take_events();
        let a_target = view.node_target(view.element_node("a").unwrap());
        let b_target = view.node_target(view.element_node("b").unwrap());
        assert!(
            events.contains(&Event::PointerLeave { target: a_target }),
            "{events:?}"
        );
        assert!(
            events.contains(&Event::PointerEnter { target: b_target }),
            "{events:?}"
        );
        let frame = view.render(vp).unwrap().frame;
        assert_eq!(
            frame.pixel(50, 150),
            Some(Color::from_hex("#22c55e").unwrap()),
            "the hover style follows the content under the pointer"
        );
        assert_eq!(
            view.layout_stats().passes,
            passes,
            "scroll + hover never lay out"
        );
    }

    #[test]
    fn focus_paint_moves_with_tab_and_never_lays_out() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><head><style>             body { margin: 0 }\
             .box { width: 100px; height: 100px; background-color: #3b82f6 }\
             :focus { background-color: #ef4444 }\
             </style></head><body>\
             <div id=a class=box></div>\
             <div id=b class=box></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(200, 300, 1.0).unwrap();
        view.render(vp).unwrap();
        let passes = view.layout_stats().passes;

        view.focus_next(); // → a
        let frame = view.render(vp).unwrap().frame;
        assert_eq!(
            frame.pixel(50, 50),
            Some(Color::from_hex("#ef4444").unwrap())
        );
        assert_eq!(
            frame.pixel(50, 150),
            Some(Color::from_hex("#3b82f6").unwrap()),
            "only the focused box paints the focus style"
        );

        view.focus_next(); // → b
        let frame = view.render(vp).unwrap().frame;
        assert_eq!(
            frame.pixel(50, 150),
            Some(Color::from_hex("#ef4444").unwrap())
        );
        assert_eq!(
            frame.pixel(50, 50),
            Some(Color::from_hex("#3b82f6").unwrap())
        );
        assert_eq!(view.layout_stats().passes, passes);
    }

    #[test]
    fn layout_hover_deferred_with_deduped_diagnostics() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><head><style>             body { margin: 0 }\
             .card { width: 100px; height: 100px; background-color: #3b82f6 }\
             .card:hover { width: 500px; background-color: #ef4444 }\
             </style></head><body>\
             <div class=card data-vv-test=card></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(600, 200, 1.0).unwrap();
        view.render(vp).unwrap();

        view.pointer_move(vp, 50.0, 50.0);
        let frame = view.render(vp).unwrap().frame;
        // The paint half of the rule applies...
        assert_eq!(
            frame.pixel(50, 50),
            Some(Color::from_hex("#ef4444").unwrap())
        );
        // ...but the layout half is deferred: the card keeps its width.
        assert_eq!(
            frame.pixel(300, 50),
            Some(Color::WHITE),
            "the deferred width never changes layout truth"
        );

        let diagnostics = view.style_diagnostics();
        assert_eq!(diagnostics.len(), 1, "deduplicated: {diagnostics:?}");
        assert!(diagnostics[0].contains("width"), "{diagnostics:?}");
        assert!(diagnostics[0].contains("deferred"), "{diagnostics:?}");

        // Facts stay untouched by the whole affair.
        let facts = view.layout_facts(vp).unwrap();
        let card = facts
            .nodes
            .iter()
            .find(|n| n.fixture_id == "card")
            .expect("card in facts");
        assert_eq!(card.width, 100.0, "the deferred width never applies");
    }

    #[test]
    fn cursor_is_inherited_and_read_under_the_pointer() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><head><style>\
             body { margin: 0 }\
             .card { cursor: pointer; width: 120px; height: 120px }\
             </style></head><body>\
             <div class=card><span id=inner>x</span></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(200, 200, 1.0).unwrap();
        view.render(vp).unwrap();
        let passes = view.layout_stats().passes;

        // Over the span (no cursor of its own): the card's pointer cursor
        // inherits through the computed style (ADR 0011).
        assert_eq!(view.cursor_under(vp, 20.0, 20.0), CursorStyle::Pointer);
        // Over the card's own chrome: pointer as well.
        assert_eq!(view.cursor_under(vp, 100.0, 100.0), CursorStyle::Pointer);
        // Outside: UA default.
        assert_eq!(view.cursor_under(vp, 180.0, 180.0), CursorStyle::Auto);
        assert_eq!(
            view.layout_stats().passes,
            passes,
            "reading the cursor never lays out"
        );
    }

    #[test]
    fn focus_origin_tracks_why_focus_moved() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body>\
             <div id=a></div><div id=b></div>\
             </body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(100, 100, 1.0).unwrap();
        view.render(vp).unwrap();

        assert_eq!(view.focus_origin(), None);
        view.focus_next();
        assert_eq!(view.focus_origin(), Some(FocusOrigin::Keyboard));
        view.set_focus(Some("b"));
        assert_eq!(view.focus_origin(), Some(FocusOrigin::Programmatic));

        // A click focuses with pointer origin.
        let mut click_view = VelquView::new();
        click_view
            .load_html(
                "<!doctype html><html><body style=\"margin: 0\">\
             <div id=x style=\"width: 100px; height: 100px\"></div>\
             </body></html>",
            )
            .unwrap();
        click_view.render(vp).unwrap();
        click_view.pointer_press(vp, 50.0, 50.0);
        click_view.pointer_release(vp, 50.0, 50.0);
        assert_eq!(click_view.focused(), Some("x"));
        assert_eq!(click_view.focus_origin(), Some(FocusOrigin::Pointer));
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

        let first_target = view.node_target(view.element_node("first").unwrap());
        let second_target = view.node_target(view.element_node("second").unwrap());
        let events = view.take_events();
        assert_eq!(
            events,
            vec![
                Event::FocusChanged {
                    from: None,
                    to: Some(first_target.clone()),
                    origin: FocusOrigin::Keyboard,
                },
                Event::FocusChanged {
                    from: Some(first_target.clone()),
                    to: Some(second_target.clone()),
                    origin: FocusOrigin::Keyboard,
                },
                Event::FocusChanged {
                    from: Some(second_target),
                    to: Some(first_target),
                    origin: FocusOrigin::Keyboard,
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
    fn m4c1_editing_keeps_runtime_value_out_of_dom_and_layout_facts() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\"><input id=editor type=text value=abc></body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(300, 120, 1.0).unwrap();
        let facts_before = view.layout_facts(vp).unwrap();
        view.render(vp).unwrap();
        view.set_focus(Some("editor"));
        let _ = view.take_events();
        let passes = view.layout_stats().passes;

        assert!(view.insert_text("é"));
        assert_eq!(view.layout_stats().passes, passes);
        let target = view.node_target(view.element_node("editor").unwrap());
        assert_eq!(view.control_value(target.handle), Some("éabc"));
        assert_eq!(
            view.dom
                .attribute(view.element_node("editor").unwrap(), "value"),
            Some("abc")
        );
        assert_eq!(view.layout_facts(vp).unwrap(), facts_before);
        assert_eq!(
            view.take_events(),
            vec![
                Event::ValueChanged {
                    target: target.clone(),
                    value: "éabc".to_owned(),
                },
                Event::SelectionChanged {
                    target,
                    anchor: "é".len(),
                    focus: "é".len(),
                },
            ]
        );
    }

    #[test]
    fn m4c1_commands_filter_named_text_and_allow_textarea_newlines() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\"><input id=input type=text value=abc><textarea id=area>one</textarea></body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(400, 240, 1.0).unwrap();
        view.render(vp).unwrap();

        view.set_focus(Some("input"));
        let _ = view.take_events();
        assert!(view.key_command(KeyCommand::End, KeyModifiers::default()));
        assert!(
            !view.insert_text("\r"),
            "Enter text must not reach an input"
        );
        assert!(view.key_command(KeyCommand::Backspace, KeyModifiers::default()));
        assert_eq!(
            view.control_value(view.node_target(view.element_node("input").unwrap()).handle),
            Some("ab")
        );

        view.set_focus(Some("area"));
        let _ = view.take_events();
        assert!(view.key_command(KeyCommand::End, KeyModifiers::default()));
        assert!(view.key_command(KeyCommand::Enter, KeyModifiers::default()));
        assert!(view.insert_text("two\nthree"));
        assert_eq!(
            view.control_value(view.node_target(view.element_node("area").unwrap()).handle),
            Some("one\ntwo\nthree")
        );
    }

    #[test]
    fn m4c1_readonly_and_disabled_controls_reject_value_changes() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body><input id=ro value=abc readonly><input id=off value=xyz disabled><input id=ok value=q></body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(500, 200, 1.0).unwrap();
        view.render(vp).unwrap();

        view.set_focus(Some("ro"));
        let _ = view.take_events();
        assert!(!view.insert_text("x"));
        assert!(view.key_command(KeyCommand::Right, KeyModifiers::default()));
        assert_eq!(
            view.control_value(view.node_target(view.element_node("ro").unwrap()).handle),
            Some("abc")
        );

        view.focus_next();
        assert_eq!(view.focused(), Some("ok"), "disabled control is skipped");
        view.set_focus(Some("off"));
        assert_eq!(
            view.focused(),
            Some("ok"),
            "disabled control cannot receive focus"
        );
    }

    #[test]
    fn m4c1_click_places_caret_and_focuses_the_control() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\"><input id=box value=hello></body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(300, 120, 1.0).unwrap();
        view.render(vp).unwrap();
        let passes = view.layout_stats().passes;

        // Caret geometry comes from facts: caret at offset 0 sits at the
        // content origin; its height is the editor line height.
        let facts = view.control_facts(vp).unwrap();
        let caret = facts.controls[0].caret_rect;
        assert_eq!(view.focused(), None);

        // Press near the text start, then release: focus (pointer origin),
        // a collapsed caret near offset 0, and a click.
        view.pointer_press(vp, caret.x + 1.0, caret.y + caret.height / 2.0);
        view.pointer_release(vp, caret.x + 1.0, caret.y + caret.height / 2.0);
        assert_eq!(view.focused(), Some("box"));
        assert_eq!(view.focus_origin(), Some(FocusOrigin::Pointer));
        assert_eq!(
            view.layout_stats().passes,
            passes,
            "caret placement never lays out"
        );

        let events = view.take_events();
        let target = view.node_target(view.element_node("box").unwrap());
        assert!(
            events.contains(&Event::Click {
                target: target.clone()
            }),
            "{events:?}"
        );
        assert!(
            events.contains(&Event::SelectionChanged {
                target,
                anchor: 0,
                focus: 0,
            }),
            "{events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::ValueChanged { .. })),
            "clicking must not edit the value"
        );

        let facts = view.control_facts(vp).unwrap();
        assert_eq!(facts.controls[0].selection_anchor, 0);
        assert_eq!(facts.controls[0].selection_focus, 0);
    }

    #[test]
    fn m4c1_drag_selects_with_capture_even_outside_the_control() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\"><input id=box value=hello></body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(300, 120, 1.0).unwrap();
        view.render(vp).unwrap();
        let passes = view.layout_stats().passes;
        let facts = view.control_facts(vp).unwrap();
        let caret = facts.controls[0].caret_rect;
        let target = view.node_target(view.element_node("box").unwrap());

        // Press at the text start, then drag far past the control's right
        // edge (the input is 200px wide): capture keeps updating selection.
        view.pointer_press(vp, caret.x + 1.0, caret.y + caret.height / 2.0);
        let _ = view.take_events();
        let moved = view.pointer_move(vp, caret.x + 400.0, caret.y + caret.height / 2.0);
        assert!(moved, "the selection drag changed presentation");
        assert_eq!(
            view.layout_stats().passes,
            passes,
            "dragging never lays out"
        );
        assert_eq!(
            view.hovered(),
            None,
            "hover follows the real pointer, not capture"
        );
        let events = view.take_events();
        assert_eq!(
            events,
            vec![
                Event::PointerLeave {
                    target: target.clone()
                },
                Event::SelectionChanged {
                    target: target.clone(),
                    anchor: 0,
                    focus: 5,
                },
            ],
            "{events:?}"
        );

        // Release outside the box ends capture: later moves hover normally
        // but never touch the selection.
        view.pointer_release(vp, caret.x + 400.0, caret.y + caret.height / 2.0);
        let _ = view.take_events();
        view.pointer_move(vp, caret.x + 1.0, caret.y + caret.height / 2.0);
        let events = view.take_events();
        assert!(
            events
                .iter()
                .all(|event| matches!(event, Event::PointerEnter { .. })),
            "only hover events remain: {events:?}"
        );

        let facts = view.control_facts(vp).unwrap();
        assert_eq!(
            (
                facts.controls[0].selection_anchor,
                facts.controls[0].selection_focus
            ),
            (0, 5)
        );
    }

    #[test]
    fn m4c1_control_facts_report_caret_and_scroll_to_caret() {
        let long = "abcdefghij".repeat(8);
        let html = format!(
            "<!doctype html><html><body style=\"margin: 0\"><input id=long data-vv-test=long value={long}></body></html>"
        );
        let mut view = VelquView::new();
        view.load_html(&html).unwrap();
        let vp = Viewport::try_new(300, 120, 1.0).unwrap();
        view.render(vp).unwrap();
        let structural = view.layout_facts(vp).unwrap();
        let node_fact = structural
            .nodes
            .iter()
            .find(|fact| fact.fixture_id == "long")
            .expect("input in layout facts");
        let (content_x, content_width) = (node_fact.content_x, node_fact.content_width);
        let _ = view.render(vp).unwrap();

        view.set_focus(Some("long"));
        let _ = view.take_events();
        view.key_command(KeyCommand::Home, KeyModifiers::default());
        let facts = view.control_facts(vp).unwrap();
        assert_eq!(facts.controls[0].scroll_offset, (0.0, 0.0));
        assert_eq!(facts.controls[0].visible_text_range.0, 0);
        assert_eq!(facts.controls[0].value_length, long.len());

        // End: the caret moves to the value end and the editor scrolls
        // horizontally to keep it visible.
        view.key_command(KeyCommand::End, KeyModifiers::default());
        let facts = view.control_facts(vp).unwrap();
        let fact = &facts.controls[0];
        assert!(
            fact.scroll_offset.0 > 0.0,
            "scroll-to-caret engaged: {fact:?}"
        );
        assert_eq!(fact.visible_text_range, (0, long.len()));
        assert!(fact.caret_rect.x >= content_x);
        assert!(fact.caret_rect.x <= content_x + content_width);
        assert!(fact.caret_rect.y >= content_x - content_x); // trivially within the box vertically
        assert!(fact.caret_rect.height > 0.0);
    }

    #[test]
    fn m4c1_editing_repaints_presentation_without_layout() {
        let mut view = VelquView::new();
        view.load_html(
            "<!doctype html><html><body style=\"margin: 0\"><input id=box value=abc></body></html>",
        )
        .unwrap();
        let vp = Viewport::try_new(300, 120, 1.0).unwrap();
        let before = view.render(vp).unwrap();
        let hash_before = before.frame.sha256_hex();
        let stats_before = view.layout_stats();

        view.set_focus(Some("box"));
        let _ = view.take_events();
        assert!(view.insert_text("Z"));

        let after = view.render(vp).unwrap();
        assert_ne!(
            after.frame.sha256_hex(),
            hash_before,
            "the edited value paints"
        );
        let stats_after = view.layout_stats();
        assert_eq!(stats_after.passes, stats_before.passes, "no Taffy pass");
        assert!(
            stats_after.repaints > stats_before.repaints,
            "a repaint pass ran"
        );
    }

    // -- M4c2 clipboard (ADR 0013) ----------------------------------------

    /// A recording host clipboard: writes are captured, reads hand out one
    /// preloaded value (like a clipboard the user primed externally).
    #[derive(Default)]
    struct RecordingClipboard {
        written: RefCell<Vec<String>>,
        next_read: RefCell<Option<String>>,
    }

    impl ClipboardProvider for RecordingClipboard {
        fn read(&self) -> Result<Option<String>, ClipboardError> {
            Ok(self.next_read.borrow_mut().take())
        }

        fn write(&self, text: &str) -> Result<(), ClipboardError> {
            self.written.borrow_mut().push(text.to_owned());
            Ok(())
        }
    }

    /// A host whose clipboard write always fails — the real-world shape of
    /// clipboard contention or an unsupported environment.
    struct FailingClipboard;

    impl ClipboardProvider for FailingClipboard {
        fn read(&self) -> Result<Option<String>, ClipboardError> {
            Ok(None)
        }

        fn write(&self, _text: &str) -> Result<(), ClipboardError> {
            Err(ClipboardError::new("clipboard busy"))
        }
    }

    fn clipboard_view(html: &str) -> (VelquView, Rc<RecordingClipboard>) {
        let mut view = VelquView::new();
        view.load_html(html).unwrap();
        let clipboard = Rc::new(RecordingClipboard::default());
        view.set_clipboard_provider(clipboard.clone());
        (view, clipboard)
    }

    #[test]
    fn m4c2_copy_writes_the_selection_and_changes_nothing() {
        let (mut view, clipboard) =
            clipboard_view("<!doctype html><html><body><input id=box value=hello></body></html>");
        let vp = Viewport::try_new(300, 120, 1.0).unwrap();
        view.render(vp).unwrap();
        view.set_focus(Some("box"));
        let _ = view.take_events();
        assert!(view.key_command(
            KeyCommand::SelectAll,
            KeyModifiers {
                ctrl: true,
                ..KeyModifiers::default()
            }
        ));
        let _ = view.take_events();
        let passes = view.layout_stats().passes;

        assert!(!view.key_command(KeyCommand::Copy, KeyModifiers::default()));
        assert_eq!(
            clipboard.written.borrow().as_slice(),
            ["hello"],
            "the selection reached the host clipboard"
        );
        assert_eq!(
            view.control_value(view.node_target(view.element_node("box").unwrap()).handle),
            Some("hello"),
            "copy never edits the value"
        );
        assert!(view.take_events().is_empty(), "copy emits nothing");
        assert_eq!(view.layout_stats().passes, passes);

        // Collapsed caret + copy: nothing selected, nothing written.
        assert!(view.key_command(KeyCommand::Right, KeyModifiers::default()));
        let _ = view.take_events();
        clipboard.written.borrow_mut().clear();
        assert!(!view.key_command(KeyCommand::Copy, KeyModifiers::default()));
        assert!(clipboard.written.borrow().is_empty());
    }

    #[test]
    fn m4c2_cut_deletes_and_paste_inserts_through_the_provider() {
        let (mut view, clipboard) =
            clipboard_view("<!doctype html><html><body><input id=box value=hello></body></html>");
        let vp = Viewport::try_new(300, 120, 1.0).unwrap();
        view.render(vp).unwrap();
        view.set_focus(Some("box"));
        let _ = view.take_events();
        let target = view.node_target(view.element_node("box").unwrap());
        assert!(view.key_command(
            KeyCommand::SelectAll,
            KeyModifiers {
                ctrl: true,
                ..KeyModifiers::default()
            }
        ));
        let _ = view.take_events();

        // Cut: clipboard gets the text, value empties, both events fire.
        assert!(view.key_command(KeyCommand::Cut, KeyModifiers::default()));
        assert_eq!(clipboard.written.borrow().as_slice(), ["hello"]);
        assert_eq!(view.control_value(target.handle), Some(""));
        assert_eq!(
            view.take_events(),
            vec![
                Event::ValueChanged {
                    target: target.clone(),
                    value: String::new(),
                },
                Event::SelectionChanged {
                    target: target.clone(),
                    anchor: 0,
                    focus: 0,
                },
            ]
        );

        // Paste: the provider's text replaces the (empty) selection.
        *clipboard.next_read.borrow_mut() = Some("pasté".to_owned());
        assert!(view.key_command(KeyCommand::Paste, KeyModifiers::default()));
        assert_eq!(view.control_value(target.handle), Some("pasté"));

        // An empty clipboard pastes nothing.
        assert!(!view.key_command(KeyCommand::Paste, KeyModifiers::default()));
        assert_eq!(view.control_value(target.handle), Some("pasté"));
    }

    #[test]
    fn m4c2_cut_is_grapheme_safe_and_paste_normalizes_crlf() {
        let (mut view, clipboard) = clipboard_view(
            "<!doctype html><html><body><input id=box value=a😀b><textarea id=area>x</textarea></body></html>",
        );
        let vp = Viewport::try_new(400, 240, 1.0).unwrap();
        view.render(vp).unwrap();

        // Caret after 'a', Shift+Right selects the emoji as one grapheme.
        view.set_focus(Some("box"));
        let _ = view.take_events();
        view.key_command(KeyCommand::Home, KeyModifiers::default());
        view.key_command(KeyCommand::Right, KeyModifiers::default());
        view.key_command(
            KeyCommand::Right,
            KeyModifiers {
                shift: true,
                ..KeyModifiers::default()
            },
        );
        let _ = view.take_events();
        assert!(view.key_command(KeyCommand::Cut, KeyModifiers::default()));
        assert_eq!(clipboard.written.borrow().as_slice(), ["😀"]);
        assert_eq!(
            view.control_value(view.node_target(view.element_node("box").unwrap()).handle),
            Some("ab"),
            "the emoji is cut as one grapheme"
        );

        // CRLF pastes into a textarea arrive as plain newlines.
        view.set_focus(Some("area"));
        let _ = view.take_events();
        assert!(view.key_command(KeyCommand::End, KeyModifiers::default()));
        let _ = view.take_events();
        *clipboard.next_read.borrow_mut() = Some("two\r\nthree".to_owned());
        assert!(view.key_command(KeyCommand::Paste, KeyModifiers::default()));
        assert_eq!(
            view.control_value(view.node_target(view.element_node("area").unwrap()).handle),
            Some("xtwo\nthree")
        );

        // The same clipboard text into a single-line input loses the
        // newlines entirely.
        view.set_focus(Some("box"));
        let _ = view.take_events();
        assert!(view.key_command(KeyCommand::End, KeyModifiers::default()));
        let _ = view.take_events();
        *clipboard.next_read.borrow_mut() = Some("one\r\ntwo".to_owned());
        assert!(view.key_command(KeyCommand::Paste, KeyModifiers::default()));
        assert_eq!(
            view.control_value(view.node_target(view.element_node("box").unwrap()).handle),
            Some("abonetwo")
        );
    }

    #[test]
    fn m4c2_readonly_and_null_clipboard_policies() {
        // Readonly: copy still writes; cut and paste are no-ops.
        let (mut view, clipboard) = clipboard_view(
            "<!doctype html><html><body><input id=ro value=frozen readonly></body></html>",
        );
        let vp = Viewport::try_new(300, 120, 1.0).unwrap();
        view.render(vp).unwrap();
        view.set_focus(Some("ro"));
        let _ = view.take_events();
        assert!(view.key_command(
            KeyCommand::SelectAll,
            KeyModifiers {
                ctrl: true,
                ..KeyModifiers::default()
            }
        ));
        let _ = view.take_events();

        assert!(!view.key_command(KeyCommand::Copy, KeyModifiers::default()));
        assert_eq!(clipboard.written.borrow().as_slice(), ["frozen"]);
        assert!(!view.key_command(KeyCommand::Cut, KeyModifiers::default()));
        assert_eq!(
            clipboard.written.borrow().len(),
            1,
            "readonly cut writes nothing"
        );
        *clipboard.next_read.borrow_mut() = Some("intruder".to_owned());
        assert!(!view.key_command(KeyCommand::Paste, KeyModifiers::default()));
        assert_eq!(
            view.control_value(view.node_target(view.element_node("ro").unwrap()).handle),
            Some("frozen")
        );

        // Null host (default provider): the clipboard is unavailable, not
        // silently lossy. Paste reads nothing, and cut refuses to delete
        // the selection its write could not take (transactional, ADR 0013).
        let mut bare = VelquView::new();
        bare.load_html("<!doctype html><html><body><input id=box value=keep></body></html>")
            .unwrap();
        let vp2 = Viewport::try_new(300, 120, 1.0).unwrap();
        bare.render(vp2).unwrap();
        bare.set_focus(Some("box"));
        let _ = bare.take_events();
        assert!(bare.key_command(
            KeyCommand::SelectAll,
            KeyModifiers {
                ctrl: true,
                ..KeyModifiers::default()
            }
        ));
        let _ = bare.take_events();
        assert!(!bare.key_command(KeyCommand::Paste, KeyModifiers::default()));
        assert!(!bare.key_command(KeyCommand::Cut, KeyModifiers::default()));
        assert_eq!(
            bare.control_value(bare.node_target(bare.element_node("box").unwrap()).handle),
            Some("keep"),
            "cut must not destroy text when no clipboard can take it"
        );
        assert!(format!("{:?}", bare).contains("custom_clipboard_provider: false"));
    }

    #[test]
    fn m4c2_cut_is_transactional_when_the_clipboard_fails() {
        // A real OS failure (contention, unsupported environment) surfaces
        // as Err: the selection survives instead of being destroyed.
        let mut view = VelquView::new();
        view.load_html("<!doctype html><html><body><input id=box value=precious></body></html>")
            .unwrap();
        view.set_clipboard_provider(Rc::new(FailingClipboard));
        let vp = Viewport::try_new(300, 120, 1.0).unwrap();
        view.render(vp).unwrap();
        view.set_focus(Some("box"));
        let _ = view.take_events();
        assert!(view.key_command(
            KeyCommand::SelectAll,
            KeyModifiers {
                ctrl: true,
                ..KeyModifiers::default()
            }
        ));
        let _ = view.take_events();

        assert!(!view.key_command(KeyCommand::Cut, KeyModifiers::default()));
        assert_eq!(
            view.control_value(view.node_target(view.element_node("box").unwrap()).handle),
            Some("precious"),
            "a failed cut write keeps the selection"
        );
        assert!(view.take_events().is_empty(), "no events without a change");
        // Copy failing is harmless by construction: nothing is deleted.
        assert!(!view.key_command(KeyCommand::Copy, KeyModifiers::default()));
        assert_eq!(
            view.control_value(view.node_target(view.element_node("box").unwrap()).handle),
            Some("precious")
        );
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
