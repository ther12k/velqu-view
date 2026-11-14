//! CPU rasterizer for display lists produced by the layout pass.
//!
//! The painter makes **no layout decisions** (ADR 0005): it consumes a
//! finished [`DisplayList`] — fills, text runs, clip scoping — plus a
//! background color and produces pixels. Clip scopes
//! ([`DisplayItem::PushClip`]/[`DisplayItem::PopClip`]) intersect with the
//! running clip stack; every item is bounded by the innermost clip.
//!
//! Determinism rules (fixture hashes depend on them):
//! * shape coverage is per-pixel (ADR 0026): exact span fractions for
//!   straight edges, a signed-distance classification with a fixed 4×4
//!   subsample grid for rounded boundaries. All math is pure `f32` on a
//!   fixed grid, so coverage cannot vary across runs; fractional layout
//!   coordinates reach the pixels unrounded (only text and images raster
//!   at whole pixels);
//! * gradients evaluate per pixel through the same coverage path;
//! * glyph coverage comes from fontdue's scalar rasterizer at whole-pixel
//!   sizes (see `font.rs`);
//! * frame buffers are pixel-bounded upstream and allocated fallibly.

use crate::color::Color;
use crate::display_list::{Affine2D, DisplayItem, DisplayList, Rect, ResolvedGradient, ResolvedGradientKind};
use crate::font::FontStore;
use crate::viewport::Viewport;
use crate::{Frame, VelquError};

/// One axis of the deterministic per-pixel subsample grid (ADR 0026):
/// `SUBGRID²` samples per boundary pixel at `(i + 0.5) / SUBGRID`
/// offsets. 4×4 keeps corner banding invisible at UI scale while the
/// SDF fast path keeps interior pixels at one distance evaluation.
const SUBGRID: usize = 4;

/// Precomputed subsample offsets within one pixel.
const SUB_OFFSETS: [f32; SUBGRID] = [0.125, 0.375, 0.625, 0.875];

/// A pixel whose center has |sdf| above this margin is fully inside or
/// fully outside the shape: the distance-to-set function is 1-Lipschitz
/// and the farthest pixel point from its center is the corner at
/// √2/2 ≈ 0.7071, so 0.71 guarantees the whole pixel is on one side.
const SDF_MARGIN: f32 = 0.71;

/// Paints `list` over `background` into a frame for `viewport`.
///
/// `previous` seeds partial-damage painting: `Some((pixels, damage))`
/// copies the previous frame's pixels and repaints **only** the damage
/// rectangle (device px, already the union the caller diffed); `None`
/// clears the whole surface. Returns the frame, the number of items
/// painted, and the number of glyphs composited (reported through
/// [`crate::RenderStats`]).
pub(crate) fn paint_document(
    list: &DisplayList,
    background: Color,
    viewport: Viewport,
    fonts: &mut FontStore,
    previous: Option<(&[u8], Rect)>,
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

    // Damage clipping: with a previous frame, keep its pixels and clear +
    // repaint only inside the damage rect. The rect is clamped and
    // integer-snapped outward so every partial item stays covered.
    let damage = previous.as_ref().map(|(_, rect)| clamp_damage(rect, viewport));
    match (&previous, damage) {
        (Some((pixels, _)), Some(rect)) => {
            let (dx0, dy0, dx1, dy1) = rect;
            let w = viewport.width() as usize;
            if dx1 > dx0 && dy1 > dy0 {
                rgba.copy_from_slice(pixels);
                for py in dy0..dy1 {
                    let row = py as usize * w * 4;
                    for px in rgba[row + dx0 as usize * 4..row + dx1 as usize * 4]
                        .chunks_exact_mut(4)
                    {
                        px[0] = background.r;
                        px[1] = background.g;
                        px[2] = background.b;
                        px[3] = background.a;
                    }
                }
            }
        }
        _ => {
            for px in rgba.chunks_exact_mut(4) {
                px[0] = background.r;
                px[1] = background.g;
                px[2] = background.b;
                px[3] = background.a;
            }
        }
    }

    let mut ctx = PaintCtx {
        width: viewport.width(),
        height: viewport.height(),
        rgba: &mut rgba,
        glyphs: 0,
        items: 0,
        clip: Vec::new(),
        transform: Affine2D::IDENTITY,
        transform_stack: Vec::new(),
        opacity: 1.0,
        opacity_stack: Vec::new(),
        damage,
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
            DisplayItem::GradientFill {
                rect,
                radius,
                gradient,
            } => {
                ctx.fill_gradient(rect, *radius, gradient);
                ctx.items += 1;
            }
            DisplayItem::TextRun {
                x,
                y,
                text,
                px,
                bold,
                color,
                family,
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
                        family,
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
            DisplayItem::PushTransform { matrix } => ctx.push_transform(*matrix),
            DisplayItem::PopTransform => ctx.pop_transform(),
            DisplayItem::PushOpacity(alpha) => ctx.push_opacity(*alpha),
            DisplayItem::PopOpacity => ctx.pop_opacity(),
            DisplayItem::OuterShadow {
                bounds,
                radius,
                inner,
                inner_radius,
                blur,
                color,
            } => {
                ctx.draw_outer_shadow(*bounds, *radius, *inner, *inner_radius, *blur, *color);
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
    /// Clip stack: every active scope bounds every pixel (a pixel must be
    /// inside all of them; balanced push/pop emission keeps the stack
    /// correct for nesting).
    clip: Vec<ClipShape>,
    /// Current transform (accumulated scroll + element transforms, ADR
    /// 0008/0024) plus the saved transforms of enclosing scopes.
    transform: Affine2D,
    transform_stack: Vec<Affine2D>,
    /// Current cumulative source opacity (ADR 0024) plus the saved values
    /// of enclosing scopes.
    opacity: f32,
    opacity_stack: Vec<f32>,
    /// Partial-damage window (`None` = paint the whole surface): rows and
    /// columns outside it keep the previous frame's pixels. Half-open
    /// integer device bounds `(x0, y0, x1, y1)`.
    damage: Option<(i64, i64, i64, i64)>,
}

/// Clamps a device-pixel damage rect to the viewport, snapping outward to
/// whole pixels (half-open integer bounds). Degenerate rects collapse.
fn clamp_damage(rect: &Rect, viewport: Viewport) -> (i64, i64, i64, i64) {
    let x0 = rect.x.floor().max(0.0) as i64;
    let y0 = rect.y.floor().max(0.0) as i64;
    let x1 = (rect.x + rect.w).ceil().clamp(0.0, viewport.width() as f32) as i64;
    let y1 = (rect.y + rect.h).ceil().clamp(0.0, viewport.height() as f32) as i64;
    (x0, y0, x1.max(x0), y1.max(y0))
}

/// One active clip scope: a rectangle in device pixels, optionally with
/// rounded corners.
#[derive(Debug, Clone, Copy)]
struct ClipShape {
    rect: Rect,
    radius: f32,
}

impl PaintCtx<'_> {
    /// Enters a transform scope: items are mapped by `matrix` after (inside)
    /// the enclosing transform until the matching `pop_transform`.
    fn push_transform(&mut self, matrix: Affine2D) {
        self.transform_stack.push(self.transform);
        self.transform = self.transform.compose(&matrix);
    }

    /// Leaves the innermost transform scope, restoring the enclosing
    /// transform. Emission is balanced; a stray pop is a harmless no-op.
    fn pop_transform(&mut self) {
        if let Some(transform) = self.transform_stack.pop() {
            self.transform = transform;
        }
    }

    /// Enters an opacity scope: source alpha is multiplied by `alpha`
    /// (clamped to `[0, 1]`) until the matching `pop_opacity`.
    fn push_opacity(&mut self, alpha: f32) {
        self.opacity_stack.push(self.opacity);
        self.opacity *= alpha.clamp(0.0, 1.0);
    }

    /// Leaves the innermost opacity scope. Emission is balanced; a stray
    /// pop is a harmless no-op.
    fn pop_opacity(&mut self) {
        if let Some(opacity) = self.opacity_stack.pop() {
            self.opacity = opacity;
        }
    }

    /// Scales a color's alpha by the current opacity scope chain. The
    /// rounding is deterministic (`f32` multiply + `round`), and identity
    /// opacity (1.0) passes every alpha through unchanged.
    fn modulate(&self, color: Color) -> Color {
        if self.opacity >= 1.0 {
            return color;
        }
        let a = (color.a as f32 * self.opacity).round().clamp(0.0, 255.0) as u8;
        Color::from_rgba8(color.r, color.g, color.b, a)
    }

    /// Pushes a clip scope. Clip shapes live in the space of the scope
    /// they are pushed in (scroll containers clip in parent space), so the
    /// current transform applies. Corner radii scale by the transform's
    /// scale factor (equal-axis scales only, per ADR 0024).
    fn push_clip(&mut self, rect: Rect, radius: f32) {
        self.clip.push(ClipShape {
            rect: self.transform.apply_rect(&rect),
            radius: (radius * self.transform.scale_factor()).max(0.0),
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
        // Damage safety net: any path that reaches a pixel outside the
        // damage window keeps the previous frame's pixel. Row/column
        // scoping in the fills and loops makes this rare; correctness here
        // makes it impossible to leak writes.
        if let Some((dx0, dy0, dx1, dy1)) = self.damage {
            let (xi, yi) = (x as i64, y as i64);
            if xi < dx0 || xi >= dx1 || yi < dy0 || yi >= dy1 {
                return;
            }
        }
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

    /// Blends `color` at fractional shape coverage `cov ∈ (0, 1)` (the
    /// rounded fraction of the fixed sample grid, ADR 0026): the source
    /// alpha scales by `round(cov·255)` before the normal blend.
    fn blend_pixel_coverage(&mut self, x: usize, y: usize, color: Color, cov: f32) {
        let coverage_alpha = (cov * 255.0).round().clamp(0.0, 255.0) as u8;
        if coverage_alpha == 0 {
            return;
        }
        let shade = Color::from_rgba8(color.r, color.g, color.b, scale_u8(coverage_alpha, color.a));
        self.blend_pixel(x, y, shade);
    }

    /// Fills an axis-aligned rectangle with exact per-pixel coverage
    /// (ADR 0026): the overlap of the pixel square with the rect spans is
    /// a 1D fraction per axis, so fractional coordinates render at their
    /// true coverage — no edge snapping. Interior pixels take the
    /// full-strength path.
    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let color = self.modulate(color);
        if self.transform.is_axis_aligned() {
            let placed = self.transform.apply_rect(&Rect { x, y, w, h });
            let x0 = placed.x.max(0.0);
            let y0 = placed.y.max(0.0);
            let x1 = (placed.x + placed.w).min(self.width as f32);
            let y1 = (placed.y + placed.h).min(self.height as f32);
            if x1 <= x0 || y1 <= y0 {
                return;
            }
            let (rows0, rows1) = self.damage_rows(y0.floor() as i64, y1.ceil() as i64);
            for py in rows0..rows1 {
                let fy = span_coverage(py as f32, y0, y1);
                if fy <= 0.0 {
                    continue;
                }
                let (cols0, cols1) = self.damage_cols(x0.floor() as i64, x1.ceil() as i64);
                for px in cols0..cols1 {
                    if !self.clip_contains(px, py) {
                        continue;
                    }
                    let cov = span_coverage(px as f32, x0, x1) * fy;
                    if cov >= 1.0 {
                        self.blend_pixel(px as usize, py as usize, color);
                    } else if cov > 0.0 {
                        self.blend_pixel_coverage(px as usize, py as usize, color, cov);
                    }
                }
            }
        } else if let Some(inv) = self.transform.inverse() {
            let (x0, y0, x1, y1) = transform_bounds(
                &self.transform,
                &Rect { x, y, w, h },
                self.width,
                self.height,
            );
            let (rows0, rows1) = self.damage_rows(y0 as i64, y1 as i64);
            for py in rows0..rows1 {
                for px in x0..x1 {
                    if !self.clip_contains(px as i64, py) {
                        continue;
                    }
                    let cov = subsample_coverage(&inv, px as f32, py as f32, |lx, ly| {
                        lx >= x && lx < x + w && ly >= y && ly < y + h
                    });
                    if cov >= 1.0 {
                        self.blend_pixel(px as usize, py as usize, color);
                    } else if cov > 0.0 {
                        self.blend_pixel_coverage(px as usize, py as usize, color, cov);
                    }
                }
            }
        }
    }

    /// Intersects a half-open row range with the damage window.
    fn damage_rows(&self, y0: i64, y1: i64) -> (i64, i64) {
        match self.damage {
            None => (y0, y1),
            Some((_, dy0, _, dy1)) => (y0.max(dy0), y1.min(dy1)),
        }
    }

    /// Intersects a half-open column range with the damage window.
    fn damage_cols(&self, x0: i64, x1: i64) -> (i64, i64) {
        match self.damage {
            None => (x0, x1),
            Some((dx0, _, dx1, _)) => (x0.max(dx0), x1.min(dx1)),
        }
    }

    /// Fills a rounded rectangle with per-pixel coverage (ADR 0026): a
    /// signed-distance classification fast-paths interior pixels (one
    /// `sqrt`), and boundary-band pixels evaluate the fixed 4×4 subsample
    /// grid — fractional positions and radii render at true coverage. The
    /// corner radius scales with the transform (ADR 0024). Rotated shapes
    /// subsample through inverse transform mapping.
    ///
    /// Row spans: per scanline the inside interval is derived from the same
    /// corner math (conservative ±2px, exact predicate inside), so the
    /// outside-span scan disappears.
    fn fill_rounded(&mut self, rect: &Rect, radius: f32, color: Color) {
        let color = self.modulate(color);
        if self.transform.is_axis_aligned() {
            let placed = self.transform.apply_rect(rect);
            let radius = radius * self.transform.scale_factor();
            let x0 = placed.x.max(0.0).floor() as i64;
            let y0 = placed.y.max(0.0).floor() as i64;
            let x1 = (placed.x + placed.w).min(self.width as f32).ceil() as i64;
            let y1 = (placed.y + placed.h).min(self.height as f32).ceil() as i64;
            let (rows0, rows1) = self.damage_rows(y0, y1);
            // Straight-zone rows: the span equals the true shape extent, so
            // pixels ≥2px inside it are fully covered (≥1.5px from any
            // boundary) and skip the SDF entirely; corner rows and edge
            // pixels classify per pixel as before. Identical pixels, less
            // per-pixel math on the interior.
            let outer_r = radius.min(placed.w / 2.0).min(placed.h / 2.0).max(0.0);
            let straight_top = (placed.y + outer_r).ceil() as i64;
            let straight_bottom = (placed.y + placed.h - outer_r).floor() as i64;
            let left_edge = placed.x.ceil() as i64 + 2;
            let right_edge = (placed.x + placed.w).floor() as i64 - 2;
            for py in rows0..rows1 {
                let (span0, span1) = rounded_row_span(&placed, radius, py);
                if span0 >= span1 {
                    continue;
                }
                let straight_row = py >= straight_top && py < straight_bottom;
                let (cols0, cols1) = self.damage_cols(span0 - 2, span1 + 2);
                for px in cols0.max(x0)..cols1.min(x1) {
                    if !self.clip_contains(px, py) {
                        continue;
                    }
                    if straight_row && px >= left_edge && px < right_edge {
                        self.blend_pixel(px as usize, py as usize, color);
                        continue;
                    }
                    let cx = px as f32 + 0.5;
                    let cy = py as f32 + 0.5;
                    let sd = rounded_rect_sdf(cx, cy, &placed, radius);
                    if sd <= -SDF_MARGIN {
                        self.blend_pixel(px as usize, py as usize, color);
                    } else if sd < SDF_MARGIN {
                        let cov = rounded_subsample_coverage(&placed, radius, px as f32, py as f32);
                        self.blend_pixel_coverage(px as usize, py as usize, color, cov);
                    }
                }
            }
        } else if let Some(inv) = self.transform.inverse() {
            let (x0, y0, x1, y1) = transform_bounds(&self.transform, rect, self.width, self.height);
            let (rows0, rows1) = self.damage_rows(y0 as i64, y1 as i64);
            for py in rows0..rows1 {
                for px in x0..x1 {
                    if !self.clip_contains(px as i64, py) {
                        continue;
                    }
                    let cov = subsample_coverage(&inv, px as f32, py as f32, |lx, ly| {
                        point_in_rounded_rect(lx, ly, rect, radius)
                    });
                    if cov >= 1.0 {
                        self.blend_pixel(px as usize, py as usize, color);
                    } else if cov > 0.0 {
                        self.blend_pixel_coverage(px as usize, py as usize, color, cov);
                    }
                }
            }
        }
    }

    /// Fills a rounded border ring with coverage `cov_outer − cov_inner`
    /// (ADR 0026): a 1px hairline at any fractional position renders at
    /// its true partial coverage — no wobble, no disappearing rows.
    /// Outer pixels classify through the rounded-rect SDF; inner-shape
    /// coverage subsamples only when the pixel can touch the hole's
    /// boundary (straight-edge and corner-zone shortcuts). Inner corner
    /// radii are the outer radius minus the adjacent border widths
    /// (clamped at 0). Transforms map both edges (ADR 0024).
    fn fill_rounded_ring(
        &mut self,
        outer: &Rect,
        radius: f32,
        inner: &Rect,
        inner_radii: &[f32; 4],
        color: Color,
    ) {
        let color = self.modulate(color);
        if self.transform.is_axis_aligned() {
            let scale = self.transform.scale_factor();
            let placed_outer = self.transform.apply_rect(outer);
            let placed_inner = self.transform.apply_rect(inner);
            let radius = radius * scale;
            let inner_radii = [
                inner_radii[0] * scale,
                inner_radii[1] * scale,
                inner_radii[2] * scale,
                inner_radii[3] * scale,
            ];
            let x0 = placed_outer.x.max(0.0).floor() as i64;
            let y0 = placed_outer.y.max(0.0).floor() as i64;
            let x1 = (placed_outer.x + placed_outer.w)
                .min(self.width as f32)
                .ceil() as i64;
            let y1 = (placed_outer.y + placed_outer.h)
                .min(self.height as f32)
                .ceil() as i64;
            let (rows0, rows1) = self.damage_rows(y0, y1);
            // Per-row the ring is two horizontal bands (left/right of the
            // hole) on rows where every pixel is *certainly* deep inside
            // the hole, and the full outer span otherwise. The exclusion
            // mirrors `pixel_deep_inside_inner` in range form: a pixel is
            // skippable only when its whole 4x4 sample footprint (±0.375
            // around the center) sits ≥ [`SDF_MARGIN`] inside every hole
            // edge and outside every corner zone (`r_max + SDF_MARGIN`
            // reach). Everything the range form cannot guarantee is
            // visited and decided per pixel, so coverage is bit-identical
            // to the full-span scan while the hole interior — eight rings
            // × the whole shadow bbox during a morph — is never touched.
            let sub_radius = (SUBGRID - 1) as f32 / (2.0 * SUBGRID as f32);
            // The corner-zone reach uses the hole radius clamped to the
            // hole's half-dimensions exactly like
            // `point_in_rounded_rect4` clamps per-corner radii — a
            // `rounded-full` author radius (9999px) must not turn the
            // zone into the whole shape and void the interior exclusion.
            let r_inner_max = inner_radii
                .iter()
                .cloned()
                .fold(0.0f32, f32::max)
                .min(placed_inner.w / 2.0)
                .min(placed_inner.h / 2.0)
                .max(0.0);
            let zone = r_inner_max + SDF_MARGIN;
            let y0e = placed_inner.y;
            let y1e = placed_inner.y + placed_inner.h;
            let cross_lo = (y0e + SDF_MARGIN + sub_radius - 0.5).ceil() as i64;
            let cross_hi = (y1e - SDF_MARGIN - sub_radius - 0.5).ceil() as i64;
            for py in rows0..rows1 {
                let (span0, span1) = rounded_row_span(&placed_outer, radius, py);
                if span0 >= span1 {
                    continue;
                }
                let cy = py as f32 + 0.5;
                // Skippable interior on crossing rows: pixels whose whole
                // sample footprint sits beyond every corner zone reach
                // (`zone`) and ≥ [`SDF_MARGIN`] inside the straight edges.
                // Corner-zone rows keep only their middle range — the
                // zones hug the hole's left/right ends.
                let x_pad = zone + sub_radius;
                let excl_lo = (placed_inner.x + x_pad - 0.5).ceil() as i64;
                let excl_hi = (placed_inner.x + placed_inner.w - x_pad - 0.5).ceil() as i64;
                let crossing = py >= cross_lo && py < cross_hi;
                let bands: [(i64, i64); 2] = if crossing {
                    [
                        (span0.max(x0), excl_lo.min(span1)),
                        (excl_hi.max(span0), span1.min(x1)),
                    ]
                } else {
                    [(span0.max(x0), span1.min(x1)), (0, 0)]
                };
                for (band0, band1) in bands {
                    if band1 <= band0 {
                        continue;
                    }
                    // The +-2px wing covers the SDF boundary band (±0.71
                    // around the exact span edge — fractional shapes put
                    // real partial coverage there); the per-pixel SDF
                    // classifies wing pixels exactly as the full scan did.
                    let (cols0, cols1) = self.damage_cols(band0 - 2, band1 + 2);
                    for px in cols0.max(x0)..cols1.min(x1) {
                        if !self.clip_contains(px, py) {
                            continue;
                        }
                        let cx = px as f32 + 0.5;
                        // Squared SDF classification (no sqrt): with
                        // q = |p - center| - (half_extent - r), the signed
                        // distance is sqrt(dx^2+dy^2) + min(max(qx,qy),0)
                        // - r, so `sd >= m` reduces to `d2 >= (r+m)^2` and
                        // `sd <= -m` to `qx <= -m || qy <= -m` — exact in
                        // every region of the distance field.
                        let qxo = (cx - (placed_outer.x + placed_outer.w / 2.0)).abs()
                            - outer_hw(radius, &placed_outer);
                        let qyo = (cy - (placed_outer.y + placed_outer.h / 2.0)).abs()
                            - outer_hh(radius, &placed_outer);
                        let dxo = qxo.max(0.0);
                        let dyo = qyo.max(0.0);
                        if dxo * dxo + dyo * dyo
                            >= (radius + SDF_MARGIN) * (radius + SDF_MARGIN)
                        {
                            continue; // fully outside the outer shape
                        }
                        // Fast path: deep inside the outer shape and safely
                        // outside the hole — coverage is exactly 1 and the
                        // 16x2 subsample grid would only confirm it. The
                        // hole test uses the smallest per-corner radius, so
                        // the assumed hole is the LARGEST the shape can be
                        // and the "safely outside" verdict is conservative.
                        let r_inner_min = inner_radii
                            .iter()
                            .cloned()
                            .fold(f32::MAX, f32::min)
                            .min(placed_inner.w / 2.0)
                            .min(placed_inner.h / 2.0)
                            .max(0.0);
                        let qxi = (cx - (placed_inner.x + placed_inner.w / 2.0)).abs()
                            - outer_hw(r_inner_min, &placed_inner);
                        let qyi = (cy - (placed_inner.y + placed_inner.h / 2.0)).abs()
                            - outer_hh(r_inner_min, &placed_inner);
                        let dxi = qxi.max(0.0);
                        let dyi = qyi.max(0.0);
                        let safe_outside_hole = dxi * dxi + dyi * dyi
                            >= (r_inner_min + SDF_MARGIN) * (r_inner_min + SDF_MARGIN);
                        let deep_in_outer = qxo <= -SDF_MARGIN || qyo <= -SDF_MARGIN;
                        if deep_in_outer && safe_outside_hole {
                            self.blend_pixel(px as usize, py as usize, color);
                            continue;
                        }
                        if pixel_deep_inside_inner(cx, cy, &placed_inner, &inner_radii) {
                            continue; // fully inside the hole: ring alpha ≤ 0
                        }
                        let cov = ring_subsample_coverage(
                            &placed_outer,
                            radius,
                            &placed_inner,
                            &inner_radii,
                            px as f32,
                            py as f32,
                        );
                        self.blend_pixel_coverage(px as usize, py as usize, color, cov);
                    }
                }
            }
        } else if let Some(inv) = self.transform.inverse() {
            let (x0, y0, x1, y1) =
                transform_bounds(&self.transform, outer, self.width, self.height);
            let (rows0, rows1) = self.damage_rows(y0 as i64, y1 as i64);
            for py in rows0..rows1 {
                for px in x0..x1 {
                    if !self.clip_contains(px as i64, py) {
                        continue;
                    }
                    let mut outer_hits = 0usize;
                    let mut inner_hits = 0usize;
                    for sy in SUB_OFFSETS {
                        for sx in SUB_OFFSETS {
                            let (lx, ly) = inv.apply_point(px as f32 + sx, py as f32 + sy);
                            if point_in_rounded_rect(lx, ly, outer, radius) {
                                outer_hits += 1;
                            }
                            if point_in_rounded_rect4(lx, ly, inner, inner_radii) {
                                inner_hits += 1;
                            }
                        }
                    }
                    let cov = (outer_hits as f32 - inner_hits as f32)
                        / (SUBGRID * SUBGRID) as f32;
                    if cov >= 1.0 {
                        self.blend_pixel(px as usize, py as usize, color);
                    } else if cov > 0.0 {
                        self.blend_pixel_coverage(px as usize, py as usize, color, cov);
                    }
                }
            }
        }
    }

    /// Paints one outer box-shadow layer in a **single pass** (ADR 0026):
    /// per pixel, the signed distance `d` to the shadow shape (the bounds
    /// rect — offset and spread already applied) drives a linear falloff —
    /// full strength inside the shape, fading to transparent at `blur`
    /// beyond it — masked by the element's border box so the shadow never
    /// shows inside a transparent element (CSS clips outer shadows at the
    /// border edge). This is the exact envelope the previous
    /// [`SHADOW_FEATHER_STEPS`]-ring stack approximated (its equal alpha
    /// slices accumulated to ≈ `a·(1 − d/blur)`), rasterized in one sweep
    /// of the band instead of eight — the morph-frame paint cost drops
    /// ~8×. A zero blur paints one solid ring. Composes under the current
    /// transform/clip/opacity like any fill.
    fn draw_outer_shadow(
        &mut self,
        bounds: Rect,
        radius: f32,
        inner: Rect,
        inner_radius: f32,
        blur: f32,
        color: Color,
    ) {
        if color.a == 0 || bounds.w <= 0.0 || bounds.h <= 0.0 {
            return;
        }
        if blur <= 0.0 {
            self.fill_rounded_ring(&bounds, radius, &inner, &[inner_radius; 4], color);
            return;
        }
        if self.transform.is_axis_aligned() {
            let scale = self.transform.scale_factor();
            let placed_bounds = self.transform.apply_rect(&bounds);
            let placed_inner = self.transform.apply_rect(&inner);
            let blur = blur * scale;
            let inner_radius = inner_radius * scale;
            let x0 = (placed_bounds.x - blur).max(0.0).floor() as i64;
            let y0 = (placed_bounds.y - blur).max(0.0).floor() as i64;
            let x1 = (placed_bounds.x + placed_bounds.w + blur)
                .min(self.width as f32)
                .ceil() as i64;
            let y1 = (placed_bounds.y + placed_bounds.h + blur)
                .min(self.height as f32)
                .ceil() as i64;
            let (rows0, rows1) = self.damage_rows(y0, y1);
            for py in rows0..rows1 {
                let (cols0, cols1) = self.damage_cols(x0, x1);
                for px in cols0..cols1 {
                    if !self.clip_contains(px, py) {
                        continue;
                    }
                    let cx = px as f32 + 0.5;
                    let cy = py as f32 + 0.5;
                    let d = rounded_rect_sdf(cx, cy, &placed_bounds, radius);
                    if d >= blur {
                        continue; // beyond the falloff
                    }
                    // Mask at the element's border box: the shadow never
                    // shows inside it. The mask subsamples only within
                    // ±√2/2 px of the card edge; the falloff needs one SDF.
                    let d_card = rounded_rect_sdf(cx, cy, &placed_inner, inner_radius);
                    let outside = if d_card >= SDF_MARGIN {
                        1.0
                    } else if d_card <= -SDF_MARGIN {
                        0.0
                    } else {
                        1.0
                            - rounded_subsample_coverage(
                                &placed_inner,
                                inner_radius,
                                px as f32,
                                py as f32,
                            )
                    };
                    if outside <= 0.0 {
                        continue;
                    }
                    let falloff = (1.0 - d / blur).clamp(0.0, 1.0);
                    let cov = falloff * outside;
                    if cov > 0.0 {
                        self.blend_pixel_coverage(px as usize, py as usize, color, cov);
                    }
                }
            }
        } else if let Some(inv) = self.transform.inverse() {
            // Rotated: per-sample falloff × card mask averaged over the
            // fixed grid, matching the other transformed fills.
            let (bx0, by0, bx1, by1) =
                transform_bounds(&self.transform, &bounds, self.width, self.height);
            let x0 = ((bx0 as f32 - blur).max(0.0)).floor() as u32;
            let y0 = ((by0 as f32 - blur).max(0.0)).floor() as u32;
            let x1 = ((bx1 as f32 + blur).min(self.width as f32)).ceil() as u32;
            let y1 = ((by1 as f32 + blur).min(self.height as f32)).ceil() as u32;
            let (rows0, rows1) = self.damage_rows(y0 as i64, y1 as i64);
            for py in rows0..rows1 {
                for px in x0..x1 {
                    if !self.clip_contains(px as i64, py as i64) {
                        continue;
                    }
                    let mut accumulated = 0.0f32;
                    for sy in SUB_OFFSETS {
                        for sx in SUB_OFFSETS {
                            let (lx, ly) = inv.apply_point(px as f32 + sx, py as f32 + sy);
                            let d = rounded_rect_sdf(lx, ly, &bounds, radius);
                            if d >= blur {
                                continue;
                            }
                            if rounded_rect_sdf(lx, ly, &inner, inner_radius) <= 0.0 {
                                continue; // inside the element: masked
                            }
                            accumulated += (1.0 - d / blur).clamp(0.0, 1.0);
                        }
                    }
                    let cov = accumulated / (SUBGRID * SUBGRID) as f32;
                    if cov > 0.0 {
                        self.blend_pixel_coverage(px as usize, py as usize, color, cov);
                    }
                }
            }
        }
    }

    /// Paints a resolved gradient inside `rect` (rounded when
    /// `radius > 0`, same shape semantics as the solid fills). Per pixel:
    /// the shape coverage path (spans/SDF/subsamples) times the gradient
    /// color at the pixel's gradient coordinate — linear gradients project
    /// onto the gradient line, radial gradients take the normalized
    /// elliptical distance. Stop interpolation is channel-wise `f32` lerp,
    /// so identical gradients produce identical pixels (ADR 0026).
    fn fill_gradient(&mut self, rect: &Rect, radius: f32, gradient: &ResolvedGradient) {
        if gradient.stops.is_empty() || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        if self.transform.is_axis_aligned() {
            let placed = self.transform.apply_rect(rect);
            let radius = radius * self.transform.scale_factor();
            let Some(inv) = self.transform.inverse() else {
                return;
            };
            let x0 = placed.x.max(0.0).floor() as i64;
            let y0 = placed.y.max(0.0).floor() as i64;
            let x1 = (placed.x + placed.w).min(self.width as f32).ceil() as i64;
            let y1 = (placed.y + placed.h).min(self.height as f32).ceil() as i64;
            let (rows0, rows1) = self.damage_rows(y0, y1);
            for py in rows0..rows1 {
                let (span0, span1) = if radius > 0.0 {
                    rounded_row_span(&placed, radius, py)
                } else {
                    rounded_row_span(&placed, 0.0, py)
                };
                if span0 >= span1 {
                    continue;
                }
                let (cols0, cols1) = self.damage_cols(span0 - 2, span1 + 2);
                for px in cols0.max(x0)..cols1.min(x1) {
                    if !self.clip_contains(px, py) {
                        continue;
                    }
                    let cx = px as f32 + 0.5;
                    let cy = py as f32 + 0.5;
                    let sd = rounded_rect_sdf(cx, cy, &placed, radius);
                    let cov = if sd <= -SDF_MARGIN {
                        1.0
                    } else if sd < SDF_MARGIN {
                        rounded_subsample_coverage(&placed, radius, px as f32, py as f32)
                    } else {
                        0.0
                    };
                    if cov <= 0.0 {
                        continue;
                    }
                    let (lx, ly) = inv.apply_point(cx, cy);
                    let t = gradient_project(gradient, lx, ly);
                    if t.is_nan() {
                        continue;
                    }
                    let color = self.modulate(gradient_color_at(&gradient.stops, t));
                    if cov >= 1.0 {
                        self.blend_pixel(px as usize, py as usize, color);
                    } else {
                        self.blend_pixel_coverage(px as usize, py as usize, color, cov);
                    }
                }
            }
        } else if let Some(inv) = self.transform.inverse() {
            let (x0, y0, x1, y1) = transform_bounds(&self.transform, rect, self.width, self.height);
            let (rows0, rows1) = self.damage_rows(y0 as i64, y1 as i64);
            for py in rows0..rows1 {
                for px in x0..x1 {
                    if !self.clip_contains(px as i64, py) {
                        continue;
                    }
                    let (lx, ly) = inv.apply_point(px as f32 + 0.5, py as f32 + 0.5);
                    let cov = subsample_coverage(&inv, px as f32, py as f32, |lx, ly| {
                        lx >= rect.x && lx < rect.x + rect.w && ly >= rect.y && ly < rect.y + rect.h
                    });
                    if cov <= 0.0 {
                        continue;
                    }
                    let t = gradient_project(gradient, lx, ly);
                    if t.is_nan() {
                        continue;
                    }
                    let color = self.modulate(gradient_color_at(&gradient.stops, t));
                    if cov >= 1.0 {
                        self.blend_pixel(px as usize, py as usize, color);
                    } else {
                        self.blend_pixel_coverage(px as usize, py as usize, color, cov);
                    }
                }
            }
        }
    }

    /// Blits a decoded image into `rect` with nearest-neighbor sampling
    /// (`object-fit: fill`). Edges snap like fills; source indices come from
    /// integer math only, so sampling is deterministic across runs. The
    /// transform maps the destination rect (ADR 0024).
    fn draw_image(&mut self, rect: &Rect, image: &crate::image::DecodedImage) {
        let src_w = i64::from(image.width.max(1));
        let src_h = i64::from(image.height.max(1));
        if self.transform.is_axis_aligned() {
            let placed = self.transform.apply_rect(rect);
            let x0 = placed.x.round().max(0.0) as i64;
            let y0 = placed.y.round().max(0.0) as i64;
            let x1 = (placed.x + placed.w).round().clamp(0.0, self.width as f32) as i64;
            let y1 = (placed.y + placed.h).round().clamp(0.0, self.height as f32) as i64;
            let (rows0, rows1) = self.damage_rows(y0, y1);
            let dst_w = (x1 - x0).max(1);
            let dst_h = (y1 - y0).max(1);
            for dy in rows0..rows1 {
                let sy = ((dy - y0) * src_h / dst_h) as usize;
                for dx in x0..x1 {
                    if !self.clip_contains(dx, dy) {
                        continue;
                    }
                    let sx = ((dx - x0) * src_w / dst_w) as usize;
                    let i = (sy * src_w as usize + sx) * 4;
                    let color = self.modulate(Color::from_rgba8(
                        image.rgba[i],
                        image.rgba[i + 1],
                        image.rgba[i + 2],
                        image.rgba[i + 3],
                    ));
                    self.blend_pixel(dx as usize, dy as usize, color);
                }
            }
        } else if let Some(inv) = self.transform.inverse() {
            let (x0, y0, x1, y1) = transform_bounds(&self.transform, rect, self.width, self.height);
            let rect_w = rect.w.max(1.0);
            let rect_h = rect.h.max(1.0);
            for py in y0..y1 {
                for px in x0..x1 {
                    if !self.clip_contains(px as i64, py as i64) {
                        continue;
                    }
                    let (lx, ly) = inv.apply_point(px as f32 + 0.5, py as f32 + 0.5);
                    if lx >= rect.x && lx < rect.x + rect.w && ly >= rect.y && ly < rect.y + rect.h
                    {
                        let sx = (((lx - rect.x) / rect_w * src_w as f32) as i64)
                            .clamp(0, src_w - 1) as usize;
                        let sy = (((ly - rect.y) / rect_h * src_h as f32) as i64)
                            .clamp(0, src_h - 1) as usize;
                        let i = (sy * src_w as usize + sx) * 4;
                        let color = self.modulate(Color::from_rgba8(
                            image.rgba[i],
                            image.rgba[i + 1],
                            image.rgba[i + 2],
                            image.rgba[i + 3],
                        ));
                        self.blend_pixel(px as usize, py as usize, color);
                    }
                }
            }
        }
    }

    /// Draws one run: glyphs are placed so the run's *baseline* sits at
    /// `run.baseline_y`, matching the layout pass's line-box centering.
    /// Pixels outside the active clip are skipped. Under a scaled
    /// transform (ADR 0024) the pen position maps through the matrix and
    /// the raster size scales by the transform's scale factor (rounded to
    /// a whole-pixel size — fontdue's deterministic whole-pixel contract).
    fn draw_text(&mut self, fonts: &mut FontStore, run: &TextRunPlacement<'_>) {
        let weight = if run.bold {
            crate::font::FontWeight::Bold
        } else {
            crate::font::FontWeight::Regular
        };
        let face = fonts.face_index(run.family.as_deref(), weight);
        let scale = self.transform.scale_factor();
        let (pen_start, baseline_start) = self
            .transform
            .apply_point(run.x.round(), run.baseline_y.round());
        let px = (run.px as f32 * scale).round().max(1.0) as u16;
        let color = self.modulate(run.color);
        let mut pen_x = pen_start;
        let baseline = baseline_start;
        // Damage rows: glyphs fully outside the damage window skip their
        // coverage scans (the blend damage check already guards writes).
        let (damage_y0, damage_y1) = self
            .damage
            .map(|(_, dy0, _, dy1)| (dy0, dy1))
            .unwrap_or((0, self.height as i64));
        for ch in run.text.chars() {
            let glyph = fonts.glyph(face, ch, px);
            let advance = glyph.advance * scale;
            let bitmap_left = pen_x as i64 + glyph.xmin as i64;
            let bitmap_top = baseline as i64 - glyph.ymin as i64 - glyph.height as i64;
            let bitmap_bottom = bitmap_top + glyph.height as i64;
            if bitmap_bottom <= damage_y0 || bitmap_top >= damage_y1 {
                self.glyphs += 1;
                pen_x += advance;
                continue;
            }
            for (row, coverage_row) in glyph.coverage.chunks_exact(glyph.width.max(1)).enumerate() {
                let dy = bitmap_top + row as i64;
                if dy < damage_y0 || dy >= damage_y1 {
                    continue;
                }
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
                    let shade =
                        Color::from_rgba8(color.r, color.g, color.b, scale_u8(*alpha, color.a));
                    self.blend_pixel(dx as usize, dy as usize, shade);
                }
            }
            self.glyphs += 1;
            pen_x += advance;
        }
    }
}

/// Per-item device-pixel bounds, in paint order — **one entry per item**
/// (index-aligned with `items`; scoping items that paint nothing carry the
/// empty [`Rect::default`]). Transform scopes are resolved
/// (`PushTransform`/`PopTransform`); clip scopes only shrink what an item
/// paints and are ignored (over-damage is safe, under-damage is not).
/// Every bound is inflated by a small margin for edge snapping and glyph
/// overshoot. The render loop diffs consecutive lists through these bounds
/// to derive the partial-damage window.
pub(crate) fn display_list_bounds(items: &[DisplayItem]) -> Vec<Rect> {
    const MARGIN: f32 = 3.0;
    let mut bounds = Vec::with_capacity(items.len());
    let mut transform = Affine2D::IDENTITY;
    let mut stack: Vec<Affine2D> = Vec::new();
    // Running clip intersection, mirroring the painter's clip semantics
    // (clips live in the scope's parent space, applied by the current
    // transform). Without this, children of a zero-height overflow-hidden
    // container (a morphing card at height 0) carry full unclipped
    // geometry far outside anything they can paint, and the damage diff
    // over-covers from their (item, bounds) mismatches alone.
    let mut clip: Option<Rect> = None;
    let mut clip_stack: Vec<Option<Rect>> = Vec::new();
    for item in items {
        let rect = match item {
            DisplayItem::FillRect { rect, .. }
            | DisplayItem::RoundedFill { rect, .. }
            | DisplayItem::GradientFill { rect, .. }
            | DisplayItem::DrawImage { rect, .. } => Some(*rect),
            DisplayItem::RoundedBorder { outer, .. } => Some(*outer),
            DisplayItem::OuterShadow {
                bounds: b, blur, ..
            } => {
                let reach = blur.max(0.0);
                Some(Rect {
                    x: b.x - reach,
                    y: b.y - reach,
                    w: b.w + 2.0 * reach,
                    h: b.h + 2.0 * reach,
                })
            }
            DisplayItem::TextRun { x, y, text, px, .. } => {
                // Conservative: 1em per character covers every glyph's
                // advance; the vertical band covers ascender overshoot and
                // descenders for whole-pixel sizes.
                let (bx, baseline) = transform.apply_point(x.round(), y.round());
                let px_scaled = f32::from(*px) * transform.scale_factor();
                let width = text.chars().count() as f32 * px_scaled.max(1.0);
                Some(Rect {
                    x: bx,
                    y: baseline - 2.0 * px_scaled.max(1.0),
                    w: width,
                    h: 3.0 * px_scaled.max(1.0),
                })
            }
            DisplayItem::PushTransform { matrix } => {
                stack.push(transform);
                transform = transform.compose(matrix);
                None
            }
            DisplayItem::PopTransform => {
                transform = stack.pop().unwrap_or(Affine2D::IDENTITY);
                None
            }
            DisplayItem::PushClip(clip_rect) => {
                clip_stack.push(clip);
                let placed = transform.apply_rect(clip_rect);
                clip = Some(match clip {
                    Some(outer) => intersect_rects(outer, placed),
                    None => placed,
                });
                None
            }
            DisplayItem::PushClipRounded { rect, .. } => {
                clip_stack.push(clip);
                let placed = transform.apply_rect(rect);
                clip = Some(match clip {
                    Some(outer) => intersect_rects(outer, placed),
                    None => placed,
                });
                None
            }
            DisplayItem::PopClip => {
                clip = clip_stack.pop().flatten();
                None
            }
            DisplayItem::PushOpacity(_) | DisplayItem::PopOpacity => None,
        };
        bounds.push(rect.map_or_else(Rect::default, |placed_rect| {
            let placed = transform.apply_rect(&placed_rect);
            let placed = match clip {
                Some(outer) => intersect_rects(outer, placed),
                None => placed,
            };
            Rect {
                x: placed.x - MARGIN,
                y: placed.y - MARGIN,
                w: placed.w + 2.0 * MARGIN,
                h: placed.h + 2.0 * MARGIN,
            }
        }));
    }
    bounds
}

/// Integer-free rect intersection (empty when disjoint).
fn intersect_rects(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    Rect {
        x: x0,
        y: y0,
        w: (x1 - x0).max(0.0),
        h: (y1 - y0).max(0.0),
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
    /// Normalized `font-family` (`None` = bundled default face).
    family: &'a Option<String>,
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

/// The half-open pixel-column interval on row `py` that can possibly pass
/// [`point_in_rounded_rect`] for this shape. Derived from the same corner
/// math (`dx ≤ √(r²−dy²)` on the row), widened by a 2px f32-safety margin;
/// the caller still runs the exact predicate inside the span, so coverage
/// is bit-identical to the full-scan while the outside scan disappears.
/// Empty (or inverted) spans mean the row has no coverage.
fn rounded_row_span(rect: &Rect, radius: f32, py: i64) -> (i64, i64) {
    let r = radius.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
    let cy = py as f32 + 0.5;
    if r <= 0.0 {
        // floor/ceil (not round): the span must include partially covered
        // edge pixels under coverage rasterization (ADR 0026).
        let x0 = rect.x.floor().max(0.0) as i64;
        let x1 = (rect.x + rect.w).ceil().clamp(0.0, i64::MAX as f32) as i64;
        return (x0, x1);
    }
    let dy = (rect.y + r - cy).max(cy - (rect.y + rect.h - r)).max(0.0);
    if dy > r {
        return (0, 0);
    }
    let dx_max = (r * r - dy * dy).sqrt();
    let span0 = (rect.x + r - dx_max).floor().max(0.0) as i64;
    let span1 = (rect.x + rect.w - r + dx_max).ceil().clamp(0.0, i64::MAX as f32) as i64;
    (span0, span1)
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

/// Fraction of the unit interval [px, px+1] covered by [lo, hi] — the
/// exact straight-edge coverage fraction (ADR 0026).
fn span_coverage(px: f32, lo: f32, hi: f32) -> f32 {
    (hi.min(px + 1.0) - lo.max(px)).clamp(0.0, 1.0)
}

/// Signed distance to a uniform-radius rounded-rectangle boundary:
/// negative inside, positive outside. Pure `f32` (one `sqrt`), so the
/// classification is bit-identical across runs (ADR 0026). `sd ≤ 0` is
/// equivalent to [`point_in_rounded_rect`].
/// Half-extent minus radius on the x axis (the SDF's inset rectangle).
fn outer_hw(radius: f32, rect: &Rect) -> f32 {
    (rect.w / 2.0 - radius.min(rect.w / 2.0).min(rect.h / 2.0)).max(0.0)
}

/// Half-extent minus radius on the y axis.
fn outer_hh(radius: f32, rect: &Rect) -> f32 {
    (rect.h / 2.0 - radius.min(rect.w / 2.0).min(rect.h / 2.0)).max(0.0)
}

fn rounded_rect_sdf(x: f32, y: f32, rect: &Rect, radius: f32) -> f32 {
    let r = radius.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
    let hw = rect.w / 2.0 - r;
    let hh = rect.h / 2.0 - r;
    let qx = (x - (rect.x + rect.w / 2.0)).abs() - hw;
    let qy = (y - (rect.y + rect.h / 2.0)).abs() - hh;
    let dx = qx.max(0.0);
    let dy = qy.max(0.0);
    (dx * dx + dy * dy).sqrt() + qx.max(qy).min(0.0) - r
}

/// Fraction of the fixed `SUBGRID²` samples of device pixel `(px, py)`
/// inside the uniform-radius rounded rect.
fn rounded_subsample_coverage(rect: &Rect, radius: f32, px: f32, py: f32) -> f32 {
    let mut inside = 0usize;
    for sy in SUB_OFFSETS {
        for sx in SUB_OFFSETS {
            if point_in_rounded_rect(px + sx, py + sy, rect, radius) {
                inside += 1;
            }
        }
    }
    inside as f32 / (SUBGRID * SUBGRID) as f32
}

/// Ring coverage: outer samples minus inner samples, clamped at zero
/// (the inner shape nests inside the outer, so the difference is the
/// exact per-sample ring area).
fn ring_subsample_coverage(
    outer: &Rect,
    radius: f32,
    inner: &Rect,
    inner_radii: &[f32; 4],
    px: f32,
    py: f32,
) -> f32 {
    let mut outer_inside = 0usize;
    let mut inner_inside = 0usize;
    for sy in SUB_OFFSETS {
        for sx in SUB_OFFSETS {
            let x = px + sx;
            let y = py + sy;
            if point_in_rounded_rect(x, y, outer, radius) {
                outer_inside += 1;
            }
            if point_in_rounded_rect4(x, y, inner, inner_radii) {
                inner_inside += 1;
            }
        }
    }
    // saturating: a degenerate ring (outer == inner bounds) can count a
    // sample on the shared edge inside the inner shape only — measure
    // zero, deterministic, never worth a negative alpha.
    (outer_inside.saturating_sub(inner_inside) as f32 / (SUBGRID * SUBGRID) as f32).max(0.0)
}

/// True when the whole pixel is certainly inside the per-corner rounded
/// rect: at least [`SDF_MARGIN`] inside every straight edge and outside
/// every corner zone expanded by the margin (the rounded shape is the
/// plain rect minus corner cutouts, so this is a conservative
/// whole-pixel-in-hole test). Used to skip the ring's interior.
fn pixel_deep_inside_inner(cx: f32, cy: f32, rect: &Rect, radii: &[f32; 4]) -> bool {
    let r_max = radii[0].max(radii[1]).max(radii[2]).max(radii[3]);
    let x0 = rect.x;
    let y0 = rect.y;
    let x1 = rect.x + rect.w;
    let y1 = rect.y + rect.h;
    if cx < x0 + SDF_MARGIN
        || cx > x1 - SDF_MARGIN
        || cy < y0 + SDF_MARGIN
        || cy > y1 - SDF_MARGIN
    {
        return false;
    }
    let zone = r_max.max(0.0) + SDF_MARGIN;
    if cx < x0 + zone && cy < y0 + zone {
        return false;
    }
    if cx > x1 - zone && cy < y0 + zone {
        return false;
    }
    if cx > x1 - zone && cy > y1 - zone {
        return false;
    }
    if cx < x0 + zone && cy > y1 - zone {
        return false;
    }
    true
}

/// Coverage of a shape (tested in local space through `inv`) over the
/// fixed `SUBGRID²` samples of device pixel `(px, py)` — the rotated /
/// transformed counterpart of the direct subsample functions.
fn subsample_coverage(
    inv: &Affine2D,
    px: f32,
    py: f32,
    inside: impl Fn(f32, f32) -> bool,
) -> f32 {
    let mut hits = 0usize;
    for sy in SUB_OFFSETS {
        for sx in SUB_OFFSETS {
            let (lx, ly) = inv.apply_point(px + sx, py + sy);
            if inside(lx, ly) {
                hits += 1;
            }
        }
    }
    hits as f32 / (SUBGRID * SUBGRID) as f32
}

/// The gradient coordinate at a local-space point: linear gradients
/// project onto the gradient line (`0` at start, `1` at end); radial
/// gradients take the normalized elliptical distance (1 on the shape).
fn gradient_project(gradient: &ResolvedGradient, x: f32, y: f32) -> f32 {
    match &gradient.kind {
        ResolvedGradientKind::Linear { start, end } => {
            let dx = end.0 - start.0;
            let dy = end.1 - start.1;
            ((x - start.0) * dx + (y - start.1) * dy) / (dx * dx + dy * dy).max(1e-6)
        }
        ResolvedGradientKind::Radial { center, rx, ry } => {
            (((x - center.0) / rx).powi(2) + ((y - center.1) / ry).powi(2)).sqrt()
        }
    }
}

/// Evaluates the stop list at `t`: end colors clamp, the bracketing
/// segment channel-lerps (the animation engine's rounding), and equal
/// adjacent positions act as hard stops (later color wins at the seam).
fn gradient_color_at(stops: &[(f32, Color)], t: f32) -> Color {
    let first = &stops[0];
    if t <= first.0 {
        return first.1;
    }
    let last = &stops[stops.len() - 1];
    if t >= last.0 {
        return last.1;
    }
    let mut index = 0usize;
    while index + 2 < stops.len() && stops[index + 1].0 <= t {
        index += 1;
    }
    let (p0, c0) = stops[index];
    let (p1, c1) = stops[index + 1];
    let span = p1 - p0;
    let u = if span > 1e-6 {
        ((t - p0) / span).clamp(0.0, 1.0)
    } else if t >= p1 {
        1.0
    } else {
        0.0
    };
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * u).round() as u8;
    Color::from_rgba8(
        channel(c0.r, c1.r),
        channel(c0.g, c1.g),
        channel(c0.b, c1.b),
        channel(c0.a, c1.a),
    )
}

/// Computes the pixel bounds of a transformed rectangle, clamped to the viewport.
fn transform_bounds(
    transform: &Affine2D,
    rect: &Rect,
    viewport_w: u32,
    viewport_h: u32,
) -> (u32, u32, u32, u32) {
    let p0 = transform.apply_point(rect.x, rect.y);
    let p1 = transform.apply_point(rect.x + rect.w, rect.y);
    let p2 = transform.apply_point(rect.x + rect.w, rect.y + rect.h);
    let p3 = transform.apply_point(rect.x, rect.y + rect.h);
    let min_x = p0.0.min(p1.0).min(p2.0).min(p3.0);
    let max_x = p0.0.max(p1.0).max(p2.0).max(p3.0);
    let min_y = p0.1.min(p1.1).min(p2.1).min(p3.1);
    let max_y = p0.1.max(p1.1).max(p2.1).max(p3.1);
    let x0 = min_x.floor().max(0.0) as u32;
    let y0 = min_y.floor().max(0.0) as u32;
    let x1 = max_x.ceil().clamp(0.0, viewport_w as f32) as u32;
    let y1 = max_y.ceil().clamp(0.0, viewport_h as f32) as u32;
    (x0, y0, x1, y1)
}
