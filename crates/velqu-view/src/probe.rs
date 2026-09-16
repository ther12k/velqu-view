//! The M1 *paint probe* scene.
//!
//! This is a deliberately explicit debug scene — not fake HTML rendering.
//! It exists so the M1 exit gate ("deterministic Hello fixture") exercises
//! the whole pipeline offscreen: background fill, rectangles (solid +
//! outlined), text in two weights, DPI-derived status text, and content
//! derived from the loaded document (byte sizes prove `load_html`/`load_css`
//! actually feed the renderer).
//!
//! M2 replaces this with the HTML/style/layout pipeline lowering into the
//! same [`Scene`] primitives.

use crate::Viewport;
use crate::scene::{FontWeight, Item, Scene};

/// Palette (dark slate + Tailwind-ish accents) for the probe.
mod palette {
    use crate::color::Color;
    pub(crate) const BACKGROUND: Color = Color::from_rgb8(0x10, 0x14, 0x1c);
    pub(crate) const TITLE: Color = Color::from_rgb8(0xf4, 0xf6, 0xfb);
    pub(crate) const SUBTITLE: Color = Color::from_rgb8(0x93, 0xa0, 0xb4);
    pub(crate) const STATUS: Color = Color::from_rgb8(0x4a, 0xde, 0x80);
    pub(crate) const FOOTER: Color = Color::from_rgb8(0x66, 0x73, 0x8c);
    pub(crate) const ACCENT: Color = Color::from_rgb8(0x3b, 0x82, 0xf6);
    pub(crate) const BORDER: Color = Color::from_rgb8(0x2a, 0x32, 0x42);
    pub(crate) const SWATCHES: [Color; 5] = [
        Color::from_rgb8(0xef, 0x44, 0x44),
        Color::from_rgb8(0x22, 0xc5, 0x5e),
        Color::from_rgb8(0x3b, 0x82, 0xf6),
        Color::from_rgb8(0xf5, 0x9e, 0x0b),
        Color::from_rgb8(0x8b, 0x5c, 0xf6),
    ];
}

/// Swatch geometry in logical pixels; fixture pixel probes aim at the centers.
pub(crate) const SWATCH_SIZE: f32 = 24.0;
pub(crate) const SWATCH_GAP: f32 = 8.0;
pub(crate) const SWATCH_Y: f32 = 132.0;
pub(crate) const SWATCH_X0: f32 = 32.0;

/// Builds the probe scene for the given document sources and viewport.
pub(crate) fn build(
    document: &crate::DocumentSource,
    stylesheets: &[crate::StylesheetSource],
    viewport: Viewport,
) -> Scene {
    let mut scene = Scene::new(palette::BACKGROUND);

    // Viewport frame (inset outline) — proves logical geometry scales with DPI.
    let inset = 16.0;
    let lw = viewport.logical_width();
    let lh = viewport.logical_height();
    scene.push(Item::RectOutline {
        x: inset,
        y: inset,
        w: (lw - 2.0 * inset).max(0.0),
        h: (lh - 2.0 * inset).max(0.0),
        thickness: 1.0,
        color: palette::BORDER,
    });

    // Header.
    scene.push(Item::Text {
        x: 32.0,
        y: 28.0,
        text: "VelquView".into(),
        size: 28.0,
        color: palette::TITLE,
        weight: FontWeight::Bold,
    });
    scene.push(Item::Text {
        x: 32.0,
        y: 66.0,
        text: "M1 native paint probe — window, rectangle, text, DPI".into(),
        size: 14.0,
        color: palette::SUBTITLE,
        weight: FontWeight::Regular,
    });
    scene.push(Item::Rect {
        x: 32.0,
        y: 96.0,
        w: 64.0,
        h: 4.0,
        color: palette::ACCENT,
    });

    // Exact-color swatches: the fixture's deterministic pixel probes.
    for (i, color) in palette::SWATCHES.iter().enumerate() {
        scene.push(Item::Rect {
            x: SWATCH_X0 + i as f32 * (SWATCH_SIZE + SWATCH_GAP),
            y: SWATCH_Y,
            w: SWATCH_SIZE,
            h: SWATCH_SIZE,
            color: *color,
        });
    }
    scene.push(Item::Text {
        x: SWATCH_X0,
        y: SWATCH_Y + SWATCH_SIZE + 6.0,
        text: "swatches / exact-pixel conformance".into(),
        size: 11.0,
        color: palette::FOOTER,
        weight: FontWeight::Regular,
    });

    // Status lines: viewport/DPI and loaded document facts.
    let status = format!(
        "viewport {}x{} px @ {:.2}x scale",
        viewport.width(),
        viewport.height(),
        viewport.scale_factor()
    );
    scene.push(Item::Text {
        x: 32.0,
        y: SWATCH_Y + 56.0,
        text: status,
        size: 12.0,
        color: palette::STATUS,
        weight: FontWeight::Regular,
    });

    let css_bytes: usize = stylesheets.iter().map(|sheet| sheet.css.len()).sum();
    let doc = format!(
        "document html {} B / css {} B in {} sheet(s)",
        document.html.len(),
        css_bytes,
        stylesheets.len()
    );
    scene.push(Item::Text {
        x: 32.0,
        y: SWATCH_Y + 76.0,
        text: doc,
        size: 12.0,
        color: palette::STATUS,
        weight: FontWeight::Regular,
    });

    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DocumentSource, StylesheetSource};

    fn vp(w: u32, h: u32, scale: f32) -> Viewport {
        Viewport::try_new(w, h, scale).unwrap()
    }

    fn probe_items(html: &str, css: &[&str], viewport: Viewport) -> Vec<Item> {
        let sheets: Vec<StylesheetSource> = css
            .iter()
            .enumerate()
            .map(|(i, css)| StylesheetSource::new(format!("t{i}.css"), *css))
            .collect();
        build(&DocumentSource::new("index.html", html), &sheets, viewport).items
    }

    #[test]
    fn scene_shape_is_stable() {
        let items = probe_items("<html></html>", &[], vp(800, 600, 1.0));
        // outline + title + subtitle + accent + 5 swatches + label + 2 status lines.
        assert_eq!(items.len(), 12);
        assert!(items.iter().any(|i| matches!(i, Item::RectOutline { .. })));
        assert_eq!(
            items
                .iter()
                .filter(|i| matches!(i, Item::Text { .. }))
                .count(),
            5
        );
    }

    #[test]
    fn swatch_layout_math() {
        let items = probe_items("<html></html>", &[], vp(800, 600, 1.0));
        let swatches: Vec<&Item> = items
            .iter()
            .filter(|i| matches!(i, Item::Rect { y, h, .. } if *y == SWATCH_Y && *h == SWATCH_SIZE))
            .collect();
        assert_eq!(swatches.len(), 5);
        let Item::Rect { x, w, .. } = *swatches[4] else {
            panic!("expected rect");
        };
        assert!((x - (SWATCH_X0 + 4.0 * (SWATCH_SIZE + SWATCH_GAP))).abs() < f32::EPSILON);
        assert!((w - SWATCH_SIZE).abs() < f32::EPSILON);
    }
}
