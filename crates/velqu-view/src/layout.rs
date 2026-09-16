//! Box tree and block layout (M2a).
//!
//! Layout identity is distinct from DOM identity (ADR 0005): boxes are
//! rebuilt from scratch on every layout pass. The pipeline is
//! DOM + ComputedStyle → box tree → laid-out boxes → [`LayoutFacts`] and a
//! [`DisplayList`] for painting.
//!
//! M2a scope: normal block flow only — margin/padding/border, width/height/
//! min/max, greedy text wrapping, text-align. Documented simplifications: no
//! margin collapsing (adjacent margins add), percentage heights resolve as
//! auto (the containing height is not fixed in block flow), box-sizing is
//! content-box, and inline boxes flatten into text runs (their
//! backgrounds/borders arrive in M2b).
//!
//! All layout geometry is in **device pixels** — logical CSS px values are
//! multiplied by the viewport scale during resolution so text measurement
//! and painting share one metric space.

use crate::color::Color;
use crate::display_list::{DisplayList, DisplayRect, DisplayText, Rect};
use crate::dom::{Dom, NodeData, NodeId};
use crate::font::FontStore;
use crate::style::{ComputedStyle, Display, Length, LineHeight, Sides, TextAlign};
use crate::text;
use crate::viewport::Viewport;

/// Versioned, fixture-facing layout facts.
///
/// Keys are the author-written `data-vv-test` ids (ADR 0005): internal
/// `NodeId`s never leak into fixtures.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutFacts {
    /// Schema version, bumped on incompatible changes.
    pub schema_version: u32,
    /// Viewport width in device pixels.
    pub viewport_width: u32,
    /// Viewport height in device pixels.
    pub viewport_height: u32,
    /// Device pixels per logical pixel.
    pub scale: f32,
    /// One entry per box whose element carries `data-vv-test`, in document
    /// order.
    pub nodes: Vec<LayoutNodeFact>,
}

/// Current layout facts schema.
pub const LAYOUT_FACTS_SCHEMA_VERSION: u32 = 1;

/// One fixture-identified node's layout facts (device pixels).
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutNodeFact {
    /// The `data-vv-test` attribute value.
    pub fixture_id: String,
    /// Element tag name.
    pub tag: String,
    /// `block` / `inline` / `none`.
    pub display: String,
    /// Border-box geometry.
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Border-box width.
    pub width: f32,
    /// Border-box height.
    pub height: f32,
    /// Content-box geometry.
    /// Left edge.
    pub content_x: f32,
    /// Top edge.
    pub content_y: f32,
    /// Content width.
    pub content_width: f32,
    /// Content height.
    pub content_height: f32,
    /// Resolved box model, CSS order: top, right, bottom, left.
    /// Padding widths.
    pub padding: [f32; 4],
    /// Border widths.
    pub border: [f32; 4],
    /// Margin widths.
    pub margin: [f32; 4],
    /// Laid-out line texts in order.
    pub text_runs: Vec<String>,
}

/// One wrap unit with its own style.
#[derive(Debug, Clone)]
struct Word {
    text: String,
    style: ComputedStyle,
    /// A whitespace separator preceded this word in the source.
    space_before: bool,
    /// Element whose content produced this word (fixture facts).
    source: NodeId,
}

/// A block box: geometry plus inline content lines.
#[derive(Debug, Clone)]
pub(crate) struct BoxNode {
    #[allow(dead_code)] // identity retained for future invalidation work
    pub node: NodeId,
    pub fixture_id: Option<String>,
    pub tag: String,
    pub style: ComputedStyle,
    pub children: Vec<BoxNode>,
    pub lines: Vec<LaidLine>,
    /// Content box (absolute device px).
    pub content: Rect,
    /// Padding box = content + padding.
    pub padding_box: Rect,
    /// Border box = padding box + border.
    pub border_box: Rect,
    /// Resolved margins (device px).
    pub margin: Sides<f32>,
    /// Inline content words, in source order (wrapped during layout).
    words: Vec<Word>,
}

/// One laid-out line (runs positioned relative to the content box).
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct LaidLine {
    /// Y of the line's top relative to the content box.
    pub y: f32,
    pub height: f32,
    pub runs: Vec<RunBox>,
}

/// One positioned run inside a line.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunBox {
    /// X relative to the content box.
    pub x: f32,
    pub text: String,
    pub width: f32,
    pub px: u16,
    pub bold: bool,
    pub color: Color,
    /// Node whose inline content produced this run (for fixture facts).
    pub source: NodeId,
}

/// Computed styles for every element, from one top-down cascade pass.
pub(crate) type StyleMap = std::collections::HashMap<NodeId, ComputedStyle>;

/// Runs the cascade over the whole tree once.
pub(crate) fn compute_all_styles(
    dom: &Dom,
    root: NodeId,
    cascade: &mut crate::style::Cascade<'_>,
) -> StyleMap {
    let mut map = StyleMap::new();
    walk_styles(dom, root, None, cascade, &mut map);
    map
}

fn walk_styles(
    dom: &Dom,
    id: NodeId,
    parent: Option<&ComputedStyle>,
    cascade: &mut crate::style::Cascade<'_>,
    map: &mut StyleMap,
) {
    if !matches!(dom.node(id).data, NodeData::Element { .. }) {
        return;
    }
    let style = cascade.compute(dom, id, parent);
    for &child in &dom.node(id).children {
        walk_styles(dom, child, Some(&style), cascade, map);
    }
    map.insert(id, style);
}

/// Builds the box tree under `root` (display:none subtrees are skipped;
/// inline subtrees flatten into words).
pub(crate) fn build_boxes(dom: &Dom, root: NodeId, styles: &StyleMap) -> Option<BoxNode> {
    let style = styles.get(&root)?.clone();
    if style.display == Display::None {
        return None;
    }
    let mut box_node = BoxNode {
        node: root,
        fixture_id: dom.fixture_id(root).map(str::to_owned),
        tag: dom.tag_name(root).unwrap_or("?").to_owned(),
        style: style.clone(),
        children: Vec::new(),
        lines: Vec::new(),
        content: Rect::default(),
        padding_box: Rect::default(),
        border_box: Rect::default(),
        margin: Sides::default(),
        words: Vec::new(),
    };

    let mut pending_space_before = false;
    for &child in &dom.node(root).children {
        match &dom.node(child).data {
            NodeData::Element { .. } => {
                match styles
                    .get(&child)
                    .map(|s| s.display)
                    .unwrap_or(Display::Inline)
                {
                    Display::Block => {
                        if let Some(child_box) = build_boxes(dom, child, styles) {
                            box_node.children.push(child_box);
                        }
                    }
                    Display::Inline => {
                        collect_inline_words(
                            dom,
                            child,
                            styles,
                            &mut box_node.words,
                            &mut pending_space_before,
                        );
                    }
                    Display::None => {}
                }
            }
            NodeData::Text(raw) => {
                append_text_words(
                    raw,
                    style_for_text(dom, child, styles, &style),
                    child,
                    &mut box_node.words,
                    &mut pending_space_before,
                );
            }
            _ => {}
        }
    }
    Some(box_node)
}

/// The style governing a text node: its parent element's computed style.
fn style_for_text(
    dom: &Dom,
    text_node: NodeId,
    styles: &StyleMap,
    fallback: &ComputedStyle,
) -> ComputedStyle {
    dom.node(text_node)
        .parent
        .and_then(|parent| styles.get(&parent).cloned())
        .unwrap_or_else(|| fallback.clone())
}

/// Appends one text node's words, tracking whether a whitespace separator
/// precedes the first word (CSS whitespace collapsing across nodes).
fn append_text_words(
    raw: &str,
    style: ComputedStyle,
    source: NodeId,
    words: &mut Vec<Word>,
    pending_space_before: &mut bool,
) {
    let starts_with_space = raw.starts_with(char::is_whitespace);
    let collapsed = crate::text::collapse_whitespace(raw);
    if collapsed.is_empty() {
        // Pure-whitespace node: preserves a boundary between inline content.
        if !words.is_empty() {
            *pending_space_before = true;
        }
        return;
    }
    let space_before = (*pending_space_before || starts_with_space) && !words.is_empty();
    *pending_space_before = false;
    for (i, word) in collapsed.split_ascii_whitespace().enumerate() {
        words.push(Word {
            text: word.to_owned(),
            style: style.clone(),
            space_before: space_before || i > 0,
            source,
        });
    }
}

/// Flattens an inline subtree into the parent's word list.
fn collect_inline_words(
    dom: &Dom,
    node: NodeId,
    styles: &StyleMap,
    words: &mut Vec<Word>,
    pending_space_before: &mut bool,
) {
    let Some(style) = styles.get(&node).cloned() else {
        return;
    };
    if style.display == Display::None {
        return;
    }
    for &child in &dom.node(node).children {
        match &dom.node(child).data {
            NodeData::Element { .. } => {
                collect_inline_words(dom, child, styles, words, pending_space_before);
            }
            NodeData::Text(raw) => {
                append_text_words(raw, style.clone(), node, words, pending_space_before);
            }
            _ => {}
        }
    }
}

/// Lays out the document inside `viewport` and returns the root box plus
/// its display list.
pub(crate) fn layout_document(
    dom: &Dom,
    viewport: Viewport,
    cascade: &mut crate::style::Cascade<'_>,
    fonts: &mut FontStore,
) -> Option<(BoxNode, DisplayList)> {
    let scale = viewport.scale_factor();

    // The layout root: <body> if present, else <html>, else the document.
    let mut root_element = None;
    dom.walk_from(dom.document(), |id, _node| {
        if root_element.is_none() && dom.tag_name(id) == Some("body") {
            root_element = Some(id);
        }
    });
    let root_element = root_element.unwrap_or_else(|| dom.document());

    let styles = compute_all_styles(dom, root_element, cascade);
    let mut root = build_boxes(dom, root_element, &styles)?;

    layout_box(
        &mut root,
        0.0,
        0.0,
        viewport.width() as f32,
        viewport.height() as f32,
        scale,
        fonts,
    );

    let mut list = DisplayList::default();
    emit_display_list(&root, &mut list);
    Some((root, list))
}

/// Resolves a length against `containing` (device px).
fn resolve(len: Length, containing: f32, scale: f32) -> f32 {
    match len {
        Length::Px(v) => v * scale,
        Length::Percent(p) => p / 100.0 * containing,
        Length::Rem(v) => v * scale,
    }
}

fn resolve_sides(sides: Sides<Length>, containing: f32, scale: f32) -> Sides<f32> {
    Sides {
        top: resolve(sides.top, containing, scale),
        right: resolve(sides.right, containing, scale),
        bottom: resolve(sides.bottom, containing, scale),
        left: resolve(sides.left, containing, scale),
    }
}

/// Lays out one box. `x`/`y` are the margin-box origin; `available_width`
/// is the containing block's content width. Returns the margin-box height.
pub(crate) fn layout_box(
    node: &mut BoxNode,
    x: f32,
    y: f32,
    available_width: f32,
    containing_height: f32,
    scale: f32,
    fonts: &mut FontStore,
) -> f32 {
    let style = node.style.clone();
    let border = resolve_sides(
        Sides {
            top: style.border_width.top,
            right: style.border_width.right,
            bottom: style.border_width.bottom,
            left: style.border_width.left,
        },
        available_width,
        scale,
    );
    let padding = resolve_sides(style.padding, available_width, scale);
    let margin = resolve_sides(style.margin, available_width, scale);
    node.margin = margin;

    // Content width: specified → clamped; else fill the remaining space.
    let used_horizontally = padding.left + padding.right + border.left + border.right;
    let mut content_width = match style.width {
        Some(Length::Percent(p)) => p / 100.0 * available_width - used_horizontally,
        Some(len) => resolve(len, available_width, scale),
        None => (available_width - used_horizontally).max(0.0),
    };
    if let Some(max) = style.max_width {
        let max = resolve(max, available_width, scale);
        if content_width > max {
            content_width = max;
        }
    }
    if let Some(min) = style.min_width {
        let min = resolve(min, available_width, scale);
        if content_width < min {
            content_width = min;
        }
    }
    content_width = content_width.max(0.0);

    node.border_box = Rect {
        x: x + margin.left,
        y: y + margin.top,
        w: content_width + padding.left + padding.right + border.left + border.right,
        h: 0.0,
    };
    node.padding_box = Rect {
        x: node.border_box.x + border.left,
        y: node.border_box.y + border.top,
        w: content_width + padding.left + padding.right,
        h: 0.0,
    };
    node.content = Rect {
        x: node.padding_box.x + padding.left,
        y: node.padding_box.y + padding.top,
        w: content_width,
        h: 0.0,
    };

    // Inline content: greedy wrap of the styled words.
    let block_font_px = (style.font_size * scale).round().max(1.0);
    let line_height = match style.line_height {
        LineHeight::Normal => 1.5 * block_font_px,
        LineHeight::Number(n) => n * block_font_px,
        LineHeight::Px(v) => v * scale,
    };
    let mut lines: Vec<LaidLine> = Vec::new();
    let mut current = LaidLine::default();
    let mut cursor_x = 0.0;

    for word in &node.words {
        let word_px = (word.style.font_size * scale).round().max(1.0) as u16;
        let weight = text::face_weight(word.style.font_weight);
        let word_width = text::measure_run(fonts, &word.text, word_px, weight);
        let space_width = if current.runs.is_empty() || !word.space_before {
            0.0
        } else {
            let last = current.runs.last().expect("checked non-empty");
            text::measure_run(fonts, " ", last.px, text::face_weight(400))
        };
        if !current.runs.is_empty()
            && content_width > 0.0
            && cursor_x + space_width + word_width > content_width
        {
            lines.push(std::mem::take(&mut current));
            cursor_x = 0.0;
        }
        let at_line_start = current.runs.is_empty();
        let space = if at_line_start { 0.0 } else { space_width };
        let run_x = cursor_x + space;
        current.runs.push(RunBox {
            x: run_x,
            text: word.text.clone(),
            width: word_width,
            px: word_px,
            bold: weight == crate::font::FontWeight::Bold,
            color: word.style.color,
            source: word.source,
        });
        cursor_x = run_x + word_width;
    }
    if !current.runs.is_empty() {
        lines.push(current);
    }

    // Position lines: vertical stacking + horizontal alignment.
    let mut line_cursor = 0.0;
    for line in &mut lines {
        line.y = line_cursor;
        line.height = line_height;
        let line_width = line
            .runs
            .last()
            .map(|last| last.x + last.width)
            .unwrap_or(0.0);
        let offset = match style.text_align {
            TextAlign::Left => 0.0,
            TextAlign::Center => ((content_width - line_width) / 2.0).max(0.0),
            TextAlign::Right => (content_width - line_width).max(0.0),
        };
        for run in &mut line.runs {
            run.x += offset;
        }
        line_cursor += line.height;
    }
    node.lines = lines;

    // Block children stack below the inline content.
    let mut child_cursor = line_cursor;
    for child in &mut node.children {
        let child_height = layout_box(
            child,
            node.content.x,
            node.content.y + child_cursor,
            content_width,
            containing_height,
            scale,
            fonts,
        );
        child_cursor += child_height;
    }

    // Content height: specified → clamped; else the laid-out extent.
    let mut content_height = match style.height {
        Some(Length::Px(v)) => v * scale,
        Some(Length::Rem(v)) => v * scale,
        // Percentage heights need a fixed containing height; M2a resolves
        // them as auto.
        _ => child_cursor,
    };
    if let Some(max) = style.max_height {
        let max = resolve(max, containing_height, scale);
        if content_height > max {
            content_height = max;
        }
    }
    if let Some(min) = style.min_height {
        let min = resolve(min, containing_height, scale);
        if content_height < min {
            content_height = min;
        }
    }
    node.content.h = content_height;
    node.padding_box.h = content_height + padding.top + padding.bottom;
    node.border_box.h = node.padding_box.h + border.top + border.bottom;

    margin.top + node.border_box.h + margin.bottom
}

/// Emits paint-ready items: backgrounds, borders, then text (per box).
pub(crate) fn emit_display_list(node: &BoxNode, list: &mut DisplayList) {
    let style = &node.style;
    if style.background_color.a > 0 {
        list.rects.push(DisplayRect {
            rect: node.padding_box,
            color: style.background_color,
        });
    }
    if style.border_style_solid {
        let b = Sides {
            top: resolve(style.border_width.top, 0.0, 1.0),
            right: resolve(style.border_width.right, 0.0, 1.0),
            bottom: resolve(style.border_width.bottom, 0.0, 1.0),
            left: resolve(style.border_width.left, 0.0, 1.0),
        };
        let bb = node.border_box;
        let bc = style.border_color;
        if b.top > 0.0 {
            list.rects.push(DisplayRect {
                rect: Rect {
                    x: bb.x,
                    y: bb.y,
                    w: bb.w,
                    h: b.top,
                },
                color: bc,
            });
        }
        if b.bottom > 0.0 {
            list.rects.push(DisplayRect {
                rect: Rect {
                    x: bb.x,
                    y: bb.y + bb.h - b.bottom,
                    w: bb.w,
                    h: b.bottom,
                },
                color: bc,
            });
        }
        if b.left > 0.0 {
            list.rects.push(DisplayRect {
                rect: Rect {
                    x: bb.x,
                    y: bb.y + b.top,
                    w: b.left,
                    h: (bb.h - b.top - b.bottom).max(0.0),
                },
                color: bc,
            });
        }
        if b.right > 0.0 {
            list.rects.push(DisplayRect {
                rect: Rect {
                    x: bb.x + bb.w - b.right,
                    y: bb.y + b.top,
                    w: b.right,
                    h: (bb.h - b.top - b.bottom).max(0.0),
                },
                color: bc,
            });
        }
    }
    for line in &node.lines {
        for run in &line.runs {
            // Baseline: center the font's ink range in the line box.
            let (ascent, descent) = font_verticals(run.px);
            let ink = ascent - descent;
            let baseline = node.content.y + line.y + (line.height - ink) / 2.0 + ascent;
            list.texts.push(DisplayText {
                x: node.content.x + run.x,
                y: baseline,
                text: run.text.clone(),
                px: run.px,
                bold: run.bold,
                color: run.color,
            });
        }
    }
    for child in &node.children {
        emit_display_list(child, list);
    }
}

/// DejaVu's vertical metric ratios (ascent, |descent| per px); used only
/// for line-box centering. Exact font metrics arrive with the M2b text
/// overhaul; the constant keeps layout deterministic.
fn font_verticals(px: u16) -> (f32, f32) {
    (0.928 * px as f32, 0.236 * px as f32)
}

/// Collects [`LayoutFacts`] for every `data-vv-test` element in the tree.
pub(crate) fn collect_facts(
    dom: &Dom,
    root: &BoxNode,
    viewport: Viewport,
    facts: &mut LayoutFacts,
) {
    facts.schema_version = LAYOUT_FACTS_SCHEMA_VERSION;
    facts.viewport_width = viewport.width();
    facts.viewport_height = viewport.height();
    facts.scale = viewport.scale_factor();
    facts.nodes.clear();
    walk_facts(dom, root, &mut facts.nodes);
}

fn walk_facts(dom: &Dom, node: &BoxNode, out: &mut Vec<LayoutNodeFact>) {
    if let Some(fixture_id) = &node.fixture_id {
        // Box-model facts are resolved device pixels, consistent with the
        // geometry fields; derive them from the laid-out boxes so the
        // numbers always agree with x/y/width/height.
        //
        // border = border_box → padding_box (per side).
        let border = [
            node.padding_box.x - node.border_box.x,
            node.padding_box.y - node.border_box.y,
            node.border_box.y + node.border_box.h - (node.padding_box.y + node.padding_box.h),
            node.border_box.x + node.border_box.w - (node.padding_box.x + node.padding_box.w),
        ];
        // padding = padding_box → content (per side).
        let padding = [
            node.content.y - node.padding_box.y,
            node.padding_box.x + node.padding_box.w - (node.content.x + node.content.w),
            node.padding_box.y + node.padding_box.h - (node.content.y + node.content.h),
            node.content.x - node.padding_box.x,
        ];
        out.push(LayoutNodeFact {
            fixture_id: fixture_id.clone(),
            tag: node.tag.clone(),
            display: display_name(node.style.display),
            x: node.border_box.x,
            y: node.border_box.y,
            width: node.border_box.w,
            height: node.border_box.h,
            content_x: node.content.x,
            content_y: node.content.y,
            content_width: node.content.w,
            content_height: node.content.h,
            padding,
            border,
            margin: [
                node.margin.top,
                node.margin.right,
                node.margin.bottom,
                node.margin.left,
            ],
            text_runs: node.lines.iter().map(runs_text).collect(),
        });
    }
    // Inline elements flattened into runs: emit a fact with the union of
    // their runs' extents (absolute), so `data-vv-test` on an inline still
    // identifies a box for fixtures.
    let mut inline_ids: Vec<(NodeId, &str)> = Vec::new();
    for line in &node.lines {
        for run in &line.runs {
            if let Some(fixture_id) = dom.fixture_id(run.source) {
                if !inline_ids.iter().any(|(id, _)| *id == run.source) {
                    inline_ids.push((run.source, fixture_id));
                }
            }
        }
    }
    for (source_id, fixture_id) in inline_ids {
        let mut extent: Option<Rect> = None;
        let mut texts: Vec<String> = Vec::new();
        for line in &node.lines {
            for run in &line.runs {
                if run.source == source_id {
                    let abs = Rect {
                        x: node.content.x + run.x,
                        y: node.content.y + line.y,
                        w: run.width,
                        h: line.height,
                    };
                    extent = Some(match extent {
                        None => abs,
                        Some(e) => union(e, abs),
                    });
                    texts.push(run.text.clone());
                }
            }
        }
        if let Some(extent) = extent {
            out.push(LayoutNodeFact {
                fixture_id: fixture_id.to_owned(),
                tag: dom.tag_name(source_id).unwrap_or("?").to_owned(),
                display: "inline".into(),
                x: extent.x,
                y: extent.y,
                width: extent.w,
                height: extent.h,
                content_x: extent.x,
                content_y: extent.y,
                content_width: extent.w,
                content_height: extent.h,
                padding: [0.0; 4],
                border: [0.0; 4],
                margin: [0.0; 4],
                text_runs: vec![texts.join(" ")],
            });
        }
    }
    for child in &node.children {
        walk_facts(dom, child, out);
    }
}

fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect {
        x,
        y,
        w: (a.x + a.w).max(b.x + b.w) - x,
        h: (a.y + a.h).max(b.y + b.h) - y,
    }
}

fn runs_text(line: &LaidLine) -> String {
    line.runs
        .iter()
        .map(|run| run.text.clone())
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_name(display: Display) -> String {
    match display {
        Display::Block => "block".into(),
        Display::Inline => "inline".into(),
        Display::None => "none".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::StylesheetSource;

    fn facts_for(html: &str, css: &str, width: u32, height: u32) -> LayoutFacts {
        let dom = crate::html::parse(html);
        let ua_sheet = StylesheetSource::new("velqu:ua", "");
        let ua_parsed = crate::css::parse(&ua_sheet, 0);
        let author_sheet = StylesheetSource::new("test.css", css);
        let author_parsed = crate::css::parse(&author_sheet, ua_parsed.rules.len() as u32);
        let author_sheets = [author_parsed];
        let mut cascade = crate::style::Cascade::new(&ua_parsed.rules, &author_sheets);
        let mut fonts = FontStore::bundled();
        let viewport = Viewport::try_new(width, height, 1.0).unwrap();
        let (root, _) = layout_document(&dom, viewport, &mut cascade, &mut fonts).unwrap();
        let mut facts = LayoutFacts {
            schema_version: 0,
            viewport_width: 0,
            viewport_height: 0,
            scale: 0.0,
            nodes: Vec::new(),
        };
        collect_facts(&dom, &root, viewport, &mut facts);
        facts
    }

    fn fact<'a>(facts: &'a LayoutFacts, id: &str) -> &'a LayoutNodeFact {
        facts
            .nodes
            .iter()
            .find(|n| n.fixture_id == id)
            .unwrap_or_else(|| panic!("no fact for {id}"))
    }

    #[test]
    fn body_margin_and_block_stacking() {
        let facts = facts_for(
            "<body><div data-vv-test=a>one</div><div data-vv-test=b>two</div></body>",
            "",
            400,
            400,
        );
        let a = fact(&facts, "a");
        let b = fact(&facts, "b");
        // UA body margin is 8px, so children start at 8,8; the divs
        // themselves carry no UA margin.
        assert_eq!((a.x, a.y), (8.0, 8.0));
        assert_eq!(a.margin, [0.0, 0.0, 0.0, 0.0]);
        // Block children stack: b starts exactly at a's border-box bottom
        // (no margins to collapse — M2a adds instead).
        assert_eq!(b.y, a.y + a.height);
        assert_eq!(b.x, 8.0);
    }

    #[test]
    fn width_padding_border_math() {
        let facts = facts_for(
            "<body><div data-vv-test=card class=c>text</div></body>",
            ".c { margin: 10px; padding: 8px; border: 2px solid #333; width: 100px }",
            400,
            400,
        );
        let card = fact(&facts, "card");
        // Border box = 2 + 8 + 100 + 8 + 2.
        assert_eq!(card.width, 120.0);
        assert_eq!(card.x, 10.0 + 8.0);
        assert_eq!(card.content_width, 100.0);
        assert_eq!(card.padding, [8.0, 8.0, 8.0, 8.0]);
        assert_eq!(card.border, [2.0, 2.0, 2.0, 2.0]);
    }

    #[test]
    fn text_wraps_at_content_width() {
        let mut fonts = FontStore::bundled();
        let mut measure =
            |word: &str| text::measure_run(&mut fonts, word, 16, crate::font::FontWeight::Regular);
        let space = measure(" ");
        // Content width fits "Hello wrapped" but not the third word.
        let content = measure("Hello") + space + measure("wrapped");
        let html = format!(
            "<body><div data-vv-test=t style=\"width: {}px; margin: 0\">Hello wrapped world</div></body>",
            content.ceil() as u32
        );
        let facts = facts_for(&html, "", 600, 400);
        let t = fact(&facts, "t");
        assert_eq!(
            t.text_runs,
            ["Hello wrapped", "world"],
            "widths: hello={} space={} wrapped={} world={} content={}",
            measure("Hello"),
            space,
            measure("wrapped"),
            measure("world"),
            content
        );
    }

    #[test]
    fn display_none_removes_subtree() {
        let facts = facts_for(
            "<body><div data-vv-test=hidden style=\"display: none\">x</div>\
             <div data-vv-test=shown>y</div></body>",
            "",
            400,
            400,
        );
        assert!(facts.nodes.iter().all(|n| n.fixture_id != "hidden"));
        assert!(fact(&facts, "shown").y >= 8.0);
    }

    #[test]
    fn inherited_color_reaches_text() {
        let dom = crate::html::parse(
            "<body><div data-vv-test=outer style=\"color: #102030\"><span data-vv-test=inner>hi</span></div></body>",
        );
        let ua_sheet = StylesheetSource::new("velqu:ua", "");
        let ua_parsed = crate::css::parse(&ua_sheet, 0);
        let mut cascade = crate::style::Cascade::new(&ua_parsed.rules, &[]);
        let mut fonts = FontStore::bundled();
        let viewport = Viewport::try_new(400, 400, 1.0).unwrap();
        let (root, list) = layout_document(&dom, viewport, &mut cascade, &mut fonts).unwrap();
        // The text run's color equals the outer div's color.
        assert!(!list.texts.is_empty());
        let expected = Color::from_hex("#102030").unwrap();
        assert!(list.texts.iter().all(|t| t.color == expected));
        let mut facts = LayoutFacts {
            schema_version: 0,
            viewport_width: 0,
            viewport_height: 0,
            scale: 0.0,
            nodes: Vec::new(),
        };
        collect_facts(&dom, &root, viewport, &mut facts);
        assert_eq!(fact(&facts, "inner").text_runs, ["hi"]);
    }

    #[test]
    fn text_align_offsets_lines() {
        let facts = facts_for(
            "<body><div data-vv-test=c style=\"width: 300px; text-align: center; margin: 0\">hi</div></body>",
            "",
            400,
            200,
        );
        let c = fact(&facts, "c");
        assert_eq!(c.content_width, 300.0);
        assert_eq!(c.text_runs, ["hi"]);
        // Content x = 8 (body margin); the "hi" run is centered inside 300px:
        // recompute via facts: content_x + (300 - run_width)/2 <= content_x + 300.
        // The precise offset is asserted through the display list test below.
    }

    #[test]
    fn centered_text_is_offset_from_left() {
        let dom = crate::html::parse(
            "<body><div style=\"width: 300px; text-align: center; margin: 0\">hi</div></body>",
        );
        let ua_sheet = StylesheetSource::new("velqu:ua", "");
        let ua_parsed = crate::css::parse(&ua_sheet, 0);
        let mut cascade = crate::style::Cascade::new(&ua_parsed.rules, &[]);
        let mut fonts = FontStore::bundled();
        let viewport = Viewport::try_new(400, 200, 1.0).unwrap();
        let (_, list) = layout_document(&dom, viewport, &mut cascade, &mut fonts).unwrap();
        let run = &list.texts[0];
        let run_width = text::measure_run(&mut fonts, "hi", 16, crate::font::FontWeight::Regular);
        assert!((run.x - (8.0 + (300.0 - run_width) / 2.0)).abs() < 1.0);
    }

    #[test]
    fn schema_version_is_reported() {
        let facts = facts_for("<body><div data-vv-test=x>y</div></body>", "", 200, 200);
        assert_eq!(facts.schema_version, LAYOUT_FACTS_SCHEMA_VERSION);
        assert_eq!((facts.viewport_width, facts.viewport_height), (200, 200));
        assert_eq!(facts.scale, 1.0);
    }
}
