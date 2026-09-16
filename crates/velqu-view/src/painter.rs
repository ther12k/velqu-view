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
    };

    for item in &list.items {
        match item {
            DisplayItem::FillRect { rect, color } => {
                ctx.fill_rect(rect.x, rect.y, rect.w, rect.h, *color);
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
            DisplayItem::PushClip(rect) => ctx.push_clip(*rect),
            DisplayItem::PopClip => ctx.pop_clip(),
            DisplayItem::DrawImage { rect, image } => {
                ctx.draw_image(rect, image);
                ctx.items += 1;
            }
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
    /// Clip stack: the innermost (last) entry bounds every pixel. Balanced
    /// push/pop emission keeps restore semantics correct for nested scopes.
    clip: Vec<Rect>,
}

impl PaintCtx<'_> {
    /// Intersects the pushed rect with the running clip and scopes it.
    fn push_clip(&mut self, rect: Rect) {
        let clipped = match self.clip.last() {
            None => rect,
            Some(outer) => intersect(*outer, rect),
        };
        self.clip.push(clipped);
    }

    /// Ends the innermost clip scope, restoring the enclosing one. M2c
    /// emission is balanced; a stray PopClip is a harmless no-op.
    fn pop_clip(&mut self) {
        self.clip.pop();
    }

    fn clip_contains(&self, x: i64, y: i64) -> bool {
        match self.clip.last() {
            None => true,
            Some(clip) => {
                let cx = clip.x as i64;
                let cy = clip.y as i64;
                x >= cx && x < cx + clip.w as i64 && y >= cy && y < cy + clip.h as i64
            }
        }
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
        // Rects arrive already in device pixels; snap the edges only.
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

    /// Blits a decoded image into `rect` with nearest-neighbor sampling
    /// (`object-fit: fill`). Edges snap like fills; source indices come from
    /// integer math only, so sampling is deterministic across runs.
    fn draw_image(&mut self, rect: &Rect, image: &crate::image::DecodedImage) {
        let x0 = rect.x.round().max(0.0) as i64;
        let y0 = rect.y.round().max(0.0) as i64;
        let x1 = (rect.x + rect.w).round().clamp(0.0, self.width as f32) as i64;
        let y1 = (rect.y + rect.h).round().clamp(0.0, self.height as f32) as i64;
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
        let mut pen_x = run.x.round();
        for ch in run.text.chars() {
            let glyph = fonts.glyph(weight, ch, run.px);
            let advance = glyph.advance;
            let bitmap_left = pen_x as i64 + glyph.xmin as i64;
            let bitmap_top =
                run.baseline_y.round() as i64 - glyph.ymin as i64 - glyph.height as i64;
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

fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.w).min(b.x + b.w);
    let bottom = (a.y + a.h).min(b.y + b.h);
    Rect {
        x,
        y,
        w: (right - x).max(0.0),
        h: (bottom - y).max(0.0),
    }
}

/// Coverage-to-alpha scaling that stays exact at the ends (0, 255).
fn scale_u8(a: u8, b: u8) -> u8 {
    ((a as u16 * b as u16 + 127) / 255) as u8
}
