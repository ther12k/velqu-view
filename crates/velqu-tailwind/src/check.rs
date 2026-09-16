//! The profile checker: parsed CSS in, per-construct verdicts out —
//! the API behind the `velqu-css-check` CLI (ADR 0009). Parsing is
//! cssparser-based (the same crate the renderer uses) and reports 1-based
//! source lines; classification is the existing concept-level API.

use std::fmt;

use cssparser::{Parser, Token};

use crate::{Classification, classify_at_rule, classify_declaration};

/// One check result: a construct, where it sits in the source, and its
/// verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckItem {
    /// 1-based source line of the construct's start.
    pub line: usize,
    /// What was checked.
    pub kind: CheckKind,
    /// The verdict (with replacement suggestion, when known).
    pub classification: Classification,
}

/// The construct a [`CheckItem`] describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckKind {
    /// A declaration `property: value`.
    Declaration {
        /// Lowercased property name.
        property: String,
        /// Raw value text (trimmed).
        value: String,
    },
    /// An at-rule `@name`.
    AtRule {
        /// Lowercased at-rule name, without the `@`.
        name: String,
    },
}

impl fmt::Display for CheckItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            CheckKind::Declaration { property, value } => {
                write!(
                    f,
                    "line {}: {}: {} — {}",
                    self.line, property, value, self.classification.compatibility
                )?;
            }
            CheckKind::AtRule { name } => {
                write!(
                    f,
                    "line {}: @{} — {}",
                    self.line, name, self.classification.compatibility
                )?;
            }
        }
        if let Some(replacement) = self.classification.replacement {
            write!(f, "\n    suggestion: {replacement}")?;
        }
        Ok(())
    }
}

/// Outcome counts for one [`check_css`] run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CheckSummary {
    /// Constructs inside the profile.
    pub supported: usize,
    /// Constructs accepted and converted by the pipeline (e.g. wide-gamut
    /// colors).
    pub normalized: usize,
    /// Constructs outside the profile.
    pub unsupported: usize,
}

impl CheckSummary {
    /// Counts one verdict into its bucket.
    pub fn count(&mut self, classification: &Classification) {
        match classification.compatibility {
            crate::Compatibility::Supported => self.supported += 1,
            crate::Compatibility::Normalized => self.normalized += 1,
            crate::Compatibility::Unsupported => self.unsupported += 1,
        }
    }
}

/// Checks a CSS source: every declaration and at-rule is classified in
/// source order. Selector text is not classified (the profile covers
/// declarations and at-rules). `@media` blocks are recursed into — their
/// declarations apply at render time; other at-rule blocks are skipped
/// (already diagnosed at the at-rule itself).
pub fn check_css(source: &str) -> Vec<CheckItem> {
    let mut items = Vec::new();
    let mut parser = Parser::new(source);
    check_rules(&mut parser, &mut items);
    items
}

/// Summarizes a check run.
pub fn summarize(items: &[CheckItem]) -> CheckSummary {
    let mut summary = CheckSummary::default();
    for item in items {
        summary.count(&item.classification);
    }
    summary
}

/// Walks a rule list (top level or inside a block). Ends at EOF or at the
/// enclosing block's closing brace.
fn check_rules(parser: &mut Parser<'_>, items: &mut Vec<CheckItem>) {
    loop {
        match parser.next_including_whitespace() {
            Err(_) => break,
            Ok(Token::Comment(_)) | Ok(Token::WhiteSpace(_)) | Ok(Token::Semicolon) => {}
            Ok(Token::CloseCurlyBracket) => break,
            Ok(Token::AtKeyword(name)) => {
                let name = name.to_string().to_ascii_lowercase();
                let line = parser.current_source_location().line + 1;
                items.push(CheckItem {
                    line: line as usize,
                    kind: CheckKind::AtRule { name: name.clone() },
                    classification: classify_at_rule(&name),
                });
                consume_at_rule_body(parser, &name, items);
            }
            Ok(_) => {
                // Qualified rule: skip the prelude, then check the block.
                if at_rule_or_rule_block(parser) {
                    check_declarations_block(parser, items);
                }
            }
        }
    }
}

/// Consumes a qualified-rule prelude up to its `{`. Returns whether a
/// block was found (a stray `;` or EOF returns false).
fn at_rule_or_rule_block(parser: &mut Parser<'_>) -> bool {
    loop {
        match parser.next_including_whitespace() {
            Err(_) => return false,
            Ok(Token::Semicolon) => return false,
            Ok(Token::CurlyBracketBlock) => return true,
            Ok(Token::CloseCurlyBracket) => return false,
            Ok(_) => {}
        }
    }
}

/// Consumes an at-rule's body: the prelude up to `;` or `{`; blocks are
/// recursed for `@media` (their contents render) and skipped otherwise.
fn consume_at_rule_body(parser: &mut Parser<'_>, name: &str, items: &mut Vec<CheckItem>) {
    let is_block;
    loop {
        match parser.next_including_whitespace() {
            Err(_) => return,
            Ok(Token::Semicolon) => return,
            Ok(Token::CurlyBracketBlock) => {
                is_block = true;
                break;
            }
            Ok(Token::CloseCurlyBracket) => return,
            Ok(_) => {}
        }
    }
    if !is_block {
        return;
    }
    if name == "media" {
        let _ = parser.parse_nested_block::<_, (), cssparser::BasicParseError>(|inner| {
            check_rules(inner, items);
            Ok(())
        });
    } else {
        let _ = parser.parse_nested_block::<_, (), cssparser::BasicParseError>(|_| Ok(()));
    }
}

/// Checks the declarations of one rule block (the `{` was consumed).
fn check_declarations_block(parser: &mut Parser<'_>, items: &mut Vec<CheckItem>) {
    let _ = parser.parse_nested_block::<_, (), cssparser::BasicParseError>(|inner| {
        loop {
            match inner.next_including_whitespace() {
                Err(_) => break Ok(()),
                Ok(Token::Comment(_)) | Ok(Token::WhiteSpace(_)) | Ok(Token::Semicolon) => {}
                Ok(Token::CloseCurlyBracket) => break Ok(()),
                Ok(Token::Ident(property)) => {
                    let property = property.to_string().to_ascii_lowercase();
                    let line = inner.current_source_location().line + 1;
                    // Expect the colon; malformed declarations skip to the
                    // next semicolon.
                    match inner.next() {
                        Ok(Token::Colon) => {}
                        _ => {
                            skip_until_semicolon(inner);
                            continue;
                        }
                    }
                    let (value, end) = read_value(inner);
                    let classification = match &value {
                        Some(text) => classify_declaration(&property, text),
                        None => Classification::tier(crate::Compatibility::Unsupported),
                    };
                    items.push(CheckItem {
                        line: line as usize,
                        kind: CheckKind::Declaration {
                            property,
                            value: value.unwrap_or_default(),
                        },
                        classification,
                    });
                    let _ = end;
                }
                Ok(_) => {
                    skip_until_semicolon(inner);
                }
            }
        }
    });
}

/// Reads a declaration value as the raw source slice up to the terminating
/// semicolon or block end (the same technique the renderer's CSS parser
/// uses, so both see byte-identical value text). `None` for an empty value.
fn read_value(parser: &mut Parser<'_>) -> (Option<String>, cssparser::SourcePosition) {
    let start = parser.position();
    let mut end = start;
    loop {
        match parser.next_including_whitespace() {
            Err(_) => break,
            Ok(Token::Semicolon) => break,
            Ok(Token::CloseCurlyBracket) => break,
            Ok(Token::Function(_))
            | Ok(Token::ParenthesisBlock)
            | Ok(Token::SquareBracketBlock)
            | Ok(Token::CurlyBracketBlock) => {
                let _ = parser.parse_nested_block::<_, (), cssparser::BasicParseError>(|_| Ok(()));
                end = parser.position();
            }
            Ok(Token::WhiteSpace(_)) => {}
            Ok(_) => end = parser.position(),
        }
    }
    let text = parser.slice(start..end).trim().to_owned();
    let value = if text.is_empty() { None } else { Some(text) };
    (value, end)
}

fn skip_until_semicolon(parser: &mut Parser<'_>) {
    loop {
        match parser.next_including_whitespace() {
            Err(_) => break,
            Ok(Token::Semicolon) | Ok(Token::CloseCurlyBracket) => break,
            Ok(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Compatibility;

    #[test]
    fn declarations_classify_with_lines() {
        let items =
            check_css("a {\n  color: red;\n  position: sticky;\n  backdrop-filter: blur(4px);\n}");
        assert_eq!(items.len(), 3, "{items:?}");
        assert_eq!(items[0].line, 2);
        assert_eq!(
            items[0].classification.compatibility,
            Compatibility::Supported
        );
        assert_eq!(items[1].line, 3);
        assert_eq!(
            items[1].classification.compatibility,
            Compatibility::Unsupported
        );
        assert!(items[1].classification.replacement.is_some());
        // Unknown property: unsupported, no suggestion.
        assert_eq!(
            items[2].classification.compatibility,
            Compatibility::Unsupported
        );
        assert_eq!(items[2].classification.replacement, None);
    }

    #[test]
    fn at_rules_and_media_recursion() {
        let items = check_css(
            "@media (min-width: 100px) {\n  a { color: red; }\n}\n\
             @keyframes spin { from { opacity: 1 } }\n\
             @font-face { font-family: x }",
        );
        // @media, its inner declaration, @keyframes, @font-face.
        assert_eq!(items.len(), 4, "{items:?}");
        assert_eq!(
            items[0].classification.compatibility,
            Compatibility::Supported
        );
        // The declaration inside @media is still checked.
        assert_eq!(items[1].line, 2);
        assert_eq!(
            items[1].classification.compatibility,
            Compatibility::Supported
        );
        assert_eq!(
            items[2].kind,
            CheckKind::AtRule {
                name: "keyframes".into()
            }
        );
        assert_eq!(
            items[2].classification.compatibility,
            Compatibility::Unsupported
        );
        assert_eq!(
            items[3].kind,
            CheckKind::AtRule {
                name: "font-face".into()
            }
        );
    }

    #[test]
    fn wide_gamut_colors_are_normalized() {
        let items = check_css("a { background-color: oklch(0.7 0.1 250) }");
        assert_eq!(
            items[0].classification.compatibility,
            Compatibility::Normalized
        );
    }

    #[test]
    fn nested_function_values_survive_parsing() {
        let items = check_css("a { transform: translate(2px, min(4px, 8px)); color: red }");
        assert_eq!(items.len(), 2, "{items:?}");
        let CheckKind::Declaration { value, .. } = &items[0].kind else {
            panic!("expected a declaration");
        };
        assert_eq!(value, "translate(2px, min(4px, 8px))");
        assert_eq!(
            items[1].classification.compatibility,
            Compatibility::Supported
        );
    }

    #[test]
    fn malformed_declarations_do_not_stall_the_checker() {
        let items = check_css("a { color red; width: 10px; }");
        // `color red` is malformed (skipped); `width: 10px` still checks.
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(
            items[0].classification.compatibility,
            Compatibility::Supported
        );
    }

    #[test]
    fn summarize_buckets_verdicts() {
        let items =
            check_css("a { color: red; position: sticky; background-color: oklch(0.5 0.1 20) }");
        let summary = summarize(&items);
        assert_eq!(
            (summary.supported, summary.normalized, summary.unsupported),
            (1, 1, 1)
        );
    }

    #[test]
    fn display_formats_like_the_cli_prints() {
        let items = check_css("a {\n  position: sticky\n}");
        let text = items[0].to_string();
        assert!(text.contains("line 2"), "{text}");
        assert!(text.contains("position: sticky — unsupported"), "{text}");
        assert!(text.contains("suggestion:"), "{text}");
    }
}
