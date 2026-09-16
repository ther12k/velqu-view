//! CPU rasterizer for display lists produced by the layout pass.
//!
//! The painter makes **no layout decisions** (ADR 0005): it consumes a
//! finished [`DisplayList`] — fills, text runs, clip scoping — plus a
//! background color and produces pixels. Clip scopes
//! ([`DisplayItem::PushClip`]/[`DisplayItem::PopClip`]) intersect with the
//! running clip stack; every item is bounded by the innermost clip.
//!
//! Determinism rules (fixture hashes depend on them):
//! * rectangles are solid fills snapped to device-pixel edges — no
//!   anti-aliasing, so coverage math cannot vary;
//! * glyph coverage comes from fontdue's scalar rasterizer at whole-pixel
//!   sizes (see `font.rs`);
//! * frame buffers are pixel-bounded upstream and allocated fallibly.

use crate::color::Color;
use crate::display_list::{DisplayItem, DisplayList, Rect};
use crate::font::FontStore;
use crate::viewport::Viewport;
use crate::{Frame, VelquError};

/// Paints `list` over `background` into a frame for `viewport`.
///
/// Returns the frame, the number of items painted, and the number of glyphs
/// composited (reported through [`crate::RenderStats`]).
pub(crate) fn paint_document(
    list: &DisplayList,
    background: Color,
    viewport: Viewport,
    fonts: &mut FontStore,
) -> Result<(Frame, usize, usize), VelquError> {
    let byte_count = u64::from(viewport.width()) * u64::from(viewport.height()) * 4;
    let byte_count =
        usize::try_from(byte_count).map_err(|_| VelquError::FrameAllocationFailed {
            width: viewport.width(),
            height: viewport.height(),
        })?;
    let mut rgba: Vec<u8> = Vec::new();
    rgba.try_reserve_exact(byte_count)
        .map_err(|_| VelquError::FrameAllocationFailed {
            width: viewport.width(),
            height: viewport.height(),
        })?;
    rgba.resize(byte_count, 0);

    for px in rgba.chunks_exact_mut(4) {
        px[0] = background.r;
        px[1] = background.g;
        px[2] = background.b;
        px[3] = background.a;
    }

    let mut ctx = PaintCtx {
        width: viewport.width(),
        height: viewport.height(),
        rgba: &mut rgba,
        glyphs: 0,
        items: 0,
        clip: Vec::new(),
        offset: (0.0, 0.0),
        offset_stack: Vec::new(),
    };

    for item in &list.items {
        match item {
            DisplayItem::FillRect { rect, color } => {
                ctx.fill_rect(rect.x, rect.y, rect.w, rect.h, *color);
                ctx.items += 1;
            }
            DisplayItem::RoundedFill {
                rect,
                radius,
                color,
            } => {
                ctx.fill_rounded(rect, *radius, *color);
                ctx.items += 1;
            }
            DisplayItem::RoundedBorder {
                outer,
                inner,
                radius,
                inner_radii,
                color,
            } => {
                ctx.fill_rounded_ring(outer, *radius, inner, inner_radii, *color);
                ctx.items += 1;
            }
            DisplayItem::TextRun {
                x,
                y,
                text,
                px,
                bold,
                color,
            } => {
                ctx.draw_text(
                    fonts,
                    &TextRunPlacement {
                        x: *x,
                        baseline_y: *y,
                        text,
                        px: *px,
                        bold: *bold,
                        color: *color,
                    },
                );
                ctx.items += 1;
            }
            DisplayItem::PushClip(rect) => ctx.push_clip(*rect, 0.0),
            DisplayItem::PushClipRounded { rect, radius } => ctx.push_clip(*rect, *radius),
            DisplayItem::PopClip => ctx.pop_clip(),
            DisplayItem::DrawImage { rect, image } => {
                ctx.draw_image(rect, image);
                ctx.items += 1;
            }
            DisplayItem::PushTransform { x, y } => ctx.push_transform(*x, *y),
            DisplayItem::PopTransform => ctx.pop_transform(),
        }
    }

    let glyphs = ctx.glyphs;
    let items = ctx.items;
    Ok((
        Frame::from_parts(viewport.width(), viewport.height(), rgba),
        items,
        glyphs,
    ))
}

struct PaintCtx<'a> {
    width: u32,
    height: u32,
    rgba: &'a mut [u8],
    glyphs: usize,
    items: usize,
    /// Clip stack: every active scope bounds every pixel (a pixel must be
    /// inside all of them; balanced push/pop emission keeps the stack
    /// correct for nesting).
    clip: Vec<ClipShape>,
    /// Current translation (accumulated scroll transforms, ADR 0008) plus
    /// the saved offsets of enclosing scopes.
    offset: (f32, f32),
    offset_stack: Vec<(f32, f32)>,
}

/// One active clip scope: a rectangle in device pixels, optionally with
/// rounded corners.
#[derive(Debug, Clone, Copy)]
struct ClipShape {
    rect: Rect,
    radius: f32,
}

impl PaintCtx<'_> {
    /// Enters a transform scope: items are translated by `(x, y)` until the
    /// matching `pop_transform`.
    fn push_transform(&mut self, x: f32, y: f32) {
        self.offset_stack.push(self.offset);
        self.offset.0 += x;
        self.offset.1 += y;
    }

    /// Leaves the innermost transform scope, restoring the enclosing
    /// translation. Emission is balanced; a stray pop is a harmless no-op.
    fn pop_transform(&mut self) {
        if let Some(offset) = self.offset_stack.pop() {
            self.offset = offset;
        }
    }

    /// Pushes a clip scope. Clip shapes live in the space of the scope
    /// they are pushed in (scroll containers clip in parent space), so the
    /// current translation applies.
    fn push_clip(&mut self, rect: Rect, radius: f32) {
        self.clip.push(ClipShape {
            rect: Rect {
                x: rect.x + self.offset.0,
                y: rect.y + self.offset.1,
                w: rect.w,
                h: rect.h,
            },
            radius: radius.max(0.0),
        });
    }

    /// Ends the innermost clip scope, restoring the enclosing one. M2c
    /// emission is balanced; a stray PopClip is a harmless no-op.
    fn pop_clip(&mut self) {
        self.clip.pop();
    }

    fn clip_contains(&self, x: i64, y: i64) -> bool {
        self.clip.iter().all(|shape| shape_contains(shape, x, y))
    }

    fn blend_pixel(&mut self, x: usize, y: usize, color: Color) {
        let i = (y * self.width as usize + x) * 4;
        let dst = Color::from_rgba8(
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        );
        let out = color.blend_over(dst);
        self.rgba[i] = out.r;
        self.rgba[i + 1] = out.g;
        self.rgba[i + 2] = out.b;
        self.rgba[i + 3] = out.a;
    }

    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        // Rects arrive in layout space; the current translation (scroll
        // scopes) applies before edge snapping.
        let x = x + self.offset.0;
        let y = y + self.offset.1;
        // Snap the edges only.
        let x0 = x.round().max(0.0) as u32;
        let y0 = y.round().max(0.0) as u32;
        let x1 = (x + w).round().clamp(0.0, self.width as f32) as u32;
        let y1 = (y + h).round().clamp(0.0, self.height as f32) as u32;
        for py in y0..y1 {
            for px in x0..x1 {
                if self.clip_contains(px as i64, py as i64) {
                    self.blend_pixel(px as usize, py as usize, color);
                }
            }
        }
    }

    /// Fills a rounded rectangle: same edge snapping as `fill_rect`, with
    /// per-pixel shape coverage (pixel **center** inside the rounded
    /// bounds). No anti-aliasing — coverage is a strict in/out test, so
    /// results are deterministic.
    fn fill_rounded(&mut self, rect: &Rect, radius: f32, color: Color) {
        let placed = Rect {
            x: rect.x + self.offset.0,
            y: rect.y + self.offset.1,
            w: rect.w,
            h: rect.h,
        };
        let x0 = placed.x.round().max(0.0) as i64;
        let y0 = placed.y.round().max(0.0) as i64;
        let x1 = (placed.x + placed.w).round().clamp(0.0, self.width as f32) as i64;
        let y1 = (placed.y + placed.h).round().clamp(0.0, self.height as f32) as i64;
        for py in y0..y1 {
            for px in x0..x1 {
                if !self.clip_contains(px, py) {
                    continue;
                }
                let cx = px as f32 + 0.5;
                let cy = py as f32 + 0.5;
                if point_in_rounded_rect(cx, cy, &placed, radius) {
                    self.blend_pixel(px as usize, py as usize, color);
                }
            }
        }
    }

    /// Fills a rounded border ring: pixels inside the outer rounded shape
    /// and outside the inner one. Inner corner radii are the outer radius
    /// minus the adjacent border widths (clamped at 0), so uneven borders
    /// keep a sensible ring.
    fn fill_rounded_ring(
        &mut self,
        outer: &Rect,
        radius: f32,
        inner: &Rect,
        inner_radii: &[f32; 4],
        color: Color,
    ) {
        let placed_outer = Rect {
            x: outer.x + self.offset.0,
            y: outer.y + self.offset.1,
            w: outer.w,
            h: outer.h,
        };
        let placed_inner = Rect {
            x: inner.x + self.offset.0,
            y: inner.y + self.offset.1,
            w: inner.w,
            h: inner.h,
        };
        let x0 = placed_outer.x.round().max(0.0) as i64;
        let y0 = placed_outer.y.round().max(0.0) as i64;
        let x1 = (placed_outer.x + placed_outer.w)
            .round()
            .clamp(0.0, self.width as f32) as i64;
        let y1 = (placed_outer.y + placed_outer.h)
            .round()
            .clamp(0.0, self.height as f32) as i64;
        for py in y0..y1 {
            for px in x0..x1 {
                if !self.clip_contains(px, py) {
                    continue;
                }
                let cx = px as f32 + 0.5;
                let cy = py as f32 + 0.5;
                let in_outer = point_in_rounded_rect(cx, cy, &placed_outer, radius);
                let in_inner = point_in_rounded_rect4(cx, cy, &placed_inner, inner_radii);
                if in_outer && !in_inner {
                    self.blend_pixel(px as usize, py as usize, color);
                }
            }
        }
    }

    /// Blits a decoded image into `rect` with nearest-neighbor sampling
    /// (`object-fit: fill`). Edges snap like fills; source indices come from
    /// integer math only, so sampling is deterministic across runs.
    fn draw_image(&mut self, rect: &Rect, image: &crate::image::DecodedImage) {
        let x = rect.x + self.offset.0;
        let y = rect.y + self.offset.1;
        let x0 = x.round().max(0.0) as i64;
        let y0 = y.round().max(0.0) as i64;
        let x1 = (x + rect.w).round().clamp(0.0, self.width as f32) as i64;
        let y1 = (y + rect.h).round().clamp(0.0, self.height as f32) as i64;
        let dst_w = (x1 - x0).max(1);
        let dst_h = (y1 - y0).max(1);
        let src_w = i64::from(image.width.max(1));
        let src_h = i64::from(image.height.max(1));
        for dy in y0..y1 {
            let sy = ((dy - y0) * src_h / dst_h) as usize;
            for dx in x0..x1 {
                if !self.clip_contains(dx, dy) {
                    continue;
                }
                let sx = ((dx - x0) * src_w / dst_w) as usize;
                let i = (sy * src_w as usize + sx) * 4;
                let color = Color::from_rgba8(
                    image.rgba[i],
                    image.rgba[i + 1],
                    image.rgba[i + 2],
                    image.rgba[i + 3],
                );
                self.blend_pixel(dx as usize, dy as usize, color);
            }
        }
    }

    /// Draws one run: glyphs are placed so the run's *baseline* sits at
    /// `run.baseline_y`, matching the layout pass's line-box centering.
    /// Pixels outside the active clip are skipped.
    fn draw_text(&mut self, fonts: &mut FontStore, run: &TextRunPlacement<'_>) {
        let weight = if run.bold {
            crate::font::FontWeight::Bold
        } else {
            crate::font::FontWeight::Regular
        };
        let mut pen_x = run.x.round() + self.offset.0;
        let baseline = run.baseline_y.round() + self.offset.1;
        for ch in run.text.chars() {
            let glyph = fonts.glyph(weight, ch, run.px);
            let advance = glyph.advance;
            let bitmap_left = pen_x as i64 + glyph.xmin as i64;
            let bitmap_top = baseline as i64 - glyph.ymin as i64 - glyph.height as i64;
            for (row, coverage_row) in glyph.coverage.chunks_exact(glyph.width.max(1)).enumerate() {
                let dy = bitmap_top + row as i64;
                if dy < 0 || dy >= self.height as i64 {
                    continue;
                }
                for (col, alpha) in coverage_row.iter().enumerate() {
                    let dx = bitmap_left + col as i64;
                    if dx < 0 || dx >= self.width as i64 || *alpha == 0 {
                        continue;
                    }
                    if !self.clip_contains(dx, dy) {
                        continue;
                    }
                    let shade = Color::from_rgba8(
                        run.color.r,
                        run.color.g,
                        run.color.b,
                        scale_u8(*alpha, run.color.a),
                    );
                    self.blend_pixel(dx as usize, dy as usize, shade);
                }
            }
            self.glyphs += 1;
            pen_x += advance;
        }
    }
}

/// Borrowed view of one [`DisplayItem::TextRun`] for painting.
struct TextRunPlacement<'a> {
    x: f32,
    baseline_y: f32,
    text: &'a str,
    px: u16,
    bold: bool,
    color: Color,
}

/// Does the pixel (x, y) sit inside every part of the clip shape? Square
/// shapes use the integer half-open test; rounded shapes test the pixel
/// **center** against the rounded bounds.
fn shape_contains(shape: &ClipShape, x: i64, y: i64) -> bool {
    let cx = shape.rect.x as i64;
    let cy = shape.rect.y as i64;
    let in_rect =
        x >= cx && x < cx + shape.rect.w as i64 && y >= cy && y < cy + shape.rect.h as i64;
    if !in_rect {
        return false;
    }
    if shape.radius <= 0.0 {
        return true;
    }
    point_in_rounded_rect(x as f32 + 0.5, y as f32 + 0.5, &shape.rect, shape.radius)
}

/// Strict inside-test for a uniform-radius rounded rectangle (corner order
/// irrelevant — all four are the same). Deterministic pure f32 math.
fn point_in_rounded_rect(x: f32, y: f32, rect: &Rect, radius: f32) -> bool {
    let r = radius.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
    let x0 = rect.x;
    let y0 = rect.y;
    let x1 = rect.x + rect.w;
    let y1 = rect.y + rect.h;
    if r <= 0.0 {
        return x >= x0 && x < x1 && y >= y0 && y < y1;
    }
    // Distance from the nearest corner-circle center, 0 inside the
    // straight zones.
    let dx = (x0 + r - x).max(x - (x1 - r)).max(0.0);
    let dy = (y0 + r - y).max(y - (y1 - r)).max(0.0);
    dx * dx + dy * dy <= r * r
}

/// Strict inside-test for a rounded rectangle with per-corner radii
/// (`tl, tr, br, bl`) — used for the inner edge of rounded borders, where
/// uneven border widths make corners shrink by different amounts.
fn point_in_rounded_rect4(x: f32, y: f32, rect: &Rect, radii: &[f32; 4]) -> bool {
    let x0 = rect.x;
    let y0 = rect.y;
    let x1 = rect.x + rect.w;
    let y1 = rect.y + rect.h;
    if x < x0 || x > x1 || y < y0 || y > y1 {
        return false;
    }
    let half = rect.w.min(rect.h) / 2.0;
    let clamp = |r: f32| r.clamp(0.0, half);
    let (r_tl, r_tr, r_br, r_bl) = (
        clamp(radii[0]),
        clamp(radii[1]),
        clamp(radii[2]),
        clamp(radii[3]),
    );
    // Corner zones: circle tests against the adjacent corner's center.
    if x < x0 + r_tl && y < y0 + r_tl {
        let dx = x0 + r_tl - x;
        let dy = y0 + r_tl - y;
        return dx * dx + dy * dy <= r_tl * r_tl;
    }
    if x > x1 - r_tr && y < y0 + r_tr {
        let dx = x - (x1 - r_tr);
        let dy = y0 + r_tr - y;
        return dx * dx + dy * dy <= r_tr * r_tr;
    }
    if x > x1 - r_br && y > y1 - r_br {
        let dx = x - (x1 - r_br);
        let dy = y - (y1 - r_br);
        return dx * dx + dy * dy <= r_br * r_br;
    }
    if x < x0 + r_bl && y > y1 - r_bl {
        let dx = x0 + r_bl - x;
        let dy = y - (y1 - r_bl);
        return dx * dx + dy * dy <= r_bl * r_bl;
    }
    true
}

/// Coverage-to-alpha scaling that stays exact at the ends (0, 255).
fn scale_u8(a: u8, b: u8) -> u8 {
    ((a as u16 * b as u16 + 127) / 255) as u8
}
