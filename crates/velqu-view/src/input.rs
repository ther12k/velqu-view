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

/// The public result of a hit test: what the pointer is over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitTarget {
    /// The element's HTML `id`, if any — the public interaction identity
    /// (matching scroll targets, ADR 0008).
    pub element_id: Option<String>,
    /// The element's tag name.
    pub tag: String,
    /// Is this box a scroll container (`overflow: auto`/`scroll`)?
    pub scroll_container: bool,
}

impl HitTarget {
    /// The scroll target key for this element: its `id`, or `None` (the
    /// document-level scroller) when it has none.
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
}

/// What a wheel gesture (or any scroll change) targets: the
/// document-level scroller or one container element. Node identity is the
/// runtime state key (ADR 0011); the HTML `id`, when present, is the
/// public identity carried in events.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WheelResult {
    /// `None` = the document-level scroller; `Some(node)` = the container.
    pub node: Option<NodeId>,
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
                element_id: None,
                offset,
                previous_applied: ctx.document_offset,
            })
        }
    }
}

/// What a scroll change targets (ADR 0011): the document-level scroller
/// or one scroll container. Runtime state is keyed by node identity; this
/// public shape carries the element's HTML `id` when it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrollTarget {
    /// The document-level scroller (the viewport).
    Document,
    /// A scroll container element — id-less containers are first-class
    /// targets too.
    Element {
        /// The container's HTML `id`, when it has one.
        id: Option<String>,
    },
}

/// One interaction event, in the order they occurred. Drained through
/// [`crate::VelquView::take_events`].
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The pointer moved off an element (`None` = the element had no id).
    PointerLeave {
        /// The element id just left.
        element: Option<String>,
    },
    /// The pointer moved onto an element (`None` = under no element or
    /// the element has no id).
    PointerEnter {
        /// The element id now under the pointer.
        element: Option<String>,
    },
    /// A pressed-then-released click on the same element.
    Click {
        /// The clicked element's id (`None` if it has none).
        element: Option<String>,
    },
    /// Focus moved between focusable elements (`None` = nothing focused).
    FocusChanged {
        /// Previously focused element id.
        from: Option<String>,
        /// Newly focused element id.
        to: Option<String>,
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
}
