//! The private Taffy layout backend (M2b, ADR 0007).
//!
//! Strategy: the Velqu [`BoxNode`] tree stays canonical and is **projected
//! in full** onto a private `TaffyTree` for every layout pass — the same
//! lifetime as the box tree itself, which is rebuilt per pass (ADR 0005).
//! Taffy runs its block/flex algorithms; results are written back into the
//! box tree. No Taffy type escapes this module (ADR 0003).
//!
//! Why full projection over per-flex-island projection: intrinsic
//! measurement, available-space propagation, and percentage resolution cross
//! engine boundaries whenever nesting mixes display types; a single backend
//! tree keeps those semantics in one place. The Taffy low-level
//! `LayoutPartialTree` adapter (zero-copy) remains a contained future
//! optimization behind this module's seam.
//!
//! Rounding policy (ADR 0007): Taffy rounding is disabled. Layout computes
//! in device pixels (logical CSS px are scaled at projection) and stays
//! unrounded f32 through [`LayoutFacts`]; the painter snaps rect edges to
//! whole pixels at raster time. One rounding owner: Velqu.

use std::collections::HashMap;

use taffy::geometry::{Point as TaffyPoint, Rect as TaffyRect};
use taffy::style::{
    BoxSizing, Dimension as TaffyDimension, LengthPercentage, LengthPercentageAuto,
};
use taffy::tree::NodeId as TaffyId;
use taffy::tree::{LayoutInput, LayoutOutput, RunMode};
use taffy::{AvailableSpace, Size as TaffySize, Style as TaffyStyle, TaffyTree};

use crate::display_list::Rect;
use crate::font::FontStore;
use crate::layout::{BoxNode, LaidLine, RunBox, TextOrigin};
use crate::style::{
    AlignItems, AlignSelf, ComputedStyle, Display, JustifyContent, Length, LineHeight, Overflow,
    Sides, TextAlign,
};
use crate::viewport::Viewport;

/// The page roots establish a block formatting context (Taffy `FlowRoot`)
/// so child margins do not collapse through them — matching the M2a page
/// model (documented intentional migration for other boxes, where CSS
/// sibling margin collapsing now applies via Taffy's block algorithm).
const PAGE_ROOT_TAGS: [&str; 2] = ["html", "body"];

/// Lays out the box tree with Taffy, then positions inline content.
pub(crate) fn layout_box_tree(root: &mut BoxNode, viewport: Viewport, fonts: &mut FontStore) {
    let scale = viewport.scale_factor();
    let mut taffy = TaffyTree::new();
    // One rounding owner: Velqu rounds at raster; Taffy stays unrounded.
    taffy.disable_rounding();

    // Projection: box tree → Taffy tree. Leaf measurement maps to the box's
    // inline words via a side table (id → words source).
    let mut leaf_sources: HashMap<TaffyId, &BoxNode> = HashMap::new();
    let mirror = project(root, &mut taffy, &mut leaf_sources, scale);

    // A viewport root wraps the page so the page root's own margins behave
    // like M2a's (root nodes in Taffy ignore their margins; a wrapper makes
    // them ordinary child margins).
    let viewport_root = taffy
        .new_with_children(
            TaffyStyle {
                display: taffy::style::Display::Block,
                size: TaffySize {
                    width: TaffyDimension::length(viewport.width() as f32),
                    height: TaffyDimension::length(viewport.height() as f32),
                },
                ..TaffyStyle::default()
            },
            &[mirror.taffy_id],
        )
        .expect("viewport root");

    taffy
        .compute_layout_with_measure(
            viewport_root,
            TaffySize {
                width: AvailableSpace::Definite(viewport.width() as f32),
                height: AvailableSpace::Definite(viewport.height() as f32),
            },
            |input: LayoutInput, id: TaffyId, _, style: &TaffyStyle| {
                let Some(box_node) = leaf_sources.get(&id).copied() else {
                    return LayoutOutput::HIDDEN;
                };
                // Delegate the CSS size/min/max/inset handling to Taffy's own
                // leaf layout; only the text content measurement is ours.
                if input.run_mode == RunMode::PerformHiddenLayout {
                    return LayoutOutput::HIDDEN;
                }
                taffy::compute_leaf_layout(
                    input,
                    style,
                    |_, _| 0.0,
                    |known: TaffySize<Option<f32>>, available: TaffySize<AvailableSpace>| {
                        text_content_size(box_node, known.width, available.width, fonts, scale)
                    },
                )
            },
        )
        .expect("taffy layout succeeds for a well-formed projection");

    // Write back geometry, then position inline content.
    write_back(root, &mirror, &taffy);
    layout_inline_content(root, fonts, scale);
}

// -- projection ---------------------------------------------------------------

/// Mirror of the box tree in Taffy node space, aligned with the box tree
/// for the write-back walk. Anonymous words nodes carry `anonymous = true`
/// and have no BoxNode counterpart.
struct MirrorNode {
    taffy_id: TaffyId,
    anonymous: bool,
    children: Vec<MirrorNode>,
}

fn project<'a>(
    node: &'a BoxNode,
    taffy: &mut TaffyTree,
    leaf_sources: &mut HashMap<TaffyId, &'a BoxNode>,
    scale: f32,
) -> MirrorNode {
    let style = map_style(
        &node.style,
        scale,
        PAGE_ROOT_TAGS.contains(&node.tag.as_str()),
    );

    // Leaves: no block children — the box's own words (possibly none).
    if node.children.is_empty() {
        let id = taffy.new_leaf(style).expect("new leaf");
        leaf_sources.insert(id, node);
        return MirrorNode {
            taffy_id: id,
            anonymous: false,
            children: Vec::new(),
        };
    }

    // Containers: an anonymous words node (CSS anonymous block/flex item)
    // precedes the real children, matching the M2a ordering where inline
    // content renders above block children.
    let mut taffy_children: Vec<TaffyId> = Vec::new();
    let mut children: Vec<MirrorNode> = Vec::new();
    if !node.words.is_empty() {
        // Clone: the container itself needs the original style below.
        let mut anon_style = style.clone();
        // The anonymous item behaves like a plain block in flex contexts.
        anon_style.display = taffy::style::Display::Block;
        anon_style.flex_grow = 0.0;
        anon_style.flex_shrink = 0.0;
        let id = taffy.new_leaf(anon_style).expect("new anon leaf");
        leaf_sources.insert(id, node);
        children.push(MirrorNode {
            taffy_id: id,
            anonymous: true,
            children: Vec::new(),
        });
        taffy_children.push(id);
    }
    for child in &node.children {
        let mirror = project(child, taffy, leaf_sources, scale);
        taffy_children.push(mirror.taffy_id);
        children.push(mirror);
    }
    let id = taffy
        .new_with_children(style, &taffy_children)
        .expect("new container");
    MirrorNode {
        taffy_id: id,
        anonymous: false,
        children,
    }
}

/// Maps a Velqu computed style onto a Taffy style. All CSS lengths are
/// resolved to device pixels here so Taffy computes in one metric space.
fn map_style(style: &ComputedStyle, scale: f32, is_page_root: bool) -> TaffyStyle {
    let display: taffy::style::Display = match style.display {
        Display::None => taffy::style::Display::None,
        Display::Flex => taffy::style::Display::Flex,
        Display::Inline | Display::Block => {
            if is_page_root {
                taffy::style::Display::FlowRoot
            } else {
                taffy::style::Display::Block
            }
        }
    };

    let align_items = match style.align_items {
        // Baseline is deferred (ADR 0007): diagnosed at parse time, laid
        // out as flex-start.
        AlignItems::Baseline => Some(taffy::style::AlignItems::FLEX_START),
        AlignItems::Stretch => Some(taffy::style::AlignItems::STRETCH),
        AlignItems::FlexStart => Some(taffy::style::AlignItems::FLEX_START),
        AlignItems::Center => Some(taffy::style::AlignItems::CENTER),
        AlignItems::FlexEnd => Some(taffy::style::AlignItems::FLEX_END),
    };
    let align_self = match style.align_self {
        AlignSelf::Auto => None,
        AlignSelf::Baseline => Some(taffy::style::AlignSelf::FLEX_START),
        AlignSelf::Stretch => Some(taffy::style::AlignSelf::STRETCH),
        AlignSelf::FlexStart => Some(taffy::style::AlignSelf::FLEX_START),
        AlignSelf::Center => Some(taffy::style::AlignSelf::CENTER),
        AlignSelf::FlexEnd => Some(taffy::style::AlignSelf::FLEX_END),
    };

    let length = |len: Length| length_percentage(len, scale);
    let length_auto = |len: Length| length_percentage_auto(len, scale);

    TaffyStyle {
        display,
        box_sizing: BoxSizing::ContentBox,
        overflow: TaffyPoint {
            x: map_overflow(style.overflow_x),
            y: map_overflow(style.overflow_y),
        },
        size: TaffySize {
            width: dim(style.width, scale),
            height: dim(style.height, scale),
        },
        min_size: TaffySize {
            width: style
                .min_width
                .map(length_auto)
                .unwrap_or(LengthPercentageAuto::auto()),
            height: style
                .min_height
                .map(length_auto)
                .unwrap_or(LengthPercentageAuto::auto()),
        },
        max_size: TaffySize {
            width: style
                .max_width
                .map(length_auto)
                .unwrap_or(LengthPercentageAuto::auto()),
            height: style
                .max_height
                .map(length_auto)
                .unwrap_or(LengthPercentageAuto::auto()),
        },
        margin: TaffyRect {
            left: length_auto(style.margin.left),
            right: length_auto(style.margin.right),
            top: length_auto(style.margin.top),
            bottom: length_auto(style.margin.bottom),
        },
        padding: TaffyRect {
            left: length(style.padding.left),
            right: length(style.padding.right),
            top: length(style.padding.top),
            bottom: length(style.padding.bottom),
        },
        border: TaffyRect {
            left: length(style.border_width.left),
            right: length(style.border_width.right),
            top: length(style.border_width.top),
            bottom: length(style.border_width.bottom),
        },
        flex_direction: match style.flex_direction {
            crate::style::FlexDirection::Row => taffy::style::FlexDirection::Row,
            crate::style::FlexDirection::RowReverse => taffy::style::FlexDirection::RowReverse,
            crate::style::FlexDirection::Column => taffy::style::FlexDirection::Column,
            crate::style::FlexDirection::ColumnReverse => {
                taffy::style::FlexDirection::ColumnReverse
            }
        },
        flex_wrap: match style.flex_wrap {
            crate::style::FlexWrap::Nowrap => taffy::style::FlexWrap::NoWrap,
            crate::style::FlexWrap::Wrap => taffy::style::FlexWrap::Wrap,
        },
        flex_grow: style.flex_grow,
        flex_shrink: style.flex_shrink,
        flex_basis: style
            .flex_basis
            .map(|len| dim(Some(len), scale))
            .unwrap_or(TaffyDimension::auto()),
        justify_content: match style.justify_content {
            JustifyContent::FlexStart => Some(taffy::style::JustifyContent::FLEX_START),
            JustifyContent::Center => Some(taffy::style::JustifyContent::CENTER),
            JustifyContent::FlexEnd => Some(taffy::style::JustifyContent::FLEX_END),
            JustifyContent::SpaceBetween => Some(taffy::style::JustifyContent::SPACE_BETWEEN),
        },
        align_items,
        align_self,
        gap: TaffySize {
            width: style
                .column_gap
                .map(length)
                .unwrap_or(LengthPercentage::length(0.0)),
            height: style
                .row_gap
                .map(length)
                .unwrap_or(LengthPercentage::length(0.0)),
        },
        ..TaffyStyle::default()
    }
}

fn map_overflow(overflow: Overflow) -> taffy::style::Overflow {
    match overflow {
        Overflow::Visible => taffy::style::Overflow::Visible,
        // `clip` behaves like `hidden` for Taffy's layout containment; the
        // difference (no scrolling ever) is a paint-side concern here.
        Overflow::Hidden | Overflow::Clip => taffy::style::Overflow::Hidden,
    }
}

fn dim(len: Option<Length>, scale: f32) -> TaffyDimension {
    match len {
        None => TaffyDimension::auto(),
        Some(Length::Px(v)) => TaffyDimension::length(v * scale),
        Some(Length::Percent(p)) => TaffyDimension::percent(p / 100.0),
        Some(Length::Rem(v)) => TaffyDimension::length(v * scale),
    }
}

/// Resolves a length to an absolute device-pixel length. Percentages stay
/// percentages (Taffy resolves them against the propagated parent size).
fn length_percentage(len: Length, scale: f32) -> LengthPercentage {
    match len {
        Length::Px(v) => LengthPercentage::length(v * scale),
        Length::Percent(p) => LengthPercentage::percent(p / 100.0),
        Length::Rem(v) => LengthPercentage::length(v * scale),
    }
}

fn length_percentage_auto(len: Length, scale: f32) -> LengthPercentageAuto {
    match len {
        Length::Px(v) => LengthPercentageAuto::length(v * scale),
        Length::Percent(p) => LengthPercentageAuto::percent(p / 100.0),
        Length::Rem(v) => LengthPercentageAuto::length(v * scale),
    }
}

// -- leaf measurement ---------------------------------------------------------

/// Measures a text leaf's **content size**: wraps the box's words at the
/// available width. CSS size/min/max/inset handling is delegated to
/// Taffy's `compute_leaf_layout` by the caller.
fn text_content_size(
    box_node: &BoxNode,
    known_width: Option<f32>,
    available_width: AvailableSpace,
    fonts: &mut FontStore,
    scale: f32,
) -> TaffySize<f32> {
    let line_height = box_line_height(&box_node.style, scale);

    let wrap_width = match available_width {
        AvailableSpace::Definite(w) => known_width.or(Some(w)),
        // MaxContent: no wrapping; MinContent: wrap at the longest word.
        AvailableSpace::MaxContent => None,
        AvailableSpace::MinContent => Some(longest_word(box_node, fonts, scale)),
    };

    let width = match wrap_width {
        None => unwrapped_width(box_node, fonts, scale),
        Some(w) => break_lines(&box_node.words, w, fonts, scale)
            .last()
            .map(line_width)
            .unwrap_or(0.0),
    };
    let height = line_height * count_lines(box_node, wrap_width, fonts, scale) as f32;
    TaffySize { width, height }
}

/// The box's line height in device px (CSS px scaled).
fn box_line_height(style: &ComputedStyle, scale: f32) -> f32 {
    let font_px = (style.font_size * scale).max(1.0);
    match style.line_height {
        LineHeight::Normal => 1.5 * font_px,
        LineHeight::Number(n) => n * font_px,
        LineHeight::Px(v) => v * scale,
    }
}

fn line_width(line: &LaidLine) -> f32 {
    line.runs.last().map(|run| run.x + run.width).unwrap_or(0.0)
}

fn longest_word(box_node: &BoxNode, fonts: &mut FontStore, scale: f32) -> f32 {
    box_node
        .words
        .iter()
        .map(|word| {
            let px = word_px(word, scale);
            text_run_width(fonts, &word.text, px, word.style.font_weight)
        })
        .fold(0.0, f32::max)
}

fn unwrapped_width(box_node: &BoxNode, fonts: &mut FontStore, scale: f32) -> f32 {
    let mut width = 0.0;
    let mut last_px = 16;
    let mut first = true;
    for word in &box_node.words {
        let px = word_px(word, scale);
        let space = if first {
            0.0
        } else {
            // Space width follows the previous word's size, matching
            // break_lines' placement.
            text_run_width(fonts, " ", last_px, 400)
        };
        width += space + text_run_width(fonts, &word.text, px, word.style.font_weight);
        last_px = px;
        first = false;
    }
    width
}

fn count_lines(
    box_node: &BoxNode,
    wrap_width: Option<f32>,
    fonts: &mut FontStore,
    scale: f32,
) -> usize {
    match wrap_width {
        None => {
            if box_node.words.is_empty() {
                0
            } else {
                1
            }
        }
        Some(width) => break_lines(&box_node.words, width, fonts, scale).len(),
    }
}

fn word_px(word: &crate::layout::Word, scale: f32) -> u16 {
    (word.style.font_size * scale).round().max(1.0) as u16
}

fn text_run_width(fonts: &mut FontStore, text: &str, px: u16, weight: u16) -> f32 {
    crate::text::measure_run(fonts, text, px, crate::text::face_weight(weight))
}

// -- write-back ----------------------------------------------------------------

/// Copies Taffy's layout results into the box tree geometry.
fn write_back(node: &mut BoxNode, mirror: &MirrorNode, taffy: &TaffyTree) {
    write_back_at(node, mirror, taffy, 0.0, 0.0);
}

/// `origin` is the parent's border-box origin: Taffy locations are
/// parent-relative, so absolute geometry accumulates down the walk.
fn write_back_at(
    node: &mut BoxNode,
    mirror: &MirrorNode,
    taffy: &TaffyTree,
    origin_x: f32,
    origin_y: f32,
) {
    let mut anon_origin: Option<(f32, f32, f32)> = None; // (x, y, width)

    let layout = taffy.layout(mirror.taffy_id).expect("layout exists");
    let abs_x = origin_x + layout.location.x;
    let abs_y = origin_y + layout.location.y;
    if !mirror.anonymous {
        node.border_box = Rect {
            x: abs_x,
            y: abs_y,
            w: layout.size.width,
            h: layout.size.height,
        };
        node.padding_box = Rect {
            x: node.border_box.x + layout.border.left,
            y: node.border_box.y + layout.border.top,
            w: node.border_box.w - layout.border.left - layout.border.right,
            h: node.border_box.h - layout.border.top - layout.border.bottom,
        };
        node.content = Rect {
            x: node.padding_box.x + layout.padding.left,
            y: node.padding_box.y + layout.padding.top,
            w: node.padding_box.w - layout.padding.left - layout.padding.right,
            h: node.padding_box.h - layout.padding.top - layout.padding.bottom,
        };
        node.margin = Sides {
            top: layout.margin.top,
            right: layout.margin.right,
            bottom: layout.margin.bottom,
            left: layout.margin.left,
        };
    }

    // Pair mirror children with box children in order, capturing the first
    // anonymous (words) node's content origin for the inline pass.
    let mut box_index = 0usize;
    for child_mirror in &mirror.children {
        if child_mirror.anonymous {
            let anon = taffy.layout(child_mirror.taffy_id).expect("anon layout");
            // Content origin of the anonymous node = its border box + its
            // own insets (its style has none, so border box == content box).
            anon_origin = Some((
                abs_x + anon.location.x,
                abs_y + anon.location.y,
                anon.size.width,
            ));
            continue;
        }
        if let Some(child) = node.children.get_mut(box_index) {
            write_back_at(child, child_mirror, taffy, abs_x, abs_y);
        }
        box_index += 1;
    }
    if let Some((x, y, width)) = anon_origin {
        node.text_origin = Some(TextOrigin { x, y, width });
    }
}

// -- inline content positioning --------------------------------------------------

/// Breaks and positions each box's words inside its (or its anonymous
/// node's) content area; fills `node.lines`.
fn layout_inline_content(node: &mut BoxNode, fonts: &mut FontStore, scale: f32) {
    if !node.words.is_empty() {
        // Only the wrap width matters here; emit reads the absolute text
        // origin from the box when painting.
        let wrap_width = match node.text_origin {
            Some(origin) => origin.width,
            None => node.content.w,
        };
        let line_height = match node.style.line_height {
            LineHeight::Normal => 1.5 * (node.style.font_size * scale).max(1.0),
            LineHeight::Number(n) => n * (node.style.font_size * scale).max(1.0),
            LineHeight::Px(v) => v * scale,
        };
        let mut lines = break_lines(&node.words, wrap_width, fonts, scale);
        let mut cursor = 0.0;
        for line in &mut lines {
            line.y = cursor;
            line.height = line_height;
            let width = line_width(line);
            let offset = match node.style.text_align {
                TextAlign::Left => 0.0,
                TextAlign::Center => ((wrap_width - width) / 2.0).max(0.0),
                TextAlign::Right => (wrap_width - width).max(0.0),
            };
            for run in &mut line.runs {
                run.x += offset;
            }
            cursor += line.height;
        }
        node.lines = lines;
    }
    for child in &mut node.children {
        layout_inline_content(child, fonts, scale);
    }
}

/// Shared line breaking: words → unaligned laid lines (device px).
pub(crate) fn break_lines(
    words: &[crate::layout::Word],
    max_width: f32,
    fonts: &mut FontStore,
    scale: f32,
) -> Vec<LaidLine> {
    let mut lines: Vec<LaidLine> = Vec::new();
    let mut current = LaidLine::default();
    let mut cursor_x = 0.0;
    for word in words {
        let px = word_px(word, scale);
        let weight = crate::text::face_weight(word.style.font_weight);
        let word_width = crate::text::measure_run(fonts, &word.text, px, weight);
        let space_width = if current.runs.is_empty() || !word.space_before {
            0.0
        } else {
            let last = current.runs.last().expect("checked non-empty");
            crate::text::measure_run(fonts, " ", last.px, crate::text::face_weight(400))
        };
        if !current.runs.is_empty()
            && max_width > 0.0
            && cursor_x + space_width + word_width > max_width
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
            px,
            bold: weight == crate::font::FontWeight::Bold,
            color: word.style.color,
            source: word.source,
        });
        cursor_x = run_x + word_width;
    }
    if !current.runs.is_empty() {
        lines.push(current);
    }
    lines
}
