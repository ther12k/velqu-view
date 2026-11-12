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

/// A 2D affine transform in CSS `matrix(a, b, c, d, e, f)` layout:
/// `x' = a·x + c·y + e`, `y' = b·x + d·y + f` (ADR 0024). Layout emits
/// only translate + positive scale (never rotation or skew), so rects
/// map to rects and pixel-edge snapping survives; the painter still
/// treats it as a general affine so later slices stay source-compatible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Affine2D {
    /// x-scale.
    pub a: f32,
    /// y-shear (always 0 today).
    pub b: f32,
    /// x-shear (always 0 today).
    pub c: f32,
    /// y-scale.
    pub d: f32,
    /// x-translation.
    pub e: f32,
    /// y-translation.
    pub f: f32,
}

impl Default for Affine2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine2D {
    /// No-op transform.
    pub(crate) const IDENTITY: Affine2D = Affine2D {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// Pure translation (the scroll-transform case, ADR 0008).
    pub(crate) const fn translation(x: f32, y: f32) -> Self {
        Affine2D {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: x,
            f: y,
        }
    }

    /// Independent axis scales plus a translation (the v0 `Transform2D`
    /// emission, ADR 0024). Scale is applied about the origin; layout
    /// pre-bakes the element-center origin into the translation.
    pub(crate) const fn translate_scale(scale_x: f32, scale_y: f32, tx: f32, ty: f32) -> Self {
        Affine2D {
            a: scale_x,
            b: 0.0,
            c: 0.0,
            d: scale_y,
            e: tx,
            f: ty,
        }
    }

    /// Independent axis scales, rotation in degrees, transform origin (ox, oy),
    /// and translation (tx, ty). Composes scaling and rotation about (ox, oy)
    /// followed by translation (tx, ty).
    pub(crate) fn translate_scale_rotate(
        scale_x: f32,
        scale_y: f32,
        rotate_deg: f32,
        ox: f32,
        oy: f32,
        tx: f32,
        ty: f32,
    ) -> Self {
        if rotate_deg == 0.0 {
            return Self::translate_scale(
                scale_x,
                scale_y,
                ox * (1.0 - scale_x) + tx,
                oy * (1.0 - scale_y) + ty,
            );
        }
        let rad = rotate_deg.to_radians();
        let cos = rad.cos();
        let sin = rad.sin();
        let a = scale_x * cos;
        let b = scale_x * sin;
        let c = -scale_y * sin;
        let d = scale_y * cos;
        let e = ox - (a * ox + c * oy) + tx;
        let f = oy - (b * ox + d * oy) + ty;
        Affine2D { a, b, c, d, e, f }
    }

    /// Inverts this 2D affine matrix if its determinant is non-zero (non-degenerate).
    /// Used for transformed hit testing and inverse-mapped rasterization.
    pub(crate) fn inverse(&self) -> Option<Affine2D> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-6 || !det.is_finite() {
            return None;
        }
        let inv_det = 1.0 / det;
        Some(Affine2D {
            a: self.d * inv_det,
            b: -self.b * inv_det,
            c: -self.c * inv_det,
            d: self.a * inv_det,
            e: (self.c * self.f - self.d * self.e) * inv_det,
            f: (self.b * self.e - self.a * self.f) * inv_det,
        })
    }

    /// True when the transform does not introduce rotation or shearing
    /// (b == 0 and c == 0), so rectangles map to axis-aligned rectangles.
    pub(crate) fn is_axis_aligned(&self) -> bool {
        self.b.abs() < 1e-6 && self.c.abs() < 1e-6
    }

    /// `self ∘ inner`: applies `inner` first, then `self` (pushing a
    /// transform scope composes this way — outer context wraps inner).
    pub(crate) fn compose(&self, inner: &Affine2D) -> Affine2D {
        Affine2D {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    /// Maps one point.
    pub(crate) fn apply_point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// Maps a rect through opposite corners, normalizing the result to
    /// positive extents (exact for translate+scale: `w·1.0` is `w`).
    pub(crate) fn apply_rect(&self, rect: &Rect) -> Rect {
        let (x0, y0) = self.apply_point(rect.x, rect.y);
        let (x1, y1) = self.apply_point(rect.x + rect.w, rect.y + rect.h);
        Rect {
            x: x0.min(x1),
            y: y0.min(y1),
            w: (x1 - x0).abs(),
            h: (y1 - y0).abs(),
        }
    }

    /// The uniform-ish scale factor (radii and text sizes scale by it).
    pub(crate) fn scale_factor(&self) -> f32 {
        if self.is_axis_aligned() {
            self.a.abs()
        } else {
            (self.a * self.a + self.b * self.b).sqrt()
        }
    }
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
        /// Normalized `font-family` (`None` = bundled default face).
        family: Option<String>,
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
    /// Applies an affine transform to all items until the matching
    /// [`DisplayItem::PopTransform`] (M2c scrolling: the negated scroll
    /// offset as a translation; ADR 0024: paint-time element transforms).
    /// Scopes nest; clips pushed inside a transform scope are positioned in
    /// the *parent* space, items in the transformed space.
    PushTransform { matrix: Affine2D },
    /// Ends the innermost transform scope.
    PopTransform,
    /// Multiplies the source alpha of every item until the matching
    /// [`DisplayItem::PopOpacity`] by `alpha ∈ [0, 1]` (ADR 0024: the
    /// paint-time `opacity` property and its transitions). Scopes nest.
    PushOpacity(f32),
    /// Ends the innermost opacity scope.
    PopOpacity,
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
                    family: _,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_inverse_identity_and_translation() {
        let id = Affine2D::IDENTITY;
        let inv = id.inverse().expect("identity has inverse");
        assert_eq!(inv, Affine2D::IDENTITY);

        let t = Affine2D::translation(30.0, -50.0);
        let inv = t.inverse().expect("translation has inverse");
        let composed = t.compose(&inv);
        assert!((composed.a - 1.0).abs() < 1e-5);
        assert!((composed.d - 1.0).abs() < 1e-5);
        assert!(composed.e.abs() < 1e-5);
        assert!(composed.f.abs() < 1e-5);
    }

    #[test]
    fn affine_inverse_scale_and_rotation() {
        let rot = Affine2D::translate_scale_rotate(2.0, 3.0, 90.0, 10.0, 20.0, 5.0, 5.0);
        let inv = rot.inverse().expect("scale+rotate has inverse");
        let p = (15.0, 25.0);
        let mapped = rot.apply_point(p.0, p.1);
        let back = inv.apply_point(mapped.0, mapped.1);
        assert!((back.0 - p.0).abs() < 1e-4, "x: {} vs {}", back.0, p.0);
        assert!((back.1 - p.1).abs() < 1e-4, "y: {} vs {}", back.1, p.1);
    }
}
