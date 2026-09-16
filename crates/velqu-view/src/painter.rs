//! CPU rasterizer for display lists produced by the layout pass.
//!
//! The painter makes **no layout decisions** (ADR 0005): it consumes a
//! finished [`DisplayList`] plus a background color and produces pixels.
//!
//! Determinism rules (fixture hashes depend on them):
//! * rectangles are solid fills snapped to device-pixel edges — no
//!   anti-aliasing, so coverage math cannot vary;
//! * glyph coverage comes from fontdue's scalar rasterizer at whole-pixel
//!   sizes (see `font.rs`);
//! * frame buffers are pixel-bounded upstream and allocated fallibly.

use crate::color::Color;
use crate::display_list::{DisplayList, DisplayRect, DisplayText};
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
    };

    let mut items = 0;
    for DisplayRect { rect, color } in &list.rects {
        ctx.fill_rect(rect.x, rect.y, rect.w, rect.h, *color);
        items += 1;
    }
    for text in &list.texts {
        ctx.draw_text(fonts, text);
        items += 1;
    }

    let glyphs = ctx.glyphs;
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
}

impl PaintCtx<'_> {
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
                self.blend_pixel(px as usize, py as usize, color);
            }
        }
    }

    /// Draws one run: glyphs are placed so the run's *baseline* sits at
    /// `text.y`, matching the layout pass's line-box centering.
    fn draw_text(&mut self, fonts: &mut FontStore, text: &DisplayText) {
        let weight = if text.bold {
            crate::font::FontWeight::Bold
        } else {
            crate::font::FontWeight::Regular
        };
        let mut pen_x = text.x.round();
        for ch in text.text.chars() {
            let glyph = fonts.glyph(weight, ch, text.px);
            let advance = glyph.advance;
            let bitmap_left = pen_x as i64 + glyph.xmin as i64;
            let bitmap_top = text.y.round() as i64 - glyph.ymin as i64 - glyph.height as i64;
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
                    let shade = Color::from_rgba8(
                        text.color.r,
                        text.color.g,
                        text.color.b,
                        scale_u8(*alpha, text.color.a),
                    );
                    self.blend_pixel(dx as usize, dy as usize, shade);
                }
            }
            self.glyphs += 1;
            pen_x += advance;
        }
    }
}

/// Coverage-to-alpha scaling that stays exact at the ends (0, 255).
fn scale_u8(a: u8, b: u8) -> u8 {
    ((a as u16 * b as u16 + 127) / 255) as u8
}
