//! Frame-path instrumentation (`LEKUVO_PERF=1` or `VV_PERF=1`).
//!
//! Purely observational: it times the phases of [`crate::VelquView::render`]
//! (stylesheet assembly, cascade style compute, box build, Taffy, display
//! list emission, rasterization) and logs one compact line per frame to
//! stderr. It never reads or writes renderer state, so pixels, fixture
//! hashes, and determinism are unaffected. The env probe is cached in a
//! `OnceLock`; with the flag unset the struct is inert and render paths
//! only pay a handful of `Instant::now()` reads.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Which invalidation path a frame took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FramePath {
    /// Full structural pass (cascade + box tree + Taffy + display list).
    Structural,
    /// Presentation-only repaint (cascade + patch, zero Taffy passes).
    Presentation,
    /// Nothing changed: the cached display list was repainted unchanged.
    Cached,
}

impl FramePath {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FramePath::Structural => "structural",
            FramePath::Presentation => "presentation",
            FramePath::Cached => "cached",
        }
    }
}

/// Phase timings for one `render` call. Reset at the top of every render;
/// populated as the pipeline runs; logged at the end.
#[derive(Debug, Default)]
pub(crate) struct FramePerf {
    pub(crate) on: bool,
    /// Wall time between the start of consecutive render calls (the host's
    /// frame interval at the call site).
    pub(crate) interval: Option<Duration>,
    last_render: Option<Instant>,
    /// Assembling + parsing the cascade inputs (UA sheet, generated
    /// utilities, author sheets, `<style>` blocks).
    pub(crate) css_parse: Duration,
    /// Cascade style compute that runs outside the layout pass (pending
    /// transition diffs) plus the presentation-repaint compute/patch.
    pub(crate) style: Duration,
    /// The whole structural layout pass (`run_layout`), including the
    /// sub-phases below.
    pub(crate) layout_total: Duration,
    pub(crate) style_in_pass: Duration,
    pub(crate) build_boxes: Duration,
    pub(crate) taffy: Duration,
    pub(crate) emit: Duration,
    /// The presentation-repaint branch (`repaint_presentation`).
    pub(crate) presentation: Duration,
    /// Rasterization (`painter::paint_document`).
    pub(crate) paint: Duration,
    /// Scratch: ring scan pixels reported by the painter.
    pub(crate) ring: u64,
    /// Scratch: the damage rect passed to the painter.
    pub(crate) damage_rect: Option<(i64, i64, i64, i64)>,
    pub(crate) items: usize,
    pub(crate) glyphs: usize,
    /// Damage actually rasterized: full-surface paints report 1 rect of
    /// the whole viewport; a scoped painter reports its resolved rects.
    pub(crate) damage_rects: usize,
    pub(crate) damage_area: u64,
    pub(crate) viewport: (u32, u32),
    pub(crate) path: Option<FramePath>,
}

impl FramePerf {
    /// Constructs the recorder with the env probe applied.
    pub(crate) fn new() -> Self {
        Self {
            on: Self::enabled_from_env(),
            ..Self::default()
        }
    }

    /// Env probe, computed once per process. Any value (including empty)
    /// enables the logs; `0` disables.
    pub(crate) fn enabled_from_env() -> bool {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        *ENABLED.get_or_init(|| {
            ["LEKUVO_PERF", "VV_PERF"].iter().any(|key| {
                std::env::var_os(key).is_some_and(|value| value != "0")
            })
        })
    }

    /// Starts a frame: resets the accumulators and stamps the interval.
    pub(crate) fn begin(&mut self) {
        let now = Instant::now();
        self.interval = self.last_render.map(|last| now.duration_since(last));
        self.last_render = Some(now);
        self.css_parse = Duration::ZERO;
        self.style = Duration::ZERO;
        self.layout_total = Duration::ZERO;
        self.style_in_pass = Duration::ZERO;
        self.build_boxes = Duration::ZERO;
        self.taffy = Duration::ZERO;
        self.emit = Duration::ZERO;
        self.presentation = Duration::ZERO;
        self.paint = Duration::ZERO;
        self.items = 0;
        self.glyphs = 0;
        self.damage_rects = 0;
        self.damage_area = 0;
        self.damage_rect = None;
        self.viewport = (0, 0);
        self.path = None;
    }

    /// Emits the one-line frame record (only when enabled).
    pub(crate) fn log(&self, frame_index: u64) {
        if !self.on {
            return;
        }
        let ms = |d: Duration| format!("{:.2}", d.as_secs_f64() * 1e3);
        let interval = self
            .interval
            .map(|d| format!("{:.2}", d.as_secs_f64() * 1e3))
            .unwrap_or_else(|| "-".to_owned());
        eprintln!(
            "vv-perf f={frame_index} path={} css={}ms style={}ms layout={}ms (style={}ms boxes={}ms taffy={}ms emit={}ms) present={}ms paint={}ms items={} glyphs={} damage={}rects/{}px {:?} ring={} viewport={}x{} interval={}ms",
            self.path.map(FramePath::as_str).unwrap_or("none"),
            ms(self.css_parse),
            ms(self.style),
            ms(self.layout_total),
            ms(self.style_in_pass),
            ms(self.build_boxes),
            ms(self.taffy),
            ms(self.emit),
            ms(self.presentation),
            ms(self.paint),
            self.items,
            self.glyphs,
            self.damage_rects,
            self.damage_area,
            self.damage_rect,
            self.ring,
            self.viewport.0,
            self.viewport.1,
            interval,
        );
    }
}

/// Optional sub-phase timings filled by `layout::layout_document` when the
/// caller (the render loop) wants the breakdown; `None` from other callers.
#[derive(Debug, Default)]
pub(crate) struct PassTimings {
    pub(crate) style: Duration,
    pub(crate) boxes: Duration,
    pub(crate) taffy: Duration,
    pub(crate) emit: Duration,
}
