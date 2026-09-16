//! CSS syntax: stylesheet → rules → selectors + declarations, parsed with
//! cssparser (CSS Syntax Level 3 tokenization) and lowered into the small
//! Velqu model immediately — no browser-sized AST.
//!
//! Scope is the M2a CSS profile (ADR 0006): type/class/id/universal
//! selectors with descendant/child combinators, plain declarations, and
//! `!important` capture. Anything else (at-rules, exotic selectors) produces
//! a deterministic diagnostic instead of being silently ignored.

use std::fmt;

use cssparser::{Parser, ToCss, Token};

use crate::source::StylesheetSource;

/// Selector specificity, ordered (ids, classes, element types).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Specificity {
    pub ids: u16,
    pub classes: u16,
    pub elements: u16,
}

/// One simple selector within a compound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Simple {
    /// `div` (case-insensitive per HTML).
    Type(String),
    /// `*`.
    Universal,
    /// `.card` (case-sensitive).
    Class(String),
    /// `#main` (case-sensitive).
    Id(String),
}

/// One compound selector: simple selectors with no combinator between them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Compound {
    pub simples: Vec<Simple>,
}

impl Compound {
    fn specificity(&self) -> Specificity {
        let mut spec = Specificity {
            ids: 0,
            classes: 0,
            elements: 0,
        };
        for simple in &self.simples {
            match simple {
                Simple::Id(_) => spec.ids += 1,
                Simple::Class(_) => spec.classes += 1,
                Simple::Type(_) => spec.elements += 1,
                Simple::Universal => {}
            }
        }
        spec
    }
}

/// Relationship from a compound to the compound on its left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Combinator {
    /// `A B`
    Descendant,
    /// `A > B`
    Child,
}

/// One segment: a compound plus the combinator linking it to the previous
/// segment (`None` for the leftmost).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SelectorSegment {
    pub combinator: Option<Combinator>,
    pub compound: Compound,
}

/// A full complex selector with precomputed specificity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Selector {
    pub segments: Vec<SelectorSegment>,
    pub specificity: Specificity,
}

/// One `property: value` declaration. Values are stored as a normalized
/// token serialization; semantic parsing happens in the cascade/layout
/// stages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declaration {
    pub property: String,
    pub value: String,
    pub important: bool,
    /// Line in the stylesheet source (1-based) for diagnostics.
    pub line: u32,
}

/// One style rule: selector alternates + a declaration block.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    /// Position of this rule across the whole cascade (all sheets), used as
    /// the final tie-breaker after specificity and source order.
    pub order: u32,
}

/// A deterministic, machine-readable note about skipped/invalid CSS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CssDiagnostic {
    /// An at-rule outside the M2a profile; its block was skipped.
    SkippedAtRule { name: String, line: u32 },
    /// A selector failed to parse; the rule was skipped.
    InvalidSelector { line: u32 },
}

impl fmt::Display for CssDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CssDiagnostic::SkippedAtRule { name, line } => {
                write!(
                    f,
                    "line {line}: at-rule @{name} is outside the M2a profile; skipped"
                )
            }
            CssDiagnostic::InvalidSelector { line } => {
                write!(f, "line {line}: selector could not be parsed; rule skipped")
            }
        }
    }
}

/// A parsed stylesheet: rules in source order plus diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Stylesheet {
    pub source: crate::source::SourceId,
    pub rules: Vec<Rule>,
    pub diagnostics: Vec<CssDiagnostic>,
}

/// Parses a stylesheet source into [`Stylesheet`]. Lenient: invalid
/// constructs produce diagnostics, never errors.
pub(crate) fn parse(sheet: &StylesheetSource, order_start: u32) -> Stylesheet {
    let mut parsed = Stylesheet {
        source: sheet.id.clone(),
        rules: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut parser = Parser::new(&sheet.css);
    let mut order = order_start;

    loop {
        let state = parser.state();
        let line = parser.current_source_location().line;
        match parser.next_including_whitespace() {
            Ok(Token::CDO) | Ok(Token::CDC) | Ok(Token::WhiteSpace(_)) | Ok(Token::Comment(_)) => {
                continue;
            }
            Ok(Token::AtKeyword(name)) => {
                let name = name.to_string();
                // Consume the at-rule prelude up to its block (or a
                // semicolon for statement at-rules); content is not
                // interpreted in M2a.
                loop {
                    match parser.next_including_whitespace() {
                        Ok(Token::CurlyBracketBlock) => {
                            let skipped = parser
                                .parse_nested_block::<_, (), cssparser::BasicParseError>(
                                    |_nested: &mut Parser<'_>| Ok(()),
                                );
                            let _ = skipped;
                            break;
                        }
                        Ok(Token::Semicolon) | Err(_) => break,
                        _ => continue,
                    }
                }
                parsed.diagnostics.push(CssDiagnostic::SkippedAtRule {
                    name,
                    line: line + 1,
                });
            }
            Ok(_) => {
                parser.reset(&state);
                match parse_style_rule(&mut parser, order) {
                    Ok(rule) => {
                        order += 1;
                        parsed.rules.push(rule);
                    }
                    Err(diagnostic) => {
                        parsed.diagnostics.push(diagnostic);
                        skip_rule_body(&mut parser);
                    }
                }
            }
            Err(_) => break,
        }
    }

    parsed
}

/// Parses one `selectors { declarations }` rule, consuming through the end
/// of its block.
fn parse_style_rule(parser: &mut Parser<'_>, order: u32) -> Result<Rule, CssDiagnostic> {
    let line = parser.current_source_location().line;
    let (selectors, end) = parse_selector_prelude(parser)?;
    let PreludeEnd::Block(declarations) = end;
    if selectors.is_empty() {
        return Err(CssDiagnostic::InvalidSelector { line });
    }
    Ok(Rule {
        selectors,
        declarations,
        order,
    })
}

/// How the selector prelude ended.
enum PreludeEnd {
    /// A `{…}` block was consumed; its declarations are inside.
    Block(Vec<Declaration>),
}

/// Incremental selector-prelude parser state.
struct PreludeState {
    selectors: Vec<Selector>,
    segments: Vec<SelectorSegment>,
    /// Compound under construction.
    current: Compound,
    /// This compound's combinator to its left (taken when it starts).
    current_combinator: Option<Combinator>,
    /// Combinator for the compound that starts next (`>` / whitespace).
    pending: Option<Combinator>,
}

impl PreludeState {
    fn new() -> Self {
        Self {
            selectors: Vec::new(),
            segments: Vec::new(),
            current: Compound::default(),
            current_combinator: None,
            pending: None,
        }
    }

    /// Adds a simple selector, opening a new compound when needed.
    fn push_simple(&mut self, simple: Simple) {
        if self.current.simples.is_empty() {
            self.current_combinator = self.pending.take();
        }
        self.current.simples.push(simple);
    }

    /// Closes the current compound (if any) into a segment.
    fn boundary(&mut self) {
        if !self.current.simples.is_empty() {
            let combinator = self.current_combinator.take();
            let compound = std::mem::take(&mut self.current);
            self.segments.push(SelectorSegment {
                combinator,
                compound,
            });
        }
    }

    /// Completes the selector under construction and starts a new one.
    fn finish_selector(&mut self) {
        self.boundary();
        if !self.segments.is_empty() {
            let specificity = self.segments.iter().fold(
                Specificity {
                    ids: 0,
                    classes: 0,
                    elements: 0,
                },
                |a, s| {
                    let b = s.compound.specificity();
                    Specificity {
                        ids: a.ids + b.ids,
                        classes: a.classes + b.classes,
                        elements: a.elements + b.elements,
                    }
                },
            );
            self.selectors.push(Selector {
                segments: std::mem::take(&mut self.segments),
                specificity,
            });
        }
        self.pending = None;
    }
}

/// Reads selector prelude tokens until the rule's `{` (which it consumes
/// and parses as a declaration block) or a premature end.
fn parse_selector_prelude(
    parser: &mut Parser<'_>,
) -> Result<(Vec<Selector>, PreludeEnd), CssDiagnostic> {
    let line = parser.current_source_location().line;
    let mut state = PreludeState::new();

    loop {
        let token = match parser.next_including_whitespace() {
            Ok(token) => token,
            Err(_) => return Err(CssDiagnostic::InvalidSelector { line }),
        };
        match token {
            Token::Comment(_) => {}
            Token::WhiteSpace(_) => {
                if !state.current.simples.is_empty() {
                    state.boundary();
                    state.pending = Some(Combinator::Descendant);
                }
            }
            Token::Delim(star) if *star == '*' => state.push_simple(Simple::Universal),
            Token::Delim(gt) if *gt == '>' => {
                // Surrounding whitespace may already have flushed the
                // compound ("ul > li"), so an empty `current` here is
                // normal; only a leading `>` is invalid.
                state.boundary();
                if state.segments.is_empty()
                    && state.current.simples.is_empty()
                    && state.pending.is_none()
                {
                    return Err(CssDiagnostic::InvalidSelector { line });
                }
                state.pending = Some(Combinator::Child);
            }
            Token::Delim(dot) if *dot == '.' => {
                let class = match parser.next_including_whitespace() {
                    Ok(Token::Ident(name)) => name.to_string(),
                    _ => return Err(CssDiagnostic::InvalidSelector { line }),
                };
                state.push_simple(Simple::Class(class));
            }
            Token::IDHash(name) => state.push_simple(Simple::Id(name.to_string())),
            Token::Ident(name) => {
                state.push_simple(Simple::Type(name.to_string().to_ascii_lowercase()));
            }
            Token::Comma => state.finish_selector(),
            Token::CurlyBracketBlock => {
                state.finish_selector();
                let declarations: Vec<Declaration> = parser
                    .parse_nested_block::<_, Vec<Declaration>, cssparser::BasicParseError>(
                        parse_declarations,
                    )
                    .unwrap_or_default();
                return Ok((state.selectors, PreludeEnd::Block(declarations)));
            }
            Token::Semicolon => {
                return Err(CssDiagnostic::InvalidSelector { line });
            }
            _ => return Err(CssDiagnostic::InvalidSelector { line }),
        }
    }
}

fn skip_rule_body(parser: &mut Parser<'_>) {
    // Consume everything up to and including the next block, or a semicolon.
    loop {
        match parser.next_including_whitespace() {
            Ok(Token::CurlyBracketBlock) => {
                let skipped = parser.parse_nested_block::<_, (), cssparser::BasicParseError>(
                    |_nested: &mut Parser<'_>| Ok(()),
                );
                let _ = skipped;
                break;
            }
            Ok(Token::Semicolon) | Err(_) => break,
            _ => continue,
        }
    }
}

/// Parses the declaration list inside a rule block.
fn parse_declarations(
    input: &mut Parser<'_>,
) -> Result<Vec<Declaration>, cssparser::ParseError<cssparser::BasicParseError>> {
    let mut declarations = Vec::new();
    loop {
        input.skip_whitespace();
        match input.next_including_whitespace() {
            Err(_) => break,
            Ok(Token::Comment(_)) | Ok(Token::WhiteSpace(_)) | Ok(Token::Semicolon) => continue,
            Ok(Token::Ident(property)) => {
                let property = property.to_string().to_ascii_lowercase();
                let line = input.current_source_location().line;
                match input.next() {
                    Ok(Token::Colon) => {}
                    _ => {
                        // Malformed declaration: skip to the next semicolon.
                        skip_until_semicolon(input);
                        continue;
                    }
                }
                let (value, important) = read_value(input);
                declarations.push(Declaration {
                    property,
                    value,
                    important,
                    line: line + 1, // cssparser lines are 0-based; store 1-based
                });
            }
            Ok(_) => {
                skip_until_semicolon(input);
            }
        }
    }
    Ok(declarations)
}

fn skip_until_semicolon(input: &mut Parser<'_>) {
    loop {
        match input.next_including_whitespace() {
            Ok(Token::Semicolon) | Err(_) => break,
            _ => continue,
        }
    }
}

/// Appends `serialized` to `value`, inserting a single space when the
/// previous token wanted a separator (never right after an open bracket).
fn write_serialized(value: &mut String, pending_space: &mut bool, serialized: &str) {
    let last = value.chars().last();
    if !value.is_empty() && *pending_space && last != Some('(') {
        value.push(' ');
    }
    value.push_str(serialized);
    *pending_space = false;
}

/// Reads declaration value tokens up to the terminating semicolon,
/// serializing them with minimal separators. Function/bracket blocks are
/// consumed recursively so their contents are included. Returns the value
/// and whether `!important` was present.
fn read_value(input: &mut Parser<'_>) -> (String, bool) {
    let mut value = String::new();
    let mut pending_space = false;
    let mut important = false;

    while let Ok(token) = input.next_including_whitespace() {
        match token {
            Token::Comment(_) => {}
            Token::WhiteSpace(_) => pending_space = true,
            Token::Semicolon => break,
            Token::Delim(bang) if *bang == '!' => {
                // !important (or an invalid bang; either way the value ends).
                input.skip_whitespace();
                important = matches!(
                    input.next(),
                    Ok(Token::Ident(name)) if name.eq_ignore_ascii_case("important")
                );
                if !important {
                    break;
                }
                // Consume any trailing tokens up to the semicolon.
                continue;
            }
            Token::Function(_)
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock
            | Token::CurlyBracketBlock => {
                let close = match token {
                    Token::SquareBracketBlock => ']',
                    Token::CurlyBracketBlock => '}',
                    _ => ')',
                };
                // The token itself serializes as the opening part only
                // ("rgb(", "("); its contents are the nested block.
                let open = token.to_css_string();
                write_serialized(&mut value, &mut pending_space, &open);
                let nested =
                    input.parse_nested_block::<_, (), cssparser::BasicParseError>(|nested| {
                        loop {
                            match nested.next_including_whitespace() {
                                Err(_) => break,
                                Ok(Token::Comment(_)) => {}
                                Ok(Token::WhiteSpace(_)) => pending_space = true,
                                Ok(Token::CloseParenthesis)
                                | Ok(Token::CloseSquareBracket)
                                | Ok(Token::CloseCurlyBracket) => {
                                    value.push(close);
                                    pending_space = false;
                                }
                                Ok(other) => {
                                    let serialized = other.to_css_string();
                                    write_serialized(&mut value, &mut pending_space, &serialized);
                                }
                            }
                        }
                        Ok(())
                    });
                let _ = nested;
                // parse_nested_block consumed everything up to the block's
                // end; emit the closing bracket if the stream didn't.
                if !value.ends_with(close) {
                    value.push(close);
                }
                pending_space = false;
            }
            other => {
                let serialized = other.to_css_string();
                write_serialized(&mut value, &mut pending_space, &serialized);
            }
        }
    }
    (value, important)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::StylesheetSource;

    fn parse_css(css: &str) -> Stylesheet {
        let sheet = StylesheetSource::new("test.css", css);
        parse(&sheet, 0)
    }

    fn first_selector_specificity(sheet: &Stylesheet) -> Specificity {
        sheet.rules[0].selectors[0].specificity
    }

    #[test]
    fn parses_class_rule_with_declarations() {
        let sheet = parse_css(".card { margin: 4px; padding: 8px 16px; color: #333 }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].declarations.len(), 3);
        assert_eq!(sheet.rules[0].declarations[0].property, "margin");
        assert_eq!(sheet.rules[0].declarations[0].value, "4px");
        assert_eq!(sheet.rules[0].declarations[1].value, "8px 16px");
        assert_eq!(sheet.rules[0].order, 0);
        assert!(sheet.diagnostics.is_empty());
    }

    #[test]
    fn specificity_counts_ids_classes_types() {
        let sheet = parse_css("#main .card > p, div span { color: red }");
        let rule = &sheet.rules[0];
        assert_eq!(rule.selectors.len(), 2);
        let s1 = rule.selectors[0].specificity;
        assert_eq!((s1.ids, s1.classes, s1.elements), (1, 1, 1));
        let s2 = rule.selectors[1].specificity;
        assert_eq!((s2.ids, s2.classes, s2.elements), (0, 0, 2));
        assert!(s1 > s2);
    }

    #[test]
    fn descendant_and_child_combinators_parse() {
        let sheet = parse_css("nav ul > li.item { color: blue }");
        let segments = &sheet.rules[0].selectors[0].segments;
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].combinator, None);
        assert_eq!(segments[1].combinator, Some(Combinator::Descendant));
        assert_eq!(segments[2].combinator, Some(Combinator::Child));
        assert_eq!(
            segments[2].compound.simples,
            [Simple::Type("li".into()), Simple::Class("item".into())]
        );
    }

    #[test]
    fn important_is_captured() {
        let sheet = parse_css(".a { color: red !important; width: 10px }");
        assert!(sheet.rules[0].declarations[0].important);
        assert!(!sheet.rules[0].declarations[1].important);
    }

    #[test]
    fn at_rules_are_skipped_with_diagnostics() {
        let sheet =
            parse_css("@media (min-width: 640px) { .card { width: 50% } }\n.card { color: red }");
        assert_eq!(sheet.rules.len(), 1, "only the plain rule is kept");
        assert_eq!(sheet.rules[0].declarations[0].property, "color");
        assert!(matches!(
            sheet.diagnostics[0],
            CssDiagnostic::SkippedAtRule { ref name, .. } if name == "media"
        ));
    }

    #[test]
    fn invalid_selector_is_diagnosed_not_fatal() {
        let sheet = parse_css(".good { color: red }\n>broken { color: blue }");
        assert_eq!(sheet.rules.len(), 1);
        assert!(
            sheet
                .diagnostics
                .iter()
                .any(|d| matches!(d, CssDiagnostic::InvalidSelector { .. }))
        );
    }

    #[test]
    fn function_values_serialize_intact() {
        let sheet = parse_css(".a { color: rgb(1, 2, 3); transform: translate(4px, 8px) }");
        let decls = &sheet.rules[0].declarations;
        assert_eq!(decls[0].value, "rgb(1, 2, 3)");
        assert_eq!(decls[1].value, "translate(4px, 8px)");
    }

    #[test]
    fn source_order_is_global_and_lines_recorded() {
        let sheet = parse_css(".a { top: 1px }\n.b { top: 2px }");
        assert_eq!(sheet.rules[0].order, 0);
        assert_eq!(sheet.rules[1].order, 1);
        assert_eq!(sheet.rules[1].declarations[0].line, 2);
    }

    #[test]
    fn first_selector_specificity_helper_agrees() {
        let sheet = parse_css("div { margin: 0 }");
        assert_eq!(
            first_selector_specificity(&sheet),
            Specificity {
                ids: 0,
                classes: 0,
                elements: 1
            }
        );
    }
}
