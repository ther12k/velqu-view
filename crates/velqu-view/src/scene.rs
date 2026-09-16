//! Internal paint scene for the M1 renderer.
//!
//! A [`Scene`] is a flat list of primitives in logical (pre-DPI) coordinates.
//! The painter resolves logical to device pixels at raster time, which is what
//! makes resize/DPI a pure function of the [`crate::Viewport`].
//!
//! This is deliberately *not* a DOM: it exists so M1 can prove the paint
//! pipeline while M2 builds the real HTML/style/layout stack that will
//! lower into scenes like this one.

use crate::color::Color;

/// Text weight, resolved to one of the bundled faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FontWeight {
    /// The regular bundled face.
    Regular,
    /// The bold bundled face.
    Bold,
}

/// One paint primitive.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Item {
    /// Axis-aligned filled rectangle in logical pixels.
    Rect {
        /// Logical x of the left edge.
        x: f32,
        /// Logical y of the top edge.
        y: f32,
        /// Logical width.
        w: f32,
        /// Logical height.
        h: f32,
        /// Fill color.
        color: Color,
    },
    /// Rectangle outline (`thickness` grows inward) in logical pixels.
    RectOutline {
        /// Logical x of the left edge.
        x: f32,
        /// Logical y of the top edge.
        y: f32,
        /// Logical width.
        w: f32,
        /// Logical height.
        h: f32,
        /// Outline thickness in logical pixels.
        thickness: f32,
        /// Outline color.
        color: Color,
    },
    /// Left-aligned text block in logical pixels.
    Text {
        /// Logical x of the left edge of the text block.
        x: f32,
        /// Logical y of the top of the (first) line box.
        y: f32,
        /// Text; may contain `\n` for additional lines.
        text: String,
        /// Nominal glyph size in logical pixels.
        size: f32,
        /// Ink color.
        color: Color,
        /// Face selection.
        weight: FontWeight,
    },
}

/// A flat paint list plus a background.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Scene {
    /// Fill applied before any item.
    pub background: Color,
    /// Primitives, painted in order.
    pub items: Vec<Item>,
}

impl Scene {
    /// An empty scene over `background`.
    pub(crate) fn new(background: Color) -> Self {
        Self {
            background,
            items: Vec::new(),
        }
    }

    /// Appends a primitive, returning the scene for chaining.
    pub(crate) fn push(&mut self, item: Item) -> &mut Self {
        self.items.push(item);
        self
    }
}
