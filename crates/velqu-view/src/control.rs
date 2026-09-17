//! Editable-control state and facts for M4c1.
//!
//! A control's initial value comes from the DOM, but its current value lives
//! here at runtime. Layout only sees the control's replaced outer box; editor
//! state is never written back into the DOM or `LayoutFacts`.

use crate::color::Color;
use crate::display_list::Rect;
use crate::dom::{Dom, NodeData, NodeId};
use crate::editor::EditorState;
use crate::font::{FontStore, FontWeight};
use crate::input::ElementTarget;
use crate::layout::BoxNode;
use crate::style::{ComputedStyle, LineHeight};
use crate::text;
use unicode_segmentation::UnicodeSegmentation;

/// The M4c1 editable-control profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    /// A single-line `<input type="text">` control.
    InputText,
    /// A multiline `<textarea>` control.
    Textarea,
}

impl ControlKind {
    /// Whether this control accepts a newline as direct text input.
    pub(crate) fn accepts_newline(self) -> bool {
        matches!(self, Self::Textarea)
    }

    /// Filters platform/clipboard text down to what this control accepts
    /// as inserted text: control characters drop, newlines survive only in
    /// textarea, and `\r` always drops so CRLF pastes normalize to `\n`
    /// (M4c2).
    pub(crate) fn filter_text(self, text: &str) -> String {
        text.chars()
            .filter(|character| {
                if *character == '\n' {
                    self.accepts_newline()
                } else if *character == '\r' {
                    false
                } else {
                    !character.is_control()
                }
            })
            .collect()
    }

    /// Deterministic intrinsic outer size used when CSS leaves dimensions
    /// automatic. Author width/height declarations still determine the box.
    pub(crate) fn intrinsic_size(self, scale: f32) -> (f32, f32) {
        match self {
            Self::InputText => (200.0 * scale, 32.0 * scale),
            Self::Textarea => (240.0 * scale, 96.0 * scale),
        }
    }
}

/// A control's runtime value, selection, internal scroll, and IME
/// composition state (M4c3, ADR 0014). The composition is presentation
/// state: it never enters `editor`'s value — only an explicit commit
/// converts it into text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ControlState {
    pub(crate) kind: ControlKind,
    pub(crate) editor: EditorState,
    pub(crate) readonly: bool,
    pub(crate) disabled: bool,
    pub(crate) dirty: bool,
    pub(crate) scroll_offset: (f32, f32),
    pub(crate) composition: Option<CompositionState>,
}

impl ControlState {
    pub(crate) fn new(kind: ControlKind, value: String, readonly: bool, disabled: bool) -> Self {
        Self {
            kind,
            editor: EditorState::new(value),
            readonly,
            disabled,
            dirty: false,
            scroll_offset: (0.0, 0.0),
            composition: None,
        }
    }

    pub(crate) fn value(&self) -> &str {
        self.editor.value()
    }

    pub(crate) fn selection(&self) -> (usize, usize) {
        (self.editor.anchor(), self.editor.focus())
    }

    /// The text the editor *paints*: the real value with the composition
    /// spliced over the range it replaces. Layout of this text is
    /// presentation-only; the value stays untouched until commit.
    pub(crate) fn effective_text(&self) -> String {
        match &self.composition {
            None => self.value().to_owned(),
            Some(composition) => format!(
                "{}{}{}",
                &self.value()[..composition.range.0],
                composition.text,
                &self.value()[composition.range.1..]
            ),
        }
    }

    /// Where the visual caret sits in `effective_text` byte offsets: the
    /// composition cursor while composing, the editor caret otherwise.
    pub(crate) fn effective_caret(&self) -> usize {
        match &self.composition {
            None => self.editor.focus(),
            Some(composition) => composition.range.0 + composition.cursor.1,
        }
    }
}

/// An active IME composition (M4c3, ADR 0014). All offsets are valid
/// UTF-8 byte offsets; platform-supplied indexes are clamped here —
/// never trusted (the gate's scenario 6/7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompositionState {
    /// The preedit text. Never written into the control value.
    pub(crate) text: String,
    /// Cursor/selection within the preedit text, as clamped byte offsets.
    pub(crate) cursor: (usize, usize),
    /// The value range (captured from the selection at composition start)
    /// that a commit will replace.
    pub(crate) range: (usize, usize),
}

impl CompositionState {
    /// Clamps `cursor` into `text` on valid UTF-8 boundaries, ordered.
    pub(crate) fn new(text: String, cursor: Option<(usize, usize)>, range: (usize, usize)) -> Self {
        let clamp = |offset: usize| {
            let mut offset = offset.min(text.len());
            while offset > 0 && !text.is_char_boundary(offset) {
                offset -= 1;
            }
            offset
        };
        let (mut start, mut end) = cursor.unwrap_or((text.len(), text.len()));
        start = clamp(start);
        end = clamp(end);
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        Self {
            text,
            cursor: (start, end),
            range,
        }
    }
}

/// One control's structural/runtime facts, separate from `LayoutFacts`.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlFact {
    /// Opaque element identity plus optional author id.
    pub target: ElementTarget,
    /// Control kind in the M4c1 profile.
    pub kind: ControlKind,
    /// Current value length in UTF-8 bytes.
    pub value_length: usize,
    /// Selection anchor in the current value, as a valid UTF-8 byte offset.
    pub selection_anchor: usize,
    /// Selection focus/caret in the current value, as a valid UTF-8 byte
    /// offset.
    pub selection_focus: usize,
    /// Caret rectangle in device pixels.
    pub caret_rect: ControlRect,
    /// Visible value range in UTF-8 byte offsets (M4c1 starts with the full
    /// range; internal scrolling narrows it as the editor grows).
    pub visible_text_range: (usize, usize),
    /// Internal editor scroll offset in device pixels.
    pub scroll_offset: (f32, f32),
}

/// A device-pixel rectangle used by control facts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlRect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// Snapshot of all editable controls in document order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControlFacts {
    /// One entry per supported current-document control.
    pub controls: Vec<ControlFact>,
}

/// DOM-derived initialization for one supported control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ControlInit {
    pub node: NodeId,
    pub kind: ControlKind,
    pub initial_value: String,
    pub readonly: bool,
    pub disabled: bool,
}

/// Discovers supported controls and deterministic diagnostics for unsupported
/// input types. The returned order is DOM preorder.
pub(crate) fn discover(dom: &Dom) -> (Vec<ControlInit>, Vec<String>) {
    let mut controls = Vec::new();
    let mut diagnostics = Vec::new();
    dom.walk(|node, data| {
        let NodeData::Element { name, attrs, .. } = &data.data else {
            return;
        };
        let attr = |key: &str| attrs.iter().find(|attr| attr.name == key);
        let bool_attr = |key: &str| attr(key).is_some();
        match name.as_str() {
            "textarea" => controls.push(ControlInit {
                node,
                kind: ControlKind::Textarea,
                initial_value: dom.descendant_text(node),
                readonly: bool_attr("readonly"),
                disabled: bool_attr("disabled"),
            }),
            "input" => {
                let input_type = attr("type")
                    .map(|attribute| attribute.value.to_ascii_lowercase())
                    .unwrap_or_else(|| "text".to_owned());
                if input_type == "text" {
                    controls.push(ControlInit {
                        node,
                        kind: ControlKind::InputText,
                        initial_value: attr("value")
                            .map(|attribute| attribute.value.clone())
                            .unwrap_or_default(),
                        readonly: bool_attr("readonly"),
                        disabled: bool_attr("disabled"),
                    });
                } else {
                    diagnostics.push(format!(
                        "input node {node}: type {input_type:?} is outside the M4c1 editable-control profile"
                    ));
                }
            }
            _ => {}
        }
    });
    (controls, diagnostics)
}

/// Returns a supported control kind for a DOM node, if any.
pub(crate) fn kind_for_node(dom: &Dom, node: NodeId) -> Option<ControlKind> {
    let NodeData::Element { name, attrs, .. } = &dom.node(node).data else {
        return None;
    };
    match name.as_str() {
        "textarea" => Some(ControlKind::Textarea),
        "input" => {
            let input_type = attrs
                .iter()
                .find(|attribute| attribute.name == "type")
                .map(|attribute| attribute.value.as_str())
                .unwrap_or("text");
            input_type
                .eq_ignore_ascii_case("text")
                .then_some(ControlKind::InputText)
        }
        _ => None,
    }
}

/// Runtime geometry used by control facts and pointer mapping. The rectangle
/// is in the same unscrolled document space as the cached box tree; enclosing
/// scroll transforms are applied by the display list and hit-test callers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EditorGeometry {
    pub(crate) caret: Rect,
    pub(crate) visible_range: (usize, usize),
    pub(crate) scroll_offset: (f32, f32),
}

/// Builds runtime control paint items and geometry without changing the box
/// tree. The returned items are inserted at each control's existing paint
/// position, so enclosing clip and scroll scopes remain authoritative.
pub(crate) fn build_paint_items(
    root: &BoxNode,
    controls: &mut std::collections::HashMap<NodeId, ControlState>,
    focused: Option<NodeId>,
    fonts: &mut FontStore,
    scale: f32,
) -> (
    std::collections::HashMap<NodeId, Vec<crate::display_list::DisplayItem>>,
    std::collections::HashMap<NodeId, EditorGeometry>,
) {
    let mut items = std::collections::HashMap::new();
    let mut geometry = std::collections::HashMap::new();
    collect_paint_items(
        root,
        controls,
        focused,
        fonts,
        scale,
        &mut items,
        &mut geometry,
    );
    (items, geometry)
}

fn collect_paint_items(
    node: &BoxNode,
    controls: &mut std::collections::HashMap<NodeId, ControlState>,
    focused: Option<NodeId>,
    fonts: &mut FontStore,
    scale: f32,
    items: &mut std::collections::HashMap<NodeId, Vec<crate::display_list::DisplayItem>>,
    geometry: &mut std::collections::HashMap<NodeId, EditorGeometry>,
) {
    if node.control.is_some() {
        if let Some(state) = controls.get_mut(&node.node) {
            let (paint, facts) =
                paint_control(node, state, focused == Some(node.node), fonts, scale);
            items.insert(node.node, paint);
            geometry.insert(node.node, facts);
        }
    }
    for child in &node.children {
        collect_paint_items(child, controls, focused, fonts, scale, items, geometry);
    }
}

fn paint_control(
    node: &BoxNode,
    state: &mut ControlState,
    focused: bool,
    fonts: &mut FontStore,
    scale: f32,
) -> (Vec<crate::display_list::DisplayItem>, EditorGeometry) {
    let style = &node.style;
    let px = (style.font_size * scale).round().max(1.0) as u16;
    let weight = crate::text::face_weight(style.font_weight);
    let line_height = line_height(style, scale);
    let content = node.content;
    // The editor paints its effective text: value + composition (M4c3,
    // ADR 0014). The real value and selection are untouched by preedit.
    let value = state.effective_text();
    let caret = state.effective_caret();
    let composition_span = state.composition.as_ref().map(|composition| {
        (
            composition.range.0,
            composition.range.0 + composition.text.len(),
        )
    });
    let lines = line_spans(&value, state.kind);
    let caret_line = line_for_offset(&lines, caret);
    let caret_line_start = lines[caret_line].0;
    let caret_column = caret.saturating_sub(caret_line_start).min(value.len());
    let caret_x = prefix_width(
        &value,
        caret_line_start,
        caret_column + caret_line_start,
        fonts,
        px,
        weight,
    );
    let line_widths: Vec<f32> = lines
        .iter()
        .map(|(start, end, _)| text::measure_run(fonts, &value[*start..*end], px, weight))
        .collect();

    let mut scroll_x = state.scroll_offset.0.max(0.0);
    let mut scroll_y = state.scroll_offset.1.max(0.0);
    let inner_width = content.w.max(1.0);
    let inner_height = content.h.max(1.0);
    if state.kind == ControlKind::InputText {
        let max_scroll = (line_widths[0] - inner_width).max(0.0);
        if caret_x < scroll_x {
            scroll_x = caret_x;
        }
        if caret_x > scroll_x + inner_width - 1.0 {
            scroll_x = caret_x - inner_width + 1.0;
        }
        scroll_x = scroll_x.min(max_scroll);
        scroll_y = 0.0;
    } else {
        let caret_y = caret_line as f32 * line_height;
        let max_scroll_y = ((lines.len() as f32 * line_height) - inner_height).max(0.0);
        if caret_y < scroll_y {
            scroll_y = caret_y;
        }
        if caret_y + line_height > scroll_y + inner_height {
            scroll_y = (caret_y + line_height - inner_height).max(0.0);
        }
        scroll_y = scroll_y.min(max_scroll_y);
        let max_line_width = line_widths.iter().copied().fold(0.0, f32::max);
        let max_scroll_x = (max_line_width - inner_width).max(0.0);
        if caret_x < scroll_x {
            scroll_x = caret_x;
        }
        if caret_x > scroll_x + inner_width - 1.0 {
            scroll_x = caret_x - inner_width + 1.0;
        }
        scroll_x = scroll_x.min(max_scroll_x);
    }
    state.scroll_offset = (scroll_x, scroll_y);

    let caret_rect = Rect {
        x: content.x + caret_x - scroll_x,
        y: content.y + caret_line as f32 * line_height - scroll_y,
        w: 1.0,
        h: line_height.min(content.h.max(1.0)),
    };
    let visible_range = visible_range(&lines, line_height, content.h, scroll_y);

    let mut paint = Vec::new();
    if style.background_color.a == 0 {
        paint.push(crate::display_list::DisplayItem::FillRect {
            rect: node.padding_box,
            color: Color::WHITE,
        });
    }
    if !style.border_style_solid {
        // Fixed 1px runtime ring in UA profile colors (M4c1); an author
        // border/background paints through the structural path instead.
        let pb = node.padding_box;
        if pb.w >= 2.0 && pb.h >= 2.0 {
            let ring = Color::from_rgb8(0x9c, 0xa3, 0xaf);
            for rect in [
                Rect {
                    x: pb.x,
                    y: pb.y,
                    w: pb.w,
                    h: 1.0,
                },
                Rect {
                    x: pb.x,
                    y: pb.y + pb.h - 1.0,
                    w: pb.w,
                    h: 1.0,
                },
                Rect {
                    x: pb.x,
                    y: pb.y + 1.0,
                    w: 1.0,
                    h: pb.h - 2.0,
                },
                Rect {
                    x: pb.x + pb.w - 1.0,
                    y: pb.y + 1.0,
                    w: 1.0,
                    h: pb.h - 2.0,
                },
            ] {
                paint.push(crate::display_list::DisplayItem::FillRect { rect, color: ring });
            }
        }
    }
    paint.push(crate::display_list::DisplayItem::PushClip(content));
    // While composing, the selection is suspended — the composition owns
    // the replaced range — so only paint the composition underline.
    let selection = if composition_span.is_none() {
        state.editor.selected_range()
    } else {
        None
    };
    if let Some((composition_start, composition_end)) = composition_span {
        // Composition underline: a thin rule under each painted line the
        // composition touches (fixed profile styling, like the M4c1 chrome).
        for (line_index, (start, end, _)) in lines.iter().enumerate() {
            let from = composition_start.max(*start).min(*end);
            let to = composition_end.max(*start).min(*end);
            if from >= to {
                continue;
            }
            let x0 = prefix_width(&value, *start, from, fonts, px, weight);
            let x1 = prefix_width(&value, *start, to, fonts, px, weight);
            paint.push(crate::display_list::DisplayItem::FillRect {
                rect: Rect {
                    x: content.x + x0 - scroll_x,
                    y: content.y + line_index as f32 * line_height - scroll_y + line_height - 2.0,
                    w: (x1 - x0).max(1.0),
                    h: 2.0,
                },
                color: Color::from_rgb8(0x25, 0x62, 0xb9),
            });
        }
    }
    if let Some((selection_start, selection_end)) = selection {
        for (line_index, (start, end, _)) in lines.iter().enumerate() {
            let from = selection_start.max(*start).min(*end);
            let to = selection_end.max(*start).min(*end);
            if from >= to {
                continue;
            }
            let x0 = prefix_width(&value, *start, from, fonts, px, weight);
            let x1 = prefix_width(&value, *start, to, fonts, px, weight);
            paint.push(crate::display_list::DisplayItem::FillRect {
                rect: Rect {
                    x: content.x + x0 - scroll_x,
                    y: content.y + line_index as f32 * line_height - scroll_y,
                    w: (x1 - x0).max(1.0),
                    h: line_height,
                },
                color: Color::from_rgb8(0xbf, 0xdb, 0xfe),
            });
        }
    }
    for (line_index, (start, end, _)) in lines.iter().enumerate() {
        if start == end {
            continue;
        }
        paint.push(crate::display_list::DisplayItem::TextRun {
            x: content.x - scroll_x,
            y: baseline(
                content.y + line_index as f32 * line_height - scroll_y,
                line_height,
                px,
            ),
            text: value[*start..*end].to_owned(),
            px,
            bold: weight == FontWeight::Bold,
            color: style.color,
        });
    }
    if focused && !state.disabled {
        paint.push(crate::display_list::DisplayItem::FillRect {
            rect: caret_rect,
            color: Color::from_rgb8(0x11, 0x18, 0x27),
        });
    }
    paint.push(crate::display_list::DisplayItem::PopClip);
    (
        paint,
        EditorGeometry {
            caret: caret_rect,
            visible_range,
            scroll_offset: state.scroll_offset,
        },
    )
}

fn line_height(style: &ComputedStyle, scale: f32) -> f32 {
    let px = (style.font_size * scale).max(1.0);
    match style.line_height {
        LineHeight::Normal => 1.5 * px,
        LineHeight::Number(value) => value * px,
        LineHeight::Px(value) => value * scale,
    }
}

fn baseline(top: f32, height: f32, px: u16) -> f32 {
    let ascent = 0.928 * px as f32;
    let descent = 0.236 * px as f32;
    top + (height - (ascent - descent)) / 2.0 + ascent
}

fn line_spans(value: &str, kind: ControlKind) -> Vec<(usize, usize, usize)> {
    if kind == ControlKind::InputText {
        return vec![(0, value.len(), 0)];
    }
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, character) in value.char_indices() {
        if character == '\n' {
            lines.push((start, index, lines.len()));
            start = index + character.len_utf8();
        }
    }
    lines.push((start, value.len(), lines.len()));
    lines
}

fn line_for_offset(lines: &[(usize, usize, usize)], offset: usize) -> usize {
    let offset = offset.min(lines.last().map_or(0, |line| line.1));
    lines
        .iter()
        .position(|(start, end, _)| offset >= *start && offset <= *end)
        .unwrap_or_else(|| lines.len().saturating_sub(1))
}

fn prefix_width(
    value: &str,
    start: usize,
    end: usize,
    fonts: &mut FontStore,
    px: u16,
    weight: FontWeight,
) -> f32 {
    let start = start.min(value.len());
    let end = end.clamp(start, value.len());
    text::measure_run(fonts, &value[start..end], px, weight)
}

/// The visible byte range for the current vertical scroll: the union of the
/// lines intersecting the content window. Line-based (M4c1); horizontal
/// visibility inside a line is not narrowed.
fn visible_range(
    lines: &[(usize, usize, usize)],
    line_height: f32,
    content_height: f32,
    scroll_y: f32,
) -> (usize, usize) {
    let last_index = lines.len() - 1;
    let top_line = (scroll_y / line_height).floor().max(0.0) as usize;
    let bottom_line = ((scroll_y + content_height.max(1.0)) / line_height).ceil() as usize;
    let first = lines[top_line.min(last_index)].0;
    let last = lines[bottom_line.saturating_sub(1).min(last_index)].1;
    (first, last)
}

/// Maps a pointer in the control's document-space content box to the nearest
/// valid UTF-8/grapheme boundary. The caller supplies the cached geometry and
/// current internal scroll offset; no layout pass is needed.
pub(crate) fn offset_at_point(
    node: &BoxNode,
    state: &ControlState,
    fonts: &mut FontStore,
    scale: f32,
    x: f32,
    y: f32,
) -> usize {
    let px = (node.style.font_size * scale).round().max(1.0) as u16;
    let weight = crate::text::face_weight(node.style.font_weight);
    let height = line_height(&node.style, scale);
    let lines = line_spans(state.value(), state.kind);
    let line_index = (((y - node.content.y + state.scroll_offset.1) / height)
        .floor()
        .max(0.0) as usize)
        .min(lines.len().saturating_sub(1));
    let (start, end, _) = lines[line_index];
    let local_x = x - node.content.x + state.scroll_offset.0;
    let text = &state.value()[start..end];
    let mut previous = 0.0;
    for (offset, grapheme) in text.grapheme_indices(true) {
        let absolute = start + offset;
        let width = text::measure_run(fonts, grapheme, px, weight);
        if local_x < previous + width / 2.0 {
            return absolute;
        }
        previous += width;
    }
    end
}
