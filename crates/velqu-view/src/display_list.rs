//! The display list: the paint-ready output of layout, consumed by the
//! painter.
//!
//! This boundary (ADR 0005) keeps paint logic decoupled from the layout
//! representation: the painter sees only positioned primitives plus clip
//! scoping and makes no layout decisions. Layout owns where things are; the
//! display list owns paint ordering and clipping; the painter executes.
//!
//! M2b items: fills, text runs, and clip scoping (`PushClip`/`PopClip`) for
//! `overflow: hidden`/`clip` boxes. Borders are lowered to fills by layout.
//! M2c adds `DrawImage` (replaced content, ADR 0008) and
//! `PushTransform`/`PopTransform` (scroll offsets) without disturbing the
//! division of labor: layout geometry stays unscrolled; the transform
//! carries only the runtime scroll offset.

use std::rc::Rc;

use crate::color::Color;
use crate::image::DecodedImage;

/// A device-pixel rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// One paint-ready item, in paint order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DisplayItem {
    /// Filled rectangle (backgrounds, borders).
    FillRect { rect: Rect, color: Color },
    /// A text run; `y` is the baseline (absolute device coordinates).
    TextRun {
        x: f32,
        y: f32,
        text: String,
        px: u16,
        bold: bool,
        color: Color,
    },
    /// Constrains all items until the matching [`DisplayItem::PopClip`] to
    /// this rectangle (intersected with any enclosing clip).
    PushClip(Rect),
    /// [`DisplayItem::PushClip`] with rounded corners (M3: `overflow`
    /// clipping on a box that also has `border-radius`). Pixel coverage
    /// follows the rounded shape; the rect still intersects enclosing
    /// clips.
    PushClipRounded { rect: Rect, radius: f32 },
    /// Ends the innermost clip scope.
    PopClip,
    /// Blits a decoded image into `rect` (nearest-neighbor, `object-fit:
    /// fill`). The pixels travel with the item so the painter stays
    /// stateless beyond its font store.
    DrawImage { rect: Rect, image: Rc<DecodedImage> },
    /// Filled rectangle with rounded corners (M3: `background-color` +
    /// `border-radius`). The rect edges are the shape's bounds; coverage
    /// is a deterministic per-pixel test, no anti-aliasing.
    RoundedFill {
        rect: Rect,
        radius: f32,
        color: Color,
    },
    /// A border ring with rounded corners (M3): pixels inside `outer`
    /// (corner `radius`) and outside `inner` (per-corner `inner_radii`,
    /// order top-left, top-right, bottom-right, bottom-left) paint.
    RoundedBorder {
        outer: Rect,
        inner: Rect,
        radius: f32,
        inner_radii: [f32; 4],
        color: Color,
    },
    /// Translates all items until the matching [`DisplayItem::PopTransform`]
    /// by `(x, y)` device px (M2c scrolling: the negated scroll offset).
    /// Scopes nest; clips pushed inside a transform scope are positioned in
    /// the *parent* space, items in the translated space.
    PushTransform { x: f32, y: f32 },
    /// Ends the innermost transform scope.
    PopTransform,
}

/// The paint-ready result of one layout pass.
#[derive(Debug, Clone, Default)]
pub(crate) struct DisplayList {
    pub items: Vec<DisplayItem>,
}

impl DisplayList {
    /// Convenience for tests: all text runs in paint order (cloned).
    #[allow(dead_code)]
    pub(crate) fn text_runs(&self) -> Vec<DisplayText> {
        self.items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::TextRun {
                    x,
                    y,
                    text,
                    px,
                    bold,
                    color,
                } => Some(DisplayText {
                    x: *x,
                    y: *y,
                    text: text.clone(),
                    px: *px,
                    bold: *bold,
                    color: *color,
                }),
                _ => None,
            })
            .collect()
    }
}

/// A text run destined for painting (absolute device coordinates; `y` is
/// the baseline). Borrowing view used by [`DisplayList::text_runs`].
/// Consumed by layout tests and the future inspector.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DisplayText {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub px: u16,
    pub bold: bool,
    pub color: Color,
}
