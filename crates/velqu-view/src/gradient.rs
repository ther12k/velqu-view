//! `linear-gradient`/`radial-gradient` background layers (P0 of the
//! notch-port work list).
//!
//! Parsing produces the abstract form (direction, shape, stop list);
//! geometry resolves at display-list emission ([`Gradient::resolve`])
//! where the padding box is known in device pixels. Stop positions
//! follow the CSS Images rules the reference styles use: percentages of
//! the gradient line (linear) or shape radii (radial), px positions
//! scaled by the same reference, missing positions distributed evenly,
//! adjacent equal positions as hard stops (the notch corner-carve
//! `transparent 9.5px, #000 10px` pattern), and clamping so positions
//! never go backwards.
//!
//! v1 profile: one layer per element; `var()` inside stops is not
//! resolved (the declaration is skipped with a diagnostic — custom
//! properties are a separate cascade feature).

use crate::color::Color;
use crate::display_list::{Rect, ResolvedGradient, ResolvedGradientKind};

/// One authored `linear-gradient`/`radial-gradient` layer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Gradient {
    pub(crate) kind: GradientKind,
    /// Stops in authored order (the first stop sits at the gradient's
    /// start). At least one; empty lists never parse.
    pub(crate) stops: Vec<GradientStop>,
}

/// The gradient's direction (linear) or shape (radial).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum GradientKind {
    Linear {
        direction: GradientDirection,
    },
    Radial {
        shape: RadialShape,
        size: RadialSize,
        /// `at <x> <y>`; `None` axes default to the box center.
        position: (Option<GradientLength>, Option<GradientLength>),
    },
}

/// A linear gradient's direction: an authored angle (degrees clockwise
/// from "to top") or a corner keyword whose angle depends on the box
/// aspect (resolved at [`Gradient::resolve`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum GradientDirection {
    Angle(f32),
    Corner {
        right: bool,
        bottom: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RadialShape {
    Circle,
    Ellipse,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RadialSize {
    /// CSS default: the shape touches the farthest corner.
    FarthestCorner,
    /// Two explicit sizes (ellipse): width/height in px or % of the box.
    Ellipse {
        rx: GradientLength,
        ry: GradientLength,
    },
    /// One explicit size (circle): a px radius.
    Circle {
        r: GradientLength,
    },
}

/// A stop position or explicit radial size: px or a percentage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum GradientLength {
    Px(f32),
    Percent(f32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GradientStop {
    pub color: Color,
    pub position: Option<GradientLength>,
}

/// Parses a `linear-gradient(...)`/`radial-gradient(...)` value.
pub(crate) fn parse_gradient(value: &str) -> Option<Gradient> {
    let value = value.trim();
    if let Some(inner) = function_body(value, "linear-gradient") {
        return parse_linear_gradient(inner);
    }
    if let Some(inner) = function_body(value, "radial-gradient") {
        return parse_radial_gradient(inner);
    }
    None
}

fn function_body<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    value
        .strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('('))
        .filter(|rest| rest.ends_with(')'))
        .map(|rest| &rest[..rest.len() - 1])
}

fn parse_linear_gradient(inner: &str) -> Option<Gradient> {
    let segments = split_top_level_commas(inner);
    let (direction, stop_segments) = match segments.first().map(|s| s.trim()) {
        Some(first) if first.starts_with("to ") => (
            parse_direction(first)?,
            &segments[1..],
        ),
        Some(first) if first.ends_with("deg") => {
            let angle = first[..first.len() - 3].trim().parse::<f32>().ok()?;
            (
                GradientDirection::Angle(angle),
                &segments[1..],
            )
        }
        _ => (GradientDirection::Angle(180.0), segments.as_slice()),
    };
    let stops = parse_stops(stop_segments)?;
    Some(Gradient {
        kind: GradientKind::Linear { direction },
        stops,
    })
}

/// `to top`, `to right`, `to bottom right`, …
fn parse_direction(value: &str) -> Option<GradientDirection> {
    let words: Vec<&str> = value.split_ascii_whitespace().collect();
    if words.first() != Some(&"to") {
        return None;
    }
    match words.len() {
        2 => match words[1] {
            "top" => Some(GradientDirection::Angle(0.0)),
            "right" => Some(GradientDirection::Angle(90.0)),
            "bottom" => Some(GradientDirection::Angle(180.0)),
            "left" => Some(GradientDirection::Angle(270.0)),
            _ => None,
        },
        3 => {
            // (right, bottom) flags for the corner diagonal.
            let flags = match (words[1], words[2]) {
                ("top", "right") | ("right", "top") => (true, false),
                ("bottom", "right") | ("right", "bottom") => (true, true),
                ("bottom", "left") | ("left", "bottom") => (false, true),
                ("top", "left") | ("left", "top") => (false, false),
                _ => return None,
            };
            Some(GradientDirection::Corner {
                right: flags.0,
                bottom: flags.1,
            })
        }
        _ => None,
    }
}

fn parse_radial_gradient(inner: &str) -> Option<Gradient> {
    let segments = split_top_level_commas(inner);
    let (shape_part, at_part, stop_segments) = match segments.first().map(|s| s.trim()) {
        // The first segment holds the shape/size/`at` only when it is not
        // itself a color stop: a bare color parses as a stop, so probe
        // for the known shape syntax pieces first.
        Some(first)
            if first.contains(" at ")
                || first == "circle"
                || first == "ellipse"
                || first == "circle farthest-corner"
                || first == "ellipse farthest-corner"
                || is_size_pair(first) =>
        {
            let (shape_part, at_part) = match first.split_once(" at ") {
                Some((shape, at)) => (shape.trim(), Some(at.trim())),
                None => (first, None),
            };
            (shape_part, at_part, &segments[1..])
        }
        _ => ("", None, segments.as_slice()),
    };
    let (shape, size) = parse_radial_shape_size(shape_part)?;
    let position = match at_part {
        Some(at) => parse_gradient_point(at)?,
        None => (None, None),
    };
    let stops = parse_stops(stop_segments)?;
    Some(Gradient {
        kind: GradientKind::Radial {
            shape,
            size,
            position,
        },
        stops,
    })
}

/// Does the text look like two explicit sizes (`120% 140%`, `500px
/// 200px`)? Used to distinguish the size header from a first color stop.
fn is_size_pair(value: &str) -> bool {
    let mut lengths = 0usize;
    for token in value.split_ascii_whitespace() {
        if parse_gradient_length(token).is_some() {
            lengths += 1;
        }
    }
    lengths == 2
}

fn parse_radial_shape_size(value: &str) -> Option<(RadialShape, RadialSize)> {
    let value = value.trim();
    if value.is_empty() {
        return Some((RadialShape::Ellipse, RadialSize::FarthestCorner));
    }
    let mut words = value.split_ascii_whitespace();
    let first = words.next()?;
    match first {
        "circle" => {
            let size = match words.next() {
                None => RadialSize::FarthestCorner,
                Some("farthest-corner") => RadialSize::FarthestCorner,
                Some(len) => RadialSize::Circle {
                    r: px_only(parse_gradient_length(len)?, "circle size")?,
                },
            };
            Some((RadialShape::Circle, size))
        }
        "ellipse" => {
            let size = match words.next() {
                None => RadialSize::FarthestCorner,
                Some("farthest-corner") => RadialSize::FarthestCorner,
                Some(len) => RadialSize::Ellipse {
                    rx: parse_gradient_length(len)?,
                    ry: words
                        .next()
                        .and_then(parse_gradient_length)
                        .unwrap_or(len_px_or_percent(len)),
                },
            };
            Some((RadialShape::Ellipse, size))
        }
        _ => {
            // No shape keyword: one length = circle radius, two = ellipse.
            let rx = parse_gradient_length(first)?;
            match words.next() {
                None => Some((
                    RadialShape::Circle,
                    RadialSize::Circle {
                        r: px_only(rx, "circle size")?,
                    },
                )),
                Some(second) => Some((
                    RadialShape::Ellipse,
                    RadialSize::Ellipse {
                        rx,
                        ry: parse_gradient_length(second)?,
                    },
                )),
            }
        }
    }
}

/// Percent sizes are meaningless for a circle radius in this profile.
fn px_only(length: GradientLength, what: &str) -> Option<GradientLength> {
    match length {
        GradientLength::Px(_) => Some(length),
        GradientLength::Percent(_) => {
            let _ = what;
            None
        }
    }
}

fn len_px_or_percent(value: &str) -> GradientLength {
    parse_gradient_length(value).unwrap_or(GradientLength::Percent(0.0))
}

/// `at <x> <y>` with px/% lengths or the side keywords.
fn parse_gradient_point(value: &str) -> Option<(Option<GradientLength>, Option<GradientLength>)> {
    let mut words = value.split_ascii_whitespace();
    let x = words.next()?;
    let y = words.next().unwrap_or("center");
    if words.next().is_some() {
        return None;
    }
    Some((
        parse_gradient_axis(x),
        parse_gradient_axis(y),
    ))
}

fn parse_gradient_axis(word: &str) -> Option<GradientLength> {
    match word {
        "left" | "top" => Some(GradientLength::Percent(0.0)),
        "center" => Some(GradientLength::Percent(50.0)),
        "right" | "bottom" => Some(GradientLength::Percent(100.0)),
        other => parse_gradient_length(other),
    }
}

fn parse_gradient_length(value: &str) -> Option<GradientLength> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        return percent.trim().parse::<f32>().ok().map(GradientLength::Percent);
    }
    if let Some(px) = value.strip_suffix("px") {
        return px.trim().parse::<f32>().ok().map(GradientLength::Px);
    }
    // CSS unitless zero is a valid length (`circle at 0 100%`).
    if value == "0" {
        return Some(GradientLength::Px(0.0));
    }
    None
}

/// Parses the comma-separated stop list: each stop is a color with an
/// optional position (either `<color> <pos>` or `<pos> <color>`).
fn parse_stops(segments: &[&str]) -> Option<Vec<GradientStop>> {
    if segments.is_empty() {
        return None;
    }
    let mut stops = Vec::with_capacity(segments.len());
    for segment in segments {
        let tokens: Vec<&str> = segment
            .split_ascii_whitespace()
            // Keep `rgba(1, 2, 3, 0.5)` in one token: split on top-level
            // whitespace only (commas inside functions are part of the
            // color), so a crude split works because color functions
            // never contain whitespace at depth 0... they can (`rgb(1 2
            // / 0.5)`), so join tokens until parens balance.
            .collect();
        let tokens = balanced_join(&tokens);
        match tokens.len() {
            1 => stops.push(GradientStop {
                color: crate::style::parse_color(&tokens[0])?,
                position: None,
            }),
            2 => {
                let (color, position) = if parse_gradient_length(&tokens[1]).is_some() {
                    (
                        crate::style::parse_color(&tokens[0])?,
                        parse_gradient_length(&tokens[1])?,
                    )
                } else {
                    (
                        crate::style::parse_color(&tokens[1])?,
                        parse_gradient_length(&tokens[0])?,
                    )
                };
                stops.push(GradientStop {
                    color,
                    position: Some(position),
                });
            }
            _ => return None,
        }
    }
    Some(stops)
}

/// Merges whitespace-split tokens back into balanced function calls
/// (`rgb(1 2 / 0.5)`, `rgba(1, 2, 3, 0.5)` become single tokens).
fn balanced_join(tokens: &[&str]) -> Vec<String> {
    let mut joined = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for token in tokens {
        if depth == 0 && !current.is_empty() {
            joined.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(token);
        depth += token.matches('(').count();
        depth -= token.matches(')').count();
    }
    if !current.is_empty() {
        joined.push(current);
    }
    joined
}

fn split_top_level_commas(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&value[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
}

impl Gradient {
    /// Resolves the abstract gradient into paint-ready device geometry
    /// and stop fractions for `box_rect` (the padding box, device px).
    pub(crate) fn resolve(&self, box_rect: &Rect) -> ResolvedGradient {
        let w = box_rect.w;
        let h = box_rect.h;
        let kind = match &self.kind {
            GradientKind::Linear { direction } => {
                let (dx, dy) = match *direction {
                    GradientDirection::Angle(angle) => {
                        let rad = angle.to_radians();
                        // 0deg points up (−y); clockwise positive.
                        (rad.sin(), -rad.cos())
                    }
                    GradientDirection::Corner { right, bottom } => {
                        // CSS corner rule: the gradient line points at the
                        // corner — the unit direction is the box diagonal.
                        let norm = (w * w + h * h).sqrt().max(1e-6);
                        (
                            if right { w / norm } else { -w / norm },
                            if bottom { h / norm } else { -h / norm },
                        )
                    }
                };
                // The line covering the whole box through the center.
                let len = (w * dx.abs() + h * dy.abs()).max(1e-6);
                let cx = box_rect.x + w / 2.0;
                let cy = box_rect.y + h / 2.0;
                ResolvedGradientKind::Linear {
                    start: (cx - dx * len / 2.0, cy - dy * len / 2.0),
                    end: (cx + dx * len / 2.0, cy + dy * len / 2.0),
                }
            }
            GradientKind::Radial {
                shape,
                size,
                position,
            } => {
                let cx = box_rect.x
                    + position
                        .0
                        .map(|len| resolve_axis(len, w))
                        .unwrap_or(w / 2.0);
                let cy = box_rect.y
                    + position
                        .1
                        .map(|len| resolve_axis(len, h))
                        .unwrap_or(h / 2.0);
                let (rx, ry) = match (shape, size) {
                    (RadialShape::Circle, RadialSize::Circle { r }) => {
                        let radius = resolve_axis(*r, w).max(1e-3);
                        (radius, radius)
                    }
                    (
                        RadialShape::Circle,
                        RadialSize::FarthestCorner,
                    ) => {
                        let radius = farthest_corner_distance(box_rect, cx, cy).max(1e-3);
                        (radius, radius)
                    }
                    (
                        RadialShape::Ellipse,
                        RadialSize::Ellipse { rx, ry },
                    ) => (resolve_axis(*rx, w).max(1e-3), resolve_axis(*ry, h).max(1e-3)),
                    (RadialShape::Ellipse, RadialSize::FarthestCorner) => {
                        let rx = (box_rect.x - cx)
                            .abs()
                            .max((box_rect.x + w - cx).abs())
                            .max(1e-3);
                        let ry = (box_rect.y - cy)
                            .abs()
                            .max((box_rect.y + h - cy).abs())
                            .max(1e-3);
                        (rx, ry)
                    }
                    // A circle with an ellipse size pair: use rx for both.
                    (RadialShape::Circle, RadialSize::Ellipse { rx, .. }) => {
                        let radius = resolve_axis(*rx, w).max(1e-3);
                        (radius, radius)
                    }
                    // An ellipse with a single circle size: ry mirrors rx.
                    (RadialShape::Ellipse, RadialSize::Circle { r }) => {
                        let radius = resolve_axis(*r, w).max(1e-3);
                        (radius, radius)
                    }
                };
                ResolvedGradientKind::Radial { center: (cx, cy), rx, ry }
            }
        };
        let reference = match &kind {
            ResolvedGradientKind::Linear { start, end } => {
                let dx = end.0 - start.0;
                let dy = end.1 - start.1;
                (dx * dx + dy * dy).sqrt().max(1e-6)
            }
            ResolvedGradientKind::Radial { rx, .. } => *rx,
        };
        ResolvedGradient {
            kind,
            stops: resolve_stops(&self.stops, reference),
        }
    }
}

fn resolve_axis(length: GradientLength, extent: f32) -> f32 {
    match length {
        GradientLength::Px(px) => px,
        GradientLength::Percent(percent) => percent / 100.0 * extent,
    }
}

fn farthest_corner_distance(rect: &Rect, cx: f32, cy: f32) -> f32 {
    [
        ((rect.x - cx), (rect.y - cy)),
        ((rect.x + rect.w - cx), (rect.y - cy)),
        ((rect.x + rect.w - cx), (rect.y + rect.h - cy)),
        ((rect.x - cx), (rect.y + rect.h - cy)),
    ]
    .iter()
    .map(|(dx, dy)| (dx * dx + dy * dy).sqrt())
    .fold(0.0f32, f32::max)
}

/// Resolves authored stop positions against the gradient reference
/// length: percentages map directly, px divide by the reference.
/// Missing positions fill per CSS (first 0, last 1, interiors midway),
/// then clamp monotonic so equal-adjacent pairs stay hard stops.
fn resolve_stops(stops: &[GradientStop], reference: f32) -> Vec<(f32, Color)> {
    let mut resolved: Vec<(Option<f32>, Color)> = stops
        .iter()
        .map(|stop| {
            (
                stop.position
                    .map(|len| match len {
                        GradientLength::Percent(percent) => percent / 100.0,
                        GradientLength::Px(px) => px / reference,
                    }),
                stop.color,
            )
        })
        .collect();
    if resolved.is_empty() {
        return Vec::new();
    }
    if resolved.first().unwrap().0.is_none() {
        resolved[0].0 = Some(0.0);
    }
    let last = resolved.len() - 1;
    if resolved[last].0.is_none() {
        resolved[last].0 = Some(1.0);
    }
    // Interior gaps: each missing position sits midway between its
    // nearest resolved neighbors.
    let mut i = 0;
    while i < resolved.len() {
        if resolved[i].0.is_some() {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < resolved.len() && resolved[i].0.is_none() {
            i += 1;
        }
        let run_end = i; // first resolved index after the run (exists: last is set)
        let before = resolved[run_start - 1].0.unwrap_or(0.0);
        let after = resolved[run_end].0.unwrap_or(1.0);
        for (offset, index) in (run_start..run_end).enumerate() {
            let t = (offset + 1) as f32 / (run_end - run_start + 1) as f32;
            resolved[index].0 = Some(before + (after - before) * t);
        }
    }
    // Clamp monotonic (CSS: a stop below its predecessor rides up to it).
    let mut previous = 0.0f32;
    resolved
        .into_iter()
        .map(|(position, color)| {
            let position = position.unwrap_or(1.0).max(previous);
            previous = position;
            (position, color)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn parses_linear_angle_and_stops() {
        let gradient = parse_gradient(
            "linear-gradient(160deg, #ffd9c2 0%, rgba(1, 2, 3, 0.5) 100%)",
        )
        .expect("parses");
        let GradientKind::Linear {
            direction: GradientDirection::Angle(angle),
        } = gradient.kind
        else {
            panic!("linear kind");
        };
        assert!((angle - 160.0).abs() < 1e-5);
        assert_eq!(gradient.stops.len(), 2);
        assert_eq!(gradient.stops[0].color, Color::from_hex("#ffd9c2").unwrap());
        assert_eq!(
            gradient.stops[0].position,
            Some(GradientLength::Percent(0.0))
        );
        assert_eq!(
            gradient.stops[1].position,
            Some(GradientLength::Percent(100.0))
        );
    }

    #[test]
    fn parses_direction_keywords() {
        let gradient =
            parse_gradient("linear-gradient(to bottom right, #fff, #000)").expect("parses");
        let GradientKind::Linear {
            direction: GradientDirection::Corner { right, bottom },
        } = gradient.kind
        else {
            panic!("corner kind");
        };
        assert!(right && bottom);
        // `to right` maps to the 90deg clock position.
        let gradient = parse_gradient("linear-gradient(to right, #fff, #000)").unwrap();
        let GradientKind::Linear {
            direction: GradientDirection::Angle(angle),
        } = gradient.kind
        else {
            panic!("angle kind");
        };
        assert!((angle - 90.0).abs() < 1e-5);
    }

    #[test]
    fn parses_ellipse_size_at_position() {
        let gradient = parse_gradient(
            "radial-gradient(120% 140% at 0% 0%, rgba(9, 8, 7, 0.55), transparent 70%)",
        )
        .expect("parses");
        let GradientKind::Radial {
            shape,
            size,
            position,
        } = gradient.kind
        else {
            panic!("radial kind");
        };
        assert_eq!(shape, RadialShape::Ellipse);
        assert_eq!(
            size,
            RadialSize::Ellipse {
                rx: GradientLength::Percent(120.0),
                ry: GradientLength::Percent(140.0),
            }
        );
        assert_eq!(
            position,
            (
                Some(GradientLength::Percent(0.0)),
                Some(GradientLength::Percent(0.0))
            )
        );
        assert_eq!(gradient.stops.len(), 2);
        assert_eq!(gradient.stops[1].position, Some(GradientLength::Percent(70.0)));
    }

    #[test]
    fn parses_circle_at_px_position_with_hard_stops() {
        let gradient = parse_gradient(
            "radial-gradient(circle at 0 100%, transparent 9.5px, #000 10px)",
        )
        .expect("parses");
        let GradientKind::Radial {
            shape,
            size,
            position,
        } = gradient.kind
        else {
            panic!("radial kind");
        };
        assert_eq!(shape, RadialShape::Circle);
        assert_eq!(size, RadialSize::FarthestCorner);
        assert_eq!(
            position,
            (
                Some(GradientLength::Px(0.0)),
                Some(GradientLength::Percent(100.0))
            )
        );
        // The corner-carve hard stop pair.
        assert_eq!(gradient.stops[0].position, Some(GradientLength::Px(9.5)));
        assert_eq!(gradient.stops[1].position, Some(GradientLength::Px(10.0)));
    }

    #[test]
    fn var_in_stops_fails_gracefully() {
        assert!(parse_gradient("linear-gradient(90deg, var(--on), #000)").is_none());
    }

    #[test]
    fn resolves_linear_geometry_for_90deg() {
        // `to right` over a 100×40 box: gradient line from (0, 20) to
        // (100, 20), length 100.
        let gradient = parse_gradient("linear-gradient(to right, #000, #fff)").unwrap();
        let resolved = gradient.resolve(&rect(0.0, 0.0, 100.0, 40.0));
        let ResolvedGradientKind::Linear { start, end } = resolved.kind else {
            panic!("linear");
        };
        assert!((start.0 - 0.0).abs() < 1e-4 && (start.1 - 20.0).abs() < 1e-4);
        assert!((end.0 - 100.0).abs() < 1e-4 && (end.1 - 20.0).abs() < 1e-4);
        assert_eq!(resolved.stops.len(), 2);
        assert!((resolved.stops[0].0).abs() < 1e-6);
        assert!((resolved.stops[1].0 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn resolves_corner_carve_positions_by_radius() {
        // `circle at 0 100%` on a 20×20 box: farthest corner is (20, 0),
        // distance √(20² + 20²) ≈ 28.28. The 10px stop lands at 10/28.28.
        let gradient = parse_gradient(
            "radial-gradient(circle at 0 100%, transparent 9.5px, #000 10px)",
        )
        .unwrap();
        let resolved = gradient.resolve(&rect(0.0, 0.0, 20.0, 20.0));
        let ResolvedGradientKind::Radial { center, rx, ry } = resolved.kind else {
            panic!("radial");
        };
        assert!((center.0 - 0.0).abs() < 1e-5 && (center.1 - 20.0).abs() < 1e-5);
        assert!((rx - ry).abs() < 1e-6, "circle has equal radii");
        let expected = 10.0 / rx;
        assert!((resolved.stops[1].0 - expected).abs() < 1e-5);
        assert!(
            resolved.stops[0].0 < resolved.stops[1].0,
            "9.5px < 10px keeps its hard-stop pair"
        );
    }

    #[test]
    fn missing_positions_distribute_evenly() {
        let gradient = parse_gradient("linear-gradient(to right, #000, #111, #222)").unwrap();
        let resolved = gradient.resolve(&rect(0.0, 0.0, 100.0, 10.0));
        assert_eq!(resolved.stops.len(), 3);
        assert!((resolved.stops[1].0 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn stops_never_go_backwards() {
        let gradient = parse_gradient(
            "linear-gradient(to right, #000 60%, #fff 20%)",
        )
        .unwrap();
        let resolved = gradient.resolve(&rect(0.0, 0.0, 100.0, 10.0));
        assert!(
            resolved.stops[0].0 <= resolved.stops[1].0,
            "clamped monotonic: {:?}",
            resolved.stops
        );
    }
}
