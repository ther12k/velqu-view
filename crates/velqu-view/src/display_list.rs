//! The display list: the paint-ready output of layout, consumed by the
//! painter.
//!
//! This boundary (ADR 0005) keeps paint logic decoupled from the layout
//! representation: the painter sees only positioned primitives and makes no
//! layout decisions. M2a items are fills and text runs; borders are lowered
//! to four fill rects by layout. Clipping, rounded corners, and images
//! extend this list in later milestones.

use crate::color::Color;

/// A device-pixel rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A text run destined for painting (absolute device coordinates; `y` is
/// the baseline).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DisplayText {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub px: u16,
    pub bold: bool,
    pub color: Color,
}

/// A filled rectangle destined for painting (backgrounds, borders).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DisplayRect {
    pub rect: Rect,
    pub color: Color,
}

/// The paint-ready result of one layout pass.
#[derive(Debug, Clone, Default)]
pub(crate) struct DisplayList {
    /// Filled rectangles, painted first (in order).
    pub rects: Vec<DisplayRect>,
    /// Text runs, painted after rectangles.
    pub texts: Vec<DisplayText>,
}
