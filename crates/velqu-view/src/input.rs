//! The input gate (M4a, ADR 0010): hit testing and runtime interaction
//! state on top of the laid-out box tree.
//!
//! Hit testing is the inverse of painting: later siblings win over
//! earlier ones, children win over their parent's chrome, clip scopes
//! exclude hits outside them, and scroll containers translate hit points
//! by their applied (clamped) scroll offset — the same math the painter
//! applies, run backwards. Geometry stays the unscrolled layout truth;
//! input never triggers layout.
//!
//! Interaction state (hover, focus, click) is **runtime presentation
//! state** like scroll offsets: it produces events and can drive later
//! styling milestones, but it changes no layout facts.

use crate::display_list::Rect;
use crate::dom::NodeId;
use crate::layout::{BoxNode, ScrollExtent};
use crate::viewport::Viewport;

/// Opaque identity for an element in the currently loaded document.
///
/// A handle is stable across resize/restyle layout rebuilds, but is invalid
/// after the document is replaced. Its fields and constructor stay private so
/// callers cannot manufacture a handle for another document or node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ElementHandle {
    generation: u64,
    node: NodeId,
}

impl ElementHandle {
    pub(crate) fn new(generation: u64, node: NodeId) -> Self {
        Self { generation, node }
    }

    pub(crate) fn generation(self) -> u64 {
        self.generation
    }

    pub(crate) fn node(self) -> NodeId {
        self.node
    }
}

/// A public element reference carrying both opaque identity and author id.
///
/// The id is descriptive and optional; the handle is the identity that stays
/// unambiguous for id-less elements and across author-id collisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementTarget {
    /// Opaque identity valid for the current document only.
    pub handle: ElementHandle,
    /// The element's HTML `id`, if any.
    pub id: Option<String>,
}

/// The public result of a hit test: what the pointer is over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitTarget {
    /// Opaque identity valid for the current document only.
    pub handle: ElementHandle,
    /// The element's HTML `id`, if any — a descriptive public attribute,
    /// not the interaction identity.
    pub element_id: Option<String>,
    /// The element's tag name.
    pub tag: String,
    /// Is this box a scroll container (`overflow: auto`/`scroll`)?
    pub scroll_container: bool,
}

impl HitTarget {
    /// The author-provided HTML `id`, when present.
    ///
    /// This is a convenience lookup only. Use [`HitTarget::handle`] as the
    /// unambiguous identity, especially for id-less elements.
    pub fn scroll_key(&self) -> Option<&str> {
        self.element_id.as_deref()
    }
}

fn contains(rect: &Rect, x: f32, y: f32) -> bool {
    x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
}

/// Finds the topmost box at `(x, y)` (absolute device px, viewport space).
///
/// `document_scroll` is the clamped document-level scroll offset: the
/// caller's point is in *viewport* space, so descending into the page
/// translates by it. Returns the deepest box whose chrome or content is
/// under the point, honoring paint order, clip scopes, and per-container
/// scroll offsets.
pub(crate) fn hit_at(
    root: &BoxNode,
    document_scroll: (f32, f32),
    x: f32,
    y: f32,
) -> Option<&BoxNode> {
    // Enter the page in content space (undo the document scroll).
    hit_in(root, x + document_scroll.0, y + document_scroll.1)
}

/// Maps a viewport point into the target node's own geometry space by
/// reversing the same document and nested-scroll transforms used by hit test.
/// It intentionally ignores clipping so an active pointer capture can keep
/// updating selection after the pointer leaves the control.
pub(crate) fn point_in_node(
    root: &BoxNode,
    document_scroll: (f32, f32),
    target: NodeId,
    x: f32,
    y: f32,
) -> Option<(f32, f32)> {
    fn walk(node: &BoxNode, target: NodeId, x: f32, y: f32) -> Option<(f32, f32)> {
        if node.node == target {
            return Some((x, y));
        }
        let (ox, oy) = node.applied_scroll;
        for child in &node.children {
            if let Some(point) = walk(child, target, x + ox, y + oy) {
                return Some(point);
            }
        }
        None
    }
    walk(root, target, x + document_scroll.0, y + document_scroll.1)
}

/// The translation `point_in_node` would apply for `target`: the document
/// scroll plus every ancestor's applied scroll. Subtracting it from a
/// node-space point yields viewport space (M4c3: the IME candidate rect).
pub(crate) fn accumulated_scroll_offset(
    root: &BoxNode,
    document_scroll: (f32, f32),
    target: NodeId,
) -> Option<(f32, f32)> {
    fn walk(node: &BoxNode, target: NodeId, x: f32, y: f32) -> Option<(f32, f32)> {
        if node.node == target {
            return Some((x, y));
        }
        let (ox, oy) = node.applied_scroll;
        for child in &node.children {
            if let Some(found) = walk(child, target, x + ox, y + oy) {
                return Some(found);
            }
        }
        None
    }
    walk(root, target, document_scroll.0, document_scroll.1)
}

/// Recursive hit walk in the given coordinate space. `(x, y)` is in the
/// space the node's own geometry lives in; scrolling translates the point
/// only when *descending into children* (their geometry is content-space),
/// while the clip test and own-chrome test use the incoming point.
fn hit_in(node: &BoxNode, x: f32, y: f32) -> Option<&BoxNode> {
    let clips = node.style.overflow_y != crate::style::Overflow::Visible
        || node.style.overflow_x != crate::style::Overflow::Visible;
    let (ox, oy) = node.applied_scroll;

    // Children, topmost (later) first — inside this container's clip, in
    // its content space.
    if !clips || contains(&node.padding_box, x, y) {
        for child in node.children.iter().rev() {
            if let Some(hit) = hit_in(child, x + ox, y + oy) {
                return Some(hit);
            }
        }
    }

    // Own chrome (background/border) is below all children and is not
    // clipped by the box's own overflow.
    if contains(&node.border_box, x, y) {
        Some(node)
    } else {
        None
    }
}

/// Walks from `root` down to `target`, returning the nearest ancestor (or
/// the target itself) that is a scroll container. Used by wheel input to
/// decide what a scroll gesture scrolls.
pub(crate) fn nearest_scrollable_ancestor<'a>(
    root: &'a BoxNode,
    target: &'a BoxNode,
) -> Option<&'a BoxNode> {
    fn walk<'a>(node: &'a BoxNode, target: &'a BoxNode, found: &mut Option<&'a BoxNode>) -> bool {
        if std::ptr::eq(node, target) {
            if node.style.overflow_y.is_scroll_container() {
                *found = Some(node);
            }
            return true;
        }
        for child in &node.children {
            if walk(child, target, found) {
                // On the way back up, keep the *nearest* container: only
                // set when still unset.
                if node.style.overflow_y.is_scroll_container() && found.is_none() {
                    *found = Some(node);
                }
                return true;
            }
        }
        false
    }
    let mut found = None;
    walk(root, target, &mut found);
    found
}

/// The document-level context a wheel gesture resolves against: the
/// cached laid-out tree, the clamped document offset it was painted with,
/// the document's scrollable extent, and the viewport (the document
/// scroller's scrollport).
pub(crate) struct WheelContext<'a> {
    pub root: &'a BoxNode,
    pub document_offset: (f32, f32),
    pub document_extent: ScrollExtent,
    pub viewport: Viewport,
    pub generation: u64,
}

/// What a wheel gesture (or any scroll change) targets: the
/// document-level scroller or one container element. Node identity is the
/// runtime state key (ADR 0011); the HTML `id`, when present, is the
/// public identity carried in events.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WheelResult {
    /// `None` = the document-level scroller; `Some(node)` = the container.
    pub node: Option<NodeId>,
    /// The container's opaque public identity, when it has one.
    pub handle: Option<ElementHandle>,
    /// The container's HTML `id`, when it has one.
    pub element_id: Option<String>,
    /// The new clamped offset.
    pub offset: (f32, f32),
    /// The offset the target was painted with before this gesture — the
    /// change baseline (a stored raw request can exceed the current clamp
    /// after a relayout).
    pub previous_applied: (f32, f32),
}

/// Computes the wheel result at `(x, y)` (viewport px): hit test, find the
/// nearest scrollable ancestor (or the document-level scroller), add the
/// delta to that container's current clamped offset, and clamp again.
/// Returns `None` when nothing is under the point. No layout runs; the
/// math reads the cached laid-out tree only.
pub(crate) fn wheel_target(
    ctx: &WheelContext<'_>,
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
) -> Option<WheelResult> {
    let node = hit_at(ctx.root, ctx.document_offset, x, y)?;
    match nearest_scrollable_ancestor(ctx.root, node) {
        Some(container) => {
            let extent = container.scroll?;
            let offset = crate::layout::clamp_scroll_offset(
                (
                    container.applied_scroll.0 + dx,
                    container.applied_scroll.1 + dy,
                ),
                (extent.width, extent.height),
                (container.padding_box.w, container.padding_box.h),
            );
            Some(WheelResult {
                node: Some(container.node),
                handle: Some(ElementHandle::new(ctx.generation, container.node)),
                element_id: container.element_id.clone(),
                offset,
                previous_applied: container.applied_scroll,
            })
        }
        None => {
            let offset = crate::layout::clamp_scroll_offset(
                (ctx.document_offset.0 + dx, ctx.document_offset.1 + dy),
                (ctx.document_extent.width, ctx.document_extent.height),
                (ctx.viewport.width() as f32, ctx.viewport.height() as f32),
            );
            Some(WheelResult {
                node: None,
                handle: None,
                element_id: None,
                offset,
                previous_applied: ctx.document_offset,
            })
        }
    }
}

/// What a scroll change targets (ADR 0011): the document-level scroller
/// or one scroll container. Runtime state is keyed by node identity; this
/// public shape carries the opaque element identity and its optional HTML id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrollTarget {
    /// The document-level scroller (the viewport).
    Document,
    /// A scroll container element — id-less containers are first-class
    /// targets too.
    Element {
        /// Opaque identity valid for the current document only.
        handle: ElementHandle,
        /// The container's HTML `id`, when it has one.
        id: Option<String>,
    },
}

/// Why focus moved (M4b, ADR 0011): keyboard-only focus rings,
/// accessibility behavior, and native-feeling text controls all need the
/// origin — which is unrecoverable after the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusOrigin {
    /// The pointer clicked the element.
    Pointer,
    /// Keyboard navigation (Tab / Shift+Tab).
    Keyboard,
    /// A programmatic `set_focus` call.
    Programmatic,
}

/// One interaction event, in the order they occurred. Drained through
/// [`crate::VelquView::take_events`].
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The pointer moved off an element.
    PointerLeave {
        /// The element's opaque identity.
        target: ElementTarget,
    },
    /// The pointer moved onto an element.
    PointerEnter {
        /// The element's opaque identity.
        target: ElementTarget,
    },
    /// A pressed-then-released click on the same element.
    Click {
        /// The clicked element's opaque identity.
        target: ElementTarget,
    },
    /// Focus moved between focusable elements (`None` = nothing focused).
    FocusChanged {
        /// Previously focused element.
        from: Option<ElementTarget>,
        /// Newly focused element.
        to: Option<ElementTarget>,
        /// What moved focus.
        origin: FocusOrigin,
    },
    /// A scroll container's clamped offset changed (wheel input or
    /// programmatic).
    Scrolled {
        /// What scrolled.
        target: ScrollTarget,
        /// Clamped x offset in device px.
        x: f32,
        /// Clamped y offset in device px.
        y: f32,
    },
    /// A control's current runtime value changed.
    ValueChanged {
        /// The edited control's opaque identity and optional HTML id.
        target: ElementTarget,
        /// Current value, never written into the DOM.
        value: String,
    },
    /// A control's selection or caret moved.
    SelectionChanged {
        /// The control whose selection changed.
        target: ElementTarget,
        /// Selection anchor as a valid UTF-8 byte offset.
        anchor: usize,
        /// Selection focus/caret as a valid UTF-8 byte offset.
        focus: usize,
    },
}
