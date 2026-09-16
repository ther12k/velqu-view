//! CPU rasterizer for the M1 paint scene.
//!
//! Determinism rules (fixture hashes depend on them):
//!
//! * logical → device conversion is `round(v * scale_factor)`;
//! * rectangles are solid fills snapped to device-pixel edges — no
//!   anti-aliasing, so coverage math cannot vary;
//! * glyph coverage comes from fontdue's scalar rasterizer at whole-pixel
//!   sizes (see `font.rs`).
//!
//! Anti-aliasing of primitives, clipping, and transforms are M2 concerns and
//! will arrive with the real renderer backend behind the same API.

use crate::color::Color;
use crate::font::FontStore;
use crate::scene::{Item, Scene};
use crate::{Frame, Viewport};

/// Line spacing multiplier applied to the nominal text size.
const LINE_HEIGHT_FACTOR: f32 = 1.5;

/// Paints `scene` into a frame for `viewport`.
///
/// Returns the frame, the number of items painted, and the number of glyphs
/// composited (reported through [`crate::RenderStats`]).
pub(crate) fn paint(
    scene: &Scene,
    viewport: Viewport,
    fonts: &mut FontStore,
) -> (Frame, usize, usize) {
    let width = viewport.width as usize;
    let height = viewport.height as usize;
    let mut rgba = vec![0u8; width * height * 4];
    let bg = scene.background;
    for px in rgba.chunks_exact_mut(4) {
        px[0] = bg.r;
        px[1] = bg.g;
        px[2] = bg.b;
        px[3] = bg.a;
    }

    let mut ctx = PaintCtx {
        width: viewport.width,
        height: viewport.height,
        scale: viewport.scale_factor,
        rgba: &mut rgba,
        glyphs: 0,
    };

    let mut items = 0;
    for item in &scene.items {
        let painted = match item {
            Item::Rect { x, y, w, h, color } => {
                ctx.fill_rect(*x, *y, *w, *h, *color);
                true
            }
            Item::RectOutline {
                x,
                y,
                w,
                h,
                thickness,
                color,
            } => {
                ctx.stroke_rect(*x, *y, *w, *h, *thickness, *color);
                true
            }
            Item::Text {
                x,
                y,
                text,
                size,
                color,
                weight,
            } => {
                ctx.draw_text(
                    fonts,
                    &TextRun {
                        x: *x,
                        y: *y,
                        text,
                        size: *size,
                        color: *color,
                        weight: *weight,
                    },
                );
                true
            }
        };
        if painted {
            items += 1;
        }
    }

    let glyphs = ctx.glyphs;
    (
        Frame::from_parts(viewport.width, viewport.height, rgba),
        items,
        glyphs,
    )
}

struct PaintCtx<'a> {
    width: u32,
    height: u32,
    scale: f32,
    rgba: &'a mut [u8],
    glyphs: usize,
}

impl PaintCtx<'_> {
    fn to_device(&self, v: f32) -> i64 {
        (v * self.scale).round() as i64
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
        let x0 = self.to_device(x).max(0) as u32;
        let y0 = self.to_device(y).max(0) as u32;
        let x1 = self.to_device(x + w).clamp(0, self.width as i64) as u32;
        let y1 = self.to_device(y + h).clamp(0, self.height as i64) as u32;
        for py in y0..y1 {
            for px in x0..x1 {
                self.blend_pixel(px as usize, py as usize, color);
            }
        }
    }

    fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, thickness: f32, color: Color) {
        self.fill_rect(x, y, w, thickness, color); // top
        self.fill_rect(x, y + h - thickness, w, thickness, color); // bottom
        self.fill_rect(x, y + thickness, thickness, h - 2.0 * thickness, color); // left
        self.fill_rect(
            x + w - thickness,
            y + thickness,
            thickness,
            h - 2.0 * thickness,
            color,
        ); // right
    }

    fn draw_text(&mut self, fonts: &mut FontStore, run: &TextRun<'_>) {
        let px = (run.size * self.scale).round().max(1.0) as u16;
        let ascent = fonts.ascent(run.weight, px);
        let line_height = px as f32 * LINE_HEIGHT_FACTOR;
        let mut line_index = 0u32;
        for line in run.text.split('\n') {
            let pen_y = run.y * self.scale + line_index as f32 * line_height + ascent;
            let mut pen_x = run.x * self.scale;
            for ch in line.chars() {
                let glyph = fonts.glyph(run.weight, ch, px);
                let advance = glyph.advance;
                let bitmap_left = pen_x as i64 + glyph.xmin as i64;
                let bitmap_top = (pen_y.round() as i64) - glyph.ymin as i64 - glyph.height as i64;
                for (row, coverage_row) in
                    glyph.coverage.chunks_exact(glyph.width.max(1)).enumerate()
                {
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
            line_index += 1;
        }
    }
}

/// A text item borrowed for painting.
struct TextRun<'a> {
    x: f32,
    y: f32,
    text: &'a str,
    size: f32,
    color: Color,
    weight: crate::scene::FontWeight,
}

/// Coverage-to-alpha scaling that stays exact at the ends (0, 255).
fn scale_u8(a: u8, b: u8) -> u8 {
    ((a as u16 * b as u16 + 127) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Item;

    #[test]
    fn fill_rect_snaps_to_device_pixels() {
        let scene = Scene {
            background: Color::BLACK,
            items: vec![Item::Rect {
                x: 10.3,
                y: 10.3,
                w: 20.4,
                h: 20.4,
                color: Color::WHITE,
            }],
        };
        let (frame, items, glyphs) = paint(
            &scene,
            Viewport::new(64, 64, 1.0),
            &mut FontStore::bundled(),
        );
        assert_eq!((items, glyphs), (1, 0));
        // round(10.3)=10 .. round(30.7)=31.
        assert_eq!(frame.pixel(10, 10), Some(Color::WHITE));
        assert_eq!(frame.pixel(30, 30), Some(Color::WHITE));
        assert_eq!(frame.pixel(31, 30), Some(Color::BLACK));
        assert_eq!(frame.pixel(10, 9), Some(Color::BLACK));
    }

    #[test]
    fn fill_rect_clips_to_viewport() {
        let scene = Scene {
            background: Color::BLACK,
            items: vec![Item::Rect {
                x: -10.0,
                y: -10.0,
                w: 200.0,
                h: 200.0,
                color: Color::WHITE,
            }],
        };
        let (frame, _, _) = paint(
            &scene,
            Viewport::new(64, 64, 1.0),
            &mut FontStore::bundled(),
        );
        assert_eq!(frame.pixel(0, 0), Some(Color::WHITE));
        assert_eq!(frame.pixel(63, 63), Some(Color::WHITE));
    }

    #[test]
    fn dpi_scale_grows_text_coverage() {
        let text_scene = |size: f32| Scene {
            background: Color::BLACK,
            items: vec![Item::Text {
                x: 4.0,
                y: 4.0,
                text: "VelquView".into(),
                size,
                color: Color::WHITE,
                weight: crate::scene::FontWeight::Regular,
            }],
        };
        let mut fonts = FontStore::bundled();
        let (small, _, g1) = paint(&text_scene(12.0), Viewport::new(200, 80, 1.0), &mut fonts);
        let (large, _, g2) = paint(&text_scene(12.0), Viewport::new(400, 160, 2.0), &mut fonts);
        assert_eq!(g1, g2, "same glyph count at any DPI");
        let ink = |f: &Frame| f.pixels().chunks_exact(4).filter(|p| p[0] > 0).count();
        let small_ink = ink(&small);
        let large_ink = ink(&large);
        assert!(large_ink > small_ink * 3, "2x DPI covers ~4x the pixels");
    }
}
