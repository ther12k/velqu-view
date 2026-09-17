//! Style cascade: selector matching, specificity/order resolution, UA
//! defaults, inheritance, and inline styles → `ComputedStyle`.
//!
//! Scope is the M2a CSS profile (ADR 0006):
//! * selectors: type/class/id/universal, descendant/child combinators;
//! * cascade: specificity → source order → inline style; `!important`
//!   outranks normal declarations of its own origin;
//! * inheritance: typography/color properties inherit; box properties reset;
//! * anything unrecognized produces a deterministic diagnostic — no silent
//!   dropping.
//!
//! Computed values keep percentages unresolved (they need the containing
//! block at layout time); `rem` resolves against the root font size here.

// The cascade output is consumed by the box tree/layout stages landing in
// the next M2a commit; until then only tests drive parts of this module.
#![allow(dead_code)]

use std::fmt;

use cssparser::Parser;

use crate::color::Color;
use crate::css::{Combinator, Declaration, Rule, Selector, Simple, Specificity, Stylesheet};
use crate::dom::{Dom, NodeData, NodeId};

/// A resolved CSS length (percentages preserved for layout resolution).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Length {
    Px(f32),
    /// Percentage of the containing block's relevant axis.
    Percent(f32),
    /// Root-relative: already multiplied by the root font size (16px).
    Rem(f32),
}

impl Length {
    /// Absolute pixel value for lengths that do not depend on layout
    /// (margins/padding resolve percentages against... nothing useful in
    /// block flow; M2a treats percentage margins as 0 per CSS edge rules).
    pub(crate) fn px_or_zero(&self) -> f32 {
        match *self {
            Length::Px(v) => v,
            Length::Percent(_) => 0.0,
            Length::Rem(v) => v,
        }
    }
}

/// `display` values in the M2a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Display {
    Block,
    Inline,
    /// M2b: children laid out with the flex algorithm (Taffy-backed).
    Flex,
    /// M2c: children placed on a grid (frozen M2c profile, ADR 0008).
    Grid,
    None,
}

/// M2b flex profile: main-axis direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FlexDirection {
    #[default]
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

/// M2b flex profile: line wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FlexWrap {
    #[default]
    Nowrap,
    Wrap,
}

/// M2b flex profile: main-axis distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum JustifyContent {
    #[default]
    FlexStart,
    Center,
    FlexEnd,
    SpaceBetween,
}

/// M2b flex profile: cross-axis alignment of items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AlignItems {
    #[default]
    Stretch,
    FlexStart,
    Center,
    FlexEnd,
    /// Deferred (ADR 0007): accepted by the parser with a diagnostic,
    /// laid out as FlexStart — never silently.
    Baseline,
}

/// align-self: auto inherits the container align-items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum AlignSelf {
    #[default]
    Auto,
    Stretch,
    FlexStart,
    Center,
    FlexEnd,
    Baseline,
}

/// M2b overflow: visible | hidden | clip. Clipping executes in the
/// display list/painter; layout only switches containment.
///
/// M2c adds `auto`/`scroll`: the box becomes a **scroll container**. In the
/// Velqu profile they behave identically (no scrollbar gutter modeling);
/// scrolling is paint-side (`PushTransform`), and layout geometry stays the
/// unscrolled truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Overflow {
    #[default]
    Visible,
    Hidden,
    Clip,
    Auto,
    Scroll,
}

impl Overflow {
    /// Does this value make the box a scroll container?
    pub(crate) fn is_scroll_container(self) -> bool {
        matches!(self, Overflow::Auto | Overflow::Scroll)
    }
}

/// M2c grid profile (ADR 0008): one track sizing function.
///
/// Frozen surface: px, %, fr, auto, `minmax(min, max)`, and
/// `repeat(<fixed integer>, …)`. Named lines, template areas, auto-fill/
/// auto-fit, subgrid, and masonry are outside the profile (diagnostics).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GridTrack {
    Px(f32),
    Percent(f32),
    Fr(f32),
    Auto,
    /// `minmax(min, max)`; the min side is a length/auto, the max side a
    /// length/auto/fr (profile restriction).
    MinMax(GridTrackMin, GridTrackMax),
    /// `repeat(count, tracks)` with a fixed integer count; inner tracks may
    /// not themselves be repeats.
    Repeat {
        count: u16,
        tracks: Vec<GridTrack>,
    },
}

/// The min side of `minmax()`: a length, percentage, or auto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum GridTrackMin {
    Px(f32),
    Percent(f32),
    Auto,
}

/// The max side of `minmax()`: a length, percentage, fr, or auto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum GridTrackMax {
    Px(f32),
    Percent(f32),
    Fr(f32),
    Auto,
}

/// M2c grid profile: one side of a `grid-column`/`grid-row` placement.
///
/// Positive integer lines and spans only; negative line numbers (from the
/// end) are outside the profile (diagnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GridLine {
    #[default]
    Auto,
    /// 1-based line index from the start.
    Index(i16),
    /// Span this many tracks.
    Span(u16),
}

/// M2c grid profile: start/end placement for one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct GridPlacement {
    pub start: GridLine,
    pub end: GridLine,
}

/// M2c grid profile: auto-placement direction. `dense` packing is outside
/// the profile (diagnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GridAutoFlow {
    #[default]
    Row,
    Column,
}

/// Which box `width`/`height` size (M3): content box (the CSS default and
/// the M2a–M2c renderer behavior) or border box (Tailwind's preflight
/// default, used by the utility pipeline).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

/// Text alignment in the M2a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextAlign {
    Left,
    Center,
    Right,
}

/// `white-space` subset in the M2a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WhiteSpace {
    /// Collapse runs of whitespace, wrap at box edge.
    Normal,
    /// Preserve whitespace and newlines, wrap long lines.
    Pre,
    /// Collapse whitespace but never wrap.
    Nowrap,
}

/// `line-height`: a unitless multiplier or an absolute pixel size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum LineHeight {
    Normal,
    Number(f32),
    Px(f32),
}

/// The four box sides, in CSS order: top, right, bottom, left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sides<T> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

impl Default for Sides<Length> {
    fn default() -> Self {
        Sides {
            top: Length::Px(0.0),
            right: Length::Px(0.0),
            bottom: Length::Px(0.0),
            left: Length::Px(0.0),
        }
    }
}

impl Default for Sides<f32> {
    fn default() -> Self {
        Sides {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        }
    }
}

impl Sides<Length> {
    pub(crate) fn px_or_zero(&self) -> Sides<f32> {
        Sides {
            top: self.top.px_or_zero(),
            right: self.right.px_or_zero(),
            bottom: self.bottom.px_or_zero(),
            left: self.left.px_or_zero(),
        }
    }
}

/// Fully computed style for one element (M2a profile properties).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ComputedStyle {
    pub display: Display,
    pub width: Option<Length>,
    pub height: Option<Length>,
    pub min_width: Option<Length>,
    pub min_height: Option<Length>,
    pub max_width: Option<Length>,
    pub max_height: Option<Length>,
    pub margin: Sides<Length>,
    pub padding: Sides<Length>,
    /// Border widths per side; only drawn where `border_style` is solid.
    pub border_width: Sides<Length>,
    pub border_color: Color,
    pub border_style_solid: bool,
    pub border_radius: f32,
    pub color: Color,
    pub background_color: Color,
    pub font_size: f32,
    pub font_weight: u16,
    pub line_height: LineHeight,
    pub text_align: TextAlign,
    pub white_space: WhiteSpace,
    /// M2b flex profile (all non-inheriting).
    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    /// `None` = auto.
    pub flex_basis: Option<Length>,
    pub justify_content: JustifyContent,
    pub align_items: AlignItems,
    pub align_self: AlignSelf,
    /// `gap` shorthand sets both; percentages resolve at layout.
    pub row_gap: Option<Length>,
    pub column_gap: Option<Length>,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    /// M2c grid profile (only meaningful when `display` is `Grid`).
    pub grid_template_columns: Vec<GridTrack>,
    pub grid_template_rows: Vec<GridTrack>,
    pub grid_column: GridPlacement,
    pub grid_row: GridPlacement,
    pub grid_auto_flow: GridAutoFlow,
    /// Grid/flex item alignment along the container's inline axis; same
    /// value space as `align_items` (start/center/end/stretch).
    pub justify_items: AlignItems,
    pub justify_self: AlignSelf,
    /// M3: which box `width`/`height` size (Tailwind preflight sets
    /// border-box document-wide via the generated utility sheet).
    pub box_sizing: BoxSizing,
}

impl ComputedStyle {
    /// The style used for text-only inline runs and as a base for defaults.
    pub(crate) fn inherited_default() -> Self {
        Self {
            display: Display::Inline,
            width: None,
            height: None,
            min_width: None,
            min_height: None,
            max_width: None,
            max_height: None,
            margin: Sides::default(),
            padding: Sides::default(),
            border_width: Sides::default(),
            border_color: Color::BLACK,
            border_style_solid: false,
            border_radius: 0.0,
            color: Color::BLACK,
            background_color: Color::TRANSPARENT,
            font_size: 16.0,
            font_weight: 400,
            line_height: LineHeight::Normal,
            text_align: TextAlign::Left,
            white_space: WhiteSpace::Normal,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Nowrap,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: None,
            justify_content: JustifyContent::FlexStart,
            align_items: AlignItems::Stretch,
            align_self: AlignSelf::Auto,
            row_gap: None,
            column_gap: None,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_column: GridPlacement::default(),
            grid_row: GridPlacement::default(),
            grid_auto_flow: GridAutoFlow::Row,
            justify_items: AlignItems::Stretch,
            justify_self: AlignSelf::Auto,
            box_sizing: BoxSizing::ContentBox,
        }
    }
}

/// A deterministic note about CSS the cascade could not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StyleDiagnostic {
    /// Which source the declaration came from.
    pub source: String,
    /// 1-based source line.
    pub line: u32,
    /// What was skipped and why.
    pub message: String,
}

impl fmt::Display for StyleDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.source, self.line, self.message)
    }
}

/// A declaration paired with its cascade position.
#[derive(Debug, Clone)]
struct Ranked<'a> {
    declaration: &'a Declaration,
    specificity: Specificity,
    /// Rule order across all author sheets (UA rules sort before these).
    order: u32,
    /// Inline styles rank above every rule of the same importance.
    inline: bool,
}

/// The cascaded stylesheet set: UA defaults + author sheets, in order.
// The cascade output is consumed by the box tree/layout stages landing in
// the next M2a commit; until then only tests drive this module.
#[allow(dead_code)]
pub(crate) struct Cascade<'a> {
    ua_rules: &'a [Rule],
    author_sheets: &'a [Stylesheet],
    /// Diagnostics accumulated while applying declarations.
    pub diagnostics: Vec<StyleDiagnostic>,
}

impl<'a> Cascade<'a> {
    pub(crate) fn new(ua_rules: &'a [Rule], author_sheets: &'a [Stylesheet]) -> Self {
        Self {
            ua_rules,
            author_sheets,
            diagnostics: Vec::new(),
        }
    }

    /// Computes the style for one element, given its parent's computed
    /// style (for inheritance). `style_attr` is the element's inline
    /// `style=""` value if any. `interaction` carries the runtime hover/
    /// focus/active state for selector matching — `None` matches nothing
    /// stateful (structural-truth mode).
    pub(crate) fn compute(
        &mut self,
        dom: &Dom,
        node: NodeId,
        parent: Option<&ComputedStyle>,
        interaction: Option<&InteractionState>,
    ) -> ComputedStyle {
        // 1. Start from UA defaults specialized by tag, then inherit.
        let mut style = ua_default(dom.tag_name(node).unwrap_or("span"));
        if let Some(parent) = parent {
            style.inherit_from(parent);
        }

        // 2. Collect matching declarations with cascade ranks.
        let style_attr = element_style_attr(dom, node);
        let inline_declarations = style_attr
            .as_deref()
            .map(parse_inline_declarations)
            .unwrap_or_default();
        let mut ranked: Vec<Ranked<'_>> = Vec::new();
        for rule in self.ua_rules {
            for selector in &rule.selectors {
                if selector_matches(dom, node, selector, interaction) {
                    for declaration in &rule.declarations {
                        ranked.push(Ranked {
                            declaration,
                            specificity: selector.specificity,
                            order: rule.order,
                            inline: false,
                        });
                    }
                }
            }
        }
        for sheet in self.author_sheets {
            for rule in &sheet.rules {
                for selector in &rule.selectors {
                    if selector_matches(dom, node, selector, interaction) {
                        // Interaction selectors are paint-only in M4b
                        // (ADR 0011): layout-affecting declarations are
                        // deferred with a diagnostic, so pointer motion
                        // can never invalidate layout. Pseudo-classes
                        // contribute class-level specificity normally.
                        let stateful = selector.segments.iter().any(|s| {
                            s.compound
                                .simples
                                .iter()
                                .any(|simple| simple.is_interaction())
                        });
                        for declaration in &rule.declarations {
                            if stateful && !is_presentation_property(&declaration.property) {
                                self.diagnostics.push(StyleDiagnostic {
                                    source: dom.tag_name(node).unwrap_or("?").to_owned(),
                                    line: declaration.line,
                                    message: format!(
                                        "interaction selector may only change paint; \
                                         property \"{}\" deferred (M4b profile)",
                                        declaration.property
                                    ),
                                });
                                continue;
                            }
                            ranked.push(Ranked {
                                declaration,
                                specificity: selector.specificity,
                                order: rule.order,
                                inline: false,
                            });
                        }
                    }
                }
            }
        }
        for declaration in &inline_declarations {
            ranked.push(Ranked {
                declaration,
                specificity: Specificity {
                    ids: 0,
                    classes: 0,
                    elements: 0,
                },
                order: u32::MAX,
                inline: true,
            });
        }

        // 3. Sort by (important, inline, specificity, order) ascending; last
        //    write per property wins.
        ranked.sort_by(|a, b| {
            (
                a.declaration.important,
                a.inline,
                a.specificity.ids,
                a.specificity.classes,
                a.specificity.elements,
                a.order,
            )
                .cmp(&(
                    b.declaration.important,
                    b.inline,
                    b.specificity.ids,
                    b.specificity.classes,
                    b.specificity.elements,
                    b.order,
                ))
        });

        // 4. Apply in order.
        let source_name = dom.tag_name(node).unwrap_or("?").to_owned();
        for ranked in &ranked {
            apply_declaration(
                &mut style,
                ranked.declaration,
                &source_name,
                &mut self.diagnostics,
            );
        }
        style
    }
}

impl ComputedStyle {
    /// Copies inheritable properties from `parent`.
    fn inherit_from(&mut self, parent: &ComputedStyle) {
        self.color = parent.color;
        self.font_size = parent.font_size;
        self.font_weight = parent.font_weight;
        self.line_height = parent.line_height;
        self.text_align = parent.text_align;
        self.white_space = parent.white_space;
        self.background_color = Color::TRANSPARENT; // backgrounds do not inherit
    }
}

/// UA default style per tag name (M2a subset, px-based).
fn ua_default(tag: &str) -> ComputedStyle {
    let mut style = ComputedStyle::inherited_default();
    match tag {
        "html" | "body" => {
            style.display = Display::Block;
        }
        "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "li" | "section"
        | "article" | "header" | "footer" | "nav" | "aside" | "main" | "form" | "figure"
        | "blockquote" | "table" | "img" => {
            // `img` (M2c): replaced elements participate as real boxes
            // (block/flex items); inline text flow around them is deferred.
            style.display = Display::Block;
        }
        "head" | "script" | "style" | "template" | "meta" | "title" | "link" => {
            style.display = Display::None;
        }
        _ => {
            // span, b, i, em, strong, code, label, … stay inline.
            style.display = Display::Inline;
        }
    }
    match tag {
        "body" => {
            style.margin = sides_px(8.0);
            style.background_color = Color::TRANSPARENT;
        }
        "h1" => {
            style.font_size = 32.0;
            style.font_weight = 700;
            style.margin = Sides {
                top: Length::Px(21.0),
                right: Length::Px(0.0),
                bottom: Length::Px(21.0),
                left: Length::Px(0.0),
            };
        }
        "h2" => {
            style.font_size = 24.0;
            style.font_weight = 700;
            style.margin = Sides {
                top: Length::Px(19.0),
                right: Length::Px(0.0),
                bottom: Length::Px(19.0),
                left: Length::Px(0.0),
            };
        }
        "h3" => {
            style.font_size = 19.0;
            style.font_weight = 700;
            style.margin = sides_v(18.0);
        }
        "p" => {
            style.margin = sides_v(16.0);
        }
        "b" | "strong" => style.font_weight = 700,
        "code" | "pre" => {
            // Font family differentiation is a later milestone; keep mono
            // metrics approximation via a slight size adjustment for now.
            style.font_size = 14.0;
        }
        _ => {}
    }
    style
}

fn sides_px(v: f32) -> Sides<Length> {
    Sides {
        top: Length::Px(v),
        right: Length::Px(v),
        bottom: Length::Px(v),
        left: Length::Px(v),
    }
}

fn sides_v(v: f32) -> Sides<Length> {
    Sides {
        top: Length::Px(v),
        right: Length::Px(0.0),
        bottom: Length::Px(v),
        left: Length::Px(0.0),
    }
}

/// The element's inline `style=""` attribute, if any.
fn element_style_attr(dom: &Dom, node: NodeId) -> Option<String> {
    let NodeData::Element { attrs, .. } = &dom.node(node).data else {
        return None;
    };
    attrs
        .iter()
        .find(|a| a.name == "style")
        .map(|a| a.value.clone())
}

/// Parses an inline `style=""` value with the same declaration parser as
/// rule blocks.
fn parse_inline_declarations(text: &str) -> Vec<Declaration> {
    let mut parser = Parser::new(text);
    crate::css::parse_declarations_pub(&mut parser)
}

// -- selector matching -------------------------------------------------------

/// The runtime interaction state selectors can match against (M4b, ADR
/// 0011). Paths run from the stateful element up through its ancestors:
/// CSS `:hover`/`:active` apply through the ancestor chain, so a rule
/// like `.card:hover` activates when the pointer is over any descendant
/// of `.card`. `:focus` matches only the focused element itself
/// (`:focus-within` is a later slice).
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct InteractionState {
    /// Nodes under the pointer, hit target first, root last.
    pub hover_path: Vec<NodeId>,
    /// Nodes in the pressed chain, pressed target first, root last.
    pub active_path: Vec<NodeId>,
    /// The focused element (exact match only).
    pub focus: Option<NodeId>,
}

impl InteractionState {
    /// `None`-shaped view of "no interaction at all" for truth-seeking
    /// paths (layout facts) that must never see state.
    pub(crate) fn is_empty(&self) -> bool {
        self.hover_path.is_empty() && self.active_path.is_empty() && self.focus.is_none()
    }
}

/// Right-to-left matching: the last compound must match `node`, then walk
/// combinators through ancestors. Interaction pseudo-classes consult
/// `interaction` (`None` = match nothing stateful — the structural-truth
/// mode used by layout facts).
pub(crate) fn selector_matches(
    dom: &Dom,
    node: NodeId,
    selector: &Selector,
    interaction: Option<&InteractionState>,
) -> bool {
    let Some(last) = selector.segments.last() else {
        return false;
    };
    if !compound_matches(dom, node, &last.compound, interaction) {
        return false;
    }
    let mut cursor = node;
    for segment in selector.segments.iter().rev().skip(1) {
        match segment.combinator {
            Some(Combinator::Child) => {
                let Some(parent) = dom.node(cursor).parent else {
                    return false;
                };
                cursor = parent;
                if !compound_matches(dom, cursor, &segment.compound, interaction) {
                    return false;
                }
            }
            _ => {
                // Descendant: walk ancestors until one matches.
                let mut matched = false;
                let mut ancestor = dom.node(cursor).parent;
                while let Some(candidate) = ancestor {
                    if compound_matches(dom, candidate, &segment.compound, interaction) {
                        cursor = candidate;
                        matched = true;
                        break;
                    }
                    ancestor = dom.node(candidate).parent;
                }
                if !matched {
                    return false;
                }
            }
        }
    }
    true
}

fn compound_matches(
    dom: &Dom,
    node: NodeId,
    compound: &crate::css::Compound,
    interaction: Option<&InteractionState>,
) -> bool {
    let NodeData::Element { name, attrs, .. } = &dom.node(node).data else {
        return false;
    };
    for simple in &compound.simples {
        match simple {
            Simple::Type(t) => {
                if name != t {
                    return false;
                }
            }
            Simple::Universal => {}
            Simple::Class(class) => {
                let Some(class_attr) = attrs.iter().find(|a| a.name == "class") else {
                    return false;
                };
                if !class_attr
                    .value
                    .split_ascii_whitespace()
                    .any(|c| c == class)
                {
                    return false;
                }
            }
            Simple::PseudoClass(pseudo) => {
                let Some(state) = interaction else {
                    return false;
                };
                let matched = match pseudo.as_str() {
                    "hover" => state.hover_path.contains(&node),
                    "active" => state.active_path.contains(&node),
                    "focus" => state.focus == Some(node),
                    _ => false,
                };
                if !matched {
                    return false;
                }
            }
            Simple::Id(id) => {
                let Some(id_attr) = attrs.iter().find(|a| a.name == "id") else {
                    return false;
                };
                if id_attr.value != *id {
                    return false;
                }
            }
        }
    }
    true
}

// -- declaration application -------------------------------------------------

/// The properties an interaction selector (`:hover`/`:focus`/`:active`)
/// may change in M4b: paint/presentation only (ADR 0011). The frozen list
/// lives in velqu-tailwind next to the profile concepts, so the cascade
/// and the checker can never drift.
pub(crate) fn is_presentation_property(property: &str) -> bool {
    velqu_tailwind::is_interaction_paint_property(property)
}

/// Applies one declaration to `style`; unrecognized properties/values
/// produce a diagnostic.
fn apply_declaration(
    style: &mut ComputedStyle,
    declaration: &Declaration,
    source: &str,
    diagnostics: &mut Vec<StyleDiagnostic>,
) {
    let skip = |message: String| StyleDiagnostic {
        source: source.to_owned(),
        line: declaration.line,
        message,
    };
    let unsupported = |what: &str| {
        skip(format!(
            "unsupported {}: {:?} (M2a profile)",
            what, declaration.value
        ))
    };

    let values: Vec<&str> = declaration.value.split_ascii_whitespace().collect();
    match declaration.property.as_str() {
        "width" => match parse_length(&declaration.value) {
            Some(len) => style.width = Some(len),
            None => diagnostics.push(unsupported("length")),
        },
        "height" => match parse_length(&declaration.value) {
            Some(len) => style.height = Some(len),
            None => diagnostics.push(unsupported("length")),
        },
        "min-width" => match parse_length(&declaration.value) {
            Some(len) => style.min_width = Some(len),
            None => diagnostics.push(unsupported("length")),
        },
        "min-height" => match parse_length(&declaration.value) {
            Some(len) => style.min_height = Some(len),
            None => diagnostics.push(unsupported("length")),
        },
        "max-width" => match parse_length(&declaration.value) {
            Some(len) => style.max_width = Some(len),
            None => diagnostics.push(unsupported("length")),
        },
        "max-height" => match parse_length(&declaration.value) {
            Some(len) => style.max_height = Some(len),
            None => diagnostics.push(unsupported("length")),
        },
        "margin" | "padding" => {
            let mut lengths: Vec<Length> = Vec::new();
            let mut all_ok = !values.is_empty();
            for value in &values {
                match parse_length(value) {
                    Some(len) => lengths.push(len),
                    None => {
                        all_ok = false;
                        break;
                    }
                }
            }
            if all_ok {
                let sides = expand_sides(&lengths);
                if declaration.property == "margin" {
                    style.margin = sides;
                } else {
                    style.padding = sides;
                }
            } else {
                diagnostics.push(unsupported("margin/padding value"));
            }
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => {
            match parse_length(&declaration.value) {
                Some(len) => {
                    set_side(&mut style.margin, &declaration.property, len);
                }
                None => diagnostics.push(unsupported("length")),
            }
        }
        "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => {
            match parse_length(&declaration.value) {
                Some(len) => {
                    set_side(&mut style.padding, &declaration.property, len);
                }
                None => diagnostics.push(unsupported("length")),
            }
        }
        "border" => {
            // "1px solid #ccc" (M2a accepts any width/style/color order but
            // requires all three).
            let mut width = None;
            let mut color = None;
            let mut solid = false;
            for value in &values {
                if let Some(len) = parse_length(value) {
                    width = Some(len);
                } else if *value == "solid" {
                    solid = true;
                } else if let Some(c) = parse_color(value) {
                    color = Some(c);
                } else if *value == "none" {
                    solid = false;
                    width = Some(Length::Px(0.0));
                }
            }
            match (width, color, solid) {
                (Some(w), Some(c), true) => {
                    style.border_width = sides_all(w);
                    style.border_color = c;
                    style.border_style_solid = true;
                }
                _ if *values.first().unwrap_or(&"") == "none" || values.is_empty() => {
                    style.border_style_solid = false;
                }
                _ => diagnostics.push(unsupported("border value")),
            }
        }
        "border-width" => match parse_length(&declaration.value) {
            Some(len) => style.border_width = sides_all(len),
            None => diagnostics.push(unsupported("length")),
        },
        "border-color" => match parse_color(&declaration.value) {
            Some(c) => style.border_color = c,
            None => diagnostics.push(unsupported("color")),
        },
        "border-style" => match declaration.value.as_str() {
            "solid" => style.border_style_solid = true,
            "none" => style.border_style_solid = false,
            _ => diagnostics.push(unsupported("border-style value")),
        },
        "border-radius" => match parse_length(&declaration.value) {
            Some(Length::Px(v)) => style.border_radius = v,
            Some(_) => diagnostics.push(unsupported("border-radius value")),
            None => diagnostics.push(unsupported("length")),
        },
        "color" => match parse_color(&declaration.value) {
            Some(c) => style.color = c,
            None => diagnostics.push(unsupported("color")),
        },
        "background-color" | "background" => match parse_color(&declaration.value) {
            Some(c) => style.background_color = c,
            None => diagnostics.push(unsupported("color")),
        },
        "font-size" => match parse_length(&declaration.value) {
            Some(Length::Px(v)) => style.font_size = v,
            Some(Length::Rem(v)) => style.font_size = v,
            Some(_) => diagnostics.push(unsupported("font-size value")),
            None => diagnostics.push(unsupported("font-size value")),
        },
        "font-weight" => match declaration.value.as_str() {
            "normal" => style.font_weight = 400,
            "bold" => style.font_weight = 700,
            "100" | "200" | "300" | "400" | "500" | "600" | "700" | "800" | "900" => {
                style.font_weight = declaration.value.parse().unwrap_or(400);
            }
            _ => diagnostics.push(unsupported("font-weight value")),
        },
        "line-height" => match declaration.value.as_str() {
            "normal" => style.line_height = LineHeight::Normal,
            other => {
                if let Ok(number) = other.parse::<f32>() {
                    style.line_height = LineHeight::Number(number);
                } else if let Some(Length::Px(px)) = parse_length(other) {
                    style.line_height = LineHeight::Px(px);
                } else {
                    diagnostics.push(unsupported("line-height value"));
                }
            }
        },
        "text-align" => match declaration.value.as_str() {
            "left" | "start" => style.text_align = TextAlign::Left,
            "center" => style.text_align = TextAlign::Center,
            "right" | "end" => style.text_align = TextAlign::Right,
            _ => diagnostics.push(unsupported("text-align value")),
        },
        "white-space" => match declaration.value.as_str() {
            "normal" => style.white_space = WhiteSpace::Normal,
            "pre" | "pre-wrap" => style.white_space = WhiteSpace::Pre,
            "nowrap" => style.white_space = WhiteSpace::Nowrap,
            _ => diagnostics.push(unsupported("white-space value")),
        },
        "display" => match declaration.value.as_str() {
            "block" => style.display = Display::Block,
            "inline" => style.display = Display::Inline,
            "flex" => style.display = Display::Flex,
            "grid" => style.display = Display::Grid,
            "none" => style.display = Display::None,
            _ => diagnostics.push(unsupported("display value")),
        },
        "flex-direction" => match declaration.value.as_str() {
            "row" => style.flex_direction = FlexDirection::Row,
            "row-reverse" => style.flex_direction = FlexDirection::RowReverse,
            "column" => style.flex_direction = FlexDirection::Column,
            "column-reverse" => style.flex_direction = FlexDirection::ColumnReverse,
            _ => diagnostics.push(unsupported("flex-direction value")),
        },
        "flex-wrap" => match declaration.value.as_str() {
            "nowrap" => style.flex_wrap = FlexWrap::Nowrap,
            "wrap" => style.flex_wrap = FlexWrap::Wrap,
            _ => diagnostics.push(unsupported("flex-wrap value")),
        },
        "flex-grow" => match declaration.value.parse::<f32>() {
            Ok(grow) if grow >= 0.0 => style.flex_grow = grow,
            _ => diagnostics.push(unsupported("flex-grow value")),
        },
        "flex-shrink" => match declaration.value.parse::<f32>() {
            Ok(shrink) if shrink >= 0.0 => style.flex_shrink = shrink,
            _ => diagnostics.push(unsupported("flex-shrink value")),
        },
        "flex-basis" => match declaration.value.as_str() {
            "auto" => style.flex_basis = None,
            other => match parse_length(other) {
                Some(len) => style.flex_basis = Some(len),
                None => diagnostics.push(unsupported("flex-basis value")),
            },
        },
        "justify-content" => match declaration.value.as_str() {
            "flex-start" | "start" | "normal" => style.justify_content = JustifyContent::FlexStart,
            "center" => style.justify_content = JustifyContent::Center,
            "flex-end" | "end" => style.justify_content = JustifyContent::FlexEnd,
            "space-between" => style.justify_content = JustifyContent::SpaceBetween,
            _ => diagnostics.push(unsupported("justify-content value")),
        },
        "align-items" => match declaration.value.as_str() {
            "stretch" | "normal" => style.align_items = AlignItems::Stretch,
            "flex-start" | "start" => style.align_items = AlignItems::FlexStart,
            "center" => style.align_items = AlignItems::Center,
            "flex-end" | "end" => style.align_items = AlignItems::FlexEnd,
            // Baseline alignment is deferred (ADR 0007): diagnosed loudly,
            // laid out as flex-start. Never a silent fallback.
            "baseline" => {
                style.align_items = AlignItems::Baseline;
                diagnostics.push(skip(
                    "align-items: baseline is deferred in the M2b profile; \
                     laid out as flex-start"
                        .into(),
                ));
            }
            _ => diagnostics.push(unsupported("align-items value")),
        },
        "align-self" => match declaration.value.as_str() {
            "auto" => style.align_self = AlignSelf::Auto,
            "stretch" | "normal" => style.align_self = AlignSelf::Stretch,
            "flex-start" | "start" => style.align_self = AlignSelf::FlexStart,
            "center" => style.align_self = AlignSelf::Center,
            "flex-end" | "end" => style.align_self = AlignSelf::FlexEnd,
            "baseline" => {
                style.align_self = AlignSelf::Baseline;
                diagnostics.push(skip(
                    "align-self: baseline is deferred in the M2b profile; \
                     laid out as flex-start"
                        .into(),
                ));
            }
            _ => diagnostics.push(unsupported("align-self value")),
        },
        "gap" => {
            let mut lengths: Vec<Length> = Vec::new();
            let mut all_ok = !values.is_empty();
            for value in &values {
                match parse_length(value) {
                    Some(len) => lengths.push(len),
                    None => {
                        all_ok = false;
                        break;
                    }
                }
            }
            if all_ok {
                let (row, column) = match lengths.as_slice() {
                    [one] => (*one, *one),
                    [row, column] => (*row, *column),
                    _ => (Length::Px(0.0), Length::Px(0.0)),
                };
                style.row_gap = Some(row);
                style.column_gap = Some(column);
            } else {
                diagnostics.push(unsupported("gap value"));
            }
        }
        "row-gap" => match parse_length(&declaration.value) {
            Some(len) => style.row_gap = Some(len),
            None => diagnostics.push(unsupported("length")),
        },
        "column-gap" => match parse_length(&declaration.value) {
            Some(len) => style.column_gap = Some(len),
            None => diagnostics.push(unsupported("length")),
        },
        "overflow" => match declaration.value.as_str() {
            "visible" => {
                style.overflow_x = Overflow::Visible;
                style.overflow_y = Overflow::Visible;
            }
            "hidden" => {
                style.overflow_x = Overflow::Hidden;
                style.overflow_y = Overflow::Hidden;
            }
            "clip" => {
                style.overflow_x = Overflow::Clip;
                style.overflow_y = Overflow::Clip;
            }
            "auto" | "scroll" => {
                style.overflow_x = Overflow::Auto;
                style.overflow_y = Overflow::Auto;
            }
            _ => diagnostics.push(unsupported("overflow value")),
        },
        "overflow-x" => match declaration.value.as_str() {
            "visible" => style.overflow_x = Overflow::Visible,
            "hidden" => style.overflow_x = Overflow::Hidden,
            "clip" => style.overflow_x = Overflow::Clip,
            "auto" | "scroll" => style.overflow_x = Overflow::Auto,
            _ => diagnostics.push(unsupported("overflow value")),
        },
        "overflow-y" => match declaration.value.as_str() {
            "visible" => style.overflow_y = Overflow::Visible,
            "hidden" => style.overflow_y = Overflow::Hidden,
            "clip" => style.overflow_y = Overflow::Clip,
            "auto" | "scroll" => style.overflow_y = Overflow::Auto,
            _ => diagnostics.push(unsupported("overflow value")),
        },
        "grid-template-columns" | "grid-template-rows" => {
            match parse_track_list(&declaration.value) {
                Ok(tracks) => {
                    if declaration.property == "grid-template-columns" {
                        style.grid_template_columns = tracks;
                    } else {
                        style.grid_template_rows = tracks;
                    }
                }
                Err(message) => diagnostics.push(skip(format!(
                    "unsupported {} value {:?}: {}",
                    declaration.property, declaration.value, message
                ))),
            }
        }
        "grid-column" | "grid-row" => match parse_grid_placement(&declaration.value) {
            Ok(placement) => {
                if declaration.property == "grid-column" {
                    style.grid_column = placement;
                } else {
                    style.grid_row = placement;
                }
            }
            Err(message) => diagnostics.push(skip(format!(
                "unsupported {} value {:?}: {}",
                declaration.property, declaration.value, message
            ))),
        },
        "grid-auto-flow" => match declaration.value.as_str() {
            "row" => style.grid_auto_flow = GridAutoFlow::Row,
            "column" => style.grid_auto_flow = GridAutoFlow::Column,
            _ => diagnostics.push(unsupported(
                "grid-auto-flow value (dense is outside the M2c profile)",
            )),
        },
        "justify-items" => match declaration.value.as_str() {
            "stretch" | "normal" => style.justify_items = AlignItems::Stretch,
            "flex-start" | "start" => style.justify_items = AlignItems::FlexStart,
            "center" => style.justify_items = AlignItems::Center,
            "flex-end" | "end" => style.justify_items = AlignItems::FlexEnd,
            "baseline" => {
                style.justify_items = AlignItems::Baseline;
                diagnostics.push(skip(
                    "justify-items: baseline is deferred in the M2c profile; \
                     laid out as start"
                        .into(),
                ));
            }
            _ => diagnostics.push(unsupported("justify-items value")),
        },
        "justify-self" => match declaration.value.as_str() {
            "auto" => style.justify_self = AlignSelf::Auto,
            "stretch" | "normal" => style.justify_self = AlignSelf::Stretch,
            "flex-start" | "start" => style.justify_self = AlignSelf::FlexStart,
            "center" => style.justify_self = AlignSelf::Center,
            "flex-end" | "end" => style.justify_self = AlignSelf::FlexEnd,
            "baseline" => {
                style.justify_self = AlignSelf::Baseline;
                diagnostics.push(skip(
                    "justify-self: baseline is deferred in the M2c profile; \
                     laid out as start"
                        .into(),
                ));
            }
            _ => diagnostics.push(unsupported("justify-self value")),
        },
        "box-sizing" => match declaration.value.as_str() {
            "content-box" => style.box_sizing = BoxSizing::ContentBox,
            "border-box" => style.box_sizing = BoxSizing::BorderBox,
            _ => diagnostics.push(unsupported("box-sizing value")),
        },
        _ => diagnostics.push(skip(format!(
            "property {:?} is outside the M2b profile",
            declaration.property
        ))),
    }
}

// -- grid track / placement parsing (M2c profile) ----------------------------

/// Splits `value` on `sep`, ignoring separators nested in parentheses
/// (so `minmax(20px, 1fr)` survives a comma split).
fn split_top_level(value: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c == sep && depth == 0 => {
                parts.push(value[start..i].trim());
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim());
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

/// Splits a track list into top-level items: whitespace-separated, but a
/// parenthesized group (`minmax(…)`, `repeat(…)` — including nested
/// repeats) is always one item.
fn top_level_tokens(value: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    for (i, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c.is_ascii_whitespace() && depth == 0 => {
                if let Some(s) = start.take() {
                    tokens.push(&value[s..i]);
                }
            }
            _ if start.is_none() => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        tokens.push(&value[s..]);
    }
    tokens
}

/// Splits `s` at its first top-level (non-parenthesized) comma.
fn split_first_top_level_comma(s: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return Some((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Parses a `grid-template-columns`/`grid-template-rows` track list
/// (M2c profile: px, %, fr, auto, minmax(), repeat(fixed integer, …)).
fn parse_track_list(value: &str) -> Result<Vec<GridTrack>, String> {
    top_level_tokens(value)
        .iter()
        .map(|token| parse_grid_track(token))
        .collect()
}

fn parse_grid_track(token: &str) -> Result<GridTrack, String> {
    if let Some(rest) = token.strip_prefix("repeat(") {
        let Some(inner) = rest.strip_suffix(')') else {
            return Err("unbalanced parentheses".into());
        };
        // Grammar inside repeat(): `count , tracks…`. Only the count is
        // comma-separated; the tracks are whitespace-separated and may
        // contain nested parens (which is how nested repeats are detected).
        let Some((count_token, tracks_str)) = split_first_top_level_comma(inner) else {
            return Err("repeat(count, tracks…) needs a count and at least one track".into());
        };
        let count: u16 = count_token.trim().parse().map_err(|_| {
            String::from(
                "repeat count must be a fixed integer (auto-fill/auto-fit are \
                 outside the M2c profile)",
            )
        })?;
        if count == 0 {
            return Err("repeat count must be >= 1".into());
        }
        let mut tracks = Vec::new();
        for track_token in top_level_tokens(tracks_str) {
            tracks.push(parse_grid_track(track_token)?);
        }
        if tracks.is_empty() {
            return Err("repeat needs at least one track".into());
        }
        if tracks
            .iter()
            .any(|track| matches!(track, GridTrack::Repeat { .. }))
        {
            return Err("nested repeat() is outside the M2c profile".into());
        }
        Ok(GridTrack::Repeat { count, tracks })
    } else if let Some(rest) = token.strip_prefix("minmax(") {
        let Some(inner) = rest.strip_suffix(')') else {
            return Err("unbalanced parentheses".into());
        };
        let parts = split_top_level(inner, ',');
        let [min, max] = parts.as_slice() else {
            return Err("minmax(min, max) takes exactly two values".into());
        };
        Ok(GridTrack::MinMax(
            parse_track_min(min.trim())?,
            parse_track_max(max.trim())?,
        ))
    } else if token == "auto" {
        Ok(GridTrack::Auto)
    } else if let Some(fr) = token.strip_suffix("fr") {
        let value: f32 = fr
            .trim()
            .parse()
            .map_err(|_| format!("invalid fr value {token:?}"))?;
        if value < 0.0 {
            return Err("fr values must be >= 0".into());
        }
        Ok(GridTrack::Fr(value))
    } else {
        match parse_length(token) {
            Some(Length::Px(v)) => Ok(GridTrack::Px(v)),
            // rem resolves against the root font size in parse_length.
            Some(Length::Rem(v)) => Ok(GridTrack::Px(v)),
            Some(Length::Percent(p)) => Ok(GridTrack::Percent(p)),
            None => Err(format!(
                "unsupported track {token:?} (named lines and \
                template areas are outside the M2c profile)"
            )),
        }
    }
}

fn parse_track_min(token: &str) -> Result<GridTrackMin, String> {
    match token {
        "auto" => Ok(GridTrackMin::Auto),
        _ => match parse_length(token) {
            Some(Length::Px(v)) | Some(Length::Rem(v)) => Ok(GridTrackMin::Px(v)),
            Some(Length::Percent(p)) => Ok(GridTrackMin::Percent(p)),
            None => Err(format!("invalid minmax min {token:?} (length or auto)")),
        },
    }
}

fn parse_track_max(token: &str) -> Result<GridTrackMax, String> {
    match token {
        "auto" => Ok(GridTrackMax::Auto),
        _ if token.ends_with("fr") => {
            let value: f32 = token
                .strip_suffix("fr")
                .and_then(|v| v.trim().parse().ok())
                .ok_or_else(|| format!("invalid fr value {token:?}"))?;
            if value < 0.0 {
                return Err("fr values must be >= 0".into());
            }
            Ok(GridTrackMax::Fr(value))
        }
        _ => match parse_length(token) {
            Some(Length::Px(v)) | Some(Length::Rem(v)) => Ok(GridTrackMax::Px(v)),
            Some(Length::Percent(p)) => Ok(GridTrackMax::Percent(p)),
            None => Err(format!(
                "invalid minmax max {token:?} (length, %, fr, or auto)"
            )),
        },
    }
}

/// Parses a `grid-column`/`grid-row` value: `<start>` or `<start> / <end>`.
fn parse_grid_placement(value: &str) -> Result<GridPlacement, String> {
    let sides = split_top_level(value, '/');
    let (start_token, end_token) = match sides.as_slice() {
        [only] => (*only, "auto"),
        [start, end] => (*start, *end),
        _ => return Err("placement is `<start> / <end>`".into()),
    };
    Ok(GridPlacement {
        start: parse_grid_line(start_token)?,
        end: parse_grid_line(end_token)?,
    })
}

fn parse_grid_line(token: &str) -> Result<GridLine, String> {
    if token == "auto" {
        Ok(GridLine::Auto)
    } else if let Some(span) = token.strip_prefix("span") {
        let value: u16 = span
            .trim()
            .parse()
            .map_err(|_| format!("span needs an integer >= 1, got {token:?}"))?;
        if value == 0 {
            return Err("span must be >= 1".into());
        }
        Ok(GridLine::Span(value))
    } else if let Ok(index) = token.parse::<i16>() {
        if index >= 1 {
            Ok(GridLine::Index(index))
        } else {
            Err("negative line numbers are outside the M2c profile".into())
        }
    } else {
        Err(format!(
            "unsupported grid line {token:?} (named lines are outside the M2c profile)"
        ))
    }
}

fn sides_all(len: Length) -> Sides<Length> {
    Sides {
        top: len,
        right: len,
        bottom: len,
        left: len,
    }
}

fn set_side(sides: &mut Sides<Length>, property: &str, len: Length) {
    match property {
        "margin-top" | "padding-top" => sides.top = len,
        "margin-right" | "padding-right" => sides.right = len,
        "margin-bottom" | "padding-bottom" => sides.bottom = len,
        "margin-left" | "padding-left" => sides.left = len,
        _ => {}
    }
}

/// CSS side shorthand expansion: 1–4 values (top right bottom left).
fn expand_sides(values: &[Length]) -> Sides<Length> {
    match values.len() {
        1 => sides_all(values[0]),
        2 => Sides {
            top: values[0],
            right: values[1],
            bottom: values[0],
            left: values[1],
        },
        3 => Sides {
            top: values[0],
            right: values[1],
            bottom: values[2],
            left: values[1],
        },
        _ => Sides {
            top: values[0],
            right: values[1],
            bottom: values[2],
            left: values[3],
        },
    }
}

/// Parses `12px`, `50%`, `2.5rem` (em is deferred per the M2a profile).
pub(crate) fn parse_length(value: &str) -> Option<Length> {
    let value = value.trim();
    let (number, unit) = value.split_at(
        value
            .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
            .unwrap_or(value.len()),
    );
    let number: f32 = number.parse().ok()?;
    match unit {
        "px" | "" => Some(Length::Px(number)),
        "%" => Some(Length::Percent(number)),
        "rem" => Some(Length::Rem(number * 16.0)),
        _ => None,
    }
}

/// Named colors + `#rgb`/`#rrggbb` + `rgb(r, g, b)` (sRGB only in M2a).
pub(crate) fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();
    if let Ok(color) = Color::from_hex(value) {
        return Some(color);
    }
    // 3-digit hex (#rgb → #rrggbb).
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 3 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            let expanded: String = hex.chars().flat_map(|c| [c, c]).collect();
            if let Ok(color) = Color::from_hex(&expanded) {
                return Some(color);
            }
        }
    }
    if let Some(inner) = value
        .strip_prefix("rgb(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let channels: Vec<&str> = inner.split(',').map(str::trim).collect();
        if channels.len() == 3 {
            let (r, g, b) = (
                channels[0].parse().ok()?,
                channels[1].parse().ok()?,
                channels[2].parse().ok()?,
            );
            return Some(Color::from_rgb8(r, g, b));
        }
    }
    Some(match value {
        "black" => Color::from_rgb8(0, 0, 0),
        "white" => Color::from_rgb8(255, 255, 255),
        "red" => Color::from_rgb8(255, 0, 0),
        "green" => Color::from_rgb8(0, 128, 0),
        "blue" => Color::from_rgb8(0, 0, 255),
        "yellow" => Color::from_rgb8(255, 255, 0),
        "orange" => Color::from_rgb8(255, 165, 0),
        "gray" | "grey" => Color::from_rgb8(128, 128, 128),
        "transparent" => Color::TRANSPARENT,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css;
    use crate::source::StylesheetSource;

    /// Builds a Dom + author sheets and computes styles for all elements.
    struct Fixture {
        dom: Dom,
        cascade_sheets: Vec<Stylesheet>,
        ua_rules: Vec<Rule>,
    }

    fn build(html: &str, author_css: &[&str]) -> Fixture {
        let dom = crate::html::parse(html);
        let mut order = 0;
        let cascade_sheets: Vec<Stylesheet> = author_css
            .iter()
            .enumerate()
            .map(|(i, css_text)| {
                let sheet = StylesheetSource::new(format!("s{i}.css"), *css_text);
                let parsed = css::parse(&sheet, order);
                order += parsed.rules.len() as u32;
                parsed
            })
            .collect();
        let ua = StylesheetSource::new("ua.css", "");
        let ua_rules = css::parse(&ua, 0).rules;
        Fixture {
            dom,
            cascade_sheets,
            ua_rules,
        }
    }

    impl Fixture {
        /// Computes the style for the element with `fixture_id`, walking
        /// its ancestor chain so inheritance applies.
        fn compute_for(&mut self, fixture_id: &str) -> (ComputedStyle, Vec<StyleDiagnostic>) {
            self.compute_for_with(fixture_id, None)
        }

        /// [`Fixture::compute_for`] with explicit interaction state for
        /// stateful-selector tests (M4b).
        fn compute_for_with(
            &mut self,
            fixture_id: &str,
            interaction: Option<&InteractionState>,
        ) -> (ComputedStyle, Vec<StyleDiagnostic>) {
            let mut target = None;
            self.dom.walk(|id, node| {
                if let NodeData::Element {
                    fixture_id: key, ..
                } = &node.data
                {
                    if key.as_deref() == Some(fixture_id) {
                        target = Some(id);
                    }
                }
            });
            let target = target.expect("fixture id present");
            let mut chain = Vec::new();
            let mut cursor = Some(target);
            while let Some(id) = cursor {
                chain.push(id);
                cursor = self.dom.node(id).parent;
            }
            chain.reverse();
            let mut cascade = Cascade::new(&self.ua_rules, &self.cascade_sheets);
            let mut style = None;
            for id in chain {
                let parent = style.clone();
                style = Some(cascade.compute(&self.dom, id, parent.as_ref(), interaction));
            }
            (style.unwrap(), std::mem::take(&mut cascade.diagnostics))
        }
    }

    #[test]
    fn id_beats_class_beats_type() {
        let mut fx = build(
            "<div id=main class=card><p data-vv-test=p>text</p></div>",
            &[
                "p { color: #000001 }",
                ".card p { color: #000002 }",
                "#main p { color: #000003 }",
            ],
        );
        let (style, _) = fx.compute_for("p");
        assert_eq!(style.color, Color::from_hex("#000003").unwrap());
    }

    #[test]
    fn later_rule_wins_at_equal_specificity() {
        let mut fx = build(
            "<p data-vv-test=p>x</p>",
            &["p { color: #000001 }", "p { color: #000002 }"],
        );
        let (style, _) = fx.compute_for("p");
        assert_eq!(style.color, Color::from_hex("#000002").unwrap());
    }

    // -- M4b interaction selectors (ADR 0011) -----------------------------

    impl Fixture {
        /// The DOM node carrying `fixture_id`.
        fn node_of(&self, fixture_id: &str) -> NodeId {
            let mut target = None;
            self.dom.walk(|id, node| {
                if let NodeData::Element {
                    fixture_id: key, ..
                } = &node.data
                {
                    if key.as_deref() == Some(fixture_id) {
                        target = Some(id);
                    }
                }
            });
            target.expect("fixture id present")
        }

        /// State matching `fixture_id` being hovered: the hover path runs
        /// target → root, so ancestors match `:hover` too.
        fn hover_state(&self, fixture_id: &str) -> InteractionState {
            let mut path = Vec::new();
            let mut cursor = Some(self.node_of(fixture_id));
            while let Some(id) = cursor {
                path.push(id);
                cursor = self.dom.node(id).parent;
            }
            InteractionState {
                hover_path: path,
                active_path: Vec::new(),
                focus: None,
            }
        }

        /// State matching `fixture_id` being focused (exact, no chain).
        fn focus_state(&self, fixture_id: &str) -> InteractionState {
            InteractionState {
                hover_path: Vec::new(),
                active_path: Vec::new(),
                focus: Some(self.node_of(fixture_id)),
            }
        }
    }

    #[test]
    fn hover_selects_only_under_state() {
        let mut fx = build(
            "<div class=card><p data-vv-test=p>x</p></div>",
            &[".card { color: #000001 }", ".card:hover { color: #000002 }"],
        );
        // No hover: base color.
        let (plain, _) = fx.compute_for("p");
        assert_eq!(plain.color, Color::from_hex("#000001").unwrap());
        // Hovering the paragraph itself: `.card:hover` activates through
        // the ancestor chain, and the color inherits to `p`.
        let state = fx.hover_state("p");
        let (hovered, _) = fx.compute_for_with("p", Some(&state));
        assert_eq!(hovered.color, Color::from_hex("#000002").unwrap());
    }

    #[test]
    fn parent_hover_activates_from_a_hovered_child() {
        let mut fx = build(
            "<div class=card data-vv-test=card><span data-vv-test=span>label</span></div>",
            &[".card:hover { background-color: #112233 }"],
        );
        // Pointer over the span: the card is in the span's hover chain.
        let state = fx.hover_state("span");
        let (card, _) = fx.compute_for_with("card", Some(&state));
        assert_eq!(
            card.background_color,
            Color::from_hex("#112233").unwrap(),
            "an ancestor matches :hover when a descendant is hovered"
        );
    }

    #[test]
    fn hover_specificity_is_class_level() {
        let mut fx = build(
            "<p data-vv-test=p class=card>x</p>",
            &[
                // The :hover rule comes FIRST but has higher specificity.
                ".card:hover { background-color: #112233 }",
                ".card { background-color: #445566 }",
            ],
        );
        let state = fx.hover_state("p");
        let (hovered, _) = fx.compute_for_with("p", Some(&state));
        assert_eq!(
            hovered.background_color,
            Color::from_hex("#112233").unwrap()
        );
        let (plain, _) = fx.compute_for("p");
        assert_eq!(plain.background_color, Color::from_hex("#445566").unwrap());
    }

    #[test]
    fn focus_matches_the_focused_element_exactly() {
        let mut fx = build(
            "<div class=card data-vv-test=card><button data-vv-test=btn>x</button></div>",
            &[
                ".card:focus { background-color: #112233 }",
                ":focus { color: #000044 }",
            ],
        );
        let state = fx.focus_state("btn");
        // :focus does NOT chain: the card is not focused when its child is.
        let (card, _) = fx.compute_for_with("card", Some(&state));
        assert_eq!(card.background_color, Color::TRANSPARENT);
        let (btn, _) = fx.compute_for_with("btn", Some(&state));
        assert_eq!(btn.color, Color::from_hex("#000044").unwrap());
    }

    #[test]
    fn interaction_selectors_may_only_change_paint() {
        let mut fx = build(
            "<div class=card data-vv-test=card>x</div>",
            &[
                ".card { width: 100px; background-color: #445566 }",
                // Layout-affecting hover: deferred with a diagnostic.
                ".card:hover { width: 500px; background-color: #112233 }",
            ],
        );
        let state = fx.hover_state("card");
        let (card, diagnostics) = fx.compute_for_with("card", Some(&state));
        // Paint applies; layout defers.
        assert_eq!(card.background_color, Color::from_hex("#112233").unwrap());
        assert_eq!(card.width, Some(crate::style::Length::Px(100.0)));
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].message.contains("width"));
        assert!(diagnostics[0].message.contains("deferred"));
        // With no state the rule never matches and never diagnoses.
        let (_, none) = fx.compute_for("card");
        assert!(none.is_empty(), "{none:?}");
    }

    #[test]
    fn unsupported_pseudo_classes_are_diagnosed_and_skipped() {
        let mut fx = build(
            "<p data-vv-test=p>x</p>",
            &["p:first-child { color: #000001 }", "p { color: #000002 }"],
        );
        let (style, _) = fx.compute_for("p");
        assert_eq!(style.color, Color::from_hex("#000002").unwrap());
        let sheet = css::parse(
            &StylesheetSource::new("t.css", "p:first-child { color: #000001 }"),
            0,
        );
        assert!(sheet.rules.is_empty(), "the rule is skipped entirely");
        assert_eq!(sheet.diagnostics.len(), 1);
        assert!(
            sheet.diagnostics[0].to_string().contains("first-child"),
            "{:?}",
            sheet.diagnostics
        );
    }

    #[test]
    fn important_outranks_higher_specificity() {
        let mut fx = build(
            "<div class=card><p data-vv-test=p>x</p></div>",
            &[
                ".card p { color: #000001 }",
                "p { color: #000002 !important }",
            ],
        );
        let (style, _) = fx.compute_for("p");
        assert_eq!(style.color, Color::from_hex("#000002").unwrap());
    }

    #[test]
    fn inline_style_beats_normal_author_rules() {
        let mut fx = build(
            "<p data-vv-test=p style=\"color: #000009\">x</p>",
            &["p { color: #000001 }"],
        );
        let (style, _) = fx.compute_for("p");
        assert_eq!(style.color, Color::from_hex("#000009").unwrap());
    }

    #[test]
    fn descendant_and_child_selectors_match_correctly() {
        let mut fx = build(
            "<section><div><span data-vv-test=deep>t</span></div></section>\
             <section><p data-vv-test=shallow>t</p></section>",
            &[
                "section > span { display: none }",
                "section span { display: inline }",
            ],
        );
        // deep: section span matches (display inline), section > span does not.
        let (style, _) = fx.compute_for("deep");
        assert_eq!(style.display, Display::Inline);
        // shallow is a p, not a span: neither matches; UA p default is block.
        let (style, _) = fx.compute_for("shallow");
        assert_eq!(style.display, Display::Block);
    }

    #[test]
    fn inheritance_carries_typography() {
        let mut fx = build(
            "<div data-vv-test=outer style=\"color: #123456; font-size: 20px\">\
               <span data-vv-test=inner>t</span></div>",
            &[],
        );
        let (inner, _) = fx.compute_for("inner");
        assert_eq!(inner.color, Color::from_hex("#123456").unwrap());
        assert_eq!(inner.font_size, 20.0);
        // Box properties do not inherit.
        assert_eq!(inner.margin.top, Length::Px(0.0));
    }

    #[test]
    fn ua_defaults_body_margin_and_head_hidden() {
        let dom = crate::html::parse(
            "<html><head><title>t</title></head>\
             <body data-vv-test=body><div>x</div></body></html>",
        );
        let ua_css = StylesheetSource::new("ua.css", "");
        let ua_rules = css::parse(&ua_css, 0).rules;
        let mut cascade = Cascade::new(&ua_rules, &[]);
        // Find body.
        let mut body = None;
        dom.walk(|id, _node| {
            if dom.tag_name(id) == Some("body") {
                body = Some(id);
            }
        });
        let style = cascade.compute(&dom, body.unwrap(), None, None);
        assert_eq!(style.margin.top, Length::Px(8.0));
        assert_eq!(style.display, Display::Block);
    }

    #[test]
    fn shorthands_expand_and_lengths_parse() {
        let mut fx = build(
            "<div data-vv-test=d style=\"margin: 1px 2px 3px 4px; padding: 5% 10px\">x</div>",
            &[],
        );
        let (style, _) = fx.compute_for("d");
        assert_eq!(
            (
                style.margin.top.px_or_zero(),
                style.margin.right.px_or_zero(),
                style.margin.bottom.px_or_zero(),
                style.margin.left.px_or_zero()
            ),
            (1.0, 2.0, 3.0, 4.0)
        );
        assert_eq!(style.padding.top, Length::Percent(5.0));
        assert_eq!(style.padding.left, Length::Px(10.0));
    }

    #[test]
    fn grid_track_lists_parse_into_the_profile() {
        let mut fx = build("<p data-vv-test=p>x</p>", &[]);
        let (style, _) = fx.compute_for("p");
        // Defaults: empty templates, auto placements.
        assert!(style.grid_template_columns.is_empty());
        assert_eq!(style.grid_column, GridPlacement::default());

        let mut fx = build(
            "<p data-vv-test=p>x</p>",
            &["p { grid-template-columns: 1fr 2fr; grid-template-rows: 100px auto }"],
        );
        let (style, diagnostics) = fx.compute_for("p");
        assert!(diagnostics.is_empty());
        assert_eq!(
            style.grid_template_columns,
            [GridTrack::Fr(1.0), GridTrack::Fr(2.0)]
        );
        assert_eq!(
            style.grid_template_rows,
            [GridTrack::Px(100.0), GridTrack::Auto]
        );

        let mut fx = build(
            "<p data-vv-test=p>x</p>",
            &["p { grid-template-columns: minmax(20px, 1fr) repeat(3, 50% auto) }"],
        );
        let (style, diagnostics) = fx.compute_for("p");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            style.grid_template_columns,
            [
                GridTrack::MinMax(GridTrackMin::Px(20.0), GridTrackMax::Fr(1.0)),
                GridTrack::Repeat {
                    count: 3,
                    tracks: vec![GridTrack::Percent(50.0), GridTrack::Auto],
                },
            ]
        );
    }

    #[test]
    fn grid_profile_deferrals_are_loud() {
        // auto-fill/auto-fit, dense packing, named lines, negative lines.
        for (property, value, forbidden) in [
            (
                "grid-template-columns",
                "repeat(auto-fill, 1fr)",
                "fixed integer",
            ),
            (
                "grid-template-columns",
                "repeat(2, repeat(2, 1fr))",
                "nested repeat",
            ),
            (
                "grid-template-columns",
                "[main] 1fr [alt]",
                "unsupported track",
            ),
            ("grid-auto-flow", "row dense", "grid-auto-flow"),
            ("grid-column", "-1 / 3", "negative"),
        ] {
            let mut fx = build(
                "<p data-vv-test=p>x</p>",
                &[&format!("p {{ {property}: {value} }}")],
            );
            let (_style, diagnostics) = fx.compute_for("p");
            assert!(
                diagnostics.iter().any(|d| d.message.contains(forbidden)),
                "{property}: {value} should be diagnosed ({diagnostics:?})"
            );
        }
    }

    #[test]
    fn grid_placements_parse() {
        let mut fx = build(
            "<div data-vv-test=a style=\"grid-column: 1 / 3; grid-row: span 2\">x</div>",
            &[],
        );
        let (style, diagnostics) = fx.compute_for("a");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(
            style.grid_column,
            GridPlacement {
                start: GridLine::Index(1),
                end: GridLine::Index(3),
            }
        );
        assert_eq!(
            style.grid_row,
            GridPlacement {
                start: GridLine::Span(2),
                end: GridLine::Auto,
            }
        );
    }

    #[test]
    fn unknown_property_produces_diagnostic() {
        let mut fx = build(
            "<p data-vv-test=p style=\"order: 2; color: red\">x</p>",
            &["p { backdrop-filter: blur(2px); font-weight: 700 }"],
        );
        let (style, diagnostics) = fx.compute_for("p");
        assert_eq!(style.font_weight, 700, "valid declarations still apply");
        assert_eq!(diagnostics.len(), 2, "unknown property + unknown value");
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("backdrop-filter"))
        );
        assert!(diagnostics.iter().any(|d| d.message.contains("order")));
    }

    #[test]
    fn rem_resolves_against_root_font_size() {
        let mut fx = build("<div data-vv-test=d style=\"margin: 2rem\">x</div>", &[]);
        let (style, _) = fx.compute_for("d");
        assert_eq!(style.margin.top, Length::Rem(32.0));
        assert_eq!(style.margin.top.px_or_zero(), 32.0);
    }

    #[test]
    fn display_none_comes_from_author_style() {
        let mut fx = build(
            "<div data-vv-test=hidden style=\"display: none\">x</div>",
            &[],
        );
        let (style, _) = fx.compute_for("hidden");
        assert_eq!(style.display, Display::None);
    }
}
