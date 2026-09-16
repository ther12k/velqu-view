//! Box tree and layout orchestration (M2a block semantics; M2b adds flex).
//!
//! Layout identity is distinct from DOM identity (ADR 0005): boxes are
//! rebuilt from scratch on every layout pass. The pipeline is
//! DOM + ComputedStyle → box tree → **Taffy** (private backend,
//! `taffy_backend.rs`, ADR 0007) → laid-out boxes → [`LayoutFacts`] and a
//! [`DisplayList`] for painting.
//!
//! This module owns the box tree, projection input ordering, inline content
//! positioning, facts collection, and display-list emission. The layout
//! algorithms themselves (block, flex) run in the private Taffy backend;
//! rounding happens only at raster time (ADR 0007).
//!
//! All layout geometry is in **device pixels** — logical CSS px values are
//! scaled during projection so text measurement and painting share one
//! metric space.

use crate::color::Color;
use crate::display_list::{DisplayItem, DisplayList, Rect};
use crate::dom::{Dom, NodeData, NodeId};
use crate::font::FontStore;
use crate::style::{ComputedStyle, Display};
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
pub(crate) struct Word {
    pub text: String,
    pub style: ComputedStyle,
    /// A whitespace separator preceded this word in the source.
    pub space_before: bool,
    /// Element whose content produced this word (fixture facts).
    pub source: NodeId,
}

/// Where a box's inline content (lines) is anchored. Differs from the
/// content origin when the words were wrapped into an anonymous child by
/// the Taffy projection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextOrigin {
    pub x: f32,
    pub y: f32,
    /// Wrap width for line breaking and alignment.
    pub width: f32,
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
    pub margin: crate::style::Sides<f32>,
    /// Inline content words, in source order (wrapped during layout).
    pub words: Vec<Word>,
    /// Set by the backend when words live in an anonymous child.
    pub text_origin: Option<TextOrigin>,
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
        margin: crate::style::Sides::default(),
        words: Vec::new(),
        text_origin: None,
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
                    Display::Block | Display::Flex => {
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
///
/// The box tree is built here (Velqu-canonical), then laid out by the
/// private Taffy backend (ADR 0007), then lowered to a display list.
pub(crate) fn layout_document(
    dom: &Dom,
    viewport: Viewport,
    cascade: &mut crate::style::Cascade<'_>,
    fonts: &mut FontStore,
) -> Option<(BoxNode, DisplayList)> {
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

    crate::taffy_backend::layout_box_tree(&mut root, viewport, fonts);

    let mut list = DisplayList::default();
    emit_display_list(&root, &mut list);
    Some((root, list))
}

/// Emits paint-ready items: backgrounds, borders, text, with
/// `overflow: hidden`/`clip` boxes scoping their descendants via clip
/// items. Layout owns geometry; the display list owns paint ordering and
/// clipping; the painter executes.
pub(crate) fn emit_display_list(node: &BoxNode, list: &mut DisplayList) {
    emit_box(node, list, None);
}

fn emit_box(node: &BoxNode, list: &mut DisplayList, parent_clip: Option<Rect>) {
    let style = &node.style;
    let _ = parent_clip;

    // Own chrome (background, border): the painter applies the current
    // clip stack, which at this point is the ancestors' clips.
    if style.background_color.a > 0 {
        list.items.push(DisplayItem::FillRect {
            rect: node.padding_box,
            color: style.background_color,
        });
    }
    if style.border_style_solid {
        // Border widths derive from the laid-out boxes so they always
        // agree with the geometry facts.
        let b = crate::style::Sides {
            top: node.padding_box.y - node.border_box.y,
            left: node.padding_box.x - node.border_box.x,
            right: node.border_box.x + node.border_box.w
                - (node.padding_box.x + node.padding_box.w),
            bottom: node.border_box.y + node.border_box.h
                - (node.padding_box.y + node.padding_box.h),
        };
        let bb = node.border_box;
        let bc = style.border_color;
        for (rect, width) in [
            (
                Rect {
                    x: bb.x,
                    y: bb.y,
                    w: bb.w,
                    h: b.top,
                },
                b.top,
            ),
            (
                Rect {
                    x: bb.x,
                    y: bb.y + bb.h - b.bottom,
                    w: bb.w,
                    h: b.bottom,
                },
                b.bottom,
            ),
            (
                Rect {
                    x: bb.x,
                    y: bb.y + b.top,
                    w: b.left,
                    h: (bb.h - b.top - b.bottom).max(0.0),
                },
                b.left,
            ),
            (
                Rect {
                    x: bb.x + bb.w - b.right,
                    y: bb.y + b.top,
                    w: b.right,
                    h: (bb.h - b.top - b.bottom).max(0.0),
                },
                b.right,
            ),
        ] {
            if width > 0.0 {
                list.items.push(DisplayItem::FillRect { rect, color: bc });
            }
        }
    }

    // Inline content: anchored at the box's text origin (content origin,
    // or the anonymous words node when the backend created one).
    let (origin_x, origin_y) = match node.text_origin {
        Some(origin) => (origin.x, origin.y),
        None => (node.content.x, node.content.y),
    };
    for line in &node.lines {
        for run in &line.runs {
            // Baseline: center the font's ink range in the line box.
            let (ascent, descent) = font_verticals(run.px);
            let ink = ascent - descent;
            let baseline = origin_y + line.y + (line.height - ink) / 2.0 + ascent;
            list.items.push(DisplayItem::TextRun {
                x: origin_x + run.x,
                y: baseline,
                text: run.text.clone(),
                px: run.px,
                bold: run.bold,
                color: run.color,
            });
        }
    }

    // Children, scoped by this box's clip when it clips overflow.
    match style.overflow_y {
        crate::style::Overflow::Visible => {
            for child in &node.children {
                emit_box(child, list, parent_clip);
            }
        }
        crate::style::Overflow::Hidden | crate::style::Overflow::Clip => {
            list.items.push(DisplayItem::PushClip(node.padding_box));
            for child in &node.children {
                emit_box(child, list, Some(node.padding_box));
            }
            list.items.push(DisplayItem::PopClip);
        }
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
        Display::Flex => "flex".into(),
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
        let mut measure = |word: &str| {
            crate::text::measure_run(&mut fonts, word, 16, crate::font::FontWeight::Regular)
        };
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
        let runs = list.text_runs();
        assert!(!runs.is_empty());
        let expected = Color::from_hex("#102030").unwrap();
        assert!(runs.iter().all(|t| t.color == expected));
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
        let run = &list.text_runs()[0];
        let run_width =
            crate::text::measure_run(&mut fonts, "hi", 16, crate::font::FontWeight::Regular);
        assert!((run.x - (8.0 + (300.0 - run_width) / 2.0)).abs() < 1.0);
    }

    #[test]
    fn schema_version_is_reported() {
        let facts = facts_for("<body><div data-vv-test=x>y</div></body>", "", 200, 200);
        assert_eq!(facts.schema_version, LAYOUT_FACTS_SCHEMA_VERSION);
        assert_eq!((facts.viewport_width, facts.viewport_height), (200, 200));
        assert_eq!(facts.scale, 1.0);
    }

    #[test]
    fn flex_grow_distributes_fractional_widths() {
        // Three grow:1 items in 101px: widths cannot all be integers; the
        // contract is the deterministic 101/3 split and its exact sum.
        let facts = facts_for(
            "<body><div data-vv-test=row class=row>\
             <div data-vv-test=a class=ga></div>\
             <div data-vv-test=b class=gb></div>\
             <div data-vv-test=c class=gc></div></div></body>",
            "body { margin: 0 } .row { display: flex; width: 101px; height: 30px } \
             .ga, .gb, .gc { flex-grow: 1 }",
            400,
            300,
        );
        let a = fact(&facts, "a");
        let b = fact(&facts, "b");
        let c = fact(&facts, "c");
        assert_eq!(a.width, 101.0 / 3.0);
        assert_eq!(b.width, 101.0 / 3.0);
        assert_eq!(c.width, 101.0 / 3.0);
        // Deterministic containment: the items tile the container exactly.
        assert_eq!(a.x + a.width, b.x);
        assert_eq!(b.x + b.width, c.x);
        assert!((c.x + c.width - 101.0).abs() < 1e-3);
        // Cross-axis stretch by default align-items.
        assert_eq!(a.height, 30.0);
    }

    #[test]
    fn flex_row_nests_block_and_column() {
        // block → flex → (text | block | flex column) — the mixed-nesting
        // shape that a flex-island architecture cannot handle.
        let facts = facts_for(
            "<body><div data-vv-test=mix class=mix>\
             <span data-vv-test=t1>hi</span>\
             <div data-vv-test=blk class=blk></div>\
             <div data-vv-test=col class=col>\
             <div data-vv-test=c1>x</div>\
             <div data-vv-test=c2>y</div>\
             </div></div></body>",
            "body { margin: 0 } .mix { display: flex } .blk { width: 50px; height: 30px } \
             .col { display: flex; flex-direction: column }",
            400,
            300,
        );
        let mix = fact(&facts, "mix");
        let t1 = fact(&facts, "t1");
        let blk = fact(&facts, "blk");
        let col = fact(&facts, "col");
        let c1 = fact(&facts, "c1");
        let c2 = fact(&facts, "c2");
        // Row order: text, block, column — strictly increasing x.
        assert!(
            t1.x < blk.x && blk.x < col.x,
            "{} {} {}",
            t1.x,
            blk.x,
            col.x
        );
        assert_eq!(blk.y, mix.y);
        assert_eq!(blk.width, 50.0);
        // Column children stack vertically.
        assert_eq!(c1.x, col.x);
        assert!(c2.y > c1.y, "column stacks: {} {}", c1.y, c2.y);
        assert_eq!(t1.text_runs, ["hi"]);
    }

    #[test]
    fn overflow_hidden_clips_paint_not_layout() {
        // The child keeps its full laid-out size; clipping is a paint-side
        // concern (PushClip in the display list).
        let dom = crate::html::parse(
            "<body><div data-vv-test=clip class=clip>\
             <div data-vv-test=big class=big>wide child text</div>\
             </div></body>",
        );
        let ua_sheet = StylesheetSource::new("velqu:ua", "");
        let ua_parsed = crate::css::parse(&ua_sheet, 0);
        let author = StylesheetSource::new(
            "t.css",
            "body { margin: 0 } .clip { overflow: hidden; width: 60px; height: 20px } \
             .big { width: 200px; height: 40px }",
        );
        let author_parsed = crate::css::parse(&author, 0);
        let sheets = [author_parsed];
        let mut cascade = crate::style::Cascade::new(&ua_parsed.rules, &sheets);
        let mut fonts = FontStore::bundled();
        let viewport = Viewport::try_new(400, 300, 1.0).unwrap();
        let (root, list) = layout_document(&dom, viewport, &mut cascade, &mut fonts).unwrap();
        let mut facts = LayoutFacts {
            schema_version: 0,
            viewport_width: 0,
            viewport_height: 0,
            scale: 0.0,
            nodes: Vec::new(),
        };
        collect_facts(&dom, &root, viewport, &mut facts);
        // Layout truth: the child is fully sized and positioned.
        let big = fact(&facts, "big");
        assert_eq!(big.width, 200.0);
        assert_eq!(big.x, 0.0);
        // Paint truth: a clip scope wraps the container's children.
        let has_clip = list
            .items
            .iter()
            .any(|item| matches!(item, crate::display_list::DisplayItem::PushClip(_)));
        assert!(has_clip, "overflow:hidden emits PushClip");
    }
}
